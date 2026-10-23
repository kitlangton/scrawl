//! Handy (handy.computer, the open-source local dictation app) as an optional transcriber
//! (`"transcriber": "handy"`). Handy 0.9.0 and later transcribe a WAV headlessly with the model
//! and language chosen in Handy (`handy --transcribe-file <wav> --json`), so Scrawl runs that
//! under the command transcriber's supervision (`command_transcriber::Program`: no shell, its
//! own process group, a wall-clock limit, an 8 MiB stdout cap) and reads `text` and `model`
//! from the one JSON line it prints. Scrawl never downloads or picks a Handy model.
//!
//! Handy returns text without word times. Each run loads the model again (about 0.4 s), and a
//! clip of half a second loses the context that names its words, so the whole recording goes
//! in one run and its words are spread over the speech islands by length afterwards. Those
//! times are estimates (`TranscriptData::estimated`), as for Hex: spoken cues never cut
//! keyframes from them.
//!
//! Only macOS looks for Handy for now: the runner needs Unix process groups, and Windows
//! recordings keep the local engine.

use std::path::{Path, PathBuf};

use anyhow::Result;
use serde_json::Value;

use crate::audio::{self, RecordedAudio};
use crate::command_transcriber::{Failure, Program};
use crate::model::{TranscriptData, TranscriptSegment, WordTiming};

/// The first Handy with `--transcribe-file`, as (major, minor).
const HEADLESS_SINCE: (u64, u64) = (0, 9);

/// Handy's executable in `/Applications/Handy.app` or `~/Applications/Handy.app`, or `None`
/// (not installed, older than 0.9.0, or not macOS). An older Handy isn't offered at all: it
/// doesn't know `--transcribe-file`.
pub fn executable() -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let mut apps = vec![PathBuf::from("/Applications/Handy.app")];
    apps.extend(dirs::home_dir().map(|home| home.join("Applications/Handy.app")));
    apps.into_iter()
        .filter(|app| {
            std::fs::read_to_string(app.join("Contents/Info.plist"))
                .is_ok_and(|plist| version(&plist).is_some_and(|v| v >= HEADLESS_SINCE))
        })
        .map(|app| app.join("Contents/MacOS/handy"))
        .find(|path| path.is_file())
}

/// The major and minor `CFBundleShortVersionString` of an XML `Info.plist` (Handy's is XML).
fn version(plist: &str) -> Option<(u64, u64)> {
    let (_, rest) = plist.split_once("<key>CFBundleShortVersionString</key>")?;
    let (value, _) = rest
        .trim_start()
        .strip_prefix("<string>")?
        .split_once("</string>")?;
    let mut parts = value.trim().split('.').map(|part| part.parse::<u64>().ok());
    Some((parts.next()??, parts.next().flatten().unwrap_or(0)))
}

/// The headless run: `{audio}` becomes the recording's WAV (`Program::arguments`).
fn program(executable: &Path) -> Result<Program> {
    let argv = vec![
        executable.to_string_lossy().into_owned(),
        "--transcribe-file".into(),
        "{audio}".into(),
        "--json".into(),
    ];
    Ok(Program::new(argv, None)?)
}

/// Transcribes `wav` (the file `audio` was written to) with the Handy at `executable`.
pub fn transcribe(executable: &Path, wav: &Path, audio: &RecordedAudio) -> Result<TranscriptData> {
    // Decoded first, so a WAV Scrawl can't read fails before Handy loads its model.
    let samples = audio::decode_wav_16k(&audio.wav_bytes)?;
    let stdout = program(executable)?.output(wav, audio.sample_count)?;
    let (text, model) = parse(&stdout)?;
    Ok(transcript(
        &text,
        model,
        crate::hex::speech_clips(&samples),
        audio.duration_ms,
    ))
}

/// The text and model id of Handy's `--json` line (its last line); timings and the backend it
/// also prints are ignored. Like the command transcriber's, failures never quote the output.
fn parse(stdout: &[u8]) -> Result<(String, Option<String>), Failure> {
    let line = stdout
        .split(|byte| *byte == b'\n')
        .rfind(|line| !line.trim_ascii().is_empty())
        .unwrap_or_default();
    let Ok(Value::Object(object)) = serde_json::from_slice::<Value>(line) else {
        return Err(Failure::NotJson);
    };
    let Some(Value::String(text)) = object.get("text") else {
        return Err(Failure::NoText);
    };
    let model = object
        .get("model")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|model| !model.is_empty())
        .map(str::to_owned);
    Ok((text.split_whitespace().collect::<Vec<_>>().join(" "), model))
}

