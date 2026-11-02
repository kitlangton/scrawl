//! In-process media teardown and bundle finalization, run only on the background executor.

use std::path::PathBuf;
use std::time::Instant;

use super::super::CaptureSource;
use super::whiteboard;
use crate::capture::engine::runtime::apparent_log;
use crate::shared::model::{ApparentBundle, InteractionEvent, Stroke, TargetInfo};

pub(in crate::capture::desktop) type CaptureResult =
    anyhow::Result<(PathBuf, ApparentBundle, String)>;

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
            // Stop the producer before waiting for a possibly contended writer lease.
            let finalized = session.stop().and_then(|recording| {
                complete_bundle(&bundle_dir, &final_whiteboard, || {
                    crate::capture::engine::runtime::phone::finalize_live_capture(
                        &bundle_dir,
                        recording,
                        events,
                        strokes,
                        None,
                    )
                })
            });
            // Only the outer publication result may enqueue native movie shrinking.
            let finalized = after_publication(
                finalized,
                crate::capture::engine::bundle::retention::after_phone_capture,
            );
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
        Err(error) => return (Err(error), duration_ms),
    };
    let finalized = complete_bundle(&bundle_dir, &final_whiteboard, || {
        crate::capture::engine::bundle::bundle_processing::finalize_capture_bundle(
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
    });
    (finalized, duration_ms)
}

fn after_publication(
    result: CaptureResult,
    on_success: impl FnOnce(&std::path::Path),
) -> CaptureResult {
    if let Ok((dir, _, _)) = &result {
        on_success(dir);
    }
    result
}

/// Both recording sources must finish optional drawing mutations and validate
/// the surviving capture before Desktop can see any successful result.
fn complete_bundle(
    path: &std::path::Path,
    board: &whiteboard::Board,
    build: impl FnOnce() -> CaptureResult,
) -> CaptureResult {
    complete_bundle_with(path, build, |dir, bundle, summary| {
        whiteboard::finish_handoff(dir, bundle, summary, board)
    })
}

fn complete_bundle_with(
    path: &std::path::Path,
    build: impl FnOnce() -> CaptureResult,
    finish: impl FnOnce(&std::path::Path, ApparentBundle, String) -> (ApparentBundle, String),
) -> CaptureResult {
    crate::capture::engine::bundle::bundle_processing::publication::publish_after(path, || {
        let (dir, bundle, summary) = build()?;
        let (bundle, summary) = finish(&dir, bundle, summary);
        Ok((dir, bundle, summary))
    })
}

#[cfg(test)]
#[path = "../../engine/bundle/bundle_processing/publication/completion_tests.rs"]
mod publication_tests;
