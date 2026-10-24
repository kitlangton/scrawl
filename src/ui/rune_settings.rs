//! Scrawl's Settings window drawn with Rune, in Scrawl's own process (`"ui": "rune"`). A port of
//! `helper/Settings.swift`: the same JSON state in, the same actions out, the same sizes,
//! colours, copy and timings. `settings::Panel` still decides everything.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use rune::{
    Align, App, AppHandle, AppSpec, Chrome, Color, Cx, El, Family, Icon, Phase, PhysicalInput,
    PhysicalKeyCode, Pixels, PopupItem, PopupMenu, Side, Spring, SpringPair, TextEdit, TextStyle, Transition, Tx,
    WindowOptions, WindowSpec, col, el, row, spacer, text, text_input,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::rune_keys::{self, FIELD, KeyController};
use super::rune_session::{Attached, Host, Link};
use super::rune_symbols::{symbol, weight};

// MARK: Colours

/// A colour in sRGB with straight alpha. Fills are composited over their known background in sRGB,
/// as AppKit does, so `white @ 0.045` reads the same as in SwiftUI (Rune blends in linear light).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const WHITE: Rgba = Rgba::rgb(1.0, 1.0, 1.0);
    pub const BLACK: Rgba = Rgba::rgb(0.0, 0.0, 0.0);

    pub const fn rgb(r: f32, g: f32, b: f32) -> Self {
        Self { r, g, b, a: 1.0 }
    }

    pub const fn gray(w: f32) -> Self {
        Self::rgb(w, w, w)
    }

    pub const fn hex(v: u32) -> Self {
        Self::rgb(
            ((v >> 16) & 0xff) as f32 / 255.0,
            ((v >> 8) & 0xff) as f32 / 255.0,
            (v & 0xff) as f32 / 255.0,
        )
    }

    /// SwiftUI's `.opacity(a)`.
    pub const fn alpha(self, a: f32) -> Self {
        Self { a: self.a * a, ..self }
    }

    pub fn mix(self, other: Rgba, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let l = |a: f32, b: f32| a + (b - a) * t;
        Self { r: l(self.r, other.r), g: l(self.g, other.g), b: l(self.b, other.b), a: l(self.a, other.a) }
    }

    /// Over an opaque background, in sRGB: an opaque colour.
    pub fn over_rgba(self, bg: Rgba) -> Rgba {
        let a = self.a;
        Rgba::rgb(a * self.r + (1.0 - a) * bg.r, a * self.g + (1.0 - a) * bg.g, a * self.b + (1.0 - a) * bg.b)
    }

    pub fn over(self, bg: Rgba) -> Color {
        self.over_rgba(bg).color()
    }

    pub fn color(self) -> Color {
        use rune::color::srgb_decode;
        Color { r: srgb_decode(self.r), g: srgb_decode(self.g), b: srgb_decode(self.b), a: self.a }
    }
}

/// `SettingsTheme`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Theme {
    pub dark: bool,
}

impl Theme {
    fn ink(self, a: f32) -> Rgba {
        if self.dark { Rgba::WHITE.alpha(a) } else { Rgba::BLACK.alpha(a) }
    }
    pub fn window(self) -> Rgba {
        Rgba::gray(if self.dark { 0.118 } else { 0.955 })
    }
    pub fn sidebar(self) -> Rgba {
        Rgba::gray(if self.dark { 0.15 } else { 0.915 })
    }
    /// A group's fill over the window, opaque.
    pub fn group_bg(self) -> Rgba {
        if self.dark { Rgba::WHITE.alpha(0.045).over_rgba(self.window()) } else { Rgba::WHITE }
    }
    pub fn group_line(self) -> Rgba {
        self.ink(if self.dark { 0.075 } else { 0.08 })
    }
    pub fn separator(self) -> Rgba {
        self.ink(if self.dark { 0.07 } else { 0.075 })
    }
    pub fn text(self) -> Rgba {
        Rgba::gray(if self.dark { 0.95 } else { 0.1 })
    }
    pub fn secondary(self) -> Rgba {
        self.ink(if self.dark { 0.55 } else { 0.5 })
    }
    pub fn tertiary(self) -> Rgba {
        self.ink(if self.dark { 0.3 } else { 0.28 })
    }
    pub fn selection(self) -> Rgba {
        self.ink(if self.dark { 0.08 } else { 0.07 })
    }
    pub fn control(self) -> Rgba {
        self.ink(if self.dark { 0.08 } else { 0.06 })
    }
    pub fn control_line(self) -> Rgba {
        self.ink(0.1)
    }
    pub fn track(self) -> Rgba {
        self.ink(if self.dark { 0.12 } else { 0.13 })
    }
    pub fn accent(self) -> Rgba {
        if self.dark { Rgba::hex(0x0A84FF) } else { Rgba::hex(0x007AFF) }
    }
    pub fn positive(self) -> Rgba {
        Rgba::rgb(0.2, 0.74, 0.42)
    }
    pub fn negative(self) -> Rgba {
        Rgba::rgb(1.0, 0.27, 0.23)
    }
}

// MARK: Fonts and icons

static FONTS: OnceLock<&'static [&'static [u8]]> = OnceLock::new();

/// SF, SF Mono and SF Rounded from the system; Rune's Inter when SF is missing.
pub(crate) fn fonts() -> &'static [&'static [u8]] {
    FONTS.get_or_init(|| {
        let read = |name: &str| std::fs::read(format!("/System/Library/Fonts/{name}.ttf")).ok();
        let Some(sans) = read("SFNS") else { return &[] };
        let mut all: Vec<&'static [u8]> = vec![Box::leak(sans.into_boxed_slice())];
        for name in ["SFNSMono", "SFNSRounded"] {
            if let Some(bytes) = read(name) {
                all.push(Box::leak(bytes.into_boxed_slice()));
            }
        }
        Box::leak(all.into_boxed_slice())
    })
}

pub(crate) fn families(fonts: &[&[u8]]) -> (&'static str, &'static str) {
    if fonts.is_empty() { ("Inter", "Inter") } else { ("System Font", ".SF NS Mono") }
}

pub(crate) const ROUNDED: Family = Family::Named(".SF NS Rounded");

pub(crate) fn style(size: f32, weight: f32) -> TextStyle {
    TextStyle::new(size, weight)
}

const MONO: TextStyle = TextStyle::new(12.0, 400.0).mono();
fn digits() -> TextStyle {
    style(14.0, 400.0).features("'tnum' 1")
}

