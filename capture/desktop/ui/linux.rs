//! The Linux shell around the shared UI: the tray icon and its menu (StatusNotifierItem, the
//! same items as the macOS menu bar menu), and the X11 properties that float the pill, picker
//! and ink overlay above every window (override-redirect, on top, click-through).
//!
//! GPUI's X11 backend sees the whole X screen as one display with its origin at (0, 0), so a
//! window's DIPs are root-window pixels divided by the scale. Apparent places its floating
//! windows itself, in pixels, once GPUI has mapped them (`linux::x11::float`).

use std::cell::RefCell;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use gpui::{AnyElement, App, Div, IntoElement, Pixels, Point, Window, div, px};
use ksni::blocking::TrayMethods as _;

use super::super::{LaserOverlay, MenuAction, PILL_WINDOW_H, PILL_WINDOW_W};
use crate::capture::engine::runtime::SharedCaptureTelemetry;
use crate::capture::desktop::linux::{self, x11};

/// Where the file browser is called in buttons ("Show in Files").
pub(in crate::capture::desktop) const FILE_MANAGER: &str = "Files";
/// The desktop draws the title bar and its buttons: the review header starts at the usual inset.
pub(in crate::capture::desktop) const HEADER_LEAD: f32 = 16.0;
pub(in crate::capture::desktop) const HEADER_TRAIL: f32 = 0.0;
/// The pill's top edge on the screen, below the work area's top (GNOME's top bar).
pub(in crate::capture::desktop) const PILL_TOP: f32 = 14.0;

/// GPUI lays out in DIPs; Apparent's global coordinates are pixels.
pub(in crate::capture::desktop) fn points_per_pixel(window: &Window) -> f32 {
    1.0 / window.scale_factor().max(0.25)
}

/// The middle of the rect a window was last asked to open on (global pixels): GPUI has one
/// display for the whole X screen, so `hud_pill_origin` needs to know which monitor.
static LAST_FRAME_CENTRE: Mutex<Option<[f64; 2]>> = Mutex::new(None);

/// GPUI's one display (the X screen), remembering which monitor `frame` (global pixels) is on.
pub(in crate::capture::desktop) fn display_at(
    frame: [f32; 4],
    cx: &App,
) -> Option<std::rc::Rc<dyn gpui::PlatformDisplay>> {
    let [x, y, w, h] = frame.map(f64::from);
    *LAST_FRAME_CENTRE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = Some([x + w / 2.0, y + h / 2.0]);
    cx.primary_display()
}

/// A window covering the monitor at `frame` (global pixels): the monitor's size in DIPs.
pub(in crate::capture::desktop) fn layout_size(
    frame: [f32; 4],
    _display: Option<&dyn gpui::PlatformDisplay>,
) -> gpui::Size<gpui::Pixels> {
    let scale = x11::ui_scale() as f32;
    gpui::size(px(frame[2] / scale), px(frame[3] / scale))
}

/// The open picker's and pill's windows, to move them when they follow the pointer or the
/// recording to another display.
static PICKER_WINDOW: AtomicU32 = AtomicU32::new(0);
static PILL_WINDOW: AtomicU32 = AtomicU32::new(0);
// Moving an override-redirect window remaps it in Hyprland, losing its pin.
// Coalesce placements; compositor IPC stays off the UI/drag thread.
static PILL_NEEDS_PIN: AtomicBool = AtomicBool::new(false);

fn own_window(slot: &AtomicU32) -> Option<u32> {
    let id = slot.load(Ordering::Acquire);
    (id != 0).then_some(id)
}

/// Where GPUI first puts the picker; `configure_picker_window` moves it onto its monitor.
pub(in crate::capture::desktop) fn window_origin(_display: Option<&dyn gpui::PlatformDisplay>) -> Point<Pixels> {
    let scale = x11::ui_scale();
    LAST_FRAME_CENTRE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .and_then(|[x, y]| linux::monitor_at(x, y))
        .map_or_else(Point::default, |m| {
            gpui::point(
                px((m.rect[0] / scale) as f32),
                px((m.rect[1] / scale) as f32),
            )
        })
}

/// Moves the open picker to cover the monitor at `frame` (global pixels), on the next turn of
/// the event loop so GPUI sees the new size.
pub(in crate::capture::desktop) fn move_picker_window(frame: [f64; 4], cx: &mut App) {
    if let Some(id) = own_window(&PICKER_WINDOW) {
        cx.spawn(async move |_| x11::place(id, frame)).detach();
    }
}

/// The picker takes the keys itself (`configure_picker_window`).
pub(in crate::capture::desktop) fn take_keys(_cx: &mut App) {}

/// Hyprland tiles ordinary GPUI windows; float the mapped paper without changing its UI.
pub(in crate::capture::desktop) fn float_whiteboard(cx: &mut App) {
    if !crate::capture::desktop::linux::hyprland::running() {
        return;
    }
    cx.spawn(async move |cx| {
        let mut last_error = None;
        for _ in 0..8 {
            cx.background_executor().timer(Duration::from_millis(120)).await;
            match crate::capture::desktop::linux::hyprland::float_whiteboard() {
                Ok(()) => return,
                Err(error) => last_error = Some(error),
            }
        }
        crate::capture::engine::runtime::apparent_log(format!("Whiteboard: could not float Hyprland window: {last_error:?}"));
    })
    .detach();
}

/// The window manager focuses the window below when Apparent's closes.
pub(in crate::capture::desktop) fn hand_back_keys(_cx: &mut App) {}

/// The desktop's title bar drags the window.
pub(in crate::capture::desktop) fn window_drag_region(region: Div) -> Div {
    region
}

/// Linux's title bar owns dragging; controls in the content receive clicks normally.
pub(in crate::capture::desktop) fn drag_region_control<E>(control: E) -> E {
    control
}

/// The desktop draws the window's buttons.
pub(in crate::capture::desktop) fn caption_buttons() -> Option<AnyElement> {
    None
}

/// GNOME's "Reduce animation" (Settings › Accessibility › Seeing), read once.
pub(in crate::capture::desktop) fn reduce_motion() -> bool {
    static REDUCED: OnceLock<bool> = OnceLock::new();
    *REDUCED.get_or_init(|| {
        std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "enable-animations"])
            .output()
            .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).trim() == "false")
    })
}

