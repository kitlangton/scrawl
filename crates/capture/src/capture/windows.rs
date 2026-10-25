//! Windows capture: `RegisterHotKey` shortcuts, low-level keyboard and mouse hooks with a key
//! state poller, the foreground window, displays and windows, and the Windows.Graphics.Capture
//! recorder (`crate::win::media::Recorder`).
//!
//! Global coordinates are physical pixels with the primary display's top-left at the origin.
//! `TargetFrame` holds pixels too, so a recording is never upscaled (`MAX_CAPTURE_SCALE`).

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::Threading::GetCurrentThreadId;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    GetAsyncKeyState, HOT_KEY_MODIFIERS, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE,
    KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT,
    MOD_WIN, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
    MOUSEEVENTF_VIRTUALDESK, MOUSEINPUT, RegisterHotKey, SendInput, UnregisterHotKey, VIRTUAL_KEY,
    VK_CONTROL, VK_ESCAPE, VK_LBUTTON, VK_LWIN, VK_MENU, VK_RBUTTON, VK_RWIN, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetMessageW, GetSystemMetrics, HC_ACTION, KBDLLHOOKSTRUCT, LLKHF_INJECTED,
    LLMHF_INJECTED, MSG, MSLLHOOKSTRUCT, PM_NOREMOVE, PeekMessageW, PostThreadMessageW,
    SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_SWAPBUTTON, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
    SetForegroundWindow, SetWindowsHookExW, WH_KEYBOARD_LL, WH_MOUSE_LL, WM_APP, WM_HOTKEY,
    WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEHWHEEL, WM_MOUSEWHEEL,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN, WM_SYSKEYUP,
};

use super::{
    CARBON_CMD_KEY, CARBON_CONTROL_KEY, CARBON_OPTION_KEY, CARBON_SHIFT_KEY, Event, InputEvent,
    Sampler, SharedCaptureTelemetry, ShortcutConfig, ShortcutKind, clean_label, hotkey_combos,
    layout_default_keycode, pick_display, poll_clicks, pure_shift_state, shortcut_label,
    shortcut_matches, track_shift_ink,
};
use crate::log::scrawl_log;
use crate::model::{CaptureMode, TargetFrame, TargetInfo};
use crate::win;

/// Frames are pixels already: a recording keeps at most the screen's own resolution.
pub const MAX_CAPTURE_SCALE: f64 = 1.0;

/// Where a session's clicks and keys come from, for the log.
pub(super) const MOUSE_SOURCE: &str = "the mouse hook";
pub(super) const KEY_SOURCE: &str = "the keyboard hook";
pub(super) const NO_KEY_SOURCE: &str = "polled key state (keyboard hook off)";

/// `VK_S`. Windows maps virtual keys through the active layout, so the key that types S is
/// `VK_S` on Dvorak too.
pub const KEY_S: u16 = 0x53;
pub const KEY_V: u16 = 0x56;
const KEY_Z: u16 = 0x5A;
/// An unassigned virtual key, sent around Alt or Win coming up after a shortcut so the app in
/// front doesn't open its menu bar (or the Start menu) on the lone modifier release.
const VK_MASK: u16 = 0xE8;

/// The primary display's index in `win::monitors()`.
pub fn main_display_id() -> u32 {
    0
}

/// The virtual key that types `ch` on the active layout.
pub fn key_code_for_char(ch: char) -> Option<u16> {
    let scan = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::VkKeyScanW(ch as u16) };
    // The low byte is the key; -1 means no key types it.
    (scan != -1).then_some((scan as u16) & 0xFF)
}

pub fn layout_s_keycode() -> u16 {
    key_code_for_char('s').unwrap_or(KEY_S)
}

pub fn layout_v_keycode() -> u16 {
    key_code_for_char('v').unwrap_or(KEY_V)
}

/// Windows asks nobody for screen recording.
pub fn screen_capture_granted() -> bool {
    true
}

/// Screen capture always works on Windows: no grant to wait for, no relaunch.
pub fn screen_capture_usable(_probe: bool) -> bool {
    true
}

/// The primary display's size (`scrawl sck-probe`); Windows.Graphics.Capture needs no grant.
pub fn screen_capture_probe() -> Result<(u32, u32), String> {
    let monitor = win::primary_monitor().ok_or("no display")?;
    Ok((monitor.rect[2] as u32, monitor.rect[3] as u32))
}

