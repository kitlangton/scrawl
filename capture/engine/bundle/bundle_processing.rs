//! Saved-bundle production shared by screen, phone, recovery and synthetic callers.
//!
//! This is an in-crate migration seam, not a standalone Engine. Acquisition and
//! caller effects stay outside; existing config, media and recovery adapters remain.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::capture::engine::runtime::audio::audio::RecordedAudio;
use crate::capture::engine::runtime::write_bundle_artifacts;
use crate::capture::engine::keyframes::select_keyframes;
use crate::shared::model::{
    ApparentBundle, CaptureMode, DeviceInfo, InteractionEvent, Mark, RgbaFrame, Shape,
    Stroke, TargetInfo, TranscriptData, keyframe_budget,
};
use crate::capture::engine::sketch::{marks_of, summary_markdown};
use crate::capture::engine::runtime::audio;
use crate::capture::desktop::helper;
use crate::shared::{model, windows};
use crate::capture::engine::autocrop::autocrop as autocrop;
use crate::capture::engine::{ render};
use crate::capture::engine::bundle::recover;
use crate::capture::engine::runtime::audio::transcription::{speech_model, transcribe};
use crate::shared::clock::now_epoch_ms;

/// Matches full-resolution decoded frames back to selected keyframes by timestamp (within 250ms),
/// so a single frame decode failure only leaves that one keyframe at candidate resolution.
fn merge_full_keyframes(
    selected: Vec<(model::Keyframe, RgbaFrame)>,
    full: Vec<RgbaFrame>,
) -> (Vec<(model::Keyframe, RgbaFrame)>, usize) {
    let mut pool: Vec<Option<RgbaFrame>> = full.into_iter().map(Some).collect();
    let mut matched = 0;
    let merged = selected
        .into_iter()
        .map(|(key, fallback)| {
            let best = pool
                .iter()
                .enumerate()
                .filter_map(|(idx, f)| f.as_ref().map(|frame| (idx, frame.t_ms.abs_diff(key.t))))
                .filter(|&(_, diff)| diff <= 250)
                .min_by_key(|&(_, diff)| diff)
                .map(|(idx, _)| idx);
            if let Some(idx) = best
                && let Some(mut full_frame) = pool[idx].take()
            {
                full_frame.t_ms = key.t;
                matched += 1;
                (key, full_frame)
            } else {
                (key, fallback)
            }
        })
        .collect();
    (merged, matched)
}

/// Per-stage timing breakdown logged on every finish (`finish timings: ...`).
#[derive(Clone, Copy, Debug, Default)]
pub struct StageTimings {
    pub started_at: Option<std::time::Instant>,
    pub stop_ms: Option<u64>,
    pub audio_ms: Option<u64>,
    pub candidates_ms: Option<u64>,
    pub transcribe_ms: Option<u64>,
}

/// Local-only finish diagnostics. Parallel stages overlap; do not sum their durations.
/// This contains no transcript, screen, filenames or window metadata.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct FinishTrace {
    version: u32,
    stop_ms: Option<u64>,
    audio_ms: Option<u64>,
    candidates_ms: Option<u64>,
    transcribe_ms: Option<u64>,
    select_ms: u64,
    decode_ms: u64,
    autocrop_ms: u64,
    sheet_ms: u64,
    write_ms: u64,
    total_ms: u64,
    prepared_visuals: bool,
}

fn save_finish_trace(bundle_dir: &Path, trace: &FinishTrace) -> Result<()> {
    std::fs::write(
        bundle_dir.join("finish-trace.json"),
        serde_json::to_vec_pretty(trace)?,
    )?;
    Ok(())
}

/// Pre-rendered keyframe visuals and encoded PNGs computed while transcription is running.
pub struct PreparedVisuals {
    pub keyframes: Vec<model::Keyframe>,
    pub autocrop: Option<model::AutocropInfo>,
    pub contact_sheet_png: Vec<u8>,
    pub frame_pngs: Vec<(String, Vec<u8>)>,
    pub full_sheet_png: Option<Vec<u8>>,
    pub full_frame_pngs: Vec<(String, Vec<u8>)>,
}

#[allow(clippy::too_many_arguments)]
pub fn prepare_visuals(
    selected: Vec<(model::Keyframe, RgbaFrame)>,
    baseline: Option<&RgbaFrame>,
    crop_settings: Option<&autocrop::Settings>,
    target: &TargetInfo,
    duration_ms: u64,
    strokes: &[Stroke],
    marks: &[Mark],
    events: &[InteractionEvent],
    activity: &windows::ActivityLog,
) -> Option<PreparedVisuals> {
    if selected.is_empty() {
        return None;
    }
    let (mut annotated_frames, mut cropped) = autocrop::annotate_and_crop(
        selected,
        baseline,
        crop_settings,
        target,
        duration_ms,
        strokes,
        marks,
        events,
        &activity.windows,
        &activity.cursor,
    );
    let keyframes: Vec<model::Keyframe> =
        annotated_frames.iter().map(|(kf, _)| kf.clone()).collect();
    let (full_sheet, contact_sheet) = std::thread::scope(|scope| {
        let full_handle = cropped.as_ref().map(|(_, full)| {
            scope.spawn(|| render::render_contact_sheet_for_duration(duration_ms, full))
        });
        let sheet = render::render_contact_sheet_for_duration(duration_ms, &annotated_frames);
        (full_handle.map(|h| h.join().ok()).flatten(), sheet)
    });
    render::stamp_keyframe_badges(&mut annotated_frames);
    if let Some((_, full)) = &mut cropped {
        render::stamp_keyframe_badges(full);
    }
    let contact_sheet_png = render::encode_png(&contact_sheet).ok()?;
    let frame_pngs: Vec<(String, Vec<u8>)> = std::thread::scope(|scope| {
        let workers: Vec<_> = annotated_frames
            .iter()
            .map(|(kf, frame)| {
                scope.spawn(move || render::encode_png(frame).ok().map(|b| (kf.file.clone(), b)))
            })
            .collect();
        workers
            .into_iter()
            .map(|w| w.join().ok().flatten())
            .collect::<Option<Vec<_>>>()
    })?;
    let (autocrop, full_sheet_png, full_frame_pngs) = match (cropped, full_sheet) {
        (Some((info, full)), Some(sheet)) => {
            let sheet_png = render::encode_png(&sheet).ok()?;
            let full_pngs: Vec<(String, Vec<u8>)> = std::thread::scope(|scope| {
                let workers: Vec<_> = full
                    .iter()
                    .map(|(kf, frame)| {
                        scope.spawn(move || {
                            let name = std::path::Path::new(&kf.file)
                                .file_name()
                                .and_then(|n| n.to_str())?
                                .to_owned();
                            render::encode_png(frame).ok().map(|b| (name, b))
                        })
                    })
                    .collect();
                workers
                    .into_iter()
                    .map(|w| w.join().ok().flatten())
                    .collect::<Option<Vec<_>>>()
            })?;
            (Some(info), Some(sheet_png), full_pngs)
        }
        _ => (None, None, Vec::new()),
    };
    Some(PreparedVisuals {
        keyframes,
        autocrop,
        contact_sheet_png,
        frame_pngs,
        full_sheet_png,
        full_frame_pngs,
    })
}

