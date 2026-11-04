//! Mark recognition and compatibility exports for semantic output and drawing geometry.
pub use super::sketch_geometry::*;
pub use super::sketch_semantics::*;

use crate::shared::model::*;
#[cfg(test)]
use std::f32::consts::PI;

pub fn bounds(points: &[[f32; 2]]) -> [f32; 4] {
    let Some(&[x, y]) = points.first() else {
        return [0.0; 4];
    };
    points.iter().fold([x, y, x, y], |b, p| {
        [
            b[0].min(p[0]),
            b[1].min(p[1]),
            b[2].max(p[0]),
            b[3].max(p[1]),
        ]
    })
}

pub(super) fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

#[cfg(test)]
pub fn shape_of(stroke: &Stroke) -> Shape {
    ink_of(stroke).shape
}

/// One stroke's shape and what a mark needs from it, in normalized coordinates.
struct Ink {
    shape: Shape,
    /// What the stroke covers: a circle's loop without its lead-in or tail, else the whole stroke.
    region: [f32; 4],
    /// A one-stroke arrow's shaft start and tip.
    ends: Option<([f32; 2], [f32; 2])>,
}

fn ink_of(stroke: &Stroke) -> Ink {
    let norm = stroke.normalized_points();
    let region = bounds(&norm);
    let plain = |shape| Ink {
        shape,
        region,
        ends: None,
    };
    if stroke.pin.is_some() {
        return plain(Shape::Pin);
    }
    let points: Vec<_> = norm
        .iter()
        .map(|p| [p[0] * stroke.fw as f32, p[1] * stroke.fh as f32])
        .collect();
    let Some(&first) = points.first() else {
        return plain(Shape::Tap);
    };
    let b = bounds(&points);
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    let len: f32 = points.windows(2).map(|p| distance(p[0], p[1])).sum();
    let last = *points.last().unwrap();
    let chord = distance(first, last);
    let shoelace = points
        .windows(2)
        .map(|p| p[0][0] * p[1][1] - p[1][0] * p[0][1])
        .sum::<f32>()
        + last[0] * first[1]
        - first[0] * last[1];
    let loop_box = || loop_in(&points).map(|(i, j)| bounds(&norm[i..=j]));
    if w.hypot(h) < 14.0 {
        plain(Shape::Tap)
    } else if encloses(chord, 0.35, len, b, 0.5 * shoelace.abs()) {
        // A short tail or lead-in can still end close enough to count; what was circled is the
        // loop inside it. A stroke that goes round more than once keeps its whole box.
        Ink {
            shape: Shape::Circle,
            region: (len <= 1.5 * round(b))
                .then(loop_box)
                .flatten()
                .unwrap_or(region),
            ends: None,
        }
    } else if chord > 0.88 * len {
        plain(if w > 2.5 * h {
            Shape::Underline
        } else {
            Shape::Line
        })
    } else if let Some(region) = loop_box() {
        Ink {
            shape: Shape::Circle,
            region,
            ends: None,
        }
    } else if let Some(tip) = arrow_tip(&points, w.max(h)) {
        Ink {
            shape: Shape::Arrow,
            region,
            ends: Some((norm[0], norm[tip])),
        }
    } else {
        plain(Shape::Mark)
    }
}

/// The most points [`loop_in`] tries as a loop's ends.
const MAX_LOOP_POINTS: usize = 512;

/// Ramanujan's approximation of the perimeter of the ellipse filling box `b`.
fn round(b: [f32; 4]) -> f32 {
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    PI / 2.0 * (3.0 * (w + h) - ((3.0 * w + h) * (w + 3.0 * h)).sqrt())
}

/// Whether a path (pixels) with these measures goes round something: it ends within `max_gap` of
/// its long side of where it began, is longer than twice its long side, fills a third of its box,
/// and the box's short side is over 0.42 of its long side. Flatter loops, to 5:1, count too (a
/// wide cell or a line of text is circled with a flat loop) when they also close well within
/// their short side, so a flat C doesn't, and go round at most three times (with slack for
/// jitter), so a scribble over a line of text, which closes on itself and fills its box too,
/// doesn't either.
fn encloses(gap: f32, max_gap: f32, len: f32, b: [f32; 4], area: f32) -> bool {
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    let (big, small) = (w.max(h), w.min(h));
    gap < max_gap * big
        && len > 2.0 * big
        && small >= 18.0
        && area > 0.32 * w * h
        && (small > 0.42 * big || (small > 0.2 * big && gap < 0.6 * small && len <= 3.5 * round(b)))
}

/// The loop inside a stroke (pixels) that circles something once before or after a lead-in,
/// tail or overshoot: the stretch `i..=j`, at least 0.6 of the stroke's length, whose ends meet
/// (within 0.15 of its size), that [`encloses`] its box and goes round it at most one and a half
/// times. Of several, the one whose ends meet closest.
///
/// Trying every pair of ends is quadratic and imported strokes have no point cap, so the ends
/// are tried at every `step`th point only, but each stretch is judged by the recorded path's own
/// length and area (prefix sums over every point), so sampling can't hide backtracking or turns.
fn loop_in(points: &[[f32; 2]]) -> Option<(usize, usize)> {
    let n = points.len();
    if n < 3 {
        return None;
    }
    // Length along the path, and twice the shoelace area, up to each point (f64: pixel products
    // are too big to subtract precisely in f32, and long strokes add up many segments).
    let mut run = Vec::with_capacity(n);
    let mut twice_area = Vec::with_capacity(n);
    run.push(0.0f64);
    twice_area.push(0.0f64);
    for p in points.windows(2) {
        run.push(run.last().unwrap() + distance(p[0], p[1]) as f64);
        twice_area.push(
            twice_area.last().unwrap() + p[0][0] as f64 * p[1][1] as f64
                - p[1][0] as f64 * p[0][1] as f64,
        );
    }
    let total = run[n - 1];
    let step = n.div_ceil(MAX_LOOP_POINTS);
    let mut ends: Vec<usize> = (0..n).step_by(step).collect();
    if ends.last() != Some(&(n - 1)) {
        ends.push(n - 1);
    }
    let mut best: Option<(f32, usize, usize)> = None;
    for (k, &i) in ends.iter().enumerate() {
        let a = points[i];
        let mut b = [a[0], a[1], a[0], a[1]];
        for &j in &ends[k + 1..] {
            let z = points[j];
            b = [
                b[0].min(z[0]),
                b[1].min(z[1]),
                b[2].max(z[0]),
                b[3].max(z[1]),
            ];
            let len = run[j] - run[i];
            if len < 0.6 * total {
                continue;
            }
            let gap = distance(a, z);
            let closeness = gap / (b[2] - b[0]).max(b[3] - b[1]);
            if best.is_some_and(|(c, ..)| closeness >= c) {
                continue;
            }
            let len = len as f32;
            let twice = twice_area[j] - twice_area[i] + z[0] as f64 * a[1] as f64
                - a[0] as f64 * z[1] as f64;
            if len <= 1.5 * round(b) && encloses(gap, 0.15, len, b, 0.5 * twice.abs() as f32) {
                best = Some((closeness, i, j));
            }
        }
    }
    best.map(|(_, i, j)| (i, j))
}

/// How far `p` is from the line through `a` and `b`.
fn off_line(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    ((b[0] - a[0]) * (a[1] - p[1]) - (a[0] - p[0]) * (b[1] - a[1])).abs() / distance(a, b)
}

