//! Linux platform helpers: the session (X11 or Wayland), displays, top-level windows,
//! processes, the clipboard, the shell, autostart, the tray and the media pipeline.
//!
//! Scrawl's own windows are GPUI windows on X11, through XWayland on a Wayland session
//! (`prepare_process`), because GPUI 0.2.2's Wayland windows can't be placed, kept on top or
//! made click-through. Global coordinates are X11 root-window pixels, top-left origin, like
//! Windows' physical pixels; `TargetFrame` holds pixels too.
//!
//! What Scrawl can see depends on the session: on X11 it reads the window list, the pointer and
//! the keys straight from the X server (`x11`). On Wayland the compositor keeps those to itself:
//! the screen comes from the ScreenCast portal (or GNOME's own screencast API), the shortcut
//! from the GlobalShortcuts portal (or runtime binds on Hyprland), and clicks and ⇧ from the input devices when the user may
//! read them (`input`). See `docs/linux.md`.

pub mod atspi;
pub mod clipboard;
pub mod hyprland;
pub mod input;
pub mod ipc;
pub mod login;
pub mod media;
pub mod overlay;
pub mod portal;
pub mod privacy;
pub mod shell;
pub mod shortcuts;
pub mod x11;

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::OnceLock;

pub use x11::{
    Monitor, WindowInfo, active_window as foreground_window, monitor_at, monitors,
    primary_monitor, top_level_windows, window_info,
};

static WAYLAND_DISPLAY: OnceLock<Option<OsString>> = OnceLock::new();

/// Called first thing in `main`, before any thread starts: remembers the Wayland display (for
/// the clipboard and the portals) and hides it from GPUI, so its windows go through XWayland.
pub fn prepare_process() {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").filter(|v| !v.is_empty());
    let _ = WAYLAND_DISPLAY.set(wayland.clone());
    if wayland.is_some() && std::env::var_os("SCRAWL_NATIVE_WAYLAND").is_none() {
        // SAFETY: called from `main` before any other thread exists.
        unsafe {
            std::env::remove_var("WAYLAND_DISPLAY");
            // GTK dialogs GPUI opens (file pickers) and xdg-open's children follow along.
            std::env::set_var("SCRAWL_WAYLAND_DISPLAY", &wayland.unwrap_or_default());
        }
    }
}

/// The Wayland display name the session had, if it is a Wayland session.
pub fn wayland_display() -> Option<OsString> {
    WAYLAND_DISPLAY
        .get_or_init(|| {
            std::env::var_os("WAYLAND_DISPLAY")
                .or_else(|| std::env::var_os("SCRAWL_WAYLAND_DISPLAY"))
                .filter(|v| !v.is_empty())
        })
        .clone()
}

/// A Wayland session: the X server Scrawl talks to is XWayland, which sees only X11 apps.
pub fn wayland_session() -> bool {
    wayland_display().is_some()
        || std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t.eq_ignore_ascii_case("wayland"))
}

/// Where the pointer is (global pixels). On Wayland the X server sees it only over X11 windows,
/// so an absolute pointing device's position comes first when one moved it last
/// (`input::absolute_pointer`).
pub fn cursor_position() -> Option<(f64, f64)> {
    if wayland_session()
        && hyprland::running()
        && let Some(point) = hyprland::cursor_in_x_space(&output_rects())
    {
        return Some(point);
    }
    if wayland_session()
        && let Some([nx, ny]) = input::absolute_pointer()
        && let Some([x, y, w, h]) = desktop_rect()
    {
        return Some((x + nx * w, y + ny * h));
    }
    x11::cursor_position()
}

/// Each output's name and X11 rectangle, read at most once a second.
fn output_rects() -> Vec<(String, [f64; 4])> {
    static CACHE: std::sync::Mutex<Option<(std::time::Instant, Vec<(String, [f64; 4])>)>> =
        std::sync::Mutex::new(None);
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((at, rects)) = cache.as_ref()
        && at.elapsed() < std::time::Duration::from_secs(1)
    {
        return rects.clone();
    }
    let rects: Vec<_> = monitors().into_iter().map(|m| (m.name, m.rect)).collect();
    *cache = Some((std::time::Instant::now(), rects.clone()));
    rects
}

