use super::*;
use std::io::{BufRead, BufReader};
use std::sync::{Mutex, atomic::{AtomicUsize, Ordering}};
use crate::capture::desktop::settings::connect::{Approval, Health, SafeError, Snapshot};

type Allowed = Arc<dyn Fn() -> bool + Send + Sync>;
struct Fake {
    state: Mutex<Snapshot>,
    login: mpsc::Sender<Allowed>,
    release: Mutex<mpsc::Receiver<()>>,
    disables: AtomicUsize,
    completed: AtomicUsize,
}
impl Backend for Fake {
    fn snapshot(&self) -> Result<Snapshot, SafeError> { Ok(self.state.lock().unwrap().clone()) }
    fn login(&self, _: &str, allowed: Allowed, approval: &dyn Fn(Approval)) -> Result<(), SafeError> {
        approval(Approval { verification_uri: "https://example.com/device?code=TEST-CODE".into(), user_code: "TEST-CODE".into() });
        self.login.send(allowed).unwrap();
        self.release.lock().unwrap().recv_timeout(Duration::from_secs(5)).unwrap();
        self.completed.fetch_add(1, Ordering::SeqCst);
        // Deliver a deliberately late successful receipt. The stream/controller,
        // not a synthetic transport response, must reject this obsolete receipt.
        Ok(())
    }
    fn health(&self, _: Allowed) -> Health { panic!("local reads must not probe") }
    fn disable(&self, _: bool) -> Result<(), SafeError> {
        self.disables.fetch_add(1, Ordering::SeqCst);
        self.state.lock().unwrap().uploads_enabled = false;
        Ok(())
    }
}
struct Fixture { dir: std::path::PathBuf, config: std::path::PathBuf }
impl Fixture {
    fn new(enabled: bool) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("cs-{}", crate::shared::ids::make_bundle_id()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = dir.join("config.json");
        std::fs::write(&config, serde_json::json!({"experiments":{"connectSettings":enabled},"privateToken":"keep"}).to_string()).unwrap();
        Self { dir, config }
    }
}
impl Drop for Fixture { fn drop(&mut self) { std::fs::remove_dir_all(&self.dir).unwrap(); } }
fn fake() -> (Arc<Fake>, mpsc::Receiver<Allowed>, mpsc::Sender<()>) {
    let (login, started) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    (Arc::new(Fake { state: Mutex::new(Snapshot { server: None, connected: false, uploads_enabled: false, reauthorization_required: false, renewal_required: false }),
        login, release: Mutex::new(wait), disables: AtomicUsize::new(0), completed: AtomicUsize::new(0) }), started, release)
}
fn start(config: &Path, backend: Arc<dyn Backend>, lease: Duration) -> (BufReader<UnixStream>, std::thread::JoinHandle<()>, Arc<AtomicUsize>) {
    let (client, server) = UnixStream::pair().unwrap();
    client.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let config = config.to_owned();
    let workers = Arc::new(AtomicUsize::new(0));
    let counter = workers.clone();
    let handle = std::thread::spawn(move || run(server, clock_ms() + 3000, &config, || backend,
        || { counter.fetch_add(1, Ordering::SeqCst); }, lease));
    (BufReader::new(client), handle, workers)
}
fn read(client: &mut BufReader<UnixStream>) -> serde_json::Value {
    let mut line = String::new();
    client.read_line(&mut line).unwrap();
    serde_json::from_str(&line).unwrap()
}
fn command(client: &mut BufReader<UnixStream>, name: &str) -> serde_json::Value {
    writeln!(client.get_mut(), "{}", serde_json::json!({"connect":"command","command":name,"deadline":clock_ms()+3000})).unwrap();
    read(client)
}
fn wait_for(mut predicate: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !predicate() { assert!(Instant::now() < deadline); std::thread::sleep(Duration::from_millis(10)); }
}
#[test]
fn opening_and_commands_are_exact_and_deadline_is_not_a_renderer_option() {
    assert_eq!(opening_deadline(r#"{"connect":"settings","deadline":1}"#), Some(1));
    for raw in [r#"{"connect":"settings","deadline":1,"url":"https://x"}"#, r#"{"connect":"settings"}"#, r#"{"connect":"read","deadline":1}"#] { assert_eq!(opening_deadline(raw), None); }
    assert!(serde_json::from_str::<Message>(r#"{"connect":"command","command":"open-browser","deadline":1}"#).is_ok());
    assert!(serde_json::from_str::<Message>(r#"{"connect":"read","deadline":1,"command":"allow"}"#).is_err());
}
#[test]
fn explicit_opt_out_never_constructs_client_or_changes_config() {
    let fixture = Fixture::new(false);
    let before = std::fs::read(&fixture.config).unwrap();
    let (client, server) = UnixStream::pair().unwrap();
    run(server, clock_ms()+3000, &fixture.config, || panic!("disabled feature instantiated backend"), || panic!("worker"), LEASE);
    assert_eq!(read(&mut BufReader::new(client))["error"], "feature_disabled");
    assert_eq!(std::fs::read(&fixture.config).unwrap(), before);
}
#[test]
fn missing_config_opens_local_controls_without_writes_or_network() {
    let fixture = Fixture::new(false);
    std::fs::remove_file(&fixture.config).unwrap();
    let (backend, started, _release) = fake();
    let (mut client, handle, workers) = start(&fixture.config, backend.clone(), LEASE);
    let first = read(&mut client);
    assert_eq!(first["connect"]["status"], "Not connected");
    drop(client); handle.join().unwrap();
    assert!(started.try_recv().is_err());
    assert_eq!(workers.load(Ordering::SeqCst), 0);
    assert_eq!(backend.disables.load(Ordering::SeqCst), 0);
    assert!(!fixture.config.exists());
}
#[test]
fn stream_reads_consent_cancel_and_preserves_unrelated_config_without_network() {
    let fixture = Fixture::new(true);
    let before = std::fs::read(&fixture.config).unwrap();
    let (backend, started, _release) = fake();
    let (mut client, handle, workers) = start(&fixture.config, backend.clone(), LEASE);
    let first = read(&mut client);
    assert!(first.get("connect").is_some()); assert!(!first.to_string().contains("privateToken"));
    let consent = command(&mut client, "connect");
    assert!(consent["connect"]["buttons"].as_array().unwrap().iter().any(|b| b["command"]=="allow"));
    command(&mut client, "cancel");
    assert!(started.try_recv().is_err());
    assert_eq!(std::fs::read(&fixture.config).unwrap(), before);
    drop(client); handle.join().unwrap();
    assert_eq!(workers.load(Ordering::SeqCst), 0);
    assert_eq!(backend.disables.load(Ordering::SeqCst), 0);
}
#[test]
fn invalid_and_expired_frames_cannot_allow_a_login() {
    for invalid in [serde_json::json!({"connect":"command","command":"allow","deadline":1}),
        serde_json::json!({"connect":"command","command":"allow","deadline":u64::MAX,"url":"https://x"})] {
        let fixture = Fixture::new(true);
        let (backend, started, _release) = fake();
        let (mut client, handle, _) = start(&fixture.config, backend, LEASE);
        read(&mut client); command(&mut client, "connect");
        writeln!(client.get_mut(), "{invalid}").unwrap();
        assert!(read(&mut client).get("error").is_some());
        drop(client); handle.join().unwrap(); assert!(started.try_recv().is_err());
    }
}
#[test]
fn partial_frames_survive_read_waits_but_compound_frames_are_refused() {
    let fixture = Fixture::new(true);
    let (backend, _, _release) = fake();
    let (mut client, handle, _) = start(&fixture.config, backend, LEASE);
    read(&mut client);
    client.get_mut().write_all(b"{\"connect\":\"read\",").unwrap();
    std::thread::sleep(Duration::from_millis(100));
    writeln!(client.get_mut(), "\"deadline\":{}}}", clock_ms()+3000).unwrap();
    assert!(read(&mut client).get("connect").is_some());
    client.get_mut().write_all(b"{}\n{}\n").unwrap();
    assert_eq!(read(&mut client)["error"], "invalid_request");
    drop(client); handle.join().unwrap();
}
#[test]
fn eof_feature_disable_and_idle_lease_revoke_pending_login_before_late_receipt() {
    for mode in ["eof", "feature", "lease"] {
        let fixture = Fixture::new(true);
        let (backend, started, release) = fake();
        let lease = if mode=="lease" { Duration::from_millis(150) } else { LEASE };
        let (mut client, handle, workers) = start(&fixture.config, backend.clone(), lease);
        read(&mut client); command(&mut client, "connect"); command(&mut client, "allow");
        let allowed = started.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(allowed());
        if mode=="feature" { std::fs::write(&fixture.config, r#"{"experiments":{"connectSettings":false}}"#).unwrap(); }
        if mode=="eof" { client.get_mut().shutdown(std::net::Shutdown::Both).unwrap(); }
        handle.join().unwrap();
        assert!(!allowed(), "{mode}: login must be invalid before obsolete receipt");
        release.send(()).unwrap();
        wait_for(|| backend.disables.load(Ordering::SeqCst)==1);
        assert_eq!(workers.load(Ordering::SeqCst), 0);
        assert!(!backend.state.lock().unwrap().uploads_enabled);
    }
}

#[test]
fn completion_without_a_window_read_cannot_start_uploads_or_escape_close_cancellation() {
    let fixture = Fixture::new(true);
    let (backend, started, release) = fake();
    let (mut client, handle, workers) = start(&fixture.config, backend.clone(), LEASE);
    read(&mut client); command(&mut client, "connect"); command(&mut client, "allow");
    started.recv_timeout(Duration::from_secs(1)).unwrap();
    {
        let mut state = backend.state.lock().unwrap();
        state.connected = true; state.uploads_enabled = true;
    }
    release.send(()).unwrap();
    wait_for(|| backend.completed.load(Ordering::SeqCst)==1);
    // Leave several worker poll cycles without another window request.
    std::thread::sleep(Duration::from_millis(500));
    let before_close = workers.load(Ordering::SeqCst);
    drop(client); handle.join().unwrap();
    let revoked = Instant::now() + Duration::from_secs(1);
    while backend.disables.load(Ordering::SeqCst)==0 && Instant::now()<revoked {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(before_close, 0, "A queued completion is not a window observation");
    assert_eq!(backend.disables.load(Ordering::SeqCst), 1);
    assert!(!backend.state.lock().unwrap().uploads_enabled);
}

#[test]
fn admitted_window_read_observes_success_and_starts_the_worker_once() {
    let fixture = Fixture::new(true);
    let (backend, started, release) = fake();
    let (mut client, handle, workers) = start(&fixture.config, backend.clone(), LEASE);
    read(&mut client); command(&mut client, "connect"); command(&mut client, "allow");
    started.recv_timeout(Duration::from_secs(1)).unwrap();
    {
        let mut state = backend.state.lock().unwrap();
        state.connected = true; state.uploads_enabled = true;
    }
    release.send(()).unwrap();
    wait_for(|| backend.completed.load(Ordering::SeqCst)==1);
    let deadline = Instant::now() + Duration::from_secs(2);
    while workers.load(Ordering::SeqCst)==0 {
        assert!(Instant::now()<deadline);
        writeln!(client.get_mut(), "{}", serde_json::json!({"connect":"read","deadline":clock_ms()+3000})).unwrap();
        read(&mut client);
        std::thread::sleep(Duration::from_millis(10));
    }
    writeln!(client.get_mut(), "{}", serde_json::json!({"connect":"read","deadline":clock_ms()+3000})).unwrap();
    assert_eq!(read(&mut client)["connect"]["status"], "Uploads on");
    drop(client); handle.join().unwrap();
    assert_eq!(workers.load(Ordering::SeqCst), 1);
    assert_eq!(backend.disables.load(Ordering::SeqCst), 0);
}

struct DisableBeforeAdmission { config: PathBuf, inner: Arc<Fake>, admitted: AtomicUsize }
impl Backend for DisableBeforeAdmission {
    fn snapshot(&self) -> Result<Snapshot, SafeError> { self.inner.snapshot() }
    fn login(&self, _: &str, allowed: Allowed, _: &dyn Fn(Approval)) -> Result<(), SafeError> {
        // Deterministically interleave the external feature edit after stream
        // admission but before the transport's first authorization predicate.
        std::fs::write(&self.config, r#"{"experiments":{"connectSettings":false}}"#).unwrap();
        if allowed() { self.admitted.fetch_add(1, Ordering::SeqCst); }
        Err(SafeError::Cancelled)
    }
    fn health(&self, _: Allowed) -> Health { panic!("not a health test") }
    fn disable(&self, logout: bool) -> Result<(), SafeError> { self.inner.disable(logout) }
}

#[test]
fn feature_edit_between_command_and_transport_admission_invalidates_the_client_predicate() {
    let fixture = Fixture::new(true);
    let (inner, _started, _release) = fake();
    let backend = Arc::new(DisableBeforeAdmission { config: fixture.config.clone(), inner: inner.clone(), admitted: AtomicUsize::new(0) });
    let (mut client, handle, workers) = start(&fixture.config, backend.clone(), LEASE);
    read(&mut client); command(&mut client, "connect"); command(&mut client, "allow");
    handle.join().unwrap();
    assert_eq!(backend.admitted.load(Ordering::SeqCst), 0);
    assert_eq!(workers.load(Ordering::SeqCst), 0);
    wait_for(|| inner.disables.load(Ordering::SeqCst)==1);
}
