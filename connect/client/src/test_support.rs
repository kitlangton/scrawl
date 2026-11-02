//! Private synthetic transport fixtures, also reused by root real-provider tests.
//! Not enabled by the production dependency.
use super::*;
#[cfg(test)]
mod renewal_tests;
use std::sync::OnceLock;

pub type ConfigWriteHook = Box<dyn FnMut(&str, &Path)>;
pub trait TestHost: Send + Sync {
    fn provider(&self, path: PathBuf) -> Arc<dyn ConfigProvider>;
    fn enable_experiment(&self, name: &str);
    fn set_config_value_in(&self, path: &Path, key: &str, value: Value) -> Result<()>;
    fn set_experiment_in(&self, path: &Path, name: &str, enabled: bool) -> Result<()>;
    fn set_config_write_hook(&self, hook: Option<ConfigWriteHook>);
}
static HOST: OnceLock<Arc<dyn TestHost>> = OnceLock::new();
pub fn fixture_id() -> String {
    random_id().unwrap()
}
pub fn install_host(host: Arc<dyn TestHost>) {
    let _ = HOST.set(host);
}
fn host() -> &'static dyn TestHost {
    HOST.get().expect("root test host not installed").as_ref()
}

// Only a test provider: production config is owned by the application.
struct FixtureConfig(PathBuf);
impl ConfigProvider for FixtureConfig {
    fn config_path(&self) -> &Path {
        &self.0
    }
    fn cloud(&self) -> CloudConfig {
        fs::read(&self.0)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| serde_json::from_value(v["cloud"].clone()).ok())
            .unwrap_or_default()
    }
    fn experiment(&self, name: &str) -> bool {
        // Legacy tests explicitly enable these flags; mutable-provider regressions
        // below separately exercise fresh false reads.
        matches!(name, HOSTED_SYNC | HOSTED_SHARING)
    }
    fn set_cloud(&self, value: &CloudConfig) -> Result<()> {
        set_value(&self.0, "cloud", serde_json::to_value(value)?)
    }
    fn set_experiment(&self, name: &str, enabled: bool) -> Result<()> {
        let mut value = read_config(&self.0);
        value["experiments"][name] = enabled.into();
        atomic_json(&self.0, &value)
    }
}
fn read_config(path: &Path) -> Value {
    fs::read(path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| json!({}))
}
fn set_value(path: &Path, key: &str, value: Value) -> Result<()> {
    let mut config = read_config(path);
    config[key] = value;
    atomic_json(path, &config)
}
fn provider(path: PathBuf) -> Arc<dyn ConfigProvider> {
    HOST.get().map_or_else(
        || Arc::new(FixtureConfig(path.clone())) as Arc<dyn ConfigProvider>,
        |h| h.provider(path.clone()),
    )
}
mod config {
    use super::*;
    pub fn enable_experiment(name: &str) {
        if let Some(h) = HOST.get() {
            h.enable_experiment(name);
        }
    }
    pub fn set_config_value_in(path: &Path, key: &str, value: Value) -> Result<()> {
        if let Some(h) = HOST.get() {
            h.set_config_value_in(path, key, value)
        } else {
            set_value(path, key, value)
        }
    }
    pub fn set_experiment_in(path: &Path, name: &str, enabled: bool) -> Result<()> {
        if let Some(h) = HOST.get() {
            h.set_experiment_in(path, name, enabled)
        } else {
            FixtureConfig(path.into()).set_experiment(name, enabled)
        }
    }
    pub fn set_config_write_hook(hook: Option<ConfigWriteHook>) {
        host().set_config_write_hook(hook);
    }
}

use std::collections::BTreeMap;
use std::net::{TcpListener, TcpStream};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[cfg_attr(test, test)]
pub fn hosted_connect_disabled_upload_suggests_canonical_login() {
    let f = Fixture::new();
    let error = f.client.enqueue(&f.bundle()).unwrap_err().to_string();
    assert!(
        error.contains("apparent connect login --scope upload --consent-upload"),
        "{error}"
    );
    assert!(!error.contains("cloud login"), "{error}");
}

#[cfg(feature = "test-support")]
pub fn hosted_config_setting_snapshot_cannot_restore_consent() {
    use std::sync::mpsc::channel;
    let f = Fixture::new();
    let server = Server::new("");
    f.authorize(&server.url);
    let bundle = f.bundle();
    let credential_before = fs::read(f.client.state.join("upload.credential")).unwrap();
    let (snapshot_tx, snapshot_rx) = channel();
    let (release_tx, release_rx) = channel();
    let path = f.client.config.clone();
    let writer = std::thread::spawn(move || {
        config::set_config_write_hook(Some(Box::new(move |point, _| {
            if point == "snapshot" {
                snapshot_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            }
        })));
        let result = config::set_config_value_in(&path, "lowerPlayback", true.into());
        config::set_config_write_hook(None);
        result
    });
    snapshot_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let (started_tx, started_rx) = channel();
    let (done_tx, done_rx) = channel();
    let client = f.client.clone();
    let disable = std::thread::spawn(move || {
        config::set_config_write_hook(Some(Box::new(move |point, _| {
            if point == "before_lock" {
                started_tx.send(()).unwrap();
            }
        })));
        let result = client.disable(false);
        config::set_config_write_hook(None);
        done_tx.send(()).unwrap();
        result
    });
    started_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let returned_while_snapshot_held = done_rx.recv_timeout(Duration::from_millis(150)).is_ok();
    release_tx.send(()).unwrap();
    writer.join().unwrap().unwrap();
    disable.join().unwrap().unwrap();
    let settings = f.client.settings();
    assert!(
        !settings.enabled && !settings.upload_consent,
        "ordinary save restored stale consent"
    );
    assert!(
        !returned_while_snapshot_held,
        "disable bypassed the shared writer lock"
    );
    let raw: Value = serde_json::from_slice(&fs::read(&f.client.config).unwrap()).unwrap();
    assert_eq!(raw["lowerPlayback"], true);
    assert_eq!(raw["unrelated"]["kept"], true);
    assert!(f.client.credential("upload", &server.url).is_ok());
    assert!(fs::read(f.client.state.join("upload.credential")).unwrap() == credential_before);
    let requests = server.state.lock().unwrap().requests;
    assert!(f.client.enqueue(&bundle).is_err());
    f.client.process_once().unwrap();
    assert_eq!(server.state.lock().unwrap().requests, requests);
}

#[cfg(feature = "test-support")]
pub fn hosted_config_and_queue_locks_are_not_network_held() {
    let f = Fixture::new();
    let server = Server::new("upload");
    f.authorize(&server.url);
    let client = f.client.clone();
    let checked = Arc::new(AtomicBool::new(false));
    let observed = checked.clone();
    set_hook(move |point, _| {
        if point == "transport_poll" {
            let lock = Lock::acquire(&client.state.join("queue.lock"), true).unwrap();
            config::set_config_value_in(&client.config, "lowerPlayback", true.into()).unwrap();
            drop(lock);
            observed.store(true, Ordering::SeqCst);
        }
    });
    let result = f.client.login(&server.url, "upload", true, false);
    clear_hook();
    result.unwrap();
    assert!(checked.load(Ordering::SeqCst));
    // The loopback transfer fixture accepts its fixed synthetic credentials,
    // whereas device login deliberately returns a different synthetic grant.
    f.authorize(&server.url);
    let id = f.client.enqueue(&f.bundle()).unwrap();
    let client = f.client.clone();
    checked.store(false, Ordering::SeqCst);
    let observed = checked.clone();
    set_hook(move |point, _| {
        if point == "transport_poll" {
            let lock = Lock::acquire(&client.state.join("queue.lock"), true).unwrap();
            config::set_experiment_in(&client.config, "network-test", true).unwrap();
            drop(lock);
            observed.store(true, Ordering::SeqCst);
        }
    });
    let result = f.client.process_once();
    clear_hook();
    result.unwrap();
    assert!(checked.load(Ordering::SeqCst));
    let jobs = f.client.jobs().unwrap();
    assert!(
        jobs.iter().any(|job| job.id == id && job.status == "ready"),
        "upload did not become ready"
    );
}

// Re-enter this test executable rather than relying on a thread-only mutex proof.
#[cfg(feature = "test-support")]
pub fn hosted_config_subprocess_writer() {
    let Some(root) = std::env::var_os("APPARENT_TEST_CONFIG_WRITER_ROOT") else {
        return;
    };
    let root = PathBuf::from(root);
    let id = std::env::var("APPARENT_TEST_CONFIG_WRITER_ID").unwrap();
    let ready = root.join(format!("ready-{id}"));
    config::set_config_write_hook(Some(Box::new(move |point, _| {
        if point == "before_lock" {
            fs::write(&ready, b"ready").unwrap();
        }
    })));
    for n in 0..24 {
        config::set_experiment_in(&root.join("config.json"), &format!("flag-{id}-{n}"), true)
            .unwrap();
        config::set_config_value_in(
            &root.join("config.json"),
            &format!("key-{id}-{n}"),
            n.into(),
        )
        .unwrap();
    }
    config::set_config_write_hook(None);
}

