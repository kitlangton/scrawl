//! Small helpers the router shares with its host.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// `scrawl-<ms>-<pid>[-n]`: unique within this process even in the same millisecond.
pub fn make_bundle_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let ms = now_epoch_ms();
    let pid = std::process::id();
    match COUNT.fetch_add(1, Ordering::Relaxed) {
        0 => format!("scrawl-{ms}-{pid:x}"),
        n => format!("scrawl-{ms}-{pid:x}-{n}"),
    }
}

pub fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(byte) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The executable a running process was started from (`proc_pidpath`, no processes spawned).
#[cfg(target_os = "macos")]
pub fn process_executable(pid: i32) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let mut buf = vec![0_u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let len = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    let len = usize::try_from(len).ok().filter(|&len| len > 0)?;
    buf.truncate(len);
    Some(PathBuf::from(std::ffi::OsString::from_vec(buf)))
}

#[cfg(not(target_os = "macos"))]
pub fn process_executable(_pid: i32) -> Option<PathBuf> {
    None
}
