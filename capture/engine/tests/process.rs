use apparent_capture_engine::{Admission, Control, LeaseKind, Runtime, Status, new_id};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
const TOKEN: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("engine-{}", new_id().unwrap()));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        Self(path)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn fixture_roots_are_private_distinct_and_independently_owned() {
    let first = Fixture::new();
    let first_path = first.0.clone();
    let sentinel = first.0.join("existing");
    std::fs::write(&sentinel, b"untouched").unwrap();
    let second = Fixture::new();
    let second_path = second.0.clone();
    assert_ne!(first_path, second_path);
    for path in [&first_path, &second_path] {
        assert_eq!(
            std::fs::metadata(path).unwrap().permissions().mode() & 0o777,
            0o700
        );
    }
    assert!(std::fs::read_dir(&second_path).unwrap().next().is_none());
    drop(second);
    assert!(!second_path.exists());
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"untouched");
    drop(first);
    assert!(!first_path.exists());
}

struct Worker(Child);
impl Worker {
    fn start(path: &Path, role: &str) -> Self {
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "worker", "--nocapture"])
            .env("ENGINE_ROOT", path)
            .env("ENGINE_ROLE", role)
            .stdout(Stdio::piped())
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if line.unwrap().trim() == "READY" {
                    let _ = tx.send(());
                }
            }
        });
        rx.recv_timeout(std::time::Duration::from_secs(10))
            .expect("worker did not acquire lock");
        Self(child)
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
#[test]
fn worker() {
    let Ok(root) = std::env::var("ENGINE_ROOT") else {
        return;
    };
    let runtime = Runtime::open(Path::new(&root)).unwrap();
    let role = std::env::var("ENGINE_ROLE").unwrap();
    if role == "write-failure" {
        // Only this disposable child inherits the limit/disposition. Exercise
        // the real record writer's write_all failure, not a mock filesystem.
        unsafe {
            libc::signal(libc::SIGXFSZ, libc::SIG_IGN);
            let limit = libc::rlimit {
                rlim_cur: 0,
                rlim_max: 0,
            };
            assert_eq!(libc::setrlimit(libc::RLIMIT_FSIZE, &limit), 0);
        }
        assert!(runtime.admit(A, TOKEN, B).is_err());
        println!("READY");
        std::io::stdout().flush().unwrap();
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).unwrap();
        return;
    }
    let (mut session, _lease) = match role.as_str() {
        "read" | "write" | "finalize" | "cleanup" => {
            let kind = match role.as_str() {
                "read" => LeaseKind::Read,
                "write" => LeaseKind::Write,
                "finalize" => LeaseKind::Finalize,
                _ => LeaseKind::Maintenance,
            };
            (None, Some(runtime.lease(B, kind).unwrap().unwrap()))
        }
        _ => {
            let Admission::Acquired(guard) = runtime.admit(A, TOKEN, B).unwrap() else {
                panic!("not admitted")
            };
            (Some(guard), None)
        }
    };
    let _forked = if role == "fork" {
        Some(ForkKeeper::new())
    } else {
        None
    };
    let mut exec_child = if role == "exec" {
        Some(
            Command::new("/bin/sh")
                .args(["-c", "read line"])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .spawn()
                .unwrap(),
        )
    } else {
        None
    };
    if role == "fork" || role == "exec" {
        drop(session.take());
    }
    println!("READY");
    std::io::stdout().flush().unwrap();
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).unwrap();
    if let Some(ref mut child) = exec_child {
        child.kill().unwrap();
        child.wait().unwrap();
    }
    if ["save", "fail", "cancel", "mismatch", "proof"].contains(&role.as_str()) {
        let mut session = session.unwrap();
        session.recording().unwrap();
        session.processing().unwrap();
        let output = Path::new(&root).join("output.apparent");
        std::fs::create_dir(&output).unwrap();
        std::fs::set_permissions(&output, std::fs::Permissions::from_mode(0o700)).unwrap();
        let manifest = format!("{{\"version\":1,\"id\":\"{B}\"}}");
        std::fs::write(output.join("apparent.json"), &manifest).unwrap();
        std::fs::set_permissions(
            output.join("apparent.json"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        let result = if role == "fail" {
            Err(())
        } else {
            Ok(apparent_capture_engine::FinalizerSuccess::validate(
                if role == "mismatch" { B } else { A },
                B,
                &output,
                manifest.as_bytes(),
            )
            .unwrap())
        };
        if role == "proof" {
            assert!(result.is_ok());
            std::fs::write(Path::new(&root).join("proof-ready"), b"validated").unwrap();
            std::io::stdin().read_line(&mut line).unwrap();
        }
        session.finish(result).unwrap();
        if role == "save" {
            std::io::stdin().read_line(&mut line).unwrap();
        }
    }
}
#[test]
fn competing_admission_and_killed_owner() {
    let f = Fixture::new();
    let owner = Worker::start(&f.0, "admit");
    let runtime = Runtime::open(&f.0).unwrap();
    assert!(matches!(
        runtime.admit(B, TOKEN, A).unwrap(),
        Admission::Existing
    ));
    drop(owner);
    // The rejected request is a tombstone; only a new request can acquire.
    assert!(matches!(
        runtime.admit(B, TOKEN, A).unwrap(),
        Admission::Existing
    ));
    assert!(matches!(
        runtime
            .admit("dddddddddddddddddddddddddddddddd", TOKEN, A)
            .unwrap(),
        Admission::Acquired(_)
    ));
}

#[test]
fn failed_record_write_does_not_leave_temporary_files() {
    for existing in [false, true] {
        let f = Fixture::new();
        if existing {
            drop(Runtime::open(&f.0).unwrap().admit(A, TOKEN, B).unwrap());
        }
        let record = f.0.join(format!("request-{A}.json"));
        let before = std::fs::read(&record).ok();
        let worker = Worker::start(&f.0, "write-failure");
        let names: Vec<_> = std::fs::read_dir(&f.0)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert!(
            !names
                .iter()
                .any(|name| name.to_string_lossy().starts_with("tmp-")),
            "failed record leaked a temporary file: {names:?}"
        );
        assert_eq!(
            std::fs::read(record).ok(),
            before,
            "failed write replaced durable bytes"
        );
        drop(worker);
    }
}

#[test]
fn durable_identity_and_interrupted_receipt() {
    let f = Fixture::new();
    let owner = Worker::start(&f.0, "admit");
    let runtime = Runtime::open(&f.0).unwrap();
    assert_eq!(
        runtime.inspect(A).unwrap().unwrap().status,
        Status::Starting
    );
    assert!(
        runtime
            .request_control(A, &"e".repeat(64), Control::Cancel)
            .is_err()
    );
    assert!(runtime.request_control(B, TOKEN, Control::Cancel).is_err());
    assert!(runtime.admit(A, TOKEN, A).is_err());
    assert!(matches!(
        runtime.admit(A, TOKEN, B).unwrap(),
        Admission::Existing
    ));
    assert_eq!(
        runtime.inspect(A).unwrap().unwrap().status,
        Status::Starting
    );
    drop(owner);
    let receipt = Runtime::open(&f.0).unwrap().inspect(A).unwrap().unwrap();
    assert_eq!(receipt.status, Status::Failed);
    assert_eq!(receipt.error.as_deref(), Some("owner_interrupted"));
    assert!(receipt.bundle_path.is_none());
    assert!(matches!(
        runtime.admit(A, TOKEN, B).unwrap(),
        Admission::Existing
    ));
    assert_eq!(runtime.inspect(A).unwrap().unwrap(), receipt);
}

#[test]
fn unauthorized_retry_must_not_mutate_dead_owner_receipt() {
    let f = Fixture::new();
    let owner = Worker::start(&f.0, "admit");
    drop(owner);
    let path = f.0.join(format!("request-{A}.json"));
    let before = std::fs::read(&path).unwrap();
    assert!(
        Runtime::open(&f.0)
            .unwrap()
            .admit(A, &"e".repeat(64), B)
            .is_err()
    );
    assert!(
        std::fs::read(path).unwrap() == before,
        "unauthorized retry changed the receipt"
    );
}

#[test]
fn bundle_leases_exclude_cleanup_across_processes_and_rename() {
    for role in ["read", "write", "finalize", "admit"] {
        let f = Fixture::new();
        let bundle = f.0.join("old.apparent");
        std::fs::create_dir(&bundle).unwrap();
        let worker = Worker::start(&f.0, role);
        let runtime = Runtime::open(&f.0).unwrap();
        assert!(runtime.lease(B, LeaseKind::Maintenance).unwrap().is_none());
        std::fs::rename(&bundle, f.0.join("new.apparent")).unwrap();
        assert!(runtime.lease(B, LeaseKind::Maintenance).unwrap().is_none());
        drop(worker);
        assert!(runtime.lease(B, LeaseKind::Maintenance).unwrap().is_some());
    }
    let f = Fixture::new();
    let cleaner = Worker::start(&f.0, "cleanup");
    let runtime = Runtime::open(&f.0).unwrap();
    assert!(runtime.lease(B, LeaseKind::Read).unwrap().is_none());
    assert!(runtime.lease(B, LeaseKind::Write).unwrap().is_none());
    drop(cleaner);
}

#[test]
fn exact_finalizer_and_terminal_receipts_survive_process_exit() {
    let f = Fixture::new();
    let mut owner = Worker::start(&f.0, "save");
    owner
        .0
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"finish\n")
        .unwrap();
    let runtime = Runtime::open(&f.0).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let receipt = loop {
        let receipt = runtime.inspect(A).unwrap().unwrap();
        if receipt.status == Status::Saved {
            break receipt;
        }
        assert!(std::time::Instant::now() < deadline, "finalizer deadline");
        std::thread::sleep(std::time::Duration::from_millis(1));
    };
    drop(owner); // SIGKILL after durable Saved, not merely a clean exit.
    assert_eq!(runtime.inspect(A).unwrap().unwrap(), receipt);
    assert_eq!(receipt.status, Status::Saved);
    assert_eq!(receipt.bundle_path, Some(f.0.join("output.apparent")));
    assert_eq!(
        runtime.request_control(A, TOKEN, Control::Cancel).unwrap(),
        receipt
    );
    assert!(matches!(
        runtime.admit(A, TOKEN, B).unwrap(),
        Admission::Existing
    ));
    for role in ["fail", "cancel", "mismatch"] {
        let f = Fixture::new();
        let mut owner = Worker::start(&f.0, role);
        if role == "cancel" {
            Runtime::open(&f.0)
                .unwrap()
                .request_control(A, TOKEN, Control::Cancel)
                .unwrap();
        }
        owner
            .0
            .stdin
            .as_mut()
            .unwrap()
            .write_all(b"finish\n")
            .unwrap();
        assert!(owner.0.wait().unwrap().success());
        let receipt = Runtime::open(&f.0).unwrap().inspect(A).unwrap().unwrap();
        assert_eq!(
            receipt.status,
            if role == "cancel" {
                Status::Cancelled
            } else {
                Status::Failed
            }
        );
        assert!(receipt.bundle_path.is_none());
    }
}

