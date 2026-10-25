//! Window geometry and cursor log for Screen-mode captures.
//!
//! While a full-screen capture records, a low-rate worker writes down which windows were on
//! screen (front to back) and where the cursor was, so `scrawl autocrop` can work out which
//! window the narration and ink were about. Snapshots are deduped (only stored when the window
//! list changes), cursor samples are deduped by movement, and window titles stay in
//! `scrawl.json`: they never go into `summary.md` or the clipboard.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::model::TargetFrame;

/// How often the worker wakes to sample the cursor and check its triggers.
const TICK: Duration = Duration::from_millis(66);
/// Snapshots while the cursor moves are at most this often (about 2 Hz).
const MOVING_INTERVAL: Duration = Duration::from_millis(500);
/// A snapshot even when nothing seems to happen, to catch windows opening without input.
const IDLE_INTERVAL: Duration = Duration::from_millis(2_000);
/// Cursor samples closer than this to the previous one (in capture points) are dropped.
const CURSOR_MIN_MOVE_PT: f64 = 2.0;

pub use scrawl_router::model::{CursorSample, WindowEntry, WindowSnapshot};

/// What the worker collected over one capture.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ActivityLog {
    pub windows: Vec<WindowSnapshot>,
    pub cursor: Vec<CursorSample>,
    /// Calls to the window list and the time they took, for the log line at the end.
    pub snapshot_calls: u32,
    pub snapshot_time: Duration,
    /// When the finish sequence began and how long stopping capture streams took.
    pub stop_started: Option<Instant>,
    pub stop_ms: Option<u64>,
}

impl ActivityLog {
    /// Stores a snapshot unless the window list is identical to the last stored one.
    pub fn push_windows(&mut self, t: u64, windows: Vec<WindowEntry>) -> bool {
        if self
            .windows
            .last()
            .is_some_and(|last| last.windows == windows)
        {
            return false;
        }
        self.windows.push(WindowSnapshot { t, windows });
        true
    }

    /// Stores a cursor sample (capture points) unless it is within `CURSOR_MIN_MOVE_PT` of the
    /// last one or outside the capture.
    pub fn push_cursor(&mut self, t: u64, x_pt: f64, y_pt: f64, frame: TargetFrame) -> bool {
        let (w, h) = (frame.width.max(1.0), frame.height.max(1.0));
        if !(0.0..=w).contains(&x_pt) || !(0.0..=h).contains(&y_pt) {
            return false;
        }
        let nx = round4(x_pt / w);
        let ny = round4(y_pt / h);
        if let Some(CursorSample(_, px, py)) = self.cursor.last() {
            let dx = (nx - px) as f64 * w;
            let dy = (ny - py) as f64 * h;
            if dx.hypot(dy) < CURSOR_MIN_MOVE_PT {
                return false;
            }
        }
        self.cursor.push(CursorSample(t, nx, ny));
        true
    }
}

fn round4(v: f64) -> f32 {
    ((v * 10_000.0).round() / 10_000.0) as f32
}

/// One window as the window server reported it, in global points.
#[derive(Clone, Debug, PartialEq)]
pub struct RawWindow {
    pub id: u32,
    pub pid: i32,
    pub owner: String,
    pub title: String,
    pub bounds: [f64; 4],
    pub layer: i32,
    pub alpha: f64,
}

/// What the log needs to know about a window's app.
#[derive(Clone, Debug, PartialEq)]
pub struct AppMeta {
    pub bundle_id: Option<String>,
    /// A Dock app (`NSApplicationActivationPolicyRegular`), not an agent or overlay.
    pub regular: bool,
}

