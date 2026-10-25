//! `scrawl-helper`, the Swift executable shipped beside `scrawl` (see `helper/main.swift`).
//!
//! It owns the Apple frameworks that are Swift-only or absent from the objc2 bindings Scrawl
//! uses: SpeechAnalyzer transcription, CoreMediaIO/AVFoundation iPhone capture with a live
//! mirror window, and AVFoundation movie decoding.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;

use crate::model::{RgbaFrame, TranscriptData, TranscriptSegment, WordTiming};

pub fn path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("SCRAWL_HELPER") {
        return Ok(PathBuf::from(path));
    }
    let raw_exe = std::env::current_exe().context("Could not locate scrawl")?;
    let exe = std::fs::canonicalize(&raw_exe).unwrap_or(raw_exe);
    // Beside the app binary, or one level up from `target/*/deps` for tests.
    exe.ancestors()
        .skip(1)
        .take(2)
        .map(|dir| dir.join("scrawl-helper"))
        .find(|path| path.is_file())
        .context("scrawl-helper is missing; rebuild Scrawl")
}

pub fn ui_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("SCRAWL_UI_HELPER") {
        return Ok(PathBuf::from(path));
    }
    if let Some(helper) = std::env::var_os("SCRAWL_HELPER")
        && let Some(parent) = Path::new(&helper).parent()
    {
        let sibling = parent.join("scrawl-ui-helper");
        if sibling.is_file() {
            return Ok(sibling);
        }
    }
    let raw_exe = std::env::current_exe().context("Could not locate scrawl")?;
    let exe = std::fs::canonicalize(&raw_exe).unwrap_or(raw_exe);
    exe.ancestors()
        .skip(1)
        .take(2)
        .map(|dir| dir.join("scrawl-ui-helper"))
        .find(|path| path.is_file())
        .context("scrawl-ui-helper is missing; rebuild Scrawl")
}

/// Runs a helper command to completion and parses its JSON output.
fn run<T: for<'de> Deserialize<'de>>(args: &[&std::ffi::OsStr], timeout: Duration) -> Result<T> {
    run_program(&path()?, args, timeout)
}

fn run_program<T: for<'de> Deserialize<'de>>(
    program: &Path,
    args: &[&std::ffi::OsStr],
    timeout: Duration,
) -> Result<T> {
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Could not start scrawl-helper")?;
    let mut stdout = child.stdout.take().context("No helper stdout")?;
    let mut stderr = child.stderr.take().context("No helper stderr")?;
    let caller = std::thread::current();
    let eof = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let reader_eof = std::sync::Arc::clone(&eof);
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.read_to_end(&mut out);
        reader_eof.store(true, std::sync::atomic::Ordering::Release);
        caller.unpark();
        out
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if eof.load(std::sync::atomic::Ordering::Acquire) {
            break child.wait()?;
        }
        let now = Instant::now();
        if now >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            bail!("scrawl-helper {:?} timed out", args.first());
        }
        std::thread::park_timeout(deadline - now);
    };
    let out = reader.join().unwrap_or_default();
    if !status.success() {
        let mut message = String::new();
        let _ = stderr.read_to_string(&mut message);
        bail!("{}", message.trim());
    }
    serde_json::from_slice(&out).context("Invalid scrawl-helper output")
}

