use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

use crate::log::scrawl_log;
use crate::model::{
    CaptureMode, InteractionEvent, RgbaFrame, SCALE, Stroke, TargetFrame, TargetInfo,
};

// The operating system side of capture: global shortcuts, input monitoring, the frontmost app,
// displays and windows, and the screen recorder. Each platform module provides the same items.
#[cfg_attr(target_os = "macos", path = "capture/macos.rs")]
#[cfg_attr(target_os = "windows", path = "capture/windows.rs")]
mod platform;
pub use platform::*;

pub const FRAME_SAMPLE_INTERVAL: Duration = Duration::from_millis(220);
/// Candidate frames only feed scene diffs and keyframe selection; the keyframes themselves are
/// decoded from `recording.mp4` at full size afterwards.
pub const CANDIDATE_EDGE: f64 = 480.0;
/// Without a recording to decode from, the candidates are the keyframes, so keep them sharper.
pub(crate) const CANDIDATE_EDGE_NO_VIDEO: f64 = 1280.0;
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

    /// Forgets frames taken after `t_ms`, except the first.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub fn drop_since(&mut self, t_ms: u64) {
        let mut index = 0;
        self.frames.retain(|(frame, _)| {
            index += 1;
            index == 1 || frame.t_ms <= t_ms
        });
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

/// Live engine events emitted by the global input monitor, iPhone mirror and ingest server.
#[derive(Clone, Debug)]
pub enum Event {
    /// ⌥⇧S (or the configured shortcut) went down; `shortcut::Shortcut` decides what it means.
    CaptureShortcut,
    PasteLatestShortcut,
    UndoStroke,
    RequestSnapshot,
    /// The iPhone mirror ended on its own (unplugged, never found, or access denied).
    PhoneEnded,
    /// A ScrawlKit recording was ingested from an iPhone and compiled into a `.scrawl` bundle.
    IngestedBundle {
        dir: PathBuf,
        bundle: Box<crate::model::ScrawlBundle>,
        status: Option<String>,
    },
}

// Carbon HIToolbox modifier bits. The shortcut is stored with them on every platform.
pub const CARBON_CMD_KEY: u32 = 1 << 8;
pub const CARBON_SHIFT_KEY: u32 = 1 << 9;
pub const CARBON_OPTION_KEY: u32 = 1 << 11;
pub const CARBON_CONTROL_KEY: u32 = 1 << 12;

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

    pub(crate) fn ansi_code(self) -> u16 {
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

    /// Whether it can be turned off. Record can't: it's the way into Scrawl.
    pub fn can_unset(self) -> bool {
        self == Self::Paste
    }
}

impl ShortcutConfig {
    pub fn default_record() -> Self {
        Self {
            key_code: u32::from(layout_s_keycode()),
            carbon_modifiers: CARBON_OPTION_KEY | CARBON_SHIFT_KEY,
            label: shortcut_label(CARBON_OPTION_KEY | CARBON_SHIFT_KEY, "S"),
        }
    }

    pub fn default_paste() -> Self {
        Self {
            key_code: u32::from(layout_v_keycode()),
            carbon_modifiers: CARBON_OPTION_KEY | CARBON_SHIFT_KEY,
            label: shortcut_label(CARBON_OPTION_KEY | CARBON_SHIFT_KEY, "V"),
        }
    }

    pub fn defaults() -> (Self, Self) {
        (Self::default_record(), Self::default_paste())
    }

    /// No shortcut: nothing is registered and no key press matches. Only the paste shortcut can
    /// be turned off.
    pub fn unset() -> Self {
        Self {
            key_code: u32::MAX,
            carbon_modifiers: 0,
            label: String::new(),
        }
    }