/// Keeps normal windows (plus regular apps' panels and menus), the menu bar and the Dock that
/// overlap the capture, converts them to capture points and marks the frontmost app's first
/// normal window.
///
/// The Dock's own window covers the whole display, so it is replaced by `dock` (the strip the
/// Dock reserves, in global points) or dropped when the Dock hides.
pub fn filter_windows(
    raw: &[RawWindow],
    frame: TargetFrame,
    own_pid: i32,
    frontmost_pid: Option<i32>,
    dock: Option<[f64; 4]>,
    app_meta: &mut dyn FnMut(i32) -> AppMeta,
) -> Vec<WindowEntry> {
    let mut front_marked = false;
    let mut out = Vec::new();
    for w in raw {
        let is_dock = w.layer == 20 && w.owner == "Dock";
        let bounds = match (is_dock, dock) {
            (true, Some(rect)) => rect,
            (true, None) => continue,
            (false, _) => w.bounds,
        };
        let [x, y, width, height] = bounds;
        let menubar = w.layer == 24 && w.title == "Menubar";
        let overlaps = x < frame.x + frame.width
            && x + width > frame.x
            && y < frame.y + frame.height
            && y + height > frame.y;
        if w.pid == own_pid || w.alpha <= 0.01 || width < 24.0 || height < 16.0 || !overlaps {
            continue;
        }
        let meta = app_meta(w.pid);
        // Floating panels, sheets and pop-up menus (layers up to 101) count when a regular
        // app owns them; other high layers are system chrome, notifications and overlays.
        let keep =
            w.layer == 0 || menubar || is_dock || (meta.regular && (1..=101).contains(&w.layer));
        if !keep {
            continue;
        }
        let frontmost = !front_marked && w.layer == 0 && Some(w.pid) == frontmost_pid;
        front_marked |= frontmost;
        out.push(WindowEntry {
            id: w.id,
            app: w.owner.clone(),
            bundle_id: meta.bundle_id,
            regular: meta.regular,
            title: w.title.clone(),
            bounds: [
                (x - frame.x).round() as i32,
                (y - frame.y).round() as i32,
                width.round() as i32,
                height.round() as i32,
            ],
            layer: w.layer,
            frontmost,
        });
    }
    out
}

/// Apps whose windows are macOS asking the user something, never what a capture is about:
/// UserNotificationCenter shows the privacy alerts (ScreenCaptureKit's "bypass the system
/// private window picker" among them), SecurityAgent the password sheets.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const SYSTEM_PROMPT_OWNERS: [&str; 2] = ["UserNotificationCenter", "SecurityAgent"];

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn is_system_prompt(w: &RawWindow) -> bool {
    w.alpha > 0.01 && SYSTEM_PROMPT_OWNERS.contains(&w.owner.as_str())
}

/// System prompts (see `SYSTEM_PROMPT_OWNERS`) overlapping `frame`, by window id, sorted.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn system_prompts_over(raw: &[RawWindow], frame: TargetFrame) -> Vec<u32> {
    let mut ids: Vec<u32> = raw
        .iter()
        .filter(|w| {
            let [x, y, width, height] = w.bounds;
            is_system_prompt(w)
                && x < frame.x + frame.width
                && x + width > frame.x
                && y < frame.y + frame.height
                && y + height > frame.y
        })
        .map(|w| w.id)
        .collect();
    ids.sort_unstable();
    ids
}

/// A macOS question is in view: a system prompt, or an alert in System Settings (a second
/// window of its own, like the "Quit & Reopen" that follows a Screen Recording grant) while
/// System Settings is in front. One left behind another window isn't asking anyone.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn macos_is_asking(raw: &[RawWindow]) -> bool {
    let normal = || raw.iter().filter(|w| w.layer == 0 && w.alpha > 0.01);
    raw.iter().any(is_system_prompt)
        || (normal()
            .next()
            .is_some_and(|w| w.owner == "System Settings")
            && normal().filter(|w| w.owner == "System Settings").count() > 1)
}

/// Whether a global point is on a system prompt (they float above app windows), so a click
/// there is the user answering macOS rather than part of what they're showing.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn system_prompt_at(raw: &[RawWindow], x: f64, y: f64) -> bool {
    raw.iter().any(|w| {
        let [wx, wy, ww, wh] = w.bounds;
        is_system_prompt(w) && x >= wx && x <= wx + ww && y >= wy && y <= wy + wh
    })
}

/// Whether the topmost window under a global point (in `raw`, front to back) belongs to a process
/// `ours` accepts, looking through `passthrough` windows, which let clicks fall to whatever is
/// below them.
pub fn owned_window_at(
    raw: &[RawWindow],
    x: f64,
    y: f64,
    passthrough: &[u32],
    ours: impl Fn(i32) -> bool,
) -> bool {
    raw.iter()
        .filter(|w| {
            let [wx, wy, ww, wh] = w.bounds;
            w.alpha > 0.01 && x >= wx && x <= wx + ww && y >= wy && y <= wy + wh
        })
        .find(|w| !passthrough.contains(&w.id))
        .is_some_and(|w| ours(w.pid))
}

