//! Linux platform helpers: the session (X11 or Wayland), displays, top-level windows,
//! processes, the clipboard, the shell, autostart, the tray and the media pipeline.
//!
//! Apparent's own windows are GPUI windows on X11, through XWayland on a Wayland session
//! (`prepare_process`), because GPUI 0.2.2's Wayland windows can't be placed, kept on top or
//! made click-through. Global coordinates are X11 root-window pixels, top-left origin, like
//! Windows' physical pixels; `TargetFrame` holds pixels too.
//!
//! What Apparent can see depends on the session: on X11 it reads the window list, the pointer and
//! the keys straight from the X server (`x11`). On Wayland the compositor keeps those to itself:
//! the screen comes from the ScreenCast portal (or GNOME's own screencast API), the shortcut
//! from the GlobalShortcuts portal (or runtime binds on Hyprland), and clicks and ⇧ from the input devices when the user may
//! read them (`input`). See `capture/desktop/docs/linux/linux.md`.



pub(crate) mod accessibility;
pub(crate) mod capture;
pub(crate) mod integration;
pub(crate) mod interaction;
pub(crate) mod updates;
pub(crate) use accessibility::atspi;
pub(crate) use interaction::auto_paste;
pub(crate) use interaction::clipboard;
pub(crate) use interaction::hyprland;
pub(crate) use interaction::input;
pub(crate) use interaction::overlay;
pub(crate) use interaction::shortcuts;
pub(crate) use interaction::x11;
pub(crate) use capture::media;
pub(crate) use capture::portal;
pub(crate) use integration::appearance;
pub(crate) use integration::helper;
pub(crate) use integration::ipc;
pub(crate) use integration::login;
pub(crate) use integration::privacy;
pub(crate) use integration::shell;
pub(crate) use updates::launch_handler;
pub(crate) use updates::microphone;
pub(crate) use updates::update;

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::OnceLock;

pub use x11::{
    Monitor, WindowInfo, active_window as foreground_window, monitor_at, monitors,
    primary_monitor, top_level_windows, window_info,
};

/// RandR's name atom identifies an output while this X server is running. A list index does
/// not: hotplug or changing the primary output can reorder monitors during a portal dialog.
pub fn monitor_id(monitor: &Monitor) -> u32 {
    monitor.handle as u32
}

static WAYLAND_DISPLAY: OnceLock<Option<OsString>> = OnceLock::new();

/// Called first thing in `main`, before any thread starts: remembers the Wayland display (for
/// the clipboard and the portals) and hides it from GPUI, so its windows go through XWayland.
pub fn prepare_process() {
    // Relaunches inherit the saved display, not the variable hidden from GPUI.
    let wayland = read_wayland_display();
    let _ = WAYLAND_DISPLAY.set(wayland.clone());
    if wayland.is_some() && std::env::var_os("APPARENT_NATIVE_WAYLAND").is_none() {
        // SAFETY: called from `main` before any other thread exists.
        unsafe {
            std::env::remove_var("WAYLAND_DISPLAY");
            // GTK dialogs GPUI opens (file pickers) and xdg-open's children follow along.
            std::env::set_var("APPARENT_WAYLAND_DISPLAY", &wayland.unwrap_or_default());
        }
    }
}

/// The Wayland display name the session had, if it is a Wayland session.
pub fn wayland_display() -> Option<OsString> {
    WAYLAND_DISPLAY.get_or_init(read_wayland_display).clone()
}

fn read_wayland_display() -> Option<OsString> {
    std::env::var_os("WAYLAND_DISPLAY")
        .filter(|v| !v.is_empty())
        .or_else(|| std::env::var_os("APPARENT_WAYLAND_DISPLAY").filter(|v| !v.is_empty()))
}

/// A Wayland session: the X server Apparent talks to is XWayland, which sees only X11 apps.
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

/// What a keybinding or launcher should run for Apparent: the AppImage while one runs, else this
/// executable.
pub fn apparent_exe() -> std::io::Result<PathBuf> {
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

/// `~/.local/share/apparent`: logs and machine-local state.
pub fn local_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("apparent")
}

/// A process's executable.
pub fn process_path(pid: u32) -> Option<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/exe")).ok()
}

/// An app's name and identifier for a process: the identifier stands in for a macOS bundle
/// identifier (the executable's name, `firefox`, `gnome-terminal-server`), and the name comes
/// from its window's class when Apparent knows one (`Firefox`), else the executable's.
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

    #[test]
    fn clipboard_display_survives_relaunch() {
        const CHILD: &str = "APPARENT_TEST_CLIPBOARD_DISPLAY";
        if let Ok(expected) = std::env::var(CHILD) {
            let visible = std::env::var_os("WAYLAND_DISPLAY");
            let native = std::env::var_os("APPARENT_NATIVE_WAYLAND").is_some();
            prepare_process();
            let expected = (!expected.is_empty()).then(|| OsString::from(expected));
            assert_eq!(wayland_display(), expected);
            assert_eq!(wayland_display(), expected, "cached lookup must agree");
            if native || expected.is_none() {
                assert_eq!(std::env::var_os("WAYLAND_DISPLAY"), visible);
            } else {
                assert_eq!(std::env::var_os("WAYLAND_DISPLAY"), None);
                assert_eq!(std::env::var_os("APPARENT_WAYLAND_DISPLAY"), expected);
            }
            return;
        }

        // A fresh process exercises main's actual environment mutation and OnceLock,
        // without changing the test runner's environment or racing other tests.
        let test_name = concat!(module_path!(), "::clipboard_display_survives_relaunch")
            .split_once("::").unwrap().1;
        for (display, saved, native, expected) in [
            (Some("wayland-1"), None, false, "wayland-1"),
            (None, Some("wayland-1"), false, "wayland-1"),
            (Some(""), Some("wayland-1"), false, "wayland-1"),
            (Some("wayland-2"), Some("wayland-1"), false, "wayland-2"),
            (None, None, false, ""),
            (Some(""), Some(""), false, ""),
            (Some("wayland-1"), None, true, "wayland-1"),
            (None, Some("wayland-1"), true, "wayland-1"),
        ] {
            let mut command = std::process::Command::new(std::env::current_exe().unwrap());
            command
                .args(["--exact", test_name, "--nocapture"])
                .env_remove("WAYLAND_DISPLAY")
                .env_remove("APPARENT_WAYLAND_DISPLAY")
                .env_remove("APPARENT_NATIVE_WAYLAND")
                .env(CHILD, expected);
            if let Some(display) = display { command.env("WAYLAND_DISPLAY", display); }
            if let Some(saved) = saved { command.env("APPARENT_WAYLAND_DISPLAY", saved); }
            if native { command.env("APPARENT_NATIVE_WAYLAND", "1"); }
            let output = command.output().unwrap();
            assert!(output.status.success(), "display={display:?}, saved={saved:?}, native={native}: {}\n{}",
                String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"), "child test did not run");
        }
    }
}
