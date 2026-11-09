//! Bounded transport to the packaged headless Record policy. Never runs on the GUI thread.
use std::{io::{BufReader, Read, Write}, os::unix::net::UnixStream,
    path::PathBuf, process::{Command, Stdio}, time::{Duration, Instant}};
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(in crate::capture::desktop) struct Capture {
    pub generation: String,
    pub capture_id: String,
    pub request_id: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(super) enum Outcome {
    Saved { #[serde(rename = "bundlePath")] bundle_path: PathBuf },
    Failed { message: String },
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(in crate::capture::desktop) struct Facts {
    pub capture: Capture,
    pub platform: String,
    pub source: String,
    pub agent_requested: bool,
    pub automatic_trial_enabled: bool,
    pub lab: bool,
    pub hyprland: bool,
    pub wayland: bool,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Observation {
    pub capture: Capture,
    pub outcome: Outcome,
    pub clipboard: Clipboard,
    pub current: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum Clipboard { NotRequested, Copied, Failed }
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct State {
    context: Facts,
    pub observation: Option<Observation>,
    pub current: bool,
    connected: bool,
    copy_pending: bool,
    pub status: Option<String>,
    pub notification: Option<Notification>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Notification { pub title: String, pub body: String }
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", deny_unknown_fields)]
pub(super) enum Effect { Copy { capture: Capture, mode: CopyMode } }
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) enum CopyMode { Text, Bundle }
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Decision { pub state: State, pub effects: Vec<Effect> }

/// The native effect receipt outlives any failure of its policy acknowledgement.
pub(super) struct Completion {
    pub state: Result<State>,
    pub clipboard_receipt: Option<bool>,
}
impl Completion {
    pub fn unavailable(error: anyhow::Error) -> Self {
        Self { state: Err(error), clipboard_receipt: None }
    }
}

fn assets() -> Option<(PathBuf, PathBuf)> {
    let executable = std::env::current_exe().ok()?;
    let parent = executable.parent()?;
    #[cfg(target_os = "macos")]
    let root = parent.parent()?.join("Resources/settings");
    #[cfg(target_os = "linux")]
    let root = parent.parent()?.join("share/apparent/settings");
    let mut candidates = vec![(root.join("app"), root.join("runtime"))];
    if cfg!(debug_assertions) {
        let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("capture/desktop/ui/frontend");
        candidates.push((source.clone(), source.join("node_modules/electron/dist")));
    }
    for (app, runtime) in candidates {
        #[cfg(target_os = "macos")]
        let runtime = runtime.join("Electron.app/Contents/MacOS/Electron");
        #[cfg(target_os = "linux")]
        let runtime = runtime.join("electron");
        let worker = app.join("electron/record-completion/worker.js");
        if runtime.is_file() && worker.is_file() { return Some((runtime, worker)); }
    }
    None
}

pub(super) struct Worker {
    child: crate::capture::ui_process::UiChild,
    pipe: BufReader<UnixStream>,
    facts: Facts,
    outcome: Outcome,
    receipt: Option<String>,
}
impl Worker {
    pub fn open(facts: Facts, outcome: Outcome) -> Result<Self> {
        let (runtime, script) = assets().context("Packaged Record completion unavailable")?;
        Self::launch(Command::new(runtime).env("ELECTRON_RUN_AS_NODE", "1").arg(script), facts, outcome)
    }
    fn launch(command: &mut Command, facts: Facts, outcome: Outcome) -> Result<Self> {
        let (parent, child) = UnixStream::pair()?;
        parent.set_read_timeout(Some(Duration::from_secs(5)))?;
        parent.set_write_timeout(Some(Duration::from_secs(5)))?;
        let input: std::os::fd::OwnedFd = child.try_clone()?.into();
        let output: std::os::fd::OwnedFd = child.into();
        let child = command.stdin(Stdio::from(input)).stdout(Stdio::from(output))
            .stderr(Stdio::null()).spawn()?;
        let mut worker = Self { child: crate::capture::ui_process::UiChild::new(child),
            pipe: BufReader::new(parent), facts, outcome, receipt: None };
        worker.send(&serde_json::json!({"version":1, "context":worker.facts,
            "outcome":worker.outcome,"current":true}))?;
        Ok(worker)
    }
    fn send(&mut self, value: &serde_json::Value) -> Result<()> {
        let mut bytes = serde_json::to_vec(value)?;
        ensure!(bytes.len() <= 32768, "Record completion input too large");
        bytes.push(b'\n');
        self.pipe.get_mut().write_all(&bytes)?;
        Ok(())
    }
    pub fn read(&mut self) -> Result<Decision> {
        let mut bytes = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline.checked_duration_since(Instant::now()).context("Record response deadline")?;
            self.pipe.get_mut().set_read_timeout(Some(remaining))?;
            let mut byte = [0];
            self.pipe.read_exact(&mut byte)?;
            bytes.push(byte[0]);
            ensure!(bytes.len() <= 32768, "Record completion response too large");
            if byte[0] == b'\n' { break; }
        }
        let decision: Decision = serde_json::from_slice(&bytes)?;
        // Capability boundary: a policy process cannot substitute a path/identity,
        // fabricate a finalizer outcome or ask for an unrelated capability.
        let state = &decision.state;
        let observation = state.observation.as_ref().context("Missing finalizer observation")?;
        ensure!(state.context == self.facts && observation.capture == self.facts.capture
            && observation.outcome == self.outcome && observation.current == state.current
            && state.connected && state.status.as_ref().is_some_and(|s| s.len() <= 256)
            && state.notification.as_ref().is_none_or(|n| n.title.len() <= 256 && n.body.len() <= 16384)
            && decision.effects.len() <= 1, "Record completion identity mismatch");
        let expected = match self.receipt.as_deref() {
            Some("copied") => Clipboard::Copied,
            Some("failed") => Clipboard::Failed,
            _ => Clipboard::NotRequested,
        };
        ensure!(observation.clipboard == expected, "Record fabricated clipboard receipt");
        if let Some(receipt) = &self.receipt {
            ensure!(decision.effects.is_empty() && !state.copy_pending
                && state.current == (receipt != "revoked"), "Invalid Record terminal state");
        }
        for Effect::Copy { capture, .. } in &decision.effects {
            ensure!(*capture == self.facts.capture && matches!(self.outcome, Outcome::Saved { .. })
                && state.current && state.copy_pending && observation.clipboard == Clipboard::NotRequested,
                "Invalid Record copy capability");
        }
        Ok(decision)
    }
    pub fn acknowledge(&mut self, result: &str) -> Result<()> {
        ensure!(self.receipt.is_none() && matches!(result, "copied" | "failed" | "revoked"), "Duplicate capability acknowledgement");
        self.receipt = Some(result.to_owned());
        self.send(&serde_json::json!({"result":result}))
    }
    pub fn complete(mut self, initial: Decision, receipt: Option<&str>) -> Completion {
        let clipboard_receipt = match receipt {
            Some("copied") => Some(true),
            Some("failed") => Some(false),
            _ => None,
        };
        let state = (|| {
            let decision = if let Some(receipt) = receipt {
                self.acknowledge(receipt)?;
                self.read()?
            } else { initial };
            self.finish()?;
            Ok(decision.state)
        })();
        Completion { state, clipboard_receipt }
    }
    pub fn finish(&mut self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(1);
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait()? {
                ensure!(status.success(), "Record completion worker failed");
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        anyhow::bail!("Record completion worker did not exit")
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts() -> Facts {
        Facts {
            capture: Capture { generation: "9007199254740993".into(), capture_id: "synthetic-a".into(), request_id: Some("request-a".into()) },
            platform: if cfg!(target_os = "linux") { "linux" } else { "macos" }.into(),
            source: "screen".into(), agent_requested: false, automatic_trial_enabled: false, lab: false,
            hyprland: false, wayland: false,
        }
    }

    #[test]
    #[ignore = "provider integration: requires built frontend and its locked Electron runtime"]
    fn record_completion_electron_rust_roundtrip() {
        let facts = facts();
        for receipt in ["copied", "failed", "revoked"] {
            let outcome = Outcome::Saved { bundle_path: "/synthetic/exact-finalized.apparent".into() };
            let mut worker = Worker::open(facts.clone(), outcome.clone()).unwrap();
            let first = worker.read().unwrap();
            assert!(matches!(first.effects.as_slice(), [Effect::Copy { mode: CopyMode::Bundle, .. }]));
            assert_eq!(first.state.observation.unwrap().outcome, outcome);
            worker.acknowledge(receipt).unwrap();
            let final_state = worker.read().unwrap();
            assert!(final_state.effects.is_empty());
            assert_eq!(final_state.state.current, receipt != "revoked");
            worker.finish().unwrap();
        }
        let mut failed = Worker::open(facts, Outcome::Failed { message: "publication failed".into() }).unwrap();
        let state = failed.read().unwrap();
        assert!(state.effects.is_empty());
        assert_eq!(state.state.status.as_deref(), Some("Capture failed"));
        failed.finish().unwrap();
    }

    #[test]
    #[ignore = "native clipboard integration: run only in an isolated desktop with built frontend"]
    fn record_completion_native_clipboard_survives_worker_exit() {
        struct Fixture(PathBuf);
        impl Drop for Fixture {
            fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
        }
        let fixture = Fixture(std::env::temp_dir().join(format!("{}.apparent", crate::shared::ids::make_bundle_id())));
        std::fs::create_dir(&fixture.0).unwrap();
        let bundle = serde_json::json!({
            "version": 1, "id": "synthetic-worker-failure", "createdAtMs": 0, "durationMs": 1000,
            "target": {"mode": "display", "displayId": 0,
                "frame": {"x": 0.0, "y": 0.0, "width": 640.0, "height": 480.0, "scale": 1.0}},
            "transcript": {"source": "test", "text": "Synthetic completion receipt", "segments": []},
            "strokes": [], "marks": [], "events": [], "keyframes": [], "contactSheet": "c.png"
        });
        std::fs::write(fixture.0.join("apparent.json"), serde_json::to_vec(&bundle).unwrap()).unwrap();
        let outcome = Outcome::Saved { bundle_path: fixture.0.clone() };
        let mut worker = Worker::open(facts(), outcome.clone()).unwrap();
        let initial = worker.read().unwrap();
        assert!(matches!(initial.effects.as_slice(), [Effect::Copy { mode: CopyMode::Bundle, .. }]));
        crate::shared::handoff::copy_bundle_to_clipboard(&fixture.0).unwrap();
        #[cfg(target_os = "linux")]
        let actual = arboard::Clipboard::new().unwrap().get_text().unwrap();
        #[cfg(target_os = "macos")]
        let actual = String::from_utf8(Command::new("/usr/bin/pbpaste").output().unwrap().stdout).unwrap();
        assert!(actual.contains("Synthetic completion receipt"));
        assert!(actual.contains(fixture.0.to_str().unwrap()));
        // Crash after the real write, before its acknowledgement: this is the
        // same exchange and receipt object used by present_capture_result.
        worker.child.kill().unwrap();
        worker.child.wait().unwrap();
        let completion = worker.complete(initial, Some("copied"));
        assert!(completion.state.is_err());
        assert_eq!(completion.clipboard_receipt, Some(true));
        assert_eq!(crate::shared::handoff::load_bundle(&fixture.0).unwrap().0, fixture.0);
    }
}
