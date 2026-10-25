//! The Windows side of `scrawl-helper`: the same calls as `src/helper.rs`, answered in process.
//! Movies go through Media Foundation (`win::media`), transcription through `transcribe.cpp`
//! with Hex's model or Scrawl's own copy of it (`win::speech`), and permissions through the privacy settings
//! (`win::privacy`). There is no cabled-iPhone mirror on Windows.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use serde::Deserialize;

use crate::model::{RgbaFrame, TranscriptData};

/// There is no helper executable on Windows; its work happens in process.
pub fn path() -> Result<PathBuf> {
    bail!("scrawl-helper is macOS-only")
}

pub fn ui_path() -> Result<PathBuf> {
    bail!("scrawl-ui-helper is macOS-only")
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct PhoneDevice {
    pub id: String,
    pub name: String,
    pub model: String,
    pub width: u32,
    pub height: u32,
}

/// What Windows lets Scrawl use: the microphone and camera privacy switches. Screen capture,
/// input monitoring and pasting need no permission.
pub fn permissions()
-> Result<std::collections::BTreeMap<crate::permissions::Permission, crate::permissions::Status>> {
    Ok(crate::win::privacy::permissions())
}

/// Cabled iPhones need AVFoundation's screen capture devices, which only macOS has.
pub fn devices() -> Result<Vec<PhoneDevice>> {
    Ok(Vec::new())
}

/// One line of macOS's `scrawl-helper speech-model [--install]`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub struct SpeechModelLine {
    pub status: String,
    #[serde(default)]
    pub engine: Option<String>,
    #[serde(default)]
    pub locale: Option<String>,
    #[serde(default)]
    pub locales: Option<Vec<String>>,
    #[serde(default)]
    pub progress: Option<f64>,
    #[serde(default)]
    pub error: Option<String>,
}

type InstallSpeechFn = fn(&mut dyn FnMut(SpeechModelLine)) -> Result<()>;
type TranscribeSpeechFn = fn(&Path) -> Result<TranscriptData>;

static SPEECH_HOOKS: std::sync::OnceLock<(InstallSpeechFn, TranscribeSpeechFn)> =
    std::sync::OnceLock::new();

pub fn register_speech_hooks(install: InstallSpeechFn, transcribe: TranscribeSpeechFn) {
    let _ = SPEECH_HOOKS.set((install, transcribe));
}

/// Windows transcribes with Parakeet (`win::speech`): Hex's installed model, else Scrawl's own
/// copy, downloaded and checked here.
pub fn install_speech_model(
    _stop: &dyn Fn() -> bool,
    mut line: impl FnMut(SpeechModelLine),
) -> Result<()> {
    let Some((install, _)) = SPEECH_HOOKS.get() else {
        bail!("Windows Parakeet speech model installer is not registered");
    };
    install(&mut line)
}

/// On-device transcription with measured word timings (Parakeet through `transcribe.cpp`).
pub fn transcribe(audio: &Path) -> Result<TranscriptData> {
    let Some((_, transcribe_fn)) = SPEECH_HOOKS.get() else {
        bail!("Windows Parakeet transcriber is not registered");
    };
    transcribe_fn(audio)
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Probe {
    pub duration_ms: u64,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub audio_tracks: u32,
    /// The video track's nominal frame rate (0 when unknown).
    #[serde(default)]
    pub fps: f64,
}

pub fn probe(movie: &Path) -> Result<Probe> {
    crate::win::media::probe(movie)
}

/// Re-encodes `movie` into `out` as H.264 with its long edge at most `max_edge`.
pub fn shrink(movie: &Path, out: &Path, max_edge: u32) -> Result<crate::retention::Shrunk> {
    crate::win::media::shrink(movie, out, max_edge)
}

/// Mixes a movie's audio to 16 kHz mono WAV, aligned to the movie's start.
pub fn export_audio(movie: &Path, wav: &Path) -> Result<()> {
    crate::win::media::export_audio(movie, wav)
}

pub enum FrameTimes<'a> {
    /// Nearest decodable frame every N ms, stamped with its actual time.
    Every(u64),
    /// Exactly these times.
    At(&'a [u64]),
}

/// Decodes movie frames into memory; `max_edge` 0 keeps native size.
pub fn frames(movie: &Path, times: FrameTimes, max_edge: u32) -> Result<Vec<RgbaFrame>> {
    crate::win::media::frames(movie, times, max_edge)
}

/// Renders `text` as a compact Unicode half-block QR code for the terminal.
pub fn qr(_text: &str) -> Result<String> {
    bail!("QR codes need CoreImage (macOS)")
}
