//! App-only settings in `~/.config/scrawl/config.json` (UI runtime, native pill, video export
//! menu item, and playback volume lowering). Engine settings live in `handoff::EngineConfig`.

use std::path::Path;

use serde::Deserialize;

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    /// Feature flag: shows **Export Last Capture as Video…** in the menu (default off).
    #[serde(default)]
    pub video_export: Option<bool>,
    /// Lower the output volume while recording (`playback`) to `playbackLevel` percent of it.
    #[serde(default)]
    pub lower_playback: Option<bool>,
    #[serde(default)]
    pub playback_level: Option<u8>,
    /// Whether to use the native Metal recording pill instead of GPUI's `HudPillWindow` (default `true`).
    #[serde(default)]
    pub native_pill: Option<bool>,
    /// Visual style of the native recording pill: `"black"` (default) or `"glass"` (`NSGlassEffectView`).
    #[serde(default)]
    pub pill_style: Option<String>,
    /// `"gpui"` (default) or `"rune"`: which UI runtime draws the capsule, picker and review
    /// window (`SCRAWL_UI=rune` overrides it at launch).
    #[serde(default)]
    pub ui: Option<String>,
}

impl AppConfig {
    /// The output volume while recording when `lowerPlayback` is on, in percent of what it was.
    pub fn recording_volume(&self) -> u8 {
        self.playback_level.unwrap_or(20).min(100)
    }
}

pub fn load_from(path: &Path) -> AppConfig {
    crate::handoff::parse_config_from(path)
}

pub fn load() -> AppConfig {
    load_from(&crate::handoff::default_config_path())
}

/// Whether the app runs its UI on Rune (`"ui": "rune"` in `config.json`, or `SCRAWL_UI=rune`).
/// Defaults to GPUI (`false`).
pub fn rune_ui() -> bool {
    rune_ui_from(std::env::var("SCRAWL_UI").ok().as_deref(), &load())
}

pub fn rune_ui_from(env_ui: Option<&str>, config: &AppConfig) -> bool {
    if let Some(env_ui) = env_ui.map(str::trim).filter(|s| !s.is_empty()) {
        return env_ui.eq_ignore_ascii_case("rune");
    }
    config
        .ui
        .as_deref()
        .is_some_and(|u| u.trim().eq_ignore_ascii_case("rune"))
}

/// Whether the menu offers video export (`videoExport`, default off). `scrawl export` works
/// either way.
pub fn video_export() -> bool {
    load().video_export.unwrap_or(false)
}

/// Whether the native AppKit + Metal recording pill is enabled (`SCRAWL_NATIVE_PILL` or
/// `nativePill` in `config.json`, default `true`). macOS only: elsewhere the GPUI capsule draws.
pub fn native_pill() -> bool {
    if cfg!(not(target_os = "macos")) {
        return false;
    }
    if let Ok(val) = std::env::var("SCRAWL_NATIVE_PILL") {
        let s = val.trim().to_ascii_lowercase();
        if matches!(s.as_str(), "0" | "false" | "gpui" | "off" | "no") {
            return false;
        }
        if matches!(s.as_str(), "1" | "true" | "native" | "metal" | "on" | "yes") {
            return true;
        }
    }
    load().native_pill.unwrap_or(true)
}

/// Configured capsule style (`pillStyle` in `config.json`, default `"black"`).
pub fn pill_style() -> crate::native::pill::PillStyle {
    load()
        .pill_style
        .as_deref()
        .and_then(crate::native::pill::PillStyle::from_str_opt)
        .unwrap_or(crate::native::pill::PillStyle::Black)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn video_export_flag_defaults_off_and_bad_settings_keep_others() {
        let dir = std::env::temp_dir().join(format!(
            "scrawl-app-config-{}",
            crate::pipeline::make_bundle_id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        fs::write(&path, br#"{"autoSend":false}"#).unwrap();
        assert_eq!(load_from(&path).video_export, None);
        fs::write(
            &path,
            r#"{"videoExport": true, "nativePill": 3, "pillStyle": "glass"}"#,
        )
        .unwrap();
        let app_cfg = load_from(&path);
        assert_eq!(app_cfg.video_export, Some(true));
        assert_eq!(app_cfg.pill_style.as_deref(), Some("glass"));
        assert_eq!(app_cfg.native_pill, None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn rune_ui_selector_defaults_to_gpui_and_respects_config_or_env() {
        let dir = std::env::temp_dir().join(format!(
            "scrawl-rune-ui-{}",
            crate::pipeline::make_bundle_id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        let default_cfg = load_from(&path);
        assert!(!rune_ui_from(None, &default_cfg));
        assert!(rune_ui_from(Some("rune"), &default_cfg));
        assert!(rune_ui_from(Some(" RUNE "), &default_cfg));
        assert!(!rune_ui_from(Some("gpui"), &default_cfg));

        fs::write(&path, br#"{"ui":"rune","autoSend":false}"#).unwrap();
        let rune_cfg = load_from(&path);
        assert_eq!(rune_cfg.ui.as_deref(), Some("rune"));
        assert!(rune_ui_from(None, &rune_cfg));
        assert!(rune_ui_from(Some(""), &rune_cfg));
        assert!(!rune_ui_from(Some("gpui"), &rune_cfg));
        let _ = fs::remove_dir_all(&dir);
    }
}
