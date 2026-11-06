//! Text and PNG through X11 selections, except on KDE Wayland and Hyprland: neither reliably
//! bridges a background XWayland owner's selection to native apps. There we use wl-copy's
//! data-control protocol (bundled in the AppImage). Both backends keep an owner alive to serve
//! the data. Verified text writes on native Wayland use this module's existing in-process owner
//! protocol and exact wl-paste readback, but always return outcome-unknown: the Wayland clipboard
//! API used here has no owner-identity query, so a foreign owner offering identical bytes cannot
//! be distinguished from our still-live owner. Never infer applied from bytes plus liveness.

use std::io::{BufRead as _, Read as _, Write as _};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

use anyhow::{Context, Result, anyhow};
use wl_clipboard_rs::copy::{MimeSource, MimeType, Options, Source};

static CLIPBOARD: Mutex<Option<arboard::Clipboard>> = Mutex::new(None);
static VERIFIED_TEXT_OWNER: std::sync::OnceLock<Option<std::sync::mpsc::SyncSender<VerifiedTextOwnerRequest>>> = std::sync::OnceLock::new();
const MAX_MIME_TYPES: usize = 32;
const MAX_MIME_BYTES: usize = 256;
const MAX_CLIPBOARD_BYTES: usize = 16 * 1024 * 1024;
const OWNER_ARGUMENT: &str = "--internal-wayland-clipboard-owner";

enum VerifiedTextOwnerBackend {
    X11(std::path::PathBuf),
    Wayland(std::path::PathBuf, std::ffi::OsString),
}

struct VerifiedTextOwnerRequest {
    text: String,
    deadline: std::time::Instant,
    backend: VerifiedTextOwnerBackend,
    reply: std::sync::mpsc::SyncSender<VerifiedTextWrite>,
}

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
    let mut command = Command::new(executable);
    command.args([OWNER_ARGUMENT, "--check"]).env("WAYLAND_DISPLAY", display);
    run_bounded_command(command, std::time::Instant::now() + std::time::Duration::from_secs(3), 1024)
        .is_ok_and(|(status, stdout)| status.success() && stdout == b"READY\n")
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
    let child = copy_multi_wayland_checked(entries, executable, display, timeout, still_ours)?;
    std::thread::spawn(move || {
        let mut child = child;
        let _ = child.wait();
    });
    Ok(())
}

fn copy_multi_wayland_checked(
    entries: &[(String, Vec<u8>)],
    executable: std::path::PathBuf,
    display: std::ffi::OsString,
    timeout: std::time::Duration,
    mut owns: impl FnMut(&str, &[u8]) -> Result<bool>,
) -> Result<Child> {
    validate_entries(entries)?;
    let deadline = std::time::Instant::now() + timeout;
    let mut child = Command::new(executable)
        .arg(OWNER_ARGUMENT)
        .env("WAYLAND_DISPLAY", display)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // The helper's bounded protocol errors contain operation state only,
        // never clipboard text. Keep them with the owning GUI's diagnostics.
        .stderr(Stdio::inherit())
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
            let _ = write_receiver.recv_timeout(std::time::Duration::from_millis(100));
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
        let _ = receiver.recv_timeout(std::time::Duration::from_millis(100));
        anyhow::bail!("Rich clipboard restore was not confirmed");
    }
    if !wait_for_offered_until(entries, deadline, &mut owns) {
        stop_owner(&mut child);
        anyhow::bail!("Rich clipboard restore did not own the expected data");
    }
    if std::time::Instant::now() >= deadline {
        stop_owner(&mut child);
        anyhow::bail!("Rich clipboard owner confirmation exceeded its deadline");
    }
    Ok(child)
}

