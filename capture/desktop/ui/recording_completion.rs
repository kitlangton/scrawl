//! In-process media teardown and bundle finalization, run only on the background executor.

use crate::capture::engine::bundle::diagnostics::{CaptureFacts, Diagnose, Step};
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
    notes_finished: Option<flume::Receiver<Result<(), String>>>,
) -> (CaptureResult, u64) {
    let (candidate_frames, microphone_recorded) = match &source {
        CaptureSource::Screen { audio, .. } => (None, Some(audio.is_some())),
        CaptureSource::Phone(_) => (None, None),
        CaptureSource::Lab => (None, Some(false)),
    };
    let mut facts = CaptureFacts {
        width: target.frame.width,
        height: target.frame.height,
        scale: target.frame.scale,
        mode: target.mode,
        duration_ms: None,
        candidate_frames,
        has_video: None,
        microphone_recorded,
        notes_pending: notes_finished.is_some(),
        strokes: strokes.len(),
        interactions: events.len(),
    };
    let (result, duration_ms) = finalize_inner(
        source,
        target,
        bundle_dir,
        events,
        strokes,
        final_whiteboard,
        notes_finished,
        &mut facts,
    );
    // Phone stop does not expose a duration on failure; zero would be false evidence.
    facts.duration_ms =
        if facts.mode == crate::shared::model::CaptureMode::Device && result.is_err() {
            None
        } else {
            Some(duration_ms)
        };
    (result.map_err(|error| facts.attach(error)), duration_ms)
}

#[allow(clippy::too_many_arguments)]
fn finalize_inner(
    source: CaptureSource,
    target: TargetInfo,
    bundle_dir: PathBuf,
    events: Vec<InteractionEvent>,
    strokes: Vec<Stroke>,
    final_whiteboard: whiteboard::Board,
    notes_finished: Option<flume::Receiver<Result<(), String>>>,
    facts: &mut CaptureFacts,
) -> (CaptureResult, u64) {
    let (screen_session, audio_recorder, window_log) = match source {
        CaptureSource::Screen {
            session,
            audio,
            windows,
        } => (session, audio, windows),
        CaptureSource::Phone(session) => {
            // Stop the producer before waiting for a possibly contended writer lease.
            let finalized = session
                .stop()
                .diagnostic(Step::PhoneStop)
                .and_then(|recording| {
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
    facts.candidate_frames = Some(recording.frames.len());
    facts.has_video = Some(recording.video.is_some());
    let duration_ms = recording.duration_ms;
    let recorded_audio = match recorded_audio {
        Ok(audio) => audio,
        Err(error) => return (Err(error).diagnostic(Step::AudioStop), duration_ms),
    };
    let finalized = complete_bundle(&bundle_dir, &final_whiteboard, || {
        if let Some(receipt) = notes_finished {
            if let Err(error) = receipt
                .recv_timeout(std::time::Duration::from_secs(12))
                .map_err(|_| "Notes finish receipt unavailable".to_owned())
                .and_then(|r| r)
            {
                crate::capture::engine::bundle::recover::write(
                    &bundle_dir,
                    &crate::capture::engine::bundle::recover::Unfinished {
                        target: target.clone(),
                        duration_ms,
                        video: recording
                            .video
                            .as_ref()
                            .and_then(|p| p.file_name())
                            .map(|p| p.to_string_lossy().into_owned()),
                        video_offset_ms: recording.video_offset_ms,
                        strokes,
                        events,
                        interaction_telemetry: None,
                        note: None,
                        windows: activity.windows,
                        cursor: activity.cursor,
                    },
                )?;
                return Err(anyhow::anyhow!(error)).diagnostic(Step::NotesFinish);
            }
        }
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
    .diagnostic(Step::Publication)
}

#[cfg(test)]
#[path = "../../engine/bundle/bundle_processing/publication/completion_tests.rs"]
mod publication_tests;
