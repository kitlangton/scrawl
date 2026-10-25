//! On-device transcription on Windows, with measured word timings: Parakeet through
//! `transcribe.cpp`, the engine Hex uses. It loads a model Hex has already installed
//! (`%APPDATA%\voice-control\models`), else Scrawl's own copy of Hex's Parakeet Unified English
//! (`%LOCALAPPDATA%\Scrawl\models`), which `install` downloads once in the background and checks
//! against its SHA-256. Like the Hex client, it hears the whole recording in chunks of up to 30 s
//! (`speech_plan::Plan`); unlike it, every word keeps the time the model heard it
//! (`estimated: false`), so spoken cues ("look here") can cut keyframes.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use transcribe_cpp::{Model, RunOptions, TimestampKind};

use crate::helper::SpeechModelLine;
use crate::model::{TranscriptData, TranscriptSegment, WordTiming};
use crate::speech_plan::{self, Heard, Plan};

/// The model Scrawl downloads when Hex hasn't installed one: the same file, revision and
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
    crate::hex::support_dir().join("models")
}

/// Where Scrawl keeps the model it downloaded (`SCRAWL_MODELS_DIR` for tests).
pub fn own_models_dir() -> PathBuf {
    std::env::var_os("SCRAWL_MODELS_DIR")
        .map(PathBuf::from)
        .filter(|dir| !dir.as_os_str().is_empty())
        .unwrap_or_else(|| crate::win::local_dir().join("models"))
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
/// then the one Scrawl downloaded.
pub fn installed_models() -> Vec<PathBuf> {
    let mut models = hex_models();
    models.extend(verified_in(&own_models_dir()));
    models
}

fn hex_models() -> Vec<PathBuf> {
    let chosen = std::fs::read(crate::hex::support_dir().join("settings.json"))
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
        let status = crate::speech_model::current();
        let why = match status.state {
            crate::speech_model::State::Failed => "failed",
            _ => "downloading",
        };
        return Ok(TranscriptData {
            source: "offline".into(),
            missing: Some(crate::speech_model::missing(
                why,
                "en",
                status.error.as_deref(),
            )),
            ..Default::default()
        });
    };
    let wav =
        std::fs::read(audio).with_context(|| format!("Could not read {}", audio.display()))?;
    let samples = crate::audio::decode_wav_16k(&wav)?;
    let plan = Plan::new(&samples);
    let name = model_name(&path);
    let source = format!("parakeet:{name}");
    if plan.chunks().is_empty() {
        return Ok(TranscriptData {
            source,
            model: Some(name),
            ..Default::default()
        });
    }

    init_backends()?;
    let model = Model::load(&path).with_context(|| format!("Could not load {}", path.display()))?;
    let mut session = model.session()?;
    let options = RunOptions {
        timestamps: TimestampKind::Word,
        language: Some("en".into()),
        ..Default::default()
    };
    speech_plan::transcribe(&samples, &plan, source, Some(name), |chunk| {
        let result = session.run(chunk, &options)?;
        let ms = |ms: i64| ms.max(0) as u64;
        Ok(Heard::Timed {
            words: result
                .words
                .iter()
                .filter(|word| !word.text.trim().is_empty())
                .map(|word| WordTiming {
                    t0: ms(word.t0_ms),
                    t1: ms(word.t1_ms.max(word.t0_ms)),
                    text: word.text.trim().to_owned(),
                })
                .collect(),
            segments: result
                .segments
                .iter()
                .filter(|s| !s.text.trim().is_empty())
                .map(|s| TranscriptSegment {
                    t0: ms(s.t0_ms),
                    t1: ms(s.t1_ms.max(s.t0_ms)),
                    text: s.text.trim().to_owned(),
                })
                .collect(),
            text: result.text,
        })
    })
}

/// Loads ggml's CPU backend DLL for this CPU (beside `transcribe.dll`), once per process: the
/// registry it fills isn't safe to change while another thread loads a model.
fn init_backends() -> Result<()> {
    static INIT: std::sync::OnceLock<std::result::Result<(), String>> = std::sync::OnceLock::new();
    INIT.get_or_init(|| {
        transcribe_cpp::disable_logging();
        transcribe_cpp::init_backends_default().map_err(|error| format!("{error:#}"))
    })
    .clone()
    .map_err(|error| anyhow::anyhow!("transcribe.cpp backends: {error}"))
}

/// Makes sure a model is on disk: `installed` when Hex's or Scrawl's own is there, else downloads
/// `PARAKEET` into `own_models_dir()`, reporting progress, and keeps it only if its SHA-256
/// matches. The same lines as macOS's `scrawl-helper speech-model --install`.
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

/// Windows' own `curl.exe` (in System32 since Windows 10 1803), without a console window.
fn fetch(url: &str, partial: &Path, total: u64, progress: &mut dyn FnMut(f64)) -> Result<()> {
    use std::io::Read;
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let curl = std::env::var_os("SystemRoot")
        .map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from)
        .join(r"System32\curl.exe");
    let mut child = std::process::Command::new(&curl)
        .args([
            "--fail",
            "--location",
            "--silent",
            "--show-error",
            "--retry",
            "3",
            "--continue-at",
            "-",
            "--output",
        ])
        .arg(partial)
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .with_context(|| format!("Could not run {}", curl.display()))?;
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
            bail!("the download failed ({status}): {}", error.trim());
        }
        let have = std::fs::metadata(partial).map_or(0, |m| m.len());
        progress((have as f64 / total as f64).min(0.99));
        std::thread::sleep(Duration::from_millis(500));
    }
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
        let dir = std::env::temp_dir().join(format!("scrawl-speech-{name}-{}", std::process::id()));
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

    /// Downloads the real model (731 MB), loads it and runs it on two seconds of sound. CI's
    /// release job runs it, then again under Intel SDE as a CPU without AVX-512 (#58), with
    /// `SCRAWL_TEST_MODEL_DIR` keeping the download between the two.
    #[test]
    #[ignore]
    fn downloads_and_runs_parakeet() {
        let kept = std::env::var_os("SCRAWL_TEST_MODEL_DIR").map(PathBuf::from);
        let dir = kept.clone().unwrap_or_else(|| temp_dir("parakeet"));
        let fresh = !dir.join(PARAKEET.filename).is_file();
        let mut last = 0.0;
        let model = download(&PARAKEET, &dir, &mut |done| last = done).unwrap();
        if fresh {
            assert_eq!(last, 1.0);
        }
        if std::env::var_os("SCRAWL_TEST_NO_AVX512").is_some() {
            // Under SDE: proves the emulated CPU is the one without AVX-512.
            assert!(!std::arch::is_x86_feature_detected!("avx512f"));
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