#[test]
fn crash_after_validated_output_does_not_infer_saved() {
    let f = Fixture::new();
    let mut owner = Worker::start(&f.0, "proof");
    owner
        .0
        .stdin
        .as_mut()
        .unwrap()
        .write_all(b"finalize\n")
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !f.0.join("proof-ready").exists() {
        assert!(std::time::Instant::now() < deadline, "proof deadline");
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    drop(owner); // Kill after exact proof validation, before terminal publication.
    let runtime = Runtime::open(&f.0).unwrap();
    let receipt = runtime.inspect(A).unwrap().unwrap();
    assert_eq!(receipt.status, Status::Failed);
    assert_eq!(receipt.error.as_deref(), Some("owner_interrupted"));
    assert!(receipt.bundle_path.is_none());
    assert!(f.0.join("output.apparent/apparent.json").exists());
    assert_eq!(runtime.inspect(A).unwrap().unwrap(), receipt);
}

#[test]
fn replaced_or_symlinked_locks_fail_closed_without_touching_target() {
    use std::os::unix::fs::symlink;
    for key in [
        "metadata.lock",
        "recording.lock",
        "bundle-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb.lock",
    ] {
        for symlinked in [false, true] {
            let f = Fixture::new();
            let owner = Worker::start(&f.0, "admit");
            let path = f.0.join(key);
            std::fs::rename(&path, f.0.join("held-inode")).unwrap();
            let outside = Fixture::new();
            let target = outside.0.join("target");
            std::fs::write(&target, b"untouched").unwrap();
            if symlinked {
                symlink(&target, &path).unwrap();
            } else {
                std::fs::write(&path, b"").unwrap();
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
            }
            let runtime = Runtime::open(&f.0).unwrap();
            let denied = if key.starts_with("bundle") {
                runtime.lease(B, LeaseKind::Maintenance).is_err()
            } else {
                runtime.admit(B, TOKEN, A).is_err()
            };
            assert!(denied);
            assert_eq!(std::fs::read(target).unwrap(), b"untouched");
            drop(owner);
        }
    }
}

#[test]
fn unsafe_runtime_and_receipt_identity_are_refused() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let outside = Fixture::new();
    let alias = outside.0.join("alias");
    symlink(&f.0, &alias).unwrap();
    assert!(Runtime::open(&alias).is_err());
    std::fs::set_permissions(&f.0, std::fs::Permissions::from_mode(0o770)).unwrap();
    assert!(Runtime::open(&f.0).is_err());
    std::fs::set_permissions(&f.0, std::fs::Permissions::from_mode(0o700)).unwrap();
    let owner = Worker::start(&f.0, "admit");
    let receipt = f.0.join(format!("request-{A}.json"));
    let mut value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&receipt).unwrap()).unwrap();
    value["receipt"]["request_id"] = B.into();
    std::fs::write(&receipt, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(Runtime::open(&f.0).unwrap().inspect(A).is_err());
    drop(owner);
}

