//! `onCapture`: a command of the user's own that Scrawl runs once a capture's bundle is written,
//! so any agent (or a script, a sync, a bot) can receive it without watching `captures/`.
//!
//! ```json
//! { "onCapture": ["/Users/me/bin/send-to-my-agent", "{bundle}"], "onCaptureTimeoutSeconds": 120 }
//! ```
//!
//! It runs as an argv, never through a shell, and its first entry must be an absolute path.
//! `{bundle}` anywhere in an argument becomes the `.scrawl` folder; with no `{bundle}`, the
//! folder is added as the last argument. Exit 0 counts as delivered, and the first line it prints
//! names where the capture went ("Sent to Hermes"). Anything else, or running past the timeout,
//! is logged and the capture stays on the clipboard as it always does.

use std::io::Read;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::capture::scrawl_log;

pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(120);
/// The longest timeout honoured; `onCaptureTimeoutSeconds` past it would overflow the deadline.
const MAX_TIMEOUT: Duration = Duration::from_secs(60 * 60);
const PLACEHOLDER: &str = "{bundle}";
/// Enough of stdout for a first line, and of stderr for a log line.
const OUTPUT_CAP: usize = 16 * 1024;
/// How long to keep reading once the command exits, while something it started holds a pipe.
const OUTPUT_GRACE: Duration = Duration::from_millis(500);

/// A configured `onCapture` command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OnCapture {
    pub argv: Vec<String>,
    pub timeout: Duration,
}

/// How a run went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Exited 0; `to` is the first line it printed, if any.
    Delivered { to: Option<String> },
    /// Couldn't start, exited non-zero, or ran past the timeout.
    Failed(String),
}

/// The command from `config.json`, or `None` when it's unset or invalid (logged once). Tests
/// never see the config file.
pub fn configured() -> Option<OnCapture> {
    if cfg!(test) {
        return None;
    }
    let config = crate::handoff::load_config_from(&crate::handoff::default_config_path());
    let argv = config.on_capture?;
    let timeout = config
        .on_capture_timeout_seconds
        .filter(|&s| s > 0)
        .map_or(DEFAULT_TIMEOUT, Duration::from_secs);
    match validate(&argv) {
        Ok(()) => Some(OnCapture { argv, timeout }),
        Err(error) => {
            crate::handoff::log_config_once(format!(
                "config.json: ignoring \"onCapture\": {error}"
            ));
            None
        }
    }
}

/// An argv Scrawl will run: not empty, and starting with an absolute path (no `PATH` lookup).
pub fn validate(argv: &[String]) -> Result<(), String> {
    let Some(program) = argv.first() else {
        return Err("it's empty".to_owned());
    };
    if !Path::new(program).is_absolute() {
        return Err(format!("{program:?} isn't an absolute path"));
    }
    Ok(())
}

/// `argv` with `{bundle}` replaced by `bundle` in its arguments, or `bundle` appended when no
/// argument has it. The program (`argv[0]`) stays exactly as validated.
pub fn command_line(argv: &[String], bundle: &Path) -> Vec<String> {
    let path = bundle.display().to_string();
    let Some((program, args)) = argv.split_first() else {
        return Vec::new();
    };
    let mut line = vec![program.clone()];
    if args.iter().any(|arg| arg.contains(PLACEHOLDER)) {
        line.extend(args.iter().map(|arg| arg.replace(PLACEHOLDER, &path)));
    } else {
        line.extend(args.iter().cloned());
        line.push(path);
    }
    line
}

