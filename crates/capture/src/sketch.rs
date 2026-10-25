use crate::model::*;
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

fn distance(a: [f32; 2], b: [f32; 2]) -> f32 {
    (a[0] - b[0]).hypot(a[1] - b[1])
}

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

fn at([x, y]: [f32; 2]) -> String {
    format!("x {x:.2}, y {y:.2}")
}

pub fn describe_mark(mark: &Mark) -> String {
    let [x0, y0, x1, y1] = mark.box_norm;
    let region = format!("x {x0:.2}–{x1:.2}, y {y0:.2}–{y1:.2}");
    let many = if mark.strokes.len() > 1 {
        format!(" ({} strokes)", mark.strokes.len())
    } else {
        String::new()
    };
    let mut text = match mark.shape {
        Shape::Circle => format!("circled region {region}{many}"),
        Shape::Underline => format!(
            "underlined x {x0:.2}–{x1:.2} at y {:.2}{many}",
            (y0 + y1) / 2.0
        ),
        Shape::Line => format!(
            "drew a line from {} to {}",
            at(mark.from.unwrap_or([x0, y0])),
            at(mark.to.unwrap_or([x1, y1]))
        ),
        Shape::Arrow => format!(
            "drew an arrow from {} pointing to {}",
            at(mark.from.unwrap_or([x0, y0])),
            at(mark.to.unwrap_or([x1, y1]))
        ),
        Shape::Pin => format!("pin {} at {}", mark.pin.unwrap_or(1), at([x0, y0])),
        Shape::Tap => format!("tapped {}{many}", at([(x0 + x1) / 2.0, (y0 + y1) / 2.0])),
        Shape::Mark => format!("marked region {region}{many}"),
        Shape::Flag => "flagged this moment".into(),
    };
    if let Some(on) = &mark.on {
        text.push_str(&format!(" over {}", quote_label(on)));
    }
    text
}

/// A label in curly quotes, unless it already quotes its own parts (`button “Area” #area` from
/// `crate::ax`, `<div.capsule> in Pill (src/Pill.tsx:42)` from `crate::page`).
pub(crate) fn quote_label(label: &str) -> String {
    if label.contains('“') || label.starts_with('<') {
        label.to_owned()
    } else {
        format!("“{label}”")
    }
}

/// Whether `mark` belongs to `keyframe` (by `mark_id` or shared `frame` file).
pub fn mark_on_keyframe(mark: &Mark, keyframe: &Keyframe) -> bool {
    keyframe.mark_id.as_ref() == Some(&mark.id)
        || (!keyframe.file.is_empty() && mark.frame.as_ref() == Some(&keyframe.file))
}

/// The inked marks a keyframe draws (every mark of its screen's group), in drawing order.
pub fn marks_on<'a>(keyframe: &Keyframe, marks: &'a [Mark]) -> Vec<&'a Mark> {
    let mut shown: Vec<&Mark> = marks
        .iter()
        .filter(|m| !m.strokes.is_empty() && mark_on_keyframe(m, keyframe))
        .collect();
    shown.sort_by_key(|m| m.t0);
    shown
}

/// Coalesce each label independently, even when other events occur during a burst.
pub fn settled_events(events: &[InteractionEvent]) -> Vec<InteractionEvent> {
    let mut sorted = events.to_vec();
    sorted.sort_by_key(InteractionEvent::timestamp_ms);
    let mut out: Vec<InteractionEvent> = Vec::new();
    for mut event in sorted {
        let prior = out.iter().rposition(|prev| match (prev, &event) {
            (
                InteractionEvent::Click {
                    t: a,
                    x: x0,
                    y: y0,
                    button: b0,
                    label: l0,
                    ..
                },
                InteractionEvent::Click {
                    t: b,
                    x: x1,
                    y: y1,
                    button: b1,
                    label: l1,
                    ..
                },
            ) => {
                b0 == b1
                    && l0 == l1
                    && b.saturating_sub(*a) <= 650
                    && (x1 - x0).hypot(y1 - y0) <= 0.03
            }
            (
                InteractionEvent::Scroll { t: a, label: x, .. },
                InteractionEvent::Scroll { t: b, label: y, .. },
            ) => x == y && b.saturating_sub(*a) < 1500,
            (
                InteractionEvent::Point { t: a, label: x, .. },
                InteractionEvent::Point { t: b, label: y, .. },
            ) => x == y && b.saturating_sub(*a) < 1000,
            _ => false,
        });
        if let Some(i) = prior {
            if let (
                InteractionEvent::Scroll { dx, dy, .. },
                InteractionEvent::Scroll { dx: px, dy: py, .. },
            ) = (&mut event, &out[i])
            {
                *dx += px;
                *dy += py;
            }
            out.remove(i);
        }
        out.push(event);
    }
    out
}