/// What a source other than this Mac's screen adds to a bundle.
#[derive(Default)]
pub struct SourceExtras {
    pub source: Option<String>,
    pub interaction_telemetry: Option<model::InteractionTelemetry>,
    pub device: Option<DeviceInfo>,
    /// What the recorded app said about itself: values at the start, their changes, and screens.
    pub context: std::collections::BTreeMap<String, String>,
    pub context_changes: Vec<model::ContextChange>,
    pub screens: Vec<model::ScreenSpan>,
    /// Screen mode's window and cursor log.
    pub activity: windows::ActivityLog,
    /// Time-only markers (ms); each becomes a `Flag` mark with keyframe priority.
    pub flags: Vec<u64>,
    /// When the video starts on the bundle's clock (ms).
    pub video_offset_ms: u64,
    /// Screen-capture auto-crop: `None` follows the config, `Some(None)` is off.
    pub autocrop: Option<Option<autocrop::Settings>>,
    /// Decodes the chosen keyframes again at full quality, for sources that select on small
    /// candidate frames (phone movies).
    pub full_frames: Option<FullFrames>,
    pub prepared: Option<PreparedVisuals>,
    pub timings: StageTimings,
}

/// Decodes frames at these times at full quality.
pub type FullFrames = Box<dyn Fn(&[u64]) -> Result<Vec<RgbaFrame>>>;

/// Keyframes at native size from a Mac screen recording, whose clock starts `offset_ms` into the
/// capture. Each frame comes back stamped with the capture time it was asked for.
pub fn movie_keyframes(movie: PathBuf, offset_ms: u64) -> FullFrames {
    movie_keyframes_with_cache(movie, offset_ms, Vec::new())
}

fn movie_keyframes_with_cache(
    movie: PathBuf,
    offset_ms: u64,
    predecoded: Vec<RgbaFrame>,
) -> FullFrames {
    let cache = std::sync::Mutex::new(predecoded);
    Box::new(move |times: &[u64]| {
        let mut pool = cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut out = Vec::with_capacity(times.len());
        let mut missing = Vec::new();
        for &t in times {
            if let Some(idx) = pool.iter().position(|f| f.t_ms == t) {
                out.push(pool.swap_remove(idx));
            } else {
                missing.push(t);
            }
        }
        drop(pool);
        if !missing.is_empty() {
            out.extend(movie_frames_at(&movie, &missing, offset_ms, 0)?);
        }
        Ok(out)
    })
}

/// Frames of a movie whose clock starts `offset_ms` into the bundle's, at bundle times.
pub fn movie_frames_at(
    movie: &Path,
    times: &[u64],
    offset_ms: u64,
    max_edge: u32,
) -> Result<Vec<RgbaFrame>> {
    movie_frames_at_with(helper::frames, movie, times, offset_ms, max_edge)
}

fn movie_frames_at_with(
    decode: fn(&Path, helper::FrameTimes, u32) -> Result<Vec<RgbaFrame>>,
    movie: &Path,
    times: &[u64],
    offset_ms: u64,
    max_edge: u32,
) -> Result<Vec<RgbaFrame>> {
    let at: Vec<u64> = times.iter().map(|t| t.saturating_sub(offset_ms)).collect();
    let mut frames = decode(movie, helper::FrameTimes::At(&at), max_edge)?;
    // The helper stamps each frame with the time it was asked for, clamped to the movie (the
    // last keyframe can fall just past its end); map it back to the bundle time.
    for frame in &mut frames {
        if let Some(i) = (0..at.len()).min_by_key(|&i| at[i].abs_diff(frame.t_ms)) {
            frame.t_ms = times[i];
        }
    }
    Ok(frames)
}

