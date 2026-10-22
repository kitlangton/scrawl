//! The ink overlay on Windows: a click-through GPUI window over the display being recorded,
//! drawing `crate::ink_view::OverlayView`. It is left out of the recording
//! (`WDA_EXCLUDEFROMCAPTURE`).
//!
//! Also the Win32 window helpers Scrawl's other popups use.

use std::ffi::c_void;
use std::sync::{Arc, Mutex};

use gpui::{
    Bounds, Context, WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions, point,
    prelude::*, px, size,
};
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{
    DWMNCRP_DISABLED, DWMWA_BORDER_COLOR, DWMWA_NCRENDERING_POLICY, DWMWA_WINDOW_CORNER_PREFERENCE,
    DWMWCP_DONOTROUND, DwmSetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::ClientToScreen;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, ReleaseDC, SelectObject,
};
use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
use windows::Win32::System::LibraryLoader::{GetModuleHandleW, GetProcAddress};
use windows::Win32::UI::WindowsAndMessaging::{
    GWL_EXSTYLE, GetClientRect, GetWindowLongPtrW, GetWindowRect, HWND_TOPMOST, LWA_ALPHA,
    PostMessageW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SetLayeredWindowAttributes,
    SetWindowDisplayAffinity, SetWindowLongPtrW, SetWindowPos, ShowWindow, WDA_EXCLUDEFROMCAPTURE,
    WM_CLOSE, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
};
use windows::core::{PCSTR, w};

use crate::capture::SharedCaptureTelemetry;
use crate::ink_view::{OverlayView, Placement};
use crate::model::RgbaFrame;

/// Leaves a window out of every screen capture (Windows 10 2004 and later).
/// `SCRAWL_SHOW_OWN_WINDOWS=1` keeps them in, for screenshots of Scrawl itself.
pub fn exclude_from_capture(hwnd: HWND) {
    if std::env::var_os("SCRAWL_SHOW_OWN_WINDOWS").is_some() {
        return;
    }
    let _ = unsafe { SetWindowDisplayAffinity(hwnd, WDA_EXCLUDEFROMCAPTURE) };
}

/// Turns off what Windows adds around a transparent popup: Windows 11's rounded corners,
/// border and shadow (a ghost box), and GPUI's transparent-gradient accent (a faint veil;
/// DirectComposition already gives per-pixel alpha).
pub fn strip_chrome(hwnd: HWND) {
    let round = DWMWCP_DONOTROUND;
    let policy = DWMNCRP_DISABLED;
    unsafe {
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            (&raw const round).cast(),
            4,
        );
        let none = 0xFFFF_FFFE_u32; // DWMWA_COLOR_NONE
        let _ = DwmSetWindowAttribute(hwnd, DWMWA_BORDER_COLOR, (&raw const none).cast(), 4);
        let _ = DwmSetWindowAttribute(
            hwnd,
            DWMWA_NCRENDERING_POLICY,
            (&raw const policy).cast(),
            4,
        );
    }
    disable_accent(hwnd);
}

#[repr(C)]
struct AccentPolicy {
    state: u32,
    flags: u32,
    gradient: u32,
    animation_id: u32,
}

#[repr(C)]
struct CompositionData {
    attribute: u32,
    data: *mut c_void,
    size: usize,
}

type SetWindowCompositionAttribute = unsafe extern "system" fn(HWND, *mut CompositionData) -> i32;

/// `SetWindowCompositionAttribute(WCA_ACCENT_POLICY, ACCENT_DISABLED)`: user32 exports it
/// without a header.
fn disable_accent(hwnd: HWND) {
    unsafe {
        let Ok(user32) = GetModuleHandleW(w!("user32.dll")) else {
            return;
        };
        let Some(proc) = GetProcAddress(
            user32,
            PCSTR(c"SetWindowCompositionAttribute".as_ptr().cast()),
        ) else {
            return;
        };
        let set: SetWindowCompositionAttribute = std::mem::transmute(proc);
        let mut policy = AccentPolicy {
            state: 0,
            flags: 0,
            gradient: 0,
            animation_id: 0,
        };
        let mut data = CompositionData {
            attribute: 19,
            data: (&raw mut policy).cast(),
            size: std::mem::size_of::<AccentPolicy>(),
        };
        set(hwnd, &mut data);
    }
}

