//! What a Wayland session asks the compositor for, over D-Bus: the screen (GNOME's own
//! `org.gnome.Mutter.ScreenCast`, which records any rect of the screen without a dialog, else the
//! ScreenCast portal, which asks once and is remembered with a restore token), the shortcuts
//! (the GlobalShortcuts portal: GNOME 48+, KDE Plasma 5.27+), and keystrokes for pasting (the
//! RemoteDesktop portal, which asks once too). On X11 the recording reads the X screen directly
//! (`source_for`).

use std::collections::HashMap;
use std::os::fd::{AsRawFd, OwnedFd};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use futures_lite::StreamExt as _;

use super::media::Source;
use crate::capture::engine::runtime::apparent_log;
use crate::shared::model::{CaptureMode, TargetInfo};

/// A PipeWire stream a compositor is sending, open until dropped.
pub struct Stream {
    pub node: u32,
    /// The portal's PipeWire connection; Mutter's streams are on the user's own PipeWire.
    fd: Option<OwnedFd>,
    /// Portal compositor coordinates, not XWayland pixels; Hyprland reports (0, 0) for
    /// every output, so do not use this to identify the selected output there.
    pub rect: Option<[f64; 4]>,
    /// The output name supplied by the portal (Hyprland reports this even when position is 0).
    pub mapping_id: Option<String>,
    stop: Option<flume::Sender<()>>,
}

