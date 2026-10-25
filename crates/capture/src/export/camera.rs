//! The auto-zoom camera for video export: bundle events in, camera rectangles out.
//!
//! Pure and deterministic. `plan` turns the activity of a recording (clicks, drags, marks, laser
//! points, and the cursor's path) into a rectangle per frame; the helper only renders them.
//!
//! The approach follows ScreenDemon's click zoom (`sd-composition/src/lib.rs`, see the constants
//! below) with Kit's motion rules on top:
//!
//! - Nearby activity shares one zoom, focused on its bounding box; a zoom is held until
//!   `HOLD_MS` after the last activity, then eases back out (ScreenDemon: 1.5 s hold, 2.2 s
//!   cluster gap, 0.30 span; here 2.5 s for both, since a shareable video reads slower).
//! - The camera is critically damped springs on the view's scale and offset, not on the focus
//!   point, so a zoom travels toward its target as one move (ScreenDemon's `zoom_offset`:
//!   `offset = 0.5 − scale · focus`). No bounce. A retarget keeps position and velocity.
//! - Inside a zoom the focus follows the cursor only once it leaves a central safe zone
//!   (ScreenDemon's `follow_axis`, 0.22 / scale), so small movements don't drift the view.
//! - A zoom never starts while the cursor is travelling fast; it begins when the cursor arrives
//!   (or at the activity itself), and the follow pauses during fast travel.

use crate::autocrop::Rect;

/// Zoom for a click or other small activity (ScreenDemon `CLICK_ZOOM_SCALE`).
pub const ZOOM_SCALE: f32 = 1.8;
/// Activity needing a wider view than this scale allows is left unzoomed.
pub const MIN_ZOOM_SCALE: f32 = 1.5;
/// The activity may fill this much of the zoomed view.
pub const FIT_FRACTION: f32 = 0.7;
/// The zoom starts this long before the activity so it has arrived when it happens.
pub const LEAD_MS: u64 = 350;
/// How long a zoom is held after the last activity before easing out.
pub const HOLD_MS: u64 = 2_500;
/// Spring visual duration in seconds (Kit's `camera` row, 1.0–1.8 / 0, a little quicker for a
/// screen recording). ω = 2π / (1.2 · duration), ScreenDemon's Motion conversion.
pub const VISUAL_DURATION_S: f64 = 0.9;
/// Follow dead zone, as a fraction of the crop at scale 1 (ScreenDemon `margin = 0.22 / scale`).
pub const FOLLOW_SAFE: f32 = 0.22;
/// Cursor speed (crop widths per second) above which it is "travelling".
pub const FAST_TRAVEL: f32 = 0.9;
/// Samples further apart than this are not one movement.
const MAX_SAMPLE_GAP_MS: u64 = 400;
/// Travel stays "fast" this long after the last quick sample.
const TRAVEL_PAD_MS: u64 = 120;
/// Camera rectangles come out at this rate; the helper interpolates between them.
pub const STEP_MS: f64 = 1_000.0 / 60.0;

/// Activity at a time: a click, a drag, a mark, a laser trail. `rect` is normalized to the
/// capture, and times are on the video's clock.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Focus {
    pub t0: u64,
    pub t1: u64,
    pub rect: Rect,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CameraKey {
    pub t_ms: f64,
    /// `[x0, y0, x1, y1]` normalized to the capture; always inside the crop and with the crop's
    /// aspect.
    pub rect: Rect,
}

pub struct Input<'a> {
    pub duration_ms: u64,
    /// The export's crop: the camera never leaves it.
    pub crop: Rect,
    /// Capture width over height, to measure cursor distances.
    pub aspect: f32,
    pub focuses: &'a [Focus],
    /// `(t, x, y)` while the cursor moves, normalized to the capture.
    pub cursor: &'a [(u64, f32, f32)],
}

#[derive(Clone, Copy, Debug)]
struct Segment {
    start: u64,
    /// When the camera starts easing out if nothing else happens.
    release: u64,
    /// 1.0 for activity too spread out to zoom into: the camera goes wide for it.
    scale: f32,
    /// In crop space (0..1).
    center: [f32; 2],
}