    pub fn is_unset(&self) -> bool {
        self.label.is_empty()
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

/// `⌃⌥⇧⌘` in the order macOS writes them, for Carbon modifier bits; on Windows the same
/// modifiers as `Ctrl+Win+Alt+Shift+` (ready to take a key).
pub fn modifier_symbols(carbon_modifiers: u32) -> String {
    if cfg!(target_os = "windows") {
        let label = shortcut_label(carbon_modifiers, "");
        return label;
    }
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

/// A shortcut as this platform writes it: `⌥⇧S` on macOS, `Alt+Shift+S` on Windows.
pub fn shortcut_label(carbon_modifiers: u32, key: &str) -> String {
    let held = |bit: u32| carbon_modifiers & bit != 0;
    if cfg!(target_os = "windows") {
        let mut parts = Vec::new();
        for (bit, name) in [
            (CARBON_CONTROL_KEY, "Ctrl"),
            (CARBON_CMD_KEY, "Win"),
            (CARBON_OPTION_KEY, "Alt"),
            (CARBON_SHIFT_KEY, "Shift"),
        ] {
            if held(bit) {
                parts.push(name);
            }
        }
        parts.push(key);
        parts.join("+")
    } else {
        let mut label = String::new();
        for (bit, glyph) in [
            (CARBON_CONTROL_KEY, '⌃'),
            (CARBON_OPTION_KEY, '⌥'),
            (CARBON_SHIFT_KEY, '⇧'),
            (CARBON_CMD_KEY, '⌘'),
        ] {
            if held(bit) {
                label.push(glyph);
            }
        }
        label.push_str(key);
        label
    }
}

/// The system paste keystroke: `⌘V` on macOS, `Ctrl+V` on Windows.
pub fn system_paste_label() -> &'static str {
    if cfg!(target_os = "windows") {
        "Ctrl+V"
    } else {
        "⌘V"
    }
}

/// The keys of a shortcut label, one per keycap: `⌥⇧S` → `⌥`, `⇧`, `S`; `Alt+Shift+S` →
/// `Alt`, `Shift`, `S`.
pub fn shortcut_keys(label: &str) -> Vec<String> {
    if label.contains('+') && label.len() > 1 {
        let mut keys: Vec<String> = label
            .split('+')
            .filter(|key| !key.is_empty())
            .map(str::to_owned)
            .collect();
        // `Ctrl++` binds the plus key itself.
        if label.ends_with("++") {
            keys.push("+".into());
        }
        keys
    } else {
        label.chars().map(String::from).collect()
    }
}

/// A shortcut label split into its modifiers and key: `⌥⇧S` → (`⌥⇧`, `S`), and on Windows
/// `Alt+Shift+S` → (`Alt+Shift`, `S`).
pub fn shortcut_parts(label: &str) -> (&str, &str) {
    // The key itself may be "+" (`Ctrl++`), or a letter outside ASCII.
    let before_key = label.char_indices().last().map_or("", |(i, _)| &label[..i]);
    if let Some(plus) = before_key.rfind('+') {
        return (&label[..plus], &label[plus + 1..]);
    }
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
    if cfg.is_unset() {
        return false;
    }
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

/// The active layout's key for `kind`'s default letter (Windows' hooks and poller match on it).
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn layout_default_keycode(kind: ShortcutKind) -> u16 {
    key_code_for_char(kind.default_char()).unwrap_or(kind.ansi_code())
}

/// The key codes and Carbon modifiers `cfg` registers: itself, plus for a default its aliases
/// (⌃⌥⇧, ⌘⌥⇧, ⌃⌥⌘) on the active layout's key and the ANSI one.
pub(crate) fn hotkey_combos(kind: ShortcutKind, cfg: &ShortcutConfig) -> Vec<(u32, u32)> {
    if cfg.is_unset() {
        return Vec::new();
    }
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

pub fn selectable_window_at(windows: &[TargetInfo], x: f64, y: f64) -> Option<&TargetInfo> {
    windows.iter().find(|window| {
        let f = window.frame;
        x >= f.x && x <= f.x + f.width && y >= f.y && y <= f.y + f.height
    })
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
    // One display per recording: the area's (by its centre), trimmed to that display and at
    // least `MIN_AREA` a side inside it, so this frame is exactly the recorded `sourceRect`.
    let screens = crate::screens::displays();
    let (display_id, scale, [x, y, width, height]) =
        crate::screens::area_on_display(&screens, [x, y, width, height]).map_or_else(
            || {
                let min = crate::screens::MIN_AREA;
                let rect = [x, y, width.max(min), height.max(min)];
                (main_display_id(), MAX_CAPTURE_SCALE, rect)
            },
            |(i, rect)| (screens[i].id, screens[i].scale, rect),
        );
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
            width,
            height,
            scale,
        },
    }
}

/// The capture size for `target` with its long edge at most `long_edge` px (and at most its
/// display's pixels: a 1x display isn't upscaled, and on Windows frames are pixels already,
/// `MAX_CAPTURE_SCALE`).
pub fn capture_size(target: &TargetInfo, long_edge: f64) -> (u32, u32) {
    let max_dim = target.frame.width.max(target.frame.height).max(1.0);
    let native = target.frame.scale.clamp(1.0, MAX_CAPTURE_SCALE);
    let scale = (long_edge / max_dim).min(native);
    (
        ((target.frame.width * scale).round() as u32).max(48) & !1,
        ((target.frame.height * scale).round() as u32).max(48) & !1,
    )
}

/// What the platform's sampler thread records into. It runs until `stopping`, pushes candidate
/// frames (at once when `urgent`), stores when the movie started into `video_offset_ms`, and
/// returns the movie it wrote.
pub(crate) struct Sampler {
    pub target: TargetInfo,
    pub video_output_path: Option<PathBuf>,
    /// Scrawl's own windows to leave out of the recording.
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    pub excluded_window_ids: Arc<Mutex<Vec<u32>>>,
    pub started_at: Instant,
    pub stopping: Arc<AtomicBool>,
    pub urgent: Arc<AtomicBool>,
    pub frames: Arc<Mutex<CandidateBuffer>>,
    pub video_offset_ms: Arc<AtomicU64>,
}

/// Active screen + optional MP4 recorder (ScreenCaptureKit on macOS, Windows.Graphics.Capture
/// and Media Foundation on Windows).
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
                platform::run_sampler(Sampler {
                    target: sampler_target,
                    video_output_path,
                    excluded_window_ids,
                    started_at,
                    stopping: thread_stopping,
                    urgent: thread_urgent,
                    frames: thread_frames,
                    video_offset_ms: thread_offset,
                })
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

/// Shared state collected by the global input monitor during an active capture session.
/// Monotonic boot-time nanoseconds (`mach_absolute_time` scaled by `mach_timebase_info` on macOS,
/// matching CoreAudio and CGEvent timestamps).
pub fn boot_time_ns() -> u64 {
    #[cfg(target_os = "macos")]
    {
        #[repr(C)]
        struct MachTimebaseInfo {
            numer: u32,
            denom: u32,
        }
        unsafe extern "C" {
            fn mach_absolute_time() -> u64;
            fn mach_timebase_info(info: *mut MachTimebaseInfo) -> i32;
        }
        static TIMEBASE: std::sync::OnceLock<(u32, u32)> = std::sync::OnceLock::new();
        let &(numer, denom) = TIMEBASE.get_or_init(|| {
            let mut info = MachTimebaseInfo { numer: 0, denom: 0 };
            let status = unsafe { mach_timebase_info(&mut info) };
            if status == 0 && info.denom != 0 {
                (info.numer, info.denom)
            } else {
                (1, 1)
            }
        });
        let ticks = unsafe { mach_absolute_time() };
        let nanos = u128::from(ticks) * u128::from(numer) / u128::from(denom);
        u64::try_from(nanos).unwrap_or(u64::MAX)
    }
    #[cfg(not(target_os = "macos"))]
    {
        static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
        EPOCH.get_or_init(Instant::now).elapsed().as_nanos() as u64
    }
}

#[derive(Clone, Debug)]
pub struct InkSnapshot {
    pub target: TargetFrame,
    pub strokes: Vec<Stroke>,
    pub live: Option<Stroke>,
    pub laser: Option<Stroke>,
    pub glow: bool,
}

/// A raw input event on the capture's clock, in global top-left screen points.
#[derive(Clone, Debug, PartialEq)]
pub enum InputEvent {
    ButtonDown {
        t_ms: u64,
        x: f64,
        y: f64,
        right: bool,
        shift_only: bool,
    },
    ButtonUp {
        t_ms: u64,
        x: f64,
        y: f64,
        right: bool,
        shift_only: bool,
    },
    Scroll {
        t_ms: u64,
        x: f64,
        y: f64,
        dx: f64,
        dy: f64,
    },
    Move {
        t_ms: u64,
        x: f64,
        y: f64,
        shift_only: bool,
    },
    Laser {
        t_ms: u64,
        x: f64,
        y: f64,
    },
    KeyChord {
        t_ms: u64,
        shortcut: String,
    },
    Flag {
        t_ms: u64,
    },
    UndoStroke,
}

/// Whether `shift` is held alone without other modifiers and is not left over from a multi-modifier
/// chord (such as the capture shortcut `⌥⇧S`) before `shift` was released.
pub fn pure_shift_state(
    shift: bool,
    other_modifiers: bool,
    await_shift_release: &mut bool,
) -> bool {
    if !shift {
        *await_shift_release = false;
    } else if other_modifiers {
        *await_shift_release = true;
    }
    shift && !other_modifiers && !*await_shift_release
}

/// Ink and pointer input in global top-left points, ms since capture start.
#[derive(Clone)]
pub struct InputSink {
    telemetry: Arc<SharedCaptureTelemetry>,
}

impl Default for InputSink {
    fn default() -> Self {
        let (tx, _rx) = flume::unbounded();
        Self {
            telemetry: SharedCaptureTelemetry::new(tx, ShortcutConfig::defaults()),
        }
    }
}

impl InputSink {
    pub fn new(telemetry: Arc<SharedCaptureTelemetry>) -> Self {
        Self { telemetry }
    }

    pub fn telemetry(&self) -> &Arc<SharedCaptureTelemetry> {
        &self.telemetry
    }

    pub fn send(&self, event: InputEvent) {
        self.telemetry.handle_input(event);
    }

    pub fn button_down(&self, t_ms: u64, x: f64, y: f64, right: bool, shift_only: bool) {
        self.send(InputEvent::ButtonDown {
            t_ms,
            x,
            y,
            right,
            shift_only,
        });
    }

    pub fn button_up(&self, t_ms: u64, x: f64, y: f64, right: bool, shift_only: bool) {
        self.send(InputEvent::ButtonUp {
            t_ms,
            x,
            y,
            right,
            shift_only,
        });
    }

    pub fn pointer_down(&self, t_ms: u64, x: f64, y: f64) {
        let shift_only = self.telemetry.shift_held.load(Ordering::Acquire);
        self.button_down(t_ms, x, y, false, shift_only);
    }

    pub fn pointer_up(&self, t_ms: u64, x: f64, y: f64, right: bool) {
        let shift_only = self.telemetry.shift_held.load(Ordering::Acquire);
        self.button_up(t_ms, x, y, right, shift_only);
    }

    pub fn shift_down(&self, t_ms: u64, x: f64, y: f64) {
        self.button_down(t_ms, x, y, false, true);
    }

    pub fn shift_up(&self, t_ms: u64, x: f64, y: f64) -> bool {
        self.telemetry.shift_up(t_ms, x, y)
    }

    pub fn scroll(&self, t_ms: u64, x: f64, y: f64, dx: f64, dy: f64) {
        self.send(InputEvent::Scroll { t_ms, x, y, dx, dy });
    }

    pub fn ink(&self, t_ms: u64, x: f64, y: f64, shift_held: bool) {
        self.send(InputEvent::Move {
            t_ms,
            x,
            y,
            shift_only: shift_held,
        });
    }

    pub fn laser(&self, t_ms: u64, x: f64, y: f64) {
        self.send(InputEvent::Laser { t_ms, x, y });
    }

    pub fn key_chord(&self, t_ms: u64, shortcut: impl Into<String>) {
        self.send(InputEvent::KeyChord {
            t_ms,
            shortcut: shortcut.into(),
        });
    }

    pub fn flag(&self, t_ms: u64) {
        self.send(InputEvent::Flag { t_ms });
    }

    pub fn undo_stroke(&self) -> bool {
        self.telemetry.undo_last_stroke()
    }

    pub fn snapshot(&self) -> InkSnapshot {
        self.telemetry.ink_snapshot()
    }
}

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
    pub started_at_boot_ns: AtomicU64,
    pub picker_started_at: Mutex<Instant>,
    pub picker_rect: Mutex<Option<([f32; 4], u8)>>,
    pub target_frame: Mutex<TargetFrame>,
    pub pointer_screen: Mutex<[f32; 2]>,
    pub events: Mutex<Vec<InteractionEvent>>,
    pub strokes: Mutex<Vec<Stroke>>,
    pub flags: Mutex<Vec<u64>>,
    pub active_laser: Mutex<Option<Stroke>>,
    pub live_ink: Mutex<Option<Stroke>>,
    pub excluded_window_ids: Arc<Mutex<Vec<u32>>>,
    /// Raised when a click, ink stroke or shortcut starts, so the Screen-mode window log takes
    /// a snapshot right away.
    pub window_log_request: Arc<AtomicBool>,
    pub ui_sender: flume::Sender<Event>,
    /// Scrawl's windows that let clicks through (the ink overlay), skipped when deciding whether
    /// a click landed on Scrawl itself.
    pub passthrough_window_ids: Mutex<Vec<u32>>,
    /// The `scrawl-helper` whose window is what's being recorded (the iPhone mirror): its clicks
    /// count, unlike those on other helper windows (Settings, onboarding). 0 when none.
    capture_source_pid: AtomicI32,
    /// The last `on_scrawl_window` answer (when, where, what), reused for a burst of scrolls.
    scrawl_hit: Mutex<Option<(Instant, f64, f64, bool)>>,
    /// macOS's key event tap (`CFMachPortRef`), made once Accessibility is on (`check_key_tap`).
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    tap_port: AtomicU64,
    /// The key event tap thread's run loop (retained), so `check_key_tap` can stop it.
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    tap_run_loop: AtomicU64,
    /// A key tap thread is being started.
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    tap_starting: AtomicBool,
    /// When the last key tap was tried (ms since `UNIX_EPOCH`), to space out retries.
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    tap_tried_ms: AtomicU64,
    /// Whether key presses are arriving (macOS's key tap, Windows' keyboard hook).
    tap_live: AtomicBool,
    /// Esc went down since the UI loop last looked (`take_escape`).
    escape_pressed: AtomicBool,
    /// A Screen recording started straight from the shortcut: the overlay flashes an edge glow.
    pub start_glow: AtomicBool,
    /// Clicks are arriving (macOS's global mouse monitor, Windows' mouse hook). Until then the
    /// poller records clicks from polled button state.
    mouse_live: AtomicBool,
    last_shortcut_ms: AtomicU64,
    drag_start: Mutex<Option<(u64, f32, f32, Option<String>, TargetFrame)>>,
    /// The `agentContext` experiment was on when this capture started: clicks and strokes are
    /// looked up in the Accessibility tree (`crate::ax`, macOS only).
    agent_context: AtomicBool,
    /// A ⇧-press inside the target: when, where (global points) and on which frame.
    shift_press: Mutex<Option<(u64, f64, f64, TargetFrame)>>,
}

impl SharedCaptureTelemetry {
    pub fn new(
        ui_sender: flume::Sender<Event>,
        shortcuts: (ShortcutConfig, ShortcutConfig),
    ) -> Arc<Self> {
        let (record, paste) = shortcuts;
        Arc::new(Self {
            active: AtomicBool::new(false),
            selecting: AtomicBool::new(false),
            shift_held: AtomicBool::new(false),
            capture_mode: AtomicU64::new(0),
            shortcut: Mutex::new(record),
            paste_shortcut: Mutex::new(paste),
            shortcuts_paused: AtomicBool::new(false),
            started_at: Mutex::new(Instant::now()),
            started_at_boot_ns: AtomicU64::new(boot_time_ns()),
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
            flags: Mutex::new(Vec::new()),
            active_laser: Mutex::new(None),
            live_ink: Mutex::new(None),
            excluded_window_ids: Arc::new(Mutex::new(Vec::new())),
            window_log_request: Arc::new(AtomicBool::new(false)),
            ui_sender,
            passthrough_window_ids: Mutex::new(Vec::new()),
            capture_source_pid: AtomicI32::new(0),
            scrawl_hit: Mutex::new(None),
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
            agent_context: AtomicBool::new(false),
            shift_press: Mutex::new(None),
        })
    }

