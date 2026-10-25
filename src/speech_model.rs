//! Apple's on-device speech model. A fresh Mac doesn't have it, and each app has to reserve it
//! for itself (`AssetInventory`, attributed to `scrawl-helper`), so Scrawl checks and installs
//! it in the background at launch (`scrawl-helper speech-model --install`) instead of during the
//! first capture. Onboarding and Settings show how that's going (`view`). On macOS 15 there's
//! nothing to download: the check is for the Speech Recognition permission (`NeedsPermission`,
//! asked for in onboarding) and Dictation's on-device model.
//!
//! A capture that finishes without a model says so in its summary (`TranscriptData::missing`,
//! from `missing`). When the reason can pass (still downloading, a failed download), the bundle
//! gets a `.transcript-pending` marker, and once the model is installed it's transcribed again
//! in place: `scrawl.json` and `summary.md` are rewritten with the transcript.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use serde::Serialize;

use crate::model::MissingTranscript;

/// A locale id such as `zh-TW` (`_` becomes `-`), or `None` for Automatic (`auto`, blank) and
/// anything that isn't shaped like one. The helper matches it against the supported locales.
pub fn locale_id(value: &str) -> Option<String> {
    let value = value.trim();
    let shaped = value.starts_with(|c: char| c.is_ascii_alphabetic())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    (shaped && !value.eq_ignore_ascii_case("auto")).then(|| value.replace('_', "-"))
}

/// `--locale ID` for the helper's `transcribe`, `transcribe-stream` and `speech-model` when
/// Settings › Recording › Language (`appleLocale`) isn't Automatic.
pub fn locale_args() -> Vec<std::ffi::OsString> {
    crate::handoff::apple_locale()
        .map(|id| vec!["--locale".into(), id.into()])
        .unwrap_or_default()
}

/// Marks a bundle that should be transcribed again once the model is installed.
pub const PENDING: &str = ".transcript-pending";

/// Waits between failed installs; after the last one it stops until the next launch.
const RETRY_AFTER: [Duration; 3] = [
    Duration::from_secs(60),
    Duration::from_secs(5 * 60),
    Duration::from_secs(30 * 60),
];

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum State {
    /// Not checked yet (or the configured transcriber is Hex).
    #[default]
    Unknown,
    Checking,
    Downloading,
    Ready,
    /// No Apple speech model for this Mac and language.
    Unavailable,
    /// macOS 15: Speech Recognition isn't allowed yet (onboarding asks for it). Checked again
    /// once it is (`note_grants`).
    NeedsPermission,
    Failed,
}

/// What onboarding and Settings show.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub state: State,
    /// `SpeechTranscriber`, `DictationTranscriber` on Macs without SpeechTranscriber, or
    /// `SFSpeechRecognizer` on macOS 15.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    /// `en_US`; the helper window shows its name.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub locales: Vec<String>,
    /// 0…1 while downloading.
    pub progress: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Default)]
struct Worker {
    status: Status,
    running: bool,
    enabled: bool,
    generation: u64,
}

impl Worker {
    fn request(&mut self, restart: bool, enabled: bool) -> Option<u64> {
        self.enabled = enabled;
        if restart {
            self.generation += 1;
            self.status = Status {
                locales: std::mem::take(&mut self.status.locales),
                ..Default::default()
            };
        }
        if !self.enabled
            || self.running
            || matches!(self.status.state, State::Ready | State::Unavailable)
        {
            return None;
        }
        self.running = true;
        Some(self.generation)
    }

    fn apply(&mut self, generation: u64, line: &crate::helper::SpeechModelLine) -> bool {
        if self.stale(generation) {
            return false;
        }
        let s = &mut self.status;
        s.engine = line.engine.clone().or(s.engine.take());
        s.locale = line.locale.clone().or(s.locale.take());
        if let Some(locales) = &line.locales {
            s.locales = locales.clone();
        }
        s.state = match line.status.as_str() {
            "installed" => State::Ready,
            "unsupported" => State::Unavailable,
            "permission" => State::NeedsPermission,
            "failed" => State::Failed,
            _ => State::Downloading,
        };
        s.progress = line.progress.unwrap_or(s.progress);
        s.error = line.error.clone();
        true
    }

