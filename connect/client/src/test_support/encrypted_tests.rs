//! Real client + curl + loopback opaque storage. This synthetic approval registry
//! is not the Worker/service, and performs no encryption or decryption itself.
use super::*;

fn encryption_server(mode: &str) -> Server {
    let server = Server::new(mode);
    server.state.lock().unwrap().encryption = Some(EncryptionState::default());
    server
}

fn encrypted(f: &Fixture) {
    f.client
        .provider
        .set_experiment(HOSTED_ENCRYPTION, true)
        .unwrap();
}

// Worker-path cases start with synthetic credentials but real generated endpoint
// identities. The separate login test below exercises actual approval admission.
fn pair(f: &Fixture, server: &Server) {
    f.authorize(&server.url);
    encrypted(f);
    for scope in ["upload", "read"] {
        let identity_id = random_id().unwrap();
        let identity = encryption::load_or_create_identity(
            &f.client.state.join(format!("{identity_id}.identity")),
        )
        .unwrap();
        let mut credential = f.client.credential(scope, &server.url).unwrap();
        credential.encryption_identity = Some(identity_id);
        credential
            .apply_reply(
                &json!({"scope":scope,
            "accessToken":format!("synthetic-{scope}-secret"),
            "refreshToken":format!("synthetic-refresh-{scope}"),
            "grantId":format!("opaque-approved-{scope}-grant"),
            "expiresIn":3600,"refreshExpiresIn":86400}),
                now(),
                false,
            )
            .unwrap();
        atomic_json(
            &f.client.state.join(format!("{scope}.credential")),
            &credential,
        )
        .unwrap();
        server
            .state
            .lock()
            .unwrap()
            .encryption
            .as_mut()
            .unwrap()
            .devices
            .push(json!({"grantId":credential.grant_id,"scope":scope,"key":identity.public_key()}));
    }
}

fn job(f: &Fixture) -> Job {
    f.client.jobs().unwrap().remove(0)
}
fn assert_ready(f: &Fixture) {
    let queued = job(f);
    if queued.status != "ready" {
        // Retry the actual upload entry solely to expose its typed error before
        // process_once sanitizes it. Never accept a successful diagnostic retry
        // in place of the worker's failed positive control, nor print a job or
        // credential (both can carry private filesystem/authentication details).
        let mut diagnostic = queued;
        let error = f
            .client
            .upload(&mut diagnostic)
            .err()
            .map(|e| format!("{e:#}"));
        panic!("encrypted worker positive control not ready; direct upload error: {error:?}");
    }
}
fn upload(f: &Fixture, server: &Server) -> (PathBuf, String) {
    pair(f, server);
    let bundle = f.bundle();
    let id = f.client.enqueue(&bundle).unwrap();
    f.client.process_once().unwrap();
    assert_ready(f);
    assert!(server.state.lock().unwrap().ready);
    (bundle, id)
}
fn no_media(server: &Server) {
    let state = server.state.lock().unwrap();
    assert!(state.data.is_empty());
    assert!(!state.ready);
    assert!(
        !state
            .request_routes
            .iter()
            .any(|r| r.starts_with("PUT ") || r.ends_with("/complete"))
    );
}
fn assert_bundle(source: &Path, out: &Path) {
    let stage = Scratch::new_output(out.parent().unwrap()).unwrap();
    let files = snapshot(source, &stage.0).unwrap();
    for file in files {
        assert_eq!(
            fs::read(source.join(&file.path)).unwrap(),
            fs::read(out.join(&file.path)).unwrap(),
            "decrypted bundle differs at {}",
            file.path
        );
    }
}
fn fetch_refused(f: &Fixture, id: &str, name: &str, expected: &str) {
    let out = f.root.join(format!("{name}.apparent"));
    let error = f.client.fetch(id, &out, 0).unwrap_err().to_string();
    assert!(
        error.contains(expected),
        "wrong refusal for {name}: {error}"
    );
    assert!(!out.exists(), "published rejected plaintext");
    assert!(
        !fs::read_dir(&f.root).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".apparent-fetch-")),
        "failed fetch retained plaintext scratch"
    );
}

