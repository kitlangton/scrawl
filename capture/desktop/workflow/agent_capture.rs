//! Local, same-user control of the running GUI. Never launches an app or sends a capture.
use apparent_capture as client;
#[cfg(test)]
use client::command;
pub use client::{Action, execute, new_id};
use client::{clock_ms, same_user, socket_path, valid_id, connect_until, DeadlineStream};
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Result, bail, ensure};
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Idle,
    Selecting,
    Starting,
    Recording,
    Processing,
    Completed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct State {
    pub phase: Phase,
    /// Opaque, per-capture identity. Absent outside a pending or active capture.
    pub capture_id: Option<String>,
}

/// Small in-process receipts, not capture history on disk. Media stays in the bundle.
#[derive(Default)]
pub struct Registry(HashMap<String, serde_json::Value>);

impl Registry {
    pub fn decide(&self, state: &State, action: Action, id: &str) -> Outcome {
        if action == Action::Stop
            && matches!(
                self.inspect(id)["status"].as_str(),
                Some("processing" | "saved" | "cancelled" | "failed")
            )
        {
            return Outcome::AlreadyEnded;
        }
        decide(state, action, id)
    }
    /// Called on the GUI thread, before any recording side effect. Rejected IDs are
    /// tombstoned too: retrying a rejected/expired request cannot launch later.
    pub fn admit(&mut self, id: &str, occupied: bool, expired: bool) -> bool {
        if self.0.contains_key(id) {
            return false;
        }
        if expired || occupied {
            self.set(
                id,
                "rejected",
                None,
                Some(if expired {
                    "admission_expired"
                } else {
                    "occupied"
                }),
            );
            return false;
        }
        self.begin(id);
        true
    }

    pub fn begin(&mut self, id: &str) {
        self.set(id, "starting", None, None);
    }

    pub fn set(
        &mut self,
        id: &str,
        status: &str,
        path: Option<&std::path::Path>,
        error: Option<&str>,
    ) {
        self.0.insert(
            id.into(),
            serde_json::json!({
                "capture_id": id, "status": status, "bundle_path": path, "error": error,
            }),
        );
    }

    /// Only the successful production finalizer can publish a saved path.
    pub fn finish(&mut self, id: &str, result: Result<&std::path::Path, String>) {
        match result {
            Ok(path) => self.set(id, "saved", Some(path), None),
            Err(message) => {
                self.set(id, "failed", None, Some("finalization_failed"));
                self.0.get_mut(id).unwrap()["message"] = message.into();
            }
        }
    }

    pub fn inspect(&self, id: &str) -> serde_json::Value {
        self.0.get(id).cloned().unwrap_or_else(|| serde_json::json!({
            "capture_id": id, "status": "unknown", "bundle_path": null, "error": "unknown_capture",
        }))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Stopping,
    Cancelled,
    AlreadyEnded,
    WrongCapture,
    NotReady,
}

pub fn decide(state: &State, action: Action, id: &str) -> Outcome {
    let Some(current) = &state.capture_id else {
        return Outcome::AlreadyEnded;
    };
    if id != current {
        return Outcome::WrongCapture;
    }
    match (&state.phase, action) {
        (Phase::Recording, Action::Stop) => Outcome::Stopping,
        (Phase::Starting | Phase::Recording, Action::Cancel) => Outcome::Cancelled,
        _ => Outcome::NotReady,
    }
}

pub struct Request {
    pub action: Option<(Action, String)>,
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    pub native_host: Option<super::native_host::Admitted>,
    pub reply: std::sync::mpsc::SyncSender<String>,
    pub deadline_ms: u64,
}

/// Same-host monotonic deadline, including time spent in the socket accept queue.
impl Request {
    pub fn expired(&self) -> bool {
        clock_ms() >= self.deadline_ms
    }
}

/// libuv hrtime uses mach_continuous_time on macOS. Keep Storage separate from
/// the legacy Rust-client CLOCK_MONOTONIC timeline.
pub(crate) fn storage_clock_ms() -> u64 {
    #[cfg(target_os = "macos")]
    {
        let mut time: libc::timespec = unsafe { std::mem::zeroed() };
        assert_eq!(unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC_RAW, &mut time) }, 0);
        time.tv_sec as u64 * 1000 + time.tv_nsec as u64 / 1_000_000
    }
    #[cfg(not(target_os = "macos"))]
    { clock_ms() }
}

#[derive(serde::Deserialize, Debug)]
#[serde(tag = "storage", rename_all = "lowercase", deny_unknown_fields)]
enum Storage {
    Read { deadline: u64 },
    Set { path: String, expected: String, deadline: u64 },
    Reset { expected: String, deadline: u64 },
    LocationFacts { deadline: u64 },
    ProbeLocation { path: String, expected: String, deadline: u64 },
    SetLocation { path: String, expected: String, deadline: u64 },
    ResetLocation { expected: String, deadline: u64 },
    Retention { deadline: u64 },
    SaveRetention { policy: super::storage_preferences::SavePolicy, expected: String, deadline: u64 },
    ResetRetention { expected: String, deadline: u64 },
    Usage { deadline: u64 },
}

impl Storage {
    fn deadline(&self) -> u64 {
        match self {
            Self::Read { deadline } | Self::Set { deadline, .. } | Self::Reset { deadline, .. }
            | Self::LocationFacts { deadline } | Self::ProbeLocation { deadline, .. }
            | Self::SetLocation { deadline, .. } | Self::ResetLocation { deadline, .. }
            | Self::Retention { deadline } | Self::SaveRetention { deadline, .. }
            | Self::ResetRetention { deadline, .. } | Self::Usage { deadline } => *deadline,
        }
    }

    fn execute(&self, config: &std::path::Path, deadline: u64) -> serde_json::Value {
        use crate::shared::handoff::capture_location;
        if storage_clock_ms() >= deadline { return serde_json::json!({"error":"admission_expired"}); }
        // Rust's config is authority. No environment or renderer flag activates this slice.
        match capture_location::enabled_in(config) {
            Ok(true) => {},
            Ok(false) => return serde_json::json!({"error":"feature_disabled"}),
            Err(_) => return serde_json::json!({"error":"config_read_failed"}),
        }
        let preferences = match self {
            Self::Retention { .. } => Some(super::storage_preferences::read(config)),
            Self::SaveRetention { policy, expected, .. } => Some(super::storage_preferences::save(config, Some(policy), expected, || storage_clock_ms() < deadline)),
            Self::ResetRetention { expected, .. } => Some(super::storage_preferences::save(config, None, expected, || storage_clock_ms() < deadline)),
            Self::Usage { .. } => Some(super::storage_preferences::usage(config, || storage_clock_ms() < deadline)),
            _ => None,
        };
        if let Some(result) = preferences {
            return result.unwrap_or_else(|code| serde_json::json!({"error":code}));
        }
        match self {
            Self::LocationFacts { .. } => return capture_location::facts_in(config)
                .map(|facts| serde_json::json!({"location":facts}))
                .unwrap_or_else(|_| serde_json::json!({"error":"config_read_failed"})),
            Self::ProbeLocation { path, expected, .. } => {
                let before = match capture_location::facts_in(config) {
                    Ok(facts) => facts,
                    Err(_) => return serde_json::json!({"error":"config_read_failed"}),
                };
                if before.revision != *expected { return serde_json::json!({"error":"conflict"}); }
                let available = capture_location::probe(std::path::Path::new(path));
                if storage_clock_ms() >= deadline { return serde_json::json!({"error":"admission_expired"}); }
                let after = match capture_location::facts_in(config) {
                    Ok(facts) => facts,
                    Err(_) => return serde_json::json!({"error":"config_read_failed"}),
                };
                if after.revision != *expected { return serde_json::json!({"error":"conflict"}); }
                return serde_json::json!({"probe":{"available":available}});
            }
            _ => {}
        }
        let mutation = match self {
            Self::Read { .. } => Ok(()),
            Self::Set { path, expected, .. } => capture_location::set_in(config, Some(path), Some(expected), || storage_clock_ms() < deadline),
            Self::Reset { expected, .. } => capture_location::set_in(config, None, Some(expected), || storage_clock_ms() < deadline),
            Self::SetLocation { path, expected, .. } => capture_location::set_in(config, Some(path), Some(expected), || storage_clock_ms() < deadline),
            Self::ResetLocation { expected, .. } => capture_location::set_in(config, None, Some(expected), || storage_clock_ms() < deadline),
            _ => unreachable!(),
        };
        if let Err(code) = mutation { return serde_json::json!({"error":code}); }
        match self {
          Self::SetLocation { .. } | Self::ResetLocation { .. } => capture_location::facts_in(config)
            .map(|facts| serde_json::json!({"location":facts}))
            .unwrap_or_else(|_| serde_json::json!({"error":"config_read_failed"})),
          _ => match capture_location::read_in(config) {
            Ok(snapshot) => serde_json::json!({"storage": snapshot}),
            Err(_) => serde_json::json!({"error":"config_read_failed"}),
          },
        }
    }
}

