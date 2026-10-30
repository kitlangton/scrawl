//! macOS capture: Carbon hotkeys, the CoreGraphics event tap and HID state poller, NSWorkspace
//! for the frontmost app, and ScreenCaptureKit for displays, windows and the recording.

use std::ffi::c_void;
use std::path::PathBuf;
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use screencapturekit::prelude::*;
use screencapturekit::recording_output::{
    SCRecordingOutput, SCRecordingOutputCodec, SCRecordingOutputConfiguration,
    SCRecordingOutputFileType,
};
use screencapturekit::screenshot_manager::{CGImageExt, SCScreenshotManager};

use super::{
    CANDIDATE_EDGE, CANDIDATE_EDGE_NO_VIDEO, CARBON_CMD_KEY, CARBON_CONTROL_KEY, CARBON_OPTION_KEY,
    CARBON_SHIFT_KEY, FRAME_SAMPLE_INTERVAL, LiveOverlayEvent, Sampler, SharedCaptureTelemetry,
    ShortcutConfig, ShortcutKind, capture_size, clean_label, hotkey_combos, pick_display,
    poll_clicks, scrawl_log, track_shift_ink,
};
use crate::model::{CaptureMode, InteractionEvent, RgbaFrame, TargetInfo};

/// Recordings may hold up to twice a frame's size in points (Retina pixels).
pub const MAX_CAPTURE_SCALE: f64 = 2.0;

/// Where a session's clicks and keys come from, for the log.
pub(super) const MOUSE_SOURCE: &str = "the mouse monitor";
pub(super) const KEY_SOURCE: &str = "the key tap";
pub(super) const NO_KEY_SOURCE: &str = "nowhere (Accessibility off; the record shortcut still works)";

/// `~/Library/Logs/Scrawl`.
pub fn log_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join("Library/Logs/Scrawl"))
}

pub fn main_display_id() -> u32 {
    unsafe { CGMainDisplayID() }
}

/// CGWindowList's windows, front to back, in an autorelease pool.
pub(super) fn raw_windows_now() -> Vec<crate::windows::RawWindow> {
    objc2::rc::autoreleasepool(|_| crate::windows::raw_windows())
}

/// Whether `pid` runs this Scrawl's `scrawl-helper` (the iPhone mirror, Settings, onboarding, the
/// permission panel), going by its executable, so a helper started by another helper counts too.
pub(super) fn is_helper_process(pid: i32) -> bool {
    static HELPER: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    HELPER
        .get_or_init(|| {
            let path = crate::helper::path().ok()?;
            Some(std::fs::canonicalize(&path).unwrap_or(path))
        })
        .as_ref()
        .is_some_and(|helper| process_path(pid).as_ref() == Some(helper))
}

fn process_path(pid: i32) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    let mut buf = vec![0_u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let len = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    let len = usize::try_from(len).ok().filter(|&len| len > 0)?;
    buf.truncate(len);
    Some(PathBuf::from(std::ffi::OsString::from_vec(buf)))
}

type CfTypeRef = *const c_void;
type EventRef = *mut c_void;

const EVENT_LEFT_MOUSE_DOWN: u32 = 1;
const EVENT_LEFT_MOUSE_UP: u32 = 2;
const EVENT_RIGHT_MOUSE_DOWN: u32 = 3;
const EVENT_RIGHT_MOUSE_UP: u32 = 4;
const EVENT_KEY_DOWN: u32 = 10;
const EVENT_SCROLL_WHEEL: u32 = 22;
const EVENT_TAP_DISABLED_BY_TIMEOUT: u32 = u32::MAX - 1;
const EVENT_TAP_DISABLED_BY_USER_INPUT: u32 = u32::MAX;

const KEYBOARD_EVENT_KEYCODE: u32 = 9;
const KEYBOARD_EVENT_AUTOREPEAT: u32 = 8;
const SCROLL_WHEEL_DELTA_AXIS_1: u32 = 11;
const SCROLL_WHEEL_DELTA_AXIS_2: u32 = 12;

const FLAG_SHIFT: u64 = 1 << 17;
const FLAG_CONTROL: u64 = 1 << 18;
const FLAG_ALTERNATE: u64 = 1 << 19;
const FLAG_COMMAND: u64 = 1 << 20;

/// The S key on the ANSI layout (`kVK_ANSI_S`).
pub const KEY_S: u16 = 1;
const KEY_ESCAPE: u16 = 53;
const KEY_Z: u16 = 6;
pub const KEY_V: u16 = 9;

// Carbon HIToolbox global hotkey constants (requires zero TCC permissions).
const K_EVENT_CLASS_KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
const K_EVENT_HOT_KEY_PRESSED: u32 = 5;
const K_EVENT_PARAM_DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");
const TYPE_EVENT_HOT_KEY_ID: u32 = u32::from_be_bytes(*b"hkid");
const HOTKEY_SIG: u32 = u32::from_be_bytes(*b"SCRW");
const HOTKEY_ID_TOGGLE: u32 = 1;
const HOTKEY_ID_ALT1: u32 = 3;
const HOTKEY_ID_PASTE_BASE: u32 = 100;

#[repr(C, packed(2))]
#[derive(Clone, Copy)]
struct EventTypeSpec {
    event_class: u32,
    event_kind: u32,
}

#[repr(C, packed(2))]
#[derive(Clone, Copy)]
struct EventHotKeyID {
    signature: u32,
    id: u32,
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn GetApplicationEventTarget() -> *mut c_void;
    fn GetEventDispatcherTarget() -> *mut c_void;
    fn InstallEventHandler(
        target: *mut c_void,
        handler: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> i32,
        num_types: usize,
        list: *const EventTypeSpec,
        user_data: *mut c_void,
        out_ref: *mut *mut c_void,
    ) -> i32;
    fn RegisterEventHotKey(
        hot_key_code: u32,
        hot_key_modifiers: u32,
        hot_key_id: EventHotKeyID,
        target: *mut c_void,
        options: u32,
        out_ref: *mut *mut c_void,
    ) -> i32;
    fn UnregisterEventHotKey(hot_key: *mut c_void) -> i32;
    fn GetEventParameter(
        event: *mut c_void,
        name: u32,
        desired_type: u32,
        actual_type: *mut u32,
        buffer_size: usize,
        actual_size: *mut usize,
        data: *mut c_void,
    ) -> i32;
    fn TISCopyCurrentKeyboardLayoutInputSource() -> *const c_void;
    fn TISCopyCurrentASCIICapableKeyboardLayoutInputSource() -> *const c_void;
    static kTISPropertyUnicodeKeyLayoutData: *const c_void;
    fn TISGetInputSourceProperty(
        input_source: *const c_void,
        property_key: *const c_void,
    ) -> *const c_void;
    fn UCKeyTranslate(
        layout: *const u8,
        key_code: u16,
        key_action: u16,
        modifier_state: u32,
        keyboard_type: u32,
        options: isize,
        dead_key_state: *mut u32,
        max_length: isize,
        actual_length: *mut isize,
        output: *mut u16,
    ) -> i32;
    fn LMGetKbdType() -> u8;
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
struct CGPoint {
    x: f64,
    y: f64,
}

type EventTapCallback = unsafe extern "C" fn(
    proxy: *mut c_void,
    event_type: u32,
    event: EventRef,
    user_info: *mut c_void,
) -> EventRef;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        events_of_interest: u64,
        callback: EventTapCallback,
        user_info: *mut c_void,
    ) -> *mut c_void;
    fn CGEventTapEnable(tap: *mut c_void, enable: bool);
    fn CGEventTapIsEnabled(tap: *mut c_void) -> bool;
    fn CGEventSourceButtonState(state_id: u32, button: u32) -> bool;
    fn CGEventCreate(source: *const c_void) -> EventRef;
    fn CGEventCreateMouseEvent(
        source: *const c_void,
        mouse_type: u32,
        mouse_cursor_position: CGPoint,
        mouse_button: u32,
    ) -> EventRef;
    fn CGEventCreateKeyboardEvent(
        source: *const c_void,
        virtual_key: u16,
        key_down: bool,
    ) -> EventRef;
    fn CGEventSetFlags(event: EventRef, flags: u64);
    fn CGEventPost(tap: u32, event: EventRef);
    fn CGEventSourceFlagsState(state_id: u32) -> u64;
    fn CGEventGetFlags(event: EventRef) -> u64;
    fn CGEventGetType(event: EventRef) -> u32;
    fn CGEventGetLocation(event: EventRef) -> CGPoint;
    fn CGEventGetIntegerValueField(event: EventRef, field: u32) -> i64;
    fn CGMainDisplayID() -> u32;
    fn CGDisplayPixelsWide(display: u32) -> usize;
    fn CGDisplayPixelsHigh(display: u32) -> usize;
    fn CGPreflightScreenCaptureAccess() -> bool;
    fn CGRequestScreenCaptureAccess() -> bool;
    fn AXIsProcessTrusted() -> bool;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFDataGetBytePtr(data: *const c_void) -> *const u8;
    fn CFMachPortCreateRunLoopSource(
        allocator: *const c_void,
        port: *mut c_void,
        order: isize,
    ) -> *mut c_void;
    fn CFRunLoopAddSource(run_loop: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFRunLoopRemoveSource(run_loop: *mut c_void, source: *mut c_void, mode: *const c_void);
    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopRun();
    fn CFRunLoopStop(run_loop: *mut c_void);
    fn CFRetain(value: CfTypeRef) -> CfTypeRef;
    fn CFRelease(value: CfTypeRef);
    static kCFRunLoopCommonModes: *const c_void;
}

static LAYOUT_KEY_CODES: std::sync::OnceLock<std::collections::HashMap<char, u16>> =
    std::sync::OnceLock::new();
/// The reverse view for naming shortcuts: virtual keycode -> the character ⌘ + that key types.
static LAYOUT_KEY_CHARS: std::sync::OnceLock<std::collections::HashMap<u16, char>> =
    std::sync::OnceLock::new();

