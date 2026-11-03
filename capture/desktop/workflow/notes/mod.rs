//! The notes-only Electron surface. Its private pipe exposes no shell or filesystem commands.
pub(crate) mod audio;
use crate::capture::engine::runtime::recording::notes::Store;
use crate::capture::engine::runtime::{LiveOverlayEvent, SharedCaptureTelemetry};
use anyhow::{Context, Result, ensure};
use serde::Deserialize;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, atomic::Ordering};

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "action", rename_all = "camelCase", deny_unknown_fields)]
pub enum CommandMessage {
    Ready,
    Hidden,
    Flight { active: bool },
    Outside,
    Finished,
    Draft {
        pin: u32,
        text: String,
    },
    Save {
        pin: u32,
        text: String,
    },
    Delete {
        pin: u32,
    },
    Edit {
        pin: u32,
    },
    Dismiss,
    List,
    Windows {
        ids: Vec<u32>,
    },
    Surface {
        id: u32,
        rect: [i32; 4],
        focus: bool,
    },
}
#[derive(Clone, Debug)]
pub struct Anchor {
    pub pin: u32,
    pub t: u64,
    pub x: f32,
    pub y: f32,
    pub screen_x: f64,
    pub screen_y: f64,
    pub focus: u64,
    pub owner: u32,
}
#[derive(Debug)]
enum Message {
    Open(Anchor),
    Undo(u32, flume::Sender<bool>),
    Action(CommandMessage),
    Stop,
    BadgePosition([i32; 4]),
    Finish(flume::Sender<std::result::Result<(), String>>),
}
pub struct Session {
    sender: flume::Sender<Message>,
    stopping: Arc<std::sync::atomic::AtomicBool>,
    finishing: bool,
    badge_position: Option<[i32; 4]>,
}