/// Only installed AppImages are eligible; debug builds stay untouched.
pub(in crate::capture::desktop) fn start_updates() {
    crate::capture::desktop::linux::update::start();
}

pub(in crate::capture::desktop) fn check_for_updates() {
    std::thread::spawn(crate::capture::desktop::linux::update::check_now);
}

pub(in crate::capture::desktop) fn open_overlay(
    telemetry: Arc<SharedCaptureTelemetry>,
    on: Option<[f64; 4]>,
    cx: &mut gpui::Context<super::super::ApparentCoordinator>,
) -> Result<LaserOverlay, String> {
    LaserOverlay::open(telemetry, on, cx)
}

/// The app icon, box-filtered down to `size`, as RGBA.
fn icon_rgba(size: usize) -> Option<Vec<u8>> {
    scaled_icon_rgba(
        include_bytes!("../../../capture/desktop/assets/app-icon.png"),
        size,
    )
}

fn scaled_icon_rgba(bytes: &[u8], size: usize) -> Option<Vec<u8>> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buffer).ok()?;
    let channels = match info.color_type {
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
        _ => return None,
    };
    let (sw, sh) = (info.width as usize, info.height as usize);
    let mut out = vec![0_u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let x0 = x * sw / size;
            let x1 = ((x + 1) * sw / size).max(x0 + 1);
            let y0 = y * sh / size;
            let y1 = ((y + 1) * sh / size).max(y0 + 1);
            let mut acc = [0_u64; 4];
            let mut n = 0_u64;
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let p = &buffer[(sy * sw + sx) * channels..][..channels];
                    let a = if channels == 4 { u64::from(p[3]) } else { 255 };
                    acc[0] += u64::from(p[0]) * a;
                    acc[1] += u64::from(p[1]) * a;
                    acc[2] += u64::from(p[2]) * a;
                    acc[3] += a;
                    n += 1;
                }
            }
            let o = &mut out[(y * size + x) * 4..][..4];
            if let Some(alpha) = std::num::NonZeroU64::new(acc[3]) {
                o[0] = (acc[0] / alpha) as u8;
                o[1] = (acc[1] / alpha) as u8;
                o[2] = (acc[2] / alpha) as u8;
            }
            o[3] = (acc[3] / n.max(1)) as u8;
        }
    }
    Some(out)
}

/// The app icon on a window (`_NET_WM_ICON`) and its class, so the dock and the window switcher
/// show Apparent's icon and match it to `apparent.desktop`.
pub(crate) fn set_app_icon(window: &Window) {
    let Some(id) = x11::window_id(window) else {
        return;
    };
    x11::set_class(id);
    static ICON: OnceLock<Option<Vec<u32>>> = OnceLock::new();
    let icon = ICON.get_or_init(|| {
        let mut data = Vec::new();
        for size in [32_usize, 64, 128] {
            let rgba = icon_rgba(size)?;
            data.extend([size as u32, size as u32]);
            data.extend(rgba.chunks_exact(4).map(|p| {
                u32::from(p[3]) << 24 | u32::from(p[0]) << 16 | u32::from(p[1]) << 8 | u32::from(p[2])
            }));
        }
        Some(data)
    });
    if let Some(icon) = icon {
        x11::set_icon(id, icon);
    }
}

/// Brings one of Apparent's floating windows to the top.
pub(in crate::capture::desktop) fn raise_window(window: &mut Window) {
    if let Some(id) = x11::window_id(window) {
        x11::raise(id);
        raise_note_surfaces();
    }
}

/// `apparent lab`'s backdrop stays an ordinary popup on Linux.
pub(in crate::capture::desktop) fn configure_lab_backdrop(_w: f32, _h: f32) {}

struct KeepAlive;

impl gpui::Render for KeepAlive {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
    }
}

/// GPUI on Linux quits when its last window closes, and Apparent lives in the tray with no window
/// most of the time: an unmanaged 1 px window off the screen keeps it running.
pub(in crate::capture::desktop) fn keep_alive(cx: &mut App) {
    let _ = cx.open_window(
        gpui::WindowOptions {
            window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds {
                origin: gpui::point(px(-100.0), px(-100.0)),
                size: gpui::size(px(1.0), px(1.0)),
            })),
            titlebar: None,
            focus: false,
            show: false,
            kind: gpui::WindowKind::PopUp,
            is_movable: false,
            is_resizable: false,
            is_minimizable: false,
            ..Default::default()
        },
        |window, cx| {
            // GPUI maps every window; this one is taken off the screen and out of the taskbar.
            if let Some(id) = x11::window_id(window) {
                x11::float(id, [-100.0, -100.0, 1.0, 1.0]);
                x11::set_click_through(id, true);
            }
            gpui::AppContext::new(cx, |_| KeepAlive)
        },
    );
}

/// Opens a window Apparent's way on this platform.
pub(in crate::capture::desktop) fn decorate_app_window(window: &mut Window) {
    set_app_icon(window);
}

/// The picker: on top of everything over the whole monitor at `frame` (global pixels), taking
/// clicks and keys.
pub(in crate::capture::desktop) fn configure_picker_window(
    window: &mut Window,
    _lab: bool,
    frame: [f64; 4],
    cx: &mut App,
) -> Option<u32> {
    let id = x11::window_id(window)?;
    PICKER_WINDOW.store(id, Ordering::Release);
    x11::accept_pointer(id);
    x11::float(id, frame);
    cx.spawn(async move |_| {
        x11::place(id, frame);
        x11::focus(id);
    })
    .detach();
    Some(id)
}

/// The pill window's rect (global pixels) on the monitor holding `point`.
fn pill_rect_at(point: [f64; 2]) -> Option<[f64; 4]> {
    let screens = crate::capture::engine::runtime::screens::displays();
    let screen = screens.get(crate::capture::engine::runtime::screens::index_at(&screens, point)?)?;
    Some(crate::capture::engine::runtime::screens::pill_window_rect(
        screen,
        f64::from(PILL_WINDOW_W),
        f64::from(PILL_WINDOW_H),
        f64::from(PILL_TOP),
    ))
}

