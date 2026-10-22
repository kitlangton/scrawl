//! Linux capture: the shortcuts, input, the frontmost app, displays and windows, and the
//! GStreamer recorder (`crate::linux::media`).
//!
//! On X11 the X server answers everything: shortcuts are passive key grabs on the root window,
//! and the pointer, buttons and keys are polled. On a Wayland session the compositor keeps
//! other apps' input to itself, so the shortcuts come from the GlobalShortcuts portal (or, on
//! GNOME without it, custom keybindings that run `scrawl shortcut record`), and ⇧, clicks and
//! Esc from the input devices when the user may read them (`crate::linux::input`). See
//! `docs/linux.md`.
//!
//! Shortcuts are stored as Windows virtual keys (`VK_S` is `0x53`), as on Windows: a key's name
//! and code then mean the same on both, and the X keysym or evdev code is looked up when it's
//! bound. Global coordinates are X11 root-window pixels; `TargetFrame` holds pixels too.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use x11rb::connection::Connection as _;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{ConnectionExt as _, GrabMode, ModMask};
use x11rb::protocol::xtest::ConnectionExt as _;
use x11rb::rust_connection::RustConnection;

use super::{
    CARBON_CMD_KEY, CARBON_CONTROL_KEY, CARBON_OPTION_KEY, CARBON_SHIFT_KEY, LiveOverlayEvent,
    Sampler, SharedCaptureTelemetry, ShortcutConfig, ShortcutKind, clean_label, hotkey_combos,
    layout_default_keycode, pick_display, poll_clicks, scrawl_log, shortcut_label,
    shortcut_matches, track_shift_ink,
};
use crate::linux::{self, input, x11};
use crate::model::{CaptureMode, InteractionEvent, TargetFrame, TargetInfo};

/// Frames are pixels already: a recording keeps at most the screen's own resolution.
pub const MAX_CAPTURE_SCALE: f64 = 1.0;

/// Where a session's clicks and keys come from, for the log. Clicks are always polled.
pub(super) const MOUSE_SOURCE: &str = "polled mouse buttons";
pub(super) const KEY_SOURCE: &str = "polled key state (the X server or the input devices)";
pub(super) const NO_KEY_SOURCE: &str = "nothing outside Scrawl's windows (no input devices)";

/// `VK_S`, `VK_V` and `VK_Z`: shortcuts are stored as Windows virtual keys.
pub const KEY_S: u16 = 0x53;
pub const KEY_V: u16 = 0x56;
const KEY_Z: u16 = 0x5A;
const VK_ESCAPE: u16 = 0x1B;

/// `~/.local/share/scrawl/Logs`.
pub fn log_dir() -> Option<PathBuf> {
    Some(linux::local_dir().join("Logs"))
}

/// The primary display's index in `linux::monitors()`.
pub fn main_display_id() -> u32 {
    0
}

/// The virtual key for `ch`. Keys are bound by keysym (`keysym_for_vk`), so the key that types
/// S is found on Dvorak too.
pub fn key_code_for_char(ch: char) -> Option<u16> {
    let upper = ch.to_ascii_uppercase();
    match upper {
        'A'..='Z' | '0'..='9' => Some(upper as u16),
        ' ' => Some(0x20),
        _ => None,
    }
}

pub fn layout_s_keycode() -> u16 {
    KEY_S
}

pub fn layout_v_keycode() -> u16 {
    KEY_V
}

/// Nobody grants screen recording up front on Linux: X11 needs nothing, and on Wayland the
/// ScreenCast portal asks when a recording starts (once, remembered with a restore token).
pub fn screen_capture_granted() -> bool {
    true
}

pub fn screen_capture_usable(_probe: bool) -> bool {
    true
}

/// The primary display's size (`scrawl sck-probe`).
pub fn screen_capture_probe() -> Result<(u32, u32), String> {
    let monitor = linux::primary_monitor().ok_or("no display")?;
    Ok((monitor.rect[2] as u32, monitor.rect[3] as u32))
}

pub fn request_screen_capture() {}

/// Pasting needs no grant up front: XTEST on X11, and on Wayland the RemoteDesktop portal asks
/// the first time.
pub fn accessibility_granted() -> bool {
    true
}

// MARK: Keys

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct Modifiers {
    super_key: bool,
    ctrl: bool,
    alt: bool,
    shift: bool,
}

/// What is held: the modifiers, the mouse buttons and the keys worth knowing (as virtual keys).
#[derive(Clone, Debug, Default)]
struct Held {
    mods: Modifiers,
    left: bool,
    right: bool,
    keys: HashSet<u16>,
}

impl Modifiers {
    fn any(self) -> bool {
        self.super_key || self.ctrl || self.alt || self.shift
    }

    fn bits(self) -> u32 {
        let mut bits = 0;
        for (held, bit) in [
            (self.super_key, CARBON_CMD_KEY),
            (self.ctrl, CARBON_CONTROL_KEY),
            (self.alt, CARBON_OPTION_KEY),
            (self.shift, CARBON_SHIFT_KEY),
        ] {
            if held {
                bits |= bit;
            }
        }
        bits
    }
}