impl Stream {
    /// The PipeWire fd for `pipewiresrc fd=`, or -1 for the default PipeWire.
    pub fn fd(&self) -> i32 {
        self.fd.as_ref().map_or(-1, AsRawFd::as_raw_fd)
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

/// Where to record `target` from.
pub fn source_for(target: &TargetInfo, stopping: &AtomicBool) -> Result<Source> {
    let f = target.frame;
    let rect = [f.x, f.y, f.width, f.height];
    anyhow::ensure!(target.mode != CaptureMode::Window || target.window_id.is_some(),
        "Window capture has no selected window ID");
    if !super::super::wayland_session() {
        return Ok(match (target.mode, target.window_id) {
            (CaptureMode::Window, Some(xid)) => Source::X11Window { xid },
            _ => Source::X11 { rect },
        });
    }
    // An X11 app's window on XWayland can be read like on X11.
    if target.mode == CaptureMode::Window
        && let Some(xid) = target.window_id
    {
        return Ok(Source::X11Window { xid });
    }
    if super::super::gnome() && target.mode != CaptureMode::Window {
        match mutter_area(rect) {
            Ok(session) => {
                return Ok(Source::PipeWire {
                    session,
                    crop: None,
                });
            }
            Err(error) => apparent_log(format!("GNOME screencast unavailable: {error:#}")),
        }
    }
    let window = target.mode == CaptureMode::Window;
    let monitors = if window { Vec::new() } else { super::super::monitors() };
    let output = if window {
        None
    } else {
        let output = selected_monitor(&monitors, target.display_id)
            .context("The selected display is no longer connected")?;
        // A connected output can still move after selection; do not use stale coordinates.
        if !target_fits_output(target.mode, rect, output.rect) {
            bail!("The selected display moved or changed; choose the target again");
        }
        Some(output)
    };
    // A token for another output must never silently override the target chosen in the picker.
    let key = screencast_key(output.map(|m| m.name.as_str()), monitors.len());
    let (session, token) = portal_screencast(window, &key, stopping)?;
    if let Some(output) = output
        && !output_still_selected(output, &super::super::monitors(), monitors.len())
    {
        bail!("The display layout changed while screen sharing was chosen; choose the target again");
    }
    if let Some(output) = output
        && super::super::hyprland()
        && monitors.len() > 1
    {
        if !selected_output_matches(&output.name, session.mapping_id.as_deref()) {
            // Discard a stale restore token too, otherwise every retry restores the wrong screen.
            save_token(&key, None);
            bail!("The sharing portal selected {:?}, not {}. Choose that screen in the sharing dialog and try again.", session.mapping_id, output.name);
        }
    }
    // Portal positions are in compositor coordinates, not XWayland pixels. Crop relative to
    // the chosen RandR output; its pixel size is the captured stream's pixel size.
    let crop_rect = if super::super::hyprland() {
        output.map(|m| m.rect)
    } else {
        session.rect
    };
    let crop = portal_crop(target.mode, rect, crop_rect);
    if target.mode == CaptureMode::Region && crop.is_none() {
        bail!("The selected screen does not contain the capture region (or the portal did not report its position). Choose the screen containing the region and try again.");
    }
    if !window {
        save_token(&key, token.as_deref());
    }
    Ok(Source::PipeWire { session, crop })
}

fn screencast_key(output: Option<&str>, count: usize) -> String {
    match output {
        None => "screencast-window".into(),
        Some(name) if count > 1 => format!("screencast:{name}"),
        Some(_) => "screencast".into(),
    }
}

fn selected_output_matches(name: &str, mapping_id: Option<&str>) -> bool {
    mapping_id == Some(name)
}

fn selected_monitor(monitors: &[super::super::Monitor], id: u32) -> Option<&super::super::Monitor> {
    monitors.iter().find(|m| super::super::monitor_id(m) == id)
}

fn output_still_selected(selected: &super::super::Monitor, now: &[super::super::Monitor], count: usize) -> bool {
    now.len() == count
        && selected_monitor(now, super::super::monitor_id(selected)).is_some_and(|current| {
            current.name == selected.name
                && current.rect.iter().zip(selected.rect).all(|(a, b)| (a - b).abs() <= 1.0)
        })
}

fn target_fits_output(mode: CaptureMode, target: [f64; 4], output: [f64; 4]) -> bool {
    const EPSILON: f64 = 1.0;
    match mode {
        CaptureMode::Display => target.iter().zip(output).all(|(a, b)| (a - b).abs() <= EPSILON),
        CaptureMode::Region => {
            target[0] >= output[0] - EPSILON
                && target[1] >= output[1] - EPSILON
                && target[0] + target[2] <= output[0] + output[2] + EPSILON
                && target[1] + target[3] <= output[1] + output[3] + EPSILON
        }
        CaptureMode::Window => true,
        CaptureMode::Device => false,
    }
}

fn portal_crop(mode: CaptureMode, rect: [f64; 4], stream: Option<[f64; 4]>) -> Option<[u32; 4]> {
    match (mode, stream) {
        (CaptureMode::Region, Some([sx, sy, sw, sh])) if sw > 0.0 && sh > 0.0 => {
            let left = rect[0].max(sx);
            let top = rect[1].max(sy);
            let right = (rect[0] + rect[2]).min(sx + sw);
            let bottom = (rect[1] + rect[3]).min(sy + sh);
            (right > left && bottom > top).then_some([
                (left - sx) as u32,
                (top - sy) as u32,
                (right - left) as u32,
                (bottom - top) as u32,
            ])
        }
        _ => None,
    }
}

#[cfg(test)]
mod display_tests {
    use super::*;

    #[test]
    fn restore_tokens_are_scoped_to_the_selected_output() {
        assert_eq!(screencast_key(Some("Virtual-1"), 2), "screencast:Virtual-1");
        assert_eq!(screencast_key(Some("HEADLESS-1"), 2), "screencast:HEADLESS-1");
        assert_eq!(screencast_key(Some("Virtual-1"), 1), "screencast");
        assert_eq!(screencast_key(None, 2), "screencast-window");
    }

    #[test]
    fn portal_output_is_checked_by_name_even_when_positions_overlap() {
        assert!(selected_output_matches("HEADLESS-1", Some("HEADLESS-1")));
        assert!(!selected_output_matches("HEADLESS-1", Some("Virtual-1")));
        assert!(!selected_output_matches("HEADLESS-1", None));
    }

    #[test]
    fn a_reordered_display_id_cannot_capture_a_different_output() {
        let chosen = [1280.0, 0.0, 1920.0, 1080.0];
        assert!(target_fits_output(CaptureMode::Display, chosen, chosen));
        assert!(!target_fits_output(CaptureMode::Display, chosen, [0.0, 0.0, 1280.0, 800.0]));
        assert!(target_fits_output(CaptureMode::Region, [1480.0, 120.0, 300.0, 240.0], chosen));
        assert!(!target_fits_output(CaptureMode::Region, [1480.0, 120.0, 300.0, 240.0], [0.0, 0.0, 1280.0, 800.0]));
    }

    #[test]
    fn region_selection_keeps_its_output_after_monitors_reorder_or_overlap() {
        let monitor = |handle, name: &str| super::super::super::Monitor {
            handle,
            name: name.into(),
            rect: [0.0, 0.0, 1920.0, 1080.0],
            work: [0.0, 0.0, 1920.0, 1080.0],
            scale: 1.0,
            primary: false,
        };
        let selected = monitor(100, "HEADLESS-1");
        let other = monitor(101, "Virtual-1");
        let id = super::super::super::monitor_id(&selected);
        let reordered = [other, selected];
        assert_eq!(selected_monitor(&reordered, id).unwrap().name, "HEADLESS-1");
        assert!(selected_monitor(&reordered, 102).is_none());
        // Both rectangles contain this Region; geometry alone cannot identify its output.
        assert!(target_fits_output(CaptureMode::Region, [20.0, 20.0, 100.0, 100.0], reordered[0].rect));
    }

    #[test]
    fn portal_rejects_a_layout_change_while_the_sharing_dialog_is_open() {
        let selected = super::super::super::Monitor {
            handle: 100,
            name: "Virtual-1".into(),
            rect: [0.0, 0.0, 1280.0, 800.0],
            work: [0.0, 0.0, 1280.0, 800.0],
            scale: 1.0,
            primary: true,
        };
        let mut moved = selected.clone();
        moved.rect[0] = 1280.0;
        let mut replaced = selected.clone();
        replaced.handle = 101;
        assert!(output_still_selected(&selected, &[selected.clone()], 1));
        assert!(!output_still_selected(&selected, &[selected.clone(), moved.clone()], 1));
        assert!(!output_still_selected(&selected, &[selected.clone()], 2));
        assert!(!output_still_selected(&selected, &[moved], 1));
        assert!(!output_still_selected(&selected, &[replaced], 1));
    }

    #[test]
    fn a_region_on_another_output_cannot_crop_outside_the_portal_stream() {
        assert_eq!(
            portal_crop(CaptureMode::Region, [2040.0, 80.0, 420.0, 320.0], Some([0.0, 0.0, 1920.0, 1080.0])),
            None,
            "an off-stream crop yields an empty frame and a PipeWire not-negotiated error"
        );
    }

    #[test]
    fn a_region_is_trimmed_to_the_selected_stream_in_both_axes() {
        assert_eq!(
            portal_crop(CaptureMode::Region, [2080.0, 600.0, 400.0, 300.0], Some([1920.0, 0.0, 320.0, 720.0])),
            Some([160, 600, 160, 120])
        );
    }

    #[test]
    fn second_display_region_uses_the_selected_streams_origin() {
        assert_eq!(
            portal_crop(CaptureMode::Region, [2070.0, 110.0, 400.0, 250.0], Some([1920.0, 0.0, 1280.0, 720.0])),
            Some([150, 110, 400, 250])
        );
        assert_eq!(
            portal_crop(CaptureMode::Region, [2070.0, 110.0, 400.0, 250.0], None),
            None,
            "unknown stream position must not silently record a whole display"
        );
    }
}

// MARK: GNOME's screencast

/// `org.gnome.Mutter.ScreenCast.Session.RecordArea`: any rect of the screen (logical pixels,
/// which are XWayland's at scale 1), with the pointer drawn in, no dialog.
fn mutter_area(rect: [f64; 4]) -> Result<Stream> {
    let (ready_tx, ready_rx) = flume::bounded::<Result<u32>>(1);
    let (stop_tx, stop_rx) = flume::bounded::<()>(1);
    std::thread::Builder::new()
        .name("apparent-mutter-screencast".into())
        .spawn(move || {
            let run = || -> Result<(zbus::blocking::Connection, zbus::zvariant::OwnedObjectPath, u32)> {
                let bus = zbus::blocking::Connection::session()?;
                let cast = zbus::blocking::Proxy::new(
                    &bus,
                    "org.gnome.Mutter.ScreenCast",
                    "/org/gnome/Mutter/ScreenCast",
                    "org.gnome.Mutter.ScreenCast",
                )?;
                let props: HashMap<&str, zbus::zvariant::Value> = HashMap::new();
                let session_path: zbus::zvariant::OwnedObjectPath =
                    cast.call("CreateSession", &(props,))?;
                let session = zbus::blocking::Proxy::new(
                    &bus,
                    "org.gnome.Mutter.ScreenCast",
                    session_path.clone(),
                    "org.gnome.Mutter.ScreenCast.Session",
                )?;
                let mut props: HashMap<&str, zbus::zvariant::Value> = HashMap::new();
                // 1: the pointer drawn into the frames.
                props.insert("cursor-mode", zbus::zvariant::Value::U32(1));
                props.insert("is-recording", zbus::zvariant::Value::Bool(true));
                let [x, y, w, h] = rect.map(|v| v.round() as i32);
                let stream_path: zbus::zvariant::OwnedObjectPath =
                    session.call("RecordArea", &(x, y, w, h, props))?;
                let stream = zbus::blocking::Proxy::new(
                    &bus,
                    "org.gnome.Mutter.ScreenCast",
                    stream_path,
                    "org.gnome.Mutter.ScreenCast.Stream",
                )?;
                let mut added = stream.receive_signal("PipeWireStreamAdded")?;
                session.call_method("Start", &())?;
                let node: u32 = added
                    .next()
                    .context("No stream")?
                    .body()
                    .deserialize()?;
                Ok((bus, session_path, node))
            };
            match run() {
                Ok((bus, session_path, node)) => {
                    let _ = ready_tx.send(Ok(node));
                    let _ = stop_rx.recv();
                    let _ = bus.call_method(
                        Some("org.gnome.Mutter.ScreenCast"),
                        session_path.as_str(),
                        Some("org.gnome.Mutter.ScreenCast.Session"),
                        "Stop",
                        &(),
                    );
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                }
            }
        })?;
    let node = ready_rx
        .recv_timeout(Duration::from_secs(5))
        .map_err(|_| anyhow!("GNOME's screencast didn't answer"))??;
    Ok(Stream {
        node,
        fd: None,
        rect: Some(rect),
        mapping_id: None,
        stop: Some(stop_tx),
    })
}

// MARK: The ScreenCast portal

fn state_path() -> PathBuf {
    super::super::local_dir().join("portal.json")
}

fn saved_token(key: &str) -> Option<String> {
    let bytes = std::fs::read(state_path()).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value[key].as_str().map(str::to_owned)
}

fn save_token(key: &str, token: Option<&str>) {
    let path = state_path();
    let mut value: serde_json::Value = std::fs::read(&path)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(|| serde_json::json!({}));
    value[key] = token.map_or(serde_json::Value::Null, |t| t.into());
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&path, value.to_string());
}

/// A display (or a window, `window`) through the ScreenCast portal. The first time, the
/// desktop asks which; the answer is remembered (a restore token), so later recordings start
/// without asking.
fn portal_screencast(window: bool, key: &str, stopping: &AtomicBool) -> Result<(Stream, Option<String>)> {
    use ashpd::desktop::PersistMode;
    use ashpd::desktop::screencast::{CursorMode, Screencast, SourceType};

    let (ready_tx, ready_rx) = flume::bounded::<Result<(u32, OwnedFd, Option<[f64; 4]>, Option<String>, Option<String>)>>(1);
    let (stop_tx, stop_rx) = flume::bounded::<()>(1);
    let request_key = key.to_owned();
    std::thread::Builder::new()
        .name("apparent-portal-screencast".into())
        .spawn(move || {
            futures_lite::future::block_on(async move {
                let run = async {
                    let proxy = Screencast::new().await?;
                    let session = proxy.create_session().await?;
                    let request = async {
                        let token = saved_token(&request_key);
                        let kind = if window {
                            SourceType::Window
                        } else {
                            SourceType::Monitor
                        };
                        proxy
                            .select_sources(
                                &session,
                                CursorMode::Embedded,
                                kind.into(),
                                false,
                                token.as_deref(),
                                // Windows come and go: ask each time for one.
                                if window {
                                    PersistMode::DoNot
                                } else {
                                    PersistMode::ExplicitlyRevoked
                                },
                            )
                            .await?;
                        let streams = proxy.start(&session, None).await?.response()?;
                        let token = streams.restore_token().map(str::to_owned);
                        let stream = streams.streams().first().context("Nothing was shared")?;
                        let rect = match (stream.position(), stream.size()) {
                            (Some((x, y)), Some((w, h))) => {
                                Some([f64::from(x), f64::from(y), f64::from(w), f64::from(h)])
                            }
                            _ => None,
                        };
                        let node = stream.pipe_wire_node_id();
                        let mapping_id = stream.mapping_id().map(str::to_owned);
                        let fd = proxy.open_pipe_wire_remote(&session).await?;
                        anyhow::Ok((node, fd, rect, mapping_id, token))
                    };
                    let cancelled = async {
                        let _ = stop_rx.recv_async().await;
                        bail!("Screen sharing was cancelled")
                    };
                    match futures_lite::future::race(request, cancelled).await {
                        Ok((node, fd, rect, mapping_id, token)) => anyhow::Ok((proxy, session, node, fd, rect, mapping_id, token)),
                        Err(error) => {
                            let _ = session.close().await;
                            Err(error)
                        }
                    }
                };
                match run.await {
                    Ok((_proxy, session, node, fd, rect, mapping_id, token)) => {
                        let _ = ready_tx.send(Ok((node, fd, rect, mapping_id, token)));
                        let _ = stop_rx.recv_async().await;
                        let _ = session.close().await;
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                    }
                }
            });
        })?;
    // The first time the desktop asks which display, so wait for the person to choose.
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    let (node, fd, rect, mapping_id, token) = loop {
        if stopping.load(Ordering::Acquire) || std::time::Instant::now() >= deadline {
            let _ = stop_tx.try_send(());
            bail!("Screen sharing was cancelled or the dialog wasn't answered");
        }
        match ready_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(Ok(stream)) => break stream,
            Ok(Err(error)) => {
                save_token(key, None);
                return Err(error);
            }
            Err(flume::RecvTimeoutError::Timeout) => {}
            Err(error) => return Err(error.into()),
        }
    };
    Ok((Stream {
        node,
        fd: Some(fd),
        rect,
        mapping_id,
        stop: Some(stop_tx),
    }, token))
}

