#[cfg(test)]
use super::*;
use std::net::{TcpListener, TcpStream};

#[test]
fn development_children_cannot_inherit_a_production_reporting_marker() {
    let mut command = Command::new("unused");
    command.env("APPARENT_DESKTOP_REPORTING", "1");
    configure_child(&mut command);
    assert_eq!(command.get_envs().find(|(key, _)| *key == "APPARENT_DESKTOP_REPORTING").unwrap().1.unwrap(), "0");
}

#[test]
fn electron_failures_are_content_free_unhandled_and_attributed_to_the_child() {
    for (failure, process) in [
        (Failure::ElectronMainException, "electron-main"),
        (Failure::ElectronMainRejection, "electron-main"),
        (Failure::ElectronRendererGone, "electron-renderer"),
        (Failure::ElectronChildGone, "electron-child"),
    ] {
        let report = Report::new(failure, std::panic::Location::caller(), Consent { generation: "private-generation".into() });
        let payload = report.payload("phc_synthetic").unwrap();
        assert_eq!(payload["properties"]["process"], process);
        assert_eq!(payload["properties"]["$exception_list"][0]["mechanism"]["handled"], false);
        assert_eq!(payload["properties"]["operation"], failure.code());
        assert!(!payload.to_string().contains("private-generation"));
    }
}

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        use std::os::unix::fs::DirBuilderExt;
        let path = std::env::temp_dir().join(format!(
            "apparent-errors-{}",
            crate::shared::ids::make_bundle_id()
        ));
        std::fs::DirBuilder::new()
            .mode(0o700)
            .create(&path)
            .unwrap();
        Self(path)
    }
    fn config(&self) -> PathBuf {
        self.0.join("config.json")
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn consent_defaults_on_but_corrupt_or_unreadable_config_never_enables_reporting() {
    let scratch = Scratch::new();
    let path = scratch.config();
    assert!(enabled_in(&path));
    for invalid in [
        "{",
        "[]",
        "null",
        r#"{"errorReporting":"true"}"#,
        r#"{"errorReporting":false}"#,
        r#"{"errorReportingGeneration":3}"#,
    ] {
        std::fs::write(&path, invalid).unwrap();
        assert!(!enabled_in(&path), "{invalid}");
    }
    std::fs::write(&path, " ".repeat(65_537)).unwrap();
    assert!(!enabled_in(&path));
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    assert!(!enabled_in(&path));
    std::fs::remove_dir(&path).unwrap();
    std::os::unix::fs::symlink(scratch.0.join("missing"), &path).unwrap();
    assert!(!enabled_in(&path));
}

#[test]
fn narrow_settings_bridge_reads_default_and_persists_explicit_changes_without_dropping_keys() {
    let scratch = Scratch::new();
    let path = scratch.config();
    assert_eq!(error_reporting_bridge_in(&path, "read", None, || true), Ok(true));
    std::fs::write(&path, r#"{"sentinel":"keep"}"#).unwrap();
    assert_eq!(error_reporting_bridge_in(&path, "set", Some(false), || true), Ok(false));
    let saved: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["sentinel"], "keep");
    assert_eq!(saved["errorReporting"], false);
    assert_eq!(error_reporting_bridge_in(&path, "read", None, || true), Ok(false));
    assert_eq!(error_reporting_bridge_in(&path, "set", Some(true), || true), Ok(true));
    assert_eq!(error_reporting_bridge_in(&path, "unknown", None, || true), Err("invalid_request"));
    let before = std::fs::read(&path).unwrap();
    assert_eq!(error_reporting_bridge_in(&path, "set", Some(false), || false), Err("admission_expired"));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    std::fs::write(&path, "malformed").unwrap();
    assert_eq!(error_reporting_bridge_in(&path, "read", None, || true), Err("config_read_failed"));
    assert_eq!(error_reporting_bridge_in(&path, "set", Some(true), || true), Err("config_write_failed"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "malformed");
}

#[test]
fn disable_reenable_rotates_consent_and_preserves_unrelated_preferences() {
    use std::os::unix::fs::PermissionsExt;
    let scratch = Scratch::new();
    let path = scratch.config();
    std::fs::write(
        &path,
        r#"{"unknown":{"keep":true},"token":"synthetic-private-canary"}"#,
    )
    .unwrap();
    let original = consent_in(&path).unwrap();
    set_enabled_in(&path, false).unwrap();
    assert!(!enabled_in(&path));
    set_enabled_in(&path, true).unwrap();
    assert_ne!(
        consent_in(&path).unwrap(),
        original,
        "old queued reports must not revive"
    );
    let config: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
    assert_eq!(config["unknown"]["keep"], true);
    assert_eq!(config["token"], "synthetic-private-canary");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    std::fs::write(&path, "broken").unwrap();
    assert!(set_enabled_in(&path, true).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "broken");
}

#[test]
fn payload_is_allowlisted_with_no_user_or_error_text() {
    let consent = Consent {
        generation: "private-consent-canary".into(),
    };
    let report = Report::new(
        Failure::FinalizeCapture,
        std::panic::Location::caller(),
        consent,
    );
    let payload = report.payload("phc_synthetic_public_token").unwrap();
    assert_eq!(payload["event"], "$exception");
    assert_eq!(
        payload["properties"]["operation"],
        "capture_finalization_failed"
    );
    assert_eq!(
        payload["properties"]["$exception_list"][0]["stacktrace"]["frames"][0]["platform"],
        "native"
    );
    assert_eq!(payload["properties"]["$process_person_profile"], false);
    assert_eq!(payload["properties"]["$geoip_disable"], true);
    assert!(payload["properties"]["$ip"].is_null());
    let mut keys: Vec<_> = payload["properties"]
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort();
    let mut allowed = vec![
        "$process_person_profile",
        "$geoip_disable",
        "$ip",
        "$lib",
        "$app_version",
        "build",
        "commit_sha",
        "build_modified",
        "release_version",
        "package_version",
        "channel",
        "environment",
        "component",
        "process",
        "diagnostics_schema_version",
        "$os",
        "architecture",
        "operation",
        "$exception_level",
        "$exception_fingerprint",
        "$exception_list",
    ];
    if !env!("APPARENT_POSTHOG_RELEASE_ID").is_empty() {
        allowed.push("$release_id");
        assert_eq!(payload["properties"]["$release_id"], env!("APPARENT_POSTHOG_RELEASE_ID"));
    } else {
        assert!(payload["properties"].get("$release_id").is_none());
    }
    allowed.sort();
    assert_eq!(keys, allowed);
    assert_eq!(payload["properties"]["commit_sha"], env!("APPARENT_COMMIT_SHA"));
    assert_eq!(payload["properties"]["release_version"], env!("APPARENT_RELEASE_VERSION"));
    assert_eq!(payload["properties"]["build_modified"], env!("APPARENT_BUILD_MODIFIED") == "true");
    assert!(!payload.to_string().contains("private-consent-canary"));
    assert_ne!(
        payload["distinct_id"],
        report.payload("phc_synthetic_public_token").unwrap()["distinct_id"]
    );
    assert_eq!(source_file("/home/private/customer.rs"), None);
    assert_eq!(source_file("capture/../../private.rs"), None);
    assert_eq!(source_file("capture/private\nsecret.rs"), None);
    assert_eq!(
        source_file("capture/desktop/mod.rs").as_deref(),
        Some("capture/desktop/mod.rs")
    );
}

fn request(stream: &mut TcpStream) -> Value {
    stream
        .set_read_timeout(Some(Duration::from_secs(4)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut chunk = [0; 4096];
    loop {
        let n = stream.read(&mut chunk).unwrap();
        assert!(n > 0);
        bytes.extend_from_slice(&chunk[..n]);
        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
            let headers = String::from_utf8_lossy(&bytes[..end]);
            assert!(headers.starts_with("POST /i/v0/e/ HTTP/1.1"));
            let length: usize = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .map(str::to_owned)
                })
                .unwrap()
                .parse()
                .unwrap();
            if bytes.len() >= end + 4 + length {
                return serde_json::from_slice(&bytes[end + 4..end + 4 + length]).unwrap();
            }
        }
    }
}

fn accept(listener: &TcpListener) -> TcpStream {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).unwrap();
                return stream;
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10))
            }
            Err(error) => panic!("synthetic receiver failed: {error}"),
        }
    }
}