/// Crop-space rectangle of one focus: `[x0, y0, x1, y1]`.
fn to_crop(crop: Rect, r: Rect) -> Rect {
    let (cw, ch) = ((crop[2] - crop[0]).max(1e-6), (crop[3] - crop[1]).max(1e-6));
    [
        ((r[0] - crop[0]) / cw).clamp(0.0, 1.0),
        ((r[1] - crop[1]) / ch).clamp(0.0, 1.0),
        ((r[2] - crop[0]) / cw).clamp(0.0, 1.0),
        ((r[3] - crop[1]) / ch).clamp(0.0, 1.0),
    ]
}

fn center_in(crop: Rect, r: Rect) -> bool {
    let (cx, cy) = ((r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0);
    cx >= crop[0] && cx <= crop[2] && cy >= crop[1] && cy <= crop[3]
}

fn span(r: Rect) -> f32 {
    (r[2] - r[0]).max(r[3] - r[1])
}

fn union(a: Rect, b: Rect) -> Rect {
    [
        a[0].min(b[0]),
        a[1].min(b[1]),
        a[2].max(b[2]),
        a[3].max(b[3]),
    ]
}

/// Where the cursor moves fast, as `[from, to]` ms ranges.
fn fast_travel(input: &Input) -> Vec<(u64, u64)> {
    let crop_w = ((input.crop[2] - input.crop[0]) * input.aspect).max(1e-6);
    let mut ranges: Vec<(u64, u64)> = Vec::new();
    for pair in input.cursor.windows(2) {
        let ((t0, x0, y0), (t1, x1, y1)) = (pair[0], pair[1]);
        let dt = t1.saturating_sub(t0);
        if dt == 0 || dt > MAX_SAMPLE_GAP_MS {
            continue;
        }
        let dist = ((x1 - x0) * input.aspect).hypot(y1 - y0) / crop_w;
        if dist / (dt as f32 / 1_000.0) <= FAST_TRAVEL {
            continue;
        }
        let end = t1 + TRAVEL_PAD_MS;
        match ranges.last_mut() {
            Some(last) if t0 <= last.1 => last.1 = last.1.max(end),
            _ => ranges.push((t0, end)),
        }
    }
    ranges
}

fn segments(input: &Input, travel: &[(u64, u64)]) -> Vec<Segment> {
    let mut focuses: Vec<(u64, u64, Rect)> = input
        .focuses
        .iter()
        .filter(|f| center_in(input.crop, f.rect))
        .map(|f| (f.t0, f.t1.max(f.t0), to_crop(input.crop, f.rect)))
        .collect();
    focuses.sort_by_key(|f| f.0);

    let max_span = FIT_FRACTION / MIN_ZOOM_SCALE;
    let mut out = Vec::new();
    let mut i = 0;
    while i < focuses.len() {
        let (t0, mut end, mut rect) = focuses[i];
        i += 1;
        while i < focuses.len() {
            let (next_t0, next_t1, next_rect) = focuses[i];
            let merged = union(rect, next_rect);
            if next_t0.saturating_sub(end) > HOLD_MS || span(merged) > max_span {
                break;
            }
            rect = merged;
            end = end.max(next_t1);
            i += 1;
        }
        let zoomable = span(rect) <= max_span;
        let scale = if zoomable {
            ZOOM_SCALE.min(FIT_FRACTION / span(rect).max(1e-3))
        } else {
            1.0
        };
        let mut start = if zoomable {
            t0.saturating_sub(LEAD_MS)
        } else {
            t0
        };
        // A zoom waits for the cursor to arrive, but never past the activity itself.
        if let Some(&(_, arrives)) = travel.iter().find(|&&(a, b)| a <= start && start < b) {
            start = arrives.min(t0);
        }
        out.push(Segment {
            start,
            release: end + HOLD_MS,
            scale,
            center: [(rect[0] + rect[2]) / 2.0, (rect[1] + rect[3]) / 2.0],
        });
    }
    out
}

/// Where the cursor is at `t` in crop space (it holds its last position between samples).
fn cursor_at(input: &Input, t: u64) -> Option<[f32; 2]> {
    let samples = input.cursor;
    let after = samples.partition_point(|s| s.0 <= t);
    let sample = |s: &(u64, f32, f32)| to_crop(input.crop, [s.1, s.2, s.1, s.2]);
    if after == 0 {
        return None;
    }
    let prev = samples[after - 1];
    let Some(next) = samples.get(after) else {
        let p = sample(&prev);
        return Some([p[0], p[1]]);
    };
    if next.0.saturating_sub(prev.0) > MAX_SAMPLE_GAP_MS {
        let p = sample(&prev);
        return Some([p[0], p[1]]);
    }
    let f = (t - prev.0) as f32 / (next.0 - prev.0).max(1) as f32;
    let (a, b) = (sample(&prev), sample(next));
    Some([a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f])
}

fn follow_axis(focus: f32, pointer: f32, margin: f32) -> f32 {
    let delta = pointer - focus;
    if delta.abs() <= margin {
        focus
    } else {
        (pointer - margin.copysign(delta)).clamp(0.0, 1.0)
    }
}

/// One critically damped spring step of `dt` seconds toward a constant target; exact.
fn spring(x: &mut f64, v: &mut f64, target: f64, omega: f64, dt: f64) {
    let d = *x - target;
    let c = *v + omega * d;
    let decay = (-omega * dt).exp();
    *x = target + (d + c * dt) * decay;
    *v = (*v - omega * c * dt) * decay;
}

fn in_ranges(ranges: &[(u64, u64)], t: u64) -> bool {
    ranges.iter().any(|&(a, b)| a <= t && t < b)
}

/// The camera path for a recording: a rectangle every `STEP_MS`, with runs of identical
/// rectangles reduced to their ends (interpolating the result reproduces the path).
pub fn plan(input: &Input) -> Vec<CameraKey> {
    thin(dense(input))
}

fn dense(input: &Input) -> Vec<CameraKey> {
    let travel = fast_travel(input);
    let segments = segments(input, &travel);
    let omega = 2.0 * std::f64::consts::PI / (1.2 * VISUAL_DURATION_S);
    let dt = STEP_MS / 1_000.0;
    let steps = (input.duration_ms as f64 / STEP_MS).ceil() as usize + 1;

    // View state in crop space: x_view = scale · x + offset, 0..1 on screen.
    let (mut s, mut vs) = (1.0_f64, 0.0_f64);
    let (mut ox, mut vox) = (0.0_f64, 0.0_f64);
    let (mut oy, mut voy) = (0.0_f64, 0.0_f64);
    let mut active: Option<usize> = None;
    let mut focus = [0.5_f32, 0.5];
    let mut rects: Vec<CameraKey> = Vec::with_capacity(steps);

    for k in 0..steps {
        let t_ms = k as f64 * STEP_MS;
        let t = t_ms as u64;
        let current = segments
            .iter()
            .rposition(|seg| seg.start <= t)
            .filter(|&i| t < segments[i].release);
        if current != active {
            active = current;
            if let Some(i) = current {
                focus = segments[i].center;
            }
        }
        let (target_scale, target_focus) = match current.map(|i| &segments[i]) {
            Some(seg) if seg.scale > 1.0 => {
                if !in_ranges(&travel, t)
                    && let Some(p) = cursor_at(input, t)
                {
                    let margin = FOLLOW_SAFE / seg.scale;
                    focus = [
                        follow_axis(focus[0], p[0], margin),
                        follow_axis(focus[1], p[1], margin),
                    ];
                }
                (seg.scale as f64, focus)
            }
            _ => (1.0, [0.5, 0.5]),
        };
        let offset = |f: f32| (0.5 - target_scale * f as f64).clamp(1.0 - target_scale, 0.0);
        spring(&mut s, &mut vs, target_scale, omega, dt);
        spring(&mut ox, &mut vox, offset(target_focus[0]), omega, dt);
        spring(&mut oy, &mut voy, offset(target_focus[1]), omega, dt);
        if s < 1.0 {
            (s, vs) = (1.0, 0.0);
        }
        let (lo_x, lo_y) = (1.0 - s, 1.0 - s);
        ox = ox.clamp(lo_x, 0.0);
        oy = oy.clamp(lo_y, 0.0);
        if target_scale == 1.0 && (s - 1.0).abs() < 0.002 && ox.abs() < 0.002 && oy.abs() < 0.002 {
            (s, vs, ox, vox, oy, voy) = (1.0, 0.0, 0.0, 0.0, 0.0, 0.0);
        }
        let (cx, cy, cw, ch) = (
            input.crop[0] as f64,
            input.crop[1] as f64,
            (input.crop[2] - input.crop[0]) as f64,
            (input.crop[3] - input.crop[1]) as f64,
        );
        let (vx0, vy0) = (-ox / s, -oy / s);
        let (vx1, vy1) = (vx0 + 1.0 / s, vy0 + 1.0 / s);
        rects.push(CameraKey {
            t_ms,
            rect: [
                (cx + vx0 * cw) as f32,
                (cy + vy0 * ch) as f32,
                (cx + vx1 * cw) as f32,
                (cy + vy1 * ch) as f32,
            ],
        });
    }
    rects
}

/// Drops a key whose rectangle equals both neighbours'.
fn thin(keys: Vec<CameraKey>) -> Vec<CameraKey> {
    let same = |a: &Rect, b: &Rect| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-6);
    let mut out: Vec<CameraKey> = Vec::with_capacity(keys.len());
    for (i, key) in keys.iter().enumerate() {
        let flat = i > 0
            && i + 1 < keys.len()
            && same(&keys[i - 1].rect, &key.rect)
            && same(&key.rect, &keys[i + 1].rect);
        if !flat {
            out.push(*key);
        }
    }
    out
}

