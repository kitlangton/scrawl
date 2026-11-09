//! Native device-enrollment resource. Connect remains the credential/config writer;
//! shared TypeScript owns account presentation. No upload enrollment or consent.
use std::{path::PathBuf, sync::{Arc, atomic::{AtomicBool, Ordering}}};
use apparent_connect_client::settings::{Approval, SafeError};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct View {
    status: &'static str,
    error: Option<&'static str>,
    user_code: Option<String>,
    verification_uri: Option<String>,
}

impl View {
    fn state(status: &'static str) -> Self { Self {status, error: None, user_code: None, verification_uri: None} }
    fn failed(error: SafeError) -> Self {
        let code = match error {
            SafeError::Cancelled => "cancelled", SafeError::Offline => "offline",
            SafeError::AuthRequired => "auth-required", SafeError::Rejected => "rejected",
            SafeError::Expired => "expired", SafeError::LocalState => "local-state",
            SafeError::InvalidServer => "invalid-server", SafeError::Unavailable => "unavailable",
        };
        Self {error: Some(code), ..Self::state("failed")}
    }
}

pub(super) struct Account {
    config: PathBuf,
    generation: u64,
    allowed: Arc<AtomicBool>,
    sender: flume::Sender<(u64, View)>,
    receiver: flume::Receiver<(u64, View)>,
    view: View,
}

impl Account {
    pub(super) fn new(config: PathBuf) -> Self {
        let (sender, receiver) = flume::unbounded();
        let account = Self {config: config.clone(), generation: 0, allowed: Arc::new(AtomicBool::new(false)),
            sender: sender.clone(), receiver, view: View::state("loading")};
        std::thread::spawn(move || {
            let view = match crate::connect::hosted::client_at(config).settings_snapshot() {
                Ok(snapshot) => View::state(if snapshot.connected {"signed-in"} else {"signed-out"}),
                Err(error) => View::failed(error),
            };
            let _ = sender.send((0, view));
        });
        account
    }

    pub(super) fn start(&mut self) {
        self.poll();
        if matches!(self.view.status, "loading" | "starting" | "approval" | "signed-in") { return; }
        self.cancel();
        self.generation += 1;
        let generation = self.generation;
        self.allowed = Arc::new(AtomicBool::new(true));
        self.view = View::state("starting");
        let allowed = self.allowed.clone();
        let sender = self.sender.clone();
        let config = self.config.clone();
        std::thread::spawn(move || {
            let client = crate::connect::hosted::client_at(config);
            let active = || allowed.load(Ordering::Acquire);
            let result = client.settings_snapshot().and_then(|snapshot| {
                if snapshot.connected { return Ok(()); }
                let server = snapshot.server.as_deref().unwrap_or(apparent_connect_client::settings::DEFAULT_SERVER);
                client.login_with_approval(server, "read", false, &active, &|approval: Approval| {
                    if active() {
                        let _ = sender.send((generation, View {status: "approval", error: None,
                            user_code: Some(approval.user_code), verification_uri: Some(approval.verification_uri)}));
                    }
                })
            });
            if !active() { return; }
            // Opening a browser or an approved response alone is not a signed-in receipt.
            let view = match result.and_then(|()| client.settings_snapshot()) {
                Ok(snapshot) if snapshot.connected => View::state("signed-in"),
                Ok(_) => View::failed(SafeError::AuthRequired),
                Err(error) => View::failed(error),
            };
            let _ = sender.send((generation, view));
        });
    }

    pub(super) fn cancel(&mut self) {
        self.allowed.store(false, Ordering::Release);
        self.generation += 1;
        if matches!(self.view.status, "starting" | "approval" | "loading") { self.view = View::state("signed-out"); }
    }

    pub(super) fn browser_uri(&self) -> Option<&str> {
        (self.view.status == "approval" && self.allowed.load(Ordering::Acquire))
            .then(|| self.view.verification_uri.as_deref()).flatten()
    }
    pub(super) fn browser_failed(&mut self) { self.view.error = Some("browser"); }

    pub(super) fn poll(&mut self) -> bool {
        let before = self.view.clone();
        for (generation, view) in self.receiver.try_iter() {
            if generation == self.generation { self.view = view; }
        }
        before != self.view
    }
    pub(super) fn view(&self) -> &View { &self.view }
}

impl Drop for Account {
    fn drop(&mut self) { self.allowed.store(false, Ordering::Release); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::{BufRead, Read, Write}, net::TcpListener, time::{Duration, Instant}};