/// One element's stroke, as Rune reads stroke attributes per element.
macro_rules! stroked {
    ($($e:literal),* $(,)?) => {
        concat!($(concat!($e, r#" fill="none" stroke="black" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"/>"#)),*)
    };
}

macro_rules! icon_def {
    ($id:ident, $name:literal, $body:expr) => {
        pub(crate) static $id: Icon = Icon { name: $name, view: 24.0, body: $body };
    };
}

pub(crate) mod icons {
    use rune::Icon;

    icon_def!(GEAR_FILL, "gearshape.fill", const_format_gear());
    const fn const_format_gear() -> &'static str {
        concat!(
            r#"<path fill="black" fill-rule="evenodd" d=""#,
            "M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z",
            "M15.2 12a3.2 3.2 0 1 1-6.4 0a3.2 3.2 0 1 1 6.4 0Z",
            r#""/>"#,
            r#"<path d="M12.22 2h-.44a2 2 0 0 0-2 2v.18a2 2 0 0 1-1 1.73l-.43.25a2 2 0 0 1-2 0l-.15-.08a2 2 0 0 0-2.73.73l-.22.38a2 2 0 0 0 .73 2.73l.15.1a2 2 0 0 1 1 1.72v.51a2 2 0 0 1-1 1.74l-.15.09a2 2 0 0 0-.73 2.73l.22.38a2 2 0 0 0 2.73.73l.15-.08a2 2 0 0 1 2 0l.43.25a2 2 0 0 1 1 1.73V20a2 2 0 0 0 2 2h.44a2 2 0 0 0 2-2v-.18a2 2 0 0 1 1-1.73l.43-.25a2 2 0 0 1 2 0l.15.08a2 2 0 0 0 2.73-.73l.22-.39a2 2 0 0 0-.73-2.73l-.15-.08a2 2 0 0 1-1-1.74v-.5a2 2 0 0 1 1-1.74l.15-.09a2 2 0 0 0 .73-2.73l-.22-.38a2 2 0 0 0-2.73-.73l-.15.08a2 2 0 0 1-2 0l-.43-.25a2 2 0 0 1-1-1.73V4a2 2 0 0 0-2-2z" fill="none" stroke="black" stroke-width="1.6" stroke-linejoin="round"/>"#,
        )
    }

    icon_def!(RECORD, "record.circle", concat!(
        stroked!(r#"<circle cx="12" cy="12" r="9""#),
        r#"<circle cx="12" cy="12" r="4.6" fill="black"/>"#,
    ));
    icon_def!(DRIVE_FILL, "internaldrive.fill", concat!(
        r#"<path fill="black" fill-rule="evenodd" d="M6.3 4.5h11.4a2.5 2.5 0 0 1 2.3 1.5l2.2 5.3v6.2a2.5 2.5 0 0 1-2.5 2.5H4.3a2.5 2.5 0 0 1-2.5-2.5v-6.2L4 6a2.5 2.5 0 0 1 2.3-1.5Z M17.4 15.6a1.2 1.2 0 1 0 2.4 0a1.2 1.2 0 1 0-2.4 0Z M3.9 12.2h16.2v-.6H3.9Z"/>"#,
    ));
    icon_def!(HAND, "hand.raised.fill", stroked!(
        r#"<path d="M18 11V6a2 2 0 0 0-4 0""#,
        r#"<path d="M14 10V4a2 2 0 0 0-4 0v2""#,
        r#"<path d="M10 10.5V6a2 2 0 0 0-4 0v8""#,
        r#"<path d="M18 8a2 2 0 1 1 4 0v6a8 8 0 0 1-8 8h-2c-2.8 0-4.5-.86-5.99-2.34l-3.6-3.6a2 2 0 0 1 2.83-2.82L7 15""#,
    ));
    icon_def!(TERMINAL, "terminal.fill", concat!(
        r#"<path fill="black" fill-rule="evenodd" d="M5 3.5h14a3 3 0 0 1 3 3v11a3 3 0 0 1-3 3H5a3 3 0 0 1-3-3v-11a3 3 0 0 1 3-3Z M6.3 8.4a.9.9 0 0 0 0 1.3l2.6 2.3-2.6 2.3a.9.9 0 1 0 1.2 1.4l3.4-3a.9.9 0 0 0 0-1.4l-3.4-3a.9.9 0 0 0-1.2.1Z M12.5 14.6a.9.9 0 0 0 0 1.8h5a.9.9 0 0 0 0-1.8Z"/>"#,
    ));
    icon_def!(INFO, "info", concat!(
        r#"<path d="M12 10.5v8.5" fill="none" stroke="black" stroke-width="3.2" stroke-linecap="round"/>"#,
        r#"<circle cx="12" cy="5.3" r="2" fill="black"/>"#,
    ));
    icon_def!(CHEVRONS, "chevron.up.chevron.down", concat!(
        r#"<path d="M7.5 9.5 12 5l4.5 4.5" fill="none" stroke="black" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"/>"#,
        r#"<path d="M7.5 14.5 12 19l4.5-4.5" fill="none" stroke="black" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"/>"#,
    ));
    icon_def!(RESET, "arrow.counterclockwise", concat!(
        r#"<path d="M4 12a8 8 0 1 0 2.6-5.9L4 8.5" fill="none" stroke="black" stroke-width="3.4" stroke-linecap="round" stroke-linejoin="round"/>"#,
        r#"<path d="M4 3.5v5h5" fill="none" stroke="black" stroke-width="3.4" stroke-linecap="round" stroke-linejoin="round"/>"#,
    ));
    icon_def!(X, "xmark", concat!(
        r#"<path d="M18 6 6 18" fill="none" stroke="black" stroke-width="3.6" stroke-linecap="round"/>"#,
        r#"<path d="m6 6 12 12" fill="none" stroke="black" stroke-width="3.6" stroke-linecap="round"/>"#,
    ));
    icon_def!(ARROW_UP_RIGHT, "arrow.up.right", concat!(
        r#"<path d="M7 7h10v10" fill="none" stroke="black" stroke-width="3" stroke-linecap="round" stroke-linejoin="round"/>"#,
        r#"<path d="M7 17 17 7" fill="none" stroke="black" stroke-width="3" stroke-linecap="round"/>"#,
    ));
    icon_def!(CHECK, "checkmark", concat!(
        r#"<path d="M5.5 12.5 10 17l8.5-9.5" fill="none" stroke="black" stroke-width="3.6" stroke-linecap="round" stroke-linejoin="round"/>"#,
    ));
    icon_def!(CHECK_CIRCLE_FILL, "checkmark.circle.fill", concat!(
        r#"<circle cx="12" cy="12" r="10" fill="black"/>"#,
    ));
    icon_def!(MIC, "mic", concat!(
        stroked!(r#"<rect x="9" y="2.5" width="6" height="11.5" rx="3""#),
        stroked!(r#"<path d="M5 10.5v.5a7 7 0 0 0 14 0v-.5""#, r#"<path d="M12 18v3.5""#),
    ));
    icon_def!(CURSOR_CLICK, "cursorarrow.click", concat!(
        r#"<path d="M9 8v12.5l3-2.8 2.1 4.6 2-.9-2.1-4.5 4-.2z" fill="black"/>"#,
        stroked!(r#"<path d="M9 2v2.5""#, r#"<path d="M4.2 4.2 6 6""#, r#"<path d="M2 9h2.5""#, r#"<path d="M13.8 4.2 12 6""#),
    ));
    icon_def!(MIC_FILL, "mic.fill", concat!(
        r#"<rect x="8.5" y="2" width="7" height="12.5" rx="3.5" fill="black"/>"#,
        stroked!(r#"<path d="M5 10.5v.5a7 7 0 0 0 14 0v-.5""#, r#"<path d="M12 18v3.5""#),
    ));
    icon_def!(SCREEN_RECORD, "rectangle.dashed.badge.record", concat!(
        stroked!(
            r#"<path d="M2.5 9V7.5a2 2 0 0 1 2-2H6""#,
            r#"<path d="M9.5 5.5h5""#,
            r#"<path d="M18 5.5h1.5a2 2 0 0 1 2 2V9""#,
            r#"<path d="M2.5 12.5v0""#,
            r#"<path d="M2.5 16v.5a2 2 0 0 0 2 2H6""#,
            r#"<path d="M9.5 18.5h2""#,
        ),
        r#"<circle cx="18.5" cy="16.5" r="4.2" fill="black"/>"#,
    ));
    icon_def!(ACCESSIBILITY, "accessibility", concat!(
        r#"<circle cx="12" cy="4.2" r="2.3" fill="black"/>"#,
        stroked!(
            r#"<path d="M4.5 8.6 12 10l7.5-1.4""#,
            r#"<path d="M12 10v4.2""#,
            r#"<path d="m12 14.2-3.2 6.8""#,
            r#"<path d="m12 14.2 3.2 6.8""#,
        ),
    ));
    icon_def!(IPHONE, "iphone", stroked!(
        r#"<rect x="6" y="2" width="12" height="20" rx="3""#,
        r#"<path d="M10.5 5.5h3""#,
    ));
    icon_def!(WAVEFORM, "waveform", stroked!(
        r#"<path d="M3.5 10.5v3""#,
        r#"<path d="M7.5 7v10""#,
        r#"<path d="M12 3.5v17""#,
        r#"<path d="M16.5 7.5v9""#,
        r#"<path d="M20.5 10.5v3""#,
    ));
    icon_def!(LOCK, "lock", stroked!(
        r#"<rect x="4" y="11" width="16" height="10" rx="2""#,
        r#"<path d="M8 11V7a4 4 0 0 1 8 0v4""#,
    ));
}

// MARK: State (`SettingsState`)

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
struct Login {
    on: bool,
    #[serde(default = "yes")]
    available: bool,
    needs_approval: bool,
}

fn yes() -> bool {
    true
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub(crate) struct Shortcut {
    kind: String,
    title: String,
    label: String,
    key: String,
    shift: bool,
    control: bool,
    option: bool,
    command: bool,
    is_default: bool,
    clearable: bool,
    recording: bool,
    error: Option<String>,
}

impl Default for Shortcut {
    fn default() -> Self {
        Self {
            kind: "record".into(),
            title: "Record".into(),
            label: "⌥⇧S".into(),
            key: "S".into(),
            shift: true,
            control: false,
            option: true,
            command: false,
            is_default: true,
            clearable: false,
            recording: false,
            error: None,
        }
    }
}

impl Shortcut {
    fn paste() -> Self {
        Self {
            kind: "paste".into(),
            title: "Paste last capture".into(),
            label: "⌥⇧V".into(),
            key: "V".into(),
            clearable: true,
            ..Self::default()
        }
    }

    fn off(&self) -> bool {
        self.label.is_empty()
    }

    fn labels(&self) -> Vec<String> {
        if self.off() {
            return Vec::new();
        }
        rune_keys::key_labels(self.control, self.option, self.shift, self.command, &self.key)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
struct ModelOption {
    id: String,
    title: String,
    chosen: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
struct MenuOption {
    id: String,
    title: String,
    #[serde(default = "yes")]
    enabled: bool,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default)]
struct Microphone {
    selected: String,
    value: String,
    options: Vec<MenuOption>,
    note: Option<String>,
    meter: Option<String>,
}

impl Default for Microphone {
    fn default() -> Self {
        Self {
            selected: "default".into(),
            value: "System default".into(),
            options: vec![MenuOption { id: "default".into(), title: "System default".into(), enabled: true }],
            note: None,
            meter: None,
        }
    }
}

/// Apple's speech model (`SpeechStatus`).
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub(crate) struct SpeechStatus {
    pub state: String,
    pub engine: Option<String>,
    pub locale: Option<String>,
    pub progress: f64,
    pub locales: Option<Vec<String>>,
    pub error: Option<String>,
}

impl Default for SpeechStatus {
    fn default() -> Self {
        Self { state: "unknown".into(), engine: None, locale: None, progress: 0.0, locales: None, error: None }
    }
}

impl SpeechStatus {
    fn language(&self) -> String {
        self.locale.as_deref().map_or_else(|| "your language".to_owned(), locale_name)
    }

    pub fn percent_in_parens(&self) -> String {
        if self.progress > 0.0 { format!(" ({}%)", (self.progress * 100.0) as i64) } else { String::new() }
    }

    /// One line for onboarding and Settings, or nothing to say.
    pub fn line(&self) -> Option<String> {
        let percent = if self.progress > 0.0 { format!(" {}%", (self.progress * 100.0) as i64) } else { String::new() };
        Some(match self.state.as_str() {
            "checking" => "Checking for the speech model…".to_owned(),
            "downloading" => format!("Downloading the speech model…{percent}"),
            "ready" => format!("Speech model ready ({})", self.language()),
            "unavailable" => format!("No Apple speech model for {} on this Mac", self.language()),
            "needsPermission" => self
                .error
                .clone()
                .unwrap_or_else(|| "Speech Recognition isn't allowed yet, so captures have no transcript.".to_owned()),
            "failed" if self.engine.as_deref() == Some("SFSpeechRecognizer") && self.error.is_some() => {
                self.error.clone().unwrap_or_default()
            }
            "failed" => "The speech model didn't download. Scrawl will try again.".to_owned(),
            _ => return None,
        })
    }

    pub fn not_ready(&self) -> bool {
        matches!(self.state.as_str(), "checking" | "downloading" | "failed" | "unavailable" | "needsPermission")
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
struct Transcriber {
    engine: String,
    hex_title: String,
    hex_enabled: bool,
    language: String,
    apple_locale: String,
    models: Vec<ModelOption>,
    speech: Option<SpeechStatus>,
    command: Option<String>,
}

impl Default for Transcriber {
    fn default() -> Self {
        Self {
            engine: "apple".into(),
            hex_title: "Hex (not running)".into(),
            hex_enabled: false,
            language: "en".into(),
            apple_locale: "auto".into(),
            models: Vec::new(),
            speech: None,
            command: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
struct RetentionOption {
    id: String,
    label: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default)]
struct Retention {
    preset: Option<String>,
    options: Vec<RetentionOption>,
    note: String,
}

impl Default for Retention {
    fn default() -> Self {
        let o = |id: &str, label: &str| RetentionOption { id: id.into(), label: label.into() };
        Self {
            preset: Some("week".into()),
            options: vec![o("week", "Video for 7 Days"), o("month", "Video for 30 Days"), o("forever", "Forever")],
            note: "Video and audio go to the Trash after 7 days, the rest of each capture after 30 days.".into(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default)]
struct PermissionRow {
    permission: String,
    status: String,
    required: bool,
    requested: bool,
    relaunch: bool,
}

impl Default for PermissionRow {
    fn default() -> Self {
        Self { permission: String::new(), status: "not-determined".into(), required: true, requested: false, relaunch: false }
    }
}

impl PermissionRow {
    fn granted(&self) -> bool {
        self.status == "granted"
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default)]
struct CapturesDir {
    path: String,
    custom: bool,
}

impl Default for CapturesDir {
    fn default() -> Self {
        Self { path: "~/Library/Application Support/scrawl/captures".into(), custom: false }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default)]
struct CommandLine {
    path: Option<String>,
    button: Option<String>,
    enabled: bool,
    caption: String,
    error: bool,
}

impl Default for CommandLine {
    fn default() -> Self {
        Self {
            path: None,
            button: Some("install".into()),
            enabled: true,
            caption: "Links scrawl into /opt/homebrew/bin, so agents and your terminal can run it.".into(),
            error: false,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
struct View {
    version: String,
    login: Login,
    auto_send: bool,
    shortcuts: Vec<Shortcut>,
    microphone: Microphone,
    transcriber: Transcriber,
    video_export: bool,
    lower_playback: bool,
    playback_level: i64,
    retention: Retention,
    captures_size: Option<String>,
    captures_dir: CapturesDir,
    permissions: Vec<PermissionRow>,
    updates: bool,
    command_line: CommandLine,
    skill_command: String,
}

impl Default for View {
    fn default() -> Self {
        let row = |p: &str, required: bool| PermissionRow { permission: p.into(), required, ..PermissionRow::default() };
        Self {
            version: "0.3.2".into(),
            login: Login { on: false, available: true, needs_approval: false },
            auto_send: true,
            shortcuts: vec![Shortcut::default(), Shortcut::paste()],
            microphone: Microphone::default(),
            transcriber: Transcriber::default(),
            video_export: false,
            lower_playback: false,
            playback_level: 20,
            retention: Retention::default(),
            captures_size: None,
            captures_dir: CapturesDir::default(),
            permissions: vec![
                row("screen-recording", true),
                row("microphone", true),
                row("accessibility", true),
                row("camera", false),
            ],
            updates: true,
            command_line: CommandLine::default(),
            skill_command: "bunx skills add kitlangton/scrawl".into(),
        }
    }
}

impl View {
    fn paste_label(&self) -> String {
        self.shortcuts.iter().find(|s| s.kind == "paste").map_or_else(|| "⌥⇧V".to_owned(), |s| s.label.clone())
    }

    fn recording(&self) -> Option<String> {
        self.shortcuts.iter().find(|s| s.recording).map(|s| s.kind.clone())
    }
}

// MARK: Locales

/// The current locale's name for a locale id ("English (United States)"), as
/// `Locale.current.localizedString(forIdentifier:)`.
pub(crate) fn locale_name(id: &str) -> String {
    #[cfg(target_os = "macos")]
    {
        use objc2::msg_send;
        use objc2::runtime::{AnyClass, AnyObject};
        use objc2_foundation::NSString;
        let Some(class) = AnyClass::get(c"NSLocale") else { return id.to_owned() };
        unsafe {
            let current: *mut AnyObject = msg_send![class, currentLocale];
            if current.is_null() {
                return id.to_owned();
            }
            let key = NSString::from_str(id);
            let name: *mut NSString = msg_send![current, localizedStringForLocaleIdentifier: &*key];
            let name = if name.is_null() { String::new() } else { (*name).to_string() };
            if name.is_empty() { id.to_owned() } else { name }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        id.to_owned()
    }
}

fn current_locale() -> String {
    #[cfg(target_os = "macos")]
    {
        use objc2::msg_send;
        use objc2::runtime::{AnyClass, AnyObject};
        use objc2_foundation::NSString;
        let Some(class) = AnyClass::get(c"NSLocale") else { return "en_US".into() };
        unsafe {
            let current: *mut AnyObject = msg_send![class, currentLocale];
            if current.is_null() {
                return "en_US".into();
            }
            let id: *mut NSString = msg_send![current, localeIdentifier];
            if id.is_null() { "en_US".into() } else { (*id).to_string() }
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        "en_US".into()
    }
}

fn same_language(a: &str, b: &str) -> bool {
    let norm = |s: &str| s.replace('_', "-").to_ascii_lowercase();
    let (a, b) = (norm(a), norm(b));
    if a == b {
        return true;
    }
    // "en" matches its most likely region; the region-less side stands for the language.
    let lang = |s: &str| s.split('-').next().unwrap_or_default().to_owned();
    (!a.contains('-') || !b.contains('-')) && lang(&a) == lang(&b)
}

#[derive(Clone, Debug, PartialEq)]
struct Choice {
    id: String,
    title: String,
    enabled: bool,
}

fn choice(id: &str, title: &str, enabled: bool) -> Choice {
    Choice { id: id.into(), title: title.into(), enabled }
}

/// `SettingsTranscriber.languageMenu`: the value shown, the selected id, the options and a note.
fn language_menu(saved: &str, speech: Option<&SpeechStatus>) -> (String, String, Vec<Choice>, Option<String>) {
    let here = current_locale();
    let effective = speech.and_then(|s| s.locale.clone()).unwrap_or_else(|| here.clone());
    let automatic = format!("Automatic ({})", locale_name(if saved == "auto" { &effective } else { &here }));
    let mut supported: Vec<Choice> = speech
        .and_then(|s| s.locales.clone())
        .unwrap_or_default()
        .iter()
        .map(|id| choice(id, &locale_name(id), true))
        .collect();
    supported.sort_by_key(|c| c.title.to_lowercase());
    let mut options = vec![choice("auto", &automatic, true), choice("-", "", false)];
    options.extend(supported.iter().cloned());
    if saved == "auto" {
        return ("Automatic".into(), "auto".into(), options, None);
    }
    if let Some(m) = supported.iter().find(|c| same_language(&c.id, saved)) {
        return (m.title.clone(), m.id.clone(), options, None);
    }
    let unknown = supported.is_empty();
    let title = if unknown { locale_name(saved) } else { format!("{} (not available)", locale_name(saved)) };
    options.push(choice(saved, &title, unknown));
    let note = (!unknown)
        .then(|| format!("{} isn't available on this Mac. Using {}.", locale_name(saved), locale_name(&effective)));
    (title, saved.into(), options, note)
}

// MARK: The window's state

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Pane {
    General,
    Recording,
    Storage,
    Permissions,
    Agents,
    About,
}

impl Pane {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "general" => Self::General,
            "recording" => Self::Recording,
            "storage" => Self::Storage,
            "permissions" => Self::Permissions,
            "agents" => Self::Agents,
            "about" => Self::About,
            _ => return None,
        })
    }

    fn title(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Recording => "Recording",
            Self::Storage => "Storage",
            Self::Permissions => "Permissions",
            Self::Agents => "Agents",
            Self::About => "About",
        }
    }

    fn symbol(self) -> &'static Icon {
        match self {
            Self::General => &icons::GEAR_FILL,
            Self::Recording => &icons::RECORD,
            Self::Storage => &icons::DRIVE_FILL,
            Self::Permissions => &icons::HAND,
            Self::Agents => &icons::TERMINAL,
            Self::About => &icons::INFO,
        }
    }

    fn tint(self) -> (Rgba, Rgba) {
        match self {
            Self::General | Self::About => (Rgba::gray(0.62), Rgba::gray(0.45)),
            Self::Recording => (Rgba::rgb(1.0, 0.42, 0.38), Rgba::rgb(0.92, 0.2, 0.17)),
            Self::Storage => (Rgba::rgb(0.36, 0.66, 1.0), Rgba::rgb(0.16, 0.45, 0.95)),
            Self::Permissions => (Rgba::rgb(0.36, 0.82, 0.56), Rgba::rgb(0.16, 0.66, 0.4)),
            Self::Agents => (Rgba::gray(0.36), Rgba::gray(0.18)),
        }
    }
}

type Events = Arc<dyn Fn(crate::settings::Event) + Send + Sync>;

/// An open pop-up menu: what it sets, its choices, which one is current.
#[derive(Clone, Debug)]
struct Menu {
    anchor: String,
    setting: &'static str,
    options: Vec<Choice>,
    selected: Option<String>,
}

pub(crate) struct SettingsWin {
    view: View,
    pane: Pane,
    keys: HashMap<String, KeyController>,
    now: f64,
    /// Renders: the moment the shortcut keys are drawn at, instead of the clock.
    render_time: Option<f64>,
    reduce: bool,
    hovering: Option<String>,
    menu: Option<Menu>,
    /// The picker whose native menu is open, drawn pressed until it answers.
    popup_open: Option<String>,
    copied_until: f64,
    /// The slider's value while it's dragged; sent when it's let go.
    playback: Option<f64>,
    hex: TextEdit,
    hex_for: String,
    held: Vec<String>,
    focused: bool,
    /// 0…1, the microphone's level while the Recording pane shows (`MicMeter`).
    meter: f32,
    meter_watch: Option<Meter>,
    events: Option<Events>,
    icon: Option<Pixels>,
    /// The session driving the window (`rune_session`).
    session: u64,
}

impl Attached for SettingsWin {
    fn session(&self) -> u64 {
        self.session
    }

    fn set_session(&mut self, id: u64) {
        self.session = id;
    }
}

impl SettingsWin {
    fn new() -> Self {
        Self {
            view: View::default(),
            pane: Pane::General,
            keys: HashMap::new(),
            now: 0.0,
            render_time: None,
            reduce: reduce_motion(),
            hovering: None,
            menu: None,
            popup_open: None,
            copied_until: -1.0,
            playback: None,
            hex: TextEdit::new("en"),
            hex_for: "en".into(),
            held: Vec::new(),
            focused: false,
            meter: 0.0,
            meter_watch: None,
            events: None,
            icon: app_icon(),
            session: 0,
        }
    }

    fn t(&self) -> f64 {
        self.render_time.unwrap_or(self.now)
    }

    fn send(&self, message: Value) {
        let Some(events) = &self.events else { return };
        match serde_json::from_value::<crate::settings::Action>(message.clone()) {
            Ok(action) => events(crate::settings::Event::Action(action)),
            Err(error) => crate::capture::scrawl_log(format!("settings (rune): {error}: {message}")),
        }
    }

    fn act(&self, action: &str) {
        self.send(json!({ "action": action }));
    }

    fn act_kind(&self, action: &str, kind: &str) {
        self.send(json!({ "action": action, "kind": kind }));
    }

    fn set(&self, key: &str, value: Value) {
        self.send(json!({ "action": "set", "key": key, "value": value }));
    }

    /// `SettingsModel.show`.
    fn show(&mut self, view: View, t: f64) {
        for shortcut in &view.shortcuts {
            let labels = shortcut.labels();
            let reduce = self.reduce;
            self.keys
                .entry(shortcut.kind.clone())
                .or_insert_with(|| KeyController::new(FIELD, labels.clone(), reduce, t))
                .update(labels, shortcut.recording, shortcut.error.is_some(), t);
        }
        if view.transcriber.language != self.hex_for || view.transcriber.engine != "hex" {
            self.hex_for = view.transcriber.language.clone();
            self.discard_hex_draft();
        }
        if view.playback_level != self.view.playback_level {
            self.playback = None;
        }
        self.view = view;
    }

    fn hold(&mut self, held: Vec<String>, t: f64) {
        if held == self.held {
            return;
        }
        self.held = held.clone();
        for keys in self.keys.values_mut() {
            keys.hold(held.clone(), t);
        }
    }

    fn type_key(&mut self, kind: &str, label: &str, t: f64) {
        if let Some(keys) = self.keys.get_mut(kind) {
            keys.type_key(label, t);
        }
    }

    fn advance(&mut self, t: f64) {
        for keys in self.keys.values_mut() {
            keys.advance(t);
        }
    }

    fn keys_still(&self, t: f64) -> bool {
        self.keys.values().all(|k| k.motion.still(t))
    }

    /// A message from `settings::Session`.
    fn message(&mut self, message: &Value, tx: &mut Tx) {
        match message["type"].as_str() {
            Some("state") => {
                let Ok(view) = serde_json::from_value::<View>(message.clone()) else {
                    crate::capture::scrawl_log("settings (rune): bad state");
                    return;
                };
                let t = self.now;
                self.show(view, t);
                self.update_meter();
                if self.reduce {
                    tx.motion(Spring::new(0.15, 0.0));
                } else {
                    tx.motion(Spring::new(0.25, 0.0));
                }
                tx.color_motion(Spring::new(0.25, 0.0));
                tx.wake_after(Duration::from_millis(16));
            }
            Some("show") => {
                if let Some(pane) = message["pane"].as_str().and_then(Pane::parse) {
                    self.set_pane(pane);
                    self.update_meter();
                }
            }
            Some("request") => {
                if let Some(p) = message["permission"].as_str() {
                    super::rune_permissions::request(p);
                }
            }
            Some("open-settings") => {
                if let Some(p) = message["permission"].as_str() {
                    super::rune_permissions::open_settings(p, None);
                }
            }
            _ => {}
        }
    }

    fn select(&mut self, pane: Pane) {
        if self.view.recording().is_some() {
            self.act("cancel-shortcut");
        }
        self.menu = None;
        self.set_pane(pane);
        self.update_meter();
    }

    fn set_pane(&mut self, pane: Pane) {
        if self.pane != pane {
            self.discard_hex_draft();
        }
        self.pane = pane;
    }

    /// The Hex language field commits on Return or Apply. As in SwiftUI, where its draft is the
    /// field's own state, an unapplied draft goes once the field does: on a pane switch, when
    /// Hex is no longer the engine, and when the window closes or another session takes it.
    fn discard_hex_draft(&mut self) {
        if self.hex.text != self.hex_for {
            self.hex = TextEdit::new(self.hex_for.clone());
        }
    }

    /// The level shows only while it can be seen: the Recording pane, with Microphone allowed.
    fn update_meter(&mut self) {
        let device = (self.pane == Pane::Recording && self.events.is_some())
            .then(|| self.view.microphone.meter.clone())
            .flatten();
        if self.meter_watch.as_ref().map(|m| m.device.as_str()) != device.as_deref() {
            self.meter_watch = device.and_then(Meter::start);
            self.meter = 0.0;
        }
    }

    /// Keys while a recorder listens (`SettingsWindow.listen`), and held modifiers.
    fn physical(&mut self, event: &PhysicalInput, tx: &mut Tx) -> bool {
        let t = self.now;
        tx.wake_after(Duration::from_millis(16));
        let recording = self.view.recording();
        match *event {
            PhysicalInput::Modifiers(mods) => {
                self.hold(rune_keys::held_labels(mods), t);
                recording.is_some()
            }
            PhysicalInput::FocusLost => {
                if recording.is_some() {
                    self.act("cancel-shortcut");
                }
                false
            }
            PhysicalInput::Key { key, mods, pressed, .. } => {
                let Some(kind) = recording else { return false };
                if is_modifier(key) {
                    return true;
                }
                if !pressed {
                    return true;
                }
                if key == PhysicalKeyCode::Escape {
                    self.act("cancel-shortcut");
                    return true;
                }
                let Some(code) = kvk(key) else { return true };
                let modifiers = carbon(mods);
                let typed = crate::capture::recorder_key(code as u16);
                if let Some(label) = crate::settings::key_label(code, &typed) {
                    self.type_key(&kind, &label, t);
                }
                self.send(json!({
                    "action": "shortcut",
                    "kind": kind,
                    "keyCode": code,
                    "modifiers": modifiers,
                    "key": typed,
                    "reserved": crate::capture::reserved_by_macos(code, modifiers),
                }));
                true
            }
        }
    }

    fn choose_captures_folder(&self) {
        if let Some(path) = choose_folder(&self.view.captures_dir.path) {
            self.set("capturesDir", Value::String(path));
        }
    }
}

pub(crate) fn reduce_motion() -> bool {
    #[cfg(target_os = "macos")]
    {
        super::platform::reduce_motion()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

fn is_modifier(key: PhysicalKeyCode) -> bool {
    use PhysicalKeyCode as K;
    matches!(
        key,
        K::ShiftLeft | K::ShiftRight | K::ControlLeft | K::ControlRight | K::AltLeft | K::AltRight
            | K::SuperLeft | K::SuperRight | K::CapsLock | K::Fn | K::Meta
    )
}

/// Carbon modifier bits (⌘ 1<<8, ⇧ 1<<9, ⌥ 1<<11, ⌃ 1<<12).
fn carbon(mods: rune::Mods) -> u32 {
    let mut bits = 0;
    if mods.cmd || mods.logo {
        bits |= 1 << 8;
    }
    if mods.shift {
        bits |= 1 << 9;
    }
    if mods.alt {
        bits |= 1 << 11;
    }
    if mods.ctrl {
        bits |= 1 << 12;
    }
    bits
}

/// The Carbon virtual key code at a physical key (ANSI positions, as winit names them).
fn kvk(key: PhysicalKeyCode) -> Option<u32> {
    use PhysicalKeyCode as K;
    Some(match key {
        K::KeyA => 0, K::KeyS => 1, K::KeyD => 2, K::KeyF => 3, K::KeyH => 4, K::KeyG => 5,
        K::KeyZ => 6, K::KeyX => 7, K::KeyC => 8, K::KeyV => 9, K::IntlBackslash => 10, K::KeyB => 11,
        K::KeyQ => 12, K::KeyW => 13, K::KeyE => 14, K::KeyR => 15, K::KeyY => 16, K::KeyT => 17,
        K::Digit1 => 18, K::Digit2 => 19, K::Digit3 => 20, K::Digit4 => 21, K::Digit6 => 22,
        K::Digit5 => 23, K::Equal => 24, K::Digit9 => 25, K::Digit7 => 26, K::Minus => 27,
        K::Digit8 => 28, K::Digit0 => 29, K::BracketRight => 30, K::KeyO => 31, K::KeyU => 32,
        K::BracketLeft => 33, K::KeyI => 34, K::KeyP => 35, K::Enter => 36, K::KeyL => 37,
        K::KeyJ => 38, K::Quote => 39, K::KeyK => 40, K::Semicolon => 41, K::Backslash => 42,
        K::Comma => 43, K::Slash => 44, K::KeyN => 45, K::KeyM => 46, K::Period => 47, K::Tab => 48,
        K::Space => 49, K::Backquote => 50, K::Backspace => 51, K::Escape => 53,
        K::NumpadDecimal => 65, K::NumpadMultiply => 67, K::NumpadAdd => 69, K::NumpadDivide => 75,
        K::NumpadEnter => 76, K::NumpadSubtract => 78, K::NumpadEqual => 81, K::Numpad0 => 82,
        K::Numpad1 => 83, K::Numpad2 => 84, K::Numpad3 => 85, K::Numpad4 => 86, K::Numpad5 => 87,
        K::Numpad6 => 88, K::Numpad7 => 89, K::Numpad8 => 91, K::Numpad9 => 92,
        K::F17 => 64, K::F18 => 79, K::F19 => 80, K::F20 => 90, K::F5 => 96, K::F6 => 97, K::F7 => 98,
        K::F3 => 99, K::F8 => 100, K::F9 => 101, K::F11 => 103, K::F13 => 105, K::F16 => 106,
        K::F14 => 107, K::F10 => 109, K::F12 => 111, K::F15 => 113, K::Insert => 114, K::Home => 115,
        K::PageUp => 116, K::Delete => 117, K::F4 => 118, K::End => 119, K::F2 => 120,
        K::PageDown => 121, K::F1 => 122, K::ArrowLeft => 123, K::ArrowRight => 124,
        K::ArrowDown => 125, K::ArrowUp => 126,
        _ => return None,
    })
}

/// CGEventFlags → keycap labels: ⇧ 1<<17, ⌃ 1<<18, ⌥ 1<<19, ⌘ 1<<20.
fn flag_labels(flags: u64) -> Vec<String> {
    rune_keys::held_labels(rune::Mods {
        shift: flags & (1 << 17) != 0,
        ctrl: flags & (1 << 18) != 0,
        alt: flags & (1 << 19) != 0,
        cmd: flags & (1 << 20) != 0,
        logo: false,
    })
}

fn app_icon() -> Option<Pixels> {
    app_icon_px(96)
}

/// Scrawl's icon, `size` pixels square.
pub(crate) fn app_icon_px(size: u32) -> Option<Pixels> {
    static BYTES: &[u8] = include_bytes!("../../assets/app-icon.png");
    static SOURCE: OnceLock<Option<(u32, u32, Vec<u8>)>> = OnceLock::new();
    let (sw, sh, src) = SOURCE
        .get_or_init(|| {
            let decoder = png::Decoder::new(std::io::Cursor::new(BYTES));
            let mut reader = decoder.read_info().ok()?;
            let mut buf = vec![0; reader.output_buffer_size()?];
            let info = reader.next_frame(&mut buf).ok()?;
            if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight {
                return None;
            }
            buf.truncate(info.buffer_size());
            Some((info.width, info.height, buf))
        })
        .as_ref()?;
    let (w, h, rgba) = downscale(src, *sw, *sh, size);
    let pixels = Pixels::new();
    pixels.set(w, h, rgba);
    Some(pixels)
}

/// Area-averaged downscale of premultiplied-on-the-fly RGBA to `size` square.
fn downscale(src: &[u8], w: u32, h: u32, size: u32) -> (u32, u32, Vec<u8>) {
    if w <= size {
        return (w, h, src.to_vec());
    }
    let (w, h) = (w as usize, h as usize);
    let n = size as usize;
    let mut out = vec![0u8; n * n * 4];
    for oy in 0..n {
        let (y0, y1) = (oy * h / n, ((oy + 1) * h / n).max(oy * h / n + 1));
        for ox in 0..n {
            let (x0, x1) = (ox * w / n, ((ox + 1) * w / n).max(ox * w / n + 1));
            let mut acc = [0f64; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y * w + x) * 4;
                    let a = f64::from(src[i + 3]) / 255.0;
                    acc[0] += f64::from(src[i]) * a;
                    acc[1] += f64::from(src[i + 1]) * a;
                    acc[2] += f64::from(src[i + 2]) * a;
                    acc[3] += a;
                }
            }
            let count = ((y1 - y0) * (x1 - x0)) as f64;
            let o = (oy * n + ox) * 4;
            if acc[3] > 0.0 {
                for c in 0..3 {
                    out[o + c] = (acc[c] / acc[3]).round().clamp(0.0, 255.0) as u8;
                }
            }
            out[o + 3] = (acc[3] / count * 255.0).round().clamp(0.0, 255.0) as u8;
        }
    }
    (size, size, out)
}

/// Change… under Storage: a folder picker (`NSOpenPanel`), run modally like SwiftUI's.
fn choose_folder(current: &str) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        use objc2::msg_send;
        use objc2::runtime::{AnyClass, AnyObject, Bool};
        use objc2_foundation::NSString;
        let class = AnyClass::get(c"NSOpenPanel")?;
        unsafe {
            let panel: *mut AnyObject = msg_send![class, openPanel];
            if panel.is_null() {
                return None;
            }
            let _: () = msg_send![panel, setCanChooseFiles: Bool::NO];
            let _: () = msg_send![panel, setCanChooseDirectories: Bool::YES];
            let _: () = msg_send![panel, setCanCreateDirectories: Bool::YES];
            let _: () = msg_send![panel, setAllowsMultipleSelection: Bool::NO];
            let prompt = NSString::from_str("Choose");
            let _: () = msg_send![panel, setPrompt: &*prompt];
            let message =
                NSString::from_str("New captures go to this folder. Captures already made stay where they are.");
            let _: () = msg_send![panel, setMessage: &*message];
            let expanded = match current.strip_prefix("~/") {
                Some(rest) => dirs::home_dir().unwrap_or_default().join(rest).to_string_lossy().into_owned(),
                None => current.to_owned(),
            };
            if let Some(parent) = std::path::Path::new(&expanded).parent()
                && let Some(url_class) = AnyClass::get(c"NSURL")
            {
                let path = NSString::from_str(&parent.to_string_lossy());
                let url: *mut AnyObject = msg_send![url_class, fileURLWithPath: &*path];
                if !url.is_null() {
                    let _: () = msg_send![panel, setDirectoryURL: url];
                }
            }
            if let Some(app_class) = AnyClass::get(c"NSApplication") {
                let app: *mut AnyObject = msg_send![app_class, sharedApplication];
                let _: () = msg_send![app, activateIgnoringOtherApps: Bool::YES];
            }
            let response: isize = msg_send![panel, runModal];
            if response != 1 {
                return None;
            }
            let url: *mut AnyObject = msg_send![panel, URL];
            if url.is_null() {
                return None;
            }
            let path: *mut NSString = msg_send![url, path];
            (!path.is_null()).then(|| (*path).to_string())
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = current;
        None
    }
}

fn open_url(url: &str) {
    let _ = std::process::Command::new("/usr/bin/open").arg(url).spawn();
}

// MARK: Microphone level

/// Meters one input device with cpal while the Recording pane shows: 0…1 over −50…0 dBFS,
/// rising at once and falling 0.035 per 30 Hz tick (`MicMeter`).
struct Meter {
    device: String,
    level: Arc<std::sync::atomic::AtomicU32>,
    _stream: Option<cpal::Stream>,
}

impl Meter {
    fn start(device: String) -> Option<Self> {
        use cpal::traits::{DeviceTrait, StreamTrait};
        let level = Arc::new(std::sync::atomic::AtomicU32::new(0));
        let input = crate::mic::device(&device)?;
        let config = input.default_input_config().ok()?;
        if config.sample_format() != cpal::SampleFormat::F32 {
            return None;
        }
        let peak = Arc::clone(&level);
        let stream = input
            .build_input_stream(
                config.into(),
                move |data: &[f32], _| {
                    let rms = (data.iter().map(|s| s * s).sum::<f32>() / data.len().max(1) as f32).sqrt();
                    let db = 20.0 * rms.max(1e-9).log10();
                    let target = ((db + 50.0) / 50.0).clamp(0.0, 1.0);
                    let old = f32::from_bits(peak.load(std::sync::atomic::Ordering::Relaxed));
                    if target > old {
                        peak.store(target.to_bits(), std::sync::atomic::Ordering::Relaxed);
                    }
                },
                |_| {},
                None,
            )
            .ok()?;
        stream.play().ok()?;
        Some(Self { device, level, _stream: Some(stream) })
    }

    /// The loudest moment since the last read.
    fn take(&self) -> f32 {
        f32::from_bits(self.level.swap(0, std::sync::atomic::Ordering::Relaxed))
    }
}

// MARK: Views

const WIDTH: f32 = 620.0;
const HEIGHT: f32 = 448.0;
const SIDEBAR: f32 = 210.0;
const TITLEBAR: f32 = 52.0;
const INSET: f32 = 20.0;
const ROW: f32 = 42.0;
const RADIUS: f32 = 12.0;

fn tile(sym: &'static Icon, tint: (Rgba, Rgba), size: f32) -> El {
    el().size(size, size)
        .radius(size * 0.27)
        .gradient(tint.0.color(), tint.1.color())
        .border(0.5, Rgba::WHITE.alpha(0.14).over(tint.0.mix(tint.1, 0.5)))
        .center()
        .child(symbol(sym, size * 0.5, weight::SEMIBOLD, Color::WHITE))
}

fn label(s: &str, size: f32, weight: f32, color: Color) -> El {
    text(s).font(style(size, weight)).color(color).single_line()
}

fn switch(on: bool, enabled: bool, theme: Theme, bg: Rgba, key: &str, f: impl Fn(&mut SettingsWin) + 'static) -> El {
    let fill = if on { theme.accent() } else { theme.track() };
    el().key(key.to_owned())
        .size(38.0, 22.0)
        .radius(11.0)
        .fill(fill.over(bg))
        .opacity(if enabled { 1.0 } else { 0.4 })
        .cursor(rune::Cursor::Pointer)
        .child(
            el().key(format!("{key}-knob"))
                .absolute(if on { 18.0 } else { 2.0 }, 2.0)
                .size(18.0, 18.0)
                .radius(9.0)
                .fill(Color::WHITE)
                .box_shadow(rune::Shadow { x: 0.0, y: 0.5, blur: 2.0, spread: 0.0, color: Rgba::BLACK.alpha(0.22).color(), inset: false })
                .motion(Spring::new(0.25, 0.0)),
        )
        .on_press(move |s: &mut SettingsWin| {
            if enabled {
                f(s);
            }
        })
}

fn button(title: &str, enabled: bool, theme: Theme, bg: Rgba, f: impl Fn(&mut SettingsWin, &mut Tx) + 'static) -> El {
    let fill = theme.control().over_rgba(bg);
    let e = el().h(24.0)
        .px(11.0)
        .center()
        .radius(12.0)
        .fill(fill.color())
        .border(1.0, theme.control_line().over(fill))
        .opacity(if enabled { 1.0 } else { 0.45 })
        .child(label(title, 12.0, 500.0, theme.text().over(fill)));
    if enabled {
        e.hover(theme.ink(if theme.dark { 0.13 } else { 0.1 }).over(bg))
            .press(theme.ink(if theme.dark { 0.18 } else { 0.14 }).over(bg))
            .on_press_tx(f)
    } else {
        e
    }
}

fn circle_button(sym: &'static Icon, glyph: f32, theme: Theme, bg: Rgba, key: String, f: impl Fn(&mut SettingsWin) + 'static) -> El {
    let fill = theme.control().over_rgba(bg);
    el().key(key)
        .size(20.0, 20.0)
        .radius(10.0)
        .fill(fill.color())
        .center()
        .hover(theme.ink(if theme.dark { 0.14 } else { 0.1 }).over(bg))
        .child(symbol(sym, glyph, weight::BOLD, theme.secondary().over(fill)))
        .on_press(f)
        .transition(replace())
}

fn group(theme: Theme, rows: Vec<El>) -> El {
    col().w_full()
        .radius(RADIUS)
        .fill(theme.group_bg().color())
        .border(1.0, theme.group_line().over(theme.group_bg()))
        .clip_rounded(RADIUS)
        .children(rows)
}

fn separator(theme: Theme) -> El {
    el().w_full().h(1.0).fill(theme.separator().over(theme.group_bg()))
}

/// Rows separated by hairlines.
fn rows(theme: Theme, items: Vec<El>) -> Vec<El> {
    let mut out = Vec::new();
    for (i, item) in items.into_iter().enumerate() {
        if i > 0 {
            out.push(separator(theme));
        }
        out.push(item);
    }
    out
}

fn setting_row(title: &str, enabled: bool, theme: Theme, trailing: Vec<El>) -> El {
    let bg = theme.group_bg();
    row().w_full()
        .h(ROW)
        .px(12.0)
        .gap(12.0)
        .center_y()
        .child(label(title, 14.0, 400.0, if enabled { theme.text().over(bg) } else { theme.secondary().over(bg) }))
        .child(spacer().min_w(8.0))
        .children(trailing)
}

fn header(title: &str, theme: Theme) -> El {
    el().w_full().pt(20.0).pb(6.0).child(
        row().w_full().h(20.0).px(8.0).center_y().child(label(title, 13.0, 600.0, theme.secondary().over(theme.window()))),
    )
}

fn caption_style() -> TextStyle {
    style(12.0, 400.0).lh(17.0)
}

fn caption(s: &str, error: bool, theme: Theme) -> El {
    let color = if error { theme.negative() } else { theme.secondary() };
    // SwiftUI's lineSpacing(2) goes only between lines; Rune's line height puts half above the
    // first line and half below the last.
    el().w_full().px(8.0).pt(7.0).child(
        text(s).font(caption_style()).color(color.over(theme.window())).wrap().w_full().mt(-1.0).mb(-1.0),
    )
}

/// `SettingsMotion.replace`: a caption replaced in place, blurring a little as it fades.
fn replace() -> Transition {
    Transition::blur(1.5).springs(SpringPair::new(Spring::new(0.25, 0.0), Spring::new(0.18, 0.0)))
}

fn popup(
    s: &SettingsWin,
    theme: Theme,
    anchor: &str,
    setting: &'static str,
    value: &str,
    options: Vec<Choice>,
    selected: Option<String>,
) -> El {
    let bg = theme.group_bg();
    let open = s.menu.as_ref().is_some_and(|m| m.anchor == anchor) || s.popup_open.as_deref() == Some(anchor);
    let anchor_key = anchor.to_owned();
    row().key(anchor.to_owned())
        .gap(8.0)
        .center_y()
        .child(
            text(middle_fit(value, style(14.0, 400.0), 190.0))
                .font(style(14.0, 400.0))
                .color(theme.text().alpha(0.85).over(bg))
                .single_line()
                .truncate()
                .max_w(190.0),
        )
        .child(
            el().size(20.0, 20.0)
                .radius(10.0)
                .border(1.0, theme.control_line().over(bg))
                .fill(if open { theme.control().over(bg) } else { bg.color() })
                .center()
                .child(symbol(&icons::CHEVRONS, 9.0, weight::BOLD, theme.secondary().over(bg))),
        )
        .on_press_tx(move |s: &mut SettingsWin, tx: &mut Tx| {
            if rune::popup_menu_supported() {
                s.menu = None;
                s.popup_open = Some(anchor_key.clone());
                tx.popup_menu(native_menu(&options, selected.as_deref()), anchor_key.clone(), move |s: &mut SettingsWin, picked, _| {
                    s.popup_open = None;
                    if let Some(id) = picked {
                        s.set(setting, Value::String(id.to_owned()));
                    }
                });
            } else if s.menu.as_ref().is_some_and(|m| m.anchor == anchor_key) {
                s.menu = None;
            } else {
                s.menu = Some(Menu {
                    anchor: anchor_key.clone(),
                    setting,
                    options: options.clone(),
                    selected: selected.clone(),
                });
            }
        })
}

/// The native menu for a picker: the current choice checked and opening over the control, as
/// SwiftUI's menu-style `Picker` does. `"-"` is a separator.
fn native_menu(options: &[Choice], selected: Option<&str>) -> PopupMenu {
    let mut menu = PopupMenu::new();
    for option in options {
        menu = if option.id == "-" {
            menu.separator()
        } else {
            menu.item(
                PopupItem::new(&option.id, &option.title)
                    .enabled(option.enabled)
                    .checked(selected == Some(option.id.as_str())),
            )
        };
    }
    match selected {
        Some(id) => menu.selected(id),
        None => menu,
    }
}

/// The drawn menu where native pop-up menus aren't available: a check beside the current choice.
fn menu_view(menu: &Menu, theme: Theme) -> El {
    let dark = theme.dark;
    let bg = if dark { Rgba::gray(0.2) } else { Rgba::gray(0.985) };
    let text_c = if dark { Rgba::gray(0.95) } else { Rgba::gray(0.1) };
    let mut items = Vec::new();
    for option in &menu.options {
        if option.id == "-" {
            items.push(el().w_full().py(4.0).px(10.0).child(el().w_full().h(1.0).fill(theme.separator().over(bg))));
            continue;
        }
        let chosen = menu.selected.as_deref() == Some(option.id.as_str());
        let id = option.id.clone();
        let setting = menu.setting;
        let color = if option.enabled { text_c } else { text_c.alpha(0.35) };
        let mut item = row()
            .w_full()
            .h(24.0)
            .pl(6.0)
            .pr(14.0)
            .gap(4.0)
            .radius(5.0)
            .center_y()
            .child(el().size(16.0, 16.0).center().child(if chosen {
                symbol(&icons::CHECK, 11.0, weight::SEMIBOLD, color.over(bg))
            } else {
                el()
            }))
            .child(label(&option.title, 13.0, 400.0, color.over(bg)));
        if option.enabled {
            item = item.hover(theme.accent().color()).on_press(move |s: &mut SettingsWin| {
                s.menu = None;
                s.set(setting, Value::String(id.clone()));
            });
        }
        items.push(item);
    }
    col().key("menu")
        .anchored(menu.anchor.clone(), Side::Bottom, Align::End, 6.0)
        .min_w(180.0)
        .padding(5.0)
        .radius(9.0)
        .fill(bg.color())
        .border(1.0, (if dark { Rgba::WHITE.alpha(0.12) } else { Rgba::BLACK.alpha(0.1) }).over(bg))
        .box_shadow(rune::Shadow { x: 0.0, y: 8.0, blur: 24.0, spread: 0.0, color: Rgba::BLACK.alpha(if dark { 0.45 } else { 0.18 }).color(), inset: false })
        .children(items)
        .modal(true, |s: &mut SettingsWin| s.menu = None)
        .transition(Transition::opacity().springs(SpringPair::new(Spring::new(0.15, 0.0), Spring::new(0.12, 0.0))))
}

fn level_meter(level: f32, theme: Theme) -> El {
    let bg = theme.group_bg();
    let lit = (level * 16.0).round() as usize;
    row().gap(3.0).label(format!("Input level, {} percent", (level * 100.0) as i32)).children((0..16).map(|i| {
        el().size(5.0, 12.0)
            .radius(1.5)
            .fill(if i < lit { theme.text().alpha(0.7).over(bg) } else { theme.track().over(bg) })
    }))
}

fn recorder(s: &SettingsWin, shortcut: &Shortcut, theme: Theme) -> El {
    let bg = theme.group_bg();
    let kind = shortcut.kind.clone();
    let t = s.t();
    let hovering = s.hovering.as_deref() == Some(kind.as_str());
    let mut line = row()
        .key(format!("recorder-{kind}"))
        .w_full()
        .h(50.0)
        .pl(12.0)
        .pr(8.0)
        .gap(8.0)
        .center_y()
        .child(label(&shortcut.title, 14.0, 400.0, theme.text().over(bg)))
        .child(spacer().min_w(8.0));
    if !shortcut.is_default && !shortcut.recording {
        let k = kind.clone();
        let default = if kind == "paste" { "⌥⇧V" } else { "⌥⇧S" };
        line = line.child(
            circle_button(&icons::RESET, 9.0, theme, bg, format!("reset-{kind}"), move |s| s.act_kind("reset-shortcut", &k))
                .label(format!("Reset to {default}"))
                .help(format!("Reset to {default}")),
        );
    }
    if shortcut.clearable && !shortcut.off() && !shortcut.recording {
        let k = kind.clone();
        line = line.child(
            circle_button(&icons::X, 8.0, theme, bg, format!("clear-{kind}"), move |s| s.act_kind("clear-shortcut", &k))
                .label(format!("Turn off {} shortcut", shortcut.title))
                .help("Turn off"),
        );
    }
    if let Some(keys) = s.keys.get(&kind) {
        line = line.child(rune_keys::field(&keys.motion, t, theme, hovering));
    }
    let (k_press, k_hover) = (kind.clone(), kind.clone());
    let recording = shortcut.recording;
    let value = if recording { "Listening" } else if shortcut.off() { "Off" } else { shortcut.label.as_str() };
    line = line
        .label(format!("{} shortcut, {value}", shortcut.title))
        .focusable(true)
        .focus_ring(0.0, 0.0, Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 })
        .on_press(move |s: &mut SettingsWin| {
            s.menu = None;
            s.act_kind(if recording { "cancel-shortcut" } else { "record-shortcut" }, &k_press);
        })
        .on_hover_change(move |s: &mut SettingsWin, on: bool, tx: &mut Tx| {
            if on {
                s.hovering = Some(k_hover.clone());
            } else if s.hovering.as_deref() == Some(k_hover.as_str()) {
                s.hovering = None;
            }
            tx.instant();
        })
        .on_physical_input(|s: &mut SettingsWin, event: &PhysicalInput, tx: &mut Tx| s.physical(event, tx));
    let text_caption = if shortcut.recording {
        if shortcut.clearable { "Esc cancels. Delete turns it off." } else { "Esc cancels." }.to_owned()
    } else if let Some(error) = &shortcut.error {
        error.clone()
    } else if shortcut.kind == "paste" {
        "Pastes the last capture into the app in front.".to_owned()
    } else {
        "Records the screen. Press again to finish.".to_owned()
    };
    let error = shortcut.error.is_some() && !shortcut.recording;
    col().w_full()
        .child(group(theme, vec![line]))
        .child(
            el().w_full().h(26.0).child(
                el().key(format!("caption-{kind}-{text_caption}"))
                    .absolute(0.0, 0.0)
                    .w_full()
                    .child(caption(&text_caption, error, theme))
                    .transition(replace()),
            ),
        )
}

fn playback_row(s: &SettingsWin, theme: Theme) -> El {
    let bg = theme.group_bg();
    let value = s.playback.unwrap_or(s.view.playback_level as f64).clamp(0.0, 100.0);
    let w = 150.0_f32;
    let x = w * value as f32 / 100.0;
    let slider = el()
        .size(w, 20.0)
        .child(
            el().absolute(0.0, 8.0)
                .size(w, 4.0)
                .radius(2.0)
                .fill(theme.track().over(bg))
                .child(el().size(x, 4.0).radius(2.0).fill(theme.accent().color())),
        )
        .child(
            el().absolute((x - 9.0).clamp(0.0, w - 18.0), 1.0)
                .size(18.0, 18.0)
                .radius(9.0)
                .fill(Color::WHITE)
                .border(0.5, Rgba::BLACK.alpha(0.12).over(Rgba::WHITE))
                .box_shadow(rune::Shadow { x: 0.0, y: 0.5, blur: 2.0, spread: 0.0, color: Rgba::BLACK.alpha(0.25).color(), inset: false }),
        )
        .cursor(rune::Cursor::Pointer)
        .on_drag(move |s: &mut SettingsWin, d: &mut rune::Drag, tx: &mut Tx| {
            let v = f64::from((d.x / d.w).clamp(0.0, 1.0) * 100.0).round();
            tx.instant();
            match d.phase {
                Phase::Down | Phase::Move => s.playback = Some(v),
                Phase::Up => {
                    s.playback = Some(v);
                    s.set("playbackLevel", json!(v as i64));
                }
            }
        });
    setting_row(
        "Lower to",
        true,
        theme,
        vec![
            slider,
            text(format!("{}%", value.round() as i64))
                .font(digits())
                .color(theme.secondary().over(bg))
                .single_line()
                .w(40.0)
                .align(Align::End),
        ],
    )
}

fn hex_language(s: &SettingsWin, theme: Theme) -> El {
    let bg = theme.group_bg();
    let draft = s.hex.text.trim().to_lowercase();
    let valid = draft.is_empty() || valid_language(&draft);
    let changed = (if draft.is_empty() { "en" } else { draft.as_str() }) != s.view.transcriber.language;
    let field_bg = if theme.dark { Rgba::WHITE.alpha(0.06).over_rgba(bg) } else { Rgba::WHITE };
    let field_style = style(13.0, 400.0);
    let line = field_style.line_px();
    let field = text_input(&s.hex, field_style)
        .single_line()
        .placeholder("en", theme.tertiary().over(field_bg))
        .caret(theme.accent().color(), theme.accent().alpha(0.3).color())
        .color(theme.text().over(field_bg))
        .on_edit(|s: &mut SettingsWin| &mut s.hex)
        .on_submit(|s: &mut SettingsWin, _tx: &mut Tx| apply_hex(s))
        .label("Hex language code")
        .key("hex-language")
        .w(120.0)
        .h(22.0)
        .px(6.0)
        .pt(((22.0 - line) / 2.0).max(0.0))
        .radius(5.0)
        .fill(field_bg.color())
        .border(1.0, theme.control_line().over(field_bg));
    setting_row("Hex language", true, theme, vec![field, system_button("Apply", valid && changed, theme, bg, apply_hex)]).help(
        "Language code, e.g. en, nl or auto. Support depends on the Hex model; some ignore it. This does not follow Hex’s dictation language.",
    )
}

/// A plain SwiftUI `Button` (bordered, no custom style): unlike `SettingsButton` it greys its
/// fill and title when disabled rather than fading the whole capsule.
fn system_button(title: &str, enabled: bool, theme: Theme, bg: Rgba, f: fn(&mut SettingsWin)) -> El {
    let dark = theme.dark;
    let fill = if enabled { theme.control().over_rgba(bg) } else { theme.ink(if dark { 0.05 } else { 0.04 }).over_rgba(bg) };
    let ink = if enabled { theme.text().over(fill) } else { theme.ink(if dark { 0.25 } else { 0.3 }).over(fill) };
    let e = el().h(24.0).px(11.0).center().radius(7.0).fill(fill.color()).child(label(title, 13.0, 400.0, ink));
    if enabled {
        e.border(1.0, theme.control_line().over(fill))
            .hover(theme.ink(if dark { 0.13 } else { 0.1 }).over(bg))
            .press(theme.ink(if dark { 0.18 } else { 0.14 }).over(bg))
            .on_press(f)
    } else {
        e
    }
}

fn valid_language(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && s.len() <= 35
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn apply_hex(s: &mut SettingsWin) {
    let draft = s.hex.text.trim().to_lowercase();
    if draft.is_empty() || valid_language(&draft) {
        s.set("hexLanguage", Value::String(draft));
    }
}

fn general(s: &SettingsWin, theme: Theme) -> Vec<El> {
    let v = &s.view;
    let bg = theme.group_bg();
    let mut items = vec![
        setting_row(
            "Launch at login",
            v.login.available,
            theme,
            vec![switch(v.login.on, v.login.available, theme, bg, "login", |s| s.set("launchAtLogin", json!(!s.view.login.on)))],
        ),
        setting_row(
            "Send dev-site captures to OpenCode",
            true,
            theme,
            vec![switch(v.auto_send, true, theme, bg, "autosend", |s| s.set("autoSend", json!(!s.view.auto_send)))],
        ),
        setting_row(
            "Lower other audio while recording",
            true,
            theme,
            vec![switch(v.lower_playback, true, theme, bg, "lower", |s| s.set("lowerPlayback", json!(!s.view.lower_playback)))],
        ),
    ];
    if v.lower_playback {
        items.push(playback_row(s, theme));
    }
    let mut out = vec![group(theme, rows(theme, items))];
    if v.login.needs_approval {
        out.push(caption("Allow Scrawl in System Settings › General › Login Items.", false, theme));
    } else if !v.login.available {
        out.push(caption("Launch at login works in the installed Scrawl.app.", false, theme));
    }
    for shortcut in &v.shortcuts {
        out.push(el().h(14.0));
        out.push(recorder(s, shortcut, theme));
    }
    out
}

fn recording(s: &SettingsWin, theme: Theme) -> Vec<El> {
    let v = &s.view;
    let t = &v.transcriber;
    let (language_value, language_selected, language_options, language_note) =
        language_menu(&t.apple_locale, t.speech.as_ref());
    let mut engines = vec![choice("apple", "Apple Speech", true), choice("hex", &t.hex_title, t.hex_enabled)];
    if let Some(command) = &t.command {
        engines.push(choice("command", &format!("Command ({command})"), true));
    }
    let engine_title = engines.iter().find(|e| e.id == t.engine).map_or("Apple Speech".to_owned(), |e| e.title.clone());
    let mic = &v.microphone;
    let mut mic_rows = vec![setting_row(
        "Microphone",
        true,
        theme,
        vec![popup(
            s,
            theme,
            "pop-mic",
            "microphone",
            &mic.value,
            mic.options.iter().map(|o| choice(&o.id, &o.title, o.enabled)).collect(),
            Some(mic.selected.clone()),
        )],
    )];
    if mic.meter.is_some() {
        mic_rows.push(setting_row("Input level", true, theme, vec![level_meter(s.meter, theme)]));
    }
    let mut out = vec![group(theme, rows(theme, mic_rows))];
    if let Some(note) = &mic.note {
        out.push(caption(note, false, theme));
    }
    out.push(el().h(20.0));
    let mut engine_rows = vec![setting_row(
        "Transcription",
        true,
        theme,
        vec![popup(s, theme, "pop-engine", "transcriber", &engine_title, engines.clone(), Some(t.engine.clone()))],
    )];
    if t.engine == "apple" {
        engine_rows.push(setting_row(
            "Language",
            true,
            theme,
            vec![popup(s, theme, "pop-language", "appleLocale", &language_value, language_options, Some(language_selected))],
        ));
    }
    if t.engine == "hex" && !t.models.is_empty() {
        let chosen = t.models.iter().find(|m| m.chosen);
        engine_rows.push(setting_row(
            "Hex model",
            true,
            theme,
            vec![popup(
                s,
                theme,
                "pop-model",
                "hexModel",
                chosen.map_or("Choose", |m| m.title.as_str()),
                t.models.iter().map(|m| choice(&m.id, &m.title, true)).collect(),
                chosen.map(|m| m.id.clone()),
            )],
        ));
    }
    if t.engine == "hex" {
        engine_rows.push(hex_language(s, theme));
    }
    out.push(group(theme, rows(theme, engine_rows)));
    out.push(caption(
        if t.engine == "command" {
            "Command runs transcribeCommand from config.json. If it fails, that recording uses Apple Speech."
        } else if t.hex_enabled || t.engine == "hex" {
            "Apple Speech times each word. Hex uses a model you have in Hex, and its word times are estimates."
        } else {
            "Apple Speech runs on this Mac and times each word. Hex shows up here while Hex is running."
        },
        false,
        theme,
    ));
    if t.engine == "apple"
        && let Some(note) = &language_note
    {
        out.push(caption(note, false, theme));
    }
    if t.engine == "apple"
        && let Some(speech) = &t.speech
        && speech.not_ready()
        && let Some(line) = speech.line()
    {
        out.push(caption(&line, false, theme));
    }
    out.push(header("Experimental", theme));
    out.push(group(
        theme,
        vec![setting_row(
            "Video export",
            true,
            theme,
            vec![switch(v.video_export, true, theme, theme.group_bg(), "video", |s| {
                s.set("videoExport", json!(!s.view.video_export))
            })],
        )],
    ));
    out.push(caption("Adds Export Last Capture as Video… to the menu.", false, theme));
    out
}

/// A line's width in points as Rune lays it out at 2x.
pub(crate) fn measure(s: &str, style: TextStyle) -> f32 {
    thread_local! {
        static TEXT: std::cell::RefCell<Option<rune::text::TextSystem>> = const { std::cell::RefCell::new(None) };
    }
    TEXT.with(|t| {
        let mut t = t.borrow_mut();
        let fonts = fonts();
        t.get_or_insert_with(|| rune::text::TextSystem::new(fonts, families(fonts))).measure(s, style, 2.0).0
    })
}

/// `s` cut at the front with "…" to fit `width` (SwiftUI's `.truncationMode(.head)`, which
/// Rune lacks).
fn head_fit(s: &str, style: TextStyle, width: f32) -> String {
    if measure(s, style) <= width {
        return s.to_owned();
    }
    let chars: Vec<char> = s.chars().collect();
    let (mut lo, mut hi) = (0, chars.len());
    while lo < hi {
        let keep = (lo + hi).div_ceil(2);
        let tail: String = chars[chars.len() - keep..].iter().collect();
        if measure(&format!("…{tail}"), style) <= width { lo = keep } else { hi = keep - 1 }
    }
    format!("…{}", chars[chars.len() - lo..].iter().collect::<String>())
}

/// `s` cut in the middle with "…" to fit `width` (`.truncationMode(.middle)`).
fn middle_fit(s: &str, style: TextStyle, width: f32) -> String {
    if measure(s, style) <= width {
        return s.to_owned();
    }
    let chars: Vec<char> = s.chars().collect();
    let cut = |keep: usize| {
        let head: String = chars[..keep.div_ceil(2)].iter().collect();
        let tail: String = chars[chars.len() - keep / 2..].iter().collect();
        format!("{head}…{tail}")
    };
    let (mut lo, mut hi) = (0, chars.len());
    while lo < hi {
        let keep = (lo + hi).div_ceil(2);
        if measure(&cut(keep), style) <= width { lo = keep } else { hi = keep - 1 }
    }
    cut(lo)
}

/// What a `button` labelled `title` takes in a row.
fn button_w(title: &str) -> f32 {
    measure(title, style(12.0, 500.0)) + 22.0
}

/// The text column's room in a group row ahead of a spacer and `trailing` widths.
fn room_beside(trailing: &[f32]) -> f32 {
    let row = WIDTH - SIDEBAR - 1.0 - 2.0 * INSET - 24.0;
    row - 12.0 - 8.0 - trailing.iter().map(|w| w + 12.0).sum::<f32>()
}

fn two_line(title: &str, detail: Option<&str>, theme: Theme, title_weight: f32, detail_style: TextStyle) -> El {
    let bg = theme.group_bg();
    let mut c = col().gap(1.0).shrink().min_w(0.0).child(label(title, 14.0, title_weight, theme.text().over(bg)));
    if let Some(detail) = detail {
        c = c.child(text(detail).font(detail_style).color(theme.secondary().over(bg)).single_line().truncate());
    }
    c
}

fn storage(s: &SettingsWin, theme: Theme) -> Vec<El> {
    let v = &s.view;
    let bg = theme.group_bg();
    let r = &v.retention;
    let current = r.options.iter().find(|o| Some(&o.id) == r.preset.as_ref()).map_or("Custom".to_owned(), |o| o.label.clone());
    let mut out = vec![group(
        theme,
        vec![setting_row(
            "Keep recordings",
            true,
            theme,
            vec![popup(
                s,
                theme,
                "pop-retention",
                "retention",
                &current,
                r.options.iter().map(|o| choice(&o.id, &o.label, true)).collect(),
                r.preset.clone(),
            )],
        )],
    )];
    if !r.note.is_empty() {
        out.push(caption(&r.note, false, theme));
    }
    out.push(header("Captures", theme));
    let mut location = row()
        .w_full()
        .h(52.0)
        .px(12.0)
        .gap(12.0)
        .center_y()
        .child({
            let mut trailing = vec![button_w("Change…")];
            if v.captures_dir.custom {
                trailing.push(button_w("Reset"));
            }
            let path = head_fit(&v.captures_dir.path, MONO, room_beside(&trailing));
            two_line("Location", Some(&path), theme, 400.0, MONO)
        })
        .child(spacer().min_w(8.0));
    if v.captures_dir.custom {
        location = location.child(button("Reset", true, theme, bg, |s, _| s.set("capturesDir", json!(""))));
    }
    location = location.child(button("Change…", true, theme, bg, |s, _| s.choose_captures_folder()));
    out.push(group(
        theme,
        rows(
            theme,
            vec![
                location,
                setting_row(
                    "Size",
                    true,
                    theme,
                    vec![
                        text(v.captures_size.clone().unwrap_or_else(|| "Measuring…".to_owned()))
                            .font(digits())
                            .color(theme.secondary().over(bg))
                            .single_line(),
                        button("Show in Finder", true, theme, bg, |s, _| s.act("show-captures")),
                    ],
                ),
            ],
        ),
    ));
    out
}

pub(crate) fn permission_symbol(p: &str) -> &'static Icon {
    match p {
        "screen-recording" => &icons::SCREEN_RECORD,
        "microphone" => &icons::MIC,
        "accessibility" => &icons::ACCESSIBILITY,
        "camera" => &icons::IPHONE,
        "speech-recognition" => &icons::WAVEFORM,
        _ => &icons::LOCK,
    }
}

fn permission_tint(p: &str) -> (Rgba, Rgba) {
    match p {
        "screen-recording" => (Rgba::rgb(0.62, 0.5, 1.0), Rgba::rgb(0.44, 0.32, 0.92)),
        "microphone" => (Rgba::rgb(1.0, 0.62, 0.3), Rgba::rgb(0.95, 0.44, 0.12)),
        "accessibility" => (Rgba::rgb(0.36, 0.66, 1.0), Rgba::rgb(0.16, 0.45, 0.95)),
        "camera" => (Rgba::rgb(0.36, 0.82, 0.56), Rgba::rgb(0.16, 0.66, 0.4)),
        "speech-recognition" => (Rgba::rgb(1.0, 0.42, 0.56), Rgba::rgb(0.9, 0.24, 0.4)),
        _ => (Rgba::gray(0.62), Rgba::gray(0.45)),
    }
}

/// `checkmark.circle.fill` at `point` (medium weight) in `color`.
pub(crate) fn check_disc(point: f32, color: Rgba) -> El {
    symbol(&icons::CHECK_CIRCLE_FILL, point, weight::MEDIUM, color.color())
}

fn permission_line(row_: &PermissionRow, paste: &str, theme: Theme) -> El {
    let bg = theme.group_bg();
    let name = match row_.permission.as_str() {
        "screen-recording" => "Screen Recording",
        "accessibility" => "Accessibility",
        "microphone" => "Microphone",
        "camera" => "Camera",
        "speech-recognition" => "Speech Recognition",
        other => other,
    };
    let why = match row_.permission.as_str() {
        "screen-recording" => "To see your screen".to_owned(),
        "microphone" => "To hear you".to_owned(),
        "accessibility" => format!("For ⌘Z and {paste}"),
        "camera" => "To show a cabled iPhone".to_owned(),
        "speech-recognition" => "To write down what you say".to_owned(),
        _ => String::new(),
    };
    let permission = row_.permission.clone();
    let trailing = if row_.granted() && row_.relaunch {
        button("Reopen Scrawl", true, theme, bg, |s, _| s.act("reopen")).key(format!("perm-{permission}-reopen"))
    } else if row_.granted() {
        row().key(format!("perm-{permission}-allowed"))
            .gap(5.0)
            .center_y()
            .child(check_disc(12.0, theme.positive()))
            .child(label("Allowed", 12.0, 500.0, theme.secondary().over(bg)))
    } else {
        let title = if row_.requested || row_.status == "denied" { "Open Settings" } else { "Allow" };
        let p = permission.clone();
        button(title, true, theme, bg, move |s, _| s.send(json!({ "action": "grant", "permission": p })))
            .key(format!("perm-{permission}-{title}"))
    };
    row().w_full()
        .h(52.0)
        .px(12.0)
        .gap(12.0)
        .center_y()
        .child(tile(permission_symbol(&row_.permission), permission_tint(&row_.permission), 26.0))
        .child(two_line(name, Some(&why), theme, 400.0, style(12.0, 400.0)))
        .child(spacer().min_w(8.0))
        .child(trailing.transition(replace()))
}

fn permissions(s: &SettingsWin, theme: Theme) -> Vec<El> {
    let v = &s.view;
    let paste = v.paste_label();
    let lines = |required: bool| -> Vec<El> {
        rows(
            theme,
            v.permissions.iter().filter(|r| r.required == required).map(|r| permission_line(r, &paste, theme)).collect(),
        )
    };
    vec![
        group(theme, lines(true)),
        caption("Scrawl needs these three to record.", false, theme),
        header("When you use them", theme),
        group(theme, lines(false)),
    ]
}

fn agents(s: &SettingsWin, theme: Theme) -> Vec<El> {
    let v = &s.view;
    let bg = theme.group_bg();
    let cli = &v.command_line;
    let mut line = row()
        .w_full()
        .h(if cli.path.is_none() { ROW } else { 52.0 })
        .px(12.0)
        .gap(12.0)
        .center_y()
        .child({
            let trailing: Vec<f32> = cli.button.iter().map(|b| button_w(if b == "uninstall" { "Uninstall" } else { "Install" })).collect();
            let path = cli.path.as_deref().map(|p| head_fit(p, MONO, room_beside(&trailing)));
            two_line("Command-line tool", path.as_deref(), theme, 400.0, MONO)
        })
        .child(spacer().min_w(8.0));
    if let Some(b) = &cli.button {
        let uninstall = b == "uninstall";
        line = line.child(button(if uninstall { "Uninstall" } else { "Install" }, cli.enabled, theme, bg, move |s, _| {
            s.act(if uninstall { "uninstall-cli" } else { "install-cli" })
        }));
    }
    let copied = s.now < s.copied_until;
    let command = v.skill_command.clone();
    vec![
        group(theme, vec![line]),
        caption(&cli.caption, cli.error, theme),
        header("Agent skill", theme),
        group(
            theme,
            vec![
                row().w_full()
                    .h(ROW)
                    .px(12.0)
                    .gap(12.0)
                    .center_y()
                    .child(
                        text(&v.skill_command)
                            .font(MONO)
                            .color(theme.text().over(bg))
                            .single_line()
                            .truncate()
                            .shrink()
                            .selectable(true)
                            .selection_color(theme.accent().alpha(0.3).color()),
                    )
                    .child(spacer().min_w(8.0))
                    .child(button(if copied { "Copied" } else { "Copy" }, true, theme, bg, move |s, tx| {
                        tx.copy(command.clone());
                        s.copied_until = s.now + 1.5;
                        tx.wake_after(Duration::from_millis(1510));
                    })),
            ],
        ),
        caption(
            "Run it in a terminal. It teaches Claude Code, OpenCode, Codex and other agents to read your captures and use scrawl.",
            false,
            theme,
        ),
    ]
}

fn link(title: &str, shown: &str, url: &'static str, theme: Theme) -> El {
    let bg = theme.group_bg();
    setting_row(
        title,
        true,
        theme,
        vec![
            row().gap(5.0)
                .center_y()
                .child(label(shown, 14.0, 400.0, theme.secondary().over(bg)))
                .child(symbol(&icons::ARROW_UP_RIGHT, 10.0, weight::SEMIBOLD, theme.tertiary().over(bg))),
        ],
    )
    .cursor(rune::Cursor::Pointer)
    .on_press(move |_: &mut SettingsWin| open_url(url))
}

fn about(s: &SettingsWin, theme: Theme) -> Vec<El> {
    let v = &s.view;
    let bg = theme.group_bg();
    let mut top = row().w_full().h(64.0).px(12.0).gap(12.0).center_y();
    if let Some(pixels) = &s.icon {
        top = top.child(el().size(40.0, 40.0).pixels(pixels));
    }
    top = top
        .child(two_line(
            "Scrawl",
            Some(&format!("Version {}", v.version)),
            theme,
            600.0,
            style(12.0, 400.0).features("'tnum' 1"),
        ))
        .child(spacer())
        .child(button("Check for Updates…", v.updates, theme, bg, |s, _| s.act("check-for-updates")));
    let mut out = vec![group(theme, vec![top])];
    if !v.updates {
        out.push(caption("Updates work in the installed Scrawl.app.", false, theme));
    }
    out.push(header("Links", theme));
    out.push(group(
        theme,
        rows(
            theme,
            vec![
                link("Website", "scrawl.kitlangton.com", "https://scrawl.kitlangton.com", theme),
                link("Source", "github.com/kitlangton/scrawl", "https://github.com/kitlangton/scrawl", theme),
                setting_row("Welcome tour", true, theme, vec![button("Show", true, theme, bg, |s, _| s.act("tour"))]),
            ],
        ),
    ));
    out
}

fn sidebar_item(s: &SettingsWin, pane: Pane, theme: Theme) -> El {
    let selected = s.pane == pane;
    let bg = theme.sidebar();
    row().w_full()
        .h(36.0)
        .px(8.0)
        .gap(10.0)
        .center_y()
        .radius(9.0)
        .fill(if selected { theme.selection().over(bg) } else { bg.color() })
        .child(tile(pane.symbol(), pane.tint(), 24.0))
        .child(label(pane.title(), 14.0, 400.0, theme.text().over(if selected { theme.selection().over_rgba(bg) } else { bg })))
        // Selects on mouse-down, as a sidebar does.
        .on_drag(move |s: &mut SettingsWin, d: &mut rune::Drag, tx: &mut Tx| {
            if d.phase == Phase::Down {
                tx.instant();
                s.select(pane);
            }
        })
}

fn section(title: &str, theme: Theme) -> El {
    el().pl(10.0).pt(16.0).pb(4.0).child(label(title, 12.0, 600.0, theme.secondary().over(theme.sidebar())))
}

fn traffic_lights() -> El {
    let dot = |c: Rgba| el().size(12.0, 12.0).radius(6.0).fill(c.color());
    row().h(TITLEBAR)
        .pl(10.0)
        .gap(8.0)
        .center_y()
        .child(dot(Rgba::rgb(1.0, 0.37, 0.34)))
        .child(dot(Rgba::rgb(1.0, 0.74, 0.18)))
        .child(dot(Rgba::rgb(0.16, 0.79, 0.25)))
}

fn view(s: &SettingsWin, cx: &mut Cx<SettingsWin>) -> El {
    let theme = Theme { dark: cx.system_dark() };
    let sidebar = col()
        .w(SIDEBAR)
        .h_full()
        .px(10.0)
        .fill(theme.sidebar().color())
        .window_drag()
        .child(if s.render_time.is_some() { traffic_lights() } else { el().h(TITLEBAR) })
        .child(sidebar_item(s, Pane::General, theme))
        .child(section("Settings", theme))
        .child(
            col().w_full()
                .gap(2.0)
                .child(sidebar_item(s, Pane::Recording, theme))
                .child(sidebar_item(s, Pane::Storage, theme))
                .child(sidebar_item(s, Pane::Permissions, theme))
                .child(sidebar_item(s, Pane::Agents, theme)),
        )
        .child(section("Scrawl", theme))
        .child(sidebar_item(s, Pane::About, theme));
    let pane = s.pane;
    let body = match pane {
        Pane::General => general(s, theme),
        Pane::Recording => recording(s, theme),
        Pane::Storage => storage(s, theme),
        Pane::Permissions => permissions(s, theme),
        Pane::Agents => agents(s, theme),
        Pane::About => about(s, theme),
    };
    let content = col()
        .key(format!("pane-{}", pane.title()))
        .grow(1.0)
        .h_full()
        .px(INSET)
        .fill(theme.window().color())
        .child(
            row().h(TITLEBAR)
                .w_full()
                .gap(10.0)
                .center_y()
                .window_drag()
                .child(tile(pane.symbol(), pane.tint(), 24.0))
                .child(label(pane.title(), 16.0, 600.0, theme.text().over(theme.window()))),
        )
        .children(body)
        .on_press(|s: &mut SettingsWin| {
            s.menu = None;
            if s.view.recording().is_some() {
                s.act("cancel-shortcut");
            }
        });
    let mut root = row()
        .size(WIDTH, HEIGHT)
        .fill(theme.window().color())
        .child(sidebar)
        .child(el().w(1.0).h_full().fill(theme.separator().over(theme.window())))
        .child(content);
    if s.render_time.is_some() {
        root = root.clip_rounded(16.0);
    }
    if let Some(menu) = &s.menu {
        root = root.child(menu_view(menu, theme));
    }
    root
}

// MARK: Clock, keys, focus

fn poll(s: &mut SettingsWin, t: f64, tx: &mut Tx) -> bool {
    s.now = t;
    if s.render_time.is_some() {
        return false;
    }
    let mut changed = false;
    if s.focused {
        let held = flag_labels(crate::capture::modifier_flags());
        if held != s.held {
            s.hold(held, t);
            changed = true;
        }
        tx.wake_after(Duration::from_millis(33));
    }
    if let Some(meter) = &s.meter_watch {
        let target = meter.take();
        let next = if target >= s.meter { target } else { target.max(s.meter - 0.035) };
        if next != s.meter {
            s.meter = next;
            changed = true;
        }
        tx.wake_after(Duration::from_millis(33));
    }
    s.advance(t);
    if !s.keys_still(t) {
        changed = true;
        tx.wake_after(Duration::from_millis(16));
    }
    if changed {
        tx.instant();
    }
    changed
}

fn on_focus(s: &mut SettingsWin, focused: bool, tx: &mut Tx) {
    s.focused = focused;
    if !focused {
        if s.view.recording().is_some() {
            s.act("cancel-shortcut");
        }
        let t = s.now;
        s.hold(Vec::new(), t);
        s.menu = None;
    }
    tx.wake_after(Duration::from_millis(16));
}

fn on_close(s: &mut SettingsWin, _tx: &mut Tx) {
    s.meter_watch = None;
    s.menu = None;
    s.discard_hex_draft();
    if let Some(events) = s.events.take() {
        events(crate::settings::Event::Action(crate::settings::Action::Close));
        events(crate::settings::Event::Exited);
    }
    hand_back_keys();
}

/// As the SwiftUI window's `bringForward`: the app in front is remembered on every show, so
/// closing the window gives it its keys back (unless another Scrawl window still has them).
fn take_keys() {
    #[cfg(target_os = "macos")]
    {
        super::platform::hold_keys(super::platform::KeyHolder::Settings, true);
        super::platform::take_keys_now();
    }
}

fn hand_back_keys() {
    #[cfg(target_os = "macos")]
    {
        super::platform::hold_keys(super::platform::KeyHolder::Settings, false);
        super::platform::hand_back_keys_now();
    }
}

fn on_key(s: &mut SettingsWin, key: &rune::KeyEvent, tx: &mut Tx) -> bool {
    if s.view.recording().is_some() {
        return true;
    }
    if matches!(key.key, rune::Key::Command('w')) || (key.mods.cmd && matches!(key.key, rune::Key::Text(ref c) if c == "w")) {
        tx.close_window();
        return true;
    }
    if matches!(key.key, rune::Key::Escape) && s.menu.is_some() {
        s.menu = None;
        return true;
    }
    false
}

fn init() -> SettingsWin {
    SettingsWin::new()
}

fn spec(fonts: &'static [&'static [u8]]) -> AppSpec<SettingsWin> {
    AppSpec {
        title: "Scrawl Settings",
        size: (WIDTH, HEIGHT),
        min_size: Some((WIDTH, HEIGHT)),
        init,
        view,
        scenes: SCENES,
        fonts,
        families: families(fonts),
        setup: None,
        poll: Some(poll),
        on_key: Some(on_key),
    }
}

const CHROME: Chrome = Chrome { full_size_content: true, traffic_lights: Some((20.0, 19.0)), drag_regions: true };

// MARK: Hosting

static HANDLE: OnceLock<AppHandle<SettingsWin>> = OnceLock::new();

/// Adds the hidden Settings window to Scrawl's Rune app; `request_open` shows it.
pub(crate) fn install<S: 'static>(app: &mut App<S>) {
    let fonts = fonts();
    let spec = WindowSpec::new("settings", spec(fonts))
        .options(WindowOptions {
            visible: false,
            resizable: false,
            min_size: Some((WIDTH, HEIGHT)),
            max_size: Some((WIDTH, HEIGHT)),
            ..Default::default()
        })
        .chrome(CHROME)
        .on_focus(on_focus);
    let mut spec = spec;
    spec.app = spec.app.on_close(on_close);
    let (handle, _window) = app.window(spec);
    let _ = HANDLE.set(handle);
}

/// Whether the Rune window is there to take `settings::Session`'s messages.
pub(crate) fn available() -> bool {
    HANDLE.get().is_some()
}

/// Attaches a session: its messages drive the window, its actions go to `events`.
pub(crate) fn request_open(
    messages: flume::Receiver<Value>,
    pane: Option<crate::settings::Pane>,
    events: Events,
) -> bool {
    let Some(handle) = HANDLE.get().cloned() else { return false };
    let pane = pane
        .and_then(|p| serde_json::to_value(p).ok())
        .and_then(|v| v.as_str().and_then(Pane::parse))
        .unwrap_or(Pane::General);
    let link = attach(handle, pane, events);
    std::thread::Builder::new()
        .name("rune-settings".into())
        .spawn(move || {
            let mut presented = false;
            while let Ok(message) = messages.recv() {
                if !on_message(&link, message, &mut presented) {
                    return;
                }
            }
            close(&link);
        })
        .is_ok()
}

fn attach<H: Host<SettingsWin>>(host: H, pane: Pane, events: Events) -> Link<SettingsWin, H> {
    Link::attach(host, move |s: &mut SettingsWin, _tx| {
        s.events = Some(events);
        s.reduce = reduce_motion();
        s.discard_hex_draft();
        s.pane = pane;
        s.menu = None;
        s.update_meter();
    })
}

/// One message from the session. False once it has closed the window.
fn on_message<H: Host<SettingsWin>>(link: &Link<SettingsWin, H>, message: Value, presented: &mut bool) -> bool {
    let kind = message["type"].as_str().unwrap_or_default().to_owned();
    if kind == "close" {
        close(link);
        return false;
    }
    link.update(move |s, tx| s.message(&message, tx));
    if (kind == "state" && !*presented) || kind == "show" {
        *presented = true;
        link.update(|s, _| {
            s.reduce = reduce_motion();
            take_keys();
        });
        link.open();
    }
    true
}

/// The session has ended: its window goes, unless a newer session has it by now.
fn close<H: Host<SettingsWin>>(link: &Link<SettingsWin, H>) {
    link.update(|s, _| {
        s.events = None;
        s.meter_watch = None;
        s.discard_hex_draft();
        hand_back_keys();
    });
    link.hide();
}

// MARK: Renders

const SCENE_NAMES: &[&str] = &[
    "general",
    "general-held",
    "general-listening",
    "general-partial",
    "general-refused",
    "general-refused-reason",
    "general-accepted",
    "general-custom",
    "general-paste-listening",
    "general-paste-cleared",
    "general-paste-off",
    "general-lower-playback",
    "recording",
    "recording-hex",
    "recording-mic-chosen",
    "recording-mic-missing",
    "recording-mic-not-allowed",
    "recording-downloading",
    "recording-ready",
    "recording-language",
    "recording-language-loading",
    "recording-language-unsupported",
    "recording-command",
    "storage",
    "storage-custom",
    "permissions",
    "permissions-missing",
    "agents",
    "agents-installed",
    "agents-local",
    "agents-homebrew",
    "agents-taken",
    "agents-dev",
    "about",
    "about-dev",
];

macro_rules! scenes {
    ($($name:literal),* $(,)?) => {
        &[$(($name, {
            fn apply(s: &mut SettingsWin) { sample(s, $name) }
            apply as fn(&mut SettingsWin)
        })),*]
    };
}

static SCENES: &[(&str, fn(&mut SettingsWin))] = scenes!(
    "general",
    "general-held",
    "general-listening",
    "general-partial",
    "general-refused",
    "general-refused-reason",
    "general-accepted",
    "general-custom",
    "general-paste-listening",
    "general-paste-cleared",
    "general-paste-off",
    "general-lower-playback",
    "recording",
    "recording-hex",
    "recording-mic-chosen",
    "recording-mic-missing",
    "recording-mic-not-allowed",
    "recording-downloading",
    "recording-ready",
    "recording-language",
    "recording-language-loading",
    "recording-language-unsupported",
    "recording-command",
    "storage",
    "storage-custom",
    "permissions",
    "permissions-missing",
    "agents",
    "agents-installed",
    "agents-local",
    "agents-homebrew",
    "agents-taken",
    "agents-dev",
    "about",
    "about-dev",
    "live",
);

/// The samples `scrawl-helper settings --render` draws, from the same states.
fn sample(s: &mut SettingsWin, name: &str) {
    if name == "live" {
        return;
    }
    let mut base = View { captures_size: Some("1.4 GB".into()), ..View::default() };
    for row in &mut base.permissions {
        row.status = if row.required { "granted".into() } else { "denied".into() };
    }
    let opt = |id: &str, title: &str| MenuOption { id: id.into(), title: title.into(), enabled: true };
    let devices = [opt("BuiltInMicrophoneDevice", "MacBook Pro Microphone"), opt("AppleUSBAudioEngine:Shure:MV7", "Shure MV7")];
    let mut mic_options = vec![
        opt("default", "System default (MacBook Pro Microphone)"),
        MenuOption { id: "-".into(), title: String::new(), enabled: false },
    ];
    mic_options.extend(devices.iter().cloned());
    base.microphone = Microphone {
        selected: "default".into(),
        value: "System default".into(),
        options: mic_options.clone(),
        note: None,
        meter: Some(devices[0].id.clone()),
    };
    let mut listening = base.clone();
    listening.shortcuts[0].recording = true;
    let mut refused = base.clone();
    refused.shortcuts[0].error = Some("Apps use ⌘V. Add ⌥ or ⌃.".into());
    let mut custom = base.clone();
    custom.shortcuts[0] = Shortcut {
        label: "⌃⌥⌘R".into(),
        key: "R".into(),
        shift: false,
        control: true,
        option: true,
        command: true,
        is_default: false,
        ..Shortcut::default()
    };
    custom.login.on = true;
    let mut paste_off = base.clone();
    paste_off.shortcuts[1] = Shortcut {
        label: String::new(),
        key: String::new(),
        shift: false,
        option: false,
        is_default: false,
        ..Shortcut::paste()
    };
    let mut paste_listening = base.clone();
    paste_listening.shortcuts[1].recording = true;

    let held = |m: &[&str]| m.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
    type Step = (f64, Box<dyn Fn(&mut SettingsWin, f64)>);
    let show = |v: &View| {
        let v = v.clone();
        Box::new(move |s: &mut SettingsWin, t: f64| s.show(v.clone(), t)) as Box<dyn Fn(&mut SettingsWin, f64)>
    };
    let hold = |m: Vec<String>| Box::new(move |s: &mut SettingsWin, t: f64| s.hold(m.clone(), t)) as Box<dyn Fn(&mut SettingsWin, f64)>;
    let typed = |kind: &'static str, l: &'static str| {
        Box::new(move |s: &mut SettingsWin, t: f64| s.type_key(kind, l, t)) as Box<dyn Fn(&mut SettingsWin, f64)>
    };
    let rest = || -> Vec<Step> { vec![(0.0, show(&base))] };
    let listen = || -> Vec<Step> {
        let mut v = rest();
        v.push((1.0, show(&listening)));
        v
    };
    let refuse = || -> Vec<Step> {
        let mut v = listen();
        v.extend([
            (1.3, hold(held(&["⌘"]))),
            (1.6, typed("record", "V")),
            (1.62, show(&refused)),
            (1.8, hold(Vec::new())),
        ]);
        v
    };
    let accept = || -> Vec<Step> {
        let mut v = listen();
        v.extend([
            (1.3, hold(held(&["⌃", "⌥", "⌘"]))),
            (1.6, typed("record", "R")),
            (1.62, show(&custom)),
            (1.9, hold(Vec::new())),
        ]);
        v
    };
    let stages: Option<(Vec<Step>, f64)> = match name {
        "general" => Some((rest(), 5.0)),
        "general-held" => Some(({
            let mut v = rest();
            v.push((1.0, hold(held(&["⌥", "⇧"]))));
            v
        }, 2.0)),
        "general-listening" => Some((listen(), 2.5)),
        "general-partial" => Some(({
            let mut v = listen();
            v.push((1.5, hold(held(&["⌃", "⌥"]))));
            v
        }, 2.5)),
        "general-refused" => Some((refuse(), 2.02)),
        "general-refused-reason" => Some((refuse(), 3.2)),
        "general-accepted" => Some((accept(), 1.86)),
        "general-custom" => Some((vec![(0.0, show(&custom))], 5.0)),
        "general-paste-listening" => Some(({
            let mut v = rest();
            v.push((1.0, show(&paste_listening)));
            v
        }, 2.5)),
        "general-paste-cleared" => Some(({
            let mut v = rest();
            v.extend([(1.0, show(&paste_listening)), (1.5, typed("paste", "⌫")), (1.52, show(&paste_off))]);
            v
        }, 1.7)),
        "general-paste-off" => Some((vec![(0.0, show(&paste_off))], 5.0)),
        _ => None,
    };

    let mut lowered = base.clone();
    lowered.lower_playback = true;
    let mut hex = base.clone();
    hex.transcriber = Transcriber {
        engine: "hex".into(),
        hex_title: "Hex".into(),
        hex_enabled: true,
        language: "nl".into(),
        models: vec![
            ModelOption { id: "parakeet-tdt-0.6b-v3".into(), title: "Parakeet TDT v3".into(), chosen: true },
            ModelOption { id: "whisper-large-v3-turbo".into(), title: "Whisper Large v3 Turbo".into(), chosen: false },
        ],
        ..Transcriber::default()
    };
    hex.video_export = true;
    let speech = |state: &str, locale: &str, progress: f64, locales: Option<Vec<String>>| SpeechStatus {
        state: state.into(),
        locale: Some(locale.into()),
        progress,
        locales,
        ..SpeechStatus::default()
    };
    let locales: Vec<String> = ["de-DE", "en-GB", "en-US", "es-ES", "fr-FR", "ja-JP", "ko-KR", "yue-CN", "zh-CN", "zh-TW"]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
    let mut downloading = base.clone();
    downloading.transcriber.speech = Some(speech("downloading", "en_US", 0.42, None));
    let mut ready = base.clone();
    ready.transcriber.speech = Some(speech("ready", "en_US", 1.0, Some(locales.clone())));
    let mut language = base.clone();
    language.transcriber.apple_locale = "zh-TW".into();
    language.transcriber.speech = Some(speech("ready", "zh_TW", 1.0, Some(locales.clone())));
    let mut language_loading = language.clone();
    language_loading.transcriber.speech = None;
    let mut language_unsupported = ready.clone();
    language_unsupported.transcriber.apple_locale = "xx-XX".into();
    let mut command = base.clone();
    command.transcriber = Transcriber { engine: "command".into(), command: Some("whisper-scrawl".into()), ..Transcriber::default() };
    let mut missing = base.clone();
    missing.permissions[0].status = "granted".into();
    missing.permissions[0].relaunch = true;
    missing.permissions[2].status = "not-determined".into();
    missing.permissions[2].requested = true;
    let mut no_updates = base.clone();
    no_updates.updates = false;
    let mut storage_custom = base.clone();
    storage_custom.captures_dir = CapturesDir { path: "~/Documents/Scrawl Captures".into(), custom: true };
    let cli = |path: Option<&str>, button: Option<&str>, enabled: bool, caption: &str, error: bool| {
        let mut v = base.clone();
        v.command_line = CommandLine {
            path: path.map(str::to_owned),
            button: button.map(str::to_owned),
            enabled,
            caption: caption.into(),
            error,
        };
        v
    };
    let mut mic_chosen = base.clone();
    mic_chosen.microphone = Microphone {
        selected: devices[1].id.clone(),
        value: "Shure MV7".into(),
        options: mic_options.clone(),
        note: None,
        meter: Some(devices[1].id.clone()),
    };
    let mut mic_missing = base.clone();
    let mut missing_options = mic_options.clone();
    missing_options.push(MenuOption { id: "airpods".into(), title: "AirPods Pro (not connected)".into(), enabled: false });
    mic_missing.microphone = Microphone {
        selected: "airpods".into(),
        value: "AirPods Pro".into(),
        options: missing_options,
        note: Some("AirPods Pro isn't connected. Using MacBook Pro Microphone.".into()),
        meter: Some(devices[0].id.clone()),
    };
    let mut mic_not_allowed = base.clone();
    mic_not_allowed.microphone.meter = None;

    let (pane, state) = match name {
        "general-lower-playback" => (Pane::General, lowered),
        n if n.starts_with("general") => (Pane::General, base.clone()),
        "recording" => (Pane::Recording, base.clone()),
        "recording-hex" => (Pane::Recording, hex),
        "recording-mic-chosen" => (Pane::Recording, mic_chosen),
        "recording-mic-missing" => (Pane::Recording, mic_missing),
        "recording-mic-not-allowed" => (Pane::Recording, mic_not_allowed),
        "recording-downloading" => (Pane::Recording, downloading),
        "recording-ready" => (Pane::Recording, ready),
        "recording-language" => (Pane::Recording, language),
        "recording-language-loading" => (Pane::Recording, language_loading),
        "recording-language-unsupported" => (Pane::Recording, language_unsupported),
        "recording-command" => (Pane::Recording, command),
        "storage" => (Pane::Storage, base.clone()),
        "storage-custom" => (Pane::Storage, storage_custom),
        "permissions" => (Pane::Permissions, base.clone()),
        "permissions-missing" => (Pane::Permissions, missing),
        "agents" => (Pane::Agents, base.clone()),
        "agents-installed" => (Pane::Agents, cli(
            Some("/opt/homebrew/bin/scrawl"),
            Some("uninstall"),
            true,
            "Agents and your terminal can run scrawl to read captures, zoom into a moment and export video.",
            false,
        )),
        "agents-local" => (Pane::Agents, cli(
            Some("~/.local/bin/scrawl"),
            Some("uninstall"),
            true,
            "If your shell can't find scrawl, add ~/.local/bin to your PATH.",
            false,
        )),
        "agents-homebrew" => (Pane::Agents, cli(
            Some("/opt/homebrew/bin/scrawl"),
            None,
            true,
            "Installed with Homebrew. Agents and your terminal can run scrawl to read captures, zoom into a moment and export video.",
            false,
        )),
        "agents-taken" => (Pane::Agents, cli(None, None, true, "/usr/local/bin/scrawl is another program, so Scrawl leaves it alone.", true)),
        "agents-dev" => (Pane::Agents, cli(None, Some("install"), false, "The command-line tool works in the installed Scrawl.app.", false)),
        "about" => (Pane::About, base.clone()),
        "about-dev" => (Pane::About, no_updates),
        _ => (Pane::General, base.clone()),
    };

    s.keys.clear();
    s.reduce = false;
    if let Some((steps, t)) = stages {
        for (at, step) in steps {
            s.advance(at);
            step(s, at);
        }
        s.advance(t);
        s.render_time = Some(t);
    } else {
        s.show(state, 0.0);
        s.render_time = Some(5.0);
    }
    s.pane = pane;
    s.meter = 0.44;
}

/// `--snap` one sample (`settings-<name>`, `@light` for the light appearance) headless.
pub(crate) fn run_headless(scene: &str, mut args: Vec<String>) {
    let name = scene.trim_start_matches("settings-");
    let (name, light) = match name.strip_suffix("@light") {
        Some(n) => (n, true),
        None => (name.strip_suffix("@dark").unwrap_or(name), false),
    };
    if let Some(i) = args.iter().position(|a| a == "--scene") {
        args[i + 1] = name.to_owned();
    }
    args.extend(["--theme".to_owned(), if light { "light" } else { "dark" }.to_owned()]);
    let fonts = fonts();
    let app = App::new(spec(fonts)).chrome(CHROME).args(args);
    app.run();
}

/// Every sample in the order `scrawl-helper settings --render` numbers them.
pub(crate) fn render_names() -> Vec<String> {
    let mut out = Vec::new();
    for light in [false, true] {
        for name in SCENE_NAMES {
            if light && name.contains('-') && !name.starts_with("general-") && !name.starts_with("recording-mic") {
                continue;
            }
            out.push(format!("settings-{name}{}", if light { "@light" } else { "" }));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picker_menu_checks_the_current_choice_and_opens_over_it() {
        let options = vec![choice("auto", "Automatic", true), choice("-", "", false), choice("en-US", "English", true), choice("fr-FR", "French", false)];
        let expected = PopupMenu::new()
            .item(PopupItem::new("auto", "Automatic"))
            .separator()
            .item(PopupItem::new("en-US", "English").checked(true))
            .item(PopupItem::new("fr-FR", "French").enabled(false))
            .selected("en-US");
        assert_eq!(native_menu(&options, Some("en-US")), expected);
    }

    #[test]
    fn every_sample_has_a_scene_and_decodes_as_swift_does() {
        assert_eq!(SCENES.len(), SCENE_NAMES.len() + 1);
        for (name, _) in SCENES {
            if *name != "live" {
                assert!(SCENE_NAMES.contains(name), "{name}");
            }
        }
        let view: View = serde_json::from_value(json!({"type": "state", "version": "1.0"})).unwrap();
        assert_eq!(view.version, "1.0");
        assert_eq!(view.shortcuts.len(), 2);
        assert!(view.login.available);
    }

    #[test]
    fn a_replaced_session_leaves_the_new_one_alone() {
        use super::super::rune_session::test_host::Recorder;
        let state = json!({"type": "state", "version": "1.0"});
        let close = json!({"type": "close"});
        for close_first in [true, false] {
            let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
            let events = |tag: &'static str| -> Events {
                let seen = Arc::clone(&seen);
                Arc::new(move |_| seen.lock().unwrap().push(tag))
            };
            let host = Recorder::default();
            let (mut win, mut visible) = (SettingsWin::new(), false);
            let old = attach(host.clone(), Pane::General, events("old"));
            assert!(on_message(&old, state.clone(), &mut false));
            host.run(&mut win, &mut visible);
            assert!(visible);

            let new = attach(host.clone(), Pane::Storage, events("new"));
            if close_first {
                assert!(!on_message(&old, close.clone(), &mut true));
                assert!(on_message(&new, state.clone(), &mut false));
            } else {
                assert!(on_message(&new, state.clone(), &mut false));
                assert!(!on_message(&old, close.clone(), &mut true));
            }
            host.run(&mut win, &mut visible);
            assert!(visible, "close_first: {close_first}");
            assert_eq!(win.pane, Pane::Storage);
            (win.events.as_ref().expect("the new session's events"))(crate::settings::Event::Exited);
            assert_eq!(*seen.lock().unwrap(), ["new"]);

            assert!(!on_message(&new, close.clone(), &mut true));
            host.run(&mut win, &mut visible);
            assert!(!visible && win.events.is_none());
        }
    }

    #[test]
    fn an_unapplied_hex_language_goes_with_its_field() {
        let state = json!({"type": "state", "transcriber": {"engine": "hex", "language": "nl"}});
        let mut win = SettingsWin::new();
        win.message(&state, &mut Tx::default());
        win.select(Pane::Recording);
        assert_eq!(win.hex.text, "nl");
        win.hex = TextEdit::new("fr".to_owned());
        win.message(&state, &mut Tx::default());
        assert_eq!(win.hex.text, "fr", "a state with the same language keeps the draft");
        win.select(Pane::General);
        assert_eq!(win.hex.text, "nl");
        win.select(Pane::Recording);
        win.hex = TextEdit::new("fr".to_owned());
        on_close(&mut win, &mut Tx::default());
        assert_eq!(win.hex.text, "nl");
    }

    #[test]
    fn languages_match_with_or_without_a_region() {
        assert!(same_language("zh-TW", "zh_TW"));
        assert!(same_language("en", "en-US"));
        assert!(!same_language("zh-TW", "zh-CN"));
        assert!(valid_language("pt-br"));
        assert!(!valid_language("1en"));
    }

    #[test]
    fn physical_keys_map_to_carbon_codes() {
        assert_eq!(kvk(PhysicalKeyCode::KeyS), Some(1));
        assert_eq!(kvk(PhysicalKeyCode::Backspace), Some(51));
        assert_eq!(kvk(PhysicalKeyCode::F1), Some(122));
        assert_eq!(carbon(rune::Mods { alt: true, shift: true, ..Default::default() }), (1 << 11) | (1 << 9));
    }
}
