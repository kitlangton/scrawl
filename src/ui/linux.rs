//! The Linux shell around the shared UI: the tray icon and its menu (StatusNotifierItem, the
//! same items as the macOS menu bar menu), and the X11 properties that float the pill, picker
//! and ink overlay above every window (override-redirect, on top, click-through).
//!
//! GPUI's X11 backend sees the whole X screen as one display with its origin at (0, 0), so a
//! window's DIPs are root-window pixels divided by the scale. Scrawl places its floating
//! windows itself, in pixels, once GPUI has mapped them (`linux::x11::float`).

use std::cell::RefCell;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use gpui::{AnyElement, App, Div, IntoElement, Pixels, Point, Window, div, px};
use ksni::blocking::TrayMethods as _;

use super::{LaserOverlay, MenuAction, PILL_WINDOW_H, PILL_WINDOW_W};
use crate::capture::SharedCaptureTelemetry;
use crate::linux::{self, x11};

/// Where the file browser is called in buttons ("Show in Files").
pub(super) const FILE_MANAGER: &str = "Files";
/// The desktop draws the title bar and its buttons: the review header starts at the usual inset.
pub(super) const HEADER_LEAD: f32 = 16.0;
pub(super) const HEADER_TRAIL: f32 = 0.0;
/// The pill's top edge on the screen, below the work area's top (GNOME's top bar).
pub(super) const PILL_TOP: f32 = 14.0;

/// GPUI lays out in DIPs; Scrawl's global coordinates are pixels.
pub(super) fn points_per_pixel(window: &Window) -> f32 {
    1.0 / window.scale_factor().max(0.25)
}

/// The middle of the rect a window was last asked to open on (global pixels): GPUI has one
/// display for the whole X screen, so `hud_pill_origin` needs to know which monitor.
static LAST_FRAME_CENTRE: Mutex<Option<[f64; 2]>> = Mutex::new(None);

