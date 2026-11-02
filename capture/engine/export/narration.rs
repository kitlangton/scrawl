//! Explicit, offline export narration. Cue times use the exported video's clock.
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::{
    fs,
    io::Read,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

const RATE: u32 = 24_000;
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Script {
    version: u32,
    cues: Vec<Cue>,
}
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Cue {
    start_ms: u64,
    end_ms: u64,
    text: String,
}

fn parse(bytes: &[u8], duration: u64) -> Result<Script> {
    ensure!(
        bytes.len() <= 1_048_576,
        "[NARRATION_SCRIPT] Script exceeds 1 MiB"
    );
    let script: Script = serde_json::from_slice(bytes)
        .context("[NARRATION_SCRIPT] Expected version:1 and cues:[{startMs,endMs,text}]")?;
    ensure!(
        script.version == 1,
        "[NARRATION_SCRIPT] Unsupported version (expected 1)"
    );
    ensure!(
        duration > 0 && duration <= 3_600_000,
        "[NARRATION_SCRIPT] Narration supports videos up to one hour"
    );
    ensure!(
        !script.cues.is_empty() && script.cues.len() <= 256,
        "[NARRATION_SCRIPT] Expected 1..256 cues"
    );
    let mut previous = 0;
    let mut total = 0;
    for (index, cue) in script.cues.iter().enumerate() {
        ensure!(
            cue.start_ms >= previous && cue.end_ms > cue.start_ms && cue.end_ms <= duration,
            "[NARRATION_SCRIPT] Cue {} must be ordered, non-overlapping and inside the video",
            index + 1
        );
        ensure!(
            !cue.text.trim().is_empty()
                && cue.text.chars().count() <= 1000
                && !cue
                    .text
                    .chars()
                    .any(|c| c.is_control() && !c.is_whitespace()),
            "[NARRATION_SCRIPT] Cue {} needs 1..1000 plain-text characters",
            index + 1
        );
        total += cue.text.len();
        ensure!(
            total <= 32_000,
            "[NARRATION_SCRIPT] Total text exceeds 32000 UTF-8 bytes"
        );
        previous = cue.end_ms;
    }
    Ok(script)
}

pub(super) struct Rendered {
    directory: PathBuf,
}
impl Rendered {
    pub(super) fn path(&self) -> PathBuf {
        self.directory.join("narration.wav")
    }
}
impl Drop for Rendered {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.directory);
    }
}

fn read_script(path: &Path, duration: u64) -> Result<Script> {
    // O_NONBLOCK avoids waiting for a FIFO writer. Check and read the same handle.
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .with_context(|| format!("[NARRATION_SCRIPT] Read {}", path.display()))?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file(),
        "[NARRATION_SCRIPT] Script must be a regular file"
    );
    ensure!(
        metadata.len() <= 1_048_576,
        "[NARRATION_SCRIPT] Script exceeds 1 MiB"
    );
    let mut bytes = Vec::new();
    file.take(1_048_577)
        .read_to_end(&mut bytes)
        .context("[NARRATION_SCRIPT] Could not read script")?;
    parse(&bytes, duration)
}

pub(super) fn render(path: &Path, duration: u64) -> Result<Rendered> {
    ensure!(
        crate::shared::handoff::experiment("exportNarration"),
        "[NARRATION_DISABLED] Local export narration is disabled by experiments.exportNarration=false in ~/.config/apparent/config.json"
    );
    let script = read_script(path, duration)?;
    let cache = dirs::home_dir()
        .context("Home directory unavailable")?
        .join(".local/share/apparent/narrator");
    synthesize(&script, path, duration, &cache)
}

