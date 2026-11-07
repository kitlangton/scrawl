//! Supervised private process for retained Media/2 jobs.
//!
//! The service must authenticate the capture owner before calling `Active::start`.
//! This module repeats that check before admission, runs pin/hash/decode in the
//! re-exec child, and reconciles durable status only after the child is reaped.

#[cfg(all(any(target_os = "linux", target_os = "macos"), feature = "media"))]
mod native {
    #[cfg(target_os = "linux")]
    use crate::materials_media::execute_retained_linux as execute_retained;
    #[cfg(target_os = "macos")]
    use crate::materials_media::execute_retained_macos as execute_retained;
    use crate::materials_media::{
        CancelObservation, Checkpoint, JobOutcome, JobReference, JobRegistry, MaterialsKey,
        NativeRequest,
    };
    use apparent_capture_engine::{Runtime, SecureStore};
    use serde::{Deserialize, Serialize};
    use sha2::{Digest, Sha256};
    use std::{
        fs::File,
        io::{Read, Write},
        os::fd::FromRawFd,
        os::unix::net::UnixStream,
        path::{Component, Path, PathBuf},
        process::Child,
        sync::Arc,
        time::Instant,
    };

    type Result<T> = std::result::Result<T, &'static str>;
    const REQUEST_LIMIT: usize = 256 * 1024;
    const CHILD_INPUT_LIMIT: usize = REQUEST_LIMIT + 16 * 1024;

    #[derive(Serialize, Deserialize)]
    #[serde(deny_unknown_fields, rename_all = "camelCase")]
    struct PrivateInput {
        runtime_path: String,
        capture_id: String,
        owner_token: String,
        request_json: String,
    }

    pub struct Active {
        child: Child,
        _lifetime: UnixStream,
        started: Instant,
        deadline_ms: u64,
        reference: JobReference,
        registry: JobRegistry,
        output: Arc<SecureStore>,
        terminal: bool,
    }

    impl Active {
        pub fn start(
            runtime_path: &Path,
            capture_id: &str,
            owner_token: &str,
            request_json: &str,
        ) -> std::result::Result<Self, &'static str> {
            let request = validate_initial_request(request_json, capture_id)?;
            if !runtime_path.is_absolute() {
                return Err("malformed_request");
            }
            let runtime = Runtime::open(runtime_path).map_err(|_| "source_mismatch")?;
            let capture_directory = runtime_path.join("capture-v2");
            // This is an auth-only journal read. It precedes registry and output
            // namespace access; the child repeats it before its own admission.
            let journal = crate::retained::read_authenticated_journal(
                &capture_directory,
                capture_id,
                owner_token,
            )?;
            let retained_root = journal.document["root"].as_str().ok_or("source_mismatch")?;
            if !normalized_absolute(retained_root)
                || journal.document["materials"]["key"]["captureId"] != request.materials.capture_id
                || journal.document["materials"]["key"]["materialsId"]
                    != request.materials.materials_id
                || journal.document["materials"]["key"]["revision"] != request.materials.revision
                || journal.document["materials"]["leaseId"] != request.lease_id
                || paths_overlap(
                    Path::new(&request.output_directory),
                    Path::new(retained_root),
                )
                || paths_overlap(Path::new(&request.output_directory), runtime_path)
            {
                return Err("invalid_output");
            }
            let output = Arc::new(
                SecureStore::create(Path::new(&request.output_directory))
                    .map_err(|_| "invalid_output")?,
            );
            let root = Arc::new(SecureStore::open(runtime_path).map_err(|_| "source_mismatch")?);
            let registry = JobRegistry::open(root).map_err(|_| "internal")?;
            let request_sha256 = digest(request_json.as_bytes());
            let reference = JobReference {
                capture_id: capture_id.to_owned(),
                materials: MaterialsKey {
                    capture_id: request.materials.capture_id,
                    materials_id: request.materials.materials_id,
                    revision: request.materials.revision,
                },
                lease_id: request.lease_id,
                job_id: request.job_id,
                request_sha256,
            };
            let internal = serde_json::to_vec(&PrivateInput {
                runtime_path: runtime_path.to_string_lossy().into_owned(),
                capture_id: capture_id.to_owned(),
                owner_token: owner_token.to_owned(),
                request_json: request_json.to_owned(),
            })
            .map_err(|_| "internal")?;
            let started = Instant::now();
            let (child, lifetime) =
                crate::job::spawn_reexec(&["--media2-native-job"], &internal, &[])?;
            let deadline_ms = request_deadline(request_json)?;
            drop(runtime);
            let mut active = Self {
                child,
                _lifetime: lifetime,
                started,
                deadline_ms,
                reference,
                registry,
                output,
                terminal: false,
            };
            // Admission is durable before the child begins full source hashing.
            // Do not report a running job until that original intent is visible.
            loop {
                match active.registry.lookup_authenticated(
                    capture_id,
                    &active.reference.job_id,
                    &active.reference.request_sha256,
                ) {
                    Ok(_) => return Ok(active),
                    Err(error) if job_not_yet_admitted(&*error) => {
                        if let Some(status) = active.child.try_wait().map_err(|_| "internal")? {
                            active.reconcile(if status.code().is_none() {
                                Checkpoint::OwnerLost
                            } else {
                                Checkpoint::Failed
                            })?;
                            active.terminal = true;
                            return Err("internal");
                        }
                        if active.started.elapsed().as_millis()
                            >= u128::from(active.deadline_ms.min(1_000))
                        {
                            active.stop(Checkpoint::Deadline)?;
                            return Err("deadline_exceeded");
                        }
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                    Err(_) => {
                        if active.child.try_wait().map_err(|_| "internal")?.is_none() {
                            let _ = active.child.kill();
                        }
                        let _ = active.child.wait();
                        active.terminal = true;
                        return Err("identity_conflict");
                    }
                }
            }
        }