#[test]
fn encrypted_login_binds_public_key_before_approval_and_read_never_enables_upload() {
    let f = Fixture::new();
    let server = encryption_server("");
    encrypted(&f);
    let gate = Arc::new(MediaGate::default());
    server.state.lock().unwrap().media_gate = Some(gate.clone());
    let client = f.client.clone();
    let origin = server.url.clone();
    let worker = std::thread::spawn(move || client.login(&origin, "read", false, false));
    let deadline = Instant::now() + Duration::from_secs(8);
    while !gate.started.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    let reached = gate.started.load(Ordering::SeqCst);
    // Always release/join before asserting, including fixture failure.
    let before_approval = {
        let state = server.state.lock().unwrap();
        let registry = state.encryption.as_ref().unwrap();
        registry.devices.is_empty()
            && registry.pending_key.is_some()
            && !f.client.state.join("read.credential").exists()
    };
    let identity_before = fs::read_dir(&f.client.state)
        .unwrap()
        .filter_map(|entry| {
            let path = entry.unwrap().path();
            (path.extension().is_some_and(|e| e == "identity")).then_some(path)
        })
        .collect::<Vec<_>>();
    gate.release.store(true, Ordering::SeqCst);
    worker.join().unwrap().unwrap();
    assert!(
        reached && before_approval,
        "approval did not gate public key admission"
    );
    assert_eq!(
        identity_before.len(),
        1,
        "identity was not persisted before approval"
    );
    let identity = encryption::load_identity(&identity_before[0]).unwrap();
    let state = server.state.lock().unwrap();
    assert!(
        state.encryption.as_ref().unwrap().devices[0]["key"] == identity.public_key(),
        "approved grant was not bound to the submitted public identity"
    );
    drop(state);
    assert!(!f.client.enabled());
    assert!(!f.client.settings().upload_consent);
    assert!(!f.client.state.join("upload.credential").exists());
    assert!(f.client.enqueue(&f.bundle()).is_err());
    no_media(&server);
    // Normal explicit upload approval on the same endpoint; no synthetic worker
    // credential shortcut in this positive round trip.
    f.client.login(&server.url, "upload", true, false).unwrap();
    let bundle = f.root.join("test.apparent");
    let id = f.client.enqueue(&bundle).unwrap();
    f.client.process_once().unwrap();
    assert_ready(&f);
    let out = f.root.join("login-roundtrip.apparent");
    f.client.fetch(&id, &out, 0).unwrap();
    assert_bundle(&bundle, &out);
}

#[test]
fn encrypted_settings_approval_preserves_endpoint_enrollment_and_roundtrip() {
    let f = Fixture::new();
    let server = encryption_server("");
    encrypted(&f);
    for scope in ["read", "upload"] {
        let presented = std::cell::Cell::new(false);
        f.client
            .login_with_approval(
                &server.url,
                scope,
                scope == "upload",
                &|| true,
                &|approval| {
                    assert!(
                        !presented.replace(true),
                        "approval was presented more than once"
                    );
                    assert_eq!(approval.user_code, "TEST-CODE");
                    assert_eq!(
                        approval.verification_uri,
                        format!("{}/hosted/device?code=TEST-CODE", server.url)
                    );
                    let state = server.state.lock().unwrap();
                    let registry = state.encryption.as_ref().unwrap();
                    assert!(
                        registry.pending_key.is_some(),
                        "public endpoint key was not submitted before presentation"
                    );
                    assert!(!f.client.state.join(format!("{scope}.credential")).exists());
                },
            )
            .unwrap();
        assert!(presented.get());
        let credential = f.client.stored_credential(scope).unwrap();
        let identity_id = credential.encryption_identity.as_ref().unwrap();
        let identity =
            encryption::load_identity(&f.client.state.join(format!("{identity_id}.identity")))
                .unwrap();
        let state = server.state.lock().unwrap();
        assert!(
            state
                .encryption
                .as_ref()
                .unwrap()
                .devices
                .iter()
                .any(|peer| peer["scope"] == scope && peer["key"] == identity.public_key()),
            "Settings approval lost the immutable endpoint key binding"
        );
        drop(state);
        if scope == "read" {
            assert!(!f.client.enabled());
            assert!(!f.client.settings_snapshot().unwrap().uploads_enabled);
            no_media(&server);
        }
    }
    assert!(f.client.settings_snapshot().unwrap().uploads_enabled);
    let bundle = f.bundle();
    let id = f.client.enqueue(&bundle).unwrap();
    f.client.process_once().unwrap();
    assert_ready(&f);
    let out = f.root.join("settings-encrypted-roundtrip.apparent");
    f.client.fetch(&id, &out, 0).unwrap();
    assert_bundle(&bundle, &out);
}