#[derive(serde::Deserialize, Debug)]
#[serde(tag = "reference", rename_all = "lowercase", deny_unknown_fields)]
enum Reference {
    Read { deadline: u64 },
    Copy { expected: String, deadline: u64 },
}

impl Reference {
    fn admission(self) -> Option<(Option<(Action, String)>, u64)> {
        let (action, expected, deadline) = match self {
            Self::Read { deadline } => (Action::ReferenceRead, String::new(), deadline),
            Self::Copy { expected, deadline } => {
                if expected.is_empty() || expected.len() > 4096 || expected.contains(['\0', '/']) { return None; }
                (Action::ReferenceCopy, expected, deadline)
            }
        };
        // Node uses the same clock as Storage. Convert only the remaining budget
        // to the GUI request clock; never compare different macOS clock origins.
        let remaining = deadline.saturating_sub(storage_clock_ms()).min(3000);
        Some((Some((action, expected)), clock_ms() + remaining))
    }
}

#[derive(serde::Deserialize, Debug)]
#[serde(tag = "drawing", rename_all = "lowercase", deny_unknown_fields)]
enum Drawing {
    Read { deadline: u64 },
    Undo { expected: String, deadline: u64 },
}


pub(crate) fn drawing_admission(current: Option<&str>, expected: Option<&str>, expired: bool) -> Result<(), &'static str> {
    if expired { return Err("admission_expired"); }
    if let Some(expected) = expected {
        if current != Some(expected) { return Err("conflict"); }
    }
    Ok(())
}

enum Wire {
    Capture(Option<(Action, String)>, u64),
    PublicShare(Action, String, u64),
    Storage(Storage),
    ErrorReporting(ErrorReportingRequest),
    Microphone(super::notes::audio::Request),
    Connect(u64),
    ConnectBackend(u64),
    NativeHost(super::native_host::Admitted),
}

// A single rendezvous worker handles the two bounded Settings contracts. A
// contended config flock must not stall capture-control socket acceptance.
enum SettingsRequest {
    Storage(Storage),
    ErrorReporting(ErrorReportingRequest),
    Microphone(super::notes::audio::Request),
}

impl SettingsRequest {
    fn execute(&self, config: &std::path::Path, deadline: u64) -> serde_json::Value {
        match self {
            Self::Storage(request) => request.execute(config, deadline),
            Self::ErrorReporting(request) => request.execute(config, deadline),
            Self::Microphone(request) => request.execute(config, deadline),
        }
    }
}

#[derive(serde::Deserialize, Debug)]
#[serde(tag = "errorReporting", rename_all = "lowercase", deny_unknown_fields)]
enum ErrorReportingRequest {
    Read { deadline: u64 },
    Set { enabled: bool, deadline: u64 },
    Exception { kind: ExceptionKind, deadline: u64 },
}

#[derive(serde::Deserialize, Debug)]
#[serde(rename_all = "snake_case")]
enum ExceptionKind { MainException, MainRejection, RendererException, RendererRejection }

