//! Text and PNG through X11 selections, except on KDE Wayland and Hyprland: neither reliably
//! bridges a background XWayland owner's selection to native apps. There we use wl-copy's
//! data-control protocol (bundled in the AppImage). Both backends keep an owner alive to serve
//! the data.

use std::io::{BufRead as _, Read as _, Write as _};
use std::process::{Command, Stdio};
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow};
use wl_clipboard_rs::copy::{MimeSource, MimeType, Options, Source};

static CLIPBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);
const MAX_MIME_TYPES: usize = 32;
const MAX_MIME_BYTES: usize = 256;
const MAX_CLIPBOARD_BYTES: usize = 16 * 1024 * 1024;
const OWNER_ARGUMENT: &str = "--internal-wayland-clipboard-owner";

fn with<T>(f: impl FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error>) -> Result<T> {
    let mut slot = CLIPBOARD
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if slot.is_none() {
        *slot = Some(arboard::Clipboard::new().map_err(|e| anyhow!("{e}"))?);
    }
    let clipboard = slot.as_mut().context("No clipboard")?;
    f(clipboard).map_err(|e| anyhow!("Clipboard: {e}"))
}

/// Desktops whose XWayland selections don't reach native Wayland apps.
fn native_wayland_clipboard() -> bool {
    (super::super::kde() || super::super::hyprland()) && super::super::wayland_session()
}

/// This is a preflight only. A failed offer after this check must not replace
/// clipboard contents: another process may have become the selection owner.
pub fn combined_offer_supported() -> bool {
    native_wayland_clipboard() && owner_backend_ready()
}

// Probe the actual replacement in a bounded child, without claiming a selection.
// A successful wl-paste probe does not establish that this backend can start.
fn owner_backend_ready() -> bool {
    let (Ok(executable), Some(display)) = (owner_executable(), super::super::wayland_display())
    else {
        return false;
    };
    Command::new("timeout")
        .arg("3s")
        .arg(executable)
        .args([OWNER_ARGUMENT, "--check"])
        .env("WAYLAND_DISPLAY", display)
        .output()
        .is_ok_and(|output| output.status.success() && output.stdout == b"READY\n")
}

fn owner_executable() -> std::io::Result<std::path::PathBuf> {
    // Native integration tests run inside a Rust test harness; re-exec the
    // separately built application (or AppImage), which owns the real protocol.
    #[cfg(test)]
    if let Some(path) = std::env::var_os("APPARENT_TEST_CLIPBOARD_OWNER") {
        return Ok(path.into());
    }
    std::env::current_exe()
}

pub fn check_owner_backend() -> Result<()> {
    use wl_clipboard_rs::paste::{ClipboardType, Error, Seat, get_mime_types};
    match get_mime_types(ClipboardType::Regular, Seat::Unspecified) {
        Ok(_) | Err(Error::ClipboardEmpty) => {
            println!("READY");
            Ok(())
        }
        Err(error) => Err(error).context("Native clipboard backend unavailable"),
    }
}

pub struct TextSnapshot {
    entries: Vec<(String, Vec<u8>)>,
}

fn plain_text_mime(types: &str) -> Result<&str> {
    let offered: Vec<_> = types.lines().collect();
    anyhow::ensure!(!offered.is_empty(), "Clipboard has no offered MIME types");
    offered
        .iter()
        .copied()
        .find(|mime| *mime == "text/plain;charset=utf-8")
        .or_else(|| offered.iter().copied().find(|mime| *mime == "text/plain"))
        .or_else(|| offered.first().copied())
        .context("Clipboard has no MIME type")
}