    /// Each source (Carbon or `RegisterHotKey`, the key tap or hook) reports the same press within
    /// a few ms; a press after that is a new one, as when S is pressed again to switch modes.
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
            let _ = self.ui_sender.try_send(Event::CaptureShortcut);
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
            let _ = self.ui_sender.try_send(Event::PasteLatestShortcut);
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

    pub fn started_at_boot_ns(&self) -> u64 {
        self.started_at_boot_ns.load(Ordering::Acquire)
    }

    pub fn reset_for_session(&self, frame: TargetFrame) {
        self.reset_for_session_at(frame, None);
    }

    pub fn reset_for_session_at(&self, frame: TargetFrame, t0_boot_ns: Option<u64>) {
        self.selecting.store(false, Ordering::Release);
        *self
            .picker_rect
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        let now_boot_ns = boot_time_ns();
        let boot_ns = t0_boot_ns.unwrap_or(now_boot_ns);
        self.started_at_boot_ns.store(boot_ns, Ordering::Release);
        let started_instant = Instant::now()
            .checked_sub(Duration::from_nanos(now_boot_ns.saturating_sub(boot_ns)))
            .unwrap_or_else(Instant::now);
        *self
            .started_at
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = started_instant;
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
        self.flags
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
        *self
            .shift_press
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        self.excluded_window_ids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        self.shift_held.store(false, Ordering::Release);
        self.window_log_request.store(false, Ordering::Release);
        let agent_context = crate::handoff::experiment(crate::handoff::AGENT_CONTEXT);
        self.agent_context.store(agent_context, Ordering::Release);
        if agent_context {
            crate::ax::Prober::shared().begin();
        }
        self.active.store(true, Ordering::Release);
        let clicks = if self.mouse_live.load(Ordering::Acquire) {
            platform::MOUSE_SOURCE
        } else {
            "polled mouse buttons"
        };
        let keys = if self.tap_live.load(Ordering::Acquire) {
            platform::KEY_SOURCE
        } else {
            platform::NO_KEY_SOURCE
        };
        scrawl_log(format!("session clicks from {clicks}, keys from {keys}"));
    }

