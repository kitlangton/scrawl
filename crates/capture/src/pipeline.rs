use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};

use crate::audio::{self, RecordedAudio};
use crate::autocrop;
use crate::capture::{self, write_bundle_artifacts};
use crate::handoff;
use crate::helper;
use crate::keyframes::select_keyframes;
use crate::model::{
    self, CaptureMode, DeviceInfo, InteractionEvent, Mark, RgbaFrame, SCALE, ScrawlBundle, Shape,
    Stroke, TargetFrame, TargetInfo, TranscriptData, TranscriptSegment, WordTiming,
    keyframe_budget,
};
use crate::recover;
use crate::render::{self, render_synthetic_demo_frame};
use crate::sketch::{marks_of, summary_markdown};
use crate::speech_model;
use crate::transcribe;
use crate::windows;

pub fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn make_bundle_id() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    // Two IDs in the same millisecond (parallel tests, overlapping ingests) must still differ.
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let ms = now_epoch_ms();
    let pid = std::process::id();
    match COUNT.fetch_add(1, Ordering::Relaxed) {
        0 => format!("scrawl-{ms}-{pid:x}"),
        n => format!("scrawl-{ms}-{pid:x}-{n}"),
    }
}

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
    /// Whether the `agentContext` summary format is enabled (`None` reads `handoff::engine_config()`).
    pub agent_context: Option<bool>,
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