pub fn snapshot_text_wayland() -> Result<TextSnapshot> {
    anyhow::ensure!(
        native_wayland_clipboard(),
        "Draft clipboard restoration requires KDE or Hyprland Wayland"
    );
    let types = read_wayland(&["--list-types"], 8192)?;
    let types = std::str::from_utf8(&types).context("Invalid clipboard types")?;
    if types.is_empty() {
        return Ok(TextSnapshot {
            entries: Vec::new(),
        });
    }
    let _ = plain_text_mime(types)?;
    let mimes = snapshot_mime_types(types)?;
    let mut entries = Vec::with_capacity(mimes.len());
    let mut size = 0usize;
    for mime in mimes {
        let bytes = read_wayland(
            &["--no-newline", "--type", mime],
            MAX_CLIPBOARD_BYTES - size,
        )?;
        size += bytes.len();
        entries.push((mime.to_owned(), bytes));
    }
    anyhow::ensure!(
        read_wayland(&["--list-types"], 8192)? == types.as_bytes(),
        "Clipboard changed during snapshot"
    );
    if entries.len() > 1 {
        anyhow::ensure!(
            owner_backend_ready(),
            "Rich clipboard owner is unavailable; original clipboard untouched"
        );
    }
    Ok(TextSnapshot { entries })
}

fn snapshot_mime_types(types: &str) -> Result<Vec<&str>> {
    let mut mimes: Vec<_> = types.lines().collect();
    anyhow::ensure!(
        mimes.len() <= MAX_MIME_TYPES,
        "Clipboard has too many MIME types to preserve"
    );
    for mime in &mimes {
        anyhow::ensure!(
            !mime.is_empty() && mime.len() <= MAX_MIME_BYTES,
            "Invalid clipboard MIME type"
        );
    }
    // wl-copy can advertise its explicit text type again among its text
    // aliases. One MIME name identifies one payload; preserve its first entry
    // instead of feeding duplicate names into the replacement owner protocol.
    let mut seen = std::collections::HashSet::new();
    mimes.retain(|mime| seen.insert(*mime));
    Ok(mimes)
}

pub fn restore_text_wayland(snapshot: TextSnapshot, offered: Option<(&str, &[u8])>) -> Result<()> {
    restore_text_wayland_if_owned(snapshot, offered, still_ours, |snapshot| {
        match snapshot.entries.as_slice() {
            [] => clear_wayland(),
            [(mime, bytes)] => copy_wayland(mime, bytes),
            _ => copy_multi_wayland(&snapshot.entries),
        }
    })
}

fn restore_text_wayland_if_owned(
    snapshot: TextSnapshot,
    offered: Option<(&str, &[u8])>,
    mut owns: impl FnMut(&str, &[u8]) -> Result<bool>,
    restore: impl FnOnce(TextSnapshot) -> Result<()>,
) -> Result<()> {
    let Some((mime, expected)) = offered else {
        return Ok(());
    };
    anyhow::ensure!(
        owns(mime, expected)?,
        "Clipboard changed during handoff; leaving the newer content untouched"
    );
    restore(snapshot)
}

fn copy_multi_wayland(entries: &[(String, Vec<u8>)]) -> Result<()> {
    let display = super::super::wayland_display().context("No Wayland display")?;
    copy_multi_wayland_with(
        entries,
        owner_executable().context("Locating clipboard owner")?,
        display,
        std::time::Duration::from_secs(4),
    )
}

fn copy_multi_wayland_with(
    entries: &[(String, Vec<u8>)],
    executable: std::path::PathBuf,
    display: std::ffi::OsString,
    timeout: std::time::Duration,
) -> Result<()> {
    copy_multi_wayland_checked(entries, executable, display, timeout, still_ours)
}

