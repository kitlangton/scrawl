//! Hyprland's IPC socket. XWayland sees the pointer only over X11 windows, and Hyprland doesn't
//! hand it to the overlay while ⇧ is held (`overlay::take_pointer`), so on Hyprland the
//! compositor says where the pointer is, and a bind holds ⇧-clicks back from the apps.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use x11rb::protocol::xproto::ConnectionExt as _;

fn send(command: &str) -> Option<Vec<u8>> {
    let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    let runtime = std::env::var("XDG_RUNTIME_DIR").ok()?;
    let mut socket =
        UnixStream::connect(format!("{runtime}/hypr/{signature}/.socket.sock")).ok()?;
    socket.set_read_timeout(Some(Duration::from_millis(50))).ok()?;
    socket.write_all(command.as_bytes()).ok()?;
    let mut reply = Vec::new();
    socket.read_to_end(&mut reply).ok()?;
    Some(reply)
}

fn request(command: &str) -> Option<serde_json::Value> {
    serde_json::from_slice(&send(&format!("j/{command}"))?).ok()
}

/// Keeps plain-⇧ left clicks from the apps while `take` (they drop pins on the overlay), with
/// a bind that does nothing: Hyprland gives the pointer to no override-redirect window that
/// doesn't take focus, and one that does would take the keyboard from the app on hover.
pub fn take_shift_clicks(take: bool) {
    if !running() {
        return;
    }
    let code = if take {
        r#"hl.bind("SHIFT + mouse:272", function() end)"#
    } else {
        r#"hl.unbind("SHIFT + mouse:272")"#
    };
    let _ = send(&format!("eval {code}"));
}

/// A Hyprland monitor: its layout origin and size in logical pixels, and where it sits in X11
/// root pixels (XWayland's size for it: its pixel size with `force_zero_scaling`, else logical).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Output {
    logical: [f64; 4],
    scale: f64,
    x11_origin: [f64; 2],
}

/// Hyprland's monitors, matched to the X server's RandR monitors by name, read at most once a
/// second.
fn outputs() -> Vec<Output> {
    static CACHE: std::sync::Mutex<Option<(Instant, Vec<Output>)>> = std::sync::Mutex::new(None);
    let mut cache = CACHE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some((at, list)) = cache.as_ref()
        && at.elapsed() < Duration::from_secs(1)
    {
        return list.clone();
    }
    let x11_names: Vec<(String, [f64; 4])> = super::x11::x()
        .map(|x| {
            super::monitors()
                .into_iter()
                .filter_map(|m| {
                    let name = x.conn.get_atom_name(m.handle as u32).ok()?.reply().ok()?.name;
                    Some((String::from_utf8_lossy(&name).into_owned(), m.rect))
                })
                .collect()
        })
        .unwrap_or_default();
    let list = request("monitors")
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|m| {
            let f = |key: &str| m.get(key)?.as_f64();
            let scale = f("scale").filter(|s| *s > 0.0)?;
            let (x, y) = (f("x")?, f("y")?);
            let name = m.get("name")?.as_str()?;
            let logical = [x, y, f("width")? / scale, f("height")? / scale];
            let (x11_origin, scale) = x11_names
                .iter()
                .find(|(n, _)| n == name)
                .map_or(([x * scale, y * scale], scale), |(_, r)| {
                    ([r[0], r[1]], r[2] / logical[2])
                });
            Some(Output {
                logical,
                scale,
                x11_origin,
            })
        })
        .collect::<Vec<_>>();
    *cache = Some((Instant::now(), list.clone()));
    list
}

/// Maps a logical layout point to X11 root pixels on the monitor holding it.
fn to_x11(outputs: &[Output], (x, y): (f64, f64)) -> Option<(f64, f64)> {
    let o = outputs.iter().find(|o| {
        let [ox, oy, w, h] = o.logical;
        x >= ox && x < ox + w && y >= oy && y < oy + h
    })?;
    Some((
        o.x11_origin[0] + (x - o.logical[0]) * o.scale,
        o.x11_origin[1] + (y - o.logical[1]) * o.scale,
    ))
}

pub fn running() -> bool {
    std::env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some()
}

/// The pointer in X11 root pixels, or `None` when this isn't Hyprland.
pub fn cursor_position() -> Option<(f64, f64)> {
    if !running() {
        return None;
    }
    let pos = request("cursorpos")?;
    let point = (pos.get("x")?.as_f64()?, pos.get("y")?.as_f64()?);
    to_x11(&outputs(), point)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logical_points_map_to_x11_pixels_per_monitor() {
        let outputs = [
            Output {
                logical: [0.0, 0.0, 2400.0, 1350.0],
                scale: 1.6,
                x11_origin: [0.0, 0.0],
            },
            Output {
                logical: [2400.0, 0.0, 1920.0, 1080.0],
                scale: 1.0,
                x11_origin: [3840.0, 0.0],
            },
        ];
        assert_eq!(to_x11(&outputs, (1200.0, 675.0)), Some((1920.0, 1080.0)));
        assert_eq!(to_x11(&outputs, (2410.0, 5.0)), Some((3850.0, 5.0)));
        assert_eq!(to_x11(&outputs, (-5.0, 0.0)), None);
    }
}