#[test]
fn encrypted_settings_cancelled_approval_never_publishes_credentials_or_media() {
    let f = Fixture::new();
    let server = encryption_server("");
    encrypted(&f);
    let allowed = std::cell::Cell::new(true);
    let result =
        f.client
            .login_with_approval(&server.url, "upload", true, &|| allowed.get(), &|_| {
                assert!(server
                    .state
                    .lock()
                    .unwrap()
                    .encryption
                    .as_ref()
                    .unwrap()
                    .pending_key
                    .is_some());
                allowed.set(false);
            });
    assert_eq!(result, Err(crate::settings::SafeError::Cancelled));
    assert!(!f.client.state.join("upload.credential").exists());
    assert!(!f.client.settings().upload_consent);
    assert!(server
        .state
        .lock()
        .unwrap()
        .encryption
        .as_ref()
        .unwrap()
        .devices
        .is_empty());
    assert_eq!(server.state.lock().unwrap().polls, 0);
    no_media(&server);
}

#[test]
fn encrypted_roundtrip_has_only_opaque_wire_assets_and_no_plaintext_metadata() {
    let f = Fixture::new();
    let server = encryption_server("");
    let (bundle, id) = upload(&f, &server);
    let stage = Scratch::new(&f.client.state).unwrap();
    let original = snapshot(&bundle, &stage.0).unwrap();
    // Exercise the real producer, not a hand-built private crypto stage. The
    // directory creation mode must constrain inherited ACL masks too; umask is
    // intentionally not changed process-wide by this test.
    for name in ["frames", "frames-full"] {
        assert_eq!(
            fs::metadata(stage.0.join(name)).unwrap().mode() & 0o777,
            0o700
        );
    }
    {
        let state = server.state.lock().unwrap();
        assert_eq!(state.format.as_deref(), Some("apparent-encrypted-v1"));
        assert!(state.data.contains_key("envelope.json"));
        assert!(state.data.keys().any(|p| p.starts_with("sealed/part-")));
        let captured = &state.encryption.as_ref().unwrap().captured;
        for (_, bytes) in captured {
            let visible = String::from_utf8_lossy(bytes);
            for forbidden in [
                "synthetic summary",
                "frames/frame-01.png",
                "recording.mov",
                "summary.md",
            ] {
                assert!(
                    !visible.contains(forbidden),
                    "plaintext appeared in captured wire body"
                );
            }
            for asset in &original {
                assert!(
                    !visible.contains(&asset.sha256),
                    "original content digest appeared on wire"
                );
            }
        }
    }
    let out = f.root.join("roundtrip.apparent");
    f.client.fetch(&id, &out, 0).unwrap();
    assert_bundle(&bundle, &out);
    assert!(
        f.client.fetch(&id, &out, 0).is_err(),
        "fetch overwrote existing output"
    );
}

#[test]
fn encrypted_waits_for_reader_without_reservation_then_disable_wins() {
    let f = Fixture::new();
    let server = encryption_server("");
    pair(&f, &server);
    server
        .state
        .lock()
        .unwrap()
        .encryption
        .as_mut()
        .unwrap()
        .devices
        .retain(|p| p["scope"] != "read");
    f.client.enqueue(&f.bundle()).unwrap();
    f.client.process_once().unwrap();
    let waiting = job(&f);
    assert_eq!(waiting.status, "pending");
    assert_eq!(
        waiting.error.as_deref(),
        Some("encryption_pending_recipient")
    );
    assert_eq!(waiting.attempts, 0);
    assert!(server.state.lock().unwrap().capture_id.is_empty());
    no_media(&server);
    f.client.disable(false).unwrap();
    f.client.process_once().unwrap();
    assert_eq!(job(&f).status, "cancelled");
    no_media(&server);
}