// The fork child runs ONLY async-signal-safe libc calls after fork, never the Rust harness.
struct ForkKeeper {
    pid: i32,
    write: i32,
}
impl ForkKeeper {
    fn new() -> Self {
        unsafe {
            let mut pipe = [0; 2];
            assert_eq!(libc::pipe(pipe.as_mut_ptr()), 0);
            let pid = libc::fork();
            assert!(pid >= 0);
            if pid == 0 {
                libc::close(pipe[1]);
                libc::signal(libc::SIGALRM, libc::SIG_DFL);
                libc::alarm(10);
                let mut byte = 0u8;
                libc::read(pipe[0], (&mut byte as *mut u8).cast(), 1);
                libc::_exit(0);
            }
            libc::close(pipe[0]);
            Self {
                pid,
                write: pipe[1],
            }
        }
    }
}
impl Drop for ForkKeeper {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.write);
            libc::waitpid(self.pid, std::ptr::null_mut(), 0);
        }
    }
}

#[test]
fn exec_closes_descriptors_but_forked_copies_hold_locks_until_last_close() {
    for role in ["exec", "fork"] {
        let f = Fixture::new();
        let mut worker = Worker::start(&f.0, role);
        let runtime = Runtime::open(&f.0).unwrap();
        let result = runtime.admit(B, TOKEN, A).unwrap();
        assert_eq!(matches!(result, Admission::Acquired(_)), role == "exec");
        drop(result);
        worker
            .0
            .stdin
            .as_mut()
            .unwrap()
            .write_all(b"exit\n")
            .unwrap();
        assert!(worker.0.wait().unwrap().success());
        assert!(matches!(
            runtime
                .admit("dddddddddddddddddddddddddddddddd", TOKEN, A)
                .unwrap(),
            Admission::Acquired(_)
        ));
        assert_eq!(runtime.inspect(A).unwrap().unwrap().status, Status::Failed);
    }
}
