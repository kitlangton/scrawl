//! The capsule's spacing, type and colours, as values rather than constants, so the pill lab can
//! tune them live from `~/.config/scrawl/pill-lab.json` (`$SCRAWL_PILL_LAB`). Only the lab loads
//! that file; everyone else draws `Style::DEFAULT`, the capsule's own values.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use rune::TextStyle;

use crate::native::pill::model::{CHIP_GAP, CHIP_PX, HUD_TEXT};

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Style {
    /// Between chips of one group.
    pub chip_gap: f32,
    /// Between groups (lead | chips | ✕).
    pub group_gap: f32,
    /// Each side of a text chip's label (Record, Cancel, Finish, Paste, Inspect, routes).
    pub chip_px: f32,
    /// After the lead's text (timer, done label).
    pub lead_trail: f32,
    pub text_size: f32,
    pub tracking: f32,
    pub medium: f32,
    pub semibold: f32,
    /// sRGB 0…1.
    pub text: [f32; 3],
    /// Cancel, Inspect and route chips' labels.
    pub secondary: [f32; 3],
    pub accent: [f32; 3],
    pub accent_hover: [f32; 3],
    pub green: [f32; 3],
    pub shell: [f32; 3],
    pub shell_alpha: f32,
    pub shadow_alpha: f32,
    /// White behind Cancel, Inspect and route chips at rest (0: none, as today).
    pub chip_fill: f32,
}

impl Style {
    pub const DEFAULT: Style = Style {
        chip_gap: CHIP_GAP,
        group_gap: 6.0,
        chip_px: CHIP_PX,
        lead_trail: 12.0,
        text_size: HUD_TEXT,
        tracking: 0.0,
        medium: 500.0,
        semibold: 600.0,
        text: [0.961, 0.961, 0.969],
        secondary: [0.74, 0.74, 0.74],
        accent: [0.231, 0.361, 0.965],
        accent_hover: [0.294, 0.420, 1.0],
        green: [0.412, 0.847, 0.624],
        shell: [20.0 / 255.0, 20.0 / 255.0, 22.0 / 255.0],
        shell_alpha: 0.99,
        shadow_alpha: 0.34,
        chip_fill: 0.0,
    };

    pub fn medium_text(&self) -> TextStyle {
        TextStyle::new(self.text_size, self.medium).tracking(self.tracking)
    }

    pub fn semibold_text(&self) -> TextStyle {
        TextStyle::new(self.text_size, self.semibold).tracking(self.tracking)
    }

    /// Every key the lab file takes, with its value here (`pill-lab.json`'s shape).
    pub fn to_json(&self) -> serde_json::Value {
        let hex = |c: [f32; 3]| {
            let b = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            format!("#{:02x}{:02x}{:02x}", b(c[0]), b(c[1]), b(c[2]))
        };
        serde_json::json!({
            "chipGap": self.chip_gap,
            "groupGap": self.group_gap,
            "chipPadX": self.chip_px,
            "leadTrail": self.lead_trail,
            "textSize": self.text_size,
            "tracking": self.tracking,
            "weight": self.medium,
            "weightBold": self.semibold,
            "text": hex(self.text),
            "secondary": hex(self.secondary),
            "accent": hex(self.accent),
            "accentHover": hex(self.accent_hover),
            "green": hex(self.green),
            "shell": hex(self.shell),
            "shellAlpha": self.shell_alpha,
            "shadowAlpha": self.shadow_alpha,
            "chipFill": self.chip_fill,
        })
    }

    /// `DEFAULT` with the keys `value` sets (numbers, or `#rrggbb` colours).
    pub fn from_json(value: &serde_json::Value) -> Style {
        let mut s = Style::DEFAULT;
        let num = |key: &str, into: &mut f32| {
            if let Some(v) = value.get(key).and_then(serde_json::Value::as_f64) {
                *into = v as f32;
            }
        };
        num("chipGap", &mut s.chip_gap);
        num("groupGap", &mut s.group_gap);
        num("chipPadX", &mut s.chip_px);
        num("leadTrail", &mut s.lead_trail);
        num("textSize", &mut s.text_size);
        num("tracking", &mut s.tracking);
        num("weight", &mut s.medium);
        num("weightBold", &mut s.semibold);
        num("shellAlpha", &mut s.shell_alpha);
        num("shadowAlpha", &mut s.shadow_alpha);
        num("chipFill", &mut s.chip_fill);
        let color = |key: &str, into: &mut [f32; 3]| {
            if let Some(c) = value.get(key).and_then(serde_json::Value::as_str).and_then(parse_hex) {
                *into = c;
            }
        };
        color("text", &mut s.text);
        color("secondary", &mut s.secondary);
        color("accent", &mut s.accent);
        color("accentHover", &mut s.accent_hover);
        color("green", &mut s.green);
        color("shell", &mut s.shell);
        s.text_size = s.text_size.clamp(8.0, 24.0);
        s.chip_px = s.chip_px.clamp(0.0, 40.0);
        s
    }
}

fn parse_hex(s: &str) -> Option<[f32; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let b = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).ok().map(|v| f32::from(v) / 255.0);
    Some([b(0)?, b(2)?, b(4)?])
}

thread_local! {
    static CURRENT: Cell<Style> = const { Cell::new(Style::DEFAULT) };
    static FILE: RefCell<(Option<Instant>, Option<SystemTime>)> = const { RefCell::new((None, None)) };
}

/// The values the capsule draws with on this thread.
pub(crate) fn style() -> Style {
    CURRENT.get()
}

pub(crate) fn lab_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("SCRAWL_PILL_LAB") {
        return Some(PathBuf::from(path));
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config/scrawl/pill-lab.json"))
}

/// The lab's reload: re-reads `pill-lab.json` if it changed (at most four times a second). True
/// when the style changed.
pub(crate) fn poll_lab_file() -> bool {
    FILE.with_borrow_mut(|(checked, stamp)| {
        if checked.is_some_and(|at| at.elapsed() < Duration::from_millis(250)) {
            return false;
        }
        let first = checked.is_none();
        *checked = Some(Instant::now());
        let Some(path) = lab_path() else { return false };
        let now = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        if now == *stamp && !first {
            return false;
        }
        *stamp = now;
        let next = now
            .and_then(|_| std::fs::read_to_string(&path).ok())
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .map_or(Style::DEFAULT, |value| Style::from_json(&value));
        let changed = next != CURRENT.get();
        CURRENT.set(next);
        changed
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_lab_file_is_the_capsule_as_it_ships() {
        assert_eq!(Style::from_json(&serde_json::json!({})), Style::DEFAULT);
        let tuned = Style::from_json(&serde_json::json!({ "chipGap": 4, "accent": "#ff0000" }));
        assert_eq!(tuned.chip_gap, 4.0);
        assert_eq!(tuned.accent, [1.0, 0.0, 0.0]);
        // Every key it writes, it reads back.
        let round = Style::from_json(&Style::DEFAULT.to_json());
        assert_eq!(round.chip_px, Style::DEFAULT.chip_px);
        assert_eq!(round.text_size, Style::DEFAULT.text_size);
    }
}