/// The keysym a virtual key is bound by (unshifted: `s`, `1`, `F5`).
fn keysym_for_vk(vk: u32) -> Option<u32> {
    match vk {
        0x41..=0x5A => Some(vk + 0x20),
        0x30..=0x39 | 0x20 => Some(vk),
        0x70..=0x87 => Some(0xffbe + vk - 0x70),
        0x1B => Some(0xff1b),
        _ => None,
    }
}

/// The virtual key a keysym stands for, for the keys Scrawl notices.
fn vk_for_keysym(sym: u32) -> Option<u16> {
    Some(match sym {
        0x61..=0x7a => (sym - 0x20) as u16,
        0x41..=0x5a | 0x30..=0x39 | 0x20 => sym as u16,
        0xff0d => 0x0D,
        0xff09 => 0x09,
        0xff08 => 0x08,
        0xffff => 0x2E,
        0xff1b => VK_ESCAPE,
        0xffbe..=0xffd5 => (0x70 + sym - 0xffbe) as u16,
        _ => return None,
    })
}

/// Whether the input devices are the source: a Wayland session where they can be read.
fn from_devices() -> bool {
    linux::wayland_session() && input::held().is_some()
}

fn held_now() -> Held {
    held_from(input::held())
}

/// What the poller sees: like `held_now`, with presses too short to last until the next poll.
fn held_polled() -> Held {
    held_from(input::take_held())
}

fn held_from(devices: Option<input::Held>) -> Held {
    if linux::wayland_session()
        && let Some(devices) = devices
    {
        return Held {
            mods: Modifiers {
                super_key: devices.super_key(),
                ctrl: devices.ctrl(),
                alt: devices.alt(),
                shift: devices.shift(),
            },
            left: devices.left(),
            right: devices.right(),
            keys: devices
                .keys
                .iter()
                .filter_map(|&code| input::vk_for_evdev_code(code))
                .collect(),
        };
    }
    let pointer = x11::pointer_state().unwrap_or_default();
    let keys = x11::keys_down()
        .map(|bits| {
            (8..=255_u8)
                .filter(|&code| x11::key_is_down(&bits, code))
                .filter_map(|code| x11::keysym_at(code).and_then(vk_for_keysym))
                .collect()
        })
        .unwrap_or_default();
    Held {
        mods: Modifiers {
            super_key: pointer.super_key,
            ctrl: pointer.ctrl,
            alt: pointer.alt,
            shift: pointer.shift,
        },
        left: pointer.left,
        right: pointer.right,
        keys,
    }
}

/// The Super, Ctrl, Alt and Shift keys held right now, as a bit mask.
pub fn modifier_flags() -> u64 {
    u64::from(held_now().mods.bits())
}

/// The virtual key and label for a key GPUI names (`a`, `f5`, `space`).
fn key_from_name(key: &str) -> Option<(u16, String)> {
    let lower = key.to_ascii_lowercase();
    if lower.chars().count() == 1
        && let Some(ch) = lower.chars().next()
        && ch.is_ascii_alphanumeric()
    {
        let upper = ch.to_ascii_uppercase();
        return Some((upper as u16, upper.to_string()));
    }
    if lower == "space" || lower == " " {
        return Some((0x20, "Space".into()));
    }
    if let Some(n) = lower.strip_prefix('f').and_then(|n| n.parse::<u16>().ok())
        && (1..=24).contains(&n)
    {
        return Some((0x6F + n, format!("F{n}")));
    }
    None
}

pub fn shortcut_from_key_and_mods(
    key: &str,
    cmd: bool,
    ctrl: bool,
    alt: bool,
    shift: bool,
) -> Option<ShortcutConfig> {
    let (key_code, key_label) = key_from_name(key)?;
    let is_fn_key = key_label.starts_with('F') && key_label.len() >= 2;
    if !cmd && !ctrl && !alt && !is_fn_key {
        return None;
    }
    let mut carbon_modifiers = 0_u32;
    for (held, bit) in [
        (ctrl, CARBON_CONTROL_KEY),
        (alt, CARBON_OPTION_KEY),
        (shift, CARBON_SHIFT_KEY),
        (cmd, CARBON_CMD_KEY),
    ] {
        if held {
            carbon_modifiers |= bit;
        }
    }
    Some(ShortcutConfig {
        key_code: u32::from(key_code),
        carbon_modifiers,
        label: shortcut_label(carbon_modifiers, &key_label),
    })
}

/// A key's name in a logged chord (`Ctrl+Shift+C`), for the keys worth logging.
fn shortcut_key_name(vk: u16) -> Option<String> {
    match vk {
        0x30..=0x39 | 0x41..=0x5A => Some((vk as u8 as char).to_string()),
        0x0D => Some("Enter".into()),
        0x09 => Some("Tab".into()),
        0x20 => Some("Space".into()),
        0x08 => Some("Backspace".into()),
        0x2E => Some("Delete".into()),
        0x70..=0x87 => Some(format!("F{}", vk - 0x6F)),
        _ => None,
    }
}

// MARK: The frontmost app

static LAST_EXTERNAL_WINDOW: AtomicIsize = AtomicIsize::new(0);