fn pill_point() -> Option<[f64; 2]> {
    LAST_FRAME_CENTRE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .or_else(crate::capture::engine::runtime::screens::pointer)
}

struct PillDrag {
    id: u32,
    panel: [f64; 4],
    pointer_down: [f64; 2],
    display: crate::capture::engine::runtime::screens::Screen,
    owner: PillDragOwner,
}

enum PillDragOwner {
    Native,
    Electron { pid: u32, alive: Arc<AtomicBool>, position: Arc<Mutex<Option<[f64; 4]>>> },
}

impl PillDragOwner {
    fn current(&self, id: u32) -> bool {
        match self {
            Self::Native => own_window(&PILL_WINDOW) == Some(id),
            Self::Electron { pid, alive, .. } => alive.load(Ordering::Acquire)
                && crate::capture::ui_process::owns(*pid as i32)
                && x11::window_info(id as isize).is_some_and(|window| window.pid == *pid),
        }
    }
    fn move_to(&self, id: u32, rect: [f64; 4]) {
        match self {
            Self::Native => {
                x11::move_origin(id, [rect[0], rect[1]]);
                PILL_NEEDS_PIN.store(true, Ordering::Release);
            }
            Self::Electron { position, .. } => {
                // The child alone maps/moves its Chromium surface. Two native
                // window owners can leave X11 mapped but absent from Hyprland.
                *position.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(rect);
            }
        }
    }
}

static PILL_DRAG: std::sync::Mutex<Option<PillDrag>> = std::sync::Mutex::new(None);
// Once moved, the capsule stays on its original output for this recording even
// after the mouse button is released. The ink overlay may still follow the pointer.
static PILL_USER_MOVED: AtomicBool = AtomicBool::new(false);

fn drag_left_down(x11_left: bool, kernel: Option<&linux::input::Held>) -> bool {
    // An XWayland unmap cancels its implicit grab and clears its button mask.
    // The existing input reader retains the actual held/released device state.
    kernel.map_or(x11_left, |held| held.left())
}

#[test]
fn pill_drag_uses_kernel_button_across_xwayland_remaps() {
    let mut held = linux::input::Held::default();
    held.keys.insert(linux::input::code::BTN_LEFT);
    assert!(drag_left_down(false, Some(&held)), "A remap is not a physical release");
    held.keys.clear();
    assert!(!drag_left_down(true, Some(&held)), "A stale X11 press must not prolong dragging");
    assert!(drag_left_down(true, None));
    assert!(!drag_left_down(false, None));
}

/// Only the recording status/clock starts a drag, not the actionable chips.
pub(in crate::capture::desktop) fn begin_pill_drag() {
    let Some(id) = own_window(&PILL_WINDOW) else { return };
    begin_owned_pill_drag(id, PillDragOwner::Native, None);
}

pub(in crate::capture::desktop) fn begin_electron_pill_drag(
    id: u32, pid: u32, alive: Arc<AtomicBool>,
    position: Arc<Mutex<Option<[f64; 4]>>>, point: [f64; 2],
) {
    begin_owned_pill_drag(id, PillDragOwner::Electron { pid, alive, position }, Some(point));
}

fn begin_owned_pill_drag(id: u32, owner: PillDragOwner, point: Option<[f64; 2]>) {
    if !crate::shared::handoff::experiment("movableRecordingControls") || !owner.current(id) { return; }
    let Some(pointer) = x11::pointer_state() else { return };
    if !drag_left_down(pointer.left, linux::input::held().as_ref()) { return; }
    let [x, y] = point.unwrap_or_else(|| {
        let (x, y) = linux::cursor_position().unwrap_or((pointer.x, pointer.y));
        [x, y]
    });
    let Some(panel) = x11::geometry(id) else { return };
    if ![x, y].iter().all(|v| v.is_finite()) || x < panel[0] || y < panel[1]
        || x >= panel[0] + panel[2] || y >= panel[1] + panel[3] { return; }
    let screens = crate::capture::engine::runtime::screens::displays();
    let centre = [panel[0] + panel[2] / 2.0, panel[1] + panel[3] / 2.0];
    let Some(display) = crate::capture::engine::runtime::screens::index_at(&screens, centre).map(|i| screens[i].clone()) else {
        return;
    };
    *PILL_DRAG.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(PillDrag {
        id,
        panel,
        pointer_down: [x, y],
        display,
        owner,
    });
}

pub(in crate::capture::desktop) fn tick_pill_drag() {
    let mut slot = PILL_DRAG.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(drag) = slot.as_ref() else { return };
    let Some(pointer) = x11::pointer_state() else { return };
    if !drag_left_down(pointer.left, linux::input::held().as_ref()) || !drag.owner.current(drag.id) {
        *slot = None;
        return;
    }
    // XWayland's root pointer freezes above native Wayland windows. Ask the
    // compositor first; the existing kernel reader keeps the button state over remaps.
    let (px, py) = linux::cursor_position().unwrap_or((pointer.x, pointer.y));
    let Some(live) = x11::geometry(drag.id) else { *slot = None; return; };
    // Electron owns content size. Keep the original grab offset while clamping
    // against its live dimensions, without restoring a stale warning/reveal size.
    let panel = [drag.panel[0], drag.panel[1], live[2], live[3]];
    let [x, y] = crate::capture::engine::runtime::screens::drag_pill_origin(
        &drag.display,
        panel,
        drag.pointer_down,
        [px, py],
    );
    if (x - drag.panel[0]).abs() >= 1.0 || (y - drag.panel[1]).abs() >= 1.0 {
        // The compositor pointer query may have waited while the child retired
        // or the user released the button. Refuse a stale move after that wait.
        if !drag.owner.current(drag.id) || !x11::pointer_state()
            .is_some_and(|p| drag_left_down(p.left, linux::input::held().as_ref())) {
            *slot = None;
            return;
        }
        PILL_USER_MOVED.store(true, Ordering::Release);
        let rect = [x, y, live[2], live[3]];
        drag.owner.move_to(drag.id, rect);
    }
}

pub(in crate::capture::desktop) fn cancel_pill_drag() {
    *PILL_DRAG.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    PILL_USER_MOVED.store(false, Ordering::Release);
}

