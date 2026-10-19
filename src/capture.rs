use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use screencapturekit::prelude::*;
use screencapturekit::recording_output::{
    SCRecordingOutput, SCRecordingOutputCodec, SCRecordingOutputConfiguration,
    SCRecordingOutputFileType,
};
use screencapturekit::screenshot_manager::{CGImageExt, SCScreenshotManager};

use crate::model::{
    CaptureMode, InteractionEvent, RgbaFrame, SCALE, Stroke, TargetFrame, TargetInfo,
};

pub const FRAME_SAMPLE_INTERVAL: Duration = Duration::from_millis(220);
/// Candidate frames only feed scene diffs and keyframe selection; the keyframes themselves are
/// decoded from `recording.mp4` at full size afterwards.
pub const CANDIDATE_EDGE: f64 = 480.0;
/// Without a recording to decode from, the candidates are the keyframes, so keep them sharper.
const CANDIDATE_EDGE_NO_VIDEO: f64 = 1280.0;
/// The candidates' memory ceiling, however long the capture runs.
pub const CANDIDATE_BUDGET_BYTES: usize = 384 << 20;

/// The candidate frames of a capture, bounded in memory. When full it drops every other frame
/// (keeping the first and the ones taken for a click, stroke or shortcut) and samples half as
/// often from then on, so a long capture keeps its whole timeline, evenly, at a coarser step.
pub struct CandidateBuffer {
    frames: Vec<(RgbaFrame, bool)>,
    cap: usize,
    budget: usize,
    interval: Duration,
}

impl CandidateBuffer {
    pub fn new(budget_bytes: usize) -> Self {
        Self {
            frames: Vec::new(),
            cap: 0,
            budget: budget_bytes,
            interval: FRAME_SAMPLE_INTERVAL,
        }
    }

    /// How often to sample now.
    pub fn interval(&self) -> Duration {
        self.interval
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.frames.len()
    }

    pub fn push(&mut self, frame: RgbaFrame, urgent: bool) {
        if self.cap == 0 {
            self.cap = (self.budget / frame.rgba.len().max(1)).clamp(60, 1_200);
        }
        self.frames.push((frame, urgent));
        if self.frames.len() >= self.cap {
            self.thin();
        }
    }

    fn thin(&mut self) {
        let mut regular = 0_usize;
        let mut kept: Vec<(RgbaFrame, bool)> = std::mem::take(&mut self.frames)
            .into_iter()
            .enumerate()
            .filter(|(i, (_, urgent))| {
                let keep = *i == 0 || *urgent || regular.is_multiple_of(2);
                if !urgent {
                    regular += 1;
                }
                keep
            })
            .map(|(_, f)| f)
            .collect();
        // Mostly urgent frames (a storm of clicks): thin those too.
        if kept.len() > self.cap * 3 / 4 {
            kept = kept
                .into_iter()
                .enumerate()
                .filter(|(i, _)| i.is_multiple_of(2))
                .map(|(_, f)| f)
                .collect();
        }
        self.frames = kept;
        self.interval *= 2;
    }

    pub fn into_frames(self) -> Vec<RgbaFrame> {
        self.frames.into_iter().map(|(f, _)| f).collect()
    }
}

/// What a finished screen capture leaves behind.
pub struct ScreenRecording {
    /// Small candidate frames for keyframe selection (see `CANDIDATE_EDGE`).
    pub frames: Vec<RgbaFrame>,
    pub video: Option<PathBuf>,
    pub duration_ms: u64,
    /// The capture time (on the frames' clock) of the video's start.
    pub video_offset_ms: u64,
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

const KEY_S: u16 = 1;
const KEY_ESCAPE: u16 = 53;
const KEY_Z: u16 = 6;
const KEY_V: u16 = 9;

// Carbon HIToolbox global hotkey constants (requires zero TCC permissions).
pub const CARBON_CMD_KEY: u32 = 1 << 8;
pub const CARBON_SHIFT_KEY: u32 = 1 << 9;
pub const CARBON_OPTION_KEY: u32 = 1 << 11;
pub const CARBON_CONTROL_KEY: u32 = 1 << 12;
const K_EVENT_CLASS_KEYBOARD: u32 = u32::from_be_bytes(*b"keyb");
const K_EVENT_HOT_KEY_PRESSED: u32 = 5;
const K_EVENT_PARAM_DIRECT_OBJECT: u32 = u32::from_be_bytes(*b"----");
const TYPE_EVENT_HOT_KEY_ID: u32 = u32::from_be_bytes(*b"hkid");
const HOTKEY_SIG: u32 = u32::from_be_bytes(*b"SCRW");
const HOTKEY_ID_TOGGLE: u32 = 1;
const HOTKEY_ID_ESCAPE: u32 = 2;
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

/// Live UI events emitted by the global input monitor to drive Scrawl's HUD & Review windows.
#[derive(Clone, Debug)]
pub enum LiveOverlayEvent {
    /// ⌥⇧S (or the configured shortcut) went down; `hold::Switcher` decides what it means.
    CaptureShortcut,
    PasteLatestShortcut,
    UndoStroke,
    CancelCapture,
    RequestSnapshot,
    /// The iPhone mirror ended on its own (unplugged, never found, or access denied).
    PhoneEnded,
    /// The welcome window: a button, fresh permission states, or the helper exiting.
    Onboarding(crate::onboarding::Event),
    /// The Settings window: an action, fresh permission states, or the helper exiting.
    Settings(crate::settings::Event),
    /// A ScrawlKit recording was ingested from an iPhone and compiled into a `.scrawl` bundle.
    IngestedBundle {
        dir: PathBuf,
        bundle: Box<crate::model::ScrawlBundle>,
        status: Option<String>,
    },
}

static LAYOUT_KEY_CODES: std::sync::OnceLock<std::collections::HashMap<char, u16>> =
    std::sync::OnceLock::new();

fn translate_keycode(layout: *const u8, key_code: u16) -> Option<char> {
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
            0,
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
                }
            }
            unsafe { CFRelease(source) };
        }
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

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ShortcutConfig {
    pub key_code: u32,
    pub carbon_modifiers: u32,
    pub label: String,
}

impl Default for ShortcutConfig {
    fn default() -> Self {
        Self::default_record()
    }
}

/// Scrawl's two global shortcuts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ShortcutKind {
    /// Start, switch or finish a recording (⌥⇧S).
    Record,
    /// Paste the latest capture into the app in front (⌥⇧V).
    Paste,
}

impl ShortcutKind {
    pub const ALL: [Self; 2] = [Self::Record, Self::Paste];

    pub fn default_config(self) -> ShortcutConfig {
        match self {
            Self::Record => ShortcutConfig::default_record(),
            Self::Paste => ShortcutConfig::default_paste(),
        }
    }

    /// The letter of the default, on the active layout (Dvorak's S is not ANSI's).
    fn default_char(self) -> char {
        match self {
            Self::Record => 's',
            Self::Paste => 'v',
        }
    }

    fn ansi_code(self) -> u16 {
        match self {
            Self::Record => KEY_S,
            Self::Paste => KEY_V,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Record => "Record",
            Self::Paste => "Paste last capture",
        }
    }
}

impl ShortcutConfig {
    pub fn default_record() -> Self {
        Self {
            key_code: u32::from(layout_s_keycode()),
            carbon_modifiers: CARBON_OPTION_KEY | CARBON_SHIFT_KEY,
            label: "⌥⇧S".into(),
        }
    }

    pub fn default_paste() -> Self {
        Self {
            key_code: u32::from(layout_v_keycode()),
            carbon_modifiers: CARBON_OPTION_KEY | CARBON_SHIFT_KEY,
            label: "⌥⇧V".into(),
        }
    }

