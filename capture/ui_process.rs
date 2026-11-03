//! Capture-internal ownership of separately rendered desktop windows.
use std::{collections::HashMap, process::{Child, ExitStatus}, sync::{Arc, Mutex, Weak}};

static CHILDREN: Mutex<Option<HashMap<u32, Weak<Mutex<Child>>>>> = Mutex::new(None);

/// Keeps ownership tied to the exact child handle, rather than an executable name
/// or a PID that could be reused after the child is reaped.
pub(crate) struct UiChild(Arc<Mutex<Child>>);

impl UiChild {
    pub(crate) fn new(child: Child) -> Self {
        let pid = child.id();
        let child = Arc::new(Mutex::new(child));
        CHILDREN.lock().unwrap().get_or_insert_with(HashMap::new)
            .insert(pid, Arc::downgrade(&child));
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
            && children.get(&pid).is_some_and(|entry| entry.ptr_eq(&Arc::downgrade(&self.0)))
        {
            children.remove(&pid);
        }
    }
}

pub(crate) fn owns(pid: i32) -> bool {
    let child = CHILDREN.lock().unwrap().as_ref()
        .and_then(|children| children.get(&(pid as u32))).and_then(Weak::upgrade);
    // try_wait caches a completed status. Even if the owner has already reaped
    // this child, a different process reusing its PID cannot become ours.
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
