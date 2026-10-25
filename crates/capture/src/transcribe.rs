//! Transcription: Apple's SpeechAnalyzer through `scrawl-helper` (measured word and segment
//! timings), Hex when `"transcriber": "hex"` is set and Hex is connected (`crate::hex`; word
//! times estimated), or a program of the user's when `"transcriber": "command"`
//! (`crate::command_transcriber`). Any Hex or command trouble falls back to Apple for that
//! recording.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};

use anyhow::Result;

use crate::audio::RecordedAudio;
use crate::model::{TranscriptData, TranscriptSegment, WordTiming};

/// Keep the previous English default; Hex, not Scrawl, validates model/language support.
pub const DEFAULT_LANGUAGE: &str = "en";

/// Accept language-code syntax without maintaining another list of Hex's languages.
/// Blank resets to English; disallow query delimiters before constructing the API URL.
pub fn language_code(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return Some(DEFAULT_LANGUAGE.into());
    }
    (value.len() <= 35
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        && value.as_bytes()[0].is_ascii_alphabetic())
    .then(|| value.to_ascii_lowercase())
}

const MIN_ISLAND_MS: u64 = 120;
const MERGE_GAP_MS: u64 = 700;
const MAX_MERGED_MS: u64 = 30_000;
const MAX_FORCED_MERGE_MS: u64 = 8 * 60_000;
pub const MAX_CLIPS: usize = 24;

/// Hex's support folder, as Hex's `app_paths::support_dir` resolves it: `~/Library/Application
/// Support/hex` on macOS (`voice-control` before Hex moved it), `%APPDATA%\voice-control` on Windows.
pub fn hex_support_dir() -> PathBuf {
    std::env::var_os("HEX_APPLICATION_SUPPORT_DIR")
        .map(PathBuf::from)
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| {
            let data = dirs::data_dir().unwrap_or_else(|| PathBuf::from("."));
            let legacy = data.join("voice-control");
            if cfg!(target_os = "macos")
                && (std::fs::read_link(&legacy).is_ok_and(|target| target.ends_with("hex"))
                    || !legacy.exists())
            {
                data.join("hex")
            } else {
                legacy
            }
        })
}

/// Speech spans worth one request each: tiny blips dropped, near neighbours joined so a clip
/// holds whole phrases, and never more than `MAX_CLIPS`.
pub fn plan_clips(islands: &[(u64, u64)], duration_ms: u64) -> Vec<(u64, u64)> {
    let mut clips: Vec<(u64, u64)> = Vec::new();
    for &(start, end) in islands {
        let (start, end) = (start.min(duration_ms), end.min(duration_ms));
        if end.saturating_sub(start) < MIN_ISLAND_MS {
            continue;
        }
        match clips.last_mut() {
            Some(last)
                if start.saturating_sub(last.1) < MERGE_GAP_MS && end - last.0 <= MAX_MERGED_MS =>
            {
                last.1 = end;
            }
            _ => clips.push((start, end)),
        }
    }
    while clips.len() > MAX_CLIPS {
        let Some(index) = (1..clips.len())
            .filter(|&i| clips[i].1 - clips[i - 1].0 <= MAX_FORCED_MERGE_MS)
            .min_by_key(|&i| clips[i].0 - clips[i - 1].1)
        else {
            break;
        };
        clips[index - 1].1 = clips[index].1;
        clips.remove(index);
    }
    clips
}

fn ends_sentence(word: &str) -> bool {
    word.trim_end_matches(['"', '\'', '”', '’', ')', ']'])
        .ends_with(['.', '!', '?', ';', ':'])
}

