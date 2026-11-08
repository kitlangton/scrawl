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
    use std::io::BufRead;
    let (app, runtime) = assets().ok_or_else(|| anyhow::anyhow!("Packaged Storage Settings unavailable"))?;
    // Electron selects its display backend before loading main.cjs. Match the
    // Linux parent's XWayland choice while preserving a live Wayland opt-in.
    let x11 = cfg!(target_os = "linux") && std::env::var_os("WAYLAND_DISPLAY").is_none();
    let mut child = command(runtime, app, x11)
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let child = crate::capture::ui_process::UiChild::new(child);
    std::thread::spawn(move || {
        if let Some(stdout) = stdout {
            for line in std::io::BufReader::new(stdout).lines().map_while(Result::ok) {
                // This presentation may navigate or reveal the effective folder;
                // it cannot invoke other native Settings actions through this pipe.
                if let Ok(action) = serde_json::from_str::<Action>(&line)
                    && matches!(action, Action::Navigate { .. } | Action::ShowCaptures | Action::Close)
                {
                    events(Event::Action(action));
                }
            }
        }
        events(Event::Exited);
    });
    Ok(Transport::Helper { child, stdin })
}

fn command(runtime: PathBuf, app: PathBuf, x11: bool) -> Command {
    let mut command = Command::new(runtime);
    if x11 { command.arg("--ozone-platform=x11"); }
    command.arg(app).arg("--settings-storage").arg("--settings-parent");
    command
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use super::*;

    #[test]
    fn settings_child_gets_its_native_backend_before_the_application_entry() {
        for x11 in [false, true] {
            let child = command("/runtime/electron".into(), "/settings/app".into(), x11);
            assert_eq!(child.get_program(), "/runtime/electron");
            let args: Vec<_> = child.get_args().map(|arg| arg.to_str().unwrap()).collect();
            let mut expected = vec![];
            if x11 { expected.push("--ozone-platform=x11"); }
            expected.extend(["/settings/app", "--settings-storage", "--settings-parent"]);
            assert_eq!(args, expected);
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