fn wait_config_child(child: &mut std::process::Child) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            return;
        }
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let _ = child.wait();
            panic!("config writer subprocess timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(feature = "test-support")]
pub fn hosted_config_cross_process_nested_and_top_level_updates() {
    use std::sync::mpsc::channel;
    let f = Fixture::new();
    f.authorize("http://127.0.0.1:1");
    let alias = f.root.join("alias");
    std::os::unix::fs::symlink(&f.root, &alias).unwrap();
    let (snapshot_tx, snapshot_rx) = channel();
    let (release_tx, release_rx) = channel();
    let path = f.client.config.clone();
    let writer = std::thread::spawn(move || {
        config::set_config_write_hook(Some(Box::new(move |point, _| {
            if point == "snapshot" {
                snapshot_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            }
        })));
        let result = config::set_experiment_in(&path, "parent-flag", true);
        config::set_config_write_hook(None);
        result
    });
    snapshot_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let mut children: Vec<_> = (0..3)
        .map(|id| {
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "connect::hosted::hosted_tests::hosted_config_subprocess_writer",
                    "--test-threads=1",
                ])
                .env(
                    "APPARENT_TEST_CONFIG_WRITER_ROOT",
                    if id == 1 { &alias } else { &f.root },
                )
                .env("APPARENT_TEST_CONFIG_WRITER_ID", id.to_string())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap()
        })
        .collect();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !(0..3).all(|id| f.root.join(format!("ready-{id}")).exists()) && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(10));
    }
    let all_started = (0..3).all(|id| f.root.join(format!("ready-{id}")).exists());
    std::thread::sleep(Duration::from_millis(150));
    let all_blocked = children
        .iter_mut()
        .all(|child| child.try_wait().unwrap().is_none());
    release_tx.send(()).unwrap();
    writer.join().unwrap().unwrap();
    for child in &mut children {
        wait_config_child(child);
    }
    assert!(
        all_started && all_blocked,
        "separate processes bypassed the snapshot lock"
    );
    let raw: Value = serde_json::from_slice(&fs::read(&f.client.config).unwrap()).unwrap();
    assert_eq!(raw["experiments"]["parent-flag"], true);
    assert_eq!(raw["experiments"][HOSTED_SYNC], true);
    assert_eq!(raw["unrelated"]["kept"], true);
    for id in 0..3 {
        for n in 0..24 {
            assert_eq!(raw["experiments"][format!("flag-{id}-{n}")], true);
            assert_eq!(raw[format!("key-{id}-{n}")], n);
        }
    }
    assert!(f.client.credential("upload", "http://127.0.0.1:1").is_ok());
}

#[cfg(feature = "test-support")]
pub fn hosted_config_lock_rejects_symlinks_and_hardlinks() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let f = Fixture::new();
    let outside = f.root.join("outside");
    fs::write(&outside, b"untouched").unwrap();
    fs::set_permissions(&outside, fs::Permissions::from_mode(0o600)).unwrap();
    let lock = f.root.join("config.json.lock");
    symlink(&outside, &lock).unwrap();
    assert!(config::set_config_value_in(&f.client.config, "lowerPlayback", true.into()).is_err());
    fs::remove_file(&lock).unwrap();
    fs::hard_link(&outside, &lock).unwrap();
    assert!(config::set_experiment_in(&f.client.config, "test", true).is_err());
    assert_eq!(fs::read(&outside).unwrap(), b"untouched");
    assert!(!f.client.config.exists());
}

#[cfg(feature = "test-support")]
pub fn hosted_config_pins_parent_through_publication() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let parent = f.root.join("parent");
    let moved = f.root.join("moved");
    let outside = f.root.join("outside");
    fs::DirBuilder::new().mode(0o700).create(&parent).unwrap();
    fs::DirBuilder::new().mode(0o700).create(&outside).unwrap();
    let path = parent.join("config.json");
    let swap_parent = parent.clone();
    let swap_moved = moved.clone();
    let swap_outside = outside.clone();
    config::set_config_write_hook(Some(Box::new(move |point, _| {
        if point == "snapshot" {
            fs::rename(&swap_parent, &swap_moved).unwrap();
            symlink(&swap_outside, &swap_parent).unwrap();
        }
    })));
    let result = config::set_config_value_in(&path, "lowerPlayback", true.into());
    config::set_config_write_hook(None);
    result.unwrap();
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 0);
    assert!(moved.join("config.json").is_file());
}

type Hook = Box<dyn FnMut(&str, &Path)>;
thread_local! {
    static HOOK: std::cell::RefCell<Option<Hook>> = const { std::cell::RefCell::new(None) };
}
pub(super) fn run_hook(point: &str, path: &Path) {
    HOOK.with(|hook| {
        // A hook can call another client operation; do not recurse into itself.
        if let Ok(mut slot) = hook.try_borrow_mut() {
            if let Some(hook) = slot.as_mut() {
                hook(point, path);
            }
        }
    });
}
fn set_hook(hook: impl FnMut(&str, &Path) + 'static) {
    HOOK.with(|slot| *slot.borrow_mut() = Some(Box::new(hook)));
}
fn clear_hook() {
    HOOK.with(|slot| *slot.borrow_mut() = None);
}

#[cfg_attr(test, test)]
pub fn hosted_security_fetch_parent_swap_fails_closed() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let f = Fixture::new();
    let server = Server::new("");
    f.authorize(&server.url);
    let id = f.client.enqueue(&f.bundle()).unwrap();
    f.client.process_once().unwrap();
    let parent = f.root.join("shared");
    fs::create_dir(&parent).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o777)).unwrap();
    let outside = f.root.join("outside");
    fs::create_dir(&outside).unwrap();
    let redirect = outside.clone();
    set_hook(move |point, stage| {
        if point == "fetch_stage" {
            fs::rename(stage, stage.with_extension("stolen")).unwrap();
            symlink(&redirect, stage).unwrap();
        }
    });
    let result = f.client.fetch(&id, &parent.join("download.apparent"), 0);
    clear_hook();
    assert!(
        result.is_err(),
        "shared parent must be rejected before staging"
    );
    assert_eq!(
        fs::read_dir(&outside).unwrap().count(),
        0,
        "wrote through swapped stage"
    );
}

#[cfg_attr(test, test)]
pub fn hosted_security_source_parent_swap_fails_closed() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let f = Fixture::new();
    let bundle = f.bundle();
    fs::set_permissions(&bundle, fs::Permissions::from_mode(0o777)).unwrap();
    let outside = f.root.join("unrelated");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("frame-01.png"), b"private unrelated content").unwrap();
    set_hook(move |point, path| {
        if point == "snapshot_open" && path.ends_with("frames/frame-01.png") {
            let frames = path.parent().unwrap();
            fs::rename(frames, frames.with_extension("stolen")).unwrap();
            symlink(&outside, frames).unwrap();
        }
    });
    let stage = Scratch::new(&f.client.state).unwrap();
    let result = snapshot(&bundle, &stage.0);
    clear_hook();
    assert!(
        result.is_err(),
        "shared source directory permitted an intermediate swap"
    );
    assert!(
        !stage.0.join("frames/frame-01.png").exists(),
        "staged unrelated content"
    );
}

/// Hold the real login transport while the coordinator delivers deferred UI events.
pub fn deferred_cancelled_settings_login(
    active: Arc<AtomicBool>,
    cancel: impl FnOnce(),
) -> Result<()> {
    let f = Fixture::new();
    let server = Server::new("upload");
    atomic_json(&f.client.config, &json!({"unrelated":{"kept":true},"cloud":{"enabled":false,"uploadConsent":false,"server":server.url}})).unwrap();
    let before = fs::read(&f.client.config).unwrap();
    let gate = Arc::new(MediaGate::default());
    server.state.lock().unwrap().media_gate = Some(gate.clone());
    let client = f.client.clone();
    let url = server.url.clone();
    let worker = std::thread::spawn(move || {
        client.login_allowed(&url, "upload", true, false, &|| {
            active.load(Ordering::SeqCst)
        })
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    while !gate.started.load(Ordering::SeqCst) && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(5));
    }
    let reached_approval = gate.started.load(Ordering::SeqCst);
    cancel();
    gate.release.store(true, Ordering::SeqCst); // Approve B only after Settings B closes.
    let result = worker.join().unwrap();
    assert!(reached_approval, "login never reached deferred approval");
    assert_eq!(server.state.lock().unwrap().polls, 2);
    assert!(
        !f.client.state.join("upload.credential").exists(),
        "cancelled login published credentials"
    );
    assert_eq!(
        fs::read(&f.client.config).unwrap(),
        before,
        "cancelled login published consent"
    );
    assert!(!f.client.enabled());
    assert!(f.client.jobs().unwrap().is_empty());
    result
}