/// Spreads `text` over `start..end`, each word's share weighted by its length. Words tile the
/// span exactly (no gaps, no overlap), so the first begins at `start` and the last ends at `end`.
pub fn spread_words(text: &str, start: u64, end: u64) -> (Vec<TranscriptSegment>, Vec<WordTiming>) {
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let end = end.max(start);
    let total: u128 = tokens
        .iter()
        .map(|token| token.chars().count().max(1) as u128)
        .sum();
    let mut words: Vec<WordTiming> = Vec::with_capacity(tokens.len());
    let mut segments = Vec::new();
    let mut segment_start = 0;
    let mut weight: u128 = 0;
    for (index, token) in tokens.iter().enumerate() {
        let at = |weight: u128| start + (u128::from(end - start) * weight / total) as u64;
        let t0 = at(weight);
        weight += token.chars().count().max(1) as u128;
        words.push(WordTiming {
            t0,
            t1: at(weight),
            text: (*token).to_owned(),
        });
        if ends_sentence(token) || index + 1 == tokens.len() {
            let phrase = &words[segment_start..];
            segments.push(TranscriptSegment {
                t0: phrase[0].t0,
                t1: phrase[phrase.len() - 1].t1,
                text: phrase
                    .iter()
                    .map(|word| word.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            });
            segment_start = words.len();
        }
    }
    (segments, words)
}

/// Builds sentence segments from word timings when a transcriber returns only `words`.
pub fn segments_from_words(words: &[WordTiming]) -> Vec<TranscriptSegment> {
    let mut segments = Vec::new();
    let mut segment_start = 0;
    for (index, word) in words.iter().enumerate() {
        if ends_sentence(&word.text) || index + 1 == words.len() {
            let phrase = &words[segment_start..=index];
            segments.push(TranscriptSegment {
                t0: phrase[0].t0,
                t1: phrase[phrase.len() - 1].t1,
                text: phrase
                    .iter()
                    .map(|w| w.text.as_str())
                    .collect::<Vec<_>>()
                    .join(" "),
            });
            segment_start = index + 1;
        }
    }
    segments
}

/// Shifts word and segment timestamps by `start_offset_ms` onto the capture clock without
/// altering `segments`.
pub fn shift_transcript(mut transcript: TranscriptData, start_offset_ms: u64) -> TranscriptData {
    if start_offset_ms > 0 {
        for word in &mut transcript.words {
            word.t0 = word.t0.saturating_add(start_offset_ms);
            word.t1 = word.t1.saturating_add(start_offset_ms);
        }
        for segment in &mut transcript.segments {
            segment.t0 = segment.t0.saturating_add(start_offset_ms);
            segment.t1 = segment.t1.saturating_add(start_offset_ms);
        }
    }
    transcript
}

/// Shifts word and segment timestamps by `start_offset_ms` onto the capture clock and populates
/// `segments` from `words` when `segments` is empty (for host [`Transcriber`] implementations
/// that return word timings only).
pub fn normalize_transcript(transcript: TranscriptData, start_offset_ms: u64) -> TranscriptData {
    let mut transcript = shift_transcript(transcript, start_offset_ms);
    if transcript.segments.is_empty() && !transcript.words.is_empty() {
        transcript.segments = segments_from_words(&transcript.words);
    }
    transcript
}

fn with_temp_wav<T>(audio: &RecordedAudio, f: impl FnOnce(&Path) -> Result<T>) -> Result<T> {
    let wav = audio.wav_bytes_16k()?;
    let path = std::env::temp_dir().join(format!(
        "scrawl-transcribe-{}-{}.wav",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    std::fs::write(&path, &*wav)?;
    let out = f(&path);
    let _ = std::fs::remove_file(&path);
    out
}

/// Pluggable speech-to-text engine returning words with start/end times (`WordTiming`) and an
/// `estimated` flag (`TranscriptData::estimated`: `false` for measured word times that can cut
/// spoken-cue keyframes, `true` when word times are estimated across speech islands).
///
/// Word times (`t0`, `t1`) returned by [`Self::transcribe`] are milliseconds from
/// `audio.samples[0]`; [`transcribe_with`] shifts them by `audio.start_offset_ms` onto the
/// capture clock.
pub trait Transcriber: Send + Sync {
    fn transcribe(&self, audio: &RecordedAudio) -> Result<Option<TranscriptData>>;

    fn transcribe_wav(&self, wav_path: &Path, _duration_ms: u64) -> Result<Option<TranscriptData>> {
        let bytes = std::fs::read(wav_path)?;
        let audio = crate::audio::recorded_audio_from_wav(bytes)?;
        self.transcribe(&audio)
    }
}

#[derive(Clone, Debug, Default)]
pub struct AppleTranscriber {
    pub locale: Option<String>,
}

impl Transcriber for AppleTranscriber {
    fn transcribe(&self, audio: &RecordedAudio) -> Result<Option<TranscriptData>> {
        with_temp_wav(audio, |wav_path| Ok(Some(apple(wav_path))))
    }

    fn transcribe_wav(&self, wav_path: &Path, _duration_ms: u64) -> Result<Option<TranscriptData>> {
        Ok(Some(apple(wav_path)))
    }
}

#[derive(Clone, Debug)]
pub struct CommandTranscriber {
    pub program: crate::command_transcriber::Program,
}

impl Transcriber for CommandTranscriber {
    fn transcribe(&self, audio: &RecordedAudio) -> Result<Option<TranscriptData>> {
        let duration_ms = audio
            .total_duration_ms()
            .saturating_sub(audio.start_offset_ms);
        with_temp_wav(audio, |wav_path| self.transcribe_wav(wav_path, duration_ms))
    }

    fn transcribe_wav(&self, wav_path: &Path, duration_ms: u64) -> Result<Option<TranscriptData>> {
        let sample_count = (duration_ms.saturating_mul(16)) as usize;
        match crate::command_transcriber::transcribe(
            &self.program,
            wav_path,
            sample_count,
            &mut |line| crate::log::scrawl_log(line),
        ) {
            Ok(transcript) => Ok(Some(transcript)),
            Err(error) => {
                crate::log::scrawl_log(format!("Command transcription failed: {error:#}"));
                Err(error.into())
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Engine {
    #[default]
    Apple,
    Hex,
    Command,
}

impl Engine {
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "apple" => Some(Self::Apple),
            "hex" => Some(Self::Hex),
            "command" => Some(Self::Command),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Apple => "apple",
            Self::Hex => "hex",
            Self::Command => "command",
        }
    }

    /// The configured engine in `config`: `"transcriber"` in the config file, Apple when missing
    /// or unknown.
    pub fn configured_in(config: &crate::handoff::EngineConfig) -> Self {
        config
            .transcriber
            .as_deref()
            .and_then(Self::parse)
            .unwrap_or_default()
    }

    /// The configured engine: `"transcriber"` in the config file, Apple when missing or unknown.
    pub fn configured() -> Self {
        Self::configured_in(&crate::handoff::engine_config())
    }

    /// What recordings use now: a command-line `--transcriber`, else the configured engine.
    pub fn preferred() -> Self {
        match OVERRIDE.load(Ordering::Relaxed) {
            1 => Self::Apple,
            2 => Self::Hex,
            3 => Self::Command,
            _ => Self::configured(),
        }
    }
}

/// The on-device engine's name in menus: Apple's SpeechAnalyzer on macOS; on Windows, Parakeet
/// through `transcribe.cpp` (`crate::win::speech`). Both are `Engine::Apple` in the config.
#[cfg(target_os = "macos")]
pub const LOCAL_ENGINE_TITLE: &str = "Apple Speech";
#[cfg(not(target_os = "macos"))]
pub const LOCAL_ENGINE_TITLE: &str = "Parakeet (on this PC)";

static OVERRIDE: AtomicU8 = AtomicU8::new(0);
static LOGGED_NOT_CONNECTED: std::sync::Once = std::sync::Once::new();
static LOGGED_NO_COMMAND: std::sync::Once = std::sync::Once::new();

/// What the Settings window's Transcription controls show. A Hex preference stays saved while Hex
/// is away, but the active choice follows what recordings would really use.
#[derive(Debug, PartialEq, Eq)]
pub struct MenuState {
    pub hex_title: &'static str,
    pub hex_enabled: bool,
    pub hex_checked: bool,
    /// Ready Hex models as `(id, title, chosen)`, empty when Hex is away.
    pub models: Vec<(String, String, bool)>,
}

#[cfg(feature = "hex-client")]
pub fn menu_state(
    preferred: Engine,
    status: Option<&crate::hex::Status>,
    configured_model: Option<&str>,
) -> MenuState {
    let Some(status) = status else {
        return MenuState {
            hex_title: "Hex (not running)",
            hex_enabled: false,
            hex_checked: false,
            models: Vec::new(),
        };
    };
    let chosen = crate::hex::pick_model(&status.models, configured_model);
    let models: Vec<_> = status
        .usable_models()
        .map(|model| {
            let title = if model.name.is_empty() {
                &model.id
            } else {
                &model.name
            };
            (
                model.id.clone(),
                title.clone(),
                chosen.as_deref() == Some(model.id.as_str()),
            )
        })
        .collect();
    let ready = chosen.is_some();
    MenuState {
        hex_title: if ready { "Hex" } else { "Hex (no model ready)" },
        hex_enabled: ready,
        hex_checked: ready && preferred == Engine::Hex,
        models,
    }
}

/// Pins the engine for this process (`scrawl reprocess --transcriber`).
pub fn set_override(engine: Engine) {
    OVERRIDE.store(
        match engine {
            Engine::Apple => 1,
            Engine::Hex => 2,
            Engine::Command => 3,
        },
        Ordering::Relaxed,
    );
}

fn has_audio_samples(audio: &RecordedAudio) -> bool {
    audio.sample_count() > 0
}

/// Missing microphones and a failed transcriber never prevent saving a bundle.
pub fn transcribe(wav: Option<&Path>, audio: Option<&RecordedAudio>) -> TranscriptData {
    let (Some(wav), Some(audio)) = (wav, audio.filter(|a| has_audio_samples(a))) else {
        return TranscriptData {
            source: "none".into(),
            ..Default::default()
        };
    };
    let live = audio.take_live();
    let engine = Engine::preferred();
    let transcript = run(
        engine,
        || match engine {
            Engine::Apple => Ok(None),
            Engine::Hex => try_hex(audio),
            Engine::Command => try_command(wav, audio),
        },
        || {
            live.and_then(|live| finish_live(live, audio))
                .unwrap_or_else(|| apple(wav))
        },
    );
    shift_transcript(transcript, audio.start_offset_ms)
}

/// Transcribes `audio` (or `wav` when only a WAV file on disk is available) using an explicit
/// [`Transcriber`] implementation (such as a host app's own speech engine), without re-reading
/// a WAV when `audio` is already in memory.
pub fn transcribe_with(
    wav: Option<&Path>,
    audio: Option<&RecordedAudio>,
    transcriber: &dyn Transcriber,
) -> TranscriptData {
    if let Some(audio) = audio.filter(|a| has_audio_samples(a)) {
        if let Some(live) = audio.take_live().and_then(|live| finish_live(live, audio)) {
            return normalize_transcript(live, audio.start_offset_ms);
        }
        match transcriber.transcribe(audio) {
            Ok(Some(transcript)) => return normalize_transcript(transcript, audio.start_offset_ms),
            Ok(None) => {}
            Err(error) => {
                crate::log::scrawl_log(format!("Transcription failed: {error:#}"));
            }
        }
        return TranscriptData {
            source: "offline".into(),
            ..Default::default()
        };
    }
    if let Some(wav) = wav {
        match transcriber.transcribe_wav(wav, 0) {
            Ok(Some(transcript)) => return normalize_transcript(transcript, 0),
            Ok(None) => {}
            Err(error) => {
                crate::log::scrawl_log(format!("Transcription failed: {error:#}"));
            }
        }
    }
    TranscriptData {
        source: "none".into(),
        ..Default::default()
    }
}

/// The transcript made while recording, or `None` (logged) to transcribe the WAV instead.
fn finish_live(
    live: crate::live_transcribe::Live,
    audio: &RecordedAudio,
) -> Option<TranscriptData> {
    let wav_bytes = audio.wav_bytes_16k().ok()?;
    match live.finish(&wav_bytes) {
        Ok(done) => {
            crate::log::scrawl_log(format!(
                "Live transcription: {} words, {} of {} ms streamed, tail {} ms",
                done.transcript.words.len(),
                done.streamed_samples as u64 * 1_000 / 16_000,
                audio.duration_ms(),
                done.tail_ms
            ));
            Some(done.transcript)
        }
        Err(error) => {
            crate::log::scrawl_log(format!(
                "Live transcription failed, transcribing the recording: {error:#}"
            ));
            None
        }
    }
}

/// The preferred engine when it isn't Apple, with Apple as the answer to anything it can't do.
/// `other` returns `Ok(None)` when that engine isn't there at all (Hex not connected or without a
/// ready model, no usable `transcribeCommand`).
fn run(
    engine: Engine,
    other: impl FnOnce() -> Result<Option<TranscriptData>>,
    apple: impl FnOnce() -> TranscriptData,
) -> TranscriptData {
    let name = match engine {
        Engine::Apple => return apple(),
        Engine::Hex => "Hex",
        Engine::Command => "Command",
    };
    match other() {
        Ok(Some(transcript)) => return transcript,
        Ok(None) => {}
        Err(error) => crate::log::scrawl_log(format!(
            "{name} transcription failed, using {LOCAL_ENGINE_TITLE}: {error:#}"
        )),
    }
    apple()
}

/// Runs `transcribeCommand`. Its errors are fixed reasons (`command_transcriber::Failure`),
/// never what the program printed or was given.
fn try_command(wav: &Path, audio: &RecordedAudio) -> Result<Option<TranscriptData>> {
    let program = match crate::handoff::transcribe_command() {
        Ok(program) => program,
        Err(unusable) => {
            LOGGED_NO_COMMAND.call_once(|| {
                crate::log::scrawl_log(format!(
                    "Command transcriber preferred but {unusable}; using {LOCAL_ENGINE_TITLE}"
                ));
            });
            return Ok(None);
        }
    };
    let transcript =
        crate::command_transcriber::transcribe(&program, wav, audio.sample_count(), &mut |line| {
            crate::log::scrawl_log(line)
        })?;
    Ok(Some(transcript))
}

#[cfg(feature = "hex-client")]
fn try_hex(audio: &RecordedAudio) -> Result<Option<TranscriptData>> {
    let not_connected = |reason: String| {
        LOGGED_NOT_CONNECTED.call_once(|| {
            crate::log::scrawl_log(format!(
                "Hex preferred but unavailable ({reason}); using {LOCAL_ENGINE_TITLE}"
            ));
        });
        Ok(None)
    };
    let client = match crate::hex::Client::connect() {
        Ok(client) => client,
        Err(error) => return not_connected(format!("{error:#}")),
    };
    let models = client.models()?;
    let configured = crate::handoff::hex_model();
    let Some(model) = crate::hex::pick_model(&models, configured.as_deref()) else {
        return not_connected("no ready model".into());
    };
    crate::hex::transcribe_audio(&client, &model, &crate::handoff::hex_language(), audio).map(Some)
}

#[cfg(not(feature = "hex-client"))]
fn try_hex(_audio: &RecordedAudio) -> Result<Option<TranscriptData>> {
    LOGGED_NOT_CONNECTED.call_once(|| {
        crate::log::scrawl_log(format!(
            "Hex preferred but hex-client feature is disabled; using {LOCAL_ENGINE_TITLE}"
        ));
    });
    Ok(None)
}

fn apple(wav: &Path) -> TranscriptData {
    let transcript = crate::helper::transcribe(wav).unwrap_or_else(|error| {
        crate::log::scrawl_log(format!("{LOCAL_ENGINE_TITLE} failed: {error:#}"));
        TranscriptData {
            source: "offline".into(),
            missing: Some(crate::model::MissingTranscript {
                reason: format!("{LOCAL_ENGINE_TITLE} failed ({error:#})."),
                retry: false,
            }),
            ..Default::default()
        }
    });
    if let Some(missing) = &transcript.missing {
        crate::log::scrawl_log(format!("No transcript: {}", missing.reason));
        if missing.retry {
            crate::speech_model::ensure();
        }
    }
    transcript
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(source: &str) -> TranscriptData {
        TranscriptData {
            source: source.into(),
            ..Default::default()
        }
    }

    #[test]
    fn engine_names_parse_and_unknown_is_not_an_engine() {
        assert_eq!(Engine::parse("hex"), Some(Engine::Hex));
        assert_eq!(Engine::parse(" Apple "), Some(Engine::Apple));
        assert_eq!(Engine::parse("Command"), Some(Engine::Command));
        assert_eq!(Engine::parse("whisper"), None);
        assert_eq!(Engine::default(), Engine::Apple);
    }

    #[test]
    fn apple_preference_never_asks_another_engine() {
        let result = run(
            Engine::Apple,
            || panic!("no other engine may be asked"),
            || named("speech-analyzer"),
        );
        assert_eq!(result.source, "speech-analyzer");
    }

    #[test]
    fn the_preferred_engine_is_used_when_it_answers() {
        for (engine, source) in [
            (Engine::Hex, "hex:parakeet_v3"),
            (Engine::Command, "command:whisper-scrawl"),
        ] {
            let result = run(
                engine,
                || Ok(Some(named(source))),
                || panic!("Apple must not be asked"),
            );
            assert_eq!(result.source, source);
        }
    }

    #[test]
    fn an_unavailable_or_failing_engine_falls_back_to_apple() {
        for engine in [Engine::Hex, Engine::Command] {
            let apple = || named("speech-analyzer");
            assert_eq!(run(engine, || Ok(None), apple).source, "speech-analyzer");
            let failed = run(engine, || Err(anyhow::anyhow!("took too long")), apple);
            assert_eq!(failed.source, "speech-analyzer");
        }
    }

    #[test]
    #[cfg(feature = "hex-client")]
    fn menu_follows_what_recordings_would_use() {
        use crate::hex::{Model, Status};
        let status = |models: Vec<Model>| Status {
            version: "2.2.0".into(),
            models,
        };
        let ready = |id: &str, name: &str| Model {
            id: id.into(),
            name: name.into(),
            installed: true,
            verified: true,
        };

        let away = menu_state(Engine::Hex, None, None);
        assert!(!away.hex_checked && !away.hex_enabled);
        assert_eq!(away.hex_title, "Hex (not running)");

        let up = status(vec![
            ready("parakeet_v3", "Parakeet v3"),
            ready("qwen3_asr06_b", ""),
        ]);
        let hex = menu_state(Engine::Hex, Some(&up), Some("qwen3_asr06_b"));
        assert!(hex.hex_checked && hex.hex_enabled);
        assert_eq!(
            hex.models,
            vec![
                ("parakeet_v3".into(), "Parakeet v3".into(), false),
                ("qwen3_asr06_b".into(), "qwen3_asr06_b".into(), true),
            ]
        );
        let apple = menu_state(Engine::Apple, Some(&up), None);
        assert!(!apple.hex_checked && apple.hex_enabled);

        let empty = status(vec![Model {
            id: "parakeet_v3".into(),
            name: String::new(),
            installed: false,
            verified: false,
        }]);
        let none = menu_state(Engine::Hex, Some(&empty), None);
        assert!(!none.hex_checked && !none.hex_enabled && none.models.is_empty());
        assert_eq!(none.hex_title, "Hex (no model ready)");
    }

    #[test]
    fn no_audio_is_none_without_asking_any_engine() {
        assert_eq!(transcribe(None, None).source, "none");
    }

    #[test]
    fn in_memory_transcriber_shifts_words_and_builds_segments_without_reading_wav() {
        struct InMemory;
        impl Transcriber for InMemory {
            fn transcribe(&self, audio: &RecordedAudio) -> Result<Option<TranscriptData>> {
                assert_eq!(audio.sample_rate, 16_000);
                assert_eq!(audio.samples.len(), 160);
                Ok(Some(TranscriptData {
                    source: "hex".into(),
                    model: Some("parakeet_v3".into()),
                    text: "hello world.".into(),
                    words: vec![
                        WordTiming {
                            t0: 10,
                            t1: 50,
                            text: "hello".into(),
                        },
                        WordTiming {
                            t0: 60,
                            t1: 100,
                            text: "world.".into(),
                        },
                    ],
                    ..Default::default()
                }))
            }
        }

        let audio = RecordedAudio::new(
            vec![0.1; 160],
            16_000,
            250,
            vec![crate::audio::GapSpan {
                start_ms: 0,
                end_ms: 250,
            }],
        );
        let tr = transcribe_with(None, Some(&audio), &InMemory);
        assert_eq!(tr.source, "hex");
        assert_eq!(tr.words[0].t0, 260);
        assert_eq!(tr.words[0].t1, 300);
        assert_eq!(tr.words[1].t0, 310);
        assert_eq!(tr.words[1].t1, 350);
        assert_eq!(tr.segments.len(), 1);
        assert_eq!(tr.segments[0].t0, 260);
        assert_eq!(tr.segments[0].t1, 350);
        assert_eq!(tr.segments[0].text, "hello world.");
    }
}