pub fn request_screen_capture() {}

/// Sending Ctrl+V needs no permission on Windows (only an elevated app in front refuses it).
pub fn accessibility_granted() -> bool {
    true
}

fn key_down(vk: u16) -> bool {
    let state = unsafe { GetAsyncKeyState(i32::from(vk)) };
    state < 0
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
struct Modifiers {
    win: bool,
    ctrl: bool,
    alt: bool,
    shift: bool,
}

impl Modifiers {
    fn now() -> Self {
        Self {
            win: key_down(VK_LWIN.0) || key_down(VK_RWIN.0),
            ctrl: key_down(VK_CONTROL.0),
            alt: key_down(VK_MENU.0),
            shift: key_down(VK_SHIFT.0),
        }
    }

    fn any(self) -> bool {
        self.win || self.ctrl || self.alt || self.shift
    }

    /// As Carbon modifier bits, the format the shortcut is stored in (`⌘` is the Windows key).
    fn bits(self) -> u64 {
        let mut bits = 0;
        for (held, bit) in [
            (self.win, CARBON_CMD_KEY),
            (self.ctrl, CARBON_CONTROL_KEY),
            (self.alt, CARBON_OPTION_KEY),
            (self.shift, CARBON_SHIFT_KEY),
        ] {
            if held {
                bits |= u64::from(bit);
            }
        }
        bits
    }
}

/// The Win, Ctrl, Alt and Shift keys held right now, as a bit mask.
pub fn modifier_flags() -> u64 {
    Modifiers::now().bits()
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
    if let Some(front) = win::foreground_window()
        && !is_ours(front.pid)
    {
        LAST_EXTERNAL_WINDOW.store(front.hwnd, Ordering::Release);
    }
}

/// Puts the last app that was in front (before Scrawl's own window took focus) back in front,
/// so a paste lands in it.
pub fn ensure_external_app_active() {
    if let Some(front) = win::foreground_window()
        && !is_ours(front.pid)
    {
        LAST_EXTERNAL_WINDOW.store(front.hwnd, Ordering::Release);
        return;
    }
    let target = LAST_EXTERNAL_WINDOW.load(Ordering::Acquire);
    if target == 0 {
        return;
    }
    let _ = unsafe { SetForegroundWindow(win::hwnd(target)) };
    let deadline = Instant::now() + Duration::from_millis(220);
    while Instant::now() < deadline {
        if win::foreground_window().is_some_and(|w| w.hwnd == target) {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    thread::sleep(Duration::from_millis(25));
}

fn external_front_window() -> Option<win::WindowInfo> {
    if let Some(front) = win::foreground_window()
        && !is_ours(front.pid)
    {
        LAST_EXTERNAL_WINDOW.store(front.hwnd, Ordering::Release);
        return Some(front);
    }
    match LAST_EXTERNAL_WINDOW.load(Ordering::Acquire) {
        0 => None,
        id => win::window_info(id),
    }
}

/// The frontmost app's pid, or the last external one while Scrawl itself is in front.
pub fn frontmost_external_pid() -> Option<i32> {
    external_front_window().map(|w| w.pid as i32)
}

/// The frontmost app (pid, name, executable as its identifier), or the last one before Scrawl.
pub fn frontmost_app_info() -> Option<(i32, String, Option<String>)> {
    let front = external_front_window()?;
    let (name, id) = win::app_identity(front.pid)?;
    Some((front.pid as i32, name, Some(id)))
}

pub fn current_mouse_screen() -> Option<[f32; 2]> {
    win::cursor_position().map(|(x, y)| [x as f32, y as f32])
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

/// Selectable app windows (not Scrawl's), front to back, with their app names.
fn app_windows() -> Vec<(win::WindowInfo, String, String)> {
    let mut names: std::collections::HashMap<u32, Option<(String, String)>> =
        std::collections::HashMap::new();
    win::top_level_windows()
        .into_iter()
        .filter(|w| w.on_screen() && w.is_app_window() && !is_ours(w.pid))
        .filter_map(|w| {
            let identity = names
                .entry(w.pid)
                .or_insert_with(|| win::app_identity(w.pid))
                .clone()?;
            Some((w, identity.0, identity.1))
        })
        .collect()
}

fn window_target(window: &win::WindowInfo, app: &str, id: &str) -> TargetInfo {
    let monitor = win::monitor_at(
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

fn display_index(monitor: Option<&win::Monitor>) -> u32 {
    let Some(monitor) = monitor else {
        return 0;
    };
    win::monitors()
        .iter()
        .position(|m| m.handle == monitor.handle)
        .unwrap_or(0) as u32
}

pub fn discover_target(window_query: Option<&str>) -> TargetInfo {
    discover_target_at(window_query, win::cursor_position().map(|(x, y)| [x, y]))
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
    let monitors = win::monitors();
    let front = external_front_window();
    let rects: Vec<[f64; 4]> = monitors.iter().map(|m| m.rect).collect();
    let index = pick_display(&rects, point, front.as_ref().map(|w| w.rect));
    let (rect, scale) = monitors
        .get(index)
        .map_or(([0.0, 0.0, 1920.0, 1080.0], 1.0), |m| (m.rect, m.scale));
    TargetInfo {
        mode: CaptureMode::Display,
        app: frontmost.as_ref().map(|(_, name, _)| name.clone()),
        bundle_id: frontmost.and_then(|(_, _, id)| id),
        window_title: front
            .map(|w| clean_label(&w.title))
            .filter(|t| !t.is_empty()),
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

/// Windows has no `scrawl-helper`: every Scrawl window is in its own process.
pub(super) fn is_helper_process(_pid: i32) -> bool {
    false
}

// MARK: Synthetic input

fn keyboard_input(vk: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VIRTUAL_KEY(vk),
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

fn send(inputs: &[INPUT]) -> bool {
    let sent = unsafe { SendInput(inputs, std::mem::size_of::<INPUT>() as i32) };
    sent as usize == inputs.len()
}

pub fn wait_for_modifiers_released(max_wait_ms: u64) {
    let deadline = Instant::now() + Duration::from_millis(max_wait_ms);
    while Instant::now() < deadline {
        if !Modifiers::now().any() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// Ctrl+V into the app in front. A Ctrl the user still holds is left alone.
pub fn post_synthetic_cmd_v() {
    let held = key_down(VK_CONTROL.0);
    let mut inputs = Vec::with_capacity(4);
    if !held {
        inputs.push(keyboard_input(VK_CONTROL.0, KEYBD_EVENT_FLAGS(0)));
    }
    inputs.push(keyboard_input(KEY_V, KEYBD_EVENT_FLAGS(0)));
    inputs.push(keyboard_input(KEY_V, KEYEVENTF_KEYUP));
    if !held {
        inputs.push(keyboard_input(VK_CONTROL.0, KEYEVENTF_KEYUP));
    }
    if !send(&inputs) {
        scrawl_log("Ctrl+V was refused (an elevated app in front ignores Scrawl's input)");
    }
}

/// A left click at a global point.
pub fn post_synthetic_click(screen_x: f64, screen_y: f64) {
    let (vx, vy, vw, vh) = unsafe {
        (
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1),
            GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1),
        )
    };
    let nx = ((screen_x - f64::from(vx)) * 65535.0 / f64::from(vw - 1).max(1.0)).round() as i32;
    let ny = ((screen_y - f64::from(vy)) * 65535.0 / f64::from(vh - 1).max(1.0)).round() as i32;
    let mouse = |flags| INPUT {
        r#type: INPUT_MOUSE,
        Anonymous: INPUT_0 {
            mi: MOUSEINPUT {
                dx: nx,
                dy: ny,
                mouseData: 0,
                dwFlags: flags | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    send(&[mouse(MOUSEEVENTF_LEFTDOWN)]);
    thread::sleep(Duration::from_millis(14));
    send(&[mouse(MOUSEEVENTF_LEFTUP)]);
}

/// While Alt or Win is held for a shortcut, taps an unassigned key, so letting go of Alt doesn't
/// open the menu bar of the app in front (or Win the Start menu).
fn mask_menu_key() {
    if key_down(VK_MENU.0) || key_down(VK_LWIN.0) || key_down(VK_RWIN.0) {
        send(&[
            keyboard_input(VK_MASK, KEYBD_EVENT_FLAGS(0)),
            keyboard_input(VK_MASK, KEYEVENTF_KEYUP),
        ]);
    }
}

// MARK: Shortcuts

const HOTKEY_ID_TOGGLE: i32 = 1;
const HOTKEY_ID_ALT1: i32 = 3;
const HOTKEY_ID_PASTE_BASE: i32 = 100;
/// Posted to the hotkey thread to register the configured shortcut again.
const WM_UPDATE_SHORTCUT: u32 = WM_APP + 1;

static HOTKEY_THREAD: AtomicU32 = AtomicU32::new(0);
static HOTKEY_TELEMETRY: OnceLock<Arc<SharedCaptureTelemetry>> = OnceLock::new();

fn hot_key_modifiers(carbon: u32) -> HOT_KEY_MODIFIERS {
    let mut mods = MOD_NOREPEAT;
    for (bit, flag) in [
        (CARBON_CMD_KEY, MOD_WIN),
        (CARBON_CONTROL_KEY, MOD_CONTROL),
        (CARBON_OPTION_KEY, MOD_ALT),
        (CARBON_SHIFT_KEY, MOD_SHIFT),
    ] {
        if carbon & bit != 0 {
            mods |= flag;
        }
    }
    mods
}

/// What the hotkey thread is asked to do. Hotkeys belong to the thread that registers them,
/// so every change goes through it.
enum HotkeyRequest {
    /// Replace every registered hotkey with these (record, paste); answers with the refused one.
    Register(
        ShortcutConfig,
        ShortcutConfig,
        std::sync::mpsc::Sender<Result<(), ShortcutKind>>,
    ),
    /// Unregister everything, so Settings' recorder gets the keys.
    Pause(std::sync::mpsc::Sender<Result<(), ShortcutKind>>),
}

static HOTKEY_REQUESTS: OnceLock<Mutex<std::sync::mpsc::Sender<HotkeyRequest>>> = OnceLock::new();

fn hotkey_id(kind: ShortcutKind, index: usize) -> i32 {
    match (kind, index) {
        (ShortcutKind::Record, 0) => HOTKEY_ID_TOGGLE,
        (ShortcutKind::Record, i) => HOTKEY_ID_ALT1 + i as i32,
        (ShortcutKind::Paste, i) => HOTKEY_ID_PASTE_BASE + i as i32,
    }
}

/// Replaces every registered hotkey with `record`'s and `paste`'s. Fails with the shortcut whose
/// own combo Windows refused (another app holds it); its aliases are best effort.
fn register_shortcuts(
    ids: &mut Vec<i32>,
    record: &ShortcutConfig,
    paste: &ShortcutConfig,
) -> Result<(), ShortcutKind> {
    unregister_all(ids);
    let mut failed = None;
    for (kind, cfg) in [(ShortcutKind::Record, record), (ShortcutKind::Paste, paste)] {
        for (i, (code, mods)) in hotkey_combos(kind, cfg).into_iter().enumerate() {
            let id = hotkey_id(kind, i);
            let ok = unsafe { RegisterHotKey(None, id, hot_key_modifiers(mods), code) }.is_ok();
            scrawl_log(format!(
                "RegisterHotKey({kind:?} {}) -> {}",
                if i == 0 {
                    cfg.label.clone()
                } else {
                    shortcut_label(mods, cfg.key_label())
                },
                if ok { "ok" } else { "taken" }
            ));
            if ok {
                ids.push(id);
            } else if i == 0 && failed.is_none() {
                failed = Some(kind);
            }
        }
    }
    failed.map_or(Ok(()), Err)
}

fn unregister_all(ids: &mut Vec<i32>) {
    for id in ids.drain(..) {
        let _ = unsafe { UnregisterHotKey(None, id) };
    }
}

/// Hands a request to the hotkey thread and waits for its answer; `None` before it runs.
fn ask_hotkey_thread(
    request: impl FnOnce(std::sync::mpsc::Sender<Result<(), ShortcutKind>>) -> HotkeyRequest,
) -> Option<Result<(), ShortcutKind>> {
    let thread_id = HOTKEY_THREAD.load(Ordering::Acquire);
    let requests = HOTKEY_REQUESTS.get()?;
    if thread_id == 0 {
        return None;
    }
    let (reply_tx, reply_rx) = std::sync::mpsc::channel();
    requests
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .send(request(reply_tx))
        .ok()?;
    unsafe { PostThreadMessageW(thread_id, WM_UPDATE_SHORTCUT, WPARAM(0), LPARAM(0)) }.ok()?;
    reply_rx.recv_timeout(Duration::from_secs(2)).ok()
}

/// Registers both shortcuts (and the default's aliases) system-wide on a thread of their own,
/// which receives `WM_HOTKEY`. A registered shortcut doesn't reach the app in front.
pub fn install_global_hotkeys(telemetry: &Arc<SharedCaptureTelemetry>) {
    if HOTKEY_TELEMETRY.set(Arc::clone(telemetry)).is_err() {
        return;
    }
    let (request_tx, request_rx) = std::sync::mpsc::channel::<HotkeyRequest>();
    let _ = HOTKEY_REQUESTS.set(Mutex::new(request_tx));
    let (ready_tx, ready_rx) = std::sync::mpsc::channel();
    let spawned = thread::Builder::new()
        .name("scrawl-hotkeys".into())
        .spawn(move || {
            let mut msg = MSG::default();
            // Make the thread's message queue before anyone posts to it.
            let _ = unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE) };
            HOTKEY_THREAD.store(unsafe { GetCurrentThreadId() }, Ordering::Release);
            let Some(telemetry) = HOTKEY_TELEMETRY.get() else {
                return;
            };
            let mut ids = Vec::new();
            let (record, paste) = telemetry.shortcuts();
            if let Err(kind) = register_shortcuts(&mut ids, &record, &paste) {
                scrawl_log(format!(
                    "install_global_hotkeys: {kind:?} shortcut could not be registered"
                ));
            }
            let _ = ready_tx.send(());
            loop {
                let status = unsafe { GetMessageW(&mut msg, None, 0, 0) };
                if status.0 <= 0 {
                    break;
                }
                match msg.message {
                    WM_HOTKEY => {
                        let id = msg.wParam.0 as i32;
                        mask_menu_key();
                        if id >= HOTKEY_ID_PASTE_BASE {
                            telemetry.emit_paste_shortcut();
                        } else {
                            telemetry.emit_capture_shortcut();
                        }
                    }
                    WM_UPDATE_SHORTCUT => {
                        while let Ok(request) = request_rx.try_recv() {
                            match request {
                                HotkeyRequest::Register(record, paste, reply) => {
                                    let _ =
                                        reply.send(register_shortcuts(&mut ids, &record, &paste));
                                }
                                HotkeyRequest::Pause(reply) => {
                                    unregister_all(&mut ids);
                                    let _ = reply.send(Ok(()));
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        });
    if spawned.is_ok() {
        let _ = ready_rx.recv_timeout(Duration::from_secs(2));
    }
}

/// Sets both shortcuts and registers them. If Windows refuses either, the old ones are put back
/// and the refused one is returned. Returns the accepted `(record, paste)` config so the app can
/// save it (`crate::settings` does that).
pub fn set_shortcuts(
    telemetry: &SharedCaptureTelemetry,
    record: ShortcutConfig,
    paste: ShortcutConfig,
) -> Result<(ShortcutConfig, ShortcutConfig), ShortcutKind> {
    let (old_record, old_paste) = telemetry.shortcuts();
    if !telemetry.shortcuts_paused.load(Ordering::Acquire)
        && let Some(Err(kind)) =
            ask_hotkey_thread(|reply| HotkeyRequest::Register(record.clone(), paste.clone(), reply))
    {
        let _ = ask_hotkey_thread(|reply| HotkeyRequest::Register(old_record, old_paste, reply));
        return Err(kind);
    }
    *telemetry
        .shortcut
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = record.clone();
    *telemetry
        .paste_shortcut
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = paste.clone();
    Ok((record, paste))
}

/// While Settings records a new shortcut, Scrawl's own stop answering, so pressing the current
/// combo reaches the recorder instead of starting a capture.
pub fn pause_shortcuts(telemetry: &SharedCaptureTelemetry, paused: bool) {
    if telemetry.shortcuts_paused.swap(paused, Ordering::AcqRel) == paused {
        return;
    }
    if paused {
        let _ = ask_hotkey_thread(HotkeyRequest::Pause);
    } else {
        let (record, paste) = telemetry.shortcuts();
        let _ = ask_hotkey_thread(|reply| HotkeyRequest::Register(record, paste, reply));
    }
}

/// The review window's own recorder: sets the record shortcut and returns the accepted config.
pub fn update_global_shortcut(
    telemetry: &SharedCaptureTelemetry,
    config: ShortcutConfig,
) -> Option<ShortcutConfig> {
    let (_, paste) = telemetry.shortcuts();
    match set_shortcuts(telemetry, config, paste) {
        Ok((accepted, _)) => Some(accepted),
        Err(kind) => {
            scrawl_log(format!("update_global_shortcut: {kind:?} refused"));
            None
        }
    }
}

// MARK: Input hooks

/// What the low-level hooks saw, handled off the hook thread so the hooks return at once.
enum HookEvent {
    KeyDown {
        vk: u16,
        repeat: bool,
        mods: Modifiers,
    },
    Button {
        down: bool,
        right: bool,
        x: f64,
        y: f64,
        pure_shift: bool,
    },
    Wheel {
        x: f64,
        y: f64,
        dx: f32,
        dy: f32,
    },
}

static HOOK_SENDER: OnceLock<flume::Sender<HookEvent>> = OnceLock::new();
static KEYBOARD_HOOK_LIVE: AtomicBool = AtomicBool::new(false);
static MOUSE_HOOK_LIVE: AtomicBool = AtomicBool::new(false);

static KEYS_DOWN: Mutex<Option<HashSet<u32>>> = Mutex::new(None);

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let event = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
        let injected = event.flags.0 & LLKHF_INJECTED.0 != 0;
        let message = wparam.0 as u32;
        let vk = event.vkCode;
        let mut keys = KEYS_DOWN
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let keys = keys.get_or_insert_with(HashSet::new);
        match message {
            WM_KEYDOWN | WM_SYSKEYDOWN if !injected => {
                let repeat = !keys.insert(vk);
                if let Some(sender) = HOOK_SENDER.get() {
                    let _ = sender.try_send(HookEvent::KeyDown {
                        vk: vk as u16,
                        repeat,
                        mods: Modifiers::now(),
                    });
                }
            }
            WM_KEYUP | WM_SYSKEYUP if !injected => {
                keys.remove(&vk);
            }
            _ => {}
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

unsafe extern "system" fn mouse_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let event = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
        let injected = event.flags & LLMHF_INJECTED != 0;
        let (x, y) = (f64::from(event.pt.x), f64::from(event.pt.y));
        let message = wparam.0 as u32;
        let button = match message {
            WM_LBUTTONDOWN => Some((true, false)),
            WM_LBUTTONUP => Some((false, false)),
            WM_RBUTTONDOWN => Some((true, true)),
            WM_RBUTTONUP => Some((false, true)),
            _ => None,
        };
        if !injected && let Some(sender) = HOOK_SENDER.get() {
            if let Some((down, right)) = button {
                let mods = Modifiers::now();
                let _ = sender.try_send(HookEvent::Button {
                    down,
                    right,
                    x,
                    y,
                    pure_shift: mods.shift && !mods.ctrl && !mods.alt && !mods.win,
                });
            } else if message == WM_MOUSEWHEEL || message == WM_MOUSEHWHEEL {
                let delta = f32::from((event.mouseData >> 16) as u16 as i16) / 120.0;
                let (dx, dy) = if message == WM_MOUSEWHEEL {
                    (0.0, delta)
                } else {
                    (delta, 0.0)
                };
                let _ = sender.try_send(HookEvent::Wheel { x, y, dx, dy });
            }
        }
    }
    unsafe { CallNextHookEx(None, code, wparam, lparam) }
}

/// Handles what the hooks saw: the capture shortcut, and during a capture the clicks, drags,
/// scrolls, Ctrl+Z (undo the last stroke) and modifier chords.
fn handle_hook_event(telemetry: &SharedCaptureTelemetry, event: HookEvent) {
    match event {
        HookEvent::KeyDown { vk, repeat, mods } => {
            // Settings is recording a shortcut: the keys are for it, and the menu-mask key
            // tapped below would reach it as a key press of its own.
            if telemetry.shortcuts_paused.load(Ordering::Acquire) {
                return;
            }
            let (record, paste) = telemetry.shortcuts();
            let hit = |kind: ShortcutKind, cfg: &ShortcutConfig| {
                let layout = layout_default_keycode(kind);
                shortcut_matches(
                    kind, cfg, layout, vk, mods.win, mods.ctrl, mods.alt, mods.shift,
                )
            };
            if hit(ShortcutKind::Record, &record) {
                if !repeat {
                    if mods.alt || mods.win {
                        mask_menu_key();
                    }
                    telemetry.emit_capture_shortcut();
                }
                return;
            }
            if hit(ShortcutKind::Paste, &paste) {
                if !repeat && !telemetry.active.load(Ordering::Acquire) {
                    mask_menu_key();
                    telemetry.emit_paste_shortcut();
                }
                return;
            }
            if vk == VK_ESCAPE.0 && !repeat && !mods.win && !mods.ctrl {
                telemetry.escape_pressed.store(true, Ordering::Release);
            }
            if !telemetry.active.load(Ordering::Acquire) || repeat {
                return;
            }
            if vk == KEY_Z && mods.ctrl && !mods.alt && !mods.shift && !mods.win {
                let _ = telemetry.ui_sender.try_send(Event::UndoStroke);
            } else if (mods.ctrl || mods.win)
                && let Some(key) = shortcut_key_name(vk)
            {
                // Only modifier chords, never ordinary typing or secrets.
                let mut carbon = 0;
                for (held, bit) in [
                    (mods.ctrl, CARBON_CONTROL_KEY),
                    (mods.alt, CARBON_OPTION_KEY),
                    (mods.shift, CARBON_SHIFT_KEY),
                    (mods.win, CARBON_CMD_KEY),
                ] {
                    if held {
                        carbon |= bit;
                    }
                }
                let t_ms = telemetry.elapsed_ms();
                telemetry.handle_input(InputEvent::KeyChord {
                    t_ms,
                    shortcut: shortcut_label(carbon, &key),
                });
            }
        }
        HookEvent::Button {
            down,
            right,
            x,
            y,
            pure_shift,
        } => {
            if !telemetry.active.load(Ordering::Acquire) {
                return;
            }
            let t_ms = telemetry.elapsed_ms();
            let event = if down {
                InputEvent::ButtonDown {
                    t_ms,
                    x,
                    y,
                    right,
                    shift_only: pure_shift,
                }
            } else {
                InputEvent::ButtonUp {
                    t_ms,
                    x,
                    y,
                    right,
                    shift_only: pure_shift,
                }
            };
            telemetry.handle_input(event);
        }
        HookEvent::Wheel { x, y, dx, dy } => {
            if !telemetry.active.load(Ordering::Acquire) {
                return;
            }
            let t_ms = telemetry.elapsed_ms();
            telemetry.handle_input(InputEvent::Scroll {
                t_ms,
                x,
                y,
                dx: f64::from(dx),
                dy: f64::from(dy),
            });
        }
    }
}

/// Notes which hooks are installed: the keyboard hook brings keys (`tap_live`), the mouse hook
/// clicks (`mouse_live`). Windows needs no permission for either.
fn check_tap(telemetry: &SharedCaptureTelemetry) -> bool {
    let keys = KEYBOARD_HOOK_LIVE.load(Ordering::Acquire);
    let mouse = MOUSE_HOOK_LIVE.load(Ordering::Acquire);
    let was_keys = telemetry.tap_live.swap(keys, Ordering::AcqRel);
    let was_mouse = telemetry.mouse_live.swap(mouse, Ordering::AcqRel);
    if (was_keys, was_mouse) != (keys, mouse) {
        scrawl_log(format!(
            "input hooks: keyboard {}, mouse {}",
            if keys { "live" } else { "off (polling keys)" },
            if mouse {
                "live"
            } else {
                "off (polling the mouse buttons)"
            },
        ));
    }
    keys && mouse
}

pub fn start_global_input_tap(telemetry: Arc<SharedCaptureTelemetry>) {
    let (sender, receiver) = flume::bounded::<HookEvent>(512);
    if HOOK_SENDER.set(sender).is_err() {
        return;
    }
    let worker_telemetry = Arc::clone(&telemetry);
    let _ = thread::Builder::new()
        .name("scrawl-input-events".into())
        .spawn(move || {
            while let Ok(event) = receiver.recv() {
                handle_hook_event(&worker_telemetry, event);
            }
        });
    let _ = thread::Builder::new()
        .name("scrawl-input-hooks".into())
        .spawn(|| {
            let module: Option<HINSTANCE> = None;
            let keyboard =
                unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), module, 0) };
            let mouse = unsafe { SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook), module, 0) };
            scrawl_log(format!(
                "input hooks: keyboard {}, mouse {}",
                if keyboard.is_ok() { "ok" } else { "failed" },
                if mouse.is_ok() { "ok" } else { "failed" }
            ));
            KEYBOARD_HOOK_LIVE.store(keyboard.is_ok(), Ordering::Release);
            MOUSE_HOOK_LIVE.store(mouse.is_ok(), Ordering::Release);
            let mut msg = MSG::default();
            while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {}
            KEYBOARD_HOOK_LIVE.store(false, Ordering::Release);
            MOUSE_HOOK_LIVE.store(false, Ordering::Release);
        });
    let poller_telemetry = Arc::clone(&telemetry);
    let _ = thread::Builder::new()
        .name("scrawl-key-poller".into())
        .spawn(move || run_key_poller(poller_telemetry));
}

/// 125 Hz key state poller: the shortcut and paste chords (when the hooks miss them), Ctrl+Z,
/// the pointer for the overlay, clicks while the hooks are off, and Shift ink.
fn run_key_poller(telemetry: Arc<SharedCaptureTelemetry>) {
    let mut prev_shortcut = false;
    let mut prev_paste = false;
    let mut prev_shift_ink = false;
    let mut prev_undo = false;
    let mut prev_escape = false;
    let mut await_shift_release = false;
    let mut prev_buttons = [false; 2];
    let mut last_tap_check = Instant::now();
    loop {
        thread::sleep(Duration::from_millis(8));
        if last_tap_check.elapsed() >= Duration::from_millis(500) {
            check_tap(&telemetry);
            last_tap_check = Instant::now();
        }
        let mods = Modifiers::now();
        let Modifiers {
            win,
            ctrl,
            alt,
            shift,
        } = mods;
        let pure_shift = pure_shift_state(shift, win || ctrl || alt, &mut await_shift_release);

        // Esc for the picker, from key state while the keyboard hook is off.
        let escape_now = key_down(VK_ESCAPE.0);
        if escape_now && !prev_escape && !KEYBOARD_HOOK_LIVE.load(Ordering::Acquire) {
            telemetry.escape_pressed.store(true, Ordering::Release);
        }
        prev_escape = escape_now;

        let (record_cfg, paste_cfg) = telemetry.shortcuts();
        // Whether `kind`'s shortcut is down: its own key, or for a default its alias keys.
        let shortcut_down = |kind: ShortcutKind, cfg: &ShortcutConfig| {
            let mut codes = vec![cfg.key_code as u16];
            if cfg.is_default_for(kind) {
                codes.extend([layout_default_keycode(kind), kind.ansi_code()]);
            }
            codes.into_iter().any(|code| {
                key_down(code) && shortcut_matches(kind, cfg, code, code, win, ctrl, alt, shift)
            })
        };
        let shortcut_now = shortcut_down(ShortcutKind::Record, &record_cfg);
        if shortcut_now && !prev_shortcut {
            telemetry.emit_capture_shortcut();
        }
        prev_shortcut = shortcut_now;

        let paste_now = shortcut_down(ShortcutKind::Paste, &paste_cfg);
        if paste_now && !prev_paste && !telemetry.active.load(Ordering::Acquire) {
            telemetry.emit_paste_shortcut();
        }
        prev_paste = paste_now;

        let swapped = unsafe { GetSystemMetrics(SM_SWAPBUTTON) } != 0;
        let (left, right) = (key_down(VK_LBUTTON.0), key_down(VK_RBUTTON.0));
        let buttons = if swapped {
            [right, left]
        } else {
            [left, right]
        };
        if !telemetry.active.load(Ordering::Acquire) {
            prev_shift_ink = false;
            prev_undo = false;
            prev_buttons = buttons;
            continue;
        }

        let undo_now = ctrl && !win && !alt && !shift && key_down(KEY_Z);
        if undo_now && !prev_undo && !KEYBOARD_HOOK_LIVE.load(Ordering::Acquire) {
            let _ = telemetry.ui_sender.try_send(Event::UndoStroke);
        }
        prev_undo = undo_now;

        let Some((x, y)) = win::cursor_position() else {
            continue;
        };
        *telemetry
            .pointer_screen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = [x as f32, y as f32];

        poll_clicks(
            &telemetry,
            &mut prev_buttons,
            buttons,
            (x, y),
            shift && !win && !ctrl && !alt,
            |x, y| telemetry.on_scrawl_window(x, y),
        );

        // Hold Shift + move the cursor inside the capture target = draw a persistent stroke.
        track_shift_ink(&telemetry, (x, y), pure_shift, prev_shift_ink);
        prev_shift_ink = pure_shift;
    }
}

// MARK: Recording

pub(super) fn run_sampler(sampler: Sampler) -> Option<PathBuf> {
    crate::win::media::run_sampler(sampler)
}