    fn finish(&mut self, generation: u64) -> Option<u64> {
        if self.enabled && self.stale(generation) {
            return Some(self.generation);
        }
        self.running = false;
        None
    }

    fn stale(&self, generation: u64) -> bool {
        generation != self.generation
    }
}

static WORKER: Mutex<Worker> = Mutex::new(Worker {
    status: Status {
        state: State::Unknown,
        engine: None,
        locale: None,
        locales: Vec::new(),
        progress: 0.0,
        error: None,
    },
    running: false,
    enabled: false,
    generation: 0,
});

fn lock() -> MutexGuard<'static, Worker> {
    WORKER
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub fn current() -> Status {
    lock().status.clone()
}

/// Checks the model in the background and installs it if it's missing, unless the transcriber
/// is Hex (whose users don't need it) or a check is already running. Cheap to call again.
pub fn ensure() {
    start(false);
}

/// Checks the selected language again, superseding any earlier check or download.
pub fn restart() {
    start(true);
}

fn start(restart: bool) {
    let enabled = crate::transcribe::Engine::configured() != crate::transcribe::Engine::Hex;
    if !restart && !enabled {
        return;
    }
    let mut worker = lock();
    let Some(mut generation) = worker.request(restart, enabled) else {
        return;
    };
    let spawned = std::thread::Builder::new()
        .name("scrawl-speech-model".into())
        .spawn(move || {
            loop {
                work(generation);
                match lock().finish(generation) {
                    Some(next) => generation = next,
                    None => break,
                }
            }
        });
    if spawned.is_err() {
        worker.running = false;
    }
}

fn work(generation: u64) {
    for attempt in 0..=RETRY_AFTER.len() {
        {
            let mut worker = lock();
            if worker.stale(generation) {
                return;
            }
            if worker.status.state != State::Downloading {
                worker.status.state = State::Checking;
            }
        }
        let result = crate::helper::install_speech_model(&|| lock().stale(generation), |line| {
            lock().apply(generation, &line);
        });
        let status = {
            let mut worker = lock();
            if worker.stale(generation) {
                return;
            }
            if let Err(error) = result {
                worker.status.state = State::Failed;
                worker.status.error = Some(format!("{error:#}"));
            }
            worker.status.clone()
        };
        crate::log::scrawl_log(format!(
            "speech model: {:?} {} {}{}",
            status.state,
            status.engine.as_deref().unwrap_or("?"),
            status.locale.as_deref().unwrap_or("?"),
            status
                .error
                .as_ref()
                .map(|e| format!(" ({e})"))
                .unwrap_or_default()
        ));
        match status.state {
            State::Ready => {
                if !lock().stale(generation) {
                    transcribe_pending(&crate::handoff::default_captures_dir());
                }
                return;
            }
            State::Unavailable | State::NeedsPermission => return,
            _ => {}
        }
        if let Some(wait) = RETRY_AFTER.get(attempt) {
            let deadline = Instant::now() + *wait;
            while let Some(left) = deadline.checked_duration_since(Instant::now()) {
                if lock().stale(generation) {
                    return;
                }
                std::thread::sleep(left.min(Duration::from_secs(1)));
            }
        }
    }
}

/// Speech Recognition was just allowed (macOS 15): check the model again now.
pub fn note_grants(grants: &crate::permissions::Grants) {
    if current().state == State::NeedsPermission
        && grants.granted(crate::permissions::Permission::SpeechRecognition)
    {
        ensure();
    }
}

/// The plain sentence for a capture without a transcript, from the helper's `missing`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn missing(why: &str, locale: &str, error: Option<&str>) -> MissingTranscript {
    let model = if cfg!(target_os = "windows") {
        format!("The Parakeet speech model ({locale})")
    } else {
        format!("Apple's speech model ({locale})")
    };
    match why {
        "downloading" => MissingTranscript {
            reason: format!(
                "{model} was still downloading. The audio is saved, and Scrawl adds the \
                 transcript to this bundle when the download finishes."
            ),
            retry: true,
        },
        "permission" => MissingTranscript {
            reason: "Speech Recognition is off for Scrawl. The audio is saved, and Scrawl adds \
                     the transcript to this bundle once it's on (Scrawl Settings › Permissions)."
                .to_owned(),
            retry: true,
        },
        "dictation" => MissingTranscript {
            reason: "Dictation's on-device model isn't on this Mac. Turn on Dictation in System \
                     Settings › Keyboard, and Scrawl adds the transcript to this bundle once it's \
                     there."
                .to_owned(),
            retry: true,
        },
        "unsupported" => MissingTranscript {
            reason: format!(
                "this Mac has no Apple speech model for {locale}. Hex can transcribe instead \
                 (Scrawl Settings › Recording)."
            ),
            retry: false,
        },
        _ => MissingTranscript {
            reason: format!(
                "{model} couldn't be installed{}. The audio is saved, and Scrawl adds the \
                 transcript to this bundle if a later try works.",
                error.map(|e| format!(" ({e})")).unwrap_or_default()
            ),
            retry: true,
        },
    }
}

