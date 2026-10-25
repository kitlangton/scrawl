//! The command transcriber (`"transcriber": "command"`): Scrawl runs a program the user names
//! (`"transcribeCommand"`, an argv array) on the recording's WAV and reads one versioned JSON
//! transcript from its stdout. A small wrapper adapts any local model to it; Scrawl knows about
//! none of them.
//!
//! The program runs without a shell, in its own process group, with stdin `/dev/null` and
//! stderr discarded, under a wall-clock limit and an 8 MiB stdout cap. On timeout or overflow
//! the whole group is killed, and Scrawl never waits on a pipe a leftover process still holds.
//!
//! Anything the program says or is given (stdout, stderr, argv, environment, transcript text)
//! can hold what the user said, so none of it is logged or put in an error: every failure is a
//! fixed reason, plus an exit status, a signal or a field location such as `words[12].t1`.
//!
//! Only Unix runs the program for now. Elsewhere there's no `Program` (`Unusable::Unsupported`),
//! so recordings use the local engine.
#![cfg_attr(not(unix), allow(dead_code, unused_imports))]

use std::ffi::OsString;
use std::io::{ErrorKind, Read};
#[cfg(unix)]
use std::os::fd::AsRawFd;
#[cfg(unix)]
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{ChildStdout, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use serde_json::{Map, Value};

use crate::model::{TranscriptData, TranscriptSegment, WordTiming};

/// The only protocol version (`SCRAWL_TRANSCRIBE_VERSION`, the output's `version`).
const VERSION: u64 = 1;
/// Most stdout read from the program.
const OUTPUT_LIMIT: usize = 8 * 1024 * 1024;

const AUDIO: &str = "{audio}";
const SAMPLES_PER_MS: u64 = 16;
/// Spans may end this far past the audio (padding, rounding); they're clamped to it.
const END_PAD_MS: u64 = 500;
const MIN_LIMIT: Duration = Duration::from_secs(300);
const MAX_TIMEOUT_SECONDS: f64 = 7.0 * 24.0 * 3_600.0;
/// A normal exit closes stdout at once; past this, something the program started holds it.
const LEFTOVER_GRACE: Duration = Duration::from_secs(1);
const POLL: Duration = Duration::from_millis(10);

/// The configured program: `transcribeCommand` and `transcribeTimeoutSeconds`.
#[derive(Clone, PartialEq, Eq)]
pub struct Program {
    argv: Vec<String>,
    timeout: Option<Duration>,
}

impl std::fmt::Debug for Program {
    // Even the program's file name is argv, so only the shape shows.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Program")
            .field("arguments", &(self.argv.len() - 1))
            .field("timeout", &self.timeout)
            .finish()
    }
}

/// Why there is no program to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unusable {
    Missing,
    NotAList,
    NotAbsolute,
    BadTimeout,
    Unsupported,
}

impl std::fmt::Display for Unusable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Missing => "transcribeCommand is not set",
            Self::NotAList => "transcribeCommand is not a list of strings",
            Self::NotAbsolute => "transcribeCommand[0] is not an absolute path",
            Self::BadTimeout => "transcribeTimeoutSeconds is not a positive number",
            Self::Unsupported => "the command transcriber only runs on macOS for now",
        })
    }
}

impl std::error::Error for Unusable {}

/// Why a run gave no transcript. `Display` is fixed text, safe to log.
#[derive(Debug, PartialEq, Eq)]
pub enum Failure {
    Spawn(std::io::ErrorKind),
    Exit(i32),
    Signal(i32),
    Timeout(Duration),
    TooMuchOutput,
    /// The program exited but something it started still holds stdout.
    Leftover,
    Read,
    NotJson,
    NoVersion,
    UnsupportedVersion,
    NoText,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spawn(kind) => write!(f, "could not start the program ({kind})"),
            Self::Exit(code) => write!(f, "the program exited with status {code}"),
            Self::Signal(signal) => write!(f, "the program was killed by signal {signal}"),
            Self::Timeout(limit) => write!(f, "the program ran longer than {limit:?}"),
            Self::TooMuchOutput => f.write_str("the program wrote more than 8 MiB to stdout"),
            Self::Leftover => {
                f.write_str("the program exited but a process it started kept stdout open")
            }
            Self::Read => f.write_str("could not read the program's stdout"),
            Self::NotJson => f.write_str("stdout is not exactly one JSON object"),
            Self::NoVersion => f.write_str("the output has no version"),
            Self::UnsupportedVersion => f.write_str("the output's version is not 1"),
            Self::NoText => f.write_str("the output's text is missing or not a string"),
        }
    }
}

impl std::error::Error for Failure {}

impl Program {
    /// `argv[0]` must be an absolute path; `timeout` replaces the default limit.
    fn new(argv: Vec<String>, timeout: Option<Duration>) -> Result<Self, Unusable> {
        if cfg!(not(unix)) {
            return Err(Unusable::Unsupported);
        }
        let program = argv.first().ok_or(Unusable::NotAList)?;
        if !Path::new(program).is_absolute() {
            return Err(Unusable::NotAbsolute);
        }
        Ok(Self { argv, timeout })
    }

    /// Reads only the command keys of the config file at `path`, so a malformed one never
    /// affects the rest of the config (and the rest never affects these).
    pub fn load_from(path: &Path) -> Result<Self, Unusable> {
        let config: Value = std::fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let argv = match config.get("transcribeCommand") {
            None | Some(Value::Null) => return Err(Unusable::Missing),
            Some(Value::Array(items)) => items
                .iter()
                .map(|item| item.as_str().map(str::to_owned))
                .collect::<Option<Vec<_>>>()
                .ok_or(Unusable::NotAList)?,
            Some(_) => return Err(Unusable::NotAList),
        };
        let timeout = match config.get("transcribeTimeoutSeconds") {
            None | Some(Value::Null) => None,
            Some(value) => match value.as_f64() {
                Some(seconds) if seconds > 0.0 && seconds.is_finite() => {
                    Some(Duration::from_secs_f64(seconds.min(MAX_TIMEOUT_SECONDS)))
                }
                _ => return Err(Unusable::BadTimeout),
            },
        };
        Self::new(argv, timeout)
    }