pub(in crate::capture::desktop) fn pill_user_moved() -> bool {
    PILL_USER_MOVED.load(Ordering::Acquire)
}

/// Where the pill window opens (DIPs of GPUI's one display): below the work area's top on the
/// monitor in use.
pub(in crate::capture::desktop) fn hud_pill_origin(display: gpui::Bounds<Pixels>) -> Point<Pixels> {
    let scale = x11::ui_scale();
    match pill_point().and_then(pill_rect_at) {
        Some([x, y, _, _]) => gpui::point(px((x / scale) as f32), px((y / scale) as f32)),
        None => gpui::point(
            display.origin.x + (display.size.width - px(PILL_WINDOW_W)) * 0.5,
            display.origin.y + px(PILL_TOP),
        ),
    }
}

/// Moves the pill window to the monitor holding `point` (global pixels).
pub(in crate::capture::desktop) fn place_hud_pill(point: [f64; 2]) {
    let (Some(id), Some(rect)) = (own_window(&PILL_WINDOW), pill_rect_at(point)) else {
        return;
    };
    // Moving through detached threads lets an older placement run after a newer one when
    // the pointer crosses displays quickly. Keep the capsule's move in event order.
    x11::place(id, rect);
    PILL_NEEDS_PIN.store(true, Ordering::Release);
}

/// The pill's window. Linux has no native shell for it: GPUI draws the capsule, its shadow and
/// its springs, and the window never takes focus from the app being recorded.
pub(in crate::capture::desktop) struct PillChrome {
    id: Option<u32>,
}

/// The mapped recording controls, in the same X11 root pixels as the board.
pub(in crate::capture::desktop) fn coach_anchor() -> Option<[f32; 4]> {
    let xid = own_window(&PILL_WINDOW)?;
    let info = x11::window_info(xid as isize)?;
    info.on_screen().then(|| info.rect.map(|v| v as f32))
}

impl Drop for PillChrome {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            let _ = PILL_WINDOW.compare_exchange(id, 0, Ordering::AcqRel, Ordering::Acquire);
        }
    }
}

impl PillChrome {
    pub(in crate::capture::desktop) fn open(window: &mut Window, cx: &mut App) -> Self {
        let id = x11::window_id(window);
        if let Some(id) = id {
            PILL_WINDOW.store(id, Ordering::Release);
            let rect = pill_point().and_then(pill_rect_at).unwrap_or_else(|| {
                let b = window.bounds();
                let s = f64::from(window.scale_factor());
                [
                    f64::from(f32::from(b.origin.x)) * s,
                    f64::from(f32::from(b.origin.y)) * s,
                    f64::from(f32::from(b.size.width)) * s,
                    f64::from(f32::from(b.size.height)) * s,
                ]
            });
            x11::accept_pointer(id);
            x11::float(id, rect);
            if linux::hyprland::running() {
                x11::set_title(id, &linux::hyprland::pill_title(id));
                PILL_NEEDS_PIN.store(true, Ordering::Release);
                cx.spawn(async move |cx| {
                    loop {
                        cx.background_executor().timer(Duration::from_millis(120)).await;
                        if own_window(&PILL_WINDOW) != Some(id) {
                            return;
                        }
                        if !PILL_NEEDS_PIN.swap(false, Ordering::AcqRel) {
                            continue;
                        }
                        let mut last_error = None;
                        for _ in 0..8 {
                            if own_window(&PILL_WINDOW) != Some(id) {
                                return;
                            }
                            let result = cx.background_executor().spawn(async move {
                                linux::hyprland::pin_pill(id)
                            }).await;
                            match result {
                                Ok(()) => { raise_note_surfaces(); last_error = None; break; }
                                Err(error) => last_error = Some(error),
                            }
                            cx.background_executor().timer(Duration::from_millis(120)).await;
                        }
                        if let Some(error) = last_error {
                            crate::capture::engine::runtime::apparent_log(format!(
                                "Recording pill: could not pin Hyprland window: {error}"
                            ));
                        }
                    }
                }).detach();
            }
        }
        Self { id }
    }

    pub(in crate::capture::desktop) fn window_number(&self) -> Option<u32> {
        self.id
    }

    pub(in crate::capture::desktop) fn has_shell(&self) -> bool {
        false
    }

    pub(in crate::capture::desktop) fn raise(&self) {
        if let Some(id) = self.id {
            x11::raise(id);
            raise_note_surfaces();
        }
    }

    /// Motion traces need the native shell's geometry, which Linux doesn't have.
    #[allow(clippy::too_many_arguments)]
    pub(in crate::capture::desktop) fn trace(
        &self,
        _width: f32,
        _scale: f32,
        _y_offset: f32,
        _opacity: f32,
        _content_alpha: f32,
        _blur_radius: f32,
        _content_shrink: f32,
    ) -> Option<super::super::ui::motion::Xf> {
        None
    }

    #[allow(clippy::too_many_arguments)]
    pub(in crate::capture::desktop) fn apply(
        &mut self,
        _pill_width: f32,
        _pill_height: f32,
        _scale: f32,
        _y_offset: f32,
        _opacity: f32,
        _is_completed: bool,
        _blur_radius: f32,
        _content_shrink: f32,
    ) {
    }
}

/// Whether the pointer is over the pill's window.
pub(in crate::capture::desktop) fn pointer_on_hud_pill() -> bool {
    let (Some(id), Some((x, y))) = (own_window(&PILL_WINDOW), linux::cursor_position()) else {
        return false;
    };
    x11::geometry(id).is_some_and(|[wx, wy, ww, wh]| {
        x >= wx && x <= wx + ww && y >= wy && y <= wy + wh
    })
}

/// The largest of Apparent's windows (for `apparent preview --capture-screenshot`).
pub(in crate::capture::desktop) fn front_own_window() -> Option<isize> {
    let pid = std::process::id();
    linux::top_level_windows()
        .into_iter()
        .filter(|w| w.pid == pid && w.rect[2] >= 600.0 && w.rect[3] >= 400.0)
        .max_by_key(|w| (w.rect[2] * w.rect[3]) as i64)
        .map(|w| w.hwnd)
}