#[test]
fn encrypted_restart_reuses_committed_ciphertext_despite_original_mutation() {
    let f = Fixture::new();
    let server = encryption_server("fail-file");
    pair(&f, &server);
    let bundle = f.bundle();
    let original_summary = fs::read(bundle.join("summary.md")).unwrap();
    let id = f.client.enqueue(&bundle).unwrap();
    f.client.process_once().unwrap();
    let first = job(&f);
    assert!(!first.files.is_empty() && first.encryption.is_some());
    assert!(!server.state.lock().unwrap().ready);
    let spool = f.client.state.join(format!("{id}.sealed"));
    let first_bytes: Vec<_> = first
        .files
        .iter()
        .map(|asset| fs::read(spool.join(&asset.path)).unwrap())
        .collect();
    fs::write(bundle.join("summary.md"), b"changed after interruption").unwrap();
    server.state.lock().unwrap().mode.clear();
    let restarted = Client::new(f.client.provider.clone());
    restarted.retry().unwrap();
    restarted.process_once().unwrap();
    let completed = job(&f);
    assert_eq!(completed.id, id);
    assert!(same_manifest(&completed.files, &first.files));
    assert_eq!(completed.status, "ready");
    for (asset, bytes) in first.files.iter().zip(first_bytes) {
        assert_eq!(server.state.lock().unwrap().data[&asset.path], bytes);
    }
    let out = f.root.join("restarted.apparent");
    restarted.fetch(&id, &out, 0).unwrap();
    assert_eq!(fs::read(out.join("summary.md")).unwrap(), original_summary);
}

#[test]
fn encrypted_missing_key_lost_spool_and_toggle_cannot_fall_back_to_plaintext() {
    for fault in ["missing-key", "lost-spool", "toggle"] {
        let f = Fixture::new();
        let server = encryption_server("fail-file");
        pair(&f, &server);
        f.client.enqueue(&f.bundle()).unwrap();
        if fault != "missing-key" {
            f.client.process_once().unwrap();
        }
        match fault {
            "missing-key" => {
                let credential = f.client.credential("upload", &server.url).unwrap();
                fs::remove_file(f.client.state.join(format!(
                    "{}.identity",
                    credential.encryption_identity.unwrap()
                )))
                .unwrap();
            }
            "lost-spool" => {
                fs::remove_dir_all(f.client.state.join(format!("{}.sealed", job(&f).id))).unwrap()
            }
            _ => f
                .client
                .provider
                .set_experiment(HOSTED_ENCRYPTION, false)
                .unwrap(),
        }
        let requests = server.state.lock().unwrap().requests;
        let routes = server.state.lock().unwrap().request_routes.len();
        server.state.lock().unwrap().mode.clear();
        f.client.retry().unwrap();
        f.client.process_once().unwrap();
        assert_ne!(job(&f).status, "ready", "{fault} unexpectedly uploaded");
        let state = server.state.lock().unwrap();
        assert!(state.data.is_empty() && !state.ready);
        // The intentional interruption attempted one ciphertext PUT before the
        // fault. No new send/reservation/completion is allowed after key/spool
        // loss or toggling; previous failed attempts are not plaintext fallback.
        assert!(
            !state.request_routes[routes..]
                .iter()
                .any(|route| route.starts_with("PUT ") || route.starts_with("POST /v1/captures/"))
        );
        drop(state);
        if fault == "toggle" {
            assert_eq!(server.state.lock().unwrap().requests, requests);
        }
    }
}

#[test]
fn encrypted_disable_during_real_snapshot_cancels_before_any_media() {
    let f = Fixture::new();
    let server = encryption_server("");
    pair(&f, &server);
    f.client.enqueue(&f.bundle()).unwrap();
    let client = f.client.clone();
    let triggered = Arc::new(AtomicBool::new(false));
    let observed = triggered.clone();
    set_hook(move |point, _| {
        if point == "snapshot_open" && !observed.swap(true, Ordering::SeqCst) {
            client.disable(false).unwrap();
        }
    });
    let result = f.client.process_once();
    clear_hook();
    result.unwrap();
    assert!(triggered.load(Ordering::SeqCst));
    assert_eq!(job(&f).status, "cancelled");
    no_media(&server);
}