/// GPUI's one display (the X screen), remembering which monitor `frame` (global pixels) is on.
pub(super) fn display_at(
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
pub(super) fn layout_size(
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

fn own_window(slot: &AtomicU32) -> Option<u32> {
    let id = slot.load(Ordering::Acquire);
    (id != 0).then_some(id)
}

/// Where GPUI first puts the picker; `configure_picker_window` moves it onto its monitor.
pub(super) fn window_origin(_display: Option<&dyn gpui::PlatformDisplay>) -> Point<Pixels> {
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
pub(super) fn move_picker_window(frame: [f64; 4], cx: &mut App) {
    if let Some(id) = own_window(&PICKER_WINDOW) {
        cx.spawn(async move |_| x11::place(id, frame)).detach();
    }
}

/// The picker takes the keys itself (`configure_picker_window`).
pub(super) fn take_keys(_cx: &mut App) {}

/// The window manager focuses the window below when Scrawl's closes.
pub(super) fn hand_back_keys(_cx: &mut App) {}

/// The desktop's title bar drags the window.
pub(super) fn window_drag_region(region: Div) -> Div {
    region
}

/// Linux's title bar owns dragging; controls in the content receive clicks normally.
pub(super) fn drag_region_control<E>(control: E) -> E {
    control
}

/// The desktop draws the window's buttons.
pub(super) fn caption_buttons() -> Option<AnyElement> {
    None
}

/// GNOME's "Reduce animation" (Settings › Accessibility › Seeing), read once.
pub(super) fn reduce_motion() -> bool {
    static REDUCED: OnceLock<bool> = OnceLock::new();
    *REDUCED.get_or_init(|| {
        std::process::Command::new("gsettings")
            .args(["get", "org.gnome.desktop.interface", "enable-animations"])
            .output()
            .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).trim() == "false")
    })
}

/// Updates come from GitHub releases on Linux; the menu item opens the page.
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

/// The app icon, box-filtered down to `size`, as RGBA.
fn icon_rgba(size: usize) -> Option<Vec<u8>> {
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
    Some(out)
}

/// The app icon on a window (`_NET_WM_ICON`) and its class, so the dock and the window switcher
/// show Scrawl's icon and match it to `scrawl.desktop`.
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

/// Brings one of Scrawl's floating windows to the top.
pub(super) fn raise_window(window: &mut Window) {
    if let Some(id) = x11::window_id(window) {
        x11::raise(id);
    }
}

/// `scrawl lab`'s backdrop stays an ordinary popup on Linux.
pub(super) fn configure_lab_backdrop(_w: f32, _h: f32) {}

struct KeepAlive;

impl gpui::Render for KeepAlive {
    fn render(&mut self, _: &mut Window, _: &mut gpui::Context<Self>) -> impl IntoElement {
        div()
    }
}

/// GPUI on Linux quits when its last window closes, and Scrawl lives in the tray with no window
/// most of the time: an unmanaged 1 px window off the screen keeps it running.
pub(super) fn keep_alive(cx: &mut App) {
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

/// Opens a window Scrawl's way on this platform.
pub(super) fn decorate_app_window(window: &mut Window) {
    set_app_icon(window);
}

/// The picker: on top of everything over the whole monitor at `frame` (global pixels), taking
/// clicks and keys.
pub(super) fn configure_picker_window(
    window: &mut Window,
    _lab: bool,
    frame: [f64; 4],
    cx: &mut App,
) -> Option<u32> {
    let id = x11::window_id(window)?;
    PICKER_WINDOW.store(id, Ordering::Release);
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
    let screens = crate::screens::displays();
    let screen = screens.get(crate::screens::index_at(&screens, point)?)?;
    Some(crate::screens::pill_window_rect(
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
        .or_else(crate::screens::pointer)
}

/// Where the pill window opens (DIPs of GPUI's one display): below the work area's top on the
/// monitor in use.
pub(super) fn hud_pill_origin(display: gpui::Bounds<Pixels>) -> Point<Pixels> {
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
pub(super) fn place_hud_pill(point: [f64; 2]) {
    let (Some(id), Some(rect)) = (own_window(&PILL_WINDOW), pill_rect_at(point)) else {
        return;
    };
    std::thread::spawn(move || x11::place(id, rect));
}

/// The pill's window. Linux has no native shell for it: GPUI draws the capsule, its shadow and
/// its springs, and the window never takes focus from the app being recorded.
pub(super) struct PillChrome {
    id: Option<u32>,
}

impl Drop for PillChrome {
    fn drop(&mut self) {
        if let Some(id) = self.id {
            let _ = PILL_WINDOW.compare_exchange(id, 0, Ordering::AcqRel, Ordering::Acquire);
        }
    }
}

impl PillChrome {
    pub(super) fn open(window: &mut Window, _cx: &mut App) -> Self {
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
            x11::float(id, rect);
        }
        Self { id }
    }

    pub(super) fn window_number(&self) -> Option<u32> {
        self.id
    }

    pub(super) fn has_shell(&self) -> bool {
        false
    }

    pub(super) fn raise(&self) {
        if let Some(id) = self.id {
            x11::raise(id);
        }
    }

    /// Motion traces need the native shell's geometry, which Linux doesn't have.
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

/// Whether the pointer is over the pill's window.
pub(super) fn pointer_on_hud_pill() -> bool {
    let (Some(id), Some((x, y))) = (own_window(&PILL_WINDOW), linux::cursor_position()) else {
        return false;
    };
    x11::geometry(id).is_some_and(|[wx, wy, ww, wh]| {
        x >= wx && x <= wx + ww && y >= wy && y <= wy + wh
    })
}

/// The largest of Scrawl's windows (for `scrawl preview --capture-screenshot`).
pub(super) fn front_own_window() -> Option<isize> {
    let pid = std::process::id();
    linux::top_level_windows()
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
    let info = linux::window_info(window)
        .ok_or_else(|| anyhow::anyhow!("Scrawl's window went away"))?;
    let frame = x11::capture_rect(info.rect)?;
    crate::render::save_png(&frame, out_path)
}

// MARK: The tray menu

/// The tray icon (a StatusNotifierItem: KDE Plasma, and GNOME with the AppIndicator extension,
/// which Ubuntu turns on). Its menu is rebuilt whenever the host asks, so it's always current.
struct ScrawlTray {
    actions: flume::Sender<MenuAction>,
    status: Option<String>,
    inbox: usize,
}

/// "Record Screen" with the record shortcut as it is now (Settings can change it).
fn record_title() -> String {
    format!(
        "Record Screen ({})",
        crate::settings::Shortcuts::load().record.label
    )
}

impl ScrawlTray {
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

impl ksni::Tray for ScrawlTray {
    fn id(&self) -> String {
        "dev.kitlangton.scrawl".into()
    }

    fn title(&self) -> String {
        "Scrawl".into()
    }

    fn icon_pixmap(&self) -> Vec<ksni::Icon> {
        static ICONS: OnceLock<Vec<(usize, Vec<u8>)>> = OnceLock::new();
        ICONS
            .get_or_init(|| {
                [22_usize, 32, 48]
                    .into_iter()
                    .filter_map(|size| {
                        let mut argb = icon_rgba(size)?;
                        // RGBA → ARGB, in network byte order.
                        for p in argb.chunks_exact_mut(4) {
                            p.rotate_right(1);
                        }
                        Some((size, argb))
                    })
                    .collect()
            })
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
            title: "Scrawl".into(),
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
        let inbox = crate::inbox::Inbox::load_default();
        if inbox.len() > 0 {
            let now = crate::now_epoch_ms();
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
        items.push(ksni::MenuItem::Separator);
        items.push(self.item("Open Latest Capture", MenuAction::OpenLatest));
        if crate::handoff::video_export() {
            items.push(self.item("Export Last Capture as Video…", MenuAction::ExportVideo));
        }
        items.push(ksni::MenuItem::Separator);
        items.push(self.item("Settings…", MenuAction::Settings));
        items.push(self.item("Check for Updates…", MenuAction::CheckForUpdates));
        items.push(self.item("Quit Scrawl", MenuAction::Quit));
        items
    }
}

thread_local! {
    static TRAY: RefCell<Option<ksni::blocking::Handle<ScrawlTray>>> = const { RefCell::new(None) };
}

pub(super) fn install_status_bar() -> Option<flume::Receiver<MenuAction>> {
    let (sender, receiver) = flume::bounded(16);
    // A second launch (`linux::ipc`) opens Settings: GNOME without the AppIndicator extension
    // shows no tray icon, and this is how to reach Scrawl there.
    let open = linux::ipc::open_requests();
    let open_sender = sender.clone();
    let _ = std::thread::Builder::new()
        .name("scrawl-open-requests".into())
        .spawn(move || {
            while open.recv().is_ok() {
                let _ = open_sender.try_send(MenuAction::Settings);
            }
        });
    let tray = ScrawlTray {
        actions: sender,
        status: None,
        inbox: 0,
    };
    match tray.spawn() {
        Ok(handle) => TRAY.with(|slot| *slot.borrow_mut() = Some(handle)),
        Err(error) => {
            crate::capture::scrawl_log(format!(
                "Tray icon unavailable ({error}); launch Scrawl again to open Settings"
            ));
            linux::shell::notify(
                "Scrawl is running",
                &format!(
                    "Press {} to record. Your desktop shows no tray icons, so open Scrawl again for Settings.",
                    crate::settings::Shortcuts::load().record.label
                ),
            );
        }
    }
    Some(receiver)
}

/// Called from the UI loop: refreshes the menu now and then (the record shortcut's label and the
/// export item can change while Scrawl runs).
pub(super) fn poll_status_bar() {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if last.is_some_and(|at| at.elapsed() < Duration::from_secs(5)) {
        return;
    }
    *last = Some(Instant::now());
    TRAY.with(|slot| {
        if let Some(handle) = slot.borrow().as_ref() {
            handle.update(|_| {});
        }
    });
}

/// A video export's progress, in the icon's tooltip.
pub(super) fn set_status_text(text: &str) {
    let text = text.to_owned();
    TRAY.with(|slot| {
        if let Some(handle) = slot.borrow().as_ref() {
            handle.update(move |tray| tray.status = Some(text));
        }
    });
}

/// The unhandled iPhone capture count, in the icon's tooltip.
pub(super) fn set_status_inbox_count(count: usize) {
    TRAY.with(|slot| {
        if let Some(handle) = slot.borrow().as_ref() {
            handle.update(move |tray| {
                tray.status = None;
                tray.inbox = count;
            });
        }
    });
}
