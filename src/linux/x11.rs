//! The X server: displays (RandR), the window list (EWMH), the pointer and keys (polled), and
//! the properties that make Scrawl's GPUI windows float (override-redirect, always on top,
//! click-through input shapes).
//!
//! On a Wayland session this is XWayland: it holds Scrawl's own windows and any X11 apps, but
//! not native Wayland windows, and it hears the pointer and keys only while an X11 window has
//! them. `crate::linux::wayland_session` says which.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as _;
use x11rb::protocol::shape::SK;
use x11rb::protocol::xfixes::ConnectionExt as _;
use x11rb::protocol::xproto::{
    AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent, ConfigureWindowAux,
    ConnectionExt as _, EventMask, InputFocus, MapState, PropMode, StackMode, Window,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

pub struct X {
    pub conn: RustConnection,
    pub root: Window,
    atoms: Mutex<HashMap<&'static str, u32>>,
    xfixes: bool,
    keymap: Mutex<Option<Keymap>>,
}

static X: OnceLock<Option<X>> = OnceLock::new();

/// The connection to the X server (XWayland on Wayland), opened once.
pub fn x() -> Option<&'static X> {
    X.get_or_init(|| {
        let (conn, screen) = RustConnection::connect(None).ok()?;
        let root = conn.setup().roots.get(screen)?.root;
        let xfixes = conn
            .xfixes_query_version(5, 0)
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some();
        Some(X {
            conn,
            root,
            atoms: Mutex::new(HashMap::new()),
            xfixes,
            keymap: Mutex::new(None),
        })
    })
    .as_ref()
}

impl X {
    pub fn atom(&self, name: &'static str) -> u32 {
        let mut atoms = self
            .atoms
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(&atom) = atoms.get(name) {
            return atom;
        }
        let atom = self
            .conn
            .intern_atom(false, name.as_bytes())
            .ok()
            .and_then(|c| c.reply().ok())
            .map_or(0, |r| r.atom);
        if atom != 0 {
            atoms.insert(name, atom);
        }
        atom
    }

    fn property32(&self, window: Window, name: &'static str) -> Vec<u32> {
        let atom = self.atom(name);
        self.conn
            .get_property(false, window, atom, AtomEnum::ANY, 0, 4096)
            .ok()
            .and_then(|c| c.reply().ok())
            .and_then(|r| r.value32().map(Iterator::collect))
            .unwrap_or_default()
    }

    fn property_bytes(&self, window: Window, atom: u32) -> Vec<u8> {
        self.conn
            .get_property(false, window, atom, AtomEnum::ANY, 0, 4096)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|r| r.value)
            .unwrap_or_default()
    }

    fn text(&self, window: Window) -> String {
        let utf8 = self.property_bytes(window, self.atom("_NET_WM_NAME"));
        let bytes = if utf8.is_empty() {
            self.property_bytes(window, AtomEnum::WM_NAME.into())
        } else {
            utf8
        };
        String::from_utf8_lossy(&bytes).trim().to_owned()
    }

    /// `WM_CLASS`: (instance, class), e.g. ("gnome-terminal-server", "Gnome-terminal").
    fn class(&self, window: Window) -> (String, String) {
        let bytes = self.property_bytes(window, AtomEnum::WM_CLASS.into());
        let mut parts = bytes
            .split(|&b| b == 0)
            .map(|p| String::from_utf8_lossy(p).into_owned());
        (
            parts.next().unwrap_or_default(),
            parts.next().unwrap_or_default(),
        )
    }

    fn flush(&self) {
        let _ = self.conn.flush();
    }
}

// MARK: Displays

/// A display: its RandR monitor's bounds and the work area on it (without panels), in global
/// pixels, and the scale Scrawl's windows draw at (X11 has one for the whole screen).
#[derive(Clone, Debug)]
pub struct Monitor {
    /// The RandR monitor's name atom; stable while it stays connected.
    pub handle: isize,
    pub rect: [f64; 4],
    pub work: [f64; 4],
    pub scale: f64,
    pub primary: bool,
}

fn intersect(a: [f64; 4], b: [f64; 4]) -> Option<[f64; 4]> {
    let x0 = a[0].max(b[0]);
    let y0 = a[1].max(b[1]);
    let x1 = (a[0] + a[2]).min(b[0] + b[2]);
    let y1 = (a[1] + a[3]).min(b[1] + b[3]);
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
}

