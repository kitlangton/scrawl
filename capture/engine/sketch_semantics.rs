//! Time-aligned descriptions and bundle Markdown output.
use crate::shared::model::*;

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
/// `crate::shared::ax`, `<div.capsule> in Pill (src/Pill.tsx:42)` from `crate::page`).
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
        InteractionEvent::Note { pin, text, .. } => format!("typed note on pin {pin}: {text}"),
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

pub fn timeline(bundle: &ApparentBundle) -> Vec<String> {
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

/// Lossless typed feedback shared by the ordinary and interleaved timelines.
pub(crate) fn typed_note_text(bundle: &ApparentBundle, pin: u32, text: &str) -> String {
    let frame = bundle
        .marks
        .iter()
        .find(|m| m.pin == Some(pin))
        .and_then(|m| m.frame.as_deref());
    let mut label = format!(
        "Typed note · pin {pin}{}:",
        frame.map(|f| format!(" · {f}")).unwrap_or_default()
    );
    for line in text.lines() {
        label.push_str(&format!("\n    {line}"));
    }
    label
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
pub const OPTIONAL_IMAGES: &str = "Visuals are saved in this bundle, but are optional for this text-first handoff. Use apparent zoom if the words need screen context; contact-sheet.png and frames/ are also available there.";

pub fn summary_markdown(bundle: &ApparentBundle, bundle_dir: &std::path::Path) -> String {
    let agent_context = crate::shared::handoff::experiment(crate::shared::handoff::AGENT_CONTEXT);
    summary_markdown_with(bundle, bundle_dir, agent_context)
}

/// `agent_context` is the `agentContext` experiment (`agent_context::keyframe_texts`).
pub fn summary_markdown_with(
    bundle: &ApparentBundle,
    bundle_dir: &std::path::Path,
    agent_context: bool,
) -> String {
    summary_markdown_with_visual_policy(bundle, bundle_dir, agent_context)
}

pub(crate) fn summary_markdown_with_visual_policy(
    bundle: &ApparentBundle,
    bundle_dir: &std::path::Path,
    agent_context: bool,
) -> String {
    summary_markdown_formatted(bundle, bundle_dir, agent_context, None)
}

/// Clipboard transport instructions are supplied by Handoff; archived summaries keep
/// their full local inspection context. Words, marks, notes and warnings stay semantic.
pub(crate) fn clipboard_markdown(
    bundle: &ApparentBundle,
    bundle_dir: &std::path::Path,
    hosted: bool,
) -> String {
    summary_markdown_formatted(
        bundle,
        bundle_dir,
        crate::shared::handoff::experiment(crate::shared::handoff::AGENT_CONTEXT),
        Some(hosted),
    )
}

fn summary_markdown_formatted(
    bundle: &ApparentBundle,
    bundle_dir: &std::path::Path,
    agent_context: bool,
    clipboard_hosted: Option<bool>,
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
    let mut out = match clipboard_hosted {
        Some(true) => format!("[apparent {}]\n", format_clock(bundle.duration_ms)),
        Some(false) => format!(
            "[apparent {} · {}]\n",
            format_clock(bundle.duration_ms),
            bundle_dir.display()
        ),
        None => format!(
            "[apparent {} · {target} · {}]\n",
            format_clock(bundle.duration_ms),
            bundle_dir.display()
        ),
    };
    if clipboard_hosted.is_none()
        && let Some(dest) = &bundle.destination
    {
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
    if clipboard_hosted.is_none() && !bundle.keyframes.is_empty() {
        out.push_str(if bundle.has_screen_click() {
            LOOK_AT_IMAGES
        } else {
            OPTIONAL_IMAGES
        });
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
    if bundle.interaction_telemetry == Some(crate::shared::model::InteractionTelemetry::VideoOnly) {
        out.push_str("Interaction context: timed CLI video only; clicks and Shift ink were not collected. A visible click in the video will not appear in events, marks or click keyframes. Use the app's recording shortcut for interaction context.\n");
    }
    if let Some(warning) = &bundle.image_quality_warning {
        out.push_str(&format!("Image quality: {warning}\n"));
    }
    if clipboard_hosted.is_none()
        && let Some(crop) = &bundle.autocrop
    {
        out.push_str(&format!(
            "Cropped to: {} (full frames in {}/)\n",
            crop.label, crop.full_frames
        ));
    }
    if bundle.source.as_deref() != Some("iphone")
        && !bundle
            .context
            .get("microphoneCapture")
            .is_some_and(|v| v == "off")
    {
        // Evidence snapshots deliberately defer declared audio; its absence is
        // not a microphone failure on the recording computer.
        let deferred_audio =
            bundle.audio.is_some() && bundle_dir.join(".evidence-only.json").is_file();
        if !bundle.has_microphone_samples(bundle_dir) && !deferred_audio {
            out.push_str("No microphone audio was saved. Check the microphone and try again.\n");
        } else if bundle.transcript.text.trim().is_empty()
            && (bundle.transcript.missing.is_some() || bundle.transcript.source == "none")
        {
            if let Some(missing) = &bundle.transcript.missing {
                out.push_str(&format!("No transcript: {}\n", missing.reason));
            } else {
                out.push_str("No transcript: Recognition did not run. The audio is saved.\n");
            }
        } else if bundle.transcript.text.trim().is_empty() {
            out.push_str("No words transcribed. Your audio was saved—check it or try again closer to the microphone.\n");
        }
    }
    // App events and context changes nest under the keyframe on screen when they happened.
    let mut app_events: Vec<(u64, String)> = bundle
        .events
        .iter()
        .filter_map(|event| match event {
            InteractionEvent::AppEvent { t, label } => Some((*t, format!("app: {label}"))),
            InteractionEvent::Note { t, pin, text, .. } => {
                Some((*t, typed_note_text(bundle, *pin, text)))
            }
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
    // Text-only captures still need one timed stream: adding a note must never
    // suppress speech merely because no keyframe was selected.
    if bundle.keyframes.is_empty() {
        app_events.extend(
            bundle
                .transcript
                .segments
                .iter()
                .map(|s| (s.t0, format!("“{}”", s.text))),
        );
        if bundle.transcript.segments.is_empty() && !bundle.transcript.text.trim().is_empty() {
            app_events.push((0, format!("“{}”", bundle.transcript.text.trim())));
        }
    }
    app_events.sort_by_key(|(t, _)| *t);
    if agent_context && !bundle.keyframes.is_empty() {
        let texts = crate::capture::agent::context::keyframe_texts(bundle);
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
