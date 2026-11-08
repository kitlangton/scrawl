//! The native pill's values shared with the coordinator, on every platform.

use super::model::Pres;
use crate::capture::desktop::{SelectionMode, ui::motion};

/// Capsule visual style (`pillStyle: "black" | "glass"`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PillStyle {
    #[default]
    Black,
    Glass,
}

#[allow(dead_code)]
impl PillStyle {
    pub fn from_str_opt(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "glass" | "liquid" | "liquid-glass" | "1" | "true" => Some(Self::Glass),
            "black" | "dark" | "metal" | "0" | "false" => Some(Self::Black),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Black => "black",
            Self::Glass => "glass",
        }
    }

    /// Resolves the active `PillStyle` from `APPARENT_PILL_STYLE`, `tune.json`, or `config.json`.
    pub fn current() -> Self {
        if let Ok(env) = std::env::var("APPARENT_PILL_STYLE")
            && let Some(style) = Self::from_str_opt(&env)
        {
            return style;
        }
        if let Some(style) = motion::tune().pill_style_override() {
            return style;
        }
        crate::shared::handoff::pill_style()
    }
}

/// User actions emitted by clicking or pressing keys on the native pill (macOS only, so
/// elsewhere nothing makes them).
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub enum PillAction {
    SelectMode(SelectionMode),
    SwitchRecording(SelectionMode),
    RecordSelected,
    Cancel,
    Whiteboard,
    Finish,
    Paste(Option<[f32; 2]>),
    CopyLink,
    Inspect,
    DismissCompleted,
    /// A session chip on the Completed island (smart routing), by position.
    Route(usize),
}

/// Coordinator state snapshot synced to `NativePill`.
#[derive(Clone, Debug, PartialEq)]
pub struct PillSnapshot {
    pub pres: Pres,
    pub clock: String,
    pub mic_warning: Option<crate::capture::engine::runtime::audio::audio::InputWarning>,
    pub reduced_motion: bool,
    pub style: PillStyle,
}

/// The warning describes the microphone; the named action must match the visible control.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn mic_warning_accessibility_label(warning: crate::capture::engine::runtime::audio::audio::InputWarning, chips: bool) -> String {
    format!(
        "Apparent recording controls. {}. {} available",
        warning.announcement(),
        if chips { "Cancel recording" } else { "Finish recording" }
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mic_warning_accessibility_names_the_actual_control() {
        let warning = crate::capture::engine::runtime::audio::audio::InputWarning::Stopped;
        assert!(mic_warning_accessibility_label(warning, true).contains("Cancel recording available"));
        assert!(mic_warning_accessibility_label(warning, false).contains("Finish recording available"));
        assert!(mic_warning_accessibility_label(warning, true).contains("screen recording continues"));
    }
}