// MARK: Shortcuts

/// Binds Apparent's shortcuts with the GlobalShortcuts portal and calls `pressed` with
/// `record` or `paste` when one goes down. `Err` when the desktop has no such portal (GNOME
/// before 48), so the caller can fall back to a desktop keybinding (`bind_gnome_keybindings`).
pub fn global_shortcuts(
    record_trigger: String,
    paste_trigger: Option<String>,
    pressed: impl Fn(&str) + Send + 'static,
) -> Result<()> {
    use ashpd::desktop::global_shortcuts::{GlobalShortcuts, NewShortcut};
    let (ready_tx, ready_rx) = flume::bounded::<Result<()>>(1);
    std::thread::Builder::new()
        .name("apparent-global-shortcuts".into())
        .spawn(move || {
            futures_lite::future::block_on(async move {
                let setup = async {
                    let proxy = GlobalShortcuts::new().await?;
                    let session = proxy.create_session().await?;
                    let mut shortcuts = vec![
                        NewShortcut::new("record", "Record the screen, or finish recording")
                            .preferred_trigger(record_trigger.as_str()),
                    ];
                    if let Some(paste_trigger) = &paste_trigger {
                        shortcuts.push(
                            NewShortcut::new("paste", "Paste the latest capture")
                                .preferred_trigger(paste_trigger.as_str()),
                        );
                    }
                    let bound = proxy
                        .bind_shortcuts(&session, &shortcuts, None)
                        .await?
                        .response()?;
                    // Plasma 5.27 returns success with an empty list. That is not a binding:
                    // let the caller try native bindings rather than silently losing shortcuts.
                    if !shortcuts_bound(
                        paste_trigger.is_some(),
                        bound
                            .shortcuts()
                            .iter()
                            .map(|s| (s.id(), s.trigger_description())),
                    ) {
                        let _ = session.close().await;
                        bail!("The desktop did not bind both shortcuts");
                    }
                    let activated = proxy.receive_activated().await?;
                    anyhow::Ok((proxy, session, activated))
                };
                match setup.await {
                    Ok((_proxy, _session, mut activated)) => {
                        let _ = ready_tx.send(Ok(()));
                        while let Some(event) = activated.next().await {
                            pressed(event.shortcut_id());
                        }
                    }
                    Err(error) => {
                        let _ = ready_tx.send(Err(error));
                    }
                }
            });
        })?;
    ready_rx
        .recv_timeout(Duration::from_secs(60))
        .map_err(|_| anyhow!("The shortcuts portal didn't answer"))?
}