/// The tip of an arrow drawn in one stroke (pixels, `big` its box's long side): a straight shaft
/// from the first point to the tip, where the stroke turns back into one or two straight barbs
/// that end behind the tip, and the head reaches both sides of the shaft. Two barbs (out, back
/// and out again) are each at most 0.9 of the shaft; a lone barb, which must then run on across
/// the shaft, is at most 0.6 of it and clearly off it. An underline whose end hooks back to one
/// side, check marks, V's and zigzags fail.
fn arrow_tip(points: &[[f32; 2]], big: f32) -> Option<usize> {
    let start = points[0];
    // The tip is where the stroke first heads back toward its start, once half its size away.
    let mut tip = 0;
    let mut turned = false;
    for (i, &p) in points.iter().enumerate() {
        let (d, far) = (distance(start, p), distance(start, points[tip]));
        if d >= far {
            tip = i;
        } else if far >= 0.5 * big && d < 0.85 * far {
            turned = true;
            break;
        }
    }
    let t = points[tip];
    let shaft = distance(start, t);
    if !turned
        || points[..tip]
            .iter()
            .any(|&p| off_line(p, start, t) > 0.1 * shaft)
    {
        return None;
    }
    // The direction the shaft arrives in, over its last 30%.
    let arc: f32 = points[..=tip]
        .windows(2)
        .map(|p| distance(p[0], p[1]))
        .sum();
    let mut run = 0.0;
    let back = (0..tip)
        .find(|&i| {
            run += distance(points[i], points[i + 1]);
            run >= 0.7 * arc
        })
        .unwrap_or(0);
    let along = distance(points[back], t);
    if along < 1.0 {
        return None;
    }
    let dir = [
        (t[0] - points[back][0]) / along,
        (t[1] - points[back][1]) / along,
    ];
    // Barbs: runs away from the tip by at least `slack`, each from the nearest point before it.
    let slack = 0.15 * shaft;
    let mut barbs: Vec<(usize, usize)> = Vec::new();
    let (mut out, mut near, mut rising) = (tip, tip, true);
    for (i, &p) in points.iter().enumerate().skip(tip + 1) {
        if (p[0] - t[0]) * dir[0] + (p[1] - t[1]) * dir[1] > slack {
            return None;
        }
        let d = distance(t, p);
        if rising {
            if d >= distance(t, points[out]) {
                out = i;
            } else if d < distance(t, points[out]) - slack {
                barbs.push((near, out));
                (near, rising) = (i, false);
            }
        } else if d <= distance(t, points[near]) {
            near = i;
        } else if d > distance(t, points[near]) + slack {
            (out, rising) = (i, true);
        }
    }
    if rising && distance(t, points[out]) >= slack {
        barbs.push((near, out));
    }
    let cos = |deg: f32| deg.to_radians().cos();
    // Each barb's length and the cosine of its angle to the shaft pointing back.
    let mut measured = Vec::with_capacity(barbs.len());
    for (from, end) in barbs {
        let (a, z) = (points[from], points[end]);
        let span = distance(a, z);
        if points[from..=end]
            .iter()
            .any(|&p| off_line(p, a, z) > 0.15 * span)
        {
            return None;
        }
        let e = [z[0] - t[0], z[1] - t[1]];
        let d = e[0].hypot(e[1]);
        let c = -(e[0] * dir[0] + e[1] * dir[1]) / d;
        if d > 0.9 * shaft || c < cos(75.0) {
            return None;
        }
        measured.push((d, c));
    }
    let head = match measured[..] {
        [(d, c)] => d <= 0.6 * shaft && c <= cos(15.0),
        [(_, c0), (_, c1)] => c0.max(c1) <= cos(5.0),
        _ => false,
    };
    // The head spreads to both sides of the shaft; an underline that hooks back doesn't.
    let side = |p: [f32; 2]| dir[0] * (p[1] - t[1]) - dir[1] * (p[0] - t[0]);
    let reach = |sign: f32| {
        points[tip..]
            .iter()
            .map(|&p| sign * side(p))
            .fold(0.0, f32::max)
    };
    let (left, right) = (reach(1.0), reach(-1.0));
    (head && left.min(right) > 0.3 * left.max(right)).then_some(tip)
}