/// Core pipeline that turns raw frames, strokes, events, and audio/transcript into a complete `.scrawl` bundle on disk.
#[allow(clippy::too_many_arguments)]
///
/// The recording's candidate frames are small; the keyframes are decoded again from its video at
/// full size, falling back to the candidates if that fails.
pub fn finalize_capture_bundle(
    bundle_dir: &Path,
    target: TargetInfo,
    recording: capture::ScreenRecording,
    strokes: Vec<Stroke>,
    events: Vec<InteractionEvent>,
    recorded_audio: Option<RecordedAudio>,
    note: Option<String>,
    activity: windows::ActivityLog,
) -> Result<(PathBuf, ScrawlBundle, String)> {
    finalize_capture_bundle_with(
        bundle_dir,
        target,
        recording,
        strokes,
        events,
        recorded_audio,
        note,
        activity,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn finalize_capture_bundle_with(
    bundle_dir: &Path,
    target: TargetInfo,
    recording: capture::ScreenRecording,
    strokes: Vec<Stroke>,
    events: Vec<InteractionEvent>,
    recorded_audio: Option<RecordedAudio>,
    note: Option<String>,
    activity: windows::ActivityLog,
    transcriber: Option<&dyn transcribe::Transcriber>,
) -> Result<(PathBuf, ScrawlBundle, String)> {
    finalize_capture_bundle_with_flags(
        bundle_dir,
        target,
        recording,
        strokes,
        events,
        Vec::new(),
        recorded_audio,
        note,
        activity,
        transcriber,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn finalize_capture_bundle_with_flags(
    bundle_dir: &Path,
    target: TargetInfo,
    recording: capture::ScreenRecording,
    strokes: Vec<Stroke>,
    events: Vec<InteractionEvent>,
    flags: Vec<u64>,
    recorded_audio: Option<RecordedAudio>,
    note: Option<String>,
    activity: windows::ActivityLog,
    transcriber: Option<&dyn transcribe::Transcriber>,
) -> Result<(PathBuf, ScrawlBundle, String)> {
    finalize_capture_bundle_for_config(
        bundle_dir,
        target,
        recording,
        strokes,
        events,
        flags,
        recorded_audio,
        note,
        activity,
        transcriber,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub fn finalize_capture_bundle_for_config(
    bundle_dir: &Path,
    target: TargetInfo,
    recording: capture::ScreenRecording,
    strokes: Vec<Stroke>,
    events: Vec<InteractionEvent>,
    flags: Vec<u64>,
    recorded_audio: Option<RecordedAudio>,
    note: Option<String>,
    activity: windows::ActivityLog,
    transcriber: Option<&dyn transcribe::Transcriber>,
    engine_cfg: Option<&handoff::EngineConfig>,
) -> Result<(PathBuf, ScrawlBundle, String)> {
    let started_at = activity
        .stop_started
        .unwrap_or_else(std::time::Instant::now);
    let stop_ms = activity.stop_ms;
    let capture::ScreenRecording {
        frames,
        video: video_path,
        duration_ms,
        video_offset_ms,
    } = recording;
    if let Some(ref audio) = recorded_audio {
        if let Ok(wav_bytes) = audio.wav_bytes_16k() {
            if !wav_bytes.is_empty() {
                let wav_path = bundle_dir.join("audio.wav");
                let _ = std::fs::create_dir_all(bundle_dir);
                let _ = crate::routing::undo::write_private(&wav_path, &wav_bytes);
            }
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
            note: note.clone(),
            windows: activity.windows.clone(),
            cursor: activity.cursor.clone(),
        },
    );
    let crop_settings = (target.mode == CaptureMode::Display)
        .then(|| match engine_cfg {
            Some(cfg) => autocrop::Settings::from_engine_config(cfg),
            None => autocrop::Settings::configured(),
        })
        .flatten();
    let agent_context = engine_cfg.map(|cfg| cfg.experiment(handoff::AGENT_CONTEXT));
    let ((transcript, transcribe_ms), (predecoded, prepared)) = std::thread::scope(|scope| {
        let transcribe_handle = scope.spawn(|| {
            let t0 = std::time::Instant::now();
            let tr = match transcriber {
                Some(transcriber) => transcribe::transcribe_with(
                    has_wav.then_some(wav_path.as_path()),
                    recorded_audio.as_ref(),
                    transcriber,
                ),
                None => transcribe::transcribe(
                    has_wav.then_some(wav_path.as_path()),
                    recorded_audio.as_ref(),
                ),
            };
            (tr, t0.elapsed().as_millis() as u64)
        });
        let pre = if let Some(ref movie) = video_path
            && !frames.is_empty()
        {
            let mut preview_marks = marks_of(&strokes);
            for (i, &t) in flags.iter().enumerate() {
                preview_marks.push(model::Mark {
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
                let (merged, _) = merge_full_keyframes(preview, decoded);
                let vis = prepare_visuals(
                    merged,
                    frames.first(),
                    crop_settings.as_ref(),
                    &target,
                    duration_ms,
                    &strokes,
                    &preview_marks,
                    &events,
                    &activity,
                );
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
            flags,
            video_offset_ms,
            autocrop: Some(crop_settings),
            agent_context,
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
/// and what the recording logged: `scrawl reprocess` and `scrawl recover`.
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
        Some(audio::recorded_audio_from_wav(std::fs::read(&wav)?)?)
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
fn mark_transcript_pending(bundle_dir: &Path, bundle: &ScrawlBundle) {
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
) -> Result<(PathBuf, ScrawlBundle, String)> {
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
    let bench = std::env::var_os("SCRAWL_BENCH").is_some();
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
                            crate::log::scrawl_log(format!(
                                "Full-quality keyframes: decoded {matched} of {total}"
                            ));
                        }
                        merged
                    }
                    Err(error) => {
                        crate::log::scrawl_log(format!("Full-quality keyframes failed: {error:#}"));
                        selected
                    }
                }
            }
            _ => selected,
        };
        let selected_at = std::time::Instant::now();

        let crop_settings = match extras.autocrop {
            Some(settings) => settings,
            None if target.mode == CaptureMode::Display => autocrop::Settings::configured(),
            None => None,
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
        .unwrap_or("scrawl-bundle")
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

    let bundle = ScrawlBundle {
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
        keyframes,
        contact_sheet: "contact-sheet.png".into(),
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
    let summary = match extras.agent_context {
        Some(agent_context) => {
            crate::sketch::summary_markdown_with(&bundle, bundle_dir, agent_context)
        }
        None => summary_markdown(&bundle, bundle_dir),
    };

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
            crate::routing::undo::write_private(
                &bundle_dir.join(&bundle.contact_sheet),
                &prep.contact_sheet_png,
            )?;
            for (rel, bytes) in &prep.frame_pngs {
                crate::routing::undo::write_private(&bundle_dir.join(rel), bytes)?;
            }
            if let (Some(info), Some(full_sheet_png)) = (&bundle.autocrop, &prep.full_sheet_png) {
                let full_dir = bundle_dir.join(&info.full_frames);
                std::fs::create_dir_all(&full_dir)?;
                crate::routing::undo::write_private(
                    &bundle_dir.join(&info.full_contact_sheet),
                    full_sheet_png,
                )?;
                for (name, bytes) in &prep.full_frame_pngs {
                    crate::routing::undo::write_private(&full_dir.join(name), bytes)?;
                }
            }
            let manifest_bytes = serde_json::to_vec_pretty(&bundle)?;
            crate::routing::undo::write_private(&bundle_dir.join("scrawl.json"), &manifest_bytes)?;
            crate::routing::undo::write_private(
                &bundle_dir.join("summary.md"),
                summary.as_bytes(),
            )?;
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
    crate::log::scrawl_log(format!("finish timings: {}", parts.join(" · ")));
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

/// Generates a rich, deterministic multi-frame demo bundle showcasing clicks, circles, arrows, pins, and aligned speech.
pub fn generate_demo_bundle(out: Option<PathBuf>) -> Result<(PathBuf, ScrawlBundle, String)> {
    let bundle_dir = out.unwrap_or_else(|| {
        handoff::default_captures_dir().join(format!("{}.scrawl", make_bundle_id()))
    });
    let (fw, fh) = (1280_u32, 800_u32);
    let target = TargetInfo {
        mode: CaptureMode::Window,
        app: Some("OpenCode".into()),
        bundle_id: Some("ai.opencode.desktop".into()),
        window_title: Some("session/tool-running — OpenCode".into()),
        window_id: Some(4210),
        display_id: 1,
        frame: TargetFrame {
            x: 100.0,
            y: 60.0,
            width: f64::from(fw),
            height: f64::from(fh),
            scale: 2.0,
        },
    };

    let frames = vec![
        render_synthetic_demo_frame(fw, fh, 400, 0),
        render_synthetic_demo_frame(fw, fh, 1_500, 1),
        render_synthetic_demo_frame(fw, fh, 3_600, 1),
        render_synthetic_demo_frame(fw, fh, 6_400, 2),
        render_synthetic_demo_frame(fw, fh, 9_200, 2),
    ];

    // 1. Circle around the shifted header band (cx=0.52, cy=0.14, r_x=0.14, r_y=0.06)
    let mut circle_pts = Vec::new();
    for i in 0..=36 {
        let angle = (i as f32 / 36.0) * std::f32::consts::TAU;
        let x = ((0.52 + 0.13 * angle.cos()) * SCALE as f32).round() as i32;
        let y = ((0.15 + 0.055 * angle.sin()) * SCALE as f32).round() as i32;
        circle_pts.extend_from_slice(&[x, y, i * 18]);
    }
    let circle_stroke = Stroke {
        t0: 2_900,
        t1: 2_900 + 36 * 18,
        fw,
        fh,
        on: Some("Band header".into()),
        pin: None,
        laser: false,
        pts: circle_pts,
    };

    // 2. Arrow pointing from sidebar item to the tool card (shaft + arrowhead)
    let mut shaft_pts = Vec::new();
    for i in 0..=12 {
        let t = i as f32 / 12.0;
        let x = ((0.22 + (0.44 - 0.22) * t) * SCALE as f32).round() as i32;
        let y = ((0.48 + (0.36 - 0.48) * t) * SCALE as f32).round() as i32;
        shaft_pts.extend_from_slice(&[x, y, i * 16]);
    }
    let arrow_shaft = Stroke {
        t0: 5_800,
        t1: 5_800 + 12 * 16,
        fw,
        fh,
        on: Some("Used 12 tools".into()),
        pin: None,
        laser: false,
        pts: shaft_pts,
    };
    let arrow_head = Stroke {
        t0: 6_100,
        t1: 6_160,
        fw,
        fh,
        on: Some("Used 12 tools".into()),
        pin: None,
        laser: false,
        pts: vec![
            (0.41 * SCALE as f32) as i32,
            (0.35 * SCALE as f32) as i32,
            0,
            (0.44 * SCALE as f32) as i32,
            (0.36 * SCALE as f32) as i32,
            30,
            (0.42 * SCALE as f32) as i32,
            (0.40 * SCALE as f32) as i32,
            60,
        ],
    };

    // 3. Numbered pin on the composer send button
    let pin_stroke = Stroke {
        t0: 8_900,
        t1: 8_900,
        fw,
        fh,
        on: Some("Send button".into()),
        pin: Some(1),
        laser: false,
        pts: vec![
            (0.88 * SCALE as f32) as i32,
            (0.88 * SCALE as f32) as i32,
            0,
        ],
    };

    let strokes = vec![circle_stroke, arrow_shaft, arrow_head, pin_stroke];
    let events = vec![
        InteractionEvent::Click {
            t: 1_250,
            x: 0.46,
            y: 0.36,
            button: Some("left".into()),
            label: Some("Used 12 tools".into()),
            role: Some("button".into()),
        },
        InteractionEvent::Scroll {
            t: 4_800,
            x: 0.55,
            y: 0.55,
            dx: 0.0,
            dy: -240.0,
            label: Some("Transcript".into()),
        },
    ];

    let transcript = TranscriptData {
        source: "speech-analyzer".into(),
        model: Some("SpeechTranscriber en_US".into()),
        text: "When I click Used 12 tools this band header jumps down by four pixels, and the row spacing under the card feels too tight.".into(),
        segments: vec![
            TranscriptSegment {
                t0: 600,
                t1: 4_100,
                text: "When I click Used 12 tools this band header jumps down by four pixels,".into(),
            },
            TranscriptSegment {
                t0: 5_400,
                t1: 9_400,
                text: "and the row spacing under the card feels too tight.".into(),
            },
        ],
        words: vec![
            WordTiming { t0: 600, t1: 850, text: "When".into() },
            WordTiming { t0: 850, t1: 980, text: "I".into() },
            WordTiming { t0: 980, t1: 1_320, text: "click".into() },
            WordTiming { t0: 1_320, t1: 1_650, text: "Used".into() },
            WordTiming { t0: 1_650, t1: 1_900, text: "12".into() },
            WordTiming { t0: 1_900, t1: 2_300, text: "tools".into() },
            WordTiming { t0: 2_450, t1: 2_700, text: "this".into() },
            WordTiming { t0: 2_700, t1: 3_050, text: "band".into() },
            WordTiming { t0: 3_050, t1: 3_400, text: "header".into() },
            WordTiming { t0: 3_400, t1: 3_700, text: "jumps".into() },
            WordTiming { t0: 3_700, t1: 3_900, text: "down".into() },
            WordTiming { t0: 3_900, t1: 4_100, text: "by four pixels,".into() },
            WordTiming { t0: 5_400, t1: 5_800, text: "and the".into() },
            WordTiming { t0: 5_800, t1: 6_300, text: "row spacing".into() },
            WordTiming { t0: 6_300, t1: 7_200, text: "under the card".into() },
            WordTiming { t0: 7_800, t1: 9_400, text: "feels too tight.".into() },
        ],
        estimated: false,
        missing: None,
    };

    build_bundle_from_parts(
        &bundle_dir,
        target,
        9_800,
        frames,
        strokes,
        events,
        transcript,
        None,
        Some("Header inset shifts on expand".into()),
        SourceExtras::default(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_failures_do_not_publish_a_manifest() {
        let root = std::env::temp_dir().join(make_bundle_id());
        let (_, mut bundle, _) = generate_demo_bundle(Some(root.join("demo.scrawl"))).unwrap();
        let frame = RgbaFrame::new(0, 1, 1, vec![20, 30, 40, 255]);
        let mut key = bundle.keyframes[0].clone();
        key.file = "frames/missing/frame.png".into();
        let frame_failure = root.join("frame-failure.scrawl");
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
        assert!(!frame_failure.join("scrawl.json").exists());

        bundle.contact_sheet = "missing/contact-sheet.png".into();
        let sheet_failure = root.join("sheet-failure.scrawl");
        assert!(write_bundle_artifacts(&sheet_failure, &bundle, &[], &frame, "summary").is_err());
        assert!(!sheet_failure.join("scrawl.json").exists());

        bundle.contact_sheet = "contact-sheet.png".into();
        let empty_frames = root.join("empty-frames.scrawl");
        write_bundle_artifacts(&empty_frames, &bundle, &[], &frame, "summary").unwrap();
        assert!(empty_frames.join("scrawl.json").exists());
        assert!(empty_frames.join("contact-sheet.png").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for name in ["scrawl.json", "summary.md", "contact-sheet.png"] {
                let mode = std::fs::metadata(empty_frames.join(name))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777;
                assert_eq!(mode, 0o600, "{name} should be created 0600");
            }
        }
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
}