/// Whether every shortcut that was requested (paste only while it is set) came back bound.
fn shortcuts_bound<'a>(
    paste: bool,
    shortcuts: impl Iterator<Item = (&'a str, &'a str)> + Clone,
) -> bool {
    let ids: &[&str] = if paste { &["record", "paste"] } else { &["record"] };
    ids.iter().all(|id| {
        shortcuts
            .clone()
            .any(|(bound, trigger)| bound == *id && !trigger.is_empty())
    })
}

/// The portal's trigger syntax for a shortcut (`ALT+SHIFT+s`), from its Carbon bits and key.
pub fn portal_trigger(carbon_modifiers: u32, key: &str) -> String {
    use crate::capture::engine::runtime::{CARBON_CMD_KEY, CARBON_CONTROL_KEY, CARBON_OPTION_KEY, CARBON_SHIFT_KEY};
    let mut parts = Vec::new();
    for (bit, name) in [
        (CARBON_CONTROL_KEY, "CTRL"),
        (CARBON_OPTION_KEY, "ALT"),
        (CARBON_SHIFT_KEY, "SHIFT"),
        (CARBON_CMD_KEY, "LOGO"),
    ] {
        if carbon_modifiers & bit != 0 {
            parts.push(name.to_owned());
        }
    }
    parts.push(key.to_ascii_lowercase());
    parts.join("+")
}

