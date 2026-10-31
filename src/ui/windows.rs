//! The Windows shell around the shared UI: the notification area icon and its menu (the same
//! items as the macOS menu bar menu), Win32 window styles for the pill, picker and ink overlay
//! (always on top, never activating, left out of recordings), and the review window's caption
//! buttons.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicIsize, Ordering};

use gpui::{
    AnyElement, App, Div, InteractiveElement, IntoElement, ParentElement, Pixels, Point, Styled,
    Window, WindowControlArea, div, px, rgb, rgba,
};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use tray_icon::menu::{IsMenuItem, Menu, MenuEvent, MenuId, MenuItem, PredefinedMenuItem};
use tray_icon::{MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use windows::Win32::Foundation::HWND;
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetWindowLongPtrW, GetWindowThreadProcessId, HWND_TOPMOST, IsWindow,
    SPI_GETCLIENTAREAANIMATION, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SWP_SHOWWINDOW, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetForegroundWindow, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, SystemParametersInfoW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST,
};
use windows::core::BOOL;

use super::{LaserOverlay, MenuAction, PILL_WINDOW_H, PILL_WINDOW_W, TEXT_COLOR};
use crate::capture::SharedCaptureTelemetry;
use crate::win;

/// Where the file browser is called in buttons ("Show in File Explorer").
pub(super) const FILE_MANAGER: &str = "File Explorer";
/// No traffic lights: the review header starts at the usual inset and ends at the caption
/// buttons.
pub(super) const HEADER_LEAD: f32 = 16.0;
pub(super) const HEADER_TRAIL: f32 = 0.0;
/// The pill's top edge on the screen.
pub(super) const PILL_TOP: f32 = 14.0;

/// GPUI lays out in DIPs; Scrawl's global coordinates are pixels.
pub(super) fn points_per_pixel(window: &Window) -> f32 {
    1.0 / window.scale_factor().max(0.25)
}

/// The GPUI display for the monitor at `frame` (global pixels), else the primary one.
pub(super) fn display_at(
    frame: [f32; 4],
    cx: &App,
) -> Option<std::rc::Rc<dyn gpui::PlatformDisplay>> {
    let [x, y, w, h] = frame.map(f64::from);
    win::monitor_at(x + w / 2.0, y + h / 2.0)
        .and_then(|monitor| gpui_display_for(&monitor, cx))
        .or_else(|| cx.primary_display())
}

/// A window covering the monitor at `frame` (global pixels) lays out in DIPs: the display's size.
pub(super) fn layout_size(
    frame: [f32; 4],
    display: Option<&dyn gpui::PlatformDisplay>,
) -> gpui::Size<gpui::Pixels> {
    display.map_or_else(
        || {
            let [x, y, w, h] = frame.map(f64::from);
            let scale = win::monitor_at(x + w / 2.0, y + h / 2.0).map_or(1.0, |m| m.scale) as f32;
            gpui::size(px(frame[2] / scale), px(frame[3] / scale))
        },
        |d| d.bounds().size,
    )
}

/// GPUI lays displays out in DIPs: the one whose origin, scaled, is the monitor's.
pub(crate) fn gpui_display_for(
    monitor: &win::Monitor,
    cx: &App,
) -> Option<std::rc::Rc<dyn gpui::PlatformDisplay>> {
    let s = monitor.scale as f32;
    cx.displays().into_iter().find(|d| {
        let b = d.bounds();
        (f32::from(b.origin.x) * s - monitor.rect[0] as f32).abs() < 2.0
            && (f32::from(b.origin.y) * s - monitor.rect[1] as f32).abs() < 2.0
    })
}

/// The open picker's and pill's windows, to move them when they follow the pointer or the
/// recording to another display.
static PICKER_HWND: AtomicIsize = AtomicIsize::new(0);
static PILL_HWND: AtomicIsize = AtomicIsize::new(0);

