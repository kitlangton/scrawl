//! Hyprland: Scrawl's shortcuts as runtime binds that run `scrawl shortcut record|paste`, which
//! hand the press to the running app (`super::ipc`), like GNOME's custom keybindings do. The
//! compositor consumes the chord, so it doesn't also reach the app in front, and nothing needs
//! the `input` group.
//!
//! Hyprland's GlobalShortcuts portal can't stand in: `xdg-desktop-portal` refuses an app without
//! an app id, and `xdg-desktop-portal-hyprland` only fires shortcuts the user bound by hand with
//! `bind = …, global, <app id>:<name>`.
//!
//! A config reload drops runtime binds, so `watch_reloads` tells Scrawl to bind them again.
//! Hyprland 0.55+ can read its config as Lua (Omarchy 4 does), where binds are added with
//! `hyprctl eval`; the older hyprlang config takes `hyprctl keyword`. `Syntax` tells them apart.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::capture::{CARBON_CMD_KEY, CARBON_CONTROL_KEY, CARBON_OPTION_KEY, CARBON_SHIFT_KEY};

/// Hyprland's modifier bits, as `hyprctl binds` reports them in `modmask`.
const MOD_SHIFT: u32 = 1;
const MOD_CTRL: u32 = 4;
const MOD_ALT: u32 = 8;
const MOD_SUPER: u32 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Syntax {
    Lua,
    Hyprlang,
}

static SYNTAX: Mutex<Option<Syntax>> = Mutex::new(None);
/// The chords Scrawl bound, by shortcut id (`record`, `paste`), to take them off again.
static BOUND: Mutex<Option<HashMap<&'static str, (u32, String)>>> = Mutex::new(None);

/// This session is Hyprland, and its socket answers where `HYPRLAND_INSTANCE_SIGNATURE` says.
pub fn running() -> bool {
    static RUNNING: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *RUNNING.get_or_init(|| instance_dir().is_some_and(|dir| dir.join(".socket.sock").exists()))
}

fn instance_dir() -> Option<PathBuf> {
    let signature = std::env::var("HYPRLAND_INSTANCE_SIGNATURE")
        .ok()
        .filter(|s| !s.is_empty())?;
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").filter(|s| !s.is_empty())?;
    Some(PathBuf::from(runtime).join("hypr").join(signature))
}

/// One request on Hyprland's command socket, as `hyprctl` sends it (`eval …`, `keyword …`,
/// `j/binds`).
fn request(command: &str) -> Result<String> {
    let dir = instance_dir().context("Not a Hyprland session")?;
    let mut stream = UnixStream::connect(dir.join(".socket.sock")).context("Hyprland socket")?;
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    stream.write_all(command.as_bytes())?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    Ok(reply)
}

fn syntax() -> Result<Syntax> {
    let mut cached = SYNTAX
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(syntax) = *cached {
        return Ok(syntax);
    }
    // `keyword` answers "can't work with non-legacy parsers" under a Lua config, and
    // complains about the unknown option under a hyprlang one, so the probe changes nothing.
    let reply = request("keyword scrawl_probe 1")?;
    let syntax = if reply.contains("non-legacy") {
        Syntax::Lua
    } else {
        Syntax::Hyprlang
    };
    *cached = Some(syntax);
    Ok(syntax)
}

/// Hyprland's `SUPER` mask for Scrawl's Carbon modifier bits (⌥ is Alt, ⌘ is Super).
fn modmask(carbon_modifiers: u32) -> u32 {
    [
        (CARBON_SHIFT_KEY, MOD_SHIFT),
        (CARBON_CONTROL_KEY, MOD_CTRL),
        (CARBON_OPTION_KEY, MOD_ALT),
        (CARBON_CMD_KEY, MOD_SUPER),
    ]
    .into_iter()
    .filter(|(carbon, _)| carbon_modifiers & carbon != 0)
    .fold(0, |mask, (_, bit)| mask | bit)
}

fn mod_names(mask: u32) -> Vec<&'static str> {
    [
        (MOD_SUPER, "SUPER"),
        (MOD_CTRL, "CTRL"),
        (MOD_ALT, "ALT"),
        (MOD_SHIFT, "SHIFT"),
    ]
    .into_iter()
    .filter(|(bit, _)| mask & bit != 0)
    .map(|(_, name)| name)
    .collect()
}