#[test]
fn real_curl_transport_posts_the_exception_and_never_follows_redirects() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/i/v0/e/", listener.local_addr().unwrap());
    let server = std::thread::spawn(move || {
        let mut stream = accept(&listener);
        let payload = request(&mut stream);
        stream.write_all(b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
        payload
    });
    let report = Report::new(
        Failure::ScreenStart,
        std::panic::Location::caller(),
        Consent {
            generation: "initial".into(),
        },
    );
    let payload = report.payload("phc_synthetic_public_token").unwrap();
    assert!(send(&endpoint, &payload).unwrap());
    assert_eq!(server.join().unwrap(), payload);
}

#[test]
fn worker_drops_pre_optout_reports_even_after_reenable() {
    let scratch = Scratch::new();
    let path = scratch.config();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let endpoint = format!("http://{}/i/v0/e/", listener.local_addr().unwrap());
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let mut stream = accept(&listener);
        let first = request(&mut stream);
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(4)).unwrap();
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
        let mut stream = accept(&listener);
        let second = request(&mut stream);
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
            .unwrap();
        (first, second)
    });
    let reporter =
        Reporter::start(path.clone(), "phc_synthetic_public_token".into(), endpoint).unwrap();
    reporter.capture(Failure::ScreenStart, std::panic::Location::caller());
    entered_rx.recv_timeout(Duration::from_secs(4)).unwrap();
    reporter.capture(Failure::AudioStart, std::panic::Location::caller());
    set_enabled_in(&path, false).unwrap();
    reporter.capture(Failure::PhoneStart, std::panic::Location::caller());
    set_enabled_in(&path, true).unwrap();
    reporter.capture(Failure::VideoExport, std::panic::Location::caller());
    release_tx.send(()).unwrap();
    let (first, second) = server.join().unwrap();
    assert_eq!(first["properties"]["operation"], "screen_start_failed");
    assert_eq!(second["properties"]["operation"], "video_export_failed");
}

