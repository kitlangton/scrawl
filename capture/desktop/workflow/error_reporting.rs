//! Desktop-only, structured PostHog diagnostics. Never forward log/error/panic text.
//! No SDK, persistent identity, disk spool, retries, or Engine -> Desktop dependency.
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{OnceLock, mpsc};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

const ENDPOINT: &str = "https://eu.i.posthog.com/i/v0/e/";
const QUEUE_SIZE: usize = 16;
static REPORTER: OnceLock<Reporter> = OnceLock::new();

/// Only reviewed, static failure categories can enter the reporting boundary.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Failure {
    ProtectRecording,
    AudioStart,
    ScreenStart,
    PhoneStart,
    FinalizeCapture,
    VideoExport,
    RustPanic,
}

impl Failure {
    fn code(self) -> &'static str {
        match self {
            Self::ProtectRecording => "recording_storage_failed",
            Self::AudioStart => "audio_start_failed",
            Self::ScreenStart => "screen_start_failed",
            Self::PhoneStart => "phone_start_failed",
            Self::FinalizeCapture => "capture_finalization_failed",
            Self::VideoExport => "video_export_failed",
            Self::RustPanic => "rust_panic",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Consent {
    generation: String,
}

/// A missing file/key defaults on; corrupt, unreadable, oversized, symlinked or
/// wrongly typed config fails closed. Do not use the forgiving general loader.
fn reporting_config_in(path: &Path) -> Option<Value> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path);
    let value: Value = match file {
        Ok(file) => {
            if !file.metadata().ok()?.is_file() {
                return None;
            }
            let mut bytes = Vec::new();
            file.take(65_537).read_to_end(&mut bytes).ok()?;
            if bytes.len() > 65_536 {
                return None;
            }
            serde_json::from_slice(&bytes).ok()?
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(_) => return None,
    };
    value.as_object()?;
    Some(value)
}

fn consent_in(path: &Path) -> Option<Consent> {
    let value = reporting_config_in(path)?;
    match value.get("errorReporting") {
        None | Some(Value::Bool(true)) => {}
        _ => return None,
    }
    let generation = match value.get("errorReportingGeneration") {
        None => "initial".to_owned(),
        Some(Value::String(value)) if value.len() <= 128 => value.clone(),
        _ => return None,
    };
    Some(Consent { generation })
}

pub(crate) fn enabled_in(path: &Path) -> bool {
    consent_in(path).is_some()
}

pub(crate) fn set_enabled_in(path: &Path, on: bool) -> anyhow::Result<()> {
    set_enabled_admitted_in(path, on, || true).map_err(anyhow::Error::msg)
}

fn set_enabled_admitted_in(
    path: &Path,
    on: bool,
    admit: impl FnOnce() -> bool,
) -> Result<(), &'static str> {
    let mut expired = false;
    let result = crate::shared::handoff::update_config_in(path, |config| {
        // This check runs against the fresh config after acquiring its flock.
        if !admit() {
            expired = true;
            anyhow::bail!("expired");
        }
        config["errorReporting"] = json!(on);
        // Rotate on EVERY explicit change, so disable/re-enable cannot revive a
        // queued report, even if another process makes both edits between reads.
        config["errorReportingGeneration"] = json!(crate::shared::ids::make_bundle_id());
        Ok(())
    });
    result.map_err(|_| {
        if expired {
            "admission_expired"
        } else {
            "config_write_failed"
        }
    })
}

fn read_enabled_in(path: &Path) -> Result<bool, &'static str> {
    let config = reporting_config_in(path).ok_or("config_read_failed")?;
    let generation = config.get("errorReportingGeneration");
    if generation.is_some_and(|v| !v.as_str().is_some_and(|s| s.len() <= 128)) {
        return Err("config_read_failed");
    }
    match config.get("errorReporting") {
        None | Some(Value::Bool(true)) => Ok(true),
        Some(Value::Bool(false)) => Ok(false),
        _ => Err("config_read_failed"),
    }
}

pub(crate) fn error_reporting_bridge_in(
    path: &Path,
    operation: &str,
    enabled: Option<bool>,
    admit: impl FnOnce() -> bool,
) -> Result<bool, &'static str> {
    match (operation, enabled) {
        ("read", None) => read_enabled_in(path),
        ("set", Some(value)) => {
            set_enabled_admitted_in(path, value, admit)?;
            read_enabled_in(path)
        }
        _ => Err("invalid_request"),
    }
}

#[derive(Clone)]
struct Report {
    failure: Failure,
    file: Option<String>,
    line: u32,
    consent: Consent,
    delivered: Option<mpsc::SyncSender<()>>,
    diagnostics: Option<Value>,
}

impl Report {
    fn new(failure: Failure, location: &std::panic::Location<'_>, consent: Consent) -> Self {
        Self {
            failure,
            file: source_file(location.file()),
            line: location.line(),
            consent,
            delivered: None,
            diagnostics: None,
        }
    }