/// The owner protocol queues its selection request before serving starts. A
/// readback may briefly report an empty selection until the server dispatches
/// that request, so confirm only by bounded readback polling. This does not
/// establish owner identity; callers must keep Wayland results conservative.
fn wait_for_offered_until(
    entries: &[(String, Vec<u8>)],
    deadline: std::time::Instant,
    owns: &mut impl FnMut(&str, &[u8]) -> Result<bool>,
) -> bool {
    loop {
        let exact = entries
            .iter()
            .all(|(mime, bytes)| matches!(owns(mime, bytes), Ok(true)));
        if exact {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
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
    install_parent_death_signal()?;
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
    still_ours_until(mime, expected, std::time::Instant::now() + std::time::Duration::from_secs(3))
}

fn still_ours_until(mime: &str, expected: &[u8], deadline: std::time::Instant) -> Result<bool> {
    let offered = read_wayland_until(&["--list-types"], 8192, deadline)?;
    let offered = std::str::from_utf8(&offered).context("Invalid clipboard types")?;
    if !offered.lines().any(|item| item == mime) {
        return Ok(false);
    }
    Ok(read_wayland_until(&["--no-newline", "--type", mime], 16 * 1024 * 1024, deadline)? == expected)
}

fn read_wayland(args: &[&str], limit: usize) -> Result<Vec<u8>> {
    read_wayland_until(args, limit, std::time::Instant::now() + std::time::Duration::from_secs(3))
}

fn read_wayland_until(args: &[&str], limit: usize, deadline: std::time::Instant) -> Result<Vec<u8>> {
    let executable = clipboard_tool("wl-paste").context("Native Wayland clipboard needs bundled wl-paste or wl-paste on PATH")?;
    let display = super::super::wayland_display().context("No Wayland display")?;
    let mut command = Command::new(executable);
    command.args(args).env("WAYLAND_DISPLAY", display);
    let (status, bytes) = run_bounded_command(command, deadline, limit)?;
    if args == ["--list-types"] && status.code() == Some(1) && bytes.is_empty() {
        return Ok(bytes);
    }
    anyhow::ensure!(
        status.success(),
        "Could not read clipboard (wl-paste: {status})"
    );
    Ok(bytes)
}

fn run_bounded_command(
    mut command: Command,
    deadline: std::time::Instant,
    stdout_limit: usize,
) -> Result<(std::process::ExitStatus, Vec<u8>)> {
    let mut child = command.stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    let stdout = child.stdout.take().context("No clipboard command output")?;
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout.take(stdout_limit as u64 + 1).read_to_end(&mut bytes);
        let _ = tx.send((result, bytes));
    });
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(error) => {
                stop_owner(&mut child);
                let _ = rx.recv_timeout(std::time::Duration::from_millis(100));
                return Err(error.into());
            }
        }
        if std::time::Instant::now() >= deadline {
            stop_owner(&mut child);
            let _ = rx.recv_timeout(std::time::Duration::from_millis(100));
            anyhow::bail!("Clipboard command deadline expired");
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    let (read, bytes) = rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
        .context("Clipboard command output did not close")?;
    read.context("Reading clipboard command output")?;
    anyhow::ensure!(bytes.len() <= stdout_limit, "Clipboard output exceeds limit");
    Ok((status, bytes))
}

pub fn set_text(text: &str) -> Result<()> {
    if native_wayland_clipboard() {
        return copy_wayland("text/plain;charset=utf-8", text.as_bytes());
    }
    with(|c| c.set_text(text.to_owned()))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VerifiedTextWrite {
    Applied,
    Unavailable,
    OutcomeUnknown,
}

/// Writes through the existing clipboard backend and confirms both its owner and exact text.
/// The text is never returned or logged. Unknown leaves whatever owner currently has the
/// selection untouched.
pub fn set_text_verified_until(text: &str, deadline: std::time::Instant) -> VerifiedTextWrite {
    if text.contains('\0') {
        return VerifiedTextWrite::Unavailable;
    }
    if std::time::Instant::now() >= deadline {
        return VerifiedTextWrite::Unavailable;
    }
    let executable = match std::env::current_exe() {
        Ok(path) => path,
        Err(_) => return VerifiedTextWrite::Unavailable,
    };
    let backend = if native_wayland_clipboard() {
        let Some(display) = super::super::wayland_display() else {
            return VerifiedTextWrite::Unavailable;
        };
        if clipboard_tool("wl-paste").is_none() {
            return VerifiedTextWrite::Unavailable;
        }
        VerifiedTextOwnerBackend::Wayland(executable, display)
    } else {
        VerifiedTextOwnerBackend::X11(executable)
    };
    submit_verified_text_owner(text, deadline, backend)
}

#[cfg(target_os = "linux")]
fn install_parent_death_signal() -> Result<()> {
    let parent = unsafe { libc::getppid() };
    let result = unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) };
    anyhow::ensure!(result == 0, "Could not bind clipboard owner to GUI lifetime");
    anyhow::ensure!(unsafe { libc::getppid() } == parent && parent != 1, "Clipboard owner parent exited during startup");
    Ok(())
}