/// A window's pixels, even when covered (`PrintWindow` with full content).
pub fn capture_window(id: isize) -> anyhow::Result<RgbaFrame> {
    let hwnd = super::hwnd(id);
    let mut rect = Default::default();
    unsafe { GetWindowRect(hwnd, &mut rect) }?;
    let (width, height) = (
        (rect.right - rect.left).max(1),
        (rect.bottom - rect.top).max(1),
    );
    unsafe {
        let screen = GetDC(None);
        let dc = CreateCompatibleDC(Some(screen));
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        let old = SelectObject(dc, bitmap.into());
        // PW_RENDERFULLCONTENT captures DirectComposition content.
        let _ = PrintWindow(hwnd, dc, PRINT_WINDOW_FLAGS(2));
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bgra = vec![0_u8; (width * height * 4) as usize];
        GetDIBits(
            dc,
            bitmap,
            0,
            height as u32,
            Some(bgra.as_mut_ptr().cast()),
            &mut info,
            DIB_RGB_COLORS,
        );
        SelectObject(dc, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(dc);
        ReleaseDC(None, screen);
        for px in bgra.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }
        Ok(RgbaFrame::new(0, width as u32, height as u32, bgra))
    }
}

/// The overlay for one display. Dropping it closes the window.
pub struct InkOverlay {
    hwnd: isize,
    placement: Arc<Mutex<Placement>>,
}

impl InkOverlay {
    /// Covers the display holding the centre of `on` (global pixels), else the primary.
    pub fn open<T: 'static>(
        telemetry: Arc<SharedCaptureTelemetry>,
        on: Option<[f64; 4]>,
        cx: &mut Context<T>,
    ) -> Result<Self, String> {
        let monitor = on
            .and_then(|[x, y, w, h]| super::monitor_at(x + w / 2.0, y + h / 2.0))
            .or_else(super::primary_monitor)
            .ok_or("No display")?;
        let display_id = crate::ui::gpui_display_for(&monitor, cx).map(|d| d.id());
        let scale = monitor.scale as f32;
        let bounds = Bounds {
            origin: point(
                px(monitor.rect[0] as f32 / scale),
                px(monitor.rect[1] as f32 / scale),
            ),
            size: size(
                px(monitor.rect[2] as f32 / scale),
                px(monitor.rect[3] as f32 / scale),
            ),
        };
        let rect = monitor.rect;
        let placement = Arc::new(Mutex::new(Placement { rect, scale }));
        let view_placement = Arc::clone(&placement);
        let handle = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    display_id,
                    titlebar: None,
                    focus: false,
                    show: false,
                    kind: WindowKind::PopUp,
                    is_movable: false,
                    is_resizable: false,
                    is_minimizable: false,
                    window_background: WindowBackgroundAppearance::Transparent,
                    ..Default::default()
                },
                move |window, cx| {
                    // Layered before GPUI's first frame, or DirectComposition never shows it.
                    // Clicks fall through only a window that is both layered and transparent;
                    // alpha 255 keeps GPUI's per-pixel alpha as it is.
                    if let Some(hwnd) = crate::ui::hwnd_of(window) {
                        let style = unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32;
                        let extra = WS_EX_TRANSPARENT.0
                            | WS_EX_LAYERED.0
                            | WS_EX_TOOLWINDOW.0
                            | WS_EX_NOACTIVATE.0
                            | WS_EX_TOPMOST.0;
                        unsafe {
                            SetWindowLongPtrW(hwnd, GWL_EXSTYLE, (style | extra) as isize);
                            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
                        }
                        strip_chrome(hwnd);
                        exclude_from_capture(hwnd);
                    }
                    cx.new(|_| OverlayView::new(telemetry, view_placement))
                },
            )
            .map_err(|error| format!("{error:#}"))?;
        let hwnd = handle
            .update(cx, |_, window, _| crate::ui::hwnd_of(window))
            .ok()
            .flatten()
            .ok_or("The overlay has no window")?;
        // Resized on the next turn of the event loop: called from inside an update, GPUI's
        // resize callback can't reach the app and the window keeps its default layout size.
        let id = hwnd.0 as isize;
        cx.spawn(async move |_, _| place(id, rect)).detach();
        Ok(Self {
            hwnd: hwnd.0 as isize,
            placement,
        })
    }

    pub fn window_number(&self) -> u32 {
        self.hwnd as usize as u32
    }

    /// The display it covers, global pixels.
    pub fn screen(&self) -> [f64; 4] {
        self.placement
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .rect
    }

    /// Moves to the display holding the centre of `rect` (global pixels). Returns whether it
    /// moved: false when it's on that display already or there's none there.
    pub fn move_to(&mut self, rect: [f64; 4]) -> bool {
        let [x, y, w, h] = rect;
        let Some(monitor) = super::monitor_at(x + w / 2.0, y + h / 2.0) else {
            return false;
        };
        {
            let mut placement = self
                .placement
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if (0..4).all(|i| (placement.rect[i] - monitor.rect[i]).abs() < 1.0) {
                return false;
            }
            *placement = Placement {
                rect: monitor.rect,
                scale: monitor.scale as f32,
            };
        }
        // Off the UI thread: this runs inside a GPUI update, where the resize callback can't
        // reach the app (see `open`); the window's thread handles the move from its loop.
        let id = self.hwnd;
        std::thread::spawn(move || place(id, monitor.rect));
        true
    }
}

