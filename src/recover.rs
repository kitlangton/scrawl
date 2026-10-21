//! A Screen capture writes `unfinished.json` beside its video and audio before its bundle is built
//! and removes it once `scrawl.json` is there: the target, ink, clicks and window log that only
//! lived in memory. If the build dies with the process (#58: transcription crashed on CPUs without
//! AVX-512), the next launch runs `scrawl recover` in a child process, which builds the bundle
//! from those files in place. Each bundle gets one try (`.recovering`), so a crash that happens
//! again can't take every launch down with it.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::capture::scrawl_log;
use crate::model::{InteractionEvent, Stroke, TargetInfo};
use crate::windows::{CursorSample, WindowSnapshot};

pub const UNFINISHED: &str = "unfinished.json";
pub const RECOVERING: &str = ".recovering";

/// A journal younger than this may belong to a build still running in another process.
const SETTLE: Duration = Duration::from_secs(15);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Unfinished {
    pub target: TargetInfo,
    pub duration_ms: u64,
    #[serde(default)]
    pub video: Option<String>,
    #[serde(default)]
    pub video_offset_ms: u64,
    #[serde(default)]
    pub strokes: Vec<Stroke>,
    #[serde(default)]
    pub events: Vec<InteractionEvent>,
    #[serde(default)]
    pub note: Option<String>,
    #[serde(default)]
    pub windows: Vec<WindowSnapshot>,
    #[serde(default)]
    pub cursor: Vec<CursorSample>,
}

pub fn write(bundle_dir: &Path, unfinished: &Unfinished) {
    let written = serde_json::to_vec(unfinished)
        .map_err(anyhow::Error::from)
        .and_then(|bytes| Ok(std::fs::write(bundle_dir.join(UNFINISHED), bytes)?));
    if let Err(error) = written {
        scrawl_log(format!("recover: could not write {UNFINISHED}: {error:#}"));
    }
}

pub fn finished(bundle_dir: &Path) {
    let _ = std::fs::remove_file(bundle_dir.join(UNFINISHED));
    let _ = std::fs::remove_file(bundle_dir.join(RECOVERING));
}

pub fn read(bundle_dir: &Path) -> Result<Unfinished> {
    let path = bundle_dir.join(UNFINISHED);
    serde_json::from_slice(
        &std::fs::read(&path).with_context(|| format!("Could not read {}", path.display()))?,
    )
    .with_context(|| format!("Invalid {}", path.display()))
}

/// Bundles in `captures` whose build never finished and hasn't been retried.
pub fn pending(captures: &Path, now: SystemTime) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(captures) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|dir| {
            dir.extension().is_some_and(|ext| ext == "scrawl")
                && !dir.join("scrawl.json").exists()
                && !dir.join(RECOVERING).exists()
                && std::fs::metadata(dir.join(UNFINISHED))
                    .and_then(|m| m.modified())
                    .is_ok_and(|at| now.duration_since(at).is_ok_and(|age| age >= SETTLE))
        })
        .collect();
    dirs.sort();
    dirs
}

/// At launch: rebuilds unfinished bundles in a child process, so a crash there leaves the app up.
pub fn start() {
    let captures = crate::handoff::default_captures_dir();
    if pending(&captures, SystemTime::now()).is_empty() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("scrawl-recover".into())
        .spawn(move || {
            let Ok(exe) = std::env::current_exe() else {
                return;
            };
            let mut command = std::process::Command::new(exe);
            command
                .arg("recover")
                .arg("--captures")
                .arg(&captures)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null());
            #[cfg(target_os = "windows")]
            {
                use std::os::windows::process::CommandExt;
                const CREATE_NO_WINDOW: u32 = 0x0800_0000;
                command.creation_flags(CREATE_NO_WINDOW);
            }
            match command.status() {
                Ok(status) => scrawl_log(format!("recover: finished ({status})")),
                Err(error) => scrawl_log(format!("recover: could not start: {error}")),
            }
        });
}

/// `scrawl recover`: rebuilds each pending bundle once, in place.
pub fn run(captures: &Path) -> Vec<(PathBuf, Result<String>)> {
    pending(captures, SystemTime::now())
        .into_iter()
        .map(|dir| {
            let _ = std::fs::write(dir.join(RECOVERING), "");
            scrawl_log(format!("recover: rebuilding {}", dir.display()));
            let result =
                read(&dir).and_then(|unfinished| crate::rebuild_screen_bundle(&dir, unfinished));
            match &result {
                Ok(_) => {
                    finished(&dir);
                    scrawl_log(format!("recover: rebuilt {}", dir.display()));
                }
                Err(error) => scrawl_log(format!(
                    "recover: could not rebuild {}: {error:#}",
                    dir.display()
                )),
            }
            (dir, result)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("scrawl-recover-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn unfinished() -> Unfinished {
        serde_json::from_value(serde_json::json!({
            "target": {
                "mode": "display",
                "displayId": 1,
                "frame": { "x": 0.0, "y": 0.0, "width": 1440.0, "height": 900.0, "scale": 2.0 },
            },
            "durationMs": 4200,
            "video": "recording.mp4",
        }))
        .unwrap()
    }

    #[test]
    fn a_bundle_without_its_manifest_is_rebuilt_once_after_it_settles() {
        let captures = temp_dir("pending");
        let bundle = |name: &str| {
            let dir = captures.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            write(&dir, &unfinished());
            dir
        };
        let crashed = bundle("a.scrawl");
        let built = bundle("b.scrawl");
        std::fs::write(built.join("scrawl.json"), "{}").unwrap();
        let tried = bundle("c.scrawl");
        std::fs::write(tried.join(RECOVERING), "").unwrap();
        std::fs::create_dir_all(captures.join("d.scrawl")).unwrap();

        let now = SystemTime::now();
        assert!(pending(&captures, now).is_empty(), "still settling");
        let later = now + SETTLE + Duration::from_secs(1);
        assert_eq!(pending(&captures, later), vec![crashed.clone()]);
        assert_eq!(read(&crashed).unwrap(), unfinished());

        finished(&crashed);
        assert!(pending(&captures, later).is_empty());
    }
}
