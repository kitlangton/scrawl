//! Window-owned Connect stream; the existing controller/client own all policy.
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::{Duration, Instant};

use serde::Deserialize;
use crate::capture::desktop::settings::connect::{Approval, Backend, Command, Controller, Event, Health, SafeError, Snapshot, CONNECT_SETTINGS};

const MAX_FRAME: usize = 4096;
const FRAME_MS: u64 = 3000;
const LEASE: Duration = Duration::from_secs(10);

#[derive(Deserialize)]
#[serde(tag = "connect", rename_all = "lowercase", deny_unknown_fields)]
enum Opening { Settings { deadline: u64 } }
#[derive(Deserialize)]
#[serde(tag = "connect", rename_all = "lowercase", deny_unknown_fields)]
enum Message {
    Read { deadline: u64 },
    Command { command: Command, deadline: u64 },
}

pub(super) fn opening_deadline(line: &str) -> Option<u64> {
    if line.len() > MAX_FRAME { return None; }
    let Opening::Settings { deadline } = serde_json::from_str(line).ok()?;
    Some(deadline)
}
fn clock_ms() -> u64 { super::agent_capture::storage_clock_ms() }
fn enabled(config: &Path) -> Result<(), &'static str> {
    match crate::shared::handoff::capture_location::experiment_in(config, CONNECT_SETTINGS) {
        Ok(true) => Ok(()),
        Ok(false) => Err("feature_disabled"),
        Err(_) => Err("config_read_failed"),
    }
}
fn write_frame(stream: &mut UnixStream, value: &serde_json::Value) -> std::io::Result<()> {
    serde_json::to_writer(&mut *stream, value)?;
    stream.write_all(b"\n")
}
fn refuse(stream: &mut UnixStream, code: &'static str) {
    let _ = write_frame(stream, &serde_json::json!({"error":code}));
}
// The feature is a window/lifecycle gate, not a replacement consent store. Carry
// its fresh read into the client's existing transport/publication predicate.
struct FeatureBackend { inner: Arc<dyn Backend>, config: PathBuf }
impl FeatureBackend {
    fn allowed(&self, caller: Arc<dyn Fn() -> bool + Send + Sync>) -> Arc<dyn Fn() -> bool + Send + Sync> {
        let config = self.config.clone();
        Arc::new(move || caller() && enabled(&config).is_ok())
    }
}
impl Backend for FeatureBackend {
    fn snapshot(&self) -> Result<Snapshot, SafeError> { self.inner.snapshot() }
    fn shared_links_url(&self) -> Option<String> { self.inner.shared_links_url() }
    fn login(&self, server: &str, allowed: Arc<dyn Fn() -> bool + Send + Sync>, approval: &dyn Fn(Approval)) -> Result<(), SafeError> {
        self.inner.login(server, self.allowed(allowed), approval)
    }
    fn health(&self, allowed: Arc<dyn Fn() -> bool + Send + Sync>) -> Health { self.inner.health(self.allowed(allowed)) }
    // Revocation must still settle when the feature has been disabled.
    fn disable(&self, logout: bool) -> Result<(), SafeError> { self.inner.disable(logout) }
}
fn drain(controller: &mut Controller, events: &mpsc::Receiver<Event>, worker: &mut impl FnMut(), config: &Path) -> Result<(), &'static str> {
    while let Ok(event) = events.try_recv() {
        if let Err(code) = enabled(config) { controller.close(); return Err(code); }
        if controller.receive(event) {
            if let Err(code) = enabled(config) {
                controller.command(Command::Disable);
                controller.close();
                return Err(code);
            }
            worker();
        }
    }
    Ok(())
}

pub(super) fn serve(stream: UnixStream, deadline: u64, config: &Path) {
    run(stream, deadline, config, || crate::connect::hosted::settings_backend(config),
        crate::connect::hosted::start_worker, LEASE);
}