        pub fn reference(&self) -> &JobReference {
            &self.reference
        }

        /// Returns true only after child reap and durable terminal reconciliation.
        pub fn poll(&mut self) -> std::result::Result<bool, &'static str> {
            if self.terminal {
                return Ok(true);
            }
            if let Some(status) = self.child.try_wait().map_err(|_| "internal")? {
                let reason = if status.code().is_none() {
                    Checkpoint::OwnerLost
                } else {
                    Checkpoint::Failed
                };
                self.reconcile(reason)?;
                self.terminal = true;
                return Ok(true);
            }
            if self.started.elapsed().as_millis() >= u128::from(self.deadline_ms) {
                self.stop(Checkpoint::Deadline)?;
                return Ok(true);
            }
            match self.registry.observe_cancel(&self.reference) {
                Ok(CancelObservation::Requested) => {
                    self.stop(Checkpoint::Cancel)?;
                    Ok(true)
                }
                Ok(_) => Ok(false),
                // The child may not have persisted admission yet. Other errors
                // are not treated as cancellation and are surfaced to caller.
                Err(_) => Err("internal"),
            }
        }

        /// Kill and reap first; only then can the durable terminal state change.
        pub fn stop(&mut self, reason: Checkpoint) -> std::result::Result<(), &'static str> {
            if self.terminal {
                return Ok(());
            }
            if self.child.try_wait().map_err(|_| "internal")?.is_none() {
                let _ = self.child.kill();
            }
            self.child.wait().map_err(|_| "internal")?;
            self.reconcile(reason)?;
            self.terminal = true;
            Ok(())
        }

        fn reconcile(&self, reason: Checkpoint) -> Result<()> {
            self.registry
                .finish_intent_reaped(&self.reference, self.output.clone(), reason)
                .map(|_| ())
                .map_err(|_| "internal")
        }

        pub fn child(&mut self) -> &mut Child {
            &mut self.child
        }
    }

    impl Drop for Active {
        fn drop(&mut self) {
            if !self.terminal {
                let _ = self.stop(Checkpoint::OwnerLost);
            }
        }
    }

    pub fn child() -> std::result::Result<(), &'static str> {
        // Reuse the Mac worker's existing physical-footprint enforcement through
        // pinning, PNG/sheet assembly and durable receipt publication as well as
        // AVFoundation decoding. This guard exits only the owned re-exec child.
        #[cfg(target_os = "macos")]
        let _footprint = crate::macos::FootprintGuard::worker(1536 * 1024 * 1024);
        let mut lifetime = unsafe { File::from_raw_fd(5) };
        std::thread::spawn(move || {
            let mut byte = [0u8; 1];
            let _ = lifetime.read(&mut byte);
            std::process::exit(1);
        });
        let mut bytes = Vec::new();
        std::io::stdin()
            .take(CHILD_INPUT_LIMIT as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "malformed_request")?;
        if bytes.len() > CHILD_INPUT_LIMIT {
            return Err("malformed_request");
        }
        let input: PrivateInput =
            serde_json::from_slice(&bytes).map_err(|_| "malformed_request")?;
        let request = validate_initial_request(&input.request_json, &input.capture_id)?;
        let runtime_path = Path::new(&input.runtime_path);
        let runtime = Runtime::open(runtime_path).map_err(|_| "source_mismatch")?;
        let capture_directory = runtime_path.join("capture-v2");
        crate::retained::read_authenticated_journal(
            &capture_directory,
            &input.capture_id,
            &input.owner_token,
        )?;
        let root = Arc::new(SecureStore::open(runtime_path).map_err(|_| "source_mismatch")?);
        let registry = JobRegistry::open(root).map_err(|_| "internal")?;
        let reference = JobReference {
            capture_id: input.capture_id.clone(),
            materials: request.materials.clone(),
            lease_id: request.lease_id.clone(),
            job_id: request.job_id.clone(),
            request_sha256: digest(input.request_json.as_bytes()),
        };
        let started = Instant::now();
        let mut sink = StoreSink::new(
            Arc::new(
                SecureStore::open(Path::new(&request.output_directory))
                    .map_err(|_| "invalid_output")?,
            ),
            registry.clone(),
            reference.clone(),
        );
        let result = execute_retained(
            &registry,
            &runtime,
            &capture_directory,
            &input.capture_id,
            &input.owner_token,
            &crate::retained::MaterialKey {
                capture_id: request.materials.capture_id.clone(),
                materials_id: request.materials.materials_id.clone(),
                revision: request.materials.revision.clone(),
            },
            &request.lease_id,
            input.request_json.as_bytes(),
            sink.root.clone(),
            &mut sink,
            || {
                if started.elapsed().as_millis() >= u128::from(request.limits.deadline_ms) {
                    Checkpoint::Deadline
                } else {
                    match registry.observe_cancel(&reference) {
                        Ok(CancelObservation::Requested) => Checkpoint::Cancel,
                        Ok(CancelObservation::DeadlineExceeded) => Checkpoint::Deadline,
                        Ok(CancelObservation::OwnerLost) => Checkpoint::OwnerLost,
                        Ok(_) => Checkpoint::Continue,
                        Err(_) => Checkpoint::Continue,
                    }
                }
            },
        );
        match result {
            Ok(JobOutcome::Complete(_) | JobOutcome::ExistingComplete(_, _)) => Ok(()),
            Ok(JobOutcome::ExistingFailed(code)) => Err(code),
            Ok(
                JobOutcome::ExistingInProgress
                | JobOutcome::ExistingPreparing
                | JobOutcome::ExistingRunning,
            ) => Err("busy"),
            Ok(JobOutcome::Owned(_)) => Err("internal"),
            Err(error) => Err(map_error(&*error)),
        }
    }

    struct StoreSink {
        root: Arc<SecureStore>,
        frames: Option<Arc<SecureStore>>,
        registry: JobRegistry,
        reference: JobReference,
    }

    impl StoreSink {
        fn new(root: Arc<SecureStore>, registry: JobRegistry, reference: JobReference) -> Self {
            Self {
                root,
                frames: None,
                registry,
                reference,
            }
        }
    }

    impl apparent_capture_engine::materials_media::DerivativeOutputSink for StoreSink {
        fn write_new_synced(
            &mut self,
            relative_path: &str,
            bytes: &[u8],
        ) -> anyhow::Result<apparent_capture_engine::materials_media::SealedFile> {
            use std::os::unix::fs::MetadataExt;
            let (store, leaf) = if let Some(leaf) = relative_path.strip_prefix("frames/") {
                if leaf.is_empty() || leaf.contains('/') {
                    anyhow::bail!("invalid output path");
                }
                let frames = match &self.frames {
                    Some(frames) => frames.clone(),
                    None => {
                        self.registry
                            .reserve_output_directory(&self.reference, &self.root)
                            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                        let frames = self.root.create_child("frames")?;
                        self.registry
                            .record_output_directory(&self.reference, &self.root, &frames)
                            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                        self.frames = Some(frames.clone());
                        frames
                    }
                };
                (frames, leaf.to_owned())
            } else if matches!(relative_path, "receipt.json" | "contact-sheet.png") {
                (self.root.clone(), relative_path.to_owned())
            } else {
                anyhow::bail!("invalid output path");
            };
            self.registry
                .reserve_output_file(&self.reference, &self.root, relative_path)
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            let file = store.file(&leaf, true)?;
            self.registry
                .record_output_file(&self.reference, &self.root, relative_path, &file)
                .map_err(|error| anyhow::anyhow!(error.to_string()))?;
            let mut file = file;
            file.write_all(bytes)?;
            file.sync_all()?;
            store.directory_file()?.sync_all()?;
            let metadata = file.metadata()?;
            let identity = apparent_capture_engine::materials_media::FileIdentity {
                device: metadata.dev().to_string(),
                inode: metadata.ino().to_string(),
                size: metadata.len(),
                modified_ns: (i128::from(metadata.mtime()) * 1_000_000_000
                    + i128::from(metadata.mtime_nsec()))
                .to_string(),
            };
            Ok(apparent_capture_engine::materials_media::SealedFile {
                relative_path: relative_path.to_owned(),
                identity,
                sha256: digest(bytes),
            })
        }
    }

    fn request_deadline(request_json: &str) -> Result<u64> {
        let request: NativeRequest =
            serde_json::from_str(request_json).map_err(|_| "malformed_request")?;
        Ok(request.limits.deadline_ms)
    }

    fn normalized_absolute(value: &str) -> bool {
        if value.len() > 4096 || !Path::new(value).is_absolute() {
            return false;
        }
        let mut normalized = PathBuf::new();
        for component in Path::new(value).components() {
            match component {
                Component::RootDir => normalized.push("/"),
                Component::Normal(part) => normalized.push(part),
                _ => return false,
            }
        }
        normalized.to_str() == Some(value)
    }

    fn validate_initial_request(request_json: &str, capture_id: &str) -> Result<NativeRequest> {
        if request_json.len() > REQUEST_LIMIT || !valid_id(capture_id) {
            return Err("malformed_request");
        }
        let request: NativeRequest =
            serde_json::from_str(request_json).map_err(|_| "malformed_request")?;
        if serde_json::to_string(&request).map_err(|_| "internal")? != request_json
            || request.schema != "apparent.retained-frame-request/1"
            || request.materials.capture_id != capture_id
            || !valid_id(&request.materials.materials_id)
            || !valid_hash(&request.materials.revision)
            || !valid_id(&request.lease_id)
            || !valid_id(&request.job_id)
            || !normalized_absolute(&request.output_directory)
            || request.limits.max_frames == 0
            || request.limits.max_frames > 48
            || request.limits.max_edge == 0
            || request.limits.max_edge > 4096
            || request.limits.max_total_pixels == 0
            || request.limits.max_total_pixels > 64_000_000
            || request.limits.deadline_ms == 0
            || request.limits.deadline_ms > 60_000
            || request.requested_bundle_ms.is_empty()
            || request.requested_bundle_ms.len() > request.limits.max_frames as usize
            || request.requested_bundle_ms.len() > 48
            || request
                .requested_bundle_ms
                .iter()
                .any(|time| *time > 9_007_199_254_740_991)
        {
            return Err("malformed_request");
        }
        if let Some(layout) = &request.contact_sheet {
            layout
                .validate(request.requested_bundle_ms.len())
                .map_err(|_| "malformed_request")?;
            let sheet_pixels = u64::from(layout.width)
                .checked_mul(u64::from(layout.height))
                .and_then(|pixels| pixels.checked_add(request.requested_bundle_ms.len() as u64))
                .ok_or("malformed_request")?;
            if sheet_pixels > request.limits.max_total_pixels {
                return Err("malformed_request");
            }
        }
        Ok(request)
    }

    fn valid_id(value: &str) -> bool {
        value.len() == 32
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }

    fn valid_hash(value: &str) -> bool {
        value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    }

    fn paths_overlap(left: &Path, right: &Path) -> bool {
        left.starts_with(right) || right.starts_with(left)
    }

    fn job_not_yet_admitted(error: &(dyn std::error::Error + 'static)) -> bool {
        error
            .downcast_ref::<std::io::Error>()
            .is_some_and(|e| e.kind() == std::io::ErrorKind::NotFound)
            || error.to_string() == "job reference not found"
    }

    fn digest(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn map_error(error: &dyn std::error::Error) -> &'static str {
        let text = error.to_string();
        [
            "cancelled",
            "deadline_exceeded",
            "source_mismatch",
            "invalid_materials",
            "lease_busy",
            "limits_exceeded",
            "decode_failed",
            "invalid_output",
            "busy",
        ]
        .into_iter()
        .find(|code| text.contains(code))
        .unwrap_or("internal")
    }

    pub use Active as PublicActive;

    #[cfg(test)]
    mod tests {
        use super::*;
        use crate::materials_media::Binding;
        use apparent_capture_engine::materials_media::DerivativeOutputSink;
        use std::os::unix::fs::PermissionsExt;

        #[test]
        fn output_paths_must_be_normalized_and_disjoint_from_private_roots() {
            assert!(normalized_absolute("/tmp/apparent-output"));
            assert!(!normalized_absolute("/tmp/apparent/../output"));
            assert!(!normalized_absolute("/tmp//apparent-output"));
            assert!(!normalized_absolute("/tmp/apparent-output/"));

            let materials = Path::new("/runtime/capture-v2/materials/capture");
            assert!(paths_overlap(
                Path::new("/runtime/capture-v2/materials/capture/child"),
                materials
            ));
            assert!(paths_overlap(Path::new("/runtime"), Path::new("/runtime")));
            assert!(paths_overlap(
                Path::new("/runtime"),
                Path::new("/runtime/capture-v2")
            ));
            assert!(!paths_overlap(
                Path::new("/runtime-other/output"),
                Path::new("/runtime")
            ));
        }

        #[test]
        fn start_rejects_materials_and_runtime_overlap_before_output_creation() {
            let base = std::env::temp_dir().canonicalize().unwrap().join(format!(
                "media2-worker-overlap-{}",
                apparent_capture_engine::new_id().unwrap()
            ));
            std::fs::create_dir(&base).unwrap();
            std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700)).unwrap();
            let runtime_path = base.join("runtime");
            std::fs::create_dir(&runtime_path).unwrap();
            std::fs::set_permissions(&runtime_path, std::fs::Permissions::from_mode(0o700))
                .unwrap();
            let runtime = Runtime::open(&runtime_path).unwrap();
            drop(runtime);
            let capture_directory = runtime_path.join("capture-v2");
            let captures = SecureStore::create(&capture_directory).unwrap();
            let capture_id = apparent_capture_engine::new_id().unwrap();
            let owner_token = apparent_capture_engine::new_owner_token().unwrap();
            let materials_id = apparent_capture_engine::new_id().unwrap();
            let lease_id = apparent_capture_engine::new_id().unwrap();
            let revision = "a".repeat(64);
            let materials_root = capture_directory.join("materials").join(&capture_id);
            std::fs::create_dir_all(&materials_root).unwrap();
            std::fs::set_permissions(
                materials_root.parent().unwrap(),
                std::fs::Permissions::from_mode(0o700),
            )
            .unwrap();
            std::fs::set_permissions(&materials_root, std::fs::Permissions::from_mode(0o700))
                .unwrap();
            captures
                .write_new(
                    &format!("{capture_id}.json"),
                    &serde_json::to_vec(&serde_json::json!({
                        "capture_id":capture_id,
                        "owner_token":owner_token,
                        "root":materials_root,
                        "materials":{
                            "key":{"captureId":capture_id,"materialsId":materials_id,"revision":revision},
                            "leaseId":lease_id
                        }
                    }))
                    .unwrap(),
                )
                .unwrap();
            let request = NativeRequest {
                schema: "apparent.retained-frame-request/1".into(),
                materials: MaterialsKey {
                    capture_id: capture_id.clone(),
                    materials_id,
                    revision,
                },
                lease_id,
                job_id: apparent_capture_engine::new_id().unwrap(),
                output_directory: String::new(),
                requested_bundle_ms: vec![0],
                limits: crate::materials_media::Limits {
                    max_frames: 1,
                    max_edge: 64,
                    max_total_pixels: 4096,
                    deadline_ms: 1000,
                },
                contact_sheet: None,
            };
            let materials_overlap = materials_root.join("unsafe-output");
            let mut request = request;
            request.output_directory = materials_overlap.to_string_lossy().into_owned();
            let bytes = serde_json::to_string(&request).unwrap();
            assert!(Active::start(&runtime_path, &capture_id, &owner_token, &bytes).is_err());
            assert!(!materials_overlap.exists());

            let runtime_overlap = runtime_path.join("unsafe-output");
            request.output_directory = runtime_overlap.to_string_lossy().into_owned();
            let bytes = serde_json::to_string(&request).unwrap();
            assert!(Active::start(&runtime_path, &capture_id, &owner_token, &bytes).is_err());
            assert!(!runtime_overlap.exists());
            std::fs::remove_dir_all(base).unwrap();
        }

        #[test]
        fn store_sink_tracks_native_files_for_post_reap_cleanup() {
            use std::os::unix::fs::MetadataExt;

            let base = std::env::temp_dir().canonicalize().unwrap().join(format!(
                "media2-worker-output-cleanup-{}",
                apparent_capture_engine::new_id().unwrap()
            ));
            std::fs::create_dir(&base).unwrap();
            std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700)).unwrap();
            let runtime = Arc::new(SecureStore::create(&base.join("runtime")).unwrap());
            let output = Arc::new(SecureStore::create(&base.join("output")).unwrap());
            let metadata = output.directory_metadata().unwrap();
            let registry = JobRegistry::open(runtime).unwrap();
            let binding: Binding = serde_json::from_value(serde_json::json!({
                "schema":"apparent.media2-job/1",
                "captureId":"a".repeat(32),
                "materials":{"captureId":"a".repeat(32),"materialsId":"b".repeat(32),"revision":"c".repeat(64)},
                "materialsRoot":base.join("retained").display().to_string(),
                "materialsRootIdentity":{"device":"1","inode":"2"},
                "leaseId":"d".repeat(32),
                "sourceVideo":{"relativePath":"recording.mov","identity":{"device":"1","inode":"2","size":1,"modifiedNs":"3"},"sha256":"e".repeat(64)},
                "videoOffsetMs":0,
                "jobId":apparent_capture_engine::new_id().unwrap(),
                "receiptId":apparent_capture_engine::new_id().unwrap(),
                "requestSha256":"f".repeat(64),
                "requestedBundleMs":[0],
                "limits":{"maxFrames":1,"maxEdge":64,"maxTotalPixels":4096,"deadlineMs":1000},
                "outputRoot":base.join("output").display().to_string(),
                "outputIdentity":{"device":metadata.dev().to_string(),"inode":metadata.ino().to_string()},
                "layout":null
            })).unwrap();
            let JobOutcome::Owned(claim) = registry.begin(binding.clone(), output.clone()).unwrap()
            else {
                panic!()
            };
            let reference = JobReference {
                capture_id: binding.capture_id.clone(),
                materials: binding.materials.clone(),
                lease_id: binding.lease_id.clone(),
                job_id: binding.job_id.clone(),
                request_sha256: binding.request_sha256.clone(),
            };
            let mut sink = StoreSink::new(output.clone(), registry.clone(), reference);
            sink.write_new_synced("frames/frame-001.png", b"native partial frame")
                .unwrap();
            std::fs::write(base.join("output/caller-owned.txt"), b"preserve").unwrap();
            drop(sink);
            drop(claim);
            assert!(matches!(
                registry
                    .finish_reaped(&binding, output, Checkpoint::Failed)
                    .unwrap(),
                JobOutcome::ExistingFailed("failed")
            ));
            assert!(!base.join("output/frames").exists());
            assert_eq!(
                std::fs::read(base.join("output/caller-owned.txt")).unwrap(),
                b"preserve"
            );
            std::fs::remove_dir_all(base).unwrap();
        }
    }
}

