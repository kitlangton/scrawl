use super::*;

#[test]
fn renewal_pending_revocation_blocks_fresh_read_and_upload_admission() {
    let f = Fixture::new();
    let server = RenewalServer::new("rotate");
    f.authorize(&server.url);
    let bundle = f.bundle();
    expire_fixture(&f, &server.url, "upload");
    expire_fixture(&f, &server.url, "read");
    atomic_json(&f.client.state.join("revocation-synthetic.intent"), &true).unwrap();
    let fresh = Client::new(provider(f.client.config.clone()));
    assert!(fresh.enqueue(&bundle).is_err());
    assert!(fresh.prepare_link(&bundle, &|| true).is_err());
    assert!(fresh.shares().is_err());
    assert!(fresh.status(true).is_err());
    fresh.process_once().unwrap();
    assert_eq!(server.refreshes.load(Ordering::SeqCst), 0);
    assert_eq!(server.requests.load(Ordering::SeqCst), 0);
}

struct RenewalServer {
    url: String,
    refreshes: Arc<std::sync::atomic::AtomicUsize>,
    requests: Arc<std::sync::atomic::AtomicUsize>,
    gate: Arc<MediaGate>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl RenewalServer {
    fn new(mode: &str) -> Self {
        Self::with_accept_mode(mode, false)
    }
    fn with_accept_mode(mode: &str, nonblocking_accept: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let refreshes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let gate = Arc::new(MediaGate::default());
        let (count, calls, done, barrier, mode) = (
            refreshes.clone(),
            requests.clone(),
            stop.clone(),
            gate.clone(),
            mode.to_owned(),
        );
        let thread = std::thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(Duration::from_millis(5));
                    continue;
                };
                if nonblocking_accept {
                    stream.set_nonblocking(true).unwrap();
                }
                // Match the general fixture: accept's flag inheritance differs
                // by platform, but these bounded request reads must block.
                stream.set_nonblocking(false).unwrap();
                barrier.transport_paused.store(true, Ordering::SeqCst);
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut bytes = Vec::new();
                let end = loop {
                    let mut buf = [0; 4096];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        break None;
                    }
                    bytes.extend_from_slice(&buf[..n]);
                    if let Some(i) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
                        break Some(i + 4);
                    }
                };
                let Some(end) = end else {
                    continue;
                };
                let head = String::from_utf8(bytes[..end].to_vec()).unwrap();
                let length = head
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("content-length:")
                            .and_then(|s| s.trim().parse::<usize>().ok())
                    })
                    .unwrap_or(0);
                while bytes.len() < end + length {
                    let mut buf = [0; 4096];
                    let n = stream.read(&mut buf).unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    bytes.extend_from_slice(&buf[..n]);
                }
                calls.fetch_add(1, Ordering::SeqCst);
                let (status, value) = if head.starts_with("POST /v1/token/refresh ") {
                    count.fetch_add(1, Ordering::SeqCst);
                    let body: Value = serde_json::from_slice(&bytes[end..]).unwrap();
                    let refresh = if mode.starts_with("read") {
                        "synthetic-refresh-read"
                    } else {
                        "synthetic-refresh-old"
                    };
                    assert_eq!(body, json!({"refreshToken":refresh}));
                    barrier.started.store(true, Ordering::SeqCst);
                    if mode == "delayed" {
                        let deadline = Instant::now() + Duration::from_secs(8);
                        while !barrier.release.load(Ordering::SeqCst)
                            && !done.load(Ordering::SeqCst)
                            && Instant::now() < deadline
                        {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                    }
                    match mode.as_str() {
                        "transient" => (503, json!({"error":{"code":"internal_error"}})),
                        "rate" => (429, json!({"error":{"code":"rate_limited"}})),
                        "network" => {
                            continue;
                        }
                        "malformed" => {
                            (200, json!({"unexpected":"synthetic-sensitive-remote-text"}))
                        }
                        "lifetime" => {
                            let mut v = reply("upload", "server-grant:not-local-hex");
                            v["expiresIn"] = json!(0);
                            (200, v)
                        }
                        "revoked" => (401, json!({"error":{"code":"reauthorization_required"}})),
                        "scope" => (200, reply("read", "server-grant:not-local-hex")),
                        "read" | "read-delayed" => (200, reply("read", "grant-read")),
                        "grant" => (200, reply("upload", "different-grant")),
                        _ => (200, reply("upload", "server-grant:not-local-hex")),
                    }
                } else if head.starts_with("POST /v1/token/revoke ") {
                    (200, json!({"ok":true}))
                } else if head.contains("synthetic-access-new") {
                    (200, json!({"ok":true}))
                } else {
                    (401, json!({"error":{"code":"unauthorized"}}))
                };
                let body = value.to_string();
                let response = format!(
                    "HTTP/1.1 {status} Result\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        Self {
            url,
            refreshes,
            requests,
            gate,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for RenewalServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.gate.release.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}
#[test]
fn hosted_renewal_fixture_reads_fragmented_nonblocking_accept() {
    for split_body in [false, true] {
        let server = RenewalServer::with_accept_mode("revoked", true);
        let mut stream = TcpStream::connect(server.url.strip_prefix("http://").unwrap()).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        while !server.gate.transport_paused.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(server.gate.transport_paused.load(Ordering::SeqCst));
        let body = json!({"refreshToken":"synthetic-refresh-old"}).to_string();
        let head = format!(
            "POST /v1/token/refresh HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            body.len()
        );
        let request = format!("{head}{body}");
        let split = if split_body { head.len() + 5 } else { 12 };
        let first = stream.write_all(&request.as_bytes()[..split]);
        std::thread::sleep(Duration::from_millis(50));
        let last = stream.write_all(&request.as_bytes()[split..]);
        let _ = stream.shutdown(std::net::Shutdown::Write);
        let mut response = String::new();
        let read = stream.read_to_string(&mut response);
        let body = json!({"error":{"code":"reauthorization_required"}}).to_string();
        let expected = format!(
            "HTTP/1.1 401 Result\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        assert_eq!(
            response, expected,
            "split_body={split_body}; first={first:?}; last={last:?}; read={read:?}"
        );
        assert!(first.is_ok() && last.is_ok() && read.is_ok());
        assert_eq!(server.refreshes.load(Ordering::SeqCst), 1);
        assert_eq!(server.requests.load(Ordering::SeqCst), 1);
    }
}
fn reply(scope: &str, grant: &str) -> Value {
    json!({"accessToken":"synthetic-access-new","refreshToken":"synthetic-refresh-new","scope":scope,"expiresIn":3600,"refreshExpiresIn":86400,"grantId":grant,"accountInitial":"M"})
}
fn renewable(f: &Fixture, server: &str) {
    f.authorize(server);
    atomic_json(&f.client.state.join("upload.credential"), &json!({"server":server,"scope":"upload","token":"synthetic-access-old","generation":"a".repeat(32),"refreshToken":"synthetic-refresh-old","expiresAt":now()-1,"refreshExpiresAt":now()+86400,"grantId":"server-grant:not-local-hex"})).unwrap();
}
fn renewable_read(f: &Fixture, server: &str) {
    f.authorize(server);
    atomic_json(&f.client.state.join("read.credential"), &json!({"server":server,"scope":"read","token":"synthetic-read-old","generation":"a".repeat(32),"refreshToken":"synthetic-refresh-read","expiresAt":now()-1,"refreshExpiresAt":now()+86400,"grantId":"grant-read"})).unwrap();
}

#[test]
fn recipient_rejection_marks_only_the_exact_current_read_token_without_network() {
    for mismatch in ["none", "token", "generation", "epoch", "revocation"] {
        let f = Fixture::new();
        let server = RenewalServer::new("read");
        renewable_read(&f, &server.url);
        let read = f.client.credential("read", &server.url).unwrap();
        let before = read_private(&f.client.state.join("read.credential"), 32768).unwrap();
        let upload = fs::read(f.client.state.join("upload.credential")).unwrap();
        let config = fs::read(&f.client.config).unwrap();
        if mismatch == "epoch" { atomic_json(&f.client.state.join("cancellation"), &"new").unwrap(); }
        if mismatch == "revocation" { atomic_json(&f.client.state.join("revocation-test.intent"), &true).unwrap(); }
        let result = f.client.reject_recipient_access(&server.url,
            if mismatch == "generation" { "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" } else { &read.generation },
            if mismatch == "token" { "previous-token" } else { &read.token }, None, &|| true);
        if mismatch == "none" {
            result.unwrap();
            let rejected = f.client.credential("read", &server.url).unwrap();
            assert!(rejected.auth_required); assert_eq!(rejected.token, read.token);
        } else {
            if mismatch != "token" { assert!(result.is_err()); } else { result.unwrap(); }
            assert_eq!(read_private(&f.client.state.join("read.credential"), 32768).unwrap(), before);
        }
        assert_eq!(server.requests.load(Ordering::SeqCst), 0);
        assert_eq!(fs::read(f.client.state.join("upload.credential")).unwrap(), upload);
        assert_eq!(fs::read(&f.client.config).unwrap(), config);
    }
}

#[test]
fn recipient_renewal_due_and_forced_preserve_upload_authority_and_config() {
    for force in [false, true] {
        let f = Fixture::new();
        let server = RenewalServer::new("read");
        renewable_read(&f, &server.url);
        if force {
            let mut read = f.client.credential("read", &server.url).unwrap();
            read.expires_at = Some(now() + 3600);
            atomic_json(&f.client.state.join("read.credential"), &read).unwrap();
        }
        let upload_before = read_private(&f.client.state.join("upload.credential"), 32768).unwrap();
        let config_before = fs::read(&f.client.config).unwrap();
        let read = f.client.credential("read", &server.url).unwrap();
        f.client
            .renew_recipient_access(
                &server.url,
                &read.generation,
                &read.token,
                None,
                force,
                &|| true,
            )
            .unwrap();
        let renewed = f.client.credential("read", &server.url).unwrap();
        assert_eq!(renewed.token, "synthetic-access-new");
        assert_eq!(renewed.generation, "a".repeat(32));
        assert_eq!(
            read_private(&f.client.state.join("upload.credential"), 32768).unwrap(),
            upload_before
        );
        assert_eq!(fs::read(&f.client.config).unwrap(), config_before);
        assert_eq!(server.refreshes.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn recipient_renewal_skips_changed_token_and_rejects_stale_or_revoked_context() {
    // A concurrent writer already rotated the access token: do not POST again.
    let f = Fixture::new();
    let server = RenewalServer::new("read");
    renewable_read(&f, &server.url);
    let mut read = f.client.credential("read", &server.url).unwrap();
    let old_token = read.token.clone();
    read.token = "synthetic-access-newer".into();
    atomic_json(&f.client.state.join("read.credential"), &read).unwrap();
    f.client
        .renew_recipient_access(
            &server.url,
            &read.generation,
            &old_token,
            None,
            true,
            &|| true,
        )
        .unwrap();
    assert_eq!(server.refreshes.load(Ordering::SeqCst), 0);

    for invalid in ["generation", "epoch", "revocation", "auth"] {
        let f = Fixture::new();
        let server = RenewalServer::new("read");
        renewable_read(&f, &server.url);
        let read = f.client.credential("read", &server.url).unwrap();
        let before = read_private(&f.client.state.join("read.credential"), 32768).unwrap();
        let config_before = fs::read(&f.client.config).unwrap();
        let mut generation = read.generation.clone();
        let mut epoch = None;
        if invalid == "generation" {
            generation = "b".repeat(32);
        }
        if invalid == "epoch" {
            atomic_json(&f.client.state.join("cancellation"), &"new-epoch").unwrap();
            epoch = Some("old-epoch");
        }
        if invalid == "revocation" {
            atomic_json(&f.client.state.join("revocation-synthetic.intent"), &true).unwrap();
        }
        if invalid == "auth" {
            let mut credential = read.clone();
            credential.auth_required = true;
            atomic_json(&f.client.state.join("read.credential"), &credential).unwrap();
        }
        let result = f.client.renew_recipient_access(
            &server.url,
            &generation,
            &read.token,
            epoch,
            true,
            &|| true,
        );
        assert!(result.is_err(), "invalid={invalid}");
        assert_eq!(
            server.refreshes.load(Ordering::SeqCst),
            0,
            "invalid={invalid}"
        );
        if invalid != "auth" {
            assert_eq!(
                read_private(&f.client.state.join("read.credential"), 32768).unwrap(),
                before
            );
        }
        assert_eq!(fs::read(&f.client.config).unwrap(), config_before);
    }
}

#[test]
fn recipient_renewal_recovers_completed_response_using_saved_issue_time() {
    let f = Fixture::new();
    let server = RenewalServer::new("read");
    renewable_read(&f, &server.url);
    let mut base = f.client.credential("read", &server.url).unwrap();
    base.refresh_expires_at = Some(1);
    atomic_json(&f.client.state.join("read.credential"), &base).unwrap();
    let issued = now().saturating_sub(100);
    atomic_json(
        &f.client.state.join("read.renewal"),
        &json!({"base":base,"issued":issued,"completed":true}),
    )
    .unwrap();
    atomic_json(
        &f.client.state.join("read.response"),
        &reply("read", "grant-read"),
    )
    .unwrap();
    f.client
        .renew_recipient_access(
            &server.url,
            &base.generation,
            &base.token,
            None,
            false,
            &|| true,
        )
        .unwrap();
    let renewed = f.client.credential("read", &server.url).unwrap();
    assert_eq!(renewed.expires_at, Some(issued + 3600));
    assert_eq!(server.refreshes.load(Ordering::SeqCst), 0);
}

#[test]
fn recipient_renewal_cancellation_and_new_login_block_delayed_publication() {
    for new_login in [false, true] {
        let f = Fixture::new();
        let server = RenewalServer::new("read-delayed");
        renewable_read(&f, &server.url);
        let before = read_private(&f.client.state.join("read.credential"), 32768).unwrap();
        let client = f.client.clone();
        let url = server.url.clone();
        let worker = std::thread::spawn(move || {
            client.renew_recipient_access(
                &url,
                &"a".repeat(32),
                "synthetic-read-old",
                None,
                true,
                &|| true,
            )
        });
        wait_started(&server);
        if new_login {
            let mut replacement = f.client.credential("read", &server.url).unwrap();
            replacement.generation = "b".repeat(32);
            replacement.token = "synthetic-read-new-login".into();
            atomic_json(&f.client.state.join("read.credential"), &replacement).unwrap();
        } else {
            atomic_json(&f.client.state.join("cancellation"), &"cancelled-epoch").unwrap();
        }
        server.gate.release.store(true, Ordering::SeqCst);
        assert!(worker.join().unwrap().is_err());
        let after = read_private(&f.client.state.join("read.credential"), 32768).unwrap();
        if new_login {
            assert_ne!(after, before);
            assert_eq!(
                f.client.credential("read", &server.url).unwrap().generation,
                "b".repeat(32)
            );
        } else {
            assert_eq!(after, before);
        }
    }
}
fn settings_health_contended_renewal(cancel: bool) {
    use crate::settings::Health;
    use std::sync::mpsc::channel;
    for phase in [
        "renewal_preparation",
        "renewal_completion",
        "renewal_before_publication",
        "renewal_auth_required",
    ] {
        let f = Fixture::new();
        let server = RenewalServer::new(if phase == "renewal_auth_required" {
            "revoked"
        } else {
            "rotate"
        });
        renewable(&f, &server.url);
        let credential = fs::read(f.client.state.join("upload.credential")).unwrap();
        let config = fs::read(&f.client.config).unwrap();
        let active = Arc::new(AtomicBool::new(true));
        let running = active.clone();
        let client = f.client.clone();
        let (held_tx, held_rx) = channel();
        let (done_tx, done_rx) = channel();
        let worker = std::thread::spawn(move || {
            let state = client.state.clone();
            set_hook(move |point, _| {
                if point == phase {
                    held_tx
                        .send(Lock::acquire(&state.join("queue.lock"), true).unwrap())
                        .unwrap();
                }
            });
            let result = client.settings_health(&|| running.load(Ordering::SeqCst));
            clear_hook();
            done_tx.send(result).unwrap();
        });
        let held = held_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        if cancel {
            active.store(false, Ordering::SeqCst);
        }
        let completed = done_rx.recv_timeout(if cancel {
            Duration::from_secs(1)
        } else {
            Duration::from_secs(4)
        });
        // Release and join before asserting, including on broken behavior.
        drop(held);
        worker.join().unwrap();
        assert_eq!(
            completed.unwrap_or_else(|_| panic!(
                "health worker remained blocked at {phase}; cancel={cancel}"
            )),
            Health::Offline
        );
        assert_eq!(
            fs::read(f.client.state.join("upload.credential")).unwrap(),
            credential,
            "late credential write at {phase}"
        );
        assert_eq!(fs::read(&f.client.config).unwrap(), config);
        assert_eq!(
            server.refreshes.load(Ordering::SeqCst),
            usize::from(phase != "renewal_preparation")
        );
    }
}

#[test]
fn settings_health_renewal_queue_deadline_bounds_complete_call() {
    settings_health_contended_renewal(false);
}

#[test]
fn settings_health_renewal_queue_cancel_exits_without_late_writes() {
    settings_health_contended_renewal(true);
}

#[test]
fn settings_health_renews_without_fabricating_local_expiry_success() {
    let f = Fixture::new();
    let server = RenewalServer::new("");
    renewable(&f, &server.url);
    let local = f.client.settings_snapshot().unwrap();
    assert!(!local.reauthorization_required, "a valid refresh grant must not ask the user to sign in again");
    assert!(!local.uploads_enabled);
    assert!(local.renewal_required);
    let restarted = Client::new(provider(f.client.config.clone()));
    assert_eq!(restarted.settings_snapshot().unwrap(), local);
    assert_eq!(server.requests.load(Ordering::SeqCst), 0);
    assert_eq!(
        f.client.settings_health(&|| true),
        crate::settings::Health::Online
    );
    let renewed = f.client.settings_snapshot().unwrap();
    assert!(renewed.connected && renewed.uploads_enabled);
    assert!(!renewed.reauthorization_required);
    assert!(!renewed.renewal_required);
    assert_eq!(renewed.account_initial.as_deref(), Some("M"));
    assert_eq!(Client::new(provider(f.client.config.clone())).settings_snapshot().unwrap().account_initial.as_deref(), Some("M"));
    assert_eq!(server.refreshes.load(Ordering::SeqCst), 1);
}

#[test]
fn settings_expired_refresh_or_revoked_grant_requires_reapproval() {
    for revoked in [false, true] {
        let f = Fixture::new();
        let server = RenewalServer::new("");
        renewable(&f, &server.url);
        let mut credential = f.client.stored_credential("upload").unwrap();
        credential.account_initial = Some("M".into());
        if revoked {
            credential.auth_required = true;
        } else {
            credential.refresh_expires_at = Some(now().saturating_sub(1));
        }
        atomic_json(&f.client.state.join("upload.credential"), &credential).unwrap();
        let snapshot = f.client.settings_snapshot().unwrap();
        assert!(snapshot.reauthorization_required);
        assert!(!snapshot.renewal_required && !snapshot.uploads_enabled);
        assert!(snapshot.account_initial.is_none());
        assert_eq!(server.requests.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn account_initial_is_optional_display_metadata_not_authorization() {
    let f = Fixture::new();
    let server = RenewalServer::new("");
    f.authorize(&server.url);
    let mut credential = f.client.stored_credential("upload").unwrap();
    assert!(credential.account_initial.is_none(), "legacy grants have no identity display");
    for value in [json!("M"), json!("SS"), json!("界"), json!("m@example.com"), json!("\u{202e}M"), json!("ABCD"), json!(42), Value::Null] {
        let mut response = reply("upload", "synthetic-grant");
        response["accountInitial"] = value.clone();
        credential.apply_reply(&response, now(), false).unwrap();
        let expected = value.as_str().filter(|s| ["M", "SS", "界"].contains(s));
        assert_eq!(credential.account_initial.as_deref(), expected);
        atomic_json(&f.client.state.join("upload.credential"), &credential).unwrap();
        assert!(f.client.settings_snapshot().unwrap().uploads_enabled);
    }
    // Corrupted optional display metadata on disk must not reach a renderer.
    credential.account_initial = Some("m@example.com".into());
    atomic_json(&f.client.state.join("upload.credential"), &credential).unwrap();
    assert!(f.client.settings_snapshot().unwrap().account_initial.is_none());
    credential.account_initial = Some("M".into());
    credential.expires_at = Some(now().saturating_sub(1));
    atomic_json(&f.client.state.join("upload.credential"), &credential).unwrap();
    assert_eq!(f.client.settings_snapshot().unwrap().account_initial.as_deref(), Some("M"));
    f.client.settings_disable(true).unwrap();
    assert!(f.client.settings_snapshot().unwrap().account_initial.is_none());
}

#[test]
fn hosted_renewal_expired_account_renews_and_persists() {
    let f = Fixture::new();
    let server = RenewalServer::new("");
    renewable(&f, &server.url);
    let result = f.client.status(true);
    assert!(
        result.is_ok(),
        "expired credential must renew before account transport: {:?}",
        result.err()
    );
    assert_eq!(server.refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(server.requests.load(Ordering::SeqCst), 2);
    let persisted: Value = serde_json::from_slice(
        &read_private(&f.client.state.join("upload.credential"), 32768).unwrap(),
    )
    .unwrap();
    assert_eq!(persisted["generation"], "a".repeat(32));
    assert_eq!(persisted["refreshToken"], "synthetic-refresh-new");
    let restarted = Client::new(provider(f.client.config.clone()));
    restarted.status(true).unwrap();
    assert_eq!(server.refreshes.load(Ordering::SeqCst), 1);
}

#[test]
fn hosted_renewal_failures_keep_identity_and_classify_retry() {
    for mode in [
        "transient",
        "rate",
        "network",
        "revoked",
        "scope",
        "grant",
        "malformed",
        "lifetime",
    ] {
        let f = Fixture::new();
        let server = RenewalServer::new(mode);
        renewable(&f, &server.url);
        let id = f.client.enqueue(&f.bundle()).unwrap();
        let before = read_private(&f.client.state.join("upload.credential"), 32768).unwrap();
        f.client.process_once().unwrap();
        let jobs = f.client.jobs().unwrap();
        let job = &jobs[0];
        assert_eq!(job.id, id);
        assert_eq!(job.generation, "a".repeat(32));
        assert!(!job.files.is_empty());
        if matches!(mode, "transient" | "rate" | "network") {
            assert_eq!(job.status, "pending", "mode={mode}");
            assert_eq!(job.error.as_deref(), Some("upload_failed"));
            assert_eq!(
                read_private(&f.client.state.join("upload.credential"), 32768).unwrap(),
                before
            );
        } else {
            assert_eq!(job.status, "failed", "mode={mode}");
            assert_eq!(job.error.as_deref(), Some("auth_required"));
            let c = f.client.credential("upload", &server.url).unwrap();
            assert!(c.auth_required);
            assert_eq!(c.token, "synthetic-access-old");
        }
        assert_eq!(server.refreshes.load(Ordering::SeqCst), 1);
        assert_eq!(server.requests.load(Ordering::SeqCst), 1);
    }
}

#[test]
fn hosted_renewal_401_before_local_expiry_and_legacy_reapproval() {
    let f = Fixture::new();
    let server = RenewalServer::new("");
    renewable(&f, &server.url);
    let mut c = f.client.credential("upload", &server.url).unwrap();
    c.expires_at = Some(now() + 3600);
    atomic_json(&f.client.state.join("upload.credential"), &c).unwrap();
    f.client.status(true).unwrap();
    assert_eq!(server.refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(server.requests.load(Ordering::SeqCst), 3);
    f.authorize(&server.url);
    let error = f.client.status(true).unwrap_err().to_string();
    assert!(error.contains("auth_required") && error.contains("apparent connect login"));
    assert_eq!(
        server.refreshes.load(Ordering::SeqCst),
        1,
        "legacy grant attempted refresh"
    );
}

fn wait_started(server: &RenewalServer) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !server.gate.started.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(server.gate.started.load(Ordering::SeqCst));
}

#[test]
fn hosted_renewal_disable_cancels_jobs_without_network_locks_or_gate_restore() {
    let f = Fixture::new();
    let server = RenewalServer::new("delayed");
    renewable(&f, &server.url);
    let id = f.client.enqueue(&f.bundle()).unwrap();
    let before = read_private(&f.client.state.join("upload.credential"), 32768).unwrap();
    let client = f.client.clone();
    let worker = std::thread::spawn(move || client.process_once());
    wait_started(&server);
    let queue = Lock::acquire(&f.client.state.join("queue.lock"), true).unwrap();
    config::set_config_value_in(&f.client.config, "unrelated", json!({"kept":true})).unwrap();
    drop(queue);
    f.client.disable(false).unwrap();
    assert!(!f.client.enabled());
    server.gate.release.store(true, Ordering::SeqCst);
    worker.join().unwrap().unwrap();
    let jobs = f.client.jobs().unwrap();
    assert_eq!(jobs[0].id, id);
    assert_eq!(jobs[0].status, "cancelled");
    assert!(!jobs[0].files.is_empty());
    assert_eq!(
        read_private(&f.client.state.join("upload.credential"), 32768).unwrap(),
        before
    );
    assert_eq!(server.requests.load(Ordering::SeqCst), 1);
    assert!(!f.client.enabled());
    // Account use can recover the already received response without reviving uploads.
    f.client.status(true).unwrap();
    assert!(!f.client.enabled());
    assert_eq!(server.refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(f.client.jobs().unwrap()[0].status, "cancelled");
}

#[test]
fn hosted_renewal_late_response_cannot_replace_new_login_or_logout() {
    for logout in [false, true] {
        let f = Fixture::new();
        let server = RenewalServer::new("delayed");
        renewable(&f, &server.url);
        let client = f.client.clone();
        let worker = std::thread::spawn(move || client.status(true));
        wait_started(&server);
        let replacement = if logout {
            f.client.disable(true).unwrap();
            None
        } else {
            let _queue = Lock::acquire(&f.client.state.join("queue.lock"), false).unwrap();
            let mut c = f.client.credential("upload", &server.url).unwrap();
            c.generation = "b".repeat(32);
            c.token = "synthetic-access-new".into();
            atomic_json(&f.client.state.join("upload.credential"), &c).unwrap();
            Some(read_private(&f.client.state.join("upload.credential"), 32768).unwrap())
        };
        server.gate.release.store(true, Ordering::SeqCst);
        assert!(worker.join().unwrap().is_err());
        match replacement {
            Some(bytes) => assert_eq!(
                read_private(&f.client.state.join("upload.credential"), 32768).unwrap(),
                bytes
            ),
            None => {
                assert!(!f.client.state.join("upload.credential").exists());
                assert!(!f.client.state.join("read.credential").exists());
                assert!(!f.client.state.join("upload.renewal").exists());
                assert!(!f.client.state.join("upload.response").exists());
            }
        }
    }
}

const CHILD: &str = "test_support::renewal_tests::hosted_renewal_process_child";
#[test]
fn hosted_renewal_disable_at_publication_boundary_keeps_credentials_and_gates() {
    let f = Fixture::new();
    let server = RenewalServer::new("");
    renewable(&f, &server.url);
    let before = read_private(&f.client.state.join("upload.credential"), 32768).unwrap();
    let client = f.client.clone();
    set_hook(move |point, _| {
        if point == "renewal_before_publication" {
            client.disable(false).unwrap();
        }
    });
    let result = f.client.status(true);
    set_hook(|_, _| {});
    assert!(result.unwrap_err().to_string().contains("cancelled"));
    assert_eq!(
        read_private(&f.client.state.join("upload.credential"), 32768).unwrap(),
        before
    );
    assert!(!f.client.enabled());
    f.client.status(true).unwrap();
    assert!(!f.client.enabled());
    assert_eq!(server.refreshes.load(Ordering::SeqCst), 1);
}
#[test]
fn hosted_renewal_recovery_can_outlive_old_refresh_deadline() {
    let f = Fixture::new();
    let server = RenewalServer::new("");
    renewable(&f, &server.url);
    assert_eq!(finish_child(child(&f, true)).code(), Some(42));
    let mut old = f.client.credential("upload", &server.url).unwrap();
    old.refresh_expires_at = Some(1);
    atomic_json(&f.client.state.join("upload.credential"), &old).unwrap();
    let journal = f.client.state.join("upload.renewal");
    let mut record: Value =
        serde_json::from_slice(&read_private(&journal, 65536).unwrap()).unwrap();
    record["base"] = serde_json::to_value(old).unwrap();
    atomic_json(&journal, &record).unwrap();
    f.client.status(true).unwrap();
    assert_eq!(server.refreshes.load(Ordering::SeqCst), 1);
    assert!(f.client.credential("upload", &server.url).unwrap().usable());
}
fn expire_fixture(f: &Fixture, server: &str, scope: &str) {
    let mut c = f.client.credential(scope, server).unwrap();
    c.refresh_token = Some(format!("synthetic-refresh-{scope}"));
    c.grant_id = Some(format!("grant-{scope}"));
    c.expires_at = Some(1);
    c.refresh_expires_at = Some(now() + 86400);
    atomic_json(&f.client.state.join(format!("{scope}.credential")), &c).unwrap();
}
#[test]
fn hosted_renewal_all_transfer_sharing_and_account_consumers() {
    config::enable_experiment(HOSTED_SHARING);
    let f = Fixture::new();
    let server = Server::new("");
    f.authorize(&server.url);
    expire_fixture(&f, &server.url, "upload");
    let bundle = f.bundle();
    let id = f.client.enqueue(&bundle).unwrap();
    f.client.process_once().unwrap();
    assert_eq!(f.client.jobs().unwrap()[0].status, "ready");
    expire_fixture(&f, &server.url, "read");
    let out = f.root.join("renewed.apparent");
    f.client.fetch(&id, &out, 0).unwrap();
    assert_eq!(
        fs::read(out.join("summary.md")).unwrap(),
        fs::read(bundle.join("summary.md")).unwrap()
    );
    expire_fixture(&f, &server.url, "upload");
    let prepared = f.client.prepare_link(&bundle, &|| true).unwrap();
    assert_eq!(prepared.job.id, id);
    expire_fixture(&f, &server.url, "upload");
    let job = f.client.jobs().unwrap().remove(0);
    f.client.notify_progress(&job, "interrupted");
    assert!(
        server
            .state
            .lock()
            .unwrap()
            .progress
            .contains(&"interrupted".to_string())
    );
    expire_fixture(&f, &server.url, "read");
    assert_eq!(
        f.client.shares().unwrap()["shares"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    expire_fixture(&f, &server.url, "upload");
    f.client.revoke_share(&"c".repeat(32)).unwrap();
    expire_fixture(&f, &server.url, "upload");
    f.client.status(true).unwrap();
    let state = server.state.lock().unwrap();
    assert_eq!(
        state
            .request_routes
            .iter()
            .filter(|route| *route == "POST /v1/token/refresh")
            .count(),
        7
    );
    drop(state);
    assert!(prepared.authorized());
    assert_eq!(f.client.jobs().unwrap()[0].generation, "a".repeat(32));
    assert!(
        !f.client
            .status(false)
            .unwrap()
            .to_string()
            .contains("synthetic")
    );
}

#[test]
fn hosted_renewal_logout_revokes_expired_grant_and_succeeds_offline() {
    let f = Fixture::new();
    let server = Server::new("");
    f.authorize(&server.url);
    expire_fixture(&f, &server.url, "upload");
    expire_fixture(&f, &server.url, "read");
    // Read login may have switched the active server while retaining an upload grant.
    config::set_config_value_in(
        &f.client.config,
        "cloud",
        json!({"enabled":false,"uploadConsent":false,"server":"http://127.0.0.1:1"}),
    )
    .unwrap();
    let key = f.client.state.join("future-decryption-key");
    atomic_json(&key, &json!({"key":"synthetic-independent-key"})).unwrap();
    f.client.disable(true).unwrap();
    assert_eq!(
        server
            .state
            .lock()
            .unwrap()
            .request_routes
            .iter()
            .filter(|route| *route == "POST /v1/token/revoke")
            .count(),
        2
    );
    assert!(key.exists());
    let offline = Fixture::new();
    offline.authorize("http://127.0.0.1:1");
    let id = offline.client.enqueue(&offline.bundle()).unwrap();
    offline.client.disable(true).unwrap();
    assert!(!offline.client.state.join("upload.credential").exists());
    assert!(!offline.client.enabled());
    assert_eq!(offline.client.jobs().unwrap()[0].id, id);
    assert_eq!(offline.client.jobs().unwrap()[0].status, "cancelled");
}

#[test]
fn hosted_renewal_local_status_is_secret_free_and_expiry_is_actionable() {
    let f = Fixture::new();
    let server = RenewalServer::new("");
    renewable(&f, &server.url);
    let status = f.client.status(false).unwrap();
    assert_eq!(status["uploadAuth"]["status"], "renewal_due");
    assert!(!status.to_string().contains("synthetic"));
    let mut c = f.client.credential("upload", &server.url).unwrap();
    c.refresh_expires_at = Some(1);
    atomic_json(&f.client.state.join("upload.credential"), &c).unwrap();
    let status = f.client.status(false).unwrap();
    assert_eq!(status["uploadAuth"]["status"], "auth_required");
    assert_eq!(status["uploadSignedIn"], false);
    assert!(
        f.client
            .status(true)
            .unwrap_err()
            .to_string()
            .contains("auth_required")
    );
    assert_eq!(server.requests.load(Ordering::SeqCst), 0);
}
#[test]
fn hosted_renewal_process_child() {
    let Some(config) = std::env::var_os("APPARENT_RENEWAL_TEST_CONFIG") else {
        return;
    };
    let crash = std::env::var_os("APPARENT_RENEWAL_CRASH").is_some();
    let ready = PathBuf::from(&config).with_file_name("renewal-waiting");
    set_hook(move |point, _| {
        if point == "renewal_waiting" {
            atomic_json(&ready, &true).unwrap();
        }
        if crash && point == "renewal_response_persisted" {
            std::process::exit(42);
        }
    });
    let client = Client::new(provider(config.into()));
    client.status(true).unwrap();
}
fn child(f: &Fixture, crash: bool) -> std::process::Child {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", CHILD, "--nocapture"])
        .env("APPARENT_RENEWAL_TEST_CONFIG", &f.client.config)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if crash {
        command.env("APPARENT_RENEWAL_CRASH", "1");
    }
    command.spawn().unwrap()
}
fn finish_child(mut child: std::process::Child) -> std::process::ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("renewal child timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
fn hosted_renewal_separate_processes_have_one_owner() {
    let f = Fixture::new();
    let server = RenewalServer::new("delayed");
    renewable(&f, &server.url);
    let first = child(&f, false);
    wait_started(&server);
    let second = child(&f, false);
    let ready = f.client.config.with_file_name("renewal-waiting");
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    let waited = ready.exists();
    assert_eq!(server.refreshes.load(Ordering::SeqCst), 1);
    server.gate.release.store(true, Ordering::SeqCst);
    assert!(finish_child(first).success());
    assert!(finish_child(second).success());
    assert!(
        waited,
        "second process never reached contested renewal lock"
    );
    assert_eq!(server.refreshes.load(Ordering::SeqCst), 1);
    assert_eq!(server.requests.load(Ordering::SeqCst), 3);
}
#[test]
fn hosted_renewal_crash_after_response_recovers_without_rotation() {
    let f = Fixture::new();
    let server = RenewalServer::new("");
    renewable(&f, &server.url);
    assert_eq!(finish_child(child(&f, true)).code(), Some(42));
    assert!(f.client.state.join("upload.response").exists());
    assert_eq!(
        f.client.credential("upload", &server.url).unwrap().token,
        "synthetic-access-old"
    );
    f.client.status(true).unwrap();
    assert_eq!(server.refreshes.load(Ordering::SeqCst), 1);
    assert!(!f.client.state.join("upload.response").exists());
    assert_eq!(
        f.client
            .credential("upload", &server.url)
            .unwrap()
            .generation,
        "a".repeat(32)
    );
}
