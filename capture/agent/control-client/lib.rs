use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Start,
    Inspect,
    Stop,
    Cancel,
    DrawingRead,
    DrawingUndo,
    ReferenceRead,
    ReferenceCopy,
    PublicShareStart,
    PublicShareRead,
    PublicShareDeliver,
    PublicShareCancel,
}

pub fn new_id() -> Result<String> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

pub fn socket_path(create: bool) -> Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    let dir = PathBuf::from(home).join(".config/apparent/agent-control");
    if create {
        std::fs::create_dir_all(&dir)?;
    }
    let meta = std::fs::symlink_metadata(&dir)?;
    ensure!(
        meta.is_dir() && meta.uid() == unsafe { libc::geteuid() },
        "unsafe control directory"
    );
    if create {
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
    }
    ensure!(
        std::fs::metadata(&dir)?.permissions().mode() & 0o077 == 0,
        "control directory is not private"
    );
    Ok(dir.join("capture.sock"))
}

pub fn same_user(stream: &UnixStream) -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        let rc = unsafe {
            libc::getsockopt(
                stream.as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_PEERCRED,
                &mut cred as *mut _ as *mut _,
                &mut len,
            )
        };
        ensure!(
            rc == 0
                && len as usize == std::mem::size_of::<libc::ucred>()
                && cred.uid == unsafe { libc::geteuid() },
            "control peer is not this user"
        );
    }
    #[cfg(target_os = "macos")]
    {
        let mut uid = 0;
        let mut gid = 0;
        let rc = unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
        ensure!(
            rc == 0 && uid == unsafe { libc::geteuid() },
            "control peer is not this user"
        );
    }
    Ok(())
}

pub fn clock_ms() -> u64 {
    let mut time: libc::timespec = unsafe { std::mem::zeroed() };
    // CLOCK_MONOTONIC is available on both supported Unix targets.
    assert_eq!(
        unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut time) },
        0
    );
    time.tv_sec as u64 * 1000 + time.tv_nsec as u64 / 1_000_000
}

pub fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

pub fn command(action: Option<(Action, &str)>) -> Result<String> {
    command_for(action, Duration::from_secs(4))
}

fn deadline_error() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::TimedOut,
        "GUI capture control deadline expired",
    )
}

fn remaining_until(deadline: Instant) -> std::io::Result<Duration> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        Err(deadline_error())
    } else {
        Ok(remaining)
    }
}

fn wait_ready(fd: libc::c_int, events: libc::c_short, deadline: Instant) -> std::io::Result<()> {
    loop {
        let remaining = remaining_until(deadline)?;
        let mut descriptor = libc::pollfd {
            fd,
            events,
            revents: 0,
        };
        let millis = remaining
            .as_millis()
            .saturating_add(1)
            .min(i32::MAX as u128) as i32;
        let result = unsafe { libc::poll(&mut descriptor, 1, millis) };
        if result > 0 {
            remaining_until(deadline)?;
            return Ok(());
        }
        if result < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            return Err(std::io::Error::last_os_error());
        }
    }
}

/// Nonblocking from socket creation onward. Linux reports a full Unix accept queue as
/// EAGAIN (not EINPROGRESS); retry with a fresh socket without leaving a worker behind.
pub fn connect_until(path: &std::path::Path, deadline: Instant) -> std::io::Result<UnixStream> {
    let bytes = path.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.len() >= address.sun_path.len() || bytes.contains(&0) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid control socket path",
        ));
    }
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    for (dest, source) in address.sun_path.iter_mut().zip(bytes) {
        *dest = *source as libc::c_char;
    }
    let length =
        (std::mem::offset_of!(libc::sockaddr_un, sun_path) + bytes.len() + 1) as libc::socklen_t;
    #[cfg(target_os = "macos")]
    {
        address.sun_len = length as u8;
    }
    loop {
        remaining_until(deadline)?;
        #[cfg(target_os = "linux")]
        let flags = libc::SOCK_STREAM | libc::SOCK_CLOEXEC;
        #[cfg(not(target_os = "linux"))]
        let flags = libc::SOCK_STREAM;
        let fd = unsafe { libc::socket(libc::AF_UNIX, flags, 0) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // Own the descriptor immediately, including every error path below.
        let stream = unsafe { UnixStream::from_raw_fd(fd) };
        #[cfg(target_os = "macos")]
        {
            if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
                return Err(std::io::Error::last_os_error());
            }
            let enabled: libc::c_int = 1;
            if unsafe {
                libc::setsockopt(
                    fd,
                    libc::SOL_SOCKET,
                    libc::SO_NOSIGPIPE,
                    (&enabled as *const libc::c_int).cast(),
                    std::mem::size_of_val(&enabled) as libc::socklen_t,
                )
            } < 0
            {
                return Err(std::io::Error::last_os_error());
            }
        }
        stream.set_nonblocking(true)?;
        if unsafe { libc::connect(fd, (&address as *const libc::sockaddr_un).cast(), length) } == 0
        {
            return Ok(stream);
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EINPROGRESS) {
            wait_ready(fd, libc::POLLOUT, deadline)?;
            if let Some(error) = stream.take_error()? {
                return Err(error);
            }
            return Ok(stream);
        }
        if !matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
        ) {
            return Err(error);
        }
        drop(stream);
        std::thread::sleep(Duration::from_millis(2).min(remaining_until(deadline)?));
    }
}