/// Reads the on-screen window list (front to back) from the window server.
#[cfg(target_os = "macos")]
pub fn raw_windows() -> Vec<RawWindow> {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::number::CFNumber;
    use core_foundation::string::CFString;
    use core_graphics::window::{
        copy_window_info, kCGNullWindowID, kCGWindowListExcludeDesktopElements,
        kCGWindowListOptionOnScreenOnly,
    };

    let Some(list) = copy_window_info(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
        kCGNullWindowID,
    ) else {
        return Vec::new();
    };
    let key = CFString::from_static_string;
    let (k_number, k_pid, k_owner, k_name, k_layer, k_alpha, k_bounds) = (
        key("kCGWindowNumber"),
        key("kCGWindowOwnerPID"),
        key("kCGWindowOwnerName"),
        key("kCGWindowName"),
        key("kCGWindowLayer"),
        key("kCGWindowAlpha"),
        key("kCGWindowBounds"),
    );
    let (k_x, k_y, k_w, k_h) = (key("X"), key("Y"), key("Width"), key("Height"));
    let number = |dict: &CFDictionary<CFString, CFType>, k: &CFString| -> Option<f64> {
        dict.find(k)?.downcast::<CFNumber>()?.to_f64()
    };
    let string = |dict: &CFDictionary<CFString, CFType>, k: &CFString| -> String {
        dict.find(k)
            .and_then(|v| v.downcast::<CFString>())
            .map(|s| s.to_string())
            .unwrap_or_default()
    };
    let mut out = Vec::new();
    for item in list.iter() {
        let ptr = *item as CFDictionaryRef;
        if ptr.is_null() {
            continue;
        }
        let dict: CFDictionary<CFString, CFType> =
            unsafe { CFDictionary::wrap_under_get_rule(ptr) };
        let Some(bounds) = dict
            .find(&k_bounds)
            .and_then(|v| v.downcast::<CFDictionary>())
        else {
            continue;
        };
        let bounds: CFDictionary<CFString, CFType> =
            unsafe { CFDictionary::wrap_under_get_rule(bounds.as_concrete_TypeRef()) };
        out.push(RawWindow {
            id: number(&dict, &k_number).unwrap_or(0.0) as u32,
            pid: number(&dict, &k_pid).unwrap_or(0.0) as i32,
            owner: string(&dict, &k_owner),
            title: string(&dict, &k_name),
            bounds: [
                number(&bounds, &k_x).unwrap_or(0.0),
                number(&bounds, &k_y).unwrap_or(0.0),
                number(&bounds, &k_w).unwrap_or(0.0),
                number(&bounds, &k_h).unwrap_or(0.0),
            ],
            layer: number(&dict, &k_layer).unwrap_or(0.0) as i32,
            alpha: number(&dict, &k_alpha).unwrap_or(1.0),
        });
    }
    out
}

/// Reads the visible top-level windows (front to back). Like CGWindowList: the taskbar is the
/// `Dock` (layer 20), always-on-top and tool windows sit above layer 0, and cloaked (on another
/// virtual desktop) or minimized windows have no alpha.
#[cfg(target_os = "windows")]
pub fn raw_windows() -> Vec<RawWindow> {
    let mut names: HashMap<u32, String> = HashMap::new();
    crate::win::top_level_windows()
        .into_iter()
        .map(|w| {
            let taskbar = matches!(w.class.as_str(), "Shell_TrayWnd" | "Shell_SecondaryTrayWnd");
            let owner = if taskbar {
                "Dock".to_owned()
            } else {
                names
                    .entry(w.pid)
                    .or_insert_with(|| {
                        crate::win::app_identity(w.pid)
                            .map(|(name, _)| name)
                            .unwrap_or_default()
                    })
                    .clone()
            };
            let layer = if taskbar {
                20
            } else if w.is_app_window() && !w.topmost() {
                0
            } else {
                3
            };
            RawWindow {
                id: w.hwnd as u32,
                pid: w.pid as i32,
                owner,
                title: w.title.clone(),
                bounds: w.rect,
                layer,
                alpha: if w.on_screen() { 1.0 } else { 0.0 },
            }
        })
        .collect()
}

#[cfg(target_os = "windows")]
fn app_meta_of(pid: i32) -> AppMeta {
    let id = crate::win::app_identity(pid as u32).map(|(_, id)| id);
    let regular = crate::win::top_level_windows()
        .iter()
        .any(|w| w.pid == pid as u32 && w.is_app_window());
    AppMeta {
        bundle_id: id,
        regular,
    }
}

