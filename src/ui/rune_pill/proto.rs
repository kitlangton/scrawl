//! Lab-only designs for the capsule after a recording, drawn beside today's in the pill lab
//! (`scrawl lab pill grid --rune`) so they compare at real size over the same backdrops. They
//! are pictures: no routing, no `Pres`, nothing the app can reach. Each borrows the capsule's
//! body, rim, type, colours and spacing (`style()`), so lab knobs move them with the real one.
//!
//! A design is a few frames (`Frame`) and the times it moves between them (`steps`), so a
//! looping cell plays its whole flow: the key press, the list dropping, the send.

use rune::{Color, El, LineCap, Path, Shape, Spring, TextStyle, Transition, canvas, col, el, row};

use super::style::style;
use super::surface::{DONE_TEXT_X, LEAD_TEXT_X};
use super::{
    Meter, PANEL_H, PANEL_W, TEXT, accent, check, dot, green, icon, label, medium, rgb3, rim, semibold, shell_body,
    srgb, text_color, timer_style, veil, white,
};
use crate::native::pill::model::{CHIP_H, CX, ElemId, ISLAND_H, ISLAND_INSET, TOP};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Proto {
    /// f, sure where it goes: "Sent to CONCLAVE", esc undoes it for a few seconds, ⌥⇧V sends
    /// it elsewhere.
    FSure,
    /// f, unsure: "Copied", and the list opens by itself with the suggestions on top.
    FUnsure,
    /// f, nothing fits: "Copied"; ⌥⇧V opens the list of recent sessions.
    FNoMatch,
    /// g: the recording shows where it'll go and ⌥⇧1–3 switch it; afterwards only "Sent · Undo".
    ChooseFirst,
    /// h: no routing UI at all: sent when Scrawl is sure (a dev site), else copied.
    NoRouting,
    /// b: the destination named in the lead, one chip to send it elsewhere instead.
    SentNamed,
    /// c: up to three destinations, each with the key that picks it while the pill is up.
    Numbered,
    /// d: a pinned destination's own shortcut was used, so it went straight there.
    PinnedShortcut,
    /// e: the recording says where it will go.
    RecordingDest,
}

/// One picture: the capsule's items, and what hangs below it.
#[derive(Clone, Debug)]
pub(super) struct Frame {
    items: Vec<Item>,
    list: Option<List>,
    /// g's ⌥⇧ strip, with this destination picked.
    strip: Option<usize>,
}

impl Frame {
    fn pill(items: Vec<Item>) -> Self {
        Self { items, list: None, strip: None }
    }

    fn list(mut self, list: List) -> Self {
        self.list = Some(list);
        self
    }

    fn strip(mut self, picked: usize) -> Self {
        self.strip = Some(picked);
        self
    }
}

/// f's send-to list under the open pill (whose field holds what's typed): suggested sessions
/// with why, then recent ones, filtered by what's typed.
#[derive(Clone, Copy, Debug)]
struct List {
    filter: &'static str,
    selected: usize,
    suggest: Suggest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Suggest {
    /// Nothing fits: recent sessions only.
    None,
    Some,
    /// It was already sent to the first suggestion.
    Sent,
}

const SUGGESTED: [(&str, &str); 2] = [("CONCLAVE", "dev site · localhost:5173"), ("HEX", "last sent")];
const RECENT: [(&str, &str); 3] = [
    ("Scrawl · Rune pill lab follow-ups", "2 min ago"),
    ("OpenCode Desktop · Problems board rewrite", "1 h ago"),
    ("HOME", "yesterday"),
];
const STRIP: [&str; 3] = ["CONCLAVE", "HEX", "HOME"];

impl List {
    fn sections(self) -> Vec<(&'static str, Vec<(&'static str, &'static str)>)> {
        let filter = self.filter.to_lowercase();
        let keep = |rows: &[(&'static str, &'static str)]| -> Vec<(&'static str, &'static str)> {
            rows.iter().copied().filter(|(name, _)| name.to_lowercase().contains(&filter)).collect()
        };
        let mut suggested = if self.suggest == Suggest::None { Vec::new() } else { keep(&SUGGESTED) };
        if self.suggest == Suggest::Sent
            && let Some(first) = suggested.first_mut()
            && first.0 == SUGGESTED[0].0
        {
            first.1 = "sent";
        }
        [("Suggested", suggested), ("Recent", keep(&RECENT))].into_iter().filter(|(_, rows)| !rows.is_empty()).collect()
    }

