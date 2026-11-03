//! One capture-bound consent surface. Only Rust admits sharing and clipboard effects.
use super::*;
use std::{
    io::{BufRead, Write},
    process::{ChildStdin, Command, Stdio},
    sync::mpsc,
};

pub(crate) struct Popup {
    pub id: String,
    bundle: PathBuf,
    inode: (u64, u64),
    generation: u64,
    owner: u32,
    capture: String,
    commands: mpsc::Receiver<String>,
    input: Option<ChildStdin>,
    child: Option<crate::capture::ui_process::UiChild>,
    operation: Option<String>,
    controller: Controller,
    state: String,
}
impl Popup {
    pub fn open(bundle: PathBuf, generation: u64, controller: Controller) -> anyhow::Result<Self> {
        crate::capture::engine::runtime::apparent_log("public-share resolve-runtime");
        let resources = crate::capture::desktop::workflow::electron::resolve()?;
        let entry = resources.root.join("electron/share/main.cjs");
        anyhow::ensure!(entry.is_file(), "Public sharing UI is not packaged");
        let metadata = std::fs::metadata(&bundle)?;
        let capture = bundle
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| anyhow::anyhow!("Invalid capture name"))?
            .to_owned();
        let mut command = Command::new(resources.executable);
        crate::capture::engine::runtime::apparent_log("public-share spawn-start");
        #[cfg(target_os = "linux")]
        command.arg("--ozone-platform=x11");
        let mut child = command
            .arg(entry)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let owner = child.id();
        crate::capture::engine::runtime::apparent_log(format!("public-share spawned pid={owner}"));
        let input = child.stdin.take();
        let output = child.stdout.take().unwrap();
        let (tx, commands) = mpsc::sync_channel(8);
        std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(output);
            loop {
                let mut line = String::new();
                match reader.by_ref().take(128).read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) if !line.ends_with('\n') => break,
                    _ => {
                        if tx.try_send(line.trim().to_owned()).is_err() {
                            break;
                        }
                    }
                }
            }
            let _ = tx.try_send("cancel".into());
        });
        let mut popup = Self {
            id: operation_id()?,
            bundle,
            inode: (metadata.dev(), metadata.ino()),
            generation,
            owner,
            capture,
            commands,
            input,
            child: Some(crate::capture::ui_process::UiChild::new(child)),
            operation: None,
            controller,
            state: "confirm".into(),
        };
        popup.publish("confirm")?;
        crate::capture::engine::runtime::apparent_log("public-share initial-state-sent");
        Ok(popup)
    }
    pub fn copied(&self) -> bool {
        self.state == "copied"
    }
    fn publish(&mut self, state: &str) -> anyhow::Result<()> {
        self.state = state.into();
        let input = self
            .input
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("Closed popup"))?;
        writeln!(
            input,
            "{}",
            serde_json::json!({"capture":self.capture,"state":state})
        )?;
        Ok(())
    }
    fn deliver_if_focused(
        &self,
        foreground: u32,
        copy: impl FnOnce(&str) -> anyhow::Result<()>,
    ) -> &'static str {
        let Some(operation) = self.operation.as_deref() else {
            return "failed";
        };
        if self.owner == 0 || foreground != self.owner {
            self.controller.cancel(operation);
            return "failed";
        }
        self.controller
            .deliver(
                operation,
                &self.capture,
                &self.bundle,
                self.generation,
                copy,
            )
            .unwrap_or("failed")
    }
    /// Called only on the GUI thread; no background worker writes the clipboard.
    pub fn tick(&mut self, bundle: Option<&Path>, generation: u64) -> bool {
        if bundle != Some(self.bundle.as_path())
            || generation != self.generation
            || !std::fs::metadata(&self.bundle).is_ok_and(|m| (m.dev(), m.ino()) == self.inode)
            || self
                .child
                .as_mut()
                .is_none_or(|c| match c.try_wait() {
                    Ok(None) => false,
                    Ok(Some(status)) => {
                        use std::os::unix::process::ExitStatusExt;
                        crate::capture::engine::runtime::apparent_log(format!(
                            "public-share child-exit code={:?} signal={:?}", status.code(), status.signal()));
                        true
                    }
                    Err(_) => {
                        crate::capture::engine::runtime::apparent_log("public-share child-wait-failed");
                        true
                    }
                })
        {
            crate::capture::engine::runtime::apparent_log("public-share child-closed-or-capture-invalidated");
            return false;
        }
        while let Ok(command) = self.commands.try_recv() {
            match command.as_str() {
                "cancel" => return false,
                "confirm" if self.operation.is_none() && self.state == "confirm" => {
                    match self.controller.start(
                        self.capture.clone(),
                        self.bundle.clone(),
                        generation,
                    ) {
                        Ok(id) => {
                            self.operation = Some(id);
                            if self.publish("preparing").is_err() {
                                return false;
                            }
                        }
                        Err(_) => {
                            if self.publish("failed").is_err() {
                                return false;
                            }
                        }
                    }
                }
                "deliver" if self.state == "prepared" => {
                    let result = self.deliver_if_focused(
                        crate::capture::desktop::platform::share_foreground_pid(),
                        crate::shared::handoff::copy_string_to_clipboard,
                    );
                    if self.publish(result).is_err() {
                        return false;
                    }
                }
                _ => return false,
            }
        }
        if let Some(operation) = self
            .operation
            .as_ref()
            .filter(|_| self.state == "preparing")
        {
            let state =
                match self
                    .controller
                    .read(operation, &self.capture, &self.bundle, generation)
                {
                    Ok(state) => state,
                    Err(_) => "failed",
                };
            if state != self.state && self.publish(state).is_err() {
                return false;
            }
        }
        true
    }
}
impl Drop for Popup {
    fn drop(&mut self) {
        if let Some(operation) = &self.operation {
            self.controller.cancel(operation);
        }
        self.input.take();
        if let Some(mut child) = self.child.take() {
            std::thread::spawn(move || {
                for _ in 0..40 {
                    if !matches!(child.try_wait(), Ok(None)) {
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                let _ = child.kill();
                let _ = child.wait();
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (Popup, mpsc::SyncSender<String>, PathBuf) {
        let bundle = std::env::temp_dir().join(format!("share-popup-{}", operation_id().unwrap()));
        std::fs::create_dir(&bundle).unwrap();
        let metadata = std::fs::metadata(&bundle).unwrap();
        let mut child = Command::new("/bin/sh")
            .args(["-c", "cat >/dev/null"])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let input = child.stdin.take();
        let (sender, commands) = mpsc::sync_channel(8);
        (
            Popup {
                id: operation_id().unwrap(),
                bundle: bundle.clone(),
                inode: (metadata.dev(), metadata.ino()),
                generation: 1,
                owner: 123,
                capture: "synthetic.apparent".into(),
                commands,
                input,
                child: Some(crate::capture::ui_process::UiChild::new(child)),
                operation: None,
                controller: Controller::default(),
                state: "confirm".into(),
            },
            sender,
            bundle,
        )
    }
    #[test]
    fn popup_never_admits_commands_for_changed_capture_or_generation() {
        for fault in ["path", "generation", "inode"] {
            let (mut popup, sender, bundle) = fixture();
            sender.send("confirm".into()).unwrap();
            if fault == "inode" {
                popup.inode = (0, 0);
            }
            assert!(!popup.tick(
                if fault == "path" { None } else { Some(&bundle) },
                if fault == "generation" { 2 } else { 1 }
            ));
            assert!(popup.controller.0.lock().unwrap().is_none());
            drop(popup);
            std::fs::remove_dir(bundle).unwrap();
        }
    }
    #[test]
    fn popup_cancel_and_unadmitted_delivery_have_no_share_effect() {
        for action in ["cancel", "deliver", "unknown"] {
            let (mut popup, sender, bundle) = fixture();
            sender.send(action.into()).unwrap();
            assert!(!popup.tick(Some(&bundle), 1));
            assert!(popup.controller.0.lock().unwrap().is_none());
            drop(popup);
            std::fs::remove_dir(bundle).unwrap();
        }
    }
    #[test]
    fn focus_loss_between_request_and_delivery_never_writes_clipboard() {
        struct Link;
        impl ReadyLink for Link {
            fn authorized(&self) -> bool {
                true
            }
            fn text(&self) -> &str {
                "synthetic"
            }
        }
        for foreground in [0, 124, 123] {
            let (mut popup, _, bundle) = fixture();
            let id = operation_id().unwrap();
            *popup.controller.0.lock().unwrap() = Some(Entry {
                id: id.clone(),
                capture: popup.capture.clone(),
                bundle: bundle.clone(),
                inode: popup.inode,
                generation: 1,
                expires: Instant::now() + std::time::Duration::from_secs(10),
                active: Arc::new(AtomicBool::new(true)),
                prepared: Some(Box::new(Link)),
                state: "prepared",
            });
            popup.operation = Some(id);
            let mut wrote = false;
            let result = popup.deliver_if_focused(foreground, |_| {
                wrote = true;
                Ok(())
            });
            assert_eq!(wrote, foreground == 123);
            assert_eq!(
                result,
                if foreground == 123 {
                    "copied"
                } else {
                    "failed"
                }
            );
            drop(popup);
            std::fs::remove_dir(bundle).unwrap();
        }
    }
}