/// The taskbar's strip on the display at `frame` (global pixels): the difference between the
/// display and its work area.
#[cfg(target_os = "windows")]
pub fn dock_rect(frame: TargetFrame) -> Option<[f64; 4]> {
    let monitor = crate::win::monitors().into_iter().find(|m| {
        let [x, y, w, h] = m.rect;
        (x - frame.x).abs() < 1.0
            && (y - frame.y).abs() < 1.0
            && (w - frame.width).abs() < 1.0
            && (h - frame.height).abs() < 1.0
    })?;
    let [fx, fy, fw, fh] = monitor.rect;
    let [vx, vy, vw, vh] = monitor.work;
    let (left, right, top, bottom) = (
        vx - fx,
        (fx + fw) - (vx + vw),
        vy - fy,
        (fy + fh) - (vy + vh),
    );
    let min = 8.0;
    if bottom >= min {
        Some([fx, vy + vh, fw, bottom])
    } else if top >= min {
        Some([fx, fy, fw, top])
    } else if left >= min {
        Some([fx, vy, left, vh])
    } else if right >= min {
        Some([vx + vw, vy, right, vh])
    } else {
        None
    }
}

/// One timed snapshot of the windows overlapping `frame`.
#[cfg(target_os = "windows")]
pub fn snapshot(
    frame: TargetFrame,
    dock: Option<[f64; 4]>,
    cache: &mut HashMap<i32, AppMeta>,
) -> Vec<WindowEntry> {
    let raw = raw_windows();
    let front = crate::capture::frontmost_external_pid();
    filter_windows(
        &raw,
        frame,
        std::process::id() as i32,
        front,
        dock,
        &mut |pid| cache.entry(pid).or_insert_with(|| app_meta_of(pid)).clone(),
    )
}

#[cfg(target_os = "macos")]
fn app_meta_of(pid: i32) -> AppMeta {
    use objc2_app_kit::{NSApplicationActivationPolicy, NSRunningApplication};
    match NSRunningApplication::runningApplicationWithProcessIdentifier(pid) {
        Some(app) => AppMeta {
            bundle_id: app.bundleIdentifier().map(|b| b.to_string()),
            regular: app.activationPolicy() == NSApplicationActivationPolicy::Regular,
        },
        None => AppMeta {
            bundle_id: None,
            regular: false,
        },
    }
}

/// The strip the Dock reserves on the display at `frame` (global top-left points), from the
/// difference between the screen's frame and visible frame. `None` when the Dock hides or off
/// the main thread (`NSScreen` is main-thread only).
#[cfg(target_os = "macos")]
pub fn dock_rect(frame: TargetFrame) -> Option<[f64; 4]> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSScreen;
    let mtm = MainThreadMarker::new()?;
    let screens = NSScreen::screens(mtm);
    // AppKit's origin is the primary display's bottom-left corner, y up.
    let primary_h = screens.firstObject()?.frame().size.height;
    let flip = |r: objc2_foundation::NSRect| {
        [
            r.origin.x,
            primary_h - r.origin.y - r.size.height,
            r.size.width,
            r.size.height,
        ]
    };
    let screen = screens.iter().find(|s| {
        let [x, y, w, h] = flip(s.frame());
        (x - frame.x).abs() < 1.0
            && (y - frame.y).abs() < 1.0
            && (w - frame.width).abs() < 1.0
            && (h - frame.height).abs() < 1.0
    })?;
    let [fx, fy, fw, fh] = flip(screen.frame());
    let [vx, vy, vw, vh] = flip(screen.visibleFrame());
    let (left, right, bottom) = (vx - fx, (fx + fw) - (vx + vw), (fy + fh) - (vy + vh));
    // A hidden Dock still leaves a few points at the edge.
    let min = 8.0;
    if bottom >= min {
        Some([fx, vy + vh, fw, bottom])
    } else if left >= min {
        Some([fx, vy, left, vh])
    } else if right >= min {
        Some([vx + vw, vy, right, vh])
    } else {
        None
    }
}