/// The scale GPUI draws Scrawl's windows at: `GPUI_X11_SCALE_FACTOR`, else `Xft.dpi` / 96.
pub fn ui_scale() -> f64 {
    if let Some(scale) = std::env::var("GPUI_X11_SCALE_FACTOR")
        .ok()
        .and_then(|v| v.parse::<f64>().ok())
        .filter(|s| s.is_finite() && *s > 0.0)
    {
        return scale;
    }
    let Some(x) = x() else {
        return 1.0;
    };
    let resources = x.property_bytes(x.root, AtomEnum::RESOURCE_MANAGER.into());
    String::from_utf8_lossy(&resources)
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(':')?;
            (key.trim() == "Xft.dpi").then(|| value.trim().parse::<f64>().ok())?
        })
        .filter(|dpi| *dpi > 0.0)
        .map_or(1.0, |dpi| dpi / 96.0)
}

/// Every display, the primary first.
pub fn monitors() -> Vec<Monitor> {
    let Some(x) = x() else {
        return Vec::new();
    };
    let scale = ui_scale();
    let workarea = {
        let w = x.property32(x.root, "_NET_WORKAREA");
        (w.len() >= 4).then(|| [w[0], w[1], w[2], w[3]].map(f64::from))
    };
    let mut list: Vec<Monitor> = x
        .conn
        .randr_get_monitors(x.root, true)
        .ok()
        .and_then(|c| c.reply().ok())
        .map(|reply| reply.monitors)
        .unwrap_or_default()
        .into_iter()
        .map(|m| {
            let rect = [
                f64::from(m.x),
                f64::from(m.y),
                f64::from(m.width),
                f64::from(m.height),
            ];
            Monitor {
                handle: m.name as isize,
                rect,
                work: workarea.and_then(|w| intersect(rect, w)).unwrap_or(rect),
                scale,
                primary: m.primary,
            }
        })
        .collect();
    if list.is_empty() {
        let screen = &x.conn.setup().roots[0];
        let rect = [
            0.0,
            0.0,
            f64::from(screen.width_in_pixels),
            f64::from(screen.height_in_pixels),
        ];
        list.push(Monitor {
            handle: 0,
            rect,
            work: workarea.and_then(|w| intersect(rect, w)).unwrap_or(rect),
            scale,
            primary: true,
        });
    }
    if !list.iter().any(|m| m.primary) {
        list[0].primary = true;
    }
    list.sort_by_key(|m| !m.primary);
    list
}

pub fn primary_monitor() -> Option<Monitor> {
    monitors().into_iter().next()
}

/// The display holding a global point, else the nearest one.
pub fn monitor_at(px: f64, py: f64) -> Option<Monitor> {
    let list = monitors();
    let distance = |m: &Monitor| {
        let [x, y, w, h] = m.rect;
        let dx = (x - px).max(px - (x + w)).max(0.0);
        let dy = (y - py).max(py - (y + h)).max(0.0);
        dx.hypot(dy)
    };
    list.into_iter()
        .min_by(|a, b| distance(a).total_cmp(&distance(b)))
}

// MARK: The pointer and keys

/// The pointer, held modifiers and buttons, as the X server last saw them.
#[derive(Clone, Copy, Debug, Default)]
pub struct PointerState {
    pub x: f64,
    pub y: f64,
    pub shift: bool,
    pub ctrl: bool,
    pub alt: bool,
    pub super_key: bool,
    pub left: bool,
    pub right: bool,
}

pub fn pointer_state() -> Option<PointerState> {
    let x = x()?;
    let reply = x.conn.query_pointer(x.root).ok()?.reply().ok()?;
    let mask = u16::from(reply.mask);
    let has = |bit: x11rb::protocol::xproto::KeyButMask| mask & u16::from(bit) != 0;
    use x11rb::protocol::xproto::KeyButMask as M;
    Some(PointerState {
        x: f64::from(reply.root_x),
        y: f64::from(reply.root_y),
        shift: has(M::SHIFT),
        ctrl: has(M::CONTROL),
        alt: has(M::MOD1),
        super_key: has(M::MOD4),
        left: has(M::BUTTON1),
        right: has(M::BUTTON3),
    })
}

/// The pointer's global position in pixels.
pub fn cursor_position() -> Option<(f64, f64)> {
    pointer_state().map(|p| (p.x, p.y))
}

/// Keysyms by keycode, read once (and again when the layout changes, `refresh_keymap`).
struct Keymap {
    min: u8,
    per: usize,
    syms: Vec<u32>,
}