pub(crate) fn direction(dx: f32, dy: f32, threshold: f32) -> String {
    let h = if dx.abs() > threshold {
        if dx > 0.0 { "right" } else { "left" }
    } else {
        ""
    };
    let v = if dy.abs() > threshold {
        if dy > 0.0 { "down" } else { "up" }
    } else {
        ""
    };
    let parts = if dx.abs() > 2.0 * dy.abs() {
        vec![h]
    } else if dy.abs() > 2.0 * dx.abs() {
        vec![v]
    } else {
        vec![v, h]
    };
    let text = parts
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" and ");
    if text.is_empty() {
        "a little".into()
    } else {
        text
    }
}

pub fn describe_event(event: &InteractionEvent) -> String {
    match event {
        InteractionEvent::Click {
            x,
            y,
            label,
            button,
            role,
            ..
        } => {
            let verb = if button.as_deref() == Some("touch") {
                "tapped"
            } else {
                "clicked"
            };
            let role = role
                .as_deref()
                .filter(|r| !r.is_empty())
                .map(|r| format!(" {r}"))
                .unwrap_or_default();
            match label.as_deref() {
                Some(l) if !l.is_empty() => {
                    format!("{verb}{role} {} ({})", quote_label(l), at([*x, *y]))
                }
                _ => format!("{verb}{role} ({})", at([*x, *y])),
            }
        }
        InteractionEvent::Drag {
            x0,
            y0,
            x1,
            y1,
            label,
            button,
            ..
        } => {
            if label.as_deref() == Some("long press") {
                format!("long-pressed ({})", at([*x0, *y0]))
            } else if let Some(target) = label
                .as_deref()
                .and_then(|l| l.strip_prefix("long press:"))
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                format!("long-pressed “{target}” ({})", at([*x0, *y0]))
            } else {
                format!(
                    "{}{} {} ({} → {})",
                    if button.as_deref() == Some("touch") {
                        "swiped"
                    } else {
                        "dragged"
                    },
                    label
                        .as_ref()
                        .filter(|s| !s.is_empty())
                        .map(|s| format!(" {}", quote_label(s)))
                        .unwrap_or_default(),
                    direction(x1 - x0, y1 - y0, 0.04),
                    at([*x0, *y0]),
                    at([*x1, *y1])
                )
            }
        }
        InteractionEvent::Scroll { dx, dy, label, .. } => match label.as_deref() {
            Some(l) if !l.is_empty() => format!("scrolled “{l}” {}", direction(*dx, *dy, 0.0)),
            _ => format!("scrolled {}", direction(*dx, *dy, 0.0)),
        },
        InteractionEvent::Point { x, y, label, .. } => match label.as_deref() {
            Some(l) if !l.is_empty() => format!("pointed at “{l}” ({})", at([*x, *y])),
            _ => format!("pointed at ({})", at([*x, *y])),
        },
        InteractionEvent::Key { shortcut, .. } => format!("pressed {shortcut}"),
        InteractionEvent::AppEvent { label, .. } => format!("app: {label}"),
        InteractionEvent::AppChange {
            app, window_title, ..
        } => format!(
            "switched to {app}{}",
            window_title
                .as_ref()
                .map(|s| format!(" — “{s}”"))
                .unwrap_or_default()
        ),
    }
}

