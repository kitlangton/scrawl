//! A Screen capture writes `unfinished.json` beside its video and audio before its bundle is built
//! and removes it once `apparent.json` is there: the target, ink, clicks and window log that only
//! lived in memory. If the build dies with the process (#58: transcription crashed on CPUs without
//! AVX-512), the next launch runs `apparent recover` in a child process, which builds the bundle
//! from those files in place. Each bundle gets one try (`.recovering`), so a crash that happens
//! again can't take every launch down with it.
//!
//! Captures from before the journal (0.3.10 on Windows crashed the same way) left only
//! `recording.mp4` and `audio.wav`. Once those have sat untouched for `ORPHAN_SETTLE` they are
//! rebuilt too, from the video and audio alone: the whole frame, no ink or clicks, keyframes from
//! scene changes and the transcript. New captures write narration continuously. Their kernel
//! lease excludes live writers; a dead writer's narration can be repaired without ever decoding
//! a privacy-excluded movie. Recording-time recovery is audio-only, not a Screen bundle.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::capture::engine::runtime::apparent_log;
use crate::capture::engine::runtime::audio::persistence;
use crate::shared::model::{
    CaptureMode, InteractionEvent, RgbaFrame, Stroke, TargetFrame, TargetInfo, TranscriptData,
};
use crate::shared::windows::{CursorSample, WindowSnapshot};

pub const UNFINISHED: &str = "unfinished.json";
pub const RECOVERING: &str = ".recovering";
/// A live capture without safe screenshots must not be rebuilt from its potentially private movie.
pub const UNSAFE_VIDEO: &str = ".unsafe-video";
pub const CANCELLED: &str = ".cancelled";

fn cancelled(dir: &Path) -> bool {
    dir.join(CANCELLED).exists()
        || dir.file_name().is_some_and(|name| name.to_string_lossy().starts_with(".cancelled-"))
}

/// Replace metadata atomically rather than following an existing symlink or hard link.
fn write_metadata(dir: &Path, name: &str, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let temporary = dir.join(format!(".metadata-{}", crate::shared::ids::make_bundle_id()));
    let result = (|| -> Result<()> {
        let mut file = std::fs::OpenOptions::new().write(true).create_new(true)
            .mode(0o600).custom_flags(libc::O_NOFOLLOW).open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temporary, dir.join(name))?;
        std::fs::File::open(dir)?.sync_all()?;
        Ok(())
    })();
    if result.is_err() { let _ = std::fs::remove_file(&temporary); }
    result
}

/// Persist cancellation before stopping the writer: a crash or failed deletion must not revive it.
pub fn cancel(dir: &Path) -> Result<()> {
    write_metadata(dir, CANCELLED, b"Cancelled by the recording owner; do not recover\n")
}

/// Call after all capture writers stop. Move outside the capture namespace before recursive
/// deletion, so removal of the internal tombstone cannot resurrect partially deleted media.
pub fn quarantine_cancelled(dir: &Path) -> Result<PathBuf> {
    anyhow::ensure!(cancelled(dir), "Cancellation must be durable before cleanup");
    let parent = dir.parent().context("Cancelled capture has no parent")?;
    let destination = parent.join(format!(".cancelled-{}", crate::shared::ids::make_bundle_id()));
    anyhow::ensure!(!destination.exists(), "Cancelled capture destination already exists");
    std::fs::rename(dir, &destination)?;
    std::fs::File::open(parent)?.sync_all()?;
    Ok(destination)
}

pub fn delete_cancelled(dir: &Path) -> Result<()> {
    let destination = quarantine_cancelled(dir)?;
    std::fs::remove_dir_all(&destination)
        .with_context(|| format!("Cancelled assets remain at {}; recovery is forbidden", destination.display()))?;
    std::fs::File::open(dir.parent().context("Missing capture parent")?)?.sync_all()?;
    Ok(())
}

