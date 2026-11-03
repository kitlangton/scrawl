//! Production Settings presentation against the real synthetic HTTP transport.
use super::*;
use crate::settings::{Health, SafeError};

#[test]
fn settings_mixed_grants_follow_upload_precedence() {
    let f = Fixture::new();
    let server = Server::new("");
    f.authorize(&server.url);
    let mut read = f.client.stored_credential("read").unwrap();
    read.expires_at = Some(now().saturating_sub(1));
    atomic_json(&f.client.state.join("read.credential"), &read).unwrap();
    let snapshot = f.client.settings_snapshot().unwrap();
    assert!(snapshot.connected && snapshot.uploads_enabled);
    assert!(!snapshot.reauthorization_required);
    let mut upload = f.client.stored_credential("upload").unwrap();
    upload.auth_required = true;
    atomic_json(&f.client.state.join("upload.credential"), &upload).unwrap();
    read.expires_at = None;
    atomic_json(&f.client.state.join("read.credential"), &read).unwrap();
    let snapshot = f.client.settings_snapshot().unwrap();
    assert!(snapshot.connected && !snapshot.uploads_enabled);
    assert!(snapshot.reauthorization_required);
}

#[test]
fn settings_newer_logout_intent_is_not_removed_by_older_disable() {
    let f = Fixture::new();
    let server = Server::new("");
    f.authorize(&server.url);
    f.client.enqueue(&f.bundle()).unwrap();
    let client = f.client.clone();
    set_hook(move |point, _| {
        if point == "disable_intents_snapshot" {
            // The older transaction owns queue.lock. A newer logout times out
            // but its independently published intent must survive older cleanup.
            assert_eq!(client.settings_disable(true), Err(SafeError::LocalState));
        }
    });
    let result = f.client.settings_disable(false);
    clear_hook();
    result.unwrap();
    assert!(f.client.revocation_pending());
    assert!(f.client.stored_credential("upload").is_ok());
    let fresh = Client::new(provider(f.client.config.clone()));
    assert_eq!(fresh.settings_snapshot(), Err(SafeError::LocalState));
    // Retrying Disable must settle the retained stronger logout, not downgrade it.
    fresh.settings_disable(false).unwrap();
    assert!(!fresh.revocation_pending());
    assert!(fresh.stored_credential("upload").is_err());
    assert!(fresh.stored_credential("read").is_err());
    assert_eq!(fresh.jobs().unwrap()[0].status, "cancelled");
}

#[test]
fn settings_unreadable_intent_and_failed_publication_are_local_errors() {
    let f = Fixture::new();
    private_dir(&f.client.state).unwrap();
    let path = f.client.state.join("revocation-synthetic.intent");
    std::os::unix::fs::symlink(f.root.join("missing"), &path).unwrap();
    assert!(f.client.revocation_pending());
    assert!(!f.client.enabled());
    assert_eq!(f.client.settings_snapshot(), Err(SafeError::LocalState));
    assert_eq!(f.client.settings_disable(false), Err(SafeError::LocalState));
    assert!(fs::symlink_metadata(path).unwrap().file_type().is_symlink());
    let broken = Fixture::new();
    fs::write(&broken.client.state, b"not a directory").unwrap();
    assert_eq!(
        broken.client.settings_disable(false),
        Err(SafeError::LocalState)
    );
}

#[test]
fn settings_snapshot_is_fresh_local_and_read_does_not_enable_upload() {
    let f = Fixture::new();
    let server = Server::new("");
    assert_eq!(
        f.client.settings_snapshot().unwrap(),
        crate::settings::Snapshot {
            server: None,
            connected: false,
            uploads_enabled: false,
            reauthorization_required: false, renewal_required: false,
        }
    );
    f.authorize(&server.url);
    let (client, provider) = mutable_client(&f);
    assert!(client.settings_snapshot().unwrap().uploads_enabled);
    provider.sync.store(false, Ordering::SeqCst);
    assert!(!client.settings_snapshot().unwrap().uploads_enabled);
    provider.sync.store(true, Ordering::SeqCst);
    fs::remove_file(client.state.join("upload.credential")).unwrap();
    let read = client.settings_snapshot().unwrap();
    assert!(read.connected);
    assert!(!read.uploads_enabled);
    let mut credential = client.stored_credential("read").unwrap();
    credential.expires_at = Some(now().saturating_sub(1));
    atomic_json(&client.state.join("read.credential"), &credential).unwrap();
    let expired = client.settings_snapshot().unwrap();
    assert!(expired.reauthorization_required);
    assert!(!expired.connected && !expired.uploads_enabled);
    credential.expires_at = None;
    credential.auth_required = true;
    atomic_json(&client.state.join("read.credential"), &credential).unwrap();
    assert!(client.settings_snapshot().unwrap().reauthorization_required);
    atomic_json(
        &client.state.join("read.credential"),
        &json!({"token":"synthetic-secret"}),
    )
    .unwrap();
    assert_eq!(client.settings_snapshot(), Err(SafeError::LocalState));
    fs::remove_file(client.state.join("read.credential")).unwrap();
    atomic_json(
        &client.config,
        &json!({"cloud":{"server":"https://user:secret@example.com"}}),
    )
    .unwrap();
    assert_eq!(client.settings_snapshot(), Err(SafeError::InvalidServer));
    fs::write(&client.config, b"{invalid synthetic secret").unwrap();
    assert_eq!(client.settings_snapshot(), Err(SafeError::LocalState));
    assert_eq!(server.state.lock().unwrap().requests, 0);
}