struct Fixture {
    root: PathBuf,
    client: Client,
}
impl Fixture {
    fn new() -> Self {
        // macOS temporary roots can be aliases; the fixture itself must use a
        // canonical tree so symlink rejection tests reach their intended boundary.
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("hosted-test-{}", random_id().unwrap()));
        fs::DirBuilder::new().mode(0o700).create(&root).unwrap();
        let client = Client::new(provider(root.join("config.json")));
        Self { root, client }
    }
    fn authorize(&self, server: &str) {
        config::enable_experiment(HOSTED_SYNC);
        atomic_json(&self.client.config, &json!({"unrelated":{"kept":true}, "experiments":{"hostedSync":true},"cloud":{"enabled":true,"uploadConsent":true,"server":server}})).unwrap();
        for scope in ["upload", "read"] {
            atomic_json(
                &self.client.state.join(format!("{scope}.credential")),
                &Credential {
                    site: Some(server.into()),
                    server: server.into(),
                    scope: scope.into(),
                    token: format!("synthetic-{scope}-secret"),
                    generation: "a".repeat(32),
                    refresh_token: None,
                    expires_at: None,
                    refresh_expires_at: None,
                    grant_id: None,
                    auth_required: false,
                },
            )
            .unwrap();
        }
    }
    fn bundle(&self) -> PathBuf {
        let path = self.root.join("test.apparent");
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path.join("frames"))
            .unwrap();
        fs::DirBuilder::new()
            .mode(0o700)
            .create(path.join("frames-full"))
            .unwrap();
        for (name, data) in [("apparent.json", b"{\"video\":\"recording.mov\",\"audio\":\"audio.wav\",\"keyframes\":[{\"file\":\"frames/frame-01.png\"}]}".as_slice()), ("summary.md", b"synthetic summary"), ("contact-sheet.png", b"sheet"), ("frames/frame-01.png", b"frame"), ("frames-full/frame-01.png", b"full frame"), ("contact-sheet-full.png", b"full sheet"), ("audio.wav", b"audio"), ("recording.mov", b"video"), ("timeline.json", b"[]"), ("finish-trace.json", b"{}") ] { fs::write(path.join(name), data).unwrap(); }
        path
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[derive(Default)]
struct State {
    request_routes: Vec<String>,
    list_routes: Vec<String>,
    progress: Vec<String>,
    shares: usize,
    revoked: bool,
    files: Vec<Asset>,
    capture_id: String,
    tombstones: BTreeSet<String>,
    data: BTreeMap<String, Vec<u8>>,
    ready: bool,
    polls: usize,
    requests: usize,
    mode: String,
    media_gate: Option<Arc<MediaGate>>,
}
#[derive(Default)]
struct MediaGate {
    started: AtomicBool,
    release: AtomicBool,
    transport_paused: AtomicBool,
    transport_release: AtomicBool,
}
struct Server {
    url: String,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Server {
    fn new(mode: &str) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://{addr}");
        let state = Arc::new(Mutex::new(State {
            mode: mode.into(),
            ..State::default()
        }));
        let stop = Arc::new(AtomicBool::new(false));
        let (s, done, origin) = (state.clone(), stop.clone(), url.clone());
        let thread = std::thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, _)) => serve(stream, &s, &origin),
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5))
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            url,
            state,
            stop,
            thread: Some(thread),
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        self.thread.take().unwrap().join().unwrap();
    }
}
// Exercise the real fixture reader with an accepted nonblocking socket. This
// models a possible BSD accept contract on Linux; it is not a Mac failure trace.
fn fragmented_nonblocking_request(split_in_body: bool) {
    // Keep accept blocking; the test explicitly makes the accepted stream nonblocking below.
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    client
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let (stream, _) = listener.accept().unwrap();
    stream.set_nonblocking(true).unwrap();
    let state = Arc::new(Mutex::new(State::default()));
    let served_state = state.clone();
    let request = b"PUT /v1/captures/test/files/summary.md HTTP/1.1\r\nAuthorization: Bearer synthetic-upload-secret\r\nContent-Length: 10\r\n\r\nfragmented";
    let split = if split_in_body { request.len() - 5 } else { 20 };
    client.write_all(&request[..split]).unwrap();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        started_tx.send(()).unwrap();
        serve(stream, &served_state, "http://127.0.0.1");
        done_tx.send(()).unwrap();
    });
    started_rx.recv_timeout(Duration::from_secs(3)).unwrap();
    // Hold back the remaining bytes while serve is running. A blocking reader
    // waits here; the old nonblocking reader returns at the arrival gap.
    let _ = done_rx.recv_timeout(Duration::from_millis(100));
    let write_error = client.write_all(&request[split..]).err().map(|e| e.kind());
    let _ = client.shutdown(std::net::Shutdown::Write);
    let mut response = Vec::new();
    let read_error = client.read_to_end(&mut response).err().map(|e| e.kind());
    worker.join().unwrap();
    assert_eq!(
        response, b"HTTP/1.1 200 Result\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}",
        "write_error={write_error:?}; read_error={read_error:?}"
    );
    assert_eq!(write_error, None);
    assert_eq!(read_error, None);
    let state = state.lock().unwrap();
    assert_eq!(
        state.request_routes,
        ["PUT /v1/captures/test/files/summary.md"]
    );
    assert_eq!(state.data.get("summary.md").unwrap(), b"fragmented");
}

#[cfg_attr(test, test)]
pub fn hosted_fixture_reads_fragmented_headers_from_nonblocking_accept() {
    fragmented_nonblocking_request(false);
}

#[cfg_attr(test, test)]
pub fn hosted_fixture_reads_fragmented_body_from_nonblocking_accept() {
    fragmented_nonblocking_request(true);
}