/// Puts one of Scrawl's GPUI windows (the overlay, picker or pill) over `rect` (global pixels)
/// on top, and shows it without activating it. Call it outside GPUI updates: the moves it makes
/// reach GPUI's resize and DPI handlers at once.
pub(crate) fn place(id: isize, rect: [f64; 4]) {
    let hwnd = super::hwnd(id);
    let [x, y, w, h] = rect.map(|v| v.round() as i32);
    let _ = unsafe { SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE) };
    // GPUI keeps a resize frame around the client area; grow the window by it so the client
    // area (what GPUI draws) covers `rect` exactly. Measured from where the window is now: a
    // move onto a monitor of another DPI has GPUI resize it to Windows' suggested rect.
    let (mut window, mut client, mut origin) = (RECT::default(), RECT::default(), POINT::default());
    let placed = unsafe {
        GetWindowRect(hwnd, &mut window).is_ok()
            && GetClientRect(hwnd, &mut client).is_ok()
            && ClientToScreen(hwnd, &mut origin).as_bool()
    };
    if placed {
        let [x, y, w, h] = grown_by_frame([x, y, w, h], window, origin, client);
        let _ = unsafe { SetWindowPos(hwnd, Some(HWND_TOPMOST), x, y, w, h, SWP_NOACTIVATE) };
    }
    let _ = unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
}

/// The window rect whose client area is `target`, given where the window is now (`window`), its
/// client area's screen origin and its client rect.
fn grown_by_frame(target: [i32; 4], window: RECT, origin: POINT, client: RECT) -> [i32; 4] {
    let [x, y, w, h] = target;
    let (left, top) = (origin.x - window.left, origin.y - window.top);
    let right = window.right - origin.x - client.right;
    let bottom = window.bottom - origin.y - client.bottom;
    [x - left, y - top, w + left + right, h + top + bottom]
}

impl Drop for InkOverlay {
    fn drop(&mut self) {
        let _ =
            unsafe { PostMessageW(Some(super::hwnd(self.hwnd)), WM_CLOSE, WPARAM(0), LPARAM(0)) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_window_is_grown_by_its_frame_as_it_is_now() {
        let rect = |left, top, right, bottom| RECT {
            left,
            top,
            right,
            bottom,
        };
        // Asked for 1920×1080 at the origin, then resized by a DPI change to 2880×1620, with a
        // 1 px frame left, right and bottom and 2 px on top.
        let grown = grown_by_frame(
            [0, 0, 1920, 1080],
            rect(0, 0, 2880, 1620),
            POINT { x: 1, y: 2 },
            rect(0, 0, 2878, 1617),
        );
        assert_eq!(grown, [-1, -2, 1922, 1083]);
        // No frame: the target itself.
        let none = grown_by_frame(
            [-1920, 0, 400, 100],
            rect(-1920, 0, -1520, 100),
            POINT { x: -1920, y: 0 },
            rect(0, 0, 400, 100),
        );
        assert_eq!(none, [-1920, 0, 400, 100]);
    }
}