fn with_keymap<T>(f: impl FnOnce(&Keymap) -> T) -> Option<T> {
    let x = x()?;
    let mut slot = x
        .keymap
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if slot.is_none() {
        let setup = x.conn.setup();
        let (min, max) = (setup.min_keycode, setup.max_keycode);
        let reply = x
            .conn
            .get_keyboard_mapping(min, max - min + 1)
            .ok()?
            .reply()
            .ok()?;
        *slot = Some(Keymap {
            min,
            per: usize::from(reply.keysyms_per_keycode).max(1),
            syms: reply.keysyms,
        });
    }
    slot.as_ref().map(f)
}

/// Forgets the keyboard mapping, so the next lookup reads the current layout.
pub fn refresh_keymap() {
    if let Some(x) = x() {
        *x.keymap
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    }
}

/// The keycodes that type `keysym` (unshifted), on the current layout.
pub fn keycodes_for(keysym: u32) -> Vec<u8> {
    with_keymap(|map| {
        map.syms
            .chunks(map.per)
            .enumerate()
            .filter(|(_, syms)| syms.first() == Some(&keysym) || syms.get(1) == Some(&keysym))
            .map(|(i, _)| map.min.saturating_add(i as u8))
            .collect()
    })
    .unwrap_or_default()
}

/// The keysym a keycode types without modifiers.
pub fn keysym_at(keycode: u8) -> Option<u32> {
    with_keymap(|map| {
        let index = usize::from(keycode.checked_sub(map.min)?) * map.per;
        map.syms.get(index).copied().filter(|&sym| sym != 0)
    })
    .flatten()
}

/// The 256 keys' state (one bit each), as the X server last saw them.
pub fn keys_down() -> Option<[u8; 32]> {
    let x = x()?;
    let reply = x.conn.query_keymap().ok()?.reply().ok()?;
    Some(reply.keys)
}

pub fn key_is_down(keys: &[u8; 32], keycode: u8) -> bool {
    keys[usize::from(keycode / 8)] & (1 << (keycode % 8)) != 0
}

// MARK: Windows

/// A top-level window, front to back as `top_level_windows` lists them.
#[derive(Clone, Debug)]
pub struct WindowInfo {
    /// The X11 window id (named as on Windows, so `crate::windows` reads both).
    pub hwnd: isize,
    pub pid: u32,
    pub title: String,
    /// `WM_CLASS`'s class (`Gnome-terminal`, `firefox`).
    pub class: String,
    /// The client area in global pixels (without the window manager's frame, or GTK's shadow).
    pub rect: [f64; 4],
    pub kind: WindowKind,
    pub hidden: bool,
    pub above: bool,
    pub skip_taskbar: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WindowKind {
    Normal,
    Dialog,
    Dock,
    Desktop,
    Other,
}

impl WindowInfo {
    pub fn on_screen(&self) -> bool {
        !self.hidden && self.rect[2] > 0.0 && self.rect[3] > 0.0
    }

    /// An app window someone would pick: a normal window or dialog, not a panel, the desktop
    /// or a popup that keeps out of the taskbar.
    pub fn is_app_window(&self) -> bool {
        matches!(self.kind, WindowKind::Normal | WindowKind::Dialog) && !self.skip_taskbar
    }

