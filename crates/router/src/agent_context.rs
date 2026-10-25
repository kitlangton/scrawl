//! The `agentContext` experiment's text: each keyframe's speech broken up by what happened while
//! it was said (marks, clicks, drags, shortcuts, windows opening and closing, app events), so the
//! agent reads "this part" next to the mark drawn as it was said. Marks sharing a keyframe are
//! numbered ①②③ in drawing order (the images show only the ink; `scrawl zoom --mark` and
//! `scrawl inspect --marks` look marks up by these numbers).
//! Nothing here gives raw coordinates: the images show where, so a place is only written when
//! something names it (an Accessibility label, the dev page's element).

use crate::model::*;
use crate::sketch::{
    describe_event, direction, mark_on_keyframe, marks_on, quote_label, settled_events,
};

/// One keyframe's text: `head` follows `- #N (M:SS)` on the bullet line, `lines` go under it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FrameText {
    pub head: String,
    pub lines: Vec<String>,
}

/// How far back an item's place in the speech moves to the start of its clause.
const CLAUSE_SNAP_MS: u64 = 3_000;

/// How far forward an item's place moves to the end of its clause when the start is too far back.
const CLAUSE_SNAP_AFTER_MS: u64 = 1_200;

/// A pause this long ends a clause (`keyframes::CLAUSE_GAP_MS`).
const CLAUSE_GAP_MS: u64 = 1_500;

/// How long before a click keyframe its click can be.
const CLICK_ANCHOR_MS: u64 = 1_500;

/// An unnamed drag shorter than this (normalized) says nothing the images don't.
const MIN_DRAG: f32 = 0.15;

/// How close a drag is to the dev page's report of a video scrub it caused.
const SCRUB_MS: u64 = 1_500;

/// Repeats of one thing closer together than this read as one line (`pressed ⌘T ×3`).
const REPEAT_MS: u64 = 3_000;

/// More on-screen windows than this changing at once (a Space switch) is one line.
const WINDOW_BURST: usize = 4;

#[derive(Clone, Debug)]
enum Item<'a> {
    Mark {
        mark: &'a Mark,
        number: Option<usize>,
    },
    Event {
        text: String,
    },
}

#[derive(Clone, Debug)]
struct Timed<'a> {
    t: u64,
    item: Item<'a>,
}

/// The circled number for the `n`th mark (1-based) on a shared keyframe.
pub fn circled(n: usize) -> String {
    const DIGITS: [char; 20] = [
        '①', '②', '③', '④', '⑤', '⑥', '⑦', '⑧', '⑨', '⑩', '⑪', '⑫', '⑬', '⑭', '⑮', '⑯', '⑰', '⑱',
        '⑲', '⑳',
    ];
    DIGITS
        .get(n.wrapping_sub(1))
        .map_or_else(|| format!("({n})"), char::to_string)
}

pub fn keyframe_texts(bundle: &ScrawlBundle) -> Vec<FrameText> {
    let frames = &bundle.keyframes;
    if frames.is_empty() {
        return Vec::new();
    }
    let words = frame_words(bundle);
    let starts = frame_starts(bundle, words.as_deref());
    let owner = |t: u64| starts.iter().rposition(|&s| s <= t).unwrap_or(0);

    let mut items: Vec<Vec<Timed>> = vec![Vec::new(); frames.len()];
    for (i, frame) in frames.iter().enumerate() {
        let shared = marks_on(frame, &bundle.marks);
        let numbered = shared.len() >= 2;
        for (n, mark) in shared.into_iter().enumerate() {
            items[i].push(Timed {
                t: mark.t0,
                item: Item::Mark {
                    mark,
                    number: numbered.then_some(n + 1),
                },
            });
        }
    }
    for (t, text) in events(bundle) {
        items[owner(t)].push(Timed {
            t,
            item: Item::Event { text },
        });
    }

    frames
        .iter()
        .enumerate()
        .map(|(i, frame)| {
            let mut timed = std::mem::take(&mut items[i]);
            timed.sort_by_key(|item| item.t);
            let mut text = match &words {
                Some(words) => interleave(&words[i], &timed),
                None => interleave_unsplit(frame.said.as_deref(), &timed),
            };
            if let Some(changed) = changed_regions(bundle, i) {
                text.head = if text.head.is_empty() {
                    format!("[changed: {changed}]")
                } else {
                    format!("[changed: {changed}] {}", text.head)
                };
            }
            if text.head.is_empty() && text.lines.is_empty() {
                text.head = without_positions(&frame.event);
            }
            text
        })
        .collect()
}

/// A mark as the text names it: on keyframe `#keyframe`, the `number`th of its marks in drawing
/// order (shown as ①②③ only when `numbered`, i.e. the keyframe draws two or more).
#[derive(Clone, Debug)]
pub struct MarkEntry<'a> {
    pub mark: &'a Mark,
    /// 1-based keyframe index, when a keyframe draws the mark.
    pub keyframe: Option<usize>,
    pub number: usize,
    pub numbered: bool,
}