fn clipboard_tool(name: &str) -> Option<std::path::PathBuf> {
    clipboard_tool_from(name, std::env::current_exe().ok()?.parent()?, std::env::var_os("PATH").as_deref())
}

fn clipboard_tool_from(
    name: &str,
    bundled_directory: &std::path::Path,
    path: Option<&std::ffi::OsStr>,
) -> Option<std::path::PathBuf> {
    let bundled = bundled_directory.join(name);
    if bundled.is_file() {
        return Some(bundled);
    }
    std::env::split_paths(path?)
        .map(|dir| dir.join(name))
        .find(|path| path.is_file())
}

fn submit_verified_text_owner(
    text: &str,
    deadline: std::time::Instant,
    backend: VerifiedTextOwnerBackend,
) -> VerifiedTextWrite {
    use std::sync::mpsc::{TrySendError, sync_channel};

    let Some(sender) = verified_text_owner_worker() else {
        return VerifiedTextWrite::Unavailable;
    };
    let (reply, receiver) = sync_channel(1);
    let request = VerifiedTextOwnerRequest {
        text: text.to_owned(),
        deadline,
        backend,
        reply,
    };
    match sender.try_send(request) {
        Ok(()) => {}
        Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {
            return VerifiedTextWrite::Unavailable;
        }
    }
    receiver
        .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
        .unwrap_or(VerifiedTextWrite::OutcomeUnknown)
}

/// The persistent owner lane spawns clipboard helpers from a thread that lives
/// for the GUI process. Linux PR_SET_PDEATHSIG is thread-scoped, so spawning
/// from the request's short-lived deadline worker would kill the selection
/// owner as soon as that request returned.
fn verified_text_owner_worker() -> Option<&'static std::sync::mpsc::SyncSender<VerifiedTextOwnerRequest>> {
    VERIFIED_TEXT_OWNER
        .get_or_init(|| {
            use std::sync::mpsc::sync_channel;
            let (sender, receiver) = sync_channel(1);
            std::thread::Builder::new()
                .name("apparent-clipboard-owner".into())
                .spawn(move || verified_text_owner_loop(receiver))
                .ok()
                .map(|_| sender)
        })
        .as_ref()
}

fn verified_text_owner_loop(receiver: std::sync::mpsc::Receiver<VerifiedTextOwnerRequest>) {
    let mut owner: Option<Child> = None;
    while let Ok(request) = receiver.recv() {
        let result = if std::time::Instant::now() >= request.deadline {
            VerifiedTextWrite::Unavailable
        } else {
            match request.backend {
                VerifiedTextOwnerBackend::X11(executable) => {
                    set_text_x11_verified(&request.text, request.deadline, executable, &mut owner)
                }
                VerifiedTextOwnerBackend::Wayland(executable, display) => {
                    set_text_wayland_verified(
                        &request.text,
                        request.deadline,
                        executable,
                        display,
                        &mut owner,
                    )
                }
            }
        };
        let _ = request.reply.send(result);
    }
    if let Some(mut owner) = owner {
        stop_owner(&mut owner);
    }
}

fn set_text_x11_verified(
    text: &str,
    deadline: std::time::Instant,
    executable: std::path::PathBuf,
    owner: &mut Option<Child>,
) -> VerifiedTextWrite {
    use std::io::Write as _;

    let mut child = match Command::new(executable)
        .arg("--internal-native-clipboard-text-owner")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(_) => return VerifiedTextWrite::Unavailable,
    };
    let Some(mut input) = child.stdin.take() else {
        stop_owner(&mut child);
        return VerifiedTextWrite::OutcomeUnknown;
    };
    let bytes = text.as_bytes().to_vec();
    let (write_tx, write_rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result = input.write_all(&bytes);
        drop(input);
        let _ = write_tx.send(result);
    });
    if !matches!(write_rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now())), Ok(Ok(()))) {
        stop_owner(&mut child);
        let _ = write_rx.recv_timeout(std::time::Duration::from_millis(100));
        return VerifiedTextWrite::OutcomeUnknown;
    }
    let Some(mut stdout) = child.stdout.take() else {
        stop_owner(&mut child);
        return VerifiedTextWrite::OutcomeUnknown;
    };
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = std::io::BufRead::read_line(&mut std::io::BufReader::new(&mut stdout), &mut line);
        let _ = tx.send((result, line));
    });
    let response = rx.recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()));
    match response {
        Ok((Ok(_), line)) if line == "APPLIED\n" && std::time::Instant::now() < deadline => {
            if let Some(mut previous) = owner.replace(child) { stop_owner(&mut previous); }
            VerifiedTextWrite::Applied
        }
        Ok((Ok(_), line)) if line == "UNAVAILABLE\n" => {
            let _ = child.wait();
            VerifiedTextWrite::Unavailable
        }
        _ => {
            stop_owner(&mut child);
            let _ = rx.recv_timeout(std::time::Duration::from_millis(100));
            VerifiedTextWrite::OutcomeUnknown
        }
    }
}