/// The bounding box of every monitor (global pixels), read at most once a second.
fn desktop_rect() -> Option<[f64; 4]> {
    static CACHE: std::sync::Mutex<Option<(std::time::Instant, Option<[f64; 4]>)>> =
        std::sync::Mutex::new(None);
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((at, rect)) = *cache
        && at.elapsed() < std::time::Duration::from_secs(1)
    {
        return rect;
    }
    let monitors = monitors();
    let x0 = monitors.iter().map(|m| m.rect[0]).reduce(f64::min)?;
    let y0 = monitors.iter().map(|m| m.rect[1]).reduce(f64::min)?;
    let x1 = monitors.iter().map(|m| m.rect[0] + m.rect[2]).reduce(f64::max)?;
    let y1 = monitors.iter().map(|m| m.rect[1] + m.rect[3]).reduce(f64::max)?;
    let rect = Some([x0, y0, x1 - x0, y1 - y0]);
    *cache = Some((std::time::Instant::now(), rect));
    rect
}

/// The desktop, lowercased (`gnome`, `kde`, `ubuntu:gnome`, …).
pub fn desktop() -> String {
    std::env::var("XDG_CURRENT_DESKTOP")
        .unwrap_or_default()
        .to_ascii_lowercase()
}

pub fn gnome() -> bool {
    desktop().split(':').any(|d| d == "gnome")
}

pub fn kde() -> bool {
    desktop().split(':').any(|d| d == "kde")
}

/// A Hyprland session (Omarchy's, for one). `XDG_CURRENT_DESKTOP` says `Hyprland` there, but a
/// session started another way may leave it out, so the compositor's socket counts too.
pub fn hyprland() -> bool {
    desktop().split(':').any(|d| d == "hyprland") || hyprland::running()
}

/// What a keybinding or launcher should run for Scrawl: the AppImage while one runs, else this
/// executable.
pub fn scrawl_exe() -> std::io::Result<PathBuf> {
    std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .map_or_else(std::env::current_exe, Ok)
}

/// What the session is, for logs and Settings: "GNOME on Wayland", "KDE Plasma on X11", …
pub fn session_label() -> String {
    let desktop = if gnome() {
        "GNOME".to_owned()
    } else if kde() {
        "KDE Plasma".to_owned()
    } else {
        std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_else(|_| "Linux".to_owned())
    };
    format!(
        "{desktop} on {}",
        if wayland_session() { "Wayland" } else { "X11" }
    )
}

/// `~/.local/share/scrawl`: logs and machine-local state.
pub fn local_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("scrawl")
}

/// A process's executable.
pub fn process_path(pid: u32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/exe")).ok()
}

/// An app's name and identifier for a process: the identifier stands in for a macOS bundle
/// identifier (the executable's name, `firefox`, `gnome-terminal-server`), and the name comes
/// from its window's class when Scrawl knows one (`Firefox`), else the executable's.
pub fn app_identity(pid: u32) -> Option<(String, String)> {
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .ok()
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    let exe = process_path(pid)
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .or(comm)?;
    let name = x11::class_of_pid(pid)
        .map(|class| pretty_app_name(&class))
        .unwrap_or_else(|| pretty_app_name(&exe));
    Some((name, exe.to_ascii_lowercase()))
}

/// `gnome-terminal-server` → `Gnome Terminal Server`; `firefox` → `Firefox`; a class that
/// already has capitals (`Google-chrome`) keeps them.
fn pretty_app_name(raw: &str) -> String {
    let raw = raw.rsplit('.').next().unwrap_or(raw);
    raw.split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect::<String>(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_names_read_like_names() {
        assert_eq!(pretty_app_name("firefox"), "Firefox");
        assert_eq!(pretty_app_name("gnome-terminal-server"), "Gnome Terminal Server");
        assert_eq!(pretty_app_name("org.gnome.Nautilus"), "Nautilus");
        assert_eq!(pretty_app_name("Google-chrome"), "Google Chrome");
    }
}