/// The window in `slot`, while it's still one of Scrawl's.
fn own_window(slot: &AtomicIsize) -> Option<isize> {
    let id = slot.load(Ordering::Acquire);
    if id == 0 {
        return None;
    }
    let hwnd = win::hwnd(id);
    let mut pid = 0;
    unsafe {
        if !IsWindow(Some(hwnd)).as_bool() {
            return None;
        }
        GetWindowThreadProcessId(hwnd, Some(&raw mut pid));
    }
    (pid == std::process::id()).then_some(id)
}

/// GPUI windows are placed in global DIPs on Windows: the display's own origin.
pub(super) fn window_origin(display: Option<&dyn gpui::PlatformDisplay>) -> Point<Pixels> {
    display.map_or_else(Point::default, |d| d.bounds().origin)
}

/// Moves the open picker to cover the monitor at `frame` (global pixels). On the next turn of
/// the event loop: from inside an update, GPUI's resize and DPI callbacks can't reach the app,
/// and the window would keep its old layout size and scale.
pub(super) fn move_picker_window(frame: [f64; 4], cx: &mut App) {
    if let Some(id) = own_window(&PICKER_HWND) {
        cx.spawn(async move |_| win::overlay::place(id, frame))
            .detach();
    }
}

/// The picker takes the keys itself (`configure_picker_window`); the review window opens in front.
pub(super) fn take_keys(_cx: &mut App) {}

/// Windows activates the window below when Scrawl's closes.
pub(super) fn hand_back_keys(_cx: &mut App) {}

/// Pressing a region drags its window (`HTCAPTION`), with Aero Snap.
pub(super) fn window_drag_region(region: Div) -> Div {
    region
        .occlude()
        .window_control_area(WindowControlArea::Drag)
}

/// A control inside a `window_drag_region`. Windows hit-tests a press before it is delivered:
/// over a control that doesn't occlude, the region answers `HTCAPTION`, and the caption's move
/// loop swallows the mouse-up, so the click never fires.
pub(super) fn drag_region_control<E: InteractiveElement>(control: E) -> E {
    control.occlude()
}

/// Minimize, maximize and close, drawn with Segoe Fluent Icons like every Windows 11 app.
pub(super) fn caption_buttons() -> Option<AnyElement> {
    let glyph_font = if win::shell::windows_11() {
        "Segoe Fluent Icons"
    } else {
        "Segoe MDL2 Assets"
    };
    let button = |id: &'static str, glyph: &'static str, area: WindowControlArea, close: bool| {
        div()
            .id(id)
            .occlude()
            .window_control_area(area)
            .w(px(46.0))
            .h_full()
            .flex()
            .items_center()
            .justify_center()
            .font_family(glyph_font)
            .text_size(px(10.0))
            .text_color(rgb(TEXT_COLOR))
            .hover(move |style| {
                if close {
                    style.bg(rgb(0xc42b1c)).text_color(rgb(0xffffff))
                } else {
                    style.bg(rgba(0xffffff14))
                }
            })
            .child(glyph)
    };
    Some(
        div()
            .flex_none()
            .h_full()
            .ml_4()
            .flex()
            .child(button(
                "caption-min",
                "\u{E921}",
                WindowControlArea::Min,
                false,
            ))
            .child(button(
                "caption-max",
                "\u{E922}",
                WindowControlArea::Max,
                false,
            ))
            .child(button(
                "caption-close",
                "\u{E8BB}",
                WindowControlArea::Close,
                true,
            ))
            .into_any_element(),
    )
}

/// Windows' "Animation effects" setting (Settings › Accessibility › Visual effects).
pub(super) fn reduce_motion() -> bool {
    let mut enabled = BOOL(1);
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETCLIENTAREAANIMATION,
            0,
            Some((&raw mut enabled).cast()),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    ok.is_ok() && !enabled.as_bool()
}