/// Scrawl's key names (`Enter`, `Backspace`) as xkb names, which Hyprland takes.
fn key_name(key: &str) -> String {
    match key {
        "Enter" => "Return".into(),
        "Backspace" => "BackSpace".into(),
        other => other.to_owned(),
    }
}

/// The Lua config's chord syntax: `ALT + SHIFT + S`.
fn lua_chord(mask: u32, key: &str) -> String {
    let mut parts: Vec<String> = mod_names(mask).into_iter().map(str::to_owned).collect();
    parts.push(key_name(key));
    parts.join(" + ")
}

fn lua_str(text: &str) -> String {
    format!(
        "\"{}\"",
        text.replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
    )
}

/// A path or word safe to put in a shell command.
fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn description(id: &str) -> String {
    format!("Scrawl: {id}")
}

fn command(id: &str) -> Result<String> {
    let exe = super::scrawl_exe()?;
    Ok(format!(
        "{} shortcut {id}",
        shell_quote(&exe.display().to_string())
    ))
}

/// What already sits on a chord, from `hyprctl binds -j`.
#[derive(Debug, PartialEq, Eq)]
enum Existing {
    Free,
    /// A bind with Scrawl's description: ours from before, or one the user made permanent
    /// from the snippet in `docs/linux.md`.
    Scrawl,
    Other,
}

fn existing(binds_json: &str, mask: u32, key: &str, id: &str) -> Existing {
    let Ok(binds) = serde_json::from_str::<Vec<serde_json::Value>>(binds_json) else {
        return Existing::Free;
    };
    let ours = description(id);
    let mut found = Existing::Free;
    for bind in &binds {
        let same_mask = bind["modmask"].as_u64() == Some(u64::from(mask));
        let same_key = bind["key"]
            .as_str()
            .is_some_and(|k| k.eq_ignore_ascii_case(&key_name(key)));
        if !(same_mask && same_key) {
            continue;
        }
        if bind["description"].as_str() == Some(ours.as_str()) {
            return Existing::Scrawl;
        }
        found = Existing::Other;
    }
    found
}

fn run(command: &str) -> Result<()> {
    let reply = request(command)?;
    if reply.trim() == "ok" {
        Ok(())
    } else {
        bail!("Hyprland: {}", reply.trim())
    }
}

fn unbind(carbon_modifiers: u32, key: &str) -> Result<()> {
    let mask = modmask(carbon_modifiers);
    match syntax()? {
        Syntax::Lua => run(&format!(
            "eval hl.unbind({})",
            lua_str(&lua_chord(mask, key))
        )),
        Syntax::Hyprlang => run(&format!(
            "keyword unbind {}, {}",
            mod_names(mask).join(" "),
            key_name(key)
        )),
    }
}

/// Binds (or, with `None`, unbinds) one shortcut's chord. A chord something else holds is left
/// alone and reported as an error, like the other backends' "already taken".
pub fn bind(id: &'static str, chord: Option<(u32, &str)>) -> Result<()> {
    let mut bound = BOUND
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let bound = bound.get_or_insert_with(HashMap::new);
    if let Some((modifiers, key)) = bound.remove(id) {
        let _ = unbind(modifiers, &key);
    }
    let Some((modifiers, key)) = chord else {
        return Ok(());
    };
    let mask = modmask(modifiers);
    match existing(&request("j/binds")?, mask, key, id) {
        Existing::Scrawl => return Ok(()),
        Existing::Other => bail!("{} is already bound", lua_chord(mask, key)),
        Existing::Free => {}
    }
    let (command, description) = (command(id)?, description(id));
    match syntax()? {
        Syntax::Lua => run(&format!(
            "eval hl.bind({}, hl.dsp.exec_cmd({}), {{ description = {} }})",
            lua_str(&lua_chord(mask, key)),
            lua_str(&command),
            lua_str(&description),
        ))?,
        Syntax::Hyprlang => run(&format!(
            "keyword bindd {}, {}, {description}, exec, {command}",
            mod_names(mask).join(" "),
            key_name(key),
        ))?,
    }
    bound.insert(id, (modifiers, key.to_owned()));
    Ok(())
}