#[cfg(all(any(target_os = "linux", target_os = "macos"), feature = "media"))]
pub use native::PublicActive as Active;

#[cfg(all(any(target_os = "linux", target_os = "macos"), feature = "media"))]
pub fn child() -> std::result::Result<(), &'static str> {
    native::child()
}

#[cfg(not(all(any(target_os = "linux", target_os = "macos"), feature = "media")))]
pub struct Active;

#[cfg(not(all(any(target_os = "linux", target_os = "macos"), feature = "media")))]
impl Active {
    pub fn start(
        _runtime_path: &std::path::Path,
        _capture_id: &str,
        _owner_token: &str,
        _request_json: &str,
    ) -> std::result::Result<Self, &'static str> {
        Err("native_unavailable")
    }

    pub fn reference(&self) -> &crate::materials_media::JobReference {
        unreachable!()
    }

    pub fn poll(&mut self) -> std::result::Result<bool, &'static str> {
        Err("native_unavailable")
    }

    pub fn stop(
        &mut self,
        _reason: crate::materials_media::Checkpoint,
    ) -> std::result::Result<(), &'static str> {
        Err("native_unavailable")
    }
}

#[cfg(not(all(any(target_os = "linux", target_os = "macos"), feature = "media")))]
pub fn child() -> std::result::Result<(), &'static str> {
    Err("native_unavailable")
}