impl MarkEntry<'_> {
    /// `#2 ✎② 0:09 marked (2 strokes) · m2`
    pub fn line(&self) -> String {
        let frame = self.keyframe.map_or("#-".to_owned(), |k| format!("#{k}"));
        let num = self.numbered.then_some(self.number);
        format!("{frame} {} · {}", mark_label(self.mark, num), self.mark.id)
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "keyframe": self.keyframe,
            "number": self.number,
            "numbered": self.numbered,
            "id": self.mark.id,
            "t0": self.mark.t0,
            "t1": self.mark.t1,
            "time": format_clock(self.mark.t0),
            "shape": self.mark.shape,
            "on": self.mark.on,
            "strokes": self.mark.strokes.len(),
            "text": mark_text(self.mark),
            "frame": self.mark.frame,
        })
    }
}

/// Every mark, in time order, numbered the way the summary numbers them.
pub fn mark_entries(bundle: &ScrawlBundle) -> Vec<MarkEntry<'_>> {
    let mut entries: Vec<MarkEntry> = bundle
        .marks
        .iter()
        .map(|mark| {
            let frame = bundle.keyframes.iter().find(|k| mark_on_keyframe(mark, k));
            let shown = frame
                .map(|k| marks_on(k, &bundle.marks))
                .unwrap_or_default();
            let pos = shown.iter().position(|m| std::ptr::eq(*m, mark));
            MarkEntry {
                mark,
                keyframe: frame.map(|k| k.index),
                number: pos.map_or(1, |n| n + 1),
                numbered: pos.is_some() && shown.len() >= 2,
            }
        })
        .collect();
    entries.sort_by_key(|e| e.mark.t0);
    entries
}

/// The mark `query` names: its id (`m2`), or its number as the text shows it (`2` or `②`), on
/// keyframe `keyframe` when given. A number on several keyframes needs the keyframe; numbers
/// shown as ①②③ win over single, unnumbered marks.
pub fn find_mark<'a>(
    bundle: &'a ScrawlBundle,
    query: &str,
    keyframe: Option<usize>,
) -> Result<MarkEntry<'a>, String> {
    let entries = mark_entries(bundle);
    let query = query.trim();
    if let Some(entry) = entries.iter().find(|e| e.mark.id == query) {
        return Ok(entry.clone());
    }
    let number = query
        .parse::<usize>()
        .ok()
        .or_else(|| (1..=20).find(|&n| circled(n) == query))
        .ok_or_else(|| format!("“{query}” is neither a mark number nor a mark id"))?;
    let on_frame: Vec<&MarkEntry> = entries
        .iter()
        .filter(|e| e.number == number && keyframe.is_none_or(|k| e.keyframe == Some(k)))
        .collect();
    let shown: Vec<&MarkEntry> = on_frame.iter().copied().filter(|e| e.numbered).collect();
    let candidates = if shown.is_empty() { on_frame } else { shown };
    match candidates.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(match keyframe {
            Some(k) => format!("Keyframe #{k} has no mark {}", circled(number)),
            None => format!("No mark {} in this bundle", circled(number)),
        }),
        many => Err(format!(
            "Mark {} is on several keyframes ({}); add --frame <N> or use the mark id",
            circled(number),
            many.iter().map(|e| e.line()).collect::<Vec<_>>().join("; ")
        )),
    }
}

