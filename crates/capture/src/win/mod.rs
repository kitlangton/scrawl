//! Windows platform helpers: displays, top-level windows, processes, Scrawl's own window styles,
//! the clipboard, the shell, and the media pipeline. Global coordinates are physical pixels on
//! the virtual screen (Scrawl is per-monitor DPI aware), with the primary display's top-left at
//! the origin, like CoreGraphics' points on macOS.

pub mod clipboard;
pub mod media;
pub mod privacy;
pub mod shell;
pub mod uia;

use std::ffi::c_void;
use std::path::PathBuf;

use windows::Win32::Foundation::{CloseHandle, HWND, LPARAM, MAX_PATH, POINT, RECT};
use windows::Win32::Graphics::Dwm::{
    DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute,
};
use windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITOR_DEFAULTTONEAREST,
    MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MONITORINFOEXW, MonitorFromPoint,
};
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GA_ROOTOWNER, GW_OWNER, GWL_EXSTYLE, GWL_STYLE, GetAncestor, GetClassNameW,
    GetCursorPos, GetForegroundWindow, GetWindow, GetWindowLongPtrW, GetWindowTextLengthW,
    GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindowVisible, WS_CHILD, WS_EX_APPWINDOW,
    WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT,
};
use windows::core::{BOOL, PWSTR};

/// A display: its handle, bounds and work area (without the taskbar) in global pixels, and its
/// scale (1.0 at 96 DPI).
#[derive(Clone, Debug)]
pub struct Monitor {
    pub handle: isize,
    pub rect: [f64; 4],
    pub work: [f64; 4],
    pub scale: f64,
    pub primary: bool,
}

fn rect_xywh(r: RECT) -> [f64; 4] {
    [
        f64::from(r.left),
        f64::from(r.top),
        f64::from(r.right - r.left),
        f64::from(r.bottom - r.top),
    ]
}

fn monitor_info(handle: HMONITOR) -> Option<Monitor> {
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    let ok = unsafe { GetMonitorInfoW(handle, (&raw mut info).cast::<MONITORINFO>()) };
    if !ok.as_bool() {
        return None;
    }
    let (mut dpi_x, mut dpi_y) = (96_u32, 96_u32);
    let _ = unsafe { GetDpiForMonitor(handle, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y) };
    Some(Monitor {
        handle: handle.0 as isize,
        rect: rect_xywh(info.monitorInfo.rcMonitor),
        work: rect_xywh(info.monitorInfo.rcWork),
        scale: f64::from(dpi_x.max(1)) / 96.0,
        // MONITORINFOF_PRIMARY
        primary: info.monitorInfo.dwFlags & 1 != 0,
    })
}

/// Every display, the primary first.
pub fn monitors() -> Vec<Monitor> {
    unsafe extern "system" fn collect(
        handle: HMONITOR,
        _: HDC,
        _: *mut RECT,
        data: LPARAM,
    ) -> BOOL {
        let list = unsafe { &mut *(data.0 as *mut Vec<HMONITOR>) };
        list.push(handle);
        BOOL(1)
    }
    let mut handles: Vec<HMONITOR> = Vec::new();
    let _ = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(collect),
            LPARAM((&raw mut handles) as isize),
        )
    };
    let mut list: Vec<Monitor> = handles.into_iter().filter_map(monitor_info).collect();
    list.sort_by_key(|m| !m.primary);
    list
}

/// The primary display.
pub fn primary_monitor() -> Option<Monitor> {
    monitor_info(unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) })
}

/// The display holding a global point, else the nearest one.
pub fn monitor_at(x: f64, y: f64) -> Option<Monitor> {
    let point = POINT {
        x: x.round() as i32,
        y: y.round() as i32,
    };
    monitor_info(unsafe { MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST) })
}

/// The cursor's global position in pixels.
pub fn cursor_position() -> Option<(f64, f64)> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }.ok()?;
    Some((f64::from(point.x), f64::from(point.y)))
}

pub fn hwnd(id: isize) -> HWND {
    HWND(id as *mut c_void)
}

/// A top-level window, front to back as `top_level_windows` lists them.
#[derive(Clone, Debug)]
pub struct WindowInfo {
    pub hwnd: isize,
    pub pid: u32,
    pub title: String,
    pub class: String,
    /// The visible frame (without the invisible resize borders), in global pixels.
    pub rect: [f64; 4],
    pub ex_style: u32,
    pub owned: bool,
    pub minimized: bool,
    pub cloaked: bool,
}

impl WindowInfo {
    /// Shown, on the current virtual desktop, and not minimized.
    pub fn on_screen(&self) -> bool {
        !self.cloaked && !self.minimized && self.rect[2] > 0.0 && self.rect[3] > 0.0
    }

    /// An app window someone would pick: not a tool window, a popup owned by another window,
    /// or a click-through overlay.
    pub fn is_app_window(&self) -> bool {
        let ex = self.ex_style;
        let tool = ex & WS_EX_TOOLWINDOW.0 != 0;
        let app = ex & WS_EX_APPWINDOW.0 != 0;
        let passthrough = ex & WS_EX_TRANSPARENT.0 != 0 || ex & WS_EX_NOACTIVATE.0 != 0;
        (app || (!tool && !self.owned))
            && !passthrough
            && !matches!(
                self.class.as_str(),
                "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd"
            )
    }