    pub fn topmost(&self) -> bool {
        self.above
    }
}

fn describe(x: &X, window: Window) -> Option<WindowInfo> {
    let attrs = x.conn.get_window_attributes(window).ok()?.reply().ok()?;
    let geometry = x.conn.get_geometry(window).ok()?.reply().ok()?;
    let origin = x
        .conn
        .translate_coordinates(window, x.root, 0, 0)
        .ok()?
        .reply()
        .ok()?;
    let mut rect = [
        f64::from(origin.dst_x),
        f64::from(origin.dst_y),
        f64::from(geometry.width),
        f64::from(geometry.height),
    ];
    // GTK's client-side decorations draw their shadow inside the window.
    let gtk = x.property32(window, "_GTK_FRAME_EXTENTS");
    if gtk.len() == 4 {
        let [l, r, t, b] = [gtk[0], gtk[1], gtk[2], gtk[3]].map(f64::from);
        rect = [rect[0] + l, rect[1] + t, (rect[2] - l - r).max(0.0), (rect[3] - t - b).max(0.0)];
    }
    let types = x.property32(window, "_NET_WM_WINDOW_TYPE");
    let kind = match types.first() {
        None => WindowKind::Normal,
        Some(&t) if t == x.atom("_NET_WM_WINDOW_TYPE_NORMAL") => WindowKind::Normal,
        Some(&t) if t == x.atom("_NET_WM_WINDOW_TYPE_DIALOG") => WindowKind::Dialog,
        Some(&t) if t == x.atom("_NET_WM_WINDOW_TYPE_DOCK") => WindowKind::Dock,
        Some(&t) if t == x.atom("_NET_WM_WINDOW_TYPE_DESKTOP") => WindowKind::Desktop,
        Some(_) => WindowKind::Other,
    };
    let state = x.property32(window, "_NET_WM_STATE");
    let has = |name| state.contains(&x.atom(name));
    Some(WindowInfo {
        hwnd: window as isize,
        pid: x
            .property32(window, "_NET_WM_PID")
            .first()
            .copied()
            .unwrap_or(0),
        title: x.text(window),
        class: x.class(window).1,
        rect,
        kind,
        hidden: attrs.map_state != MapState::VIEWABLE || has("_NET_WM_STATE_HIDDEN"),
        above: has("_NET_WM_STATE_ABOVE"),
        skip_taskbar: has("_NET_WM_STATE_SKIP_TASKBAR"),
    })
}

/// Managed top-level windows, front to back (`_NET_CLIENT_LIST_STACKING`).
pub fn top_level_windows() -> Vec<WindowInfo> {
    let Some(x) = x() else {
        return Vec::new();
    };
    let mut ids = x.property32(x.root, "_NET_CLIENT_LIST_STACKING");
    if ids.is_empty() {
        ids = x.property32(x.root, "_NET_CLIENT_LIST");
    }
    ids.into_iter()
        .rev()
        .filter_map(|id| describe(x, id))
        .collect()
}

/// Scrawl's own floating windows (override-redirect, so not in the window manager's list),
/// front to back.
pub fn own_floating_windows() -> Vec<WindowInfo> {
    let Some(x) = x() else {
        return Vec::new();
    };
    let pid = std::process::id();
    let Some(tree) = x
        .conn
        .query_tree(x.root)
        .ok()
        .and_then(|c| c.reply().ok())
    else {
        return Vec::new();
    };
    tree.children
        .into_iter()
        .rev()
        .filter(|&id| {
            x.conn
                .get_window_attributes(id)
                .ok()
                .and_then(|c| c.reply().ok())
                .is_some_and(|a| a.override_redirect && a.map_state == MapState::VIEWABLE)
                && x.property32(id, "_NET_WM_PID").first() == Some(&pid)
        })
        .filter_map(|id| describe(x, id))
        .collect()
}

pub fn window_info(id: isize) -> Option<WindowInfo> {
    describe(x()?, id as Window)
}

/// The focused window (`_NET_ACTIVE_WINDOW`).
pub fn active_window() -> Option<WindowInfo> {
    let x = x()?;
    let id = *x.property32(x.root, "_NET_ACTIVE_WINDOW").first()?;
    (id != 0).then(|| describe(x, id)).flatten()
}

/// The class of a window `pid` owns, for its app's name.
pub fn class_of_pid(pid: u32) -> Option<String> {
    let x = x()?;
    x.property32(x.root, "_NET_CLIENT_LIST")
        .into_iter()
        .find(|&id| x.property32(id, "_NET_WM_PID").first() == Some(&pid))
        .map(|id| x.class(id).1)
        .filter(|class| !class.is_empty())
}

/// Asks the window manager to focus `id` (`_NET_ACTIVE_WINDOW`, as a pager would).
pub fn activate(id: isize) {
    let Some(x) = x() else {
        return;
    };
    let event = ClientMessageEvent::new(
        32,
        id as Window,
        x.atom("_NET_ACTIVE_WINDOW"),
        [2, x11rb::CURRENT_TIME, 0, 0, 0],
    );
    let _ = x.conn.send_event(
        false,
        x.root,
        EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY,
        event,
    );
    x.flush();
}

// MARK: Scrawl's own windows

/// Takes one of Scrawl's GPUI windows out of the window manager's hands (no frame, no
/// placement, above ordinary windows, not in the taskbar), places it over `rect` (global
/// pixels) and shows it. GPUI has mapped it already, so it is unmapped and mapped again.
pub fn float(id: u32, rect: [f64; 4]) {
    let Some(x) = x() else {
        return;
    };
    // GPUI's map is a request the window manager answers in its own time: let it finish, so it
    // doesn't map the window again after it's been taken over.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
    while std::time::Instant::now() < deadline
        && x.conn
            .get_window_attributes(id)
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some_and(|a| a.map_state != MapState::VIEWABLE)
    {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let _ = x.conn.unmap_window(id);
    // The window manager takes its frame off (reparenting the window back to the root) in its
    // own time; until it has, GNOME on Wayland would keep drawing the window at the frame's
    // offset.
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(250);
    while std::time::Instant::now() < deadline
        && x.conn
            .query_tree(id)
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some_and(|tree| tree.parent != x.root)
    {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let _ = x.conn.change_window_attributes(
        id,
        &ChangeWindowAttributesAux::new().override_redirect(1),
    );
    // A compositor must keep compositing it: it is see-through.
    let _ = x.conn.change_property32(
        PropMode::REPLACE,
        id,
        x.atom("_NET_WM_BYPASS_COMPOSITOR"),
        AtomEnum::CARDINAL,
        &[2],
    );
    let _ = x.conn.change_property32(
        PropMode::REPLACE,
        id,
        x.atom("_NET_WM_STATE"),
        AtomEnum::ATOM,
        &[
            x.atom("_NET_WM_STATE_ABOVE"),
            x.atom("_NET_WM_STATE_SKIP_TASKBAR"),
            x.atom("_NET_WM_STATE_SKIP_PAGER"),
        ],
    );
    configure(x, id, rect);
    let _ = x.conn.map_window(id);
    raise_on(x, id);
    x.flush();
}

fn configure(x: &X, id: u32, rect: [f64; 4]) {
    let [px, py, w, h] = rect.map(|v| v.round() as i32);
    let _ = x.conn.configure_window(
        id,
        &ConfigureWindowAux::new()
            .x(px)
            .y(py)
            .width(w.max(1) as u32)
            .height(h.max(1) as u32),
    );
}

/// Moves and resizes one of Scrawl's windows to `rect` (global pixels).
pub fn place(id: u32, rect: [f64; 4]) {
    if let Some(x) = x() {
        configure(x, id, rect);
        raise_on(x, id);
        x.flush();
    }
}

fn raise_on(x: &X, id: u32) {
    let _ = x
        .conn
        .configure_window(id, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE));
}

pub fn raise(id: u32) {
    if let Some(x) = x() {
        raise_on(x, id);
        x.flush();
    }
}

pub fn set_visible(id: u32, visible: bool) {
    if let Some(x) = x() {
        if visible {
            let _ = x.conn.map_window(id);
            raise_on(x, id);
        } else {
            let _ = x.conn.unmap_window(id);
        }
        x.flush();
    }
}

/// Asks a window to close (`WM_DELETE_WINDOW`), as its close button would: for GPUI windows
/// Scrawl holds no handle to.
pub fn request_close(id: u32) {
    let Some(x) = x() else {
        return;
    };
    let event = ClientMessageEvent::new(
        32,
        id,
        x.atom("WM_PROTOCOLS"),
        [x.atom("WM_DELETE_WINDOW"), x11rb::CURRENT_TIME, 0, 0, 0],
    );
    let _ = x.conn.send_event(false, id, EventMask::NO_EVENT, event);
    x.flush();
}

/// Gives one of Scrawl's override-redirect windows the keyboard (the picker's Esc and digits).
pub fn focus(id: u32) {
    if let Some(x) = x() {
        let _ = x
            .conn
            .set_input_focus(InputFocus::PARENT, id, x11rb::CURRENT_TIME);
        x.flush();
    }
}

/// Makes a window let every click through to what's below it (an empty input shape), or take
/// them again.
pub fn set_click_through(id: u32, through: bool) {
    let Some(x) = x() else {
        return;
    };
    if !x.xfixes {
        return;
    }
    let region = if through {
        let Ok(region) = x.conn.generate_id() else {
            return;
        };
        if x.conn.xfixes_create_region(region, &[]).is_err() {
            return;
        }
        region
    } else {
        x11rb::NONE
    };
    let _ = x.conn.xfixes_set_window_shape_region(id, SK::INPUT, 0, 0, region);
    if through {
        let _ = x.conn.xfixes_destroy_region(region);
    }
    x.flush();
}

/// Scrawl's own windows, including KWin's frame → wrapper → client nesting.
fn own_gpui_windows(x: &X) -> Vec<Window> {
    let pid = std::process::id();
    let children = |window: Window| -> Vec<Window> {
        x.conn
            .query_tree(window)
            .ok()
            .and_then(|c| c.reply().ok())
            .map(|tree| tree.children)
            .unwrap_or_default()
    };
    let mut found = Vec::new();
    let mut level = children(x.root);
    for _ in 0..3 {
        let mut next = Vec::new();
        for id in level {
            if x.property32(id, "_NET_WM_PID").first() == Some(&pid) {
                found.push(id);
            } else {
                next.extend(children(id));
            }
        }
        level = next;
    }
    found
}

/// GPUI windows' X11 ids, by GPUI window id.
static GPUI_WINDOWS: Mutex<Option<HashMap<u64, Window>>> = Mutex::new(None);

/// The X11 window id of a GPUI window. GPUI 0.2.2 doesn't hand out its X11 window's handle, so
/// the first time a window is asked about (right after it opens), it's the newest window of
/// this process not yet known. Called from the window's build callback, that is the window
/// just made.
pub fn window_id(window: &gpui::Window) -> Option<u32> {
    let key = window.window_handle().window_id().as_u64();
    let x = x()?;
    let mut slot = GPUI_WINDOWS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let known = slot.get_or_insert_with(HashMap::new);
    if let Some(&id) = known.get(&key) {
        return Some(id);
    }
    // Forget windows that have closed.
    known.retain(|_, id| {
        x.conn
            .get_window_attributes(*id)
            .ok()
            .and_then(|c| c.reply().ok())
            .is_some()
    });
    let fresh = own_gpui_windows(x)
        .into_iter()
        .filter(|id| !known.values().any(|v| v == id))
        .max()?;
    known.insert(key, fresh);
    Some(fresh)
}

/// Sets a window's `WM_CLASS`, so docks and the window switcher match it to `scrawl.desktop`.
pub fn set_class(id: u32) {
    if let Some(x) = x() {
        let _ = x.conn.change_property8(
            PropMode::REPLACE,
            id,
            AtomEnum::WM_CLASS,
            AtomEnum::STRING,
            b"scrawl\0Scrawl\0",
        );
        x.flush();
    }
}

/// Sets a window's icon (`_NET_WM_ICON`: width, height, then ARGB pixels, for each size).
pub fn set_icon(id: u32, data: &[u32]) {
    if let Some(x) = x() {
        let _ = x.conn.change_property32(
            PropMode::REPLACE,
            id,
            x.atom("_NET_WM_ICON"),
            AtomEnum::CARDINAL,
            data,
        );
        x.flush();
    }
}

/// A window's rect in global pixels.
pub fn geometry(id: u32) -> Option<[f64; 4]> {
    let x = x()?;
    let geometry = x.conn.get_geometry(id).ok()?.reply().ok()?;
    let origin = x
        .conn
        .translate_coordinates(id, x.root, 0, 0)
        .ok()?
        .reply()
        .ok()?;
    Some([
        f64::from(origin.dst_x),
        f64::from(origin.dst_y),
        f64::from(geometry.width),
        f64::from(geometry.height),
    ])
}

/// A window's pixels (for `scrawl preview --capture-screenshot`), from the root window.
pub fn capture_rect(rect: [f64; 4]) -> anyhow::Result<crate::model::RgbaFrame> {
    use anyhow::Context as _;
    let x = x().context("No X server")?;
    let [px, py, w, h] = rect.map(|v| v.round() as i32);
    let reply = x
        .conn
        .get_image(
            x11rb::protocol::xproto::ImageFormat::Z_PIXMAP,
            x.root,
            px as i16,
            py as i16,
            w.max(1) as u16,
            h.max(1) as u16,
            u32::MAX,
        )?
        .reply()?;
    let mut rgba = reply.data;
    for pixel in rgba.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    Ok(crate::model::RgbaFrame::new(0, w.max(1) as u32, h.max(1) as u32, rgba))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_areas_are_clipped_to_their_display() {
        assert_eq!(
            intersect([0.0, 0.0, 1920.0, 1080.0], [0.0, 32.0, 3840.0, 1048.0]),
            Some([0.0, 32.0, 1920.0, 1048.0])
        );
        assert_eq!(intersect([1920.0, 0.0, 1920.0, 1080.0], [0.0, 0.0, 1920.0, 1080.0]), None);
    }
}