/// Handy's text as a transcript, its words spread over the recording's speech (`clips`, as Hex
/// would send them), or over the whole recording when there are none: Handy heard all of it.
fn transcript(
    text: &str,
    model: Option<String>,
    mut clips: Vec<(u64, u64)>,
    duration_ms: u64,
) -> TranscriptData {
    if clips.is_empty() {
        clips.push((0, duration_ms));
    }
    let (segments, words) = spread_over(text, &clips);
    TranscriptData {
        // A model id can be a repository path; its file name tells the models apart.
        source: match model.as_deref() {
            Some(model) => format!("handy:{}", model.rsplit('/').next().unwrap_or(model)),
            None => "handy".into(),
        },
        model,
        text: text.to_owned(),
        segments,
        words,
        estimated: true,
        missing: None,
    }
}

/// Spreads `text` over `clips` (in order, not overlapping): each clip gets the words whose
/// middle falls in its share of the speech by length, and spreads them across itself as
/// `hex::spread_words` does, so no word is placed in a pause. No clips place nothing
/// (`transcript` always passes one).
fn spread_over(text: &str, clips: &[(u64, u64)]) -> (Vec<TranscriptSegment>, Vec<WordTiming>) {
    if clips.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let weight = |token: &str| token.chars().count().max(1) as u128;
    let total: u128 = tokens.iter().map(|token| weight(token)).sum();
    let speech: u128 = clips
        .iter()
        .map(|&(start, end)| u128::from(end.saturating_sub(start)))
        .sum();
    let mut groups: Vec<Vec<&str>> = vec![Vec::new(); clips.len()];
    let (mut before, mut clip, mut clip_end) = (0_u128, 0, 0_u128);
    for token in tokens {
        // Where the word's middle falls in the speech, scaled by 2 × `total` to stay whole.
        let middle = (2 * before + weight(token)) * speech;
        before += weight(token);
        while clip + 1 < clips.len() {
            let end = clip_end + u128::from(clips[clip].1.saturating_sub(clips[clip].0));
            if middle < 2 * total * end {
                break;
            }
            clip_end = end;
            clip += 1;
        }
        groups[clip].push(token);
    }
    let (mut segments, mut words) = (Vec::new(), Vec::new());
    for (&(start, end), group) in clips.iter().zip(groups) {
        if group.is_empty() {
            continue;
        }
        let (clip_segments, clip_words) = crate::hex::spread_words(&group.join(" "), start, end);
        segments.extend(clip_segments);
        words.extend(clip_words);
    }
    (segments, words)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What Handy 0.9.7 prints for `--transcribe-file x.wav --json`.
    const ANSWER: &[u8] = br#"{"audio_secs":3.1,"best_ms":423,"bound_backend":"MTL0","load_ms":274,"model":"handy-computer/parakeet-unified-en-0.6b-gguf/parakeet-unified-en-0.6b-Q8_0.gguf","requested_device":"settings","rtf":7.3,"text":"Click here.  Now look\nat this.","transcribe_ms":[423]}
"#;
    const MODEL: &str =
        "handy-computer/parakeet-unified-en-0.6b-gguf/parakeet-unified-en-0.6b-Q8_0.gguf";

    #[test]
    fn only_a_handy_with_headless_mode_is_offered() {
        let plist = |version: &str| {
            format!(
                "<dict>\n\t<key>CFBundleName</key>\n\t<string>Handy</string>\n\t<key>CFBundleShortVersionString</key>\n\t<string>{version}</string>\n</dict>"
            )
        };
        assert_eq!(version(&plist("0.9.7")), Some((0, 9)));
        assert_eq!(version(&plist("1")), Some((1, 0)));
        assert!(version(&plist("0.8.2")).unwrap() < HEADLESS_SINCE);
        assert!(version(&plist("0.10.0")).unwrap() >= HEADLESS_SINCE);
        assert_eq!(version(&plist("beta")), None);
        assert_eq!(version("<dict></dict>"), None);
    }

    #[test]
    fn reads_the_text_and_model_of_the_json_line() {
        let (text, model) = parse(ANSWER).unwrap();
        assert_eq!(text, "Click here. Now look at this.");
        assert_eq!(model.as_deref(), Some(MODEL));

        let mut noisy = b"loading model\n".to_vec();
        noisy.extend_from_slice(br#"{"text":"","model":" "}"#);
        assert_eq!(parse(&noisy).unwrap(), (String::new(), None));
    }

    #[test]
    fn output_without_json_or_text_is_a_failure() {
        assert_eq!(parse(b""), Err(Failure::NotJson));
        assert_eq!(parse(b"text: Click here."), Err(Failure::NotJson));
        assert_eq!(parse(br#"["Click here."]"#), Err(Failure::NotJson));
        assert_eq!(parse(br#"{"model":"parakeet"}"#), Err(Failure::NoText));
        assert_eq!(parse(br#"{"text":42}"#), Err(Failure::NoText));
    }

    #[test]
    fn words_go_to_the_clips_they_were_spoken_in_and_never_into_pauses() {
        // Two equal clips with a long pause between them, two sentences of equal length.
        let clips = [(1_000, 3_000), (8_000, 10_000)];
        let (segments, words) = spread_over("Open the menu. Click the button.", &clips);
        let texts: Vec<_> = words.iter().map(|word| word.text.as_str()).collect();
        assert_eq!(texts, ["Open", "the", "menu.", "Click", "the", "button."]);
        assert!(words[..3].iter().all(|w| w.t0 >= 1_000 && w.t1 <= 3_000));
        assert!(words[3..].iter().all(|w| w.t0 >= 8_000 && w.t1 <= 10_000));
        assert_eq!((words[0].t0, words[2].t1), (1_000, 3_000));
        assert_eq!((words[3].t0, words[5].t1), (8_000, 10_000));
        let spans: Vec<_> = segments
            .iter()
            .map(|s| (s.t0, s.t1, s.text.as_str()))
            .collect();
        assert_eq!(
            spans,
            [
                (1_000, 3_000, "Open the menu."),
                (8_000, 10_000, "Click the button.")
            ]
        );
    }

    #[test]
    fn a_longer_clip_gets_more_of_the_words() {
        let clips = [(0, 1_000), (2_000, 5_000)];
        let (_, words) = spread_over("one two three four", &clips);
        assert_eq!(words.len(), 4);
        assert!(words[0].t1 <= 1_000);
        assert!(words[1..].iter().all(|w| w.t0 >= 2_000 && w.t1 <= 5_000));
        assert!(words.windows(2).all(|pair| pair[0].t0 <= pair[1].t0));
    }

    #[test]
    fn more_clips_than_words_leaves_some_clips_empty() {
        let clips = [(0, 1_000), (2_000, 3_000), (4_000, 5_000)];
        let (segments, words) = spread_over("Hi.", &clips);
        assert_eq!(words.len(), 1);
        assert_eq!((words[0].t0, words[0].t1), (2_000, 3_000));
        assert_eq!(segments.len(), 1);
        assert_eq!(spread_over("", &clips), (Vec::new(), Vec::new()));
        assert_eq!(spread_over("Hi.", &[]), (Vec::new(), Vec::new()));
    }

    #[test]
    fn the_transcript_is_estimated_and_names_handy_and_its_model() {
        let named = transcript("Click here.", Some(MODEL.into()), vec![(500, 1_500)], 2_000);
        assert_eq!(named.source, "handy:parakeet-unified-en-0.6b-Q8_0.gguf");
        assert_eq!(named.model.as_deref(), Some(MODEL));
        assert!(named.estimated);
        assert_eq!((named.words[0].t0, named.words[1].t1), (500, 1_500));

        // Too quiet or short to find speech by energy: the words cover the whole recording.
        let quiet = transcript("Click here.", None, Vec::new(), 2_000);
        assert_eq!(quiet.source, "handy");
        assert_eq!((quiet.words[0].t0, quiet.words[1].t1), (0, 2_000));

        let silent = transcript("", None, vec![(500, 1_500)], 2_000);
        assert!(silent.text.is_empty() && silent.words.is_empty() && silent.segments.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn runs_handy_headlessly_on_the_recording() {
        use std::os::unix::fs::PermissionsExt;

        let dir = std::env::temp_dir().join(format!("scrawl handy {}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A stand-in for Handy that answers only the headless call Scrawl makes.
        let fake = dir.join("handy");
        std::fs::write(
            &fake,
            "#!/bin/sh\n[ \"$1 $3\" = \"--transcribe-file --json\" ] && [ -f \"$2\" ] || exit 2\necho 'loading' >&2\necho '{\"model\":\"parakeet-tdt-0.6b-v3\",\"text\":\"Open the menu. Click the button.\"}'\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Speech from 1 s to 2 s and from 4 s to 5 s of six.
        let mut samples = vec![0.0_f32; 16_000 * 6];
        samples[16_000..16_000 * 2].fill(0.2);
        samples[16_000 * 4..16_000 * 5].fill(0.2);
        let wav_bytes = audio::encode_wav_16k(&samples).unwrap();
        let wav = dir.join("audio.wav");
        std::fs::write(&wav, &wav_bytes).unwrap();
        let recorded = RecordedAudio {
            sample_count: samples.len(),
            duration_ms: 6_000,
            wav_bytes,
            live: Default::default(),
        };

        let transcript = transcribe(&fake, &wav, &recorded).unwrap();
        assert_eq!(transcript.source, "handy:parakeet-tdt-0.6b-v3");
        assert_eq!(transcript.text, "Open the menu. Click the button.");
        assert!(transcript.estimated);
        assert_eq!(transcript.words.len(), 6);
        assert!(
            transcript.words[..3]
                .iter()
                .all(|w| w.t0 >= 1_000 && w.t1 <= 2_000)
        );
        assert!(
            transcript.words[3..]
                .iter()
                .all(|w| w.t0 >= 4_000 && w.t1 <= 5_000)
        );

        std::fs::write(&fake, "#!/bin/sh\nexit 1\n").unwrap();
        let failed = transcribe(&fake, &wav, &recorded).unwrap_err();
        assert_eq!(failed.to_string(), Failure::Exit(1).to_string());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
