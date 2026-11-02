//! In-process media teardown and bundle finalization, run only on the background executor.

use std::path::PathBuf;
use std::time::Instant;

use super::super::CaptureSource;
use super::whiteboard;
use crate::capture::engine::runtime::apparent_log;
use crate::shared::model::{ApparentBundle, InteractionEvent, Stroke, TargetInfo};

pub(in crate::capture::desktop) type CaptureResult = anyhow::Result<(PathBuf, ApparentBundle, String)>;

/// The GUI has already stopped telemetry, closed the overlay and snapshotted the paper.
pub(in crate::capture::desktop) fn finalize(
    source: CaptureSource,
    target: TargetInfo,
    bundle_dir: PathBuf,
    events: Vec<InteractionEvent>,
    strokes: Vec<Stroke>,
    final_whiteboard: whiteboard::Board,
) -> (CaptureResult, u64) {
    let (screen_session, audio_recorder, window_log) = match source {
        CaptureSource::Screen {
            session,
            audio,
            windows,
        } => (session, audio, windows),
        CaptureSource::Phone(session) => {
            let finalized = session
                .stop()
                .and_then(|recording| {
                    crate::capture::engine::runtime::phone::finalize_live_capture(
                        &bundle_dir,
                        recording,
                        events,
                        strokes,
                        None,
                    )
                })
                .map(|(dir, bundle, summary)| {
                    let (bundle, summary) =
                        whiteboard::finish_handoff(&dir, bundle, summary, &final_whiteboard);
                    (dir, bundle, summary)
                });
            if let Err(error) = &finalized {
                apparent_log(format!("iPhone capture failed: {error:#}"));
            }
            let duration_ms = finalized.as_ref().map_or(0, |(_, b, _)| b.duration_ms);
            return (finalized, duration_ms);
        }
        CaptureSource::Lab => {
            return (Err(anyhow::anyhow!("a lab recording has no bundle")), 0);
        }
    };
    let stop_started = Instant::now();
    let (recording, recorded_audio, mut activity) = std::thread::scope(|s| {
        let screen_handle = s.spawn(|| screen_session.stop());
        let win_handle = s.spawn(|| {
            window_log
                .map(crate::shared::windows::WindowLogger::stop)
                .unwrap_or_default()
        });
        let audio = crate::capture::engine::runtime::audio::audio::stop_optional(audio_recorder);
        (
            screen_handle.join().unwrap(),
            audio,
            win_handle.join().unwrap_or_default(),
        )
    });
    activity.stop_started = Some(stop_started);
    activity.stop_ms = Some(stop_started.elapsed().as_millis() as u64);
    let duration_ms = recording.duration_ms;
    let recorded_audio = match recorded_audio {
        Ok(audio) => audio,
        Err(error) => return (Err(error.context(format!("Narration save failed; saved assets remain in {}", bundle_dir.display()))), duration_ms),
    };
    let finalized = crate::capture::engine::bundle::bundle_processing::finalize_capture_bundle(
        &bundle_dir,
        target,
        recording,
        strokes,
        events,
        recorded_audio,
        None,
        activity,
        None,
    )
    .map(|(dir, bundle, summary)| {
        let (bundle, summary) =
            whiteboard::finish_handoff(&dir, bundle, summary, &final_whiteboard);
        (dir, bundle, summary)
    });
    (finalized, duration_ms)
}