pub fn timeline(bundle: &ScrawlBundle) -> Vec<String> {
    let mut items: Vec<(u64, String)> = bundle
        .transcript
        .segments
        .iter()
        .map(|s| (s.t0, format!("{} “{}”", format_clock(s.t0), s.text)))
        .collect();
    for mark in &bundle.marks {
        let frame = bundle.keyframes.iter().find(|k| mark_on_keyframe(mark, k));
        let suffix = frame
            .map(|k| format!(" — frame #{}", k.index))
            .unwrap_or_default();
        items.push((
            mark.t0,
            format!(
                "{} ✎ {}{suffix}",
                format_clock(mark.t0),
                describe_mark(mark)
            ),
        ));
    }
    for event in settled_events(&bundle.events) {
        items.push((
            event.timestamp_ms(),
            format!(
                "{} · {}",
                format_clock(event.timestamp_ms()),
                describe_event(&event)
            ),
        ));
    }
    items.sort_by_key(|item| item.0);
    items.into_iter().map(|item| item.1).collect()
}

/// Longest app-provided screen name or context value shown in the summary.
const CONTEXT_VALUE_CHARS: usize = 60;

/// An app-provided string on one line, cut to `CONTEXT_VALUE_CHARS`.
fn context_value(raw: &str) -> String {
    let compact = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= CONTEXT_VALUE_CHARS {
        compact
    } else {
        let cut: String = compact.chars().take(CONTEXT_VALUE_CHARS - 1).collect();
        format!("{}…", cut.trim_end())
    }
}

/// Agents given only the text fix what was said and skip the pictures, missing what only shows there.
pub const LOOK_AT_IMAGES: &str = "Open contact-sheet.png and frames/ in that folder before changing anything: the marks and words only make sense on the images, which can show more than the text does.";

pub fn summary_markdown(bundle: &ScrawlBundle, bundle_dir: &std::path::Path) -> String {
    summary_markdown_for_config(bundle, bundle_dir, &crate::handoff::engine_config())
}

pub fn summary_markdown_for_config(
    bundle: &ScrawlBundle,
    bundle_dir: &std::path::Path,
    config: &crate::handoff::EngineConfig,
) -> String {
    let agent_context = config.experiment(crate::handoff::AGENT_CONTEXT);
    summary_markdown_with(bundle, bundle_dir, agent_context)
}