    pub fn topmost(&self) -> bool {
        self.ex_style & WS_EX_TOPMOST.0 != 0
    }
}

fn window_text(handle: HWND) -> String {
    let len = unsafe { GetWindowTextLengthW(handle) };
    if len <= 0 {
        return String::new();
    }
    let mut buffer = vec![0_u16; len as usize + 1];
    let read = unsafe { GetWindowTextW(handle, &mut buffer) };
    String::from_utf16_lossy(&buffer[..read.max(0) as usize])
}

fn class_name(handle: HWND) -> String {
    let mut buffer = [0_u16; 256];
    let read = unsafe { GetClassNameW(handle, &mut buffer) };
    String::from_utf16_lossy(&buffer[..read.max(0) as usize])
}

/// A window's visible frame: DWM's extended frame bounds, which leave out the invisible resize
/// borders that `GetWindowRect` includes.
pub fn window_frame(handle: HWND) -> Option<[f64; 4]> {
    let mut rect = RECT::default();
    unsafe {
        DwmGetWindowAttribute(
            handle,
            DWMWA_EXTENDED_FRAME_BOUNDS,
            (&raw mut rect).cast(),
            std::mem::size_of::<RECT>() as u32,
        )
    }
    .ok()?;
    Some(rect_xywh(rect))
}

fn describe_window(handle: HWND) -> Option<WindowInfo> {
    if !unsafe { IsWindowVisible(handle) }.as_bool() {
        return None;
    }
    let style = unsafe { GetWindowLongPtrW(handle, GWL_STYLE) } as u32;
    if style & WS_CHILD.0 != 0 {
        return None;
    }
    let mut pid = 0_u32;
    unsafe { GetWindowThreadProcessId(handle, Some(&mut pid)) };
    let mut cloaked = 0_u32;
    let _ = unsafe {
        DwmGetWindowAttribute(
            handle,
            DWMWA_CLOAKED,
            (&raw mut cloaked).cast(),
            std::mem::size_of::<u32>() as u32,
        )
    };
    let owner = unsafe { GetWindow(handle, GW_OWNER) }.unwrap_or_default();
    Some(WindowInfo {
        hwnd: handle.0 as isize,
        pid,
        title: window_text(handle),
        class: class_name(handle),
        rect: window_frame(handle).unwrap_or([0.0; 4]),
        ex_style: unsafe { GetWindowLongPtrW(handle, GWL_EXSTYLE) } as u32,
        owned: !owner.is_invalid(),
        minimized: unsafe { IsIconic(handle) }.as_bool(),
        cloaked: cloaked != 0,
    })
}

/// Visible top-level windows, front to back.
pub fn top_level_windows() -> Vec<WindowInfo> {
    unsafe extern "system" fn collect(handle: HWND, data: LPARAM) -> BOOL {
        let list = unsafe { &mut *(data.0 as *mut Vec<HWND>) };
        list.push(handle);
        BOOL(1)
    }
    let mut handles: Vec<HWND> = Vec::new();
    let _ = unsafe { EnumWindows(Some(collect), LPARAM((&raw mut handles) as isize)) };
    handles.into_iter().filter_map(describe_window).collect()
}

pub fn window_info(id: isize) -> Option<WindowInfo> {
    describe_window(hwnd(id))
}

/// The window in front (its root owner), if it is someone else's.
pub fn foreground_window() -> Option<WindowInfo> {
    let front = unsafe { GetForegroundWindow() };
    if front.is_invalid() {
        return None;
    }
    let root = unsafe { GetAncestor(front, GA_ROOTOWNER) };
    describe_window(if root.is_invalid() { front } else { root })
}

/// A process's executable path.
pub fn process_path(pid: u32) -> Option<PathBuf> {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
    let mut buffer = vec![0_u16; MAX_PATH as usize * 4];
    let mut len = buffer.len() as u32;
    let result = unsafe {
        QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        )
    };
    let _ = unsafe { CloseHandle(handle) };
    result.ok()?;
    Some(PathBuf::from(String::from_utf16_lossy(
        &buffer[..len as usize],
    )))
}

/// The name people know an app by: its executable's FileDescription ("Google Chrome",
/// "Visual Studio Code"), else the executable's name without `.exe`.
pub fn app_name(path: &std::path::Path) -> Option<String> {
    shell::file_description(path)
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            path.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
}

/// An app's name and identifier (its executable's file name, lowercased, e.g. `chrome.exe`,
/// standing in for a macOS bundle identifier).
pub fn app_identity(pid: u32) -> Option<(String, String)> {
    let path = process_path(pid)?;
    let id = path.file_name()?.to_string_lossy().to_ascii_lowercase();
    Some((app_name(&path).unwrap_or_else(|| id.clone()), id))
}

/// Encodes a string as a NUL-terminated UTF-16 buffer.
pub fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

/// `%LOCALAPPDATA%\Scrawl`: logs and machine-local state.
pub fn local_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Scrawl")
}
