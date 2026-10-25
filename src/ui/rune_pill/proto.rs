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
    /// f: "Copied" with one hint; ⌥⇧V drops a list of sessions below the pill.
    SendList,
    /// f, auto-routed: "Sent to CONCLAVE"; the same list sends it elsewhere.
    SendListAuto,
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

/// f's send-to list: suggested sessions (with why), then recent ones, filtered by what's typed.
#[derive(Clone, Copy, Debug)]
struct List {
    filter: &'static str,
    selected: usize,
    /// It was already sent to the first suggestion.
    auto: bool,
}

const SUGGESTED: [(&str, &str); 2] = [("CONCLAVE", "dev site localhost:5173"), ("HEX", "last sent")];
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
        let mut suggested = keep(&SUGGESTED);
        if self.auto
            && let Some(first) = suggested.first_mut()
            && first.0 == SUGGESTED[0].0
        {
            first.1 = "sent · localhost:5173";
        }
        [("Suggested", suggested), ("Recent", keep(&RECENT))].into_iter().filter(|(_, rows)| !rows.is_empty()).collect()
    }

    fn height(self) -> f32 {
        let rows: f32 = self.sections().iter().map(|(_, rows)| HEADER_H + rows.len() as f32 * ROW_H).sum();
        2.0 * LIST_PAD + FILTER_H + 1.0 + rows + FOOTER_H
    }
}

const LIST_W: f32 = 340.0;
const LIST_PAD: f32 = 6.0;
const LIST_Y: f32 = TOP + ISLAND_H + 8.0;
const FILTER_H: f32 = 34.0;
const HEADER_H: f32 = 22.0;
const ROW_H: f32 = 30.0;
const FOOTER_H: f32 = 30.0;
const STRIP_H: f32 = 30.0;

/// Frames move on one plain spring (the real capsule's are keyed on the kind of change).
const MOVE: Spring = Spring::new(0.32, 0.0);

impl Proto {
    /// f–h, the designs after Kit's look at b–e, first in the lab.
    pub const NEW: [Proto; 4] = [Self::SendList, Self::SendListAuto, Self::ChooseFirst, Self::NoRouting];
    pub const OLD: [Proto; 4] = [Self::SentNamed, Self::Numbered, Self::PinnedShortcut, Self::RecordingDest];