/// `UCKeyTranslate`'s modifier state for ⌘ (`cmdKey >> 8`), so a "Dvorak – QWERTY ⌘" layout
/// names ⌘ shortcuts by their QWERTY letters, as macOS does.
const UC_COMMAND_MODIFIER: u32 = 1;

fn translate_keycode(layout: *const u8, key_code: u16) -> Option<char> {
    translate_keycode_with(layout, key_code, 0)
}

fn translate_keycode_with(layout: *const u8, key_code: u16, modifier_state: u32) -> Option<char> {
    if layout.is_null() {
        return None;
    }
    let mut dead_key_state = 0_u32;
    let mut output = [0_u16; 4];
    let mut length = 0_isize;
    let status = unsafe {
        UCKeyTranslate(
            layout,
            key_code,
            3, // kUCKeyActionDisplay
            modifier_state,
            u32::from(LMGetKbdType()),
            0,
            &raw mut dead_key_state,
            output.len() as isize,
            &raw mut length,
            output.as_mut_ptr(),
        )
    };
    if status == 0 && length > 0 {
        String::from_utf16(&output[..length as usize])
            .ok()?
            .chars()
            .next()
            .map(|c| c.to_ascii_lowercase())
    } else {
        None
    }
}

/// Builds the active keyboard layout character -> virtual keycode map (e.g. Dvorak `'s' -> 41`, `'z' -> 44`).
/// Must be called on the main thread during startup before background threads query it.
pub fn init_keyboard_layout_map() -> &'static std::collections::HashMap<char, u16> {
    LAYOUT_KEY_CODES.get_or_init(|| {
        let mut map = std::collections::HashMap::new();
        let mut chars = std::collections::HashMap::new();
        for source in unsafe {
            [
                TISCopyCurrentKeyboardLayoutInputSource(),
                TISCopyCurrentASCIICapableKeyboardLayoutInputSource(),
            ]
        } {
            if source.is_null() {
                continue;
            }
            let data =
                unsafe { TISGetInputSourceProperty(source, kTISPropertyUnicodeKeyLayoutData) };
            if !data.is_null() {
                let bytes = unsafe { CFDataGetBytePtr(data) };
                for code in 0..128_u16 {
                    if let Some(ch) = translate_keycode(bytes, code) {
                        map.entry(ch).or_insert(code);
                    }
                    if let Some(ch) = translate_keycode_with(bytes, code, UC_COMMAND_MODIFIER)
                        .filter(|c| !c.is_control() && !c.is_whitespace())
                    {
                        chars.entry(code).or_insert(ch);
                    }
                }
            }
            unsafe { CFRelease(source) };
        }
        let _ = LAYOUT_KEY_CHARS.set(chars);
        scrawl_log(format!(
            "keyboard layout resolved: 's'={:?} 'z'={:?}",
            map.get(&'s'),
            map.get(&'z')
        ));
        map
    })
}

pub fn key_code_for_char(ch: char) -> Option<u16> {
    init_keyboard_layout_map()
        .get(&ch.to_ascii_lowercase())
        .copied()
}

pub fn layout_s_keycode() -> u16 {
    key_code_for_char('s').unwrap_or(KEY_S)
}

pub fn layout_z_keycode() -> u16 {
    key_code_for_char('z').unwrap_or(KEY_Z)
}

pub fn layout_v_keycode() -> u16 {
    key_code_for_char('v').unwrap_or(KEY_V)
}

/// The registered Carbon hotkeys, so a change can replace all of them.
struct CarbonHotKeyState {
    refs: Vec<usize>,
}

static CARBON_HOTKEYS: std::sync::OnceLock<Mutex<CarbonHotKeyState>> = std::sync::OnceLock::new();

unsafe extern "C" fn carbon_hotkey_handler(
    _next_handler: *mut c_void,
    event: *mut c_void,
    user_data: *mut c_void,
) -> i32 {
    scrawl_log(format!(
        "carbon_hotkey_handler entered: event={event:p} user_data={user_data:p}"
    ));
    if event.is_null() || user_data.is_null() {
        return -1;
    }
    let mut hotkey_id = EventHotKeyID {
        signature: 0,
        id: 0,
    };
    let status = unsafe {
        GetEventParameter(
            event,
            K_EVENT_PARAM_DIRECT_OBJECT,
            TYPE_EVENT_HOT_KEY_ID,
            ptr::null_mut(),
            std::mem::size_of::<EventHotKeyID>(),
            ptr::null_mut(),
            (&raw mut hotkey_id).cast(),
        )
    };
    let sig = hotkey_id.signature;
    let id = hotkey_id.id;
    scrawl_log(format!(
        "carbon_hotkey_handler GetEventParameter status={status} sig={sig:#x} id={id}"
    ));
    if status == 0 && sig == HOTKEY_SIG {
        let telemetry = unsafe { &*user_data.cast::<SharedCaptureTelemetry>() };
        if id >= HOTKEY_ID_PASTE_BASE {
            telemetry.emit_paste_shortcut();
        } else {
            telemetry.emit_capture_shortcut();
        }
    }
    0
}

/// Installs Carbon `RegisterEventHotKey` on both `GetEventDispatcherTarget()` and
/// `GetApplicationEventTarget()`. Works system-wide across all apps with zero TCC permissions.
pub fn install_global_hotkeys(telemetry: &Arc<SharedCaptureTelemetry>) {
    let _ = init_keyboard_layout_map();
    let s_code = u32::from(layout_s_keycode());
    let state_mutex = CARBON_HOTKEYS.get_or_init(|| {
        let spec = EventTypeSpec {
            event_class: K_EVENT_CLASS_KEYBOARD,
            event_kind: K_EVENT_HOT_KEY_PRESSED,
        };
        let user_data = Arc::into_raw(Arc::clone(telemetry))
            .cast_mut()
            .cast::<c_void>();
        unsafe {
            let mut handler_ref: *mut c_void = ptr::null_mut();
            let dispatcher = GetEventDispatcherTarget();
            let app_target = GetApplicationEventTarget();
            scrawl_log(format!(
                "install_global_hotkeys: dispatcher={dispatcher:p} app_target={app_target:p} s_code={s_code}"
            ));
            if !dispatcher.is_null() {
                let rc = InstallEventHandler(
                    dispatcher,
                    carbon_hotkey_handler,
                    1,
                    &raw const spec,
                    user_data,
                    &raw mut handler_ref,
                );
                scrawl_log(format!(
                    "InstallEventHandler(dispatcher) -> rc={rc} handler_ref={handler_ref:p}"
                ));
            }
            if !app_target.is_null() && app_target != dispatcher {
                let mut app_handler_ref: *mut c_void = ptr::null_mut();
                let rc = InstallEventHandler(
                    app_target,
                    carbon_hotkey_handler,
                    1,
                    &raw const spec,
                    user_data,
                    &raw mut app_handler_ref,
                );
                scrawl_log(format!(
                    "InstallEventHandler(app_target) -> rc={rc} handler_ref={app_handler_ref:p}"
                ));
            }
        }
        Mutex::new(CarbonHotKeyState { refs: Vec::new() })
    });
    let (record, paste) = telemetry.shortcuts();
    if let Err(kind) = register_shortcuts(state_mutex, &record, &paste) {
        scrawl_log(format!(
            "install_global_hotkeys: {kind:?} shortcut could not be registered"
        ));
    }
}

fn hotkey_target() -> *mut c_void {
    unsafe {
        let a = GetApplicationEventTarget();
        if a.is_null() {
            GetEventDispatcherTarget()
        } else {
            a
        }
    }
}

fn unregister_all(state: &mut CarbonHotKeyState) {
    for r in state.refs.drain(..) {
        unsafe { UnregisterEventHotKey(r as *mut c_void) };
    }
}

/// Replaces every registered hotkey with `record`'s and `paste`'s. Fails with the shortcut whose
/// own combo macOS refused (another app may hold it); its aliases are best effort.
fn register_shortcuts(
    state_mutex: &Mutex<CarbonHotKeyState>,
    record: &ShortcutConfig,
    paste: &ShortcutConfig,
) -> Result<(), ShortcutKind> {
    let mut state = state_mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    unregister_all(&mut state);
    let target = hotkey_target();
    let mut failed = None;
    for (kind, cfg) in [(ShortcutKind::Record, record), (ShortcutKind::Paste, paste)] {
        for (i, (code, mods)) in hotkey_combos(kind, cfg).into_iter().enumerate() {
            let mut r: *mut c_void = ptr::null_mut();
            let id = match (kind, i) {
                (ShortcutKind::Record, 0) => HOTKEY_ID_TOGGLE,
                (ShortcutKind::Record, i) => HOTKEY_ID_ALT1 + i as u32,
                (ShortcutKind::Paste, i) => HOTKEY_ID_PASTE_BASE + i as u32,
            };
            let rc = unsafe {
                RegisterEventHotKey(
                    code,
                    mods,
                    EventHotKeyID {
                        signature: HOTKEY_SIG,
                        id,
                    },
                    target,
                    0,
                    &raw mut r,
                )
            };
            scrawl_log(format!(
                "RegisterEventHotKey({kind:?} {}, code={code}, mods={mods}) -> rc={rc} ref={r:p}",
                if i == 0 { cfg.label.as_str() } else { "alias" }
            ));
            if rc == 0 && !r.is_null() {
                state.refs.push(r as usize);
            } else if i == 0 && failed.is_none() {
                failed = Some(kind);
            }
        }
    }
    failed.map_or(Ok(()), Err)
}