/// A monitor as `hyprctl monitors` reports it: where it sits in Hyprland's layout, in layout
/// pixels (the size is in physical pixels, so it takes the scale to cover its layout area).
#[derive(Clone, Debug, serde::Deserialize)]
struct Monitor {
    name: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    scale: f64,
    /// Odd values turn the monitor on its side.
    #[serde(default)]
    transform: i64,
}

impl Monitor {
    /// The layout area the monitor covers.
    fn layout_size(&self) -> (f64, f64) {
        let scale = self.scale.max(f64::EPSILON);
        let (width, height) = if self.transform % 2 != 0 {
            (self.height, self.width)
        } else {
            (self.width, self.height)
        };
        (width / scale, height / scale)
    }
}

#[derive(serde::Deserialize)]
struct CursorPos {
    x: f64,
    y: f64,
}

/// A point in Hyprland's layout as XWayland's coordinates, which Scrawl's windows and captures
/// use. XWayland lays the monitors out by itself (`xrandr` can put a monitor at y 0 where
/// Hyprland has it at y 1080), so the point is taken to the same place on the same output,
/// found by name, and scaled by how big XWayland draws it. `x_monitors` is each output's name
/// and X11 rectangle.
fn to_x_space(
    point: [f64; 2],
    layout: &[Monitor],
    x_monitors: &[(String, [f64; 4])],
) -> Option<(f64, f64)> {
    let monitor = layout.iter().find(|m| {
        let (width, height) = m.layout_size();
        point[0] >= m.x && point[0] < m.x + width && point[1] >= m.y && point[1] < m.y + height
    })?;
    let (_, rect) = x_monitors.iter().find(|(name, _)| *name == monitor.name)?;
    let factor = rect[2] / monitor.layout_size().0;
    Some((
        rect[0] + (point[0] - monitor.x) * factor,
        rect[1] + (point[1] - monitor.y) * factor,
    ))
}

/// Where the pointer is, in XWayland's coordinates. XWayland itself sees the pointer only over
/// X11 windows and keeps the last place it saw it over the rest.
pub fn cursor_in_x_space(x_monitors: &[(String, [f64; 4])]) -> Option<(f64, f64)> {
    let cursor: CursorPos = serde_json::from_str(&request("j/cursorpos").ok()?).ok()?;
    let layout: Vec<Monitor> = serde_json::from_str(&request("j/monitors").ok()?).ok()?;
    to_x_space([cursor.x, cursor.y], &layout, x_monitors)
}