struct Worker(std::process::Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn synthesize(script: &Script, path: &Path, duration: u64, cache: &Path) -> Result<Rendered> {
    let worker = cache.join("apparent-narrator");
    ensure!(
        worker.is_file()
            && cache.join("model.onnx").is_file()
            && cache.join("voices/bm_fable.bin").is_file(),
        "[NARRATION_SETUP] Local narrator is not provisioned. Run python3 capture/agent/scripts/provision-narrator.py from the Apparent source checkout. Export never downloads models or falls back to recorded audio."
    );
    // An exclusive private directory outside the bundle; all cue files disappear on failure too.
    let directory =
        std::env::temp_dir().join(format!("apparent-narration-{}", crate::shared::ids::make_bundle_id()));
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new().mode(0o700).create(&directory)?;
    }
    let rendered = Rendered { directory };
    let mut pcm = vec![0i16; (duration * u64::from(RATE)).div_ceil(1000) as usize];
    for (index, cue) in script.cues.iter().enumerate() {
        let text = rendered.directory.join("cue.txt");
        let wav = rendered.directory.join("cue.wav");
        fs::write(&text, &cue.text)?;
        let mut child = Worker(Command::new(&worker)
            .arg(cache)
            .arg(&text)
            .arg(&wav)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context(
                "[NARRATION_SETUP] Could not start local narrator; reprovision for this platform",
            )?);
        let started = Instant::now();
        loop {
            if let Some(status) = child
                .0
                .try_wait()
                .context("[NARRATION_SYNTHESIS] Could not wait for worker")?
            {
                ensure!(
                    status.success(),
                    "[NARRATION_SYNTHESIS] Cue {} failed ({status}); no export written. Verify/reprovision the pinned narrator assets.",
                    index + 1
                );
                break;
            }
            if started.elapsed() > Duration::from_secs(120) {
                anyhow::bail!(
                    "[NARRATION_SYNTHESIS] Cue {} exceeded 120 seconds; no export written",
                    index + 1
                );
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        ensure!(
            fs::metadata(&wav)?.len() <= 24_000 * 2 * 120 + 4096,
            "[NARRATION_SYNTHESIS] Cue output exceeds limit"
        );
        let mut reader = hound::WavReader::open(&wav)?;
        let spec = reader.spec();
        ensure!(
            spec.channels == 1
                && spec.sample_rate == RATE
                && spec.bits_per_sample == 16
                && spec.sample_format == hound::SampleFormat::Int,
            "[NARRATION_SYNTHESIS] Worker must return mono 24kHz PCM16"
        );
        let samples = reader
            .samples::<i16>()
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ensure!(
            !samples.is_empty(),
            "[NARRATION_SYNTHESIS] Cue {} returned an empty WAV",
            index + 1
        );
        let start = (cue.start_ms * u64::from(RATE) / 1000) as usize;
        let available = ((cue.end_ms - cue.start_ms) * u64::from(RATE) / 1000) as usize;
        ensure!(
            samples.len() <= available,
            "[NARRATION_TIMING] Cue {} needs {} ms but its slot is {} ms. Shorten the text or extend endMs; speech is never clipped or sped up.",
            index + 1,
            (samples.len() as u64 * 1000).div_ceil(u64::from(RATE)),
            cue.end_ms - cue.start_ms
        );
        pcm[start..start + samples.len()].copy_from_slice(&samples);
        fs::remove_file(wav)?;
    }
    let mut writer = hound::WavWriter::create(
        rendered.path(),
        hound::WavSpec {
            channels: 1,
            sample_rate: RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        },
    )?;
    for sample in pcm {
        writer.write_sample(sample)?;
    }
    writer.finalize()?;
    eprintln!(
        "Narration: local Kokoro 82M int8, bm_fable, speed 1, CPU/3 threads; 9.6kHz notch; script {} ({} cues). Recorded audio replaced only in export; source bundle unchanged.",
        path.display(),
        script.cues.len()
    );
    Ok(rendered)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn script_reader_rejects_special_files_and_bounds_regular_reads() {
        if let Some(path) = std::env::var_os("APPARENT_TEST_SCRIPT_PATH") {
            assert!(read_script(Path::new(&path), 1000).is_err());
            return;
        }
        // Keep a blocking-open regression from hanging the full Cargo suite.
        let rejects_without_blocking = |path: &Path| {
            let mut child = Worker(Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "export::narration::tests::script_reader_rejects_special_files_and_bounds_regular_reads"])
                .env("APPARENT_TEST_SCRIPT_PATH", path)
                .stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap());
            let started = Instant::now();
            loop {
                if let Some(status) = child.0.try_wait().unwrap() {
                    assert!(status.success());
                    break;
                }
                assert!(
                    started.elapsed() < Duration::from_secs(3),
                    "script reader blocked"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        };
        let root = std::env::temp_dir().join(format!("narrator-input-{}", crate::shared::ids::make_bundle_id()));
        fs::create_dir(&root).unwrap();
        let fifo = root.join("script.fifo");
        assert!(
            Command::new("mkfifo")
                .arg(&fifo)
                .status()
                .unwrap()
                .success()
        );
        assert!(!fs::metadata(&fifo).unwrap().is_file());
        let ordinary = root.join("script.json");
        fs::write(&ordinary, vec![b' '; 1_048_577]).unwrap();
        assert!(read_script(&ordinary, 1000).is_err());
        rejects_without_blocking(&fifo);
        rejects_without_blocking(Path::new("/dev/zero"));
        fs::write(
            &ordinary,
            br#"{"version":1,"cues":[{"startMs":0,"endMs":100,"text":"Hi"}]}"#,
        )
        .unwrap();
        assert!(read_script(&ordinary, 1000).is_ok());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn timed_renderer_places_samples_without_modifying_inputs_and_rejects_overrun() {
        use std::os::unix::fs::PermissionsExt;
        let root = std::env::temp_dir().join(format!("narrator-test-{}", crate::shared::ids::make_bundle_id()));
        fs::create_dir_all(root.join("voices")).unwrap();
        fs::write(
            root.join("model.onnx"),
            "test adapter fixture, not an inference model",
        )
        .unwrap();
        fs::write(root.join("voices/bm_fable.bin"), "test voice fixture").unwrap();
        let fixture = root.join("fixture.wav");
        let mut writer = hound::WavWriter::create(
            &fixture,
            hound::WavSpec {
                channels: 1,
                sample_rate: RATE,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for _ in 0..2400 {
            writer.write_sample(1000i16).unwrap();
        }
        writer.finalize().unwrap();
        // Exercise orchestration deterministically; real Kokoro/mux proof is separate.
        let worker = root.join("apparent-narrator");
        fs::write(&worker, "#!/bin/sh\ncp \"$1/fixture.wav\" \"$3\"\n").unwrap();
        fs::set_permissions(&worker, fs::Permissions::from_mode(0o700)).unwrap();
        let input = br#"{"version":1,"cues":[{"startMs":200,"endMs":500,"text":"Fixture"},{"startMs":600,"endMs":900,"text":"Second"}]}"#;
        let path = root.join("script.json");
        fs::write(&path, input).unwrap();
        let script = parse(input, 1000).unwrap();
        let rendered = synthesize(&script, &path, 1000, &root).unwrap();
        let samples = hound::WavReader::open(rendered.path())
            .unwrap()
            .samples::<i16>()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(samples.len(), 24_000);
        assert!(samples[..4800].iter().all(|&s| s == 0));
        assert!(samples[4800..7200].iter().all(|&s| s == 1000));
        assert!(samples[7200..14400].iter().all(|&s| s == 0));
        assert!(samples[14400..16800].iter().all(|&s| s == 1000));
        assert!(samples[16800..].iter().all(|&s| s == 0));
        let temporary = rendered.directory.clone();
        drop(rendered);
        assert!(!temporary.exists());
        assert_eq!(fs::read(&path).unwrap(), input);
        let overrun = parse(
            br#"{"version":1,"cues":[{"startMs":0,"endMs":50,"text":"Too short"}]}"#,
            1000,
        )
        .unwrap();
        assert!(
            format!(
                "{:#}",
                synthesize(&overrun, &path, 1000, &root).err().unwrap()
            )
            .contains("[NARRATION_TIMING]")
        );
        fs::write(&worker, "#!/bin/sh\nexit 7\n").unwrap();
        assert!(
            format!(
                "{:#}",
                synthesize(&script, &path, 1000, &root).err().unwrap()
            )
            .contains("[NARRATION_SYNTHESIS]")
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn schema_rejects_ambiguous_or_unsafe_timing() {
        let good = br#"{"version":1,"cues":[{"startMs":10,"endMs":900,"text":"Hello."}]}"#;
        assert!(parse(good, 1000).is_ok());
        for bad in [
            r#"{"version":2,"cues":[]}"#,
            r#"{"version":1,"cues":[]}"#,
            r#"{"version":1,"cues":[{"startMs":0,"endMs":1200,"text":"Hi"}]}"#,
            r#"{"version":1,"cues":[{"startMs":0,"endMs":10,"text":" "}]}"#,
            r#"{"version":1,"cues":[{"startMs":0,"endMs":10,"text":"Hi","voice":"other"}]}"#,
            r#"{"version":1,"cues":[{"startMs":0,"endMs":20,"text":"Hi"},{"startMs":10,"endMs":30,"text":"Hi"}]}"#,
            r#"{"version":1,"cues":[{"startMs":-1,"endMs":10,"text":"Hi"}]}"#,
            "not JSON",
        ] {
            assert!(parse(bad.as_bytes(), 1000).is_err(), "{bad}");
        }
    }
}