/// Startup has local producers but no ActiveRecording yet. Cancellation owns that directory too.
pub fn abandon_start(dir: &Path, stop_producers: impl FnOnce()) -> Result<()> {
    let cancelled = cancel(dir);
    stop_producers();
    cancelled?;
    delete_cancelled(dir)
}

/// A journal younger than this may belong to a build still running in another process.
const SETTLE: Duration = Duration::from_secs(15);
/// Video and audio without a journal are left alone until nothing in the bundle changed for this
/// long.
const ORPHAN_SETTLE: Duration = Duration::from_secs(10 * 60);
const VIDEO: &str = "recording.mp4";
const AUDIO: &str = "audio.wav";
/// The movie starts this much after the audio at most (measured: 50–500 ms).
const MAX_VIDEO_OFFSET_MS: u64 = 1_000;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interaction_telemetry: Option<crate::shared::model::InteractionTelemetry>,
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
        apparent_log(format!("recover: could not write {UNFINISHED}: {error:#}"));
    }
}

pub fn finished(bundle_dir: &Path) {
    let _ = std::fs::remove_file(bundle_dir.join(UNFINISHED));
    let _ = std::fs::remove_file(bundle_dir.join(RECOVERING));
    let _ = std::fs::remove_file(bundle_dir.join(UNSAFE_VIDEO));
    let _ = std::fs::remove_file(bundle_dir.join(persistence::RECORDING));
}

/// Protect a new screen recording before its movie can be written. Refuse occupied directories:
/// an old orphan's recovery could already be in flight and must not decode the new movie.
pub fn begin_screen_capture(bundle_dir: &Path) -> Result<()> {
    std::fs::create_dir_all(bundle_dir)?;
    anyhow::ensure!(
        std::fs::read_dir(bundle_dir)?.next().is_none(),
        "{} is not empty; use a new output directory for a screen capture",
        bundle_dir.display()
    );
    use std::io::Write;
    let mut marker = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(bundle_dir.join(UNSAFE_VIDEO))
        .with_context(|| format!("Could not protect raw video in {}", bundle_dir.display()))?;
    marker.write_all(b"No safe screenshots; do not recover movie frames\n")?;
    marker.sync_all()?;
    std::fs::File::open(bundle_dir)?.sync_all()?;
    Ok(())
}

pub fn mark_unsafe_video(bundle_dir: &Path) -> Result<()> {
    std::fs::write(
        bundle_dir.join(UNSAFE_VIDEO),
        b"No safe screenshots; do not recover movie frames\n",
    )
    .with_context(|| format!("Could not protect raw video in {}", bundle_dir.display()))
}

pub fn read(bundle_dir: &Path) -> Result<Unfinished> {
    let path = bundle_dir.join(UNFINISHED);
    serde_json::from_slice(
        &std::fs::read(&path).with_context(|| format!("Could not read {}", path.display()))?,
    )
    .with_context(|| format!("Invalid {}", path.display()))
}

/// A bundle with a Screen capture's video and audio and no journal: its build never ran. A phone
/// recording's (`timeline.json`) is left to the phone, which sends it again.
fn orphaned(dir: &Path) -> bool {
    dir.join(VIDEO).is_file()
        && dir.join(AUDIO).is_file()
        && !dir.join(UNFINISHED).exists()
        && !dir.join("timeline.json").exists()
}

/// Whether a build is still owed to `dir`: nothing built it and it hasn't been tried.
pub fn owed(dir: &Path) -> bool {
    !dir.join("apparent.json").exists()
        && !cancelled(dir)
        && !dir.join(RECOVERING).exists()
        && !persistence::live(dir)
        && ((dir.join(persistence::RECORDING).is_file() && dir.join(AUDIO).is_file())
            || (!dir.join(UNSAFE_VIDEO).exists()
                && (dir.join(UNFINISHED).is_file() || orphaned(dir))))
}