pub(in crate::capture::desktop) fn capture_own_window_screenshot(
    window: Option<isize>,
    out_path: &Path,
) -> anyhow::Result<()> {
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let window = window.ok_or_else(|| anyhow::anyhow!("Could not find Apparent's window"))?;
    let info = linux::window_info(window)
        .ok_or_else(|| anyhow::anyhow!("Apparent's window went away"))?;
    let frame = x11::capture_rect(info.rect)?;
    crate::capture::engine::render::save_png(&frame, out_path)
}

// MARK: The tray menu

/// The tray icon (a StatusNotifierItem: KDE Plasma, and GNOME with the AppIndicator extension,
/// which Ubuntu turns on). Its menu is rebuilt whenever the host asks, so it's always current.
struct ApparentTray {
    actions: flume::Sender<MenuAction>,
    status: Option<String>,
    inbox: usize,
    dark: bool,
}

static TRAY_AVAILABLE: AtomicBool = AtomicBool::new(true);
static CONTROLS_PENDING: AtomicBool = AtomicBool::new(false);

pub(in crate::capture::desktop) fn controls_needed() -> bool {
    !TRAY_AVAILABLE.load(Ordering::Acquire)
}

fn tray_available(available: bool) {
    let previous = TRAY_AVAILABLE.swap(available, Ordering::AcqRel);
    if previous && !available {
        CONTROLS_PENDING.store(true, Ordering::Release);
    }
}

pub(in crate::capture::desktop) fn show_controls(cx: &mut App) {
    CONTROLS.with(|controls| {
        if let Some(controls) = controls.borrow_mut().as_mut() {
            controls.show(cx);
            // A deliberate launch also satisfies a deferred automatic request.
            // Closing this window must not reopen it when Settings later closes.
            CONTROLS_PENDING.store(false, Ordering::Release);
        }
    });
}

pub(in crate::capture::desktop) fn poll_controls(busy: bool, onboarding: bool, settings: bool, cx: &mut App) {
    if take_controls_request(&CONTROLS_PENDING, !controls_needed(), busy, onboarding, settings) {
        crate::capture::desktop::workflow::launch_instance::request_open();
    }
    CONTROLS.with(|controls| {
        if let Some(controls) = controls.borrow_mut().as_mut() { controls.poll(busy || onboarding, cx); }
    });
}

fn take_controls_request(pending: &AtomicBool, tray: bool, busy: bool, onboarding: bool, settings: bool) -> bool {
    // Automatic host-loss/startup presentation must not cover Settings (including
    // About reopened after an update). Explicit launch requests stay independent.
    !tray && !busy && !onboarding && !settings && pending.swap(false, Ordering::AcqRel)
}

fn tray_host_usable(host: bool, items: &[String], pid: u32, owner: impl Fn(&str) -> Option<u32>) -> bool {
    host && items.iter().take(128).any(|item| {
        let name = item.split('/').next().unwrap_or("");
        owner(name) == Some(pid)
    })
}

#[cfg(test)]
mod controls_tests {
    use super::*;

    #[test]
    fn a_host_without_our_registered_item_cannot_suppress_fallback() {
        let ours = vec![":1.42/StatusNotifierItem".to_owned()];
        assert!(!tray_host_usable(true, &[], 42, |_| Some(42)));
        assert!(!tray_host_usable(true, &ours, 42, |_| Some(99)));
        assert!(!tray_host_usable(true, &ours, 42, |_| None));
        assert!(!tray_host_usable(false, &ours, 42, |_| Some(42)));
        assert!(tray_host_usable(true, &ours, 42, |name| {
            assert_eq!(name, ":1.42"); Some(42)
        }));
    }

    #[test]
    fn no_tray_access_waits_for_setup_and_capture_without_losing_the_request() {
        let pending = AtomicBool::new(true);
        assert!(!take_controls_request(&pending, false, false, true, false));
        assert!(!take_controls_request(&pending, false, true, false, false));
        assert!(!take_controls_request(&pending, false, true, true, false));
        assert!(pending.load(Ordering::Acquire));
        assert!(take_controls_request(&pending, false, false, false, false));
        assert!(!take_controls_request(&pending, false, false, false, false));
    }

    #[test]
    fn automatic_controls_wait_for_settings_then_present_once_after_close() {
        let pending = AtomicBool::new(true);
        assert!(!take_controls_request(&pending, false, false, false, true));
        assert!(pending.load(Ordering::Acquire));
        assert!(take_controls_request(&pending, false, false, false, false));
        assert!(!take_controls_request(&pending, false, false, false, false));
    }

    #[test]
    fn a_returned_tray_does_not_open_controls() {
        let pending = AtomicBool::new(true);
        assert!(!take_controls_request(&pending, true, false, false, false));
        assert!(pending.load(Ordering::Acquire));
        assert!(take_controls_request(&pending, false, false, false, false));
    }
}

/// "Record Screen" with the record shortcut as it is now (Settings can change it).
fn record_title() -> String {
    format!(
        "Record Screen ({})",
        crate::capture::desktop::settings::Shortcuts::load().record.label
    )
}

impl ApparentTray {
    fn item(&self, label: impl Into<String>, action: MenuAction) -> ksni::MenuItem<Self> {
        ksni::menu::StandardItem {
            label: label.into(),
            activate: Box::new(move |tray: &mut Self| {
                let _ = tray.actions.try_send(action.clone());
            }),
            ..Default::default()
        }
        .into()
    }
}

impl ksni::Tray for ApparentTray {
    fn watcher_online(&self) { /* The host-property probe confirms actual availability. */ }

    fn watcher_offline(&self, _reason: ksni::OfflineReason) -> bool {
        tray_available(false);
        true
    }
    fn id(&self) -> String {
        "dev.kitlangton.apparent".into()
    }

    fn title(&self) -> String {
        "Apparent".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        static ICONS: OnceLock<[Vec<(usize, Vec<u8>)>; 2]> = OnceLock::new();
        let icons = ICONS.get_or_init(|| {
            [false, true].map(|dark| {
                [22_usize, 32, 48]
                    .into_iter()
                    .filter_map(|size| {
                        let mut argb = scaled_icon_rgba(super::status_icon(dark), size)?;
                        // RGBA → ARGB, in network byte order.
                        for p in argb.chunks_exact_mut(4) {
                            p.rotate_right(1);
                        }
                        Some((size, argb))
                    })
                    .collect()
            })
        });
        icons[usize::from(self.dark)]
            .iter()
            .map(|(size, data)| ksni::Icon {
                width: *size as i32,
                height: *size as i32,
                data: data.clone(),
            })
            .collect()
    }