pub fn run_x11_text_owner(text: &str) -> Result<()> {
    install_parent_death_signal()?;
    anyhow::ensure!(!native_wayland_clipboard(), "X11 helper is unavailable on native Wayland");
    if !x11_resource_available() {
        println!("UNAVAILABLE");
        std::io::stdout().flush()?;
        return Ok(());
    }
    if with(|clipboard| clipboard.set_text(text.to_owned())).is_err() {
        println!("UNKNOWN");
        std::io::stdout().flush()?;
        return Ok(());
    }
    let pid = std::process::id();
    let owner_before = x11_clipboard_owner_pid();
    let exact = with(|clipboard| clipboard.get_text().map(|actual| actual == text)).unwrap_or(false);
    let owner_after = x11_clipboard_owner_pid();
    if !x11_write_confirmed(owner_before, exact, owner_after, pid) {
        println!("UNKNOWN");
        std::io::stdout().flush()?;
        return Ok(());
    }
    println!("APPLIED");
    std::io::stdout().flush()?;
    while x11_clipboard_owner_pid() == Some(pid) {
        std::thread::sleep(std::time::Duration::from_millis(40));
    }
    Ok(())
}

fn set_text_wayland_verified(
    text: &str,
    deadline: std::time::Instant,
    executable: std::path::PathBuf,
    display: std::ffi::OsString,
    owner: &mut Option<Child>,
) -> VerifiedTextWrite {
    if std::time::Instant::now() >= deadline { return VerifiedTextWrite::Unavailable; }
    let entries = vec![("text/plain;charset=utf-8".to_owned(), text.as_bytes().to_vec())];
    let child = match copy_multi_wayland_checked(
        &entries,
        executable,
        display,
        deadline.saturating_duration_since(std::time::Instant::now()),
        |mime, bytes| still_ours_until(mime, bytes, deadline),
    ) {
        Ok(child) => child,
        Err(_) => return VerifiedTextWrite::OutcomeUnknown,
    };
    if std::time::Instant::now() >= deadline {
        let mut child = child;
        stop_owner(&mut child);
        return VerifiedTextWrite::OutcomeUnknown;
    }
    let mut child = child;
    if !matches!(child.try_wait(), Ok(None)) {
        stop_owner(&mut child);
        return VerifiedTextWrite::OutcomeUnknown;
    }
    if let Some(mut previous) = owner.replace(child) {
        stop_owner(&mut previous);
    }
    // Wayland has no supported selection-owner identity query. The owner helper's READY event
    // proves acquisition and wl-paste proves bytes, but an identical-content foreign owner can
    // replace it before the helper processes the loss event. Never upgrade that to Applied.
    wayland_verified_result(true, true)
}

fn x11_write_confirmed(owner_before: Option<u32>, exact_readback: bool, owner_after: Option<u32>, pid: u32) -> bool {
    owner_before == Some(pid) && exact_readback && owner_after == Some(pid)
}

fn wayland_verified_result(_acquisition_acknowledged: bool, _exact_readback: bool) -> VerifiedTextWrite {
    VerifiedTextWrite::OutcomeUnknown
}

fn x11_resource_available() -> bool {
    use x11rb::{
        connection::Connection,
        protocol::{res::{ClientIdMask, ClientIdSpec, ConnectionExt as _}, xproto::ConnectionExt as _},
    };
    let Ok((connection, _)) = x11rb::connect(None) else { return false; };
    if !connection.query_extension(b"X-Resource").ok()
        .and_then(|reply| reply.reply().ok()).is_some_and(|reply| reply.present) {
        return false;
    }
    let client = connection.setup().resource_id_base;
    connection.res_query_client_ids(&[ClientIdSpec { client, mask: ClientIdMask::LOCAL_CLIENT_PID }]).ok()
        .and_then(|reply| reply.reply().ok())
        .is_some_and(|reply| reply.ids.into_iter().any(|id|
            id.spec.client == client && id.spec.mask == ClientIdMask::LOCAL_CLIENT_PID
                && id.value.as_slice() == [std::process::id()]))
}

