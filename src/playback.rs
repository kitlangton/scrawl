//! Lowers the output volume while recording (`lowerPlayback`). A warm `scrawl-helper
//! playback-volume` ramps it without blocking recording or the UI, and puts it back when told
//! to stop, when this process exits or crashes (its stdin closes), or when it is terminated.
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use crate::app_config;

/// Restores the volume when dropped.
pub struct Guard;

static HELPER: Mutex<Option<Child>> = Mutex::new(None);

fn command(line: Option<&str>) -> anyhow::Result<()> {
    let mut helper = HELPER.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(child) = helper.as_mut()
        && child.try_wait()?.is_some()
    {
        helper.take();
    }
    if helper.is_none() {
        *helper = Some(
            Command::new(crate::helper::ui_path()?)
                .args(["playback-volume", "--server"])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .spawn()?,
        );
    }
    if let Some(line) = line {
        let stdin = helper
            .as_mut()
            .and_then(|child| child.stdin.as_mut())
            .ok_or_else(|| anyhow::anyhow!("Playback helper stdin missing"))?;
        writeln!(stdin, "{line}")?;
        stdin.flush()?;
    }
    Ok(())
}

/// Starts the helper ahead of the first recording, when the setting is on.
pub fn warm_up() {
    if app_config::load().lower_playback == Some(true)
        && let Err(error) = command(None)
    {
        crate::log::scrawl_log(format!("Playback helper: {error:#}"));
    }
}

impl Guard {
    /// Lowers the volume if the setting is on.
    pub fn start() -> Option<Self> {
        let config = app_config::load();
        if config.lower_playback != Some(true) {
            return None;
        }
        match command(Some(&format!("start {}", config.recording_volume()))) {
            Ok(()) => Some(Self),
            Err(error) => {
                crate::log::scrawl_log(format!("Playback volume: {error:#}"));
                None
            }
        }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        if let Err(error) = command(Some("stop")) {
            crate::log::scrawl_log(format!("Restoring playback volume: {error:#}"));
        }
    }
}