#[test]
fn encrypted_peer_revoked_between_requests_stops_remaining_sends_and_completion() {
    let f = Fixture::new();
    let server = encryption_server("");
    pair(&f, &server);
    server
        .state
        .lock()
        .unwrap()
        .encryption
        .as_mut()
        .unwrap()
        .revoke_reader_after_put = true;
    f.client.enqueue(&f.bundle()).unwrap();
    f.client.process_once().unwrap();
    let state = server.state.lock().unwrap();
    assert_eq!(
        state
            .request_routes
            .iter()
            .filter(|r| r.starts_with("PUT "))
            .count(),
        1
    );
    assert_eq!(state.data.len(), 1);
    assert!(!state.ready);
    assert!(
        !state
            .request_routes
            .iter()
            .any(|r| r.ends_with("/complete"))
    );
    drop(state);
    assert_eq!(job(&f).status, "failed");
    assert_eq!(job(&f).error.as_deref(), Some("encryption_action_required"));
}

#[test]
fn encrypted_fetch_wrong_key_tampering_truncation_and_context_publish_nothing() {
    for fault in ["wrong-key", "tamper", "truncate", "context"] {
        let f = Fixture::new();
        let server = encryption_server("");
        let (_, id) = upload(&f, &server);
        match fault {
            "wrong-key" => {
                let mut credential = f.client.credential("read", &server.url).unwrap();
                let replacement = random_id().unwrap();
                let identity = encryption::load_or_create_identity(
                    &f.client.state.join(format!("{replacement}.identity")),
                )
                .unwrap();
                credential.encryption_identity = Some(replacement);
                atomic_json(&f.client.state.join("read.credential"), &credential).unwrap();
                for peer in &mut server
                    .state
                    .lock()
                    .unwrap()
                    .encryption
                    .as_mut()
                    .unwrap()
                    .devices
                {
                    if peer["scope"] == "read" {
                        peer["key"] = identity.public_key();
                    }
                }
            }
            "context" => {
                let mut state = server.state.lock().unwrap();
                let envelope = state.data.get_mut("envelope.json").unwrap();
                let mut value: Value = serde_json::from_slice(envelope).unwrap();
                value["signed"]["context"]["accountId"] = "b".repeat(64).into();
                *envelope = serde_json::to_vec(&value).unwrap();
            }
            _ => {
                let mut state = server.state.lock().unwrap();
                let path = state
                    .data
                    .keys()
                    .find(|p| p.starts_with("sealed/"))
                    .unwrap()
                    .clone();
                let bytes = state.data.get_mut(&path).unwrap();
                if fault == "truncate" {
                    bytes.truncate(bytes.len() / 2);
                } else {
                    let last = bytes.len() - 1;
                    bytes[last] ^= 1;
                }
            }
        }
        // Recompute only the unauthenticated storage hashes, so failures must
        // cross transport verification and reach the cryptographic/context guard.
        {
            let mut state = server.state.lock().unwrap();
            for index in 0..state.files.len() {
                let path = state.files[index].path.clone();
                let bytes = state.data[&path].clone();
                let probe = f.root.join("opaque-probe");
                fs::write(&probe, &bytes).unwrap();
                state.files[index].size = bytes.len() as u64;
                state.files[index].sha256 = digest_file(&probe).unwrap();
            }
        }
        fetch_refused(
            &f,
            &id,
            fault,
            if fault == "context" {
                "context"
            } else {
                "encrypt"
            },
        );
    }
}

#[test]
fn encrypted_public_sharing_has_no_outbound_and_fetch_refuses_unknown_or_legacy_format() {
    let f = Fixture::new();
    let server = encryption_server("");
    let (_, id) = upload(&f, &server);
    let before = server.state.lock().unwrap().requests;
    let error = f
        .client
        .prepare_link(&f.root.join("test.apparent"), &|| true)
        .err()
        .unwrap()
        .to_string();
    assert!(error.contains("encrypted_sharing_unsupported"));
    assert_eq!(server.state.lock().unwrap().requests, before);
    for format in [Some("future-encryption-v9"), Some("plaintext-v1"), None] {
        server.state.lock().unwrap().format = format.map(str::to_owned);
        fetch_refused(
            &f,
            &id,
            "format",
            if format == Some("future-encryption-v9") {
                "unsupported_encryption_format"
            } else {
                "encryption_downgrade_refused"
            },
        );
    }
}