    /// The file name of `argv[0]`, as in `source: "command:<name>"`.
    pub fn name(&self) -> String {
        let program = Path::new(&self.argv[0]);
        program
            .file_name()
            .unwrap_or(program.as_os_str())
            .to_string_lossy()
            .into_owned()
    }

    /// The wall-clock limit for audio of `sample_count` 16 kHz samples: the configured timeout,
    /// else `max(300 s, 60 s + 10 × the audio's length)`.
    pub fn limit(&self, sample_count: usize) -> Duration {
        self.timeout.unwrap_or_else(|| {
            MIN_LIMIT.max(Duration::from_millis(
                60_000 + 10 * sample_count as u64 / SAMPLES_PER_MS,
            ))
        })
    }

    /// `argv[1..]` with every `{audio}` replaced by `wav`, or `wav` appended when none has one.
    fn arguments(&self, wav: &Path) -> Vec<OsString> {
        let mut placed = false;
        let mut arguments: Vec<OsString> = self.argv[1..]
            .iter()
            .map(|argument| {
                let mut out = OsString::new();
                for (index, part) in argument.split(AUDIO).enumerate() {
                    if index > 0 {
                        out.push(wav);
                        placed = true;
                    }
                    out.push(part);
                }
                out
            })
            .collect();
        if !placed {
            arguments.push(wav.into());
        }
        arguments
    }

    /// Runs the program to completion and returns its stdout.
    ///
    /// The program is reaped only after any signal to its group: until then its pid (a zombie
    /// once it exits) keeps the group id from being reused, so the kill can't reach another
    /// process. Its exit is watched with `waitid(WNOWAIT)`, which doesn't reap.
    #[cfg(unix)]
    fn run(&self, wav: &Path, limit: Duration) -> Result<Vec<u8>, Failure> {
        let deadline = Instant::now() + limit;
        let mut child = std::process::Command::new(&self.argv[0])
            .args(self.arguments(wav))
            .env("SCRAWL_TRANSCRIBE_VERSION", VERSION.to_string())
            .env("SCRAWL_AUDIO", wav)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()
            .map_err(|error| Failure::Spawn(error.kind()))?;
        let pid = child.id() as libc::pid_t;
        // Read here, without blocking, so the pipe is closed on every return: a process that
        // left the group can't keep a reader of Scrawl's waiting on it.
        let mut stdout = child.stdout.take().filter(nonblocking);
        if stdout.is_none() {
            kill_group(pid);
            let _ = child.wait();
            return Err(Failure::Read);
        }
        let mut output = Vec::new();
        let mut exited: Option<Instant> = None;
        let stop = loop {
            if let Some(pipe) = stdout.as_mut() {
                match drain(pipe, &mut output) {
                    Ok(true) => stdout = None,
                    Ok(false) => {}
                    Err(_) => break Failure::Read,
                }
                if output.len() > OUTPUT_LIMIT {
                    break Failure::TooMuchOutput;
                }
            }
            if exited.is_none() {
                match has_exited(pid) {
                    Ok(true) => exited = Some(Instant::now() + LEFTOVER_GRACE),
                    Ok(false) => {}
                    // Someone else reaped it, so its group id may already be reused: no kill.
                    Err(_) => return Err(Failure::Read),
                }
            }
            let now = Instant::now();
            match exited {
                // Exited and stdout closed: nothing is left to kill.
                Some(_) if stdout.is_none() => {
                    let status = child.wait().map_err(|_| Failure::Read)?;
                    return if status.success() {
                        Ok(output)
                    } else {
                        Err(ended(status))
                    };
                }
                Some(grace) if now >= grace.min(deadline) => {
                    break if now >= deadline {
                        Failure::Timeout(limit)
                    } else {
                        Failure::Leftover
                    };
                }
                None if now >= deadline => break Failure::Timeout(limit),
                _ => {}
            }
            // The program's exit can't be polled portably, so look again every `POLL`.
            let wait = POLL.min(deadline.saturating_duration_since(now));
            match &stdout {
                Some(pipe) => readable(pipe, wait),
                None => std::thread::sleep(wait),
            }
        };
        drop(stdout);
        // Not reaped yet (see above), so the group id is still the program's.
        kill_group(pid);
        let status = child.wait().map_err(|_| Failure::Read)?;
        Err(match stop {
            Failure::Leftover if !status.success() => ended(status),
            stop => stop,
        })
    }

    #[cfg(not(unix))]
    fn run(&self, _wav: &Path, _limit: Duration) -> Result<Vec<u8>, Failure> {
        unreachable!("there's no Program off Unix (Program::new)")
    }
}

/// Makes our end of the program's stdout nonblocking.
#[cfg(unix)]
fn nonblocking(pipe: &ChildStdout) -> bool {
    let fd = pipe.as_raw_fd();
    // SAFETY: `fd` is open for as long as `pipe` is borrowed.
    unsafe {
        let flags = libc::fcntl(fd, libc::F_GETFL);
        flags != -1 && libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) != -1
    }
}