/// Marks a freshly written bundle whose transcript can come later; if the model has arrived
/// meanwhile, transcribes it now.
pub fn mark_pending(bundle_dir: &Path) {
    let _ = std::fs::write(bundle_dir.join(PENDING), b"");
    if current().state == State::Ready {
        let dir = bundle_dir.to_path_buf();
        std::thread::spawn(move || transcribe_one(&dir));
    }
}

/// Every marked bundle in `captures`.
pub fn transcribe_pending(captures: &Path) {
    let Ok(entries) = std::fs::read_dir(captures) else {
        return;
    };
    let pending: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|dir| dir.join(PENDING).is_file())
        .collect();
    for dir in pending {
        transcribe_one(&dir);
    }
}

fn transcribe_one(dir: &Path) {
    match transcribe_again(dir) {
        Ok(done) => crate::log::scrawl_log(format!(
            "transcribed again {}: {}",
            dir.display(),
            if done { "done" } else { "still no model" }
        )),
        Err(error) => {
            crate::log::scrawl_log(format!(
                "transcribing {} again failed: {error:#}",
                dir.display()
            ));
            let _ = std::fs::remove_file(dir.join(PENDING));
        }
    }
}

/// Transcribes a bundle's `audio.wav` with Apple Speech and rewrites its transcript, each
/// keyframe's words, `scrawl.json` and `summary.md`. Returns `false` (and keeps the marker)
/// while there's still no model.
pub fn transcribe_again(dir: &Path) -> Result<bool> {
    let (dir, mut bundle) = crate::handoff::load_bundle(dir)?;
    let wav = dir.join(bundle.audio.as_deref().unwrap_or("audio.wav"));
    anyhow::ensure!(wav.is_file(), "the bundle has no audio.wav any more");
    let transcript = crate::helper::transcribe(&wav)?;
    if transcript
        .missing
        .as_ref()
        .is_some_and(|missing| missing.retry)
    {
        return Ok(false);
    }
    let times: Vec<u64> = bundle.keyframes.iter().map(|k| k.t).collect();
    let said = crate::keyframes::said_per_keyframe(&times, &[], &transcript);
    for (frame, said) in bundle.keyframes.iter_mut().zip(said) {
        frame.said = said;
    }
    bundle.transcript = transcript;
    std::fs::write(
        dir.join("scrawl.json"),
        serde_json::to_vec_pretty(&bundle).context("Could not encode scrawl.json")?,
    )?;
    std::fs::write(
        dir.join("summary.md"),
        crate::sketch::summary_markdown(&bundle, &dir),
    )?;
    let _ = std::fs::remove_file(dir.join(PENDING));
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(status: &str) -> crate::helper::SpeechModelLine {
        crate::helper::SpeechModelLine {
            status: status.into(),
            ..Default::default()
        }
    }

    #[test]
    fn a_restart_can_cancel_without_requesting_more_work() {
        let mut w = Worker::default();
        assert_eq!(w.request(true, false), None);
        assert!(!w.running);
        let g0 = w.request(false, true).unwrap();
        assert!(w.apply(g0, &line("downloading")));
        assert_eq!(w.request(true, false), None);
        assert!(w.stale(g0));
        assert_eq!(w.status.state, State::Unknown);
        assert!(!w.apply(g0, &line("installed")));
        assert_eq!(w.finish(g0), None);
        assert!(!w.running);
        let g1 = w.request(false, true).unwrap();
        assert_ne!(g0, g1);
        assert!(w.apply(g1, &line("installed")));
    }

    #[test]
    fn a_restart_while_running_is_never_lost() {
        let mut w = Worker::default();
        let g0 = w.request(false, true).unwrap();
        assert_eq!(w.request(true, true), None);
        assert!(!w.apply(g0, &line("installed")));
        assert_eq!(w.status.state, State::Unknown);
        let g1 = w.finish(g0).expect("runs again for the new language");
        assert!(w.apply(g1, &line("installed")));
        assert_eq!(w.status.state, State::Ready);
        assert_eq!(w.finish(g1), None);
        assert!(!w.running);
        assert_eq!(w.request(false, true), None);
        assert!(w.request(true, true).is_some());
    }

    #[test]
    fn restarting_between_the_last_attempt_and_going_idle_still_runs() {
        let mut w = Worker::default();
        let g0 = w.request(false, true).unwrap();
        assert!(w.apply(g0, &line("installed")));
        assert_eq!(w.request(true, true), None);
        assert!(w.finish(g0).is_some());
    }

    #[test]
    fn a_b_c_changes_only_publish_the_latest() {
        let mut w = Worker::default();
        let g0 = w.request(false, true).unwrap();
        assert_eq!(w.request(true, true), None);
        let g1 = w.generation;
        assert_eq!(w.request(true, true), None);
        let g2 = w.generation;
        assert!(w.stale(g0));
        assert!(w.stale(g1));
        assert!(!w.stale(g2));
        assert!(!w.apply(g0, &line("installed")));
        assert!(!w.apply(g1, &line("failed")));
        assert_eq!(w.status.state, State::Unknown);
        assert_eq!(w.finish(g0), Some(g2));
        assert!(w.apply(g2, &line("installed")));
        assert_eq!(w.finish(g2), None);
        assert!(!w.running);
        assert_eq!(w.request(false, true), None);
    }

    #[test]
    fn locales_survive_a_restart() {
        let mut w = Worker::default();
        let g0 = w.request(false, true).unwrap();
        let mut failed = line("failed");
        failed.engine = Some("SpeechTranscriber".into());
        failed.locale = Some("en_US".into());
        failed.locales = Some(vec!["en-US".into(), "zh-TW".into()]);
        failed.progress = Some(0.5);
        failed.error = Some("download failed".into());
        assert!(w.apply(g0, &failed));
        let locales = w.status.locales.clone();
        assert_eq!(w.request(true, true), None);
        assert_eq!(
            w.status,
            Status {
                locales: locales.clone(),
                ..Default::default()
            }
        );
        let g1 = w.finish(g0).unwrap();
        assert!(w.apply(g1, &line("installed")));
        assert_eq!(w.status.locales, locales);
    }

    #[test]
    fn locale_ids_are_plain_ids_and_auto_is_none() {
        assert_eq!(locale_id(" zh-TW ").as_deref(), Some("zh-TW"));
        assert_eq!(locale_id("zh_Hant_TW").as_deref(), Some("zh-Hant-TW"));
        assert_eq!(locale_id("yue-CN").as_deref(), Some("yue-CN"));
        for automatic_or_bad in ["", "auto", "AUTO", "zh-TW --install", "../x", "-zh", "中文"] {
            assert_eq!(locale_id(automatic_or_bad), None, "{automatic_or_bad:?}");
        }
    }

    #[test]
    fn a_download_in_progress_will_be_retried_but_a_missing_language_will_not() {
        let downloading = missing("downloading", "en_US", None);
        assert!(downloading.retry);
        assert!(downloading.reason.contains("still downloading"));
        let unsupported = missing("unsupported", "xx_XX", None);
        assert!(!unsupported.retry);
        assert!(
            unsupported
                .reason
                .contains("no Apple speech model for xx_XX")
        );
        let failed = missing("failed", "en_US", Some("asset unavailable"));
        assert!(failed.retry && failed.reason.contains("(asset unavailable)"));
        let dictation = missing("dictation", "en_US", None);
        assert!(dictation.retry && dictation.reason.contains("Turn on Dictation"));
        // macOS 15 without Speech Recognition: transcribed again once it's allowed.
        let off = missing("permission", "en_US", Some("Speech Recognition is off"));
        assert!(
            off.retry
                && off
                    .reason
                    .starts_with("Speech Recognition is off for Scrawl.")
        );
    }
}
