//! On-device transcription on Windows and Linux, with measured word timings: Parakeet through
//! `transcribe.cpp`, the engine Hex uses. It loads a model Hex has already installed
//! (`voice-control/models` in the user's data folder), else Apparent's own copy of Hex's Parakeet
//! Unified English (`%LOCALAPPDATA%\Apparent\models`, `~/.local/share/apparent/models`), which
//! `install` downloads once in the background and checks against its SHA-256. Like the Hex
//! client, it transcribes each speech island on its own; unlike it, every word keeps the time the
//! model heard it (`estimated: false`), so spoken cues ("look here") can cut keyframes.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use transcribe_cpp::{Model, RunOptions, Session, TimestampKind};

use crate::capture::desktop::helper::SpeechModelLine;
use crate::shared::model::{TranscriptData, TranscriptSegment, WordTiming};

const KHZ: u64 = 16;
/// Silence kept around each clip, so first and last words aren't clipped.
const CLIP_PAD_MS: u64 = 200;

/// Reuse one Parakeet session across speech chunks discovered during a recording.
pub struct Incremental {
    session: Session,
    name: String,
}

impl Incremental {
    pub fn new() -> Result<Self> {
        let path = installed_models().into_iter().next().context("No Parakeet model installed")?;
        Self::load(&path)
    }

    pub fn load(path: &Path) -> Result<Self> {
        init_backends()?;
        let session = Model::load(path)?.session()?;
        Ok(Self { session, name: model_name(path) })
    }

    pub fn transcribe(&mut self, samples: &[f32], offset_ms: u64) -> Result<TranscriptData> {
        let options = RunOptions {
            timestamps: TimestampKind::Word,
            language: Some("en".into()),
            ..Default::default()
        };
        // One-shot runs replace native results (including word times); the binding
        // materializes owned output. Do not use a streaming context across captures.
        let result = self.session.run(samples, &options)?;
        let text = result.text.split_whitespace().collect::<Vec<_>>().join(" ");
        let at = |ms: i64| offset_ms + ms.max(0) as u64;
        let mut transcript = TranscriptData {
            source: format!("parakeet:{}", self.name),
            model: Some(self.name.clone()),
            text: text.clone(),
            ..Default::default()
        };
        if text.is_empty() { return Ok(transcript); }
        transcript.words = result.words.iter().filter(|w| !w.text.trim().is_empty()).map(|w| WordTiming {
            t0: at(w.t0_ms), t1: at(w.t1_ms.max(w.t0_ms)), text: w.text.trim().to_owned(),
        }).collect();
        if transcript.words.is_empty() {
            let (segments, words) = crate::shared::hex::spread_words(&text, offset_ms, offset_ms + samples.len() as u64 / KHZ);
            transcript.estimated = true;
            transcript.segments = segments;
            transcript.words = words;
        } else {
            transcript.segments = result.segments.iter().filter(|s| !s.text.trim().is_empty()).map(|s| TranscriptSegment {
                t0: at(s.t0_ms), t1: at(s.t1_ms.max(s.t0_ms)), text: s.text.trim().to_owned(),
            }).collect();
            if transcript.segments.is_empty() {
                transcript.segments.push(TranscriptSegment { t0: transcript.words[0].t0, t1: transcript.words.last().unwrap().t1, text });
            }
        }
        Ok(transcript)
    }
}

/// The model Apparent downloads when Hex hasn't installed one: the same file, revision and
/// checksum as Hex's `PARAKEET_UNIFIED_ENGLISH_ARTIFACT`.
pub struct Artifact {
    pub filename: &'static str,
    pub url: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

pub const PARAKEET: Artifact = Artifact {
    filename: "parakeet-unified-en-0.6b-Q8_0.gguf",
    url: "https://huggingface.co/handy-computer/parakeet-unified-en-0.6b-gguf/resolve/7e948f21b7bdbac698d3318db9d350f1096f3b6c/parakeet-unified-en-0.6b-Q8_0.gguf",
    bytes: 731_357_568,
    sha256: "4b50b6dd862bf6e346929aaf4f5eaacec003bfa3f56462d6c874b41ef2f38795",
};

fn models_dir() -> PathBuf {
    crate::shared::hex::support_dir().join("models")
}

/// Where Apparent keeps the model it downloaded (`APPARENT_MODELS_DIR` for tests).
pub fn own_models_dir() -> PathBuf {
    std::env::var_os("APPARENT_MODELS_DIR")
        .map(PathBuf::from)
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(default_models_dir)
}

#[cfg(target_os = "windows")]
fn default_models_dir() -> PathBuf {
    crate::capture::engine::runtime::platform::legacy_windows::local_dir().join("models")
}

#[cfg(target_os = "linux")]
fn default_models_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("apparent/models")
}

