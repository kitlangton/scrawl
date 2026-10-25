//! Transcription: Apple's SpeechAnalyzer through `scrawl-helper` (measured word and segment
//! timings), Hex when `"transcriber": "hex"` is set and Hex is connected (`crate::hex`; word
//! times estimated), or a program of the user's when `"transcriber": "command"` or
//! `"command:<id>"` (`crate::command_transcriber`). Any Hex or command trouble falls back to Apple
//! for that recording.

use std::path::Path;
use std::sync::OnceLock;

use anyhow::Result;

use crate::audio::RecordedAudio;
use crate::model::TranscriptData;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Engine {
    #[default]
    Apple,
    Hex,
    Command,
}

impl Engine {
    /// `"apple"`, `"hex"`, or `"command"`/`"command:<id>"` (which command is
    /// `command_transcriber::command_id`'s to read).
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "apple" => Some(Self::Apple),
            "hex" => Some(Self::Hex),
            _ if crate::command_transcriber::command_id(name).is_some() => Some(Self::Command),
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

    /// The configured engine: `"transcriber"` in the config file, Apple when missing or unknown.
    pub fn configured() -> Self {
        crate::handoff::transcriber()
            .as_deref()
            .and_then(Self::parse)
            .unwrap_or_default()
    }

    /// What recordings use now: a command-line `--transcriber`, else the configured engine.
    pub fn preferred() -> Self {
        choice()
            .as_deref()
            .and_then(Self::parse)
            .unwrap_or_default()
    }
}

/// A `transcriber` value as Settings saves it: the engine's name, `"command:<id>"` for a
/// `transcribeCommands` entry. `None` when it names no engine.
pub fn canonical(name: &str) -> Option<String> {
    match Engine::parse(name)? {
        Engine::Command => crate::command_transcriber::command_id(name)
            .map(|id| crate::command_transcriber::choice_for(&id)),
        engine => Some(engine.name().to_owned()),
    }
}

/// The `transcriber` value recordings follow now: a command-line `--transcriber`, else the
/// config's, as written.
pub fn choice() -> Option<String> {
    OVERRIDE.get().cloned().or_else(crate::handoff::transcriber)
}

/// The on-device engine's name in menus: Apple's SpeechAnalyzer on macOS; on Windows, Parakeet
/// through `transcribe.cpp` (`crate::win::speech`). Both are `Engine::Apple` in the config.
#[cfg(target_os = "macos")]
pub const LOCAL_ENGINE_TITLE: &str = "Apple Speech";
#[cfg(not(target_os = "macos"))]
pub const LOCAL_ENGINE_TITLE: &str = "Parakeet (on this PC)";

/// `--transcriber`, canonical (`canonical`).
static OVERRIDE: OnceLock<String> = OnceLock::new();
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

/// Pins the engine, and for Command which one, for this process (`scrawl reprocess
/// --transcriber`): a value `canonical` accepted.
pub fn set_override(choice: String) {
    let _ = OVERRIDE.set(choice);
}

/// Missing microphones and a failed transcriber never prevent saving a bundle.
pub fn transcribe(wav: Option<&Path>, audio: Option<&RecordedAudio>) -> TranscriptData {
    let (Some(wav), Some(audio)) = (wav, audio.filter(|audio| audio.sample_count > 0)) else {
        return TranscriptData {
            source: "none".into(),
            ..Default::default()
        };
    };
    let live = audio.live.take();
    let engine = Engine::preferred();
    run(
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
    )
}

/// The transcript made while recording, or `None` (logged) to transcribe the WAV instead.
fn finish_live(
    live: crate::live_transcribe::Live,
    audio: &RecordedAudio,
) -> Option<TranscriptData> {
    match live.finish(&audio.wav_bytes) {
        Ok(done) => {
            crate::capture::scrawl_log(format!(
                "Live transcription: {} words, {} of {} ms streamed, tail {} ms",
                done.transcript.words.len(),
                done.streamed_samples as u64 * 1_000 / 16_000,
                audio.duration_ms,
                done.tail_ms
            ));
            Some(done.transcript)
        }
        Err(error) => {
            crate::capture::scrawl_log(format!(
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
        Err(error) => crate::capture::scrawl_log(format!(
            "{name} transcription failed, using {LOCAL_ENGINE_TITLE}: {error:#}"
        )),
    }
    apple()
}

/// Runs the chosen command. Its errors are fixed reasons (`command_transcriber::Failure`),
/// never what the program printed or was given.
fn try_command(wav: &Path, audio: &RecordedAudio) -> Result<Option<TranscriptData>> {
    let program = match crate::handoff::transcribe_command(&choice().unwrap_or_default()) {
        Ok(program) => program,
        Err(unusable) => {
            LOGGED_NO_COMMAND.call_once(|| {
                crate::capture::scrawl_log(format!(
                    "Command transcriber preferred but {unusable}; using {LOCAL_ENGINE_TITLE}"
                ));
            });
            return Ok(None);
        }
    };
    let transcript =
        crate::command_transcriber::transcribe(&program, wav, audio.sample_count, &mut |line| {
            crate::capture::scrawl_log(line)
        })?;
    Ok(Some(transcript))
}

fn try_hex(audio: &RecordedAudio) -> Result<Option<TranscriptData>> {
    let not_connected = |reason: String| {
        LOGGED_NOT_CONNECTED.call_once(|| {
            crate::capture::scrawl_log(format!(
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

fn apple(wav: &Path) -> TranscriptData {
    let transcript = crate::helper::transcribe(wav).unwrap_or_else(|error| {
        crate::capture::scrawl_log(format!("{LOCAL_ENGINE_TITLE} failed: {error:#}"));
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
        crate::capture::scrawl_log(format!("No transcript: {}", missing.reason));
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
        assert_eq!(Engine::parse("command:parakeet"), Some(Engine::Command));
        assert_eq!(Engine::parse(" Command:Whisper "), Some(Engine::Command));
        assert_eq!(Engine::parse("command:"), None);
        assert_eq!(Engine::parse("commander"), None);
        assert_eq!(Engine::parse("whisper"), None);
        assert_eq!(Engine::default(), Engine::Apple);
    }

    #[test]
    fn a_choice_is_saved_canonical() {
        assert_eq!(canonical(" Apple ").as_deref(), Some("apple"));
        assert_eq!(canonical("HEX").as_deref(), Some("hex"));
        assert_eq!(canonical("command").as_deref(), Some("command"));
        assert_eq!(canonical("command:command").as_deref(), Some("command"));
        assert_eq!(
            canonical(" Command: Parakeet ").as_deref(),
            Some("command:parakeet")
        );
        for nothing in ["whisper", "command:", "commands:x", ""] {
            assert_eq!(canonical(nothing), None, "{nothing}");
        }
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
}