    pub fn title(self) -> &'static str {
        match self {
            Self::SendList => "f · Send-to list: Copied, ⌥⇧V drops sessions below",
            Self::SendListAuto => "f · auto-routed: Sent, ⌥⇧V to send elsewhere",
            Self::ChooseFirst => "g · Choose before you finish: ⌥⇧1–3 while recording",
            Self::NoRouting => "h · No routing UI: dev site → Sent, else Copied · Paste",
            Self::SentNamed => "b · Name it: ✓ Sent to CONCLAVE, one “instead” chip",
            Self::Numbered => "c · Pick by key: ⌥⇧1…3 while the pill is up",
            Self::PinnedShortcut => "d · Pinned HOME, sent with its own shortcut",
            Self::RecordingDest => "e · While recording: where it's going",
        }
    }

    /// Its recording dot breathes while it's up.
    pub fn recording(self) -> bool {
        matches!(self, Self::RecordingDest | Self::ChooseFirst)
    }

    pub(super) fn frames(self) -> Vec<Frame> {
        use Item::*;
        let rec = |clock, dest| vec![Rec(clock, dest), Button("Cancel", false), Button("Finish", true)];
        let sent = |to| vec![Done(Lead::Check, to), Chip("Undo", None), Close];
        let send = |lead| vec![Done(Lead::Check, lead), Chip("Send…", Some("⌥⇧V")), Close];
        let list = |filter, selected, auto| List { filter, selected, auto };
        match self {
            Self::SendList => vec![
                Frame::pill(send("Copied")),
                Frame::pill(send("Copied")).list(list("", 0, false)),
                Frame::pill(send("Copied")).list(list("co", 0, false)),
                Frame::pill(sent("Sent to CONCLAVE")),
            ],
            Self::SendListAuto => vec![
                Frame::pill(send("Sent to CONCLAVE")),
                Frame::pill(send("Sent to CONCLAVE")).list(list("", 0, true)),
                Frame::pill(send("Sent to CONCLAVE")).list(list("", 1, true)),
                Frame::pill(sent("Sent to HEX")),
            ],
            Self::ChooseFirst => vec![
                Frame::pill(rec("0:07", "CONCLAVE")),
                Frame::pill(rec("0:07", "CONCLAVE")).strip(0),
                Frame::pill(rec("0:08", "HEX")).strip(1),
                Frame::pill(rec("0:09", "HEX")),
                Frame::pill(sent("Sent to HEX")),
            ],
            Self::NoRouting => vec![
                Frame::pill(vec![Done(Lead::Check, "Copied"), Button("Paste", true), Close]),
                Frame::pill(sent("Sent to CONCLAVE")),
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
        match self {
            Self::SendList => vec![(0.0, Some(0)), (1.2, Some(1)), (2.6, Some(2)), (3.8, Some(3)), (5.6, None)],
            Self::SendListAuto => vec![(0.0, Some(0)), (1.2, Some(1)), (2.3, Some(2)), (3.4, Some(3)), (5.2, None)],
            Self::ChooseFirst => {
                vec![(0.0, Some(0)), (1.0, Some(1)), (1.7, Some(2)), (2.5, Some(3)), (3.3, Some(4)), (5.1, None)]
            }
            Self::NoRouting => vec![(0.0, Some(0)), (1.8, None), (2.2, Some(1)), (4.0, None)],
            _ => vec![(0.0, Some(0)), (hold, None)],
        }
    }

    /// The frame a held cell rests on: the one that shows the idea.
    pub fn held(self) -> usize {
        match self {
            Self::SendList | Self::SendListAuto => 1,
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
            Self::Chip(text, _) => format!("chip-{text}"),
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
    let widths: Vec<f32> = items.iter().map(|i| i.width()).collect();
    let span = widths.iter().sum::<f32>() + items.windows(2).map(|w| gap(w[0], w[1])).sum::<f32>();
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
        up = up.child(list_view(list));
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

fn list_view(list: List) -> El {
    let s = style();
    let h = list.height();
    let mut body = col().absolute(0.0, 0.0).w(LIST_W).padding(LIST_PAD);
    let filter = if list.filter.is_empty() {
        row().center_y().h(FILTER_H).px(10.0).child(label("Send to…", medium(), white(0.42)))
    } else {
        row().center_y().h(FILTER_H).px(10.0).gap(1.0)
            .child(label(list.filter, medium(), text_color()))
            .child(el().size(1.5, 16.0).radius(0.75).fill(accent(false)))
    };
    body = body.child(filter).child(el().h(1.0).w_full().fill(veil(0.08)));
    let mut n = 0;
    for (title, rows) in list.sections() {
        body = body.child(
            row().center_y().h(HEADER_H).px(10.0)
                .child(label(&title.to_uppercase(), TextStyle::new(10.0, s.semibold).tracking(0.5), white(0.45))),
        );
        for (name, why) in rows {
            let why_w = measure_with(why, small());
            let max = LIST_W - 2.0 * LIST_PAD - 2.0 * 10.0 - 10.0 - 2.0 * 10.0 - why_w;
            let mut entry = row().center_y().h(ROW_H).px(10.0).gap(10.0).radius(8.0)
                .child(label(&(n + 1).to_string(), timer_style(), white(0.45)).w(10.0))
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
    let keep = if list.auto { "Keep in CONCLAVE" } else { "Keep copied" };
    body = body.child(
        row().center_y().h(FOOTER_H).px(10.0).gap(6.0)
            .child(keycap("↵", true))
            .child(label("Send", small(), rgb3(s.secondary)))
            .child(el().w(8.0))
            .child(keycap("esc", true))
            .child(label(keep, small(), rgb3(s.secondary))),
    );
    panel(el().key("list").absolute(CX - LIST_W * 0.5, LIST_Y).size(LIST_W, h), LIST_W, h, 14.0)
        .clip_rounded(14.0)
        .child(body)
        .motion(MOVE)
        .transition(Transition::offset(0.0, -6.0).combined(fade()))
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
