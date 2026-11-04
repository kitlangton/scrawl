//! Live ink animation, smoothing and freehand outlines.
use super::sketch::distance;
use std::f32::consts::PI;

/// A ⇧-click pin shows live only as a brief ring at the click, gone in this long; the pin itself
/// stays data (keyframes, summary). Ordinary clicks draw nothing live.
pub const PIN_PULSE_MS: u64 = 350;

/// The live pulse for a pin dropped `age_ms` ago: a thin ring's radius (points) and opacity. It
/// grows out fast and slows (ease-out cubic) while it fades to nothing.
pub fn pin_pulse(age_ms: u64) -> Option<(f32, f32)> {
    if age_ms >= PIN_PULSE_MS {
        return None;
    }
    let p = age_ms as f32 / PIN_PULSE_MS as f32;
    let grow = 1.0 - (1.0 - p).powi(3);
    Some((4.0 + 10.0 * grow, 0.55 * (1.0 - p) * (1.0 - p)))
}

/// Extracts the active `[now_ms - tail_ms, now_ms]` window of a timed laser stroke,
/// linearly interpolating the exact tail cutoff point at `t_cutoff` so the tail
/// glides continuously along the path at 120Hz instead of snapping vertex-by-vertex.
pub fn slice_laser_window(
    timed_points: &[(f32, f32, u64)],
    now_ms: u64,
    tail_ms: u64,
    to_screen: impl Fn(f32, f32) -> [f32; 2],
) -> Vec<([f32; 2], f32)> {
    if timed_points.is_empty() || tail_ms == 0 {
        return Vec::new();
    }
    let cutoff = now_ms.saturating_sub(tail_ms);
    let mut out = Vec::with_capacity(timed_points.len() + 1);

    for (idx, &(nx, ny, t)) in timed_points.iter().enumerate() {
        if t > now_ms.saturating_add(50) {
            break;
        }
        if t < cutoff {
            if let Some(&(nx1, ny1, t1)) = timed_points.get(idx + 1)
                && t1 >= cutoff
                && t1 > t
            {
                let frac = ((cutoff - t) as f32 / (t1 - t) as f32).clamp(0.0, 1.0);
                let p0 = to_screen(nx, ny);
                let p1 = to_screen(nx1, ny1);
                out.push((lerp(p0, p1, frac), 0.0));
            }
            continue;
        }
        let age_ms = now_ms.saturating_sub(t);
        let life = (1.0 - age_ms as f32 / tail_ms as f32).clamp(0.0, 1.0);
        out.push((to_screen(nx, ny), life));
    }
    out
}