/// Runs the command for `bundle` and waits for it, at most `timeout`.
pub fn run(on_capture: &OnCapture, bundle: &Path) -> Outcome {
    let argv = command_line(&on_capture.argv, bundle);
    let started = Instant::now();
    let outcome = run_argv(&argv, bundle, on_capture.timeout);
    let took = started.elapsed().as_millis();
    match &outcome {
        Outcome::Delivered { to } => scrawl_log(format!(
            "onCapture: {} delivered{} in {took} ms",
            bundle.display(),
            to.as_deref()
                .map(|to| format!(" to {to}"))
                .unwrap_or_default()
        )),
        Outcome::Failed(error) => {
            scrawl_log(format!(
                "onCapture: {} failed after {took} ms: {error}",
                bundle.display()
            ));
        }
    }
    outcome
}

/// Runs `onCapture` for `bundle` on its own thread, if one is configured, and hands the outcome to
/// `done` there. Returns whether it started.
pub fn spawn(bundle: &Path, done: impl FnOnce(Outcome) + Send + 'static) -> bool {
    let Some(on_capture) = configured() else {
        return false;
    };
    let bundle = bundle.to_path_buf();
    std::thread::Builder::new()
        .name("scrawl-on-capture".into())
        .spawn(move || done(run(&on_capture, &bundle)))
        .is_ok()
}

fn run_argv(argv: &[String], cwd: &Path, timeout: Duration) -> Outcome {
    let Some((program, args)) = argv.split_first() else {
        return Outcome::Failed("no command".to_owned());
    };
    let timeout = timeout.min(MAX_TIMEOUT);
    let mut command = Command::new(program);
    command
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Its own process group, so a timeout also stops whatever it started (`stop`).
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    // No console window flashing up after every capture when the command is a script.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => return Outcome::Failed(format!("couldn't start {program}: {error}")),
    };
    // Read both pipes while it runs, so a chatty command can't fill one and stall.
    let stdout = child.stdout.take().map(Output::read);
    let stderr = child.stderr.take().map(Output::read);
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                stop(&mut child);
                return Outcome::Failed(format!("timed out after {timeout:?}"));
            }
            Err(error) => return Outcome::Failed(format!("couldn't wait for it: {error}")),
        }
    };
    // Something it left running in the background can hold the pipes open; don't wait for that.
    let until = Instant::now() + OUTPUT_GRACE;
    let stdout = stdout.map(|out| out.text(until)).unwrap_or_default();
    let stderr = stderr.map(|err| err.text(until)).unwrap_or_default();
    if status.success() {
        Outcome::Delivered {
            to: first_line(&stdout),
        }
    } else {
        let error = first_line(&stderr).unwrap_or_else(|| "no output".to_owned());
        Outcome::Failed(format!("{status}: {error}"))
    }
}

/// Kills the command and, on Unix, the rest of its process group.
fn stop(child: &mut Child) {
    #[cfg(unix)]
    if let Ok(group) = i32::try_from(child.id()) {
        // The group is the child's own (`process_group(0)`), and the child isn't reaped yet, so
        // its id can't have been reused.
        unsafe { libc::killpg(group, libc::SIGKILL) };
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// One of the command's pipes, read on its own thread until it closes. The first `OUTPUT_CAP`
/// bytes are kept, and the rest is read and dropped so the command never blocks on a full pipe.
struct Output {
    kept: Arc<Mutex<Vec<u8>>>,
    reader: Option<JoinHandle<()>>,
}

impl Output {
    fn read(mut pipe: impl Read + Send + 'static) -> Self {
        let kept = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&kept);
        let reader = std::thread::Builder::new()
            .name("scrawl-on-capture-output".into())
            .spawn(move || {
                let mut chunk = [0_u8; 4096];
                while let Ok(n) = pipe.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    let mut kept = sink.lock().unwrap_or_else(PoisonError::into_inner);
                    let room = OUTPUT_CAP.saturating_sub(kept.len());
                    kept.extend_from_slice(&chunk[..n.min(room)]);
                }
            })
            .ok();
        Self { kept, reader }
    }

    /// What was read once the pipe closed, or by `until` if something still holds it open.
    fn text(self, until: Instant) -> String {
        while self.reader.as_ref().is_some_and(|r| !r.is_finished()) && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(10));
        }
        let kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
        String::from_utf8_lossy(&kept).into_owned()
    }
}