fn serve(mut stream: TcpStream, state: &Mutex<State>, origin: &str) {
    // accept's flag inheritance is platform-dependent. These bounded readers
    // require blocking I/O even though the listener polls for shutdown.
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let mut bytes = Vec::new();
    let end = loop {
        let mut buf = [0u8; 4096];
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => bytes.extend_from_slice(&buf[..n]),
        }
        if let Some(i) = bytes.windows(4).position(|b| b == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let head = String::from_utf8(bytes[..end].to_vec()).unwrap();
    let mut first = head.lines().next().unwrap().split_whitespace();
    let method = first.next().unwrap();
    let route = first.next().unwrap();
    let length = head
        .lines()
        .find_map(|l| {
            l.to_lowercase()
                .strip_prefix("content-length:")
                .and_then(|n| n.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);

    while bytes.len() < end + length {
        let mut buf = [0u8; 4096];
        match stream.read(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(n) => bytes.extend_from_slice(&buf[..n]),
        }
    }

    let body = &bytes[end..end + length];
    let mut s = state.lock().unwrap();
    s.requests += 1;
    s.request_routes.push(format!("{method} {route}"));
    let mut status = 200;
    let result: Vec<u8> = if route == "/v1/device/start" {
        json!({"deviceCode":"synthetic-device-secret", "userCode":"TEST-CODE", "verificationUri":format!("{origin}/hosted/device?code=TEST-CODE"), "expiresIn":5,"interval":1}).to_string().into_bytes()
    } else if route == "/v1/device/poll" {
        s.polls += 1;
        if s.mode == "reject" {
            json!({"status":"rejected"}).to_string().into_bytes()
        } else if s.polls == 1 {
            json!({"status":"pending"}).to_string().into_bytes()
        } else {
            if let Some(gate) = s.media_gate.clone() {
                drop(s);
                gate.started.store(true, Ordering::SeqCst);
                let deadline = Instant::now() + Duration::from_secs(10);
                while !gate.release.load(Ordering::SeqCst) && Instant::now() < deadline {
                    std::thread::sleep(Duration::from_millis(5));
                }
                s = state.lock().unwrap();
            }
            json!({"status":"approved","scope":if matches!(s.mode.as_str(), "wrong-scope" | "upload") {"upload"} else {"read"},"accessToken":"synthetic-approved-secret","refreshToken":"synthetic-refresh-secret","expiresIn":3600,"refreshExpiresIn":86400,"grantId":"synthetic-server-grant"}).to_string().into_bytes()
        }
    } else if route == "/v1/token/refresh" {
        let request: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(request.as_object().unwrap().len(), 1);
        let scope = if request["refreshToken"] == "synthetic-refresh-read" {
            "read"
        } else {
            assert_eq!(request["refreshToken"], "synthetic-refresh-upload");
            "upload"
        };
        json!({"accessToken":format!("synthetic-{scope}-secret"),"refreshToken":format!("synthetic-refresh-{scope}"),"scope":scope,"expiresIn":3600,"refreshExpiresIn":86400,"grantId":format!("grant-{scope}")}).to_string().into_bytes()
    } else if route == "/v1/token/revoke" {
        let request: Value = serde_json::from_slice(&body).unwrap();
        assert!(head.contains("Authorization:") || request["refreshToken"].is_string());
        json!({"ok":true}).to_string().into_bytes()
    } else if s.mode == "slow" {
        drop(s);
        std::thread::sleep(Duration::from_millis(400));
        b"{}".to_vec()
    } else if s.mode == "quota" {
        status = 409;
        json!({"error":{"code":"quota_exceeded","message":"synthetic-upload-secret must not appear"}}).to_string().into_bytes()
    } else {
        assert!(
            head.contains("Authorization: Bearer synthetic-upload-secret")
                || head.contains("Authorization: Bearer synthetic-read-secret")
        );
        let parts: Vec<_> = route.split('/').collect();
        let id = parts.get(3).copied().unwrap_or("");
        if route.starts_with("/v1/shares?") || route == "/v1/shares" {
            s.list_routes.push(route.into());
            if s.mode == "paged-shares" || s.mode == "looped-shares" {
                let second = route.contains('?');
                let shares: Vec<_> = (if second {100..103} else {0..100}).map(|i| json!({"id":format!("{i:032x}"),"captureId":"a".repeat(32),"url":format!("{origin}/s/{}", "b".repeat(64)),"status":"ready"})).collect();
                json!({"shares":shares,"nextCursor":if !second || s.mode == "looped-shares" {Some("1234:opaque /?&+")} else {None}}).to_string().into_bytes()
            } else {
                json!({"shares":if s.shares == 0 || s.revoked {vec![]} else {vec![json!({"id":"c".repeat(32),"captureId":s.capture_id,"url":format!("{origin}/s/{}", "b".repeat(64)),"status":"uploading","createdAt":123,"expiresAt":12345,"title":"Synthetic capture"})]},"nextCursor":null}).to_string().into_bytes()
            }
        } else if route == format!("/v1/shares/{}", "c".repeat(32)) && method == "DELETE" {
            s.revoked = true;
            json!({"ok":true}).to_string().into_bytes()
        } else if s.tombstones.contains(id) {
            status = 410;
            json!({"error":{"code":"expired"}}).to_string().into_bytes()
        } else if method == "POST" && parts.len() == 4 {
            if s.capture_id != id {
                s.capture_id = id.into();
                s.ready = false;
                s.data.clear();
            }
            s.files = serde_json::from_slice::<Value>(body).unwrap()["files"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| serde_json::from_value(v.clone()).unwrap())
                .collect();
            json!({"id":id,"status":if s.ready {"ready"} else {"pending"}})
                .to_string()
                .into_bytes()
        } else if parts.get(4) == Some(&"share") && parts.get(5) == Some(&"progress") {
            assert_eq!(method, "POST");
            let progress: Value = serde_json::from_slice(body).unwrap();
            s.progress.push(progress["state"].as_str().unwrap().into());
            json!({"ok":true}).to_string().into_bytes()
        } else if parts.get(4) == Some(&"share") {
            assert_eq!(id, s.capture_id);
            s.shares += 1;
            json!({"url":format!("{origin}/s/{}", "b".repeat(64)),"status":if s.ready {"ready"} else {"uploading"},"expiresAt":12345}).to_string().into_bytes()
        } else if parts.get(4) == Some(&"files") {
            let path = parts[5..].join("/");
            if method == "PUT" {
                if s.mode == "fail-file" {
                    status = 503;
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status} Result\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
                    );
                    return;
                }
                if path == "audio.wav" {
                    if let Some(gate) = s.media_gate.clone() {
                        drop(s);
                        gate.started.store(true, Ordering::SeqCst);
                        let deadline = Instant::now() + Duration::from_secs(5);
                        while !gate.release.load(Ordering::SeqCst) && Instant::now() < deadline {
                            std::thread::sleep(Duration::from_millis(5));
                        }
                        s = state.lock().unwrap();
                    }
                }
                s.data.insert(path, body.to_vec());
                b"{}".to_vec()
            } else if s.mode == "corrupt" {
                b"corrupt".to_vec()
            } else {
                s.data.get(&path).unwrap().clone()
            }
        } else if parts.get(4) == Some(&"complete") {
            s.ready = true;
            json!({"id":id,"status":"ready"}).to_string().into_bytes()
        } else {
            let mut files = s.files.clone();
            files.reverse();
            json!({"id":id,"status":if s.ready {"ready"} else {"pending"},"files":files})
                .to_string()
                .into_bytes()
        }
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status} Result\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        result.len()
    );
    let _ = stream.write_all(&result);
}

#[cfg_attr(test, test)]
pub fn hosted_sharing_management_follows_encoded_cursors_and_rejects_loops() {
    let f = Fixture::new();
    let server = Server::new("paged-shares");
    f.authorize(&server.url);
    let shares = f.client.shares().unwrap();
    assert_eq!(shares["shares"].as_array().unwrap().len(), 103);
    assert!(shares["nextCursor"].is_null());
    assert_eq!(
        server.state.lock().unwrap().list_routes,
        [
            "/v1/shares",
            "/v1/shares?before=1234%3Aopaque%20%2F%3F%26%2B"
        ]
    );
    server.state.lock().unwrap().mode = "looped-shares".into();
    assert!(
        f.client.shares().is_err(),
        "a looping response must not hang or return a partial list"
    );
}

#[cfg_attr(test, test)]
pub fn hosted_prepared_link_survives_upload_before_foreground_delivery() {
    let f = Fixture::new();
    let server = Server::new("");
    f.authorize(&server.url);
    config::enable_experiment(HOSTED_SHARING);
    // This is the production preparation/authorization seam used by the UI.
    // Defer foreground delivery until the real uploader has advanced its job.
    let link = f.client.prepare_link(&f.bundle(), &|| true).unwrap();
    assert_eq!(link.job.status, "pending");
    assert!(link.authorized());
    f.client.process_once().unwrap();
    assert_eq!(f.client.jobs().unwrap()[0].status, "ready");
    assert!(server.state.lock().unwrap().ready);
    assert!(
        link.authorized(),
        "upload completion revoked a valid foreground link"
    );
    assert_eq!(link.text.matches("Anyone with this link").count(), 1);
    // A valid ready result must still lose authorization when consent is removed.
    f.client.disable(false).unwrap();
    assert!(!link.authorized());
}

#[cfg_attr(test, test)]
pub fn hosted_prepared_link_rejects_changed_authorization_after_progress() {
    let f = Fixture::new();
    let server = Server::new("");
    f.authorize(&server.url);
    config::enable_experiment(HOSTED_SHARING);
    let link = f.client.prepare_link(&f.bundle(), &|| true).unwrap();
    let original = f.client.jobs().unwrap()[0].clone();
    for status in ["pending", "failed", "ready"] {
        let mut current = original.clone();
        current.status = status.into();
        f.client.save_job(&current).unwrap();
        assert!(link.authorized(), "valid progress status {status}");
    }
    // Each mutation alone must fail closed, even with all other gates open.
    for change in 0..9 {
        let mut current = original.clone();
        match change {
            0 => current.status = "cancelled".into(),
            1 => current.status = "expired".into(),
            2 => current.status = "unexpected".into(),
            3 => current.created += 1,
            4 => current.generation = "replacement-grant".into(),
            5 => current.bundle = f.root.join("replacement.apparent"),
            6 => current.server = "http://localhost:1".into(),
            7 => current.files[0].size += 1,
            _ => current.id = "d".repeat(32),
        }
        atomic_json(
            &f.client.state.join(format!("{}.job", original.id)),
            &current,
        )
        .unwrap();
        assert!(
            !link.authorized(),
            "accepted authorization mutation {change}"
        );
    }
    f.client.save_job(&original).unwrap();
    assert!(link.authorized());
    let mut credential = f.client.credential("upload", &server.url).unwrap();
    credential.generation = "e".repeat(32);
    atomic_json(&f.client.state.join("upload.credential"), &credential).unwrap();
    assert!(!link.authorized());
    credential.generation = original.generation.clone();
    atomic_json(&f.client.state.join("upload.credential"), &credential).unwrap();
    assert!(link.authorized());
    fs::remove_file(f.client.state.join(format!("{}.job", original.id))).unwrap();
    assert!(!link.authorized());
}

#[cfg_attr(test, test)]
pub fn hosted_share_request_survives_progress_during_transport() {
    let f = Fixture::new();
    let server = Server::new("");
    f.authorize(&server.url);
    config::enable_experiment(HOSTED_SHARING);
    let client = f.client.clone();
    let advanced = Arc::new(AtomicBool::new(false));
    let observed = advanced.clone();
    set_hook(move |point, _| {
        if point == "transport_poll" && !observed.swap(true, Ordering::SeqCst) {
            // Run the production uploader while the foreground request is held.
            client.process_once().unwrap();
        }
    });
    let result = f.client.prepare_link(&f.bundle(), &|| true);
    clear_hook();
    let link = result.unwrap();
    assert!(advanced.load(Ordering::SeqCst));
    assert_eq!(f.client.jobs().unwrap()[0].status, "ready");
    assert!(link.authorized());
}