    fn tool_tip(&self) -> ksni::ToolTip {
        let description = match (&self.status, self.inbox) {
            (Some(status), _) => status.clone(),
            (None, 0) => String::new(),
            (None, 1) => "1 capture from iPhone".to_owned(),
            (None, n) => format!("{n} captures from iPhone"),
        };
        ksni::ToolTip {
            title: "Apparent".into(),
            description,
            ..Default::default()
        }
    }

    /// A left click records (the menu is on a right click).
    fn activate(&mut self, _x: i32, _y: i32) {
        let _ = self.actions.try_send(MenuAction::ToggleCapture);
    }

    fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
        let mut items = Vec::new();
        if let Some(status) = &self.status {
            items.push(
                ksni::menu::StandardItem {
                    label: status.replace('_', "__"),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
            items.push(ksni::MenuItem::Separator);
        }
        let inbox = crate::capture::desktop::workflow::inbox::Inbox::load_default();
        if inbox.len() > 0 {
            let now = crate::shared::clock::now_epoch_ms();
            items.push(
                ksni::menu::StandardItem {
                    label: "From iPhone".into(),
                    enabled: false,
                    ..Default::default()
                }
                .into(),
            );
            for entry in inbox.entries() {
                items.push(self.item(
                    format!("    {}", entry.label(now)),
                    MenuAction::OpenInbox(entry.dir.clone()),
                ));
            }
            items.push(ksni::MenuItem::Separator);
        }
        items.push(self.item(record_title(), MenuAction::ToggleCapture));
        items.push(self.item("Choose What to Record…", MenuAction::ChooseTarget));
        items.push(ksni::menu::CheckmarkItem { label: "Microphone for next screen capture".into(), checked: crate::capture::engine::runtime::audio::mic::enabled(), activate: Box::new(|tray: &mut Self| { let _ = tray.actions.try_send(MenuAction::ToggleMicrophone); }), ..Default::default() }.into());
        items.push(ksni::MenuItem::Separator);
        items.push(self.item("Open Latest Capture", MenuAction::OpenLatest));
        if crate::shared::handoff::video_export() {
            items.push(
                ksni::menu::StandardItem {
                    label: "Export Last Capture as Video…".into(),
                    enabled: !super::super::EXPORTING.load(Ordering::Acquire),
                    activate: Box::new(|tray: &mut Self| {
                        let _ = tray.actions.try_send(MenuAction::ExportVideo);
                    }),
                    ..Default::default()
                }
                .into(),
            );
        }
        items.push(ksni::MenuItem::Separator);
        items.push(self.item("Settings…", MenuAction::Settings));
        items.push(self.item("Check for Updates…", MenuAction::CheckForUpdates));
        items.push(self.item("Quit Apparent", MenuAction::Quit));
        items
    }
}

thread_local! {
    static TRAY: RefCell<Option<ksni::blocking::Handle<ApparentTray>>> = const { RefCell::new(None) };
    static CONTROLS: RefCell<Option<super::controls::Controls>> = const { RefCell::new(None) };
}

pub(in crate::capture::desktop) fn install_status_bar() -> Option<flume::Receiver<MenuAction>> {
    if let Some(warning) = linux::media::encoder_warning() {
        crate::capture::engine::runtime::apparent_log(&warning);
        linux::shell::notify("Apparent setup: H.264 encoder missing", &warning);
    }
    if let Some(warning) = linux::media::decoder_warning() {
        crate::capture::engine::runtime::apparent_log(&warning);
        linux::shell::notify("Apparent setup: H.264 decoder missing", &warning);
    }
    let (sender, receiver) = flume::bounded(16);
    CONTROLS.with(|controls| *controls.borrow_mut() = Some(super::controls::Controls::new(sender.clone())));
    // Both launch transports use the same onboarding/busy/presentation policy.
    let open = linux::ipc::open_requests();
    let _ = std::thread::Builder::new()
        .name("apparent-open-requests".into())
        .spawn(move || {
            while open.recv().is_ok() {
                crate::capture::desktop::workflow::launch_instance::request_open();
            }
        });
    linux::appearance::initialize();
    linux::appearance::watch();
    let tray = ApparentTray {
        actions: sender,
        status: None,
        inbox: 0,
        dark: linux::appearance::is_dark(),
    };
    // Keep the service alive across host absence so installing/enabling a host
    // later does not require restarting Apparent.
    match tray.assume_sni_available(true).spawn() {
        Ok(handle) => {
            TRAY.with(|slot| *slot.borrow_mut() = Some(handle));
            std::thread::spawn(|| loop {
                let available = (|| -> Option<bool> {
                    let bus = zbus::blocking::Connection::session().ok()?;
                    let proxy = zbus::blocking::Proxy::new(&bus, "org.kde.StatusNotifierWatcher",
                        "/StatusNotifierWatcher", "org.kde.StatusNotifierWatcher").ok()?;
                    let host = proxy.get_property::<bool>("IsStatusNotifierHostRegistered").ok()?;
                    let items = proxy.get_property::<Vec<String>>("RegisteredStatusNotifierItems").ok()?;
                    let dbus = zbus::blocking::fdo::DBusProxy::new(&bus).ok()?;
                    Some(tray_host_usable(host, &items, std::process::id(), |name| {
                        dbus.get_connection_unix_process_id(name.try_into().ok()?).ok()
                    }))
                })().unwrap_or(false);
                tray_available(available);
                std::thread::sleep(Duration::from_secs(5));
            });
        }
        Err(error) => {
            tray_available(false);
            crate::capture::engine::runtime::apparent_log(format!(
                "Tray icon unavailable ({error}); showing Apparent controls when idle"
            ));
            linux::shell::notify(
                "Apparent is running",
                &format!(
                    "Press {} to record. Your desktop shows no tray icons; Apparent controls open when setup is complete and capture is idle.",
                    crate::capture::desktop::settings::Shortcuts::load().record.label
                ),
            );
        }
    }
    Some(receiver)
}

/// Called from the UI loop: refreshes the menu now and then (the record shortcut's label and the
/// export item can change while Apparent runs).
pub(in crate::capture::desktop) fn poll_status_bar() {
    static LAST: Mutex<Option<(Instant, u64)>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let revision = linux::appearance::revision();
    if last
        .is_some_and(|(at, previous)| at.elapsed() < Duration::from_secs(5) && previous == revision)
    {
        return;
    }
    *last = Some((Instant::now(), revision));
    TRAY.with(|slot| {
        if let Some(handle) = slot.borrow().as_ref() {
            handle.update(|tray| tray.dark = linux::appearance::is_dark());
        }
    });
}

/// A video export's progress, in the icon's tooltip and menu.
pub(in crate::capture::desktop) fn set_status_text(text: &str) {
    let text = text.to_owned();
    TRAY.with(|slot| {
        if let Some(handle) = slot.borrow().as_ref() {
            handle.update(move |tray| tray.status = Some(text));
        }
    });
}

/// The unhandled iPhone capture count, in the icon's tooltip.
pub(in crate::capture::desktop) fn set_status_inbox_count(count: usize) {
    TRAY.with(|slot| {
        if let Some(handle) = slot.borrow().as_ref() {
            handle.update(move |tray| {
                tray.status = None;
                tray.inbox = count;
            });
        }
    });
}

/// Use the desktop's own chooser (GTK/KDE on X11 and Wayland), never a shell picker.
/// Cancellation must not start the encoder.
pub(in crate::capture::desktop) fn choose_export_destination(
    default: &Path,
) -> anyhow::Result<Option<std::path::PathBuf>> {
    use anyhow::Context as _;
    use ashpd::desktop::ResponseError;
    use ashpd::desktop::file_chooser::{FileFilter, SelectedFiles};
    futures_lite::future::block_on(async {
        let name = default
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("capture.mp4");
        let request = SelectedFiles::save_file()
            .title("Export Last Capture as Video")
            .accept_label("Export")
            .current_name(name)
            .filter(FileFilter::new("MP4 video").glob("*.mp4"))
            .send()
            .await
            .context("Could not open the desktop Save dialog; check your FileChooser portal")?;
        let selected = match request.response() {
            Ok(selected) => selected,
            Err(ashpd::Error::Response(ResponseError::Cancelled)) => return Ok(None),
            Err(error) => return Err(error).context("Desktop Save dialog failed"),
        };
        let uri = selected
            .uris()
            .first()
            .context("Desktop Save dialog returned no destination")?;
        local_export_destination(uri).map(Some)
    })
}

fn local_export_destination(uri: &ashpd::url::Url) -> anyhow::Result<std::path::PathBuf> {
    let path = uri
        .to_file_path()
        .map_err(|_| anyhow::anyhow!("Choose a local MP4 file destination"))?;
    anyhow::ensure!(
        path.extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mp4")),
        "Choose a filename ending in .mp4 and retry"
    );
    Ok(path)
}

#[cfg(test)]
mod export_menu_tests {
    use super::*;
    use ksni::Tray as _;

