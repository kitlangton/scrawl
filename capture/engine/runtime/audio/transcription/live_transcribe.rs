//! Transcribing while recording (`"experiments": {"liveTranscribe": true}`, Apple Speech only).
//!
//! A long-lived `apparent-helper transcribe-stream` gets the microphone's 16 kHz samples, quantized
//! exactly as `audio.wav` will store them, about every 160 ms, and keeps SpeechAnalyzer's
//! finalized results. At finish the samples it hasn't had are sent from the finished WAV and only
//! that tail is left to feed. Apple chooses natural finality, possibly at EOF; word times use
//! the WAV's source clock, not a guarantee of batch lexical/alignment identity. Anything wrong (the
//! helper fails, the model isn't installed, it lost samples or took too long) returns an error and
//! the caller transcribes the WAV as before.

use std::io::{BufRead, BufReader, Read, Write};
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, TryLockError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail, ensure};
use serde::Deserialize;

use crate::capture::engine::runtime::audio::persistence::Samples;
use crate::shared::model::{TranscriptData, TranscriptSegment, WordTiming};

// Cap the entire helper output retained until Finish, including malformed lines.
const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_LINE_BYTES: usize = 64 * 1024;
const MAX_EVENTS: usize = 16_384;

const FEED_EVERY: Duration = Duration::from_millis(160);
/// Deadline for helper tail writes, output drain and exit, measured from Finish entry.
/// Joining a spawning feeder and decoding the saved WAV are synchronous and cannot
/// be interrupted by this deadline; this is not a total Finish latency guarantee.
const TAIL_TIMEOUT: Duration = Duration::from_secs(6);

/// Whether recordings should start a live transcription now. It streams to `apparent-helper`'s
/// SpeechAnalyzer, so only on macOS (Linux transcribes after the recording).
pub fn wanted() -> bool {
    cfg!(target_os = "macos")
        && crate::shared::handoff::experiment(crate::shared::handoff::LIVE_TRANSCRIBE)
        && crate::capture::engine::runtime::audio::transcription::transcribe::Engine::preferred()
            == crate::capture::engine::runtime::audio::transcription::transcribe::Engine::Apple
}

pub struct Live {
    stop: Arc<AtomicBool>,
    feeder: Option<JoinHandle<Result<Feeding>>>,
}

struct Feeding {
    child: Child,
    stdin: Option<ChildStdin>,
    fed: usize,
    lines: Receiver<Line>,
    group_cleaned: bool,
}

impl Feeding {
    fn cleanup_group(&mut self) {
        if self.group_cleaned {
            return;
        }
        #[cfg(unix)]
        // SAFETY: the owned leader has not been reaped, so its PID cannot be reused.
        unsafe {
            libc::killpg(self.child.id() as libc::pid_t, libc::SIGKILL);
        }
        self.group_cleaned = true;
    }