fn os(s: &str) -> &std::ffi::OsStr {
    std::ffi::OsStr::new(s)
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct PhoneDevice {
    pub id: String,
    pub name: String,
    pub model: String,
    pub width: u32,
    pub height: u32,
}

/// What macOS has granted Scrawl, from a fresh helper process (a long-running one can miss a
/// Screen Recording grant).
pub fn permissions()
-> Result<std::collections::BTreeMap<crate::permissions::Permission, crate::permissions::Status>> {
    run(&[os("permissions")], Duration::from_secs(5))
}

/// Cabled iPhones and iPads, from the USB registry (no camera permission needed). `id` is the
/// USB serial and the size is unknown until the mirror attaches.
pub fn devices() -> Result<Vec<PhoneDevice>> {
    run(&[os("devices")], Duration::from_secs(5))
}

#[derive(Deserialize)]
struct Transcript {
    #[serde(default)]
    engine: Option<String>,
    locale: String,
    text: String,
    segments: Vec<TranscriptSegment>,
    words: Vec<WordTiming>,
    /// No usable speech model: `downloading`, `unsupported` or `failed` (with `error`); on macOS
    /// 15 also `permission` (Speech Recognition is off) or `dictation` (no Dictation model).
    #[serde(default)]
    missing: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

/// On-device transcription with measured word timings: SpeechAnalyzer on macOS 26,
/// SFSpeechRecognizer on macOS 15. Without a speech model the transcript is empty and `missing`
/// says why.
pub fn transcribe(audio: &Path) -> Result<TranscriptData> {
    let locale = crate::speech_model::locale_args();
    let mut args = vec![os("transcribe"), audio.as_os_str()];
    args.extend(locale.iter().map(|arg| arg.as_os_str()));
    let transcript: Transcript = run(&args, Duration::from_secs(180))?;
    let engine = transcript
        .engine
        .unwrap_or_else(|| "SpeechTranscriber".into());
    let missing = transcript.missing.map(|why| {
        crate::speech_model::missing(&why, &transcript.locale, transcript.error.as_deref())
    });
    Ok(TranscriptData {
        source: if missing.is_some() {
            "offline".into()
        } else if engine == "SFSpeechRecognizer" {
            "sf-speech-recognizer".into()
        } else {
            "speech-analyzer".into()
        },
        model: Some(format!("{engine} {}", transcript.locale)),
        text: transcript.text,
        segments: transcript.segments,
        words: transcript.words,
        estimated: false,
        missing,
    })
}

/// One line of `scrawl-helper speech-model [--install]`.
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

/// Runs `scrawl-helper speech-model --install` for the chosen language, handing each line to
/// `line` as it comes. The download can take minutes; the helper exits when it's done or has
/// failed, and is killed as soon as `stop` says a newer check superseded it.
pub fn install_speech_model(
    stop: &dyn Fn() -> bool,
    line: impl FnMut(SpeechModelLine),
) -> Result<()> {
    let mut command = Command::new(path()?);
    command.args(["speech-model", "--install"]);
    command.args(crate::speech_model::locale_args());
    install_with(command, stop, line)
}

fn install_with(
    mut command: Command,
    stop: &dyn Fn() -> bool,
    mut line: impl FnMut(SpeechModelLine),
) -> Result<()> {
    use std::io::BufRead;
    use std::sync::mpsc::RecvTimeoutError;

    if stop() {
        return Ok(());
    }
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Could not start scrawl-helper")?;
    let stdout = child.stdout.take().context("No helper stdout")?;
    // Lines come through a thread so `stop` is checked while the helper is quiet.
    let (sender, lines) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for text in std::io::BufReader::new(stdout)
            .lines()
            .map_while(Result::ok)
        {
            if sender.send(text).is_err() {
                break;
            }
        }
    });
    loop {
        if stop() {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(());
        }
        match lines.recv_timeout(Duration::from_millis(250)) {
            Ok(text) => {
                if let Ok(parsed) = serde_json::from_str(&text) {
                    line(parsed);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    let status = child.wait()?;
    if !status.success() {
        let mut message = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let _ = stderr.read_to_string(&mut message);
        }
        bail!("{}", message.trim());
    }
    Ok(())
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
    run(&[os("probe"), movie.as_os_str()], Duration::from_secs(30))
}

/// Re-encodes `movie` into `out` as HEVC with its long edge at most `max_edge`, audio copied and
/// every sample at its original time. Runs at utility priority; minutes of video take seconds.
pub fn shrink(movie: &Path, out: &Path, max_edge: u32) -> Result<crate::retention::Shrunk> {
    let edge = max_edge.to_string();
    let helper = path()?;
    let args = [
        helper.as_os_str(),
        os("shrink"),
        movie.as_os_str(),
        os("--out"),
        out.as_os_str(),
        os("--max-edge"),
        os(&edge),
    ];
    run_program(
        Path::new("/usr/sbin/taskpolicy"),
        &[&[os("-c"), os("utility")], &args[..]].concat(),
        Duration::from_secs(1800),
    )
}

/// Mixes a movie's audio to 16 kHz mono WAV, aligned to the movie's start.
pub fn export_audio(movie: &Path, wav: &Path) -> Result<()> {
    let _: serde_json::Value = run(
        &[os("audio"), movie.as_os_str(), wav.as_os_str()],
        Duration::from_secs(120),
    )?;
    Ok(())
}

#[derive(Deserialize)]
struct FrameFile {
    t: u64,
    actual: u64,
    width: u32,
    height: u32,
    file: PathBuf,
}

pub enum FrameTimes<'a> {
    /// Nearest decodable frame every N ms, stamped with its actual time.
    Every(u64),
    /// Exactly these times.
    At(&'a [u64]),
}

/// Decodes movie frames into memory; `max_edge` 0 keeps native size.
pub fn frames(movie: &Path, times: FrameTimes, max_edge: u32) -> Result<Vec<RgbaFrame>> {
    let dir = std::env::temp_dir().join(format!(
        "scrawl-frames-{}",
        crate::pipeline::make_bundle_id()
    ));
    let (flag, value, sampled) = match times {
        FrameTimes::Every(ms) => ("--every", ms.max(1).to_string(), true),
        FrameTimes::At(times) => {
            if times.is_empty() {
                return Ok(Vec::new());
            }
            let list = times.iter().map(u64::to_string).collect::<Vec<_>>();
            ("--at", list.join(","), false)
        }
    };
    let edge = max_edge.to_string();
    let result: Result<Vec<FrameFile>> = run(
        &[
            os("frames"),
            movie.as_os_str(),
            os("--out-dir"),
            dir.as_os_str(),
            os(flag),
            os(&value),
            os("--max-edge"),
            os(&edge),
        ],
        Duration::from_secs(300),
    );
    let frames = result.and_then(|files| {
        if files.len() <= 1 {
            files
                .into_iter()
                .map(|file| {
                    let rgba = std::fs::read(&file.file)?;
                    ensure!(
                        rgba.len() == file.width as usize * file.height as usize * 4,
                        "Truncated helper frame"
                    );
                    let t = if sampled { file.actual } else { file.t };
                    Ok(RgbaFrame::new(t, file.width, file.height, rgba))
                })
                .collect()
        } else {
            let chunk_size = files.len().div_ceil(6).max(1);
            std::thread::scope(|scope| -> Result<Vec<RgbaFrame>> {
                let workers: Vec<_> = files
                    .chunks(chunk_size)
                    .map(|chunk| {
                        scope.spawn(move || -> Result<Vec<RgbaFrame>> {
                            chunk
                                .iter()
                                .map(|file| {
                                    let rgba = std::fs::read(&file.file)?;
                                    ensure!(
                                        rgba.len()
                                            == file.width as usize * file.height as usize * 4,
                                        "Truncated helper frame"
                                    );
                                    let t = if sampled { file.actual } else { file.t };
                                    Ok(RgbaFrame::new(t, file.width, file.height, rgba))
                                })
                                .collect()
                        })
                    })
                    .collect();
                let mut out = Vec::with_capacity(files.len());
                for worker in workers {
                    out.extend(
                        worker
                            .join()
                            .map_err(|_| anyhow::anyhow!("Frame read worker panicked"))??,
                    );
                }
                Ok(out)
            })
        }
    });
    std::thread::spawn(move || {
        let _ = std::fs::remove_dir_all(dir);
    });
    frames
}

#[derive(Deserialize)]
struct QrOutput {
    qr: String,
}

/// Renders `text` as a compact Unicode half-block QR code for the terminal via CoreImage.
pub fn qr(text: &str) -> Result<String> {
    let out: QrOutput = run(&[os("qr"), os(text)], Duration::from_secs(5))?;
    Ok(out.qr)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_superseded_install_stops_without_waiting_for_the_helper() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            r#"echo '{"status":"downloading","progress":0.1}'; sleep 60"#,
        ]);
        let stop = AtomicBool::new(false);
        let started = Instant::now();
        let mut lines = Vec::new();
        install_with(command, &|| stop.load(Ordering::Relaxed), |l| {
            lines.push(l);
            stop.store(true, Ordering::Relaxed);
        })
        .unwrap();
        assert!(started.elapsed() < Duration::from_secs(5));
        assert_eq!(lines[0].status, "downloading");
    }
}