/// GNOME's keybinding syntax (`<Alt><Shift>s`).
pub fn gnome_binding(carbon_modifiers: u32, key: &str) -> String {
    use crate::capture::engine::runtime::{CARBON_CMD_KEY, CARBON_CONTROL_KEY, CARBON_OPTION_KEY, CARBON_SHIFT_KEY};
    let mut out = String::new();
    for (bit, name) in [
        (CARBON_CONTROL_KEY, "<Control>"),
        (CARBON_OPTION_KEY, "<Alt>"),
        (CARBON_SHIFT_KEY, "<Shift>"),
        (CARBON_CMD_KEY, "<Super>"),
    ] {
        if carbon_modifiers & bit != 0 {
            out.push_str(name);
        }
    }
    out.push_str(&key.to_ascii_lowercase());
    out
}

const GNOME_KEYS: &str = "org.gnome.settings-daemon.plugins.media-keys";
const GNOME_PATH: &str = "/org/gnome/settings-daemon/plugins/media-keys/custom-keybindings";

/// GNOME without the GlobalShortcuts portal (before 48): custom keyboard shortcuts in GNOME's
/// own settings (Settings › Keyboard › Custom Shortcuts lists them) that run
/// `apparent shortcut record` and `apparent shortcut paste`, which hand the press to the running app
/// (`super::ipc`). Only Apparent's own two entries are written or changed.
pub fn bind_gnome_keybindings(bindings: &[(&str, String)]) -> Result<()> {
    let exe = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .map_or_else(std::env::current_exe, Ok)?;
    let gsettings = |args: &[&str]| -> Result<String> {
        let out = std::process::Command::new("gsettings")
            .args(args)
            .output()?;
        if !out.status.success() {
            bail!(
                "gsettings {}: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    };
    let list = gsettings(&["get", GNOME_KEYS, "custom-keybindings"])?;
    let mut paths: Vec<String> = list
        .trim_start_matches("@as")
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .split(',')
        .map(|p| p.trim().trim_matches('\'').to_owned())
        .filter(|p| !p.is_empty())
        .collect();
    for (id, binding) in bindings {
        let path = format!("{GNOME_PATH}/apparent-{id}/");
        if !paths.contains(&path) {
            paths.push(path.clone());
        }
        let schema = format!("{GNOME_KEYS}.custom-keybinding:{path}");
        let command = format!("'{}' shortcut {id}", exe.display());
        gsettings(&[
            "set",
            &schema,
            "name",
            &gvariant_string(&format!("Apparent: {id}")),
        ])?;
        gsettings(&["set", &schema, "command", &gvariant_string(&command)])?;
        gsettings(&["set", &schema, "binding", &gvariant_string(binding)])?;
    }
    let value = format!(
        "[{}]",
        paths
            .iter()
            .map(|p| format!("'{p}'"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    gsettings(&["set", GNOME_KEYS, "custom-keybindings", &value])?;
    Ok(())
}

/// A string as GVariant text, which `gsettings set` reads: `"…"` with `\\` and `"` escaped.
fn gvariant_string(text: &str) -> String {
    format!("\"{}\"", text.replace('\\', "\\\\").replace('"', "\\\""))
}

// MARK: Keystrokes

/// Ctrl+V (or Ctrl+Shift+V) into the focused app through the RemoteDesktop portal: Wayland lets
/// no app send another keystrokes. The desktop asks once; the answer is remembered.
pub fn send_paste(shift: bool) -> Result<()> {
    use ashpd::desktop::PersistMode;
    use ashpd::desktop::remote_desktop::{DeviceType, KeyState, RemoteDesktop};
    const CTRL: i32 = 0xffe3; // XK_Control_L
    const SHIFT: i32 = 0xffe1; // XK_Shift_L
    const V: i32 = 0x76; // XK_v
    futures_lite::future::block_on(async move {
        let proxy = RemoteDesktop::new().await?;
        let session = proxy.create_session().await?;
        let token = saved_token("remote-desktop");
        proxy
            .select_devices(
                &session,
                DeviceType::Keyboard.into(),
                token.as_deref(),
                PersistMode::ExplicitlyRevoked,
            )
            .await?;
        let started = proxy.start(&session, None).await?.response();
        let started = match started {
            Ok(started) => started,
            Err(error) => {
                save_token("remote-desktop", None);
                return Err(error.into());
            }
        };
        save_token("remote-desktop", started.restore_token());
        let mut keys = vec![CTRL];
        if shift {
            keys.push(SHIFT);
        }
        keys.push(V);
        for &key in &keys {
            proxy
                .notify_keyboard_keysym(&session, key, KeyState::Pressed)
                .await?;
        }
        for &key in keys.iter().rev() {
            proxy
                .notify_keyboard_keysym(&session, key, KeyState::Released)
                .await?;
        }
        let _ = session.close().await;
        anyhow::Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capture::engine::runtime::{CARBON_OPTION_KEY, CARBON_SHIFT_KEY};

    #[test]
    fn an_empty_success_from_plasma_is_not_a_working_shortcut_backend() {
        let record = ("record", "Alt+Shift+S");
        let paste = ("paste", "Alt+Shift+V");
        assert!(!shortcuts_bound(true, [].into_iter()));
        assert!(!shortcuts_bound(true, [record].into_iter()));
        assert!(!shortcuts_bound(true, [record, ("paste", "")].into_iter()));
        assert!(shortcuts_bound(true, [record, paste].into_iter()));
        // A cleared paste shortcut is not requested, so only record has to come back.
        assert!(shortcuts_bound(false, [record].into_iter()));
        assert!(!shortcuts_bound(false, [].into_iter()));
    }

    #[test]
    fn shortcuts_are_written_the_way_each_desktop_reads_them() {
        let mods = CARBON_OPTION_KEY | CARBON_SHIFT_KEY;
        assert_eq!(portal_trigger(mods, "S"), "ALT+SHIFT+s");
        assert_eq!(gnome_binding(mods, "S"), "<Alt><Shift>s");
        assert_eq!(
            gvariant_string("'/a b/apparent' shortcut record"),
            "\"'/a b/apparent' shortcut record\""
        );
        assert_eq!(gvariant_string(r#"say "hi"\"#), r#""say \"hi\"\\""#);
    }
}
