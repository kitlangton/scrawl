use super::*;
use crate::capture::desktop::settings::connect::{Approval, Health, SafeError, Snapshot};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread::JoinHandle,
    time::Duration,
};

type Allowed = Arc<dyn Fn() -> bool + Send + Sync>;

struct FakeBackend {
    state: Mutex<Snapshot>,
    login_started: mpsc::Sender<Allowed>,
    login_release: Mutex<mpsc::Receiver<()>>,
    disables: AtomicUsize,
    disable_gate: Mutex<Option<mpsc::Receiver<()>>>,
}
impl Backend for FakeBackend {
    fn snapshot(&self) -> Result<Snapshot, SafeError> {
        Ok(self.state.lock().unwrap().clone())
    }
    fn login(
        &self,
        _: &str,
        allowed: Allowed,
        approval: &dyn Fn(Approval),
    ) -> Result<(), SafeError> {
        approval(Approval {
            verification_uri: "https://example.test/device".into(),
            user_code: "ABCD-EFGH".into(),
        });
        self.login_started
            .send(allowed)
            .map_err(|_| SafeError::Unavailable)?;
        self.login_release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| SafeError::Unavailable)?;
        // Deliberately emulate a backend that returns success late. The host must
        // still gate worker activation on the admitted stream state and snapshot.
        let mut state = self.state.lock().unwrap();
        state.server = Some(DEFAULT_SERVER.to_owned());
        state.connected = true;
        state.uploads_enabled = true;
        Ok(())
    }
    fn health(&self, _: Allowed) -> Health {
        panic!("local reads must not probe")
    }
    fn disable(&self, _: bool) -> Result<(), SafeError> {
        self.disables.fetch_add(1, Ordering::SeqCst);
        if let Some(gate) = self.disable_gate.lock().unwrap().as_ref() {
            gate.recv_timeout(Duration::from_secs(2))
                .map_err(|_| SafeError::LocalState)?;
        }
        self.state.lock().unwrap().uploads_enabled = false;
        Ok(())
    }
}