fn is_ours(pid: u32) -> bool {
    pid == std::process::id()
}

pub fn record_external_frontmost_app() {
    if let Some(front) = linux::foreground_window()
        && !is_ours(front.pid)
    {
        LAST_EXTERNAL_WINDOW.store(front.hwnd, Ordering::Release);
    }
}

/// Puts the last app that was in front (before Scrawl's own window took focus) back in front,
/// so a paste lands in it. On Wayland only an X11 app can be asked; the compositor gives focus
/// back to the previous window by itself when Scrawl's closes.
pub fn ensure_external_app_active() {
    if let Some(front) = linux::foreground_window()
        && !is_ours(front.pid)
    {
        LAST_EXTERNAL_WINDOW.store(front.hwnd, Ordering::Release);
        return;
    }
    let target = LAST_EXTERNAL_WINDOW.load(Ordering::Acquire);
    if target == 0 {
        return;
    }
    x11::activate(target);
    let deadline = Instant::now() + Duration::from_millis(220);
    while Instant::now() < deadline {
        if linux::foreground_window().is_some_and(|w| w.hwnd == target) {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    thread::sleep(Duration::from_millis(25));
}

fn external_front_window() -> Option<linux::WindowInfo> {
    if let Some(front) = linux::foreground_window()
        && !is_ours(front.pid)
    {
        LAST_EXTERNAL_WINDOW.store(front.hwnd, Ordering::Release);
        return Some(front);
    }
    match LAST_EXTERNAL_WINDOW.load(Ordering::Acquire) {
        0 => None,
        id => linux::window_info(id),
    }
}

/// The frontmost app's pid, or the last external one while Scrawl itself is in front.
pub fn frontmost_external_pid() -> Option<i32> {
    if linux::wayland_session()
        && let Some(app) = linux::atspi::last_external_app()
    {
        return app.pid.map(|pid| pid as i32);
    }
    external_front_window().map(|w| w.pid as i32)
}

/// The frontmost app (pid, name, identifier), or the last one before Scrawl. On Wayland the
/// X server sees only X11 apps, so the accessibility bus is asked first (`linux::atspi`).
pub fn frontmost_app_info() -> Option<(i32, String, Option<String>)> {
    if linux::wayland_session()
        && let Some(app) = linux::atspi::last_external_app()
    {
        return Some((app.pid.unwrap_or(0) as i32, app.name, Some(app.id)));
    }
    let front = external_front_window()?;
    let (name, id) = linux::app_identity(front.pid)?;
    Some((front.pid as i32, name, Some(id)))
}

pub fn current_mouse_screen() -> Option<[f32; 2]> {
    linux::cursor_position().map(|(x, y)| [x as f32, y as f32])
}

// MARK: Displays and windows

fn frame_of(rect: [f64; 4], scale: f64) -> TargetFrame {
    TargetFrame {
        x: rect[0],
        y: rect[1],
        width: rect[2].max(1.0),
        height: rect[3].max(1.0),
        scale,
    }
}

/// Selectable app windows (not Scrawl's), front to back, with their app names. On Wayland only
/// X11 apps' windows are known.
fn app_windows() -> Vec<(linux::WindowInfo, String, String)> {
    let mut names: std::collections::HashMap<u32, Option<(String, String)>> =
        std::collections::HashMap::new();
    linux::top_level_windows()
        .into_iter()
        .filter(|w| w.on_screen() && w.is_app_window() && !is_ours(w.pid))
        .filter_map(|w| {
            let identity = names
                .entry(w.pid)
                .or_insert_with(|| linux::app_identity(w.pid))
                .clone()?;
            Some((w, identity.0, identity.1))
        })
        .collect()
}

fn window_target(window: &linux::WindowInfo, app: &str, id: &str) -> TargetInfo {
    let monitor = linux::monitor_at(
        window.rect[0] + window.rect[2] / 2.0,
        window.rect[1] + window.rect[3] / 2.0,
    );
    TargetInfo {
        mode: CaptureMode::Window,
        app: Some(clean_label(app)).filter(|s| !s.is_empty()),
        bundle_id: Some(id.to_owned()),
        window_title: Some(clean_label(&window.title)).filter(|t| !t.is_empty()),
        window_id: Some(window.hwnd as u32),
        display_id: display_index(monitor.as_ref()),
        frame: frame_of(window.rect, monitor.map_or(1.0, |m| m.scale)),
    }
}

fn display_index(monitor: Option<&linux::Monitor>) -> u32 {
    let Some(monitor) = monitor else {
        return 0;
    };
    linux::monitors()
        .iter()
        .position(|m| m.handle == monitor.handle)
        .unwrap_or(0) as u32
}

pub fn discover_target(window_query: Option<&str>) -> TargetInfo {
    discover_target_at(window_query, linux::cursor_position().map(|(x, y)| [x, y]))
}

/// Like `discover_target`, recording the display under `point` (see `pick_display`).
pub fn discover_target_at(window_query: Option<&str>, point: Option<[f64; 2]>) -> TargetInfo {
    let frontmost = frontmost_app_info();
    if let Some(query) = window_query {
        let q = query.to_lowercase();
        if let Some((window, app, id)) = app_windows().into_iter().find(|(w, app, _)| {
            w.title.to_lowercase().contains(&q) || app.to_lowercase().contains(&q)
        }) {
            return window_target(&window, &app, &id);
        }
    }
    let monitors = linux::monitors();
    let front = external_front_window();
    let rects: Vec<[f64; 4]> = monitors.iter().map(|m| m.rect).collect();
    let index = pick_display(&rects, point, front.as_ref().map(|w| w.rect));
    let (rect, scale) = monitors
        .get(index)
        .map_or(([0.0, 0.0, 1920.0, 1080.0], 1.0), |m| (m.rect, m.scale));
    let title = if linux::wayland_session() {
        linux::atspi::last_external_app().and_then(|app| app.title)
    } else {
        front.map(|w| w.title)
    };
    TargetInfo {
        mode: CaptureMode::Display,
        app: frontmost.as_ref().map(|(_, name, _)| name.clone()),
        bundle_id: frontmost.and_then(|(_, _, id)| id),
        window_title: title.map(|t| clean_label(&t)).filter(|t| !t.is_empty()),
        window_id: None,
        display_id: index as u32,
        frame: frame_of(rect, scale),
    }
}

/// All on-screen app windows at least 80 px each way, front to back, for the Window picker.
pub fn list_selectable_windows() -> Vec<TargetInfo> {
    app_windows()
        .into_iter()
        .filter(|(w, _, _)| w.rect[2] >= 80.0 && w.rect[3] >= 80.0)
        .map(|(w, app, id)| window_target(&w, &app, &id))
        .collect()
}

/// Top-level windows as the window log and click attribution read them.
pub(super) fn raw_windows_now() -> Vec<crate::windows::RawWindow> {
    crate::windows::raw_windows()
}

/// Linux has no `scrawl-helper`: every Scrawl window is in its own process.
pub(super) fn is_helper_process(_pid: i32) -> bool {
    false
}

// MARK: Synthetic input

pub fn wait_for_modifiers_released(max_wait_ms: u64) {
    let deadline = Instant::now() + Duration::from_millis(max_wait_ms);
    while Instant::now() < deadline {
        if !held_now().mods.any() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

const XK_CONTROL_L: u32 = 0xffe3;
const XK_SHIFT_L: u32 = 0xffe1;
const XK_V: u32 = 0x76;

/// XTEST key presses (2) and releases (3), or button ones (4, 5), in order.
fn xtest(events: &[(u8, u8)]) -> bool {
    let Some(x) = x11::x() else {
        return false;
    };
    for &(kind, detail) in events {
        if x.conn
            .xtest_fake_input(kind, detail, x11rb::CURRENT_TIME, x.root, 0, 0, 0)
            .is_err()
        {
            return false;
        }
    }
    x.conn.flush().is_ok()
}

fn keycode(keysym: u32) -> Option<u8> {
    x11::keycodes_for(keysym).into_iter().next()
}

/// Pastes into the app in front: Ctrl+V, or Ctrl+Shift+V in a terminal (Ctrl+V is a control
/// character there). On Wayland through the RemoteDesktop portal, which asks once; on X11
/// through XTEST. A Ctrl the user still holds is left alone.
pub fn post_synthetic_cmd_v() {
    let shift = super::is_frontmost_terminal();
    if linux::wayland_session() {
        if let Err(error) = linux::portal::send_paste(shift) {
            scrawl_log(format!(
                "paste: the RemoteDesktop portal refused: {error:#}"
            ));
        }
        return;
    }
    let (Some(ctrl), Some(v)) = (keycode(XK_CONTROL_L), keycode(XK_V)) else {
        scrawl_log("paste: no key types Ctrl or V");
        return;
    };
    let held = held_now().mods;
    let mut events = Vec::new();
    if !held.ctrl {
        events.push((2, ctrl));
    }
    let shift_key = shift.then(|| keycode(XK_SHIFT_L)).flatten();
    if let Some(key) = shift_key.filter(|_| !held.shift) {
        events.push((2, key));
    }
    events.extend([(2, v), (3, v)]);
    if let Some(key) = shift_key.filter(|_| !held.shift) {
        events.push((3, key));
    }
    if !held.ctrl {
        events.push((3, ctrl));
    }
    if !xtest(&events) {
        scrawl_log("paste: XTEST refused the keystroke");
    }
}

/// A left click at a global point (X11 only: Wayland lets no app move the pointer).
pub fn post_synthetic_click(screen_x: f64, screen_y: f64) {
    let Some(x) = x11::x() else {
        return;
    };
    let _ = x.conn.xtest_fake_input(
        6,
        0,
        x11rb::CURRENT_TIME,
        x.root,
        screen_x.round() as i16,
        screen_y.round() as i16,
        0,
    );
    xtest(&[(4, 1)]);
    thread::sleep(Duration::from_millis(14));
    xtest(&[(5, 1)]);
}

// MARK: Shortcuts

/// Where the shortcuts come from in this session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Backend {
    /// Not set up yet, or nothing works (the keys are still polled while a capture runs).
    None,
    /// Passive key grabs on the X root window.
    X11,
    /// The GlobalShortcuts portal: the desktop owns the binding (its settings can change it).
    Portal,
    /// GNOME custom keybindings running `scrawl shortcut …` (`linux::ipc`).
    Gnome,
    /// Plasma 5's native KGlobalAccel service.
    Kde,
    /// Polled from the input devices: the press also reaches the app in front.
    Devices,
}

static BACKEND: Mutex<Backend> = Mutex::new(Backend::None);
static HOTKEY_TELEMETRY: OnceLock<Arc<SharedCaptureTelemetry>> = OnceLock::new();

fn backend() -> Backend {
    *BACKEND
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn set_backend(backend: Backend) {
    *BACKEND
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = backend;
    scrawl_log(format!("shortcuts: {backend:?}"));
}

/// The grabs' own X connection, so their key events don't mix with anything else.
struct Grabs {
    conn: RustConnection,
    root: u32,
    /// (keycode, modifier mask without the lock keys, shortcut) for every grab held.
    held: Mutex<Vec<(u8, u16, ShortcutKind)>>,
}

static GRABS: OnceLock<Option<Grabs>> = OnceLock::new();

fn grabs() -> Option<&'static Grabs> {
    GRABS
        .get_or_init(|| {
            let (conn, screen) = RustConnection::connect(None).ok()?;
            let root = conn.setup().roots.get(screen)?.root;
            Some(Grabs {
                conn,
                root,
                held: Mutex::new(Vec::new()),
            })
        })
        .as_ref()
}

fn mod_mask(carbon: u32) -> u16 {
    let mut mask = 0_u16;
    for (bit, flag) in [
        (CARBON_CMD_KEY, ModMask::M4),
        (CARBON_CONTROL_KEY, ModMask::CONTROL),
        (CARBON_OPTION_KEY, ModMask::M1),
        (CARBON_SHIFT_KEY, ModMask::SHIFT),
    ] {
        if carbon & bit != 0 {
            mask |= u16::from(flag);
        }
    }
    mask
}

/// Caps Lock and Num Lock: a grab is made with each of them on too, or it misses while they are.
const LOCK_MASKS: [u16; 4] = [0, 0x02, 0x10, 0x12];

fn ungrab_all(grabs: &Grabs) {
    let mut held = grabs
        .held
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    for (code, mask, _) in held.drain(..) {
        for lock in LOCK_MASKS {
            let _ = grabs
                .conn
                .ungrab_key(code, grabs.root, ModMask::from(mask | lock));
        }
    }
    let _ = grabs.conn.flush();
}

/// Replaces every grab with `record`'s and `paste`'s. Fails with the shortcut whose own combo
/// another app holds; its aliases are best effort.
fn grab_shortcuts(record: &ShortcutConfig, paste: &ShortcutConfig) -> Result<(), ShortcutKind> {
    let Some(grabs) = grabs() else {
        return Ok(());
    };
    ungrab_all(grabs);
    x11::refresh_keymap();
    let mut failed = None;
    let mut held = Vec::new();
    for (kind, cfg) in [(ShortcutKind::Record, record), (ShortcutKind::Paste, paste)] {
        for (i, (vk, mods)) in hotkey_combos(kind, cfg).into_iter().enumerate() {
            let Some(code) = keysym_for_vk(vk).and_then(keycode) else {
                continue;
            };
            let mask = mod_mask(mods);
            let ok = LOCK_MASKS.iter().all(|&lock| {
                grabs
                    .conn
                    .grab_key(
                        false,
                        grabs.root,
                        ModMask::from(mask | lock),
                        code,
                        GrabMode::ASYNC,
                        GrabMode::ASYNC,
                    )
                    .ok()
                    .and_then(|cookie| cookie.check().ok())
                    .is_some()
            });
            scrawl_log(format!(
                "XGrabKey({kind:?} {}) -> {}",
                shortcut_label(mods, cfg.key_label()),
                if ok { "ok" } else { "taken" }
            ));
            if ok {
                held.push((code, mask, kind));
            } else if i == 0 && failed.is_none() {
                failed = Some(kind);
            }
        }
    }
    *grabs
        .held
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = held;
    failed.map_or(Ok(()), Err)
}

fn emit(telemetry: &SharedCaptureTelemetry, kind: ShortcutKind) {
    if telemetry.shortcuts_paused.load(Ordering::Acquire) {
        return;
    }
    match kind {
        ShortcutKind::Record => telemetry.emit_capture_shortcut(),
        ShortcutKind::Paste => {
            // The portal's binding belongs to the desktop and can't be removed in place.
            let unset = telemetry
                .paste_shortcut
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_unset();
            if !unset && !telemetry.active.load(Ordering::Acquire) {
                telemetry.emit_paste_shortcut();
            }
        }
    }
}

/// Reads the grabbed keys' presses off their connection.
fn run_grab_events(telemetry: Arc<SharedCaptureTelemetry>) {
    let Some(grabs) = grabs() else {
        return;
    };
    let mut down: Option<(u8, u16)> = None;
    while let Ok(event) = grabs.conn.wait_for_event() {
        match event {
            Event::KeyPress(press) => {
                let mask = u16::from(press.state) & !0x12 & 0xff;
                // Holding the keys repeats the press; one capture per press.
                if down == Some((press.detail, mask)) {
                    continue;
                }
                down = Some((press.detail, mask));
                let kind = grabs
                    .held
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .iter()
                    .find(|(code, m, _)| *code == press.detail && *m == mask)
                    .map(|(_, _, kind)| *kind);
                if let Some(kind) = kind {
                    emit(&telemetry, kind);
                }
            }
            Event::KeyRelease(release) => {
                if down.is_some_and(|(code, _)| code == release.detail) {
                    down = None;
                }
            }
            Event::MappingNotify(_) => x11::refresh_keymap(),
            _ => {}
        }
    }
}

fn shortcut_key(cfg: &ShortcutConfig) -> String {
    cfg.key_label().to_owned()
}

/// GNOME's keybindings for both shortcuts, or empty ones while paused.
fn bind_gnome(record: &ShortcutConfig, paste: &ShortcutConfig, paused: bool) -> anyhow::Result<()> {
    let binding = |cfg: &ShortcutConfig| {
        if paused || cfg.is_unset() {
            String::new()
        } else {
            linux::portal::gnome_binding(cfg.carbon_modifiers, &shortcut_key(cfg))
        }
    };
    linux::portal::bind_gnome_keybindings(&[("record", binding(record)), ("paste", binding(paste))])
}

/// Sets up the shortcuts for this session on a thread of its own (the portal may show a
/// dialog), and listens for `scrawl shortcut …` from GNOME's keybindings.
pub fn install_global_hotkeys(telemetry: &Arc<SharedCaptureTelemetry>) {
    if HOTKEY_TELEMETRY.set(Arc::clone(telemetry)).is_err() {
        return;
    }
    let ipc_telemetry = Arc::clone(telemetry);
    linux::ipc::listen(move |line| match line {
        "shortcut record" => emit(&ipc_telemetry, ShortcutKind::Record),
        "shortcut paste" => emit(&ipc_telemetry, ShortcutKind::Paste),
        "open" => linux::ipc::request_open(),
        other => scrawl_log(format!("ipc: unknown request {other:?}")),
    });
    scrawl_log(format!("session: {}", linux::session_label()));
    let telemetry = Arc::clone(telemetry);
    let _ = thread::Builder::new()
        .name("scrawl-hotkeys".into())
        .spawn(move || {
            let (record, paste) = telemetry.shortcuts();
            if !linux::wayland_session() {
                if let Err(kind) = grab_shortcuts(&record, &paste) {
                    scrawl_log(format!(
                        "install_global_hotkeys: {kind:?} shortcut is taken"
                    ));
                }
                set_backend(Backend::X11);
                run_grab_events(telemetry);
                return;
            }
            // Plasma 5's portal opens System Settings but never binds the requested keys.
            // KGlobalAccel consumes the chord; Plasma 6 uses the portal normally.
            let plasma5 =
                linux::kde() && std::env::var("KDE_SESSION_VERSION").is_ok_and(|v| v == "5");
            if !plasma5 && std::env::var_os("SCRAWL_NO_SHORTCUTS_PORTAL").is_none() {
                let portal_telemetry = Arc::clone(&telemetry);
                let bound = linux::portal::global_shortcuts(
                    linux::portal::portal_trigger(record.carbon_modifiers, &shortcut_key(&record)),
                    (!paste.is_unset()).then(|| {
                        linux::portal::portal_trigger(paste.carbon_modifiers, &shortcut_key(&paste))
                    }),
                    move |id| match id {
                        "record" => emit(&portal_telemetry, ShortcutKind::Record),
                        "paste" => emit(&portal_telemetry, ShortcutKind::Paste),
                        _ => {}
                    },
                );
                match bound {
                    Ok(()) => return set_backend(Backend::Portal),
                    Err(error) => scrawl_log(format!("GlobalShortcuts portal: {error:#}")),
                }
            }
            if linux::gnome() {
                match bind_gnome(&record, &paste, false) {
                    Ok(()) => return set_backend(Backend::Gnome),
                    Err(error) => scrawl_log(format!("GNOME keybindings: {error:#}")),
                }
            }
            if linux::kde() {
                let keys = Arc::clone(&telemetry);
                match linux::shortcuts::listen(record.clone(), paste.clone(), move |id| match id {
                    "record" => emit(&keys, ShortcutKind::Record),
                    "paste" => emit(&keys, ShortcutKind::Paste),
                    _ => {}
                }) {
                    Ok(()) => return set_backend(Backend::Kde),
                    Err(error) => scrawl_log(format!("KDE shortcuts: {error:#}")),
                }
            }
            if input::start() {
                set_backend(Backend::Devices);
            } else {
                scrawl_log(
                    "shortcuts: no portal, no GNOME and no readable input devices; \
                     use the tray menu or `scrawl shortcut record`",
                );
            }
        });
}

/// Sets both shortcuts and binds them. If the X server refuses either, the old ones are put
/// back and the refused one is returned. With the portal the desktop owns the binding, so the
/// new one is only remembered. Nothing is saved here (`crate::settings` does that).
pub fn set_shortcuts(
    telemetry: &SharedCaptureTelemetry,
    record: ShortcutConfig,
    paste: ShortcutConfig,
) -> Result<(), ShortcutKind> {
    let (old_record, old_paste) = telemetry.shortcuts();
    if !telemetry.shortcuts_paused.load(Ordering::Acquire) {
        match backend() {
            Backend::X11 => {
                if let Err(kind) = grab_shortcuts(&record, &paste) {
                    let _ = grab_shortcuts(&old_record, &old_paste);
                    return Err(kind);
                }
            }
            Backend::Gnome => {
                if let Err(error) = bind_gnome(&record, &paste, false) {
                    scrawl_log(format!("GNOME keybindings: {error:#}"));
                }
            }
            Backend::Portal => scrawl_log(
                "shortcuts: the desktop owns the portal's binding; change it in its settings",
            ),
            Backend::Kde => {
                if let Err(kind) = linux::shortcuts::bind(&record, &paste, false) {
                    let _ = linux::shortcuts::bind(&old_record, &old_paste, false);
                    return Err(kind);
                }
            }
            Backend::Devices | Backend::None => {}
        }
    }
    *telemetry
        .shortcut
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = record;
    *telemetry
        .paste_shortcut
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = paste;
    Ok(())
}

/// While Settings records a new shortcut, Scrawl's own stop answering, so pressing the current
/// combo reaches the recorder instead of starting a capture.
pub fn pause_shortcuts(telemetry: &SharedCaptureTelemetry, paused: bool) {
    if telemetry.shortcuts_paused.swap(paused, Ordering::AcqRel) == paused {
        return;
    }
    let (record, paste) = telemetry.shortcuts();
    match backend() {
        Backend::X11 => {
            if paused {
                if let Some(grabs) = grabs() {
                    ungrab_all(grabs);
                }
            } else {
                let _ = grab_shortcuts(&record, &paste);
            }
        }
        Backend::Gnome => {
            let _ = bind_gnome(&record, &paste, paused);
        }
        Backend::Kde => {
            let _ = linux::shortcuts::bind(&record, &paste, paused);
        }
        Backend::Portal | Backend::Devices | Backend::None => {}
    }
}

/// The review window's own recorder: sets and saves the record shortcut.
pub fn update_global_shortcut(telemetry: &SharedCaptureTelemetry, config: ShortcutConfig) {
    let (_, paste) = telemetry.shortcuts();
    match set_shortcuts(telemetry, config.clone(), paste) {
        Ok(()) => crate::settings::save_shortcut(ShortcutKind::Record, &config),
        Err(kind) => scrawl_log(format!("update_global_shortcut: {kind:?} refused")),
    }
}

// MARK: Input

/// Notes where keys come from: the X server on X11, the input devices on Wayland.
fn check_tap(telemetry: &SharedCaptureTelemetry) {
    if linux::wayland_session() {
        input::start();
    }
    let keys = !linux::wayland_session() || from_devices();
    if telemetry.tap_live.swap(keys, Ordering::AcqRel) != keys {
        scrawl_log(format!(
            "keys: {}",
            if keys {
                "readable"
            } else {
                "only over Scrawl's windows (add yourself to the input group)"
            }
        ));
    }
}

pub fn start_global_input_tap(telemetry: Arc<SharedCaptureTelemetry>) {
    if linux::wayland_session() && !input::start() {
        scrawl_log(
            "input devices unreadable: ⇧ ink, clicks and Esc work only over X11 apps \
             (see docs/linux.md)",
        );
    }
    let _ = thread::Builder::new()
        .name("scrawl-key-poller".into())
        .spawn(move || run_key_poller(telemetry));
}

/// Logs a modifier chord (`Ctrl+Shift+C`), never ordinary typing.
fn log_chord(telemetry: &SharedCaptureTelemetry, mods: Modifiers, vk: u16) {
    if !(mods.ctrl || mods.super_key) {
        return;
    }
    let Some(key) = shortcut_key_name(vk) else {
        return;
    };
    let t_ms = telemetry.elapsed_ms();
    telemetry.window_log_request.store(true, Ordering::Release);
    telemetry
        .events
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(InteractionEvent::Key {
            t: t_ms,
            shortcut: shortcut_label(mods.bits(), &key),
        });
}

/// 125 Hz poller: the shortcuts where nothing else delivers them, Esc, Ctrl+Z, logged chords,
/// clicks, the pointer for the overlay, and ⇧ ink.
fn run_key_poller(telemetry: Arc<SharedCaptureTelemetry>) {
    let mut prev_keys: HashSet<u16> = HashSet::new();
    let mut prev_shift_ink = false;
    let mut await_shift_release = false;
    let mut prev_buttons = [false; 2];
    let mut last_tap_check = Instant::now() - Duration::from_secs(1);
    // On Wayland: whether the overlay has the pointer, and where XWayland last saw it before
    // it did (stale until the pointer moves over the overlay).
    let mut pointer_taken = false;
    let mut stale_pointer: Option<(f64, f64)> = None;
    loop {
        thread::sleep(Duration::from_millis(8));
        if last_tap_check.elapsed() >= Duration::from_millis(500) {
            check_tap(&telemetry);
            last_tap_check = Instant::now();
        }
        let held = held_polled();
        let mods = held.mods;
        if !mods.shift {
            await_shift_release = false;
        } else if mods.super_key || mods.ctrl || mods.alt {
            await_shift_release = true;
        }
        let pressed: Vec<u16> = held.keys.difference(&prev_keys).copied().collect();
        prev_keys.clone_from(&held.keys);
        let active = telemetry.active.load(Ordering::Acquire);

        for &vk in &pressed {
            if vk == VK_ESCAPE && !mods.super_key && !mods.ctrl {
                telemetry.escape_pressed.store(true, Ordering::Release);
            }
            if backend() == Backend::Devices && !telemetry.shortcuts_paused.load(Ordering::Acquire)
            {
                let (record, paste) = telemetry.shortcuts();
                let hit = |kind: ShortcutKind, cfg: &ShortcutConfig| {
                    shortcut_matches(
                        kind,
                        cfg,
                        layout_default_keycode(kind),
                        vk,
                        mods.super_key,
                        mods.ctrl,
                        mods.alt,
                        mods.shift,
                    )
                };
                if hit(ShortcutKind::Record, &record) {
                    emit(&telemetry, ShortcutKind::Record);
                    continue;
                }
                if hit(ShortcutKind::Paste, &paste) {
                    emit(&telemetry, ShortcutKind::Paste);
                    continue;
                }
            }
            if !active {
                continue;
            }
            if vk == KEY_Z && mods.ctrl && !mods.alt && !mods.shift && !mods.super_key {
                let _ = telemetry.ui_sender.try_send(LiveOverlayEvent::UndoStroke);
            } else {
                log_chord(&telemetry, mods, vk);
            }
        }

        let buttons = [held.left, held.right];
        if !active {
            prev_shift_ink = false;
            prev_buttons = buttons;
            if pointer_taken {
                linux::overlay::take_pointer(false);
                pointer_taken = false;
            }
            continue;
        }

        let mut pure_shift =
            mods.shift && !mods.super_key && !mods.ctrl && !mods.alt && !await_shift_release;
        // Wayland: XWayland only sees the pointer over X11 windows, so the overlay takes it
        // while ⇧ is held (`linux::overlay::take_pointer`).
        if from_devices() && pure_shift != pointer_taken {
            linux::overlay::take_pointer(pure_shift);
            pointer_taken = pure_shift;
            stale_pointer = pure_shift.then(linux::cursor_position).flatten();
        }

        let Some((x, y)) = linux::cursor_position() else {
            continue;
        };
        if stale_pointer.is_some_and(|stale| stale == (x, y)) {
            // Not where the pointer is yet: wait for it to move over the overlay.
            pure_shift = false;
        } else {
            stale_pointer = None;
        }
        *telemetry
            .pointer_screen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = [x as f32, y as f32];

        poll_clicks(
            &telemetry,
            &mut prev_buttons,
            buttons,
            (x, y),
            mods.shift && !mods.super_key && !mods.ctrl && !mods.alt,
            |x, y| telemetry.on_scrawl_window(x, y),
        );

        // Hold Shift + move the pointer inside the capture target = draw a persistent stroke.
        track_shift_ink(&telemetry, (x, y), pure_shift, prev_shift_ink);
        prev_shift_ink = pure_shift;
    }
}

// MARK: Recording

pub(super) fn run_sampler(sampler: Sampler) -> Option<PathBuf> {
    crate::linux::media::run_sampler(sampler)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_portal_press_is_ignored_once_the_paste_shortcut_is_turned_off() {
        let (tx, rx) = flume::unbounded();
        let telemetry = SharedCaptureTelemetry::new(tx);
        *telemetry.paste_shortcut.lock().unwrap() = ShortcutConfig::unset();
        emit(&telemetry, ShortcutKind::Paste);
        assert!(rx.try_recv().is_err(), "a cleared paste shortcut must not paste");
        *telemetry.paste_shortcut.lock().unwrap() = ShortcutConfig::default_paste();
        emit(&telemetry, ShortcutKind::Paste);
        assert!(matches!(
            rx.try_recv(),
            Ok(LiveOverlayEvent::PasteLatestShortcut)
        ));
    }

    #[test]
    fn virtual_keys_round_trip_through_keysyms() {
        for vk in [
            u32::from(KEY_S),
            u32::from(KEY_V),
            0x30,
            0x39,
            0x70,
            0x87,
            0x20,
        ] {
            let sym = keysym_for_vk(vk).expect("bound");
            assert_eq!(vk_for_keysym(sym), Some(vk as u16));
        }
        // Shifted letters are the same key.
        assert_eq!(vk_for_keysym(u32::from(b'S')), Some(KEY_S));
    }

    #[test]
    fn grabs_use_the_x_modifier_bits() {
        assert_eq!(
            mod_mask(CARBON_OPTION_KEY | CARBON_SHIFT_KEY),
            u16::from(ModMask::M1) | u16::from(ModMask::SHIFT)
        );
        assert_eq!(mod_mask(CARBON_CMD_KEY), u16::from(ModMask::M4));
    }
}