fn copy_multi_wayland_checked(
    entries: &[(String, Vec<u8>)],
    executable: std::path::PathBuf,
    display: std::ffi::OsString,
    timeout: std::time::Duration,
    mut owns: impl FnMut(&str, &[u8]) -> Result<bool>,
) -> Result<()> {
    validate_entries(entries)?;
    let deadline = std::time::Instant::now() + timeout;
    let mut child = Command::new(executable)
        .arg(OWNER_ARGUMENT)
        .env("WAYLAND_DISPLAY", display)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("Starting the native Wayland clipboard owner")?;
    let stdin = child.stdin.take().context("No clipboard owner input")?;
    let entries_to_write = entries.to_vec();
    let (write_sender, write_receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let write = (|| -> Result<()> {
            let mut stdin = stdin;
            stdin.write_all(&(entries_to_write.len() as u32).to_be_bytes())?;
            for (mime, bytes) in &entries_to_write {
                stdin.write_all(&(mime.len() as u32).to_be_bytes())?;
                stdin.write_all(mime.as_bytes())?;
                stdin.write_all(&(bytes.len() as u32).to_be_bytes())?;
                stdin.write_all(bytes)?;
            }
            Ok(())
        })();
        let _ = write_sender.send(write);
    });
    let write_timeout = deadline.saturating_duration_since(std::time::Instant::now());
    let write = write_receiver.recv_timeout(write_timeout);
    match write {
        Ok(Ok(())) => {}
        Ok(Err(error)) => {
            stop_owner(&mut child);
            return Err(error);
        }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            stop_owner(&mut child);
            anyhow::bail!("Rich clipboard owner input timed out")
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            stop_owner(&mut child);
            anyhow::bail!("Rich clipboard owner input failed")
        }
    }
    let mut stdout = child
        .stdout
        .take()
        .context("No clipboard owner confirmation")?;
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        let read = std::io::BufReader::new(&mut stdout).read_line(&mut line);
        let _ = sender.send((read, line));
    });
    let confirmation =
        receiver.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()));
    if !matches!(confirmation, Ok((Ok(_), line)) if line == "READY\n") {
        stop_owner(&mut child);
        anyhow::bail!("Rich clipboard restore was not confirmed");
    }
    let offered = (|| -> Result<bool> {
        for (mime, bytes) in entries {
            if !owns(mime, bytes)? {
                return Ok(false);
            }
        }
        Ok(true)
    })();
    if !matches!(offered, Ok(true)) {
        stop_owner(&mut child);
        anyhow::bail!("Rich clipboard restore did not own the expected data");
    }
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