    /// Whether this is `kind`'s default, which also answers to its aliases (⌃⌥⇧, ⌘⌥⇧, ⌃⌥⌘
    /// and the ANSI key).
    pub fn is_default_for(&self, kind: ShortcutKind) -> bool {
        self.carbon_modifiers == CARBON_OPTION_KEY | CARBON_SHIFT_KEY
            && shortcut_parts(&self.label).1 == kind.default_char().to_ascii_uppercase().to_string()
    }

    pub fn key_label(&self) -> &str {
        shortcut_parts(&self.label).1
    }
}

/// `⌃⌥⇧⌘` in the order macOS writes them, for Carbon modifier bits.
pub fn modifier_symbols(carbon_modifiers: u32) -> String {
    [
        (CARBON_CONTROL_KEY, '⌃'),
        (CARBON_OPTION_KEY, '⌥'),
        (CARBON_SHIFT_KEY, '⇧'),
        (CARBON_CMD_KEY, '⌘'),
    ]
    .into_iter()
    .filter(|(bit, _)| carbon_modifiers & bit != 0)
    .map(|(_, symbol)| symbol)
    .collect()
}

/// The registered Carbon hotkeys, so a change can replace all of them.
struct CarbonHotKeyState {
    refs: Vec<usize>,
}

static CARBON_HOTKEYS: std::sync::OnceLock<Mutex<CarbonHotKeyState>> = std::sync::OnceLock::new();

pub fn scrawl_log(msg: impl AsRef<str>) {
    use std::io::Write;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let line = format!("[{ts}] {}\n", msg.as_ref());
    eprint!("{line}");
    let Some(dir) = dirs::home_dir().map(|home| home.join("Library/Logs/Scrawl")) else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("scrawl.log"))
    {
        let _ = f.write_all(line.as_bytes());
    }
}

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
        match id {
            HOTKEY_ID_ESCAPE => {
                let _ = telemetry
                    .ui_sender
                    .try_send(LiveOverlayEvent::CancelCapture);
            }
            id if id >= HOTKEY_ID_PASTE_BASE => {
                telemetry.emit_paste_shortcut();
            }
            _ => {
                telemetry.emit_capture_shortcut();
            }
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

/// The key codes and Carbon modifiers `cfg` registers: itself, plus for a default its aliases
/// (⌃⌥⇧, ⌘⌥⇧, ⌃⌥⌘) on the active layout's key and the ANSI one.
fn hotkey_combos(kind: ShortcutKind, cfg: &ShortcutConfig) -> Vec<(u32, u32)> {
    let mut combos = vec![(cfg.key_code, cfg.carbon_modifiers)];
    if !cfg.is_default_for(kind) {
        return combos;
    }
    let mut codes = vec![cfg.key_code];
    if cfg.key_code != u32::from(kind.ansi_code()) {
        codes.push(u32::from(kind.ansi_code()));
    }
    for code in codes {
        for mods in [
            CARBON_OPTION_KEY | CARBON_SHIFT_KEY,
            CARBON_CONTROL_KEY | CARBON_OPTION_KEY | CARBON_SHIFT_KEY,
            CARBON_CMD_KEY | CARBON_OPTION_KEY | CARBON_SHIFT_KEY,
            CARBON_CONTROL_KEY | CARBON_OPTION_KEY | CARBON_CMD_KEY,
        ] {
            if !combos.contains(&(code, mods)) {
                combos.push((code, mods));
            }
        }
    }
    combos
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

/// A shortcut label split into its modifiers and key: `⌥⇧S` → (`⌥⇧`, `S`).
pub fn shortcut_parts(label: &str) -> (&str, &str) {
    let split = label
        .char_indices()
        .find(|(_, c)| !matches!(c, '⌃' | '⌥' | '⇧' | '⌘'))
        .map_or(label.len(), |(i, _)| i);
    label.split_at(split)
}

/// Whether a key press is the record shortcut. A custom one matches exactly; the default ⌥⇧S
/// also answers with ⌃ or ⌘ added and as ⌃⌥⌘S, on the active layout's S key (`s_code`) or the
/// ANSI one.
pub fn is_capture_shortcut(
    cfg: &ShortcutConfig,
    s_code: u16,
    keycode: u16,
    cmd: bool,
    ctrl: bool,
    alt: bool,
    shift: bool,
) -> bool {
    shortcut_matches(
        ShortcutKind::Record,
        cfg,
        s_code,
        keycode,
        cmd,
        ctrl,
        alt,
        shift,
    )
}

/// `is_capture_shortcut` for any of Scrawl's shortcuts; `layout_code` is the default key on the
/// active layout.
#[allow(clippy::too_many_arguments)]
pub fn shortcut_matches(
    kind: ShortcutKind,
    cfg: &ShortcutConfig,
    layout_code: u16,
    keycode: u16,
    cmd: bool,
    ctrl: bool,
    alt: bool,
    shift: bool,
) -> bool {
    let want_cmd = (cfg.carbon_modifiers & CARBON_CMD_KEY) != 0;
    let want_ctrl = (cfg.carbon_modifiers & CARBON_CONTROL_KEY) != 0;
    let want_alt = (cfg.carbon_modifiers & CARBON_OPTION_KEY) != 0;
    let want_shift = (cfg.carbon_modifiers & CARBON_SHIFT_KEY) != 0;
    let exact = u32::from(keycode) == cfg.key_code
        && cmd == want_cmd
        && ctrl == want_ctrl
        && alt == want_alt
        && shift == want_shift;
    if exact {
        return true;
    }
    let is_key = keycode == layout_code || keycode == kind.ansi_code();
    cfg.is_default_for(kind) && is_key && alt && (shift || (ctrl && cmd))
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

/// No-op: do not register a global Escape hotkey because Karabiner maps CapsLock (Control)
/// with `to_if_alone: escape`, which would cancel recordings when releasing Control.
pub fn set_escape_hotkey_enabled(_enabled: bool) {}

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

fn clean_label(input: &str) -> String {
    let squashed = input.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = squashed.trim();
    if trimmed.chars().count() > 44 {
        let prefix: String = trimmed.chars().take(43).collect();
        format!("{}…", prefix.trim_end())
    } else {
        trimmed.to_string()
    }
}

/// Discovers the current capture target (either a specific window/app or the main display).
pub fn discover_target(window_query: Option<&str>) -> TargetInfo {
    discover_target_at(window_query, current_mouse_location().map(|p| [p.x, p.y]))
}

/// Which display to record: the one under `point`, else the one holding the focused window's
/// centre, else the first. Frames and points are global, top-left origin.
pub fn pick_display(
    displays: &[[f64; 4]],
    point: Option<[f64; 2]>,
    focused: Option<[f64; 4]>,
) -> usize {
    let inside = |p: [f64; 2]| {
        displays
            .iter()
            .position(|&[x, y, w, h]| p[0] >= x && p[0] < x + w && p[1] >= y && p[1] < y + h)
    };
    point
        .and_then(inside)
        .or_else(|| focused.and_then(|[x, y, w, h]| inside([x + w / 2.0, y + h / 2.0])))
        .unwrap_or(0)
}

/// Like `discover_target`, recording the display under `point` (see `pick_display`).
pub fn discover_target_at(window_query: Option<&str>, point: Option<[f64; 2]>) -> TargetInfo {
    let frontmost = frontmost_app_info();
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
                let frame = win.frame();
                let owner = win.owning_application();
                let display_id = content
                    .displays()
                    .first()
                    .map(|d| d.display_id())
                    .unwrap_or_else(|| unsafe { CGMainDisplayID() });
                return TargetInfo {
                    mode: CaptureMode::Window,
                    app: owner.as_ref().map(|a| a.application_name()),
                    bundle_id: owner.as_ref().map(|a| a.bundle_identifier()),
                    window_title: win.title(),
                    window_id: Some(win.window_id()),
                    display_id,
                    frame: TargetFrame {
                        x: frame.origin.x,
                        y: frame.origin.y,
                        width: frame.size.width.max(1.0),
                        height: frame.size.height.max(1.0),
                        scale: 2.0,
                    },
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
            let frame = display.frame();
            let window_title = front_window.as_ref().and_then(|w| w.title());
            return TargetInfo {
                mode: CaptureMode::Display,
                app: frontmost.as_ref().map(|(_, name, _)| name.clone()),
                bundle_id: frontmost.and_then(|(_, _, bid)| bid),
                window_title,
                window_id: None,
                display_id: display.display_id(),
                frame: TargetFrame {
                    x: frame.origin.x,
                    y: frame.origin.y,
                    width: frame.size.width.max(1.0),
                    height: frame.size.height.max(1.0),
                    scale: 2.0,
                },
            };
        }
    }

    let display_id = unsafe { CGMainDisplayID() };
    let width = (unsafe { CGDisplayPixelsWide(display_id) } as f64).max(1440.0);
    let height = (unsafe { CGDisplayPixelsHigh(display_id) } as f64).max(900.0);
    TargetInfo {
        mode: CaptureMode::Display,
        app: frontmost.as_ref().map(|(_, name, _)| name.clone()),
        bundle_id: frontmost.and_then(|(_, _, bid)| bid),
        window_title: None,
        window_id: None,
        display_id,
        frame: TargetFrame {
            x: 0.0,
            y: 0.0,
            width,
            height,
            scale: 2.0,
        },
    }
}

/// Lists all on-screen normal (`window_layer == 0`) windows in front-to-back order for the Window picker.
pub fn list_selectable_windows() -> Vec<TargetInfo> {
    let Ok(content) = SCShareableContent::get() else {
        return Vec::new();
    };
    let our_pid = std::process::id() as i32;
    let display_id = content
        .displays()
        .first()
        .map(|d| d.display_id())
        .unwrap_or_else(|| unsafe { CGMainDisplayID() });

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
                frame: TargetFrame {
                    x: f.origin.x,
                    y: f.origin.y,
                    width: f.size.width.max(1.0),
                    height: f.size.height.max(1.0),
                    scale: 2.0,
                },
            }
        })
        .collect()
}

