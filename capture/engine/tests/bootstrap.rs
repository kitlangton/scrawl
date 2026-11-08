use apparent_capture_engine::{LeaseKind, Runtime, new_id};
use std::{
    fs,
    io::{BufRead, BufReader, Write},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    time::Duration,
};
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("engine-bootstrap-{}", new_id().unwrap()));
        fs::create_dir(&path).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
struct Worker {
    child: Child,
    lines: Receiver<String>,
}
impl Worker {
    fn spawn(root: &Path, bundle: &str) -> Self {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "bootstrap_worker", "--nocapture"])
            .env("ENGINE_BOOTSTRAP_ROOT", root)
            .env("ENGINE_BOOTSTRAP_BUNDLE", bundle)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let line = line.unwrap();
                if ["GATED", "HELD", "BUSY", "DENIED"].contains(&line.trim()) {
                    let _ = tx.send(line.trim().to_owned());
                }
            }
        });
        Self { child, lines }
    }
    fn next(&self) -> String {
        self.lines
            .recv_timeout(Duration::from_secs(10))
            .expect("worker deadline")
    }
    fn release_gate(&mut self) {
        self.child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(b"go\n")
            .unwrap();
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
#[test]
fn bootstrap_worker() {
    let Ok(root) = std::env::var("ENGINE_BOOTSTRAP_ROOT") else {
        return;
    };
    let bundle = std::env::var("ENGINE_BOOTSTRAP_BUNDLE").unwrap();
    println!("GATED");
    std::io::stdout().flush().unwrap();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
    let runtime = Runtime::open(Path::new(&root)).unwrap();
    let result = runtime.lease(&bundle, LeaseKind::Maintenance);
    println!(
        "{}",
        match &result {
            Ok(Some(_)) => "HELD",
            Ok(None) => "BUSY",
            Err(_) => "DENIED",
        }
    );
    std::io::stdout().flush().unwrap();
    // Keep every successful lock alive until the parent has both outcomes.
    std::io::stdin().read_line(&mut line).unwrap();
    drop(result);
}
fn race(root: &Path, bundle: &str, first_bootstrap: bool) {
    let mut a = Worker::spawn(root, bundle);
    let mut b = Worker::spawn(root, bundle);
    assert_eq!(a.next(), "GATED");
    assert_eq!(b.next(), "GATED");
    a.release_gate();
    b.release_gate();
    let outcomes = [a.next(), b.next()];
    assert_eq!(
        outcomes.iter().filter(|s| *s == "HELD").count(),
        1,
        "{outcomes:?}"
    );
    if !first_bootstrap {
        assert!(outcomes.iter().any(|s| s == "BUSY"), "{outcomes:?}");
    } else {
        // A contender observing an incomplete initial lock/anchor pair may fail
        // closed. It must not acquire a second lock or repair the live anchor.
        assert!(
            outcomes.iter().any(|s| s == "BUSY" || s == "DENIED"),
            "{outcomes:?}"
        );
    }
    drop(a);
    drop(b);
    assert!(
        Runtime::open(root)
            .unwrap()
            .lease(bundle, LeaseKind::Maintenance)
            .unwrap()
            .is_some()
    );
}
#[test]
fn synchronized_first_bootstrap_never_splits_the_lock() {
    for _ in 0..8 {
        let f = Fixture::new();
        race(&f.0, "apparent-1793444400123-1a2b", true);
    }
}
#[test]
fn durable_bundle_identities_contend_across_processes() {
    for bundle in ["apparent-1793444400123-1a2b-3", "Customer demo — 東京"] {
        let f = Fixture::new();
        // Bootstrap only the root transaction lock, not this bundle's namespace.
        drop(
            Runtime::open(&f.0)
                .unwrap()
                .lease("warmup", LeaseKind::Read)
                .unwrap(),
        );
        race(&f.0, bundle, false);
    }
}
