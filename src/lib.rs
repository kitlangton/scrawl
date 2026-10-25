pub use scrawl_capture::*;

pub mod app_config;
#[cfg_attr(target_os = "windows", path = "win/command_line.rs")]
pub mod command_line;
#[cfg(test)]
mod feature_map;
pub mod icons;
pub mod inbox;
#[cfg(target_os = "macos")]
pub mod laser_overlay;
pub mod native;
pub mod onboarding;
pub mod onboarding_window;
// Lowers the output volume while recording on macOS; elsewhere a no-op.
#[cfg_attr(not(target_os = "macos"), path = "win/playback.rs")]
pub mod playback;
pub mod settings;
pub mod settings_window;
pub mod shortcut;
#[cfg(target_os = "macos")]
pub mod sparkle;
pub mod ui;
#[cfg(target_os = "windows")]
pub mod win {
    pub use scrawl_capture::win::*;
    #[path = "login.rs"]
    pub mod login;
    #[path = "overlay.rs"]
    pub mod overlay;
    #[path = "speech.rs"]
    pub mod speech;
}
