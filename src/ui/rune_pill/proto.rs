//! Lab-only designs for the capsule after a recording, drawn beside today's in the pill lab
//! (`scrawl lab pill grid --rune`) so they compare at real size over the same backdrops. They
//! are pictures: no routing, no `Pres`, nothing the app can reach. Each borrows the capsule's
//! body, rim, type, colours and spacing (`style()`), so lab knobs move them with the real one.

use rune::{Color, El, LineCap, Path, Shape, TextStyle, Transition, canvas, el, row};

use super::style::style;
use super::surface::{DONE_TEXT_X, LEAD_TEXT_X};
use super::{
    Meter, PANEL_H, PANEL_W, TEXT, accent, dot, green, icon, label, medium, over_shell, rgb3, rim, semibold, shell_body,
    text_color, timer_style, veil, white,
};
use crate::native::pill::model::{CHIP_H, CX, ElemId, ISLAND_H, ISLAND_INSET, TOP};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Proto {
    /// b: the destination named in the lead, one chip to send it elsewhere instead.
    SentNamed,
    /// c: up to three destinations, each with the key that picks it while the pill is up.
    Numbered,
    /// d: a pinned destination's own shortcut was used, so it went straight there.
    PinnedShortcut,
    /// e: the recording says where it will go.
    RecordingDest,
}

impl Proto {
    pub const ALL: [Proto; 4] = [Self::SentNamed, Self::Numbered, Self::PinnedShortcut, Self::RecordingDest];

    pub fn title(self) -> &'static str {
        match self {
            Self::SentNamed => "b · Name it: ✓ Sent to CONCLAVE, one “instead” chip",
            Self::Numbered => "c · Pick by key: ⌥⇧1…3 while the pill is up",
            Self::PinnedShortcut => "d · Pinned HOME, sent with its own shortcut",
            Self::RecordingDest => "e · While recording: where it's going",
        }
    }

    pub fn recording(self) -> bool {
        self == Self::RecordingDest
    }

    fn items(self) -> Vec<Item> {
        use Item::*;
        match self {
            Self::SentNamed => vec![Done(Lead::Check, "Sent to CONCLAVE"), Chip("Send to HEX instead", None), Close],
            Self::Numbered => vec![
                Done(Lead::Check, "Copied"),
                Chip("CONCLAVE", Some("⌥⇧1")),
                Chip("HEX", Some("⌥⇧2")),
                Chip("HOME", Some("⌥⇧3")),
                Close,
            ],
            Self::PinnedShortcut => vec![Done(Lead::Pin, "Sent to HOME"), Hint("⌥⇧H"), Chip("Undo", None), Close],
            Self::RecordingDest => vec![Rec("0:07", "CONCLAVE"), Button("Cancel", false), Button("Finish", true)],
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Lead {
    Check,
    Pin,
}

#[derive(Clone, Copy, Debug)]
enum Item {
    /// The done lead: a green check (or pin) and its text.
    Done(Lead, &'static str),
    /// The recording lead: the dot, the timer, then "→ destination".
    Rec(&'static str, &'static str),
    /// A quiet chip, with a keycap before its label when it has a shortcut.
    Chip(&'static str, Option<&'static str>),
    /// A keycap on its own: the shortcut that did this.
    Hint(&'static str),
    /// Cancel (quiet) or Finish (accent).
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

fn measure_with(text: &str, style: TextStyle) -> f32 {
    TEXT.with_borrow_mut(|t| t.measure(text, style, 2.0).0)
}

fn keycap_width(keys: &str) -> f32 {
    measure_with(keys, keycap_text()) + 2.0 * KEYCAP_PX
}

fn dest_text(dest: &str) -> String {
    format!("→ {dest}")
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

/// `proto` in a `PANEL_W` × `PANEL_H` panel, mounted while `shown` (entering and leaving on
/// `presence`, as the real capsule's content does).
pub(super) fn capsule(proto: Proto, meter: &Meter, shown: bool, presence: Transition) -> El {
    let root = el().size(PANEL_W, PANEL_H);
    if !shown {
        return root;
    }
    let (shell, items) = layout(&proto.items());
    let [x, y, w, h] = shell;
    let body = shell_body(el().key("shell").absolute(x, y).size(w, h), h).child(rim(w, h, !proto.recording()));
    let mut content = el().key("content").absolute(0.0, 0.0).size(PANEL_W, PANEL_H);
    for (i, (item, rect)) in items.into_iter().enumerate() {
        let [ix, iy, iw, ih] = rect;
        content = content.child(draw(item, meter).key(format!("item-{i}")).absolute(ix, iy).size(iw, ih));
    }
    root.child(el().key("proto").absolute(0.0, 0.0).size(PANEL_W, PANEL_H).child(body).child(content).transition(presence))
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
                Lead::Check => canvas(icon(ElemId::DoneLead, 13.0 * 2.6 / 24.0, green())),
                Lead::Pin => canvas(pin(13.0 * 2.4 / 24.0, green())),
            };
            el().child(el().absolute(5.0, 5.0).size(22.0, 22.0).radius(11.0).fill(over_shell(green(), 0.22)))
                .child(glyph.absolute(9.5, 9.5).size(13.0, 13.0))
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
        for proto in Proto::ALL {
            let (shell, items) = layout(&proto.items());
            assert!(shell[0] >= 0.0 && shell[0] + shell[2] <= PANEL_W, "{proto:?} {shell:?}");
            for pair in items.windows(2) {
                assert!(pair[0].1[0] + pair[0].1[2] <= pair[1].1[0] + 0.01, "{proto:?} overlaps");
            }
        }
    }
}