fn stop_owner(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn validate_entries(entries: &[(String, Vec<u8>)]) -> Result<()> {
    anyhow::ensure!(
        (1..=MAX_MIME_TYPES).contains(&entries.len()),
        "Invalid clipboard MIME count"
    );
    let mut total = 0usize;
    for (index, (mime, bytes)) in entries.iter().enumerate() {
        anyhow::ensure!(
            !mime.is_empty()
                && mime.len() <= MAX_MIME_BYTES
                && !mime.bytes().any(|byte| matches!(byte, b'\n' | b'\r' | 0)),
            "Invalid clipboard MIME type"
        );
        anyhow::ensure!(
            !entries[..index].iter().any(|(other, _)| other == mime),
            "Duplicate clipboard MIME type"
        );
        total = total
            .checked_add(bytes.len())
            .context("Clipboard snapshot is too large")?;
        anyhow::ensure!(
            total <= MAX_CLIPBOARD_BYTES,
            "Clipboard snapshot is too large"
        );
    }
    Ok(())
}

fn read_owner_entries(mut input: impl std::io::Read) -> Result<Vec<(String, Vec<u8>)>> {
    fn read_size(input: &mut impl std::io::Read) -> Result<usize> {
        let mut bytes = [0; 4];
        input
            .read_exact(&mut bytes)
            .context("Incomplete clipboard offer")?;
        Ok(u32::from_be_bytes(bytes) as usize)
    }

    let count = read_size(&mut input)?;
    anyhow::ensure!(
        (1..=MAX_MIME_TYPES).contains(&count),
        "Invalid clipboard MIME count"
    );
    let mut entries = Vec::with_capacity(count);
    let mut total = 0usize;
    for _ in 0..count {
        let mime_size = read_size(&mut input)?;
        anyhow::ensure!(
            (1..=MAX_MIME_BYTES).contains(&mime_size),
            "Invalid clipboard MIME size"
        );
        let mut mime = vec![0; mime_size];
        input
            .read_exact(&mut mime)
            .context("Incomplete clipboard MIME type")?;
        let mime = String::from_utf8(mime).context("Invalid clipboard MIME type")?;
        let size = read_size(&mut input)?;
        total = total
            .checked_add(size)
            .context("Clipboard snapshot is too large")?;
        anyhow::ensure!(
            total <= MAX_CLIPBOARD_BYTES,
            "Clipboard snapshot is too large"
        );
        let mut bytes = vec![0; size];
        input
            .read_exact(&mut bytes)
            .context("Incomplete clipboard data")?;
        entries.push((mime, bytes));
    }
    let mut trailing = [0];
    anyhow::ensure!(
        input.read(&mut trailing)? == 0,
        "Unexpected clipboard protocol data"
    );
    validate_entries(&entries)?;
    Ok(entries)
}

/// Internal entry point used by this process as the lifetime owner for a rich Wayland offer.
pub fn run_owner_protocol() -> Result<()> {
    let entries = read_owner_entries(std::io::stdin().lock())?;
    let sources = entries
        .into_iter()
        .map(|(mime, bytes)| MimeSource {
            source: Source::Bytes(bytes.into_boxed_slice()),
            mime_type: MimeType::Specific(mime),
        })
        .collect();
    let mut options = Options::new();
    options
        .foreground(true)
        .omit_additional_text_mime_types(true);
    let prepared = options
        .prepare_copy_multi(sources)
        .context("Could not acquire the Wayland clipboard")?;
    println!("READY");
    std::io::stdout().flush()?;
    prepared.serve().context("Serving the Wayland clipboard")
}

fn clear_wayland() -> Result<()> {
    let status = Command::new("wl-copy")
        .arg("--clear")
        .env(
            "WAYLAND_DISPLAY",
            super::super::wayland_display().context("No Wayland display")?,
        )
        .status()
        .context("Clearing original empty clipboard")?;
    anyhow::ensure!(status.success(), "Could not restore empty clipboard");
    Ok(())
}

pub fn still_ours(mime: &str, expected: &[u8]) -> Result<bool> {
    let offered = read_wayland(&["--list-types"], 8192)?;
    let offered = std::str::from_utf8(&offered).context("Invalid clipboard types")?;
    if !offered.lines().any(|item| item == mime) {
        return Ok(false);
    }
    Ok(read_wayland(&["--no-newline", "--type", mime], 16 * 1024 * 1024)? == expected)
}

fn read_wayland(args: &[&str], limit: usize) -> Result<Vec<u8>> {
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("wl-paste")))
        .filter(|path| path.is_file());
    let mut child = Command::new("timeout")
        .arg("3s")
        .arg(bundled.unwrap_or_else(|| "wl-paste".into()))
        .args(args)
        .env(
            "WAYLAND_DISPLAY",
            super::super::wayland_display().context("No Wayland display")?,
        )
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("Native Wayland clipboard needs wl-paste and timeout")?;
    let mut bytes = Vec::new();
    let read = child
        .stdout
        .take()
        .context("No clipboard output")?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes);
    if bytes.len() > limit || read.is_err() {
        let _ = child.kill();
    }
    let status = child.wait().context("Waiting for wl-paste")?;
    read.context("Reading clipboard")?;
    anyhow::ensure!(
        bytes.len() <= limit,
        "Clipboard too large to restore safely"
    );
    if args == ["--list-types"] && status.code() == Some(1) && bytes.is_empty() {
        return Ok(bytes);
    }
    anyhow::ensure!(
        status.success(),
        "Could not read clipboard (wl-paste: {status})"
    );
    Ok(bytes)
}

pub fn set_text(text: &str) -> Result<()> {
    if native_wayland_clipboard() {
        return copy_wayland("text/plain;charset=utf-8", text.as_bytes());
    }
    with(|c| c.set_text(text.to_owned()))
}