#[test]
fn settings_no_consent_or_cancelled_start_has_no_network_or_publication() {
    let f = Fixture::new();
    let server = Server::new("upload");
    assert!(
        f.client
            .login_with_approval(&server.url, "upload", false, &|| true, &|_| panic!(
                "no consent"
            ))
            .is_err()
    );
    assert_eq!(
        f.client
            .login_with_approval(&server.url, "upload", true, &|| false, &|_| panic!(
                "cancelled"
            )),
        Err(SafeError::Cancelled)
    );
    assert!(!f.client.state.join("upload.credential").exists());
    assert!(!f.client.settings().enabled && !f.client.settings().upload_consent);
    assert_eq!(server.state.lock().unwrap().requests, 0);
}

#[test]
fn settings_queue_wait_is_bounded() {
    let f = Fixture::new();
    private_dir(&f.client.state).unwrap();
    let lock = Lock::acquire(&f.client.state.join("queue.lock"), true).unwrap();
    let start = Instant::now();
    assert_eq!(f.client.settings_disable(false), Err(SafeError::LocalState));
    assert!(start.elapsed() >= Duration::from_secs(3));
    assert!(start.elapsed() < Duration::from_secs(5));
    drop(lock);
    f.client.settings_disable(false).unwrap();
}

#[test]
fn settings_public_callback_and_read_login() {
    let f = Fixture::new();
    let server = Server::new("");
    let called = AtomicBool::new(false);
    f.client
        .login_with_approval(&server.url, "read", false, &|| true, &|approval| {
            assert_eq!(
                approval.verification_uri,
                format!("{}/hosted/device?code=TEST-CODE", server.url)
            );
            assert_eq!(approval.user_code, "TEST-CODE");
            assert!(!approval.verification_uri.contains("secret"));
            called.store(true, Ordering::SeqCst);
            // Neither state lock spans presentation/network supervision.
            let _queue = Lock::acquire(&f.client.state.join("queue.lock"), true).unwrap();
            set_value(&f.client.config, "unrelated", json!(true)).unwrap();
        })
        .unwrap();
    assert!(called.load(Ordering::SeqCst));
    assert!(f.client.settings_snapshot().unwrap().connected);
    assert!(!f.client.settings_snapshot().unwrap().uploads_enabled);
    assert!(!f.client.state.join("upload.credential").exists());
}

#[test]
fn settings_rejects_private_or_unsafe_approval_before_callback() {
    for mode in [
        "approval-userinfo",
        "approval-http",
        "approval-secret",
        "approval-encoded-secret",
        "approval-control",
        "approval-secret-code",
        "approval-secret-substring",
    ] {
        let f = Fixture::new();
        let server = Server::new(mode);
        assert!(
            f.client
                .login_with_approval(&server.url, "upload", true, &|| true, &|_| panic!(
                    "unsafe approval exposed"
                ))
                .is_err()
        );
        assert!(!f.client.state.join("upload.credential").exists());
        assert!(!f.client.settings().upload_consent);
        assert_eq!(server.state.lock().unwrap().polls, 0);
    }
}

#[test]
fn settings_cancel_at_public_approval_publishes_nothing() {
    let f = Fixture::new();
    let server = Server::new("upload");
    let active = AtomicBool::new(true);
    assert_eq!(
        f.client.login_with_approval(
            &server.url,
            "upload",
            true,
            &|| active.load(Ordering::SeqCst),
            &|_| active.store(false, Ordering::SeqCst)
        ),
        Err(SafeError::Cancelled)
    );
    assert!(!f.client.state.join("upload.credential").exists());
    assert!(!f.client.settings().upload_consent);
    assert!(!f.client.settings().enabled);
    assert_eq!(server.state.lock().unwrap().polls, 0);
}

