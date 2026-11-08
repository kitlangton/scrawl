//! No-tray presentation. Capture admission remains in the existing coordinator.
use super::super::MenuAction;
use gpui::{App, AppContext, Context, IntoElement, Window, div, prelude::*, px};
use std::{
    io::{BufRead, Read, Write},
    process::Stdio,
    time::{Duration, Instant},
};

pub(super) struct Controls {
    sender: flume::Sender<MenuAction>,
    child: Option<(
        crate::capture::ui_process::UiChild,
        std::process::ChildStdin,
    )>,
    events: Option<flume::Receiver<String>>,
    started: Instant,
    ready: bool,
    native: Option<gpui::WindowHandle<Native>>,
    native_sender: flume::Sender<MenuAction>,
    native_commands: flume::Receiver<MenuAction>,
    shown_busy: Option<bool>,
    fallback_pending: bool,
    pipe_failed: Option<Instant>,
    shown_status: Option<Option<String>>,
}

impl Controls {
    pub(super) fn new(sender: flume::Sender<MenuAction>) -> Self {
        let (native_sender, native_commands) = flume::bounded(4);
        Self {
            sender,
            child: None,
            events: None,
            started: Instant::now(),
            ready: false,
            native: None,
            native_sender,
            native_commands,
            shown_busy: None,
            fallback_pending: false,
            pipe_failed: None,
            shown_status: None,
        }
    }

    pub(super) fn show(&mut self, cx: &mut App) {
        self.fallback_pending = false;
        if let Some(window) = self.native {
            if window
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
            {
                return;
            }
            self.native = None;
        }
        if let Some((_, input)) = self.child.as_mut() {
            if writeln!(input, "show").is_ok() {
                return;
            }
        }
        self.retire();
        if self.spawn().is_err() {
            self.fallback(cx);
        }
    }

    fn retire(&mut self) {
        if let Some((mut child, input)) = self.child.take() {
            drop(input);
            std::thread::spawn(move || {
                let _ = child.kill();
                let _ = child.wait();
            });
        }
        self.events = None;
    }

    fn spawn(&mut self) -> anyhow::Result<()> {
        let (root, executable) = crate::capture::desktop::settings::presentation::assets()
            .ok_or_else(|| anyhow::anyhow!("Controls runtime missing"))?;
        anyhow::ensure!(
            root.join("dist/src/controls/index.html").is_file(),
            "Controls assets missing"
        );
        let mut command = std::process::Command::new(executable);
        if std::env::var_os("WAYLAND_DISPLAY").is_none() {
            command.arg("--ozone-platform=x11");
        }
        let mut child = command
            .arg(root)
            .arg("--controls")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let input = child.stdin.take().unwrap();
        // Tiny atomic pipe messages must never wait for a stalled renderer.
        use std::os::fd::AsRawFd;
        let flags = unsafe { libc::fcntl(input.as_raw_fd(), libc::F_GETFL) };
        if flags < 0
            || unsafe { libc::fcntl(input.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }
                < 0
        {
            let _ = child.kill();
            let _ = child.wait();
            anyhow::bail!("Controls pipe unavailable");
        }
        let output = child.stdout.take().unwrap();
        let (tx, rx) = flume::bounded(16);
        std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(output);
            loop {
                let mut line = String::new();
                // A child cannot allocate an unbounded parent-side line.
                match (&mut reader).take(1024).read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) if !line.ends_with('\n') => break,
                    Ok(_) => {
                        let _ = tx.try_send(line);
                    }
                }
            }
        });
        self.child = Some((crate::capture::ui_process::UiChild::new(child), input));
        self.events = Some(rx);
        self.started = Instant::now();
        self.ready = false;
        self.shown_busy = None;
        self.pipe_failed = None;
        self.shown_status = None;
        Ok(())
    }

    pub(super) fn poll(&mut self, busy: bool, cx: &mut App) {
        let status = super::super::current_status_bar_text();
        if let Some(window) = self.native {
            let _ = window.update(cx, |view, _, cx| {
                if view.busy != busy || view.status != status {
                    view.busy = busy;
                    view.status = status.clone();
                    cx.notify();
                }
            });
        }
        let mut dispatched = busy;
        while let Ok(action) = self.native_commands.try_recv() {
            if !dispatched {
                let _ = self.sender.try_send(MenuAction::Controls(Box::new(action)));
                dispatched = true;
            }
        }
        if let Some(rx) = &self.events {
            while let Ok(line) = rx.try_recv() {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&line) {
                    if value == serde_json::json!({"ready": true}) {
                        self.ready = true;
                    }
                    if !dispatched
                        && let Some(action) = value
                            .get("action")
                            .and_then(|v| v.as_str())
                            .and_then(action)
                    {
                        let _ = self.sender.try_send(MenuAction::Controls(Box::new(action)));
                        dispatched = true;
                    }
                }
            }
        }
        self.poll_child(busy);
        if self.fallback_pending && !busy {
            self.fallback_pending = false;
            self.fallback(cx);
        }
    }

    fn poll_child(&mut self, busy: bool) {
        let status = self
            .child
            .as_mut()
            .and_then(|(child, _)| child.try_wait().ok().flatten());
        let failed = status.is_some_and(|status| !status.success() || !self.ready)
            || (self.child.is_some()
                && !self.ready
                && self.started.elapsed() > Duration::from_secs(15));
        if status.is_some() || failed {
            self.retire();
            self.fallback_pending |= failed;
            return;
        }
        if self.shown_busy != Some(busy)
            && let Some((_, input)) = self.child.as_mut()
        {
            if writeln!(input, "{}", if busy { "busy" } else { "idle" }).is_ok() {
                self.shown_busy = Some(busy);
                self.pipe_failed = None;
            } else {
                self.pipe_failed.get_or_insert_with(Instant::now);
            }
        }
        let status = super::super::current_status_bar_text();
        if self.shown_status.as_ref() != Some(&status)
            && let Some((_, input)) = self.child.as_mut()
        {
            if writeln!(input, "status {}", serde_json::to_string(&status).unwrap()).is_ok() {
                self.shown_status = Some(status);
                self.pipe_failed = None;
            } else {
                self.pipe_failed.get_or_insert_with(Instant::now);
            }
        }
        // EOF can precede a clean user-close exit. Give that exit a bounded
        // chance to arrive before interpreting a broken pipe as a crash.
        if self
            .pipe_failed
            .is_some_and(|at| at.elapsed() > Duration::from_secs(2))
        {
            self.retire();
            self.fallback_pending = true;
        }
    }

    fn fallback(&mut self, cx: &mut App) {
        if let Some(window) = self.native {
            if window
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
            {
                return;
            }
        }
        let sender = self.native_sender.clone();
        let bounds = gpui::Bounds::centered(None, gpui::size(px(400.), px(320.)), cx);
        self.native = cx
            .open_window(
                gpui::WindowOptions {
                    window_bounds: Some(gpui::WindowBounds::Windowed(bounds)),
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("Apparent".into()),
                        ..Default::default()
                    }),
                    app_id: Some("dev.kitlangton.apparent".into()),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|_| Native {
                        sender,
                        busy: false,
                        status: super::super::current_status_bar_text(),
                    })
                },
            )
            .ok();
    }
}