    /// Whether Esc went down since the last call (a pending shortcut or an open picker cancels on it).
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

    /// Whether to look up what clicks and strokes land on: `agentContext` is on, the target is
    /// this Mac's screen (not the iPhone mirror) and Accessibility is granted.
    fn probing(&self) -> bool {
        self.agent_context.load(Ordering::Acquire)
            && self.capture_mode.load(Ordering::Acquire) != 3
            && accessibility_granted()
    }

    /// A mouse button went down at a global point.
    pub fn pointer_down(&self, t_ms: u64, screen_x: f64, screen_y: f64) {
        if !self.contains_screen_point(screen_x, screen_y) {
            return;
        }
        #[cfg(target_os = "macos")]
        {
            let raw = objc2::rc::autoreleasepool(|_| crate::windows::raw_windows());
            if crate::windows::system_prompt_at(&raw, screen_x, screen_y) {
                return;
            }
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
        if self.probing() {
            crate::ax::Prober::shared().probe(crate::ax::Target::Pointer(t_ms), screen_x, screen_y);
        }
    }

    /// A mouse button came up: records the click or drag that `pointer_down` began.
    pub fn pointer_up(&self, t_ms: u64, screen_x: f64, screen_y: f64, right: bool) {
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
        let _ = self.ui_sender.try_send(Event::RequestSnapshot);
        let event = if dist_px < CLICK_SLOP_PT {
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

    /// The left button went down with only ⇧ held: a pin if it comes up as a click (`shift_up`).
    pub fn shift_down(&self, t_ms: u64, screen_x: f64, screen_y: f64) {
        if !self.contains_screen_point(screen_x, screen_y) {
            return;
        }
        let frame = *self
            .target_frame
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *self
            .shift_press
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            Some((t_ms, screen_x, screen_y, frame));
    }

    /// The button of a ⇧-press came up (with or without ⇧ still held). A ⇧-click drops the next
    /// numbered pin where it went down, unless ink was already drawn in this ⇧ hold: a click in
    /// the middle of a drawing belongs to the drawing, so it neither pins nor counts as a click.
    /// A ⇧-drag is left to the ink the poller is drawing. Returns whether a ⇧-press was waiting,
    /// so the release isn't also taken as a click's.
    pub fn shift_up(&self, t_ms: u64, screen_x: f64, screen_y: f64) -> bool {
        let Some((t0, x0, y0, start_frame)) = self
            .shift_press
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
        else {
            return false;
        };
        let frame = *self
            .target_frame
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if !same_target_frame(start_frame, frame)
            || !is_pin_click((t0, x0, y0), (t_ms, screen_x, screen_y))
        {
            return true;
        }
        {
            let mut live = self
                .live_ink
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if live.as_ref().is_some_and(|ink| drew_before(ink, t0)) {
                return true;
            }
            // ⇧ draws as it's held, so the live ink is only the click's jitter; further ink
            // starts a new stroke.
            *live = None;
        }
        let (nx, ny, fw, fh) = self.normalize_point(x0, y0);
        let mut strokes = self
            .strokes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let number = strokes.iter().filter(|s| s.pin.is_some()).count() as u32 + 1;
        strokes.push(Stroke {
            t0,
            t1: t_ms,
            fw,
            fh,
            on: None,
            pin: Some(number),
            laser: false,
            pts: vec![
                (nx * SCALE as f32).round() as i32,
                (ny * SCALE as f32).round() as i32,
                0,
            ],
        });
        drop(strokes);
        self.window_log_request.store(true, Ordering::Release);
        let _ = self.ui_sender.try_send(Event::RequestSnapshot);
        true
    }

    /// The helper process whose window is the capture target (the iPhone mirror).
    #[cfg_attr(target_os = "windows", allow(dead_code))]
    pub fn set_capture_source_pid(&self, pid: u32) {
        self.capture_source_pid
            .store(i32::try_from(pid).unwrap_or(0), Ordering::Release);
    }

    /// Whether the topmost window at a global point is Scrawl's own or a `scrawl-helper`'s other
    /// than the capture source (ignoring the click-through ink overlay). Asked once per press, and
    /// for scrolls, which come in bursts at one point, at most every 250 ms.
    fn on_scrawl_window(&self, x: f64, y: f64) -> bool {
        let mut hit = self
            .scrawl_hit
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some((at, hx, hy, on)) = *hit
            && at.elapsed() < Duration::from_millis(250)
            && (hx - x).abs() <= 2.0
            && (hy - y).abs() <= 2.0
        {
            return on;
        }
        let on = self.scrawl_window_in(
            &platform::raw_windows_now(),
            x,
            y,
            platform::is_helper_process,
        );
        *hit = Some((Instant::now(), x, y, on));
        on
    }

    fn scrawl_window_in(
        &self,
        windows: &[crate::windows::RawWindow],
        x: f64,
        y: f64,
        is_helper: impl Fn(i32) -> bool,
    ) -> bool {
        let passthrough = self
            .passthrough_window_ids
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let own = std::process::id() as i32;
        let source = self.capture_source_pid.load(Ordering::Acquire);
        crate::windows::owned_window_at(windows, x, y, &passthrough, |pid| {
            pid == own || (pid != source && is_helper(pid))
        })
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
        *self
            .shift_press
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
        let mut events = std::mem::take(
            &mut *self
                .events
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        let mut strokes = std::mem::take(
            &mut *self
                .strokes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
        if self.agent_context.swap(false, Ordering::AcqRel) {
            let hits = crate::ax::Prober::shared().finish();
            crate::ax::apply(&hits, &mut events, &mut strokes);
            if self.capture_mode.load(Ordering::Acquire) != 3 {
                let end = crate::pipeline::now_epoch_ms();
                let start = end.saturating_sub(self.elapsed_ms());
                let page = crate::page::between(start, end);
                crate::page::apply(&page, start, &mut events, &mut strokes);
            }
        }
        (events, strokes)
    }

    pub fn handle_input(&self, event: InputEvent) {
        self.handle_input_with(event, |x, y| self.on_scrawl_window(x, y));
    }

    pub fn handle_input_with(&self, event: InputEvent, on_own_window: impl Fn(f64, f64) -> bool) {
        match event {
            // A ⇧-press is never a click: a ⇧-click drops a pin, a ⇧-drag is ink.
            InputEvent::ButtonDown {
                t_ms,
                x,
                y,
                right,
                shift_only,
            } => {
                if !on_own_window(x, y) {
                    if !shift_only {
                        self.pointer_down(t_ms, x, y);
                    } else if !right {
                        self.shift_down(t_ms, x, y);
                    }
                }
            }
            InputEvent::ButtonUp {
                t_ms,
                x,
                y,
                right,
                shift_only,
            } => {
                let shift_press = !right && self.shift_up(t_ms, x, y);
                if !shift_press && !shift_only {
                    self.pointer_up(t_ms, x, y, right);
                }
            }
            InputEvent::Scroll { t_ms, x, y, dx, dy } => {
                if (dx.abs() > 0.0 || dy.abs() > 0.0) && !on_own_window(x, y) {
                    self.scroll_at(t_ms, x, y, dx as f32, dy as f32);
                }
            }
            InputEvent::Move {
                t_ms,
                x,
                y,
                shift_only,
            } => {
                *self
                    .pointer_screen
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = [x as f32, y as f32];
                self.track_ink_at(t_ms, (x, y), shift_only);
            }
            InputEvent::Laser { t_ms, x, y } => {
                self.laser_at(t_ms, x, y);
            }
            InputEvent::KeyChord { t_ms, shortcut } => {
                self.key_chord(t_ms, shortcut);
            }
            InputEvent::Flag { t_ms } => {
                self.flag_at(t_ms);
            }
            InputEvent::UndoStroke => {
                let _ = self.undo_last_stroke();
            }
        }
    }

    pub fn key_chord(&self, t_ms: u64, shortcut: String) {
        self.window_log_request.store(true, Ordering::Release);
        self.events
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(InteractionEvent::Key { t: t_ms, shortcut });
    }

    pub fn scroll_at(&self, t_ms: u64, screen_x: f64, screen_y: f64, dx: f32, dy: f32) {
        if (dx.abs() > 0.0 || dy.abs() > 0.0) && self.contains_screen_point(screen_x, screen_y) {
            let (nx, ny, _, _) = self.normalize_point(screen_x, screen_y);
            self.events
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

    pub fn track_ink_at(&self, t_ms: u64, (x, y): (f64, f64), shift_held: bool) {
        let prev = self.shift_held.load(Ordering::Acquire)
            || self
                .live_ink
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .is_some();
        track_shift_ink_at(self, t_ms, (x, y), shift_held, prev);
    }

    pub fn laser_at(&self, t_ms: u64, screen_x: f64, screen_y: f64) {
        if !self.contains_screen_point(screen_x, screen_y) {
            return;
        }
        let (nx, ny, fw, fh) = self.normalize_point(screen_x, screen_y);
        let sx = (nx * SCALE as f32).round() as i32;
        let sy = (ny * SCALE as f32).round() as i32;
        let mut guard = self
            .active_laser
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let stroke = guard.get_or_insert_with(|| Stroke {
            t0: t_ms,
            t1: t_ms,
            fw,
            fh,
            on: None,
            pin: None,
            laser: true,
            pts: Vec::new(),
        });
        let dt = t_ms.saturating_sub(stroke.t0) as i32;
        if stroke.pts.len() / 3 < 2_400 {
            stroke.pts.extend_from_slice(&[sx, sy, dt]);
            stroke.t1 = t_ms;
        }
    }

    pub fn flag_at(&self, t_ms: u64) {
        self.flags
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(t_ms);
        let _ = self.ui_sender.try_send(Event::RequestSnapshot);
    }

    pub fn take_flags(&self) -> Vec<u64> {
        std::mem::take(
            &mut *self
                .flags
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    pub fn undo_last_stroke(&self) -> bool {
        let mut live = self
            .live_ink
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if live.take().is_some() {
            return true;
        }
        drop(live);
        self.strokes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .pop()
            .is_some()
    }

    pub fn ink_snapshot(&self) -> InkSnapshot {
        let target = *self
            .target_frame
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let strokes = self
            .strokes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let live = self
            .live_ink
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let laser = self
            .active_laser
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let glow = self.start_glow.load(Ordering::Acquire);
        InkSnapshot {
            target,
            strokes,
            live,
            laser,
            glow,
        }
    }
}

/// Clicks from the mouse buttons' polled state, while the mouse monitor (or hook) is missing: a
/// press starts a click (unless it lands on Scrawl's own window), a release ends it. A ⇧-press may
/// be a pin, as in the mouse monitor's handler.
fn poll_clicks(
    telemetry: &SharedCaptureTelemetry,
    prev: &mut [bool; 2],
    now: [bool; 2],
    (x, y): (f64, f64),
    pure_shift: bool,
    on_scrawl: impl Fn(f64, f64) -> bool,
) {
    let was = std::mem::replace(prev, now);
    if telemetry.mouse_live.load(Ordering::Acquire) {
        return;
    }
    let t_ms = telemetry.elapsed_ms();
    for (button, (&down, &before)) in now.iter().zip(&was).enumerate() {
        let right = button != 0;
        if down && !before {
            telemetry.handle_input_with(
                InputEvent::ButtonDown {
                    t_ms,
                    x,
                    y,
                    right,
                    shift_only: pure_shift,
                },
                &on_scrawl,
            );
        } else if !down && before {
            telemetry.handle_input_with(
                InputEvent::ButtonUp {
                    t_ms,
                    x,
                    y,
                    right,
                    shift_only: pure_shift,
                },
                &on_scrawl,
            );
        }
    }
}

/// A press that moves less than this (points) before it's let go is a click, not a drag.
const CLICK_SLOP_PT: f32 = 12.0;
/// A ⇧-press held still for longer than this drops no pin.
const PIN_MAX_PRESS_MS: u64 = 1_000;
/// Ink spanning less than this (points) is a stationary ⇧ tap, not a stroke.
const MIN_INK_SPAN_PT: f32 = 6.0;

/// Whether a ⇧-press (`t`, global `x`, `y`) let go at `up` is a ⇧-click, which drops a pin.
fn is_pin_click(down: (u64, f64, f64), up: (u64, f64, f64)) -> bool {
    up.0.saturating_sub(down.0) <= PIN_MAX_PRESS_MS
        && ((up.1 - down.1).hypot(up.2 - down.2) as f32) < CLICK_SLOP_PT
}

/// Whether `stroke`'s points before `before_ms` reach at least `MIN_INK_SPAN_PT` from its start.
fn drew_before(stroke: &Stroke, before_ms: u64) -> bool {
    let Some(first) = stroke.pts.get(0..2) else {
        return false;
    };
    let mut points = 0_usize;
    let mut span = 0.0_f32;
    for p in stroke
        .pts
        .chunks_exact(3)
        .take_while(|p| stroke.t0.saturating_add(p[2].max(0) as u64) < before_ms)
    {
        points += 1;
        let dx = (p[0] - first[0]) as f32 / SCALE as f32 * stroke.fw as f32;
        let dy = (p[1] - first[1]) as f32 / SCALE as f32 * stroke.fh as f32;
        span = span.max(dx.hypot(dy));
    }
    points >= 2 && span >= MIN_INK_SPAN_PT
}

/// Whether finished ink is worth keeping as a stroke.
fn is_real_ink(stroke: &Stroke) -> bool {
    drew_before(stroke, u64::MAX)
}

fn same_target_frame(a: TargetFrame, b: TargetFrame) -> bool {
    (a.x - b.x).abs() <= 0.5
        && (a.y - b.y).abs() <= 0.5
        && (a.width - b.width).abs() <= 0.5
        && (a.height - b.height).abs() <= 0.5
}

/// One poll of the Shift ink: holding Shift alone and moving the cursor inside the capture
/// target draws a persistent stroke; letting go of Shift commits it. `pure_shift` is Shift
/// without other modifiers (and not left over from a chord such as the capture shortcut).
fn track_shift_ink(
    telemetry: &SharedCaptureTelemetry,
    (x, y): (f64, f64),
    pure_shift: bool,
    prev_shift_ink: bool,
) {
    track_shift_ink_at(
        telemetry,
        telemetry.elapsed_ms(),
        (x, y),
        pure_shift,
        prev_shift_ink,
    );
}

fn track_shift_ink_at(
    telemetry: &SharedCaptureTelemetry,
    t_ms: u64,
    (x, y): (f64, f64),
    pure_shift: bool,
    prev_shift_ink: bool,
) {
    let inside_target = telemetry.contains_screen_point(x, y);
    if pure_shift && inside_target {
        telemetry.shift_held.store(true, Ordering::Release);
        let (nx, ny, fw, fh) = telemetry.normalize_point(x, y);
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
        let should_push = stroke.pts.chunks_exact(3).last().is_none_or(|prev| {
            let dx = (sx - prev[0]) as f32 / SCALE as f32 * fw as f32;
            let dy = (sy - prev[1]) as f32 / SCALE as f32 * fh as f32;
            dx.hypot(dy) >= 1.5
        });
        if should_push && stroke.pts.len() / 3 < 2_400 {
            stroke.pts.extend_from_slice(&[sx, sy, dt]);
            stroke.t1 = t_ms;
        }
    } else if !pure_shift && prev_shift_ink {
        telemetry.shift_held.store(false, Ordering::Release);
        if let Some(stroke) = telemetry
            .live_ink
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
            // Ignore stationary Shift taps (e.g. typing a capital letter without moving the mouse).
            && is_real_ink(&stroke)
        {
            if telemetry.probing() {
                let frame = *telemetry
                    .target_frame
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some((x, y)) = crate::ax::stroke_probe_point(&stroke, frame) {
                    crate::ax::Prober::shared().probe(crate::ax::Target::Stroke(stroke.t0), x, y);
                }
            }
            telemetry
                .strokes
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push(stroke);
            let _ = telemetry.ui_sender.try_send(Event::RequestSnapshot);
        }
    }
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
        // Windows Terminal, the console host and the shells it runs.
        "windowsterminal",
        "conhost",
        "powershell",
        "pwsh",
        "cmd.exe",
        "mintty",
    ]
    .iter()
    .any(|needle| a.contains(needle) || b.contains(needle))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_stop_joins_worker_before_collecting_its_final_media() {
        let stopping = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(Mutex::new(CandidateBuffer::new(1 << 20)));
        let worker_stopping = Arc::clone(&stopping);
        let worker_frames = Arc::clone(&frames);
        let path = std::env::temp_dir().join(crate::pipeline::make_bundle_id());
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

    /// Clicks on `scrawl-helper`'s windows (the SwiftUI capsule, Settings, onboarding) are
    /// Scrawl's own, except on the iPhone mirror, which is what's being recorded.
    #[test]
    fn helper_windows_are_scrawl_s_except_the_capture_source() {
        let (sender, _receiver) = flume::unbounded();
        let telemetry = SharedCaptureTelemetry::new(sender, ShortcutConfig::defaults());
        let window = |id, pid, bounds| crate::windows::RawWindow {
            id,
            pid,
            owner: String::new(),
            title: String::new(),
            bounds,
            layer: 0,
            alpha: 1.0,
        };
        let own = std::process::id() as i32;
        let (settings, mirror, app) = (own + 1, own + 2, own + 3);
        let windows = [
            window(1, own, [0.0, 0.0, 1728.0, 1117.0]), // the ink overlay, click-through
            window(2, settings, [100.0, 100.0, 600.0, 400.0]),
            window(3, mirror, [1300.0, 100.0, 300.0, 650.0]),
            window(4, app, [0.0, 0.0, 1728.0, 1117.0]),
        ];
        telemetry.passthrough_window_ids.lock().unwrap().push(1);
        telemetry.set_capture_source_pid(mirror as u32);
        let helper = |pid| pid == settings || pid == mirror;
        let at = |x, y| telemetry.scrawl_window_in(&windows, x, y, helper);
        assert!(at(300.0, 300.0), "Settings");
        assert!(!at(1400.0, 400.0), "the iPhone mirror");
        assert!(!at(1000.0, 900.0), "the app under the ink overlay");
        telemetry.set_capture_source_pid(0);
        assert!(at(1400.0, 400.0), "a helper that isn't the source");
    }

    /// Kit's Screen capture on the Pro: the old listen-only event tap was off (no Input
    /// Monitoring), so his clicks on OpenCode's composer never reached the bundle. Clicks now
    /// come from the mouse monitor; polled button state stands in if it couldn't be installed.
    #[test]
    fn clicks_are_recorded_from_polled_buttons_while_the_mouse_monitor_is_missing() {
        let (sender, _receiver) = flume::unbounded();
        let telemetry = SharedCaptureTelemetry::new(sender, ShortcutConfig::defaults());
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
        // A ⇧-click is a pin, not a click.
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
            if u32::from(KEY_S) == default.key_code {
                4
            } else {
                8
            },
            "⌥⇧S and three aliases, on the layout's key and the ANSI one"
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
        #[cfg(target_os = "macos")]
        assert_eq!(
            modifier_symbols(CARBON_CMD_KEY | CARBON_CONTROL_KEY | CARBON_SHIFT_KEY),
            "⌃⇧⌘"
        );
        #[cfg(target_os = "windows")]
        assert_eq!(
            modifier_symbols(CARBON_CMD_KEY | CARBON_CONTROL_KEY | CARBON_SHIFT_KEY),
            "Ctrl+Win+Shift+"
        );

        assert_eq!(shortcut_parts("⌥⇧S"), ("⌥⇧", "S"));
        assert_eq!(shortcut_parts("⌃⌥⌘R"), ("⌃⌥⌘", "R"));
        assert_eq!(shortcut_parts("⌥⇧É"), ("⌥⇧", "É"));
        assert_eq!(shortcut_parts("Alt+Shift+S"), ("Alt+Shift", "S"));
        assert_eq!(shortcut_parts("Ctrl+Alt+Ä"), ("Ctrl+Alt", "Ä"));
        assert_eq!(shortcut_parts("Ctrl++"), ("Ctrl", "+"));
        assert_eq!(shortcut_parts(""), ("", ""));
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
        // A display left of the laptop, at negative x.
        let left = [[0.0, 0.0, 1728.0, 1117.0], [-1920.0, 37.0, 1920.0, 1080.0]];
        assert_eq!(pick_display(&left, Some([-5.0, 600.0]), None), 1);
        // One above it, at negative y.
        let above = [[0.0, 0.0, 1728.0, 1117.0], [0.0, -1440.0, 2560.0, 1440.0]];
        assert_eq!(pick_display(&above, Some([10.0, -1.0]), None), 1);
        assert_eq!(pick_display(&above, Some([10.0, 0.0]), None), 0);
    }

    /// Clicks and ink on a display left of, right of or above the laptop normalize into that
    /// display, and land on the recording's pixels at its own scale.
    #[test]
    fn input_on_an_external_display_normalizes_into_it() {
        let (sender, _receiver) = flume::unbounded();
        let telemetry = SharedCaptureTelemetry::new(sender, ShortcutConfig::defaults());
        let left = [-1920.0, 37.0, 1920.0, 1080.0];
        let right = [1728.0, 0.0, 1920.0, 1080.0];
        let above = [0.0, -1440.0, 2560.0, 1440.0];
        let small_left = [-1280.0, -100.0, 1280.0, 800.0];
        let below = [0.0, 1117.0, 1920.0, 1080.0];
        // (display, scale, global point, expected normalized point, recording size)
        let cases = [
            // 1x displays are recorded at their own pixels
            (left, 1.0, [-960.0, 577.0], (0.5, 0.5), (1920, 1080)),
            (right, 1.0, [2208.0, 270.0], (0.25, 0.25), (1920, 1080)),
            (below, 1.0, [480.0, 1387.0], (0.25, 0.25), (1920, 1080)),
            // 2x ones at the 2560 px long edge
            (above, 2.0, [640.0, -360.0], (0.25, 0.75), (2560, 1440)),
            (small_left, 2.0, [-320.0, 500.0], (0.75, 0.75), (2560, 1600)),
        ];
        for (rect, scale, point, expected, size) in cases {
            let frame = crate::screens::with_scale(rect, scale);
            let target = TargetInfo {
                mode: CaptureMode::Display,
                app: None,
                bundle_id: None,
                window_title: None,
                window_id: None,
                display_id: 2,
                frame,
            };
            // Windows frames are pixels already and never upscaled.
            if cfg!(target_os = "macos") {
                assert_eq!(capture_size(&target, 2560.0), size, "{rect:?}");
            }
            telemetry.reset_for_session(frame);
            assert!(telemetry.contains_screen_point(point[0], point[1]));
            // The laptop's centre is outside this target.
            assert!(!telemetry.contains_screen_point(864.0, 558.0));
            let (nx, ny, fw, fh) = telemetry.normalize_point(point[0], point[1]);
            assert_eq!((nx, ny), expected, "{rect:?}");
            assert_eq!((fw, fh), (rect[2] as u32, rect[3] as u32));
            // A click there is recorded at those coordinates.
            telemetry.pointer_down(10, point[0], point[1]);
            telemetry.pointer_up(20, point[0], point[1], false);
            let (events, _) = telemetry.finish_session();
            assert!(
                matches!(events[..], [InteractionEvent::Click { x, y, .. }] if (x, y) == (nx, ny)),
                "{events:?}"
            );
        }
        // In recording pixels: the left 1x display's centre is (960, 540).
        let frame = crate::screens::with_scale([-1920.0, 37.0, 1920.0, 1080.0], 1.0);
        telemetry.reset_for_session(frame);
        let (nx, ny, _, _) = telemetry.normalize_point(-960.0, 577.0);
        assert_eq!((nx * 1920.0, ny * 1080.0), (960.0, 540.0));
        // Above 2x: (640, -360) is a quarter in → (640, 1080) px of 2560×1440.
        let frame = crate::screens::with_scale([0.0, -1440.0, 2560.0, 1440.0], 2.0);
        telemetry.reset_for_session(frame);
        let (nx, ny, _, _) = telemetry.normalize_point(640.0, -360.0);
        assert_eq!((nx * 2560.0, ny * 1440.0), (640.0, 1080.0));
        telemetry.finish_session();
    }

    fn session(width: f64, height: f64) -> Arc<SharedCaptureTelemetry> {
        let (sender, _receiver) = flume::unbounded();
        let telemetry = SharedCaptureTelemetry::new(sender, ShortcutConfig::defaults());
        telemetry.reset_for_session(TargetFrame {
            x: 0.0,
            y: 0.0,
            width,
            height,
            scale: 2.0,
        });
        telemetry
    }

    #[test]
    fn a_long_still_shift_press_drops_no_pin() {
        let down = (1_000, 10.0, 10.0);
        let slop = f64::from(CLICK_SLOP_PT);
        assert!(is_pin_click(down, (1_150, 14.0, 13.0)));
        assert!(is_pin_click(down, (1_000 + PIN_MAX_PRESS_MS, 10.0, 10.0)));
        assert!(!is_pin_click(down, (2_500, 10.0, 10.0)));
        assert!(!is_pin_click(down, (1_100, 10.0, 10.0 + slop)));
    }

    /// The poller draws ink while ⇧ is held. A ⇧-click after ink was drawn in the same hold is
    /// part of the drawing: no pin, and the stroke carries on whole. A ⇧-click with nothing drawn
    /// first (only its own jitter) drops a pin and leaves no stroke.
    #[test]
    fn a_shift_click_mid_drawing_drops_no_pin() {
        let telemetry = session(1000.0, 1000.0);
        let ink = |pts: &[(i32, i32, i32)]| Stroke {
            t0: 0,
            t1: 0,
            fw: 1000,
            fh: 1000,
            on: None,
            pin: None,
            laser: false,
            pts: pts.iter().flat_map(|&(x, y, t)| [x, y, t]).collect(),
        };
        let press = telemetry.elapsed_ms() + 50;
        let p = press as i32;
        let s = SCALE;
        // A line drawn while ⇧ was held, then a click at its end.
        let line = ink(&[
            (s / 10, s / 2, 0),
            (s / 5, s / 2, p - 20),
            (s / 5 + s / 500, s / 2, p + 5),
        ]);
        *telemetry.live_ink.lock().unwrap() = Some(line.clone());
        telemetry.shift_down(press, 200.0, 500.0);
        assert!(telemetry.shift_up(press + 80, 201.0, 500.0));
        assert_eq!(telemetry.live_ink.lock().unwrap().as_ref(), Some(&line));
        // Letting go of ⇧ commits it as one stroke.
        let at = (200.0, 500.0);
        track_shift_ink(&telemetry, at, false, true);

        // Drawn long before the press still counts: the hold is a drawing.
        *telemetry.live_ink.lock().unwrap() = Some(ink(&[(s / 10, s / 10, 0), (s / 5, s / 10, 1)]));
        let t = telemetry.elapsed_ms() + 5_000;
        telemetry.shift_down(t, 200.0, 100.0);
        assert!(telemetry.shift_up(t + 60, 200.0, 100.0));
        *telemetry.live_ink.lock().unwrap() = None;

        // A ⇧-click with only its own few points of jitter is a pin.
        let jitter = ink(&[(s / 2, s / 2, 0), (s / 2 + 2, s / 2, 30)]);
        *telemetry.live_ink.lock().unwrap() = Some(jitter);
        let t = telemetry.elapsed_ms();
        telemetry.shift_down(t, 500.0, 500.0);
        assert!(telemetry.shift_up(t + 60, 500.0, 500.0));
        assert!(telemetry.live_ink.lock().unwrap().is_none());

        let (events, strokes) = telemetry.finish_session();
        assert!(events.is_empty(), "{events:?}");
        let kinds: Vec<_> = strokes.iter().map(|s| (s.pin, s.pts.len() / 3)).collect();
        assert_eq!(kinds, vec![(None, 3), (Some(1), 1)]);
    }

    #[test]
    fn shift_clicks_drop_pins_from_polled_buttons_too() {
        let telemetry = session(1000.0, 1000.0);
        let mut prev = [false; 2];
        // ⇧-clicks at a point, the second on Scrawl's own window.
        for (at, on_scrawl) in [((250.0, 750.0), false), ((400.0, 400.0), true)] {
            for buttons in [[true, false], [false, false]] {
                poll_clicks(&telemetry, &mut prev, buttons, at, true, |_, _| on_scrawl);
            }
        }
        let (events, strokes) = telemetry.finish_session();
        assert!(events.is_empty(), "{events:?}");
        assert_eq!(strokes.len(), 1);
        assert_eq!(strokes[0].pin, Some(1));
        assert_eq!(strokes[0].normalized_points(), vec![[0.25, 0.75]]);
    }

    #[test]
    fn input_sink_routes_clicks_pins_ink_and_chord_release() {
        let telemetry = session(1000.0, 1000.0);
        let sink = InputSink::new(Arc::clone(&telemetry));

        // Residual Shift from a multi-modifier chord (⌥⇧S) is ignored until Shift is released.
        let mut await_release = false;
        assert!(!pure_shift_state(true, true, &mut await_release));
        assert!(!pure_shift_state(true, false, &mut await_release));
        assert!(!pure_shift_state(false, false, &mut await_release));
        assert!(pure_shift_state(true, false, &mut await_release));

        // Shift-ink stroke.
        sink.ink(100, 100.0, 200.0, true);
        sink.ink(120, 140.0, 200.0, true);
        sink.ink(140, 180.0, 200.0, true);
        assert!(sink.snapshot().live.is_some());
        sink.ink(160, 180.0, 200.0, false);
        assert_eq!(sink.snapshot().strokes.len(), 1);

        // Shift-click pin via pointer_down/up while shift_held is active.
        sink.ink(200, 500.0, 500.0, true);
        sink.pointer_down(210, 500.0, 500.0);
        sink.pointer_up(240, 500.0, 500.0, false);
        sink.ink(250, 500.0, 500.0, false);
        assert_eq!(sink.snapshot().strokes.len(), 2);
        assert_eq!(sink.snapshot().strokes[1].pin, Some(1));

        // Undo removes the pin, leaving the ink stroke.
        assert!(sink.undo_stroke());
        assert_eq!(sink.snapshot().strokes.len(), 1);

        // Key chord and flag.
        sink.key_chord(300, "⌘K");
        sink.flag(350);

        let flags = telemetry.take_flags();
        let (events, strokes) = telemetry.finish_session();
        assert_eq!(flags, vec![350]);
        assert_eq!(strokes.len(), 1);
        assert_eq!(strokes[0].pin, None);
        assert!(matches!(
            &events[..],
            [InteractionEvent::Key {
                t: 300,
                shortcut
            }] if shortcut == "⌘K"
        ));
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
    fn frames_that_may_show_a_system_prompt_are_dropped_but_never_the_first() {
        let mut buffer = CandidateBuffer::new(1 << 20);
        for t in [0, 220, 440, 660, 880] {
            buffer.push(RgbaFrame::new(t, 4, 4, vec![0; 64]), false);
        }
        buffer.drop_since(440);
        let times: Vec<u64> = buffer.into_frames().iter().map(|f| f.t_ms).collect();
        assert_eq!(times, [0, 220, 440]);
        let mut buffer = CandidateBuffer::new(1 << 20);
        buffer.push(RgbaFrame::new(500, 4, 4, vec![0; 64]), false);
        buffer.drop_since(0);
        assert_eq!(buffer.len(), 1);
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
        let region = target(CaptureMode::Region, 480.0, 320.0);
        assert_eq!(capture_size(&region, CANDIDATE_EDGE), (480, 320));
        if cfg!(target_os = "windows") {
            // Frames are in pixels: never upscaled past the screen's own.
            assert_eq!(capture_size(&display, 2560.0), (1728, 1116));
            assert_eq!(capture_size(&region, 1280.0), (480, 320));
            let wide = target(CaptureMode::Display, 3440.0, 1440.0);
            assert_eq!(capture_size(&wide, 2560.0), (2560, 1072));
        } else {
            assert_eq!(capture_size(&display, 2560.0), (2560, 1654));
            assert_eq!(capture_size(&region, 1280.0), (960, 640));
        }

        // A 1x external display is recorded at its own pixels, never upscaled.
        let external = |mode, width, height| TargetInfo {
            frame: crate::screens::with_scale([-1920.0, 37.0, width, height], 1.0),
            ..target(mode, width, height)
        };
        let screen = external(CaptureMode::Display, 1920.0, 1080.0);
        assert_eq!(capture_size(&screen, 2560.0), (1920, 1080));
        assert_eq!(capture_size(&screen, CANDIDATE_EDGE), (480, 270));
        let region = external(CaptureMode::Region, 480.0, 320.0);
        assert_eq!(capture_size(&region, 1280.0), (480, 320));
        // A 2x one above the laptop keeps the 2560 px long edge.
        let above = TargetInfo {
            frame: crate::screens::with_scale([0.0, -1440.0, 2560.0, 1440.0], 2.0),
            ..target(CaptureMode::Display, 2560.0, 1440.0)
        };
        assert_eq!(capture_size(&above, 2560.0), (2560, 1440));
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