#[cfg_attr(test, test)]
pub fn hosted_sharing_sender_reports_interruption_and_recovers_under_same_url() {
    let f = Fixture::new();
    let server = Server::new("fail-file");
    f.authorize(&server.url);
    config::enable_experiment(HOSTED_SHARING);
    let bundle = f.bundle();
    let url = f.client.share(&bundle).unwrap();
    f.client.process_once().unwrap();
    assert_eq!(
        server
            .state
            .lock()
            .unwrap()
            .progress
            .last()
            .map(String::as_str),
        Some("paused")
    );
    server.state.lock().unwrap().mode.clear();
    f.client.retry().unwrap();
    f.client.process_once().unwrap();
    assert_eq!(f.client.share(&bundle).unwrap(), url);
    assert!(server.state.lock().unwrap().ready);
    server.state.lock().unwrap().mode = "fail-file".into();
    let another = f.root.join("second.apparent");
    fs::rename(&bundle, &another).unwrap();
    f.client.share(&another).unwrap();
    for _ in 0..MAX_ATTEMPTS {
        let mut jobs = f.client.jobs().unwrap();
        let job = jobs.iter_mut().find(|job| job.status == "pending").unwrap();
        job.next_attempt = 0;
        f.client.save_job(job).unwrap();
        f.client.process_once().unwrap();
    }
    assert_eq!(
        server
            .state
            .lock()
            .unwrap()
            .progress
            .last()
            .map(String::as_str),
        Some("failed")
    );
}

