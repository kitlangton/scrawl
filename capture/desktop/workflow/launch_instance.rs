//! Same-user single GUI owner. Independent of the SDK's HTTP listener.
use anyhow::{Context, Result, bail, ensure};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

static OPEN: AtomicBool = AtomicBool::new(false);

pub fn request_open() {
    OPEN.store(true, Ordering::Release);
}

pub fn take_open(onboarding_initialized: bool) -> bool {
    take_open_when_ready(&OPEN, onboarding_initialized)
}

fn take_open_when_ready(pending: &AtomicBool, onboarding_initialized: bool) -> bool {
    onboarding_initialized && pending.swap(false, Ordering::AcqRel)
}

pub fn request_url(url: &str) -> Result<()> {
    if let Some(token) = super::launch_url::hermes_request(url) {
        return super::launch_url::open_hermes(token);
    }
    ensure!(
        crate::capture::desktop::workflow::launch_url::valid(url),
        "unsupported Apparent launch URL"
    );
    ensure!(
        crate::shared::handoff::experiment("nativeLaunch"),
        "nativeLaunch experiment is disabled"
    );
    request_open();
    Ok(())
}

fn socket_path() -> Result<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("HOME").context("HOME is not set")?)
        .join(".config/apparent/launch");
    std::fs::create_dir_all(&dir)?;
    let meta = std::fs::symlink_metadata(&dir)?;
    ensure!(
        meta.is_dir() && meta.uid() == unsafe { libc::geteuid() },
        "unsafe launch directory"
    );
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    Ok(dir.join("open.sock"))
}

pub struct Owner {
    _lock: File,
    listener: UnixListener,
    path: PathBuf,
}

impl Drop for Owner {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub fn acquire() -> Result<Option<Owner>> {
    acquire_at(&socket_path()?)
}

fn acquire_at(path: &Path) -> Result<Option<Owner>> {
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path.with_extension("lock"))?;
    let meta = lock.metadata()?;
    ensure!(
        meta.is_file() && meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o777 == 0o600,
        "unsafe launch lock"
    );
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::WouldBlock {
            return Ok(None);
        }
        return Err(error.into());
    }
    if let Ok(meta) = std::fs::symlink_metadata(path) {
        ensure!(
            meta.file_type().is_socket()
                && meta.uid() == unsafe { libc::geteuid() }
                && meta.mode() & 0o777 == 0o600,
            "unsafe existing launch socket"
        );
        match connect_nonblocking(path) {
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
                std::fs::remove_file(path)?
            }
            _ => bail!("launch endpoint already in use"),
        }
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(Some(Owner {
        _lock: lock,
        listener,
        path: path.to_owned(),
    }))
}

impl Owner {
    pub fn listen(self) -> Result<()> {
        std::thread::Builder::new()
            .name("apparent-launch".into())
            .spawn(move || {
                let owner = self;
                for mut stream in owner.listener.incoming().flatten() {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(1)));
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(1)));
                    let mut bytes = Vec::new();
                    if (&mut stream).take(6).read_to_end(&mut bytes).is_ok() && bytes == b"open\n" {
                        request_open();
                        let _ = stream.write_all(b"ok\n");
                    }
                }
            })?;
        Ok(())
    }
}

/// Bound a simultaneous cold launch's wait. An unresponsive owner never permits a duplicate.
pub fn forward_open() -> Result<()> {
    forward_open_at(&socket_path()?, Instant::now() + Duration::from_secs(3))
}

fn forward_open_at(path: &Path, deadline: Instant) -> Result<()> {
    let mut stream = retry_until(deadline, || connect_nonblocking(path))?;
    let mut sent = 0;
    while sent < 5 {
        let count = retry_until(deadline, || stream.write(&b"open\n"[sent..]))?;
        ensure!(count > 0, "launch connection closed during request");
        sent += count;
    }
    stream.shutdown(std::net::Shutdown::Write)?;
    let mut reply = [0; 3];
    let mut received = 0;
    while received < reply.len() {
        let count = retry_until(deadline, || stream.read(&mut reply[received..]))?;
        ensure!(
            count > 0,
            "launch connection closed without acknowledgement"
        );
        received += count;
    }
    ensure!(&reply == b"ok\n", "launch request was not acknowledged");
    Ok(())
}