fn receipt_path(model: &Path) -> PathBuf {
    model.with_file_name(format!(
        "{}.verified.json",
        model.file_name().unwrap_or_default().to_string_lossy()
    ))
}

/// `.gguf` files in `dir` that have a `.verified.json` receipt beside them.
fn verified_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| {
            path.extension().is_some_and(|ext| ext == "gguf") && receipt_path(path).is_file()
        })
        .collect()
}

/// Hex's installed models (verified downloads), Hex's chosen one first, then Parakeet ones;
/// then the one Apparent downloaded.
pub fn installed_models() -> Vec<PathBuf> {
    let mut models = hex_models();
    models.extend(verified_in(&own_models_dir()));
    models
}

fn hex_models() -> Vec<PathBuf> {
    let chosen = std::fs::read(crate::shared::hex::support_dir().join("settings.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|settings| {
            settings["transcription"]["model"]
                .as_str()
                .map(str::to_owned)
        })
        .map(|id| id.replace('_', "-"));
    let mut models = verified_in(&models_dir());
    let rank = |path: &PathBuf| {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let picked = chosen.as_deref().is_some_and(|id| name.starts_with(id));
        (!picked, !name.starts_with("parakeet"), name.into_owned())
    };
    models.sort_by_key(rank);
    models
}