#[cfg_attr(test, test)]
pub fn hosted_sharing_management_uses_management_id_and_verifies_revoke() {
    let f = Fixture::new();
    let server = Server::new("");
    f.authorize(&server.url);
    config::enable_experiment("hostedSharing");
    f.client.share(&f.bundle()).unwrap();
    let shares = f.client.shares().unwrap();
    assert_eq!(shares["shares"][0]["id"], "c".repeat(32));
    let before = server.state.lock().unwrap().requests;
    assert!(f.client.revoke_share(&"b".repeat(64)).is_err());
    assert_eq!(server.state.lock().unwrap().requests, before);
    f.client.revoke_share(&"c".repeat(32)).unwrap();
    assert!(
        f.client.shares().unwrap()["shares"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert!(server.state.lock().unwrap().revoked);
}

#[cfg_attr(test, test)]
pub fn hosted_sharing_reserves_before_media_reuses_url_and_resumes_queue() {
    let f = Fixture::new();
    let server = Server::new("");
    f.authorize(&server.url);
    config::enable_experiment("hostedSharing");
    let bundle = f.bundle();
    let url = f.client.share(&bundle).unwrap();
    assert_eq!(url, format!("{}/s/{}", server.url, "b".repeat(64)));
    assert!(
        server.state.lock().unwrap().data.is_empty(),
        "link must be reserved before full media upload"
    );
    let id = f.client.jobs().unwrap()[0].id.clone();
    assert_eq!(f.client.share(&bundle).unwrap(), url);
    assert_eq!(f.client.jobs().unwrap().len(), 1);
    let restarted = Client::new(f.client.provider.clone());
    restarted.process_once().unwrap();
    let jobs = restarted.jobs().unwrap();
    assert_eq!(jobs[0].status, "ready");
    assert_eq!(restarted.jobs().unwrap()[0].id, id);
    assert_eq!(restarted.share(&bundle).unwrap(), url);
    let another = f.root.join("another.apparent");
    fs::rename(&bundle, &another).unwrap();
    restarted.enqueue(&another).unwrap();
    fs::rename(&another, &bundle).unwrap();
    assert!(
        restarted.jobs().unwrap().iter().any(|j| j.id == id),
        "sharing identity must survive ordinary queue pruning"
    );
    restarted.disable(false).unwrap();
    assert!(restarted.share(&bundle).is_err());
}

#[cfg_attr(test, test)]
pub fn hosted_roundtrip_pending_pointer_restart_complete_and_no_overwrite() {
    let f = Fixture::new();
    let server = Server::new("");
    f.authorize(&server.url);
    let bundle = f.bundle();
    let id = f.client.enqueue(&bundle).unwrap();
    let pointer = f.client.pointer(&bundle).unwrap();
    assert!(
        pointer.contains("pending")
            && pointer.contains(&id)
            && pointer.contains("--scope read")
            && pointer.contains("apparent connect login --server")
            && !pointer.contains("apparent cloud")
            && !pointer.contains("secret")
    );
    assert_eq!(server.state.lock().unwrap().requests, 0);
    let restarted = Client::new(f.client.provider.clone());
    restarted.process_once().unwrap();
    assert_eq!(restarted.jobs().unwrap()[0].status, "ready");
    let out = f.root.join("download.apparent");
    restarted.fetch(&id, &out, 0).unwrap();
    for entry in server.state.lock().unwrap().data.keys() {
        assert_eq!(
            fs::read(bundle.join(entry)).unwrap(),
            fs::read(out.join(entry)).unwrap()
        );
    }
    assert!(
        restarted
            .fetch(&id, &out, 0)
            .unwrap_err()
            .to_string()
            .contains("already exists")
    );
    assert_eq!(
        fs::metadata(f.client.state.join("upload.credential"))
            .unwrap()
            .mode()
            & 0o777,
        0o600
    );
    assert!(
        !restarted
            .status(false)
            .unwrap()
            .to_string()
            .contains("secret")
    );
}
#[cfg_attr(test, test)]
pub fn hosted_disable_logout_and_quota_preserve_local() {
    let f = Fixture::new();
    let server = Server::new("quota");
    f.authorize(&server.url);
    let bundle = f.bundle();
    f.client.enqueue(&bundle).unwrap();
    f.client.process_once().unwrap();
    assert_eq!(
        f.client.jobs().unwrap()[0].error.as_deref(),
        Some("quota_exceeded")
    );
    f.client.disable(true).unwrap();
    let n = server.state.lock().unwrap().requests;
    assert!(f.client.retry().is_err());
    f.client.process_once().unwrap();
    assert_eq!(server.state.lock().unwrap().requests, n);
    assert_eq!(f.client.jobs().unwrap()[0].status, "cancelled");
    assert!(bundle.join("recording.mov").exists());
    assert!(!f.client.state.join("upload.credential").exists());
    assert_eq!(
        serde_json::from_slice::<Value>(&fs::read(&f.client.config).unwrap()).unwrap()["unrelated"]
            ["kept"],
        true
    );
}
#[cfg_attr(test, test)]
pub fn hosted_login_snapshot_cannot_restore_consent_after_disable() {
    login_snapshot_then_disable("read", false);
}

#[cfg_attr(test, test)]
pub fn hosted_upload_login_snapshot_cannot_restore_consent_after_logout() {
    login_snapshot_then_disable("upload", true);
}

fn login_snapshot_then_disable(scope: &'static str, logout: bool) {
    use std::sync::mpsc::channel;
    let f = Fixture::new();
    let server = Server::new(if scope == "upload" { "upload" } else { "" });
    f.authorize(&server.url);
    let queued = f.root.join("queued.apparent");
    fs::rename(f.bundle(), &queued).unwrap();
    let id = f.client.enqueue(&queued).unwrap();
    let bundle = f.bundle();
    let (snapshot_tx, snapshot_rx) = channel();
    let (resume_login_tx, resume_login_rx) = channel();
    let (login_done_tx, login_done_rx) = channel();
    let client = f.client.clone();
    let url = server.url.clone();
    let login = std::thread::spawn(move || {
        set_hook(move |point, _| {
            if point == "login_snapshot" {
                snapshot_tx.send(()).unwrap();
                resume_login_rx
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap();
            }
        });
        let result = client.login(&url, scope, scope == "upload", false);
        clear_hook();
        login_done_tx.send(()).unwrap();
        result
    });
    snapshot_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    let (disable_started_tx, disable_started_rx) = channel();
    let (resume_disable_tx, resume_disable_rx) = channel();
    let (disable_done_tx, disable_done_rx) = channel();
    let client = f.client.clone();
    let disable = std::thread::spawn(move || {
        set_hook(move |point, _| {
            if point == "disable_before_lock" {
                disable_started_tx.send(()).unwrap();
                resume_disable_rx
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap();
            }
        });
        let result = client.disable(logout);
        clear_hook();
        disable_done_tx.send(()).unwrap();
        result
    });
    // Both implementations reach this point: old disable has already written
    // false; fixed disable has not yet acquired the login-held queue lock.
    // No wait for a config write that the fixed implementation cannot make.
    disable_started_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    resume_login_tx.send(()).unwrap();
    login_done_rx.recv_timeout(Duration::from_secs(5)).unwrap();
    resume_disable_tx.send(()).unwrap();
    disable_done_rx
        .recv_timeout(Duration::from_secs(5))
        .unwrap();
    login.join().unwrap().unwrap();
    disable.join().unwrap().unwrap();

    let settings = f.client.settings();
    assert!(
        !settings.enabled && !settings.upload_consent,
        "login restored stale consent"
    );
    let requests = server.state.lock().unwrap().requests;
    let admission = f.client.enqueue(&bundle);
    f.client.process_once().unwrap();
    assert_eq!(server.state.lock().unwrap().requests, requests);
    assert!(
        admission.is_err(),
        "ordinary admission accepted after disable"
    );
    let jobs = f.client.jobs().unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(jobs[0].id, id);
    assert_eq!(jobs[0].status, "cancelled");
    if logout {
        assert!(!f.client.state.join("upload.credential").exists());
        assert!(!f.client.state.join("read.credential").exists());
    } else {
        assert!(f.client.credential("upload", &server.url).is_ok());
    }
}

#[cfg_attr(test, test)]
pub fn hosted_read_login_preserves_upload_gates_and_active_server() {
    let f = Fixture::new();
    let server = Server::new("");
    f.authorize("https://active.example.com");
    let before = f.client.settings();
    assert!(f.client.login(&server.url, "read", false, false).is_err());
    assert_eq!(f.client.settings(), before);
    assert_eq!(
        f.client
            .credential("read", &before.server)
            .unwrap()
            .generation,
        "a".repeat(32)
    );
    f.client.disable(false).unwrap();
    f.client.login(&server.url, "read", false, false).unwrap();
    assert_eq!(
        f.client.settings(),
        CloudConfig {
            enabled: false,
            upload_consent: false,
            server: server.url.clone(),
        }
    );
    assert!(f.client.enqueue(&f.bundle()).is_err());
}

#[cfg_attr(test, test)]
pub fn hosted_login_pending_reject_scope_and_explicit_consent() {
    for mode in ["", "reject", "wrong-scope"] {
        let f = Fixture::new();
        let server = Server::new(mode);
        assert!(f.client.login(&server.url, "upload", false, false).is_err());
        assert_eq!(server.state.lock().unwrap().requests, 0);
        let result = f.client.login(&server.url, "read", false, false);
        assert_eq!(result.is_ok(), mode.is_empty());
        assert!(!f.client.enabled());
        assert!(!f.client.state.join("upload.credential").exists());
    }
}
#[cfg_attr(test, test)]
pub fn hosted_integrity_and_pending_timeout_publish_nothing() {
    let f = Fixture::new();
    let server = Server::new("");
    f.authorize(&server.url);
    let id = f.client.enqueue(&f.bundle()).unwrap();
    f.client.process_once().unwrap();
    server.state.lock().unwrap().mode = "corrupt".into();
    let out = f.root.join("bad.apparent");
    assert!(f.client.fetch(&id, &out, 0).is_err());
    assert!(!out.exists());
    server.state.lock().unwrap().ready = false;
    let start = Instant::now();
    assert!(f.client.fetch(&id, &out, 1).is_err());
    assert!(start.elapsed() < Duration::from_secs(2));
    assert!(!out.exists());
}
#[cfg_attr(test, test)]
pub fn hosted_transport_network_timeout_and_cancellation() {
    let f = Fixture::new();
    let server = Server::new("slow");
    let stage = Scratch::new(&f.client.state).unwrap();
    let start = Instant::now();
    assert!(
        f.client
            .raw_request(
                &server.url,
                Some("synthetic-secret"),
                "GET",
                "/v1/account",
                None,
                &stage.0.join("timeout"),
                100,
                Duration::from_millis(100),
                &|| true
            )
            .is_err()
    );
    assert!(start.elapsed() < Duration::from_secs(1));
    assert!(
        f.client
            .request(
                &server.url,
                None,
                "GET",
                "/v1/account",
                None,
                &stage.0.join("cancelled"),
                100,
                Duration::from_secs(1),
                &|| false
            )
            .is_err()
    );
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    assert!(
        f.client
            .request(
                &url,
                None,
                "GET",
                "/v1/account",
                None,
                &stage.0.join("offline"),
                100,
                Duration::from_secs(1),
                &|| true
            )
            .is_err()
    );
}
#[cfg_attr(test, test)]
pub fn hosted_paths_symlinks_oversize_and_unknown_assets() {
    let f = Fixture::new();
    let bundle = f.bundle();
    let stage = Scratch::new(&f.client.state).unwrap();
    fs::write(bundle.join("unexpected.svg"), b"asset").unwrap();
    assert!(
        snapshot(&bundle, &stage.0).is_err(),
        "unknown asset must not silently vanish"
    );
    fs::remove_file(bundle.join("unexpected.svg")).unwrap();
    fs::remove_file(bundle.join("audio.wav")).unwrap();
    let outside = f.root.join("private-audio");
    fs::write(&outside, b"never upload").unwrap();
    std::os::unix::fs::symlink(&outside, bundle.join("audio.wav")).unwrap();
    let stage = Scratch::new(&f.client.state).unwrap();
    assert!(snapshot(&bundle, &stage.0).is_err());
    fs::remove_file(bundle.join("audio.wav")).unwrap();
    let file = File::create(bundle.join("audio.wav")).unwrap();
    file.set_len(MAX_FILE_BYTES + 1).unwrap();
    let stage = Scratch::new(&f.client.state).unwrap();
    assert!(
        snapshot(&bundle, &stage.0)
            .unwrap_err()
            .to_string()
            .contains("file_too_large")
    );
}

#[cfg_attr(test, test)]
pub fn hosted_malicious_manifests_symlink_output_and_bounds() {
    let f = Fixture::new();
    let server = Server::new("");
    f.authorize(&server.url);
    let bundle = f.bundle();
    let id = f.client.enqueue(&bundle).unwrap();
    f.client.process_once().unwrap();
    let out = f.root.join("bad.apparent");
    let good = server.state.lock().unwrap().files.clone();
    for path in [
        "../summary.md",
        "frames/../../secret",
        "/absolute",
        "frames/.credential",
        "frames/frame-01.png/secret",
    ] {
        let mut s = server.state.lock().unwrap();
        s.files = good.clone();
        s.files[0].path = path.into();
        drop(s);
        assert!(f.client.fetch(&id, &out, 0).is_err());
        assert!(!out.exists());
    }
    server.state.lock().unwrap().files = good.clone();
    std::os::unix::fs::symlink(&bundle, &out).unwrap();
    assert!(f.client.fetch(&id, &out, 0).is_err());
    assert_eq!(fs::read(bundle.join("audio.wav")).unwrap(), b"audio");
    let mut bad = good.clone();
    bad[0].size = MAX_FILE_BYTES + 1;
    assert!(validate_assets(&bad).is_err());
    let mut bad = good.clone();
    bad[0].sha256 = "G".repeat(64);
    assert!(validate_assets(&bad).is_err());
    let mut bad = good.clone();
    bad.push(good[0].clone());
    assert!(validate_assets(&bad).is_err());
    assert!(validate_assets(&vec![good[0].clone(); MAX_FILES + 1]).is_err());
}

#[cfg_attr(test, test)]
pub fn hosted_disable_rejects_enqueue_paused_before_queue_lock() {
    for explicit in [false, true] {
        let f = Fixture::new();
        let server = Server::new("");
        f.authorize(&server.url);
        let bundle = f.bundle();
        let client = f.client.clone();
        let disabled = Arc::new(AtomicBool::new(false));
        let hook_disabled = disabled.clone();
        set_hook(move |point, _| {
            if point == "enqueue_before_lock" {
                // The initial eligibility check passed. Finish disable before
                // resuming this admission, without holding queue.lock here.
                client.disable(false).unwrap();
                hook_disabled.store(true, Ordering::SeqCst);
            }
        });
        let result = f.client.enqueue_with_intent(&bundle, explicit);
        clear_hook();
        assert!(disabled.load(Ordering::SeqCst));
        assert!(result.is_err(), "admitted a job after disable returned");
        assert!(f.client.jobs().unwrap().is_empty());
        let credential = f.client.credential("upload", &server.url).unwrap();
        let mut settings = f.client.settings();
        settings.enabled = true;
        settings.upload_consent = true;
        config::set_config_value_in(
            &f.client.config,
            "cloud",
            serde_json::to_value(settings).unwrap(),
        )
        .unwrap();
        assert_eq!(
            f.client
                .credential("upload", &server.url)
                .unwrap()
                .generation,
            credential.generation
        );
        f.client.process_once().unwrap();
        assert_eq!(server.state.lock().unwrap().requests, 0);
    }
}

#[cfg_attr(test, test)]
pub fn hosted_disable_interrupts_inflight_request_and_never_resurrects() {
    let f = Fixture::new();
    let server = Server::new("slow");
    f.authorize(&server.url);
    f.client.enqueue(&f.bundle()).unwrap();
    let client = f.client.clone();
    let worker = std::thread::spawn(move || client.process_once());
    let deadline = Instant::now() + Duration::from_secs(3);
    while server.state.lock().unwrap().requests == 0 {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
    f.client.disable(false).unwrap();
    worker.join().unwrap().unwrap();
    assert_eq!(f.client.jobs().unwrap()[0].status, "cancelled");
    assert!(server.state.lock().unwrap().data.is_empty());
}

#[cfg_attr(test, test)]
pub fn hosted_cancellation_survives_reenable_during_staging() {
    let f = Fixture::new();
    let server = Server::new("");
    f.authorize(&server.url);
    f.client.enqueue(&f.bundle()).unwrap();
    let client = f.client.clone();
    let mut fired = false;
    set_hook(move |point, _| {
        if point == "snapshot_open" && !fired {
            fired = true;
            client.disable(false).unwrap();
            let mut settings = client.settings();
            settings.enabled = true;
            settings.upload_consent = true;
            config::set_config_value_in(
                &client.config,
                "cloud",
                serde_json::to_value(settings).unwrap(),
            )
            .unwrap();
        }
    });
    let result = f.client.process_once();
    clear_hook();
    result.unwrap();
    assert_eq!(f.client.jobs().unwrap()[0].status, "cancelled");
    assert_eq!(
        server.state.lock().unwrap().requests,
        0,
        "cancelled staging sent data after reenable"
    );
}

#[cfg_attr(test, test)]
pub fn hosted_cancellation_survives_reenable_during_real_media_put() {
    let f = Fixture::new();
    let server = Server::new("");
    f.authorize(&server.url);
    f.client.enqueue(&f.bundle()).unwrap();
    let gate = Arc::new(MediaGate::default());
    server.state.lock().unwrap().media_gate = Some(gate.clone());
    let client = f.client.clone();
    let transport_gate = gate.clone();
    let worker = std::thread::spawn(move || {
        set_hook(move |point, path| {
            if point == "transport_poll" && path.ends_with("audio.wav") {
                let deadline = Instant::now() + Duration::from_secs(5);
                while !transport_gate.started.load(Ordering::SeqCst) {
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(5));
                }
                transport_gate
                    .transport_paused
                    .store(true, Ordering::SeqCst);
                while !transport_gate.transport_release.load(Ordering::SeqCst) {
                    assert!(Instant::now() < deadline);
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        });
        let result = client.process_once();
        clear_hook();
        result
    });
    let deadline = Instant::now() + Duration::from_secs(5);
    while !gate.transport_paused.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "media PUT never started");
        std::thread::sleep(Duration::from_millis(5));
    }
    let requests = server.state.lock().unwrap().requests;
    f.client.disable(false).unwrap();
    let mut settings = f.client.settings();
    settings.enabled = true;
    settings.upload_consent = true;
    config::set_config_value_in(
        &f.client.config,
        "cloud",
        serde_json::to_value(settings).unwrap(),
    )
    .unwrap();
    gate.transport_release.store(true, Ordering::SeqCst);
    std::thread::sleep(Duration::from_millis(60));
    gate.release.store(true, Ordering::SeqCst);
    worker.join().unwrap().unwrap();
    f.client.process_once().unwrap();
    assert_eq!(f.client.jobs().unwrap()[0].status, "cancelled");
    assert_eq!(
        server.state.lock().unwrap().requests,
        requests,
        "sent after durable cancellation"
    );
    assert!(!server.state.lock().unwrap().ready);
}

#[cfg_attr(test, test)]
pub fn hosted_explicit_upload_recovers_changed_expired_and_tombstoned_ids() {
    let f = Fixture::new();
    let server = Server::new("");
    f.authorize(&server.url);
    let bundle = f.bundle();
    let first = f.client.enqueue(&bundle).unwrap();
    assert_eq!(
        f.client.enqueue(&bundle).unwrap(),
        first,
        "ordinary capture must deduplicate"
    );
    // An immutable reserved manifest cannot be reused after the local bundle changes.
    let stage = Scratch::new(&f.client.state).unwrap();
    let Ok(jobs) = f.client.jobs() else {
        panic!("Could not read hosted queue for explicit upload recovery");
    };
    let Some(mut job) = jobs.into_iter().next() else {
        panic!("Hosted queue was empty for explicit upload recovery");
    };
    job.files = snapshot(&bundle, &stage.0).unwrap();
    job.status = "failed".into();
    f.client.save_job(&job).unwrap();
    fs::write(bundle.join("summary.md"), b"changed synthetic summary").unwrap();
    let changed = f.client.enqueue_explicit(&bundle).unwrap();
    assert_ne!(
        changed, first,
        "explicit changed bundle reused immutable ID"
    );
    f.client.process_once().unwrap();
    assert_eq!(
        f.client
            .jobs()
            .unwrap()
            .iter()
            .find(|j| j.id == changed)
            .unwrap()
            .status,
        "ready"
    );
    assert_eq!(
        server.state.lock().unwrap().data["summary.md"],
        b"changed synthetic summary"
    );
    let mut job = f
        .client
        .jobs()
        .unwrap()
        .into_iter()
        .find(|j| j.id == changed)
        .unwrap();
    job.created = now() - RETENTION - 1;
    job.status = "expired".into();
    f.client.save_job(&job).unwrap();
    let recovered = f.client.enqueue_explicit(&bundle).unwrap();
    assert_ne!(recovered, changed, "explicit upload reused expired ID");
    f.client.process_once().unwrap();
    // A server tombstone is permanent even when this client still thinks ready.
    server
        .state
        .lock()
        .unwrap()
        .tombstones
        .insert(recovered.clone());
    assert!(
        f.client
            .fetch(&recovered, &f.root.join("expired.apparent"), 0)
            .is_err()
    );
    let fresh = f.client.enqueue_explicit(&bundle).unwrap();
    assert_ne!(fresh, recovered, "explicit upload reused tombstoned ID");
    server.state.lock().unwrap().ready = false;
    f.client.process_once().unwrap();
    assert_eq!(
        f.client
            .jobs()
            .unwrap()
            .iter()
            .find(|j| j.id == fresh)
            .unwrap()
            .status,
        "ready"
    );
    assert_eq!(f.client.enqueue(&bundle).unwrap(), fresh);
}

#[cfg_attr(test, test)]
pub fn hosted_automatic_dedup_ends_when_terminal_record_is_pruned() {
    for terminal in ["cancelled", "ready"] {
        let f = Fixture::new();
        let server = Server::new("");
        f.authorize(&server.url);
        let a = f.root.join("a.apparent");
        fs::rename(f.bundle(), &a).unwrap();
        let b = f.bundle();
        let first = f.client.enqueue(&a).unwrap();
        let Ok(jobs) = f.client.jobs() else {
            panic!("Could not read hosted jobs for terminal-record dedup test");
        };
        let Some(mut job) = jobs.into_iter().next() else {
            panic!("Hosted queue was empty for terminal-record dedup test");
        };
        job.status = terminal.into();
        f.client.save_job(&job).unwrap();
        assert_eq!(f.client.enqueue(&a).unwrap(), first);

        let other = f.client.enqueue(&b).unwrap();
        assert!(!f.client.state.join(format!("{first}.job")).exists());
        assert_eq!(f.client.jobs().unwrap().len(), 1);
        // No scan discovers A: only another completion notification admits it.
        let fresh = f.client.enqueue(&a).unwrap();
        assert_ne!(fresh, first);
        let jobs = f.client.jobs().unwrap();
        assert_eq!(jobs.len(), 2);
        assert!(jobs.iter().any(|j| j.id == other && j.status == "pending"));
        assert!(jobs.iter().any(|j| j.id == fresh && j.status == "pending"));
        assert_eq!(server.state.lock().unwrap().requests, 0);
    }
}

#[cfg_attr(test, test)]
pub fn hosted_config_pause_and_cancelled_dedup_do_not_backfill() {
    let f = Fixture::new();
    let server = Server::new("");
    f.authorize(&server.url);
    let bundle = f.bundle();
    let id = f.client.enqueue(&bundle).unwrap();
    let mut settings = f.client.settings();
    settings.enabled = false;
    config::set_config_value_in(
        &f.client.config,
        "cloud",
        serde_json::to_value(&settings).unwrap(),
    )
    .unwrap();
    f.client.process_once().unwrap();
    assert_eq!(server.state.lock().unwrap().requests, 0);
    assert_eq!(f.client.jobs().unwrap()[0].status, "pending");
    settings.enabled = true;
    config::set_config_value_in(
        &f.client.config,
        "cloud",
        serde_json::to_value(&settings).unwrap(),
    )
    .unwrap();
    f.client.disable(false).unwrap();
    settings.upload_consent = true;
    config::set_config_value_in(
        &f.client.config,
        "cloud",
        serde_json::to_value(&settings).unwrap(),
    )
    .unwrap();
    assert_eq!(
        f.client.enqueue(&bundle).unwrap(),
        id,
        "automatic duplicate revived a cancelled capture"
    );
    f.client.process_once().unwrap();
    assert_eq!(server.state.lock().unwrap().requests, 0);
    assert_eq!(f.client.jobs().unwrap()[0].status, "cancelled");
    let fresh = f.client.enqueue_explicit(&bundle).unwrap();
    assert_ne!(fresh, id);
    assert_eq!(
        server.state.lock().unwrap().requests,
        0,
        "identity publication performed network work"
    );
    f.client.process_once().unwrap();
    assert_eq!(f.client.jobs().unwrap()[0].status, "ready");
}

#[cfg(target_os = "macos")]
#[cfg_attr(test, test)]
pub fn hosted_security_darwin_acl_grants_fail_closed() {
    let f = Fixture::new();
    // Only this owned fixture receives ACLs. Remove them before Fixture's Drop,
    // including when a refusal assertion panics; never clear ancestor/child ACLs.
    struct RestoreAcl(PathBuf);
    impl Drop for RestoreAcl {
        fn drop(&mut self) {
            let result = Command::new("/bin/chmod").arg("-N").arg(&self.0).status();
            if !matches!(result, Ok(status) if status.success()) {
                eprintln!("could not remove the hosted test fixture ACL: {result:?}");
            }
        }
    }
    let root = f.root.clone();
    let restore_acl = RestoreAcl(root.clone());
    trusted_directory_tree(&f.root, false).unwrap();
    assert!(
        Command::new("/bin/chmod")
            .args(["+a", "everyone deny delete"])
            .arg(&f.root)
            .status()
            .unwrap()
            .success()
    );
    trusted_directory_tree(&f.root, false).unwrap();
    assert!(
        Command::new("/bin/chmod")
            .args([
                "+a",
                "everyone allow add_file,add_subdirectory,delete_child"
            ])
            .arg(&f.root)
            .status()
            .unwrap()
            .success()
    );
    assert!(trusted_directory_tree(&f.root, false).is_err());
    drop(restore_acl);
    drop(f);
    assert!(
        !root.exists(),
        "the ACL fixture must be removed after the test"
    );
}

#[cfg_attr(test, test)]
pub fn hosted_security_sticky_owned_output_and_unshared_source_directories() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let parent = f.root.join("sticky");
    fs::create_dir(&parent).unwrap();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o1777)).unwrap();
    let stage = Scratch::new_output(&parent).unwrap();
    assert_eq!(fs::metadata(&stage.0).unwrap().mode() & 0o777, 0o700);
    assert!(
        trusted_directory_tree(&parent, false).is_err(),
        "sticky source allows inserting assets"
    );
    let bundle = f.bundle();
    fs::set_permissions(bundle.join("frames"), fs::Permissions::from_mode(0o777)).unwrap();
    let stage = Scratch::new(&f.client.state).unwrap();
    assert!(snapshot(&bundle, &stage.0).is_err());
}