fn run(
    mut stream: UnixStream, deadline: u64, config: &Path,
    backend: impl FnOnce() -> Arc<dyn Backend>, mut worker: impl FnMut(), lease_duration: Duration,
) {
    // Socket ownership and opening-frame admission were checked by the listener.
    // Bound both writes and partial reads; no policy/network work on that listener.
    if stream.set_nonblocking(false).is_err()
        || stream.set_read_timeout(Some(Duration::from_millis(50))).is_err()
        || stream.set_write_timeout(Some(Duration::from_millis(200))).is_err() { return; }
    if clock_ms() >= deadline { refuse(&mut stream, "admission_expired"); return; }
    if let Err(code) = enabled(config) { refuse(&mut stream, code); return; }
    let (tx, events) = mpsc::sync_channel::<Event>(64);
    let backend = Arc::new(FeatureBackend { inner: backend(), config: config.to_owned() });
    let mut controller = Controller::new(backend, Arc::new(move |event| { let _ = tx.try_send(event); }));
    if clock_ms() >= deadline { refuse(&mut stream, "admission_expired"); return; }
    if write_frame(&mut stream, &serde_json::json!({"connect":controller.view()})).is_err() { return; }
    let mut buffer = Vec::with_capacity(MAX_FRAME);
    let mut frame_deadline = None;
    let mut lease = Instant::now() + lease_duration;
    let mut next_gate = Instant::now();
    loop {
        if Instant::now() >= lease || frame_deadline.is_some_and(|until| clock_ms() >= until) { break; }
        let mut fd = libc::pollfd { fd: stream.as_raw_fd(), events: libc::POLLIN, revents: 0 };
        let ready = unsafe { libc::poll(&mut fd, 1, 50) };
        if ready < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted { continue; }
            break;
        }
        // Closing/reloading wins over queued completion receipts. Never route a
        // receipt to a replacement controller, even if the old login committed.
        if fd.revents & (libc::POLLERR | libc::POLLNVAL | libc::POLLHUP) != 0 { break; }
        if ready > 0 {
            let mut chunk = [0; 1024];
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    let until = *frame_deadline.get_or_insert_with(|| clock_ms() + FRAME_MS);
                    if buffer.len() + n > MAX_FRAME { refuse(&mut stream, "invalid_request"); break; }
                    buffer.extend_from_slice(&chunk[..n]);
                    if let Some(end) = buffer.iter().position(|b| *b == b'\n') {
                        // One request/receipt at a time, no pipelining or trailing bytes.
                        if end + 1 != buffer.len() { refuse(&mut stream, "invalid_request"); break; }
                        let message = serde_json::from_slice::<Message>(&buffer[..end]);
                        buffer.clear(); frame_deadline = None;
                        let Ok(message) = message else { refuse(&mut stream, "invalid_request"); break; };
                        let (deadline, command) = match message {
                            Message::Read { deadline } => (deadline.min(until), None),
                            Message::Command { deadline, command } => (deadline.min(until), Some(command)),
                        };
                        if clock_ms() >= deadline { refuse(&mut stream, "admission_expired"); break; }
                        if let Err(code) = enabled(config) { refuse(&mut stream, code); break; }
                        controller.refresh();
                        if clock_ms() >= deadline { refuse(&mut stream, "admission_expired"); break; }
                        if let Err(code) = enabled(config) { refuse(&mut stream, code); break; }
                        // Cancel/disable commands invalidate old requests before receipts.
                        let uri = command.and_then(|command| controller.command(command));
                        if let Err(code) = drain(&mut controller, &events, &mut worker, config) { refuse(&mut stream, code); break; }
                        let mut reply = serde_json::json!({"connect":controller.view()});
                        if let Some(uri) = uri { reply["openUri"] = uri.into(); }
                        if write_frame(&mut stream, &reply).is_err() { break; }
                        lease = Instant::now() + lease_duration;
                        next_gate = Instant::now() + Duration::from_millis(500);
                    }
                }
                Err(error) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted) => continue,
                Err(_) => break,
            }
        }
        // A feature edit applies even while a login is waiting on the network.
        // Do not continuously reread config while idle: a half-second gate suffices.
        if Instant::now() >= next_gate {
            if let Err(code) = enabled(config) { refuse(&mut stream, code); break; }
            next_gate = Instant::now() + Duration::from_millis(500);
        }
        // Receipts are consumed only for an admitted window request above.
        // Idle completion must not retire a login or authorize its worker while
        // the last delivered window state still says authorization is pending.
        // EOF/lease expiry then retains the controller's durable cancel path.
    }
    controller.close();
}

#[cfg(test)]
mod tests;