    fn height(self) -> f32 {
        let rows: f32 = self.sections().iter().map(|(_, rows)| HEADER_H + rows.len() as f32 * ROW_H).sum();
        2.0 * LIST_PAD + rows + LIST_PAD + 1.0 + FOOTER_H
    }
}

/// The open pill's width (its field takes what the rest leaves), and the list's under it.
const LIST_W: f32 = 360.0;
const FIELD_MIN: f32 = 120.0;
const LIST_PAD: f32 = 6.0;
const LIST_Y: f32 = TOP + ISLAND_H + 6.0;
const HEADER_H: f32 = 24.0;
const ROW_H: f32 = 30.0;
const FOOTER_H: f32 = 28.0;
const STRIP_H: f32 = 30.0;

/// Frames move on one plain spring (the real capsule's are keyed on the kind of change).
const MOVE: Spring = Spring::new(0.32, 0.0);

impl Proto {
    /// f's three bands, then g and h: the designs after Kit's look at b–e.
    pub const NEW: [Proto; 5] = [Self::FSure, Self::FUnsure, Self::FNoMatch, Self::ChooseFirst, Self::NoRouting];
    pub const OLD: [Proto; 4] = [Self::SentNamed, Self::Numbered, Self::PinnedShortcut, Self::RecordingDest];

    /// Its name in the lab's sidebar.
    pub fn name(self) -> &'static str {
        match self {
            Self::FSure => "f · Sure",
            Self::FUnsure => "f · Unsure",
            Self::FNoMatch => "f · Nothing fits",
            Self::ChooseFirst => "g · Choose while recording",
            Self::NoRouting => "h · No routing UI",
            Self::SentNamed => "b · Name it",
            Self::Numbered => "c · Pick by key",
            Self::PinnedShortcut => "d · Pinned shortcut",
            Self::RecordingDest => "e · Destination while recording",
        }
    }

