//! The focused app on Wayland, from the accessibility bus (AT-SPI): the compositor tells no app
//! which window has focus, but every GTK, Qt, Chromium and Firefox app reports its windows'
//! states to the accessibility bus, as screen readers rely on. Scrawl asks which top-level
//! window is active, and so which app (`GNOME Terminal`, `Firefox`) a paste goes to.

use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use zbus::blocking::{Connection, Proxy};
use zbus::zvariant::OwnedObjectPath;

/// The app whose window is active.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusedApp {
    pub pid: Option<u32>,
    /// What the app calls itself (`gnome-terminal-server`, `Firefox`), prettified.
    pub name: String,
    /// The executable's name, as on X11 (`gnome-terminal-server`, `firefox`).
    pub id: String,
    /// The active window's title.
    pub title: Option<String>,
}

const ROOT: &str = "/org/a11y/atspi/accessible/root";
const ACCESSIBLE: &str = "org.a11y.atspi.Accessible";
/// `ATSPI_STATE_ACTIVE`: the window that has focus.
const STATE_ACTIVE: u32 = 1 << 1;

fn bus() -> Option<&'static Connection> {
    static BUS: OnceLock<Option<Connection>> = OnceLock::new();
    BUS.get_or_init(|| {
        let session = Connection::session().ok()?;
        let address: String = Proxy::new(&session, "org.a11y.Bus", "/org/a11y/bus", "org.a11y.Bus")
            .ok()?
            .call("GetAddress", &())
            .ok()?;
        zbus::blocking::connection::Builder::address(address.as_str())
            .ok()?
            .build()
            .ok()
    })
    .as_ref()
}

fn accessible<'a>(bus: &'a Connection, name: &'a str, path: &'a str) -> Option<Proxy<'a>> {
    zbus::blocking::proxy::Builder::new(bus)
        .destination(name)
        .ok()?
        .path(path)
        .ok()?
        .interface(ACCESSIBLE)
        .ok()?
        .cache_properties(zbus::proxy::CacheProperties::No)
        .build()
        .ok()
}

fn children(proxy: &Proxy) -> Vec<(String, OwnedObjectPath)> {
    proxy.call("GetChildren", &()).unwrap_or_default()
}

fn find_focused() -> Option<FocusedApp> {
    let bus = bus()?;
    let registry = accessible(bus, "org.a11y.atspi.Registry", ROOT)?;
    let dbus = zbus::blocking::fdo::DBusProxy::new(bus).ok()?;
    let own = bus.unique_name().map(|n| n.to_string());
    for (name, path) in children(&registry) {
        if own.as_deref() == Some(name.as_str()) {
            continue;
        }
        let Some(app) = accessible(bus, &name, path.as_str()) else {
            continue;
        };
        for (window_bus, window_path) in children(&app) {
            let Some(window) = accessible(bus, &window_bus, window_path.as_str()) else {
                continue;
            };
            let state: Vec<u32> = window.call("GetState", &()).unwrap_or_default();
            if state.first().is_none_or(|bits| bits & STATE_ACTIVE == 0) {
                continue;
            }
            let pid = zbus::names::BusName::try_from(name.as_str())
                .ok()
                .and_then(|bus_name| dbus.get_connection_unix_process_id(bus_name).ok());
            if pid == Some(std::process::id()) {
                return None;
            }
            let app_name: String = app.get_property("Name").unwrap_or_default();
            let id = pid
                .and_then(super::process_path)
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| app_name.clone())
                .to_ascii_lowercase();
            if id == X11_FRAMES {
                return x11_front();
            }
            let identity = pid.and_then(super::app_identity);
            let title: String = window.get_property("Name").unwrap_or_default();
            return Some(FocusedApp {
                pid,
                name: identity.map_or(app_name, |(name, _)| name),
                id,
                title: Some(title).filter(|t| !t.is_empty()),
            });
        }
    }
    None
}

/// GNOME draws X11 apps' title bars in this process, whose frame windows report the app
/// window's title and focus.
const X11_FRAMES: &str = "mutter-x11-frames";

/// An X11 app is in front: XWayland knows which.
fn x11_front() -> Option<FocusedApp> {
    let front = super::foreground_window()?;
    if front.pid == std::process::id() {
        return None;
    }
    let (name, id) = super::app_identity(front.pid)?;
    Some(FocusedApp {
        pid: Some(front.pid),
        name,
        id,
        title: Some(front.title).filter(|t| !t.is_empty()),
    })
}

/// The app whose window has focus (not Scrawl), asked at most every 250 ms.
pub fn focused_app() -> Option<FocusedApp> {
    static CACHE: Mutex<Option<(Instant, Option<FocusedApp>)>> = Mutex::new(None);
    let mut cache = CACHE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((at, app)) = cache.as_ref()
        && at.elapsed() < Duration::from_millis(250)
    {
        return app.clone();
    }
    let app = find_focused();
    *cache = Some((Instant::now(), app.clone()));
    app
}

/// The last app other than Scrawl that had focus, for when Scrawl's own window has it.
pub fn last_external_app() -> Option<FocusedApp> {
    static LAST: Mutex<Option<FocusedApp>> = Mutex::new(None);
    let mut last = LAST.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(app) = focused_app() {
        *last = Some(app);
    }
    last.clone()
}