/// Core pipeline that turns raw frames, strokes, events, and audio/transcript into a complete `.apparent` bundle on disk.
#[allow(clippy::too_many_arguments)]
///
/// The recording's candidate frames are small; the keyframes are decoded again from its video at
/// full size, falling back to the candidates if that fails.
pub fn finalize_capture_bundle(
    bundle_dir: &Path,
    target: TargetInfo,
    recording: crate::capture::engine::runtime::ScreenRecording,
    strokes: Vec<Stroke>,
    events: Vec<InteractionEvent>,
    recorded_audio: Option<RecordedAudio>,
    note: Option<String>,
    activity: windows::ActivityLog,
    interaction_telemetry: Option<model::InteractionTelemetry>,
) -> Result<(PathBuf, ApparentBundle, String)> {
    let started_at = activity
        .stop_started
        .unwrap_or_else(std::time::Instant::now);
    let stop_ms = activity.stop_ms;
    let crate::capture::engine::runtime::ScreenRecording {
        frames,
        video: video_path,
        duration_ms,
        video_offset_ms,
    } = recording;
    if frames.is_empty() {
        // The MP4 stream can succeed even when every screenshot fails. Do not turn
        // it into keyframes here: the sampler drops screenshots while a system
        // permission prompt is visible, but the movie may still contain it.
        recover::mark_unsafe_video(bundle_dir)?;
        let message = "No screen frames were captured. Choose a screen in the sharing dialog and try again.";
        if let Some(ref video) = video_path {
            anyhow::bail!("{message} The raw video was kept at {}.", video.display());
        }
        anyhow::bail!("{message}");
    }
    if let Some(ref audio) = recorded_audio {
        let wav_path = bundle_dir.join("audio.wav");
        // The incremental recorder already saved this file. Never truncate its saved prefix.
        if !wav_path.exists() {
            use std::io::Write;
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&wav_path)?;
            file.write_all(&audio.wav_bytes)?;
            file.sync_all()?;
        }
    }
    let wav_path = bundle_dir.join("audio.wav");
    let has_wav = wav_path.exists();
    recover::write(
        bundle_dir,
        &recover::Unfinished {
            target: target.clone(),
            duration_ms,
            video: video_path
                .as_ref()
                .and_then(|v| v.file_name())
                .map(|name| name.to_string_lossy().into_owned()),
            video_offset_ms,
            strokes: strokes.clone(),
            events: events.clone(),
            interaction_telemetry,
            note: note.clone(),
            windows: activity.windows.clone(),
            cursor: activity.cursor.clone(),
        },
    );
    let crop_settings = (target.mode == CaptureMode::Display)
        .then(autocrop::Settings::configured)
        .flatten();
    let ((transcript, transcribe_ms), (predecoded, prepared)) = std::thread::scope(|scope| {
        let transcribe_handle = scope.spawn(|| {
            let t0 = std::time::Instant::now();
            let tr = transcribe::transcribe(
                has_wav.then_some(wav_path.as_path()),
                recorded_audio.as_ref(),
            );
            (tr, t0.elapsed().as_millis() as u64)
        });
        let pre = if let Some(ref movie) = video_path
            && !frames.is_empty()
        {
            let mut preview_marks = marks_of(&strokes);
            let budget = keyframe_budget(duration_ms);
            let preview = select_keyframes(
                &frames,
                &strokes,
                &mut preview_marks,
                &events,
                &TranscriptData::default(),
                budget,
            );
            // Marks alone filled the budget, so speech can't change the selection.
            let marks_fill = preview.len() == budget
                && preview
                    .iter()
                    .all(|(k, _)| k.reason == model::KeyframeReason::Mark);
            let times: Vec<u64> = preview.iter().map(|(k, _)| k.t).collect();
            let decoded = movie_frames_at(movie, &times, video_offset_ms, 0).unwrap_or_default();
            if duration_ms >= 20_000 {
                let cache = if marks_fill {
                    Vec::new()
                } else {
                    decoded.clone()
                };
                let (merged, matched) = merge_full_keyframes(preview, decoded);
                let vis = (matched == merged.len()).then(|| prepare_visuals(
                    merged,
                    frames.first(),
                    crop_settings.as_ref(),
                    &target,
                    duration_ms,
                    &strokes,
                    &preview_marks,
                    &events,
                    &activity,
                )).flatten();
                (cache, vis)
            } else {
                (decoded, None)
            }
        } else {
            (Vec::new(), None)
        };
        (transcribe_handle.join().unwrap(), pre)
    });

    let built = build_bundle_from_parts(
        bundle_dir,
        target,
        duration_ms,
        frames,
        strokes,
        events,
        transcript,
        video_path.clone(),
        note,
        SourceExtras {
            activity,
            interaction_telemetry,
            video_offset_ms,
            autocrop: Some(crop_settings),
            full_frames: video_path
                .map(|movie| movie_keyframes_with_cache(movie, video_offset_ms, predecoded)),
            prepared,
            timings: StageTimings {
                started_at: Some(started_at),
                stop_ms,
                transcribe_ms: Some(transcribe_ms),
                ..StageTimings::default()
            },
            ..SourceExtras::default()
        },
    );
    if built.is_ok() {
        recover::finished(bundle_dir);
    }
    built
}

/// Builds a Screen capture's bundle in `out`, which holds its video and `audio.wav`, from them
/// and what the recording logged: `apparent reprocess` and `apparent recover`.
pub fn rebuild_screen_bundle(out: &Path, old: recover::Unfinished) -> Result<String> {
    rebuild_screen_bundle_with(out, old, None, &recover::Media::SYSTEM)
}