#[test]
fn settings_cancel_at_approved_transport_and_disable_incarnation() {
    let f = Fixture::new();
    let server = Server::new("upload");
    let gate = Arc::new(MediaGate::default());
    server.state.lock().unwrap().media_gate = Some(gate.clone());
    let client = f.client.clone();
    let url = server.url.clone();
    let worker = std::thread::spawn(move || {
        client.login_with_approval(&url, "upload", true, &|| true, &|_| ())
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !gate.started.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(gate.started.load(Ordering::SeqCst));
    // A different controller/CLI disable durably invalidates the login marker.
    f.client.settings_disable(false).unwrap();
    gate.release.store(true, Ordering::SeqCst);
    assert_eq!(worker.join().unwrap(), Err(SafeError::Cancelled));
    assert!(!f.client.state.join("upload.credential").exists());
    assert!(!f.client.settings().enabled);
    assert!(!f.client.settings().upload_consent);
}

#[test]
fn settings_authenticated_health_and_cancellation() {
    let f = Fixture::new();
    let server = Server::new("");
    f.authorize(&server.url);
    assert_eq!(f.client.settings_health(&|| false), Health::Offline);
    assert_eq!(server.state.lock().unwrap().requests, 0);
    assert_eq!(f.client.settings_health(&|| true), Health::Online);
    assert_eq!(
        server.state.lock().unwrap().request_routes,
        ["GET /v1/account"]
    );
    server.state.lock().unwrap().mode = "auth-required".into();
    assert_eq!(f.client.settings_health(&|| true), Health::AuthRequired);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let offline = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    f.authorize(&offline);
    assert_eq!(f.client.settings_health(&|| true), Health::Offline);
}

#[test]
fn settings_silent_subprocess_helper() {
    let Ok(root) = std::env::var("APPARENT_SETTINGS_FIXTURE") else {
        return;
    };
    struct NoBrowser(FixtureConfig);
    impl ConfigProvider for NoBrowser {
        fn config_path(&self) -> &Path {
            self.0.config_path()
        }
        fn cloud(&self) -> CloudConfig {
            self.0.cloud()
        }
        fn experiment(&self, name: &str) -> bool {
            self.0.experiment(name)
        }
        fn set_cloud(&self, value: &CloudConfig) -> Result<()> {
            self.0.set_cloud(value)
        }
        fn set_experiment(&self, name: &str, enabled: bool) -> Result<()> {
            self.0.set_experiment(name, enabled)
        }
        fn open_browser(&self, _: &Path) {
            panic!("Settings opened browser");
        }
    }
    let client = Client::new(Arc::new(NoBrowser(FixtureConfig(
        PathBuf::from(root).join("config.json"),
    ))));
    let server = std::env::var("APPARENT_SETTINGS_SERVER").unwrap();
    if std::env::var_os("APPARENT_SETTINGS_CLI").is_some() {
        client.login(&server, "read", false, false).unwrap();
    } else {
        client
            .login_with_approval(&server, "read", false, &|| true, &|approval| {
                assert_eq!(approval.user_code, "TEST-CODE");
            })
            .unwrap();
    }
}

#[test]
fn settings_is_silent_and_cli_output_is_compatible() {
    for cli in [false, true] {
        let f = Fixture::new();
        let server = Server::new("");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "test_support::settings_tests::settings_silent_subprocess_helper",
                "--nocapture",
            ])
            .env("APPARENT_SETTINGS_FIXTURE", &f.root)
            .env("APPARENT_SETTINGS_SERVER", &server.url);
        if cli {
            command.env("APPARENT_SETTINGS_CLI", "1");
        } else {
            command.env_remove("APPARENT_SETTINGS_CLI");
        }
        let output = command.output().unwrap();
        assert!(output.status.success(), "synthetic child failed");
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(!text.contains("secret"));
        assert!(!String::from_utf8(output.stderr).unwrap().contains("secret"));
        if cli {
            assert!(
                text.contains("Open ")
                    && text.contains("Verification code: TEST-CODE")
                    && text.contains("Requested scope: read")
                    && text.contains("Signed in for read; tokens remain private on this machine.")
            );
        } else {
            for private in ["TEST-CODE", "Open ", "Signed in", &server.url] {
                assert!(!text.contains(private));
            }
        }
    }
}