/// One absolute deadline covers all partial reads/writes, not one timeout per byte.
pub struct DeadlineStream {
    pub stream: UnixStream,
    pub deadline: Instant,
}

impl Read for DeadlineStream {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        loop {
            wait_ready(self.stream.as_raw_fd(), libc::POLLIN, self.deadline)?;
            match self.stream.read(bytes) {
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                result => return result,
            }
        }
    }
}

impl Write for DeadlineStream {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        loop {
            wait_ready(self.stream.as_raw_fd(), libc::POLLOUT, self.deadline)?;
            match self.stream.write(bytes) {
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted
                    ) => {}
                result => return result,
            }
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn command_for(action: Option<(Action, &str)>, budget: Duration) -> Result<String> {
    if let Some((_, id)) = action {
        ensure!(valid_id(id), "capture ID must be 32 hex characters");
    }
    let timeout = budget
        .min(Duration::from_secs(4))
        .max(Duration::from_millis(1));
    let deadline_ms = clock_ms() + timeout.as_millis() as u64;
    let path = socket_path(false)?;
    command_at(action, timeout, deadline_ms, &path)
}

pub fn command_at(
    action: Option<(Action, &str)>,
    timeout: Duration,
    deadline_ms: u64,
    path: &std::path::Path,
) -> Result<String> {
    let deadline = Instant::now()
        + timeout.min(Duration::from_millis(
            deadline_ms.saturating_sub(clock_ms()),
        ));
    let meta = std::fs::symlink_metadata(&path).context("no running GUI control socket")?;
    ensure!(
        meta.file_type().is_socket()
            && meta.uid() == unsafe { libc::geteuid() }
            && meta.mode() & 0o777 == 0o600,
        "unsafe control socket"
    );
    let stream = connect_until(path, deadline).context("GUI capture control unavailable")?;
    same_user(&stream)?;
    let mut stream = DeadlineStream { stream, deadline };
    let request = match action {
        None => "state".to_owned(),
        Some((action, id)) => format!(
            "{} {id} {deadline_ms}",
            match action {
                Action::Start => "start",
                Action::Inspect => "inspect",
                Action::Stop => "stop",
                Action::Cancel => "cancel",
                Action::DrawingRead
                | Action::DrawingUndo
                | Action::ReferenceRead
                | Action::ReferenceCopy
                | Action::PublicShareStart
                | Action::PublicShareRead
                | Action::PublicShareDeliver
                | Action::PublicShareCancel => bail!("Typed GUI actions require the GUI protocol"),
            }
        ),
    };
    writeln!(stream, "{request}")?;
    let mut response = String::new();
    BufReader::new(stream).take(4096).read_line(&mut response)?;
    ensure!(
        !response.is_empty(),
        "GUI capture control closed without a reply"
    );
    Ok(response.trim().to_owned())
}

/// One mutation, then read-only polling by the original ID. Never infer ownership
/// from global state and never resend Stop as a toggle after a timeout.
pub fn execute(action: Option<(Action, &str)>, wait: bool, timeout: Duration) -> serde_json::Value {
    execute_with(action, wait, timeout, command_for)
}

pub fn execute_with(
    action: Option<(Action, &str)>,
    wait: bool,
    timeout: Duration,
    mut send: impl FnMut(Option<(Action, &str)>, Duration) -> Result<String>,
) -> serde_json::Value {
    let deadline = Instant::now() + timeout;
    let id = action.map(|(_, id)| id);
    let mut next = action;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return serde_json::json!({"capture_id": id, "error": "timeout", "message": "Inspect the same ID; an admitted capture is not cancelled on timeout."});
        }
        let response = send(next, remaining)
            .and_then(|text| Ok(serde_json::from_str::<serde_json::Value>(&text)?));
        if Instant::now() >= deadline
            || response.as_ref().err().is_some_and(|error| {
                error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::TimedOut)
            })
        {
            return serde_json::json!({"capture_id": id, "error": "timeout", "message": "Inspect the same ID; an admitted capture is not cancelled on timeout."});
        }
        let mut value = match response {
            Ok(value) => value,
            Err(error) => {
                return serde_json::json!({"capture_id": id, "error": "control_unavailable", "message": format!("{error:#}"), "recovery": "Inspect the same ID before retrying; admission may have succeeded."});
            }
        };
        if !value.is_object() {
            return serde_json::json!({"capture_id": id, "error": "invalid_response"});
        }
        if value["error"].is_string() {
            // Listener/transport errors may not yet have reached the GUI thread.
            value["capture_id"] = serde_json::json!(id);
            return value;
        }
        if let Some((action, id)) = action {
            // A response for a different resource is never a successful receipt.
            let status = value["status"].as_str();
            if value["capture_id"].as_str() != Some(id)
                || !matches!(
                    status,
                    Some(
                        "starting"
                            | "recording"
                            | "processing"
                            | "saved"
                            | "cancelled"
                            | "failed"
                            | "rejected"
                            | "unknown"
                    )
                )
                || (status == Some("saved")
                    && !value["bundle_path"].as_str().is_some_and(|p| !p.is_empty()))
            {
                return serde_json::json!({"capture_id": id, "error": "invalid_response"});
            }
            if !wait
                || matches!(
                    value["outcome"].as_str(),
                    Some("wrong_capture" | "not_ready")
                )
            {
                return value;
            }
            if (action == Action::Start && status == Some("recording"))
                || (action == Action::Stop
                    && status == Some("saved")
                    && value["bundle_path"].is_string())
            {
                return value;
            }
            if matches!(
                status,
                Some("cancelled" | "failed" | "rejected" | "unknown" | "saved")
            ) {
                value["error"] = serde_json::json!(match status {
                    Some("cancelled") => "cancelled",
                    Some("unknown") => "unknown_capture",
                    _ => "ended_before_ready",
                });
                return value;
            }
            next = Some((Action::Inspect, id));
        } else {
            if !matches!(value["state"]["phase"].as_str(), Some("idle" | "selecting" | "starting" | "recording" | "processing" | "completed")) {
                return serde_json::json!({"error": "invalid_response"});
            }
            return value;
        }
        std::thread::sleep(
            Duration::from_millis(50).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    fn transport_fixture() -> (PathBuf, UnixListener) {
        // Short enough for sun_path, private and unrelated to the live GUI socket.
        let dir = std::env::temp_dir().join(format!("ac-{}", new_id().unwrap()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.join("s");
        let listener = UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        (path, listener)
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn transport_full_backlog_obeys_absolute_deadline() {
        let (path, listener) = transport_fixture();
        assert_eq!(unsafe { libc::listen(listener.as_raw_fd(), 1) }, 0);
        let queued = [
            UnixStream::connect(&path).unwrap(),
            UnixStream::connect(&path).unwrap(),
        ];
        // Bound even the buggy test: unblock its blocking connect only after the assertion's
        // budget has been exceeded. No timed-out worker is abandoned.
        let server = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(450));
            drop(queued);
            drop(listener);
        });
        let started = Instant::now();
        let result = command_at(None, Duration::from_millis(80), clock_ms() + 80, &path);
        let elapsed = started.elapsed();
        server.join().unwrap();
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        assert!(result.is_err());
        assert!(
            elapsed < Duration::from_millis(300),
            "connect exceeded budget: {elapsed:?}"
        );
    }

    #[test]
    fn transport_trickled_reply_obeys_absolute_deadline() {
        let (path, listener) = transport_fixture();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(&stream).read_line(&mut request).unwrap();
            for byte in b"{\"state\":\"idle\"}\n" {
                if stream.write_all(&[*byte]).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(30));
            }
        });
        let started = Instant::now();
        let result = command_at(None, Duration::from_millis(80), clock_ms() + 80, &path);
        let elapsed = started.elapsed();
        server.join().unwrap();
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        assert!(
            result.is_err(),
            "trickle must not restart the timeout: {result:?}"
        );
        assert!(
            elapsed < Duration::from_millis(300),
            "read exceeded budget: {elapsed:?}"
        );
    }

    #[test]
    fn transport_normal_reply_preserves_request_and_peer_checks() {
        let (path, listener) = transport_fixture();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut request = String::new();
            BufReader::new(&stream).read_line(&mut request).unwrap();
            assert_eq!(request, "state\n");
            stream
                .write_all(b"{\"state\":{\"phase\":\"idle\"}}\n")
                .unwrap();
        });
        let response = command_at(None, Duration::from_secs(1), clock_ms() + 1000, &path);
        server.join().unwrap();
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        assert_eq!(response.unwrap(), "{\"state\":{\"phase\":\"idle\"}}");
    }

    #[test]
    fn transport_stalled_write_obeys_absolute_deadline() {
        let (stream, _peer) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        let started = Instant::now();
        let mut stream = DeadlineStream {
            stream,
            deadline: started + Duration::from_millis(80),
        };
        // More than the socket send queue; peer deliberately never drains it.
        let result = stream.write_all(&vec![0; 4 * 1024 * 1024]);
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_millis(300));
    }

    #[test]
    fn late_reply_cannot_turn_an_expired_wait_into_success() {
        let value = execute_with(
            Some((Action::Start, "owned")),
            true,
            Duration::from_millis(1),
            |_, _| {
                std::thread::sleep(Duration::from_millis(5));
                Ok(serde_json::json!({"capture_id":"owned", "status":"recording"}).to_string())
            },
        );
        assert_eq!(value["error"], "timeout");
        assert_eq!(value["capture_id"], "owned");
    }
    #[test]
    fn wait_timeout_preserves_owned_id_and_retry_reads_terminal_result() {
        let mut stops = 0;
        let value = execute_with(
            Some((Action::Stop, "owned")),
            true,
            Duration::from_millis(5),
            |action, _| {
                stops += usize::from(action.unwrap().0 == Action::Stop);
                Ok(serde_json::json!({"capture_id":"owned", "status":"processing", "outcome":"stopping"}).to_string())
            },
        );
        assert_eq!(value["error"], "timeout");
        assert_eq!(value["capture_id"], "owned");
        assert_eq!(stops, 1);
        let saved = execute_with(
            Some((Action::Stop, "owned")),
            true,
            Duration::from_secs(1),
            |_, _| {
                Ok(serde_json::json!({"capture_id":"owned", "status":"saved", "outcome":"already_ended", "bundle_path":"/exact"}).to_string())
            },
        );
        assert_eq!(saved["bundle_path"], "/exact");
        assert!(saved["error"].is_null());
    }
    #[test]
    fn wait_returns_failures_and_rejects_unknown_or_incomplete_receipts() {
        for (reply, expected) in [
            (
                serde_json::json!({"capture_id":"owned", "status":"failed", "error":"start_failed"}),
                "start_failed",
            ),
            (
                serde_json::json!({"capture_id":"owned", "status":"failed", "error":"finalization_failed"}),
                "finalization_failed",
            ),
            (
                serde_json::json!({"capture_id":"owned", "status":"cancelled"}),
                "cancelled",
            ),
            (
                serde_json::json!({"capture_id":"owned", "status":"unknown", "outcome":"already_ended"}),
                "unknown_capture",
            ),
            (
                serde_json::json!({"capture_id":"foreign", "status":"saved", "bundle_path":"/foreign"}),
                "invalid_response",
            ),
            (serde_json::json!({}), "invalid_response"),
            (
                serde_json::json!({"capture_id":"owned", "status":"saved"}),
                "invalid_response",
            ),
        ] {
            let value = execute_with(
                Some((Action::Stop, "owned")),
                true,
                Duration::from_millis(10),
                |_, _| Ok(reply.to_string()),
            );
            assert_eq!(value["error"], expected, "{reply}");
        }
    }
    #[test]
    fn start_wait_requires_real_readiness_receipt_for_the_owned_id() {
        let mut calls = 0;
        let value = execute_with(
            Some((Action::Start, "owned")),
            true,
            Duration::from_secs(1),
            |action, _| {
                calls += 1;
                assert_eq!(
                    action.unwrap().0,
                    if calls == 1 {
                        Action::Start
                    } else {
                        Action::Inspect
                    }
                );
                Ok(serde_json::json!({"capture_id":"owned", "status":if calls == 1 {"starting"} else {"recording"}}).to_string())
            },
        );
        assert_eq!(value["status"], "recording");
        assert_eq!(calls, 2);
    }
    #[test]
    fn stop_wait_sends_one_stop_then_inspects_only_that_id_until_saved() {
        let mut calls = vec![];
        let replies = [
            serde_json::json!({"capture_id":"owned", "status":"processing", "outcome":"stopping"}),
            serde_json::json!({"capture_id":"owned", "status":"processing"}),
            serde_json::json!({"capture_id":"owned", "status":"saved", "bundle_path":"/captures/exact.apparent"}),
        ];
        let value = execute_with(
            Some((Action::Stop, "owned")),
            true,
            Duration::from_secs(1),
            |action, _| {
                calls.push(action.map(|(a, id)| (a, id.to_owned())));
                Ok(replies[calls.len() - 1].to_string())
            },
        );
        assert_eq!(value["bundle_path"], "/captures/exact.apparent");
        assert_eq!(
            calls,
            vec![
                Some((Action::Stop, "owned".into())),
                Some((Action::Inspect, "owned".into())),
                Some((Action::Inspect, "owned".into()))
            ]
        );
    }
}