fn action(value: &str) -> Option<MenuAction> {
    Some(match value {
        "record" => MenuAction::ToggleCapture,
        "choose" => MenuAction::ChooseTarget,
        "latest" => MenuAction::OpenLatest,
        "settings" => MenuAction::Settings,
        _ => return None,
    })
}

struct Native {
    sender: flume::Sender<MenuAction>,
    busy: bool,
    status: Option<String>,
}
impl gpui::Render for Native {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let mut root = div()
            .size_full()
            .flex()
            .flex_col()
            .gap_4()
            .p_6()
            .bg(gpui::rgb(0xfaf9f6))
            .text_color(gpui::rgb(0x222222))
            .child("Apparent controls");
        if let Some(status) = &self.status {
            root = root.child(status.clone());
        }
        if self.busy {
            return root.child("Capture is busy. Use the recording controls or shortcut.");
        }
        for (id, label) in [
            ("record", "Record screen"),
            ("choose", "Choose what to record…"),
            ("latest", "Open latest capture"),
            ("settings", "Settings…"),
        ] {
            let sender = self.sender.clone();
            root = root.child(div().id(id).cursor_pointer().child(label).on_click(
                move |_, _, _| {
                    let _ = sender.try_send(action(id).unwrap());
                },
            ));
        }
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn controls_recheck_busy_at_dispatch_without_changing_tray_toggle() {
        use crate::capture::desktop::admit_controls_action;
        let queued = MenuAction::Controls(Box::new(MenuAction::ToggleCapture));
        // The controls were clicked while idle, but another input started a
        // capture before the coordinator reached the menu queue.
        assert_eq!(admit_controls_action(queued, true), None);
        assert_eq!(
            admit_controls_action(MenuAction::ToggleCapture, true),
            Some(MenuAction::ToggleCapture)
        );
        for action in [
            MenuAction::ChooseTarget,
            MenuAction::OpenLatest,
            MenuAction::Settings,
        ] {
            assert_eq!(
                admit_controls_action(MenuAction::Controls(Box::new(action)), true),
                None
            );
        }
        assert_eq!(
            admit_controls_action(
                MenuAction::Controls(Box::new(MenuAction::ToggleCapture)),
                false
            ),
            Some(MenuAction::ToggleCapture)
        );
    }

    #[test]
    fn clean_close_with_a_busy_update_does_not_request_native_fallback() {
        let (sender, _) = flume::bounded(4);
        let mut controls = Controls::new(sender);
        let mut child = std::process::Command::new("/bin/sh")
            .args(["-c", "exec 0<&-; printf 'closed\\n'; sleep 0.1; exit 0"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let mut output = std::io::BufReader::new(child.stdout.take().unwrap());
        let mut closed = String::new();
        output.read_line(&mut closed).unwrap();
        assert_eq!(closed, "closed\n");
        controls.child = Some((crate::capture::ui_process::UiChild::new(child), input));
        controls.ready = true;
        controls.poll_child(true);
        assert!(
            !controls.fallback_pending,
            "pipe EOF is not proof of a crash"
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while controls.child.is_some() && Instant::now() < deadline {
            controls.poll_child(false);
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(controls.child.is_none());
        assert!(
            !controls.fallback_pending,
            "successful dismissal must stay closed"
        );
    }

    #[test]
    fn retiring_a_controls_child_terminates_and_reaps_the_owned_process() {
        let (sender, _) = flume::bounded(4);
        let mut controls = Controls::new(sender);
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("60")
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id();
        let input = child.stdin.take().unwrap();
        controls.child = Some((crate::capture::ui_process::UiChild::new(child), input));
        assert!(crate::capture::ui_process::owns(pid as i32));
        controls.retire();
        let deadline = Instant::now() + Duration::from_secs(2);
        let process = std::path::PathBuf::from(format!("/proc/{pid}"));
        while process.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !process.exists(),
            "owned child must be reaped, not left as a zombie"
        );
        assert!(!crate::capture::ui_process::owns(pid as i32));
        assert!(controls.child.is_none());
    }
}