pub fn selectable_window_at(windows: &[TargetInfo], x: f64, y: f64) -> Option<&TargetInfo> {
    windows.iter().find(|window| {
        let f = window.frame;
        x >= f.x && x <= f.x + f.width && y >= f.y && y <= f.y + f.height
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_stop_joins_worker_before_collecting_its_final_media() {
        let stopping = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(Mutex::new(CandidateBuffer::new(1 << 20)));
        let worker_stopping = Arc::clone(&stopping);
        let worker_frames = Arc::clone(&frames);
        let path = std::env::temp_dir().join(crate::make_bundle_id());
        let worker_path = path.clone();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(2);
            while !worker_stopping.load(Ordering::Acquire) && Instant::now() < deadline {
                thread::yield_now();
            }
            assert!(worker_stopping.load(Ordering::Acquire));
            worker_frames
                .lock()
                .unwrap()
                .push(RgbaFrame::new(42, 1, 1, vec![1, 2, 3, 255]), false);
            std::fs::write(&worker_path, b"finalized media").unwrap();
            Some(worker_path)
        });
        let session = ScreenCaptureSession {
            started_at: Instant::now(),
            stopping,
            urgent_snapshot: Arc::new(AtomicBool::new(false)),
            frames,
            video_offset_ms: Arc::new(AtomicU64::new(120)),
            sampler_thread: Some(worker),
            _recording: crate::retention::Recording::start(),
        };
        let recording = session.stop();
        let frames = recording.frames;
        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].t_ms, 42);
        assert_eq!(frames[0].rgba, vec![1, 2, 3, 255]);
        assert_eq!(recording.video.as_ref(), Some(&path));
        assert_eq!(recording.video_offset_ms, 120);
        std::fs::remove_file(path).unwrap();
    }

    /// Kit's Screen capture on the Pro: the old listen-only event tap was off (no Input
    /// Monitoring), so his clicks on OpenCode's composer never reached the bundle. Clicks now
    /// come from the mouse monitor; polled button state stands in if it couldn't be installed.
    #[test]
    fn clicks_are_recorded_from_polled_buttons_while_the_mouse_monitor_is_missing() {
        let (sender, _receiver) = flume::unbounded();
        let telemetry = SharedCaptureTelemetry::new(sender);
        telemetry.reset_for_session(TargetFrame {
            x: 0.0,
            y: 0.0,
            width: 1728.0,
            height: 1117.0,
            scale: 2.0,
        });
        assert!(!telemetry.mouse_live.load(Ordering::Acquire));
        let mut prev = [false; 2];
        let not_scrawl = |_: f64, _: f64| false;
        let composer = (778.0, 1050.0);
        // A Shift-drag is ink, not a click.
        poll_clicks(
            &telemetry,
            &mut prev,
            [true, false],
            composer,
            true,
            not_scrawl,
        );
        poll_clicks(
            &telemetry,
            &mut prev,
            [false, false],
            composer,
            true,
            not_scrawl,
        );
        // A click on the composer.
        poll_clicks(
            &telemetry,
            &mut prev,
            [true, false],
            composer,
            false,
            not_scrawl,
        );
        poll_clicks(
            &telemetry,
            &mut prev,
            [true, false],
            composer,
            false,
            not_scrawl,
        );
        poll_clicks(
            &telemetry,
            &mut prev,
            [false, false],
            composer,
            false,
            not_scrawl,
        );
        // Then the stop button, which is Scrawl's own.
        let stop = (860.0, 20.0);
        poll_clicks(&telemetry, &mut prev, [true, false], stop, false, |_, _| {
            true
        });
        poll_clicks(
            &telemetry,
            &mut prev,
            [false, false],
            stop,
            false,
            |_, _| true,
        );
        // While the mouse monitor is installed it reports clicks itself.
        telemetry.mouse_live.store(true, Ordering::Release);
        poll_clicks(
            &telemetry,
            &mut prev,
            [true, false],
            composer,
            false,
            not_scrawl,
        );
        poll_clicks(
            &telemetry,
            &mut prev,
            [false, false],
            composer,
            false,
            not_scrawl,
        );

        let (events, _) = telemetry.finish_session();
        assert_eq!(events.len(), 1, "{events:?}");
        let InteractionEvent::Click { x, y, button, .. } = &events[0] else {
            panic!("{events:?}");
        };
        assert!(
            (x - 0.45).abs() < 0.01 && (y - 0.94).abs() < 0.01,
            "{x}, {y}"
        );
        assert_eq!(button.as_deref(), Some("left"));
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
        mouse_event(&telemetry, back.cast());
        let up = unsafe { CGEventCreateMouseEvent(ptr::null(), EVENT_LEFT_MOUSE_UP, point, 0) };
        mouse_event(&telemetry, up);
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

    #[test]
    fn the_mouse_monitor_installs_without_a_permission() {
        let (sender, _receiver) = flume::unbounded();
        assert!(install_mouse_monitor(SharedCaptureTelemetry::new(sender)));
    }

    #[test]
    fn the_capture_shortcut_matches_its_aliases_and_a_custom_one() {
        let s = 1_u16; // the S key on this layout
        let default = ShortcutConfig {
            key_code: u32::from(s),
            carbon_modifiers: CARBON_OPTION_KEY | CARBON_SHIFT_KEY,
            label: "⌥⇧S".into(),
        };
        // (cmd, ctrl, alt, shift)
        let hit = |cfg: &ShortcutConfig, key, (cmd, ctrl, alt, shift)| {
            is_capture_shortcut(cfg, s, key, cmd, ctrl, alt, shift)
        };
        assert!(hit(&default, s, (false, false, true, true)), "⌥⇧S");
        assert!(hit(&default, s, (true, false, true, true)), "⌥⇧⌘S");
        assert!(hit(&default, s, (false, true, true, true)), "⌃⌥⇧S");
        assert!(hit(&default, s, (true, true, true, false)), "⌃⌥⌘S");
        assert!(hit(&default, s, (true, true, true, true)), "hyper S");
        assert!(hit(&default, KEY_S, (false, false, true, true)), "ANSI S");
        assert!(!hit(&default, s, (false, false, false, true)), "⇧S");
        assert!(!hit(&default, s, (true, false, false, false)), "⌘S");
        assert!(!hit(&default, 2, (false, false, true, true)), "⌥⇧D");

        let custom = ShortcutConfig {
            key_code: 15,
            carbon_modifiers: CARBON_CONTROL_KEY | CARBON_OPTION_KEY,
            label: "⌃⌥R".into(),
        };
        assert!(hit(&custom, 15, (false, true, true, false)));
        assert!(!hit(&custom, 15, (true, true, true, false)));
        assert!(!hit(&custom, 15, (false, false, true, false)));
        // Once changed, ⌥⇧S and its aliases are free for other apps.
        assert!(!hit(&custom, s, (false, false, true, true)), "⌥⇧S");
        assert!(!hit(&custom, KEY_S, (true, true, true, true)), "hyper S");
        assert_eq!(
            hotkey_combos(ShortcutKind::Record, &custom),
            [(15, custom.carbon_modifiers)]
        );
        assert_eq!(
            hotkey_combos(ShortcutKind::Record, &default).len(),
            4,
            "⌥⇧S and three aliases"
        );

        // The paste shortcut works the same way.
        let v = 9_u16;
        let paste = ShortcutConfig {
            key_code: u32::from(v),
            carbon_modifiers: CARBON_OPTION_KEY | CARBON_SHIFT_KEY,
            label: "⌥⇧V".into(),
        };
        let paste_hit = |cfg: &ShortcutConfig, key, (cmd, ctrl, alt, shift)| {
            shortcut_matches(ShortcutKind::Paste, cfg, v, key, cmd, ctrl, alt, shift)
        };
        assert!(paste_hit(&paste, v, (false, false, true, true)), "⌥⇧V");
        assert!(paste_hit(&paste, v, (true, true, true, false)), "⌃⌥⌘V");
        assert!(
            !paste_hit(&paste, s, (false, false, true, true)),
            "⌥⇧S isn't paste"
        );
        let paste_custom = ShortcutConfig {
            key_code: 35,
            carbon_modifiers: CARBON_CONTROL_KEY | CARBON_OPTION_KEY,
            label: "⌃⌥P".into(),
        };
        assert!(paste_hit(&paste_custom, 35, (false, true, true, false)));
        assert!(!paste_hit(&paste_custom, v, (false, false, true, true)));
        assert!(
            paste.is_default_for(ShortcutKind::Paste)
                && !paste.is_default_for(ShortcutKind::Record)
        );
        assert_eq!(
            modifier_symbols(CARBON_CMD_KEY | CARBON_CONTROL_KEY | CARBON_SHIFT_KEY),
            "⌃⇧⌘"
        );

        assert_eq!(shortcut_parts("⌥⇧S"), ("⌥⇧", "S"));
        assert_eq!(shortcut_parts("⌃⌥⌘R"), ("⌃⌥⌘", "R"));
    }

    #[test]
    fn recordings_use_the_display_under_the_cursor() {
        let displays = [[0.0, 0.0, 1728.0, 1117.0], [1728.0, -200.0, 2560.0, 1440.0]];
        assert_eq!(pick_display(&displays, Some([2000.0, 300.0]), None), 1);
        assert_eq!(pick_display(&displays, Some([100.0, 100.0]), None), 0);
        // No cursor: the display holding the focused window's centre.
        let focused = Some([1900.0, 0.0, 800.0, 600.0]);
        assert_eq!(pick_display(&displays, None, focused), 1);
        assert_eq!(pick_display(&displays, Some([-50.0, 0.0]), focused), 1);
        assert_eq!(pick_display(&displays, None, None), 0);
    }

    #[test]
    fn candidate_buffer_stays_bounded_and_keeps_the_whole_timeline() {
        // A 1 MB budget of 4 KB frames: a 256-frame cap.
        let mut buffer = CandidateBuffer::new(1 << 20);
        let frame = |t| RgbaFrame::new(t, 32, 32, vec![0; 32 * 32 * 4]);
        let mut t = 0;
        let mut peak = 0;
        // Three hours at the sampler's pace, with a click frame every 10 s.
        while t < 3 * 3_600_000 {
            buffer.push(frame(t), t % 10_000 == 0);
            peak = peak.max(buffer.len());
            t += buffer.interval().as_millis() as u64;
        }
        assert!(peak <= 256, "peak {peak}");
        assert!(buffer.interval() >= FRAME_SAMPLE_INTERVAL * 64);
        let frames = buffer.into_frames();
        assert_eq!(frames[0].t_ms, 0, "the start is kept");
        assert!(frames.windows(2).all(|w| w[0].t_ms < w[1].t_ms));
        // Spread over the whole capture, not bunched at the end.
        let first_half = frames.iter().filter(|f| f.t_ms < 3 * 1_800_000).count();
        assert!(
            first_half * 3 > frames.len(),
            "{first_half} of {}",
            frames.len()
        );
    }

    #[test]
    fn candidate_buffer_keeps_urgent_frames_when_thinning() {
        let mut buffer = CandidateBuffer::new(40 * 4096);
        let frame = |t| RgbaFrame::new(t, 32, 32, vec![0; 32 * 32 * 4]);
        for t in 0..60 {
            buffer.push(frame(t * 220), t == 33);
        }
        assert!(buffer.len() < 60);
        assert_eq!(buffer.interval(), FRAME_SAMPLE_INTERVAL * 2);
        assert!(buffer.into_frames().iter().any(|f| f.t_ms == 33 * 220));
    }

    #[test]
    fn candidates_are_small_and_recordings_keep_full_size() {
        let target = |mode, width, height| TargetInfo {
            mode,
            app: None,
            bundle_id: None,
            window_title: None,
            window_id: None,
            display_id: 1,
            frame: TargetFrame {
                x: 0.0,
                y: 0.0,
                width,
                height,
                scale: 2.0,
            },
        };
        let display = target(CaptureMode::Display, 1728.0, 1117.0);
        assert_eq!(capture_size(&display, CANDIDATE_EDGE), (480, 310));
        assert_eq!(capture_size(&display, 2560.0), (2560, 1654));
        let region = target(CaptureMode::Region, 480.0, 320.0);
        assert_eq!(capture_size(&region, CANDIDATE_EDGE), (480, 320));
        assert_eq!(capture_size(&region, 1280.0), (960, 640));
    }

    #[test]
    fn cached_window_hit_preserves_front_to_back_order_and_screen_coordinates() {
        let window = |id, x, y| TargetInfo {
            mode: CaptureMode::Window,
            app: Some(format!("App {id}")),
            bundle_id: None,
            window_title: Some(format!("Window {id}")),
            window_id: Some(id),
            display_id: 1,
            frame: TargetFrame {
                x,
                y,
                width: 200.0,
                height: 100.0,
                scale: 2.0,
            },
        };
        let windows = [window(1, -100.0, 20.0), window(2, 0.0, 0.0)];
        assert_eq!(
            selectable_window_at(&windows, 50.0, 50.0)
                .unwrap()
                .window_id,
            Some(1)
        );
        assert_eq!(
            selectable_window_at(&windows, 150.0, 50.0)
                .unwrap()
                .window_id,
            Some(2)
        );
        assert_eq!(
            selectable_window_at(&windows, -100.0, 20.0)
                .unwrap()
                .window_id,
            Some(1)
        );
        assert!(selectable_window_at(&windows, -101.0, 50.0).is_none());
        assert!(selectable_window_at(&[], 0.0, 0.0).is_none());
    }
}