    #[test]
    fn real_read_enrollment_commits_private_credentials_without_upload_consent_or_tour_milestones() {
        // AppConfig intentionally suppresses config experiment flags in tests.
        // Isolate its encryption override so other parallel tests cannot inherit it.
        const CHILD: &str = "APPARENT_ONBOARDING_ENCRYPTED_ACCOUNT_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "capture::desktop::settings::onboarding::account::tests::real_read_enrollment_commits_private_credentials_without_upload_consent_or_tour_milestones", "--test-threads=1"])
                .env(CHILD, "1").output().unwrap();
            assert!(output.status.success(), "{}\n{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
            return;
        }
        crate::shared::handoff::enable_experiment("hostedEncryption");
        let root = std::env::temp_dir().join(format!("onboarding-account-{}-{}", std::process::id(), crate::capture::desktop::settings::onboarding::unix_ms()));
        std::fs::create_dir(&root).unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = root.join("config.json");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        std::fs::write(&config, serde_json::json!({"cloud":{"server":origin,"enabled":false,"uploadConsent":false,"private":"keep"},
            "ingestToken":"synthetic-private", "experiments":{"hostedEncryption":true},
            "onboarding":{"shortcutChosen":false,"firstCaptureCompleted":false}}).to_string()).unwrap();
        let server_origin = origin.clone();
        let server = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(8);
            let mut requests = Vec::new();
            while requests.len() < 2 && Instant::now() < deadline {
                let (mut stream, _) = match listener.accept() {
                    Ok(value) => value,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {std::thread::sleep(Duration::from_millis(10)); continue;},
                    Err(error) => panic!("Synthetic account listener: {error}"),
                };
                stream.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                let mut reader = std::io::BufReader::new(&mut stream);
                let mut line = String::new(); reader.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap().to_owned();
                let mut length = 0;
                loop {
                    line.clear(); reader.read_line(&mut line).unwrap();
                    if line == "\r\n" {break;}
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {length = value.trim().parse::<usize>().unwrap();}
                    assert!(line.len() < 8192);
                }
                assert!(length < 8192);
                let mut body = vec![0; length]; reader.read_exact(&mut body).unwrap();
                let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
                let reply = if path == "/v1/device/start" {
                    assert_eq!(body["scope"], "read");
                    let key = &body["encryptionKey"];
                    assert_eq!(key["version"], 1, "approval must bind the endpoint key with encryption enabled");
                    assert!(key["recipient"].as_str().is_some_and(|value| value.starts_with("age1")));
                    assert_eq!(key["signing"].as_str().map(str::len), Some(64));
                    serde_json::json!({"deviceCode":"synthetic-device-secret","userCode":"TEST-CODE","expiresIn":20,"interval":1,
                        "verificationUri":format!("{server_origin}/connect/device?code=TEST-CODE")})
                } else {
                    assert_eq!(path, "/v1/device/poll");
                    serde_json::json!({"status":"approved","scope":"read","accessToken":"synthetic-read-secret", "refreshToken":"synthetic-refresh-secret",
                        "expiresIn":3600,"refreshExpiresIn":86400,"grantId":"synthetic-read-grant"})
                }.to_string();
                requests.push(path);
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",reply.len(),reply).unwrap();
            }
            requests
        });
        let mut account = Account::new(config.clone());
        let ready = Instant::now() + Duration::from_secs(2);
        while account.view.status == "loading" && Instant::now() < ready {
            account.poll(); std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(account.view.status, "signed-out");
        account.start();
        let deadline = Instant::now() + Duration::from_secs(7);
        let mut saw_approval = false;
        while Instant::now() < deadline {
            account.poll(); saw_approval |= account.view.status == "approval";
            if matches!(account.view.status, "signed-in" | "failed") {break;}
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(saw_approval, "real client must present explicit device approval: {:?}", account.view);
        assert_eq!(account.view.status, "signed-in");
        assert!(account.browser_uri().is_none());
        assert_eq!(server.join().unwrap(), ["/v1/device/start", "/v1/device/poll"]);
        let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
        assert_eq!(saved["cloud"]["enabled"], false); assert_eq!(saved["cloud"]["uploadConsent"], false);
        assert_eq!(saved["cloud"]["private"], "keep"); assert_eq!(saved["ingestToken"], "synthetic-private");
        assert_eq!(saved["onboarding"]["shortcutChosen"], false); assert_eq!(saved["onboarding"]["firstCaptureCompleted"], false);
        assert!(root.join("hosted/read.credential").is_file()); assert!(!root.join("hosted/upload.credential").exists());
        let credential: serde_json::Value = serde_json::from_slice(&std::fs::read(root.join("hosted/read.credential")).unwrap()).unwrap();
        assert!(credential["encryption_identity"].as_str().is_some_and(|identity| !identity.is_empty()), "sign-in must retain its private endpoint identity");
        let public = serde_json::to_string(account.view()).unwrap();
        assert!(!public.contains("secret") && !public.contains("credential"));
        drop(account); std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn loading_sign_in_waits_for_the_initial_snapshot_and_keeps_its_generation() {
        let (sender, receiver) = flume::unbounded();
        let mut account = Account {config: PathBuf::new(), generation: 0, allowed: Arc::new(AtomicBool::new(false)),
            sender: sender.clone(), receiver, view: View::state("loading")};
        account.start();
        assert_eq!(account.generation, 0);
        assert_eq!(account.view.status, "loading");
        assert!(!account.allowed.load(Ordering::Acquire));
        sender.send((0, View::state("signed-in"))).unwrap();
        account.start();
        assert_eq!(account.view.status, "signed-in");
        assert_eq!(account.generation, 0, "an existing credential must not start device enrollment");
    }

    #[test]
    fn cancellation_discards_queued_approval_and_completion_and_never_opens_browser() {
        let (sender, receiver) = flume::unbounded();
        let allowed = Arc::new(AtomicBool::new(true));
        let mut account = Account {config: PathBuf::new(), generation: 9, allowed: allowed.clone(),
            sender: sender.clone(), receiver, view: View::state("starting")};
        sender.send((9, View {status: "approval", user_code: Some("PUBLIC-CODE".into()),
            verification_uri: Some("https://example.test/connect/device?code=PUBLIC-CODE".into()), error: None})).unwrap();
        sender.send((9, View::state("signed-in"))).unwrap();
        account.cancel(); account.poll();
        assert!(!allowed.load(Ordering::Acquire));
        assert_eq!(account.view.status, "signed-out");
        assert!(account.browser_uri().is_none());
        account.sender.send((account.generation, View::failed(SafeError::Expired))).unwrap();
        assert!(account.poll());
        assert_eq!(account.view.error, Some("expired"));
    }
}