/// `autocrop` as in `SourceExtras`: `None` follows the config, `Some(None)` keeps the whole frame.
pub fn rebuild_screen_bundle_with(
    out: &Path,
    old: recover::Unfinished,
    autocrop: Option<Option<autocrop::Settings>>,
    media: &recover::Media,
) -> Result<String> {
    let started_at = std::time::Instant::now();
    let video = old
        .video
        .as_ref()
        .map(|name| out.join(name))
        .filter(|p| p.is_file())
        .context("The bundle has no video")?;
    let wav = out.join("audio.wav");
    let recorded = if wav.is_file() {
        Some(audio::audio::recorded_audio_from_wav(std::fs::read(&wav)?)?)
    } else {
        None
    };
    let offset = old.video_offset_ms;
    let ((transcript, transcribe_ms), joined) = std::thread::scope(|scope| {
        let frames = scope.spawn(|| {
            let t0 = std::time::Instant::now();
            let res = (media.frames)(&video, helper::FrameTimes::Every(220), 640);
            (res, t0.elapsed().as_millis() as u64)
        });
        let t0 = std::time::Instant::now();
        let tr = (media.transcribe)(recorded.as_ref().map(|_| wav.as_path()), recorded.as_ref());
        ((tr, t0.elapsed().as_millis() as u64), frames.join())
    });
    let (frames, candidates_ms) = joined.map_err(|_| anyhow::anyhow!("Frame decoding panicked"))?;
    let mut frames = frames?;
    for frame in &mut frames {
        frame.t_ms += offset;
    }
    let (decode, movie) = (media.frames, video.clone());
    let full_frames: Option<FullFrames> = Some(Box::new(move |times: &[u64]| {
        movie_frames_at_with(decode, &movie, times, offset, 0)
    }));
    Ok(build_bundle_from_parts(
        out,
        old.target,
        old.duration_ms,
        frames,
        old.strokes,
        old.events,
        transcript,
        Some(video),
        old.note,
        SourceExtras {
            activity: windows::ActivityLog {
                windows: old.windows,
                cursor: old.cursor,
                ..Default::default()
            },
            interaction_telemetry: old.interaction_telemetry,
            full_frames,
            video_offset_ms: offset,
            autocrop,
            timings: StageTimings {
                started_at: Some(started_at),
                candidates_ms: Some(candidates_ms),
                transcribe_ms: Some(transcribe_ms),
                ..Default::default()
            },
            ..SourceExtras::default()
        },
    )?
    .2)
}

