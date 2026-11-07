//! Durable owner-local registry for native Media/2 jobs.
//!
//! The registry is not an authentication mechanism. The caller must authenticate
//! capture ownership before `admit`/inspection/cancellation, hold the exact Core
//! materials read lease, and keep both lease and job owner through supervision.

use apparent_capture_engine::SecureStore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::Read,
    os::{fd::AsRawFd, unix::fs::MetadataExt},
    sync::Arc,
};

type Error = Box<dyn std::error::Error + Send + Sync>;
type Result<T> = std::result::Result<T, Error>;

macro_rules! ensure {
    ($condition:expr, $message:literal) => {
        if !$condition {
            return Err(std::io::Error::other($message).into());
        }
    };
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Checkpoint {
    Continue,
    Cancel,
    Deadline,
    Failed,
    OwnerLost,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OutputIdentity {
    pub device: String,
    pub inode: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MaterialsKey {
    pub capture_id: String,
    pub materials_id: String,
    pub revision: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct FileIdentity {
    pub device: String,
    pub inode: String,
    pub size: u64,
    pub modified_ns: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SealedFile {
    pub relative_path: String,
    pub identity: FileIdentity,
    pub sha256: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Limits {
    pub max_frames: u64,
    pub max_edge: u32,
    pub max_total_pixels: u64,
    pub deadline_ms: u64,
}

/// Immutable job binding. Every field is compared byte-for-byte on replay.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Binding {
    pub schema: String,
    pub capture_id: String,
    pub materials: MaterialsKey,
    pub materials_root: String,
    pub materials_root_identity: OutputIdentity,
    pub lease_id: String,
    pub source_video: SealedFile,
    pub video_offset_ms: u64,
    pub job_id: String,
    pub receipt_id: String,
    pub request_sha256: String,
    pub requested_bundle_ms: Vec<u64>,
    pub limits: Limits,
    pub output_root: String,
    pub output_identity: OutputIdentity,
    pub layout: Option<super::contact_sheet::Layout>,
}

/// Original immutable request authority persisted before retained-source hashing.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct JobIntent {
    pub schema: String,
    pub capture_id: String,
    pub materials: MaterialsKey,
    pub lease_id: String,
    pub job_id: String,
    pub request_sha256: String,
    pub requested_bundle_ms: Vec<u64>,
    pub limits: Limits,
    pub output_root: String,
    pub output_identity: OutputIdentity,
    pub layout: Option<super::contact_sheet::Layout>,
    /// Wall-clock admission timestamp. Replays never create a fresh deadline.
    pub admitted_at_ms: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct OwnedOutputInventory {
    schema: String,
    files: Vec<OwnedOutputFile>,
    frames_directory: Option<OwnedOutputDirectory>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct OwnedOutputDirectory {
    identity: Option<OutputIdentity>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct OwnedOutputFile {
    relative_path: String,
    identity: Option<OutputIdentity>,
}

impl JobIntent {
    fn reference(&self) -> JobReference {
        JobReference {
            capture_id: self.capture_id.clone(),
            materials: self.materials.clone(),
            lease_id: self.lease_id.clone(),
            job_id: self.job_id.clone(),
            request_sha256: self.request_sha256.clone(),
        }
    }

    fn matches_reference(&self, reference: &JobReference) -> bool {
        self.capture_id == reference.capture_id
            && self.materials == reference.materials
            && self.lease_id == reference.lease_id
            && self.job_id == reference.job_id
            && self.request_sha256 == reference.request_sha256
    }

    fn same_request(&self, other: &Self) -> bool {
        self.schema == other.schema
            && self.capture_id == other.capture_id
            && self.materials == other.materials
            && self.lease_id == other.lease_id
            && self.job_id == other.job_id
            && self.request_sha256 == other.request_sha256
            && self.requested_bundle_ms == other.requested_bundle_ms
            && self.limits == other.limits
            && self.output_root == other.output_root
            && self.output_identity == other.output_identity
            && self.layout == other.layout
    }

    pub fn remaining_deadline_ms(&self) -> u64 {
        let now = unix_time_ms();
        let elapsed = now.checked_sub(self.admitted_at_ms).unwrap_or(u64::MAX);
        self.limits.deadline_ms.saturating_sub(elapsed)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct JobReference {
    pub capture_id: String,
    pub materials: MaterialsKey,
    pub lease_id: String,
    pub job_id: String,
    pub request_sha256: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoredJobState {
    Preparing,
    Running,
    Complete,
    Cancelled,
    DeadlineExceeded,
    Failed,
    OwnerLost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelRequestOutcome {
    Requested,
    AlreadyRequested,
    Complete,
    Cancelled,
    DeadlineExceeded,
    Failed,
    OwnerLost,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CancelObservation {
    NotRequested,
    Requested,
    Complete,
    Cancelled,
    DeadlineExceeded,
    Failed,
    OwnerLost,
}

pub struct StoredJob {
    pub intent: JobIntent,
    pub binding: Option<Binding>,
    pub state: StoredJobState,
    pub receipt: Option<NativeReceipt>,
    pub envelope: Option<Vec<u8>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SheetSeal {
    pub layout_hash: String,
    pub file: SealedFile,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct NativeReceipt {
    pub schema: String,
    pub capture_id: String,
    pub materials: MaterialsKey,
    pub lease_id: String,
    pub job_id: String,
    pub receipt_id: String,
    pub request_sha256: String,
    pub binding_sha256: String,
    pub payload_file: SealedFile,
    pub envelope_sha256: String,
    pub frames: Vec<SealedFile>,
    /// The sheet has its own output seal and layout hash; it is never a frame slot.
    pub sheet: Option<SheetSeal>,
}

impl NativeReceipt {
    pub fn new(
        binding: &Binding,
        frames: &[SealedFile],
        payload_file: SealedFile,
        envelope: &[u8],
        sheet: Option<SheetSeal>,
    ) -> Result<Self> {
        ensure!(
            frames.len() == binding.requested_bundle_ms.len(),
            "receipt frame inventory length mismatch"
        );
        ensure!(
            payload_file.relative_path == "receipt.json",
            "unexpected detached receipt path"
        );
        ensure!(
            payload_file.identity.size <= 8 * 1024 * 1024,
            "receipt exceeds SDK document bound"
        );
        ensure!(
            valid_hash(&payload_file.sha256) && valid_hash(&hex(&Sha256::digest(envelope))),
            "invalid receipt digest"
        );
        ensure!(
            envelope.len() <= 8 * 1024 * 1024,
            "derivative envelope exceeds SDK document bound"
        );
        if let Some(sheet) = &sheet {
            ensure!(
                binding
                    .layout
                    .as_ref()
                    .and_then(|layout| layout.sha256().ok())
                    .as_deref()
                    == Some(&sheet.layout_hash),
                "sheet layout hash mismatch"
            );
            ensure!(
                sheet.file.relative_path == "contact-sheet.png",
                "unexpected sheet path"
            );
        } else {
            ensure!(binding.layout.is_none(), "missing bound sheet output");
        }
        Ok(Self {
            schema: "apparent.media2-native-job-receipt/1".into(),
            capture_id: binding.capture_id.clone(),
            materials: binding.materials.clone(),
            lease_id: binding.lease_id.clone(),
            job_id: binding.job_id.clone(),
            receipt_id: binding.receipt_id.clone(),
            request_sha256: binding.request_sha256.clone(),
            binding_sha256: hex(&Sha256::digest(serde_json::to_vec(binding)?)),
            payload_file,
            envelope_sha256: hex(&Sha256::digest(envelope)),
            frames: frames.to_vec(),
            sheet,
        })
    }
}

#[derive(Clone)]
pub struct JobRegistry {
    namespace: Arc<SecureStore>,
}

pub enum JobOutcome {
    Owned(JobClaim),
    ExistingInProgress,
    ExistingPreparing,
    ExistingRunning,
    Complete(NativeReceipt),
    ExistingComplete(NativeReceipt, Vec<u8>),
    ExistingFailed(&'static str),
}

impl JobRegistry {
    pub fn open(root: Arc<SecureStore>) -> Result<Self> {
        Ok(Self {
            namespace: root.child("materials-media-2")?,
        })
    }

    /// Durably record the private `frames` directory created exclusively by
    /// the supervised output sink. This is cleanup ownership, not receipt
    /// authority; caller must be the admitted worker holding its owner lock.
    pub(crate) fn reserve_output_directory(
        &self,
        reference: &JobReference,
        output: &Arc<SecureStore>,
    ) -> Result<()> {
        let job = self.namespace.existing_child(&reference.job_id)?;
        let _control = control_lock(&job)?;
        let intent = read_job_intent(&job)?;
        ensure!(
            intent.matches_reference(reference),
            "job reference mismatch"
        );
        ensure!(
            read_job_state(&job)? == StoredJobState::Running,
            "job is not running"
        );
        check_output_identity_values(&intent.output_identity, output)?;
        ensure!(
            !output.entry_exists("frames")?,
            "unowned frames directory exists"
        );
        let mut inventory = read_owned_inventory(&job)?;
        if inventory.frames_directory.is_some() {
            return Ok(());
        }
        inventory.frames_directory = Some(OwnedOutputDirectory { identity: None });
        job.write("owned-output.json", &serde_json::to_vec(&inventory)?)?;
        Ok(())
    }

    pub(crate) fn record_output_directory(
        &self,
        reference: &JobReference,
        output: &Arc<SecureStore>,
        directory: &SecureStore,
    ) -> Result<()> {
        let job = self.namespace.existing_child(&reference.job_id)?;
        let _control = control_lock(&job)?;
        let intent = read_job_intent(&job)?;
        ensure!(
            intent.matches_reference(reference),
            "job reference mismatch"
        );
        ensure!(
            read_job_state(&job)? == StoredJobState::Running,
            "job is not running"
        );
        check_output_identity_values(&intent.output_identity, output)?;
        let expected = output.existing_child("frames")?;
        let actual_metadata = directory.directory_metadata()?;
        let expected_metadata = expected.directory_metadata()?;
        ensure!(
            actual_metadata.dev() == expected_metadata.dev()
                && actual_metadata.ino() == expected_metadata.ino(),
            "frames directory identity changed"
        );
        let identity = OutputIdentity {
            device: actual_metadata.dev().to_string(),
            inode: actual_metadata.ino().to_string(),
        };
        let mut inventory = read_owned_inventory(&job)?;
        let directory = inventory
            .frames_directory
            .as_mut()
            .ok_or_else(|| std::io::Error::other("frames directory reservation missing"))?;
        if let Some(previous) = &directory.identity {
            ensure!(previous == &identity, "frames directory identity changed");
            return Ok(());
        }
        directory.identity = Some(identity);
        job.write("owned-output.json", &serde_json::to_vec(&inventory)?)?;
        Ok(())
    }

    /// Persist an allowlisted output path before its exclusive file creation.
    /// If the worker dies before inode binding, cleanup will preserve any
    /// resulting file and report the unresolved reservation.
    pub(crate) fn reserve_output_file(
        &self,
        reference: &JobReference,
        output: &Arc<SecureStore>,
        relative_path: &str,
    ) -> Result<()> {
        let job = self.namespace.existing_child(&reference.job_id)?;
        let _control = control_lock(&job)?;
        let intent = read_job_intent(&job)?;
        ensure!(
            intent.matches_reference(reference),
            "job reference mismatch"
        );
        ensure!(
            read_job_state(&job)? == StoredJobState::Running,
            "job is not running"
        );
        check_output_identity_values(&intent.output_identity, output)?;
        let (store, leaf) = if let Some(leaf) = relative_path.strip_prefix("frames/") {
            let index = leaf
                .strip_prefix("frame-")
                .and_then(|name| name.strip_suffix(".png"))
                .and_then(|index| index.parse::<usize>().ok())
                .ok_or_else(|| std::io::Error::other("invalid output frame path"))?;
            ensure!(
                index > 0
                    && index <= intent.requested_bundle_ms.len()
                    && leaf == format!("frame-{index:03}.png"),
                "invalid output frame path"
            );
            let frames = output.existing_child("frames")?;
            let inventory = read_owned_inventory(&job)?;
            ensure!(
                inventory
                    .frames_directory
                    .as_ref()
                    .and_then(|dir| dir.identity.as_ref())
                    == Some(&OutputIdentity {
                        device: frames.directory_metadata()?.dev().to_string(),
                        inode: frames.directory_metadata()?.ino().to_string(),
                    }),
                "frames directory is not registered"
            );
            (frames, leaf.to_owned())
        } else {
            ensure!(
                relative_path == "receipt.json"
                    || (relative_path == "contact-sheet.png" && intent.layout.is_some()),
                "invalid output file path"
            );
            (output.clone(), relative_path.to_owned())
        };
        ensure!(!store.entry_exists(&leaf)?, "output path already exists");
        let mut inventory = read_owned_inventory(&job)?;
        if let Some(previous) = inventory
            .files
            .iter()
            .find(|owned| owned.relative_path == relative_path)
        {
            ensure!(previous.identity.is_none(), "output path already tracked");
            return Ok(());
        }
        ensure!(inventory.files.len() < 50, "owned output inventory full");
        inventory.files.push(OwnedOutputFile {
            relative_path: relative_path.to_owned(),
            identity: None,
        });
        job.write("owned-output.json", &serde_json::to_vec(&inventory)?)?;
        Ok(())
    }

    /// Record a newly created file inode before the worker writes bytes to it,
    /// so failures and cancellation during a write remain safely cleanable.
    pub(crate) fn record_output_file(
        &self,
        reference: &JobReference,
        output: &Arc<SecureStore>,
        relative_path: &str,
        file: &File,
    ) -> Result<()> {
        let job = self.namespace.existing_child(&reference.job_id)?;
        let _control = control_lock(&job)?;
        let intent = read_job_intent(&job)?;
        ensure!(
            intent.matches_reference(reference),
            "job reference mismatch"
        );
        ensure!(
            read_job_state(&job)? == StoredJobState::Running,
            "job is not running"
        );
        check_output_identity_values(&intent.output_identity, output)?;
        let (store, leaf) = if let Some(leaf) = relative_path.strip_prefix("frames/") {
            let index = leaf
                .strip_prefix("frame-")
                .and_then(|name| name.strip_suffix(".png"))
                .and_then(|index| index.parse::<usize>().ok())
                .ok_or_else(|| std::io::Error::other("invalid output frame path"))?;
            ensure!(
                index > 0
                    && index <= intent.requested_bundle_ms.len()
                    && leaf == format!("frame-{index:03}.png"),
                "invalid output frame path"
            );
            let frames = output.existing_child("frames")?;
            let inventory = read_owned_inventory(&job)?;
            let metadata = frames.directory_metadata()?;
            let identity = OutputIdentity {
                device: metadata.dev().to_string(),
                inode: metadata.ino().to_string(),
            };
            ensure!(
                inventory
                    .frames_directory
                    .as_ref()
                    .and_then(|dir| dir.identity.as_ref())
                    == Some(&identity),
                "frames directory ownership missing"
            );
            (frames, leaf.to_owned())
        } else {
            ensure!(
                relative_path == "receipt.json"
                    || (relative_path == "contact-sheet.png" && intent.layout.is_some()),
                "invalid output file path"
            );
            (output.clone(), relative_path.to_owned())
        };
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o777 == 0o600
                && metadata.nlink() == 1,
            "unsafe output file"
        );
        let named = store.file(&leaf, false)?;
        let named_metadata = named.metadata()?;
        ensure!(
            metadata.dev() == named_metadata.dev() && metadata.ino() == named_metadata.ino(),
            "output file identity changed"
        );
        let mut inventory = read_owned_inventory(&job)?;
        if let Some(previous) = inventory
            .files
            .iter_mut()
            .find(|owned| owned.relative_path == relative_path)
        {
            let identity = OutputIdentity {
                device: metadata.dev().to_string(),
                inode: metadata.ino().to_string(),
            };
            if let Some(existing) = &previous.identity {
                ensure!(existing == &identity, "output path already tracked");
                return Ok(());
            }
            previous.identity = Some(identity);
        } else {
            return Err(std::io::Error::other("output path reservation missing").into());
        }
        job.write("owned-output.json", &serde_json::to_vec(&inventory)?)?;
        Ok(())
    }

    /// Persist and exclusively own the exact original request before any source
    /// pin/hash work. The request intent is immutable; a replay cannot refresh
    /// its original deadline. Caller authentication must precede this method.
    pub fn admit(&self, intent: JobIntent, output: Arc<SecureStore>) -> Result<JobOutcome> {
        validate_intent(&intent)?;
        check_output_identity_values(&intent.output_identity, &output)?;
        let job = self.namespace.child(&intent.job_id)?;
        let control = control_lock(&job)?;
        if let Some(previous) = job.read("intent.json")? {
            let previous: JobIntent = serde_json::from_slice(&previous)?;
            validate_intent(&previous)?;
            ensure!(
                previous.same_request(&intent),
                "job ID is bound to a different request intent"
            );
        }
        let Some(owner) = try_owner_lock(&job)? else {
            return Ok(JobOutcome::ExistingInProgress);
        };
        let existing = job.read("intent.json")?;
        let mut intent = intent;
        let intent_bytes = if let Some(previous) = existing.as_ref() {
            let previous: JobIntent = serde_json::from_slice(previous)?;
            ensure!(
                previous.same_request(&intent),
                "job ID is bound to a different request intent"
            );
            intent.admitted_at_ms = previous.admitted_at_ms;
            serde_json::to_vec(&previous)?
        } else {
            intent.admitted_at_ms = unix_time_ms();
            serde_json::to_vec(&intent)?
        };
        ensure!(intent_bytes.len() <= 64 * 1024, "job intent too large");
        match existing.is_some() {
            true => match job.read("state.json")? {
                None => Ok(JobOutcome::ExistingPreparing),
                Some(state) => match parse_state(&state)?.as_str() {
                    "preparing" => Ok(JobOutcome::ExistingPreparing),
                    "running" => Ok(JobOutcome::ExistingRunning),
                    "complete" => {
                        drop(control);
                        let stored = self.inspect_reference(&intent.reference())?;
                        Ok(JobOutcome::ExistingComplete(
                            stored.receipt.ok_or_else(|| {
                                std::io::Error::other("completed job receipt missing")
                            })?,
                            stored.envelope.ok_or_else(|| {
                                std::io::Error::other("completed job envelope missing")
                            })?,
                        ))
                    }
                    "cancelled" => Ok(JobOutcome::ExistingFailed("cancelled")),
                    "deadline_exceeded" => Ok(JobOutcome::ExistingFailed("deadline_exceeded")),
                    "failed" => Ok(JobOutcome::ExistingFailed("failed")),
                    "owner_lost" => Ok(JobOutcome::ExistingFailed("owner_lost")),
                    _ => Err(std::io::Error::other("unknown terminal job state").into()),
                },
            },
            false => {
                // A binding without its intent is an older or interrupted job.
                // Never adopt it as fresh work or manufacture a new deadline.
                if let Some(bytes) = job.read("binding.json")? {
                    let binding: Binding = serde_json::from_slice(&bytes)?;
                    ensure!(
                        intent_from_binding(&binding)?.same_request(&intent),
                        "job ID is bound to a different request intent"
                    );
                    return Ok(JobOutcome::ExistingInProgress);
                }
                job.write_new("intent.json", &intent_bytes)?;
                job.write_new("state.json", br#"{"state":"preparing"}"#)?;
                Ok(JobOutcome::Owned(JobClaim {
                    job,
                    owner,
                    binding: None,
                    intent,
                    output,
                }))
            }
        }
    }

    /// Resolve the original durable reference after capture-owner authentication.
    /// No paths or material identity are returned until capture/job/request match.
    /// The caller MUST authenticate capture ownership before this lookup.
    pub fn lookup_authenticated(
        &self,
        capture_id: &str,
        job_id: &str,
        request_sha256: &str,
    ) -> Result<StoredJob> {
        ensure!(
            valid_id(capture_id) && valid_id(job_id) && valid_hash(request_sha256),
            "invalid job lookup"
        );
        let job = self.namespace.existing_child(job_id)?;
        let control = control_lock(&job)?;
        let bytes = job
            .read("intent.json")?
            .ok_or_else(|| std::io::Error::other("job reference not found"))?;
        ensure!(bytes.len() <= 64 * 1024, "job intent too large");
        let intent: JobIntent = serde_json::from_slice(&bytes)?;
        validate_intent(&intent)?;
        ensure!(
            intent.capture_id == capture_id
                && intent.job_id == job_id
                && intent.request_sha256 == request_sha256,
            "job reference mismatch"
        );
        drop(control);
        self.inspect_reference(&intent.reference())
    }

    /// Correlate a previously authenticated reference before returning any
    /// stored path or receipt. Completed inspection verifies the exact durable
    /// binding, native envelope, root identity and sealed output inventory.
    pub fn inspect_reference(&self, reference: &JobReference) -> Result<StoredJob> {
        ensure!(valid_id(&reference.job_id), "invalid job reference");
        let job = self.namespace.existing_child(&reference.job_id)?;
        let control = control_lock(&job)?;
        let intent_bytes = job
            .read("intent.json")?
            .ok_or_else(|| std::io::Error::other("job reference not found"))?;
        ensure!(intent_bytes.len() <= 64 * 1024, "job intent too large");
        let intent: JobIntent = serde_json::from_slice(&intent_bytes)?;
        validate_intent(&intent)?;
        ensure!(
            intent.matches_reference(reference),
            "job reference mismatch"
        );
        let binding = match job.read("binding.json")? {
            Some(bytes) => {
                let binding: Binding = serde_json::from_slice(&bytes)?;
                ensure!(
                    intent_matches_binding(&intent, &binding),
                    "job reference mismatch"
                );
                Some(binding)
            }
            None => None,
        };
        let state = match job.read("state.json")? {
            None => StoredJobState::Preparing,
            Some(bytes) => match parse_state(&bytes)?.as_str() {
                "preparing" => StoredJobState::Preparing,
                "running" => StoredJobState::Running,
                "complete" => StoredJobState::Complete,
                "cancelled" => StoredJobState::Cancelled,
                "deadline_exceeded" => StoredJobState::DeadlineExceeded,
                "failed" => StoredJobState::Failed,
                "owner_lost" => StoredJobState::OwnerLost,
                _ => return Err(std::io::Error::other("unknown job state").into()),
            },
        };
        if state == StoredJobState::Complete {
            let bound = binding
                .as_ref()
                .ok_or_else(|| std::io::Error::other("completed binding missing"))?;
            let output = Arc::new(SecureStore::open(std::path::Path::new(&bound.output_root))?);
            check_output_identity(bound, &output)?;
            let receipt: NativeReceipt = serde_json::from_slice(
                &job.read("native-receipt.json")?
                    .ok_or_else(|| std::io::Error::other("completed receipt missing"))?,
            )?;
            let envelope = job
                .read("envelope.json")?
                .ok_or_else(|| std::io::Error::other("completed envelope missing"))?;
            drop(control);
            verify_receipt_binding(bound, &receipt, &envelope)?;
            verify_output_seals(&output, &receipt, &envelope)?;
            Ok(StoredJob {
                intent,
                binding,
                state,
                receipt: Some(receipt),
                envelope: Some(envelope),
            })
        } else {
            Ok(StoredJob {
                intent,
                binding,
                state,
                receipt: None,
                envelope: None,
            })
        }
    }

    /// Reconcile an abandoned job after caller authentication. Acquiring the
    /// kernel-held execution lock excludes every native writer; admission holds
    /// it before publishing intent and an existing intent is never executed anew.
    /// Terminal reconciliation also removes only files whose native-created
    /// identities were durably recorded by this job.
    pub fn reconcile_if_ownerless(&self, reference: &JobReference) -> Result<StoredJob> {
        ensure!(valid_id(&reference.job_id), "invalid job reference");
        let job = self.namespace.existing_child(&reference.job_id)?;
        let Some(owner) = try_owner_lock(&job)? else {
            return self.inspect_reference(reference);
        };
        let control = control_lock(&job)?;
        let intent = read_job_intent(&job)?;
        ensure!(
            intent.matches_reference(reference),
            "job reference mismatch"
        );
        let state = read_job_state(&job)?;
        if matches!(state, StoredJobState::Preparing | StoredJobState::Running) {
            let terminal = if read_cancel_intent(&job, reference)?.is_some() {
                "cancelled"
            } else {
                "owner_lost"
            };
            replace_state(&job, terminal)?;
        }
        if state != StoredJobState::Complete {
            let intent = read_job_intent(&job)?;
            let output = Arc::new(SecureStore::open(std::path::Path::new(
                &intent.output_root,
            ))?);
            check_output_identity_values(&intent.output_identity, &output)?;
            cleanup_owned_output(&job, &output)?;
        }
        drop(control);
        drop(owner);
        self.inspect_reference(reference)
    }

    /// Persist a cold cancellation request under a short control lock. Caller
    /// authentication must precede the lookup. This never waits for or takes
    /// the long execution-owner lock.
    pub fn request_cancel(&self, reference: &JobReference) -> Result<CancelRequestOutcome> {
        ensure!(valid_id(&reference.job_id), "invalid job reference");
        let job = self.namespace.existing_child(&reference.job_id)?;
        let intent = read_job_intent(&job)?;
        ensure!(
            intent.matches_reference(reference),
            "job reference mismatch"
        );
        let _control = control_lock(&job)?;
        let intent = read_job_intent(&job)?;
        ensure!(
            intent.matches_reference(reference),
            "job reference mismatch"
        );
        match read_job_state(&job)? {
            StoredJobState::Complete => Ok(CancelRequestOutcome::Complete),
            StoredJobState::Cancelled => Ok(CancelRequestOutcome::Cancelled),
            StoredJobState::DeadlineExceeded => Ok(CancelRequestOutcome::DeadlineExceeded),
            StoredJobState::Failed => Ok(CancelRequestOutcome::Failed),
            StoredJobState::OwnerLost => Ok(CancelRequestOutcome::OwnerLost),
            StoredJobState::Preparing | StoredJobState::Running => {
                if let Some(bytes) = job.read("cancel-intent.json")? {
                    let existing: JobReference = serde_json::from_slice(&bytes)?;
                    ensure!(existing == *reference, "cancel intent reference mismatch");
                    return Ok(CancelRequestOutcome::AlreadyRequested);
                }
                job.write_new("cancel-intent.json", &serde_json::to_vec(reference)?)?;
                Ok(CancelRequestOutcome::Requested)
            }
        }
    }

    /// Observe a cancellation intent without acquiring the execution-owner lock.
    /// The short control lock serializes this observation with completion.
    pub fn observe_cancel(&self, reference: &JobReference) -> Result<CancelObservation> {
        ensure!(valid_id(&reference.job_id), "invalid job reference");
        let job = self.namespace.existing_child(&reference.job_id)?;
        let intent = read_job_intent(&job)?;
        ensure!(
            intent.matches_reference(reference),
            "job reference mismatch"
        );
        let _control = control_lock(&job)?;
        let state = read_job_state(&job)?;
        Ok(match state {
            StoredJobState::Complete => CancelObservation::Complete,
            StoredJobState::Cancelled => CancelObservation::Cancelled,
            StoredJobState::DeadlineExceeded => CancelObservation::DeadlineExceeded,
            StoredJobState::Failed => CancelObservation::Failed,
            StoredJobState::OwnerLost => CancelObservation::OwnerLost,
            StoredJobState::Preparing | StoredJobState::Running => {
                if read_cancel_intent(&job, reference)?.is_some() {
                    CancelObservation::Requested
                } else {
                    CancelObservation::NotRequested
                }
            }
        })
    }

    /// Reserve a fresh job ID or return its exact prior terminal result. This is
    /// called only after caller authentication and before native decode/recovery.
    #[cfg(test)]
    pub fn begin(&self, binding: Binding, output: Arc<SecureStore>) -> Result<JobOutcome> {
        validate_binding(&binding)?;
        check_output_identity(&binding, &output)?;
        let job = self.namespace.child(&binding.job_id)?;
        let owner = owner_lock(&job)?;
        let binding_bytes = serde_json::to_vec(&binding)?;
        ensure!(binding_bytes.len() <= 64 * 1024, "job binding too large");
        match job.read("binding.json")? {
            None => {
                let intent = intent_from_binding(&binding)?;
                job.write_new("intent.json", &serde_json::to_vec(&intent)?)?;
                job.write_new("state.json", br#"{"state":"preparing"}"#)?;
                job.write_new("binding.json", &binding_bytes)?;
                replace_state(&job, "running")?;
                Ok(JobOutcome::Owned(JobClaim {
                    job,
                    owner,
                    binding: Some(binding.clone()),
                    intent: intent_from_binding(&binding)?,
                    output,
                }))
            }
            Some(previous) if previous != binding_bytes => {
                Err(std::io::Error::other("job ID is bound to a different request").into())
            }
            Some(_) => {
                let Some(state) = job.read("state.json")? else {
                    return Ok(JobOutcome::ExistingPreparing);
                };
                let parsed: State = match serde_json::from_slice(&state) {
                    Ok(s) => s,
                    Err(_) => return Err(std::io::Error::other("invalid job state").into()),
                };
                match parsed.state.as_str() {
                    "complete" => {
                        let receipt: NativeReceipt =
                            serde_json::from_slice(&job.read("native-receipt.json")?.ok_or_else(
                                || std::io::Error::other("completed job receipt missing"),
                            )?)?;
                        let envelope = job.read("envelope.json")?.ok_or_else(|| {
                            std::io::Error::other("completed job envelope missing")
                        })?;
                        verify_receipt_binding(&binding, &receipt, &envelope)?;
                        verify_output_seals(&output, &receipt, &envelope)?;
                        Ok(JobOutcome::ExistingComplete(receipt, envelope))
                    }
                    "preparing" => Ok(JobOutcome::ExistingPreparing),
                    "running" => Ok(JobOutcome::ExistingRunning),
                    "cancelled" => Ok(JobOutcome::ExistingFailed("cancelled")),
                    "deadline_exceeded" => Ok(JobOutcome::ExistingFailed("deadline_exceeded")),
                    "failed" => Ok(JobOutcome::ExistingFailed("failed")),
                    "owner_lost" => Ok(JobOutcome::ExistingFailed("owner_lost")),
                    _ => Err(std::io::Error::other("unknown terminal job state").into()),
                }
            }
        }
    }

    /// Publisher-side verification. This proves only native Media/2 job output;
    /// bundle publication still validates the final manifest and full inventory.
    pub fn verify_completed(
        &self,
        binding: &Binding,
        output: Arc<SecureStore>,
    ) -> Result<(NativeReceipt, Vec<u8>)> {
        validate_binding(binding)?;
        check_output_identity(binding, &output)?;
        let job = self.namespace.existing_child(&binding.job_id)?;
        ensure!(
            job.read("binding.json")?.as_deref() == Some(serde_json::to_vec(binding)?.as_slice()),
            "native job binding mismatch"
        );
        let state: State = serde_json::from_slice(
            &job.read("state.json")?
                .ok_or_else(|| std::io::Error::other("job state missing"))?,
        )?;
        ensure!(state.state == "complete", "native job is not complete");
        let receipt: NativeReceipt = serde_json::from_slice(
            &job.read("native-receipt.json")?
                .ok_or_else(|| std::io::Error::other("native receipt missing"))?,
        )?;
        let envelope = job
            .read("envelope.json")?
            .ok_or_else(|| std::io::Error::other("native envelope missing"))?;
        verify_receipt_binding(binding, &receipt, &envelope)?;
        verify_output_seals(&output, &receipt, &envelope)?;
        Ok((receipt, envelope))
    }

    /// Record the existing supervisor's already-authorized terminal decision
    /// after its worker has been killed/reaped. The exclusive owner lock proves
    /// the child no longer owns this job. This does not authorize cancellation.
    /// Completion is immutable and wins if it was already sealed. Call before
    /// the supervisor removes any partial output.
    pub fn finish_reaped(
        &self,
        binding: &Binding,
        output: Arc<SecureStore>,
        reason: Checkpoint,
    ) -> Result<JobOutcome> {
        ensure!(
            reason != Checkpoint::Continue,
            "reaped worker needs a terminal decision"
        );
        validate_binding(binding)?;
        check_output_identity(binding, &output)?;
        let job = self.namespace.existing_child(&binding.job_id)?;
        let _owner = owner_lock(&job)?;
        let control = control_lock(&job)?;
        ensure!(
            job.read("binding.json")?.as_deref() == Some(serde_json::to_vec(binding)?.as_slice()),
            "reaped worker binding mismatch"
        );
        let state: State = serde_json::from_slice(
            &job.read("state.json")?
                .ok_or_else(|| std::io::Error::other("job state missing"))?,
        )?;
        match state.state.as_str() {
            "complete" => {
                let receipt: NativeReceipt = serde_json::from_slice(
                    &job.read("native-receipt.json")?
                        .ok_or_else(|| std::io::Error::other("completed job receipt missing"))?,
                )?;
                let envelope = job
                    .read("envelope.json")?
                    .ok_or_else(|| std::io::Error::other("completed job envelope missing"))?;
                drop(control);
                verify_receipt_binding(binding, &receipt, &envelope)?;
                verify_output_seals(&output, &receipt, &envelope)?;
                Ok(JobOutcome::ExistingComplete(receipt, envelope))
            }
            "preparing" | "running" => {
                let terminal =
                    if read_cancel_intent(&job, &intent_from_binding(binding)?.reference())?
                        .is_some()
                    {
                        "cancelled"
                    } else {
                        terminal_state(reason)
                    };
                replace_state(&job, terminal)?;
                cleanup_owned_output(&job, &output)?;
                Ok(JobOutcome::ExistingFailed(terminal))
            }
            "cancelled" | "deadline_exceeded" | "failed" | "owner_lost" => {
                cleanup_owned_output(&job, &output)?;
                Ok(JobOutcome::ExistingFailed(match state.state.as_str() {
                    "cancelled" => "cancelled",
                    "deadline_exceeded" => "deadline_exceeded",
                    "failed" => "failed",
                    _ => "owner_lost",
                }))
            }
            _ => Err(std::io::Error::other("unknown terminal job state").into()),
        }
    }

    /// Record the supervisor's terminal decision for a request that was reaped
    /// while source pinning was still preparing and before a full Binding existed.
    pub fn finish_intent_reaped(
        &self,
        reference: &JobReference,
        output: Arc<SecureStore>,
        reason: Checkpoint,
    ) -> Result<JobOutcome> {
        ensure!(
            reason != Checkpoint::Continue,
            "reaped worker needs a terminal decision"
        );
        ensure!(valid_id(&reference.job_id), "invalid job reference");
        let job = self.namespace.existing_child(&reference.job_id)?;
        let Some(_owner) = try_owner_lock(&job)? else {
            // This supervisor reaped its child, not the peer that owns this
            // exact intent. Correlate the reference, leave that writer alone,
            // and let the service continue inspecting its durable state.
            self.inspect_reference(reference)?;
            return Ok(JobOutcome::ExistingInProgress);
        };
        let control = control_lock(&job)?;
        let intent: JobIntent = serde_json::from_slice(
            &job.read("intent.json")?
                .ok_or_else(|| std::io::Error::other("job intent missing"))?,
        )?;
        validate_intent(&intent)?;
        ensure!(
            intent.matches_reference(reference),
            "job reference mismatch"
        );
        check_output_identity_values(&intent.output_identity, &output)?;
        match job.read("state.json")? {
            None => {
                let terminal = if read_cancel_intent(&job, reference)?.is_some() {
                    "cancelled"
                } else {
                    terminal_state(reason)
                };
                replace_state_create(&job, terminal)?;
                cleanup_owned_output(&job, &output)?;
                Ok(JobOutcome::ExistingFailed(terminal))
            }
            Some(bytes) => match parse_state(&bytes)?.as_str() {
                "preparing" | "running" => {
                    let terminal = if read_cancel_intent(&job, reference)?.is_some() {
                        "cancelled"
                    } else {
                        terminal_state(reason)
                    };
                    replace_state(&job, terminal)?;
                    cleanup_owned_output(&job, &output)?;
                    Ok(JobOutcome::ExistingFailed(terminal))
                }
                "complete" => {
                    drop(control);
                    let stored = self.inspect_reference(reference)?;
                    Ok(JobOutcome::ExistingComplete(
                        stored
                            .receipt
                            .ok_or_else(|| std::io::Error::other("completed receipt missing"))?,
                        stored
                            .envelope
                            .ok_or_else(|| std::io::Error::other("completed envelope missing"))?,
                    ))
                }
                "cancelled" | "deadline_exceeded" | "failed" | "owner_lost" => {
                    cleanup_owned_output(&job, &output)?;
                    Ok(JobOutcome::ExistingFailed(
                        match parse_state(&bytes)?.as_str() {
                            "cancelled" => "cancelled",
                            "deadline_exceeded" => "deadline_exceeded",
                            "failed" => "failed",
                            _ => "owner_lost",
                        },
                    ))
                }
                _ => Err(std::io::Error::other("unknown job state").into()),
            },
        }
    }
}

pub struct JobClaim {
    job: Arc<SecureStore>,
    owner: File,
    binding: Option<Binding>,
    intent: JobIntent,
    output: Arc<SecureStore>,
}

impl JobClaim {
    /// Bind the verified retained source to the already admitted request. The
    /// request intent and original deadline remain immutable across this step.
    pub fn bind_source(&mut self, binding: Binding) -> Result<()> {
        validate_binding(&binding)?;
        ensure!(
            intent_matches_binding(&self.intent, &binding),
            "source binding differs from admitted intent"
        );
        check_output_identity(&binding, &self.output)?;
        ensure!(self.binding.is_none(), "source binding already finalized");
        let _control = control_lock(&self.job)?;
        ensure!(
            read_cancel_intent(&self.job, &self.intent.reference())?.is_none(),
            "cancelled"
        );
        self.job
            .write_new("binding.json", &serde_json::to_vec(&binding)?)?;
        replace_state(&self.job, "running")?;
        self.binding = Some(binding);
        Ok(())
    }

    pub fn complete(&mut self, receipt: NativeReceipt, envelope: &[u8]) -> Result<()> {
        let binding = self
            .binding
            .as_ref()
            .ok_or_else(|| std::io::Error::other("source binding missing"))?;
        verify_receipt_binding(binding, &receipt, envelope)?;
        let envelope_bytes = envelope;
        verify_output_seals(&self.output, &receipt, envelope_bytes)?;
        self.job.write_new("envelope.json", envelope_bytes)?;
        self.job
            .write_new("native-receipt.json", &serde_json::to_vec(&receipt)?)?;
        let _control = control_lock(&self.job)?;
        match read_job_state(&self.job)? {
            StoredJobState::Complete => return Ok(()),
            StoredJobState::Cancelled => return Err(std::io::Error::other("cancelled").into()),
            StoredJobState::DeadlineExceeded => {
                return Err(std::io::Error::other("deadline_exceeded").into());
            }
            StoredJobState::Failed => return Err(std::io::Error::other("failed").into()),
            StoredJobState::OwnerLost => return Err(std::io::Error::other("owner_lost").into()),
            StoredJobState::Preparing | StoredJobState::Running => {}
        }
        if read_cancel_intent(&self.job, &self.intent.reference())?.is_some() {
            replace_state(&self.job, "cancelled")?;
            return Err(std::io::Error::other("cancelled").into());
        }
        replace_state(&self.job, "complete")?;
        let _ = self.owner.metadata()?;
        Ok(())
    }

    pub fn fail(&mut self, checkpoint: Checkpoint) -> Result<()> {
        let _control = control_lock(&self.job)?;
        match read_job_state(&self.job)? {
            StoredJobState::Complete => {
                return Err(std::io::Error::other("completed job is immutable").into());
            }
            StoredJobState::Cancelled
            | StoredJobState::DeadlineExceeded
            | StoredJobState::Failed
            | StoredJobState::OwnerLost => return Ok(()),
            StoredJobState::Preparing | StoredJobState::Running => {}
        }
        let terminal = if read_cancel_intent(&self.job, &self.intent.reference())?.is_some() {
            "cancelled"
        } else {
            terminal_state(checkpoint)
        };
        replace_state(&self.job, terminal)
    }

    pub fn cancel_requested(&self) -> Result<bool> {
        Ok(read_cancel_intent(&self.job, &self.intent.reference())?.is_some())
    }

    pub(super) fn cancel_probe(&self) -> (Arc<SecureStore>, JobReference) {
        (self.job.clone(), self.intent.reference())
    }
}

fn terminal_state(checkpoint: Checkpoint) -> &'static str {
    match checkpoint {
        Checkpoint::Cancel => "cancelled",
        Checkpoint::Deadline => "deadline_exceeded",
        Checkpoint::Continue => "failed",
        Checkpoint::Failed => "failed",
        Checkpoint::OwnerLost => "owner_lost",
    }
}

fn owner_lock(store: &SecureStore) -> Result<File> {
    try_owner_lock(store)?
        .ok_or_else(|| std::io::Error::other("job is owned by another live worker").into())
}

fn control_lock(store: &SecureStore) -> Result<File> {
    let file = match store.file("control.lock", true) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            store.file("control.lock", false)?
        }
        Err(error) => return Err(error.into()),
    };
    loop {
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if rc == 0 {
            return Ok(file);
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINTR) {
            return Err(error.into());
        }
    }
}

fn read_job_intent(job: &SecureStore) -> Result<JobIntent> {
    let bytes = job
        .read("intent.json")?
        .ok_or_else(|| std::io::Error::other("job reference not found"))?;
    ensure!(bytes.len() <= 64 * 1024, "job intent too large");
    let intent: JobIntent = serde_json::from_slice(&bytes)?;
    validate_intent(&intent)?;
    Ok(intent)
}

fn read_job_state(job: &SecureStore) -> Result<StoredJobState> {
    let Some(bytes) = job.read("state.json")? else {
        return Ok(StoredJobState::Preparing);
    };
    Ok(match parse_state(&bytes)?.as_str() {
        "preparing" => StoredJobState::Preparing,
        "running" => StoredJobState::Running,
        "complete" => StoredJobState::Complete,
        "cancelled" => StoredJobState::Cancelled,
        "deadline_exceeded" => StoredJobState::DeadlineExceeded,
        "failed" => StoredJobState::Failed,
        "owner_lost" => StoredJobState::OwnerLost,
        _ => return Err(std::io::Error::other("unknown job state").into()),
    })
}

pub(super) fn read_cancel_intent(
    job: &SecureStore,
    reference: &JobReference,
) -> Result<Option<()>> {
    let Some(bytes) = job.read("cancel-intent.json")? else {
        return Ok(None);
    };
    let stored: JobReference = serde_json::from_slice(&bytes)?;
    ensure!(stored == *reference, "cancel intent reference mismatch");
    Ok(Some(()))
}

fn try_owner_lock(store: &SecureStore) -> Result<Option<File>> {
    let file = match store.file("owner.lock", true) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            store.file("owner.lock", false)?
        }
        Err(e) => return Err(e.into()),
    };
    // SAFETY: flock operates on the live open file descriptor and keeps ownership
    // exclusive until this descriptor closes, including every early return.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        return Ok(Some(file));
    }
    let error = std::io::Error::last_os_error();
    if error
        .raw_os_error()
        .is_some_and(|code| code == libc::EWOULDBLOCK || code == libc::EAGAIN)
    {
        Ok(None)
    } else {
        Err(error.into())
    }
}

fn replace_state(job: &SecureStore, value: &str) -> Result<()> {
    let bytes = serde_json::to_vec(&State {
        state: value.into(),
    })?;
    job.replace_checked("state.json", &bytes)?;
    Ok(())
}

fn replace_state_create(job: &SecureStore, value: &str) -> Result<()> {
    let bytes = serde_json::to_vec(&State {
        state: value.into(),
    })?;
    job.write_new("state.json", &bytes)?;
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    state: String,
}

fn validate_binding(binding: &Binding) -> Result<()> {
    ensure!(
        binding.schema == "apparent.media2-job/1",
        "unsupported internal job schema"
    );
    ensure!(
        valid_id(&binding.capture_id)
            && valid_id(&binding.job_id)
            && valid_id(&binding.receipt_id)
            && valid_id(&binding.materials.materials_id),
        "invalid native job identity"
    );
    ensure!(
        binding.materials.capture_id == binding.capture_id,
        "capture/material identity mismatch"
    );
    ensure!(
        valid_hash(&binding.materials.revision),
        "invalid materials revision"
    );
    ensure!(
        valid_hash(&binding.source_video.sha256),
        "invalid source digest"
    );
    ensure!(
        valid_hash(&binding.request_sha256),
        "invalid original request digest"
    );
    ensure!(
        binding.requested_bundle_ms.len() > 0
            && binding.requested_bundle_ms.len() <= 48
            && binding.limits.max_frames > 0
            && binding.limits.max_frames <= 48
            && binding.requested_bundle_ms.len() as u64 <= binding.limits.max_frames,
        "invalid frame request count"
    );
    ensure!(
        binding.limits.max_edge > 0
            && binding.limits.max_edge <= 4096
            && binding.limits.max_total_pixels > 0
            && binding.limits.max_total_pixels <= 64_000_000
            && binding.limits.deadline_ms > 0
            && binding.limits.deadline_ms <= 60_000,
        "invalid job limits"
    );
    ensure!(
        valid_abs(&binding.output_root) && valid_abs(&binding.materials_root),
        "invalid job root"
    );
    ensure!(
        valid_decimal(&binding.materials_root_identity.device)
            && valid_decimal(&binding.materials_root_identity.inode),
        "invalid retained root identity"
    );
    let output_root = std::path::Path::new(&binding.output_root);
    let materials_root = std::path::Path::new(&binding.materials_root);
    ensure!(
        !output_root.starts_with(materials_root) && !materials_root.starts_with(output_root),
        "derivative output overlaps retained materials"
    );
    if let Some(layout) = &binding.layout {
        layout
            .validate(binding.requested_bundle_ms.len())
            .map_err(std::io::Error::other)?;
        ensure!(
            valid_hash(&layout.sha256().map_err(std::io::Error::other)?),
            "invalid layout hash"
        );
    }
    Ok(())
}

fn validate_intent(intent: &JobIntent) -> Result<()> {
    ensure!(
        intent.schema == "apparent.media2-job-intent/1",
        "unsupported job intent schema"
    );
    ensure!(
        valid_id(&intent.capture_id)
            && valid_id(&intent.job_id)
            && valid_id(&intent.lease_id)
            && valid_id(&intent.materials.materials_id),
        "invalid job intent identity"
    );
    ensure!(
        intent.materials.capture_id == intent.capture_id,
        "intent capture/material mismatch"
    );
    ensure!(
        valid_hash(&intent.materials.revision) && valid_hash(&intent.request_sha256),
        "invalid intent digest"
    );
    ensure!(
        !intent.requested_bundle_ms.is_empty()
            && intent.requested_bundle_ms.len() as u64 <= intent.limits.max_frames,
        "invalid intent frame request count"
    );
    ensure!(
        intent
            .requested_bundle_ms
            .iter()
            .all(|time| *time <= 9_007_199_254_740_991),
        "intent timestamp exceeds interoperable integer range"
    );
    ensure!(
        intent.limits.max_frames > 0
            && intent.limits.max_frames <= 64
            && intent.limits.max_edge > 0
            && intent.limits.max_edge <= 4096
            && intent.limits.max_total_pixels > 0
            && intent.limits.max_total_pixels <= 64_000_000
            && intent.limits.deadline_ms > 0
            && intent.limits.deadline_ms <= 60_000,
        "invalid intent limits"
    );
    ensure!(valid_abs(&intent.output_root), "invalid intent output root");
    ensure!(
        valid_decimal(&intent.output_identity.device)
            && valid_decimal(&intent.output_identity.inode),
        "invalid intent output identity"
    );
    if let Some(layout) = &intent.layout {
        layout
            .validate(intent.requested_bundle_ms.len())
            .map_err(std::io::Error::other)?;
    }
    ensure!(
        intent
            .admitted_at_ms
            .checked_add(intent.limits.deadline_ms)
            .is_some(),
        "invalid intent deadline"
    );
    Ok(())
}

fn intent_from_binding(binding: &Binding) -> Result<JobIntent> {
    Ok(JobIntent {
        schema: "apparent.media2-job-intent/1".into(),
        capture_id: binding.capture_id.clone(),
        materials: binding.materials.clone(),
        lease_id: binding.lease_id.clone(),
        job_id: binding.job_id.clone(),
        request_sha256: binding.request_sha256.clone(),
        requested_bundle_ms: binding.requested_bundle_ms.clone(),
        limits: binding.limits.clone(),
        output_root: binding.output_root.clone(),
        output_identity: binding.output_identity.clone(),
        layout: binding.layout.clone(),
        admitted_at_ms: unix_time_ms(),
    })
}

fn intent_matches_binding(intent: &JobIntent, binding: &Binding) -> bool {
    intent.capture_id == binding.capture_id
        && intent.materials == binding.materials
        && intent.lease_id == binding.lease_id
        && intent.job_id == binding.job_id
        && intent.request_sha256 == binding.request_sha256
        && intent.requested_bundle_ms == binding.requested_bundle_ms
        && intent.limits == binding.limits
        && intent.output_root == binding.output_root
        && intent.output_identity == binding.output_identity
        && intent.layout == binding.layout
}

fn check_output_identity_values(expected: &OutputIdentity, output: &SecureStore) -> Result<()> {
    let metadata = output.directory_metadata()?;
    ensure!(
        metadata.dev().to_string() == expected.device
            && metadata.ino().to_string() == expected.inode,
        "job output directory identity changed"
    );
    Ok(())
}

fn read_owned_inventory(job: &SecureStore) -> Result<OwnedOutputInventory> {
    match job.read("owned-output.json")? {
        Some(bytes) => {
            ensure!(bytes.len() <= 16 * 1024, "owned output inventory too large");
            let inventory: OwnedOutputInventory = serde_json::from_slice(&bytes)?;
            ensure!(
                inventory.schema == "apparent.media2-owned-output/1",
                "invalid owned output inventory"
            );
            ensure!(
                inventory.files.len() <= 50,
                "owned output inventory too large"
            );
            Ok(inventory)
        }
        None => Ok(OwnedOutputInventory {
            schema: "apparent.media2-owned-output/1".into(),
            files: Vec::new(),
            frames_directory: None,
        }),
    }
}

/// Remove only registered native output identities. The caller holds the
/// execution-owner and control locks after worker reap. Any path, identity or
/// directory ambiguity leaves the remaining evidence untouched and returns
/// an error so cleanup is never reported as complete.
fn cleanup_owned_output(job: &SecureStore, output: &Arc<SecureStore>) -> Result<()> {
    let Some(bytes) = job.read("owned-output.json")? else {
        ensure!(
            !output.entry_exists("frames")?
                && !output.entry_exists("contact-sheet.png")?
                && !output.entry_exists("receipt.json")?,
            "unowned media output entry present"
        );
        return Ok(());
    };
    let intent = read_job_intent(job)?;
    ensure!(bytes.len() <= 16 * 1024, "owned output inventory too large");
    let inventory: OwnedOutputInventory = serde_json::from_slice(&bytes)?;
    ensure!(
        inventory.schema == "apparent.media2-owned-output/1",
        "invalid owned output inventory"
    );
    ensure!(
        inventory.files.len() <= 50,
        "owned output inventory too large"
    );
    let mut names = std::collections::BTreeSet::new();
    let mut frame_entries = Vec::new();
    for file in &inventory.files {
        ensure!(
            names.insert(file.relative_path.as_str()),
            "duplicate owned output"
        );
        if let Some(leaf) = file.relative_path.strip_prefix("frames/") {
            let index = leaf
                .strip_prefix("frame-")
                .and_then(|name| name.strip_suffix(".png"))
                .and_then(|index| index.parse::<usize>().ok())
                .ok_or_else(|| std::io::Error::other("invalid owned frame path"))?;
            ensure!(
                index > 0
                    && index <= intent.requested_bundle_ms.len()
                    && leaf == format!("frame-{index:03}.png"),
                "invalid owned frame path"
            );
            frame_entries.push(file);
            continue;
        } else {
            ensure!(
                file.relative_path == "receipt.json"
                    || (file.relative_path == "contact-sheet.png" && intent.layout.is_some()),
                "invalid owned output path"
            );
            if let Some(identity) = &file.identity {
                output.remove_file_if_identity(
                    &file.relative_path,
                    identity.device.parse()?,
                    identity.inode.parse()?,
                )?;
            } else {
                ensure!(
                    !output.entry_exists(&file.relative_path)?,
                    "unidentified reserved output remains"
                );
            }
            continue;
        }
    }
    if !frame_entries.is_empty() || inventory.frames_directory.is_some() {
        if output.entry_exists("frames")? {
            let frames = output.existing_child("frames")?;
            let directory = inventory
                .frames_directory
                .as_ref()
                .ok_or_else(|| std::io::Error::other("frames directory ownership missing"))?;
            let expected = directory
                .identity
                .as_ref()
                .ok_or_else(|| std::io::Error::other("frames directory identity missing"))?;
            check_output_identity_values(expected, &frames)?;
            for file in frame_entries {
                let leaf = file.relative_path.strip_prefix("frames/").unwrap();
                if let Some(identity) = &file.identity {
                    frames.remove_file_if_identity(
                        leaf,
                        identity.device.parse()?,
                        identity.inode.parse()?,
                    )?;
                } else {
                    ensure!(
                        !frames.entry_exists(leaf)?,
                        "unidentified reserved frame remains"
                    );
                }
            }
            output.remove_empty_child_if_identity(
                "frames",
                expected.device.parse()?,
                expected.inode.parse()?,
            )?;
        } else {
            // A prior cleanup removed the tracked directory, or it was never
            // created after reservation. Its absence makes all child names absent.
        }
    } else {
        ensure!(
            !output.entry_exists("frames")?,
            "unowned frames directory present"
        );
    }
    Ok(())
}

fn parse_state(bytes: &[u8]) -> Result<String> {
    let parsed: State = serde_json::from_slice(bytes)?;
    Ok(parsed.state)
}

fn unix_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn check_output_identity(binding: &Binding, output: &SecureStore) -> Result<()> {
    let metadata = output.directory_metadata()?;
    let path_store = SecureStore::open(std::path::Path::new(&binding.output_root))?;
    let path_metadata = path_store.directory_metadata()?;
    ensure!(
        metadata.mode() & 0o777 == 0o700,
        "output directory must be private"
    );
    ensure!(
        path_metadata.dev() == metadata.dev() && path_metadata.ino() == metadata.ino(),
        "output path does not identify the pinned Store"
    );
    ensure!(
        metadata.dev().to_string() == binding.output_identity.device
            && metadata.ino().to_string() == binding.output_identity.inode,
        "output root identity changed"
    );
    Ok(())
}

fn verify_receipt_binding(
    binding: &Binding,
    receipt: &NativeReceipt,
    envelope: &[u8],
) -> Result<()> {
    let expected_hash = hex(&Sha256::digest(serde_json::to_vec(binding)?));
    ensure!(
        receipt.schema == "apparent.media2-native-job-receipt/1"
            && receipt.binding_sha256 == expected_hash,
        "native receipt binding mismatch"
    );
    ensure!(
        receipt.capture_id == binding.capture_id
            && receipt.materials.capture_id == binding.materials.capture_id
            && receipt.materials.materials_id == binding.materials.materials_id
            && receipt.materials.revision == binding.materials.revision
            && receipt.lease_id == binding.lease_id
            && receipt.job_id == binding.job_id
            && receipt.receipt_id == binding.receipt_id,
        "native receipt identity mismatch"
    );
    ensure!(
        receipt.request_sha256 == binding.request_sha256,
        "native request digest mismatch"
    );
    ensure!(
        receipt.frames.len() == binding.requested_bundle_ms.len(),
        "native receipt frame count mismatch"
    );
    ensure!(
        hex(&Sha256::digest(envelope)) == receipt.envelope_sha256,
        "native envelope digest mismatch"
    );
    let envelope_value: serde_json::Value = serde_json::from_slice(envelope)?;
    ensure!(
        envelope_value["schema"] == "apparent.media-derivative/2"
            && envelope_value["materials"] == serde_json::to_value(&binding.materials)?
            && envelope_value["root"] == binding.output_root
            && envelope_value["requestSha256"] == binding.request_sha256
            && envelope_value["receiptId"] == binding.receipt_id
            && envelope_value["leaseId"] == binding.lease_id,
        "native derivative envelope binding mismatch"
    );
    ensure!(
        envelope_value["payload"]["schema"] == "apparent.media-derivative-payload/2"
            && envelope_value["payload"]["materials"] == serde_json::to_value(&binding.materials)?
            && envelope_value["payload"]["leaseId"] == binding.lease_id
            && envelope_value["payload"]["jobId"] == binding.job_id
            && envelope_value["payload"]["receiptId"] == binding.receipt_id
            && envelope_value["payload"]["root"] == binding.output_root
            && envelope_value["payload"]["requestSha256"] == binding.request_sha256
            && envelope_value["payload"]["frames"]
                .as_array()
                .is_some_and(|frames| frames.len() == receipt.frames.len()),
        "native derivative payload binding mismatch"
    );
    let mut total_pixels = 0u64;
    match (&binding.layout, &envelope_value["payload"]["contactSheet"]) {
        (None, serde_json::Value::Null) => {
            ensure!(receipt.sheet.is_none(), "unexpected native sheet receipt")
        }
        (Some(layout), sheet) => {
            let hash = layout.sha256().map_err(std::io::Error::other)?;
            let sheet_seal = receipt
                .sheet
                .as_ref()
                .ok_or_else(|| std::io::Error::other("native sheet seal missing"))?;
            total_pixels = total_pixels
                .checked_add(u64::from(layout.width) * u64::from(layout.height))
                .ok_or_else(|| std::io::Error::other("native output pixel overflow"))?;
            ensure!(
                sheet["layout"] == serde_json::to_value(layout)?
                    && sheet["layoutSha256"] == hash
                    && sheet["width"].as_u64() == Some(u64::from(layout.width))
                    && sheet["height"].as_u64() == Some(u64::from(layout.height))
                    && sheet["encoding"] == "png"
                    && sheet["byteLength"].as_u64() == Some(sheet_seal.file.identity.size)
                    && sheet["file"] == serde_json::to_value(&sheet_seal.file)?
                    && sheet_seal.layout_hash == hash,
                "native contact sheet layout differs from original request"
            );
        }
        _ => return Err(std::io::Error::other("native contact sheet layout mismatch").into()),
    }
    for (index, seal) in receipt.frames.iter().enumerate() {
        let frame = &envelope_value["payload"]["frames"][index];
        let requested = binding.requested_bundle_ms[index];
        let width = frame["width"]
            .as_u64()
            .ok_or_else(|| std::io::Error::other("native frame width missing"))?;
        let height = frame["height"]
            .as_u64()
            .ok_or_else(|| std::io::Error::other("native frame height missing"))?;
        total_pixels = total_pixels
            .checked_add(width * height)
            .ok_or_else(|| std::io::Error::other("native output pixel overflow"))?;
        ensure!(
            frame["requestIndex"].as_u64() == Some(index as u64 + 1)
                && frame["requestedBundleMs"].as_u64() == Some(requested)
                && frame["effectiveVideoMs"].as_u64()
                    == Some(requested.saturating_sub(binding.video_offset_ms))
                && frame["actualVideoMs"].as_u64().is_some()
                && frame["clampedToStart"].as_bool() == Some(requested < binding.video_offset_ms)
                && frame["clampedToEnd"].as_bool().is_some()
                && !(frame["clampedToStart"] == true && frame["clampedToEnd"] == true)
                && frame["encoding"] == "png"
                && frame["byteLength"].as_u64() == Some(seal.identity.size)
                && width > 0
                && height > 0
                && width <= u64::from(binding.limits.max_edge)
                && height <= u64::from(binding.limits.max_edge)
                && seal.relative_path == format!("frames/frame-{:03}.png", index + 1)
                && frame["file"] == serde_json::to_value(seal)?,
            "native frame inventory differs from durable receipt"
        );
    }
    match (&receipt.sheet, &envelope_value["payload"]["contactSheet"]) {
        (None, serde_json::Value::Null) => {}
        (Some(sheet), value) => ensure!(
            value["file"] == serde_json::to_value(&sheet.file)?
                && value["layoutSha256"] == sheet.layout_hash,
            "native sheet inventory differs from durable receipt"
        ),
        _ => return Err(std::io::Error::other("native sheet inventory mismatch").into()),
    }
    ensure!(
        total_pixels <= binding.limits.max_total_pixels,
        "native output exceeds durable pixel limit"
    );
    Ok(())
}

fn verify_output_seals(
    output: &Arc<SecureStore>,
    receipt: &NativeReceipt,
    envelope: &[u8],
) -> Result<()> {
    let mut seen = std::collections::HashSet::new();
    ensure!(
        receipt
            .frames
            .iter()
            .all(|f| seen.insert(f.relative_path.as_str())),
        "duplicate frame output path"
    );
    ensure!(
        !seen.contains(receipt.payload_file.relative_path.as_str()),
        "receipt aliases frame output"
    );
    if let Some(sheet) = &receipt.sheet {
        ensure!(
            !seen.contains(sheet.file.relative_path.as_str())
                && sheet.file.relative_path != receipt.payload_file.relative_path,
            "sheet aliases another output"
        );
    }
    for seal in receipt
        .frames
        .iter()
        .chain(std::iter::once(&receipt.payload_file))
        .chain(receipt.sheet.iter().map(|s| &s.file))
    {
        let path = &seal.relative_path;
        let (store, leaf) = if let Some(leaf) = path.strip_prefix("frames/") {
            ensure!(
                !leaf.is_empty() && !leaf.contains('/'),
                "invalid output seal path"
            );
            (output.child("frames")?, leaf.to_owned())
        } else {
            ensure!(
                path == "receipt.json" || path == "contact-sheet.png",
                "invalid output seal path"
            );
            (output.clone(), path.clone())
        };
        let mut file = store.file(&leaf, false)?;
        let metadata = file.metadata()?;
        ensure!(
            metadata.dev().to_string() == seal.identity.device
                && metadata.ino().to_string() == seal.identity.inode
                && metadata.len() == seal.identity.size
                && (i128::from(metadata.mtime()) * 1_000_000_000
                    + i128::from(metadata.mtime_nsec()))
                .to_string()
                    == seal.identity.modified_ns,
            "output identity changed"
        );
        let mut hasher = Sha256::new();
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = file.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        ensure!(
            hex(&hasher.finalize()) == seal.sha256,
            "output digest changed"
        );
    }
    let payload_bytes = {
        let mut payload = output.file("receipt.json", false)?;
        let mut bytes = Vec::new();
        payload.read_to_end(&mut bytes)?;
        bytes
    };
    let payload_value: serde_json::Value = serde_json::from_slice(&payload_bytes)?;
    let envelope_value: serde_json::Value = serde_json::from_slice(envelope)?;
    ensure!(
        envelope_value["schema"] == "apparent.media-derivative/2",
        "invalid derivative envelope schema"
    );
    ensure!(
        envelope_value["materials"]["captureId"] == receipt.materials.capture_id
            && envelope_value["materials"]["materialsId"] == receipt.materials.materials_id
            && envelope_value["materials"]["revision"] == receipt.materials.revision,
        "derivative envelope materials mismatch"
    );
    ensure!(
        envelope_value["jobId"] == receipt.job_id
            && envelope_value["receiptId"] == receipt.receipt_id,
        "derivative envelope job mismatch"
    );
    ensure!(
        envelope_value["receiptFile"]["sha256"] == receipt.payload_file.sha256,
        "derivative envelope receipt reference mismatch"
    );
    ensure!(
        envelope_value["payload"] == payload_value,
        "detached receipt differs from envelope payload"
    );
    Ok(())
}

pub fn verify_completed(
    registry: &JobRegistry,
    binding: &Binding,
    output: Arc<SecureStore>,
) -> Result<(NativeReceipt, Vec<u8>)> {
    registry.verify_completed(binding, output)
}

fn valid_id(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn valid_hash(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn valid_decimal(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}
fn valid_abs(value: &str) -> bool {
    std::path::Path::new(value).is_absolute()
        && std::path::Path::new(value).components().all(|part| {
            matches!(
                part,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )
        })
}
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 15) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests;