fn x11_clipboard_owner_pid() -> Option<u32> {
    use x11rb::{
        protocol::{res::{ClientIdMask, ClientIdSpec, ConnectionExt as _}, xproto::ConnectionExt as _},
    };
    let (connection, _) = x11rb::connect(None).ok()?;
    let atom = connection.intern_atom(false, b"CLIPBOARD").ok()?.reply().ok()?.atom;
    let owner = connection.get_selection_owner(atom).ok()?.reply().ok()?.owner;
    if owner == 0 { return None; }
    let spec = ClientIdSpec { client: owner, mask: ClientIdMask::LOCAL_CLIENT_PID };
    connection.res_query_client_ids(&[spec]).ok()?.reply().ok()?.ids.into_iter()
        .find(|id| id.spec.client == owner && id.spec.mask == ClientIdMask::LOCAL_CLIENT_PID)?
        .value.into_iter().next()
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
    fn verified_text_requires_our_live_x11_selection_owner_and_exact_readback() {
        assert!(x11_write_confirmed(Some(42), true, Some(42), 42));
        assert!(!x11_write_confirmed(Some(42), false, Some(42), 42));
        assert!(!x11_write_confirmed(Some(42), true, Some(99), 42));
        assert!(!x11_write_confirmed(Some(42), true, None, 42));
        assert_eq!(wayland_verified_result(true, true), VerifiedTextWrite::OutcomeUnknown);
        assert_eq!(wayland_verified_result(true, false), VerifiedTextWrite::OutcomeUnknown);
        assert_eq!(wayland_verified_result(false, true), VerifiedTextWrite::OutcomeUnknown);
    }

    #[test]
    fn bundled_wayland_tool_resolution_works_without_path() {
        let root = std::env::temp_dir().join(format!("apparent-clipboard-tools-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let bundled = root.join("wl-paste");
        std::fs::write(&bundled, b"tool").unwrap();
        assert_eq!(clipboard_tool_from("wl-paste", &root, None).as_deref(), Some(bundled.as_path()));
        assert_eq!(clipboard_tool_from("timeout", &root, None), None);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn bounded_child_timeout_kills_and_reaps_process() {
        let started = std::time::Instant::now();
        let mut command = Command::new("/bin/sleep");
        command.arg("5");
        let result = run_bounded_command(command, started + std::time::Duration::from_millis(75), 64);
        assert!(result.is_err());
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

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
    fn wayland_owner_confirmation_waits_for_selection_readback_after_ready() {
        let mut attempts = 0;
        let confirmed = wait_for_offered_until(
            &fixture_offer(),
            std::time::Instant::now() + std::time::Duration::from_secs(1),
            &mut |_, _| {
                attempts += 1;
                match attempts {
                    1 => Err(anyhow!("selection not dispatched yet")),
                    2 => Ok(false),
                    _ => Ok(true),
                }
            },
        );
        assert!(confirmed);
        assert_eq!(attempts, 3);
    }

    #[test]
    fn wayland_owner_confirmation_times_out_without_exact_readback() {
        let started = std::time::Instant::now();
        let mut attempts = 0;
        let confirmed = wait_for_offered_until(
            &fixture_offer(),
            started + std::time::Duration::from_millis(45),
            &mut |_, _| {
                attempts += 1;
                Ok(false)
            },
        );
        assert!(!confirmed);
        assert!(attempts > 1);
        assert!(started.elapsed() < std::time::Duration::from_secs(1));
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
    fn isolated_wayland_verified_owner_survives_the_request_thread() {
        if std::env::var_os("APPARENT_TEST_WAYLAND_CLIPBOARD").is_none() {
            return;
        }
        let text = "synthetic clipboard owner thread lifetime";
        let result = std::thread::spawn(move || {
            set_text_verified_until(text, std::time::Instant::now() + std::time::Duration::from_secs(3))
        })
        .join()
        .unwrap();
        assert_eq!(result, VerifiedTextWrite::OutcomeUnknown);
        assert!(still_ours("text/plain;charset=utf-8", text.as_bytes()).unwrap());
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