#[cfg_attr(test, test)]
pub fn hosted_no_historical_scan_private_credentials_and_explicit_read_required() {
    let f = Fixture::new();
    let bundle = f.bundle();
    f.authorize("http://127.0.0.1:1");
    assert!(f.client.jobs().unwrap().is_empty());
    fs::remove_file(f.client.state.join("read.credential")).unwrap();
    assert!(
        f.client
            .fetch(&"a".repeat(32), &f.root.join("fetch.apparent"), 0)
            .is_err()
    );
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(
        f.client.state.join("upload.credential"),
        fs::Permissions::from_mode(0o644),
    )
    .unwrap();
    assert!(f.client.enqueue(&bundle).is_err());
    assert_eq!(fs::read(bundle.join("audio.wav")).unwrap(), b"audio");
}

pub fn connect_alias_scenario(run: impl Fn(&str, &Path)) {
    for name in ["connect", "cloud"] {
        let f = Fixture::new();
        let server = Server::new("");
        f.authorize(&server.url);
        let bundle = f.bundle();
        f.client.enqueue(&bundle).unwrap();
        let credential = fs::read(f.client.state.join("upload.credential")).unwrap();
        run(name, &f.client.config);
        let raw: Value = serde_json::from_slice(&fs::read(&f.client.config).unwrap()).unwrap();
        assert_eq!(raw["cloud"]["enabled"], false);
        assert_eq!(raw["cloud"]["uploadConsent"], false);
        assert_eq!(raw["unrelated"]["kept"], true);
        assert!(raw.get("connect").is_none());
        assert_eq!(f.client.jobs().unwrap()[0].status, "cancelled");
        assert_eq!(
            fs::read(f.client.state.join("upload.credential")).unwrap(),
            credential
        );
        assert_eq!(server.state.lock().unwrap().requests, 0);
    }
}