/// Resolves region metadata from the picker's snapshot, without another SCK query.
pub fn region_target(
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    windows: &[TargetInfo],
) -> TargetInfo {
    let mid_x = x + width * 0.5;
    let mid_y = y + height * 0.5;
    let hit = selectable_window_at(windows, mid_x, mid_y);
    let frontmost = frontmost_app_info();
    let display_id = unsafe { CGMainDisplayID() };
    TargetInfo {
        mode: CaptureMode::Region,
        app: hit
            .and_then(|window| window.app.clone())
            .or_else(|| frontmost.as_ref().map(|(_, name, _)| name.clone())),
        bundle_id: hit
            .and_then(|window| window.bundle_id.clone())
            .or_else(|| frontmost.and_then(|(_, _, bid)| bid)),
        window_title: hit.and_then(|window| window.window_title.clone()),
        window_id: None,
        display_id,
        frame: TargetFrame {
            x,
            y,
            width: width.max(32.0),
            height: height.max(32.0),
            scale: 2.0,
        },
    }
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

/// Active screen + optional MP4 recorder using `ScreenCaptureKit`.
pub struct ScreenCaptureSession {
    started_at: Instant,
    stopping: Arc<AtomicBool>,
    urgent_snapshot: Arc<AtomicBool>,
    frames: Arc<Mutex<CandidateBuffer>>,
    video_offset_ms: Arc<AtomicU64>,
    sampler_thread: Option<JoinHandle<Option<PathBuf>>>,
    _recording: crate::retention::Recording,
}

impl ScreenCaptureSession {
    pub fn start(
        target: TargetInfo,
        video_output_path: Option<PathBuf>,
        excluded_window_ids: Arc<Mutex<Vec<u32>>>,
    ) -> Self {
        let started_at = Instant::now();
        let stopping = Arc::new(AtomicBool::new(false));
        let urgent_snapshot = Arc::new(AtomicBool::new(true));
        let frames = Arc::new(Mutex::new(CandidateBuffer::new(CANDIDATE_BUDGET_BYTES)));
        let video_offset_ms = Arc::new(AtomicU64::new(0));

        let thread_stopping = Arc::clone(&stopping);
        let thread_urgent = Arc::clone(&urgent_snapshot);
        let thread_frames = Arc::clone(&frames);
        let thread_offset = Arc::clone(&video_offset_ms);
        let sampler_target = target;
        let sampler_thread = thread::Builder::new()
            .name("scrawl-frame-sampler".into())
            .spawn(move || {
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
                let mut last_excluded_len = initial_excluded.len();
                while !thread_stopping.load(Ordering::Acquire) {
                    let cur_excluded = excluded_window_ids
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .clone();
                    if cur_excluded.len() != last_excluded_len {
                        last_excluded_len = cur_excluded.len();
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
                            filter_and_config =
                                Some((new_filter, sck_config(&sampler_target, shot_edge)));
                        }
                    }
                    let urgent = thread_urgent.swap(false, Ordering::AcqRel);
                    let interval = thread_frames
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .interval();
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
            })
            .ok();

        Self {
            started_at,
            stopping,
            urgent_snapshot,
            frames,
            video_offset_ms,
            sampler_thread,
            _recording: crate::retention::Recording::start(),
        }
    }

    pub fn request_immediate_frame(&self) {
        self.urgent_snapshot.store(true, Ordering::Release);
        if let Some(ref handle) = self.sampler_thread {
            handle.thread().unpark();
        }
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.started_at.elapsed().as_millis() as u64
    }

    pub fn stop(mut self) -> ScreenRecording {
        let duration_ms = self.elapsed_ms();
        self.stopping.store(true, Ordering::Release);
        let video_path = self
            .sampler_thread
            .take()
            .and_then(|handle| {
                handle.thread().unpark();
                handle.join().ok()
            })
            .flatten();
        let buffer = std::mem::replace(
            &mut *self
                .frames
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
            CandidateBuffer::new(0),
        );
        let mut collected = buffer.into_frames();
        if collected.is_empty() {
            collected.push(crate::render::render_synthetic_demo_frame(1280, 800, 0, 0));
        }
        let valid_video =
            video_path.filter(|path| path.exists() && path.metadata().is_ok_and(|m| m.len() > 0));
        ScreenRecording {
            frames: collected,
            video: valid_video,
            duration_ms: duration_ms.max(100),
            video_offset_ms: self.video_offset_ms.load(Ordering::Acquire),
        }
    }
}

impl Drop for ScreenCaptureSession {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        if let Some(handle) = self.sampler_thread.take() {
            handle.thread().unpark();
            let _ = handle.join();
        }
    }
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

/// The capture size for `target` with its long edge at most `long_edge` px (and at most 2× its
/// size in points).
pub fn capture_size(target: &TargetInfo, long_edge: f64) -> (u32, u32) {
    let max_dim = target.frame.width.max(target.frame.height).max(1.0);
    let scale = (long_edge / max_dim).min(2.0);
    (
        ((target.frame.width * scale).round() as u32).max(48) & !1,
        ((target.frame.height * scale).round() as u32).max(48) & !1,
    )
}

fn sck_config(target: &TargetInfo, long_edge: f64) -> SCStreamConfiguration {
    let (width, height) = capture_size(target, long_edge);
    let mut config = SCStreamConfiguration::new()
        .with_width(width)
        .with_height(height)
        .with_shows_cursor(true);
    if target.mode == CaptureMode::Region {
        config.set_source_rect(CGRect::new(
            target.frame.x,
            target.frame.y,
            target.frame.width.max(32.0),
            target.frame.height.max(32.0),
        ));
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

/// Shared state collected by the global input monitor during an active capture session.
pub struct SharedCaptureTelemetry {
    pub active: AtomicBool,
    pub selecting: AtomicBool,
    pub shift_held: AtomicBool,
    pub capture_mode: AtomicU64,
    /// The record shortcut (`crate::settings` loads and saves it).
    pub shortcut: Mutex<ShortcutConfig>,
    pub paste_shortcut: Mutex<ShortcutConfig>,
    /// Settings is recording a new shortcut: none of Scrawl's fire (`pause_shortcuts`).
    pub shortcuts_paused: AtomicBool,
    pub started_at: Mutex<Instant>,
    pub picker_started_at: Mutex<Instant>,
    pub picker_rect: Mutex<Option<([f32; 4], u8)>>,
    pub target_frame: Mutex<TargetFrame>,
    pub pointer_screen: Mutex<[f32; 2]>,
    pub events: Mutex<Vec<InteractionEvent>>,
    pub strokes: Mutex<Vec<Stroke>>,
    pub active_laser: Mutex<Option<Stroke>>,
    pub live_ink: Mutex<Option<Stroke>>,
    pub excluded_window_ids: Arc<Mutex<Vec<u32>>>,
    /// Raised when a click, ink stroke or shortcut starts, so the Screen-mode window log takes
    /// a snapshot right away.
    pub window_log_request: Arc<AtomicBool>,
    pub ui_sender: flume::Sender<LiveOverlayEvent>,
    /// Scrawl's windows that let clicks through (the ink overlay), skipped when deciding whether
    /// a click landed on Scrawl itself.
    pub passthrough_window_ids: Mutex<Vec<u32>>,
    /// The key event tap (`CFMachPortRef`), made once Accessibility is on (`check_key_tap`).
    tap_port: AtomicU64,
    /// The key event tap thread's run loop (retained), so `check_key_tap` can stop it.
    tap_run_loop: AtomicU64,
    /// A key tap thread is being started.
    tap_starting: AtomicBool,
    /// When the last key tap was tried (ms since `UNIX_EPOCH`), to space out retries.
    tap_tried_ms: AtomicU64,
    /// Whether the key tap is delivering; logged when it changes.
    tap_live: AtomicBool,
    /// Esc went down (from the key tap) since the switcher last looked (`take_escape`).
    escape_pressed: AtomicBool,
    /// A Screen recording started straight from the shortcut: the overlay flashes an edge glow.
    pub start_glow: AtomicBool,
    /// The AppKit global mouse monitor is installed, so clicks come from it. Until then the HID
    /// poller records clicks from polled button state.
    mouse_live: AtomicBool,
    last_shortcut_ms: AtomicU64,
    drag_start: Mutex<Option<(u64, f32, f32, Option<String>, TargetFrame)>>,
}

impl SharedCaptureTelemetry {
    pub fn new(ui_sender: flume::Sender<LiveOverlayEvent>) -> Arc<Self> {
        let shortcuts = crate::settings::Shortcuts::load();
        crate::settings::note_paste_label(&shortcuts.paste.label);
        Arc::new(Self {
            active: AtomicBool::new(false),
            selecting: AtomicBool::new(false),
            shift_held: AtomicBool::new(false),
            capture_mode: AtomicU64::new(0),
            shortcut: Mutex::new(shortcuts.record),
            paste_shortcut: Mutex::new(shortcuts.paste),
            shortcuts_paused: AtomicBool::new(false),
            started_at: Mutex::new(Instant::now()),
            picker_started_at: Mutex::new(Instant::now()),
            picker_rect: Mutex::new(None),
            target_frame: Mutex::new(TargetFrame {
                x: 0.0,
                y: 0.0,
                width: 1440.0,
                height: 900.0,
                scale: 2.0,
            }),
            pointer_screen: Mutex::new([0.0, 0.0]),
            events: Mutex::new(Vec::new()),
            strokes: Mutex::new(Vec::new()),
            active_laser: Mutex::new(None),
            live_ink: Mutex::new(None),
            excluded_window_ids: Arc::new(Mutex::new(Vec::new())),
            window_log_request: Arc::new(AtomicBool::new(false)),
            ui_sender,
            passthrough_window_ids: Mutex::new(Vec::new()),
            tap_port: AtomicU64::new(0),
            tap_run_loop: AtomicU64::new(0),
            tap_starting: AtomicBool::new(false),
            tap_tried_ms: AtomicU64::new(0),
            tap_live: AtomicBool::new(false),
            escape_pressed: AtomicBool::new(false),
            start_glow: AtomicBool::new(false),
            mouse_live: AtomicBool::new(false),
            last_shortcut_ms: AtomicU64::new(0),
            drag_start: Mutex::new(None),
        })
    }

    /// Each source (Carbon, the key event tap) reports the same press within a few ms; a press
    /// after that is a new one, as when S is pressed again to switch modes.
    pub fn emit_capture_shortcut(&self) {
        if self.shortcuts_paused.load(Ordering::Acquire) {
            return;
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let prev = self.last_shortcut_ms.load(Ordering::Acquire);
        if now_ms.saturating_sub(prev) >= 90
            && self
                .last_shortcut_ms
                .compare_exchange(prev, now_ms, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            scrawl_log("emit_capture_shortcut");
            let _ = self.ui_sender.try_send(LiveOverlayEvent::CaptureShortcut);
        }
    }

    pub fn emit_paste_shortcut(&self) {
        if self.shortcuts_paused.load(Ordering::Acquire) {
            return;
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let prev = self.last_shortcut_ms.load(Ordering::Acquire);
        if now_ms.saturating_sub(prev) >= 400
            && self
                .last_shortcut_ms
                .compare_exchange(prev, now_ms, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
        {
            scrawl_log("emit_paste_shortcut -> sending PasteLatestShortcut");
            let _ = self
                .ui_sender
                .try_send(LiveOverlayEvent::PasteLatestShortcut);
        }
    }

    pub fn matches_shortcut(
        &self,
        keycode: u16,
        cmd: bool,
        ctrl: bool,
        alt: bool,
        shift: bool,
    ) -> bool {
        let cfg = self
            .shortcut
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        is_capture_shortcut(&cfg, layout_s_keycode(), keycode, cmd, ctrl, alt, shift)
    }

    /// The record and paste shortcuts.
    pub fn shortcuts(&self) -> (ShortcutConfig, ShortcutConfig) {
        let lock = |m: &Mutex<ShortcutConfig>| {
            m.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
        };
        (lock(&self.shortcut), lock(&self.paste_shortcut))
    }

    pub fn set_capture_mode(&self, mode: CaptureMode) {
        let code = match mode {
            CaptureMode::Display => 0,
            CaptureMode::Region => 1,
            CaptureMode::Window => 2,
            CaptureMode::Device => 3,
        };
        self.capture_mode.store(code, Ordering::Release);
    }

    pub fn begin_picker_session(&self) {
        *self
            .picker_started_at
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Instant::now();
        *self
            .picker_rect
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.active.store(false, Ordering::Release);
        self.selecting.store(true, Ordering::Release);
    }

    pub fn set_picker_rect(&self, rect: Option<([f32; 4], u8)>) {
        *self
            .picker_rect
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = rect;
    }

    pub fn reset_for_session(&self, frame: TargetFrame) {
        self.selecting.store(false, Ordering::Release);
        *self
            .picker_rect
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        *self
            .started_at
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Instant::now();
        *self
            .target_frame
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = frame;
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        self.strokes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        *self
            .active_laser
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        *self
            .live_ink
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        *self
            .drag_start
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.excluded_window_ids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        self.shift_held.store(false, Ordering::Release);
        self.window_log_request.store(false, Ordering::Release);
        self.active.store(true, Ordering::Release);
        let clicks = if self.mouse_live.load(Ordering::Acquire) {
            "the mouse monitor"
        } else {
            "polled mouse buttons"
        };
        let keys = if self.tap_live.load(Ordering::Acquire) {
            "the key tap"
        } else {
            "nowhere (Accessibility off; ⌥⇧S still works)"
        };
        scrawl_log(format!("session clicks from {clicks}, keys from {keys}"));
    }

    /// Whether Esc went down since the last call (the switcher cancels on it).
    pub fn take_escape(&self) -> bool {
        self.escape_pressed.swap(false, Ordering::AcqRel)
    }

    pub fn elapsed_ms(&self) -> u64 {
        self.started_at
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .elapsed()
            .as_millis() as u64
    }

    pub fn contains_screen_point(&self, screen_x: f64, screen_y: f64) -> bool {
        let frame = *self
            .target_frame
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        frame.width >= 16.0
            && frame.height >= 16.0
            && frame.x > -10_000.0
            && screen_x >= frame.x
            && screen_x <= frame.x + frame.width
            && screen_y >= frame.y
            && screen_y <= frame.y + frame.height
    }

    /// A mouse button went down at a global point.
    fn pointer_down(&self, t_ms: u64, screen_x: f64, screen_y: f64) {
        if !self.contains_screen_point(screen_x, screen_y) {
            return;
        }
        let (nx, ny, _, _) = self.normalize_point(screen_x, screen_y);
        self.window_log_request.store(true, Ordering::Release);
        let start_frame = *self
            .target_frame
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *self
            .drag_start
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some((t_ms, nx, ny, None, start_frame));
    }

    /// A mouse button came up: records the click or drag that `pointer_down` began.
    fn pointer_up(&self, t_ms: u64, screen_x: f64, screen_y: f64, right: bool) {
        let Some((t0, x0, y0, start_label, start_frame)) = self
            .drag_start
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        else {
            return;
        };
        let end_frame = *self
            .target_frame
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !same_target_frame(start_frame, end_frame) {
            return;
        }
        let (nx, ny, fw, fh) = self.normalize_point(screen_x, screen_y);
        let dist_px = ((nx - x0) * fw as f32).hypot((ny - y0) * fh as f32);
        let _ = self.ui_sender.try_send(LiveOverlayEvent::RequestSnapshot);
        let event = if dist_px < 12.0 {
            InteractionEvent::Click {
                t: t0,
                x: x0,
                y: y0,
                button: Some(if right { "right" } else { "left" }.into()),
                label: start_label,
                role: None,
            }
        } else {
            InteractionEvent::Drag {
                t: t0,
                t1: t_ms,
                x0,
                y0,
                x1: nx,
                y1: ny,
                label: start_label,
                button: None,
                path: Vec::new(),
            }
        };
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(event);
    }

    /// Whether the topmost window at a global point is Scrawl's own (ignoring the click-through
    /// ink overlay).
    fn on_scrawl_window(&self, x: f64, y: f64) -> bool {
        let passthrough = self
            .passthrough_window_ids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let windows = objc2::rc::autoreleasepool(|_| crate::windows::raw_windows());
        crate::windows::owned_window_at(&windows, x, y, std::process::id() as i32, &passthrough)
    }

    /// Keeps the key event tap in step with Accessibility, which macOS reports live: makes the
    /// tap once it's on (no relaunch), stops it if it's turned off (an active tap whose grant is
    /// gone can hold up input), and re-enables it if macOS turned it off. Returns whether it is
    /// delivering. Called every 500 ms from the HID poller.
    fn check_key_tap(self: &Arc<Self>) -> bool {
        let trusted = accessibility_granted();
        let tap = self.tap_port.load(Ordering::Acquire) as *mut c_void;
        if !trusted && !tap.is_null() {
            stop_key_tap(self);
        }
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        if trusted
            && tap.is_null()
            && now_ms.saturating_sub(self.tap_tried_ms.load(Ordering::Acquire)) >= 2_000
            && !self.tap_starting.swap(true, Ordering::AcqRel)
        {
            self.tap_tried_ms.store(now_ms, Ordering::Release);
            spawn_key_tap(Arc::clone(self));
        }
        let live = trusted
            && !tap.is_null()
            && unsafe {
                CGEventTapIsEnabled(tap) || {
                    CGEventTapEnable(tap, true);
                    CGEventTapIsEnabled(tap)
                }
            };
        if self.tap_live.swap(live, Ordering::AcqRel) != live {
            scrawl_log(if live {
                "key tap live".to_string()
            } else if !trusted {
                "key tap off: Accessibility is off".to_string()
            } else {
                "key tap off (being made, or Secure Input is on)".to_string()
            });
        }
        live
    }

    pub fn normalize_point(&self, screen_x: f64, screen_y: f64) -> (f32, f32, u32, u32) {
        let frame = *self
            .target_frame
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let nx = ((screen_x - frame.x) / frame.width.max(1.0)).clamp(0.0, 1.0) as f32;
        let ny = ((screen_y - frame.y) / frame.height.max(1.0)).clamp(0.0, 1.0) as f32;
        (
            nx,
            ny,
            frame.width.round().max(1.0) as u32,
            frame.height.round().max(1.0) as u32,
        )
    }

    pub fn finish_active_laser(&self, _ax_point: Option<(f64, f64)>) {
        if let Some(mut laser) = self
            .active_laser
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            && !laser.pts.is_empty()
        {
            if let Some(last) = laser.pts.chunks_exact(3).last() {
                let lx = last[0] as f32 / SCALE as f32;
                let ly = last[1] as f32 / SCALE as f32;
                laser.t1 = laser.t0.saturating_add(last[2].max(0) as u64);
                self.events
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(InteractionEvent::Point {
                        t: laser.t1,
                        x: lx,
                        y: ly,
                        label: None,
                    });
            }
            self.strokes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(laser);
        }
    }

    pub fn finish_session(&self) -> (Vec<InteractionEvent>, Vec<Stroke>) {
        self.active.store(false, Ordering::Release);
        self.selecting.store(false, Ordering::Release);
        *self
            .drag_start
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.finish_active_laser(None);
        if let Some(ink) = self
            .live_ink
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            && ink.pts.len() >= 3
        {
            self.strokes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(ink);
        }
        let events = std::mem::take(
            &mut *self
                .events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        let strokes = std::mem::take(
            &mut *self
                .strokes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        (events, strokes)
    }
}

/// Starts watching input across every app, with no Input Monitoring:
/// - clicks, drags and scrolls from an AppKit global mouse monitor, which needs no permission
///   (only key events need Accessibility, per `addGlobalMonitorForEvents`' documentation);
/// - ⇧ (ink), the laser and the switcher's held modifiers from polled modifier flags, which need
///   no permission either (the HID poller);
/// - key presses (the switcher's S, Esc, ⌘Z, ⌘-chords for the window log) from an active key
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
/// control, the HUD) never arrive. The monitor lives as long as the app.
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
            mouse_event(&telemetry, cg.cast());
        }
    });
    let monitor: Option<objc2::rc::Retained<AnyObject>> = unsafe {
        msg_send![class, addGlobalMonitorForEventsMatchingMask: MOUSE_MONITOR_MASK, handler: &*block]
    };
    monitor.map(std::mem::forget).is_some()
}

/// A mouse button or scroll in another app, from the mouse monitor (main thread).
fn mouse_event(telemetry: &SharedCaptureTelemetry, event: EventRef) {
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
        // A ⇧-drag is ink (the poller), not a click.
        EVENT_LEFT_MOUSE_DOWN | EVENT_RIGHT_MOUSE_DOWN if !pure_shift => {
            telemetry.pointer_down(t_ms, loc.x, loc.y);
        }
        EVENT_LEFT_MOUSE_UP | EVENT_RIGHT_MOUSE_UP if !pure_shift => {
            telemetry.pointer_up(t_ms, loc.x, loc.y, event_type == EVENT_RIGHT_MOUSE_UP);
        }
        EVENT_SCROLL_WHEEL => {
            let dy =
                unsafe { CGEventGetIntegerValueField(event, SCROLL_WHEEL_DELTA_AXIS_1) } as f32;
            let dx =
                unsafe { CGEventGetIntegerValueField(event, SCROLL_WHEEL_DELTA_AXIS_2) } as f32;
            if dx.abs() > 0.0 || dy.abs() > 0.0 {
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

/// Clicks from the mouse buttons' polled state, while the mouse monitor is missing: a press
/// starts a click (unless it lands on Scrawl's own window), a release ends it.
fn poll_clicks(
    telemetry: &SharedCaptureTelemetry,
    prev: &mut [bool; 2],
    now: [bool; 2],
    (x, y): (f64, f64),
    pure_shift: bool,
    on_scrawl: impl Fn(f64, f64) -> bool,
) {
    let was = std::mem::replace(prev, now);
    if telemetry.mouse_live.load(Ordering::Acquire) || pure_shift {
        return;
    }
    for (button, (&down, &before)) in now.iter().zip(&was).enumerate() {
        if down && !before && !on_scrawl(x, y) {
            telemetry.pointer_down(telemetry.elapsed_ms(), x, y);
        } else if !down && before {
            telemetry.pointer_up(telemetry.elapsed_ms(), x, y, button == 1);
        }
    }
}

fn same_target_frame(a: TargetFrame, b: TargetFrame) -> bool {
    (a.x - b.x).abs() <= 0.5
        && (a.y - b.y).abs() <= 0.5
        && (a.width - b.width).abs() <= 0.5
        && (a.height - b.height).abs() <= 0.5
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
        // Holding S repeats the key; only a fresh press switches modes.
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
            chord.push_str(key_name);
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

pub fn is_frontmost_terminal() -> bool {
    let Some((_pid, app, bundle_id)) = frontmost_app_info() else {
        return false;
    };
    let a = app.to_ascii_lowercase();
    let b = bundle_id.unwrap_or_default().to_ascii_lowercase();
    [
        "ghostty",
        "iterm",
        "terminal",
        "kitty",
        "wezterm",
        "alacritty",
        "warp",
    ]
    .iter()
    .any(|needle| a.contains(needle) || b.contains(needle))
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
    // Index of the point a straight run (⌥⇧) started from, while Option is held.
    let mut straight_from: Option<usize> = None;
    let mut prev_mod_mask = 0_u8;
    let mut prev_buttons = [false; 2];
    let mut last_tap_check: Option<Instant> = None;
    loop {
        thread::sleep(Duration::from_millis(8));
        if last_tap_check.is_none_or(|t| t.elapsed() >= Duration::from_millis(500)) {
            telemetry.check_key_tap();
            last_tap_check = Some(Instant::now());
        }

        // Combine COMBINED_SESSION_STATE (0, includes Karabiner virtual HID) and HID_SYSTEM_STATE (1).
        let flags = unsafe { CGEventSourceFlagsState(0) | CGEventSourceFlagsState(1) };
        let cmd = (flags & FLAG_COMMAND) != 0;
        let ctrl = (flags & FLAG_CONTROL) != 0;
        let alt = (flags & FLAG_ALTERNATE) != 0;
        let shift = (flags & FLAG_SHIFT) != 0;
        // ⌥⇧ held before a capture starts is the shortcut, not ink: wait for ⇧ to come up.
        if !shift {
            await_shift_release = false;
        } else if cmd || ctrl || (alt && !telemetry.active.load(Ordering::Acquire)) {
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
            shift && !cmd && !ctrl,
            |x, y| telemetry.on_scrawl_window(x, y),
        );

        let inside_target = telemetry.contains_screen_point(loc.x, loc.y);
        let pure_shift = shift && !cmd && !ctrl && !await_shift_release;
        // Option on top of Shift turns the stroke into a straight line from where it began.
        let straight_now = pure_shift && alt;
        if !straight_now {
            straight_from = None;
        }

        // Hold Shift + move cursor inside the capture target = draw a persistent stroke.
        if pure_shift && inside_target {
            telemetry.shift_held.store(true, Ordering::Release);
            let t_ms = telemetry.elapsed_ms();
            let (nx, ny, fw, fh) = telemetry.normalize_point(loc.x, loc.y);
            let sx = (nx * SCALE as f32).round() as i32;
            let sy = (ny * SCALE as f32).round() as i32;
            let mut guard = telemetry
                .live_ink
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if guard.is_none() {
                telemetry.window_log_request.store(true, Ordering::Release);
            }
            let stroke = guard.get_or_insert_with(|| Stroke {
                t0: t_ms,
                t1: t_ms,
                fw,
                fh,
                on: None,
                pin: None,
                laser: false,
                pts: Vec::new(),
            });
            let dt = t_ms.saturating_sub(stroke.t0) as i32;
            if straight_now {
                let anchor = *straight_from
                    .get_or_insert_with(|| (stroke.pts.len() / 3).saturating_sub(1));
                stroke.push_live_point((sx, sy), dt, Some(anchor));
                stroke.t1 = t_ms;
            } else {
                let should_push = stroke.pts.chunks_exact(3).last().is_none_or(|prev| {
                    let dx = (sx - prev[0]) as f32 / SCALE as f32 * fw as f32;
                    let dy = (sy - prev[1]) as f32 / SCALE as f32 * fh as f32;
                    dx.hypot(dy) >= 1.5
                });
                if should_push && stroke.pts.len() / 3 < 2_400 {
                    stroke.push_live_point((sx, sy), dt, None);
                    stroke.t1 = t_ms;
                }
            }
        } else if !pure_shift && prev_shift_ink {
            telemetry.shift_held.store(false, Ordering::Release);
            if let Some(stroke) = telemetry
                .live_ink
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
                && stroke.pts.len() >= 6
            {
                let first = &stroke.pts[0..2];
                let max_span_px = stroke
                    .pts
                    .chunks_exact(3)
                    .map(|p| {
                        let dx = (p[0] - first[0]) as f32 / SCALE as f32 * stroke.fw as f32;
                        let dy = (p[1] - first[1]) as f32 / SCALE as f32 * stroke.fh as f32;
                        dx.hypot(dy)
                    })
                    .fold(0.0_f32, f32::max);
                // Ignore stationary Shift taps (e.g. typing a capital letter without moving the mouse).
                if max_span_px >= 6.0 {
                    telemetry
                        .strokes
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(stroke);
                    let _ = telemetry
                        .ui_sender
                        .try_send(LiveOverlayEvent::RequestSnapshot);
                }
            }
        }

        prev_shift_ink = pure_shift;
    }
}

fn shortcut_key_name(keycode: u16) -> Option<&'static str> {
    match keycode {
        0 => Some("A"),
        1 => Some("S"),
        2 => Some("D"),
        3 => Some("F"),
        6 => Some("Z"),
        7 => Some("X"),
        8 => Some("C"),
        9 => Some("V"),
        11 => Some("B"),
        12 => Some("Q"),
        13 => Some("W"),
        14 => Some("E"),
        15 => Some("R"),
        17 => Some("T"),
        31 => Some("O"),
        35 => Some("P"),
        36 => Some("↩"),
        40 => Some("K"),
        45 => Some("N"),
        48 => Some("⇥"),
        49 => Some("Space"),
        51 => Some("⌫"),
        _ => None,
    }
}

/// Writes all bundle artifacts (`scrawl.json`, `summary.md`, `contact-sheet.png`, `frames/frame-XX.png`)
/// into a new or specified `.scrawl` bundle directory.
pub fn write_bundle_artifacts(
    out_dir: &Path,
    bundle: &crate::model::ScrawlBundle,
    annotated_frames: &[(crate::model::Keyframe, RgbaFrame)],
    contact_sheet: &RgbaFrame,
    summary_md: &str,
) -> Result<()> {
    std::fs::create_dir_all(out_dir.join("frames"))
        .with_context(|| format!("could not create {}", out_dir.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(out_dir, std::fs::Permissions::from_mode(0o700));
    }

    thread::scope(|scope| -> Result<()> {
        let workers: Vec<_> = annotated_frames
            .iter()
            .map(|(kf, frame)| {
                scope.spawn(move || -> Result<()> {
                    crate::render::save_png(frame, &out_dir.join(&kf.file))
                })
            })
            .collect();
        crate::render::save_png(contact_sheet, &out_dir.join(&bundle.contact_sheet))?;
        for worker in workers {
            worker
                .join()
                .map_err(|_| anyhow::anyhow!("PNG encoding worker panicked"))??;
        }
        Ok(())
    })?;

    let manifest_bytes = serde_json::to_vec_pretty(bundle)?;
    std::fs::write(out_dir.join("scrawl.json"), manifest_bytes)?;
    std::fs::write(out_dir.join("summary.md"), summary_md.as_bytes())?;

    Ok(())
}