    /// One line on what it shows.
    pub fn about(self) -> &'static str {
        match self {
            Self::FSure => "Scrawl is sure where it goes: Sent to CONCLAVE, esc undoes it for 3 s, ⌥⇧V sends it elsewhere.",
            Self::FUnsure => "Scrawl has a guess but isn't sure: Copied, and the list opens by itself with its suggestions on top.",
            Self::FNoMatch => "Nothing fits: Copied, and ⌥⇧V opens the list of recent sessions.",
            Self::ChooseFirst => "The recording shows where it'll go; hold ⌥⇧ and press 1–3 to switch. Afterwards only Sent · Undo.",
            Self::NoRouting => "Sent when Scrawl is sure (a dev site), else Copied · Paste.",
            Self::SentNamed => "Sent to CONCLAVE, with one chip to send it elsewhere instead.",
            Self::Numbered => "⌥⇧1…3 pick a destination while the pill is up.",
            Self::PinnedShortcut => "A pinned destination's own shortcut sent it straight there.",
            Self::RecordingDest => "The recording says where it will go.",
        }
    }

    /// Its recording dot breathes while it's up.
    pub fn recording(self) -> bool {
        matches!(self, Self::RecordingDest | Self::ChooseFirst)
    }

    pub(super) fn frames(self) -> Vec<Frame> {
        use Item::*;
        let rec = |clock, dest| vec![Rec(clock, dest), Button("Cancel", false), Button("Finish", true)];
        let sent = |to| vec![Done(Lead::Check, to), Chip("Undo", Some("esc")), Close];
        let send = |lead, chip| vec![Done(Lead::Check, lead), Chip(chip, Some("⌥⇧V")), Close];
        let open = |lead, typed| vec![Done(Lead::Check, lead), Field(typed), Close];
        let list = |filter, selected, suggest| List { filter, selected, suggest };
        let sure = |undo| vec![Done(Lead::Check, "Sent to CONCLAVE"), Chip(undo, Some("esc")), Chip("Elsewhere…", Some("⌥⇧V")), Close];
        match self {
            Self::FSure => vec![
                Frame::pill(sure("Undo 3")),
                Frame::pill(sure("Undo 2")),
                Frame::pill(sure("Undo 1")),
                Frame::pill(send("Sent to CONCLAVE", "Elsewhere…")),
                Frame::pill(open("Sent to CONCLAVE", "")).list(list("", 1, Suggest::Sent)),
                Frame::pill(open("Sent to CONCLAVE", "he")).list(list("he", 0, Suggest::Sent)),
                Frame::pill(sent("Sent to HEX")),
            ],
            Self::FUnsure => vec![
                Frame::pill(send("Copied", "Send…")),
                Frame::pill(open("Copied", "")).list(list("", 0, Suggest::Some)),
                Frame::pill(open("Copied", "")).list(list("", 1, Suggest::Some)),
                Frame::pill(sent("Sent to HEX")),
            ],
            Self::FNoMatch => vec![
                Frame::pill(send("Copied", "Send…")),
                Frame::pill(open("Copied", "")).list(list("", 0, Suggest::None)),
                Frame::pill(open("Copied", "ho")).list(list("ho", 0, Suggest::None)),
                Frame::pill(sent("Sent to HOME")),
            ],
            Self::ChooseFirst => vec![
                Frame::pill(rec("0:07", "CONCLAVE")),
                Frame::pill(rec("0:07", "CONCLAVE")).strip(0),
                Frame::pill(rec("0:08", "HEX")).strip(1),
                Frame::pill(rec("0:09", "HEX")),
                Frame::pill(vec![Done(Lead::Check, "Sent to HEX"), Chip("Undo", None), Close]),
            ],
            Self::NoRouting => vec![
                Frame::pill(vec![Done(Lead::Check, "Copied"), Button("Paste", true), Close]),
                Frame::pill(vec![Done(Lead::Check, "Sent to CONCLAVE"), Chip("Undo", None), Close]),
            ],
            Self::SentNamed => vec![Frame::pill(vec![
                Done(Lead::Check, "Sent to CONCLAVE"),
                Chip("Send to HEX instead", None),
                Close,
            ])],
            Self::Numbered => vec![Frame::pill(vec![
                Done(Lead::Check, "Copied"),
                Chip("CONCLAVE", Some("⌥⇧1")),
                Chip("HEX", Some("⌥⇧2")),
                Chip("HOME", Some("⌥⇧3")),
                Close,
            ])],
            Self::PinnedShortcut => {
                vec![Frame::pill(vec![Done(Lead::Pin, "Sent to HOME"), Hint("⌥⇧H"), Chip("Undo", None), Close])]
            }
            Self::RecordingDest => vec![Frame::pill(rec("0:07", "CONCLAVE"))],
        }
    }

    /// When it shows which frame, in seconds from its entry (`None`: it leaves). `hold` is how
    /// long a one-frame design stays up.
    pub fn steps(self, hold: f64) -> Vec<(f64, Option<usize>)> {
        let at = |times: &[f64], end: f64| {
            times.iter().enumerate().map(|(i, t)| (*t, Some(i))).chain([(end, None)]).collect()
        };
        match self {
            Self::FSure => at(&[0.0, 1.0, 2.0, 3.0, 4.3, 5.4, 6.5], 8.3),
            Self::FUnsure => at(&[0.0, 0.5, 1.9, 3.0], 4.8),
            Self::FNoMatch => at(&[0.0, 1.4, 2.5, 3.6], 5.4),
            Self::ChooseFirst => at(&[0.0, 1.0, 1.7, 2.5, 3.3], 5.1),
            Self::NoRouting => vec![(0.0, Some(0)), (1.8, None), (2.2, Some(1)), (4.0, None)],
            _ => vec![(0.0, Some(0)), (hold, None)],
        }
    }

    /// The frame a held scene rests on: the one that shows the idea.
    pub fn held(self) -> usize {
        match self {
            Self::FUnsure => 1,
            Self::ChooseFirst => 2,
            _ => 0,
        }
    }

    /// Its panel's height: tall enough for the list it drops.
    pub fn panel_h(self) -> f32 {
        let list = self.frames().iter().filter_map(|f| f.list).map(List::height).fold(0.0, f32::max);
        if list > 0.0 { (LIST_Y + list + 18.0).max(PANEL_H) } else { PANEL_H }
    }
}