    #[test]
    fn tray_icon_stays_readable_through_light_dark_light() {
        let (actions, _) = flume::bounded(1);
        let mut tray = ApparentTray {
            actions,
            status: None,
            inbox: 0,
            dark: false,
        };
        let light = tray.icon_pixmap();
        tray.dark = true;
        let dark = tray.icon_pixmap();
        tray.dark = false;
        let light_again = tray.icon_pixmap();
        assert_eq!(light.len(), 3);
        for ((light, dark), again) in light.iter().zip(&dark).zip(&light_again) {
            let size = light.width as usize;
            assert_eq!(light.data.len(), size * size * 4);
            assert_eq!(
                light.data, again.data,
                "returning to Light must restore the icon"
            );
            for (icon, background, glyph) in [
                (light, [255, 250, 249, 246], [255, 122, 88, 199]),
                (dark, [255, 2, 1, 2], [255, 183, 165, 216]),
            ] {
                assert_eq!(
                    &icon.data[..4],
                    &background,
                    "tray must use the matching boxed brand icon"
                );
                assert!(
                    icon.data.chunks_exact(4).all(|p| p[0] == 255),
                    "no transparent holes at any tray size"
                );
                assert!(
                    icon.data.chunks_exact(4).filter(|p| *p == glyph).count() > size * size / 10,
                    "the recognizable purple glyph must survive resizing"
                );
            }
            assert_ne!(
                light.data, dark.data,
                "a live appearance change must change the pixmap"
            );
        }
    }

    #[test]
    fn export_progress_and_failure_are_visible_menu_rows() {
        let (actions, _) = flume::bounded(1);
        for status in [
            "Exporting 42%",
            "Export failed — choose another destination and retry",
        ] {
            let tray = ApparentTray {
                actions: actions.clone(),
                status: Some(status.into()),
                inbox: 0,
                dark: true,
            };
            assert!(
                tray.menu().iter().any(|item| matches!(item,
                    ksni::MenuItem::Standard(item) if item.label == status && !item.enabled
                )),
                "status must be visible without hovering: {status}"
            );
        }
    }

    #[test]
    fn export_destination_accepts_local_mp4_and_decodes_filename() {
        let uri = ashpd::url::Url::parse("file:///home/test/My%20capture%20%C3%A4.MP4").unwrap();
        assert_eq!(
            local_export_destination(&uri).unwrap(),
            Path::new("/home/test/My capture ä.MP4")
        );
    }