struct Fixture {
    dir: PathBuf,
    config: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let dir = std::env::temp_dir().join(format!(
            "connect-backend-{}",
            crate::shared::ids::make_bundle_id()
        ));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = dir.join("config.json");
        std::fs::write(
            &config,
            r#"{"experiments":{"connectSettings":true},"privateToken":"preserve"}"#,
        )
        .unwrap();
        Self { dir, config }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

struct Session {
    client: BufReader<UnixStream>,
    server: Option<JoinHandle<()>>,
    workers: Arc<AtomicUsize>,
}
impl Session {
    fn start(config: &Path, backend: Arc<dyn Backend>) -> Self {
        let (client, server) = UnixStream::pair().unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let config = config.to_owned();
        let workers = Arc::new(AtomicUsize::new(0));
        let worker_count = workers.clone();
        let handle = std::thread::spawn(move || {
            run(
                server,
                clock_ms() + 3000,
                &config,
                || backend,
                move || {
                    worker_count.fetch_add(1, Ordering::SeqCst);
                },
                Duration::from_secs(2),
                None,
                None,
            )
        });
        Self {
            client: BufReader::new(client),
            server: Some(handle),
            workers,
        }
    }
    fn read(&mut self) -> serde_json::Value {
        let mut line = String::new();
        self.client.read_line(&mut line).expect("backend response");
        assert!(!line.is_empty(), "backend closed before response");
        serde_json::from_str(&line).expect("valid response JSON")
    }
    fn send(&mut self, request: serde_json::Value) {
        writeln!(self.client.get_mut(), "{request}").unwrap();
    }
    fn read_request(&mut self) -> serde_json::Value {
        self.send(serde_json::json!({"connect":"backend-read","deadline":clock_ms()+3000}));
        self.read()
    }
    fn command(&mut self, command: &str, operation: Option<u64>) -> serde_json::Value {
        let mut req = serde_json::json!({"connect":"backend-command","command":command,"deadline":clock_ms()+3000});
        if let Some(id) = operation {
            req["operation"] = id.into();
        }
        if command == "login" {
            req["origin"] = DEFAULT_SERVER.into();
        }
        self.send(req);
        self.read()
    }
    fn close(mut self) {
        let _ = self.client.get_mut().shutdown(std::net::Shutdown::Both);
        drop(self.client);
        self.server.take().unwrap().join().unwrap();
    }
}
fn fake() -> (Arc<FakeBackend>, mpsc::Receiver<Allowed>, mpsc::Sender<()>) {
    let (started_tx, started_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    (
        Arc::new(FakeBackend {
            state: Mutex::new(Snapshot {
                server: None,
                connected: false,
                uploads_enabled: false,
                reauthorization_required: false,
                renewal_required: false,
                account_initial: None,
            }),
            login_started: started_tx,
            login_release: Mutex::new(release_rx),
            disables: AtomicUsize::new(0),
            disable_gate: Mutex::new(None),
        }),
        started_rx,
        release_tx,
    )
}
fn wait_for(mut f: impl FnMut() -> bool) {
    let until = std::time::Instant::now() + Duration::from_secs(2);
    while !f() {
        assert!(
            std::time::Instant::now() < until,
            "timed out waiting for backend action"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}
fn operation(reply: &serde_json::Value) -> u64 {
    reply["account"]["operation"]
        .as_u64()
        .expect("operation id")
}

#[test]
fn local_read_never_starts_login_probe_or_worker() {
    let fixture = Fixture::new();
    let (backend, started, _) = fake();
    let mut session = Session::start(&fixture.config, backend.clone());
    let initial = session.read();
    assert_eq!(initial["account"]["snapshot"]["connected"], false);
    assert!(!initial.to_string().contains("privateToken"));
    let read = session.read_request();
    assert_eq!(read["account"]["snapshot"]["uploadsEnabled"], false);
    assert!(started.try_recv().is_err());
    assert_eq!(session.workers.load(Ordering::SeqCst), 0);
    assert_eq!(backend.disables.load(Ordering::SeqCst), 0);
    session.close();
}

#[test]
fn changed_origin_is_rejected_before_any_authorization_or_worker() {
    let fixture = Fixture::new();
    let (backend, started, _) = fake();
    let mut session = Session::start(&fixture.config, backend.clone());
    session.read();
    session.send(
        serde_json::json!({"connect":"backend-command","command":"login",
        "origin":"https://unapproved.example","deadline":clock_ms()+3000}),
    );
    let reply = session.read();
    assert_eq!(reply["account"]["error"], "invalid-server");
    assert!(reply["account"].get("operation").is_none());
    assert!(started.try_recv().is_err());
    assert_eq!(session.workers.load(Ordering::SeqCst), 0);
    assert_eq!(backend.disables.load(Ordering::SeqCst), 0);
    session.close();
}

#[test]
fn login_approval_is_consumed_by_read_then_activation_starts_worker_once() {
    let fixture = Fixture::new();
    let (backend, started, release) = fake();
    let mut session = Session::start(&fixture.config, backend.clone());
    session.read();
    let login = session.command("login", None);
    let id = operation(&login);
    let allowed = started.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(allowed());
    let approval = session.read_request();
    assert_eq!(approval["account"]["events"][0]["kind"], "approval");
    assert_eq!(session.workers.load(Ordering::SeqCst), 0);
    release.send(()).unwrap();
    let mut terminal = None;
    for _ in 0..10 {
        let reply = session.read_request();
        if reply["account"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "login")
        {
            terminal = Some(reply);
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        terminal.is_some(),
        "terminal login receipt must be read from the stream"
    );
    let activated = session.command("activate", Some(id));
    assert_eq!(activated["account"]["snapshot"]["uploadsEnabled"], true);
    assert_eq!(session.workers.load(Ordering::SeqCst), 1);
    session.command("activate", Some(id));
    assert_eq!(session.workers.load(Ordering::SeqCst), 1);
    session.close();
}

#[test]
fn eof_after_terminal_login_before_activation_revokes_and_never_starts_worker() {
    let fixture = Fixture::new();
    let (backend, started, release) = fake();
    // Hold revocation between the attempt counter and the committed effect so
    // observing the counter cannot accidentally stand in for completed cleanup.
    let (finish_disable, disable_gate) = mpsc::channel();
    *backend.disable_gate.lock().unwrap() = Some(disable_gate);
    let mut session = Session::start(&fixture.config, backend.clone());
    session.read();
    session.command("login", None);
    let allowed = started.recv_timeout(Duration::from_secs(1)).unwrap();
    release.send(()).unwrap();
    wait_for(|| backend.state.lock().unwrap().uploads_enabled);
    // Even delivering the successful terminal receipt does not acknowledge
    // activation. Closing in this gap must still durably revoke the login.
    let mut delivered = false;
    for _ in 0..10 {
        let reply = session.read_request();
        if reply["account"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "login")
        {
            delivered = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(delivered);
    let workers = session.workers.clone();
    session.close();
    assert!(!allowed());
    wait_for(|| backend.disables.load(Ordering::SeqCst) == 1);
    assert!(backend.state.lock().unwrap().uploads_enabled);
    finish_disable.send(()).unwrap();
    wait_for(|| !backend.state.lock().unwrap().uploads_enabled);
    assert_eq!(workers.load(Ordering::SeqCst), 0);
    assert!(!backend.state.lock().unwrap().uploads_enabled);
}

#[test]
fn cancel_and_disable_win_over_queued_late_login_and_durable_disable_settles_once() {
    let fixture = Fixture::new();
    let (backend, started, release) = fake();
    let mut session = Session::start(&fixture.config, backend.clone());
    session.read();
    let login = session.command("login", None);
    let id = operation(&login);
    let allowed = started.recv_timeout(Duration::from_secs(1)).unwrap();
    let cancel = session.command("cancel", None);
    assert_eq!(cancel["account"]["error"], serde_json::Value::Null);
    assert!(!allowed());
    release.send(()).unwrap();
    wait_for(|| backend.state.lock().unwrap().uploads_enabled);
    let read = session.read_request();
    assert_eq!(
        read["account"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|e| e["kind"] == "login")
            .count(),
        0
    );
    assert_eq!(session.workers.load(Ordering::SeqCst), 0);
    // A durable disable is allowed to finish even if EOF follows immediately.
    let (release_disable, disable_gate) = mpsc::channel();
    *backend.disable_gate.lock().unwrap() = Some(disable_gate);
    session.command("disable", None);
    // Repeating the same durable revocation while it is in flight must not
    // schedule a second backend transaction.
    session.command("disable", None);
    session.close();
    wait_for(|| backend.disables.load(Ordering::SeqCst) >= 1);
    release_disable.send(()).unwrap();
    wait_for(|| !backend.state.lock().unwrap().uploads_enabled);
    assert_eq!(backend.disables.load(Ordering::SeqCst), 1);
    assert!(!backend.state.lock().unwrap().uploads_enabled);
    assert_ne!(id, 0);
}