#[test]
fn exception_request_rejects_arbitrary_payloads() {
    for kind in ["main_exception", "main_rejection", "renderer_exception", "renderer_rejection"] {
        let request = serde_json::json!({"errorReporting":"exception", "kind":kind, "deadline":1});
        assert!(matches!(parse_request(&request.to_string()), Some(Wire::ErrorReporting(_))));
        let mut with_text = request.clone();
        with_text["message"] = serde_json::json!("private token and path");
        assert!(parse_request(&with_text.to_string()).is_none());
    }
    assert!(parse_request(r#"{"errorReporting":"exception","kind":"raw error","deadline":1}"#).is_none());
}

impl ErrorReportingRequest {
    fn deadline(&self) -> u64 {
        match self { Self::Read { deadline } | Self::Set { deadline, .. } | Self::Exception { deadline, .. } => *deadline }
    }

    fn execute(&self, config: &std::path::Path, deadline: u64) -> serde_json::Value {
        if storage_clock_ms() >= deadline { return serde_json::json!({"error":"admission_expired"}); }
        if let Self::Exception { kind, .. } = self {
            use super::error_reporting::{Failure, report_exception};
            report_exception(match kind {
                ExceptionKind::MainException => Failure::ElectronMainException,
                ExceptionKind::MainRejection => Failure::ElectronMainRejection,
                ExceptionKind::RendererException => Failure::ElectronRendererException,
                ExceptionKind::RendererRejection => Failure::ElectronRendererRejection,
            });
            return serde_json::json!({"accepted":true});
        }
        let (operation, enabled) = match self {
            Self::Read { .. } => ("read", None),
            Self::Set { enabled, .. } => ("set", Some(*enabled)),
            Self::Exception { .. } => unreachable!(),
        };
        match crate::capture::desktop::workflow::error_reporting::error_reporting_bridge_in(config, operation, enabled, || storage_clock_ms() < deadline) {
            Ok(enabled) => serde_json::json!({"errorReporting":{"enabled":enabled}}),
            Err(code) => serde_json::json!({"error":code}),
        }
    }
}

fn parse_request(line: &str) -> Option<Wire> {
    if line.starts_with('{')
        && let Some(request) = super::native_host::parse(line, storage_clock_ms())
    {
        return Some(Wire::NativeHost(request));
    }
    if let Some(deadline) = super::connect_settings::backend_opening_deadline(line) {
        return Some(Wire::ConnectBackend(deadline.min(storage_clock_ms() + 3000)));
    }
    if let Some(deadline) = super::connect_settings::opening_deadline(line) {
        return Some(Wire::Connect(deadline.min(storage_clock_ms() + 3000)));
    }
    if line.starts_with('{') {
        if let Ok(drawing) = serde_json::from_str::<Drawing>(line) {
            let (action, expected, deadline) = match drawing {
                Drawing::Read { deadline } => (Action::DrawingRead, String::new(), deadline),
                Drawing::Undo { expected, deadline } if valid_revision(&expected) => (Action::DrawingUndo, expected, deadline),
                _ => return None,
            };
            let remaining = deadline.saturating_sub(storage_clock_ms()).min(3000);
            return Some(Wire::Capture(Some((action, expected)), clock_ms() + remaining));
        }
        if let Ok(request) = serde_json::from_str::<PublicShareRequest>(line) {
            return request.admission();
        }

        if let Ok(request) = serde_json::from_str::<super::notes::audio::Request>(line) { return Some(Wire::Microphone(request)); }
        if let Ok(request) = serde_json::from_str::<ErrorReportingRequest>(line) {
            return Some(Wire::ErrorReporting(request));
        }
        if let Ok(reference) = serde_json::from_str::<Reference>(line) {
            let (action, deadline) = reference.admission()?;
            return Some(Wire::Capture(action, deadline));
        }
        let storage: Storage = serde_json::from_str(line).ok()?;
        match &storage {
            Storage::Set { path, expected, .. } if path.is_empty() || path.len() > 4096 || path.contains('\0') || !std::path::Path::new(path).is_absolute() || !valid_revision(expected) => return None,
            Storage::Reset { expected, .. } if !valid_revision(expected) => return None,
            Storage::ProbeLocation { path, expected, .. } if path.is_empty() || path.len() > 4096 || path.contains('\0') || !std::path::Path::new(path).is_absolute() || !valid_revision(expected) => return None,
            Storage::SetLocation { path, expected, .. } if path.is_empty() || path.len() > 4096 || path.contains('\0') || !std::path::Path::new(path).is_absolute() || !valid_revision(expected) => return None,
            Storage::ResetLocation { expected, .. } if !valid_revision(expected) => return None,
            Storage::SaveRetention { policy, expected, .. } if !policy.valid() || !valid_revision(expected) => return None,
            Storage::ResetRetention { expected, .. } if !valid_revision(expected) => return None,
            _ => {}
        }
        Some(Wire::Storage(storage))
    } else {
        if line.len() > 255 { return None; }
        parse_capture_request(line).map(|(action, deadline)| Wire::Capture(action, deadline))
    }
}

#[derive(serde::Deserialize)]
#[serde(tag = "publicShare", rename_all = "camelCase", deny_unknown_fields)]
enum PublicShareRequest {
    Start { expected: String, disclosed: bool, deadline: u64 },
    Read { operation: String, deadline: u64 },
    Deliver { operation: String, deadline: u64 },
    Cancel { operation: String, deadline: u64 },
}

impl PublicShareRequest {
    fn admission(self) -> Option<Wire> {
        let (action, value, deadline) = match self {
            Self::Start { expected, disclosed: true, deadline }
                if !expected.is_empty() && expected.len() <= 4096 && !expected.contains(['\0', '/']) =>
                    (Action::PublicShareStart, expected, deadline),
            Self::Start { .. } => return None,
            Self::Read { operation, deadline } => (Action::PublicShareRead, operation, deadline),
            Self::Deliver { operation, deadline } => (Action::PublicShareDeliver, operation, deadline),
            Self::Cancel { operation, deadline } => (Action::PublicShareCancel, operation, deadline),
        };
        if matches!(action, Action::PublicShareRead | Action::PublicShareDeliver | Action::PublicShareCancel)
            && (value.len() != 32 || !value.bytes().all(|b| b.is_ascii_hexdigit())) { return None; }
        let remaining = deadline.saturating_sub(storage_clock_ms()).min(3000);
        Some(Wire::PublicShare(action, value, clock_ms() + remaining))
    }
}

fn valid_revision(value: &str) -> bool {
    value.len() == 16 && value.bytes().all(|b| b.is_ascii_hexdigit())
}

fn parse_capture_request(line: &str) -> Option<(Option<(Action, String)>, u64)> {
    let parts: Vec<_> = line.split_whitespace().collect();
    if parts == ["state"] {
        return Some((None, clock_ms() + 3000));
    }
    if !(2..=3).contains(&parts.len()) || !valid_id(parts[1]) {
        return None;
    }
    let action = match parts[0] {
        "start" if parts.len() == 3 => Action::Start,
        "inspect" => Action::Inspect,
        "stop" => Action::Stop,
        "cancel" => Action::Cancel,
        _ => return None,
    };
    let deadline = if parts.len() == 3 {
        parts[2].parse().ok()?
    } else {
        clock_ms() + 3000
    };
    Some((
        Some((action, parts[1].into())),
        deadline.min(clock_ms() + 3000),
    ))
}

/// Separate from Linux's shortcut socket: no toggle, no unacknowledged mutation.
pub fn listen() -> Result<flume::Receiver<Request>> {
    let path = socket_path(true)?;
    listen_at(path)
}

fn listen_at(path: PathBuf) -> Result<flume::Receiver<Request>> {
    listen_at_config(path, crate::shared::handoff::default_config_path())
}

fn listen_at_config(path: PathBuf, config_path: PathBuf) -> Result<flume::Receiver<Request>> {
    // Hold this advisory lock for the listener's whole lifetime. Without it, two launches
    // recovering the same stale socket can unlink each other's freshly bound listener.
    let owner_path = path.with_extension("lock");
    let owner = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(owner_path)?;
    ensure!(
        try_lock_owner(&owner)?,
        "control socket is already owned by another GUI"
    );
    if let Ok(meta) = std::fs::symlink_metadata(&path) {
        ensure!(
            meta.file_type().is_socket()
                && meta.uid() == unsafe { libc::geteuid() }
                && meta.mode() & 0o777 == 0o600,
            "unsafe existing control socket"
        );
        match connect_until(&path, Instant::now() + Duration::from_millis(250)) {
            Err(error) if error.kind() == std::io::ErrorKind::ConnectionRefused => {
                std::fs::remove_file(&path)?
            }
            _ => bail!("control socket is already in use: {}", path.display()),
        }
    }
    let listener = UnixListener::bind(&path)?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    listener.set_nonblocking(true)?;
    let (tx, rx) = flume::bounded(16);
    // Rendezvous channel: one persistent worker, no queued/accumulating blocked writers.
    // The listener and GUI remain free while this worker waits on the existing flock.
    let (storage_tx, storage_rx) = std::sync::mpsc::sync_channel::<(SettingsRequest, u64, UnixStream)>(0);
    let (connect_tx, connect_rx) = std::sync::mpsc::sync_channel::<(bool, u64, UnixStream)>(0);
    let connect_config = config_path.clone();
    std::thread::Builder::new()
        .name("apparent-connect-settings".into())
        .spawn(move || {
            while let Ok((backend, deadline, stream)) = connect_rx.recv() {
                if backend {
                    super::connect_settings::backend::serve(stream, deadline, &connect_config);
                } else {
                    super::connect_settings::serve(stream, deadline, &connect_config);
                }
            }
        })?;
    std::thread::Builder::new().name("apparent-storage-control".into()).spawn(move || {
        while let Ok((request, deadline, mut stream)) = storage_rx.recv() {
            let response = request.execute(&config_path, deadline);
            let _ = writeln!(stream, "{response}");
        }
    })?;
    std::thread::Builder::new()
        .name("apparent-agent-control".into())
        .spawn(move || {
            let _owner = owner;
            while !tx.is_disconnected() {
                let (mut stream, _) = match listener.accept() {
                    Ok(accepted) => accepted,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(20));
                        continue;
                    }
                    Err(_) => break,
                };
                if same_user(&stream).is_err() {
                    continue;
                }
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
                let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
                let mut line = String::new();
                let read_deadline = Instant::now() + Duration::from_secs(3);
                let Ok(read_stream) = stream.try_clone() else { continue; };
                let mut reader = BufReader::new(DeadlineStream { stream: read_stream, deadline: read_deadline }.take(131073));
                let valid_frame = reader.read_line(&mut line).is_ok() && line.ends_with('\n') && line.len() <= 131072 && reader.buffer().is_empty();
                let response = if valid_frame {
                    if let Some(wire) = parse_request(line.trim()) {
                        let (action, deadline_ms) = match wire {
                            Wire::NativeHost(request) => {
                                let id = request.id.clone();
                                let deadline_ms = clock_ms() + request.deadline.saturating_sub(storage_clock_ms()).min(3000);
                                let (reply, receive) = std::sync::mpsc::sync_channel(1);
                                if tx.try_send(Request { action: None, native_host: Some(request), reply, deadline_ms }).is_ok() {
                                    let result = receive.recv_timeout(Duration::from_millis(deadline_ms.saturating_sub(clock_ms())))
                                        .unwrap_or_else(|_| r#"{"status":"outcome-unknown"}"#.into());
                                    let result = serde_json::from_str::<serde_json::Value>(&result)
                                        .unwrap_or_else(|_| serde_json::json!({"status":"outcome-unknown"}));
                                    let _ = writeln!(stream, "{}", serde_json::json!({"protocol":1,"id":id,"result":result}));
                                } else {
                                    let _ = writeln!(stream, "{}", serde_json::json!({"protocol":1,"id":id,"result":{"status":"refused","reason":"busy"}}));
                                }
                                continue;
                            }
                            wire @ (Wire::Connect(_) | Wire::ConnectBackend(_)) => {
                                let (backend, deadline) = match wire {
                                    Wire::Connect(deadline) => (false, deadline),
                                    Wire::ConnectBackend(deadline) => (true, deadline),
                                    _ => unreachable!(),
                                };
                                if line.len() > 4096 {
                                    let _ = writeln!(stream, "{{\"error\":\"invalid_request\"}}");
                                    continue;
                                }
                                match connect_tx.try_send((backend, deadline, stream)) {
                                    Ok(()) => {}
                                    Err(std::sync::mpsc::TrySendError::Full((_, _, mut stream))
                                        | std::sync::mpsc::TrySendError::Disconnected((_, _, mut stream))) => {
                                        let _ = writeln!(stream, "{{\"error\":\"app_busy\"}}");
                                    }
                                }
                                continue;
                            }
                            Wire::Capture(action, deadline) => (action, deadline),
                            Wire::PublicShare(action, value, deadline) => (Some((action, value)), deadline),
                            Wire::Storage(request) => {
                                let deadline = request.deadline().min(storage_clock_ms() + 3000);
                                match storage_tx.try_send((SettingsRequest::Storage(request), deadline, stream)) {
                                    Ok(()) => {},
                                    Err(std::sync::mpsc::TrySendError::Full((_, _, mut stream)) | std::sync::mpsc::TrySendError::Disconnected((_, _, mut stream))) => { let _ = writeln!(stream, "{{\"error\":\"app_busy\"}}"); }
                                }
                                continue;
                            }
                            Wire::Microphone(request) => {
                                let deadline = request.deadline().min(storage_clock_ms() + 3000);
                                match storage_tx.try_send((SettingsRequest::Microphone(request), deadline, stream)) {
                                    Ok(()) => {},
                                    Err(std::sync::mpsc::TrySendError::Full((_, _, mut stream)) | std::sync::mpsc::TrySendError::Disconnected((_, _, mut stream))) => { let _ = writeln!(stream, "{{\"error\":\"app_busy\"}}"); }
                                }
                                continue;
                            }
                            Wire::ErrorReporting(request) => {
                                let deadline = request.deadline().min(storage_clock_ms() + 3000);
                                match storage_tx.try_send((SettingsRequest::ErrorReporting(request), deadline, stream)) {
                                    Ok(()) => {},
                                    Err(std::sync::mpsc::TrySendError::Full((_, _, mut stream)) | std::sync::mpsc::TrySendError::Disconnected((_, _, mut stream))) => { let _ = writeln!(stream, "{{\"error\":\"app_busy\"}}"); }
                                }
                                continue;
                            }
                        };
                        let (reply, receive) = std::sync::mpsc::sync_channel(1);
                        if tx
                            .try_send(Request {
                                action,
                                #[cfg(any(target_os = "macos", target_os = "linux"))]
                                native_host: None,
                                reply,
                                deadline_ms,
                            })
                            .is_ok()
                        {
                            receive
                                .recv_timeout(Duration::from_millis(
                                    deadline_ms.saturating_sub(clock_ms()),
                                ))
                                .unwrap_or_else(|_| "{\"error\":\"app_unresponsive\"}".into())
                        } else {
                            "{\"error\":\"app_busy\"}".into()
                        }
                    } else {
                        "{\"error\":\"invalid_request\"}".to_owned()
                    }
                } else {
                    "{\"error\":\"invalid_request\"}".into()
                };
                let _ = writeln!(stream, "{response}");
            }
            // Only this listener can own the pathname until the lock is released.
            let _ = std::fs::remove_file(path);
        })?;
    Ok(rx)
}



/// Check GUI ownership without asking its UI to respond or connecting to its socket.
/// Missing/released locks mean no owner; unsafe paths and I/O errors fail closed.
pub fn running_owner() -> Result<bool> {
    match socket_path(false) {
        Ok(path) => running_owner_at(path),
        Err(error)
            if error
                .downcast_ref::<std::io::Error>()
                .is_some_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

fn running_owner_at(path: PathBuf) -> Result<bool> {
    let owner = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path.with_extension("lock"))
    {
        Ok(owner) => owner,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    Ok(!try_lock_owner(&owner)?)
}

fn try_lock_owner(owner: &std::fs::File) -> Result<bool> {
    let meta = owner.metadata()?;
    ensure!(
        meta.is_file() && meta.uid() == unsafe { libc::geteuid() } && meta.mode() & 0o777 == 0o600,
        "unsafe control owner lock"
    );
    if unsafe { libc::flock(owner.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.kind() == std::io::ErrorKind::WouldBlock {
        Ok(false)
    } else {
        Err(error.into())
    }
}

#[cfg(test)]
mod tests {
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


    #[test]
    fn drawing_admission_blocks_stale_idle_and_expired_mutations() {
        assert_eq!(super::drawing_admission(Some("new"), Some("old"), false), Err("conflict"));
        assert_eq!(super::drawing_admission(None, Some("old"), false), Err("conflict"));
        assert_eq!(super::drawing_admission(Some("new"), Some("new"), true), Err("admission_expired"));
        assert_eq!(super::drawing_admission(Some("new"), Some("new"), false), Ok(()));
        assert_eq!(super::drawing_admission(None, None, false), Ok(()));
    }

    #[test]
    fn drawing_contract_rejects_unbound_undo_and_extra_fields() {
        assert!(super::parse_request(r#"{"drawing":"read","deadline":1}"#).is_some());
        assert!(super::parse_request(r#"{"drawing":"undo","expected":"0123456789abcdef","deadline":1}"#).is_some());
        for invalid in [
            r#"{"drawing":"undo","deadline":1}"#,
            r#"{"drawing":"undo","expected":"invalid","deadline":1}"#,
            r#"{"drawing":"read","deadline":1,"expected":"0123456789abcdef"}"#,
            r#"{"drawing":"read","deadline":1,"path":"private"}"#,
        ] { assert!(super::parse_request(invalid).is_none()); }
    }

    #[test]
    fn capture_reference_policy_checks_selection_before_any_clipboard_write() {
        use super::super::super::capture_reference_response as reply;
        let directory = std::path::Path::new("/private/captures/older capture — test.apparent");
        let read = reply(Some(directory), None, true, false, |_| panic!("read wrote clipboard"));
        assert_eq!(read, serde_json::json!({"reference":{"id":"older capture — test.apparent","copied":false}}));
        assert_eq!(reply(None, None, true, false, |_| panic!())["reference"]["id"], serde_json::Value::Null);
        for (dir, expected, enabled, expired, error) in [
            (Some(directory), Some("stale.apparent"), true, false, "conflict"),
            (None, Some("stale.apparent"), true, false, "no_selection"),
            (Some(directory), Some("older capture — test.apparent"), false, false, "feature_disabled"),
            (Some(directory), Some("older capture — test.apparent"), true, true, "admission_expired"),
        ] {
            assert_eq!(reply(dir, expected, enabled, expired, |_| panic!("refused copy wrote clipboard"))["error"], error);
        }
        let calls = std::cell::Cell::new(0);
        let copied = reply(Some(directory), Some("older capture — test.apparent"), true, false, |id| {
            assert_eq!(id, "older capture — test.apparent");
            calls.set(calls.get() + 1);
            Ok(())
        });
        assert_eq!(calls.get(), 1);
        assert_eq!(copied["reference"]["copied"], true);
        let failed = reply(Some(directory), Some("older capture — test.apparent"), true, false,
            |_| anyhow::bail!("private backend detail"));
        assert_eq!(failed, serde_json::json!({"error":"clipboard_failed"}));
    }

    #[test]
    fn public_share_wire_requires_disclosure_and_translates_clock_budget() {
        let deadline = storage_clock_ms() + 20_000;
        let valid=serde_json::json!({"publicShare":"start","expected":"saved.apparent","disclosed":true,"deadline":deadline});
        let Some(Wire::PublicShare(Action::PublicShareStart,_,until))=parse_request(&valid.to_string()) else {panic!("not admitted")};
        assert!(until>=clock_ms() && until<=clock_ms()+3000);
        for value in [
            serde_json::json!({"publicShare":"start","expected":"saved.apparent","disclosed":false,"deadline":deadline}),
            serde_json::json!({"publicShare":"start","expected":"/private/path","disclosed":true,"deadline":deadline}),
            serde_json::json!({"publicShare":"deliver","operation":"invalid","deadline":deadline}),
            serde_json::json!({"publicShare":"read","operation":"a".repeat(32),"deadline":deadline,"extra":true})
        ] {assert!(parse_request(&value.to_string()).is_none());}
    }

    #[test]
    fn capture_reference_wire_admits_only_bounded_named_requests() {
        assert!(super::parse_request(r#"{"reference":"read","deadline":999999999999}"#).is_some());
        assert!(super::parse_request(r#"{"reference":"copy","deadline":999999999999,"expected":"older capture — test.apparent"}"#).is_some());
        for invalid in [
            r#"{"reference":"read","deadline":1,"expected":"unexpected"}"#,
            r#"{"reference":"copy","deadline":1}"#,
            r#"{"reference":"copy","deadline":1,"expected":"/private/path.apparent"}"#,
            r#"{"reference":"copy","deadline":1,"expected":""}"#,
            r#"{"reference":"read","deadline":1,"storage":"read"}"#,
        ] { assert!(super::parse_request(invalid).is_none(), "{invalid}"); }
    }

    #[test]
    fn capture_reference_deadlines_translate_remaining_time_and_expire() {
        let (_, expired) = super::Reference::Read { deadline: 0 }.admission().unwrap();
        assert!(expired <= super::clock_ms());
        let before = super::clock_ms();
        let (_, limited) = super::Reference::Read {
            deadline: super::storage_clock_ms() + 60_000,
        }.admission().unwrap();
        assert!(limited >= before && limited <= super::clock_ms() + 3000);
    }

    #[test]
    fn storage_retention_contract_preserves_unknowns_and_captures() {
        let (path, listener) = transport_fixture();
        drop(listener);
        let dir = path.parent().unwrap();
        let config = dir.join("config.json");
        let captures = dir.join("captures");
        std::fs::create_dir(&captures).unwrap();
        std::fs::write(captures.join("keep"), vec![42; 1_000_000]).unwrap();
        let call = |value: serde_json::Value| {
            let Some(Wire::Storage(request)) = parse_request(&value.to_string()) else { panic!("invalid request") };
            request.execute(&config, storage_clock_ms() + 3000)
        };
        let read = || call(serde_json::json!({"storage":"retention","deadline":storage_clock_ms()+3000}));
        assert!(read().get("error").is_none(), "Storage is available by default");
        std::fs::write(&config, r#"{"experiments":{"electronCaptureLocation":false}}"#).unwrap();
        assert_eq!(read()["error"], "feature_disabled");
        std::fs::write(&config, serde_json::json!({"experiments":{"electronCaptureLocation":true}, "capturesDir":captures,
            "secret":"keep", "unknown":{"nested":true}, "retention":{"mediaDays":2,"bundleDays":null,"future":"keep"}}).to_string()).unwrap();
        let initial = read();
        assert_eq!(initial["retention"]["mediaDays"], serde_json::json!({"present":true,"value":2}));
        assert_eq!(initial["retention"]["bundleDays"], serde_json::json!({"present":true,"value":null}));
        let month = serde_json::json!({"mediaDays":30,"bundleDays":90});
        let save = |policy: serde_json::Value, expected: &serde_json::Value| call(serde_json::json!({"storage":"saveretention", "policy":policy,"expected":expected,"deadline":storage_clock_ms()+3000}));
        crate::shared::handoff::set_config_value_in(&config, "concurrentSetting", "kept".into()).unwrap();
        let saved = save(month.clone(), &initial["revision"]);
        assert_eq!(saved["retention"]["mediaDays"]["value"], 30);
        assert_eq!(saved["retention"]["bundleDays"]["value"], 90);
        assert_eq!(read(), saved);
        assert_eq!(save(serde_json::json!({"mediaDays":null,"bundleDays":null}), &initial["revision"])["error"], "conflict");
        let reset = call(serde_json::json!({"storage":"resetretention","expected":saved["revision"],"deadline":storage_clock_ms()+3000}));
        assert_eq!(reset["retention"]["mediaDays"], serde_json::json!({"present":false,"value":null}));
        assert_eq!(reset["retention"]["bundleDays"], serde_json::json!({"present":false,"value":null}));
        assert_eq!(reset["custom"], false);
        let raw = crate::shared::handoff::capture_location::read_raw(&config).unwrap();
        assert_eq!(raw["secret"], "keep");
        assert_eq!(raw["unknown"]["nested"], true);
        assert_eq!(raw["concurrentSetting"], "kept");
        assert_eq!(raw["retention"]["future"], "keep");
        assert_eq!(raw["capturesDir"], captures.to_str().unwrap());
        std::fs::write(&config, r#"{"experiments":{"electronCaptureLocation":true},"retention":{"bundleDays":null}}"#).unwrap();
        let media_absent = read();
        std::fs::write(&config, r#"{"experiments":{"electronCaptureLocation":true},"retention":{"mediaDays":null,"bundleDays":null}}"#).unwrap();
        let media_explicit_null = read();
        assert_ne!(media_absent["revision"], media_explicit_null["revision"],
            "CAS fingerprints distinguish an absent owner value from explicit JSON null");
        assert_eq!(save(serde_json::json!({"mediaDays":30,"bundleDays":90}), &media_absent["revision"])["error"], "conflict");
        std::fs::write(&config, r#"{"experiments":{"electronCaptureLocation":true},"retention":{"mediaDays":"private-value","bundleDays":30}}"#).unwrap();
        let malformed = read();
        assert_eq!(malformed["retention"]["mediaDays"], serde_json::json!({"present":false,"value":null}));
        assert_eq!(malformed["retention"]["bundleDays"], serde_json::json!({"present":false,"value":null}));
        assert_eq!(malformed["custom"], true);
        assert!(!malformed.to_string().contains("private-value"));
        std::fs::write(&config, serde_json::json!({"experiments":{"electronCaptureLocation":true}, "capturesDir":captures,
            "secret":"keep", "unknown":{"nested":true}, "retention":{"mediaDays":null,"bundleDays":null,"future":"keep"}}).to_string()).unwrap();
        assert_eq!(call(serde_json::json!({"storage":"usage","deadline":storage_clock_ms()+3000}))["usage"]["formatted"], "1 MB");
        assert_eq!(super::super::storage_preferences::usage(&config, || false), Err("usage_unavailable"));
        let month_policy = super::super::storage_preferences::SavePolicy { media_days: 30.into(), bundle_days: 90.into() };
        let before_unknown = read();
        let bytes = std::fs::read(&config).unwrap();
        assert_eq!(super::super::storage_preferences::save(&config, Some(&month_policy), before_unknown["revision"].as_str().unwrap(), || false), Err("admission_expired"));
        assert_eq!(std::fs::read(&config).unwrap(), bytes);
        crate::shared::handoff::set_config_directory_sync_failure(true);
        assert_eq!(super::super::storage_preferences::save(&config, Some(&month_policy), before_unknown["revision"].as_str().unwrap(), || true), Err("outcome_unknown"));
        let after_unknown = read();
        crate::shared::handoff::set_config_directory_sync_failure(true);
        assert_eq!(super::super::storage_preferences::save(&config, Some(&month_policy), after_unknown["revision"].as_str().unwrap(), || true), Err("outcome_unknown"), "publishing identical values can also have uncertain durability");
        assert_eq!(after_unknown["retention"]["mediaDays"]["value"], 30);
        assert_eq!(after_unknown["retention"]["bundleDays"]["value"], 90);
        // Fail an actual config commit (owned malformed file), not an optimistic UI mock.
        std::fs::write(&config, "malformed").unwrap();
        assert_eq!(super::super::storage_preferences::save(&config, Some(&month_policy), after_unknown["revision"].as_str().unwrap(), || true), Err("config_write_failed"));
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "malformed");
        assert_eq!(std::fs::read(captures.join("keep")).unwrap(), vec![42; 1_000_000]);
        for value in [
            serde_json::json!({"storage":"saveretention","policy":{"mediaDays":-1,"bundleDays":90},"expected":"aaaaaaaaaaaaaaaa","deadline":1}),
            serde_json::json!({"storage":"saveretention","policy":{"mediaDays":7,"bundleDays":90,"unexpected":true},"expected":"aaaaaaaaaaaaaaaa","deadline":1}),
            serde_json::json!({"storage":"saveretention","policy":{"mediaDays":7},"expected":"aaaaaaaaaaaaaaaa","deadline":1}),
            serde_json::json!({"storage":"usage","path":"/forbidden","deadline":1}),
            serde_json::json!({"storage":"resetretention","expected":"invalid","deadline":1}),
        ] { assert!(parse_request(&value.to_string()).is_none()); }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn storage_read_is_admitted_without_removing_legacy_state() {
        assert!(super::parse_request(r#"{"storage":"read","deadline":999999999999}"#).is_some());
        assert!(super::parse_request("state").is_some());
        for invalid in [
            r#"{"storage":"read","deadline":1,"path":"/not-allowed"}"#,
            r#"{"storage":"read","deadline":1,"secret":"hidden"}"#,
            r#"{"storage":"reset","deadline":1}"#,
            r#"{"storage":"set","deadline":1,"expected":"bad","path":"/tmp"}"#,
            r#"{"storage":"read","deadline":-1}"#,
        ] { assert!(super::parse_request(invalid).is_none(), "{invalid}"); }
        for valid in [
            r#"{"storage":"locationfacts","deadline":999999999999}"#,
            r#"{"storage":"probelocation","path":"/tmp/candidate","expected":"0123456789abcdef","deadline":999999999999}"#,
            r#"{"storage":"setlocation","path":"/tmp/candidate","expected":"0123456789abcdef","deadline":999999999999}"#,
            r#"{"storage":"resetlocation","expected":"0123456789abcdef","deadline":999999999999}"#,
        ] { assert!(super::parse_request(valid).is_some(), "{valid}"); }
        for invalid in [
            r#"{"storage":"probelocation","path":"relative","expected":"0123456789abcdef","deadline":1}"#,
            r#"{"storage":"probelocation","path":"/tmp/candidate","expected":"invalid","deadline":1}"#,
            r#"{"storage":"setlocation","path":"/tmp/candidate","expected":"0123456789abcdef","deadline":1,"secret":"hidden"}"#,
        ] { assert!(super::parse_request(invalid).is_none(), "{invalid}"); }
    }

    #[test]
    fn storage_location_facts_are_bounded_and_probe_checks_owner_revision() {
        let (path, listener) = transport_fixture();
        drop(listener);
        let directory = path.parent().unwrap();
        let config = directory.join("config.json");
        std::fs::write(&config, r#"{"privateToken":"keep","capturesDir":"/saved"}"#).unwrap();
        let facts_request = Storage::LocationFacts { deadline: storage_clock_ms() + 3000 };
        let facts = facts_request.execute(&config, storage_clock_ms() + 3000);
        assert_eq!(facts["location"]["saved"]["kind"], "string");
        assert_eq!(facts["location"]["saved"]["value"], "/saved");
        assert_eq!(facts["location"]["revision"].as_str().unwrap().len(), 16);
        assert!(facts.get("privateToken").is_none());
        let revision = facts["location"]["revision"].as_str().unwrap().to_owned();
        let candidate = directory.join("probe");
        let probe = Storage::ProbeLocation { path: candidate.to_str().unwrap().into(), expected: revision.clone(), deadline: storage_clock_ms() + 3000 };
        assert_eq!(probe.execute(&config, storage_clock_ms() + 3000)["probe"]["available"], true);
        let mut raw: serde_json::Value = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
        raw["capturesDir"] = "/changed".into();
        std::fs::write(&config, serde_json::to_vec(&raw).unwrap()).unwrap();
        assert_eq!(probe.execute(&config, storage_clock_ms() + 3000)["error"], "conflict");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn storage_admission_reads_file_activation_and_reports_invalid_config() {
        let (path, listener) = transport_fixture();
        drop(listener);
        let config = path.parent().unwrap().join("config.json");
        let request = Storage::Read { deadline: storage_clock_ms() + 3000 };
        let read = || request.execute(&config, storage_clock_ms() + 3000);
        assert!(read().get("error").is_none(), "Storage is available by default");
        std::fs::write(&config, r#"{"experiments":{"electronCaptureLocation":false}}"#).unwrap();
        assert_eq!(read()["error"], "feature_disabled");
        std::fs::write(&config, r#"{"experiments":{"electronCaptureLocation":true}}"#).unwrap();
        assert!(read().get("storage").is_some());
        std::fs::write(&config, "malformed").unwrap();
        assert_eq!(read()["error"], "config_read_failed");
        assert_eq!(std::fs::read_to_string(&config).unwrap(), "malformed");
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
    }

    #[test]
    fn storage_socket_roundtrip_and_locked_expiry_leave_capture_controls_available() {
        use crate::shared::handoff::capture_location;
        let (path, stale) = transport_fixture();
        drop(stale);
        let dir = path.parent().unwrap().to_owned();
        let config = dir.join("config.json");
        std::fs::write(&config, r#"{"privateToken":"keep","experiments":{"electronCaptureLocation":true}}"#).unwrap();
        let receiver = listen_at_config(path.clone(), config.clone()).unwrap();
        let call = |value: serde_json::Value| {
            let mut stream = UnixStream::connect(&path).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(4))).unwrap();
            writeln!(stream, "{value}").unwrap();
            let mut response = String::new();
            BufReader::new(stream).read_line(&mut response).unwrap();
            serde_json::from_str::<serde_json::Value>(&response).unwrap()
        };
        std::thread::sleep(Duration::from_millis(30));
        let initial = call(serde_json::json!({"storage":"read", "deadline":storage_clock_ms()+3000}));
        assert_eq!(initial["storage"]["source"], "builtin");
        assert!(initial.get("privateToken").is_none());
        let selected = dir.join("new é captures");
        let saved = call(serde_json::json!({"storage":"set", "path":selected, "expected":initial["storage"]["revision"], "deadline":storage_clock_ms()+3000}));
        assert_eq!(saved["storage"]["savedPath"], selected.to_str().unwrap());
        assert_eq!(call(serde_json::json!({"storage":"reset", "expected":initial["storage"]["revision"], "deadline":storage_clock_ms()+3000}))["error"], "conflict");
        let reset = call(serde_json::json!({"storage":"reset", "expected":saved["storage"]["revision"], "deadline":storage_clock_ms()+3000}));
        assert!(reset["storage"]["savedPath"].is_null());
        let lock = std::fs::OpenOptions::new().read(true).write(true).open(config.with_extension("json.lock")).unwrap();
        assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) }, 0);
        let mut blocked = UnixStream::connect(&path).unwrap();
        blocked.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        // Use the full admission budget, not a 150 ms scheduling race. The held
        // lock keeps the admitted writer occupied until we explicitly release it.
        let blocked_deadline = storage_clock_ms() + 3000;
        writeln!(blocked, "{}", serde_json::json!({"storage":"set", "path": selected, "expected":reset["storage"]["revision"], "deadline":blocked_deadline})).unwrap();
        let busy = call(serde_json::json!({"storage":"read", "deadline":storage_clock_ms()+3000}));
        assert_eq!(busy["error"], "app_busy", "{busy}");
        let mut state = UnixStream::connect(&path).unwrap();
        writeln!(state, "state").unwrap();
        let request = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(request.action.is_none());
        request.reply.send("{\"state\":{\"phase\":\"idle\"}}".into()).unwrap();
        let mut response = String::new();
        BufReader::new(state).read_line(&mut response).unwrap();
        assert!(response.contains("idle"));
        std::thread::sleep(Duration::from_millis(blocked_deadline.saturating_sub(storage_clock_ms()) + 1));
        drop(lock);
        response.clear();
        BufReader::new(blocked).read_line(&mut response).unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&response).unwrap()["error"], "admission_expired");
        assert!(!capture_location::read_in(&config).unwrap().custom);
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&std::fs::read(&config).unwrap()).unwrap()["privateToken"], "keep");
        for bytes in [b"{\"storage\":\"read\"}".as_slice(), b"state\nstate\n", &[b'x'; 131073]] {
            let mut stream = UnixStream::connect(&path).unwrap();
            stream.write_all(bytes).unwrap();
            stream.shutdown(std::net::Shutdown::Write).unwrap();
            response.clear();
            BufReader::new(stream).read_line(&mut response).unwrap();
            assert!(response.contains("invalid_request"));
        }
        drop(receiver);
        for _ in 0..50 { if !path.exists() { break; } std::thread::sleep(Duration::from_millis(20)); }
        assert!(!path.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn native_host_socket_roundtrip_is_owner_bound_and_correlated() {
        let (path, stale) = transport_fixture();
        drop(stale);
        let directory = path.parent().unwrap().to_owned();
        let config = directory.join("config.json");
        let receiver = listen_at_config(path.clone(), config).unwrap();
        let id = "native-host-test-01";
        let deadline = storage_clock_ms() + 2500;
        let request = serde_json::json!({
            "protocol":1,"id":id,"owner":"record","operation":"readSnapshot","deadline":deadline
        });
        let client_path = path.clone();
        let client = std::thread::spawn(move || {
            let mut stream = UnixStream::connect(client_path).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            writeln!(stream, "{request}").unwrap();
            let mut response = String::new();
            BufReader::new(stream).read_line(&mut response).unwrap();
            serde_json::from_str::<serde_json::Value>(&response).unwrap()
        });
        let admitted = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        let native = admitted.native_host.as_ref().expect("native host request stays distinct from capture actions");
        assert_eq!(native.id, id);
        assert!(matches!(&native.operation, super::super::native_host::Operation::ReadSnapshot));
        assert!(admitted.action.is_none());
        admitted.reply.send(r#"{"api":1,"scope":"running-desktop-process"}"#.into()).unwrap();
        let response = client.join().unwrap();
        assert_eq!(response["protocol"], 1);
        assert_eq!(response["id"], id);
        assert_eq!(response["result"]["scope"], "running-desktop-process");
        drop(receiver);
        for _ in 0..50 { if !path.exists() { break; } std::thread::sleep(Duration::from_millis(20)); }
        assert!(!path.exists(), "owned control listener did not clean up");
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn native_host_public_node_client_gets_unknown_for_admitted_lost_mutation_reply() {
        let home = std::env::temp_dir().join(format!("nh-{}", &new_id().unwrap()[..8]));
        let config_dir = home.join(".config");
        let apparent_dir = config_dir.join("apparent");
        let directory = apparent_dir.join("agent-control");
        std::fs::create_dir_all(&directory).unwrap();
        for path in [&home, &config_dir, &apparent_dir, &directory] {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let config = apparent_dir.join("config.json");
        let path = directory.join("capture.sock");
        let receiver = listen_at_config(path.clone(), config).unwrap();
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("composition/native-host/test/rust-roundtrip.ts");
        let client_home = home.clone();
        let client = std::thread::spawn(move || {
            std::process::Command::new("node")
                .env("HOME", &client_home)
                .arg("--experimental-strip-types")
                .arg(script)
                .output()
                .unwrap()
        });

        let read = receiver.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(matches!(read.native_host.as_ref().unwrap().operation, super::super::native_host::Operation::ReadSnapshot));
        let snapshot = serde_json::json!({
            "api":1,"processEpoch":"rust-fixture","revision":0,
            "scope":"running-desktop-process","freshness":"cached-gui-observation",
            "permissions":[],"shortcuts":{
                "record":{"configured":null,"effective":null,"registration":"unknown","canValidate":true,"canSave":false},
                "paste":{"configured":null,"effective":null,"registration":"unknown","canValidate":true,"canSave":false}
            }
        });
        read.reply.send(snapshot.to_string()).unwrap();

        let save = receiver.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(matches!(save.native_host.as_ref().unwrap().operation, super::super::native_host::Operation::SaveRecordShortcut(_)));
        assert!(save.action.is_none(), "native host mutation must not become a capture action");
        drop(save.reply); // Simulate a dispatched mutation whose reply was lost.
        let output = client.join().unwrap();
        drop(receiver);
        for _ in 0..50 { if !path.exists() { break; } std::thread::sleep(Duration::from_millis(20)); }
        assert!(!path.exists(), "owned control listener did not clean up");
        assert!(output.status.success(), "Node client: {}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        assert!(String::from_utf8_lossy(&output.stdout).contains("preserve an admitted lost-reply outcome"));
        std::fs::remove_dir_all(home).unwrap();
    }
    #[test]
    fn reporting_socket_is_typed_private_and_rechecks_admission_after_config_lock() {
        let (path, stale) = transport_fixture();
        drop(stale);
        let dir = path.parent().unwrap().to_owned();
        let config = dir.join("config.json");
        std::fs::write(&config, r#"{"privateToken":"synthetic-keep","unknown":{"keep":true}}"#).unwrap();
        let receiver = listen_at_config(path.clone(), config.clone()).unwrap();
        let call = |value: serde_json::Value| {
            let mut stream = UnixStream::connect(&path).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(4))).unwrap();
            writeln!(stream, "{value}").unwrap();
            let mut response = String::new();
            BufReader::new(stream).read_line(&mut response).unwrap();
            serde_json::from_str::<serde_json::Value>(&response).unwrap()
        };
        std::thread::sleep(Duration::from_millis(30));
        assert_eq!(call(serde_json::json!({"errorReporting":"read","deadline":storage_clock_ms()+3000})), serde_json::json!({"errorReporting":{"enabled":true}}));
        for kind in ["main_exception", "main_rejection", "renderer_exception", "renderer_rejection"] {
            assert_eq!(call(serde_json::json!({"errorReporting":"exception","kind":kind,"deadline":storage_clock_ms()+3000})), serde_json::json!({"accepted":true}));
            assert_eq!(call(serde_json::json!({"errorReporting":"exception","kind":kind,"deadline":1}))["error"], "admission_expired");
            assert_eq!(call(serde_json::json!({"errorReporting":"exception","kind":kind,"message":"private text","deadline":storage_clock_ms()+3000}))["error"], "invalid_request");
        }
        for invalid in [
            serde_json::json!({"errorReporting":"set","deadline":1}),
            serde_json::json!({"errorReporting":"set","enabled":"false","deadline":1}),
            serde_json::json!({"errorReporting":"set","enabled":null,"deadline":1}),
            serde_json::json!({"errorReporting":"read","enabled":false,"deadline":1}),
            serde_json::json!({"errorReporting":"read","path":"/private","deadline":1}),
            serde_json::json!({"errorReporting":"other","deadline":1}),
            serde_json::json!({"errorReporting":"read","deadline":-1}),
        ] { assert_eq!(call(invalid)["error"], "invalid_request"); }
        assert_eq!(call(serde_json::json!({"errorReporting":"set","enabled":false,"deadline":storage_clock_ms()+3000}))["errorReporting"]["enabled"], false);
        assert_eq!(call(serde_json::json!({"errorReporting":"read","deadline":storage_clock_ms()+3000}))["errorReporting"]["enabled"], false);
        assert_eq!(call(serde_json::json!({"errorReporting":"set","enabled":true,"deadline":storage_clock_ms()+3000}))["errorReporting"]["enabled"], true);
        let before = std::fs::read(&config).unwrap();
        let lock = std::fs::OpenOptions::new().read(true).write(true).open(config.with_extension("json.lock")).unwrap();
        assert_eq!(unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX) }, 0);
        let mut blocked = UnixStream::connect(&path).unwrap();
        blocked.set_read_timeout(Some(Duration::from_secs(4))).unwrap();
        let deadline = storage_clock_ms()+3000;
        writeln!(blocked, "{}", serde_json::json!({"errorReporting":"set","enabled":false,"deadline":deadline})).unwrap();
        assert_eq!(call(serde_json::json!({"errorReporting":"read","deadline":storage_clock_ms()+3000}))["error"], "app_busy");
        let mut state = UnixStream::connect(&path).unwrap();
        writeln!(state, "state").unwrap();
        let request = receiver.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(request.action.is_none());
        request.reply.send("{\"state\":{\"phase\":\"idle\"}}".into()).unwrap();
        let mut response = String::new();
        BufReader::new(state).read_line(&mut response).unwrap();
        assert!(response.contains("idle"));
        std::thread::sleep(Duration::from_millis(deadline.saturating_sub(storage_clock_ms())+1));
        drop(lock);
        response.clear();
        BufReader::new(blocked).read_line(&mut response).unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&response).unwrap()["error"], "admission_expired");
        assert_eq!(std::fs::read(&config).unwrap(), before);
        let raw: serde_json::Value = serde_json::from_slice(&before).unwrap();
        assert_eq!(raw["privateToken"], "synthetic-keep");
        assert_eq!(raw["unknown"]["keep"], true);
        std::fs::write(&config, "malformed").unwrap();
        assert_eq!(call(serde_json::json!({"errorReporting":"read","deadline":storage_clock_ms()+3000}))["error"], "config_read_failed");
        drop(receiver);
        for _ in 0..50 { if !path.exists() { break; } std::thread::sleep(Duration::from_millis(20)); }
        assert!(!path.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    use super::*;

    #[test]
    #[ignore = "needs the frontend Electron binary; see frontend/README.md"]
    fn connect_settings_electron_rust_roundtrip() {
        // Unit-test builds deliberately disable default experiments. Exercise
        // the normal encrypted endpoint-enrollment path in this isolated lane.
        crate::shared::handoff::enable_experiment(apparent_connect_client::HOSTED_ENCRYPTION);
        crate::shared::handoff::enable_experiment(apparent_connect_client::HOSTED_SYNC);
        let electron = std::env::var_os("APPARENT_CONNECT_TEST_ELECTRON")
            .expect("Set APPARENT_CONNECT_TEST_ELECTRON to the frontend Electron executable");
        let home = std::env::temp_dir().join(format!("ce-{}", &new_id().unwrap()[..8]));
        let directory = home.join(".config/apparent/agent-control");
        std::fs::create_dir_all(&directory).unwrap();
        // The production client correctly refuses a group-writable state parent.
        // Do not inherit the host's umask for this disposable fixture HOME.
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = home.join("config.json");
        let path = directory.join("capture.sock");
        let receiver = listen_at_config(path.clone(), config.clone()).unwrap();
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("capture/desktop/ui/frontend/tests/rust-connect.cjs");
        let output = std::process::Command::new(electron)
            .env("ELECTRON_RUN_AS_NODE", "1")
            .arg(script).arg(&home).arg(&config).output().unwrap();
        drop(receiver);
        for _ in 0..50 { if !path.exists() { break; } std::thread::sleep(Duration::from_millis(20)); }
        assert!(!path.exists(), "Owned listener did not clean up");
        if output.status.success() { std::fs::remove_dir_all(&home).unwrap(); }
        assert!(output.status.success(), "Electron fixture retained at {}: {}{}", home.display(), String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        println!("{}", String::from_utf8_lossy(&output.stdout));
    }

    #[test]
    #[ignore = "needs the frontend Electron binary; see frontend/README.md"]
    fn storage_electron_rust_roundtrip() {
        let electron = std::env::var_os("APPARENT_STORAGE_TEST_ELECTRON")
            .expect("Set APPARENT_STORAGE_TEST_ELECTRON to the installed frontend Electron executable");
        let home = std::env::temp_dir().join(format!("se-{}", &new_id().unwrap()[..8]));
        let directory = home.join(".config/apparent/agent-control");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
        let config = home.join("config.json");
        let path = directory.join("capture.sock");
        let receiver = listen_at_config(path.clone(), config.clone()).unwrap();
        let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("capture/desktop/ui/frontend/tests/rust-storage.cjs");
        let output = std::process::Command::new(electron)
            .env("ELECTRON_RUN_AS_NODE", "1")
            .env_remove("APPARENT_CAPTURES_DIR")
            .arg(script).arg(&home).arg(&config).output().unwrap();
        drop(receiver);
        for _ in 0..50 { if !path.exists() { break; } std::thread::sleep(Duration::from_millis(20)); }
        std::fs::remove_dir_all(home).unwrap();
        assert!(output.status.success(), "Electron: {}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
        println!("{}", String::from_utf8_lossy(&output.stdout));
    }

    #[test]
    fn owned_admission_is_once_only_and_rejections_cannot_start_later() {
        let mut registry = Registry::default();
        let mut starts = 0;
        for (id, occupied, expired) in [
            ("a", false, false),
            ("b", true, false),
            ("late", false, true),
            ("a", false, false),
            ("b", false, false),
            ("late", false, false),
        ] {
            starts += usize::from(registry.admit(id, occupied, expired));
        }
        assert_eq!(starts, 1);
        assert_eq!(registry.inspect("a")["status"], "starting");
        assert_eq!(registry.inspect("b")["error"], "occupied");
        assert_eq!(registry.inspect("late")["error"], "admission_expired");
    }

    #[test]
    fn older_saved_result_survives_new_capture_and_stop_retry_does_not_stop_it() {
        let mut registry = Registry::default();
        registry.begin("old");
        registry.finish("old", Ok(std::path::Path::new("/captures/exact.apparent")));
        registry.begin("new");
        let state = State {
            phase: Phase::Recording,
            capture_id: Some("new".into()),
        };
        assert_eq!(
            registry.decide(&state, Action::Stop, "old"),
            Outcome::AlreadyEnded
        );
        assert_eq!(
            registry.inspect("old")["bundle_path"],
            "/captures/exact.apparent"
        );
        assert_eq!(
            registry.decide(&state, Action::Stop, "other"),
            Outcome::WrongCapture
        );
        assert_eq!(
            registry.decide(&state, Action::Stop, "new"),
            Outcome::Stopping
        );
        registry.set("new", "processing", None, None);
        assert_eq!(
            registry.decide(&state, Action::Stop, "new"),
            Outcome::AlreadyEnded
        );
    }

    #[test]
    fn finalization_error_or_cancel_never_publishes_a_saved_path() {
        let mut registry = Registry::default();
        registry.begin("failure");
        registry.finish("failure", Err("disk full".into()));
        let failure = registry.inspect("failure");
        assert_eq!(failure["status"], "failed");
        assert_eq!(failure["error"], "finalization_failed");
        assert_eq!(failure["message"], "disk full");
        assert!(failure["bundle_path"].is_null());
        registry.set("cancel", "cancelled", None, None);
        assert!(registry.inspect("cancel")["bundle_path"].is_null());
        assert_eq!(registry.inspect("unknown")["error"], "unknown_capture");
    }









    #[test]
    fn expired_wire_start_keeps_its_deadline_and_never_admits() {
        let id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let (action, deadline_ms) = parse_capture_request(&format!("start {id} 0")).unwrap();
        let (reply, _) = std::sync::mpsc::sync_channel(1);
        let request = Request {
            action,
            #[cfg(any(target_os = "macos", target_os = "linux"))]
            native_host: None,
            reply,
            deadline_ms,
        };
        assert!(request.expired());
        let mut registry = Registry::default();
        assert!(!registry.admit(id, false, request.expired()));
        assert!(!registry.admit(id, false, false));
        for invalid in [
            format!("start {id}"),
            format!("start {id} bad"),
            format!("stop {id} 0 extra"),
            "start bad 1".into(),
        ] {
            assert!(parse_request(&invalid).is_none());
        }
    }



    #[test]
    fn running_owner_does_not_require_an_answering_ui() {
        #[cfg(target_os = "macos")]
        let parent = PathBuf::from("/tmp");
        #[cfg(not(target_os = "macos"))]
        let parent = std::env::temp_dir();
        let dir = parent.join(format!("apparent-owner-{}", new_id().unwrap()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.join("capture.sock");
        let lock = path.with_extension("lock");
        assert!(!running_owner_at(path.clone()).unwrap());
        assert!(!lock.exists(), "a query must not create an owner lock");

        let stale = UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        drop(stale);
        let owner = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .open(&lock)
            .unwrap();
        assert_eq!(unsafe { libc::flock(owner.as_raw_fd(), libc::LOCK_EX) }, 0);
        assert!(
            running_owner_at(path.clone()).unwrap(),
            "a held owner lock must prevent another GUI even when connect is refused"
        );
        std::fs::remove_file(&path).unwrap();
        assert!(running_owner_at(path.clone()).unwrap());
        let listener = UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        listener.set_nonblocking(true).unwrap();
        assert!(running_owner_at(path.clone()).unwrap());
        assert_eq!(
            listener.accept().unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock,
            "an owner query must not connect, regardless of the listener backlog"
        );
        drop(listener);
        let inode = std::fs::symlink_metadata(&path).unwrap().ino();
        drop(owner);
        assert!(!running_owner_at(path.clone()).unwrap());
        assert_eq!(std::fs::symlink_metadata(&path).unwrap().ino(), inode);
        std::fs::remove_file(path).unwrap();
        std::fs::remove_file(lock).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn running_owner_rejects_unsafe_locks_without_changing_them() {
        #[cfg(target_os = "macos")]
        let parent = PathBuf::from("/tmp");
        #[cfg(not(target_os = "macos"))]
        let parent = std::env::temp_dir();
        let dir = parent.join(format!("apparent-owner-{}", new_id().unwrap()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.join("capture.sock");
        let lock = path.with_extension("lock");

        std::fs::write(&lock, b"do not change").unwrap();
        for mode in [0o644, 0o000] {
            std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(mode)).unwrap();
            assert!(running_owner_at(path.clone()).is_err());
            assert_eq!(
                std::fs::symlink_metadata(&lock).unwrap().mode() & 0o777,
                mode
            );
        }
        std::fs::set_permissions(&lock, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(std::fs::read(&lock).unwrap(), b"do not change");
        let target = dir.join("target");
        std::fs::rename(&lock, &target).unwrap();
        std::os::unix::fs::symlink(&target, &lock).unwrap();
        assert!(running_owner_at(path.clone()).is_err());
        assert!(
            std::fs::symlink_metadata(&lock)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        std::fs::remove_file(&lock).unwrap();
        std::fs::create_dir(&lock).unwrap();
        assert!(running_owner_at(path).is_err());
        std::fs::remove_dir(lock).unwrap();
        std::fs::remove_file(target).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn competing_launches_cannot_remove_the_winning_control_socket() {
        // Reuse the short private fixture suffix, including with CI's HOME-rooted TMPDIR.
        // Do not relocate the fixture or change the product's control socket path.
        let (path, stale) = transport_fixture();
        let dir = path.parent().unwrap().to_path_buf();
        drop(stale);

        // A launcher holding the lock may be between its stale-socket check and bind.
        // Another launcher must not unlink the socket even though connect is refused.
        let owner = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(0o600)
            .open(path.with_extension("lock"))
            .unwrap();
        assert_eq!(unsafe { libc::flock(owner.as_raw_fd(), libc::LOCK_EX) }, 0);
        let inode = std::fs::symlink_metadata(&path).unwrap().ino();
        assert!(listen_at(path.clone()).is_err());
        assert_eq!(std::fs::symlink_metadata(&path).unwrap().ino(), inode);
        drop(owner);

        let barrier = std::sync::Arc::new(std::sync::Barrier::new(3));
        let contenders: Vec<_> = (0..2)
            .map(|_| {
                let path = path.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    listen_at(path)
                })
            })
            .collect();
        barrier.wait();
        let mut results: Vec<_> = contenders
            .into_iter()
            .map(|thread| thread.join().unwrap())
            .collect();
        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        let receiver = results
            .remove(results.iter().position(Result::is_ok).unwrap())
            .unwrap();
        let mut stream = UnixStream::connect(&path).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        writeln!(stream, "state").unwrap();
        receiver
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .reply
            .send("{\"state\":\"reachable\"}".into())
            .unwrap();
        let mut response = String::new();
        BufReader::new(stream).read_line(&mut response).unwrap();
        assert!(response.contains("reachable"));
        drop(receiver);
        for _ in 0..50 {
            if !path.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(!path.exists());
        std::fs::remove_file(path.with_extension("lock")).unwrap();
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn only_the_matching_capture_can_be_stopped_or_cancelled() {
        for phase in [Phase::Starting, Phase::Recording] {
            let state = State {
                phase: phase.clone(),
                capture_id: Some("capture-a".into()),
            };
            assert_eq!(
                decide(&state, Action::Cancel, "capture-b"),
                Outcome::WrongCapture
            );
            assert_eq!(
                decide(&state, Action::Stop, "capture-b"),
                Outcome::WrongCapture
            );
            assert_eq!(
                decide(&state, Action::Cancel, "capture-a"),
                Outcome::Cancelled
            );
            assert_eq!(
                decide(&state, Action::Stop, "capture-a"),
                if phase == Phase::Recording {
                    Outcome::Stopping
                } else {
                    Outcome::NotReady
                }
            );
        }
        for phase in [
            Phase::Idle,
            Phase::Selecting,
            Phase::Processing,
            Phase::Completed,
        ] {
            let state = State {
                phase,
                capture_id: None,
            };
            assert_eq!(
                decide(&state, Action::Stop, "capture-a"),
                Outcome::AlreadyEnded
            );
            assert_eq!(
                decide(&state, Action::Cancel, "capture-a"),
                Outcome::AlreadyEnded
            );
        }
    }

    #[test]
    fn ids_are_opaque_fresh_and_command_ids_are_strict() {
        let first = new_id().unwrap();
        let second = new_id().unwrap();
        assert!(valid_id(&first));
        assert!(valid_id(&second));
        assert_ne!(first, second);
        for invalid in [
            "",
            "capture-a",
            "0\nstop 00000000000000000000000000000000",
            "g0000000000000000000000000000000",
        ] {
            assert!(!valid_id(invalid));
            assert!(command(Some((Action::Cancel, invalid))).is_err());
        }
    }

    #[test]
    fn response_phase_and_outcome_are_machine_readable() {
        let response = serde_json::json!({
            "state": State { phase: Phase::Starting, capture_id: Some("id".into()) },
            "outcome": Outcome::NotReady,
        });
        assert_eq!(response["state"]["phase"], "starting");
        assert_eq!(response["outcome"], "not_ready");
    }
}