/// Smooths timed `(x, y, life_0_to_1)` laser samples using tldraw's two-stage pipeline:
/// 1. Exponential `streamline` low-pass filter to remove mouse/trackpad stairstepping.
/// 2. Midpoint Quadratic B-Spline (`quadraticCurveTo(P_i, midpoint(P_i, P_{i+1}))`) subdivision,
///    which is variation-diminishing and $C^1$-continuous (never overshoots or kinks at corners).
/// 3. Combined arc-length start taper (`u * (2 - u)`) × time-decay life (`smoothstep`) so fast
///    strokes immediately taper to a fine needle tail and stationary strokes retract smoothly.
pub fn smooth_laser_points(points: &[([f32; 2], f32)]) -> Vec<([f32; 2], f32)> {
    if points.is_empty() {
        return Vec::new();
    }
    if points.len() == 1 {
        return vec![points[0]];
    }

    // Stage 1: tldraw streamline filter (streamline = 0.62 -> alpha = 0.47)
    let streamline_alpha = 0.47_f32;
    let mut filtered = Vec::with_capacity(points.len() + 1);
    filtered.push(points[0]);
    for &(pt, life) in &points[1..] {
        let prev = filtered.last().unwrap().0;
        let next_pt = lerp(prev, pt, streamline_alpha);
        if distance(prev, next_pt) >= 0.4 {
            filtered.push((next_pt, life));
        }
    }
    if let Some(&last) = points.last() {
        let prev = filtered.last().unwrap().0;
        if distance(prev, last.0) >= 0.4 {
            filtered.push((lerp(prev, last.0, 0.65), last.1));
            filtered.push(last);
        } else if let Some(slot) = filtered.last_mut() {
            *slot = last;
        }
    }

    if filtered.len() < 2 {
        return filtered;
    }

    // Stage 2: Midpoint Quadratic B-Spline (`quadraticCurveTo(P_i, midpoint(P_i, P_{i+1}))`)
    let mut curve = Vec::with_capacity(filtered.len() * 6);
    if filtered.len() == 2 {
        let (a, la) = filtered[0];
        let (b, lb) = filtered[1];
        let steps = (distance(a, b) / 2.5).ceil().clamp(2.0, 16.0) as usize;
        for s in 0..=steps {
            let t = s as f32 / steps as f32;
            curve.push((lerp(a, b, t), la + (lb - la) * t));
        }
    } else {
        let first = filtered[0];
        let second = filtered[1];
        let m0 = (lerp(first.0, second.0, 0.5), (first.1 + second.1) * 0.5);
        let start_steps = (distance(first.0, m0.0) / 2.5).ceil().clamp(2.0, 8.0) as usize;
        for s in 0..start_steps {
            let t = s as f32 / start_steps as f32;
            curve.push((lerp(first.0, m0.0, t), first.1 + (m0.1 - first.1) * t));
        }
        for i in 1..filtered.len() - 1 {
            let prev = filtered[i - 1];
            let ctrl = filtered[i];
            let next = filtered[i + 1];
            let a = (lerp(prev.0, ctrl.0, 0.5), (prev.1 + ctrl.1) * 0.5);
            let b = if i + 1 == filtered.len() - 1 {
                next
            } else {
                (lerp(ctrl.0, next.0, 0.5), (ctrl.1 + next.1) * 0.5)
            };
            let span_len = distance(a.0, ctrl.0) + distance(ctrl.0, b.0);
            let steps = (span_len / 2.2).ceil().clamp(3.0, 14.0) as usize;
            for s in 0..steps {
                let u = s as f32 / steps as f32;
                let inv = 1.0 - u;
                let x = inv * inv * a.0[0] + 2.0 * inv * u * ctrl.0[0] + u * u * b.0[0];
                let y = inv * inv * a.0[1] + 2.0 * inv * u * ctrl.0[1] + u * u * b.0[1];
                let life = inv * inv * a.1 + 2.0 * inv * u * ctrl.1 + u * u * b.1;
                curve.push(([x, y], life));
            }
        }
        if let Some(&last) = filtered.last() {
            curve.push(last);
        }
    }

    // Stage 3: Combine tldraw's arc-length start taper (`u * (2 - u)`) with time decay (`life`).
    let mut lengths = Vec::with_capacity(curve.len());
    let mut total_len = 0.0_f32;
    lengths.push(0.0_f32);
    for pair in curve.windows(2) {
        total_len += distance(pair[0].0, pair[1].0);
        lengths.push(total_len);
    }
    let taper_dist = total_len.clamp(16.0, 140.0);
    for (idx, item) in curve.iter_mut().enumerate() {
        let s = lengths[idx];
        let u = (s / taper_dist).clamp(0.0, 1.0);
        let arc_taper = u * (2.0 - u);
        let life = item.1.clamp(0.0, 1.0);
        let time_taper = life * life * (3.0 - 2.0 * life);
        item.1 = (arc_taper * time_taper).clamp(0.0, 1.0);
    }
    curve
}

