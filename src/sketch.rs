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
    if stroke.pin.is_some() {
        return Shape::Pin;
    }
    let points: Vec<_> = stroke
        .normalized_points()
        .into_iter()
        .map(|p| [p[0] * stroke.fw as f32, p[1] * stroke.fh as f32])
        .collect();
    let Some(&first) = points.first() else {
        return Shape::Tap;
    };
    let b = bounds(&points);
    let (w, h) = (b[2] - b[0], b[3] - b[1]);
    let big = w.max(h);
    let len: f32 = points.windows(2).map(|p| distance(p[0], p[1])).sum();
    let last = *points.last().unwrap();
    let chord = distance(first, last);
    let shoelace = points
        .windows(2)
        .map(|p| p[0][0] * p[1][1] - p[1][0] * p[0][1])
        .sum::<f32>()
        + last[0] * first[1]
        - first[0] * last[1];
    let enclosed_area = 0.5 * shoelace.abs();
    if w.hypot(h) < 14.0 {
        Shape::Tap
    } else if chord < 0.35 * big
        && len > 2.0 * big
        && w.min(h) >= 18.0
        && w.min(h) > 0.42 * big
        && enclosed_area > 0.32 * w * h
    {
        Shape::Circle
    } else if chord > 0.88 * len {
        if w > 2.5 * h {
            Shape::Underline
        } else {
            Shape::Line
        }
    } else {
        Shape::Mark
    }
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
                        (s, shape_of(s), b, pts, size)
                    })
                    .collect();
                sized.sort_by(|a, b| b.4.total_cmp(&a.4));
                let (main, shape, b, pts, _) = &sized[0];
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
                    && (rest.iter().all(|r| r.1 == *shape) || *shape == Shape::Circle)
                {
                    match shape {
                        Shape::Circle => {
                            mark.shape = Shape::Circle;
                            if !rest.iter().all(|r| r.1 == Shape::Circle) {
                                mark.box_norm = *b;
                            }
                        }
                        Shape::Line if rest.is_empty() => {
                            mark.shape = Shape::Line;
                            mark.from = Some(a);
                            mark.to = Some(z);
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

/// What `crate::ax` names an unlabelled input field by (its kind alone, so its typed value is
/// never read): shown unquoted, as a kind rather than text on the screen.
pub(crate) const FIELD_NAMES: [&str; 5] = [
    "secure text field",
    "text field",
    "text area",
    "combo box",
    "search field",
];

/// A label in curly quotes, unless it already quotes its own parts (`button “Area” #area` from
/// `crate::ax`, `<div.capsule> in Pill (src/Pill.tsx:42)` from `crate::page`) or is an input
/// field's kind alone (`search field`).
pub(crate) fn quote_label(label: &str) -> String {
    if label.contains('“') || label.starts_with('<') || FIELD_NAMES.contains(&label) {
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
                let verb = if button.as_deref() == Some("touch") {
                    "swiped"
                } else {
                    "dragged"
                };
                let label = label.as_deref().filter(|s| !s.is_empty());
                let way = direction(x1 - x0, y1 - y0, 0.04);
                let (from, to) = (at([*x0, *y0]), at([*x1, *y1]));
                match label {
                    // Text read off the screen is what the drag went across, not what it moved.
                    Some(text) if crate::screen_text::is_read(text) => {
                        format!("{verb} {way} across {text} ({from} → {to})")
                    }
                    Some(name) => format!("{verb} {} {way} ({from} → {to})", quote_label(name)),
                    None => format!("{verb} {way} ({from} → {to})"),
                }
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

pub fn summary_markdown(bundle: &ScrawlBundle, bundle_dir: &std::path::Path) -> String {
    let agent_context = crate::handoff::experiment(crate::handoff::AGENT_CONTEXT);
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
            "[scrawl 0:14 · iPhone · dev.example.App (42) · /recordings/phone]\n\
             \x20 · 0:01 app: launched\n\
             - #1 (0:05) “This row is too dense.”\n\
             \x20 · 0:06 app: rest started\n\
             - #2 (0:09) ⚑ flagged this moment\n\
             \x20 · 0:09 app: Done tapped\n"
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
        assert!(summary.starts_with(
            "[scrawl 0:25 · iPhone · OpenCode (354) · /recordings/phone]\n- #1 (0:01) “So here.”\n"
        ));
    }
}