/// Sets both shortcuts and registers them. If macOS refuses either, the old ones are put back
/// and the refused one is returned. Nothing is saved here (`crate::settings` does that).
pub fn set_shortcuts(
    telemetry: &SharedCaptureTelemetry,
    record: ShortcutConfig,
    paste: ShortcutConfig,
) -> Result<(), ShortcutKind> {
    let (old_record, old_paste) = telemetry.shortcuts();
    if let Some(state) = CARBON_HOTKEYS.get()
        && !telemetry.shortcuts_paused.load(Ordering::Acquire)
        && let Err(kind) = register_shortcuts(state, &record, &paste)
    {
        let _ = register_shortcuts(state, &old_record, &old_paste);
        return Err(kind);
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
    let Some(state) = CARBON_HOTKEYS.get() else {
        return;
    };
    if paused {
        unregister_all(
            &mut state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
    } else {
        let (record, paste) = telemetry.shortcuts();
        let _ = register_shortcuts(state, &record, &paste);
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

/// Whether this running copy of Scrawl may record the screen, by `CGPreflightScreenCaptureAccess`,
/// which keeps its first answer for the life of the process. See `screen_capture_usable`.
pub fn screen_capture_granted() -> bool {
    unsafe { CGPreflightScreenCaptureAccess() }
}

/// A ScreenCaptureKit capture worked in this process (`screen_capture_probe`).
static SCREEN_PROBE_OK: AtomicBool = AtomicBool::new(false);
static SCREEN_PROBE_AT: Mutex<Option<Instant>> = Mutex::new(None);

/// Whether this copy of Scrawl can capture the screen now: `CGPreflightScreenCaptureAccess`, or
/// a ScreenCaptureKit capture that already worked here. With `probe` (only once a fresh helper
/// says Screen Recording is granted, so no prompt can come of it, and never on the UI thread) a
/// stale preflight is checked with a real one-frame capture, at most every 2 s. Whether a grant
/// reaches a running process this way on macOS 26 and 27 is what the probe finds out; when it
/// does, Scrawl doesn't relaunch.
pub fn screen_capture_usable(probe: bool) -> bool {
    if screen_capture_granted() || SCREEN_PROBE_OK.load(Ordering::Acquire) {
        return true;
    }
    if !probe {
        return false;
    }
    {
        let mut at = SCREEN_PROBE_AT
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if at.is_some_and(|t| t.elapsed() < Duration::from_secs(2)) {
            return false;
        }
        *at = Some(Instant::now());
    }
    let started = Instant::now();
    let result = screen_capture_probe();
    scrawl_log(format!(
        "screen probe after a grant, without relaunching: {} in {} ms",
        match &result {
            Ok((w, h)) => format!("captured {w}x{h}"),
            Err(error) => format!("failed ({error})"),
        },
        started.elapsed().as_millis()
    ));
    let ok = result.is_ok();
    if ok {
        SCREEN_PROBE_OK.store(true, Ordering::Release);
    }
    ok
}

/// One small ScreenCaptureKit screenshot of the first display: `SCShareableContent`, then
/// `SCScreenshotManager`. Returns the image size. Asks for Screen Recording if it was never
/// decided, so only call it once it's known to be granted (or from `scrawl sck-probe`).
pub fn screen_capture_probe() -> Result<(u32, u32), String> {
    let content = SCShareableContent::get().map_err(|e| format!("SCShareableContent: {e:?}"))?;
    let display = content
        .displays()
        .into_iter()
        .next()
        .ok_or("SCShareableContent: no displays")?;
    let filter = SCContentFilter::create()
        .with_display(&display)
        .with_excluding_windows(&[] as &[&SCWindow])
        .build()
        .map_err(|e| format!("SCContentFilter: {e:?}"))?;
    let config = SCStreamConfiguration::new().with_width(64).with_height(40);
    let image = SCScreenshotManager::capture_image(&filter, &config)
        .map_err(|e| format!("SCScreenshotManager: {e:?}"))?;
    let (w, h) = (image.width() as u32, image.height() as u32);
    if w == 0 || h == 0 {
        return Err("SCScreenshotManager: empty image".into());
    }
    Ok((w, h))
}

/// The system prompt for Screen Recording (once; later calls do nothing).
pub fn request_screen_capture() {
    unsafe {
        let _ = CGRequestScreenCaptureAccess();
    }
}

/// Whether ⌥⇧V may post ⌘V into the app in front (System Settings › Accessibility). Unlike
/// Screen Recording, a grant shows up here while Scrawl runs.
pub fn accessibility_granted() -> bool {
    unsafe { AXIsProcessTrusted() }
}

/// The ⌃⌥⇧⌘ modifier flags held right now (as `CGEventFlags` bits). Reading modifier state
/// needs no permission (unlike key codes, which are Input Monitoring's).
pub fn modifier_flags() -> u64 {
    let flags = unsafe { CGEventSourceFlagsState(0) | CGEventSourceFlagsState(1) };
    flags & (FLAG_COMMAND | FLAG_CONTROL | FLAG_ALTERNATE | FLAG_SHIFT)
}

pub fn shortcut_from_key_and_mods(
    key: &str,
    cmd: bool,
    ctrl: bool,
    alt: bool,
    shift: bool,
) -> Option<ShortcutConfig> {
    let lower = key.to_ascii_lowercase();
    let (key_code, key_label): (u32, String) = if lower.chars().count() == 1
        && let Some(ch) = lower.chars().next()
        && ch.is_ascii_alphanumeric()
        && let Some(code) = key_code_for_char(ch)
    {
        (u32::from(code), ch.to_ascii_uppercase().to_string())
    } else {
        let (code, lbl) = match lower.as_str() {
            "a" => (0, "A"),
            "s" => (u32::from(layout_s_keycode()), "S"),
            "d" => (2, "D"),
            "f" => (3, "F"),
            "h" => (4, "H"),
            "g" => (5, "G"),
            "z" => (u32::from(layout_z_keycode()), "Z"),
            "x" => (7, "X"),
            "c" => (8, "C"),
            "v" => (9, "V"),
            "b" => (11, "B"),
            "q" => (12, "Q"),
            "w" => (13, "W"),
            "e" => (14, "E"),
            "r" => (15, "R"),
            "y" => (16, "Y"),
            "t" => (17, "T"),
            "1" => (18, "1"),
            "2" => (19, "2"),
            "3" => (20, "3"),
            "4" => (21, "4"),
            "6" => (22, "6"),
            "5" => (23, "5"),
            "9" => (25, "9"),
            "7" => (26, "7"),
            "8" => (28, "8"),
            "0" => (29, "0"),
            "o" => (31, "O"),
            "u" => (32, "U"),
            "i" => (34, "I"),
            "p" => (35, "P"),
            "l" => (37, "L"),
            "j" => (38, "J"),
            "k" => (40, "K"),
            "n" => (45, "N"),
            "m" => (46, "M"),
            "space" | " " => (49, "Space"),
            "f1" => (122, "F1"),
            "f2" => (120, "F2"),
            "f3" => (99, "F3"),
            "f4" => (118, "F4"),
            "f5" => (96, "F5"),
            "f6" => (97, "F6"),
            "f7" => (98, "F7"),
            "f8" => (100, "F8"),
            "f9" => (101, "F9"),
            "f10" => (109, "F10"),
            "f11" => (103, "F11"),
            "f12" => (111, "F12"),
            _ => return None,
        };
        (code, lbl.to_string())
    };
    let is_fn_key = key_label.starts_with('F') && key_label.len() >= 2;
    if !cmd && !ctrl && !alt && !is_fn_key {
        return None;
    }
    let mut carbon_modifiers = 0_u32;
    let mut label = String::new();
    if ctrl {
        carbon_modifiers |= CARBON_CONTROL_KEY;
        label.push('⌃');
    }
    if alt {
        carbon_modifiers |= CARBON_OPTION_KEY;
        label.push('⌥');
    }
    if shift {
        carbon_modifiers |= CARBON_SHIFT_KEY;
        label.push('⇧');
    }
    if cmd {
        carbon_modifiers |= CARBON_CMD_KEY;
        label.push('⌘');
    }
    label.push_str(&key_label);
    Some(ShortcutConfig {
        key_code,
        carbon_modifiers,
        label,
    })
}

/// Discovers the current capture target (either a specific window/app or the main display).
pub fn discover_target(window_query: Option<&str>) -> TargetInfo {
    discover_target_at(window_query, current_mouse_location().map(|p| [p.x, p.y]))
}

/// Like `discover_target`, recording the display under `point` (see `pick_display`).
pub fn discover_target_at(window_query: Option<&str>, point: Option<[f64; 2]>) -> TargetInfo {
    let frontmost = frontmost_app_info();
    let screens = crate::screens::displays();
    if let Ok(content) = SCShareableContent::get() {
        let our_pid = std::process::id() as i32;
        if let Some(query) = window_query {
            let q = query.to_lowercase();
            if let Some(win) = content.windows().into_iter().find(|w| {
                w.is_on_screen()
                    && w.window_layer() == 0
                    && w.owning_application()
                        .as_ref()
                        .is_none_or(|a| a.process_id() != our_pid)
                    && (w
                        .title()
                        .as_deref()
                        .is_some_and(|t| t.to_lowercase().contains(&q))
                        || w.owning_application()
                            .as_ref()
                            .is_some_and(|a| a.application_name().to_lowercase().contains(&q)))
            }) {
                let f = win.frame();
                let rect = [f.origin.x, f.origin.y, f.size.width, f.size.height];
                let owner = win.owning_application();
                let (display_id, scale) = display_of(&screens, rect);
                return TargetInfo {
                    mode: CaptureMode::Window,
                    app: owner.as_ref().map(|a| a.application_name()),
                    bundle_id: owner.as_ref().map(|a| a.bundle_identifier()),
                    window_title: win.title(),
                    window_id: Some(win.window_id()),
                    display_id,
                    frame: crate::screens::with_scale(rect, scale),
                };
            }
        }

        let displays = content.displays();
        let front_window = frontmost.as_ref().and_then(|(pid, _, _)| {
            content.windows().into_iter().find(|w| {
                w.is_on_screen()
                    && w.window_layer() == 0
                    && w.owning_application()
                        .as_ref()
                        .is_some_and(|a| a.process_id() == *pid)
            })
        });
        let rects: Vec<[f64; 4]> = displays
            .iter()
            .map(|d| {
                let f = d.frame();
                [f.origin.x, f.origin.y, f.size.width, f.size.height]
            })
            .collect();
        let focused = front_window.as_ref().map(|w| {
            let f = w.frame();
            [f.origin.x, f.origin.y, f.size.width, f.size.height]
        });
        if let Some(display) = displays.get(pick_display(&rects, point, focused)) {
            let f = display.frame();
            let rect = [f.origin.x, f.origin.y, f.size.width, f.size.height];
            // The title of the window in front on the recorded display. The window server's
            // list is in z-order; `SCShareableContent::windows` is not, so with several
            // windows of one app `front_window` can be any of them.
            let window_title = frontmost
                .as_ref()
                .and_then(|(pid, _, _)| {
                    let raw = crate::windows::raw_windows();
                    crate::windows::frontmost_window_of(&raw, *pid, rect).map(|w| w.title.clone())
                })
                .or_else(|| front_window.as_ref().and_then(|w| w.title()))
                .filter(|t| !t.is_empty());
            let scale = screens
                .iter()
                .find(|s| s.id == display.display_id())
                .map_or_else(|| display_of(&screens, rect).1, |s| s.scale);
            return TargetInfo {
                mode: CaptureMode::Display,
                app: frontmost.as_ref().map(|(_, name, _)| name.clone()),
                bundle_id: frontmost.and_then(|(_, _, bid)| bid),
                window_title,
                window_id: None,
                display_id: display.display_id(),
                frame: crate::screens::with_scale(rect, scale),
            };
        }
    }

    // No ScreenCaptureKit: the display under `point` from CoreGraphics.
    let rects: Vec<[f64; 4]> = screens.iter().map(|s| s.frame).collect();
    let (display_id, frame) = screens.get(pick_display(&rects, point, None)).map_or_else(
        || {
            let id = unsafe { CGMainDisplayID() };
            let w = (unsafe { CGDisplayPixelsWide(id) } as f64).max(1440.0);
            let h = (unsafe { CGDisplayPixelsHigh(id) } as f64).max(900.0);
            (id, crate::screens::with_scale([0.0, 0.0, w, h], 2.0))
        },
        |s| (s.id, crate::screens::with_scale(s.frame, s.scale)),
    );
    TargetInfo {
        mode: CaptureMode::Display,
        app: frontmost.as_ref().map(|(_, name, _)| name.clone()),
        bundle_id: frontmost.and_then(|(_, _, bid)| bid),
        window_title: None,
        window_id: None,
        display_id,
        frame,
    }
}

/// The display a global rect is on (its centre, else most of it) and that display's scale.
fn display_of(screens: &[crate::screens::Screen], rect: [f64; 4]) -> (u32, f64) {
    crate::screens::index_for_rect(screens, rect).map_or_else(
        || (unsafe { CGMainDisplayID() }, 2.0),
        |i| (screens[i].id, screens[i].scale),
    )
}

/// Lists all on-screen normal (`window_layer == 0`) windows in front-to-back order for the Window picker.
pub fn list_selectable_windows() -> Vec<TargetInfo> {
    let Ok(content) = SCShareableContent::get() else {
        return Vec::new();
    };
    let our_pid = std::process::id() as i32;
    let screens = crate::screens::displays();

    content
        .windows()
        .into_iter()
        .filter(|w| {
            let f = w.frame();
            w.is_on_screen()
                && w.window_layer() == 0
                && f.size.width >= 80.0
                && f.size.height >= 80.0
                && w.owning_application()
                    .as_ref()
                    .is_some_and(|a| a.process_id() != our_pid)
        })
        .map(|w| {
            let f = w.frame();
            let rect = [f.origin.x, f.origin.y, f.size.width, f.size.height];
            let (display_id, scale) = display_of(&screens, rect);
            let owner = w.owning_application();
            TargetInfo {
                mode: CaptureMode::Window,
                app: owner
                    .as_ref()
                    .map(|a| clean_label(&a.application_name()))
                    .filter(|s| !s.is_empty()),
                bundle_id: owner.as_ref().map(|a| a.bundle_identifier()),
                window_title: w.title().map(|t| clean_label(&t)).filter(|t| !t.is_empty()),
                window_id: Some(w.window_id()),
                display_id,
                frame: crate::screens::with_scale(rect, scale),
            }
        })
        .collect()
}

static LAST_EXTERNAL_APP_PID: AtomicI32 = AtomicI32::new(0);

pub fn record_external_frontmost_app() {
    use objc2_app_kit::NSWorkspace;
    let workspace = NSWorkspace::sharedWorkspace();
    if let Some(app) = workspace.frontmostApplication() {
        let pid = app.processIdentifier();
        if pid > 0 && pid != std::process::id() as i32 {
            LAST_EXTERNAL_APP_PID.store(pid, Ordering::Release);
        }
    }
}

pub fn ensure_external_app_active() {
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication, NSWorkspace};
    let our_pid = std::process::id() as i32;
    let workspace = NSWorkspace::sharedWorkspace();
    let front_pid = workspace
        .frontmostApplication()
        .map_or(0, |a| a.processIdentifier());
    if front_pid != 0 && front_pid != our_pid {
        LAST_EXTERNAL_APP_PID.store(front_pid, Ordering::Release);
        return;
    }
    let target_pid = LAST_EXTERNAL_APP_PID.load(Ordering::Acquire);
    if target_pid > 0
        && target_pid != our_pid
        && let Some(target_app) =
            NSRunningApplication::runningApplicationWithProcessIdentifier(target_pid)
    {
        #[allow(deprecated)]
        let _ = target_app
            .activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps);
        let deadline = Instant::now() + Duration::from_millis(220);
        while Instant::now() < deadline {
            if workspace
                .frontmostApplication()
                .is_some_and(|a| a.processIdentifier() == target_pid)
            {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

/// The frontmost app's pid, or the last external one while Scrawl itself is in front.
pub fn frontmost_external_pid() -> Option<i32> {
    use objc2_app_kit::NSWorkspace;
    let our_pid = std::process::id() as i32;
    let pid = NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .map_or(0, |a| a.processIdentifier());
    if pid > 0 && pid != our_pid {
        return Some(pid);
    }
    let fallback = LAST_EXTERNAL_APP_PID.load(Ordering::Acquire);
    (fallback > 0 && fallback != our_pid).then_some(fallback)
}

pub fn frontmost_app_info() -> Option<(i32, String, Option<String>)> {
    use objc2_app_kit::{NSRunningApplication, NSWorkspace};
    let our_pid = std::process::id() as i32;
    let workspace = NSWorkspace::sharedWorkspace();
    if let Some(app) = workspace.frontmostApplication() {
        let pid = app.processIdentifier();
        if pid != our_pid {
            LAST_EXTERNAL_APP_PID.store(pid, Ordering::Release);
            let name = app.localizedName()?.to_string();
            let bundle_id = app.bundleIdentifier().map(|b| b.to_string());
            return Some((pid, name, bundle_id));
        }
    }
    let fallback_pid = LAST_EXTERNAL_APP_PID.load(Ordering::Acquire);
    if fallback_pid > 0 && fallback_pid != our_pid {
        let app = NSRunningApplication::runningApplicationWithProcessIdentifier(fallback_pid)?;
        let name = app.localizedName()?.to_string();
        let bundle_id = app.bundleIdentifier().map(|b| b.to_string());
        return Some((fallback_pid, name, bundle_id));
    }
    None
}

/// The ScreenCaptureKit sampler: the recording stream writes `recording.mp4` while candidate
/// screenshots are taken every `buffer.interval()` (or at once when urgent).
pub(super) fn run_sampler(sampler: Sampler) -> Option<PathBuf> {
    let Sampler {
        target: sampler_target,
        video_output_path,
        excluded_window_ids,
        started_at,
        stopping: thread_stopping,
        urgent: thread_urgent,
        frames: thread_frames,
        video_offset_ms: thread_offset,
    } = sampler;
    let initial_excluded = excluded_window_ids
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    // Keep SCK initialization, capture and teardown on the owning worker.
    // stop() joins even if it arrives while setup is still in progress.
    let (stream, recording_output, video_path, filter) = setup_sck_capture(
        &sampler_target,
        video_output_path,
        &initial_excluded,
        started_at,
        &thread_offset,
    );
    crate::ui::start_metric("sck_ready_ms", started_at);
    let shot_edge = if video_path.is_some() {
        CANDIDATE_EDGE
    } else {
        CANDIDATE_EDGE_NO_VIDEO
    };
    let shot_config = sck_config(&sampler_target, shot_edge);
    let mut filter_and_config = filter.map(|f| (f, shot_config));
    let mut last_sample = Instant::now()
        .checked_sub(FRAME_SAMPLE_INTERVAL)
        .unwrap_or_else(Instant::now);
    let mut phase = 0_u32;
    let mut last_excluded = initial_excluded.clone();
    let mut prompts = SystemPrompts::new(&sampler_target);
    while !thread_stopping.load(Ordering::Acquire) {
        let mut cur_excluded = excluded_window_ids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        if prompts.check(started_at) {
            // Frames since the last look may show the prompt.
            thread_frames
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .drop_since(prompts.clear_until_ms);
        }
        cur_excluded.extend(&prompts.ids);
        if cur_excluded != last_excluded {
            last_excluded.clone_from(&cur_excluded);
            prompts.hidden = sck_lists_windows(&prompts.ids);
            if let Some((new_filter, _)) =
                build_sck_filter_and_config(&sampler_target, &cur_excluded)
            {
                // The recording must hide the same windows as the candidates, or
                // keyframes decoded from it could show Scrawl's own overlays.
                if let Some(ref stream) = stream
                    && let Err(error) = stream.update_content_filter(&new_filter)
                {
                    scrawl_log(format!("Recording filter update failed: {error:?}"));
                }
                filter_and_config = Some((new_filter, sck_config(&sampler_target, shot_edge)));
            }
        }
        let urgent = thread_urgent.swap(false, Ordering::AcqRel);
        let interval = thread_frames
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .interval();
        // A system prompt ScreenCaptureKit can't leave out: no candidates until
        // it's answered.
        if prompts.showing() {
            thread::park_timeout(Duration::from_millis(30));
            continue;
        }
        if urgent || last_sample.elapsed() >= interval {
            last_sample = Instant::now();
            let t_ms = started_at.elapsed().as_millis() as u64;
            if urgent {
                phase = phase.saturating_add(1);
            }
            let frame = if let Some((ref filter, ref config)) = filter_and_config {
                capture_sck_frame(filter, config, t_ms).unwrap_or_else(|| {
                    crate::render::render_synthetic_demo_frame(1280, 800, t_ms, phase)
                })
            } else {
                crate::render::render_synthetic_demo_frame(1280, 800, t_ms, phase)
            };
            thread_frames
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(frame, urgent);
        }
        thread::park_timeout(Duration::from_millis(30));
    }
    if let Some(stream) = stream {
        let _ = stream.stop_capture();
        if let Some(ref output) = recording_output {
            let _ = stream.remove_recording_output(output);
        }
    }
    video_path
}

fn build_sck_filter_and_config(
    target: &TargetInfo,
    excluded_window_ids: &[u32],
) -> Option<(SCContentFilter, SCStreamConfiguration)> {
    let content = match SCShareableContent::get() {
        Ok(c) => c,
        Err(err) => {
            // Screen Recording is asked for in onboarding, never mid-capture.
            scrawl_log(format!(
                "SCShareableContent::get() failed: {err:?} (screen recording granted: {})",
                screen_capture_granted()
            ));
            return None;
        }
    };
    let our_pid = std::process::id() as i32;

    let filter = if let Some(window_id) = target.window_id
        && let Some(window) = content
            .windows()
            .into_iter()
            .find(|w| w.window_id() == window_id)
    {
        SCContentFilter::create().with_window(&window).build().ok()
    } else if let Some(display) = content
        .displays()
        .into_iter()
        .find(|d| d.display_id() == target.display_id)
        .or_else(|| content.displays().into_iter().next())
    {
        // Exclude Scrawl's floating HUD pill and Metal overlay windows by explicit window ID
        // (and any non-layer-0 Scrawl panels), while keeping Scrawl's normal Review window visible.
        let excluded_overlays: Vec<SCWindow> = content
            .windows()
            .into_iter()
            .filter(|w| {
                excluded_window_ids.contains(&w.window_id())
                    || (w
                        .owning_application()
                        .as_ref()
                        .is_some_and(|a| a.process_id() == our_pid)
                        && w.window_layer() != 0)
            })
            .collect();
        let excluded_refs: Vec<&SCWindow> = excluded_overlays.iter().collect();
        SCContentFilter::create()
            .with_display(&display)
            .with_excluding_windows(&excluded_refs)
            .build()
            .ok()
    } else {
        None
    }?;

    // Full-screen captures get auto-cropped to a window or region afterwards (`scrawl autocrop`),
    // so they need enough pixels for a crop to a quarter of the screen to stay sharp.
    let long_edge = if target.mode == CaptureMode::Display {
        2560.0
    } else {
        1280.0
    };
    Some((filter, sck_config(target, long_edge)))
}

fn sck_config(target: &TargetInfo, long_edge: f64) -> SCStreamConfiguration {
    let (width, height) = capture_size(target, long_edge);
    let mut config = SCStreamConfiguration::new()
        .with_width(width)
        .with_height(height)
        .with_shows_cursor(true);
    if target.mode == CaptureMode::Region {
        // `sourceRect` is in the display's own points, not global ones.
        let b = core_graphics::display::CGDisplay::new(target.display_id).bounds();
        let display = [b.origin.x, b.origin.y, b.size.width, b.size.height];
        let area = [
            target.frame.x,
            target.frame.y,
            target.frame.width,
            target.frame.height,
        ];
        let [x, y, w, h] = if display[2] > 0.0 && display[3] > 0.0 {
            crate::screens::source_rect(area, display)
        } else {
            area
        };
        config.set_source_rect(CGRect::new(x, y, w, h));
    }
    config
}

/// Starts the recording stream; returns it with the filter for candidate screenshots. Stores
/// when the recording started (ms since `started_at`) into `video_offset_ms`.
fn setup_sck_capture(
    target: &TargetInfo,
    video_output_path: Option<PathBuf>,
    excluded_window_ids: &[u32],
    started_at: Instant,
    video_offset_ms: &AtomicU64,
) -> (
    Option<SCStream>,
    Option<SCRecordingOutput>,
    Option<PathBuf>,
    Option<SCContentFilter>,
) {
    let Some((filter, config)) = build_sck_filter_and_config(target, excluded_window_ids) else {
        return (None, None, None, None);
    };

    if let Some(video_path) = video_output_path {
        if let Ok(rec_cfg_builder) = SCRecordingOutputConfiguration::new()
            && let Ok(rec_cfg) = rec_cfg_builder.with_output_url(&video_path).map(|c| {
                c.with_video_codec(SCRecordingOutputCodec::H264)
                    .with_output_file_type(SCRecordingOutputFileType::MP4)
            })
            && let Some(rec_output) = SCRecordingOutput::new(&rec_cfg)
            && let Ok(stream) = SCStream::new(&filter, &config)
            && stream.add_recording_output(&rec_output).is_ok()
        {
            // The movie's clock starts while `start_capture` runs; its midpoint is the closest
            // estimate this side of the recording delegate.
            let before = started_at.elapsed();
            if stream.start_capture().is_ok() {
                let after = started_at.elapsed();
                video_offset_ms.store(((before + after) / 2).as_millis() as u64, Ordering::Release);
                return (
                    Some(stream),
                    Some(rec_output),
                    Some(video_path),
                    Some(filter),
                );
            }
        }
    }

    (None, None, None, Some(filter))
}

fn capture_sck_frame(
    filter: &SCContentFilter,
    config: &SCStreamConfiguration,
    t_ms: u64,
) -> Option<RgbaFrame> {
    let image = SCScreenshotManager::capture_image(filter, config).ok()?;
    let width = u32::try_from(image.width()).ok()?;
    let height = u32::try_from(image.height()).ok()?;
    if width == 0 || height == 0 {
        return None;
    }
    let rgba = image.rgba_data().ok()?;
    if rgba.len() != (width as usize) * (height as usize) * 4 {
        return None;
    }
    Some(RgbaFrame::new(t_ms, width, height, rgba))
}

/// macOS prompts over a capture (`windows::system_prompts_over`): ScreenCaptureKit's "bypass
/// the system private window picker" question shows a second or two after the first capture
/// starts. They're left out of the recording when ScreenCaptureKit lists them, and frames that
/// could show one are never candidates, so a prompt doesn't become a keyframe.
struct SystemPrompts {
    frame: Option<crate::model::TargetFrame>,
    ids: Vec<u32>,
    /// Which of `ids` the content filter leaves out.
    hidden: Vec<u32>,
    checked: Option<Instant>,
    /// The capture time of the last look that found nothing new.
    clear_until_ms: u64,
}

impl SystemPrompts {
    fn new(target: &TargetInfo) -> Self {
        Self {
            // A single window's capture never shows a prompt.
            frame: target.window_id.is_none().then_some(target.frame),
            ids: Vec::new(),
            hidden: Vec::new(),
            checked: None,
            clear_until_ms: 0,
        }
    }

    /// Looks again (four times a second for the first 10 s, then every second). Returns whether
    /// a prompt appeared since the last look.
    fn check(&mut self, started_at: Instant) -> bool {
        let Some(frame) = self.frame else {
            return false;
        };
        let elapsed = started_at.elapsed();
        let every = if elapsed < Duration::from_secs(10) {
            Duration::from_millis(250)
        } else {
            Duration::from_secs(1)
        };
        if self.checked.is_some_and(|at| at.elapsed() < every) {
            return false;
        }
        self.checked = Some(Instant::now());
        let raw = objc2::rc::autoreleasepool(|_| crate::windows::raw_windows());
        let ids = crate::windows::system_prompts_over(&raw, frame);
        let appeared = ids.iter().any(|id| !self.ids.contains(id));
        if appeared {
            scrawl_log(format!("system prompt over the capture: windows {ids:?}"));
        } else {
            self.clear_until_ms = elapsed.as_millis() as u64;
        }
        self.ids = ids;
        appeared
    }

    /// A prompt is on screen that the recording can't leave out.
    fn showing(&self) -> bool {
        self.ids.iter().any(|id| !self.hidden.contains(id))
    }
}

/// Which of `ids` ScreenCaptureKit lists (and so can leave out of a recording).
fn sck_lists_windows(ids: &[u32]) -> Vec<u32> {
    if ids.is_empty() {
        return Vec::new();
    }
    SCShareableContent::get()
        .map(|content| {
            content
                .windows()
                .into_iter()
                .map(|w| w.window_id())
                .filter(|id| ids.contains(id))
                .collect()
        })
        .unwrap_or_default()
}


/// Keeps the key event tap in step with Accessibility, which macOS reports live: makes the tap
/// once it's on (no relaunch), stops it if it's turned off (an active tap whose grant is gone can
/// hold up input), and re-enables it if macOS turned it off. Returns whether it is delivering.
/// Called every 500 ms from the HID poller.
fn check_key_tap(telemetry: &Arc<SharedCaptureTelemetry>) -> bool {
    let trusted = accessibility_granted();
    let tap = telemetry.tap_port.load(Ordering::Acquire) as *mut c_void;
    if !trusted && !tap.is_null() {
        stop_key_tap(telemetry);
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    if trusted
        && tap.is_null()
        && now_ms.saturating_sub(telemetry.tap_tried_ms.load(Ordering::Acquire)) >= 2_000
        && !telemetry.tap_starting.swap(true, Ordering::AcqRel)
    {
        telemetry.tap_tried_ms.store(now_ms, Ordering::Release);
        spawn_key_tap(Arc::clone(telemetry));
    }
    let live = trusted
        && !tap.is_null()
        && unsafe {
            CGEventTapIsEnabled(tap) || {
                CGEventTapEnable(tap, true);
                CGEventTapIsEnabled(tap)
            }
        };
    if telemetry.tap_live.swap(live, Ordering::AcqRel) != live {
        scrawl_log(if live {
            "key tap live"
        } else if !trusted {
            "key tap off: Accessibility is off"
        } else {
            "key tap off (being made, or Secure Input is on)"
        });
    }
    live
}

/// Starts watching input across every app, with no Input Monitoring:
/// - clicks, drags and scrolls from an AppKit global mouse monitor, which needs no permission
///   (only key events need Accessibility, per `addGlobalMonitorForEvents`' documentation);
/// - ⇧ (ink), the laser and the shortcut's held modifiers from polled modifier flags, which need
///   no permission either (the HID poller);
/// - key presses (Esc, ⌘Z, ⌘-chords for the window log) from an active key
///   event tap, which Accessibility allows and which is made, without a relaunch, as soon as
///   Accessibility is on (`check_key_tap`).
///
/// ⌥⇧S and ⌥⇧V themselves are Carbon hotkeys and need nothing. Call on the main thread.
pub fn start_global_input_tap(telemetry: Arc<SharedCaptureTelemetry>) {
    let live = install_mouse_monitor(Arc::clone(&telemetry));
    telemetry.mouse_live.store(live, Ordering::Release);
    scrawl_log(format!(
        "mouse monitor {}",
        if live {
            "installed"
        } else {
            "unavailable; polling the mouse buttons"
        }
    ));
    let _ = thread::Builder::new()
        .name("scrawl-hid-poller".into())
        .spawn(move || run_global_hid_poller(telemetry));
}

/// `struct __CGEvent`, so `-[NSEvent CGEvent]` passes objc2's encoding check.
#[repr(C)]
struct OpaqueCgEvent {
    _private: [u8; 0],
}

unsafe impl objc2::encode::RefEncode for OpaqueCgEvent {
    const ENCODING_REF: objc2::encode::Encoding =
        objc2::encode::Encoding::Pointer(&objc2::encode::Encoding::Struct("__CGEvent", &[]));
}

const MOUSE_MONITOR_MASK: u64 = (1 << EVENT_LEFT_MOUSE_DOWN)
    | (1 << EVENT_LEFT_MOUSE_UP)
    | (1 << EVENT_RIGHT_MOUSE_DOWN)
    | (1 << EVENT_RIGHT_MOUSE_UP)
    | (1 << EVENT_SCROLL_WHEEL);

/// `+[NSEvent addGlobalMonitorForEventsMatchingMask:handler:]` for mouse buttons and scrolls.
/// AppKit sends it events bound for other apps only, so clicks on Scrawl's own windows (the stop
/// control, the HUD) never arrive; those on `scrawl-helper`'s windows do, and `mouse_event` drops
/// them. The monitor lives as long as the app.
fn install_mouse_monitor(telemetry: Arc<SharedCaptureTelemetry>) -> bool {
    use objc2::msg_send;
    use objc2::runtime::{AnyClass, AnyObject};
    let Some(class) = AnyClass::get(c"NSEvent") else {
        return false;
    };
    let block = block2::RcBlock::new(move |event: ptr::NonNull<AnyObject>| {
        let event: &AnyObject = unsafe { event.as_ref() };
        let cg: *mut OpaqueCgEvent = unsafe { msg_send![event, CGEvent] };
        if !cg.is_null() {
            mouse_event(&telemetry, cg.cast(), |x, y| {
                telemetry.on_scrawl_window(x, y)
            });
        }
    });
    let monitor: Option<objc2::rc::Retained<AnyObject>> = unsafe {
        msg_send![class, addGlobalMonitorForEventsMatchingMask: MOUSE_MONITOR_MASK, handler: &*block]
    };
    monitor.map(std::mem::forget).is_some()
}

/// A mouse button or scroll in another app, from the mouse monitor (main thread). Presses and
/// scrolls on Scrawl's helper windows (`on_scrawl`: Settings, onboarding) aren't part of what is
/// being shown.
fn mouse_event(
    telemetry: &SharedCaptureTelemetry,
    event: EventRef,
    on_scrawl: impl Fn(f64, f64) -> bool,
) {
    if !telemetry.active.load(Ordering::Acquire) {
        return;
    }
    let event_type = unsafe { CGEventGetType(event) };
    let flags = unsafe { CGEventGetFlags(event) };
    let pure_shift = (flags & FLAG_SHIFT) != 0
        && (flags & (FLAG_COMMAND | FLAG_CONTROL | FLAG_ALTERNATE)) == 0;
    let t_ms = telemetry.elapsed_ms();
    let loc = unsafe { CGEventGetLocation(event) };
    match event_type {
        // A ⇧-press is never a click: a ⇧-click drops a pin, a ⇧-drag is ink (the poller's).
        EVENT_LEFT_MOUSE_DOWN | EVENT_RIGHT_MOUSE_DOWN if !on_scrawl(loc.x, loc.y) => {
            if !pure_shift {
                telemetry.pointer_down(t_ms, loc.x, loc.y);
            } else if event_type == EVENT_LEFT_MOUSE_DOWN {
                telemetry.shift_down(t_ms, loc.x, loc.y);
            }
        }
        EVENT_LEFT_MOUSE_UP | EVENT_RIGHT_MOUSE_UP => {
            let right = event_type == EVENT_RIGHT_MOUSE_UP;
            let shift_press = !right && telemetry.shift_up(t_ms, loc.x, loc.y);
            if !shift_press && !pure_shift {
                telemetry.pointer_up(t_ms, loc.x, loc.y, right);
            }
        }
        EVENT_SCROLL_WHEEL => {
            let dy =
                unsafe { CGEventGetIntegerValueField(event, SCROLL_WHEEL_DELTA_AXIS_1) } as f32;
            let dx =
                unsafe { CGEventGetIntegerValueField(event, SCROLL_WHEEL_DELTA_AXIS_2) } as f32;
            if (dx.abs() > 0.0 || dy.abs() > 0.0) && !on_scrawl(loc.x, loc.y) {
                let (nx, ny, _, _) = telemetry.normalize_point(loc.x, loc.y);
                telemetry
                    .events
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(InteractionEvent::Scroll {
                        t: t_ms,
                        x: nx,
                        y: ny,
                        dx,
                        dy,
                        label: None,
                    });
            }
        }
        _ => {}
    }
}

/// Stops the key tap (Accessibility was turned off). The port and its callback's `Arc` are left
/// alive: `check_key_tap` may still hold the old pointer.
fn stop_key_tap(telemetry: &SharedCaptureTelemetry) {
    scrawl_log("Accessibility is off; stopping the key tap");
    let tap = telemetry.tap_port.swap(0, Ordering::AcqRel) as *mut c_void;
    if !tap.is_null() {
        unsafe { CGEventTapEnable(tap, false) };
    }
    let old = telemetry.tap_run_loop.swap(0, Ordering::AcqRel) as *mut c_void;
    if !old.is_null() {
        unsafe {
            CFRunLoopStop(old);
            CFRelease(old.cast_const());
        }
    }
}

/// Makes the key tap on its own thread and runs it until `stop_key_tap`. It's an active tap
/// (`kCGEventTapOptionDefault`), which needs Accessibility; a listen-only one would need Input
/// Monitoring. It hands every event on unchanged.
fn spawn_key_tap(telemetry: Arc<SharedCaptureTelemetry>) {
    let starting = Arc::clone(&telemetry);
    let spawned = thread::Builder::new()
        .name("scrawl-key-tap".into())
        .spawn(move || {
            let mask = 1_u64 << EVENT_KEY_DOWN;
            let user_info = Arc::into_raw(Arc::clone(&telemetry))
                .cast_mut()
                .cast::<c_void>();
            // tap = 1 (kCGSessionEventTap), place = 1 (kCGTailAppendEventTap),
            // options = 0 (kCGEventTapOptionDefault).
            let tap = unsafe { CGEventTapCreate(1, 1, 0, mask, key_tap_callback, user_info) };
            scrawl_log(format!(
                "key tap: CGEventTapCreate -> {tap:p} (Accessibility {})",
                if accessibility_granted() { "on" } else { "off" }
            ));
            let source = if tap.is_null() {
                ptr::null_mut()
            } else {
                unsafe { CFMachPortCreateRunLoopSource(ptr::null(), tap, 0) }
            };
            if source.is_null() {
                if tap.is_null() {
                    let _ = unsafe { Arc::from_raw(user_info.cast::<SharedCaptureTelemetry>()) };
                }
                telemetry.tap_starting.store(false, Ordering::Release);
                return;
            }
            let run_loop = unsafe { CFRunLoopGetCurrent() };
            let retained = unsafe { CFRetain(run_loop.cast_const()) } as u64;
            telemetry.tap_run_loop.store(retained, Ordering::Release);
            telemetry.tap_port.store(tap as u64, Ordering::Release);
            telemetry.tap_starting.store(false, Ordering::Release);
            unsafe {
                CFRunLoopAddSource(run_loop, source, kCFRunLoopCommonModes);
                CGEventTapEnable(tap, true);
                CFRunLoopRun();
                CGEventTapEnable(tap, false);
                CFRunLoopRemoveSource(run_loop, source, kCFRunLoopCommonModes);
                CFRelease(source);
            }
            if telemetry
                .tap_run_loop
                .compare_exchange(retained, 0, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                unsafe { CFRelease(retained as CfTypeRef) };
            }
        });
    if spawned.is_err() {
        starting.tap_starting.store(false, Ordering::Release);
    }
}

/// Key presses in any app, from the key tap. Always returns the event unchanged.
unsafe extern "C" fn key_tap_callback(
    _proxy: *mut c_void,
    event_type: u32,
    event: EventRef,
    user_info: *mut c_void,
) -> EventRef {
    if user_info.is_null() || event.is_null() {
        return event;
    }
    let telemetry = unsafe { &*user_info.cast::<SharedCaptureTelemetry>() };
    if event_type == EVENT_TAP_DISABLED_BY_TIMEOUT || event_type == EVENT_TAP_DISABLED_BY_USER_INPUT
    {
        let tap = telemetry.tap_port.load(Ordering::Acquire) as *mut c_void;
        if !tap.is_null() {
            unsafe { CGEventTapEnable(tap, true) };
        }
        return event;
    }
    if event_type != EVENT_KEY_DOWN {
        return event;
    }

    let flags = unsafe { CGEventGetFlags(event) };
    let cmd = (flags & FLAG_COMMAND) != 0;
    let ctrl = (flags & FLAG_CONTROL) != 0;
    let alt = (flags & FLAG_ALTERNATE) != 0;
    let shift = (flags & FLAG_SHIFT) != 0;
    let keycode = unsafe { CGEventGetIntegerValueField(event, KEYBOARD_EVENT_KEYCODE) } as u16;
    if telemetry.matches_shortcut(keycode, cmd, ctrl, alt, shift) {
        // Holding S repeats the key; only a fresh press counts.
        if unsafe { CGEventGetIntegerValueField(event, KEYBOARD_EVENT_AUTOREPEAT) } == 0 {
            telemetry.emit_capture_shortcut();
        }
        return event;
    }
    if keycode == KEY_ESCAPE && !cmd && !ctrl {
        telemetry.escape_pressed.store(true, Ordering::Release);
    }
    if !telemetry.active.load(Ordering::Acquire) {
        return event;
    }

    let t_ms = telemetry.elapsed_ms();
    if cmd && !ctrl && !alt && !shift && (keycode == layout_z_keycode() || keycode == KEY_Z) {
        let _ = telemetry.ui_sender.try_send(LiveOverlayEvent::UndoStroke);
    } else if cmd || ctrl {
        // Only log modifier-chord shortcuts, never ordinary typing or secrets.
        if let Some(key_name) = shortcut_key_name(keycode) {
            let mut chord = String::new();
            if ctrl {
                chord.push('⌃');
            }
            if alt {
                chord.push('⌥');
            }
            if shift {
                chord.push('⇧');
            }
            if cmd {
                chord.push('⌘');
            }
            chord.push_str(&key_name);
            telemetry.window_log_request.store(true, Ordering::Release);
            telemetry
                .events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(InteractionEvent::Key {
                    t: t_ms,
                    shortcut: chord,
                });
        }
    }
    event
}

fn current_mouse_location() -> Option<CGPoint> {
    let ev = unsafe { CGEventCreate(ptr::null()) };
    if ev.is_null() {
        return None;
    }
    let loc = unsafe { CGEventGetLocation(ev) };
    unsafe { CFRelease(ev.cast()) };
    Some(loc)
}

pub fn current_mouse_screen() -> Option<[f32; 2]> {
    current_mouse_location().map(|p| [p.x as f32, p.y as f32])
}

pub fn wait_for_modifiers_released(max_wait_ms: u64) {
    let deadline = Instant::now() + Duration::from_millis(max_wait_ms);
    while Instant::now() < deadline {
        let flags = unsafe { CGEventSourceFlagsState(0) | CGEventSourceFlagsState(1) };
        let any_mod = (flags & (FLAG_COMMAND | FLAG_CONTROL | FLAG_ALTERNATE | FLAG_SHIFT)) != 0;
        if !any_mod {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
}

pub fn post_synthetic_cmd_v() {
    let v_code = layout_v_keycode();
    unsafe {
        let down = CGEventCreateKeyboardEvent(ptr::null(), v_code, true);
        if !down.is_null() {
            CGEventSetFlags(down, FLAG_COMMAND);
            CGEventPost(0, down);
            CFRelease(down.cast());
        }
        thread::sleep(Duration::from_millis(12));
        let up = CGEventCreateKeyboardEvent(ptr::null(), v_code, false);
        if !up.is_null() {
            CGEventSetFlags(up, FLAG_COMMAND);
            CGEventPost(0, up);
            CFRelease(up.cast());
        }
    }
}

pub fn post_synthetic_click(screen_x: f64, screen_y: f64) {
    let pt = CGPoint {
        x: screen_x,
        y: screen_y,
    };
    unsafe {
        let down = CGEventCreateMouseEvent(ptr::null(), EVENT_LEFT_MOUSE_DOWN, pt, 0);
        if !down.is_null() {
            CGEventPost(0, down);
            CFRelease(down.cast());
        }
        thread::sleep(Duration::from_millis(14));
        let up = CGEventCreateMouseEvent(ptr::null(), EVENT_LEFT_MOUSE_UP, pt, 0);
        if !up.is_null() {
            CGEventPost(0, up);
            CFRelease(up.cast());
        }
    }
}

/// 120 Hz poller of modifier flags and mouse buttons, neither of which needs a permission, so
/// Shift-laser (`⇧` + move) and Shift-drag ink (`⇧` + drag) work across all apps (including
/// Karabiner's virtual HID). Every 500 ms it also keeps the key tap in step with Accessibility.
/// Key codes are never polled: reading them is Input Monitoring's.
fn run_global_hid_poller(telemetry: Arc<SharedCaptureTelemetry>) {
    let mut prev_shift_ink = false;
    let mut await_shift_release = false;
    let mut prev_mod_mask = 0_u8;
    let mut prev_buttons = [false; 2];
    let mut last_tap_check: Option<Instant> = None;
    loop {
        thread::sleep(Duration::from_millis(8));
        if last_tap_check.is_none_or(|t| t.elapsed() >= Duration::from_millis(500)) {
            check_key_tap(&telemetry);
            last_tap_check = Some(Instant::now());
        }

        // Combine COMBINED_SESSION_STATE (0, includes Karabiner virtual HID) and HID_SYSTEM_STATE (1).
        let flags = unsafe { CGEventSourceFlagsState(0) | CGEventSourceFlagsState(1) };
        let cmd = (flags & FLAG_COMMAND) != 0;
        let ctrl = (flags & FLAG_CONTROL) != 0;
        let alt = (flags & FLAG_ALTERNATE) != 0;
        let shift = (flags & FLAG_SHIFT) != 0;
        if !shift {
            await_shift_release = false;
        } else if cmd || ctrl || alt {
            await_shift_release = true;
        }
        let mod_mask =
            (u8::from(cmd) << 3) | (u8::from(ctrl) << 2) | (u8::from(alt) << 1) | u8::from(shift);
        if mod_mask != prev_mod_mask {
            scrawl_log(format!(
                "hid_poller: cmd={cmd} ctrl={ctrl} alt={alt} shift={shift}"
            ));
            prev_mod_mask = mod_mask;
        }

        let buttons = unsafe {
            [
                CGEventSourceButtonState(1, 0) || CGEventSourceButtonState(0, 0),
                CGEventSourceButtonState(1, 1) || CGEventSourceButtonState(0, 1),
            ]
        };
        if !telemetry.active.load(Ordering::Acquire) {
            prev_shift_ink = false;
            prev_buttons = buttons;
            continue;
        }

        let Some(loc) = current_mouse_location() else {
            continue;
        };
        *telemetry
            .pointer_screen
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = [loc.x as f32, loc.y as f32];

        poll_clicks(
            &telemetry,
            &mut prev_buttons,
            buttons,
            (loc.x, loc.y),
            shift && !cmd && !ctrl && !alt,
            |x, y| telemetry.on_scrawl_window(x, y),
        );

        let pure_shift = shift && !cmd && !ctrl && !alt && !await_shift_release;
        // Hold Shift + move cursor inside the capture target = draw a persistent stroke.
        track_shift_ink(&telemetry, (loc.x, loc.y), pure_shift, prev_shift_ink);

        prev_shift_ink = pure_shift;
    }
}

/// The key of a logged shortcut as the user's keyboard layout labels it (Dvorak's ⌘B, not the
/// QWERTY ⌘N at the same position).
fn shortcut_key_name(keycode: u16) -> Option<String> {
    let _ = init_keyboard_layout_map();
    let chars = LAYOUT_KEY_CHARS.get();
    key_name(keycode, |code| chars.and_then(|c| c.get(&code).copied()))
}

/// Keys that type no character are named the same on every layout; the rest come from
/// `layout_char` (what ⌘ + the key types), else from the ANSI QWERTY positions.
fn key_name(keycode: u16, layout_char: impl Fn(u16) -> Option<char>) -> Option<String> {
    let fixed = match keycode {
        36 | 76 => "↩",
        48 => "⇥",
        49 => "Space",
        51 => "⌫",
        53 => "Esc",
        117 => "⌦",
        123 => "←",
        124 => "→",
        125 => "↓",
        126 => "↑",
        115 => "Home",
        119 => "End",
        116 => "PgUp",
        121 => "PgDn",
        122 => "F1",
        120 => "F2",
        99 => "F3",
        118 => "F4",
        96 => "F5",
        97 => "F6",
        98 => "F7",
        100 => "F8",
        101 => "F9",
        109 => "F10",
        103 => "F11",
        111 => "F12",
        _ => "",
    };
    if !fixed.is_empty() {
        return Some(fixed.to_string());
    }
    layout_char(keycode)
        .or_else(|| ansi_key_char(keycode))
        .map(|c| c.to_uppercase().collect())
}

fn ansi_key_char(keycode: u16) -> Option<char> {
    const ANSI: &str = "asdfhgzxcv\0bqweryt123465=97-80]ou[ip\0lj'k;\\,/nm.";
    ANSI.chars()
        .nth(usize::from(keycode))
        .filter(|&c| c != '\0')
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TargetFrame;

    #[test]
    fn shortcut_keys_are_named_by_the_layout_not_their_qwerty_position() {
        // Dvorak puts B where QWERTY has N (45) and T where QWERTY has K (40).
        let dvorak = |code: u16| match code {
            45 => Some('b'),
            40 => Some('t'),
            13 => Some(','),
            _ => None,
        };
        assert_eq!(key_name(45, dvorak).as_deref(), Some("B"));
        assert_eq!(key_name(40, dvorak).as_deref(), Some("T"));
        assert_eq!(key_name(13, dvorak).as_deref(), Some(","));
        // Keys that type nothing keep fixed names whatever the layout says.
        let odd = |_| Some('x');
        assert_eq!(key_name(36, odd).as_deref(), Some("↩"));
        assert_eq!(key_name(123, odd).as_deref(), Some("←"));
        assert_eq!(key_name(122, odd).as_deref(), Some("F1"));
        assert_eq!(key_name(53, odd).as_deref(), Some("Esc"));
    }

    #[test]
    fn shortcut_keys_fall_back_to_ansi_positions_without_a_layout() {
        let none = |_| None;
        assert_eq!(key_name(45, none).as_deref(), Some("N"));
        assert_eq!(key_name(17, none).as_deref(), Some("T"));
        assert_eq!(key_name(0, none).as_deref(), Some("A"));
        assert_eq!(key_name(47, none).as_deref(), Some("."));
        assert_eq!(key_name(29, none).as_deref(), Some("0"));
        assert_eq!(key_name(10, none), None);
        assert_eq!(key_name(200, none), None);
    }

    /// The mouse monitor's AppKit calls pass objc2's encoding checks (which debug builds, the
    /// installed app included, run on every message), and a monitored click is recorded.
    #[test]
    fn the_mouse_monitor_reads_cg_events_from_appkit_events() {
        use objc2::msg_send;
        use objc2::runtime::{AnyClass, AnyObject};
        let class = AnyClass::get(c"NSEvent").expect("NSEvent");
        let point = CGPoint { x: 100.0, y: 200.0 };
        let cg = unsafe { CGEventCreateMouseEvent(ptr::null(), EVENT_LEFT_MOUSE_DOWN, point, 0) };
        assert!(!cg.is_null());
        let event: Option<objc2::rc::Retained<AnyObject>> =
            unsafe { msg_send![class, eventWithCGEvent: cg.cast::<OpaqueCgEvent>()] };
        let event = event.expect("an NSEvent");
        let back: *mut OpaqueCgEvent = unsafe { msg_send![&*event, CGEvent] };
        assert!(!back.is_null());
        assert_eq!(unsafe { CGEventGetType(back.cast()) }, EVENT_LEFT_MOUSE_DOWN);

        let (sender, _receiver) = flume::unbounded();
        let telemetry = SharedCaptureTelemetry::new(sender);
        telemetry.reset_for_session(TargetFrame {
            x: 0.0,
            y: 0.0,
            width: 1000.0,
            height: 1000.0,
            scale: 2.0,
        });
        mouse_event(&telemetry, back.cast(), |_, _| false);
        let up = unsafe { CGEventCreateMouseEvent(ptr::null(), EVENT_LEFT_MOUSE_UP, point, 0) };
        mouse_event(&telemetry, up, |_, _| false);
        unsafe {
            CFRelease(up.cast());
            CFRelease(cg.cast());
        }
        let (events, _) = telemetry.finish_session();
        assert!(
            matches!(events[..], [InteractionEvent::Click { x, y, .. }] if (x - 0.1).abs() < 1e-3 && (y - 0.2).abs() < 1e-3),
            "{events:?}"
        );
        assert_eq!(MOUSE_MONITOR_MASK & (1 << EVENT_KEY_DOWN), 0, "keys need Accessibility");
    }

    /// A left click from the mouse monitor: down at `from` with `flags`, up at `to` with `up_flags`.
    fn click(t: &SharedCaptureTelemetry, from: (f64, f64), to: (f64, f64), flags: [u64; 2]) {
        for (kind, (x, y), flags) in [
            (EVENT_LEFT_MOUSE_DOWN, from, flags[0]),
            (EVENT_LEFT_MOUSE_UP, to, flags[1]),
        ] {
            let cg = unsafe { CGEventCreateMouseEvent(ptr::null(), kind, CGPoint { x, y }, 0) };
            assert!(!cg.is_null());
            unsafe { CGEventSetFlags(cg, flags) };
            mouse_event(t, cg, |_, _| false);
            unsafe { CFRelease(cg.cast()) };
        }
    }

    fn session(width: f64, height: f64) -> Arc<SharedCaptureTelemetry> {
        let (sender, _receiver) = flume::unbounded();
        let telemetry = SharedCaptureTelemetry::new(sender);
        telemetry.reset_for_session(TargetFrame {
            x: 0.0,
            y: 0.0,
            width,
            height,
            scale: 2.0,
        });
        telemetry
    }

    /// Issue #9: ⇧-click is documented to drop a numbered pin, but the mouse monitor skipped
    /// every ⇧ press (as ink) and nothing else made pins, so none ever landed.
    #[test]
    fn a_shift_click_drops_numbered_pins_and_is_not_a_click() {
        let telemetry = session(1000.0, 1000.0);
        let shift = [FLAG_SHIFT; 2];
        // A few points of trackpad jitter.
        click(&telemetry, (100.0, 200.0), (103.0, 202.0), shift);
        click(&telemetry, (500.0, 500.0), (500.0, 500.0), shift);
        // ⇧ let go before the button still drops the pin.
        click(&telemetry, (700.0, 300.0), (700.0, 300.0), [FLAG_SHIFT, 0]);
        // A ⇧-drag is ink (the poller's), not a pin; ⇧ with ⌘ is neither.
        click(&telemetry, (100.0, 800.0), (300.0, 800.0), shift);
        let cmd_shift = [FLAG_SHIFT | FLAG_COMMAND; 2];
        click(&telemetry, (900.0, 900.0), (900.0, 900.0), cmd_shift);

        let (events, strokes) = telemetry.finish_session();
        let pins: Vec<_> = strokes
            .iter()
            .map(|s| (s.pin, s.normalized_points()[0]))
            .collect();
        assert_eq!(
            pins,
            vec![
                (Some(1), [0.1, 0.2]),
                (Some(2), [0.5, 0.5]),
                (Some(3), [0.7, 0.3])
            ]
        );
        let shape = crate::sketch::shape_of(&strokes[0]);
        assert_eq!(shape, crate::model::Shape::Pin);
        // Only the ⌘⇧-click is an ordinary click.
        assert!(
            matches!(events[..], [InteractionEvent::Click { x, .. }] if (x - 0.9).abs() < 1e-3),
            "{events:?}"
        );
    }

    /// The helper's windows (the SwiftUI capsule's Finish, Settings, onboarding) reach the mouse
    /// monitor like another app's: 87 captures ended "clicked text “Finish”", with a keyframe and
    /// a ripple for it. Presses, ⇧-presses and scrolls there are dropped; the app's are kept.
    #[test]
    fn presses_and_scrolls_on_scrawl_s_helper_windows_are_not_recorded() {
        unsafe extern "C" {
            fn CGEventSetType(event: EventRef, kind: u32);
            fn CGEventSetLocation(event: EventRef, at: CGPoint);
            fn CGEventSetIntegerValueField(event: EventRef, field: u32, value: i64);
        }
        let telemetry = session(1000.0, 1000.0);
        // Settings covers the right half.
        let on_scrawl = |x: f64, _: f64| x >= 500.0;
        let send = |kind, (x, y), flags| {
            let cg = unsafe { CGEventCreateMouseEvent(ptr::null(), kind, CGPoint { x, y }, 0) };
            unsafe { CGEventSetFlags(cg, flags) };
            mouse_event(&telemetry, cg, on_scrawl);
            unsafe { CFRelease(cg.cast()) };
        };
        let scroll = |(x, y)| {
            let cg = unsafe { CGEventCreate(ptr::null()) };
            unsafe {
                CGEventSetType(cg, EVENT_SCROLL_WHEEL);
                CGEventSetLocation(cg, CGPoint { x, y });
                CGEventSetIntegerValueField(cg, SCROLL_WHEEL_DELTA_AXIS_1, -3);
            }
            mouse_event(&telemetry, cg, on_scrawl);
            unsafe { CFRelease(cg.cast()) };
        };
        for at in [(700.0, 100.0), (100.0, 100.0)] {
            send(EVENT_LEFT_MOUSE_DOWN, at, 0);
            send(EVENT_LEFT_MOUSE_UP, at, 0);
            send(EVENT_LEFT_MOUSE_DOWN, at, FLAG_SHIFT);
            send(EVENT_LEFT_MOUSE_UP, at, FLAG_SHIFT);
            scroll(at);
        }
        // A drag from the app that ends over Settings is still the app's.
        send(EVENT_LEFT_MOUSE_DOWN, (300.0, 500.0), 0);
        send(EVENT_LEFT_MOUSE_UP, (800.0, 500.0), 0);

        let (events, strokes) = telemetry.finish_session();
        assert!(
            matches!(
                events[..],
                [
                    InteractionEvent::Click { x: cx, .. },
                    InteractionEvent::Scroll { x: sx, .. },
                    InteractionEvent::Drag { x0, .. },
                ] if (cx - 0.1).abs() < 1e-3 && (sx - 0.1).abs() < 1e-3 && (x0 - 0.3).abs() < 1e-3
            ),
            "{events:?}"
        );
        let pins: Vec<_> = strokes.iter().map(|s| s.normalized_points()[0]).collect();
        assert_eq!(pins, vec![[0.1, 0.1]]);
    }

    #[test]
    fn helper_processes_are_known_by_their_executable() {
        let own = std::process::id() as i32;
        let exe = std::fs::canonicalize(std::env::current_exe().unwrap()).unwrap();
        assert_eq!(process_path(own), Some(exe));
        assert!(!is_helper_process(own));
        assert!(!is_helper_process(1), "launchd");
        assert_eq!(process_path(-1), None);
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let path = process_path(child.id() as i32);
        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(path, Some(PathBuf::from("/bin/sleep")));
    }

    #[test]
    fn the_mouse_monitor_installs_without_a_permission() {
        let (sender, _receiver) = flume::unbounded();
        assert!(install_mouse_monitor(SharedCaptureTelemetry::new(sender)));
    }
}