/// Smooths a persistent ink stroke into `(point, pressure_scale)` samples using
/// tldraw's streamline filter + midpoint quadratic Bezier curves + simulated velocity pressure.
pub fn smooth_ink_points(points: &[[f32; 2]]) -> Vec<([f32; 2], f32)> {
    if points.is_empty() {
        return Vec::new();
    }
    if points.len() == 1 {
        return vec![(points[0], 0.85)];
    }
    let streamline_alpha = 0.45_f32;
    let mut filtered = Vec::with_capacity(points.len() + 1);
    filtered.push((points[0], 0.85_f32));
    let mut pressure = 0.85_f32;
    for &pt in &points[1..] {
        let prev = filtered.last().unwrap().0;
        let next_pt = lerp(prev, pt, streamline_alpha);
        let d = distance(prev, next_pt);
        if d >= 0.5 {
            let speed = (d / 14.0).clamp(0.0, 1.0);
            let target_w = 1.08 - 0.36 * speed;
            pressure = pressure + (target_w - pressure) * 0.32;
            filtered.push((next_pt, pressure.clamp(0.65, 1.12)));
        }
    }
    if let Some(&last) = points.last() {
        let prev = filtered.last().unwrap().0;
        if distance(prev, last) >= 0.5 {
            filtered.push((last, pressure));
        } else if let Some(slot) = filtered.last_mut() {
            slot.0 = last;
        }
    }
    if filtered.len() <= 2 {
        return filtered;
    }
    let mut curve = Vec::with_capacity(filtered.len() * 4);
    curve.push(filtered[0]);
    for i in 1..filtered.len() - 1 {
        let prev = filtered[i - 1];
        let ctrl = filtered[i];
        let next = filtered[i + 1];
        let a = (lerp(prev.0, ctrl.0, 0.5), (prev.1 + ctrl.1) * 0.5);
        let b = if i + 1 == filtered.len() - 1 {
            next
        } else {
            (lerp(ctrl.0, next.0, 0.5), (ctrl.1 + next.1) * 0.5)
        };
        let steps = ((distance(a.0, ctrl.0) + distance(ctrl.0, b.0)) / 3.0)
            .ceil()
            .clamp(2.0, 8.0) as usize;
        for s in 0..steps {
            let u = s as f32 / steps as f32;
            let inv = 1.0 - u;
            let x = inv * inv * a.0[0] + 2.0 * inv * u * ctrl.0[0] + u * u * b.0[0];
            let y = inv * inv * a.0[1] + 2.0 * inv * u * ctrl.0[1] + u * u * b.0[1];
            let w = inv * inv * a.1 + 2.0 * inv * u * ctrl.1 + u * u * b.1;
            curve.push(([x, y], w));
        }
    }
    if let Some(&last) = filtered.last() {
        curve.push(last);
    }
    curve
}

#[derive(Clone, Copy, Debug)]
pub struct FreehandOptions {
    pub size: f32,
    pub thinning: f32,
    pub smoothing: f32,
    pub streamline: f32,
    pub simulate_pressure: bool,
    pub taper_start: bool,
    pub taper_end: bool,
}

impl Default for FreehandOptions {
    fn default() -> Self {
        Self {
            size: 16.0,
            thinning: 0.5,
            smoothing: 0.5,
            streamline: 0.5,
            simulate_pressure: true,
            taper_start: false,
            taper_end: false,
        }
    }
}

