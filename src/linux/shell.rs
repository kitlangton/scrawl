//! The desktop around Scrawl: opening files and links, showing a file in the file manager,
//! the freedesktop.org Trash, and notifications.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};

/// Opens a file, folder or URL with its default app (`xdg-open`, else `gio open`).
pub fn open(target: &Path) -> Result<()> {
    for (program, args) in [("xdg-open", &[][..]), ("gio", &["open"][..])] {
        if crate::handoff::on_path(program).is_none() {
            continue;
        }
        // Detached: xdg-open can wait for the app it starts.
        Command::new(program)
            .args(args)
            .arg(target)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .with_context(|| format!("Could not run {program}"))?;
        return Ok(());
    }
    bail!("Opening files needs xdg-open (xdg-utils)")
}

/// Shows a file selected in the file manager (`org.freedesktop.FileManager1.ShowItems`, which
/// Files, Dolphin and Nemo answer), else opens its folder.
pub fn reveal(target: &Path) -> Result<()> {
    let uri = crate::handoff::file_uri(target);
    let shown = zbus::blocking::Connection::session().and_then(|bus| {
        bus.call_method(
            Some("org.freedesktop.FileManager1"),
            "/org/freedesktop/FileManager1",
            Some("org.freedesktop.FileManager1"),
            "ShowItems",
            &(vec![uri.as_str()], ""),
        )
        .map(|_| ())
    });
    if shown.is_ok() {
        return Ok(());
    }
    open(target.parent().unwrap_or(target))
}

/// Moves `path` to the user's Trash (the freedesktop.org Trash specification's home trash):
/// the item goes to `files/`, and an `info/<name>.trashinfo` records where it came from, so the
/// file manager can put it back.
pub fn trash(path: &Path) -> Result<()> {
    trash_in(path, &trash_dir())
}

fn trash_in(path: &Path, trash: &Path) -> Result<()> {
    let path = path
        .canonicalize()
        .with_context(|| format!("{} doesn't exist", path.display()))?;
    let (files, info) = (trash.join("files"), trash.join("info"));
    std::fs::create_dir_all(&files)?;
    std::fs::create_dir_all(&info)?;
    let name = path
        .file_name()
        .context("Can't trash a root folder")?
        .to_string_lossy()
        .into_owned();
    // A free name: `name`, `name.2`, `name.3`… claimed by creating its `.trashinfo` first.
    let mut n = 1;
    let (slot, info_file) = loop {
        let slot = if n == 1 {
            name.clone()
        } else {
            format!("{name}.{n}")
        };
        let info_file = info.join(format!("{slot}.trashinfo"));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&info_file)
        {
            Ok(mut file) if !files.join(&slot).exists() => {
                use std::io::Write as _;
                write!(
                    file,
                    "[Trash Info]\nPath={}\nDeletionDate={}\n",
                    percent_path(&path),
                    deletion_date(SystemTime::now())
                )?;
                break (slot, info_file);
            }
            Ok(_) => {
                let _ = std::fs::remove_file(&info_file);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
        n += 1;
        ensure!(n < 10_000, "No free name in the Trash for {name}");
    };
    if let Err(error) = std::fs::rename(&path, files.join(&slot)) {
        let _ = std::fs::remove_file(&info_file);
        // Another file system than the home trash's: the spec's per-volume trash isn't worth it
        // for Scrawl's captures, which live under the home folder.
        return Err(error).with_context(|| format!("Could not move {} to the Trash", path.display()));
    }
    Ok(())
}

fn trash_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Trash")
}

/// An absolute path for `Path=`, with everything but unreserved characters and `/`
/// percent-encoded.
fn percent_path(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt as _;
    let mut out = String::new();
    for &byte in path.as_os_str().as_bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// `YYYY-MM-DDThh:mm:ss` in local time, as the spec asks.
fn deletion_date(now: SystemTime) -> String {
    let secs = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs() as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: `localtime_r` only writes the `tm` it is given.
    unsafe { libc::localtime_r(&secs, &mut tm) };
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

/// A desktop notification (`org.freedesktop.Notifications`); quietly nothing without one.
pub fn notify(summary: &str, body: &str) {
    let summary = summary.to_owned();
    let body = body.to_owned();
    std::thread::spawn(move || {
        let Ok(bus) = zbus::blocking::Connection::session() else {
            return;
        };
        let hints: std::collections::HashMap<&str, zbus::zvariant::Value> =
            std::collections::HashMap::new();
        let _ = bus.call_method(
            Some("org.freedesktop.Notifications"),
            "/org/freedesktop/Notifications",
            Some("org.freedesktop.Notifications"),
            "Notify",
            &(
                "Scrawl",
                0_u32,
                "dev.kitlangton.scrawl",
                summary.as_str(),
                body.as_str(),
                Vec::<&str>::new(),
                hints,
                5_000_i32,
            ),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trash_info_paths_are_percent_encoded() {
        assert_eq!(
            percent_path(Path::new("/home/kit/a b#1.scrawl")),
            "/home/kit/a%20b%231.scrawl"
        );
    }

    #[test]
    fn trashing_keeps_the_item_and_where_it_came_from() {
        let home = std::env::temp_dir().join(format!("scrawl-trash-{}", std::process::id()));
        let item = home.join("captures/one.scrawl");
        std::fs::create_dir_all(&item).unwrap();
        std::fs::write(item.join("scrawl.json"), "{}").unwrap();
        trash_in(&item, &home.join("data/Trash")).unwrap();
        assert!(!item.exists());
        let trashed = home.join("data/Trash/files/one.scrawl/scrawl.json");
        assert!(trashed.is_file());
        let info =
            std::fs::read_to_string(home.join("data/Trash/info/one.scrawl.trashinfo")).unwrap();
        assert!(info.starts_with("[Trash Info]\nPath=/"));
        assert!(info.contains("captures/one.scrawl\n"));
        let _ = std::fs::remove_dir_all(&home);
    }
}