/// The model's name for `TranscriptData.model`: its file name without the extension.
fn model_name(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

pub fn transcribe(audio: &Path) -> Result<TranscriptData> {
    let Some(path) = installed_models().into_iter().next() else {
        // The background download hasn't finished (or failed): the bundle is transcribed again
        // once it has.
        let status = crate::capture::engine::runtime::audio::transcription::speech_model::current();
        let why = match status.state {
            crate::capture::engine::runtime::audio::transcription::speech_model::State::Failed => "failed",
            _ => "downloading",
        };
        return Ok(TranscriptData {
            source: "offline".into(),
            missing: Some(crate::capture::engine::runtime::audio::transcription::speech_model::missing(
                why,
                "en",
                status.error.as_deref(),
            )),
            ..Default::default()
        });
    };
    let wav =
        std::fs::read(audio).with_context(|| format!("Could not read {}", audio.display()))?;
    let samples = crate::capture::engine::runtime::audio::audio::decode_wav_16k(&wav)?;
    let duration_ms = samples.len() as u64 / KHZ;
    let mut clips = crate::shared::hex::plan_clips(&crate::capture::engine::runtime::audio::audio::speech_islands(&samples), duration_ms);
    if clips.is_empty() && duration_ms >= 480 {
        clips.push((0, duration_ms));
    }
    let name = model_name(&path);
    let mut transcript = TranscriptData {
        source: format!("parakeet:{name}"),
        model: Some(name),
        estimated: false,
        ..Default::default()
    };
    if clips.is_empty() {
        return Ok(transcript);
    }

    init_backends()?;
    let model = Model::load(&path).with_context(|| format!("Could not load {}", path.display()))?;
    let mut session = model.session()?;
    let options = RunOptions {
        timestamps: TimestampKind::Word,
        language: Some("en".into()),
        ..Default::default()
    };
    let mut texts = Vec::new();
    for (start, end) in clips {
        let from_ms = start.saturating_sub(CLIP_PAD_MS);
        let from = (from_ms * KHZ) as usize;
        let to = (((end + CLIP_PAD_MS) * KHZ) as usize).min(samples.len());
        if from >= to {
            continue;
        }
        let result = session.run(&samples[from..to], &options)?;
        let text = result.text.split_whitespace().collect::<Vec<_>>().join(" ");
        if text.is_empty() {
            continue;
        }
        let at = |ms: i64| from_ms + ms.max(0) as u64;
        let words: Vec<WordTiming> = result
            .words
            .iter()
            .filter(|word| !word.text.trim().is_empty())
            .map(|word| WordTiming {
                t0: at(word.t0_ms),
                t1: at(word.t1_ms.max(word.t0_ms)),
                text: word.text.trim().to_owned(),
            })
            .collect();
        if words.is_empty() {
            // No word times from this model: spread the text over the clip, and say so.
            let (segments, words) = crate::shared::hex::spread_words(&text, start, end);
            transcript.estimated = true;
            transcript.segments.extend(segments);
            transcript.words.extend(words);
        } else {
            let segments: Vec<TranscriptSegment> = result
                .segments
                .iter()
                .filter(|s| !s.text.trim().is_empty())
                .map(|s| TranscriptSegment {
                    t0: at(s.t0_ms),
                    t1: at(s.t1_ms.max(s.t0_ms)),
                    text: s.text.trim().to_owned(),
                })
                .collect();
            transcript.segments.extend(if segments.is_empty() {
                vec![TranscriptSegment {
                    t0: words[0].t0,
                    t1: words[words.len() - 1].t1,
                    text: text.clone(),
                }]
            } else {
                segments
            });
            transcript.words.extend(words);
        }
        texts.push(text);
    }
    transcript.text = texts.join(" ");
    Ok(transcript)
}

/// Register the runtime-selected CPU backend once, before any thread loads a model.
/// The native registry cannot be mutated concurrently with model loading.
fn init_backends() -> Result<()> {
    static INIT: std::sync::OnceLock<std::result::Result<(), String>> = std::sync::OnceLock::new();
    INIT.get_or_init(|| {
        transcribe_cpp::disable_logging();
        transcribe_cpp::init_backends_default().map_err(|error| format!("{error:#}"))
    })
    .clone()
    .map_err(|error| anyhow::anyhow!("transcribe.cpp backends: {error}"))
}

/// Makes sure a model is on disk: `installed` when Hex's or Apparent's own is there, else downloads
/// `PARAKEET` into `own_models_dir()`, reporting progress, and keeps it only if its SHA-256
/// matches. The same lines as macOS's `apparent-helper speech-model --install`.
pub fn install(mut line: impl FnMut(SpeechModelLine)) -> Result<()> {
    let report = |status: &str, progress: Option<f64>, error: Option<String>| SpeechModelLine {
        status: status.into(),
        engine: Some("Parakeet".into()),
        locale: Some("en".into()),
        locales: None,
        progress,
        error,
    };
    if installed_models().is_empty() {
        let mut progress = |done: f64| line(report("downloading", Some(done), None));
        if let Err(error) = download(&PARAKEET, &own_models_dir(), &mut progress) {
            line(report("failed", None, Some(format!("{error:#}"))));
            return Ok(());
        }
    }
    line(report("installed", Some(1.0), None));
    Ok(())
}

/// Downloads `artifact` into `dir` (resuming a `.partial` file), checks it, and writes the
/// `.verified.json` receipt that `installed_models` looks for. A file that fails the check is
/// deleted, so the next try starts over.
pub fn download(artifact: &Artifact, dir: &Path, progress: &mut dyn FnMut(f64)) -> Result<PathBuf> {
    std::fs::create_dir_all(dir).with_context(|| format!("Could not create {}", dir.display()))?;
    let model = dir.join(artifact.filename);
    if !model.is_file() {
        let partial = dir.join(format!("{}.partial", artifact.filename));
        if std::fs::metadata(&partial).map_or(0, |m| m.len()) < artifact.bytes {
            fetch(artifact.url, &partial, artifact.bytes, progress)?;
        }
        std::fs::rename(&partial, &model)?;
    }
    if let Err(error) = verify(&model, artifact) {
        let _ = std::fs::remove_file(&model);
        return Err(error);
    }
    let receipt = serde_json::json!({
        "sha256": artifact.sha256,
        "bytes": artifact.bytes,
        "url": artifact.url,
    });
    std::fs::write(receipt_path(&model), receipt.to_string())?;
    Ok(model)
}

fn verify(path: &Path, artifact: &Artifact) -> Result<()> {
    use sha2::{Digest, Sha256};
    let len = std::fs::metadata(path)?.len();
    if len != artifact.bytes {
        bail!(
            "{} is {len} bytes, expected {}",
            artifact.filename,
            artifact.bytes
        );
    }
    let mut hasher = Sha256::new();
    std::io::copy(&mut std::fs::File::open(path)?, &mut hasher)?;
    let sha256 = format!("{:x}", hasher.finalize());
    if sha256 != artifact.sha256 {
        bail!("{} failed its checksum ({sha256})", artifact.filename);
    }
    Ok(())
}

/// Fetches `url` into `partial` (resuming it), reporting how much of `total` has arrived.
fn fetch(url: &str, partial: &Path, total: u64, progress: &mut dyn FnMut(f64)) -> Result<()> {
    use std::io::Read;
    let (program, mut child) = spawn_fetch(url, partial)?;
    loop {
        if let Some(status) = child.try_wait()? {
            if status.success() {
                progress(1.0);
                return Ok(());
            }
            let mut error = String::new();
            if let Some(mut stderr) = child.stderr.take() {
                let _ = stderr.read_to_string(&mut error);
            }
            bail!("the download failed ({program}, {status}): {}", error.trim());
        }
        let have = std::fs::metadata(partial).map_or(0, |m| m.len());
        progress((have as f64 / total as f64).min(0.99));
        std::thread::sleep(Duration::from_millis(500));
    }
}

const CURL_ARGS: [&str; 9] = [
    "--fail",
    "--location",
    "--silent",
    "--show-error",
    "--retry",
    "3",
    "--continue-at",
    "-",
    "--output",
];

/// Windows' own `curl.exe` (in System32 since Windows 10 1803), without a console window.
#[cfg(target_os = "windows")]
fn spawn_fetch(url: &str, partial: &Path) -> Result<(String, std::process::Child)> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let curl = std::env::var_os("SystemRoot")
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
        .join(r"System32\curl.exe");
    let child = std::process::Command::new(&curl)
        .args(CURL_ARGS)
        .arg(partial)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .with_context(|| format!("Could not run {}", curl.display()))?;
    Ok(("curl".into(), child))
}