    fn payload(&self, token: &str) -> std::io::Result<Value> {
        let mut random = [0u8; 16];
        std::fs::File::open("/dev/urandom")?.read_exact(&mut random)?;
        let event_id: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        let code = self.failure.code();
        let mut frame = json!({"platform":"native", "lang":"rust", "function":code, "in_app":true, "synthetic":true, "client_resolved":true});
        if let Some(file) = &self.file {
            frame["filename"] = json!(file);
            frame["lineno"] = json!(self.line);
        }
        let mut payload = json!({
            "api_key": token,
            "event": "$exception",
            // A fresh event identity, not a person, installation or session.
            "distinct_id": event_id,
            "properties": {
                "$process_person_profile": false,
                "$geoip_disable": true,
                "$ip": null,
                "$lib": "apparent-desktop",
                "$app_version": env!("CARGO_PKG_VERSION"),
                "build": env!("APPARENT_VERSION_LABEL"),
                "$os": std::env::consts::OS,
                "architecture": std::env::consts::ARCH,
                "operation": code,
                "$exception_level": "error",
                "$exception_fingerprint": format!("desktop:{code}:{}:{}", self.file.as_deref().unwrap_or("external"), if self.file.is_some() { self.line } else { 0 }),
                "$exception_list": [{
                    "type": if matches!(self.failure, Failure::RustPanic) { "RustPanic" } else { "DesktopOperationError" },
                    "value": code,
                    "mechanism": { "type":"generic", "handled": !matches!(self.failure, Failure::RustPanic), "synthetic": true },
                    "stacktrace": { "type":"raw", "frames":[frame] }
                }]
            }
        });
        if let Some(diagnostics) = &self.diagnostics {
            payload["properties"]["diagnostics"] = diagnostics.clone();
            let step = diagnostics["steps"]
                .as_array()
                .and_then(|s| s.last())
                .and_then(|s| s["step"].as_str())
                .unwrap_or("unknown");
            let kind = diagnostics["io_errors"]
                .as_array()
                .and_then(|s| s.last())
                .and_then(|s| s["kind"].as_str())
                .unwrap_or("unknown");
            payload["properties"]["$exception_list"][0]["value"] =
                json!(format!("{}: {step} ({kind})", self.failure.code()));
            payload["properties"]["$exception_fingerprint"] = json!(format!(
                "desktop:{}:{step}:{kind}:{}",
                self.failure.code(),
                std::env::consts::OS
            ));
            if let Some(steps) = diagnostics["steps"].as_array().filter(|s| !s.is_empty()) {
                payload["properties"]["$exception_list"][0]["stacktrace"]["frames"] = json!(
                    steps
                        .iter()
                        .map(|point| json!({
                            "platform": "native", "lang": "rust", "function": point["step"],
                            "filename": point["file"], "lineno": point["line"],
                            "in_app": true, "synthetic": true, "client_resolved": true,
                        }))
                        .collect::<Vec<_>>()
                );
            }
        }
        Ok(payload)
    }
}

/// Extract only typed breadcrumbs and OS codes. Never call Display/Debug on an error.
/// Bounded independently of the size or contents of a third-party error chain.
fn diagnostics(error: &anyhow::Error) -> Value {
    use crate::capture::engine::bundle::diagnostics::{Breadcrumb, CaptureFailure};
    let mut steps = Vec::new();
    let mut io = Vec::new();
    let mut capture = None;
    let mut json_error = None;
    let mut depth = 0;
    for cause in error.chain().take(32) {
        depth += 1;
        if let Some(point) = cause.downcast_ref::<Breadcrumb>() {
            // Apply the same source-path boundary as the reporting callsite.
            steps.push(
                json!({"step": point.step, "file": source_file(point.file), "line": point.line}),
            );
        }
        if let Some(failure) = cause.downcast_ref::<CaptureFailure>() {
            capture = Some(&failure.facts);
        }
        if let Some(error) = cause.downcast_ref::<serde_json::Error>() {
            json_error = Some(
                json!({"category": format!("{:?}", error.classify()), "line": error.line(), "column": error.column()}),
            );
        }
        if let Some(error) = cause.downcast_ref::<std::io::Error>() {
            io.push(
                json!({"kind": format!("{:?}", error.kind()), "os_code": error.raw_os_error()}),
            );
        }
    }
    json!({"schema_version": 1, "steps": steps, "io_errors": io, "capture": capture, "chain_depth": depth, "json_error": json_error})
}

/// Source location only, never runtime paths or dependency/build-root paths.
fn source_file(file: &str) -> Option<String> {
    let relative = file
        .strip_prefix(concat!(env!("CARGO_MANIFEST_DIR"), "/"))
        .unwrap_or(file);
    let allowed = ["capture/", "composition/", "shared/"]
        .iter()
        .any(|prefix| relative.starts_with(prefix))
        || relative == "main.rs";
    (allowed
        && relative.len() <= 256
        && relative.ends_with(".rs")
        && relative.split('/').all(|part| {
            !part.is_empty()
                && part != ".."
                && part
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
        }))
    .then(|| relative.to_owned())
}

