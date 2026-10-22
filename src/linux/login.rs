//! Launch at login: an XDG autostart entry (`~/.config/autostart/scrawl.desktop`), which GNOME,
//! KDE and the other freedesktop.org desktops start at login. GNOME Tweaks' Startup Applications
//! and KDE's Autostart settings list it, and can turn it off with `Hidden=true`.

use std::path::PathBuf;

use anyhow::{Context, Result};

fn entry() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("autostart/scrawl.desktop")
}

/// The command the desktop runs at login: this executable's tray app (the AppImage when Scrawl
/// runs from one, which `$APPIMAGE` names).
fn command() -> Result<String> {
    let exe = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .map_or_else(std::env::current_exe, Ok)?;
    Ok(format!("\"{}\" app", exe.display()))
}

fn desktop_entry(exec: &str) -> String {
    format!(
        "[Desktop Entry]\nType=Application\nName=Scrawl\nComment=Show an agent what you mean\nExec={exec}\nIcon=dev.kitlangton.scrawl\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
    )
}

/// (registered, switched off by the desktop's startup settings).
pub fn status() -> (bool, bool) {
    let Ok(text) = std::fs::read_to_string(entry()) else {
        return (false, false);
    };
    let off = text.lines().any(|line| {
        let line = line.trim();
        line.eq_ignore_ascii_case("Hidden=true")
            || line.eq_ignore_ascii_case("X-GNOME-Autostart-enabled=false")
    });
    (true, off)
}

pub fn set(on: bool) -> Result<()> {
    let path = entry();
    if on {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, desktop_entry(&command()?))
            .with_context(|| format!("Could not write {}", path.display()))
    } else {
        match std::fs::remove_file(&path) {
            Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error.into()),
            _ => Ok(()),
        }
    }
}

/// The desktop's startup apps settings, where there's one to open; else the autostart folder.
pub fn open_startup_settings() {
    let path = entry();
    let _ = super::shell::open(path.parent().unwrap_or(&path));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_autostart_entry_starts_the_tray_app() {
        let text = desktop_entry("\"/opt/Scrawl.AppImage\" app");
        assert!(text.starts_with("[Desktop Entry]\n"));
        assert!(text.contains("\nExec=\"/opt/Scrawl.AppImage\" app\n"));
        assert!(text.contains("\nTerminal=false\n"));
    }
}