#[test]
fn burst_protection_is_bounded_and_recovers_after_one_minute() {
    let now = Instant::now();
    let mut limiter = Limiter::default();
    for _ in 0..20 {
        assert!(limiter.admit(now));
    }
    assert!(!limiter.admit(now + Duration::from_secs(59)));
    assert!(limiter.admit(now + Duration::from_secs(60)));
}

#[test]
fn panic_hook_child() {
    let Some(path) = std::env::var_os("APPARENT_ERROR_TEST_CONFIG") else {
        return;
    };
    let endpoint = std::env::var("APPARENT_ERROR_TEST_ENDPOINT").unwrap();
    std::panic::set_hook(Box::new(|_| eprintln!("previous-hook-ran")));
    install_reporter(
        Reporter::start(path.into(), "phc_synthetic_public_token".into(), endpoint).unwrap(),
    );
    panic!("synthetic-private-panic-canary");
}

#[test]
fn real_panic_hook_flushes_sanitized_event_chains_hook_and_respects_optout() {
    for on in [true, false] {
        let scratch = Scratch::new();
        set_enabled_in(&scratch.config(), on).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/i/v0/e/", listener.local_addr().unwrap());
        let denied_listener = (!on).then(|| listener.try_clone().unwrap());
        let server = if on {
            Some(std::thread::spawn(move || {
                let mut stream = accept(&listener);
                let payload = request(&mut stream);
                stream
                    .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                    .unwrap();
                payload
            }))
        } else {
            listener.set_nonblocking(true).unwrap();
            // Keep this listener alive until the child has exited, so a report
            // would leave an observable accepted connection, not a refused port.
            None
        };
        let started = Instant::now();
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "capture::desktop::workflow::error_reporting::tests::panic_hook_child",
                "--nocapture",
            ])
            .env("APPARENT_ERROR_TEST_CONFIG", scratch.config())
            .env("APPARENT_ERROR_TEST_ENDPOINT", endpoint)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(started.elapsed() < Duration::from_secs(4));
        assert!(String::from_utf8_lossy(&output.stderr).contains("previous-hook-ran"));
        if let Some(listener) = denied_listener {
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        }
        if let Some(server) = server {
            let payload = server.join().unwrap();
            assert_eq!(payload["properties"]["operation"], "rust_panic");
            assert!(
                !payload
                    .to_string()
                    .contains("synthetic-private-panic-canary")
            );
            assert_eq!(
                payload["properties"]["$exception_list"][0]["mechanism"]["handled"],
                false
            );
        }
    }
}
