//! `apparent zoom`: a dense contact sheet around one moment of a bundle, for looking closely at an
//! animation or glitch the narration points at.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};

use crate::shared::model::{InteractionEvent, Keyframe, KeyframeReason, ApparentBundle, parse_clock_ms};

pub struct ZoomSheet {
    pub path: PathBuf,
    /// `(index, time ms)` of each cell, in order.
    pub cells: Vec<(usize, u64)>,
    pub from_ms: u64,
    pub to_ms: u64,
    /// The narration overlapping the window.
    pub said: String,
}

fn normalize(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .to_lowercase()
}

/// Where `phrase` starts in the narration: the first run of transcript words matching it, else
/// the segment containing it.
pub fn find_phrase_ms(bundle: &ApparentBundle, phrase: &str) -> Option<u64> {
    let needle: Vec<String> = normalize(phrase)
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    if needle.is_empty() {
        return None;
    }
    let words = &bundle.transcript.words;
    let tokens: Vec<String> = words
        .iter()
        .map(|w| normalize(&w.text).replace(' ', ""))
        .collect();
    let matched = (0..tokens.len()).find(|&i| {
        needle
            .iter()
            .enumerate()
            .all(|(j, n)| tokens.get(i + j).is_some_and(|t| t == n))
    });
    if let Some(i) = matched {
        return Some(words[i].t0);
    }
    let joined = needle.join(" ");
    bundle
        .transcript
        .segments
        .iter()
        .find(|s| {
            normalize(&s.text)
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .contains(&joined)
        })
        .map(|s| s.t0)
}

/// Resolves `--at`: a time (`0:12`, `12.5`) or a phrase from the narration.
pub fn resolve_moment(bundle: &ApparentBundle, at: &str) -> Result<u64> {
    parse_clock_ms(at)
        .or_else(|| find_phrase_ms(bundle, at))
        .with_context(|| format!("“{at}” is neither a time nor a phrase in the narration"))
}

/// The window around `at_ms`: a quarter before, the rest after, since what the narration points
/// at usually happens as or just after it is said. Clamped to the recording.
pub fn window(at_ms: u64, span_ms: u64, duration_ms: u64) -> (u64, u64) {
    let span = span_ms.clamp(200, duration_ms.max(200));
    let mut from = at_ms.saturating_sub(span / 4);
    let mut to = from + span;
    if to > duration_ms {
        to = duration_ms;
        from = to.saturating_sub(span);
    }
    (from, to)
}

/// The window centred on a mark drawn over `[t0, t1]`: at least `span_ms`, and the whole mark
/// with half a second either side. Clamped to the recording.
pub fn mark_window(t0: u64, t1: u64, span_ms: u64, duration_ms: u64) -> (u64, u64) {
    let span = span_ms
        .max(t1.saturating_sub(t0) + 1_000)
        .clamp(200, duration_ms.max(200));
    let mut from = ((t0 + t1) / 2).saturating_sub(span / 2);
    let mut to = from + span;
    if to > duration_ms {
        to = duration_ms;
        from = to.saturating_sub(span);
    }
    (from, to)
}