/// Updates come from GitHub releases on Windows; the menu item opens the page.
pub(super) fn start_updates() {}

pub(super) fn check_for_updates() {
    let _ = crate::handoff::open_path(Path::new(
        "https://github.com/kitlangton/scrawl/releases/latest",
    ));
}

pub(super) fn open_overlay(
    telemetry: Arc<SharedCaptureTelemetry>,
    on: Option<[f64; 4]>,
    cx: &mut gpui::Context<super::ScrawlCoordinator>,
) -> Result<LaserOverlay, String> {
    LaserOverlay::open(telemetry, on, cx)
}

pub(crate) fn hwnd_of(window: &Window) -> Option<HWND> {
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(HWND(handle.hwnd.get() as *mut _)),
        _ => None,
    }
}

/// The exe's icon (resource 1) on a window's title bar, taskbar button and Alt+Tab entry; GPUI
/// registers its window class without one.
pub(crate) fn set_app_icon(window: &Window) {
    use windows::Win32::Foundation::{HINSTANCE, LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        GWLP_HINSTANCE, GetSystemMetrics, ICON_BIG, ICON_SMALL, IMAGE_ICON, LR_DEFAULTCOLOR,
        LoadImageW, SM_CXICON, SM_CXSMICON, SM_CYICON, SM_CYSMICON, SendMessageW, WM_SETICON,
    };
    let Some(hwnd) = hwnd_of(window) else {
        return;
    };
    unsafe {
        let module = HINSTANCE(GetWindowLongPtrW(hwnd, GWLP_HINSTANCE) as *mut _);
        for (kind, w, h) in [
            (ICON_SMALL, SM_CXSMICON, SM_CYSMICON),
            (ICON_BIG, SM_CXICON, SM_CYICON),
        ] {
            if let Ok(icon) = LoadImageW(
                Some(module),
                // MAKEINTRESOURCE(1): resource ids travel as pointers.
                windows::core::PCWSTR(std::ptr::without_provenance(1)),
                IMAGE_ICON,
                GetSystemMetrics(w),
                GetSystemMetrics(h),
                LR_DEFAULTCOLOR,
            ) {
                SendMessageW(
                    hwnd,
                    WM_SETICON,
                    Some(WPARAM(kind as usize)),
                    Some(LPARAM(icon.0 as isize)),
                );
            }
        }
    }
}

/// Brings one of Scrawl's floating windows to the top of the always-on-top band.
pub(super) fn raise_window(window: &mut Window) {
    if let Some(hwnd) = hwnd_of(window) {
        let _ = unsafe {
            SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            )
        };
    }
}

/// `scrawl lab`'s backdrop stays an ordinary popup on Windows.
pub(super) fn configure_lab_backdrop(_w: f32, _h: f32) {}

struct KeepAlive;

impl gpui::Render for KeepAlive {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
    }
}