fn normalized(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Recovers which timed words each keyframe's `said` was built from (`keyframes::align_transcript`
/// gives every frame a run of the time-sorted words). `None` when the transcript has no word
/// times or the text doesn't line up, and the speech is then shown whole.
fn frame_words(bundle: &ScrawlBundle) -> Option<Vec<Vec<&WordTiming>>> {
    let mut words: Vec<&WordTiming> = bundle
        .transcript
        .words
        .iter()
        .filter(|w| !w.text.trim().is_empty())
        .collect();
    if words.is_empty() {
        return None;
    }
    words.sort_by_key(|w| w.t0);
    let mut next = 0;
    let mut out = Vec::with_capacity(bundle.keyframes.len());
    for frame in &bundle.keyframes {
        let target = normalized(frame.said.as_deref().unwrap_or(""));
        let mut acc = String::new();
        let mut taken = Vec::new();
        while acc.len() < target.len() && next < words.len() {
            acc.push_str(&normalized(&words[next].text));
            taken.push(words[next]);
            next += 1;
        }
        if acc != target {
            return None;
        }
        out.push(taken);
    }
    Some(out)
}

/// When each keyframe's stretch of the recording starts: its first word, first mark or own moment,
/// whichever is earliest, and never before the previous one. The first frame owns the lead-in.
fn frame_starts(bundle: &ScrawlBundle, words: Option<&[Vec<&WordTiming>]>) -> Vec<u64> {
    let mut starts: Vec<u64> = Vec::with_capacity(bundle.keyframes.len());
    for (i, frame) in bundle.keyframes.iter().enumerate() {
        let first_word = words.and_then(|w| w[i].first().map(|w| w.t0));
        let first_mark = marks_on(frame, &bundle.marks).first().map(|m| m.t0);
        // A click keyframe is cut a moment after its click.
        let click = (frame.reason == KeyframeReason::Click)
            .then(|| {
                bundle
                    .events
                    .iter()
                    .filter(|e| {
                        matches!(
                            e,
                            InteractionEvent::Click { .. } | InteractionEvent::Drag { .. }
                        )
                    })
                    .map(InteractionEvent::timestamp_ms)
                    .filter(|&t| t <= frame.t + 200 && frame.t.saturating_sub(t) <= CLICK_ANCHOR_MS)
                    .max()
            })
            .flatten();
        let start = [Some(frame.t), first_word, first_mark, click]
            .into_iter()
            .flatten()
            .min()
            .unwrap_or(frame.t);
        let start = if i == 0 { 0 } else { start.max(starts[i - 1]) };
        starts.push(start);
    }
    starts
}

/// Everything besides marks that the summary shows inline, in time order, with repeats of one
/// thing (a shortcut, a click on one control) folded into `×N` and a run of drags into one line.
fn events(bundle: &ScrawlBundle) -> Vec<(u64, String)> {
    let settled = settled_events(&bundle.events);
    // On a Mac capture these come from the dev page (`crate::page`), not an app.
    let page_scrubs: Vec<u64> = settled
        .iter()
        .filter_map(|e| match e {
            InteractionEvent::AppEvent { t, label }
                if bundle.device.is_none() && label.contains(" scrubbed to ") =>
            {
                Some(*t)
            }
            _ => None,
        })
        .collect();
    let mut out: Vec<(u64, String)> = Vec::new();
    let mut drags: Option<DragRun> = None;
    for event in &settled {
        if let InteractionEvent::Drag { t, t1, .. } = event {
            let reported = page_scrubs
                .iter()
                .any(|&s| s + SCRUB_MS >= *t && s <= t1 + SCRUB_MS);
            if let Some(drag) = drag_text(event).filter(|_| !reported) {
                match &mut drags {
                    Some(run) if run.joins(&drag, *t) => run.add(drag, *t1),
                    _ => {
                        out.extend(drags.take().map(DragRun::line));
                        drags = Some(DragRun::new(drag, *t, *t1));
                    }
                }
            }
            continue;
        }
        let line = match event {
            InteractionEvent::AppEvent { t, label } if bundle.device.is_none() => {
                Some((*t, label.clone()))
            }
            // Window titles stay in scrawl.json; the summary names apps only.
            InteractionEvent::AppChange { t, app, .. } => Some((*t, format!("switched to {app}"))),
            other => event_text(other).map(|text| (other.timestamp_ms(), text)),
        };
        out.extend(line);
    }
    out.extend(drags.map(DragRun::line));
    out.extend(window_changes(&bundle.windows));
    out.extend(bundle.context_changes.iter().map(|change| {
        let text = match &change.value {
            Some(value) => format!("{} → {}", one_line(&change.key), one_line(value)),
            None => format!("{} cleared", one_line(&change.key)),
        };
        (change.t, text)
    }));
    out.sort_by_key(|(t, _)| *t);
    fold_repeats(out)
}

fn fold_repeats(items: Vec<(u64, String)>) -> Vec<(u64, String)> {
    let mut out: Vec<(u64, String, u64, usize)> = Vec::new();
    for (t, text) in items {
        match out.last_mut() {
            Some((_, prev, last, count)) if *prev == text && t - *last < REPEAT_MS => {
                *last = t;
                *count += 1;
            }
            _ => out.push((t, text, t, 1)),
        }
    }
    out.into_iter()
        .map(|(t, text, _, count)| {
            if count > 1 {
                (t, format!("{text} ×{count}"))
            } else {
                (t, text)
            }
        })
        .collect()
}

/// A drag worth a line: one that names what it moved, or an unnamed one that went far.
#[derive(Clone, Debug, PartialEq)]
struct Drag {
    verb: &'static str,
    name: Option<String>,
    direction: String,
}

fn drag_text(event: &InteractionEvent) -> Option<Drag> {
    let InteractionEvent::Drag {
        x0,
        y0,
        x1,
        y1,
        label,
        button,
        ..
    } = event
    else {
        return None;
    };
    let label = label.as_deref().map(str::trim).filter(|l| !l.is_empty());
    if let Some(rest) = label.and_then(|l| l.strip_prefix("long press")) {
        let name = rest.trim_start_matches(':').trim();
        return Some(Drag {
            verb: "long-pressed",
            name: (!name.is_empty()).then(|| name.to_owned()),
            direction: String::new(),
        });
    }
    let touch = button.as_deref() == Some("touch");
    let (dx, dy) = (x1 - x0, y1 - y0);
    match label {
        Some(name) => {
            let lower = name.to_lowercase();
            let scrub = ["slider", "scroll bar", "scrubber", "timeline", "progress"]
                .iter()
                .any(|word| lower.contains(word));
            Some(Drag {
                verb: if scrub {
                    "scrubbed"
                } else if touch {
                    "swiped"
                } else {
                    "dragged"
                },
                name: Some(name.to_owned()),
                direction: direction(dx, dy, 0.04),
            })
        }
        None if dx.hypot(dy) >= MIN_DRAG => Some(Drag {
            verb: if touch { "swiped" } else { "dragged" },
            name: None,
            direction: direction(dx, dy, 0.04),
        }),
        None => None,
    }
}

/// Drags of one thing in quick succession, as one line.
struct DragRun {
    t: u64,
    end: u64,
    drags: Vec<Drag>,
}

impl DragRun {
    fn new(drag: Drag, t: u64, end: u64) -> Self {
        Self {
            t,
            end,
            drags: vec![drag],
        }
    }

    fn joins(&self, drag: &Drag, t: u64) -> bool {
        let first = &self.drags[0];
        first.verb == drag.verb && first.name == drag.name && t <= self.end + REPEAT_MS
    }

    fn add(&mut self, drag: Drag, end: u64) {
        self.end = self.end.max(end);
        self.drags.push(drag);
    }

    fn line(self) -> (u64, String) {
        let first = &self.drags[0];
        let mut text = first.verb.to_owned();
        if let Some(name) = &first.name {
            text.push(' ');
            text.push_str(&quote_label(name));
        }
        let count = self.drags.len();
        let same_way = self.drags.iter().all(|d| d.direction == first.direction);
        // A scrub's direction and count say nothing; the dev page reports where it landed.
        if first.verb != "scrubbed" && first.verb != "long-pressed" {
            if same_way && first.direction != "a little" {
                text.push(' ');
                text.push_str(&first.direction);
            } else if !same_way {
                text.push_str(" back and forth");
            }
        }
        if count > 1 && first.verb != "scrubbed" {
            text.push_str(&format!(" ×{count}"));
        }
        (self.t, text)
    }
}

/// A click, scroll, point or key press without coordinates; `None` for a point nothing names.
fn event_text(event: &InteractionEvent) -> Option<String> {
    match event {
        InteractionEvent::Click {
            label,
            button,
            role,
            ..
        } => {
            let mut text = if button.as_deref() == Some("touch") {
                "tapped"
            } else {
                "clicked"
            }
            .to_owned();
            if let Some(role) = role.as_deref().filter(|r| !r.is_empty()) {
                text.push(' ');
                text.push_str(role);
            }
            if let Some(label) = label.as_deref().filter(|l| !l.is_empty()) {
                text.push(' ');
                text.push_str(&quote_label(label));
            }
            Some(text)
        }
        InteractionEvent::Point { label, .. } => label
            .as_deref()
            .filter(|l| !l.is_empty())
            .map(|l| format!("pointed at {}", quote_label(l))),
        other => Some(describe_event(other)),
    }
}

/// What a mark is: its shape, what it was drawn on when something names that, and its stroke
/// count when more than one.
fn mark_text(mark: &Mark) -> String {
    let mut text = match mark.shape {
        Shape::Circle => "circled".to_owned(),
        Shape::Underline => "underlined".to_owned(),
        Shape::Line => "drew a line".to_owned(),
        Shape::Arrow => "drew an arrow".to_owned(),
        Shape::Pin => format!("pin {}", mark.pin.unwrap_or(1)),
        Shape::Tap => "tapped".to_owned(),
        Shape::Mark => "marked".to_owned(),
        Shape::Flag => "flagged this moment".to_owned(),
    };
    if let Some(on) = mark.on.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        text.push_str(match mark.shape {
            Shape::Arrow => " pointing to ",
            Shape::Pin | Shape::Line => " on ",
            _ => " ",
        });
        text.push_str(&quote_label(on));
    }
    if mark.strokes.len() > 1 {
        text.push_str(&format!(" ({} strokes)", mark.strokes.len()));
    }
    text
}

