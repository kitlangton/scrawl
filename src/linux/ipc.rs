//! A Unix socket the running app listens on (`$XDG_RUNTIME_DIR/scrawl.sock`), so another
//! `scrawl` process can reach it: `scrawl shortcut record|paste`, which GNOME's custom
//! keybindings run where there's no GlobalShortcuts portal, and a second launch from the app
//! grid, which opens Settings (GNOME without the AppIndicator extension shows no tray icon).

use std::io::{BufRead as _, BufReader, Write as _};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};

fn socket_path() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("scrawl.sock")
}

/// Sends one line to the running app. `Err` when none is running.
pub fn send(line: &str) -> Result<()> {
    let mut stream =
        UnixStream::connect(socket_path()).context("Scrawl isn't running (start it first)")?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    stream.write_all(format!("{line}\n").as_bytes())?;
    Ok(())
}

/// Whether another copy of the app is listening.
pub fn running() -> bool {
    UnixStream::connect(socket_path()).is_ok()
}

fn open_channel() -> &'static (flume::Sender<()>, flume::Receiver<()>) {
    static OPEN: std::sync::OnceLock<(flume::Sender<()>, flume::Receiver<()>)> =
        std::sync::OnceLock::new();
    OPEN.get_or_init(|| flume::bounded(4))
}

/// A second launch asked the running app to show itself (Settings).
pub fn request_open() {
    let _ = open_channel().0.try_send(());
}

/// Where `request_open`'s requests arrive (the tray hands them to the UI).
pub fn open_requests() -> flume::Receiver<()> {
    open_channel().1.clone()
}

/// Listens for lines from other `scrawl` processes and hands each to `handle`.
pub fn listen(handle: impl Fn(&str) + Send + 'static) {
    let path = socket_path();
    if running() {
        crate::capture::scrawl_log("ipc: another Scrawl is listening already");
        return;
    }
    let _ = std::fs::remove_file(&path);
    let listener = match UnixListener::bind(&path) {
        Ok(listener) => listener,
        Err(error) => {
            crate::capture::scrawl_log(format!("ipc: could not listen on {}: {error}", path.display()));
            return;
        }
    };
    {
        use std::os::unix::fs::PermissionsExt as _;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    let _ = std::thread::Builder::new()
        .name("scrawl-ipc".into())
        .spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut line = String::new();
                if BufReader::new(stream).read_line(&mut line).is_ok() {
                    handle(line.trim());
                }
            }
        });
}