/// GPUI on Windows quits when its last window closes, and Scrawl lives in the notification
/// area with no window most of the time: a hidden window keeps it running.
pub(super) fn keep_alive(cx: &mut App) {
    let _ = cx.open_window(
        gpui::WindowOptions {
            window_bounds: Some(gpui::WindowBounds::Windowed(gpui::Bounds {
                origin: gpui::point(px(-32000.0), px(-32000.0)),
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
        |_, cx| gpui::AppContext::new(cx, |_| KeepAlive),
    );
}

/// Opens a window Scrawl's way on this platform.
pub(super) fn decorate_app_window(window: &mut Window) {
    set_app_icon(window);
}

/// Floating chrome for Scrawl's own popups: no taskbar button, always on top, never in a
/// recording, no Windows 11 border or shadow, and no veil from GPUI's transparent background.
pub(crate) fn style_floating(hwnd: HWND, activates: bool) {
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
    let mut extra = WS_EX_TOOLWINDOW.0 | WS_EX_TOPMOST.0;
    if !activates {
        extra |= WS_EX_NOACTIVATE.0;
    }
    unsafe { SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (style | extra) as isize) };
    win::overlay::strip_chrome(hwnd);
    win::overlay::exclude_from_capture(hwnd);
}

/// The picker: on top of everything over the whole display at `frame` (global pixels), taking
/// clicks and keys (in `scrawl lab` replays too: a window made click-through after its first
/// frame stops drawing).
pub(super) fn configure_picker_window(
    window: &mut Window,
    _lab: bool,
    frame: [f64; 4],
    cx: &mut App,
) -> Option<u32> {
    let hwnd = hwnd_of(window)?;
    style_floating(hwnd, true);
    let id = hwnd.0 as isize;
    PICKER_HWND.store(id, Ordering::Release);
    // Placed and brought forward once `open_window` has returned (see `move_picker_window`).
    cx.spawn(async move |_| {
        win::overlay::place(id, frame);
        let _ = unsafe { SetForegroundWindow(win::hwnd(id)) };
    })
    .detach();
    Some(hwnd.0 as usize as u32)
}

/// The pill window's rect (global pixels) on the display holding `point`.
fn pill_rect_at(point: [f64; 2]) -> Option<[f64; 4]> {
    let screens = crate::screens::displays();
    let screen = screens.get(crate::screens::index_at(&screens, point)?)?;
    Some(crate::screens::pill_window_rect(
        screen,
        f64::from(PILL_WINDOW_W),
        f64::from(PILL_WINDOW_H),
        f64::from(PILL_TOP),
    ))
}

/// Where the pill window opens on `display` (GPUI's global DIPs): below the work area's top.
pub(super) fn hud_pill_origin(display: gpui::Bounds<Pixels>) -> Point<Pixels> {
    let origin = [
        f64::from(f32::from(display.origin.x)),
        f64::from(f32::from(display.origin.y)),
    ];
    let screen = crate::screens::displays().into_iter().find(|s| {
        (origin[0] * s.scale - s.frame[0]).abs() < 2.0
            && (origin[1] * s.scale - s.frame[1]).abs() < 2.0
    });
    match screen {
        Some(s) => {
            let [x, y, _, _] = crate::screens::pill_window_rect(
                &s,
                f64::from(PILL_WINDOW_W),
                f64::from(PILL_WINDOW_H),
                f64::from(PILL_TOP),
            );
            gpui::point(px((x / s.scale) as f32), px((y / s.scale) as f32))
        }
        None => gpui::point(
            display.origin.x + (display.size.width - px(PILL_WINDOW_W)) * 0.5,
            display.origin.y + px(PILL_TOP),
        ),
    }
}

/// Moves the pill window to the display holding `point` (global pixels). Off the UI thread, as
/// the ink overlay's `move_to` is: this runs inside an update, where GPUI's resize and DPI
/// callbacks can't reach the app; the window's thread handles the move from its loop.
pub(super) fn place_hud_pill(point: [f64; 2]) {
    let (Some(id), Some(rect)) = (own_window(&PILL_HWND), pill_rect_at(point)) else {
        return;
    };
    std::thread::spawn(move || win::overlay::place(id, rect));
}

/// The pill's window. Windows has no native shell for it: GPUI draws the capsule, its shadow
/// and its springs, and the window never takes focus from the app being recorded.
pub(super) struct PillChrome {
    hwnd: Option<HWND>,
}

impl Drop for PillChrome {
    fn drop(&mut self) {
        if let Some(hwnd) = self.hwnd {
            let _ =
                PILL_HWND.compare_exchange(hwnd.0 as isize, 0, Ordering::AcqRel, Ordering::Acquire);
        }
    }
}

impl PillChrome {
    /// GPUI opens the window on the display in use (`hud_pill_origin`); it's sized to the
    /// pixel once `open_window` has returned, past the frame GPUI keeps around it.
    pub(super) fn open(window: &mut Window, cx: &mut App) -> Self {
        let hwnd = hwnd_of(window);
        if let Some(hwnd) = hwnd {
            style_floating(hwnd, false);
            let _ = unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
            let id = hwnd.0 as isize;
            PILL_HWND.store(id, Ordering::Release);
            let bounds = window.bounds();
            let scale = f64::from(window.scale_factor());
            let centre = bounds.center();
            let at = [
                f64::from(f32::from(centre.x)) * scale,
                f64::from(f32::from(centre.y)) * scale,
            ];
            if let Some(rect) = pill_rect_at(at) {
                cx.spawn(async move |_| win::overlay::place(id, rect))
                    .detach();
            }
        }
        Self { hwnd }
    }

    pub(super) fn window_number(&self) -> Option<u32> {
        self.hwnd.map(|hwnd| hwnd.0 as usize as u32)
    }

    pub(super) fn has_shell(&self) -> bool {
        false
    }

    pub(super) fn raise(&self) {
        if let Some(hwnd) = self.hwnd {
            let _ = unsafe {
                SetWindowPos(
                    hwnd,
                    Some(HWND_TOPMOST),
                    0,
                    0,
                    0,
                    0,
                    SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
                )
            };
        }
    }

    /// Motion traces need the native shell's geometry, which Windows doesn't have.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn trace(
        &self,
        _width: f32,
        _scale: f32,
        _y_offset: f32,
        _opacity: f32,
        _content_alpha: f32,
        _blur_radius: f32,
        _content_shrink: f32,
    ) -> Option<super::motion::Xf> {
        None
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply(
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

/// Whether the pointer is over the pill's window (the capsule plus a small margin).
pub(super) fn pointer_on_hud_pill() -> bool {
    let Some((x, y)) = win::cursor_position() else {
        return false;
    };
    let pid = std::process::id();
    win::top_level_windows().iter().any(|w| {
        let [wx, wy, ww, wh] = w.rect;
        // The pill doesn't activate; unlike the ink overlay, it takes clicks.
        w.pid == pid
            && w.ex_style & WS_EX_NOACTIVATE.0 != 0
            && w.ex_style & windows::Win32::UI::WindowsAndMessaging::WS_EX_TRANSPARENT.0 == 0
            && x >= wx
            && x <= wx + ww
            && y >= wy
            && y <= wy + wh
    })
}

/// The largest of Scrawl's windows (for `scrawl preview --capture-screenshot`).
pub(super) fn front_own_window() -> Option<isize> {
    let pid = std::process::id();
    win::top_level_windows()
        .into_iter()
        .filter(|w| w.pid == pid && w.rect[2] >= 600.0 && w.rect[3] >= 400.0)
        .max_by_key(|w| (w.rect[2] * w.rect[3]) as i64)
        .map(|w| w.hwnd)
}

pub(super) fn capture_own_window_screenshot(
    window: Option<isize>,
    out_path: &Path,
) -> anyhow::Result<()> {
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let window = window.ok_or_else(|| anyhow::anyhow!("Could not find Scrawl's window"))?;
    let frame = win::overlay::capture_window(window)?;
    crate::render::save_png(&frame, out_path)
}

// MARK: The notification area menu

const ID_TOGGLE: &str = "toggle-capture";
const ID_CHOOSE: &str = "choose-target";
const ID_OPEN_LATEST: &str = "open-latest";
const ID_EXPORT: &str = "export-video";
const ID_SETTINGS: &str = "settings";
const ID_UPDATES: &str = "check-updates";
const ID_QUIT: &str = "quit";
const INBOX_PREFIX: &str = "inbox:";

struct Tray {
    icon: TrayIcon,
    menu: Menu,
    toggle: MenuItem,
    export: MenuItem,
    /// Whether `export` is in the menu (`"videoExport"` can change while Scrawl runs).
    export_shown: bool,
    /// Inbox rows, replaced each time the menu opens.
    dynamic: Vec<Box<dyn IsMenuItem>>,
    status: Option<String>,
    inbox: usize,
}

thread_local! {
    static TRAY: RefCell<Option<Tray>> = const { RefCell::new(None) };
    static SHOW_MENU: RefCell<Option<flume::Receiver<()>>> = const { RefCell::new(None) };
}

/// The app icon, box-filtered down to the notification area's 32 px.
fn tray_icon_image() -> Option<tray_icon::Icon> {
    let decoder = png::Decoder::new(std::io::Cursor::new(
        &include_bytes!("../../assets/app-icon.png")[..],
    ));
    let mut reader = decoder.read_info().ok()?;
    let mut buffer = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buffer).ok()?;
    if info.color_type != png::ColorType::Rgba {
        return None;
    }
    let (sw, sh) = (info.width as usize, info.height as usize);
    let size = 32_usize;
    let mut out = vec![0_u8; size * size * 4];
    for y in 0..size {
        for x in 0..size {
            let (x0, x1) = (x * sw / size, (x + 1) * sw / size);
            let (y0, y1) = (y * sh / size, (y + 1) * sh / size);
            let mut acc = [0_u64; 4];
            let mut n = 0_u64;
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let p = &buffer[(sy * sw + sx) * 4..][..4];
                    let a = u64::from(p[3]);
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
    tray_icon::Icon::from_rgba(out, size as u32, size as u32).ok()
}

/// "Record Screen" with the record shortcut as it is now (Settings can change it).
fn record_title() -> String {
    format!(
        "Record Screen\t{}",
        crate::settings::Shortcuts::load().record.label
    )
}

pub(super) fn install_status_bar() -> Option<flume::Receiver<MenuAction>> {
    let (sender, receiver) = flume::bounded(16);
    let (show_tx, show_rx) = flume::bounded(4);

    let menu = Menu::new();
    let toggle = MenuItem::with_id(ID_TOGGLE, record_title(), true, None);
    let export = MenuItem::with_id(ID_EXPORT, "Export Last Capture as Video…", true, None);
    let _ = menu.append(&toggle);
    let _ = menu.append(&MenuItem::with_id(
        ID_CHOOSE,
        "Choose What to Record…",
        true,
        None,
    ));
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&MenuItem::with_id(
        ID_OPEN_LATEST,
        "Open Latest Capture",
        true,
        None,
    ));
    let export_shown = crate::handoff::video_export();
    if export_shown {
        let _ = menu.append(&export);
    }
    let _ = menu.append(&PredefinedMenuItem::separator());
    let _ = menu.append(&MenuItem::with_id(ID_SETTINGS, "Settings…", true, None));
    let _ = menu.append(&MenuItem::with_id(
        ID_UPDATES,
        "Check for Updates…",
        true,
        None,
    ));
    let _ = menu.append(&MenuItem::with_id(ID_QUIT, "Quit Scrawl", true, None));

    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if let Some(action) = menu_action(event.id()) {
            let _ = sender.try_send(action);
        }
    }));
    // Both buttons open the menu, rebuilt first so its rows are current.
    TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
        if let TrayIconEvent::Click {
            button_state: MouseButtonState::Up,
            ..
        } = event
        {
            let _ = show_tx.try_send(());
        }
    }));

    let icon = TrayIconBuilder::new()
        .with_tooltip("Scrawl")
        .with_icon(tray_icon_image()?)
        .with_menu(Box::new(menu.clone()))
        .with_menu_on_left_click(false)
        .with_menu_on_right_click(false)
        .build()
        .map_err(|error| crate::capture::scrawl_log(format!("Tray icon failed: {error}")))
        .ok()?;

    TRAY.with(|slot| {
        *slot.borrow_mut() = Some(Tray {
            icon,
            menu,
            toggle,
            export,
            export_shown,
            dynamic: Vec::new(),
            status: None,
            inbox: 0,
        });
    });
    SHOW_MENU.with(|slot| *slot.borrow_mut() = Some(show_rx));
    Some(receiver)
}