/// The rectangle at `t_ms`, interpolating between keys (what the helper does per frame).
#[cfg(test)]
pub fn rect_at(keys: &[CameraKey], t_ms: f64) -> Rect {
    let after = keys.partition_point(|k| k.t_ms <= t_ms);
    if after == 0 {
        return keys[0].rect;
    }
    let Some(next) = keys.get(after) else {
        return keys[after - 1].rect;
    };
    let prev = keys[after - 1];
    let f = ((t_ms - prev.t_ms) / (next.t_ms - prev.t_ms)) as f32;
    std::array::from_fn(|i| prev.rect[i] + (next.rect[i] - prev.rect[i]) * f)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: Rect = [0.0, 0.0, 1.0, 1.0];

    fn click(t: u64, x: f32, y: f32) -> Focus {
        Focus {
            t0: t,
            t1: t,
            rect: [x, y, x, y],
        }
    }

    fn input<'a>(
        duration_ms: u64,
        crop: Rect,
        focuses: &'a [Focus],
        cursor: &'a [(u64, f32, f32)],
    ) -> Input<'a> {
        Input {
            duration_ms,
            crop,
            aspect: 16.0 / 9.0,
            focuses,
            cursor,
        }
    }

    fn scale_at(keys: &[CameraKey], t: f64) -> f32 {
        let r = rect_at(keys, t);
        1.0 / (r[2] - r[0])
    }

    fn centre_at(keys: &[CameraKey], t: f64) -> [f32; 2] {
        let r = rect_at(keys, t);
        [(r[0] + r[2]) / 2.0, (r[1] + r[3]) / 2.0]
    }

    #[test]
    fn zooms_in_on_a_click_holds_and_eases_out() {
        let focuses = [click(5_000, 0.7, 0.3)];
        let keys = plan(&input(14_000, FULL, &focuses, &[]));
        assert!((scale_at(&keys, 0.0) - 1.0).abs() < 1e-4);
        assert!(
            (scale_at(&keys, 4_000.0) - 1.0).abs() < 0.02,
            "no zoom early"
        );
        let mid = scale_at(&keys, 5_000.0);
        assert!(
            mid > 1.2 && mid < ZOOM_SCALE,
            "zoom is under way at the click: {mid}"
        );
        assert!(scale_at(&keys, 6_500.0) > ZOOM_SCALE - 0.02, "arrived");
        let at_hold = scale_at(&keys, 7_400.0);
        assert!(
            at_hold > ZOOM_SCALE - 0.02,
            "held until the hold ends: {at_hold}"
        );
        let c = centre_at(&keys, 6_500.0);
        assert!(
            (c[0] - 0.7).abs() < 0.03 && (c[1] - 0.3).abs() < 0.05,
            "centred on the click: {c:?}"
        );
        assert!(scale_at(&keys, 8_500.0) < ZOOM_SCALE - 0.1, "easing out");
        assert!(
            (scale_at(&keys, 13_000.0) - 1.0).abs() < 1e-3,
            "back to the full crop"
        );
        assert_eq!(rect_at(&keys, 13_900.0), FULL);
    }

    #[test]
    fn nearby_activity_shares_one_zoom_until_after_the_last() {
        let focuses = [
            click(5_000, 0.50, 0.50),
            click(6_800, 0.55, 0.52),
            click(8_500, 0.52, 0.48),
        ];
        let keys = plan(&input(16_000, FULL, &focuses, &[]));
        let floor = (6_000..11_000)
            .step_by(100)
            .map(|t| scale_at(&keys, t as f64))
            .fold(f32::MAX, f32::min);
        assert!(
            floor > ZOOM_SCALE - 0.02,
            "never eases out between clicks: {floor}"
        );
        assert!(scale_at(&keys, 13_500.0) < 1.15, "eases out after the last");
    }

    #[test]
    fn distant_activity_retargets_without_zooming_out() {
        let focuses = [click(5_000, 0.2, 0.3), click(7_000, 0.8, 0.7)];
        let keys = plan(&input(14_000, FULL, &focuses, &[]));
        let floor = (6_000..9_000)
            .step_by(50)
            .map(|t| scale_at(&keys, t as f64))
            .fold(f32::MAX, f32::min);
        assert!(
            floor > MIN_ZOOM_SCALE,
            "stays zoomed while panning across: {floor}"
        );
        let c = centre_at(&keys, 9_000.0);
        assert!(c[0] > 0.65, "ends on the second click: {c:?}");
    }

    #[test]
    fn spread_out_activity_is_left_unzoomed() {
        let focuses = [Focus {
            t0: 3_000,
            t1: 5_000,
            rect: [0.05, 0.1, 0.95, 0.9],
        }];
        let keys = plan(&input(9_000, FULL, &focuses, &[]));
        assert!(keys.iter().all(|k| k.rect == FULL));
    }

    #[test]
    fn a_wide_gesture_pulls_a_held_zoom_back_out() {
        let focuses = [
            click(3_000, 0.5, 0.5),
            Focus {
                t0: 4_000,
                t1: 4_500,
                rect: [0.05, 0.1, 0.95, 0.9],
            },
        ];
        let keys = plan(&input(12_000, FULL, &focuses, &[]));
        assert!(scale_at(&keys, 3_900.0) > 1.6);
        assert!(
            scale_at(&keys, 6_500.0) < 1.05,
            "wide again for the wide activity"
        );
    }

    #[test]
    fn does_not_zoom_while_the_cursor_travels_fast() {
        // Sweeping from the left edge to the click in 0.8 s (≈ 1.1 crop widths per second).
        let cursor: Vec<(u64, f32, f32)> = (0..=12)
            .map(|i| (2_300 + i * 67, 0.1 + 0.8 * i as f32 / 12.0, 0.5))
            .collect();
        let focuses = [click(3_100, 0.9, 0.5)];
        let keys = plan(&input(9_000, FULL, &focuses, &cursor));
        for t in (2_300..3_100).step_by(50) {
            assert!(
                scale_at(&keys, t as f64) < 1.02,
                "zoomed at {t} ms during fast travel: {}",
                scale_at(&keys, t as f64)
            );
        }
        assert!(
            scale_at(&keys, 4_800.0) > ZOOM_SCALE - 0.05,
            "zooms once there"
        );

        // The same click with a slow approach zooms ahead of it.
        let slow: Vec<(u64, f32, f32)> = (0..=12)
            .map(|i| (2_300 + i * 67, 0.85 + 0.05 * i as f32 / 12.0, 0.5))
            .collect();
        let keys = plan(&input(9_000, FULL, &focuses, &slow));
        assert!(scale_at(&keys, 3_100.0) > 1.3, "zoom leads a calm click");
    }

    #[test]
    fn follows_the_cursor_only_outside_the_safe_zone() {
        let focuses = [click(2_000, 0.5, 0.5)];
        let fidget: Vec<(u64, f32, f32)> = (0..30)
            .map(|i| (2_500 + i * 66, 0.5 + 0.02 * (i as f32 * 0.7).sin(), 0.5))
            .collect();
        let keys = plan(&input(8_000, FULL, &focuses, &fidget));
        let c = centre_at(&keys, 4_000.0);
        assert!(
            (c[0] - 0.5).abs() < 0.01,
            "small movement doesn't pan: {c:?}"
        );

        // Drifting right slowly (0.25 crop widths per second) pulls the view along.
        let drift: Vec<(u64, f32, f32)> = (0..=24)
            .map(|i| (2_500 + i * 66, 0.5 + 0.0165 * i as f32, 0.5))
            .collect();
        let keys = plan(&input(8_000, FULL, &focuses, &drift));
        let c = centre_at(&keys, 4_500.0);
        assert!(
            c[0] > 0.55,
            "follows a cursor that leaves the safe zone: {c:?}"
        );
        let r = rect_at(&keys, 4_500.0);
        assert!(r[2] <= 1.0 + 1e-5 && r[0] >= -1e-5);
    }

    #[test]
    fn rects_stay_inside_the_crop_at_the_edges() {
        let crop = [0.2, 0.1, 0.9, 0.8];
        let focuses = [
            click(2_000, 0.89, 0.79),
            click(5_000, 0.21, 0.11),
            click(8_000, 0.55, 0.45),
        ];
        let keys = plan(&input(14_000, crop, &focuses, &[]));
        for key in &keys {
            let r = key.rect;
            assert!(r[0] >= crop[0] - 1e-5 && r[1] >= crop[1] - 1e-5, "{r:?}");
            assert!(r[2] <= crop[2] + 1e-5 && r[3] <= crop[3] + 1e-5, "{r:?}");
            let aspect = (r[2] - r[0]) / (r[3] - r[1]);
            let want = (crop[2] - crop[0]) / (crop[3] - crop[1]);
            assert!((aspect - want).abs() < 1e-3, "aspect {aspect} vs {want}");
        }
    }

    #[test]
    fn activity_outside_the_crop_does_not_move_the_camera() {
        let crop = [0.0, 0.0, 0.5, 0.5];
        let focuses = [click(2_000, 0.9, 0.9)];
        let keys = plan(&input(6_000, crop, &focuses, &[]));
        assert!(keys.iter().all(|k| k.rect == crop));
    }

    #[test]
    fn the_camera_moves_smoothly_and_never_overshoots() {
        let focuses = [
            click(3_000, 0.3, 0.3),
            click(4_000, 0.7, 0.6),
            click(12_000, 0.5, 0.5),
        ];
        let keys = plan(&input(20_000, FULL, &focuses, &[]));
        let mut previous = rect_at(&keys, 0.0);
        let mut biggest = 0.0_f32;
        let mut jerk = 0.0_f32;
        let mut last_step = [0.0_f32; 4];
        let mut t = 0.0;
        while t < 20_000.0 {
            let r = rect_at(&keys, t);
            let delta: [f32; 4] = std::array::from_fn(|i| r[i] - previous[i]);
            biggest = biggest.max(delta.iter().fold(0.0, |m, d| m.max(d.abs())));
            jerk = jerk.max(
                (0..4)
                    .map(|i| (delta[i] - last_step[i]).abs())
                    .fold(0.0, f32::max),
            );
            last_step = delta;
            assert!(1.0 / (r[2] - r[0]) <= ZOOM_SCALE + 1e-3, "overshoot at {t}");
            previous = r;
            t += STEP_MS;
        }
        assert!(biggest < 0.02, "no frame jumps: {biggest}");
        assert!(jerk < 0.006, "velocity never snaps: {jerk}");
    }

    #[test]
    fn thinning_keeps_the_path() {
        let focuses = [click(2_000, 0.6, 0.4), click(7_000, 0.3, 0.6)];
        let i = input(12_000, FULL, &focuses, &[]);
        let all = dense(&i);
        let keys = plan(&i);
        assert!(
            keys.len() < all.len(),
            "idle runs collapse: {} of {}",
            keys.len(),
            all.len()
        );
        assert_eq!(keys.first().unwrap().t_ms, 0.0);
        for key in &all {
            let r = rect_at(&keys, key.t_ms);
            assert!(
                (0..4).all(|j| (r[j] - key.rect[j]).abs() < 1e-4),
                "at {}",
                key.t_ms
            );
        }
    }
}