#[test]
fn explicit_public_copy_preserves_private_identity_and_reuses_only_public_intent() {
    let f = Fixture::new();
    let server = encryption_server("");
    let (bundle, private_id) = upload(&f, &server);
    let private_path = f.client.state.join(format!("{private_id}.job"));
    let private_before = fs::read(&private_path).unwrap();
    let sealed_before = server.state.lock().unwrap().data.clone();
    assert!(!sealed_before.is_empty());
    let prepared = f.client.prepare_public_link(&bundle, &|| true).unwrap();
    assert!(prepared.authorized());
    assert!(prepared.text.contains("not end-to-end encrypted"));
    assert_ne!(prepared.job.id, private_id);
    assert!(prepared.job.public_copy && prepared.job.shared && !prepared.job.encrypted);
    assert!(prepared.job.encryption.is_none());
    assert_eq!(fs::read(&private_path).unwrap(), private_before);
    assert_eq!(f.client.enqueue(&bundle).unwrap(), private_id);
    assert!(f.client.pointer(&bundle).unwrap().contains(&private_id));
    assert!(!f.client.pointer(&bundle).unwrap().contains(&prepared.job.id));
    assert!(server.state.lock().unwrap().data.is_empty(), "reservation must not upload media");
    let restarted = Client::new(provider(f.client.config.clone()));
    let again = restarted.prepare_public_link(&bundle, &|| true).unwrap();
    assert_eq!(again.job.id, prepared.job.id);
    assert_eq!(again.text, prepared.text);
    restarted.process_once().unwrap();
    let copy = restarted.jobs().unwrap().into_iter().find(|j| j.id == prepared.job.id).unwrap();
    assert_eq!(copy.status, "ready");
    assert_eq!(fs::read(&private_path).unwrap(), private_before);
    let state = server.state.lock().unwrap();
    assert_eq!(state.capture_id, prepared.job.id);
    assert!(state.format.is_none());
    assert_eq!(state.data["frames/frame-01.png"], fs::read(bundle.join("frames/frame-01.png")).unwrap());
    assert_eq!(state.data["apparent.json"], fs::read(bundle.join("apparent.json")).unwrap());
    assert!(!state.data.keys().any(|p| p.ends_with(".identity") || p.starts_with("sealed/")));
    drop(state);
    assert_eq!(restarted.sharing_progress(&bundle), "Link copied · ready");
    let requests = server.state.lock().unwrap().requests;
    assert!(restarted.prepare_public_link(&bundle, &|| false).is_err());
    assert_eq!(server.state.lock().unwrap().requests, requests);
    restarted.disable(false).unwrap();
    let disabled_requests = server.state.lock().unwrap().requests;
    assert!(!prepared.authorized());
    assert!(restarted.prepare_public_link(&bundle, &|| true).is_err());
    restarted.process_once().unwrap();
    assert_eq!(server.state.lock().unwrap().requests, disabled_requests);
}

#[test]
fn public_intent_cannot_resurrect_cancelled_jobs_or_relabel_private_jobs() {
    let f = Fixture::new();
    let server = Server::new("");
    f.authorize(&server.url);
    encrypted(&f);
    let bundle = f.bundle();
    // Model a process exit after durable admission, before capability reservation.
    let staged_id = f.client.enqueue_with_intent(&bundle, false, true).unwrap();
    f.client.process_once().unwrap();
    f.client.retry().unwrap();
    f.client.process_once().unwrap();
    assert_eq!(server.state.lock().unwrap().requests, 0);
    let link = f.client.prepare_public_link(&bundle, &|| true).unwrap();
    assert_eq!(link.job.id, staged_id);
    let private_id = f.client.enqueue(&bundle).unwrap();
    assert_ne!(private_id, staged_id);
    let mut copy = link.job.clone();
    copy.public_copy = false;
    assert!(!f.client.active(&copy));
    let mut private = f.client.jobs().unwrap().into_iter().find(|j| j.id == private_id).unwrap();
    private.public_copy = true;
    private.shared = true;
    private.encrypted = false;
    assert!(!f.client.active(&private));
    copy = link.job.clone();
    copy.status = "cancelled".into();
    f.client.save_job(&copy).unwrap();
    assert!(!link.authorized());
    let fresh = f.client.prepare_public_link(&bundle, &|| true).unwrap();
    assert_ne!(fresh.job.id, link.job.id);
    assert_eq!(f.client.enqueue(&bundle).unwrap(), private_id);
    assert_eq!(f.client.jobs().unwrap().into_iter().find(|j| j.id == private_id).unwrap().status, "pending");
}