fn menu_action(id: &MenuId) -> Option<MenuAction> {
    let id = id.as_ref();
    Some(match id {
        ID_TOGGLE => MenuAction::ToggleCapture,
        ID_CHOOSE => MenuAction::ChooseTarget,
        ID_OPEN_LATEST => MenuAction::OpenLatest,
        ID_EXPORT => MenuAction::ExportVideo,
        ID_SETTINGS => MenuAction::Settings,
        ID_UPDATES => MenuAction::CheckForUpdates,
        ID_QUIT => MenuAction::Quit,
        _ => MenuAction::OpenInbox(PathBuf::from(id.strip_prefix(INBOX_PREFIX)?)),
    })
}

/// Brings the menu's rows up to date, as macOS's `menuNeedsUpdate:` does.
fn refresh(tray: &mut Tray) {
    tray.toggle.set_text(record_title());
    for item in tray.dynamic.drain(..) {
        let _ = tray.menu.remove(item.as_ref());
    }
    let export = crate::handoff::video_export();
    if export != tray.export_shown {
        if export {
            // After Open Latest Capture: the record rows, a separator, then Open Latest.
            let _ = tray.menu.insert(&tray.export, 4);
        } else {
            let _ = tray.menu.remove(&tray.export);
        }
        tray.export_shown = export;
    }
    let inbox = crate::inbox::Inbox::load_default();
    if inbox.len() > 0 {
        let now = crate::now_epoch_ms();
        let mut rows: Vec<Box<dyn IsMenuItem>> =
            vec![Box::new(MenuItem::new("From iPhone", false, None))];
        for entry in inbox.entries() {
            rows.push(Box::new(MenuItem::with_id(
                format!("{INBOX_PREFIX}{}", entry.dir.display()),
                format!("    {}", entry.label(now)),
                true,
                None,
            )));
        }
        rows.push(Box::new(PredefinedMenuItem::separator()));
        for (index, row) in rows.iter().enumerate() {
            let _ = tray.menu.insert(row.as_ref(), index);
        }
        tray.dynamic = rows;
    }
}