/// A bundle without its transcript yet (the speech model was still downloading) gets it later.
fn mark_transcript_pending(bundle_dir: &Path, bundle: &ApparentBundle) {
    if bundle.transcript.missing.as_ref().is_some_and(|m| m.retry) {
        speech_model::mark_pending(bundle_dir);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn build_bundle_from_parts(
    bundle_dir: &Path,
    target: TargetInfo,
    duration_ms: u64,
    frames: Vec<RgbaFrame>,
    strokes: Vec<Stroke>,
    events: Vec<InteractionEvent>,
    transcript: TranscriptData,
    video_path: Option<PathBuf>,
    note: Option<String>,
    extras: SourceExtras,
) -> Result<(PathBuf, ApparentBundle, String)> {
    let total_started = extras
        .timings
        .started_at
        .unwrap_or_else(std::time::Instant::now);
    let mut marks = marks_of(&strokes);
    for (i, &t) in extras.flags.iter().enumerate() {
        marks.push(Mark {
            id: format!("f{}", i + 1),
            strokes: Vec::new(),
            t0: t,
            t1: t,
            box_norm: [0.0, 0.0, 1.0, 1.0],
            shape: Shape::Flag,
            from: None,
            to: None,
            pin: None,
            on: None,
            description: "flagged this moment".into(),
            frame: None,
        });
    }
    let bench = std::env::var_os("APPARENT_BENCH").is_some();
    let started = std::time::Instant::now();
    let selected = select_keyframes(
        &frames,
        &strokes,
        &mut marks,
        &events,
        &transcript,
        keyframe_budget(duration_ms),
    );
    let select_only_at = std::time::Instant::now();
    let matches_prepared = extras.prepared.as_ref().is_some_and(|prep| {
        prep.keyframes.len() == selected.len()
            && prep.keyframes.iter().zip(&selected).all(|(a, (b, _))| {
                a.index == b.index
                    && a.t == b.t
                    && a.mark_id == b.mark_id
                    && a.cursor == b.cursor
                    && a.file == b.file
            })
    });
    enum BundleOutput {
        Prepared(PreparedVisuals),
        Rendered {
            annotated: Vec<(model::Keyframe, RgbaFrame)>,
            cropped: Option<(model::AutocropInfo, Vec<(model::Keyframe, RgbaFrame)>)>,
            full_sheet: Option<RgbaFrame>,
            contact_sheet: RgbaFrame,
        },
    }

    let mut extras = extras;
    let mut keyframe_warning = None;
    let (keyframes, autocrop, selected_at, annotated_at, rendered_at, output) = if matches_prepared
    {
        drop(frames);
        let mut prep = extras.prepared.take().unwrap();
        let keyframes: Vec<model::Keyframe> = selected.into_iter().map(|(kf, _)| kf).collect();
        let selected_at = std::time::Instant::now();
        let autocrop = prep.autocrop.take();
        (
            keyframes,
            autocrop,
            selected_at,
            selected_at,
            selected_at,
            BundleOutput::Prepared(prep),
        )
    } else {
        // Only the first candidate is needed from here on (auto-crop's baseline); free the rest
        // before the full-size keyframes arrive.
        let baseline = {
            let mut frames = frames;
            frames.truncate(1);
            frames.pop()
        };
        let selected = match &extras.full_frames {
            Some(decode) if !selected.is_empty() => {
                let times: Vec<u64> = selected.iter().map(|(k, _)| k.t).collect();
                match decode(&times) {
                    Ok(full) => {
                        let total = selected.len();
                        let (merged, matched) = merge_full_keyframes(selected, full);
                        if matched < total {
                            crate::capture::engine::runtime::apparent_log(format!(
                                "Full-quality keyframes: decoded {matched} of {total}"
                            ));
                            keyframe_warning = Some(format!("Only {matched} of {total} keyframes could be decoded at full quality; the rest use smaller candidate frames."));
                        }
                        merged
                    }
                    Err(error) => {
                        crate::capture::engine::runtime::apparent_log(format!("Full-quality keyframes failed: {error:#}"));
                        keyframe_warning = Some(format!("Full-quality keyframes could not be decoded from the MP4. This capture uses smaller candidate frames.{}", if cfg!(target_os = "linux") { " Check Settings › Recording for an H.264 decoder repair instruction." } else { "" }));
                        selected
                    }
                }
            }
            _ => selected,
        };
        let selected_at = std::time::Instant::now();

        // A failed or partial MP4 decode leaves only small candidates. Their coarse change
        // regions can miss the actual activity and make an idle cursor look like the target.
        // Keep the whole candidate in the agent-facing images until full decode succeeds.
        let crop_settings = if keyframe_warning.is_some() {
            None
        } else {
            match extras.autocrop {
                Some(settings) => settings,
                None if target.mode == CaptureMode::Display => autocrop::Settings::configured(),
                None => None,
            }
        };
        let (mut annotated_frames, mut cropped) = autocrop::annotate_and_crop(
            selected,
            baseline.as_ref(),
            crop_settings.as_ref(),
            &target,
            duration_ms,
            &strokes,
            &marks,
            &events,
            &extras.activity.windows,
            &extras.activity.cursor,
        );
        let keyframes: Vec<model::Keyframe> =
            annotated_frames.iter().map(|(kf, _)| kf.clone()).collect();
        let annotated_at = std::time::Instant::now();

        let (full_sheet, contact_sheet) = std::thread::scope(|scope| {
            let full_handle = cropped.as_ref().map(|(_, full)| {
                scope.spawn(|| render::render_contact_sheet_for_duration(duration_ms, full))
            });
            let sheet = render::render_contact_sheet_for_duration(duration_ms, &annotated_frames);
            (full_handle.map(|h| h.join().unwrap()), sheet)
        });
        render::stamp_keyframe_badges(&mut annotated_frames);
        if let Some((_, full)) = &mut cropped {
            render::stamp_keyframe_badges(full);
        }
        let rendered_at = std::time::Instant::now();
        let autocrop = cropped.as_ref().map(|(info, _)| info.clone());
        (
            keyframes,
            autocrop,
            selected_at,
            annotated_at,
            rendered_at,
            BundleOutput::Rendered {
                annotated: annotated_frames,
                cropped,
                full_sheet,
                contact_sheet,
            },
        )
    };

    let id = bundle_dir
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("apparent-bundle")
        .to_string();
    let video_rel = video_path.and_then(|p| {
        p.exists()
            .then(|| p.file_name()?.to_str().map(str::to_owned))
            .flatten()
    });
    let audio_rel = bundle_dir
        .join("audio.wav")
        .exists()
        .then(|| "audio.wav".to_string());

    let bundle = ApparentBundle {
        version: 1,
        id,
        created_at_ms: now_epoch_ms(),
        duration_ms,
        target,
        transcript,
        note,
        strokes,
        marks,
        events,
        interaction_telemetry: extras.interaction_telemetry,
        keyframes,
        contact_sheet: "contact-sheet.png".into(),
        image_quality_warning: keyframe_warning,
        video: video_rel,
        video_offset_ms: extras.video_offset_ms,
        audio: audio_rel,
        source: extras.source,
        device: extras.device,
        context: extras.context,
        context_changes: extras.context_changes,
        screens: extras.screens,
        windows: extras.activity.windows,
        cursor: extras.activity.cursor,
        autocrop,
        destination: None,
    };
    let summary = summary_markdown(&bundle, bundle_dir);

    match output {
        BundleOutput::Prepared(prep) => {
            std::fs::create_dir_all(bundle_dir.join("frames"))
                .with_context(|| format!("could not create {}", bundle_dir.display()))?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ =
                    std::fs::set_permissions(bundle_dir, std::fs::Permissions::from_mode(0o700));
            }
            std::fs::write(
                bundle_dir.join(&bundle.contact_sheet),
                &prep.contact_sheet_png,
            )?;
            for (rel, bytes) in &prep.frame_pngs {
                std::fs::write(bundle_dir.join(rel), bytes)?;
            }
            if let (Some(info), Some(full_sheet_png)) = (&bundle.autocrop, &prep.full_sheet_png) {
                let full_dir = bundle_dir.join(&info.full_frames);
                std::fs::create_dir_all(&full_dir)?;
                std::fs::write(bundle_dir.join(&info.full_contact_sheet), full_sheet_png)?;
                for (name, bytes) in &prep.full_frame_pngs {
                    std::fs::write(full_dir.join(name), bytes)?;
                }
            }
            let manifest_bytes = serde_json::to_vec_pretty(&bundle)?;
            std::fs::write(bundle_dir.join("apparent.json"), manifest_bytes)?;
            std::fs::write(bundle_dir.join("summary.md"), summary.as_bytes())?;
        }
        BundleOutput::Rendered {
            annotated,
            cropped,
            full_sheet,
            contact_sheet,
        } => {
            std::thread::scope(|scope| -> Result<()> {
                let full_write = match (&cropped, &full_sheet) {
                    (Some((info, full)), Some(sheet)) => Some(
                        scope.spawn(|| autocrop::write_full_frames(bundle_dir, info, full, sheet)),
                    ),
                    _ => None,
                };
                write_bundle_artifacts(bundle_dir, &bundle, &annotated, &contact_sheet, &summary)?;
                if let Some(handle) = full_write {
                    handle
                        .join()
                        .map_err(|_| anyhow::anyhow!("Full-frame writer panicked"))??;
                }
                Ok(())
            })?;
        }
    }

    let finished = std::time::Instant::now();
    let select_ms = (select_only_at - started).as_millis() as u64;
    let decode_ms = (selected_at - select_only_at).as_millis() as u64;
    let autocrop_ms = (annotated_at - selected_at).as_millis() as u64;
    let sheet_ms = (rendered_at - annotated_at).as_millis() as u64;
    let write_ms = (finished - rendered_at).as_millis() as u64;
    let total_ms = (finished - total_started).as_millis() as u64;
    let mut parts = Vec::with_capacity(10);
    if let Some(ms) = extras.timings.stop_ms {
        parts.push(format!("stop {ms}ms"));
    }
    if let Some(ms) = extras.timings.audio_ms {
        parts.push(format!("audio {ms}ms"));
    }
    if let Some(ms) = extras.timings.candidates_ms {
        parts.push(format!("candidates {ms}ms"));
    }
    if let Some(ms) = extras.timings.transcribe_ms {
        parts.push(format!("transcribe {ms}ms"));
    }
    parts.extend([
        format!("select {select_ms}ms"),
        format!("decode {decode_ms}ms"),
        format!("autocrop {autocrop_ms}ms"),
        format!("sheet {sheet_ms}ms"),
        format!("write {write_ms}ms"),
        format!("total {total_ms}ms"),
    ]);
    crate::capture::engine::runtime::apparent_log(format!("finish timings: {} · {}", bundle.id, parts.join(" · ")));
    let trace = FinishTrace {
        version: 1,
        stop_ms: extras.timings.stop_ms,
        audio_ms: extras.timings.audio_ms,
        candidates_ms: extras.timings.candidates_ms,
        transcribe_ms: extras.timings.transcribe_ms,
        select_ms,
        decode_ms,
        autocrop_ms,
        sheet_ms,
        write_ms,
        total_ms,
        prepared_visuals: matches_prepared,
    };
    if let Err(error) = save_finish_trace(bundle_dir, &trace) {
        crate::capture::engine::runtime::apparent_log(format!("Could not save finish trace: {error:#}"));
    }
    if bench && !matches_prepared {
        for (name, elapsed) in [
            ("selection_ms", selected_at - started),
            ("annotation_ms", annotated_at - selected_at),
            ("contact_sheet_ms", rendered_at - annotated_at),
            ("artifacts_ms", finished - rendered_at),
            ("pipeline_ms", finished - started),
        ] {
            eprintln!("BENCH {name}={:.3}", elapsed.as_secs_f64() * 1000.0);
        }
    }

    mark_transcript_pending(bundle_dir, &bundle);
    Ok((bundle_dir.to_path_buf(), bundle, summary))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::model::TargetFrame;
    use crate::capture::{desktop::demo::generate_demo_bundle, engine::sketch};
    use crate::shared::{handoff, ids::make_bundle_id};

    #[test]
    fn movie_without_safe_screenshots_is_not_a_bundle() {
        let dir = std::env::temp_dir().join(make_bundle_id());
        std::fs::create_dir(&dir).unwrap();
        let movie = dir.join("recording.mp4");
        std::fs::write(&movie, b"untrusted movie bytes").unwrap();
        std::fs::write(dir.join("audio.wav"), b"audio bytes").unwrap();
        let target = TargetInfo {
            mode: CaptureMode::Window,
            app: None,
            bundle_id: None,
            window_title: None,
            window_id: None,
            display_id: 1,
            frame: TargetFrame { x: 0.0, y: 0.0, width: 32.0, height: 32.0, scale: 1.0 },
        };
        let error = finalize_capture_bundle(
            &dir,
            target,
            crate::capture::engine::runtime::ScreenRecording {
                frames: Vec::new(),
                video: Some(movie.clone()),
                duration_ms: 1_000,
                video_offset_ms: 0,
            },
            Vec::new(),
            Vec::new(),
            None,
            None,
            windows::ActivityLog::default(),
            None,
        )
        .err()
        .expect("a movie alone is not a safe screenshot candidate");
        assert!(error.to_string().contains(&movie.display().to_string()));
        assert!(movie.exists());
        assert!(!dir.join("apparent.json").exists());
        assert!(dir.join(crate::capture::engine::bundle::recover::UNSAFE_VIDEO).exists());
        assert!(!crate::capture::engine::bundle::recover::owed(&dir), "a later recovery must not decode unsafe movie frames");
        assert!(crate::capture::engine::bundle::recover::rebuild(&dir, &crate::capture::engine::bundle::recover::Media::SYSTEM).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn video_only_bundle_discloses_telemetry_gap_after_reload() {
        let root = std::env::temp_dir().join(make_bundle_id());
        let dir = root.join("video-only.apparent");
        let target = TargetInfo {
            mode: CaptureMode::Display,
            app: None,
            bundle_id: None,
            window_title: None,
            window_id: None,
            display_id: 1,
            frame: TargetFrame { x: 0.0, y: 0.0, width: 32.0, height: 32.0, scale: 1.0 },
        };
        let (_, bundle, summary) = build_bundle_from_parts(
            &dir, target, 1000,
            vec![RgbaFrame::new(0, 32, 32, vec![128; 32 * 32 * 4])],
            vec![], vec![], TranscriptData::default(), None, None,
            SourceExtras {
                interaction_telemetry: Some(model::InteractionTelemetry::VideoOnly),
                autocrop: Some(None),
                ..Default::default()
            },
        ).unwrap();
        assert!(bundle.events.is_empty());
        assert!(bundle.marks.is_empty());
        assert!(summary.contains("clicks and Shift ink were not collected"));
        assert_eq!(std::fs::read_to_string(dir.join("summary.md")).unwrap(), summary);
        let (_, loaded) = handoff::load_bundle(&dir).unwrap();
        assert_eq!(loaded.interaction_telemetry, Some(model::InteractionTelemetry::VideoOnly));
        assert!(sketch::summary_markdown_with(&loaded, &dir, true).contains("clicks and Shift ink were not collected"));
        let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join("apparent.json")).unwrap()).unwrap();
        assert_eq!(manifest["interactionTelemetry"], "videoOnly");
        let old: model::ApparentBundle = serde_json::from_value({
            let mut old = manifest;
            old.as_object_mut().unwrap().remove("interactionTelemetry");
            old
        }).unwrap();
        assert_eq!(old.interaction_telemetry, None);
        assert!(!sketch::summary_markdown_with(&old, &dir, false).contains("timed CLI video only"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn demo_bundle_saves_finish_trace_without_configuration() {
        let dir = std::env::temp_dir().join(format!("{}.apparent", make_bundle_id()));
        let (built, _, _) = generate_demo_bundle(Some(dir.clone())).unwrap();
        assert_eq!(built, dir);
        let trace: serde_json::Value =
            serde_json::from_slice(&std::fs::read(dir.join("finish-trace.json")).unwrap()).unwrap();
        assert_eq!(trace["version"], 1);
        assert!(trace["totalMs"].is_u64());
        assert!(dir.join("apparent.json").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn finish_trace_contains_only_timings() {
        let dir = std::env::temp_dir().join(make_bundle_id());
        std::fs::create_dir_all(&dir).unwrap();
        let trace = FinishTrace {
            version: 1,
            stop_ms: Some(8),
            audio_ms: None,
            candidates_ms: None,
            transcribe_ms: Some(40),
            select_ms: 2,
            decode_ms: 3,
            autocrop_ms: 4,
            sheet_ms: 5,
            write_ms: 6,
            total_ms: 50,
            prepared_visuals: false,
        };
        let path = dir.join("finish-trace.json");
        save_finish_trace(&dir, &trace).unwrap();
        let json: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["transcribeMs"], 40);
        assert_eq!(json["totalMs"], 50);
        assert_eq!(json["preparedVisuals"], false);
        assert_eq!(json.as_object().unwrap().len(), 12);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn artifact_failures_do_not_publish_a_manifest() {
        let root = std::env::temp_dir().join(make_bundle_id());
        let (_, mut bundle, _) = generate_demo_bundle(Some(root.join("demo.apparent"))).unwrap();
        let frame = RgbaFrame::new(0, 1, 1, vec![20, 30, 40, 255]);
        let mut key = bundle.keyframes[0].clone();
        key.file = "frames/missing/frame.png".into();
        let frame_failure = root.join("frame-failure.apparent");
        assert!(
            write_bundle_artifacts(
                &frame_failure,
                &bundle,
                &[(key, frame.clone())],
                &frame,
                "summary"
            )
            .is_err()
        );
        assert!(!frame_failure.join("apparent.json").exists());

        bundle.contact_sheet = "missing/contact-sheet.png".into();
        let sheet_failure = root.join("sheet-failure.apparent");
        assert!(write_bundle_artifacts(&sheet_failure, &bundle, &[], &frame, "summary").is_err());
        assert!(!sheet_failure.join("apparent.json").exists());

        bundle.contact_sheet = "contact-sheet.png".into();
        let empty_frames = root.join("empty-frames.apparent");
        write_bundle_artifacts(&empty_frames, &bundle, &[], &frame, "summary").unwrap();
        assert!(empty_frames.join("apparent.json").exists());
        assert!(empty_frames.join("contact-sheet.png").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn full_keyframes_match_by_time_and_preserve_partial_decodes() {
        let kf = |index: usize, t: u64| model::Keyframe {
            index,
            t,
            reason: model::KeyframeReason::Click,
            file: format!("frames/frame-{index:02}.png"),
            event: "tap".into(),
            said: None,
            mark_id: None,
            cursor: None,
        };
        let small = |t: u64| RgbaFrame::new(t, 32, 32, vec![10; 32 * 32 * 4]);
        let large = |t: u64| RgbaFrame::new(t, 160, 160, vec![200; 160 * 160 * 4]);

        let selected = vec![
            (kf(1, 1_000), small(1_000)),
            (kf(2, 2_000), small(2_000)),
            (kf(3, 3_000), small(3_000)),
        ];
        // Frame at 2_000 failed to decode and frames arrived out of order (3_000 clamped to 2_999).
        let full = vec![large(2_999), large(1_000)];
        let (merged, matched) = merge_full_keyframes(selected, full);
        assert_eq!(matched, 2);
        assert_eq!(merged[0].1.width, 160);
        assert_eq!(merged[0].1.t_ms, 1_000);
        assert_eq!(merged[1].1.width, 32);
        assert_eq!(merged[1].1.t_ms, 2_000);
        assert_eq!(merged[2].1.width, 160);
        assert_eq!(merged[2].1.t_ms, 3_000);
    }

    #[test]
    fn failed_full_decode_explains_candidate_frames_in_written_summary() {
        let dir = std::env::temp_dir().join(make_bundle_id());
        let bundle_dir = dir.join("fallback.apparent");
        let target = TargetInfo {
            mode: CaptureMode::Window,
            app: None, bundle_id: None, window_title: None, window_id: None,
            display_id: 1,
            frame: TargetFrame { x: 0.0, y: 0.0, width: 32.0, height: 32.0, scale: 1.0 },
        };
        let frames = vec![RgbaFrame::new(0, 32, 32, vec![128; 32 * 32 * 4])];
        let (_, bundle, summary) = build_bundle_from_parts(
            &bundle_dir, target, 1000, frames, vec![], vec![],
            TranscriptData::default(), None, None,
            SourceExtras {
                full_frames: Some(Box::new(|_| anyhow::bail!("decoder missing"))),
                autocrop: Some(None),
                ..SourceExtras::default()
            },
        ).unwrap();
        assert_eq!(bundle.keyframes.len(), 1);
        assert!(summary.contains("Image quality: Full-quality keyframes could not be decoded"));
        assert!(summary.contains("smaller candidate frames"));
        assert_eq!(std::fs::read_to_string(bundle_dir.join("summary.md")).unwrap(), summary);
        let (directory, mut loaded) = handoff::load_bundle(&bundle_dir).unwrap();
        // macOS may canonicalize /var to /private/var when loading the bundle.
        // Regenerating for the original path must preserve the written summary.
        assert_eq!(sketch::summary_markdown(&loaded, &bundle_dir), summary);
        assert_eq!(directory, bundle_dir.canonicalize().unwrap());
        loaded.note = Some("a later edit".into());
        let regenerated = sketch::summary_markdown(&loaded, &directory);
        assert!(regenerated.contains("Image quality: Full-quality keyframes could not be decoded"));
        assert!(regenerated.contains("Note: a later edit"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn decoder_fallback_keeps_moving_target_in_agent_images() {
        let root = std::env::temp_dir().join(make_bundle_id());
        let bundle_dir = root.join("fallback.apparent");
        let target = TargetInfo {
            mode: CaptureMode::Display,
            app: None, bundle_id: None, window_title: None, window_id: None,
            display_id: 1,
            frame: TargetFrame { x: 0.0, y: 0.0, width: 480.0, height: 300.0, scale: 1.0 },
        };
        let frame = |t, x: usize| {
            let mut rgba = vec![255; 480 * 300 * 4];
            for y in 30..110 {
                for xx in x..x + 80 {
                    let pixel = (y * 480 + xx) * 4;
                    rgba[pixel..pixel + 3].copy_from_slice(&[220, 0, 0]);
                }
            }
            RgbaFrame::new(t, 480, 300, rgba)
        };
        let frames = vec![frame(0, 20), frame(1_000, 40)];
        let (_, bundle, summary) = build_bundle_from_parts(
            &bundle_dir, target, 1_100, frames, vec![], vec![],
            TranscriptData::default(), None, None,
            SourceExtras {
                full_frames: Some(Box::new(|_| anyhow::bail!("decoder missing"))),
                autocrop: Some(Some(autocrop::Settings::default())),
                activity: windows::ActivityLog {
                    cursor: vec![windows::CursorSample(0, 0.9, 0.85), windows::CursorSample(1_100, 0.9, 0.85)],
                    ..Default::default()
                },
                ..SourceExtras::default()
            },
        ).unwrap();
        assert!(summary.contains("Image quality:"));
        assert!(!bundle.keyframes.is_empty());
        assert!(bundle.autocrop.is_none(), "fallback frames must not be cursor-cropped");
        // The idle pointer is at bottom-right; the moving target is at top-left.
        for key in &bundle.keyframes {
            let bytes = std::fs::read(bundle_dir.join(&key.file)).unwrap();
            let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
            let mut reader = decoder.read_info().unwrap();
            let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
            let info = reader.next_frame(&mut pixels).unwrap();
            assert_eq!((info.width, info.height), (480, 300));
            assert!(pixels[..info.buffer_size()].chunks_exact(4).any(|p| p[0] > 150 && p[1] < 40 && p[2] < 40));
        }
        let sheet = std::fs::read(bundle_dir.join("contact-sheet.png")).unwrap();
        let mut reader = png::Decoder::new(std::io::Cursor::new(sheet)).read_info().unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert!(pixels[..info.buffer_size()].chunks_exact(4).any(|p| p[0] > 150 && p[1] < 40 && p[2] < 40));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn successful_full_decode_keeps_normal_autocrop() {
        let root = std::env::temp_dir().join(make_bundle_id());
        let target = TargetInfo {
            mode: CaptureMode::Display,
            app: None, bundle_id: None, window_title: None, window_id: None,
            display_id: 1,
            frame: TargetFrame { x: 0.0, y: 0.0, width: 1280.0, height: 800.0, scale: 1.0 },
        };
        let candidate = RgbaFrame::new(0, 320, 200, vec![128; 320 * 200 * 4]);
        // Successful decode still permits tight crops for deliberate input.
        // A resting cursor alone, without window metadata, must retain the screen.
        let click = model::InteractionEvent::Click {
            t: 0, x: 0.5, y: 0.5, button: None, label: None, role: None,
        };
        let (_, bundle, _) = build_bundle_from_parts(
            &root, target, 1_000, vec![candidate], vec![], vec![click],
            TranscriptData::default(), None, None,
            SourceExtras {
                full_frames: Some(Box::new(|times| Ok(times.iter().map(|&t| {
                    RgbaFrame::new(t, 1280, 800, vec![128; 1280 * 800 * 4])
                }).collect()))),
                autocrop: Some(Some(autocrop::Settings::default())),
                activity: windows::ActivityLog {
                    cursor: vec![windows::CursorSample(0, 0.5, 0.5)],
                    ..Default::default()
                },
                ..SourceExtras::default()
            },
        ).unwrap();
        assert!(bundle.image_quality_warning.is_none());
        assert!(bundle.autocrop.is_some(), "normal decode should still autocrop");
        assert!(root.join("frames-full/frame-01.png").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn partial_full_decode_keeps_both_resolutions_uncropped() {
        let root = std::env::temp_dir().join(make_bundle_id());
        let target = TargetInfo {
            mode: CaptureMode::Display,
            app: None, bundle_id: None, window_title: None, window_id: None,
            display_id: 1,
            frame: TargetFrame { x: 0.0, y: 0.0, width: 1280.0, height: 800.0, scale: 1.0 },
        };
        let candidate = |t| RgbaFrame::new(t, 480, 300, vec![180; 480 * 300 * 4]);
        let click = |t| model::InteractionEvent::Click {
            t, x: 0.1, y: 0.1, button: None, label: None, role: None,
        };
        let (_, bundle, summary) = build_bundle_from_parts(
            &root, target, 10_000, vec![candidate(0), candidate(5_000)],
            vec![], vec![click(0), click(5_000)], TranscriptData::default(), None, None,
            SourceExtras {
                full_frames: Some(Box::new(|times| Ok(vec![RgbaFrame::new(
                    times[0], 1280, 800, vec![180; 1280 * 800 * 4],
                )]))),
                autocrop: Some(Some(autocrop::Settings::default())),
                ..SourceExtras::default()
            },
        ).unwrap();
        assert!(summary.contains("Image quality:"));
        assert_eq!(bundle.keyframes.len(), 2, "one decoded and one candidate keyframe");
        assert!(bundle.autocrop.is_none());
        let sizes: Vec<_> = bundle.keyframes.iter().map(|key| {
            let bytes = std::fs::read(root.join(&key.file)).unwrap();
            let reader = png::Decoder::new(std::io::Cursor::new(bytes)).read_info().unwrap();
            (reader.info().width, reader.info().height)
        }).collect();
        assert!(sizes.contains(&(1280, 800)), "decoded full frame must survive: {sizes:?}");
        assert!(sizes.contains(&(480, 300)), "fallback candidate must survive: {sizes:?}");
        assert!(root.join("contact-sheet.png").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