pub fn marks_of(strokes: &[Stroke]) -> Vec<Mark> {
    let mut groups: Vec<Vec<usize>> = Vec::new();
    for (i, stroke) in strokes.iter().enumerate().filter(|(_, s)| !s.laser) {
        if let Some(group) = groups.last_mut() {
            let prev = &strokes[*group.last().unwrap()];
            if prev.pin.is_none()
                && stroke.pin.is_none()
                && stroke.t0.saturating_sub(prev.t1) <= MARK_GAP_MS
                && group.len() < MAX_MARK_STROKES
            {
                group.push(i);
                continue;
            }
        }
        groups.push(vec![i]);
    }
    groups
        .into_iter()
        .enumerate()
        .map(|(i, group)| {
            let first = &strokes[group[0]];
            let all: Vec<_> = group
                .iter()
                .flat_map(|&j| strokes[j].normalized_points())
                .collect();
            let mut mark = Mark {
                id: format!("m{}", i + 1),
                t0: first.t0,
                t1: group
                    .iter()
                    .map(|&j| strokes[j].t1)
                    .max()
                    .unwrap_or(first.t1),
                box_norm: bounds(&all),
                shape: Shape::Mark,
                from: None,
                to: None,
                pin: first.pin,
                on: group.iter().find_map(|&j| strokes[j].on.clone()),
                strokes: group.clone(),
                description: String::new(),
                frame: None,
            };
            if first.pin.is_some() {
                let [x, y] = first
                    .normalized_points()
                    .first()
                    .copied()
                    .unwrap_or([0.0; 2]);
                mark.box_norm = [x, y, x, y];
                mark.shape = Shape::Pin;
            } else {
                let mut sized: Vec<_> = group
                    .iter()
                    .map(|&j| {
                        let s = &strokes[j];
                        let pts = s.normalized_points();
                        let b = bounds(&pts);
                        let size = (b[2] - b[0]) * s.fw as f32 + (b[3] - b[1]) * s.fh as f32;
                        (s, ink_of(s), b, pts, size)
                    })
                    .collect();
                sized.sort_by(|a, b| b.4.total_cmp(&a.4));
                let (main, ink, _, pts, _) = &sized[0];
                let shape = &ink.shape;
                let rest = &sized[1..];
                let a = pts.first().copied().unwrap_or([0.0; 2]);
                let z = pts.last().copied().unwrap_or(a);
                if matches!(shape, Shape::Line | Shape::Underline) && (1..=2).contains(&rest.len())
                {
                    let px = |p: [f32; 2]| [p[0] * main.fw as f32, p[1] * main.fh as f32];
                    let len = distance(px(a), px(z));
                    let near = |p| {
                        rest.iter().all(|r| {
                            distance(
                                px([(r.2[0] + r.2[2]) / 2.0, (r.2[1] + r.2[3]) / 2.0]),
                                px(p),
                            ) < 0.35 * len
                        })
                    };
                    if near(z) || near(a) {
                        mark.shape = Shape::Arrow;
                        mark.from = Some(if near(z) { a } else { z });
                        mark.to = Some(if near(z) { z } else { a });
                    }
                }
                if mark.shape != Shape::Arrow
                    && (rest.iter().all(|r| r.1.shape == *shape) || *shape == Shape::Circle)
                {
                    match shape {
                        Shape::Circle => {
                            mark.shape = Shape::Circle;
                            // What was circled: the loops, without any lead-in or tail.
                            mark.box_norm = if rest.iter().all(|r| r.1.shape == Shape::Circle) {
                                bounds(
                                    &sized
                                        .iter()
                                        .flat_map(|r| {
                                            let [x0, y0, x1, y1] = r.1.region;
                                            [[x0, y0], [x1, y1]]
                                        })
                                        .collect::<Vec<_>>(),
                                )
                            } else {
                                ink.region
                            };
                        }
                        Shape::Line if rest.is_empty() => {
                            mark.shape = Shape::Line;
                            mark.from = Some(a);
                            mark.to = Some(z);
                        }
                        Shape::Arrow if rest.is_empty() => {
                            mark.shape = Shape::Arrow;
                            mark.from = ink.ends.map(|e| e.0);
                            mark.to = ink.ends.map(|e| e.1);
                        }
                        Shape::Tap | Shape::Underline => mark.shape = *shape,
                        _ => {}
                    }
                }
            }
            mark.description = describe_mark(&mark);
            mark
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A pin pulses once and is gone: nothing click-related stays on screen while recording.
    #[test]
    fn a_pin_pulse_grows_fades_and_ends() {
        let (r0, a0) = pin_pulse(0).unwrap();
        let (r1, a1) = pin_pulse(PIN_PULSE_MS / 2).unwrap();
        let (r2, a2) = pin_pulse(PIN_PULSE_MS - 1).unwrap();
        assert!(r0 < r1 && r1 < r2 && r2 <= 14.0);
        assert!(a0 > a1 && a1 > a2 && a2 < 0.01 && a0 <= 0.55);
        assert_eq!(pin_pulse(PIN_PULSE_MS), None);
        assert_eq!(pin_pulse(60_000), None);
    }

    fn stroke(points: &[[f32; 2]], t: u64) -> Stroke {
        Stroke {
            t0: t,
            t1: t + 100,
            fw: 1000,
            fh: 1000,
            on: None,
            pin: None,
            laser: false,
            pts: points
                .iter()
                .flat_map(|p| {
                    [
                        (p[0] * SCALE as f32).round() as i32,
                        (p[1] * SCALE as f32).round() as i32,
                        0,
                    ]
                })
                .collect(),
        }
    }

    #[test]
    fn shapes_are_measured_in_pixels() {
        assert_eq!(
            shape_of(&stroke(&[[0.5, 0.5], [0.505, 0.505]], 0)),
            Shape::Tap
        );
        assert_eq!(
            shape_of(&stroke(&[[0.1, 0.5], [0.8, 0.5]], 0)),
            Shape::Underline
        );
        assert_eq!(shape_of(&stroke(&[[0.2, 0.2], [0.5, 0.6]], 0)), Shape::Line);
        let circle: Vec<_> = (0..=32)
            .map(|i| {
                let a = 2.0 * PI * i as f32 / 32.0;
                [0.5 + 0.1 * a.cos(), 0.5 + 0.2 * a.sin()]
            })
            .collect();
        let mut s = stroke(&circle, 0);
        s.fh = 500;
        assert_eq!(shape_of(&s), Shape::Circle);
        s.pin = Some(1);
        assert_eq!(shape_of(&s), Shape::Pin);
    }

    #[test]
    fn arrow_points_toward_its_head_in_either_drawing_direction() {
        for shaft in [[[0.2, 0.8], [0.5, 0.5]], [[0.5, 0.5], [0.2, 0.8]]] {
            let marks = marks_of(&[
                stroke(&shaft, 0),
                stroke(&[[0.42, 0.5], [0.5, 0.5], [0.5, 0.58]], 200),
            ]);
            assert_eq!(marks.len(), 1);
            assert_eq!(marks[0].shape, Shape::Arrow);
            assert_eq!(marks[0].from, Some([0.2, 0.8]));
            assert_eq!(marks[0].to, Some([0.5, 0.5]));
            assert_eq!(
                describe_mark(&marks[0]),
                "drew an arrow from x 0.20, y 0.80 pointing to x 0.50, y 0.50"
            );
        }
    }

    /// Corners joined by points every 4 px (on the 1000 px test frame), as a mouse samples them.
    fn polyline(corners: &[[f32; 2]]) -> Vec<[f32; 2]> {
        let mut points: Vec<[f32; 2]> = corners
            .windows(2)
            .flat_map(|c| {
                let n = (distance(c[0], c[1]) / 0.004).max(1.0) as usize;
                (0..n).map(move |k| {
                    let f = k as f32 / n as f32;
                    [
                        c[0][0] + (c[1][0] - c[0][0]) * f,
                        c[0][1] + (c[1][1] - c[0][1]) * f,
                    ]
                })
            })
            .collect();
        points.extend(corners.last());
        points
    }

    fn ellipse(c: [f32; 2], r: [f32; 2], from: f32, to: f32) -> Vec<[f32; 2]> {
        (0..=60)
            .map(|k| {
                let a = from + (to - from) * k as f32 / 60.0;
                [c[0] + r[0] * a.cos(), c[1] + r[1] * a.sin()]
            })
            .collect()
    }

    /// An ellipse gone round `turns` times from its right-hand end, 120 points a turn.
    fn ellipse_turns(c: [f32; 2], r: [f32; 2], turns: f32) -> Vec<[f32; 2]> {
        let n = (120.0 * turns) as usize;
        (0..=n)
            .map(|k| {
                let a = 2.0 * PI * turns * k as f32 / n as f32;
                [c[0] + r[0] * a.cos(), c[1] + r[1] * a.sin()]
            })
            .collect()
    }

    /// The two ⇧ strokes of a real capture (`apparent-1792536608591-641c-2`), drawn with a mouse on
    /// a 1728×1117 screen: a flat loop round a table cell that ends just past where it began,
    /// and an arrow drawn in one go, shaft left to right, then a barb back above it, back to the
    /// tip and a barb below.
    fn real_stroke(pts: &[i32], t: u64) -> Stroke {
        Stroke {
            fw: 1728,
            fh: 1117,
            pts: pts.to_vec(),
            ..stroke(&[], t)
        }
    }

    const REAL_LOOP: [i32; 207] = [
        3617, 3170, 0, 3611, 3184, 48, 3595, 3194, 76, 3582, 3200, 96, 3560, 3210, 106, 3529, 3217,
        126, 3495, 3225, 146, 3449, 3229, 156, 3399, 3229, 175, 3363, 3229, 195, 3321, 3229, 215,
        3276, 3229, 225, 3249, 3229, 245, 3214, 3225, 263, 3192, 3218, 281, 3153, 3207, 290, 3136,
        3199, 310, 3093, 3172, 330, 3069, 3152, 350, 3051, 3132, 370, 3040, 3117, 388, 3033, 3103,
        397, 3026, 3084, 417, 3024, 3051, 437, 3024, 3030, 447, 3024, 3009, 457, 3024, 2971, 467,
        3024, 2928, 487, 3026, 2890, 505, 3041, 2841, 515, 3073, 2779, 544, 3139, 2696, 562, 3197,
        2640, 581, 3228, 2616, 591, 3330, 2563, 611, 3398, 2544, 631, 3477, 2533, 641, 3550, 2529,
        661, 3623, 2529, 681, 3702, 2539, 691, 3780, 2579, 710, 3852, 2634, 730, 3913, 2676, 747,
        3945, 2698, 757, 3983, 2729, 776, 4011, 2746, 785, 4035, 2767, 804, 4055, 2790, 823, 4078,
        2823, 834, 4088, 2848, 853, 4099, 2885, 873, 4103, 2918, 883, 4103, 2990, 920, 4081, 3027,
        938, 4045, 3067, 948, 4002, 3105, 968, 3921, 3159, 987, 3900, 3171, 997, 3849, 3192, 1017,
        3803, 3205, 1027, 3762, 3217, 1052, 3720, 3225, 1062, 3678, 3225, 1080, 3636, 3225, 1090,
        3594, 3225, 1110, 3566, 3225, 1128, 3535, 3225, 1148, 3516, 3222, 1158, 3502, 3218, 1178,
    ];

    const REAL_ARROW: [i32; 120] = [
        4057, 2996, 0, 4047, 2996, 68, 4075, 2996, 199, 4130, 3004, 209, 4236, 3044, 229, 4312,
        3073, 248, 4406, 3106, 256, 4490, 3121, 276, 4567, 3131, 296, 4606, 3131, 316, 4633, 3131,
        326, 4653, 3131, 365, 4621, 3111, 431, 4570, 3090, 451, 4507, 3062, 469, 4404, 3018, 488,
        4373, 3007, 506, 4345, 3000, 516, 4331, 3000, 534, 4322, 2997, 573, 4337, 2995, 624, 4376,
        2995, 644, 4436, 2999, 664, 4509, 3023, 674, 4549, 3039, 684, 4607, 3061, 694, 4638, 3076,
        713, 4652, 3083, 723, 4663, 3086, 733, 4674, 3092, 743, 4662, 3107, 830, 4648, 3115, 840,
        4609, 3147, 850, 4530, 3217, 880, 4512, 3233, 889, 4494, 3249, 899, 4483, 3259, 907, 4467,
        3275, 917, 4458, 3283, 934, 4481, 3284, 1308,
    ];

    /// Underlines from real captures (2560×1440) whose end hooks back like a lone barb.
    const REAL_HOOKED_UNDERLINES: [&[i32]; 2] = [
        &[
            3589, 6331, 0, 3612, 6331, 30, 3665, 6347, 49, 3718, 6356, 68, 3787, 6365, 78, 3817,
            6365, 86, 3856, 6365, 95, 3871, 6365, 105, 3963, 6365, 115, 4001, 6365, 134, 4062,
            6365, 143, 4139, 6365, 153, 4169, 6365, 172, 4214, 6365, 182, 4244, 6377, 199, 4258,
            6377, 218, 4272, 6381, 227, 4302, 6381, 247, 4355, 6390, 266, 4392, 6405, 276, 4407,
            6405, 286, 4422, 6411, 295, 4429, 6415, 305, 4439, 6422, 314, 4445, 6431, 334, 4450,
            6442, 353, 4453, 6451, 371, 4455, 6465, 401, 4450, 6476, 411, 4442, 6491, 430, 4436,
            6500, 449, 4426, 6510, 458, 4415, 6519, 477, 4408, 6528, 487, 4394, 6544, 497, 4372,
            6560, 506, 4357, 6567, 516, 4342, 6573, 524, 4305, 6573, 538, 4274, 6573, 548, 4267,
            6573, 558, 4244, 6565, 567, 4223, 6559, 576,
        ],
        &[
            7252, 494, 0, 7273, 484, 18, 7288, 477, 28, 7319, 468, 37, 7341, 468, 46, 7356, 461,
            56, 7371, 461, 64, 7409, 461, 81, 7477, 461, 91, 7508, 461, 99, 7530, 469, 109, 7545,
            476, 119, 7555, 476, 129, 7559, 485, 165, 7547, 504, 185, 7536, 524, 195, 7528, 532,
            224, 7514, 541, 234, 7460, 567, 254, 7407, 601, 264,
        ],
    ];

    #[test]
    fn real_hooked_underlines_are_not_arrows() {
        for pts in REAL_HOOKED_UNDERLINES {
            let stroke = Stroke {
                fw: 2560,
                fh: 1440,
                pts: pts.to_vec(),
                ..stroke(&[], 0)
            };
            assert_ne!(marks_of(&[stroke])[0].shape, Shape::Arrow);
        }
    }

    fn norm(x: i32, y: i32) -> [f32; 2] {
        [x as f32 / SCALE as f32, y as f32 / SCALE as f32]
    }

    #[test]
    fn real_flat_loop_is_a_circle_and_one_stroke_arrow_points_to_its_tip() {
        let marks = marks_of(&[
            real_stroke(&REAL_LOOP, 6208),
            real_stroke(&REAL_ARROW, 10283),
        ]);
        assert_eq!(marks.len(), 2);
        assert_eq!(marks[0].shape, Shape::Circle);
        assert_eq!(
            marks[0].description,
            "circled region x 0.30–0.41, y 0.25–0.32"
        );
        assert_eq!(marks[1].shape, Shape::Arrow);
        // From the shaft's start to where the head begins (its right-hand end), not to where the
        // pen lifted at the end of the lower barb.
        assert_eq!(marks[1].from, Some(norm(4057, 2996)));
        assert_eq!(marks[1].to, Some(norm(4653, 3131)));
        assert_eq!(
            marks[1].description,
            "drew an arrow from x 0.41, y 0.30 pointing to x 0.47, y 0.31"
        );
    }

    /// A circle that runs on into a tail, or past where it began, or starts with a lead-in, is
    /// still a circle, and what it circled is the loop alone.
    #[test]
    fn a_loop_with_a_tail_is_a_circle_round_the_loop() {
        let mut tailed = real_stroke(&REAL_LOOP, 0);
        tailed
            .pts
            .extend([3450, 3260, 0, 3380, 3330, 0, 3300, 3420, 0, 3220, 3500, 0]);
        let marks = marks_of(&[tailed]);
        assert_eq!(marks[0].shape, Shape::Circle);
        let ([x0, y0], [x1, y1]) = (norm(3024, 2529), norm(4103, 3229));
        assert_eq!(marks[0].box_norm, [x0, y0, x1, y1]);

        let flat = [0.2, 0.25, 0.4, 0.35];
        let mut tail = ellipse([0.3, 0.3], [0.1, 0.05], 0.0, 2.0 * PI);
        tail.extend(polyline(&[[0.4, 0.3], [0.52, 0.38]]));
        let overshoot = ellipse([0.3, 0.3], [0.1, 0.05], 0.0, 2.6 * PI);
        let mut lead_in = polyline(&[[0.18, 0.2], [0.3, 0.25]]);
        lead_in.extend(ellipse([0.3, 0.3], [0.1, 0.05], -PI / 2.0, 1.6 * PI));
        // A tail or lead-in short enough that the whole stroke still ends near where it began.
        let round = ellipse([0.3, 0.3], [0.1, 0.1], 0.0, 2.0 * PI);
        let mut short_tail = round.clone();
        short_tail.push([0.44, 0.32]);
        let short_lead_in = [vec![[0.44, 0.32]], round].concat();
        let disc = [0.2, 0.2, 0.4, 0.4];
        for (points, want) in [
            (tail, flat),
            (overshoot, flat),
            (lead_in, flat),
            (short_tail, disc),
            (short_lead_in, disc),
        ] {
            let mark = &marks_of(&[stroke(&points, 0)])[0];
            assert_eq!(mark.shape, Shape::Circle);
            for (got, want) in mark.box_norm.iter().zip(want) {
                assert!((got - want).abs() < 0.002, "{:?}", mark.box_norm);
            }
        }
    }

    /// Imported strokes have no point cap: a loop in a long one is still found, and a 120,000
    /// point scribble is measured quickly.
    #[test]
    fn a_long_imported_stroke_is_still_measured() {
        // 3,000 points about 2 px apart on a 10,000 px frame, then a tail.
        let mut points: Vec<_> = (0..3_000)
            .map(|k| {
                let a = 2.0 * PI * k as f32 / 2_500.0;
                [0.3 + 0.1 * a.cos(), 0.3 + 0.05 * a.sin()]
            })
            .collect();
        points.extend(polyline(&[[0.4, 0.3], [0.52, 0.38]]));
        let mut long = stroke(&points, 0);
        (long.fw, long.fh) = (10_000, 10_000);
        let mark = &marks_of(&[long])[0];
        assert_eq!(mark.shape, Shape::Circle);
        for (got, want) in mark.box_norm.iter().zip([0.2, 0.25, 0.4, 0.35]) {
            assert!((got - want).abs() < 0.002, "{:?}", mark.box_norm);
        }

        let scribble: Vec<_> = (0..120_000)
            .map(|i| {
                let i = i as f32;
                [0.3 + 0.1 * (0.9 * i).sin(), 0.3 + 0.022 * (0.37 * i).sin()]
            })
            .collect();
        let started = std::time::Instant::now();
        let mark = &marks_of(&[stroke(&scribble, 0)])[0];
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
        assert_eq!(mark.shape, Shape::Mark);
    }

    /// Recognising flatter loops never costs a circle the old rule found: round or near-closed
    /// loops, ones gone round several times, gaps along either axis.
    #[test]
    fn every_loop_the_old_rule_called_a_circle_still_is() {
        let cases = [
            ellipse([0.3, 0.3], [0.1, 0.1], 0.0, 2.0 * PI),
            ellipse([0.3, 0.3], [0.1, 0.05], 0.0, 2.0 * PI),
            // Open by about 30° at one end of its long axis.
            ellipse(
                [0.3, 0.3],
                [0.115, 0.05],
                PI / 2.0,
                PI / 2.0 + 2.0 * PI - 0.55,
            ),
            ellipse([0.3, 0.3], [0.1, 0.1], 0.3, 2.0 * PI - 0.3),
            ellipse([0.3, 0.3], [0.05, 0.1], 0.2, 2.0 * PI - 0.2),
            ellipse_turns([0.3, 0.3], [0.1, 0.08], 2.0),
            ellipse_turns([0.3, 0.3], [0.1, 0.1], 3.0),
            ellipse_turns([0.3, 0.3], [0.1, 0.1], 5.0),
        ];
        for (i, points) in cases.iter().enumerate() {
            assert_eq!(shape_of(&stroke(points, 0)), Shape::Circle, "case {i}");
        }
    }

    /// A flat loop counts as a circle up to three times round (with drawing jitter), not four.
    #[test]
    fn flat_loops_count_up_to_three_turns() {
        for turns in [1.0, 3.0] {
            let points = ellipse_turns([0.3, 0.3], [0.12, 0.03], turns);
            assert_eq!(shape_of(&stroke(&points, 0)), Shape::Circle, "{turns}");
        }
        let points = ellipse_turns([0.3, 0.3], [0.12, 0.03], 4.0);
        assert_eq!(shape_of(&stroke(&points, 0)), Shape::Mark);
    }

    /// The loop search tries every few points of a long stroke as a loop's ends, but judges each
    /// stretch by the whole recorded path, so sampling can't smooth away a zigzag or fold many
    /// turns into one.
    #[test]
    fn sampling_a_long_stroke_hides_neither_zigzags_nor_turns() {
        // 1,201 points: every third point is a smooth flat ellipse, the others zigzag across it.
        let mut zigzag = Vec::new();
        for i in 0..400 {
            let a = 2.0 * PI * i as f32 / 400.0;
            let p = [0.3 + 0.12 * a.cos(), 0.3 + 0.03 * a.sin()];
            zigzag.extend([
                p,
                [p[0] + 0.004, p[1] + 0.008],
                [p[0] - 0.004, p[1] - 0.008],
            ]);
        }
        zigzag.push(zigzag[0]);
        // 501 turns in 12,000 steps: every 24th point is a single turn.
        let turns: Vec<_> = (0..=12_000)
            .map(|k| {
                let a = 1002.0 * PI * k as f32 / 12_000.0;
                [0.3 + 0.12 * a.cos(), 0.3 + 0.03 * a.sin()]
            })
            .collect();
        for (name, points) in [("zigzag", zigzag), ("501 turns", turns)] {
            assert_eq!(shape_of(&stroke(&points, 0)), Shape::Mark, "{name}");
        }
    }

    #[test]
    fn one_stroke_arrows_with_one_or_two_barbs() {
        for head in [
            vec![[0.262, 0.236], [0.272, 0.275]],
            vec![[0.26, 0.23], [0.3, 0.25], [0.27, 0.27]],
        ] {
            let mut corners = vec![[0.1, 0.3], [0.3, 0.25]];
            corners.extend(head);
            let mark = &marks_of(&[stroke(&polyline(&corners), 0)])[0];
            assert_eq!(mark.shape, Shape::Arrow, "{corners:?}");
            assert_eq!(mark.from, Some([0.1, 0.3]));
            let to = mark.to.unwrap();
            assert!((to[0] - 0.3).abs() < 0.001 && (to[1] - 0.25).abs() < 0.001);
        }
    }

    #[test]
    fn ticks_zigzags_scribbles_hooks_and_arcs_are_not_arrows_or_circles() {
        // Back and forth over a line of text, drifting up and down: briefly, and for ten seconds
        // (which closes on itself and fills its box, so only its length tells it from a circle).
        let scribble = |n: usize| -> Vec<_> {
            (0..n)
                .map(|i| {
                    let i = i as f32;
                    [0.2 + 0.1 * (0.9 * i).sin(), 0.2 + 0.022 * (0.37 * i).sin()]
                })
                .collect()
        };
        let cases = [
            (
                "check mark",
                polyline(&[[0.1, 0.2], [0.13, 0.24], [0.22, 0.12]]),
            ),
            ("V", polyline(&[[0.1, 0.1], [0.15, 0.25], [0.2, 0.1]])),
            (
                "narrow V",
                polyline(&[[0.1, 0.1], [0.13, 0.25], [0.16, 0.1]]),
            ),
            (
                "zigzag",
                polyline(&[
                    [0.1, 0.2],
                    [0.13, 0.16],
                    [0.16, 0.2],
                    [0.19, 0.16],
                    [0.22, 0.2],
                ]),
            ),
            (
                "back and forth",
                polyline(&[
                    [0.1, 0.2],
                    [0.3, 0.21],
                    [0.11, 0.225],
                    [0.3, 0.24],
                    [0.11, 0.255],
                ]),
            ),
            ("scribble", scribble(60)),
            ("long scribble", scribble(600)),
            (
                "hooked underline",
                polyline(&[[0.1, 0.3], [0.4, 0.3], [0.405, 0.27]]),
            ),
            (
                "curled underline",
                polyline(&[
                    [0.1, 0.3],
                    [0.4, 0.3],
                    [0.415, 0.29],
                    [0.415, 0.275],
                    [0.4, 0.265],
                ]),
            ),
            (
                "one-sided barb",
                polyline(&[[0.1, 0.3], [0.3, 0.25], [0.26, 0.24]]),
            ),
            ("line", polyline(&[[0.1, 0.3], [0.4, 0.3]])),
            ("C", ellipse([0.3, 0.3], [0.1, 0.1], PI / 4.0, 1.75 * PI)),
            (
                "flat C",
                ellipse([0.3, 0.3], [0.1, 0.025], 0.8, 2.0 * PI - 0.8),
            ),
        ];
        for (name, points) in cases {
            let shape = shape_of(&stroke(&points, 0));
            assert!(
                !matches!(shape, Shape::Arrow | Shape::Circle),
                "{name} became {shape:?}"
            );
        }
    }

    #[test]
    fn pins_lasers_and_group_limits() {
        let mut strokes: Vec<_> = (0..8).map(|i| stroke(&[[0.5, 0.5]], i * 200)).collect();
        strokes[1].laser = true;
        let mut pin = stroke(&[[0.25, 0.4]], 1600);
        pin.pin = Some(1);
        strokes.push(pin);
        strokes.push(stroke(&[[0.5, 0.5]], 1700));
        strokes.push(stroke(&[[0.5, 0.5]], 4000));
        let marks = marks_of(&strokes);
        assert_eq!(
            marks.iter().map(|m| m.strokes.len()).collect::<Vec<_>>(),
            vec![6, 1, 1, 1, 1]
        );
        assert_eq!(marks[0].strokes, vec![0, 2, 3, 4, 5, 6]);
        assert_eq!(marks[2].shape, Shape::Pin);
        assert_eq!(marks[2].id, "m3");
        assert_eq!(describe_mark(&marks[2]), "pin 1 at x 0.25, y 0.40");
    }

    #[test]
    fn descriptions_include_group_count_and_label() {
        let mut strokes = vec![
            stroke(&[[0.1, 0.5], [0.8, 0.5]], 0),
            stroke(&[[0.2, 0.5], [0.7, 0.5]], 200),
        ];
        strokes[1].on = Some("Header".into());
        assert_eq!(
            marks_of(&strokes)[0].description,
            "underlined x 0.10–0.80 at y 0.50 (2 strokes) over “Header”"
        );
    }

    #[test]
    fn debounce_accumulates_scrolls_across_other_events() {
        let scroll = |t, dy| InteractionEvent::Scroll {
            t,
            x: 0.5,
            y: 0.5,
            dx: 0.0,
            dy,
            label: Some("Transcript".into()),
        };
        let point = |t| InteractionEvent::Point {
            t,
            x: 0.3,
            y: 0.12,
            label: Some("Header".into()),
        };
        let events = settled_events(&[
            scroll(0, 10.0),
            point(100),
            scroll(1000, 20.0),
            point(800),
            scroll(2600, -5.0),
        ]);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].timestamp_ms(), 800);
        assert!(matches!(
            events[1],
            InteractionEvent::Scroll {
                t: 1000,
                dy: 30.0,
                ..
            }
        ));
        assert_eq!(describe_event(&events[1]), "scrolled “Transcript” down");
        assert_eq!(
            describe_event(&events[0]),
            "pointed at “Header” (x 0.30, y 0.12)"
        );

        let click = |t, x, y| InteractionEvent::Click {
            t,
            x,
            y,
            button: Some("left".into()),
            label: None,
            role: None,
        };
        let coalesced = settled_events(&[
            click(1000, 0.42, 0.18),
            click(1220, 0.425, 0.182),
            click(1450, 0.421, 0.181),
            click(2500, 0.80, 0.50),
        ]);
        assert_eq!(coalesced.len(), 2);
        assert_eq!(coalesced[0].timestamp_ms(), 1450);
        assert_eq!(describe_event(&coalesced[0]), "clicked (x 0.42, y 0.18)");
    }

    #[test]
    fn outline_is_closed_finite_and_has_caps_and_taper() {
        let options = FreehandOptions {
            size: 10.0,
            thinning: 0.0,
            streamline: 0.0,
            ..Default::default()
        };
        assert!(freehand_outline(&[], &options).is_empty());
        for points in [
            vec![[0.0, 0.0]],
            vec![[0.0, 0.0]; 4],
            vec![[0.0, 0.0], [100.0, 0.0]],
            vec![[0.0, 0.0], [50.0, 0.0], [1.0, 1.0]],
        ] {
            let outline = freehand_outline(&points, &options);
            assert!(outline.len() > 10);
            assert_eq!(outline.first(), outline.last());
            assert!(outline.iter().flatten().all(|n| n.is_finite()));
        }
        let line = [[0.0, 0.0], [100.0, 0.0]];
        let plain = freehand_outline(&line, &options);
        let b = bounds(&plain);
        assert!(b[0] < -4.0 && b[2] > 104.0 && b[1] <= -4.9 && b[3] >= 4.9);
        let tapered = freehand_outline(
            &line,
            &FreehandOptions {
                taper_start: true,
                taper_end: true,
                ..options
            },
        );
        assert!(tapered[0][1].abs() < 0.1);
        assert!(tapered.contains(&[100.0, 0.0]));
    }

    #[test]
    fn smooth_laser_and_ink_handle_stationary_duplicate_points() {
        let stationary_laser = [([120.0, 240.0], 0.8), ([120.0, 240.0], 1.0)];
        let smoothed_laser = smooth_laser_points(&stationary_laser);
        assert_eq!(smoothed_laser.len(), 1);

        let stationary_ink = [[120.0, 240.0], [120.0, 240.0], [120.0, 240.0]];
        let smoothed_ink = smooth_ink_points(&stationary_ink);
        assert_eq!(smoothed_ink.len(), 1);
    }

    #[test]
    fn summary_interleaves_speech_marks_and_events() {
        let strokes = vec![stroke(&[[0.5, 0.5]], 5000)];
        let mut marks = marks_of(&strokes);
        marks[0].frame = Some("frames/frame-01.png".into());
        let bundle = ApparentBundle {
            version: 1,
            id: "test".into(),
            created_at_ms: 0,
            duration_ms: 14000,
            target: TargetInfo {
                mode: CaptureMode::Window,
                app: Some("App".into()),
                bundle_id: None,
                window_title: Some("Window".into()),
                window_id: None,
                display_id: 1,
                frame: TargetFrame {
                    x: 0.0,
                    y: 0.0,
                    width: 1000.0,
                    height: 1000.0,
                    scale: 1.0,
                },
            },
            transcript: TranscriptData {
                text: "This row is too dense.".into(),
                segments: vec![TranscriptSegment {
                    t0: 3000,
                    t1: 4500,
                    text: "This row is too dense.".into(),
                }],
                ..Default::default()
            },
            note: Some("Simplify it.".into()),
            strokes,
            marks,
            events: vec![InteractionEvent::Key {
                t: 4000,
                shortcut: "⌘K".into(),
            }],
            interaction_telemetry: None,
            keyframes: vec![Keyframe {
                index: 1,
                t: 5100,
                reason: KeyframeReason::Mark,
                file: "frames/frame-01.png".into(),
                event: "tapped x 0.50, y 0.50".into(),
                said: Some("This row is too dense.".into()),
                mark_id: Some("m1".into()),
                cursor: None,
            }],
            contact_sheet: "contact.png".into(),
            video: None,
            video_offset_ms: 0,
            audio: None,
            source: None,
            device: None,
            context: Default::default(),
            context_changes: Vec::new(),
            screens: Vec::new(),
            windows: Vec::new(),
            cursor: Vec::new(),
            autocrop: None,
            destination: None,
            image_quality_warning: None,
        };
        let tl = timeline(&bundle).join("\n");
        assert!(tl.find("0:03 “").unwrap() < tl.find("0:04 · pressed").unwrap());
        assert!(tl.find("0:04 · pressed").unwrap() < tl.find("0:05 ✎").unwrap());
        assert!(tl.contains("— frame #1"));

        let summary = summary_markdown_with_visual_policy(
            &bundle,
            std::path::Path::new("/recordings/test"),
            false,
        );
        assert!(summary.starts_with("[apparent 0:14 · App “Window” · /recordings/test]"));
        assert!(summary.contains("Note: Simplify it."));
        assert!(!summary.contains(LOOK_AT_IMAGES));
        assert!(summary.contains("apparent zoom"));
        assert!(summary.contains("- #1 (0:05) “This row is too dense.”"));
        assert!(!summary.contains("No transcript"));

        // Typed feedback belongs between the surrounding spoken keyframes, once,
        // retaining its exact text and the original pin/frame reference.
        let mut mixed = bundle.clone();
        mixed.events.push(InteractionEvent::Note {
            t: 6000,
            pin: 1,
            x: 0.5,
            y: 0.5,
            text: "Exact typed feedback\nSecond line".into(),
        });
        let mut later = mixed.keyframes[0].clone();
        later.index = 2;
        later.t = 9000;
        later.said = Some("Later spoken feedback".into());
        later.file = "frames/frame-02.png".into();
        later.mark_id = None;
        later.reason = KeyframeReason::SceneChange;
        mixed.keyframes.push(later);
        for context in [false, true] {
            let summary = summary_markdown_with(&mixed, std::path::Path::new("/tmp"), context);
            assert_eq!(summary.matches("Exact typed feedback").count(), 1);
            assert!(
                summary.find("This row").unwrap() < summary.find("Exact typed feedback").unwrap()
            );
            assert!(
                summary.find("Exact typed feedback").unwrap()
                    < summary.find("Later spoken feedback").unwrap()
            );
            assert!(summary.contains("Second line"));
            assert!(summary.contains("0:06 Typed note · pin 1"));
        }

        mixed.keyframes[0].reason = KeyframeReason::Whiteboard;
        let whiteboard = summary_markdown_with(&mixed, std::path::Path::new("/tmp"), true);
        assert_eq!(whiteboard.matches("Exact typed feedback").count(), 1);

        mixed.keyframes.clear();
        for context in [false, true] {
            let summary = summary_markdown_with(&mixed, std::path::Path::new("/tmp"), context);
            assert_eq!(summary.matches("Exact typed feedback").count(), 1);
            assert!(
                summary.find("This row").unwrap() < summary.find("Exact typed feedback").unwrap()
            );
        }

        // Audio but no speech model yet: the summary says so instead of staying quiet.
        let audio_dir = std::env::temp_dir().join(format!(
            "apparent-summary-audio-{}-{}",
            std::process::id(),
            crate::shared::clock::now_epoch_ms()
        ));
        std::fs::create_dir_all(&audio_dir).unwrap();
        std::fs::write(
            audio_dir.join("audio.wav"),
            crate::capture::engine::runtime::audio::audio::encode_wav_16k(&[0.0; 1280]).unwrap(),
        )
        .unwrap();
        let mut silent = bundle.clone();
        silent.audio = Some("audio.wav".into());
        silent.transcript = TranscriptData {
            source: "offline".into(),
            missing: Some(
                crate::capture::engine::runtime::audio::transcription::speech_model::missing(
                    "downloading",
                    "en_US",
                    None,
                ),
            ),
            ..Default::default()
        };
        let summary = summary_markdown(&silent, &audio_dir);
        assert!(summary.contains("No transcript: "));
        assert!(summary.contains("(en_US) was still downloading. The audio is saved"));

        let wav = std::fs::read(audio_dir.join("audio.wav")).unwrap();
        std::fs::remove_file(audio_dir.join("audio.wav")).unwrap();
        std::fs::write(audio_dir.join(".evidence-only.json"), b"{}").unwrap();
        assert!(!summary_markdown(&silent, &audio_dir).contains("No microphone audio was saved"));
        std::fs::remove_file(audio_dir.join(".evidence-only.json")).unwrap();
        assert!(summary_markdown(&silent, &audio_dir).contains("No microphone audio was saved"));
        std::fs::write(audio_dir.join("audio.wav"), wav).unwrap();

        // The handoff itself must distinguish absent audio from saved audio with no words.
        let mut no_audio = silent.clone();
        no_audio.audio = None;
        no_audio.transcript = TranscriptData {
            source: "none".into(),
            ..Default::default()
        };
        let summary = summary_markdown(&no_audio, &audio_dir);
        assert!(summary.contains("No microphone audio was saved"));
        assert!(!summary.contains("No words transcribed"));

        let mut missing_and_no_audio = silent.clone();
        missing_and_no_audio.audio = None;
        let summary = summary_markdown(&missing_and_no_audio, &audio_dir);
        assert!(summary.contains("No microphone audio was saved"));
        assert!(!summary.contains("The audio is saved"));

        let mut missing_file = silent.clone();
        missing_file.audio = Some("removed.wav".into());
        let summary = summary_markdown(&missing_file, &audio_dir);
        assert!(summary.contains("No microphone audio was saved"));
        assert!(!summary.contains("The audio is saved"));

        let mut recognized_but_file_removed = missing_file.clone();
        recognized_but_file_removed.transcript = TranscriptData {
            source: "command:test".into(),
            text: "heard before the file was removed".into(),
            ..Default::default()
        };
        let summary = summary_markdown(&recognized_but_file_removed, &audio_dir);
        assert!(summary.contains("No microphone audio was saved"));
        assert!(!summary.contains("No words transcribed"));

        let mut skipped = silent.clone();
        skipped.transcript = TranscriptData {
            source: "none".into(),
            ..Default::default()
        };
        let summary = summary_markdown(&skipped, &audio_dir);
        assert!(summary.contains("Recognition did not run"));
        assert!(!summary.contains("No words transcribed"));

        let empty_wav = crate::capture::engine::runtime::audio::audio::encode_wav_16k(&[]).unwrap();
        std::fs::write(audio_dir.join("audio.wav"), &empty_wav).unwrap();
        let stopped_without_samples =
            crate::capture::engine::runtime::audio::audio::RecordedAudio {
                sample_count: 0,
                duration_ms: 0,
                wav_bytes: empty_wav,
                saved_lease: None,
                live: Default::default(),
                #[cfg(target_os = "linux")]
                pause: Default::default(),
            };
        skipped.transcript =
            crate::capture::engine::runtime::audio::transcription::transcribe::transcribe(
                Some(&audio_dir.join("audio.wav")),
                Some(&stopped_without_samples),
            );
        assert_eq!(skipped.transcript.source, "none");
        let summary = summary_markdown(&skipped, &audio_dir);
        assert!(summary.contains("No microphone audio was saved"));
        assert!(!summary.contains("No words transcribed"));
        let truncated_wav =
            crate::capture::engine::runtime::audio::audio::encode_wav_16k(&[0.0; 1280]).unwrap();
        std::fs::write(audio_dir.join("audio.wav"), &truncated_wav[..44]).unwrap();
        let summary = summary_markdown(&skipped, &audio_dir);
        assert!(summary.contains("No microphone audio was saved"));
        assert!(!summary.contains("No words transcribed"));
        std::fs::write(
            audio_dir.join("audio.wav"),
            crate::capture::engine::runtime::audio::audio::encode_wav_16k(&[0.0; 1280]).unwrap(),
        )
        .unwrap();

        let mut no_words = silent.clone();
        no_words.audio = Some("audio.wav".into());
        no_words.transcript = TranscriptData {
            source: "command:test".into(),
            ..Default::default()
        };
        let summary = summary_markdown(&no_words, &audio_dir);
        assert!(summary.contains("No words transcribed"));
        assert!(!summary.contains("No microphone audio was saved"));

        let mut failure = silent.clone();
        failure.audio = Some("audio.wav".into());
        let summary = summary_markdown(&failure, &audio_dir);
        assert!(summary.contains("No transcript: "));
        assert!(!summary.contains("No words transcribed"));
        std::fs::remove_dir_all(&audio_dir).unwrap();

        // A phone bundle: device header, a flagged keyframe, and app events under the keyframe
        // that was on screen when they happened.
        let mut phone = bundle;
        phone.note = None;
        phone.device = Some(DeviceInfo {
            name: "iPhone".into(),
            capture: "apparentkit".into(),
            app: Some("dev.example.App".into()),
            app_version: Some("42".into()),
            ..Default::default()
        });
        phone.source = Some("iphone".into());
        let mut phone_without_words = phone.clone();
        phone_without_words.transcript = TranscriptData::default();
        let phone_summary = summary_markdown(
            &phone_without_words,
            std::path::Path::new("/recordings/phone"),
        );
        assert!(!phone_summary.contains("No microphone audio was saved"));
        assert!(!phone_summary.contains("No words transcribed"));
        phone.marks.push(Mark {
            id: "f1".into(),
            strokes: Vec::new(),
            t0: 9000,
            t1: 9000,
            box_norm: [0.0, 0.0, 1.0, 1.0],
            shape: Shape::Flag,
            from: None,
            to: None,
            pin: None,
            on: None,
            description: "flagged this moment".into(),
            frame: None,
        });
        phone.keyframes.push(Keyframe {
            index: 2,
            t: 9000,
            reason: KeyframeReason::Mark,
            file: "frames/frame-02.png".into(),
            event: "flagged this moment".into(),
            said: None,
            mark_id: Some("f1".into()),
            cursor: None,
        });
        phone.events = vec![
            InteractionEvent::AppEvent {
                t: 1000,
                label: "launched".into(),
            },
            InteractionEvent::AppEvent {
                t: 6000,
                label: "rest started".into(),
            },
            InteractionEvent::AppEvent {
                t: 9500,
                label: "Done tapped".into(),
            },
        ];
        let summary = summary_markdown_with_visual_policy(
            &phone,
            std::path::Path::new("/recordings/phone"),
            false,
        );
        assert_eq!(
            summary,
            format!(
                "[apparent 0:14 · iPhone · dev.example.App (42) · /recordings/phone]\n\
                 {OPTIONAL_IMAGES}\n\
                 \x20 · 0:01 app: launched\n\
                 - #1 (0:05) “This row is too dense.”\n\
                 \x20 · 0:06 app: rest started\n\
                 - #2 (0:09) ⚑ flagged this moment\n\
                 \x20 · 0:09 app: Done tapped\n"
            )
        );
        assert!(
            timeline(&phone)
                .iter()
                .any(|l| l == "0:06 · app: rest started")
        );

        let long_press = InteractionEvent::Drag {
            t: 1_000,
            t1: 1_800,
            x0: 0.42,
            y0: 0.68,
            x1: 0.42,
            y1: 0.68,
            label: Some("long press".into()),
            button: Some("touch".into()),
            path: vec![[0.42, 0.68], [0.42, 0.68]],
        };
        assert_eq!(describe_event(&long_press), "long-pressed (x 0.42, y 0.68)");

        phone.keyframes.push(Keyframe {
            index: 3,
            t: 11_000,
            reason: KeyframeReason::Click,
            file: "frames/frame-03.png".into(),
            event: "swiped up (x 0.50, y 0.80 → x 0.50, y 0.20)".into(),
            said: Some("And then I scroll up.".into()),
            mark_id: None,
            cursor: Some([0.50, 0.20]),
        });
        let summary_with_action =
            summary_markdown(&phone, std::path::Path::new("/recordings/phone"));
        assert!(summary_with_action.contains("- #3 (0:11) [swiped up] “And then I scroll up.”"));
    }

    #[test]
    fn summary_shows_app_version_context_and_screen_changes() {
        let mut bundle: ApparentBundle = serde_json::from_value(serde_json::json!({
            "version": 1, "id": "t", "createdAtMs": 0, "durationMs": 25_000,
            "target": {"mode": "device", "app": "OpenCode", "bundleId": "ai.opencode.mobilecode",
                "displayId": 0,
                "frame": {"x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0, "scale": 1.0}},
            "transcript": {"source": "test", "text": "", "segments": []},
            "strokes": [], "marks": [], "events": [], "keyframes": [], "contactSheet": "c.png",
            "source": "iphone",
            "device": {"name": "iPhone", "capture": "apparentkit", "width": 10, "height": 20,
                "app": "OpenCode", "appVersion": "354", "appShortVersion": "1.4"}
        }))
        .unwrap();
        let keyframe = |index: usize, t: u64, reason, event: &str, said: Option<&str>| Keyframe {
            index,
            t,
            reason,
            file: format!("frames/frame-{index:02}.png"),
            event: event.into(),
            said: said.map(str::to_owned),
            mark_id: None,
            cursor: None,
        };
        bundle.keyframes = vec![
            keyframe(
                1,
                1_000,
                KeyframeReason::Initial,
                "first frame",
                Some("So here."),
            ),
            keyframe(
                2,
                3_000,
                KeyframeReason::Click,
                "tapped “Settings” (x 0.10, y 0.90)",
                None,
            ),
            keyframe(
                3,
                8_000,
                KeyframeReason::Click,
                "tapped “Back” (x 0.05, y 0.06)",
                Some("Go back."),
            ),
            keyframe(
                4,
                20_000,
                KeyframeReason::SceneChange,
                "screen changed",
                Some("Done."),
            ),
        ];
        bundle.events.push(InteractionEvent::Click {
            t: 3_000,
            x: 0.10,
            y: 0.90,
            button: Some("touch".into()),
            label: Some("Settings".into()),
            role: None,
        });
        bundle.screens = vec![
            ScreenSpan {
                t0: 0,
                t1: 5_000,
                name: "Sessions".into(),
            },
            ScreenSpan {
                t0: 5_000,
                t1: 25_000,
                name: "Settings › Experimental".into(),
            },
        ];
        bundle.context = std::collections::BTreeMap::from([
            ("server".to_owned(), "studio".to_owned()),
            ("session".to_owned(), format!("ses_{}", "x".repeat(80))),
        ]);
        bundle.context_changes = vec![ContextChange {
            t: 9_000,
            key: "server".into(),
            value: Some("mini".into()),
        }];
        let summary = summary_markdown(&bundle, std::path::Path::new("/recordings/phone"));
        let session = format!("ses_{}…", "x".repeat(55));
        assert_eq!(
            summary,
            format!(
                "[apparent 0:25 · iPhone · OpenCode 1.4 (354) · /recordings/phone]\n\
                 {LOOK_AT_IMAGES}\n\
                 Screen: Sessions · server: studio · session: {session}\n\
                 - #1 (0:01) “So here.”\n\
                 - #2 (0:03) tapped “Settings” (x 0.10, y 0.90)\n\
                 - #3 (0:08) ⟨Settings › Experimental⟩ [tapped “Back”] “Go back.”\n\
                 \x20 · 0:09 server → mini\n\
                 - #4 (0:20) “Done.”\n"
            )
        );

        // No screens or values: header only; a build without a marketing version stays as before.
        bundle.screens.clear();
        bundle.context.clear();
        bundle.context_changes.clear();
        bundle.device.as_mut().unwrap().app_short_version = None;
        let summary = summary_markdown(&bundle, std::path::Path::new("/recordings/phone"));
        assert!(summary.starts_with(&format!(
            "[apparent 0:25 · iPhone · OpenCode (354) · /recordings/phone]\n{LOOK_AT_IMAGES}\n- #1 (0:01) “So here.”\n"
        )));
    }
}