#[derive(Clone, Copy, Debug)]
enum Lead {
    Check,
    Pin,
}

#[derive(Clone, Copy, Debug)]
enum Item {
    /// The done lead: a plain check (or pin) and its text.
    Done(Lead, &'static str),
    /// The recording lead: the dot, the timer, then "→ destination".
    Rec(&'static str, &'static str),
    /// A quiet chip, with a keycap before its label when it has a shortcut.
    Chip(&'static str, Option<&'static str>),
    /// A keycap on its own: the shortcut that did this.
    Hint(&'static str),
    /// The open pill's text field: what's typed, or a placeholder, and the caret.
    Field(&'static str),
    /// Cancel or Undo (quiet), Finish or Paste (accent).
    Button(&'static str, bool),
    Close,
}

const KEYCAP_H: f32 = 18.0;
const KEYCAP_PX: f32 = 5.0;
const KEYCAP_GAP: f32 = 6.0;
const ARROW_GAP: f32 = 7.0;

fn keycap_text() -> TextStyle {
    TextStyle::new((style().text_size - 2.0).max(8.0), style().medium)
}

fn small() -> TextStyle {
    TextStyle::new((style().text_size - 1.5).max(8.0), style().medium).tracking(style().tracking)
}

fn measure_with(text: &str, style: TextStyle) -> f32 {
    TEXT.with_borrow_mut(|t| t.measure(text, style, 2.0).0)
}

fn keycap_width(keys: &str) -> f32 {
    measure_with(keys, keycap_text()) + 2.0 * KEYCAP_PX
}

fn dest_text(dest: &str) -> String {
    format!("→ {dest}")
}

/// `text` cut to fit `max` points, ending in "…" when cut.
fn ellipsize(text: &str, style: TextStyle, max: f32) -> String {
    if measure_with(text, style) <= max {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    (1..chars.len())
        .rev()
        .map(|n| format!("{}…", chars[..n].iter().collect::<String>().trim_end()))
        .find(|cut| measure_with(cut, style) <= max)
        .unwrap_or_else(|| "…".into())
}

impl Item {
    fn width(self) -> f32 {
        let s = style();
        match self {
            Self::Done(_, text) => DONE_TEXT_X + measure_with(text, s.semibold_text()) + s.lead_trail,
            Self::Rec(clock, dest) => {
                LEAD_TEXT_X
                    + measure_with(clock, timer_style())
                    + ARROW_GAP
                    + measure_with(&dest_text(dest), s.medium_text())
                    + s.lead_trail
            }
            Self::Chip(text, key) => {
                let key = key.map_or(0.0, |k| keycap_width(k) + KEYCAP_GAP);
                key + measure_with(text, s.medium_text()) + 2.0 * s.chip_px
            }
            Self::Hint(keys) => keycap_width(keys) + 2.0 * 4.0,
            Self::Field(_) => FIELD_MIN,
            Self::Button(text, primary) => {
                measure_with(text, if primary { s.semibold_text() } else { s.medium_text() }) + 2.0 * s.chip_px
            }
            Self::Close => CHIP_H,
        }
    }

    fn group(self) -> u8 {
        match self {
            Self::Done(..) | Self::Rec(..) => 0,
            Self::Close => 3,
            _ => 2,
        }
    }

    /// Stays the same across frames while it's the same thing, so it moves rather than swaps.
    fn key(self) -> String {
        match self {
            Self::Done(..) => "done".into(),
            Self::Rec(..) => "rec".into(),
            // A chip with a key stays itself while its label changes (the undo countdown).
            Self::Chip(_, Some(key)) => format!("chip-{key}"),
            Self::Chip(text, None) => format!("chip-{text}"),
            Self::Field(_) => "field".into(),
            Self::Hint(keys) => format!("hint-{keys}"),
            Self::Button(text, _) => format!("button-{text}"),
            Self::Close => "close".into(),
        }
    }
}

/// The capsule's rect and each item's, laid out as `surface::layout` lays out the real one.
fn layout(items: &[Item]) -> ([f32; 4], Vec<(Item, [f32; 4])>) {
    let s = style();
    let gap = |a: Item, b: Item| if a.group() == b.group() { s.chip_gap } else { s.group_gap };
    let mut widths: Vec<f32> = items.iter().map(|i| i.width()).collect();
    let mut span = widths.iter().sum::<f32>() + items.windows(2).map(|w| gap(w[0], w[1])).sum::<f32>();
    // The open pill is as wide as the list under it: its field takes what the rest leaves.
    if let Some(field) = items.iter().position(|i| matches!(i, Item::Field(_))) {
        let grow = (LIST_W - 2.0 * ISLAND_INSET - span).max(0.0);
        widths[field] += grow;
        span += grow;
    }
    let w = span + 2.0 * ISLAND_INSET;
    let l = CX - w * 0.5;
    let mut x = l + ISLAND_INSET;
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        out.push((*item, [x, TOP + ISLAND_INSET, widths[i], CHIP_H]));
        if let Some(next) = items.get(i + 1) {
            x += widths[i] + gap(*item, *next);
        }
    }
    ([l, TOP, w, ISLAND_H], out)
}

fn fade() -> Transition {
    Transition::opacity().combined(Transition::blur(1.5))
}

/// `proto` at `frame` in a `PANEL_W` × `panel_h` panel, mounted while `shown` (entering and
/// leaving on `presence`, as the real capsule's content does).
pub(super) fn capsule(proto: Proto, frame: usize, meter: &Meter, shown: bool, presence: Transition) -> El {
    let root = el().size(PANEL_W, proto.panel_h());
    let frames = proto.frames();
    let Some(frame) = frames.get(frame).filter(|_| shown) else { return root };
    let (shell, items) = layout(&frame.items);
    let [x, y, w, h] = shell;
    let body = shell_body(el().key("shell").absolute(x, y).size(w, h), h).child(rim(w, h).motion(MOVE)).motion(MOVE);
    let mut content = el().key("content").absolute(0.0, 0.0).size(PANEL_W, PANEL_H);
    for (item, [ix, iy, iw, ih]) in items {
        content = content.child(draw(item, meter).key(item.key()).absolute(ix, iy).size(iw, ih).motion(MOVE).transition(fade()));
    }
    let mut up = el().key("proto").absolute(0.0, 0.0).size(PANEL_W, proto.panel_h()).child(body).child(content);
    if let Some(list) = frame.list {
        up = up.child(list_view(list, x, w));
    }
    if let Some(picked) = frame.strip {
        up = up.child(strip(picked));
    }
    root.child(up.transition(presence))
}

/// A floating panel in the capsule's material: its fill, shadow and lit hairline.
fn panel(e: El, w: f32, h: f32, radius: f32) -> El {
    let s = style();
    e.radius(radius)
        .fill(srgb(s.shell[0], s.shell[1], s.shell[2], s.shell_alpha))
        .shadow(4.0, 18.0, Color::BLACK.alpha(s.shadow_alpha))
        .child(el().absolute(0.0, 0.0).size(w, h).radius(radius).border_gradient(0.5, veil(0.15), veil(0.04)).motion(MOVE))
}

/// The list hanging under the open pill, `x` and `w` its shell's.
fn list_view(list: List, x: f32, w: f32) -> El {
    let s = style();
    let h = list.height();
    let mut body = col().absolute(0.0, 0.0).w(w).padding(LIST_PAD);
    let mut n = 0;
    for (title, rows) in list.sections() {
        body = body.child(
            row().center_y().h(HEADER_H).px(10.0)
                .child(label(&title.to_uppercase(), TextStyle::new(10.0, s.semibold).tracking(0.5), white(0.42))),
        );
        for (name, why) in rows {
            let why_w = measure_with(why, small());
            let max = w - 2.0 * LIST_PAD - 2.0 * 10.0 - 10.0 - 3.0 * 10.0 - why_w;
            let mut entry = row().center_y().h(ROW_H).px(10.0).gap(10.0).radius(8.0)
                .child(label(&(n + 1).to_string(), timer_style(), white(0.4)).w(10.0))
                .child(label(&ellipsize(name, medium(), max), medium(), text_color()))
                .child(el().grow(1.0))
                .child(label(why, small(), rgb3(s.secondary)));
            if n == list.selected {
                entry = entry.fill(veil(0.10));
            }
            body = body.child(entry);
            n += 1;
        }
    }
    let keep = if list.suggest == Suggest::Sent { "Keep CONCLAVE" } else { "Keep copied" };
    let quiet = |text: &str| label(text, small(), white(0.5));
    body = body.child(el().h(1.0).w_full().mt(LIST_PAD).fill(veil(0.07))).child(
        row().center_y().h(FOOTER_H).px(10.0).gap(5.0)
            .child(keycap("return", true))
            .child(quiet("Send"))
            .child(el().w(10.0))
            .child(keycap("esc", true))
            .child(quiet(keep)),
    );
    panel(el().key("list").absolute(x, LIST_Y).size(w, h), w, h, 14.0)
        .clip_rounded(14.0)
        .child(body)
        .motion(MOVE)
        .transition(Transition::offset(0.0, -6.0).combined(fade()))
}

/// The open pill's field: what's typed and the caret, or the placeholder.
fn field(typed: &str) -> El {
    let caret = el().size(1.5, 15.0).radius(0.75).fill(accent(false));
    let inner = if typed.is_empty() {
        row().center_y().gap(1.0).child(caret).child(label("Send to…", medium(), white(0.4)))
    } else {
        row().center_y().gap(1.0).child(label(typed, medium(), text_color())).child(caret)
    };
    row().center_y().px(12.0).radius(CHIP_H * 0.5).fill(veil(0.05)).child(inner)
}

/// g's hint while ⌥⇧ is held during a recording: the keys and where each sends it.
fn strip(picked: usize) -> El {
    let s = style();
    let mut pill = row().center_y().h(STRIP_H).px(4.0).gap(2.0).radius(STRIP_H * 0.5)
        .fill(srgb(s.shell[0], s.shell[1], s.shell[2], s.shell_alpha))
        .shadow(4.0, 18.0, Color::BLACK.alpha(s.shadow_alpha))
        .border(0.5, veil(0.12))
        .child(el().w(2.0))
        .child(keycap("⌥⇧", true))
        .child(el().w(2.0));
    for (k, name) in STRIP.into_iter().enumerate() {
        let on = k == picked;
        let mut entry = row().center_y().h(STRIP_H - 8.0).px(8.0).gap(5.0).radius((STRIP_H - 8.0) * 0.5)
            .child(label(&(k + 1).to_string(), timer_style(), white(0.45)))
            .child(label(name, medium(), if on { text_color() } else { rgb3(s.secondary) }));
        if on {
            entry = entry.fill(veil(0.14));
        }
        pill = pill.child(entry);
    }
    row().key("strip").absolute(0.0, LIST_Y).size(PANEL_W, STRIP_H).center().child(pill)
        .transition(Transition::offset(0.0, -6.0).combined(fade()))
}

fn keycap(keys: &str, quiet: bool) -> El {
    row().center().h(KEYCAP_H).w(keycap_width(keys)).radius(5.0)
        .fill(veil(if quiet { 0.07 } else { 0.12 }))
        .border(0.5, veil(0.16))
        .child(label(keys, keycap_text(), white(if quiet { 0.61 } else { 0.86 })))
}

fn draw(item: Item, meter: &Meter) -> El {
    let s = style();
    match item {
        Item::Done(lead, text) => {
            let glyph = match lead {
                Lead::Check => check(),
                Lead::Pin => canvas(pin(16.0 * 2.4 / 24.0, green())).absolute(8.0, (CHIP_H - 16.0) * 0.5).size(16.0, 16.0),
            };
            el().child(glyph)
                .child(row().center_y().absolute(DONE_TEXT_X, 0.0).h(CHIP_H).child(label(text, semibold(), text_color())))
        }
        Item::Rec(clock, dest) => {
            let clock_w = measure_with(clock, timer_style());
            el().child(dot(meter))
                .child(row().center_y().absolute(LEAD_TEXT_X, 0.0).h(CHIP_H).child(label(clock, timer_style(), text_color())))
                .child(
                    row().center_y().absolute(LEAD_TEXT_X + clock_w + ARROW_GAP, 0.0).h(CHIP_H)
                        .child(label(&dest_text(dest), medium(), rgb3(s.secondary))),
                )
        }
        Item::Chip(text, key) => {
            let mut chip = row().center().gap(KEYCAP_GAP).radius(CHIP_H * 0.5).hover(veil(0.08 + s.chip_fill));
            if s.chip_fill > 0.0 {
                chip = chip.fill(veil(s.chip_fill));
            }
            if let Some(key) = key {
                chip = chip.child(keycap(key, false));
            }
            chip.child(label(text, medium(), rgb3(s.secondary)))
        }
        Item::Hint(keys) => row().center().child(keycap(keys, true)),
        Item::Field(typed) => field(typed),
        Item::Button(text, primary) => {
            let chip = row().center().radius(CHIP_H * 0.5);
            if primary {
                chip.fill(accent(false)).child(label(text, semibold(), Color::WHITE.alpha(0.95)))
            } else {
                chip.hover(veil(0.08)).child(label(text, medium(), rgb3(s.secondary)))
            }
        }
        Item::Close => row().center().child(
            row().center().size(28.0, 28.0).radius(14.0).child(canvas(icon(ElemId::Close, 12.0 * 2.4 / 24.0, white(0.61))).size(12.0, 12.0)),
        ),
    }
}

/// A push pin on the 24 × 24 icon grid.
fn pin(width: f32, color: Color) -> Shape {
    let head = Path::new()
        .move_to([8.0, 3.0])
        .line_to([16.0, 3.0])
        .line_to([15.0, 9.0])
        .line_to([18.5, 13.5])
        .line_to([5.5, 13.5])
        .line_to([9.0, 9.0])
        .close();
    let needle = Path::new().move_to([12.0, 13.5]).line_to([12.0, 21.0]);
    Shape::new()
        .view_box(24.0, 24.0)
        .stroke_path(&head, width, LineCap::Round, color)
        .stroke_path(&needle, width, LineCap::Round, color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_design_fits_the_panel() {
        super::super::fonts();
        for proto in Proto::NEW.into_iter().chain(Proto::OLD) {
            for frame in proto.frames() {
                let (shell, items) = layout(&frame.items);
                assert!(shell[0] >= 0.0 && shell[0] + shell[2] <= PANEL_W, "{proto:?} {shell:?}");
                for pair in items.windows(2) {
                    assert!(pair[0].1[0] + pair[0].1[2] <= pair[1].1[0] + 0.01, "{proto:?} overlaps");
                }
                if let Some(list) = frame.list {
                    assert!(LIST_Y + list.height() <= proto.panel_h(), "{proto:?}: the list fits its panel");
                }
                if frame.items.iter().any(|i| matches!(i, Item::Field(_))) {
                    assert!((shell[2] - LIST_W).abs() < 0.5, "{proto:?}: the open pill is the list's width");
                }
            }
        }
    }

    #[test]
    fn every_step_names_a_frame_and_the_flow_ends_hidden() {
        for proto in Proto::NEW.into_iter().chain(Proto::OLD) {
            let frames = proto.frames().len();
            let steps = proto.steps(2.0);
            assert!(steps.iter().all(|(_, f)| f.is_none_or(|f| f < frames)), "{proto:?}");
            assert!(steps.windows(2).all(|w| w[0].0 < w[1].0), "{proto:?}");
            assert_eq!(steps.last().map(|s| s.1), Some(None), "{proto:?}");
            assert!(proto.held() < frames, "{proto:?}");
        }
    }

    #[test]
    fn long_names_end_in_an_ellipsis() {
        super::super::fonts();
        let cut = ellipsize("OpenCode Desktop · Problems board rewrite", medium(), 120.0);
        assert!(cut.ends_with('…') && measure_with(&cut, medium()) <= 120.0, "{cut}");
        assert_eq!(ellipsize("HEX", medium(), 120.0), "HEX");
    }
}
