// Desktop-owned UI pieces; the top-level desktop module remains their shared context.
pub(super) mod desktop_completion;
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(super) mod electron_pill;
#[cfg(target_os = "linux")]
pub(super) mod controls;
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

/// The same boxed brand glyph on both desktop status surfaces. Keeping its own
/// contrasting background also protects it on panels themed independently of
/// the desktop's Light/Dark preference.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn status_icon(dark: bool) -> &'static [u8] {
    if dark {
        include_bytes!("../assets/menubar-icon@2x.png")
    } else {
        include_bytes!("../assets/menubar-light@2x.png")
    }
}