/// `curl`, else `wget`: one of them is on nearly every desktop Linux.
#[cfg(target_os = "linux")]
fn spawn_fetch(url: &str, partial: &Path) -> Result<(String, std::process::Child)> {
    let mut command = if crate::shared::handoff::on_path("curl").is_some() {
        let mut command = std::process::Command::new("curl");
        command.args(CURL_ARGS).arg(partial).arg(url);
        command
    } else if crate::shared::handoff::on_path("wget").is_some() {
        let mut command = std::process::Command::new("wget");
        command
            .args(["--quiet", "--continue", "--tries=3", "--output-document"])
            .arg(partial)
            .arg(url);
        command
    } else {
        bail!("downloading the speech model needs curl or wget");
    };
    let program = command.get_program().to_string_lossy().into_owned();
    let child = command
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| format!("Could not run {program}"))?;
    Ok((program, child))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A five-byte artifact whose checksum is `sha256("hello")`.
    const HELLO: Artifact = Artifact {
        filename: "fixture.gguf",
        url: "http://127.0.0.1:9/never-fetched",
        bytes: 5,
        sha256: "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824",
    };

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("apparent-speech-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_finished_download_that_matches_gets_a_receipt_and_counts_as_installed() {
        let dir = temp_dir("good");
        std::fs::write(dir.join("fixture.gguf.partial"), b"hello").unwrap();
        let model = download(&HELLO, &dir, &mut |_| {}).unwrap();
        assert_eq!(model, dir.join("fixture.gguf"));
        assert!(receipt_path(&model).is_file());
        assert_eq!(verified_in(&dir), vec![model]);
    }

    #[test]
    fn a_download_that_fails_its_checksum_is_deleted_and_not_installed() {
        let dir = temp_dir("bad");
        std::fs::write(dir.join("fixture.gguf.partial"), b"hellp").unwrap();
        let error = download(&HELLO, &dir, &mut |_| {}).unwrap_err();
        assert!(format!("{error:#}").contains("checksum"));
        assert!(!dir.join("fixture.gguf").exists());
        assert!(verified_in(&dir).is_empty());
    }

    /// Exercise the actual compute kernels, not just model loading. Linux release CI
    /// also runs this with the packaged libraries under SDE Ice Lake (no AVX-VNNI).
    #[test]
    #[ignore]
    fn downloads_and_runs_parakeet() {
        let kept = std::env::var_os("APPARENT_TEST_MODEL_DIR").map(PathBuf::from);
        let dir = kept.clone().unwrap_or_else(|| temp_dir("parakeet"));
        let fresh = !dir.join(PARAKEET.filename).is_file();
        let mut last = 0.0;
        let model = download(&PARAKEET, &dir, &mut |done| last = done).unwrap();
        if fresh {
            assert_eq!(last, 1.0);
        }
        #[cfg(target_arch = "x86_64")]
        if std::env::var_os("APPARENT_TEST_NO_AVX512").is_some() {
            // Under SDE: proves the emulated CPU is the one without AVX-512 (#58).
            assert!(!std::arch::is_x86_feature_detected!("avx512f"));
        }
        #[cfg(target_arch = "x86_64")]
        if std::env::var_os("APPARENT_TEST_NO_AVX_VNNI").is_some() {
            assert!(!std::arch::is_x86_feature_detected!("avxvnni"));
            assert!(std::arch::is_x86_feature_detected!("avx512vnni"));
        }
        init_backends().unwrap();
        for device in transcribe_cpp::devices() {
            eprintln!("device: {} ({})", device.name, device.description);
        }
        let model = Model::load(&model).unwrap();
        let pcm: Vec<f32> = (0..32_000)
            .map(|i| (i as f32 * 440.0 * std::f32::consts::TAU / 16_000.0).sin() * 0.1)
            .collect();
        model
            .session()
            .unwrap()
            .run(&pcm, &RunOptions::default())
            .unwrap();
        if kept.is_none() {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