fn untouched_for(path: &Path, now: SystemTime, age: Duration) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .is_ok_and(|at| now.duration_since(at).is_ok_and(|ago| ago >= age))
}

/// Bundles in `captures` whose build never finished and hasn't been retried.
pub fn pending(captures: &Path, now: SystemTime) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(captures) else {
        return Vec::new();
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|dir| {
            dir.extension().is_some_and(|ext| ext == "apparent")
                && owed(dir)
                && if dir.join(persistence::RECORDING).exists() {
                    untouched_for(&dir.join(AUDIO), now, SETTLE)
                } else if orphaned(dir) {
                    [dir.clone(), dir.join(VIDEO), dir.join(AUDIO)]
                        .iter()
                        .all(|path| untouched_for(path, now, ORPHAN_SETTLE))
                } else {
                    untouched_for(&dir.join(UNFINISHED), now, SETTLE)
                }
        })
        .collect();
    dirs.sort();
    dirs
}

/// What an orphaned bundle's build would have been given: its whole video as a Screen capture.
pub fn orphan_recipe(video: &crate::capture::desktop::helper::Probe, audio_ms: Option<u64>) -> Unfinished {
    // The audio runs from the start of the capture, the movie from its first frame.
    let offset = audio_ms
        .map_or(0, |audio| audio.saturating_sub(video.duration_ms))
        .min(MAX_VIDEO_OFFSET_MS);
    Unfinished {
        target: TargetInfo {
            mode: CaptureMode::Display,
            app: None,
            bundle_id: None,
            window_title: None,
            window_id: None,
            display_id: 0,
            frame: TargetFrame {
                x: 0.0,
                y: 0.0,
                width: f64::from(video.width.max(1)),
                height: f64::from(video.height.max(1)),
                scale: 1.0,
            },
        },
        duration_ms: audio_ms.unwrap_or(0).max(offset + video.duration_ms),
        video: Some(VIDEO.into()),
        video_offset_ms: offset,
        strokes: Vec::new(),
        events: Vec::new(),
        interaction_telemetry: None,
        note: None,
        windows: Vec::new(),
        cursor: Vec::new(),
    }
}

/// The steps that read media, swapped out in tests.
#[derive(Clone, Copy)]
pub struct Media {
    pub probe: fn(&Path) -> Result<crate::capture::desktop::helper::Probe>,
    pub frames: fn(&Path, crate::capture::desktop::helper::FrameTimes, u32) -> Result<Vec<RgbaFrame>>,
    pub transcribe: fn(Option<&Path>, Option<&crate::capture::engine::runtime::audio::audio::RecordedAudio>) -> TranscriptData,
}

impl Media {
    pub const SYSTEM: Media = Media {
        probe: crate::capture::desktop::helper::probe,
        frames: crate::capture::desktop::helper::frames,
        transcribe: crate::capture::engine::runtime::audio::transcription::transcribe::transcribe,
    };
}

