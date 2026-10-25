pub mod agent_context;
pub mod audio;
pub mod autocrop;
// Windows looks elements up through UI Automation (`win/uia.rs`); other systems have no lookup.
pub mod autocrop_lab;
#[cfg_attr(
    not(any(target_os = "macos", target_os = "windows")),
    path = "ax_other.rs"
)]
pub mod ax;
pub mod capture;
#[cfg_attr(target_os = "windows", path = "win/command_line.rs")]
pub mod command_line;
pub mod command_transcriber;
pub mod delivery;
pub mod export;
#[cfg(test)]
mod feature_map;
pub mod handoff;
#[cfg_attr(target_os = "windows", path = "win/helper.rs")]
pub mod helper;
pub mod hex;
pub mod icons;
pub mod inbox;
pub mod ingest_http;
pub mod keyframes;
#[cfg(target_os = "macos")]
pub mod laser_overlay;
pub mod live_transcribe;
pub mod mic;
pub mod model;
pub mod native;
pub mod onboarding;
pub mod onboarding_window;
pub mod page;
pub mod phone;
pub mod pipeline;
// Lowers the output volume while recording on macOS; elsewhere a no-op.
#[cfg_attr(not(target_os = "macos"), path = "win/playback.rs")]
pub mod playback;
pub mod recover;
pub mod render;
pub mod retention;
pub mod routing;
pub mod screens;
pub mod settings;
pub mod settings_window;
pub mod shortcut;
pub mod sketch;
#[cfg(target_os = "macos")]
pub mod sparkle;
pub mod speech_model;
pub mod transcribe;
pub mod ui;
pub mod web;
#[cfg(target_os = "windows")]
pub mod win;
pub mod windows;
pub mod zoom;