fn retry_until<T>(
    deadline: Instant,
    mut operation: impl FnMut() -> std::io::Result<T>,
) -> std::io::Result<T> {
    loop {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            return Err(std::io::ErrorKind::TimedOut.into());
        };
        match operation() {
            Ok(value) => return Ok(value),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::Interrupted
                        | std::io::ErrorKind::NotFound
                        | std::io::ErrorKind::ConnectionRefused
                ) || error.raw_os_error() == Some(libc::EINPROGRESS) =>
            {
                std::thread::sleep(remaining.min(Duration::from_millis(10)));
            }
            Err(error) => return Err(error),
        }
    }
}

// std's UnixStream::connect has no deadline. Set O_NONBLOCK before connecting,
// including during stale-endpoint probes, so a full accept queue cannot hang us.
fn connect_nonblocking(path: &Path) -> std::io::Result<UnixStream> {
    let bytes = path.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.contains(&0) || bytes.len() >= address.sun_path.len() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid launch socket path",
        ));
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (destination, byte) in address.sun_path.iter_mut().zip(bytes) {
        *destination = *byte as libc::c_char;
    }
    let length =
        (std::mem::offset_of!(libc::sockaddr_un, sun_path) + bytes.len() + 1) as libc::socklen_t;
    #[cfg(target_os = "macos")]
    {
        address.sun_len = length as u8;
    }
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // Own the descriptor immediately so every error path closes it.
    let stream = unsafe { UnixStream::from_raw_fd(fd) };
    if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    stream.set_nonblocking(true)?;
    if unsafe { libc::connect(fd, &address as *const _ as *const libc::sockaddr, length) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cold_open_waits_for_onboarding_without_losing_the_request() {
        let pending = AtomicBool::new(true);
        assert!(!take_open_when_ready(&pending, false));
        assert!(pending.load(Ordering::Acquire));
        assert!(take_open_when_ready(&pending, true));
        assert!(!take_open_when_ready(&pending, true));
    }

    #[test]
    fn forwarding_requires_the_exact_acknowledgement() {
        for reply in [b"ok\n", b"no\n"] {
            let dir = test_dir();
            let path = dir.path().join("open.sock");
            let listener = UnixListener::bind(&path).unwrap();
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = Vec::new();
                stream.read_to_end(&mut request).unwrap();
                assert_eq!(request, b"open\n");
                stream.write_all(reply).unwrap();
            });
            let result = forward_open_at(&path, Instant::now() + Duration::from_secs(1));
            server.join().unwrap();
            assert_eq!(result.is_ok(), reply == b"ok\n");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn full_accept_queue_cannot_block_forwarding_or_stale_probe() {
        let dir = test_dir();
        let path = dir.path().join("open.sock");
        let listener = UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(unsafe { libc::listen(listener.as_raw_fd(), 0) }, 0);
        let _pending = UnixStream::connect(&path).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let forward_path = path.clone();
        let forwarding = std::thread::spawn(move || {
            let result =
                forward_open_at(&forward_path, Instant::now() + Duration::from_millis(100));
            tx.send(result.is_err()).unwrap();
        });
        let outcome = rx.recv_timeout(Duration::from_millis(500));
        // Drop the listener before asserting, so the old blocking implementation can unwind.
        if outcome.is_err() {
            drop(listener);
            let _ = forwarding.join();
            panic!("forwarding exceeded its deadline with a full accept queue");
        }
        assert!(outcome.unwrap());
        forwarding.join().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let probe_path = path.clone();
        let probe = std::thread::spawn(move || {
            tx.send(acquire_at(&probe_path).is_err()).unwrap();
        });
        let outcome = rx.recv_timeout(Duration::from_millis(500));
        drop(listener);
        probe.join().unwrap();
        assert_eq!(
            outcome.unwrap(),
            true,
            "stale probe must fail closed without blocking"
        );
        assert!(path.exists(), "live endpoint must not be unlinked");
    }

    #[test]
    fn acknowledgement_wait_shares_the_forwarding_deadline() {
        let dir = test_dir();
        let path = dir.path().join("open.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            std::thread::sleep(Duration::from_millis(600));
        });
        let start = Instant::now();
        assert!(forward_open_at(&path, start + Duration::from_millis(100)).is_err());
        let elapsed = start.elapsed();
        server.join().unwrap();
        assert!(
            elapsed < Duration::from_millis(500),
            "acknowledgement exceeded deadline: {elapsed:?}"
        );
    }

    #[test]
    fn only_one_daemon_can_own_the_launch_endpoint() {
        let dir = test_dir();
        let path = dir.path().join("open.sock");
        let first = acquire_at(&path).unwrap();
        assert!(first.is_some(), "first launch must own the daemon");
        assert!(
            acquire_at(&path).unwrap().is_none(),
            "second launch must not start a daemon"
        );
        drop(first);
        assert!(
            acquire_at(&path).unwrap().is_some(),
            "owner exit must allow restart"
        );
    }

    struct TestDir(PathBuf);

    impl TestDir {
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn test_dir() -> TestDir {
        use std::os::unix::fs::DirBuilderExt;
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        // A macOS TMPDIR can exceed sockaddr_un's limit even with a short socket name.
        let root = PathBuf::from(std::env::var_os("HOME").unwrap());
        loop {
            let path = root.join(format!(
                "al-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return TestDir(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("test directory: {error}"),
            }
        }
    }

    #[test]
    fn concurrent_launchers_leave_one_reachable_owner() {
        let dir = test_dir();
        let path = dir.path().join("open.sock");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    let owner = acquire_at(&path).unwrap();
                    barrier.wait();
                    let won = owner.is_some();
                    if won {
                        assert!(UnixStream::connect(path).is_ok());
                    }
                    barrier.wait();
                    won
                })
            })
            .collect();
        assert_eq!(
            threads
                .into_iter()
                .map(|t| usize::from(t.join().unwrap()))
                .sum::<usize>(),
            1
        );
    }

    #[test]
    fn losing_launcher_does_not_unlink_a_refused_socket() {
        let dir = test_dir();
        let path = dir.path().join("open.sock");
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path.with_extension("lock"))
            .unwrap();
        assert_eq!(
            unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
            0
        );
        drop(UnixListener::bind(&path).unwrap());
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let inode = std::fs::symlink_metadata(&path).unwrap().ino();
        assert_eq!(
            UnixStream::connect(&path).unwrap_err().kind(),
            std::io::ErrorKind::ConnectionRefused
        );
        assert!(acquire_at(&path).unwrap().is_none());
        assert_eq!(std::fs::symlink_metadata(&path).unwrap().ino(), inode);
        drop(lock);
    }

    #[test]
    fn stale_socket_is_reclaimed_but_symlinks_and_foreign_files_are_not() {
        let dir = test_dir();
        let path = dir.path().join("open.sock");
        let stale = UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        drop(stale);
        drop(acquire_at(&path).unwrap().unwrap());
        std::fs::write(&path, "not a socket").unwrap();
        assert!(acquire_at(&path).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "not a socket");
        std::fs::remove_file(&path).unwrap();
        std::fs::remove_file(path.with_extension("lock")).unwrap();
        std::os::unix::fs::symlink(dir.path().join("outside"), path.with_extension("lock"))
            .unwrap();
        assert!(acquire_at(&path).is_err());
        assert!(!dir.path().join("outside").exists());
    }

    #[test]
    fn only_valid_enabled_urls_enqueue_an_open() {
        assert!(!take_open(true));
        assert!(request_url("apparent://open").is_err());
        assert!(!take_open(true));
        crate::shared::handoff::enable_experiment("nativeLaunch");
        assert!(request_url("apparent://record").is_err());
        assert!(!take_open(true));
        request_url("apparent://open").unwrap();
        assert!(!take_open(false));
        assert!(take_open(true));
        assert!(!take_open(true));
    }
}