/// Calls `reloaded` each time Hyprland reloads its config, which drops runtime binds. Returns
/// once Hyprland's event socket closes.
pub fn watch_reloads(reloaded: impl Fn() + Send + 'static) -> Result<()> {
    let dir = instance_dir().context("Not a Hyprland session")?;
    let stream = UnixStream::connect(dir.join(".socket2.sock")).context("Hyprland events")?;
    std::thread::Builder::new()
        .name("scrawl-hyprland-events".into())
        .spawn(move || {
            for line in BufReader::new(stream).lines().map_while(Result::ok) {
                if line.starts_with("configreloaded>>") {
                    reloaded();
                }
            }
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALT_SHIFT: u32 = CARBON_OPTION_KEY | CARBON_SHIFT_KEY;

    #[test]
    fn carbon_modifiers_become_hyprland_masks() {
        assert_eq!(modmask(ALT_SHIFT), MOD_ALT | MOD_SHIFT);
        assert_eq!(
            modmask(CARBON_CMD_KEY | CARBON_CONTROL_KEY),
            MOD_SUPER | MOD_CTRL
        );
        assert_eq!(modmask(0), 0);
    }

    #[test]
    fn chords_read_like_a_lua_config() {
        assert_eq!(lua_chord(modmask(ALT_SHIFT), "S"), "ALT + SHIFT + S");
        assert_eq!(
            lua_chord(modmask(CARBON_CMD_KEY | CARBON_OPTION_KEY), "Enter"),
            "SUPER + ALT + Return"
        );
        assert_eq!(lua_chord(0, "F5"), "F5");
    }

    #[test]
    fn strings_survive_lua_and_the_shell() {
        assert_eq!(lua_str(r#"a"b\c"#), r#""a\"b\\c""#);
        assert_eq!(shell_quote("/a b/it's"), r#"'/a b/it'\''s'"#);
    }

    fn monitor(name: &str, [x, y, width, height]: [f64; 4], scale: f64, transform: i64) -> Monitor {
        Monitor {
            name: name.into(),
            x,
            y,
            width,
            height,
            scale,
            transform,
        }
    }

    #[test]
    fn layout_points_land_on_the_same_output_in_x_space() {
        let layout = vec![
            monitor("DP-3", [0.0, 0.0, 3840.0, 2160.0], 1.0, 0),
            monitor("HDMI-A-1", [3840.0, 1080.0, 1920.0, 1080.0], 1.0, 0),
        ];
        // XWayland puts the second monitor at y 0 where Hyprland has it at y 1080.
        let x_monitors = vec![
            ("DP-3".to_owned(), [0.0, 0.0, 3840.0, 2160.0]),
            ("HDMI-A-1".to_owned(), [3840.0, 0.0, 1920.0, 1080.0]),
        ];
        assert_eq!(
            to_x_space([4600.0, 1600.0], &layout, &x_monitors),
            Some((4600.0, 520.0))
        );
        assert_eq!(
            to_x_space([1000.0, 500.0], &layout, &x_monitors),
            Some((1000.0, 500.0))
        );
        // Above the second monitor, beside the first: nothing is there.
        assert_eq!(to_x_space([5000.0, 500.0], &layout, &x_monitors), None);
        assert_eq!(to_x_space([100.0, 100.0], &layout, &[]), None);
    }

    #[test]
    fn stacked_monitors_are_told_apart() {
        let layout = vec![
            monitor("DP-1", [0.0, 0.0, 1920.0, 1080.0], 1.0, 0),
            monitor("DP-2", [0.0, 1080.0, 1920.0, 1080.0], 1.0, 0),
        ];
        let x_monitors = vec![
            ("DP-1".to_owned(), [0.0, 0.0, 1920.0, 1080.0]),
            ("DP-2".to_owned(), [0.0, 1080.0, 1920.0, 1080.0]),
        ];
        assert_eq!(
            to_x_space([10.0, 1100.0], &layout, &x_monitors),
            Some((10.0, 1100.0))
        );
    }

    #[test]
    fn a_scaled_monitor_is_scaled_into_x_space() {
        let layout = vec![monitor("eDP-1", [0.0, 0.0, 3840.0, 2400.0], 2.0, 0)];
        // XWayland draws it at its physical size.
        let x_monitors = vec![("eDP-1".to_owned(), [0.0, 0.0, 3840.0, 2400.0])];
        assert_eq!(
            to_x_space([500.0, 300.0], &layout, &x_monitors),
            Some((1000.0, 600.0))
        );
    }

    #[test]
    fn a_rotated_monitor_covers_its_turned_area() {
        // 1920×1080 on its side covers 1080 wide and 1920 tall in the layout.
        let layout = vec![monitor("DP-1", [0.0, 0.0, 1920.0, 1080.0], 1.0, 1)];
        let x_monitors = vec![("DP-1".to_owned(), [0.0, 0.0, 1080.0, 1920.0])];
        assert_eq!(
            to_x_space([500.0, 1800.0], &layout, &x_monitors),
            Some((500.0, 1800.0))
        );
    }

    #[test]
    fn existing_binds_are_told_apart() {
        let binds = r#"[
            {"modmask": 72, "key": "S", "description": "Move window to scratchpad"},
            {"modmask": 9, "key": "TAB", "description": "Scrawl: record"},
            {"modmask": 9, "key": "V", "description": ""}
        ]"#;
        assert_eq!(existing(binds, 72, "S", "record"), Existing::Other);
        assert_eq!(existing(binds, 9, "Tab", "record"), Existing::Scrawl);
        assert_eq!(existing(binds, 9, "v", "paste"), Existing::Other);
        assert_eq!(existing(binds, 9, "S", "record"), Existing::Free);
        assert_eq!(existing("not json", 9, "S", "record"), Existing::Free);
    }
}