/// Builds one bundle in place from its journal, or from its video and audio alone, and marks it
/// tried first so a crash here isn't repeated at every launch.
pub fn rebuild(dir: &Path, media: &Media) -> Result<String> {
    anyhow::ensure!(!cancelled(dir), "Recording was cancelled; recovery is forbidden");
    anyhow::ensure!(!dir.join("apparent.json").exists(), "Bundle is already built; use apparent reprocess");
    let lease = dir.join(persistence::RECORDING).exists()
        .then(|| persistence::Lease::acquire(dir)).transpose()?;
    if let Some(ref lease) = lease {
        // One automatic attempt even when the file is empty/corrupt or the disk is still full.
        write_metadata(dir, RECOVERING, b"")?;
        let count = persistence::repair(dir, lease)?;
        if dir.join(UNSAFE_VIDEO).exists() {
            anyhow::ensure!(count > 0, "No narration samples reached disk in {}", dir.display());
            let message = format!(
                "Recovered narration only: {} ({} samples, {} ms). No screen bundle was rebuilt: privacy-filtered screenshots and recording-time interactions did not survive. Raw video remains excluded. The unsaved tail is unknown.\n",
                dir.join(AUDIO).display(), count, count as u64 * 1_000 / 16_000,
            );
            // A separate receipt is not a handoff or a claim that a Screen bundle exists.
            write_metadata(dir, "narration-recovered.txt", message.as_bytes())?;
            lease.remove()?;
            let _ = std::fs::remove_file(dir.join(RECOVERING));
            return Ok(message);
        }
    }
    anyhow::ensure!(
        !dir.join(UNSAFE_VIDEO).exists(),
        "{} has no privacy-filtered screenshots; raw movie recovery is unsafe",
        dir.display()
    );
    anyhow::ensure!(
        !dir.join("apparent.json").exists(),
        "{} is already built; `apparent reprocess` rebuilds it into a new bundle",
        dir.display()
    );
    write_metadata(dir, RECOVERING, b"")?;
    apparent_log(format!("recover: rebuilding {}", dir.display()));
    let result = if dir.join(UNFINISHED).is_file() {
        read(dir)
            .and_then(|unfinished| crate::capture::engine::bundle::bundle_processing::rebuild_screen_bundle_with(dir, unfinished, None, media))
    } else if orphaned(dir) {
        let probe = (media.probe)(&dir.join(VIDEO));
        let audio_ms = std::fs::read(dir.join(AUDIO))
            .ok()
            .and_then(|wav| crate::capture::engine::runtime::audio::audio::recorded_audio_from_wav(wav).ok())
            .map(|audio| audio.duration_ms);
        probe.and_then(|probe| {
            crate::capture::engine::bundle::bundle_processing::rebuild_screen_bundle_with(
                dir,
                orphan_recipe(&probe, audio_ms),
                Some(None),
                media,
            )
        })
    } else {
        Err(anyhow::anyhow!(
            "{} has neither {UNFINISHED} nor {VIDEO} and {AUDIO}",
            dir.display()
        ))
    };
    match &result {
        Ok(_) => {
            finished(dir);
            apparent_log(format!("recover: rebuilt {}", dir.display()));
        }
        Err(error) => apparent_log(format!(
            "recover: could not rebuild {}: {error:#}",
            dir.display()
        )),
    }
    result
}

/// At launch: rebuilds unfinished bundles in a child process, so a crash there leaves the app up.
pub fn start() {
    let captures = crate::shared::handoff::default_captures_dir();
    if pending(&captures, SystemTime::now()).is_empty() {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("apparent-recover".into())
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
                Ok(status) => apparent_log(format!("recover: finished ({status})")),
                Err(error) => apparent_log(format!("recover: could not start: {error}")),
            }
        });
}