fn add(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] + b[0], a[1] + b[1]]
}
fn sub(a: [f32; 2], b: [f32; 2]) -> [f32; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
fn mul(a: [f32; 2], n: f32) -> [f32; 2] {
    [a[0] * n, a[1] * n]
}
fn lerp(a: [f32; 2], b: [f32; 2], t: f32) -> [f32; 2] {
    add(a, mul(sub(b, a), t))
}
fn dot(a: [f32; 2], b: [f32; 2]) -> f32 {
    a[0] * b[0] + a[1] * b[1]
}
fn normal(a: [f32; 2]) -> [f32; 2] {
    [a[1], -a[0]]
}
fn rotate(point: [f32; 2], center: [f32; 2], angle: f32) -> [f32; 2] {
    let p = sub(point, center);
    let (sin, cos) = angle.sin_cos();
    add(center, [p[0] * cos - p[1] * sin, p[0] * sin + p[1] * cos])
}

struct StrokePoint {
    point: [f32; 2],
    vector: [f32; 2],
    distance: f32,
    length: f32,
}

/// Completed-stroke variant of perfect-freehand's streamline/pressure/outline pipeline.
/// Adapted from perfect-freehand 1.2.2, Copyright (c) 2021 Stephen Ruiz Ltd.
///
/// MIT License: Permission is hereby granted, free of charge, to any person obtaining
/// a copy of this software and associated documentation files (the "Software"), to
/// deal in the Software without restriction, including without limitation the rights
/// to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of
/// the Software, and to permit persons to whom the Software is furnished to do so,
/// subject to the following conditions: The above copyright notice and this
/// permission notice shall be included in all copies or substantial portions of the
/// Software. THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS
/// OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
/// FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
/// AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY,
/// WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN
/// CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.
pub fn freehand_outline(points: &[[f32; 2]], options: &FreehandOptions) -> Vec<[f32; 2]> {
    if !options.size.is_finite() || options.size <= 0.0 {
        return Vec::new();
    }
    let mut input: Vec<_> = points
        .iter()
        .copied()
        .filter(|p| p.iter().all(|v| v.is_finite()))
        .collect();
    input.dedup();
    if input.is_empty() {
        return Vec::new();
    }
    let size = options.size;
    let thinning = options.thinning.clamp(-1.0, 1.0);
    let radius = |pressure: f32| size * (0.5 - thinning * (0.5 - pressure));
    if input.len() == 1 {
        let r = radius(0.25).max(0.01);
        let center = input[0];
        let mut circle: Vec<_> = (0..32)
            .map(|i| {
                let angle = 2.0 * PI * i as f32 / 32.0;
                add(center, [r * angle.cos(), r * angle.sin()])
            })
            .collect();
        circle.push(circle[0]);
        return circle;
    }
    if input.len() == 2 {
        let (a, b) = (input[0], input[1]);
        input = (0..=4).map(|i| lerp(a, b, i as f32 / 4.0)).collect();
    }
    let streamline = 0.15 + (1.0 - options.streamline.clamp(0.0, 1.0)) * 0.85;
    let mut samples = vec![StrokePoint {
        point: input[0],
        vector: [0.0; 2],
        distance: 0.0,
        length: 0.0,
    }];
    let mut length = 0.0;
    let mut has_minimum_length = false;
    for (i, &p) in input.iter().enumerate().skip(1) {
        let prev = samples.last().unwrap().point;
        let point = if i == input.len() - 1 {
            p
        } else {
            lerp(prev, p, streamline)
        };
        let d = distance(prev, point);
        if d <= f32::EPSILON {
            continue;
        }
        length += d;
        if i < input.len() - 1 && !has_minimum_length {
            if length < size {
                continue;
            }
            has_minimum_length = true;
        }
        samples.push(StrokePoint {
            point,
            vector: mul(sub(prev, point), d.recip()),
            distance: d,
            length,
        });
    }
    if samples.len() == 1 {
        return freehand_outline(&[samples[0].point], options);
    }
    samples[0].vector = samples[1].vector;
    let simulate = |pressure: f32, d: f32| {
        let speed = (d / size).min(1.0);
        (pressure + (1.0 - speed - pressure) * (speed * 0.275)).min(1.0)
    };
    let mut pressure = 0.25;
    for (i, sample) in samples.iter().take(10).enumerate() {
        let next = if options.simulate_pressure {
            simulate(pressure, sample.distance)
        } else if i == 0 {
            0.25
        } else {
            0.5
        };
        pressure = (pressure + next) / 2.0;
    }
    let min_distance = (size * options.smoothing.clamp(0.0, 1.0)).powi(2);
    let mut left = Vec::new();
    let mut right = Vec::new();
    let mut prev_vector = samples[0].vector;
    let mut prev_left = samples[0].point;
    let mut prev_right = prev_left;
    let mut sharp = false;
    let mut end_radius = radius(pressure);
    let taper_length = size.max(length);
    for (i, sample) in samples.iter().enumerate() {
        let last = i == samples.len() - 1;
        if !last && length - sample.length < 3.0 {
            continue;
        }
        let p = if options.simulate_pressure {
            simulate(pressure, sample.distance)
        } else if i == 0 {
            0.25
        } else {
            0.5
        };
        let start_t = (sample.length / taper_length).min(1.0);
        let end_t = ((length - sample.length) / taper_length).min(1.0);
        let start = if options.taper_start {
            start_t * (2.0 - start_t)
        } else {
            1.0
        };
        let end = if options.taper_end {
            1.0 - (1.0 - end_t).powi(3)
        } else {
            1.0
        };
        let r = (radius(p) * start.min(end)).max(0.01);
        end_radius = r;
        let next = samples.get(i + 1).unwrap_or(sample).vector;
        let turn = dot(sample.vector, next);
        let sharp_next = turn < 0.0;
        if (dot(sample.vector, prev_vector) < 0.0 && !sharp) || sharp_next {
            let offset = mul(normal(prev_vector), r);
            for step in 0..=13 {
                let angle = PI * step as f32 / 13.0;
                prev_left = rotate(sub(sample.point, offset), sample.point, angle);
                prev_right = rotate(add(sample.point, offset), sample.point, -angle);
                left.push(prev_left);
                right.push(prev_right);
            }
            sharp = sharp_next;
            continue;
        }
        sharp = false;
        let offset = mul(
            normal(if last {
                sample.vector
            } else {
                lerp(next, sample.vector, turn)
            }),
            r,
        );
        let l = sub(sample.point, offset);
        let r = add(sample.point, offset);
        if last || i <= 1 || dot(sub(prev_left, l), sub(prev_left, l)) > min_distance {
            left.push(l);
            prev_left = l;
        }
        if last || i <= 1 || dot(sub(prev_right, r), sub(prev_right, r)) > min_distance {
            right.push(r);
            prev_right = r;
        }
        pressure = p;
        prev_vector = sample.vector;
    }
    let end = samples.last().unwrap().point;
    if options.taper_end {
        left.push(end);
    } else {
        let cap = add(
            end,
            mul(
                normal(mul(samples.last().unwrap().vector, -1.0)),
                end_radius,
            ),
        );
        for step in 1..14 {
            left.push(rotate(cap, end, PI * step as f32 / 14.0));
        }
    }
    let start_right = right.first().copied();
    left.extend(right.into_iter().rev());
    if options.taper_start {
        left.push(samples[0].point);
    } else if let Some(p) = start_right {
        for step in 1..=13 {
            left.push(rotate(p, samples[0].point, PI * step as f32 / 13.0));
        }
    }
    smooth_closed_polygon(&left)
}

/// Applies tldraw's `outline()` quadratic Bezier midpoint pass (`quadraticCurveTo(P_i, midpoint(P_i, P_{i+1}))`)
/// around a closed outline polygon so freehand strokes have zero polygonal facets.
fn smooth_closed_polygon(poly: &[[f32; 2]]) -> Vec<[f32; 2]> {
    if poly.len() < 4 {
        return poly.to_vec();
    }
    let n = poly.len();
    let mut out = Vec::with_capacity(n * 4);
    for i in 0..n {
        let prev = poly[(i + n - 1) % n];
        let ctrl = poly[i];
        let next = poly[(i + 1) % n];
        let a = lerp(prev, ctrl, 0.5);
        let b = lerp(ctrl, next, 0.5);
        for s in 0..4 {
            let u = s as f32 / 4.0;
            let inv = 1.0 - u;
            out.push([
                inv * inv * a[0] + 2.0 * inv * u * ctrl[0] + u * u * b[0],
                inv * inv * a[1] + 2.0 * inv * u * ctrl[1] + u * u * b[1],
            ]);
        }
    }
    if let Some(&first) = out.first() {
        out.push(first);
    }
    out
}