struct MutableProvider {
    inner: Arc<dyn ConfigProvider>,
    sharing: AtomicBool,
    sync: AtomicBool,
    fail_write: AtomicBool,
}
impl ConfigProvider for MutableProvider {
    fn config_path(&self) -> &Path {
        self.inner.config_path()
    }
    fn cloud(&self) -> CloudConfig {
        self.inner.cloud()
    }
    fn experiment(&self, name: &str) -> bool {
        match name {
            HOSTED_SYNC => self.sync.load(Ordering::SeqCst),
            HOSTED_SHARING => self.sharing.load(Ordering::SeqCst),
            _ => false,
        }
    }
    fn set_cloud(&self, cloud: &CloudConfig) -> Result<()> {
        ensure!(
            !self.fail_write.load(Ordering::SeqCst),
            "synthetic provider write failure"
        );
        self.inner.set_cloud(cloud)
    }
    fn set_experiment(&self, name: &str, enabled: bool) -> Result<()> {
        self.inner.set_experiment(name, enabled)
    }
}
fn mutable_client(f: &Fixture) -> (Client, Arc<MutableProvider>) {
    let provider = Arc::new(MutableProvider {
        inner: f.client.provider.clone(),
        sharing: AtomicBool::new(true),
        sync: AtomicBool::new(true),
        fail_write: AtomicBool::new(false),
    });
    (Client::new(provider.clone()), provider)
}

#[cfg_attr(test, test)]
pub fn hosted_constructed_client_and_prepared_link_read_current_provider() {
    let f = Fixture::new();
    let server = Server::new("");
    f.authorize(&server.url);
    let bundle = f.bundle();
    let (client, provider) = mutable_client(&f);
    let link = client.prepare_link(&bundle, &|| true).unwrap();
    assert!(link.authorized());
    provider.sharing.store(false, Ordering::SeqCst);
    assert!(!link.authorized());
    assert!(client.prepare_link(&bundle, &|| true).is_err());
    provider.sharing.store(true, Ordering::SeqCst);
    assert!(link.authorized());
    provider.sync.store(false, Ordering::SeqCst);
    assert!(!client.enabled());
    assert!(!link.authorized());
    let before = server.state.lock().unwrap().requests;
    client.process_once().unwrap();
    assert_eq!(server.state.lock().unwrap().requests, before);
    provider.sync.store(true, Ordering::SeqCst);
    assert!(link.authorized());
    // Same constructed client sees the provider's durable disable, not a snapshot.
    client.disable(false).unwrap();
    assert!(!client.enabled());
    assert!(!link.authorized());
    assert!(client.enqueue(&bundle).is_err());
    let stopped = server.state.lock().unwrap().requests;
    client.process_once().unwrap();
    assert_eq!(server.state.lock().unwrap().requests, stopped);
    assert_eq!(fs::read(bundle.join("audio.wav")).unwrap(), b"audio");
}

#[cfg_attr(test, test)]
pub fn hosted_provider_write_failure_never_reports_disabled_or_login_success() {
    let f = Fixture::new();
    let server = Server::new("upload");
    f.authorize(&server.url);
    let bundle = f.bundle();
    let (client, provider) = mutable_client(&f);
    let link = client.prepare_link(&bundle, &|| true).unwrap();
    let before = fs::read(&client.config).unwrap();
    provider.fail_write.store(true, Ordering::SeqCst);
    let error = client.disable(false).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("synthetic provider write failure")
    );
    assert_eq!(fs::read(&client.config).unwrap(), before);
    assert_eq!(client.jobs().unwrap()[0].status, "pending");
    assert!(
        link.authorized(),
        "failed disable must not pretend durable revocation"
    );
    provider.fail_write.store(false, Ordering::SeqCst);
    client.disable(false).unwrap();
    assert!(!link.authorized());
    let disabled = client.settings();
    provider.fail_write.store(true, Ordering::SeqCst);
    assert!(client.login(&server.url, "upload", true, false).is_err());
    assert_eq!(client.settings(), disabled);
    assert!(!client.enabled());
    assert!(!link.authorized());
    assert_eq!(client.jobs().unwrap()[0].status, "cancelled");
    assert!(server.state.lock().unwrap().data.is_empty());
    assert_eq!(fs::read(bundle.join("audio.wav")).unwrap(), b"audio");
}
