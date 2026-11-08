// Desktop-owned UI pieces; the top-level desktop module remains their shared context.
pub(super) mod desktop_completion;
pub(super) mod lab;
#[cfg(target_os = "linux")]
pub(super) mod linux;
#[cfg(target_os = "macos")]
pub(super) mod macos;
pub(super) mod motion;
pub(super) mod pill_theme;
pub(super) mod recording_completion;
pub(super) mod whiteboard;
#[cfg(target_os = "windows")]
pub(super) mod windows;
