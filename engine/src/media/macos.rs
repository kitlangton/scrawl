//! Bounded private bridge to the shared AVFoundation decoder.
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::{net::UnixStream, process::CommandExt};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

#[link(name = "proc")]
unsafe extern "C" {}

fn footprint(pid: libc::pid_t) -> Result<u64, Option<i32>> {
    let mut usage: libc::rusage_info_v2 = unsafe { std::mem::zeroed() };
    let result = unsafe { libc::proc_pid_rusage(pid, libc::RUSAGE_INFO_V2,
        (&mut usage as *mut libc::rusage_info_v2).cast()) };
    if result == 0 { Ok(usage.ri_phys_footprint) }
    else { Err(std::io::Error::last_os_error().raw_os_error()) }
}

pub struct Decoded {
    pub actual_ms: u64,
    pub clamped_to_end: bool,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

fn helper() -> Option<PathBuf> {
    Some(std::env::current_exe().ok()?.parent()?.join("apparent-engine-frames-native"))
}

struct Running(Child);
impl Drop for Running {
    fn drop(&mut self) { let _ = self.0.kill(); let _ = self.0.wait(); }
}

pub(crate) struct FootprintGuard {
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl FootprintGuard {
    fn start(helper_pid: u32, budget: u64) -> Self {
        Self::monitor(Some(helper_pid), budget)
    }
    pub(crate) fn worker(budget: u64) -> Self {
        Self::monitor(None, budget)
    }
    fn monitor(helper_pid: Option<u32>, budget: u64) -> Self {
        use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
        let stop = Arc::new(AtomicBool::new(false));
        let done = stop.clone();
        let thread = std::thread::spawn(move || {
            while !done.load(Ordering::Acquire) {
                let own = footprint(unsafe { libc::getpid() });
                let helper = helper_pid.map(|pid| footprint(pid as libc::pid_t)).unwrap_or(Ok(0));
                if matches!(helper, Err(Some(libc::ESRCH))) { break }
                let valid = match (own, helper) {
                    (Ok(a), Ok(b)) => a.checked_add(b).is_some_and(|n| n <= budget),
                    _ => false,
                };
                if !valid {
                    // Exit only this owned job. Closing its lifetime socket stops
                    // the Swift bridge; never signal a potentially recycled PID.
                    unsafe { libc::_exit(1) }
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        });
        Self { stop, thread: Some(thread) }
    }
}
impl Drop for FootprintGuard {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        if let Some(thread) = self.thread.take() { let _ = thread.join(); }
    }
}

pub fn available() -> bool {
    let Some(path) = helper() else { return false };
    let Ok(child) = Command::new(path).arg("--version").env_clear()
        .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn() else { return false };
    let mut child = Running(child);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        match child.0.try_wait() {
            Ok(Some(status)) if status.success() => {
                let mut bytes = Vec::new();
                return child.0.stdout.take().is_some_and(|out| out.take(128).read_to_end(&mut bytes).is_ok())
                    && bytes == b"apparent-engine-frames-native/1\n";
            }
            Ok(Some(_)) | Err(_) => return false,
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(10)),
        }
    }
    false
}

pub fn decode(source: &File, times: &[u64], max_edge: u32, max_pixels: u64,
    mut emit: impl FnMut(Decoded) -> Result<(), &'static str>) -> Result<(), &'static str> {
    if times.is_empty() || times.len() > 48 || !(1..=4096).contains(&max_edge)
        || !(1..=64_000_000).contains(&max_pixels) { return Err("limits_exceeded") }
    let (lifetime, monitor) = UnixStream::pair().map_err(|_| "internal")?;
    let duplicate = |fd| {
        let fd = unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 10) };
        if fd < 0 { Err("internal") } else { Ok(unsafe { File::from_raw_fd(fd) }) }
    };
    let input = duplicate(source.as_raw_fd())?;
    let life = duplicate(monitor.as_raw_fd())?;
    let mut command = Command::new(helper().ok_or("decode_failed")?);
    command.arg("--decode-pinned-frames").env_clear()
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    unsafe {
        command.pre_exec(move || {
            for (from, to) in [(input.as_raw_fd(), 3), (life.as_raw_fd(), 4)] {
                if libc::dup2(from, to) < 0 { return Err(std::io::Error::last_os_error()) }
            }
            Ok(())
        });
    }
    let mut child = Running(command.spawn().map_err(|_| "decode_failed")?);
    let _footprint = FootprintGuard::start(child.0.id(), 1536 * 1024 * 1024);
    let request = serde_json::to_vec(&serde_json::json!({"times":times,"maxEdge":max_edge,"maxPixels":max_pixels}))
        .map_err(|_| "internal")?;
    child.0.stdin.take().ok_or("internal")?.write_all(&request).map_err(|_| "decode_failed")?;
    let mut out = child.0.stdout.take().ok_or("internal")?;
    let mut total = 0u64;
    for &requested in times {
        let mut bytes = [0u8; 48];
        out.read_exact(&mut bytes).map_err(|_| "decode_failed")?;
        let mut header = [0u64; 6];
        for (i, word) in bytes.chunks_exact(8).enumerate() { header[i] = u64::from_le_bytes(word.try_into().unwrap()); }
        let [width, height, actual, duration, clamped, length] = header;
        if width == 0 && height == 0 { return Err(match actual { 1 => "limits_exceeded", 2 => "source_incomplete", _ => "decode_failed" }) }
        let pixels = width.checked_mul(height).ok_or("limits_exceeded")?;
        total = total.checked_add(pixels).ok_or("limits_exceeded")?;
        if width == 0 || height == 0 || width.max(height) > u64::from(max_edge)
            || total > max_pixels || pixels.checked_mul(4) != Some(length) {
            return Err("limits_exceeded")
        }
        if duration == 0 || actual > 9_007_199_254_740_991 || clamped > 1
            || (clamped == 1) != (requested >= duration) { return Err("decode_failed") }
        let mut rgba = vec![0; usize::try_from(length).map_err(|_| "limits_exceeded")?];
        out.read_exact(&mut rgba).map_err(|_| "decode_failed")?;
        emit(Decoded { actual_ms: actual, clamped_to_end: clamped == 1,
            width: width as u32, height: height as u32, rgba })?;
    }
    let mut extra = [0u8; 1];
    if out.read(&mut extra).map_err(|_| "decode_failed")? != 0
        || !child.0.wait().map_err(|_| "decode_failed")?.success() { return Err("decode_failed") }
    drop(lifetime);
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn native_footprint_query_is_available() {
        assert!(super::footprint(unsafe { libc::getpid() }).unwrap() > 0);
    }

    #[test]
    fn budget_aborts_only_the_owned_subprocess() {
        if std::env::var_os("APPARENT_MEMORY_PROBE").is_some() {
            let _guard = super::FootprintGuard::start(std::process::id(), 1);
            std::thread::sleep(std::time::Duration::from_secs(1));
            panic!("footprint guard failed to abort");
        }
        let result = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "macos::tests::budget_aborts_only_the_owned_subprocess"])
            .env("APPARENT_MEMORY_PROBE", "1").status().unwrap();
        assert_eq!(result.code(), Some(1));
    }
    use std::process::Command;
}