/// What to zoom into: a moment (`--at`) or one mark (`--mark`).
pub enum Focus<'a> {
    At(&'a str),
    Mark(crate::capture::agent::context::MarkEntry<'a>),
}

/// Evenly spaced times across `[from, to]`, both ends included.
pub fn sample_times(from: u64, to: u64, count: usize) -> Vec<u64> {
    let count = count.clamp(1, 48);
    if count == 1 || to <= from {
        return vec![from];
    }
    (0..count)
        .map(|i| from + (to - from) * i as u64 / (count - 1) as u64)
        .collect()
}

/// How much of a gesture still under way a cell shows: the finger now and its recent path.
const TRAIL_MS: u64 = 350;

/// The interactions as they stood at `t`. A drag still under way is cut to its last
/// `TRAIL_MS`, so the cell shows where the finger is rather than the whole gesture to come
/// (its samples are evenly spaced in time, so a point's index gives its time).
fn events_at(events: &[InteractionEvent], t: u64) -> Vec<InteractionEvent> {
    events
        .iter()
        .map(|event| match event {
            InteractionEvent::Drag {
                t: t0,
                t1,
                path,
                label,
                button,
                ..
            } if *t0 <= t && t < *t1 && path.len() >= 2 => {
                let n = path.len() as u64;
                let at = |i: usize| t0 + (t1 - t0) * i as u64 / (n - 1);
                let done: Vec<usize> = (0..path.len()).filter(|&i| at(i) <= t).collect();
                let last = *done.last().unwrap_or(&0);
                let mut kept: Vec<[f32; 2]> = done
                    .into_iter()
                    .filter(|&i| at(i) + TRAIL_MS >= t)
                    .map(|i| path[i])
                    .collect();
                if kept.is_empty() {
                    kept.push(path[last]);
                }
                let [x0, y0] = kept[0];
                let [x1, y1] = kept[kept.len() - 1];
                InteractionEvent::Drag {
                    t: t.saturating_sub(TRAIL_MS).max(*t0),
                    t1: t,
                    x0,
                    y0,
                    x1,
                    y1,
                    label: label.clone(),
                    button: button.clone(),
                    path: kept,
                }
            }
            other => other.clone(),
        })
        .collect()
}

pub fn clock(ms: u64) -> String {
    format!("{}:{:02}.{:03}", ms / 60_000, ms / 1_000 % 60, ms % 1_000)
}

fn said_between(bundle: &ApparentBundle, from: u64, to: u64) -> String {
    let words: Vec<&str> = bundle
        .transcript
        .words
        .iter()
        .filter(|w| w.t1 >= from && w.t0 <= to)
        .map(|w| w.text.as_str())
        .collect();
    if !words.is_empty() {
        return words.join(" ");
    }
    bundle
        .transcript
        .segments
        .iter()
        .filter(|s| s.t1 >= from && s.t0 <= to)
        .map(|s| s.text.trim())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The auto-crop of the keyframe nearest `t` (keyframe crops meet halfway between keyframes).
fn crop_at(bundle: &ApparentBundle, t: u64) -> Option<crate::capture::engine::autocrop::autocrop::Rect> {
    let (k, _) = bundle
        .keyframes
        .iter()
        .enumerate()
        .min_by_key(|(_, kf)| kf.t.abs_diff(t))?;
    crop_of(bundle, k)
}

/// The auto-crop of the `k`th keyframe (0-based).
fn crop_of(bundle: &ApparentBundle, k: usize) -> Option<crate::capture::engine::autocrop::autocrop::Rect> {
    let crop = bundle.autocrop.as_ref()?.crops.get(k)?;
    (crop.kind != crate::capture::engine::autocrop::autocrop::CropKind::Full).then(|| crop.rect(&bundle.target.frame))
}

/// `cropped`: in an auto-cropped bundle, crop each cell like the nearest keyframe (for a mark,
/// like the mark's keyframe, whose crop holds its ink).
pub fn zoom(
    dir: &Path,
    bundle: &ApparentBundle,
    focus: Focus,
    span_ms: u64,
    count: usize,
    cropped: bool,
) -> Result<ZoomSheet> {
    let pending = dir.join(crate::shared::handoff::MEDIA_PENDING);
    // A bundle copied from another Mac gets its video a few seconds after the rest; wait for it.
    let waited = std::time::Instant::now();
    while pending.exists() && waited.elapsed() < std::time::Duration::from_secs(120) {
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    let video = bundle
        .video
        .as_ref()
        .map(|v| dir.join(v))
        .filter(|p| p.is_file())
        .with_context(|| {
            if pending.exists() {
                "The bundle's video is still copying from the other Mac; try again in a minute"
                    .to_owned()
            } else {
                crate::capture::engine::bundle::retention::missing_media_reason(dir)
                    .unwrap_or_else(|| "The bundle has no video to zoom into".to_owned())
            }
        })?;
    let ((from_ms, to_ms), mark) = match &focus {
        Focus::At(at) => {
            let at_ms = resolve_moment(bundle, at)?;
            ensure!(
                at_ms <= bundle.duration_ms,
                "{} is past the end ({})",
                clock(at_ms),
                clock(bundle.duration_ms)
            );
            (window(at_ms, span_ms, bundle.duration_ms), None)
        }
        Focus::Mark(entry) => (
            mark_window(entry.mark.t0, entry.mark.t1, span_ms, bundle.duration_ms),
            Some(entry),
        ),
    };
    let mark_crop = mark
        .and_then(|e| e.keyframe)
        .and_then(|index| bundle.keyframes.iter().position(|k| k.index == index))
        .map(|k| crop_of(bundle, k));
    let times = sample_times(from_ms, to_ms, count);
    let crops: Vec<Option<crate::capture::engine::autocrop::autocrop::Rect>> = times
        .iter()
        .map(|&t| {
            mark_crop
                .unwrap_or_else(|| crop_at(bundle, t))
                .filter(|_| cropped)
        })
        .collect();
    let mut frames = if crops.iter().any(Option::is_some) {
        // Decoded at full size a few at a time and cropped straight away, so a sheet of 2560 px
        // screens doesn't sit in memory.
        let mut out = Vec::with_capacity(times.len());
        for (chunk, rects) in times.chunks(6).zip(crops.chunks(6)) {
            let decoded = crate::capture::engine::bundle::bundle_processing::movie_frames_at(&video, chunk, bundle.video_offset_ms, 0)?;
            for (frame, rect) in decoded.into_iter().zip(rects) {
                let frame = match rect {
                    Some(r) => crate::capture::engine::autocrop::autocrop::crop_frame(&frame, *r),
                    None => frame,
                };
                out.push(crate::capture::engine::autocrop::autocrop_lab::downscale(&frame, 1280));
            }
        }
        out
    } else {
        crate::capture::engine::bundle::bundle_processing::movie_frames_at(&video, &times, bundle.video_offset_ms, 900)?
    };
    ensure!(!frames.is_empty(), "No frames could be decoded");
    let mut cells = Vec::with_capacity(frames.len());
    for (i, frame) in frames.iter_mut().enumerate() {
        let t = frame.t_ms;
        let events = events_at(&bundle.events, t);
        let rect = crops
            .get(i)
            .copied()
            .flatten()
            .unwrap_or(crate::capture::engine::autocrop::autocrop::FULL);
        // Labels are sized for the sheet's cell, not the decoded frame.
        let cell_width = if frame.height > frame.width {
            300.0
        } else {
            480.0
        };
        let label_scale = frame.width as f32 / cell_width;
        let keyframe = Keyframe {
            index: i + 1,
            t,
            reason: KeyframeReason::Speech,
            file: String::new(),
            event: String::new(),
            said: None,
            // From the moment the mark starts, its whole ink stays drawn.
            mark_id: mark.filter(|e| t >= e.mark.t0).map(|e| e.mark.id.clone()),
            cursor: None,
        };
        let view =
            crate::capture::engine::autocrop::autocrop::crop_view(rect, &keyframe, &bundle.strokes, &bundle.marks, &events);
        crate::capture::engine::render::annotate_keyframe_in_place(
            frame,
            &view.keyframe,
            &view.strokes,
            &view.marks,
            &view.events,
        );
        crate::capture::engine::render::draw_time_label(frame, &clock(t), label_scale);
        cells.push((i + 1, t));
    }
    let sheet = crate::capture::engine::render::render_zoom_sheet(&frames);
    let out_dir = dir.join("zoom");
    std::fs::create_dir_all(&out_dir)?;
    let path = out_dir.join(format!("zoom-{from_ms}-{to_ms}.png"));
    crate::capture::engine::render::save_png(&sheet, &path)?;
    Ok(ZoomSheet {
        path,
        cells,
        from_ms,
        to_ms,
        said: said_between(bundle, from_ms, to_ms),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shared::model::{TranscriptData, WordTiming};

    fn bundle_with_words(words: &[(&str, u64)]) -> ApparentBundle {
        let mut bundle: ApparentBundle = serde_json::from_value(serde_json::json!({
            "version": 1, "id": "t", "createdAtMs": 0, "durationMs": 25_000,
            "target": {"mode": "device", "displayId": 0,
                "frame": {"x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0, "scale": 1.0}},
            "transcript": {"source": "test", "text": "", "segments": []},
            "strokes": [], "marks": [], "events": [], "keyframes": [], "contactSheet": "c.png"
        }))
        .unwrap();
        bundle.transcript = TranscriptData {
            source: "test".into(),
            words: words
                .iter()
                .map(|&(text, t0)| WordTiming {
                    t0,
                    t1: t0 + 200,
                    text: text.into(),
                })
                .collect(),
            ..Default::default()
        };
        bundle
    }

    #[test]
    fn phrases_and_times_resolve() {
        let bundle = bundle_with_words(&[
            ("but,", 9_000),
            ("the", 9_300),
            ("Width", 9_500),
            ("changes.", 9_800),
        ]);
        assert_eq!(resolve_moment(&bundle, "width changes").unwrap(), 9_500);
        assert_eq!(resolve_moment(&bundle, "0:12").unwrap(), 12_000);
        assert!(resolve_moment(&bundle, "tabular numbers").is_err());
    }

    #[test]
    fn window_leans_after_the_moment_and_stays_inside() {
        assert_eq!(window(10_000, 2_000, 25_000), (9_500, 11_500));
        assert_eq!(window(200, 2_000, 25_000), (0, 2_000));
        assert_eq!(window(24_800, 2_000, 25_000), (23_000, 25_000));
        assert_eq!(sample_times(1_000, 2_000, 3), vec![1_000, 1_500, 2_000]);
        assert_eq!(clock(61_234), "1:01.234");
    }

    #[test]
    fn a_mark_window_is_centred_on_the_mark_and_holds_all_of_it() {
        assert_eq!(mark_window(9_322, 10_908, 2_000, 13_400), (8_822, 11_408));
        assert_eq!(mark_window(6_661, 7_182, 2_000, 13_400), (5_921, 7_921));
        assert_eq!(mark_window(200, 400, 2_000, 13_400), (0, 2_000));
        assert_eq!(mark_window(13_000, 13_300, 2_000, 13_400), (11_400, 13_400));
    }
}
