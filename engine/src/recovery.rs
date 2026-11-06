//! Small native boundary for the opt-in recovery peer lifetime.
use serde_json::json;

pub const SCHEMA: &str = "apparent.engine-recovery-peer/1";

#[derive(Clone, Copy)]
pub struct Role<'a> {
    pub peer_id: &'a str,
    pub channel: &'static str,
}

pub fn nonce(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn lifecycle(role: Role<'_>, event: &'static str) {
    let value = json!({
        "schema": SCHEMA,
        "peerId": role.peer_id,
        "channel": role.channel,
        "implementation": "native",
        "role": "recovery",
        "startsFreshCapture": false,
        "onInputEof": "retain-admitted-finite-work",
        "event": event
    });
    // One small atomic-ish nonblocking write: a full or broken diagnostics pipe
    // may lose this fact, but cannot stall startup, release, or finite work.
    let Ok(mut bytes) = serde_json::to_vec(&value) else {
        return;
    };
    bytes.push(b'\n');
    let fd = libc::STDERR_FILENO;
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return;
    }
    let _ = unsafe { libc::write(fd, bytes.as_ptr().cast(), bytes.len()) };
    let _ = unsafe { libc::fcntl(fd, libc::F_SETFL, flags) };
}

/// Detach subsequent diagnostics and protocol writes from caller-owned pipes.
/// The input-closed lifecycle fact is emitted before this redirection.
#[cfg(unix)]
pub fn detach_outputs() -> std::io::Result<()> {
    use std::os::fd::AsRawFd;
    let sink = std::fs::OpenOptions::new().write(true).open("/dev/null")?;
    for fd in [libc::STDOUT_FILENO, libc::STDERR_FILENO] {
        if unsafe { libc::dup2(sink.as_raw_fd(), fd) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}