struct Reporter {
    config: PathBuf,
    sender: mpsc::SyncSender<Report>,
}

impl Reporter {
    fn start(config: PathBuf, token: String, endpoint: String) -> std::io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel::<Report>(QUEUE_SIZE);
        let path = config.clone();
        std::thread::Builder::new()
            .name("error-reports".into())
            .spawn(move || {
                let mut limiter = Limiter::default();
                while let Ok(mut report) = receiver.recv() {
                    let delivered = report.delivered.take();
                    // The final admission happens immediately before transport, not
                    // when the failing operation queued its report. No config lock
                    // spans network I/O. An already admitted request may finish.
                    if consent_in(&path).as_ref() != Some(&report.consent) {
                        continue;
                    }
                    if !limiter.admit(Instant::now()) {
                        continue;
                    }
                    if let Ok(payload) = report.payload(&token) {
                        let _ = send(&endpoint, &payload);
                    }
                    if let Some(delivered) = delivered {
                        let _ = delivered.try_send(());
                    }
                }
            })?;
        Ok(Self { config, sender })
    }

    #[cfg(test)]
    fn capture(&self, failure: Failure, location: &std::panic::Location<'_>) {
        self.capture_error(failure, location, None);
    }

    fn capture_error(
        &self,
        failure: Failure,
        location: &std::panic::Location<'_>,
        error: Option<&anyhow::Error>,
    ) {
        if let Some(consent) = consent_in(&self.config) {
            // Never wait for network or for queue capacity on a capture/UI thread.
            let mut report = Report::new(failure, location, consent);
            report.diagnostics = error.map(diagnostics);
            let _ = self.sender.try_send(report);
        }
    }

    fn capture_panic(&self, location: &std::panic::Location<'_>) {
        if std::thread::current().name() == Some("error-reports") {
            return;
        }
        let Some(consent) = consent_in(&self.config) else {
            return;
        };
        let (sender, receiver) = mpsc::sync_channel(1);
        let mut report = Report::new(Failure::RustPanic, location, consent);
        report.delivered = Some(sender);
        if self.sender.try_send(report).is_ok() {
            // Only the dying thread waits; ordinary failures never wait for I/O.
            let _ = receiver.recv_timeout(Duration::from_secs(2));
        }
    }
}

#[derive(Default)]
struct Limiter {
    window: Option<Instant>,
    count: usize,
}
impl Limiter {
    fn admit(&mut self, now: Instant) -> bool {
        if self
            .window
            .is_none_or(|start| now.duration_since(start) >= Duration::from_secs(60))
        {
            self.window = Some(now);
            self.count = 0;
        }
        if self.count >= 20 {
            return false;
        }
        self.count += 1;
        true
    }
}

/// Called only by the desktop lifecycle, never ordinary CLI invocations/tests.
/// The publishable project token is supplied by packaging, not user credentials.
pub(crate) fn install() {
    if cfg!(test) || REPORTER.get().is_some() {
        return;
    }
    let Some(token) = option_env!("APPARENT_POSTHOG_TOKEN").filter(|token| {
        token.starts_with("phc_")
            && (20..=200).contains(&token.len())
            && token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b))
    }) else {
        return;
    };
    let Ok(reporter) = Reporter::start(
        crate::shared::handoff::default_config_path(),
        token.to_owned(),
        ENDPOINT.to_owned(),
    ) else {
        return;
    };
    install_reporter(reporter);
}

fn install_reporter(reporter: Reporter) {
    if REPORTER.set(reporter).is_err() {
        return;
    }
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Deliberately ignore payload, thread name and arbitrary panic text.
        // Best effort, bounded flush before process-wide panic exit.
        if let (Some(reporter), Some(location)) = (REPORTER.get(), info.location()) {
            reporter.capture_panic(location);
        }
        previous(info);
    }));
}

#[track_caller]
pub(crate) fn report_error(failure: Failure, error: &anyhow::Error) {
    if let Some(reporter) = REPORTER.get() {
        reporter.capture_error(failure, std::panic::Location::caller(), Some(error));
    }
}

/// The only HTTP boundary. Token/body stay off argv and disk; curl config,
/// redirects and proxy environment cannot change the fixed EU destination.
/// No retry. Errors are local-only and must never recursively report themselves.
fn send(endpoint: &str, body: &Value) -> std::io::Result<bool> {
    let mut child = Command::new("curl")
        .args([
            "--disable",
            "--silent",
            "--fail",
            "--noproxy",
            "*",
            "--max-time",
            "2",
            "--connect-timeout",
            "1",
            "--max-redirs",
            "0",
            "--request",
            "POST",
            "--header",
            "Content-Type: application/json",
            "--data-binary",
            "@-",
            "--output",
            "/dev/null",
            "--url",
            endpoint,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let result = child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(body)?);
    if result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    result?;
    Ok(status.success())
}

#[cfg(test)]
mod tests;