/// `apparent recover`: rebuilds each pending bundle once, in place.
pub fn run(captures: &Path) -> Vec<(PathBuf, Result<String>)> {
    pending(captures, SystemTime::now())
        .into_iter()
        .map(|dir| {
            let result = rebuild(&dir, &Media::SYSTEM);
            (dir, result)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("apparent-recover-{name}-{}", std::process::id()));
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
        let crashed = bundle("a.apparent");
        let built = bundle("b.apparent");
        std::fs::write(built.join("apparent.json"), "{}").unwrap();
        let tried = bundle("c.apparent");
        std::fs::write(tried.join(RECOVERING), "").unwrap();
        std::fs::create_dir_all(captures.join("d.apparent")).unwrap();

        let now = SystemTime::now();
        assert!(pending(&captures, now).is_empty(), "still settling");
        let later = now + SETTLE + Duration::from_secs(1);
        assert_eq!(pending(&captures, later), vec![crashed.clone()]);
        assert_eq!(read(&crashed).unwrap(), unfinished());

        finished(&crashed);
        assert!(pending(&captures, later).is_empty());
    }

    const W: u32 = 320;
    const H: u32 = 192;
    const MOVIE_MS: u64 = 3_000;

    /// Grey for the first half, blue for the second: one scene change at 1.5 s.
    fn colour_at(ms: u64) -> [u8; 3] {
        if ms < MOVIE_MS / 2 {
            [200, 200, 200]
        } else {
            [30, 60, 220]
        }
    }

    /// What a 0.3.10 capture left: its video and audio, nothing else.
    fn orphan(captures: &Path, name: &str) -> PathBuf {
        let dir = captures.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(VIDEO), b"movie").unwrap();
        let tone: Vec<f32> = (0..16_000 * 3 + 1_600)
            .map(|i| (i as f32 * 0.05).sin() * 0.2)
            .collect();
        let wav = crate::capture::engine::runtime::audio::audio::encode_wav_16k(&tone).unwrap();
        std::fs::write(dir.join(AUDIO), wav).unwrap();
        dir
    }

    fn fake_probe(_: &Path) -> Result<crate::capture::desktop::helper::Probe> {
        Ok(serde_json::from_value(serde_json::json!({
            "durationMs": MOVIE_MS, "width": W, "height": H,
        }))?)
    }

    fn fake_frames(
        _: &Path,
        times: crate::capture::desktop::helper::FrameTimes,
        max_edge: u32,
    ) -> Result<Vec<RgbaFrame>> {
        let times: Vec<u64> = match times {
            crate::capture::desktop::helper::FrameTimes::Every(step) => {
                (0..MOVIE_MS).step_by(step as usize).collect()
            }
            crate::capture::desktop::helper::FrameTimes::At(at) => at.to_vec(),
        };
        let (w, h) = if max_edge == 0 || max_edge >= W {
            (W, H)
        } else {
            (max_edge, H * max_edge / W)
        };
        Ok(times
            .into_iter()
            .map(|t| {
                let [r, g, b] = colour_at(t.min(MOVIE_MS - 1));
                RgbaFrame::new(t, w, h, [r, g, b, 255].repeat((w * h) as usize))
            })
            .collect())
    }

    fn fake_transcript(
        _: Option<&Path>,
        _: Option<&crate::capture::engine::runtime::audio::audio::RecordedAudio>,
    ) -> TranscriptData {
        TranscriptData {
            source: "test".into(),
            text: "The login page. Now the settings are blue.".into(),
            segments: vec![
                crate::shared::model::TranscriptSegment {
                    t0: 100,
                    t1: 1_200,
                    text: "The login page.".into(),
                },
                crate::shared::model::TranscriptSegment {
                    t0: 1_600,
                    t1: 2_900,
                    text: "Now the settings are blue.".into(),
                },
            ],
            ..Default::default()
        }
    }

    const FAKE: Media = Media {
        probe: fake_probe,
        frames: fake_frames,
        transcribe: fake_transcript,
    };

    fn assert_rebuilt(dir: &Path) -> crate::shared::model::ApparentBundle {
        let bundle: crate::shared::model::ApparentBundle =
            serde_json::from_slice(&std::fs::read(dir.join("apparent.json")).unwrap()).unwrap();
        assert!(!bundle.keyframes.is_empty());
        for keyframe in &bundle.keyframes {
            assert!(dir.join(&keyframe.file).is_file(), "{}", keyframe.file);
        }
        assert!(dir.join(&bundle.contact_sheet).is_file());
        assert!(dir.join("summary.md").is_file());
        assert_eq!(bundle.video.as_deref(), Some(VIDEO));
        assert_eq!(bundle.audio.as_deref(), Some(AUDIO));
        assert_eq!(bundle.target.mode, CaptureMode::Display);
        assert_eq!(
            (bundle.target.frame.width, bundle.target.frame.height),
            (f64::from(W), f64::from(H))
        );
        assert!(bundle.autocrop.is_none(), "the whole frame, not a crop");
        assert!(bundle.strokes.is_empty() && bundle.events.is_empty());
        assert!(!dir.join(RECOVERING).exists() && !dir.join(UNFINISHED).exists());
        assert!(!owed(dir));
        bundle
    }

    #[test]
    fn recover_picks_up_old_video_and_audio_without_a_journal_but_never_a_live_capture() {
        let captures = temp_dir("orphans");
        let lost = orphan(&captures, "a.apparent");
        // Recording now: the movie grows, the audio is still in memory.
        let live = captures.join("b.apparent");
        std::fs::create_dir_all(&live).unwrap();
        std::fs::write(live.join(VIDEO), b"movie").unwrap();
        let built = orphan(&captures, "c.apparent");
        std::fs::write(built.join("apparent.json"), "{}").unwrap();
        let tried = orphan(&captures, "d.apparent");
        std::fs::write(tried.join(RECOVERING), "").unwrap();
        // Since 0.3.11 the journal follows the audio, and the journal decides.
        let journaled = orphan(&captures, "e.apparent");
        write(&journaled, &unfinished());
        let phone = orphan(&captures, "f.apparent");
        std::fs::write(phone.join("timeline.json"), "{}").unwrap();

        let now = SystemTime::now();
        assert_eq!(
            pending(&captures, now + SETTLE * 2),
            vec![journaled.clone()]
        );
        let later = now + ORPHAN_SETTLE + Duration::from_secs(1);
        assert_eq!(pending(&captures, later), vec![lost.clone(), journaled]);
        assert!(owed(&lost) && !owed(&live) && !owed(&built) && !owed(&tried) && !owed(&phone));
    }

    #[test]
    fn interrupted_screen_recording_never_recovers_raw_movie() {
        let captures = temp_dir("interrupted-screen");
        let dir = captures.join("new.apparent");
        begin_screen_capture(&dir).unwrap();
        std::fs::write(dir.join(VIDEO), b"new raw movie").unwrap();
        std::fs::write(dir.join(AUDIO), b"audio saved before the process crashed").unwrap();
        // Simulate dying before screenshots are checked or a bundle can be finalized.
        let later = SystemTime::now() + ORPHAN_SETTLE * 2;
        assert!(!owed(&dir));
        assert!(pending(&captures, later).is_empty());
        let error = rebuild(&dir, &FAKE).unwrap_err().to_string();
        assert!(error.contains("raw movie recovery is unsafe"), "{error}");
        assert!(!dir.join("apparent.json").exists());
        assert!(dir.join(UNSAFE_VIDEO).exists());
    }

    #[test]
    fn screen_capture_refuses_reused_or_recoverable_directory() {
        let captures = temp_dir("reused-screen");
        let dir = orphan(&captures, "old.apparent");
        let before_video = std::fs::read(dir.join(VIDEO)).unwrap();
        let error = begin_screen_capture(&dir).unwrap_err().to_string();
        assert!(error.contains("not empty"), "{error}");
        assert_eq!(std::fs::read(dir.join(VIDEO)).unwrap(), before_video);
        assert!(!dir.join(UNSAFE_VIDEO).exists());
        assert!(owed(&dir), "the old capture can still recover");
        let fresh = captures.join("fresh.apparent");
        begin_screen_capture(&fresh).unwrap();
        assert!(begin_screen_capture(&fresh).is_err());
        assert!(fresh.join(UNSAFE_VIDEO).exists());
    }

    #[test]
    fn completed_screen_bundle_removes_its_pre_recording_guard() {
        let captures = temp_dir("completed-screen");
        let dir = captures.join("complete.apparent");
        begin_screen_capture(&dir).unwrap();
        std::fs::write(dir.join("apparent.json"), b"{}").unwrap();
        finished(&dir);
        assert!(!dir.join(UNSAFE_VIDEO).exists());
    }

    #[test]
    fn recover_rebuilds_a_bundle_from_its_video_and_audio_alone() {
        let captures = temp_dir("rebuild");
        let dir = orphan(&captures, "lost.apparent");
        rebuild(&dir, &FAKE).unwrap();
        let bundle = assert_rebuilt(&dir);
        assert!(
            bundle.keyframes.iter().any(|k| k.t >= MOVIE_MS / 2),
            "the scene change gets a keyframe: {:?}",
            bundle.keyframes.iter().map(|k| k.t).collect::<Vec<_>>()
        );
        assert!(bundle.transcript.text.contains("settings are blue"));
        // The audio is 3.1 s, the movie 3 s: it started 100 ms in.
        assert_eq!((bundle.duration_ms, bundle.video_offset_ms), (3_100, 100));
        assert!(
            rebuild(&dir, &FAKE).is_err(),
            "built bundles are left alone"
        );
    }

    #[test]
    fn recover_tries_an_orphan_once_and_retries_only_when_asked() {
        let captures = temp_dir("once");
        let dir = orphan(&captures, "broken.apparent");
        fn no_probe(_: &Path) -> Result<crate::capture::desktop::helper::Probe> {
            anyhow::bail!("unreadable movie")
        }
        let broken = Media {
            probe: no_probe,
            ..FAKE
        };
        assert!(rebuild(&dir, &broken).is_err());
        assert!(dir.join(RECOVERING).exists() && !owed(&dir));
        let later = SystemTime::now() + ORPHAN_SETTLE * 2;
        assert!(pending(&captures, later).is_empty());
        rebuild(&dir, &FAKE).unwrap();
        assert_rebuilt(&dir);
    }

    /// A real movie through the platform's decoder: Media Foundation on Windows, AVFoundation
    /// via `apparent-helper` on a Mac (with ffmpeg on the PATH to make the movie).
    #[test]
    fn recover_rebuilds_a_real_movie() {
        let captures = temp_dir("real");
        let dir = orphan(&captures, "real.apparent");
        let frames: Vec<(u64, [u8; 3])> = (0..MOVIE_MS)
            .step_by(100)
            .map(|t| (t, colour_at(t)))
            .collect();
        let video = dir.join(VIDEO);
        let _ = std::fs::remove_file(&video);
        #[cfg(target_os = "windows")]
        crate::capture::engine::runtime::platform::legacy_windows::capture::media::write_test_movie(&video, W, H, &frames).unwrap();
        #[cfg(not(target_os = "windows"))]
        {
            let _ = &frames;
            let made = std::process::Command::new("ffmpeg")
                .args(["-v", "error", "-f", "lavfi", "-i"])
                .arg(format!(
                    "color=c=0xc8c8c8:s={W}x{H}:d=1.5:r=10[a];color=c=0x1e3cdc:s={W}x{H}:d=1.5:r=10[b];[a][b]concat=n=2:v=1"
                ))
                .args(["-c:v", "libx264", "-pix_fmt", "yuv420p"])
                .arg(&video)
                .status()
                .is_ok_and(|status| status.success());
            if !made || crate::capture::desktop::helper::path().is_err() {
                eprintln!("skipped: needs ffmpeg and apparent-helper");
                return;
            }
        }
        let media = Media {
            transcribe: fake_transcript,
            ..Media::SYSTEM
        };
        rebuild(&dir, &media).unwrap();
        let bundle = assert_rebuilt(&dir);
        let png = std::fs::read(dir.join(&bundle.keyframes.last().unwrap().file)).unwrap();
        let last = crate::capture::engine::render::decode_png(std::io::Cursor::new(png)).unwrap();
        let i = ((last.height - 4) * last.width + last.width / 2) as usize * 4;
        let (r, b) = (last.rgba[i], last.rgba[i + 2]);
        assert!(
            b > r + 80,
            "the last keyframe shows the blue scene: {r} {b}"
        );
    }
}