pub fn set_png(png_bytes: &[u8]) -> Result<()> {
    if native_wayland_clipboard() {
        return copy_wayland("image/png", png_bytes);
    }
    let decoder = png::Decoder::new(std::io::Cursor::new(png_bytes));
    let mut reader = decoder.read_info().context("Not a PNG")?;
    let mut buffer = vec![0; reader.output_buffer_size().context("PNG too large")?];
    let info = reader.next_frame(&mut buffer)?;
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buffer[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => buffer[..info.buffer_size()]
            .chunks_exact(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        other => anyhow::bail!("Unsupported PNG colour type {other:?}"),
    };
    with(|c| {
        c.set_image(arboard::ImageData {
            width: info.width as usize,
            height: info.height as usize,
            bytes: rgba.into(),
        })
    })
}

/// Offer both formats on one Wayland selection. arboard replaces its previous
/// format on X11, so the caller falls back to text there.
pub fn set_png_and_text(png_bytes: &[u8], text: &str) -> Result<()> {
    anyhow::ensure!(
        native_wayland_clipboard(),
        "Combined image and text clipboard is unavailable on this X11 session"
    );
    anyhow::ensure!(
        png_bytes.starts_with(b"\x89PNG\r\n\x1a\n"),
        "Contact sheet is not a PNG"
    );
    copy_multi_wayland(&[
        ("image/png".into(), png_bytes.to_vec()),
        ("text/plain;charset=utf-8".into(), text.as_bytes().to_vec()),
    ])
}

fn copy_wayland(mime: &str, bytes: &[u8]) -> Result<()> {
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("wl-copy")))
        .filter(|path| path.is_file());
    let mut child = Command::new(bundled.unwrap_or_else(|| "wl-copy".into()))
        .args(["--type", mime])
        .env(
            "WAYLAND_DISPLAY",
            super::super::wayland_display().context("No Wayland display")?,
        )
        .stdin(Stdio::piped())
        // wl-copy forks a clipboard owner after acquiring the selection. It must not keep
        // pipes to Apparent open; waiting for captured stdout/stderr would never finish.
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("Native Wayland clipboard needs wl-copy (install wl-clipboard)")?;
    let written = child
        .stdin
        .take()
        .context("No clipboard input")?
        .write_all(bytes);
    let status = child.wait().context("Waiting for wl-copy")?;
    written.context("Writing the Wayland clipboard")?;
    anyhow::ensure!(status.success(), "wl-copy failed: {status}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_preserves_each_mime_once_when_the_owner_advertises_duplicates() {
        assert_eq!(
            snapshot_mime_types("text/plain;charset=utf-8\ntext/plain\ntext/plain;charset=utf-8\n")
                .unwrap(),
            vec!["text/plain;charset=utf-8", "text/plain"]
        );
    }

    #[test]
    fn application_owner_fails_closed_before_acquiring_a_selection() {
        let Some(executable) = std::env::var_os("APPARENT_TEST_CLIPBOARD_OWNER") else {
            return;
        };
        let root = std::env::temp_dir().join(crate::shared::ids::make_bundle_id());
        std::fs::create_dir(&root).unwrap();
        let launch = || {
            let mut command = Command::new(&executable);
            // An owned directory with no compositor socket: never access the
            // host clipboard, even when this test runs in a desktop session.
            command
                .env("XDG_RUNTIME_DIR", &root)
                .env("WAYLAND_DISPLAY", "missing-socket");
            command
        };
        let check = launch().args([OWNER_ARGUMENT, "--check"]).output().unwrap();
        assert!(!check.status.success());
        assert!(check.stdout.is_empty());
        for bytes in [vec![0, 0, 0, 0], vec![0, 0, 0, 33], vec![0, 0, 0, 1, 0]] {
            let mut child = launch()
                .arg(OWNER_ARGUMENT)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            child.stdin.take().unwrap().write_all(&bytes).unwrap();
            let output = child.wait_with_output().unwrap();
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn rich_offer_protocol_round_trips_exact_mime_and_binary_bytes() {
        let entries = vec![
            (
                "text/html".to_owned(),
                b"<strong>one\0two</strong>".to_vec(),
            ),
            ("image/png".to_owned(), vec![0, 255, 1, 2, 0]),
        ];
        let mut wire = Vec::new();
        wire.extend_from_slice(&(entries.len() as u32).to_be_bytes());
        for (mime, bytes) in &entries {
            wire.extend_from_slice(&(mime.len() as u32).to_be_bytes());
            wire.extend_from_slice(mime.as_bytes());
            wire.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
            wire.extend_from_slice(bytes);
        }
        assert_eq!(read_owner_entries(wire.as_slice()).unwrap(), entries);
    }

    #[test]
    fn rich_offer_protocol_rejects_truncation_extra_bytes_and_duplicate_types() {
        assert!(read_owner_entries([0, 0, 0, 1, 0].as_slice()).is_err());

        let mut extra = Vec::new();
        extra.extend_from_slice(&1u32.to_be_bytes());
        extra.extend_from_slice(&9u32.to_be_bytes());
        extra.extend_from_slice(b"text/html");
        extra.extend_from_slice(&0u32.to_be_bytes());
        extra.push(1);
        assert!(read_owner_entries(extra.as_slice()).is_err());

        assert!(
            validate_entries(&[
                ("text/plain".to_owned(), b"first".to_vec()),
                ("text/plain".to_owned(), b"second".to_vec()),
            ])
            .is_err()
        );
    }

    #[test]
    fn rich_offer_protocol_enforces_existing_limits() {
        assert!(validate_entries(&[]).is_err());
        assert!(validate_entries(&[("x".repeat(MAX_MIME_BYTES + 1), vec![])]).is_err());
        assert!(
            validate_entries(&[("text/plain".to_owned(), vec![0; MAX_CLIPBOARD_BYTES + 1])])
                .is_err()
        );
        assert!(validate_entries(&vec![("x".to_owned(), vec![]); MAX_MIME_TYPES + 1]).is_err());
    }

    #[cfg(unix)]
    fn owner_fixture(script: &str) -> (std::path::PathBuf, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt as _;
        let root = std::env::temp_dir().join(crate::shared::ids::make_bundle_id());
        std::fs::create_dir(&root).unwrap();
        let executable = root.join("owner");
        std::fs::write(&executable, format!("#!/bin/sh\n{script}\n")).unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        (root, executable)
    }

    #[cfg(unix)]
    fn fixture_offer() -> Vec<(String, Vec<u8>)> {
        vec![("text/plain".into(), b"synthetic offer".to_vec())]
    }

    #[cfg(unix)]
    #[test]
    fn rich_owner_startup_failure_is_not_reported_as_success() {
        let (root, executable) = owner_fixture("exit 17");
        let result = copy_multi_wayland_with(
            &fixture_offer(),
            executable,
            "unused-test-display".into(),
            std::time::Duration::from_secs(1),
        );
        assert!(result.is_err());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rich_owner_timeout_covers_a_child_stalled_before_reading_offer() {
        let (root, executable) = owner_fixture("exec sleep 5");
        let started = std::time::Instant::now();
        let result = copy_multi_wayland_with(
            &[("text/plain".into(), vec![7; 1024 * 1024])],
            executable,
            "unused-test-display".into(),
            std::time::Duration::from_millis(150),
        );
        assert!(result.is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn rich_owner_selection_loss_after_ready_is_not_reported_as_success() {
        let (root, executable) = owner_fixture("cat >/dev/null; printf 'READY\\n'; exec sleep 5");
        let started = std::time::Instant::now();
        let result = copy_multi_wayland_checked(
            &fixture_offer(),
            executable,
            "unused-test-display".into(),
            std::time::Duration::from_secs(1),
            |_, _| Ok(false),
        );
        assert!(result.is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn restore_leaves_selection_loss_after_handoff_untouched() {
        let snapshot = TextSnapshot {
            entries: vec![("text/plain".into(), b"original".to_vec())],
        };
        let mut restored = false;
        let result = restore_text_wayland_if_owned(
            snapshot,
            Some(("image/png", b"offered")),
            |_, _| Ok(false),
            |_| {
                restored = true;
                Ok(())
            },
        );
        assert!(result.is_err());
        assert!(!restored);
    }

    #[test]
    fn snapshot_policy_accepts_rich_and_image_clipboards_without_losing_their_types() {
        assert_eq!(
            plain_text_mime("text/plain;charset=utf-8\ntext/plain\n").unwrap(),
            "text/plain;charset=utf-8"
        );
        assert!(plain_text_mime("text/plain\nimage/png\n").is_ok());
        assert!(plain_text_mime("text/html\ntext/plain\n").is_ok());
        assert!(plain_text_mime("image/png\n").is_ok());
        assert!(plain_text_mime("").is_err());
    }

    #[test]
    fn isolated_wayland_restores_exact_text_after_png_offer() {
        if std::env::var_os("APPARENT_TEST_WAYLAND_CLIPBOARD").is_none() {
            return;
        }
        let original = b"synthetic original\n";
        copy_wayland("text/plain;charset=utf-8", original).unwrap();
        let snapshot = snapshot_text_wayland().unwrap();
        assert!(
            snapshot
                .entries
                .iter()
                .any(|(mime, bytes)| mime == "text/plain;charset=utf-8" && bytes == original)
        );
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[255, 0, 0, 255])
                .unwrap();
        }
        copy_wayland("image/png", &png).unwrap();
        assert!(
            snapshot_text_wayland()
                .unwrap()
                .entries
                .iter()
                .any(|(mime, bytes)| mime == "image/png" && bytes == &png)
        );
        restore_text_wayland(snapshot, Some(("image/png", &png))).unwrap();
        assert_eq!(
            read_wayland(
                &["--no-newline", "--type", "text/plain;charset=utf-8"],
                1024
            )
            .unwrap(),
            original
        );
        let snapshot = snapshot_text_wayland().unwrap();
        copy_wayland("image/png", &png).unwrap();
        copy_wayland("text/plain;charset=utf-8", b"newer synthetic clipboard").unwrap();
        assert!(restore_text_wayland(snapshot, Some(("image/png", &png))).is_err());
        assert_eq!(
            read_wayland(
                &["--no-newline", "--type", "text/plain;charset=utf-8"],
                1024
            )
            .unwrap(),
            b"newer synthetic clipboard"
        );
    }

    #[test]
    fn isolated_combined_offer_has_both_mime_types_and_exact_bytes() {
        if std::env::var_os("APPARENT_TEST_WAYLAND_CLIPBOARD").is_none() {
            return;
        }
        let previous = snapshot_text_wayland().unwrap();
        let mut png = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[7, 8, 9, 255])
                .unwrap();
        }
        let text = "test summary";
        set_png_and_text(&png, text).unwrap();
        assert!(still_ours("image/png", &png).unwrap());
        assert!(still_ours("text/plain;charset=utf-8", text.as_bytes()).unwrap());
        restore_text_wayland(
            previous,
            Some(("text/plain;charset=utf-8", text.as_bytes())),
        )
        .unwrap();
    }

    #[test]
    fn isolated_wayland_preserves_rich_clipboard_after_two_offers() {
        if std::env::var_os("APPARENT_TEST_RICH_WAYLAND_CLIPBOARD").is_none() {
            return;
        }
        let original = snapshot_text_wayland().unwrap();
        let html = b"<strong>synthetic</strong>";
        let plain = b"synthetic";
        let marker = b"apparent-rich-restore-test-marker";
        let image = b"synthetic-image";
        let mut offered: (&str, &[u8]) = ("text/html", html);
        let probe = (|| -> Result<()> {
            copy_multi_wayland(&[
                ("text/html".into(), html.to_vec()),
                ("text/plain".into(), plain.to_vec()),
            ])?;
            let rich = snapshot_text_wayland()?;
            anyhow::ensure!(
                rich.entries
                    .iter()
                    .any(|(mime, bytes)| mime == "text/html" && bytes == html),
                "HTML not offered: {:?}",
                rich.entries
                    .iter()
                    .map(|(mime, bytes)| (mime, bytes.len()))
                    .collect::<Vec<_>>()
            );
            anyhow::ensure!(
                rich.entries
                    .iter()
                    .any(|(mime, bytes)| mime == "text/plain" && bytes == plain),
                "Plain text not offered"
            );
            offered = ("image/png", image);
            copy_wayland("image/png", image)?;
            offered = ("text/plain;charset=utf-8", marker);
            copy_wayland("text/plain;charset=utf-8", marker)?;
            restore_text_wayland(rich, Some(("text/plain;charset=utf-8", marker)))?;
            offered = ("text/html", html);
            std::thread::sleep(std::time::Duration::from_secs(1));
            anyhow::ensure!(
                read_wayland(&["--no-newline", "--type", "text/html"], 1024)? == html,
                "HTML not restored"
            );
            anyhow::ensure!(
                read_wayland(&["--no-newline", "--type", "text/plain"], 1024)? == plain,
                "Plain text not restored"
            );
            Ok(())
        })();
        if still_ours(offered.0, offered.1).unwrap_or(false) {
            restore_text_wayland(original, Some(offered)).unwrap();
        }
        probe.unwrap();
    }
}