/// The first non-empty line, trimmed, at most 80 characters.
fn first_line(text: &str) -> Option<String> {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty())?;
    Some(if line.chars().count() > 80 {
        format!("{}…", line.chars().take(79).collect::<String>())
    } else {
        line.to_owned()
    })
}

#[cfg(test)]
fn argv(parts: &[&str]) -> Vec<String> {
    parts.iter().map(|p| (*p).to_owned()).collect()
}

/// The parts that never start a process, so they run on every platform's CI.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundle_replaces_the_placeholder_or_goes_last() {
        let dir = Path::new("/tmp/x y.scrawl");
        assert_eq!(
            command_line(&argv(&["/bin/send", "{bundle}", "--quiet"]), dir),
            argv(&["/bin/send", "/tmp/x y.scrawl", "--quiet"])
        );
        assert_eq!(
            command_line(&argv(&["/bin/send", "--path={bundle}/summary.md"]), dir),
            argv(&["/bin/send", "--path=/tmp/x y.scrawl/summary.md"])
        );
        assert_eq!(
            command_line(&argv(&["/bin/send", "--to", "hermes"]), dir),
            argv(&["/bin/send", "--to", "hermes", "/tmp/x y.scrawl"])
        );
        assert_eq!(command_line(&[], dir), Vec::<String>::new());
    }

    #[test]
    fn the_program_is_run_as_validated() {
        // A `{bundle}` in the program's own path is left alone, and doesn't count as placing the
        // bundle: it's still appended.
        let dir = Path::new("/tmp/x.scrawl");
        assert_eq!(
            command_line(&argv(&["/opt/{bundle}/send"]), dir),
            argv(&["/opt/{bundle}/send", "/tmp/x.scrawl"])
        );
        assert_eq!(
            command_line(&argv(&["/opt/{bundle}/send", "{bundle}"]), dir),
            argv(&["/opt/{bundle}/send", "/tmp/x.scrawl"])
        );
    }

    #[test]
    fn only_an_absolute_program_is_run() {
        let absolute = if cfg!(windows) {
            r"C:\Tools\send.exe"
        } else {
            "/usr/local/bin/send"
        };
        assert!(validate(&argv(&[absolute])).is_ok());
        assert!(validate(&argv(&["send", "{bundle}"])).is_err());
        assert!(validate(&argv(&["./send"])).is_err());
        assert!(validate(&[]).is_err());
    }

    #[test]
    fn the_first_line_is_trimmed_and_capped() {
        assert_eq!(
            first_line("\n\n  Claude Code \n"),
            Some("Claude Code".to_owned())
        );
        assert_eq!(first_line("   \n"), None);
        assert_eq!(
            first_line(&"x".repeat(200)).map(|l| l.chars().count()),
            Some(80)
        );
    }
}

/// The parts that run `/bin/sh`: Unix only.
#[cfg(all(test, unix))]
mod process_tests {
    use super::*;
    use std::path::PathBuf;

    fn command(parts: &[&str], timeout: Duration) -> OnCapture {
        OnCapture {
            argv: argv(parts),
            timeout,
        }
    }

    fn bundle() -> PathBuf {
        std::env::temp_dir()
    }

    #[test]
    fn exit_zero_is_delivered_and_its_first_line_names_where() {
        let script = "printf '\\n  Hermes  \\nmore\\n'";
        let outcome = run(
            &command(&["/bin/sh", "-c", script], DEFAULT_TIMEOUT),
            &bundle(),
        );
        assert_eq!(
            outcome,
            Outcome::Delivered {
                to: Some("Hermes".to_owned())
            }
        );
        let quiet = run(&command(&["/usr/bin/true"], DEFAULT_TIMEOUT), &bundle());
        assert_eq!(quiet, Outcome::Delivered { to: None });
    }

