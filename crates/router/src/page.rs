//! What a dev page saw during a capture (the `agentContext` experiment): `sdk.js` on a loopback
//! page posts the element and component under clicks and ⇧ marks, `<video>` seeks and play/pause,
//! console errors and failed requests to `/web/context`. The app keeps the last few minutes per
//! tab; when a capture finishes, `apply` names its clicks and strokes from them and adds the rest
//! as app events. Pages never send request bodies, headers, cookies, query strings or what was
//! typed into a field; every string is cut short again here.

use std::collections::VecDeque;
use std::sync::Mutex;

use serde::Deserialize;

use crate::model::{InteractionEvent, Stroke};

/// Events older than this are dropped.
const KEEP_MS: u64 = 180_000;
const MAX_EVENTS_PER_TAB: usize = 300;
const MAX_TABS: usize = 16;
const MAX_TEXT: usize = 60;
const MAX_MESSAGE: usize = 160;

/// A page click this close to a Mac click (ms) is the same click.
const CLICK_MATCH_MS: u64 = 400;

/// Seeks closer together than this read as one scrub.
const SEEK_RUN_MS: u64 = 1_500;

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Target {
    #[serde(default)]
    pub tag: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub cls: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub component: Option<String>,
    /// `src/ui/Pill.tsx:142`.
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Media {
    #[serde(default)]
    pub src: String,
    #[serde(default)]
    pub time: f64,
    #[serde(default)]
    pub duration: f64,
    #[serde(default)]
    pub paused: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Event {
    /// Epoch ms.
    pub t: u64,
    /// `pointer`, `mark`, `error`, `request` or `media`.
    pub kind: String,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub target: Option<Target>,
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub method: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub status: Option<u16>,
    #[serde(default)]
    pub ms: Option<u64>,
    /// `seeked`, `play` or `pause` for media events.
    #[serde(default)]
    pub event: Option<String>,
    #[serde(default)]
    pub media: Option<Media>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Post {
    #[serde(default)]
    pub tab: String,
    #[serde(default)]
    pub events: Vec<Event>,
}

fn clip(raw: &str, max: usize) -> Option<String> {
    let compact = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() {
        return None;
    }
    Some(if compact.chars().count() <= max {
        compact
    } else {
        let cut: String = compact.chars().take(max - 1).collect();
        format!("{}…", cut.trim_end())
    })
}

fn clip_opt(value: Option<String>, max: usize) -> Option<String> {
    value.as_deref().and_then(|v| clip(v, max))
}

/// A request path without its query or fragment.
fn bare_path(raw: &str) -> Option<String> {
    let path = raw.split(['?', '#']).next()?;
    clip(path, 120)
}

/// The event as it may be kept, or `None` for an unknown kind or a time far from `now`.
fn sanitize(event: Event, now: u64) -> Option<Event> {
    let kinds = ["pointer", "mark", "error", "request", "media"];
    if !kinds.contains(&event.kind.as_str()) || event.t + KEEP_MS < now || event.t > now + 5_000 {
        return None;
    }
    let target = event.target.map(|t| Target {
        tag: t
            .tag
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .take(20)
            .collect(),
        id: clip_opt(t.id, MAX_TEXT),
        cls: clip_opt(t.cls, MAX_TEXT),
        text: clip_opt(t.text, MAX_TEXT),
        component: clip_opt(t.component, MAX_TEXT),
        source: clip_opt(t.source, 120),
    });
    let media = event.media.map(|m| Media {
        src: clip(&m.src, MAX_TEXT).unwrap_or_default(),
        time: if m.time.is_finite() {
            m.time.max(0.0)
        } else {
            0.0
        },
        duration: if m.duration.is_finite() {
            m.duration.max(0.0)
        } else {
            0.0
        },
        paused: m.paused,
    });
    Some(Event {
        t: event.t,
        kind: event.kind,
        focused: event.focused,
        target,
        message: clip_opt(event.message, MAX_MESSAGE),
        source: clip_opt(event.source, 120),
        method: event.method.map(|m| {
            m.chars()
                .filter(char::is_ascii_alphabetic)
                .take(8)
                .collect()
        }),
        path: event.path.as_deref().and_then(bare_path),
        status: event.status,
        ms: event.ms,
        event: event
            .event
            .filter(|e| matches!(e.as_str(), "seeked" | "play" | "pause")),
        media,
    })
}

#[derive(Debug, Default)]
struct Tab {
    key: (String, String),
    events: VecDeque<Event>,
}

#[derive(Debug, Default)]
pub struct Log {
    tabs: Vec<Tab>,
}

impl Log {
    pub fn record(&mut self, origin: &str, post: Post, now: u64) {
        let key = (origin.to_owned(), post.tab.chars().take(32).collect());
        let index = match self.tabs.iter().position(|t| t.key == key) {
            Some(i) => i,
            None => {
                self.tabs.push(Tab {
                    key,
                    events: VecDeque::new(),
                });
                self.tabs.len() - 1
            }
        };
        let tab = &mut self.tabs[index];
        for event in post.events.into_iter().take(64) {
            if let Some(event) = sanitize(event, now) {
                tab.events.push_back(event);
            }
        }
        while tab.events.len() > MAX_EVENTS_PER_TAB {
            tab.events.pop_front();
        }
        for tab in &mut self.tabs {
            tab.events.retain(|e| e.t + KEEP_MS >= now);
        }
        self.tabs.retain(|t| !t.events.is_empty());
        if self.tabs.len() > MAX_TABS {
            self.tabs
                .sort_by_key(|t| std::cmp::Reverse(t.events.back().map_or(0, |e| e.t)));
            self.tabs.truncate(MAX_TABS);
        }
    }

    /// Events between two epoch times from the tabs the user was in then: ones that were clicked,
    /// marked or focused. Background tabs' errors stay out.
    pub fn between(&self, from: u64, to: u64) -> Vec<Event> {
        let mut out: Vec<Event> = Vec::new();
        for tab in &self.tabs {
            let inside: Vec<&Event> = tab
                .events
                .iter()
                .filter(|e| e.t >= from && e.t <= to)
                .collect();
            let used = inside
                .iter()
                .any(|e| e.focused || e.kind == "pointer" || e.kind == "mark");
            if used {
                out.extend(inside.into_iter().cloned());
            }
        }
        out.sort_by_key(|e| e.t);
        out
    }
}

static LOG: Mutex<Log> = Mutex::new(Log { tabs: Vec::new() });

pub fn record(origin: &str, post: Post) {
    LOG.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .record(origin, post, crate::now_epoch_ms());
}

pub fn between(from: u64, to: u64) -> Vec<Event> {
    LOG.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .between(from, to)
}

/// `<button#new.tab-new> “New tab” in BrowserHeader (src/BrowserHeader.tsx:88)`.
pub fn describe(target: &Target) -> String {
    let mut element = format!(
        "<{}",
        if target.tag.is_empty() {
            "element"
        } else {
            &target.tag
        }
    );
    if let Some(id) = &target.id {
        element.push_str(&format!("#{id}"));
    }
    if let Some(cls) = &target.cls {
        element.push_str(&format!(".{cls}"));
    }
    element.push('>');
    let mut parts = vec![element];
    if let Some(text) = &target.text {
        parts.push(format!("“{text}”"));
    }
    match (&target.component, &target.source) {
        (Some(c), Some(s)) => parts.push(format!("in {c} ({s})")),
        (Some(c), None) => parts.push(format!("in {c}")),
        (None, Some(s)) => parts.push(format!("({s})")),
        (None, None) => {}
    }
    parts.join(" ")
}

fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn describe_media(event: &str, media: &Media) -> String {
    let name = if media.src.is_empty() {
        "video".to_owned()
    } else {
        format!("video {}", media.src)
    };
    let at = if media.duration > 0.0 {
        format!("{} / {}", clock(media.time), clock(media.duration))
    } else {
        clock(media.time)
    };
    match event {
        "seeked" => format!("{name} scrubbed to {at}"),
        "play" => format!("{name} played from {at}"),
        _ => format!("{name} paused at {at}"),
    }
}

/// Names the capture's clicks and strokes from what the page saw under them, and adds errors,
/// failed requests and video playback as app events. `start` is the capture's start (epoch ms).
pub fn apply(
    page: &[Event],
    start: u64,
    events: &mut Vec<InteractionEvent>,
    strokes: &mut [Stroke],
) {
    let rel = |t: u64| t.saturating_sub(start);
    for pointer in page.iter().filter(|e| e.kind == "pointer") {
        let Some(target) = &pointer.target else {
            continue;
        };
        let t = rel(pointer.t);
        let nearest = events
            .iter_mut()
            .filter_map(|event| {
                let at = match event {
                    InteractionEvent::Click { t, .. } | InteractionEvent::Drag { t, .. } => *t,
                    _ => return None,
                };
                let gap = at.abs_diff(t);
                (gap <= CLICK_MATCH_MS).then_some((gap, event))
            })
            .min_by_key(|(gap, _)| *gap);
        if let Some((_, event)) = nearest {
            match event {
                InteractionEvent::Click { label, .. } | InteractionEvent::Drag { label, .. } => {
                    *label = Some(describe(target));
                }
                _ => {}
            }
        }
    }
    for stroke in strokes.iter_mut().filter(|s| !s.laser) {
        let mut seen: Vec<(String, usize)> = Vec::new();
        for mark in page.iter().filter(|e| {
            e.kind == "mark" && rel(e.t) + 150 >= stroke.t0 && rel(e.t) <= stroke.t1 + 150
        }) {
            if let Some(target) = &mark.target {
                let text = describe(target);
                match seen.iter_mut().find(|(t, _)| *t == text) {
                    Some((_, n)) => *n += 1,
                    None => seen.push((text, 1)),
                }
            }
        }
        if let Some((text, _)) = seen.into_iter().max_by_key(|(_, n)| *n) {
            stroke.on = Some(text);
        }
    }
    let mut last_seek: Option<usize> = None;
    for event in page {
        let t = rel(event.t);
        let label = match event.kind.as_str() {
            "error" => {
                let message = event.message.as_deref().unwrap_or("error");
                match &event.source {
                    Some(source) => format!("console error: {message} ({source})"),
                    None => format!("console error: {message}"),
                }
            }
            "request" => {
                let method = event.method.as_deref().unwrap_or("GET");
                let path = event.path.as_deref().unwrap_or("?");
                match event.status {
                    Some(status) if status > 0 => format!("{method} {path} → {status}"),
                    _ => format!("{method} {path} → failed"),
                }
            }
            "media" => {
                let (Some(kind), Some(media)) = (&event.event, &event.media) else {
                    continue;
                };
                let label = describe_media(kind, media);
                // A scrub fires a seek per step; keep the run's last.
                if kind == "seeked"
                    && let Some(i) = last_seek
                    && let Some(InteractionEvent::AppEvent { t: prev, label: l }) =
                        events.get_mut(i)
                    && t.saturating_sub(*prev) < SEEK_RUN_MS
                {
                    *prev = t;
                    *l = label;
                    continue;
                }
                events.push(InteractionEvent::AppEvent { t, label });
                last_seek = (kind == "seeked").then(|| events.len() - 1);
                continue;
            }
            _ => continue,
        };
        events.push(InteractionEvent::AppEvent { t, label });
    }
    events.sort_by_key(InteractionEvent::timestamp_ms);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(t: u64, kind: &str) -> Event {
        Event {
            t,
            kind: kind.into(),
            ..Event::default()
        }
    }

    fn button() -> Target {
        Target {
            tag: "button".into(),
            id: None,
            cls: Some("tab-new".into()),
            text: Some("New tab".into()),
            component: Some("BrowserHeader".into()),
            source: Some("src/BrowserHeader.tsx:88".into()),
        }
    }

    #[test]
    fn posts_are_trimmed_and_kept_for_a_few_minutes() {
        let mut log = Log::default();
        let now = 1_000_000;
        let long = "x".repeat(500);
        let post = Post {
            tab: "t1".into(),
            events: vec![
                Event {
                    message: Some(long.clone()),
                    ..event(now, "error")
                },
                Event {
                    method: Some("POST\r\nX: y".into()),
                    path: Some("/api/save?token=secret#x".into()),
                    status: Some(500),
                    ..event(now, "request")
                },
                event(now, "keystroke"),
                event(now - KEEP_MS - 1, "error"),
            ],
        };
        log.record("http://localhost:5173", post, now);
        let kept = &log.tabs[0].events;
        assert_eq!(kept.len(), 2, "unknown kinds and stale events are dropped");
        assert_eq!(
            kept[0].message.as_ref().unwrap().chars().count(),
            MAX_MESSAGE
        );
        assert_eq!(kept[1].path.as_deref(), Some("/api/save"));
        assert_eq!(kept[1].method.as_deref(), Some("POSTXy"));
        log.record("http://localhost:5173", Post::default(), now + KEEP_MS + 1);
        assert!(log.tabs.is_empty());
    }

    #[test]
    fn only_tabs_in_use_during_the_capture_count() {
        let mut log = Log::default();
        let now = 1_000_000;
        log.record(
            "http://localhost:1",
            Post {
                tab: "background".into(),
                events: vec![event(now, "error")],
            },
            now,
        );
        log.record(
            "http://localhost:2",
            Post {
                tab: "front".into(),
                events: vec![
                    Event {
                        target: Some(button()),
                        ..event(now - 10, "pointer")
                    },
                    event(now, "error"),
                ],
            },
            now,
        );
        let got = log.between(now - 1_000, now + 1);
        assert_eq!(got.len(), 2);
        assert!(got.iter().all(|e| e.kind == "pointer" || e.t == now));
        assert!(log.between(now + 10, now + 20).is_empty());
    }

    #[test]
    fn page_facts_name_clicks_and_marks_and_add_errors_and_scrubs() {
        let start = 10_000;
        let mut events = vec![
            InteractionEvent::Click {
                t: 1_000,
                x: 0.5,
                y: 0.1,
                button: Some("left".into()),
                label: Some("“+”".into()),
                role: Some("button".into()),
            },
            InteractionEvent::Click {
                t: 5_000,
                x: 0.5,
                y: 0.1,
                button: None,
                label: None,
                role: None,
            },
        ];
        let mut strokes = vec![Stroke {
            t0: 3_000,
            t1: 3_600,
            fw: 100,
            fh: 100,
            on: Some("group".into()),
            pin: None,
            laser: false,
            pts: vec![0, 0, 0],
        }];
        let pill = Target {
            tag: "div".into(),
            cls: Some("capsule".into()),
            component: Some("Pill".into()),
            source: Some("src/Pill.tsx:42".into()),
            ..Target::default()
        };
        let media = |time| Media {
            src: "clips.mp4".into(),
            time,
            duration: 11.0,
            paused: true,
        };
        let page = vec![
            Event {
                target: Some(button()),
                ..event(start + 1_120, "pointer")
            },
            Event {
                target: Some(pill.clone()),
                ..event(start + 3_100, "mark")
            },
            Event {
                target: Some(pill),
                ..event(start + 3_400, "mark")
            },
            Event {
                message: Some("Uncaught TypeError: x is undefined".into()),
                source: Some("src/BrowserHeader.tsx:92".into()),
                ..event(start + 2_000, "error")
            },
            Event {
                method: Some("GET".into()),
                path: Some("/api/tabs".into()),
                status: Some(404),
                ..event(start + 2_500, "request")
            },
            Event {
                event: Some("seeked".into()),
                media: Some(media(1.0)),
                ..event(start + 4_000, "media")
            },
            Event {
                event: Some("seeked".into()),
                media: Some(media(4.2)),
                ..event(start + 4_600, "media")
            },
        ];
        apply(&page, start, &mut events, &mut strokes);
        let labels: Vec<String> = events.iter().map(crate::sketch::describe_event).collect();
        assert_eq!(
            labels[0],
            "clicked button <button.tab-new> “New tab” in BrowserHeader (src/BrowserHeader.tsx:88) (x 0.50, y 0.10)"
        );
        assert!(labels.contains(
            &"app: console error: Uncaught TypeError: x is undefined (src/BrowserHeader.tsx:92)"
                .to_owned()
        ));
        assert!(labels.contains(&"app: GET /api/tabs → 404".to_owned()));
        assert!(labels.contains(&"app: video clips.mp4 scrubbed to 0:04 / 0:11".to_owned()));
        assert_eq!(labels.iter().filter(|l| l.contains("scrubbed")).count(), 1);
        assert!(
            labels.last().unwrap().starts_with("clicked (x"),
            "unmatched click untouched"
        );
        assert_eq!(
            strokes[0].on.as_deref(),
            Some("<div.capsule> in Pill (src/Pill.tsx:42)")
        );
    }
}