    fn exited_without_reaping(&self) -> Result<bool> {
        #[cfg(unix)]
        {
            // SAFETY: waitid initializes this output; WNOWAIT preserves the owned
            // child identity until cleanup_group kills any surviving descendants.
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            let result = unsafe {
                libc::waitid(
                    libc::P_PID,
                    self.child.id(),
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            };
            if result != 0 {
                return Err(std::io::Error::last_os_error())
                    .context("could not observe live helper exit");
            }
            Ok(unsafe { info.si_pid() } != 0)
        }
        #[cfg(not(unix))]
        bail!("live transcription requires Unix process cleanup")
    }
}

impl Drop for Feeding {
    fn drop(&mut self) {
        self.cleanup_group();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Event {
    Done {
        done: bool,
        samples: usize,
    },
    Ready {
        ready: bool,
        locale: String,
        // Older helpers only streamed SpeechTranscriber and did not name it.
        #[serde(default = "default_engine")]
        engine: String,
    },
    Result {
        segment: TranscriptSegment,
        #[serde(default)]
        words: Vec<WordTiming>,
    },
}

enum Line {
    Event(Event),
    Invalid,
    /// stdout closed; the helper's stderr.
    Closed(String),
}

fn default_engine() -> String {
    "SpeechTranscriber".into()
}

/// What finishing took, for the log.
#[derive(Debug)]
pub struct Finished {
    pub transcript: TranscriptData,
    pub tail_ms: u64,
    pub streamed_samples: usize,
}

impl Live {
    /// Starts streaming `samples` (the recorder's rolling 16 kHz history) to a new helper. Nothing here blocks: the helper starts on the feeder
    /// thread.
    pub fn start(samples: Arc<Mutex<Samples>>) -> Result<Self> {
        let mut command = Command::new(crate::capture::desktop::helper::path()?);
        command.arg("transcribe-stream");
        command.args(
            crate::capture::engine::runtime::audio::transcription::speech_model::locale_args(),
        );
        Self::start_with(command, samples)
    }

    fn start_with(mut command: Command, samples: Arc<Mutex<Samples>>) -> Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let feeder_stop = Arc::clone(&stop);
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        command.process_group(0);
        let feeder = std::thread::Builder::new()
            .name("apparent-live-transcribe".into())
            .spawn(move || feed(command, &samples, &feeder_stop))
            .context("Could not spawn the live transcription thread")?;
        Ok(Self {
            stop,
            feeder: Some(feeder),
        })
    }

    /// Sends what the helper hasn't had of `wav_bytes` (the recording's `audio.wav`), ends its
    /// input and collects the transcript.
    pub fn finish(mut self, wav_bytes: &[u8]) -> Result<Finished> {
        self.finish_within(wav_bytes, TAIL_TIMEOUT)
    }

    fn finish_within(&mut self, wav_bytes: &[u8], timeout: Duration) -> Result<Finished> {
        let finishing = Instant::now();
        let deadline = finishing + timeout;
        let mut feeding = self.join_feeder()?;
        let samples = crate::capture::engine::runtime::audio::audio::decode_wav_pcm16(wav_bytes)?;
        ensure!(
            feeding.fed <= samples.len(),
            "streamed {} samples but the recording has {}",
            feeding.fed,
            samples.len()
        );
        let streamed = feeding.fed;
        {
            let mut stdin = feeding.stdin.take().context("No helper stdin")?;
            for chunk in samples[feeding.fed..].chunks(16_000) {
                write_pcm(&mut stdin, &le_bytes(chunk), || {
                    ensure!(
                        Instant::now() < deadline,
                        "the live transcriber took too long to finish"
                    );
                    Ok(false)
                })?;
            }
        }
        let mut locale = None;
        let mut engine = None;
        let mut segments = Vec::new();
        let mut words = Vec::new();
        let mut done = false;
        loop {
            ensure!(
                Instant::now() < deadline,
                "the live transcriber took too long to finish"
            );
            let left = deadline.saturating_duration_since(Instant::now());
            match feeding.lines.recv_timeout(left) {
                Ok(Line::Event(Event::Ready {
                    ready: true,
                    locale: l,
                    engine: e,
                })) if !done
                    && locale.is_none()
                    && !l.is_empty()
                    && matches!(e.as_str(), "SpeechTranscriber" | "DictationTranscriber") =>
                {
                    locale = Some(l);
                    engine = Some(e);
                }
                Ok(Line::Event(Event::Result {
                    segment,
                    words: more,
                })) if !done && locale.is_some() => {
                    segments.push(segment);
                    words.extend(more);
                }
                Ok(Line::Event(Event::Done {
                    done: true,
                    samples: heard,
                })) if !done && locale.is_some() => {
                    ensure!(
                        heard == samples.len(),
                        "the live transcriber heard {heard} samples of {}",
                        samples.len()
                    );
                    done = true;
                }
                Ok(Line::Closed(_)) if done => break,
                Ok(Line::Invalid | Line::Event(_)) => bail!("invalid live transcription protocol"),
                Ok(Line::Closed(stderr)) => bail!(
                    "the live transcriber stopped: {}",
                    stderr.trim().trim_start_matches("apparent-helper: ")
                ),
                Err(RecvTimeoutError::Timeout) => {
                    bail!(
                        "the live transcriber took over {} s to finish",
                        timeout.as_secs()
                    )
                }
                Err(RecvTimeoutError::Disconnected) => bail!("the live transcriber vanished"),
            }
        }
        loop {
            ensure!(
                Instant::now() < deadline,
                "the live transcriber took too long to exit"
            );
            if feeding.exited_without_reaping()? {
                feeding.cleanup_group();
                let status = feeding.child.wait()?;
                ensure!(
                    status.success(),
                    "the live transcriber exited unsuccessfully"
                );
                break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        let locale = locale.context("the live transcriber never said it was ready")?;
        let duration_ms = (samples.len() as u64 * 1000).div_ceil(16_000);
        ensure!(
            segments.iter().all(|s| s.t0 <= s.t1 && s.t1 <= duration_ms),
            "invalid live segment times"
        );
        ensure!(
            words.iter().all(|w| w.t0 <= w.t1 && w.t1 <= duration_ms),
            "invalid live word times"
        );
        Ok(Finished {
            transcript: transcript(
                &engine.context("the live transcriber never named its engine")?,
                &locale,
                segments,
                words,
            ),
            tail_ms: finishing.elapsed().as_millis() as u64,
            streamed_samples: streamed,
        })
    }

    fn join_feeder(&mut self) -> Result<Feeding> {
        self.stop.store(true, Ordering::Release);
        let feeder = self.feeder.take().context("Already finished")?;
        feeder.thread().unpark();
        feeder
            .join()
            .map_err(|_| anyhow!("The live transcription thread panicked"))?
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        if self.feeder.is_some() {
            let _ = self.join_feeder();
        }
    }
}

/// As `helper::transcribe` builds it from the batch helper's output.
fn transcript(
    engine: &str,
    locale: &str,
    mut segments: Vec<TranscriptSegment>,
    mut words: Vec<WordTiming>,
) -> TranscriptData {
    segments.sort_by_key(|s| s.t0);
    words.sort_by_key(|w| w.t0);
    TranscriptData {
        source: "speech-analyzer".into(),
        model: Some(format!("{engine} {locale}")),
        text: segments
            .iter()
            .map(|s| s.text.as_str())
            .collect::<Vec<_>>()
            .join(" "),
        segments,
        words,
        estimated: false,
        missing: None,
    }
}

fn feed(mut command: Command, samples: &Mutex<Samples>, stop: &AtomicBool) -> Result<Feeding> {
    let mut child = command
        .spawn()
        .context("Could not start apparent-helper transcribe-stream")?;
    let stdin = child.stdin.take().context("No helper stdin")?;
    let stdout = child.stdout.take().context("No helper stdout")?;
    let mut stderr = child.stderr.take().context("No helper stderr")?;
    let (sender, lines) = std::sync::mpsc::channel();
    // Acquire cleanup ownership before fallible thread/pipe setup.
    let mut feeding = Feeding {
        child,
        stdin: Some(stdin),
        fed: 0,
        lines,
        group_cleaned: false,
    };
    nonblocking(feeding.stdin.as_ref().context("No helper stdin")?)?;
    let (errors, error_text) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("apparent-live-transcribe-err".into())
        .spawn(move || {
            let mut message = Vec::new();
            let mut chunk = [0; 4096];
            while let Ok(n) = stderr.read(&mut chunk) {
                if n == 0 {
                    break;
                }
                let keep = n.min(8192_usize.saturating_sub(message.len()));
                message.extend_from_slice(&chunk[..keep]);
            }
            let _ = errors.send(String::from_utf8_lossy(&message).into_owned());
        })
        .context("Could not spawn the live transcription error reader")?;
    std::thread::Builder::new()
        .name("apparent-live-transcribe-out".into())
        .spawn(move || {
            let mut reader = BufReader::new(stdout);
            let mut total = 0_usize;
            let mut events = 0_usize;
            loop {
                let mut line = Vec::new();
                let read = reader
                    .by_ref()
                    .take((MAX_LINE_BYTES + 1) as u64)
                    .read_until(b'\n', &mut line);
                if matches!(read, Ok(0)) {
                    break;
                }
                total += line.len();
                events += 1;
                if read.is_err()
                    || line.len() > MAX_LINE_BYTES
                    || total > MAX_OUTPUT_BYTES
                    || events > MAX_EVENTS
                {
                    let _ = sender.send(Line::Invalid);
                    return;
                }
                match serde_json::from_slice::<Event>(&line) {
                    Ok(event) => {
                        if sender.send(Line::Event(event)).is_err() {
                            return;
                        }
                    }
                    Err(_) => {
                        let _ = sender.send(Line::Invalid);
                        return;
                    }
                }
            }
            let message = error_text
                .recv_timeout(Duration::from_millis(100))
                .unwrap_or_default();
            let _ = sender.send(Line::Closed(message));
        })
        .context("Could not spawn the live transcription reader")?;
    let mut bytes = Vec::new();
    while !stop.load(Ordering::Acquire) {
        bytes.clear();
        {
            let buffer = match samples.try_lock() {
                Ok(buffer) => buffer,
                Err(TryLockError::WouldBlock) => {
                    std::thread::park_timeout(Duration::from_millis(5));
                    continue;
                }
                Err(TryLockError::Poisoned(_)) => bail!("Microphone buffer lock poisoned"),
            };
            bytes = pending_pcm(&buffer, feeding.fed)?;
        }
        if !bytes.is_empty() {
            let stdin = feeding.stdin.as_mut().context("No helper stdin")?;
            feeding.fed += write_pcm(stdin, &bytes, || Ok(stop.load(Ordering::Acquire)))? / 2;
        }
        std::thread::park_timeout(FEED_EVERY);
    }
    Ok(feeding)
}

/// Preserve absolute history indices across ring wrap; only Stop may clear history.
fn pending_pcm(buffer: &Samples, fed: usize) -> Result<Vec<u8>> {
    if buffer.is_empty() {
        return Ok(Vec::new());
    }
    let fresh = buffer
        .get(fed..)
        .context("Live transcription fell behind the recording")?;
    Ok(fresh
        .iter()
        .take(16_000)
        .flat_map(|&s| crate::capture::engine::runtime::audio::audio::pcm16(s).to_le_bytes())
        .collect())
}

/// A stalled model must not hold Finish or Cancel hostage in a pipe write.
fn nonblocking(stdin: &ChildStdin) -> Result<()> {
    #[cfg(unix)]
    {
        let fd = stdin.as_raw_fd();
        // SAFETY: stdin owns this live descriptor; only its status flags are changed.
        let ok = unsafe {
            let flags = libc::fcntl(fd, libc::F_GETFL);
            flags != -1 && libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) != -1
        };
        ensure!(ok, "could not make the live transcription pipe nonblocking");
        Ok(())
    }
    #[cfg(not(unix))]
    bail!("live transcription requires a Unix pipe")
}

fn write_pcm(
    stdin: &mut ChildStdin,
    bytes: &[u8],
    mut stop: impl FnMut() -> Result<bool>,
) -> Result<usize> {
    let mut written = 0;
    while written < bytes.len() {
        if stop()? {
            break;
        }
        // <= POSIX PIPE_BUF: nonblocking writes are atomic, so cancellation leaves whole i16s.
        let end = (written + 512).min(bytes.len());
        match stdin.write(&bytes[written..end]) {
            Ok(0) => bail!("The live transcriber closed its input"),
            Ok(n) => written += n,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(5))
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e).context("The live transcriber stopped"),
        }
    }
    Ok(written)
}

fn le_bytes(samples: &[i16]) -> Vec<u8> {
    samples.iter().flat_map(|s| s.to_le_bytes()).collect()
}

/// Plays `wav_bytes` into a live transcription as a microphone would, `speed` times real time in
/// 10 ms callbacks, and returns it ready to finish (`apparent reprocess --bench N --live-speed S`).
pub fn simulate(wav_bytes: &[u8], speed: f64) -> Result<Live> {
    let samples = crate::capture::engine::runtime::audio::audio::decode_wav_16k(wav_bytes)?;
    let buffer = Arc::new(Mutex::new(Samples::default()));
    let live = Live::start(Arc::clone(&buffer))?;
    let started = Instant::now();
    for (i, chunk) in samples.chunks(160).enumerate() {
        buffer
            .lock()
            .map_err(|_| anyhow!("poisoned"))?
            .extend_from_slice(chunk);
        let due = Duration::from_secs_f64((i + 1) as f64 * 0.01 / speed.max(0.01));
        if let Some(wait) = due.checked_sub(started.elapsed()) {
            std::thread::sleep(wait);
        }
    }
    // The recorder empties its buffer when it stops.
    buffer.lock().map_err(|_| anyhow!("poisoned"))?.clear();
    Ok(live)
}

/// Shares a live transcription between copies of a recording; the first `take` gets it.
#[derive(Clone, Default)]
pub struct Slot(Arc<Mutex<Option<Live>>>);

impl Slot {
    pub fn new(live: Option<Live>) -> Self {
        Self(Arc::new(Mutex::new(live)))
    }

    pub fn take(&self) -> Option<Live> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }
}

impl std::fmt::Debug for Slot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let live = self.0.lock().map(|live| live.is_some()).unwrap_or(false);
        f.debug_tuple("Slot").field(&live).finish()
    }
}

#[cfg(test)]
mod tests;