/// A keyframe's own description without its `(x 0.50, y 0.70)` positions.
fn without_positions(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(start) = rest.find(" (x ") {
        out.push_str(&rest[..start]);
        match rest[start..].find(')') {
            Some(end) => rest = &rest[start + end + 1..],
            None => {
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

fn one_line(raw: &str) -> String {
    let compact = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() <= 60 {
        compact
    } else {
        let cut: String = compact.chars().take(59).collect();
        format!("{}…", cut.trim_end())
    }
}

/// Ordinary app windows appearing and disappearing between window-log snapshots, by app name.
fn window_changes(snapshots: &[WindowSnapshot]) -> Vec<(u64, String)> {
    let normal = |snapshot: &WindowSnapshot| -> Vec<(u32, String)> {
        snapshot
            .windows
            .iter()
            .filter(|w| w.layer == 0 && w.regular && w.app != "Scrawl")
            .map(|w| (w.id, w.app.clone()))
            .collect()
    };
    let mut out = Vec::new();
    for pair in snapshots.windows(2) {
        let (before, after) = (normal(&pair[0]), normal(&pair[1]));
        let opened: Vec<&String> = after
            .iter()
            .filter(|(id, _)| !before.iter().any(|(b, _)| b == id))
            .map(|(_, app)| app)
            .collect();
        let closed: Vec<&String> = before
            .iter()
            .filter(|(id, _)| !after.iter().any(|(a, _)| a == id))
            .map(|(_, app)| app)
            .collect();
        let t = pair[1].t;
        if opened.len() + closed.len() > WINDOW_BURST {
            out.push((
                t,
                format!(
                    "on-screen windows changed ({} opened, {} closed)",
                    opened.len(),
                    closed.len()
                ),
            ));
            continue;
        }
        for app in closed {
            out.push((t, format!("{app} window closed")));
        }
        for app in opened {
            out.push((t, format!("{app} window opened")));
        }
    }
    out
}

/// Joins recognizer words the way `keyframes::align_transcript` does.
fn join_words(words: &[&WordTiming]) -> String {
    let mut text = String::new();
    for word in words {
        let unit = word.text.trim();
        let attached = unit.starts_with([',', '.', '!', '?', ';', ':', '’', '\'', ')', ']', '”']);
        let open = text.ends_with(['(', '[', '“']);
        if !text.is_empty() && !attached && !open {
            text.push(' ');
        }
        text.push_str(unit);
    }
    text
}

fn quoted(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        String::new()
    } else {
        format!("“{text}”")
    }
}

fn mark_label(mark: &Mark, number: Option<usize>) -> String {
    let n = number.map(circled).unwrap_or_default();
    format!("✎{n} {} {}", format_clock(mark.t0), mark_text(mark))
}

fn label(item: &Timed) -> String {
    match &item.item {
        Item::Mark { mark, number } => mark_label(mark, *number),
        Item::Event { text } => format!("· {} {text}", format_clock(item.t)),
    }
}

/// Indexes of the words that start a clause: after clause punctuation (a comma once the clause
/// has three words), or after a pause of `CLAUSE_GAP_MS`, as `keyframes` breaks clauses.
fn clause_starts(words: &[&WordTiming]) -> Vec<usize> {
    let mut starts = Vec::new();
    let mut clause_words = 0;
    for (i, word) in words.iter().enumerate() {
        let new_clause = i == 0 || {
            let prev = words[i - 1];
            let text = prev.text.trim_end_matches(['"', '\'', '”', '’', ')', ']']);
            text.ends_with(['.', '!', '?', ';', ':'])
                || (clause_words >= 3 && text.ends_with(','))
                || word.t0.saturating_sub(prev.t1) >= CLAUSE_GAP_MS
        };
        if new_clause {
            starts.push(i);
            clause_words = 0;
        }
        clause_words += 1;
    }
    starts
}

/// Where the words split for an item. Speech isn't cut mid-clause when it can be helped: an item
/// moves back to the start of the clause in progress if that began within `CLAUSE_SNAP_MS`,
/// else forward to the clause's end if that comes within `CLAUSE_SNAP_AFTER_MS`.
fn split_point(t: u64, words: &[&WordTiming], clauses: &[usize]) -> u64 {
    let Some(inside) = words.iter().position(|w| w.t0 >= t) else {
        return t;
    };
    if clauses.contains(&inside) {
        return t.min(words[inside].t0);
    }
    let start = clauses
        .iter()
        .rev()
        .find(|&&c| c < inside)
        .copied()
        .unwrap_or(0);
    if t.saturating_sub(words[start].t0) <= CLAUSE_SNAP_MS {
        return words[start].t0;
    }
    // Forward to just after the clause's last word, if it ends soon.
    let end = clauses
        .iter()
        .find(|&&c| c > inside)
        .copied()
        .unwrap_or(words.len());
    if words[end - 1].t1.saturating_sub(t) <= CLAUSE_SNAP_AFTER_MS {
        words.get(end).map_or(words[end - 1].t1 + 1, |w| w.t0)
    } else {
        t
    }
}

/// Speech before the first item goes on the bullet; each mark, or run of events with no speech
/// between them, starts a line followed by the words said until the next one.
fn interleave(words: &[&WordTiming], items: &[Timed]) -> FrameText {
    let clauses = clause_starts(words);
    let mut splits: Vec<u64> = items
        .iter()
        .map(|item| split_point(item.t, words, &clauses))
        .collect();
    for i in 1..splits.len() {
        splits[i] = splits[i].max(splits[i - 1]);
    }
    let said_between = |from: Option<u64>, to: Option<u64>| -> String {
        let run: Vec<&WordTiming> = words
            .iter()
            .filter(|w| from.is_none_or(|f| w.t0 >= f) && to.is_none_or(|t| w.t0 < t))
            .copied()
            .collect();
        quoted(&join_words(&run))
    };
    let head = said_between(None, splits.first().copied());
    let mut lines: Vec<String> = Vec::new();
    let mut open: Option<String> = None;
    for (i, item) in items.iter().enumerate() {
        let is_mark = matches!(item.item, Item::Mark { .. });
        let line = match open.take() {
            Some(line) if !is_mark => format!("{line} {}", label(item)),
            previous => {
                lines.extend(previous);
                label(item)
            }
        };
        let said = said_between(Some(splits[i]), splits.get(i + 1).copied());
        if said.is_empty() && !is_mark {
            open = Some(line);
        } else if said.is_empty() {
            lines.push(line);
        } else {
            lines.push(format!("{line} {said}"));
        }
    }
    lines.extend(open);
    FrameText { head, lines }
}

/// Without word times the speech stays whole on the bullet and the items follow it.
fn interleave_unsplit(said: Option<&str>, items: &[Timed]) -> FrameText {
    FrameText {
        head: quoted(said.unwrap_or("")),
        lines: items.iter().map(label).collect(),
    }
}

/// What changed on a scene-change keyframe, when it can be named: for now only a change of the
/// whole screen (autocrop's change boxes). Smaller regions have no names yet and the image shows
/// them.
fn changed_regions(bundle: &ScrawlBundle, index: usize) -> Option<String> {
    let frame = bundle.keyframes.get(index)?;
    if frame.reason != KeyframeReason::SceneChange {
        return None;
    }
    let boxes = bundle.autocrop.as_ref()?.changes.get(index)?;
    let area = |b: &[f32; 4]| (b[2] - b[0]) * (b[3] - b[1]);
    merge_boxes(boxes)
        .iter()
        .any(|b| area(b) >= 0.9)
        .then(|| "the whole screen".into())
}

fn merge_boxes(boxes: &[[f32; 4]]) -> Vec<[f32; 4]> {
    const TOUCH: f32 = 0.02;
    let mut out: Vec<[f32; 4]> = boxes.to_vec();
    loop {
        let mut joined = false;
        'outer: for i in 0..out.len() {
            for j in i + 1..out.len() {
                let (a, b) = (out[i], out[j]);
                if a[0] - TOUCH <= b[2]
                    && b[0] - TOUCH <= a[2]
                    && a[1] - TOUCH <= b[3]
                    && b[1] - TOUCH <= a[3]
                {
                    out[i] = [
                        a[0].min(b[0]),
                        a[1].min(b[1]),
                        a[2].max(b[2]),
                        a[3].max(b[3]),
                    ];
                    out.remove(j);
                    joined = true;
                    break 'outer;
                }
            }
        }
        if !joined {
            return out;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::WindowEntry;

    fn stroke(t0: u64, x: f32, y: f32) -> Stroke {
        let p = |v: f32| (v * SCALE as f32) as i32;
        Stroke {
            t0,
            t1: t0 + 400,
            fw: 1000,
            fh: 1000,
            on: None,
            pin: None,
            laser: false,
            pts: vec![p(x), p(y), 0, p(x + 0.05), p(y), 200, p(x + 0.1), p(y), 400],
        }
    }

    fn words(list: &[(u64, &str)]) -> Vec<WordTiming> {
        list.iter()
            .map(|&(t0, text)| WordTiming {
                t0,
                t1: t0 + 250,
                text: text.into(),
            })
            .collect()
    }

    fn keyframe(index: usize, t: u64, reason: KeyframeReason, said: &str) -> Keyframe {
        Keyframe {
            index,
            t,
            reason,
            file: format!("frames/frame-{index:02}.png"),
            event: "scene changed".into(),
            said: (!said.is_empty()).then(|| said.into()),
            mark_id: None,
            cursor: None,
        }
    }

    fn bundle(words: Vec<WordTiming>, keyframes: Vec<Keyframe>) -> ScrawlBundle {
        ScrawlBundle {
            version: 1,
            id: "t".into(),
            created_at_ms: 0,
            duration_ms: 20_000,
            target: TargetInfo {
                mode: CaptureMode::Display,
                app: None,
                bundle_id: None,
                window_title: None,
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
                source: "test".into(),
                words,
                ..Default::default()
            },
            note: None,
            strokes: Vec::new(),
            marks: Vec::new(),
            events: Vec::new(),
            keyframes,
            contact_sheet: "contact-sheet.png".into(),
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
        }
    }

    #[test]
    fn marks_sharing_a_frame_are_numbered_and_take_the_speech_drawn_over() {
        let said =
            "These should stay put. Maybe this hover part can move. And drop these dividers.";
        let mut b = bundle(
            words(&[
                (1_000, "These"),
                (1_300, "should"),
                (1_600, "stay"),
                (1_900, "put."),
                (4_000, "Maybe"),
                (4_300, "this"),
                (4_600, "hover"),
                (4_900, "part"),
                (5_200, "can"),
                (5_500, "move."),
                (8_000, "And"),
                (8_300, "drop"),
                (8_600, "these"),
                (8_900, "dividers."),
            ]),
            vec![keyframe(1, 9_500, KeyframeReason::Mark, said)],
        );
        // The second mark starts mid-clause ("hover"), so its speech starts with "Maybe".
        b.strokes = vec![
            stroke(1_200, 0.1, 0.2),
            stroke(4_700, 0.4, 0.5),
            stroke(8_400, 0.6, 0.1),
        ];
        b.marks = crate::sketch::marks_of(&b.strokes);
        for mark in &mut b.marks {
            mark.frame = Some("frames/frame-01.png".into());
        }
        b.keyframes[0].mark_id = Some(b.marks[0].id.clone());
        let text = &keyframe_texts(&b)[0];
        assert_eq!(text.head, "");
        assert_eq!(text.lines.len(), 3, "{text:?}");
        assert!(text.lines[0].starts_with("✎① 0:01 "));
        assert!(text.lines[0].ends_with("“These should stay put.”"));
        assert!(text.lines[1].starts_with("✎② 0:04 "));
        assert!(text.lines[1].ends_with("“Maybe this hover part can move.”"));
        assert!(text.lines[2].starts_with("✎③ 0:08 "));
        assert!(text.lines[2].ends_with("“And drop these dividers.”"));
    }

    #[test]
    fn marks_are_found_by_their_number_in_the_text_or_their_id() {
        let mut b = bundle(
            Vec::new(),
            vec![
                keyframe(1, 3_000, KeyframeReason::Mark, ""),
                keyframe(2, 9_000, KeyframeReason::Mark, ""),
                keyframe(3, 15_000, KeyframeReason::Mark, ""),
            ],
        );
        b.strokes = vec![
            stroke(1_000, 0.1, 0.2),
            stroke(3_000, 0.4, 0.5),
            stroke(7_000, 0.6, 0.1),
            stroke(9_000, 0.2, 0.8),
            stroke(14_000, 0.5, 0.5),
        ];
        b.marks = crate::sketch::marks_of(&b.strokes);
        assert_eq!(b.marks.len(), 5);
        for (mark, frame) in b.marks.iter_mut().zip([1, 1, 2, 2, 3]) {
            mark.frame = Some(format!("frames/frame-{frame:02}.png"));
        }
        for (k, first) in [(0, 0), (1, 2), (2, 4)] {
            b.keyframes[k].mark_id = Some(b.marks[first].id.clone());
        }
        let found = |q: &str, k: Option<usize>| find_mark(&b, q, k).map(|e| e.mark.t0);
        // ② is on #1 and #2: ambiguous without a keyframe.
        assert!(found("2", None).unwrap_err().contains("--frame"));
        assert_eq!(found("2", Some(2)), Ok(9_000));
        assert_eq!(found("②", Some(1)), Ok(3_000));
        // #3's single mark is unnumbered; --mark 1 --frame 3 still reaches it.
        assert_eq!(found("1", Some(3)), Ok(14_000));
        assert!(found("3", None).is_err());
        let id = b.marks[3].id.clone();
        assert_eq!(found(&id, None), Ok(9_000));
        let entry = find_mark(&b, "2", Some(2)).unwrap();
        assert!(entry.line().starts_with("#2 ✎② 0:09 "), "{}", entry.line());
        assert_eq!(entry.json()["keyframe"], 2);
    }

    #[test]
    fn shortcuts_drags_and_windows_appear_inline_without_titles_or_jitter() {
        let said = "Okay, I hit command shift B. The whole window closes.";
        let mut b = bundle(
            words(&[
                (500, "Okay,"),
                (900, "I"),
                (1_100, "hit"),
                (1_300, "command"),
                (1_600, "shift"),
                (1_900, "B."),
                (5_000, "The"),
                (5_200, "whole"),
                (5_400, "window"),
                (5_600, "closes."),
            ]),
            vec![keyframe(1, 1_000, KeyframeReason::Click, said)],
        );
        let key = |t| InteractionEvent::Key {
            t,
            shortcut: "⇧⌘B".into(),
        };
        let drag = |t, x1: f32| InteractionEvent::Drag {
            t,
            t1: t + 300,
            x0: 0.5,
            y0: 0.5,
            x1,
            y1: 0.5,
            label: None,
            button: None,
            path: Vec::new(),
        };
        b.events = vec![
            key(2_000),
            key(2_400),
            key(2_900),
            drag(3_000, 0.503),
            drag(3_200, 0.56),
            drag(3_500, 0.8),
            drag(4_000, 0.9),
        ];
        let window = |id, title: &str| WindowEntry {
            id,
            app: "OpenCode".into(),
            bundle_id: None,
            regular: true,
            title: title.into(),
            bounds: [0, 0, 100, 100],
            layer: 0,
            frontmost: true,
        };
        let overlay = WindowEntry {
            layer: 25,
            ..window(9, "Hex")
        };
        b.windows = vec![
            WindowSnapshot {
                t: 0,
                windows: vec![window(1, "Secret plans"), overlay],
            },
            WindowSnapshot {
                t: 4_800,
                windows: Vec::new(),
            },
        ];
        let text = &keyframe_texts(&b)[0];
        let all = format!("{}\n{}", text.head, text.lines.join("\n"));
        assert!(all.contains("pressed ⇧⌘B ×3"), "{all}");
        assert!(all.contains("0:03 dragged right ×2"), "{all}");
        assert_eq!(
            all.matches("dragged").count(),
            1,
            "short drags are dropped and a run is one line: {all}"
        );
        assert!(!all.contains("x 0."), "no coordinates: {all}");
        assert!(
            all.contains("· 0:04 OpenCode window closed “The whole window closes.”"),
            "{all}"
        );
        assert!(!all.contains("Secret") && !all.contains("Hex"), "{all}");
    }

    #[test]
    fn scene_changes_say_where_the_screen_changed() {
        let mut b = bundle(
            Vec::new(),
            vec![
                keyframe(1, 1_000, KeyframeReason::SceneChange, ""),
                keyframe(2, 3_000, KeyframeReason::SceneChange, ""),
                keyframe(3, 5_000, KeyframeReason::Click, ""),
            ],
        );
        b.autocrop = Some(AutocropInfo {
            changes: vec![
                vec![[0.0, 0.0, 1.0, 0.95]],
                vec![
                    [0.70, 0.02, 0.80, 0.10],
                    [0.80, 0.02, 0.96, 0.12],
                    [0.10, 0.50, 0.12, 0.52],
                ],
                vec![[0.0, 0.0, 0.5, 0.5]],
            ],
            ..Default::default()
        });
        let texts = keyframe_texts(&b);
        assert_eq!(texts[0].head, "[changed: the whole screen]");
        assert_eq!(
            texts[1].head, "scene changed",
            "unnamed regions aren't listed"
        );
        assert_eq!(
            texts[2].head, "scene changed",
            "clicks don't describe changes"
        );
    }

    #[test]
    fn places_appear_only_with_a_name() {
        let mut b = bundle(
            Vec::new(),
            vec![keyframe(1, 9_000, KeyframeReason::Mark, "")],
        );
        b.strokes = vec![
            Stroke {
                on: Some("button “Window”".into()),
                ..stroke(1_000, 0.1, 0.2)
            },
            stroke(3_000, 0.4, 0.5),
            stroke(3_100, 0.4, 0.52),
        ];
        b.marks = crate::sketch::marks_of(&b.strokes);
        for mark in &mut b.marks {
            mark.frame = Some("frames/frame-01.png".into());
        }
        let slider = |t, x1| InteractionEvent::Drag {
            t,
            t1: t + 300,
            x0: 0.5,
            y0: 0.9,
            x1,
            y1: 0.9,
            label: Some("slider “Playback”".into()),
            button: None,
            path: Vec::new(),
        };
        b.events = vec![
            InteractionEvent::Click {
                t: 5_000,
                x: 0.3,
                y: 0.3,
                button: None,
                label: Some("“Area”".into()),
                role: Some("button".into()),
            },
            slider(6_000, 0.6),
            slider(6_800, 0.4),
            slider(7_500, 0.45),
            InteractionEvent::Point {
                t: 8_500,
                x: 0.2,
                y: 0.2,
                label: None,
            },
        ];
        let text = &keyframe_texts(&b)[0];
        let marks: Vec<&str> = text.lines.iter().map(String::as_str).collect();
        assert_eq!(
            marks,
            [
                "✎① 0:01 underlined button “Window”",
                "✎② 0:03 underlined (2 strokes)",
                "· 0:05 clicked button “Area”",
                "· 0:06 scrubbed slider “Playback”",
            ],
            "{text:?}"
        );
    }

    #[test]
    fn speech_without_word_times_stays_whole() {
        let mut b = bundle(
            Vec::new(),
            vec![keyframe(
                1,
                1_000,
                KeyframeReason::Click,
                "Make it smaller.",
            )],
        );
        b.events = vec![InteractionEvent::Key {
            t: 600,
            shortcut: "⌘K".into(),
        }];
        let text = &keyframe_texts(&b)[0];
        assert_eq!(text.head, "“Make it smaller.”");
        assert_eq!(text.lines, vec!["· 0:00 pressed ⌘K".to_owned()]);
    }

    #[test]
    fn the_summary_uses_these_lines_only_with_the_experiment() {
        let mut b = bundle(
            words(&[(500, "Look"), (800, "here.")]),
            vec![keyframe(1, 1_000, KeyframeReason::Click, "Look here.")],
        );
        b.events = vec![InteractionEvent::Key {
            t: 400,
            shortcut: "⌘K".into(),
        }];
        let dir = std::path::Path::new("/b");
        let off = crate::sketch::summary_markdown_with(&b, dir, false);
        let on = crate::sketch::summary_markdown_with(&b, dir, true);
        assert!(!off.contains("⌘K"), "{off}");
        assert!(
            on.contains("- #1 (0:01)\n  · 0:00 pressed ⌘K “Look here.”\n"),
            "{on}"
        );
    }

    #[test]
    fn repeated_clicks_fold_and_a_click_keyframe_owns_its_click() {
        let click = |t, x| InteractionEvent::Click {
            t,
            x,
            y: 0.5,
            button: None,
            label: None,
            role: None,
        };
        let mut b = bundle(
            Vec::new(),
            vec![
                keyframe(1, 500, KeyframeReason::SceneChange, ""),
                keyframe(2, 9_300, KeyframeReason::Click, ""),
            ],
        );
        b.events = vec![
            click(1_000, 0.63),
            click(2_500, 0.63),
            click(4_000, 0.63),
            click(9_000, 0.2),
        ];
        let texts = keyframe_texts(&b);
        assert_eq!(texts[0].lines, vec!["· 0:01 clicked ×3".to_owned()]);
        assert_eq!(texts[1].lines, vec!["· 0:09 clicked".to_owned()]);
        assert_eq!(texts[1].head, "");
    }
}
