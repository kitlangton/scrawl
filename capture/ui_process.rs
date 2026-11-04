//! Capture-internal ownership of separately rendered desktop windows.
use std::{collections::HashMap, process::{Child, ExitStatus}, sync::{Arc, Mutex, Weak}};

static CHILDREN: Mutex<Option<HashMap<u32, (Weak<Mutex<Child>>, bool)>>> = Mutex::new(None);

/// Override inherited shell metadata with the identity of this running backend.
/// UI helpers must never resolve a checkout or identify their Electron runtime.
pub(crate) fn configure_build_identity(command: &mut std::process::Command) {
    command.env("APPARENT_RUNNING_BUILD_LABEL", env!("APPARENT_VERSION_LABEL"));
}
#[cfg(target_os = "linux")]
static SETTINGS_WINDOWS: Mutex<Option<HashMap<u32, Weak<()>>>> = Mutex::new(None);

/// Exact Linux Settings window identity. Keeping this guard in the view binds
/// annotation admission to its lifetime, including window-ID reuse.
#[cfg(target_os = "linux")]
pub(crate) struct SettingsWindow { id: u32, identity: Arc<()> }
#[cfg(target_os = "linux")]
impl SettingsWindow {
    pub(crate) fn new(id: u32) -> Self {
        let identity = Arc::new(());
        SETTINGS_WINDOWS.lock().unwrap().get_or_insert_with(HashMap::new)
            .insert(id, Arc::downgrade(&identity));
        Self { id, identity }
    }
}
#[cfg(target_os = "linux")]
impl Drop for SettingsWindow {
    fn drop(&mut self) {
        if let Some(windows) = SETTINGS_WINDOWS.lock().unwrap().as_mut()
            && windows.get(&self.id).is_some_and(|entry| entry.ptr_eq(&Arc::downgrade(&self.identity))) {
            windows.remove(&self.id);
        }
    }
}

/// Keeps ownership tied to the exact child handle, rather than an executable name
/// or a PID that could be reused after the child is reaped.
pub(crate) struct UiChild(Arc<Mutex<Child>>);

impl UiChild {
    pub(crate) fn new(child: Child) -> Self {
        Self::with_settings_role(child, false)
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn settings(child: Child) -> Self {
        Self::with_settings_role(child, true)
    }

    fn with_settings_role(child: Child, settings: bool) -> Self {
        let pid = child.id();
        let child = Arc::new(Mutex::new(child));
        CHILDREN.lock().unwrap().get_or_insert_with(HashMap::new)
            .insert(pid, (Arc::downgrade(&child), settings));
        Self(child)
    }

    pub(crate) fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.0.lock().unwrap().try_wait()
    }

    pub(crate) fn kill(&mut self) -> std::io::Result<()> {
        self.0.lock().unwrap().kill()
    }

    pub(crate) fn wait(&mut self) -> std::io::Result<ExitStatus> {
        loop {
            if let Some(status) = self.try_wait()? { return Ok(status); }
            // Never hold the child lock while waiting for process termination:
            // capture's ownership check must remain responsive.
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

impl Drop for UiChild {
    fn drop(&mut self) {
        let pid = self.0.lock().unwrap().id();
        if let Some(children) = CHILDREN.lock().unwrap().as_mut()
            && children.get(&pid).is_some_and(|entry| entry.0.ptr_eq(&Arc::downgrade(&self.0)))
        {
            children.remove(&pid);
        }
    }
}

pub(crate) fn owns(pid: i32) -> bool {
    let child = CHILDREN.lock().unwrap().as_ref()
        .and_then(|children| children.get(&(pid as u32))).and_then(|entry| entry.0.upgrade());
    // try_wait caches a completed status. Even if the owner has already reaped
    // this child, a different process reusing its PID cannot become ours.
    child.is_some_and(|child| matches!(child.lock().unwrap().try_wait(), Ok(None)))
}

/// Linux's Shift overlay takes pointer input before a Settings control can act.
/// Only explicitly registered Settings surfaces may bypass ordinary own-UI
/// telemetry exclusion for that annotation gesture; the pill/editor never do.
#[cfg(target_os = "linux")]
pub(crate) fn settings_subject(pid: i32, window: u32) -> bool {
    if pid == std::process::id() as i32 {
        return SETTINGS_WINDOWS.lock().unwrap().as_ref()
            .and_then(|windows| windows.get(&window)).and_then(Weak::upgrade).is_some();
    }
    let child = CHILDREN.lock().unwrap().as_ref()
        .and_then(|children| children.get(&(pid as u32)))
        .filter(|entry| entry.1).and_then(|entry| entry.0.upgrade());
    child.is_some_and(|child| matches!(child.lock().unwrap().try_wait(), Ok(None)))
}

/// UI children share the application's focus identity. They must never replace
/// the last external receiver used by capture context or explicit Paste.
pub(crate) fn is_external_app(pid: i32) -> bool {
    pid > 0 && pid != std::process::id() as i32 && !owns(pid)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::process::{Command, Stdio};

    #[test]
    #[ignore = "requires built frontend and Linux Xvfb; see frontend/README.md"]
    fn build_identity_electron_rust_roundtrip() {
        let frontend = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("capture/desktop/ui/frontend");
        let mut command = Command::new("xvfb-run");
        command.args(["-a", "node", "--test", "tests/build/identity.smoke.ts"])
            .current_dir(frontend)
            .env("APPARENT_RUNNING_BUILD_LABEL", "0.0.0 · fffffff")
            .env("APPARENT_EXPECTED_BUILD_LABEL", env!("APPARENT_VERSION_LABEL"));
        configure_build_identity(&mut command);
        let output = command.output().unwrap();
        assert!(output.status.success(), "{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn settings_subjects_follow_exact_window_and_child_lifetimes() {
        let pid = std::process::id() as i32;
        let first = SettingsWindow::new(0xfeed0100);
        assert!(settings_subject(pid, 0xfeed0100));
        assert!(!settings_subject(pid, 0xfeed0101));
        assert!(!settings_subject(pid + 1, 0xfeed0100));
        let replacement = SettingsWindow::new(0xfeed0100);
        drop(first);
        assert!(settings_subject(pid, 0xfeed0100));
        drop(replacement);
        assert!(!settings_subject(pid, 0xfeed0100));
        let child = Command::new("/bin/cat").stdin(Stdio::piped()).stdout(Stdio::null()).spawn().unwrap();
        let child_pid = child.id() as i32;
        let mut settings = UiChild::settings(child);
        assert!(settings_subject(child_pid, 42));
        settings.kill().unwrap();
        settings.wait().unwrap();
        assert!(!settings_subject(child_pid, 42), "a retained handle cannot admit a reused PID");
        let child = Command::new("/bin/cat").stdin(Stdio::piped()).stdout(Stdio::null()).spawn().unwrap();
        let child_pid = child.id() as i32;
        let mut ordinary = UiChild::new(child);
        assert!(!settings_subject(child_pid, 42), "other UI children remain excluded");
        ordinary.kill().unwrap();
        ordinary.wait().unwrap();
    }
    #[test]
    fn external_receiver_excludes_the_live_owned_renderer() {
        assert!(!is_external_app(0));
        assert!(!is_external_app(std::process::id() as i32));
        let child = Command::new("/bin/cat").stdin(Stdio::piped())
            .stdout(Stdio::null()).spawn().unwrap();
        let pid = child.id() as i32;
        assert!(is_external_app(pid));
        let mut owned = UiChild::new(child);
        assert!(!is_external_app(pid));
        owned.kill().unwrap();
        owned.wait().unwrap();
    }
}