/// One timed snapshot of the windows overlapping `frame`.
#[cfg(target_os = "macos")]
pub fn snapshot(
    frame: TargetFrame,
    dock: Option<[f64; 4]>,
    cache: &mut HashMap<i32, AppMeta>,
) -> Vec<WindowEntry> {
    objc2::rc::autoreleasepool(|_| {
        let raw = raw_windows();
        let front = crate::capture::frontmost_external_pid();
        filter_windows(
            &raw,
            frame,
            std::process::id() as i32,
            front,
            dock,
            &mut |pid| cache.entry(pid).or_insert_with(|| app_meta_of(pid)).clone(),
        )
    })
}

/// Background worker that logs windows and the cursor during a Screen-mode capture.
///
/// Snapshots are taken at the start, right after `request` is raised (click, ink or laser
/// start), when the frontmost app changes, at most every 500 ms while the cursor moves, every
/// 2 s otherwise, and once more at the end. Cursor samples are taken at 15 Hz.
pub struct WindowLogger {
    stopping: Arc<AtomicBool>,
    worker: Option<JoinHandle<ActivityLog>>,
}

impl WindowLogger {
    /// Call on the main thread, so the Dock's strip can be read from `NSScreen`.
    pub fn start(frame: TargetFrame, started_at: Instant, request: Arc<AtomicBool>) -> Self {
        let stopping = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&stopping);
        let dock = dock_rect(frame);
        let worker = thread::Builder::new()
            .name("scrawl-window-log".into())
            .spawn(move || run(frame, dock, started_at, &request, &stop))
            .ok();
        Self { stopping, worker }
    }

    pub fn stop(mut self) -> ActivityLog {
        self.stopping.store(true, Ordering::Release);
        let log = self
            .worker
            .take()
            .and_then(|w| {
                w.thread().unpark();
                w.join().ok()
            })
            .unwrap_or_default();
        if log.snapshot_calls > 0 {
            crate::log::scrawl_log(format!(
                "Window log: {} snapshots taken, {} stored, {:.2} ms each; {} cursor samples",
                log.snapshot_calls,
                log.windows.len(),
                log.snapshot_time.as_secs_f64() * 1_000.0 / f64::from(log.snapshot_calls),
                log.cursor.len()
            ));
        }
        log
    }
}

impl Drop for WindowLogger {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        if let Some(ref w) = self.worker {
            w.thread().unpark();
        }
    }
}