    #[test]
    fn export_destination_refuses_remote_or_non_mp4_targets() {
        for uri in [
            "https://example.com/capture.mp4",
            "file:///home/test/capture",
            "file:///home/test/capture.txt",
        ] {
            assert!(
                local_export_destination(&ashpd::url::Url::parse(uri).unwrap()).is_err(),
                "{uri}"
            );
        }
    }
}

/// High bit distinguishes a compositor address from an X11 ID; neither comes from JS.
pub(in crate::capture::desktop) fn note_focus() -> (u64, u32) {
    if crate::capture::desktop::linux::hyprland::running() {
        if let Ok(window) = crate::capture::desktop::linux::hyprland::focused_window() {
            if let Ok(address) = u64::from_str_radix(window.address.trim_start_matches("0x"), 16) {
                if address > 0 && address < (1 << 63) { return (address | (1 << 63), window.pid); }
            }
        }
    }
    crate::capture::desktop::linux::x11::active_window().map_or((0, 0), |w| (w.hwnd as u64, w.pid))
}
pub(in crate::capture::desktop) fn restore_note_focus(token: u64, owner: u32) {
    if token & (1 << 63) != 0 {
        let _ = crate::capture::desktop::linux::hyprland::restore_note_focus(token & !(1 << 63), owner);
    } else if token != 0 && crate::capture::desktop::linux::x11::window_info(token as isize).is_some_and(|w| w.pid == owner) {
        crate::capture::desktop::linux::x11::activate(token as isize);
    }
    raise_note_surfaces();
}

/// The Electron counter follows the same native recording panel, including user drags.
pub(in crate::capture::desktop) fn note_badge_anchor(width: f32, y: f32) -> Option<[f64; 4]> {
    let rect = x11::geometry(own_window(&PILL_WINDOW)?)?;
    Some([rect[0] + rect[2] / 2.0 - f64::from(width) / 2.0,
        rect[1] + f64::from(y), f64::from(width), f64::from(super::super::ISLAND_H)])
}

#[derive(Clone, Copy, Default)]
struct NoteSurface { id: u32, generation: u64, worker: bool, rect: [i32; 4], focus: bool }
#[derive(Clone, Copy, Default)]
struct NoteSurfaces { pid: u32, slots: [NoteSurface; 2] }
static NOTE_SURFACES: std::sync::Mutex<NoteSurfaces> = std::sync::Mutex::new(NoteSurfaces {
    pid: 0, slots: [NoteSurface { id: 0, generation: 0, worker: false, rect: [0; 4], focus: false }; 2],
});

pub(in crate::capture::desktop) fn register_note_surfaces(pid: u32) {
    *NOTE_SURFACES.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = NoteSurfaces { pid, ..Default::default() };
}
pub(in crate::capture::desktop) fn clear_note_surfaces(pid: u32) {
    let mut state = NOTE_SURFACES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if state.pid == pid { *state = NoteSurfaces::default(); }
}
fn raise_note_surfaces() {
    let state = *NOTE_SURFACES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let ids: Vec<_> = state.slots.iter().filter(|surface| surface.id != 0 &&
        x11::window_info(surface.id as isize).is_some_and(|w| w.pid == state.pid && w.on_screen()))
        .map(|surface| surface.id).collect();
    x11::raise_group(&ids);
}

pub(in crate::capture::desktop) fn maintain_note_surfaces(pid: u32) {
    let owned = NOTE_SURFACES.lock().unwrap_or_else(std::sync::PoisonError::into_inner).pid == pid;
    if owned { raise_note_surfaces(); }
}

fn pin_note_surface(id: u32, pid: u32, index: usize) {
    let mut failures = 0;
    loop {
        let (generation, rect, focus) = {
            let state = NOTE_SURFACES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.pid != pid || state.slots[index].id != id { return; }
            let slot = state.slots[index];
            (slot.generation, slot.rect, slot.focus)
        };
        let visible = x11::window_info(id as isize).is_some_and(|w| w.pid == pid && w.on_screen());
        let result = if visible { linux::hyprland::place_note(id, pid, rect, focus) } else { Ok(()) };
        if visible && result.is_ok() {
            raise_note_surfaces();
            // Hyprland's focus dispatch can clear XWayland's keyboard focus
            // after the initial placement. Reconcile the same live request,
            // never a closed editor or a replacement capture's surface.
            let state = NOTE_SURFACES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.pid == pid && state.slots[index].id == id
                && state.slots[index].generation == generation && state.slots[index].focus
                && x11::window_info(id as isize).is_some_and(|w| w.pid == pid && w.on_screen())
            {
                x11::focus(id);
            }
        } else { failures += 1; }
        let retry = {
            let mut state = NOTE_SURFACES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if state.pid != pid || state.slots[index].id != id { return; }
            let slot = &mut state.slots[index];
            if slot.generation != generation { failures = 0; true }
            else if (!visible || result.is_err()) && failures < 8 { true }
            else { slot.worker = false; false }
        };
        if !retry {
            if let Err(error) = result { crate::capture::engine::runtime::apparent_log(format!("Notes pinning failed: {error:#}")); }
            return;
        }
        std::thread::sleep(Duration::from_millis(120));
    }
}

/// Native placement and pinning use only the active capture's child PID and registered XIDs.
/// One worker per surface coalesces remaps: legacy Hyprland pin is a toggle, so two
/// concurrent read-unpinned/dispatch-pin operations could otherwise undo each other.
pub(in crate::capture::desktop) fn prepare_note_surface(id: u32, pid: u32) -> bool {
    if !x11::window_info(id as isize).is_some_and(|window| window.pid == pid) { return false; }
    x11::set_title(id, &linux::hyprland::note_title(id));
    x11::accept_pointer(id);
    x11::prepare_floating(id, linux::hyprland::managed_note_windows())
}

pub(in crate::capture::desktop) fn place_note_surface(id: u32, pid: u32, rect: [i32; 4], focus: bool, popup: bool) {
    let index = usize::from(popup);
    {
        let state = NOTE_SURFACES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.pid != pid { return; }
    }
    if !x11::window_info(id as isize).is_some_and(|window| window.pid == pid) { return; }
    x11::set_title(id, &linux::hyprland::note_title(id));
    x11::accept_pointer(id);
    x11::place_without_remap(id, rect.map(f64::from));
    let start_worker = {
        let mut state = NOTE_SURFACES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.pid != pid { return; }
        let slot = &mut state.slots[index];
        slot.id = id; slot.rect = rect; slot.focus = focus; slot.generation = slot.generation.wrapping_add(1);
        let start = !slot.worker && linux::hyprland::running();
        slot.worker |= start;
        start
    };
    raise_note_surfaces();
    if focus { x11::focus(id); }
    if start_worker { std::thread::spawn(move || pin_note_surface(id, pid, index)); }
}