/// Called from the UI loop: opens the menu when the icon was clicked.
pub(super) fn poll_status_bar() {
    let clicked = SHOW_MENU.with(|slot| {
        slot.borrow()
            .as_ref()
            .is_some_and(|rx| rx.drain().count() > 0)
    });
    if !clicked {
        return;
    }
    TRAY.with(|slot| {
        if let Some(tray) = slot.borrow_mut().as_mut() {
            refresh(tray);
            tray.icon.show_menu();
        }
    });
}

fn update_tooltip(tray: &Tray) {
    let text = match (&tray.status, tray.inbox) {
        (Some(status), _) => format!("Scrawl — {status}"),
        (None, 0) => "Scrawl".to_owned(),
        (None, 1) => "Scrawl — 1 capture from iPhone".to_owned(),
        (None, n) => format!("Scrawl — {n} captures from iPhone"),
    };
    let _ = tray.icon.set_tooltip(Some(text));
}

/// A video export's progress, in the icon's tooltip (Windows icons can't show text).
pub(super) fn set_status_text(text: &str) {
    TRAY.with(|slot| {
        if let Some(tray) = slot.borrow_mut().as_mut() {
            tray.status = Some(text.to_owned());
            update_tooltip(tray);
        }
    });
}

/// The unhandled iPhone capture count, in the icon's tooltip.
pub(super) fn set_status_inbox_count(count: usize) {
    TRAY.with(|slot| {
        if let Some(tray) = slot.borrow_mut().as_mut() {
            tray.status = None;
            tray.inbox = count;
            update_tooltip(tray);
        }
    });
}
