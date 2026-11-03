//! Installed Storage presentation. Policy and settings persistence stay in Rust.
use std::{path::PathBuf, process::{Command, Stdio}, sync::Arc};
use super::settings::{Action, Event, Transport};

/// Unique across Settings sessions too: late events from a closed session must
/// never match the first window of a newly opened session.
pub(super) fn generation() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

pub(super) fn assets() -> Option<(PathBuf, PathBuf)> {
    let executable = std::env::current_exe().ok()?;
    let parent = executable.parent()?;
    #[cfg(target_os = "macos")]
    let root = parent.parent()?.join("Resources/settings");
    #[cfg(not(target_os = "macos"))]
    let root = parent.parent()?.join("share/apparent/settings");
    let development = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("capture/desktop/ui/frontend");
    let mut candidates = vec![(root.join("app"), root.join("runtime"))];
    if executable.starts_with(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target")) {
        candidates.push((development.clone(), development.join("node_modules/electron/dist")));
    }
    for (app, runtime) in candidates {
        #[cfg(target_os = "macos")]
        let runtime = runtime.join("Electron.app/Contents/MacOS/Electron");
        #[cfg(not(target_os = "macos"))]
        let runtime = runtime.join("electron");
        if runtime.is_file() && app.join("dist/settings.html").is_file() && app.join("electron/main.cjs").is_file() {
            return Some((app, runtime));
        }
    }
    None
}

pub(super) fn open(events: Arc<dyn Fn(Event) + Send + Sync>) -> anyhow::Result<Transport> {
    open_mode(events, None)
}

pub(super) fn open_connect(events: Arc<dyn Fn(Event) + Send + Sync>, command: super::connect::Command) -> anyhow::Result<Transport> {
    open_mode(events, Some(command))
}

fn open_mode(events: Arc<dyn Fn(Event) + Send + Sync>, connect: Option<super::connect::Command>) -> anyhow::Result<Transport> {
    use std::io::BufRead;
    let (app, runtime) = assets().ok_or_else(|| anyhow::anyhow!("Packaged Settings unavailable"))?;
    anyhow::ensure!(connect.is_none() || app.join("electron/preloads/connect.cjs").is_file(), "Packaged Connect Settings unavailable");
    let mut process = Command::new(runtime);
    crate::capture::desktop::workflow::error_reporting::configure_child(&mut process);
    prepare_electron_environment(&mut process);
    // Settings must share capture's XWayland input/stacking route: a native
    // Wayland window can receive a Shift-click underneath the X11 ink overlay.
    #[cfg(target_os = "linux")]
    process.arg("--ozone-platform=x11");
    process.arg(app).arg(if connect.is_some() { "--settings-connect" } else { "--settings-storage" }).arg("--settings-parent");
    if let Some(command) = connect {
        let command = serde_json::to_value(command)?;
        process.arg(format!("--connect-action={}", command.as_str().unwrap()));
    }
    let mut child = process.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    #[cfg(target_os = "linux")]
    let child = crate::capture::ui_process::UiChild::settings(child);
    #[cfg(not(target_os = "linux"))]
    let child = crate::capture::ui_process::UiChild::new(child);
    std::thread::spawn(move || {
        if let Some(stdout) = stdout {
            for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                // This presentation may navigate or reveal the effective folder;
                // it cannot invoke other native Settings actions through this pipe.
                if let Ok(action) = serde_json::from_str::<Action>(&line)
                    && (matches!(action, Action::Navigate { .. } | Action::Close)
                        || connect.is_none() && matches!(action, Action::ShowCaptures))
                {
                    events(Event::Action(action));
                }
            }
        }
        events(Event::Exited);
    });
    Ok(Transport::Helper { child, stdin })
}

fn prepare_electron_environment(process: &mut Command) {
    // Retain the compositor socket for portal/native helpers. The Settings
    // window itself explicitly uses XWayland to share capture's input route.
    #[cfg(target_os = "linux")]
    if let Some(display) = crate::capture::desktop::linux::wayland_display() {
        process.env("WAYLAND_DISPLAY", display);
    }
    #[cfg(not(target_os = "linux"))]
    let _ = process;
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn electron_environment_subprocess() {
        let Ok(expected) = std::env::var("APPARENT_PRESENTATION_DISPLAY_TEST") else { return; };
        assert!(std::env::var_os("WAYLAND_DISPLAY").is_none());
        let mut child = Command::new("/bin/sh");
        child.args(["-c", "printf '%s' \"${WAYLAND_DISPLAY-unset}\""]);
        prepare_electron_environment(&mut child);
        let output = child.output().unwrap();
        assert!(output.status.success());
        assert_eq!(String::from_utf8(output.stdout).unwrap(), expected);
        println!("display-fixture-asserted");
    }

    #[test]
    fn electron_child_restores_saved_wayland_display_and_preserves_x11() {
        for display in [Some("wayland-synthetic"), None] {
            let mut child = Command::new(std::env::current_exe().unwrap());
            child.args(["--exact", "capture::desktop::settings::presentation::tests::electron_environment_subprocess", "--nocapture", "--test-threads=1"])
                .env_remove("WAYLAND_DISPLAY").env_remove("APPARENT_WAYLAND_DISPLAY")
                .env("APPARENT_PRESENTATION_DISPLAY_TEST", display.unwrap_or("unset"));
            if let Some(display) = display { child.env("APPARENT_WAYLAND_DISPLAY", display); }
            let result = child.output().unwrap();
            assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stdout));
            assert!(String::from_utf8_lossy(&result.stdout).contains("display-fixture-asserted"), "subprocess did not execute the display assertion");
        }
    }
}

pub(super) fn retire(transport: Transport) {
    match transport {
        Transport::InProcess(sender) => drop(sender),
        Transport::Helper { mut child, stdin } => {
            drop(stdin);
            // Reap without blocking capture or the UI thread. A closing child
            // cannot hold Settings navigation or application shutdown hostage.
            std::thread::spawn(move || {
                for _ in 0..20 {
                    if !matches!(child.try_wait(), Ok(None)) { return; }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                let _ = child.kill();
                let _ = child.wait();
            });
        }
    }
}