/// `agent_context` is the `agentContext` experiment (`agent_context::keyframe_texts`).
pub fn summary_markdown_with(
    bundle: &ScrawlBundle,
    bundle_dir: &std::path::Path,
    agent_context: bool,
) -> String {
    let mut target = bundle
        .target
        .app
        .clone()
        .unwrap_or_else(|| "Display".into());
    if let Some(title) = &bundle.target.window_title
        && !title.is_empty()
        && Some(title.as_str()) != bundle.target.app.as_deref()
    {
        target.push_str(&format!(" “{title}”"));
    }
    if let Some(device) = &bundle.device {
        target = device.name.clone();
        if let Some(app) = &device.app {
            target.push_str(&format!(" · {app}"));
            let short = device
                .app_short_version
                .as_deref()
                .filter(|v| !v.is_empty());
            let build = device.app_version.as_deref().filter(|v| !v.is_empty());
            match (short, build) {
                (Some(short), Some(build)) if short != build => {
                    target.push_str(&format!(" {short} ({build})"));
                }
                (Some(short), _) => target.push_str(&format!(" {short}")),
                (None, Some(build)) => target.push_str(&format!(" ({build})")),
                (None, None) => {}
            }
        }
    }
    let mut out = format!(
        "[scrawl {} · {target} · {}]\n",
        format_clock(bundle.duration_ms),
        bundle_dir.display()
    );
    if let Some(dest) = &bundle.destination {
        let title = if dest.title.is_empty() {
            &dest.session
        } else {
            &dest.title
        };
        out.push_str(&format!("For session: {title} ({})", dest.session));
        if let Some(url) = &dest.url {
            out.push_str(&format!(" · page {url}"));
        }
        out.push('\n');
    }
    if !bundle.keyframes.is_empty() {
        out.push_str(LOOK_AT_IMAGES);
        out.push('\n');
    }
    // The screen at the start and the app's own values, on one line.
    let start_screen = bundle.screen_at(0);
    let context: Vec<String> = start_screen
        .map(|screen| format!("Screen: {}", context_value(screen)))
        .into_iter()
        .chain(
            bundle
                .context
                .iter()
                .map(|(key, value)| format!("{}: {}", context_value(key), context_value(value))),
        )
        .collect();
    if !context.is_empty() {
        out.push_str(&context.join(" · "));
        out.push('\n');
    }
    if let Some(note) = &bundle.note
        && !note.trim().is_empty()
    {
        out.push_str(&format!("Note: {}\n", note.trim()));
    }
    if let Some(crop) = &bundle.autocrop {
        out.push_str(&format!(
            "Cropped to: {} (full frames in {}/)\n",
            crop.label, crop.full_frames
        ));
    }
    if let Some(missing) = &bundle.transcript.missing
        && bundle.transcript.text.trim().is_empty()
    {
        out.push_str(&format!("No transcript: {}\n", missing.reason));
    }
    // App events and context changes nest under the keyframe on screen when they happened.
    let mut app_events: Vec<(u64, String)> = bundle
        .events
        .iter()
        .filter_map(|event| match event {
            InteractionEvent::AppEvent { t, label } => Some((*t, format!("app: {label}"))),
            _ => None,
        })
        .collect();
    app_events.extend(bundle.context_changes.iter().map(|change| {
        let key = context_value(&change.key);
        let text = match &change.value {
            Some(value) => format!("{key} → {}", context_value(value)),
            None => format!("{key} cleared"),
        };
        (change.t, text)
    }));
    app_events.sort_by_key(|(t, _)| *t);
    if agent_context && !bundle.keyframes.is_empty() {
        let texts = crate::agent_context::keyframe_texts(bundle);
        let mut shown_screen = start_screen;
        for (frame, text) in bundle.keyframes.iter().zip(texts) {
            let screen = bundle.screen_at(frame.t);
            let screen_marker = match screen {
                Some(name) if screen != shown_screen => format!(" ⟨{}⟩", context_value(name)),
                _ => String::new(),
            };
            shown_screen = screen;
            let flag = frame
                .mark_id
                .as_ref()
                .and_then(|id| bundle.marks.iter().find(|m| &m.id == id))
                .is_some_and(|m| m.shape == Shape::Flag);
            let flag_marker = if flag { " ⚑" } else { "" };
            let head = if text.head.is_empty() {
                String::new()
            } else {
                format!(" {}", text.head)
            };
            out.push_str(&format!(
                "- #{} ({}){screen_marker}{flag_marker}{head}\n",
                frame.index,
                format_clock(frame.t)
            ));
            for line in text.lines {
                out.push_str(&format!("  {line}\n"));
            }
        }
    } else if !bundle.keyframes.is_empty() {
        let first_t = bundle.keyframes[0].t;
        for (t, label) in &app_events {
            if *t < first_t {
                out.push_str(&format!("  · {} {label}\n", format_clock(*t)));
            }
        }
        let mut shown_screen = start_screen;
        for (i, frame) in bundle.keyframes.iter().enumerate() {
            let clock = format_clock(frame.t);
            let screen = bundle.screen_at(frame.t);
            let screen_marker = match screen {
                Some(name) if screen != shown_screen => format!(" ⟨{}⟩", context_value(name)),
                _ => String::new(),
            };
            shown_screen = screen;
            let flag = frame
                .mark_id
                .as_ref()
                .and_then(|id| bundle.marks.iter().find(|m| &m.id == id))
                .is_some_and(|m| m.shape == Shape::Flag);
            let flag_marker = if flag { " ⚑" } else { "" };
            match frame
                .said
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                Some(said) => {
                    let action_marker = if flag {
                        " ⚑".to_owned()
                    } else if frame.reason == KeyframeReason::Click {
                        let compact = frame
                            .event
                            .split(" (")
                            .next()
                            .unwrap_or(&frame.event)
                            .trim();
                        if compact.is_empty() {
                            String::new()
                        } else {
                            format!(" [{compact}]")
                        }
                    } else {
                        String::new()
                    };
                    out.push_str(&format!(
                        "- #{} ({clock}){screen_marker}{action_marker} “{said}”\n",
                        frame.index
                    ));
                }
                None => out.push_str(&format!(
                    "- #{} ({clock}){screen_marker}{flag_marker} {}\n",
                    frame.index, frame.event
                )),
            }
            let next = bundle.keyframes.get(i + 1).map_or(u64::MAX, |k| k.t);
            for (t, label) in &app_events {
                if (*t < next || i + 1 == bundle.keyframes.len()) && *t >= frame.t {
                    out.push_str(&format!("  · {} {label}\n", format_clock(*t)));
                }
            }
        }
    } else if !app_events.is_empty() {
        for (t, label) in &app_events {
            out.push_str(&format!("· {} {label}\n", format_clock(*t)));
        }
    } else if !bundle.transcript.text.trim().is_empty() {
        out.push_str(&format!("“{}”\n", bundle.transcript.text.trim()));
    }
    out
}

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

    /// The two ⇧ strokes of a real capture (`scrawl-1792536608591-641c-2`), drawn with a mouse on
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
        let bundle = ScrawlBundle {
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
        };
        let tl = timeline(&bundle).join("\n");
        assert!(tl.find("0:03 “").unwrap() < tl.find("0:04 · pressed").unwrap());
        assert!(tl.find("0:04 · pressed").unwrap() < tl.find("0:05 ✎").unwrap());
        assert!(tl.contains("— frame #1"));

        let summary = summary_markdown(&bundle, std::path::Path::new("/recordings/test"));
        assert!(summary.starts_with("[scrawl 0:14 · App “Window” · /recordings/test]"));
        assert!(summary.contains("Note: Simplify it."));
        assert!(summary.contains("- #1 (0:05) “This row is too dense.”"));
        assert!(!summary.contains("No transcript"));

        // Audio but no speech model yet: the summary says so instead of staying quiet.
        let mut silent = bundle.clone();
        silent.transcript = TranscriptData {
            source: "offline".into(),
            missing: Some(crate::speech_model::missing("downloading", "en_US", None)),
            ..Default::default()
        };
        let summary = summary_markdown(&silent, std::path::Path::new("/recordings/test"));
        assert!(summary.contains("No transcript: "));
        assert!(summary.contains("(en_US) was still downloading. The audio is saved"));

        // A phone bundle: device header, a flagged keyframe, and app events under the keyframe
        // that was on screen when they happened.
        let mut phone = bundle;
        phone.note = None;
        phone.device = Some(DeviceInfo {
            name: "iPhone".into(),
            capture: "scrawlkit".into(),
            app: Some("dev.example.App".into()),
            app_version: Some("42".into()),
            ..Default::default()
        });
        phone.source = Some("iphone".into());
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
        let summary = summary_markdown(&phone, std::path::Path::new("/recordings/phone"));
        assert_eq!(
            summary,
            format!(
                "[scrawl 0:14 · iPhone · dev.example.App (42) · /recordings/phone]\n\
                 {LOOK_AT_IMAGES}\n\
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
        let mut bundle: ScrawlBundle = serde_json::from_value(serde_json::json!({
            "version": 1, "id": "t", "createdAtMs": 0, "durationMs": 25_000,
            "target": {"mode": "device", "app": "OpenCode", "bundleId": "ai.opencode.mobilecode",
                "displayId": 0,
                "frame": {"x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0, "scale": 1.0}},
            "transcript": {"source": "test", "text": "", "segments": []},
            "strokes": [], "marks": [], "events": [], "keyframes": [], "contactSheet": "c.png",
            "source": "iphone",
            "device": {"name": "iPhone", "capture": "scrawlkit", "width": 10, "height": 20,
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
                "[scrawl 0:25 · iPhone · OpenCode 1.4 (354) · /recordings/phone]\n\
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
            "[scrawl 0:25 · iPhone · OpenCode (354) · /recordings/phone]\n{LOOK_AT_IMAGES}\n- #1 (0:01) “So here.”\n"
        )));
    }
}