// Chromium can still write its profile during Electron's `quit` event. Rust owns
// the directory so cleanup happens after the child has actually been reaped.
struct Profile(std::path::PathBuf);
impl Profile {
    fn create() -> Result<Self> {
        use std::os::unix::fs::DirBuilderExt;
        // Keep Chromium's Unix socket below macOS's path limit in its long TMPDIR.
        let nonce = super::agent_capture::new_id()?;
        let path = std::env::temp_dir().join(format!("an-{}", &nonce[..16]));
        std::fs::DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self(path))
    }
}
impl Drop for Profile {
    fn drop(&mut self) {
        for attempt in 0..5 {
            match std::fs::remove_dir_all(&self.0) {
                Ok(()) => return,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return,
                Err(error) if error.raw_os_error() == Some(libc::ENOTEMPTY) && attempt < 4 => {
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                Err(error) => {
                    crate::capture::engine::runtime::apparent_log(format!("Could not clean notes UI profile: {error}"));
                    return;
                }
            }
        }
    }
}

impl Session {
    pub fn start(
        directory: &Path,
        telemetry: Arc<SharedCaptureTelemetry>,
        frame: crate::shared::model::TargetFrame,
    ) -> Result<Self> {
        let resources = super::electron::resolve()?;
        let (binary, root) = (resources.executable, resources.root);
        let profile = Profile::create()?;
        use std::os::unix::fs::OpenOptionsExt;
        let errors = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(directory.join("notes-ui.log"))?;
        let mut command = Command::new(binary);
        // The existing Linux overlay mechanics use XWayland global coordinates and XIDs.
        // GPUI intentionally hides WAYLAND_DISPLAY; Electron must use that same X11 route.
        #[cfg(target_os = "linux")]
        command.arg("--ozone-platform=x11");
        let mut child = command
            .arg(root.join("electron/notes/main.cjs"))
            .arg(format!("--notes-profile={}", profile.0.to_str().context("Invalid notes profile path")?))
            .arg(format!(
                "--capture-frame={},{},{},{}",
                frame.x, frame.y, frame.width, frame.height
            ))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::from(errors))
            .spawn()
            .context("Could not open notes UI")?;
        let mut input = child.stdin.take().unwrap();
        if let Err(error) = nonblocking(&input) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
        let note_pid = child.id();
        telemetry
            .note_ui_pid
            .store(u64::from(note_pid), Ordering::Release);
        #[cfg(target_os = "linux")]
        crate::capture::desktop::platform::register_note_surfaces(note_pid);
        let output = child.stdout.take().unwrap();
        let (sender, receiver) = flume::bounded(64);
        let reader = sender.clone();
        std::thread::spawn(move || {
            let mut stream = BufReader::new(output);
            loop {
                let mut line = Vec::new();
                match std::io::Read::take(&mut stream, 65_537).read_until(b'\n', &mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) if line.len() > 65_536 || !line.ends_with(b"\n") => break,
                    _ => {
                        if let Ok(action) = serde_json::from_slice(&line) {
                            if reader.send(Message::Action(action)).is_err() {
                                break;
                            }
                        } else {
                            break;
                        }
                    }
                }
            }
            let _ = reader.send(Message::Stop);
        });
        let mut store = Store::new(directory);
        let stopping = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let stopped = Arc::clone(&stopping);
        std::thread::spawn(move || {
            let mut flying = false;
            let mut current = None;
            let mut focus = 0;
            let mut owner = 0;
            let mut position = [frame.x + frame.width / 2.0, frame.y + 80.0];
            let mut mode = "badge";
            let mut ready = false;
            let mut own_windows = Vec::new();
            let mut completion: Option<flume::Sender<std::result::Result<(), String>>> = None;
            let mut finish_deadline = None;
            let mut drained = false;
            let mut badge_position = None;
            let mut failed_writes = std::collections::BTreeSet::new();
            #[cfg(target_os = "linux")]
            let mut stack_checked_at = std::time::Instant::now();
            loop {
                let timeout = std::time::Duration::from_millis(100);
                #[cfg(target_os = "linux")]
                let timeout = {
                    // Draft traffic must not multiply synchronous X server queries.
                    if stack_checked_at.elapsed() >= timeout {
                        crate::capture::desktop::platform::maintain_note_surfaces(note_pid);
                        stack_checked_at = std::time::Instant::now();
                    }
                    timeout.saturating_sub(stack_checked_at.elapsed())
                };
                let message = match receiver.recv_timeout(timeout) {
                    Ok(message) => message,
                    Err(flume::RecvTimeoutError::Timeout)
                        if !stopped.load(Ordering::Acquire)
                            && finish_deadline.is_none_or(|d| std::time::Instant::now() < d) =>
                    {
                        continue;
                    }
                    Err(_) => break,
                };
                if stopped.load(Ordering::Acquire)
                    || matches!(message, Message::Stop)
                    || finish_deadline.is_some_and(|d| std::time::Instant::now() >= d)
                {
                    break;
                }
                let write_pin = match &message {
                    Message::Undo(pin, _) => Some(*pin),
                    Message::Action(
                        CommandMessage::Save { pin, .. } | CommandMessage::Delete { pin },
                    ) => Some(*pin),
                    _ => None,
                };
                let result: Result<()> = (|| {
                    match message {
                        Message::Stop => return Ok(()),
                        Message::BadgePosition(point) => badge_position = Some(point),
                        Message::Finish(reply) => {
                            completion = Some(reply);
                            finish_deadline =
                                Some(std::time::Instant::now() + std::time::Duration::from_secs(6));
                            write_message(
                                &mut input,
                                &serde_json::json!({"finishing": true}),
                                &stopped,
                                std::time::Duration::from_secs(2),
                            )?;
                        }
                        Message::Undo(pin, reply) => {
                            let removed = store.undo(pin);
                            let _ = reply.send(removed.is_ok());
                            removed?;
                            if current == Some(pin) { current = None; mode = "badge"; }
                            let _ = telemetry.ui_sender.try_send(LiveOverlayEvent::RequestSnapshot);
                        }
                        Message::Open(a) => {
                            store.open(a.pin, a.t, a.x, a.y)?;
                            owner = a.owner;
                            focus = a.focus;
                            current = Some(a.pin);
                            position = [a.screen_x, a.screen_y];
                            mode = "editor";
                        }
                        Message::Action(action) => match action {
                            CommandMessage::Ready => ready = true,
                            CommandMessage::Finished => {
                                ensure!(completion.is_some(), "Unexpected finish receipt");
                                drained = true;
                            }
                            CommandMessage::Outside => {
                                store.dismiss();
                                focus = 0;
                                owner = 0;
                                mode = "badge";
                            }
                            CommandMessage::Flight { active } => {
                                flying = active && mode == "badge" && completion.is_none();
                                if let Some(id) = own_windows.first() {
                                    let mut passthrough = telemetry.passthrough_window_ids.lock()
                                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                                    if flying {
                                        if !passthrough.contains(id) { passthrough.push(*id); }
                                    } else { passthrough.retain(|existing| existing != id); }
                                }
                            }
                            CommandMessage::Hidden => {
                                if mode == "badge" {
                                    let _ = telemetry
                                        .ui_sender
                                        .try_send(LiveOverlayEvent::NoteClosed { focus, owner });
                                }
                            }
                            CommandMessage::Draft { pin, text } => {
                                ensure!(current == Some(pin), "Stale note");
                                store.draft(pin, text)?;
                            }
                            CommandMessage::Save { pin, text } => {
                                ensure!(current == Some(pin), "Stale note");
                                store.save(pin, text)?;
                                mode = "badge";
                            }
                            CommandMessage::Delete { pin } => {
                                store.delete(pin)?;
                                mode = "list";
                            }
                            CommandMessage::Edit { pin } => {
                                ensure!(
                                    store.drafts().iter().any(|n| n.pin == pin),
                                    "Unknown note"
                                );
                                current = Some(pin);
                                mode = "editor";
                            }
                            CommandMessage::Dismiss => {
                                store.dismiss();
                                mode = "badge";
                            }
                            CommandMessage::List => mode = "list",
                            CommandMessage::Surface {
                                id,
                                rect,
                                focus: wants_focus,
                            } => {
                                ensure!(
                                    own_windows.contains(&id)
                                        && rect[2] > 0
                                        && rect[3] > 0
                                        && rect[2] <= 8192
                                        && rect[3] <= 8192,
                                    "Invalid notes surface"
                                );
                                if completion.is_none()
                                    && (mode != "badge" || flying || own_windows.first() != Some(&id))
                                {
                                    #[cfg(target_os = "linux")]
                                    crate::capture::desktop::platform::place_note_surface(
                                        id,
                                        note_pid,
                                        rect,
                                        wants_focus,
                                        own_windows.first() == Some(&id),
                                        flying && own_windows.first() == Some(&id),
                                    );
                                }
                            }
                            CommandMessage::Windows { ids } => {
                                ensure!(ids.len() <= 2, "Invalid notes windows");
                                // This message precedes Ready on the same pipe: native
                                // setup completes while both Electron windows are hidden.
                                #[cfg(target_os = "linux")]
                                for &id in &ids {
                                    ensure!(crate::capture::desktop::platform::prepare_note_surface(id, note_pid), "Could not prepare notes surface");
                                }
                                own_windows = ids.clone();
                                telemetry
                                    .excluded_window_ids
                                    .lock()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                    .extend(ids);
                            }
                        },
                    }
                    Ok(())
                })();
                if let Some(pin) = write_pin {
                    if result.is_ok() {
                        failed_writes.remove(&pin);
                    } else {
                        failed_writes.insert(pin);
                    }
                }
                if drained {
                    break;
                }
                telemetry
                    .notes_editing
                    .store(mode != "badge", Ordering::Release);
                if ready {
                    let snapshot = serde_json::json!({ "mode": mode, "current": current, "position": position, "badgeRect": badge_position,
                        "notes": store.notes(), "drafts": store.drafts(), "error": result.err().map(|_| "Could not save this note. Your text is kept; try again.") });
                    if write_message(
                        &mut input,
                        &snapshot,
                        &stopped,
                        std::time::Duration::from_secs(2),
                    )
                    .is_err()
                    {
                        break;
                    }
                }
            }
            if !stopped.load(Ordering::Acquire) && !drained {
                let _ = telemetry
                    .ui_sender
                    .try_send(LiveOverlayEvent::NotesUnavailable);
            }
            if let Some(id) = own_windows.first() {
                telemetry.passthrough_window_ids.lock().unwrap_or_else(std::sync::PoisonError::into_inner).retain(|existing| existing != id);
            }
            telemetry.notes_editing.store(false, Ordering::Release);
            drop(input); // EOF asks Electron to close its windows.
            let cutoff = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while child.try_wait().ok().flatten().is_none() && std::time::Instant::now() < cutoff {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            let _ = child.kill();
            let _ = child.wait();
            drop(profile);
            telemetry
                .excluded_window_ids
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .retain(|id| !own_windows.contains(id));
            let _ = telemetry
                .ui_sender
                .try_send(LiveOverlayEvent::RequestSnapshot);
            #[cfg(target_os = "linux")]
            crate::capture::desktop::platform::clear_note_surfaces(note_pid);
            let _ = telemetry.note_ui_pid.compare_exchange(
                u64::from(note_pid),
                0,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
            if let Some(reply) = completion {
                let _ = reply.send(if drained && failed_writes.is_empty() {
                    Ok(())
                } else {
                    Err("Notes did not finish saving; recording assets were retained.".into())
                });
            }
        });
        Ok(Self {
            sender,
            stopping,
            finishing: false,
            badge_position: None,
        })
    }
    pub fn undo(&self, pin: u32) -> bool {
        let (reply, result) = flume::bounded(1);
        self.sender.try_send(Message::Undo(pin, reply)).is_ok()
            && result.recv_timeout(std::time::Duration::from_secs(2)).unwrap_or(false)
    }

    pub fn outside(&self) {
        let _ = self.sender.try_send(Message::Action(CommandMessage::Outside));
    }

    pub fn position_badge(&mut self, point: [f64; 4]) {
        let point = point.map(|v| v.round() as i32);
        if self.badge_position != Some(point)
            && self.sender.try_send(Message::BadgePosition(point)).is_ok()
        {
            self.badge_position = Some(point);
        }
    }

    pub fn finish(mut self) -> flume::Receiver<std::result::Result<(), String>> {
        let (reply, result) = flume::bounded(1);
        self.finishing = true;
        let sender = self.sender.clone();
        // A full draft queue must not block the GUI thread. The child barrier orders
        // accepted renderer commands before Finished on the same private stdout pipe.
        std::thread::spawn(move || {
            let _ = sender.send(Message::Finish(reply));
        });
        result
    }
    /// A Shift gesture leaves typing without selecting another app, so retain
    /// the original focus token (unlike an actual outside click).
    pub fn resume_drawing(&self) {
        let _ = self.sender.try_send(Message::Action(CommandMessage::Dismiss));
    }
    pub fn open(&self, anchor: Anchor) {
        let _ = self.sender.try_send(Message::Open(anchor));
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        if self.finishing {
            return;
        }
        self.stopping.store(true, Ordering::Release);
        let _ = self.sender.try_send(Message::Stop);
    }
}

// A stopped renderer must not strand the actor inside a full OS pipe, preventing cleanup.
fn nonblocking(pipe: &std::process::ChildStdin) -> Result<()> {
    use std::os::fd::AsRawFd;
    let fd = pipe.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    ensure!(
        flags >= 0 && unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } >= 0,
        "Could not bound notes transport"
    );
    Ok(())
}
fn write_message(
    input: &mut std::process::ChildStdin,
    value: &serde_json::Value,
    stopping: &std::sync::atomic::AtomicBool,
    timeout: std::time::Duration,
) -> Result<()> {
    let mut data = serde_json::to_vec(value)?;
    data.push(b'\n');
    let deadline = std::time::Instant::now() + timeout;
    let mut remaining = data.as_slice();
    while !remaining.is_empty() {
        ensure!(
            !stopping.load(Ordering::Acquire) && std::time::Instant::now() < deadline,
            "Notes transport stopped reading"
        );
        match input.write(remaining) {
            Ok(0) => anyhow::bail!("Notes transport closed"),
            Ok(n) => remaining = &remaining[n..],
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(5))
            }
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
#[cfg(test)]
mod transport_tests {
    use super::*;
    #[test]
    fn profile_is_private_and_cleaned_after_the_child_finishes_writing() {
        use std::os::unix::fs::PermissionsExt;
        let profile = Profile::create().unwrap();
        let path = profile.0.clone();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o700);
        let mut child = Command::new("sh").args(["-c", "printf late > \"$1/cache\"", "profile-writer"])
            .arg(&path).spawn().unwrap();
        assert!(child.wait().unwrap().success());
        assert_eq!(std::fs::read(path.join("cache")).unwrap(), b"late");
        drop(profile);
        assert!(!path.exists());
    }
    #[test]
    fn stopped_reader_times_out_and_child_can_be_reaped() {
        let mut child = Command::new("sleep")
            .arg("30")
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = child.stdin.take().unwrap();
        let setup = nonblocking(&input);
        let start = std::time::Instant::now();
        let result = setup.and_then(|_| {
            write_message(
                &mut input,
                &serde_json::json!({"snapshot":"x".repeat(2 * 1024 * 1024)}),
                &std::sync::atomic::AtomicBool::new(false),
                std::time::Duration::from_millis(30),
            )
        });
        let _ = child.kill();
        child.wait().unwrap();
        assert!(result.is_err());
        assert!(start.elapsed() < std::time::Duration::from_secs(2));
    }
}