/// Reads what the pipe holds now, stopping one byte past `OUTPUT_LIMIT`. `true` at end of file.
fn drain(pipe: &mut ChildStdout, output: &mut Vec<u8>) -> std::io::Result<bool> {
    let mut chunk = [0_u8; 64 * 1024];
    while output.len() <= OUTPUT_LIMIT {
        let room = (OUTPUT_LIMIT + 1 - output.len()).min(chunk.len());
        match pipe.read(&mut chunk[..room]) {
            Ok(0) => return Ok(true),
            Ok(read) => output.extend_from_slice(&chunk[..read]),
            Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(false),
            Err(error) if error.kind() == ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(false)
}

/// Waits up to `timeout` for the pipe to have data or close.
#[cfg(unix)]
fn readable(pipe: &ChildStdout, timeout: Duration) {
    let mut poll = libc::pollfd {
        fd: pipe.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    let ms = timeout.as_micros().div_ceil(1_000).min(i32::MAX as u128) as libc::c_int;
    // SAFETY: one valid `pollfd`; an error or EINTR just means looking again sooner.
    unsafe {
        libc::poll(&mut poll, 1, ms);
    }
}

/// Whether the program has exited, leaving it unreaped.
#[cfg(unix)]
fn has_exited(pid: libc::pid_t) -> std::io::Result<bool> {
    loop {
        // SAFETY: `info` is a zeroed `siginfo_t` that `waitid` fills in.
        let (result, info) = unsafe {
            let mut info: libc::siginfo_t = std::mem::zeroed();
            let result = libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
            );
            (result, info)
        };
        if result == 0 {
            // With WNOHANG and nothing to report, `si_pid` stays 0.
            return Ok(unsafe { info.si_pid() } != 0);
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

/// Kills the program's process group. Only call it before the program is reaped (`run`).
#[cfg(unix)]
fn kill_group(pid: libc::pid_t) {
    // SAFETY: plain syscall; the group id is the program's pid (`process_group(0)`).
    unsafe {
        libc::killpg(pid, libc::SIGKILL);
    }
}

#[cfg(unix)]
fn ended(status: ExitStatus) -> Failure {
    match (status.code(), status.signal()) {
        (Some(code), _) => Failure::Exit(code),
        (None, Some(signal)) => Failure::Signal(signal),
        (None, None) => Failure::Exit(-1),
    }
}

/// Transcribes `wav` (`sample_count` 16 kHz samples) with `program`. Timing it can't use is
/// dropped with one line to `log`; the text is always kept.
pub fn transcribe(
    program: &Program,
    wav: &Path,
    sample_count: usize,
    log: &mut dyn FnMut(&str),
) -> Result<TranscriptData, Failure> {
    let stdout = program.run(wav, program.limit(sample_count))?;
    parse(
        &stdout,
        &program.name(),
        sample_count as u64 / SAMPLES_PER_MS,
        log,
    )
}

/// A `{t0, t1, text}` entry of `segments` or `words`, clamped to the audio.
#[derive(Debug, PartialEq, Eq)]
struct Span {
    t0: u64,
    t1: u64,
    text: String,
}

/// A version 1 program's stdout as a transcript. `model` or `estimated` of another type counts
/// as absent; an unusable `segments` or `words` is dropped (logged with where it goes wrong).
fn parse(
    stdout: &[u8],
    name: &str,
    audio_ms: u64,
    log: &mut dyn FnMut(&str),
) -> Result<TranscriptData, Failure> {
    // serde's messages can quote the output, so none of them are passed on.
    let Ok(Value::Object(object)) = serde_json::from_slice::<Value>(stdout) else {
        return Err(Failure::NotJson);
    };
    match object.get("version") {
        None | Some(Value::Null) => return Err(Failure::NoVersion),
        Some(version) if version.as_u64() == Some(VERSION) => {}
        Some(_) => return Err(Failure::UnsupportedVersion),
    }
    let Some(Value::String(text)) = object.get("text") else {
        return Err(Failure::NoText);
    };
    let text = text.trim();
    let mut transcript = TranscriptData {
        source: format!("command:{name}"),
        model: object
            .get("model")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|model| !model.is_empty())
            .map(str::to_owned),
        ..Default::default()
    };
    if text.is_empty() {
        // No speech: timing for it would mean nothing.
        return Ok(transcript);
    }
    let mut usable = |field: &str, otherwise: &str| {
        spans(&object, field, audio_ms).unwrap_or_else(|location| {
            log(&format!(
                "Command transcriber: unusable {field} ({location}); {otherwise}"
            ));
            Vec::new()
        })
    };
    let segments = usable("segments", "segments dropped");
    let words = usable("words", "spreading the text instead");
    transcript.estimated =
        words.is_empty() || object.get("estimated").and_then(Value::as_bool) != Some(false);
    transcript.words = if words.is_empty() {
        // No word times: spread the text over each segment, else over the recording.
        let mut spread: Vec<WordTiming> = if segments.is_empty() {
            crate::transcribe::spread_words(text, 0, audio_ms).1
        } else {
            segments
                .iter()
                .flat_map(|segment| {
                    crate::transcribe::spread_words(&segment.text, segment.t0, segment.t1).1
                })
                .collect()
        };
        // Overlapping segments interleave their words.
        spread.sort_by_key(|word| word.t0);
        spread
    } else {
        words
            .into_iter()
            .map(|Span { t0, t1, text }| WordTiming { t0, t1, text })
            .collect()
    };
    transcript.segments = segments
        .into_iter()
        .map(|Span { t0, t1, text }| TranscriptSegment { t0, t1, text })
        .collect();
    transcript.text = text.to_owned();
    Ok(transcript)
}

/// The `field` array checked and sorted by `t0` (ties keep their order), or where it first
/// goes wrong: not an array, an entry that isn't `{t0, t1, text}`, a negative, fractional or
/// reversed span, or one ending more than `END_PAD_MS` past the audio.
fn spans(object: &Map<String, Value>, field: &str, audio_ms: u64) -> Result<Vec<Span>, String> {
    let items = match object.get(field) {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::Array(items)) => items,
        Some(_) => return Err(field.to_owned()),
    };
    let mut spans = items
        .iter()
        .enumerate()
        .map(|(index, item)| {
            let at = |key: &str| format!("{field}[{index}]{key}");
            let entry = item.as_object().ok_or_else(|| at(""))?;
            let time = |key: &str| {
                entry
                    .get(key)
                    .and_then(milliseconds)
                    .ok_or_else(|| at(&format!(".{key}")))
            };
            let (t0, t1) = (time("t0")?, time("t1")?);
            if t1 < t0 || t1 > audio_ms + END_PAD_MS {
                return Err(at(".t1"));
            }
            let text = entry
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(|| at(".text"))?;
            Ok(Span {
                t0: t0.min(audio_ms),
                t1: t1.min(audio_ms),
                text: text.trim().to_owned(),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    spans.sort_by_key(|span| span.t0);
    Ok(spans)
}

/// A whole, non-negative number of milliseconds (`1200` or `1200.0`).
fn milliseconds(value: &Value) -> Option<u64> {
    value.as_u64().or_else(|| {
        value
            .as_f64()
            .filter(|ms| *ms >= 0.0 && ms.fract() == 0.0 && *ms < 2_f64.powi(53))
            .map(|ms| ms as u64)
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::model::{TranscriptSegment, WordTiming};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Instant;

    /// One second of audio.
    const SECOND: usize = 16_000;

    /// A temp folder (with a space in its name) for fake programs and their audio.
    struct Dir(PathBuf);

    impl Dir {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let dir = std::env::temp_dir().join(format!(
                "scrawl command {}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("audio.wav"), b"RIFF").unwrap();
            Self(dir)
        }

        fn wav(&self) -> PathBuf {
            self.0.join("audio.wav")
        }

        /// Writes an executable `/bin/sh` script. A child process writes it, so no write handle
        /// of this (multi-threaded) process can leak into another test's fork and make the exec
        /// fail with "text file busy".
        fn script(&self, name: &str, body: &str) -> PathBuf {
            let path = self.0.join(name);
            let status = std::process::Command::new("/bin/sh")
                .args([
                    "-c",
                    "printf '%s' \"$1\" > \"$2\" && chmod 755 \"$2\"",
                    "sh",
                ])
                .arg(format!("#!/bin/sh\n{body}\n"))
                .arg(&path)
                .status()
                .unwrap();
            assert!(status.success());
            path
        }

        fn program(&self, body: &str, timeout: Option<Duration>) -> Program {
            let path = self.script("fake program", body);
            Program::new(vec![path.to_string_lossy().into_owned()], timeout).unwrap()
        }

        /// A program that prints `json` and exits 0.
        fn replying(&self, json: &str) -> Program {
            self.program(&format!("cat <<'EOF'\n{json}\nEOF"), None)
        }

        fn run(
            &self,
            program: &Program,
            sample_count: usize,
        ) -> (Result<TranscriptData, Failure>, Vec<String>) {
            let mut lines = Vec::new();
            let result = transcribe(program, &self.wav(), sample_count, &mut |line| {
                lines.push(line.to_owned())
            });
            (result, lines)
        }

        /// The transcript of a program printing `json` over `sample_count` samples.
        fn reply(&self, json: &str, sample_count: usize) -> (TranscriptData, Vec<String>) {
            let (result, lines) = self.run(&self.replying(json), sample_count);
            (result.unwrap(), lines)
        }

        fn fails(&self, json: &str) -> Failure {
            self.run(&self.replying(json), 10 * SECOND).0.unwrap_err()
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn span(t0: u64, t1: u64, text: &str) -> (u64, u64, String) {
        (t0, t1, text.to_owned())
    }

    fn words(transcript: &TranscriptData) -> Vec<(u64, u64, String)> {
        transcript
            .words
            .iter()
            .map(|w| (w.t0, w.t1, w.text.clone()))
            .collect()
    }

    fn segments(transcript: &TranscriptData) -> Vec<(u64, u64, String)> {
        transcript
            .segments
            .iter()
            .map(|s| (s.t0, s.t1, s.text.clone()))
            .collect()
    }

    /// Whether `pid` is gone, giving its new parent a moment to reap it.
    fn gone(pid: &str) -> bool {
        let pid: libc::pid_t = pid.trim().parse().unwrap();
        let until = Instant::now() + Duration::from_secs(3);
        while Instant::now() < until {
            if unsafe { libc::kill(pid, 0) } != 0 {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    }

    fn config(dir: &Dir, json: &str) -> Result<Program, Unusable> {
        let path = dir.0.join("config.json");
        std::fs::write(&path, json).unwrap();
        Program::load_from(&path)
    }

    #[test]
    fn the_command_needs_an_absolute_program_and_its_keys_stand_alone() {
        let dir = Dir::new();
        assert_eq!(
            Program::load_from(&dir.0.join("none.json")),
            Err(Unusable::Missing)
        );
        assert_eq!(config(&dir, "not json"), Err(Unusable::Missing));
        assert_eq!(
            config(&dir, r#"{"transcriber":"command"}"#),
            Err(Unusable::Missing)
        );
        assert_eq!(
            config(&dir, r#"{"transcribeCommand":null}"#),
            Err(Unusable::Missing)
        );
        for bad in [r#""/bin/x {audio}""#, "[]", r#"["/bin/x", 3]"#, "{}", "7"] {
            assert_eq!(
                config(&dir, &format!(r#"{{"transcribeCommand":{bad}}}"#)),
                Err(Unusable::NotAList),
                "{bad}"
            );
        }
        for bad in [
            r#"["whisper"]"#,
            r#"["~/bin/whisper"]"#,
            r#"[""]"#,
            r#"["bin/w"]"#,
        ] {
            assert_eq!(
                config(&dir, &format!(r#"{{"transcribeCommand":{bad}}}"#)),
                Err(Unusable::NotAbsolute),
                "{bad}"
            );
        }

        // Keys the main config struct would choke on don't matter here.
        let program = config(
            &dir,
            r#"{"autoSend":"nope","usbApps":7,"transcribeCommand":["/opt/my stt/run","{audio}"],"transcribeTimeoutSeconds":600}"#,
        )
        .unwrap();
        assert_eq!(program.name(), "run");
        assert_eq!(program.limit(SECOND), Duration::from_secs(600));
        let config_timeout = |timeout: &str| {
            config(
                &dir,
                &format!(
                    r#"{{"transcribeCommand":["/bin/true"],"transcribeTimeoutSeconds":{timeout}}}"#
                ),
            )
        };
        assert_eq!(
            config_timeout("null").unwrap().limit(SECOND),
            Duration::from_secs(300)
        );
        for timeout in ["0", "-5", r#""600""#, "[600]"] {
            assert_eq!(
                config_timeout(timeout),
                Err(Unusable::BadTimeout),
                "{timeout}"
            );
        }
    }

    #[test]
    fn the_default_limit_grows_with_the_audio() {
        let program = Program::new(vec!["/bin/true".into()], None).unwrap();
        assert_eq!(program.limit(0), Duration::from_secs(300));
        assert_eq!(program.limit(60 * SECOND), Duration::from_secs(660));
        let set = Program::new(vec!["/bin/true".into()], Some(Duration::from_secs(5))).unwrap();
        assert_eq!(set.limit(3_600 * SECOND), Duration::from_secs(5));
    }

    #[test]
    fn every_placeholder_becomes_the_audio_path_and_spaces_survive() {
        let dir = Dir::new();
        let log = dir.0.join("seen");
        let body = format!(
            r#"out='{}'
printf '%s\n' "$#" "$@" > "$out"
printf '%s\n' "$SCRAWL_TRANSCRIBE_VERSION" "$SCRAWL_AUDIO" >> "$out"
cat >> "$out"
echo '{{"version":1,"text":"ok"}}'"#,
            log.display()
        );
        let path = dir.script("my stt", &body);
        let argv = [
            path.to_str().unwrap(),
            "--model",
            "large v3",
            "--in={audio}.x",
            "{audio}{audio}",
            "$HOME",
            "*",
            "~",
        ]
        .map(str::to_owned)
        .to_vec();
        let program = Program::new(argv, None).unwrap();
        let (result, _) = dir.run(&program, SECOND);
        let transcript = result.unwrap();
        assert_eq!(transcript.source, "command:my stt");
        let wav = dir.wav().to_string_lossy().into_owned();
        let seen = std::fs::read_to_string(&log).unwrap();
        let expected = [
            "7",
            "--model",
            "large v3",
            &format!("--in={wav}.x"),
            &format!("{wav}{wav}"),
            "$HOME",
            "*",
            "~",
            "1",
            &wav,
        ];
        // stdin was empty, so nothing follows the environment.
        assert_eq!(seen.lines().collect::<Vec<_>>(), expected);
    }

    #[test]
    fn without_a_placeholder_the_audio_path_comes_last() {
        let dir = Dir::new();
        let log = dir.0.join("seen");
        let path = dir.script(
            "stt",
            &format!(
                "printf '%s\\n' \"$@\" > '{}'\necho '{{\"version\":1,\"text\":\"\"}}'",
                log.display()
            ),
        );
        let program =
            Program::new(vec![path.to_string_lossy().into(), "--json".into()], None).unwrap();
        dir.run(&program, SECOND).0.unwrap();
        let wav = dir.wav().to_string_lossy().into_owned();
        assert_eq!(
            std::fs::read_to_string(&log)
                .unwrap()
                .lines()
                .collect::<Vec<_>>(),
            ["--json", wav.as_str()]
        );
    }

    #[test]
    fn a_full_answer_lands_in_the_transcript() {
        let dir = Dir::new();
        let (transcript, log) = dir.reply(
            r#"
            {
              "version": 1,
              "model": "whisper-large-v3-turbo",
              "text": "So this button here doesn't do anything.",
              "segments": [{ "t0": 0, "t1": 2140, "text": "So this button here doesn't do anything." }],
              "words": [
                { "t0": 0, "t1": 180, "text": "So" },
                { "t0": 180, "t1": 420, "text": "this" },
                { "t0": 420, "t1": 760, "text": "button" },
                { "t0": 760, "t1": 980, "text": "here" },
                { "t0": 1010, "t1": 1420, "text": "doesn't" },
                { "t0": 1420, "t1": 1600, "text": "do" },
                { "t0": 1600, "t1": 2140, "text": "anything." }
              ],
              "estimated": false
            }
            "#,
            3 * SECOND,
        );
        assert!(log.is_empty(), "{log:?}");
        assert_eq!(
            transcript,
            TranscriptData {
                source: "command:fake program".into(),
                model: Some("whisper-large-v3-turbo".into()),
                text: "So this button here doesn't do anything.".into(),
                segments: vec![TranscriptSegment {
                    t0: 0,
                    t1: 2140,
                    text: "So this button here doesn't do anything.".into(),
                }],
                words: [
                    (0, 180, "So"),
                    (180, 420, "this"),
                    (420, 760, "button"),
                    (760, 980, "here"),
                    (1010, 1420, "doesn't"),
                    (1420, 1600, "do"),
                    (1600, 2140, "anything."),
                ]
                .map(|(t0, t1, text)| WordTiming {
                    t0,
                    t1,
                    text: text.into(),
                })
                .to_vec(),
                estimated: false,
                missing: None,
            }
        );
    }

    #[test]
    fn empty_speech_is_a_success() {
        let dir = Dir::new();
        for text in ["", "  \n\t "] {
            let (transcript, _) = dir.reply(
                &format!(
                    r#"{{"version":1,"model":"whisper","text":{text:?},"segments":[{{"t0":0,"t1":5,"text":"uh"}}],"words":[{{"t0":0,"t1":5,"text":"uh"}}]}}"#
                ),
                SECOND,
            );
            assert_eq!(transcript.source, "command:fake program");
            assert_eq!(transcript.model.as_deref(), Some("whisper"));
            assert_eq!(transcript.text, "");
            assert!(transcript.segments.is_empty() && transcript.words.is_empty());
        }
    }

    #[test]
    fn text_only_segments_only_and_words_only_answers() {
        let dir = Dir::new();
        let (text, _) = dir.reply(
            r#"{"version":1,"text":" Just words. ","model":null,"segments":null,"words":[]}"#,
            SECOND,
        );
        assert_eq!(text.text, "Just words.");
        assert_eq!(text.model, None);
        assert!(text.segments.is_empty());
        // Without word times the text is spread over the recording, as estimates.
        let spread = words(&text);
        assert_eq!(
            spread.iter().map(|w| w.2.as_str()).collect::<Vec<_>>(),
            ["Just", "words."]
        );
        assert_eq!((spread[0].0, spread[1].1), (0, 1000));
        assert!(text.estimated);

        // ... or over each segment when there are segments, whatever `estimated` says.
        let (only_segments, _) = dir.reply(
            r#"{"version":1,"text":"One. Two.","segments":[{"t0":900,"t1":1500,"text":"Two."},{"t0":0,"t1":800,"text":"One."}],"estimated":false}"#,
            2 * SECOND,
        );
        assert_eq!(
            segments(&only_segments),
            [span(0, 800, "One."), span(900, 1500, "Two.")]
        );
        assert_eq!(
            words(&only_segments),
            [span(0, 800, "One."), span(900, 1500, "Two.")]
        );
        assert!(only_segments.estimated);

        let (only_words, _) = dir.reply(
            r#"{"version":1,"text":"Hi there","words":[{"t0":0,"t1":200,"text":"Hi"},{"t0":250,"t1":600,"text":"there"}],"estimated":false}"#,
            SECOND,
        );
        assert_eq!(
            words(&only_words),
            [span(0, 200, "Hi"), span(250, 600, "there")]
        );
        assert!(only_words.segments.is_empty());
        assert!(!only_words.estimated);

        let (unknown, log) = dir.reply(
            r#"{"version":1,"text":"x","future":{"a":1},"words":[{"t0":0,"t1":1,"text":"x","p":0.9}],"estimated":null}"#,
            SECOND,
        );
        assert!(log.is_empty(), "{log:?}");
        assert_eq!(words(&unknown), [span(0, 1, "x")]);
        assert!(unknown.estimated);
    }

    #[test]
    fn overlapping_tied_and_zero_length_spans_are_sorted_stably() {
        let dir = Dir::new();
        let (transcript, log) = dir.reply(
            r#"{"version":1,"text":"a b c d e","words":[
                {"t0":500,"t1":700,"text":"d"},
                {"t0":100,"t1":400,"text":"a"},
                {"t0":300,"t1":300,"text":"b"},
                {"t0":300,"t1":350,"text":"c"},
                {"t0":600,"t1":900,"text":"e"}
            ],"estimated":false}"#,
            SECOND,
        );
        assert!(log.is_empty(), "{log:?}");
        assert_eq!(
            words(&transcript),
            [
                span(100, 400, "a"),
                span(300, 300, "b"),
                span(300, 350, "c"),
                span(500, 700, "d"),
                span(600, 900, "e"),
            ]
        );
        assert!(!transcript.estimated);
    }

    #[test]
    fn words_spread_over_overlapping_segments_come_out_in_order() {
        let dir = Dir::new();
        let (transcript, _) = dir.reply(
            r#"{"version":1,"text":"aa bb cc dd x","segments":[{"t0":0,"t1":2000,"text":"aa bb cc dd"},{"t0":100,"t1":300,"text":"x"}]}"#,
            3 * SECOND,
        );
        let spread = words(&transcript);
        assert_eq!(
            spread.iter().map(|w| w.2.as_str()).collect::<Vec<_>>(),
            ["aa", "x", "bb", "cc", "dd"],
            "{spread:?}"
        );
        assert!(spread.windows(2).all(|pair| pair[0].0 <= pair[1].0));
        assert!(transcript.estimated);
    }

    #[test]
    fn ends_up_to_500_ms_past_the_audio_are_clamped() {
        let dir = Dir::new();
        // 1000.5 ms of audio.
        let samples = SECOND + 8;
        let (transcript, log) = dir.reply(
            r#"{"version":1,"text":"a b c","segments":[{"t0":0,"t1":1500,"text":"a b c"}],"words":[
                {"t0":0,"t1":400,"text":"a"},
                {"t0":900,"t1":1200,"text":"b"},
                {"t0":1300,"t1":1500,"text":"c"}
            ],"estimated":false}"#,
            samples,
        );
        assert!(log.is_empty(), "{log:?}");
        assert_eq!(segments(&transcript), [span(0, 1000, "a b c")]);
        assert_eq!(
            words(&transcript),
            [
                span(0, 400, "a"),
                span(900, 1000, "b"),
                span(1000, 1000, "c")
            ]
        );
        assert!(!transcript.estimated);

        let (late, log) = dir.reply(
            r#"{"version":1,"text":"a","words":[{"t0":0,"t1":1501,"text":"a"}],"estimated":false}"#,
            samples,
        );
        assert_eq!(log.len(), 1);
        assert!(log[0].contains("words[0].t1"), "{log:?}");
        assert!(late.estimated);
        assert_eq!(late.text, "a");
    }

    #[test]
    fn an_unusable_array_is_dropped_but_no_text_is_lost() {
        let dir = Dir::new();
        // Bad words with good segments: spread over each segment.
        let (over_segments, log) = dir.reply(
            r#"{"version":1,"text":"One two. Three four.","segments":[
                {"t0":2000,"t1":3000,"text":"Three four."},
                {"t0":0,"t1":1000,"text":"One two."}
            ],"words":[{"t0":0,"t1":100,"text":"One"},{"t0":-5,"t1":100,"text":"two."}],"estimated":false}"#,
            4 * SECOND,
        );
        assert_eq!(log.len(), 1, "{log:?}");
        assert!(log[0].contains("words[1].t0"), "{log:?}");
        assert_eq!(over_segments.text, "One two. Three four.");
        assert_eq!(segments(&over_segments).len(), 2);
        assert!(over_segments.estimated);
        let spread = words(&over_segments);
        assert_eq!(
            spread.iter().map(|w| w.2.as_str()).collect::<Vec<_>>(),
            ["One", "two.", "Three", "four."]
        );
        assert!(spread[..2].iter().all(|w| w.0 < w.1 && w.1 <= 1000));
        assert!(spread[2..].iter().all(|w| w.0 >= 2000 && w.1 <= 3000));

        // Bad words without segments: spread over the whole recording.
        let (whole, log) = dir.reply(
            r#"{"version":1,"text":"alpha beta","words":[{"t0":0,"t1":10.5,"text":"alpha"},{"t0":20,"t1":30,"text":"beta"}],"estimated":false}"#,
            2 * SECOND,
        );
        assert!(log[0].contains("words[0].t1"), "{log:?}");
        assert!(whole.estimated);
        let spread = words(&whole);
        assert_eq!(spread.len(), 2);
        assert_eq!((spread[0].0, spread[1].1), (0, 2000));

        // Bad segments are left empty; good words stay.
        let (no_segments, log) = dir.reply(
            r#"{"version":1,"text":"x y","segments":[{"t0":0,"t1":5,"text":"x"},{"t0":9,"t1":3,"text":"y"}],"words":[{"t0":0,"t1":5,"text":"x"},{"t0":5,"t1":9,"text":"y"}],"estimated":false}"#,
            SECOND,
        );
        assert!(log[0].contains("segments[1]"), "{log:?}");
        assert!(no_segments.segments.is_empty());
        assert_eq!(words(&no_segments), [span(0, 5, "x"), span(5, 9, "y")]);
        assert!(!no_segments.estimated);

        // Both bad: segments empty, the text spread over the recording.
        for (json, location) in [
            (
                r#""words":"x y","segments":[{"t0":0,"t1":2000,"text":"x y"}]"#,
                "segments[0].t1",
            ),
            (
                r#""words":[{"t0":0,"t1":1}, 5],"segments":{"t0":0}"#,
                "segments",
            ),
            (
                r#""words":[5],"segments":[{"t0":0,"t1":1,"text":7}]"#,
                "segments[0].text",
            ),
        ] {
            let (both, log) = dir.reply(&format!(r#"{{"version":1,"text":"x y",{json}}}"#), SECOND);
            assert_eq!(log.len(), 2, "{log:?}");
            assert!(log.iter().any(|line| line.contains(location)), "{log:?}");
            assert_eq!(both.text, "x y");
            assert!(both.segments.is_empty());
            assert_eq!(both.words.len(), 2);
            assert_eq!(both.words[1].t1, 1000);
            assert!(both.estimated);
        }
    }

    #[test]
    fn the_version_must_be_the_integer_one() {
        let dir = Dir::new();
        assert_eq!(dir.fails(r#"{"text":"hi"}"#), Failure::NoVersion);
        assert_eq!(
            dir.fails(r#"{"version":null,"text":"hi"}"#),
            Failure::NoVersion
        );
        for version in [r#""1""#, "2", "1.0", "true", "[1]"] {
            assert_eq!(
                dir.fails(&format!(r#"{{"version":{version},"text":"hi"}}"#)),
                Failure::UnsupportedVersion,
                "{version}"
            );
        }
    }

    #[test]
    fn stdout_must_be_one_json_object_with_text() {
        let dir = Dir::new();
        for output in [
            "",
            "Loading model...\n{\"version\":1,\"text\":\"hi\"}",
            r#"{"version":1,"text":"hi"} done"#,
            r#"{"version":1,"text":"hi"}{"version":1,"text":"hi"}"#,
            r#"[{"version":1,"text":"hi"}]"#,
            r#"{"version":1,"text":"hi""#,
        ] {
            assert_eq!(dir.fails(output), Failure::NotJson, "{output}");
        }
        assert_eq!(dir.fails(r#"{"version":1}"#), Failure::NoText);
        assert_eq!(dir.fails(r#"{"version":1,"text":null}"#), Failure::NoText);
        assert_eq!(dir.fails(r#"{"version":1,"text":["hi"]}"#), Failure::NoText);
    }

    #[test]
    fn a_program_that_closes_stdout_and_keeps_running_is_killed_at_the_deadline() {
        let dir = Dir::new();
        let pid = dir.0.join("pid");
        let program = dir.program(
            &format!(
                "sleep 30 > /dev/null &\necho $! > '{}'\necho '{{\"version\":1,\"text\":\"hi\"}}'\nexec >&-\nsleep 30",
                pid.display()
            ),
            Some(Duration::from_secs(1)),
        );
        let started = Instant::now();
        let (result, _) = dir.run(&program, SECOND);
        assert_eq!(result, Err(Failure::Timeout(Duration::from_secs(1))));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(
            gone(&std::fs::read_to_string(&pid).unwrap()),
            "the group is killed"
        );
    }

    #[test]
    fn a_child_that_keeps_stdout_open_does_not_hold_scrawl() {
        let dir = Dir::new();
        let pid = dir.0.join("pid");
        let program = dir.program(
            &format!(
                "echo '{{\"version\":1,\"text\":\"hi\"}}'\nsleep 30 &\necho $! > '{}'\nexit 0",
                pid.display()
            ),
            Some(Duration::from_secs(20)),
        );
        let started = Instant::now();
        let (result, _) = dir.run(&program, SECOND);
        assert_eq!(result, Err(Failure::Leftover));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(gone(&std::fs::read_to_string(&pid).unwrap()));
    }

    #[test]
    fn stdout_held_outside_the_group_is_closed_when_scrawl_gives_up() {
        let dir = Dir::new();
        let (leader, holder) = (dir.0.join("leader"), dir.0.join("holder"));
        // `set -m` puts the background holder in a process group of its own, out of reach of
        // the group kill. It keeps stdout, then reports whether Scrawl still reads it. The probe
        // is `/bin/echo` because bash 3.2 (macOS `/bin/sh`) keeps a failed builtin `echo`
        // buffered and writes it out with the next one, into the report.
        let program = dir.program(
            &format!(
                r#"echo $$ > '{leader}'
echo '{{"version":1,"text":"hi"}}'
set -m
sh -c 'trap "" PIPE
ps -o pgid= -p $$ > "$1"
sleep 3
if /bin/echo late; then state=open; else state=closed; fi
echo "$state" >> "$1"' sh '{holder}' &
exit 0"#,
                leader = leader.display(),
                holder = holder.display(),
            ),
            Some(Duration::from_secs(20)),
        );
        let started = Instant::now();
        assert_eq!(dir.run(&program, SECOND).0, Err(Failure::Leftover));
        assert!(started.elapsed() < Duration::from_secs(3));
        let until = Instant::now() + Duration::from_secs(10);
        let report = loop {
            let report = std::fs::read_to_string(&holder).unwrap_or_default();
            if report.lines().count() == 2 || Instant::now() > until {
                break report;
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        let lines: Vec<&str> = report.lines().map(str::trim).collect();
        let leader = std::fs::read_to_string(&leader).unwrap();
        assert_ne!(
            lines.first(),
            Some(&leader.trim()),
            "the holder left the group"
        );
        assert_eq!(lines.get(1), Some(&"closed"), "{report:?}");
    }

    #[test]
    fn more_than_8_mib_of_output_fails() {
        let dir = Dir::new();
        let exactly = |extra: usize| {
            format!(
                "json='{{\"version\":1,\"text\":\"hi\"}}'\nprintf '%s' \"$json\"\nhead -c $(({} - ${{#json}})) /dev/zero | tr '\\0' ' '",
                OUTPUT_LIMIT + extra
            )
        };
        let (fits, _) = dir.run(&dir.program(&exactly(0), None), SECOND);
        assert_eq!(fits.unwrap().text, "hi");
        let (over, _) = dir.run(&dir.program(&exactly(1), None), SECOND);
        assert_eq!(over, Err(Failure::TooMuchOutput));

        let started = Instant::now();
        let endless = dir.program("yes", Some(Duration::from_secs(20)));
        assert_eq!(dir.run(&endless, SECOND).0, Err(Failure::TooMuchOutput));
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn exit_status_signals_and_spawn_errors_fail() {
        let dir = Dir::new();
        let reply = r#"echo '{"version":1,"text":"hi"}'"#;
        let status = dir.program(&format!("{reply}\nexit 3"), None);
        assert_eq!(dir.run(&status, SECOND).0, Err(Failure::Exit(3)));
        let killed = dir.program(&format!("{reply}\nkill -9 $$"), None);
        assert_eq!(
            dir.run(&killed, SECOND).0,
            Err(Failure::Signal(libc::SIGKILL))
        );

        let missing = dir.0.join("missing").to_string_lossy().into_owned();
        let missing = Program::new(vec![missing], None).unwrap();
        assert!(matches!(
            dir.run(&missing, SECOND).0,
            Err(Failure::Spawn(_))
        ));
        let plain = dir.0.join("plain");
        std::fs::write(&plain, reply).unwrap();
        let plain = Program::new(vec![plain.to_string_lossy().into()], None).unwrap();
        assert!(matches!(dir.run(&plain, SECOND).0, Err(Failure::Spawn(_))));
    }

    #[test]
    fn nothing_the_program_says_or_is_given_reaches_the_log() {
        let dir = Dir::new();
        let secret = |what: &str| format!("SECRET_{what}_7f3a");
        let wav_dir = dir.0.join(secret("PATH"));
        std::fs::create_dir_all(&wav_dir).unwrap();
        let wav = wav_dir.join("audio.wav");
        std::fs::write(&wav, b"RIFF").unwrap();
        let stderr = format!("echo '{}' >&2", secret("STDERR"));
        let invalid_timing = format!(
            r#"{{"version":1,"model":["{m}"],"text":"{t}","segments":[{{"t0":0,"t1":"{s}","text":"{t}"}}],"words":[{{"t0":"{w}","t1":1,"text":"{t}"}}],"estimated":"{e}"}}"#,
            m = secret("MODEL"),
            t = secret("TEXT"),
            s = secret("SEGMENT"),
            w = secret("WORD"),
            e = secret("ESTIMATED"),
        );
        let bodies = [
            format!("{stderr}\necho '{}'", secret("STDOUT")),
            format!(
                "{stderr}\necho '{{\"version\":\"{}\",\"text\":\"x\"}}'",
                secret("VERSION")
            ),
            format!(
                "{stderr}\necho '{{\"version\":1,\"text\":{{\"a\":\"{}\"}}}}'",
                secret("TEXT")
            ),
            format!("{stderr}\necho '{{\"version\":1,\"{}\":1'", secret("KEY")),
            format!("{stderr}\necho '{}'\nexit 4", secret("STDOUT")),
            format!("{stderr}\ncat <<'EOF'\n{invalid_timing}\nEOF"),
            format!("{stderr}\necho '{}'\nsleep 30", secret("STDOUT")),
        ];
        let mut said = Vec::new();
        for (index, body) in bodies.iter().enumerate() {
            let path = dir.script(&format!("{}-{index}", secret("PROGRAM")), body);
            let argv = vec![
                path.to_string_lossy().into_owned(),
                secret("ARG"),
                "--audio={audio}".to_owned(),
            ];
            let program = Program::new(argv, Some(Duration::from_secs(1))).unwrap();
            said.push(format!("{program:?}"));
            let result = transcribe(&program, &wav, SECOND, &mut |line| {
                said.push(line.to_owned())
            });
            match result {
                Ok(transcript) => assert_eq!(transcript.text, secret("TEXT")),
                Err(failure) => {
                    said.push(failure.to_string());
                    said.push(format!("{failure:?}"));
                    said.push(format!("{:#}", anyhow::Error::from(failure)));
                }
            }
        }
        let unusable = Program::load_from(&dir.0.join("none")).unwrap_err();
        said.push(unusable.to_string());
        assert!(said.len() > bodies.len() * 2, "{said:?}");
        for line in &said {
            assert!(!line.contains("SECRET"), "{line}");
        }
    }
}

#[cfg(all(test, not(unix)))]
mod tests {
    use super::*;

    #[test]
    fn there_is_no_program_off_unix() {
        let program = std::env::temp_dir().join("whisper-scrawl");
        assert_eq!(
            Program::new(vec![program.to_string_lossy().into_owned()], None),
            Err(Unusable::Unsupported)
        );
    }
}