    #[test]
    fn the_bundle_path_reaches_the_command_as_one_argument() {
        let dir = std::env::temp_dir().join("on capture test.scrawl");
        std::fs::create_dir_all(&dir).unwrap();
        let outcome = run(
            &command(
                &["/bin/sh", "-c", "printf '%s' \"$1\"", "sh"],
                DEFAULT_TIMEOUT,
            ),
            &dir,
        );
        assert_eq!(
            outcome,
            Outcome::Delivered {
                to: Some(dir.display().to_string())
            }
        );
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn a_failure_reports_its_status_and_first_error_line() {
        let outcome = run(
            &command(
                &["/bin/sh", "-c", "echo 'no route to hermes' >&2; exit 3"],
                DEFAULT_TIMEOUT,
            ),
            &bundle(),
        );
        let Outcome::Failed(error) = outcome else {
            panic!("expected a failure, got {outcome:?}");
        };
        assert!(error.contains("no route to hermes"), "{error}");
        assert!(error.contains('3'), "{error}");
    }

    #[test]
    fn a_missing_program_fails_without_panicking() {
        let outcome = run(&command(&["/nonexistent/send"], DEFAULT_TIMEOUT), &bundle());
        assert!(
            matches!(outcome, Outcome::Failed(ref e) if e.contains("couldn't start")),
            "{outcome:?}"
        );
    }

    #[test]
    fn a_command_past_its_timeout_is_stopped() {
        let started = Instant::now();
        let outcome = run(
            &command(
                &["/bin/sh", "-c", "exec sleep 5", "sh"],
                Duration::from_millis(200),
            ),
            &bundle(),
        );
        assert!(
            matches!(outcome, Outcome::Failed(ref e) if e.contains("timed out")),
            "{outcome:?}"
        );
        assert!(started.elapsed() < Duration::from_secs(3));
    }

    #[test]
    fn a_timeout_also_stops_what_the_command_started() {
        let dir = std::env::temp_dir().join(format!("scrawl-on-capture-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = "sleep 30 & echo $! > \"$1/pid\"; wait";
        let outcome = run(
            &command(&["/bin/sh", "-c", script, "sh"], Duration::from_millis(300)),
            &dir,
        );
        assert!(
            matches!(outcome, Outcome::Failed(ref e) if e.contains("timed out")),
            "{outcome:?}"
        );
        let pid: i32 = std::fs::read_to_string(dir.join("pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        // Once killed it's reaped by launchd (or init); poll rather than race that.
        let gone = (0..100).any(|_| {
            std::thread::sleep(Duration::from_millis(20));
            let alive = unsafe { libc::kill(pid, 0) } == 0;
            !alive
        });
        assert!(gone, "the background sleep ({pid}) is still running");
    }

    #[test]
    fn something_left_running_doesnt_hold_up_the_outcome() {
        let started = Instant::now();
        let outcome = run(
            &command(
                &["/bin/sh", "-c", "sleep 3 & echo Hermes"],
                Duration::from_secs(10),
            ),
            &bundle(),
        );
        assert_eq!(
            outcome,
            Outcome::Delivered {
                to: Some("Hermes".to_owned())
            }
        );
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn a_huge_timeout_is_capped_instead_of_overflowing() {
        let outcome = run(
            &command(&["/usr/bin/true"], Duration::from_secs(u64::MAX)),
            &bundle(),
        );
        assert_eq!(outcome, Outcome::Delivered { to: None });
    }

    #[test]
    fn a_chatty_command_doesnt_stall_on_a_full_pipe() {
        let script = "head -c 1000000 /dev/zero | tr '\\0' x; echo; echo Slack";
        let outcome = run(
            &command(&["/bin/sh", "-c", script], Duration::from_secs(10)),
            &bundle(),
        );
        assert!(matches!(outcome, Outcome::Delivered { .. }), "{outcome:?}");
    }
}