fn run(
    frame: TargetFrame,
    dock: Option<[f64; 4]>,
    started_at: Instant,
    request: &AtomicBool,
    stopping: &AtomicBool,
) -> ActivityLog {
    let mut log = ActivityLog::default();
    let mut cache = HashMap::new();
    let mut last_snapshot: Option<Instant> = None;
    let mut last_front = None;
    let mut cursor_at_snapshot: Option<[f32; 2]> = None;
    let mut take = |log: &mut ActivityLog, t: u64| {
        let began = Instant::now();
        let windows = snapshot(frame, dock, &mut cache);
        log.snapshot_calls += 1;
        log.snapshot_time += began.elapsed();
        log.push_windows(t, windows);
    };
    loop {
        let finishing = stopping.load(Ordering::Acquire);
        let t = started_at.elapsed().as_millis() as u64;
        let cursor = crate::capture::current_mouse_screen();
        if let Some([x, y]) = cursor {
            log.push_cursor(t, f64::from(x) - frame.x, f64::from(y) - frame.y, frame);
        }
        let front = crate::capture::frontmost_external_pid();
        let since = last_snapshot.map(|s| s.elapsed());
        let moved = cursor != cursor_at_snapshot;
        let due = !finishing
            && (since.is_none()
                || request.swap(false, Ordering::AcqRel)
                || front != last_front
                || (moved && since.is_some_and(|s| s >= MOVING_INTERVAL))
                || since.is_some_and(|s| s >= IDLE_INTERVAL));
        if due {
            take(&mut log, t);
            last_snapshot = Some(Instant::now());
            last_front = front;
            cursor_at_snapshot = cursor;
        }
        if finishing {
            return log;
        }
        thread::park_timeout(TICK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame() -> TargetFrame {
        TargetFrame {
            x: 0.0,
            y: 0.0,
            width: 1000.0,
            height: 500.0,
            scale: 2.0,
        }
    }

    fn entry(id: u32, x: i32) -> WindowEntry {
        WindowEntry {
            id,
            app: format!("App {id}"),
            bundle_id: None,
            regular: true,
            title: String::new(),
            bounds: [x, 0, 400, 300],
            layer: 0,
            frontmost: id == 1,
        }
    }

    #[test]
    fn clicks_on_scrawl_windows_are_found_through_the_ink_overlay() {
        let window = |id, pid, bounds| RawWindow {
            id,
            pid,
            owner: String::new(),
            title: String::new(),
            bounds,
            layer: 0,
            alpha: 1.0,
        };
        let own = 7;
        let ours = |pid| pid == own;
        let raw = [
            window(1, own, [0.0, 0.0, 1728.0, 1117.0]), // the ink overlay, click-through
            window(2, own, [700.0, 6.0, 320.0, 44.0]),  // the recording control
            window(3, 99, [100.0, 30.0, 1200.0, 900.0]), // an app
        ];
        assert!(
            owned_window_at(&raw, 800.0, 20.0, &[1], ours),
            "the stop button"
        );
        assert!(
            !owned_window_at(&raw, 400.0, 500.0, &[1], ours),
            "the app under the overlay"
        );
        assert!(
            !owned_window_at(&raw, 1500.0, 1000.0, &[1], ours),
            "the desktop"
        );
        assert!(
            owned_window_at(&raw, 400.0, 500.0, &[], ours),
            "an overlay that takes clicks"
        );
    }

    #[test]
    fn window_snapshots_are_stored_only_when_the_list_changes() {
        let mut log = ActivityLog::default();
        assert!(log.push_windows(0, vec![entry(1, 0), entry(2, 100)]));
        assert!(!log.push_windows(500, vec![entry(1, 0), entry(2, 100)]));
        // A moved window, a reorder, and a focus change are all new snapshots.
        assert!(log.push_windows(1_000, vec![entry(1, 10), entry(2, 100)]));
        assert!(log.push_windows(1_500, vec![entry(2, 100), entry(1, 10)]));
        let mut unfocused = vec![entry(2, 100), entry(1, 10)];
        unfocused[1].frontmost = false;
        assert!(log.push_windows(2_000, unfocused.clone()));
        assert!(!log.push_windows(2_500, unfocused));
        assert_eq!(
            log.windows.iter().map(|s| s.t).collect::<Vec<_>>(),
            vec![0, 1_000, 1_500, 2_000]
        );
    }

    #[test]
    fn logger_snapshots_at_start_and_right_after_a_request() {
        let request = Arc::new(AtomicBool::new(false));
        let logger = WindowLogger::start(frame(), Instant::now(), Arc::clone(&request));
        request.store(true, Ordering::Release);
        // A slow window list (a loaded CI runner) can hold the worker for a while, so wait for
        // the request to be taken instead of sleeping a fixed time. Once it's taken, the snapshot
        // it asked for is under way and `stop` joins it.
        let deadline = Instant::now() + Duration::from_secs(20);
        while request.load(Ordering::Acquire) && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert!(!request.load(Ordering::Acquire), "the request was taken");
        let log = logger.stop();
        assert!(log.snapshot_calls >= 2, "{} snapshots", log.snapshot_calls);
        assert!(log.windows.len() <= log.snapshot_calls as usize);
    }

    #[test]
    fn cursor_samples_skip_tiny_moves_and_points_outside_the_capture() {
        let mut log = ActivityLog::default();
        assert!(log.push_cursor(0, 500.0, 250.0, frame()));
        assert!(!log.push_cursor(66, 501.0, 250.5, frame()));
        assert!(log.push_cursor(132, 520.0, 250.0, frame()));
        assert!(!log.push_cursor(198, -5.0, 250.0, frame()));
        assert!(!log.push_cursor(264, 500.0, 900.0, frame()));
        assert_eq!(
            log.cursor,
            vec![CursorSample(0, 0.5, 0.5), CursorSample(132, 0.52, 0.5)]
        );
        assert_eq!(
            serde_json::to_string(&log.cursor[1]).unwrap(),
            "[132,0.52,0.5]"
        );
    }

    #[test]
    fn filtering_keeps_overlapping_windows_menu_bar_and_dock_in_capture_points() {
        let capture = TargetFrame {
            x: 1000.0,
            ..frame()
        };
        let raw = |id, pid, owner: &str, title: &str, bounds, layer| RawWindow {
            id,
            pid,
            owner: owner.into(),
            title: title.into(),
            bounds,
            layer,
            alpha: 1.0,
        };
        let windows = [
            raw(1, 9, "Scrawl", "", [1000.0, 0.0, 300.0, 300.0], 0),
            raw(
                2,
                10,
                "Window Server",
                "Menubar",
                [1000.0, 0.0, 1000.0, 24.0],
                24,
            ),
            raw(3, 11, "Dock", "Dock", [1000.0, 0.0, 1000.0, 500.0], 20),
            raw(4, 12, "Control Center", "", [1800.0, 0.0, 40.0, 24.0], 25),
            raw(5, 13, "Safari", "Docs", [1100.5, 50.0, 600.0, 400.0], 0),
            raw(6, 14, "Ghostty", "zsh", [1200.0, 80.0, 600.0, 400.0], 0),
            raw(7, 13, "Safari", "Other", [1150.0, 60.0, 600.0, 400.0], 0),
            raw(8, 15, "Notes", "", [0.0, 0.0, 600.0, 400.0], 0),
            raw(9, 16, "Tiny", "", [1300.0, 300.0, 10.0, 10.0], 0),
            raw(10, 13, "Safari", "", [1200.0, 30.0, 200.0, 120.0], 101),
            raw(11, 14, "Ghostty", "", [1200.0, 30.0, 200.0, 120.0], 101),
        ];
        let mut lookups = Vec::new();
        let dock = Some([1000.0, 440.0, 1000.0, 60.0]);
        let out = filter_windows(&windows, capture, 9, Some(13), dock, &mut |pid| {
            lookups.push(pid);
            AppMeta {
                bundle_id: Some(format!("pid.{pid}")),
                regular: pid != 12 && pid != 14,
            }
        });
        assert_eq!(
            out.iter().map(|w| w.id).collect::<Vec<_>>(),
            vec![2, 3, 5, 6, 7, 10],
            "Safari's pop-up menu counts, not an agent app's high-layer window"
        );
        let safari = &out[2];
        assert_eq!(safari.bounds, [101, 50, 600, 400]);
        assert_eq!(safari.bundle_id.as_deref(), Some("pid.13"));
        assert!(safari.frontmost);
        assert!(
            !out[4].frontmost,
            "only the first window of the frontmost app"
        );
        assert!(!out[0].frontmost && !out[1].frontmost);
        assert_eq!(
            out[1].bounds,
            [0, 440, 1000, 60],
            "the Dock's strip, not its window"
        );
        assert!(!out[3].regular);
        assert_eq!(lookups, vec![10, 11, 12, 13, 14, 13, 13, 14]);
        // A hidden Dock has no strip, so its full-screen window is dropped.
        let hidden = filter_windows(&windows, capture, 9, Some(13), None, &mut |_| AppMeta {
            bundle_id: None,
            regular: true,
        });
        assert!(hidden.iter().all(|w| w.id != 3));
    }

    #[test]
    fn system_prompts_are_found_over_the_capture_and_clicks_on_them_are_answers() {
        let window = |id, owner: &str, bounds, layer| RawWindow {
            id,
            pid: 7,
            owner: owner.into(),
            title: String::new(),
            bounds,
            layer,
            alpha: 1.0,
        };
        let raw = [
            window(5, "UserNotificationCenter", [830.0, 188.0, 260.0, 348.0], 8),
            window(6, "SecurityAgent", [2000.0, 100.0, 300.0, 200.0], 1000),
            window(7, "Safari", [0.0, 0.0, 1000.0, 500.0], 0),
        ];
        // `frame()` is 1000 × 500 at the origin: the password sheet is on another display.
        assert_eq!(system_prompts_over(&raw, frame()), vec![5]);
        assert!(system_prompt_at(&raw, 900.0, 300.0));
        assert!(!system_prompt_at(&raw, 100.0, 100.0));
        assert!(macos_is_asking(&raw));
        let settings = [
            window(8, "System Settings", [597.0, 132.0, 723.0, 625.0], 0),
            window(9, "Safari", [0.0, 0.0, 1000.0, 500.0], 0),
        ];
        assert!(!macos_is_asking(&settings));
        let quit_and_reopen = [
            window(10, "System Settings", [829.0, 342.0, 260.0, 204.0], 0),
            settings[0].clone(),
        ];
        assert!(macos_is_asking(&quit_and_reopen));
        // The same alert behind Scrawl's window.
        let behind = [
            window(11, "Scrawl Helper", [650.0, 145.0, 620.0, 532.0], 0),
            quit_and_reopen[0].clone(),
            settings[0].clone(),
        ];
        assert!(!macos_is_asking(&behind));
    }
}
