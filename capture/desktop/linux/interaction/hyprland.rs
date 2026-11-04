//! Hyprland: Apparent's shortcuts as runtime binds that run `apparent shortcut record|paste`, which
//! hand the press to the running app (`super::ipc`), like GNOME's custom keybindings do. The
//! compositor consumes the chord, so it doesn't also reach the app in front, and nothing needs
//! the `input` group.
//!
//! Hyprland's GlobalShortcuts portal can't stand in: `xdg-desktop-portal` refuses an app without
//! an app id, and `xdg-desktop-portal-hyprland` only fires shortcuts the user bound by hand with
//! `bind = …, global, <app id>:<name>`.
//!
//! A config reload drops runtime binds, so `watch_reloads` tells Apparent to bind them again.
//! Hyprland 0.55+ can read its config as Lua (Omarchy 4 does), where binds are added with
//! `hyprctl eval`; the older hyprlang config takes `hyprctl keyword`. `Syntax` tells them apart.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use crate::capture::engine::runtime::{
    CARBON_CMD_KEY, CARBON_CONTROL_KEY, CARBON_OPTION_KEY, CARBON_SHIFT_KEY, ShortcutConfig,
};

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
/// The chords Apparent bound, by shortcut id (`record`, `paste`), to take them off again.
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

/// Exact window identity, not merely the application or its last focused window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FocusedWindow {
    pub address: String,
    pub pid: u32,
    pub class: String,
}

pub fn focused_window() -> Result<FocusedWindow> {
    let value: serde_json::Value = serde_json::from_str(&request("j/activewindow")?)?;
    let address = value["address"]
        .as_str()
        .filter(|s| !s.is_empty() && *s != "0x0")
        .context("No focused window")?
        .to_owned();
    let pid = value["pid"]
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .filter(|n| *n != 0)
        .context("Focused window has no PID")?;
    let class = value["class"]
        .as_str()
        .filter(|s| !s.is_empty())
        .context("Focused window has no class")?
        .to_owned();
    Ok(FocusedWindow {
        address,
        pid,
        class,
    })
}

fn target_address(window: &FocusedWindow) -> Result<&str> {
    anyhow::ensure!(
        window.address.starts_with("0x")
            && window.address.len() > 2
            && window.address[2..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit()),
        "Invalid Hyprland window address"
    );
    Ok(&window.address)
}

pub fn check_window(original: &FocusedWindow) -> Result<()> {
    let address = target_address(original)?;
    let clients: Vec<serde_json::Value> = serde_json::from_str(&request("j/clients")?)?;
    anyhow::ensure!(
        clients.iter().any(|client| {
            client["address"].as_str() == Some(address)
                && client["pid"].as_u64() == Some(u64::from(original.pid))
                && client["class"].as_str() == Some(original.class.as_str())
        }),
        "Paste target is gone or changed"
    );
    Ok(())
}

pub fn targeted_paste_supported(original: &FocusedWindow) -> Result<()> {
    anyhow::ensure!(
        syntax()? == Syntax::Lua,
        "Targeted paste needs Hyprland Lua dispatchers"
    );
    target_address(original)?;
    Ok(())
}

pub fn paste_into_focused(original: &FocusedWindow, terminal: bool) -> Result<()> {
    targeted_paste_supported(original)?;
    anyhow::ensure!(focused_window()? == *original, "Paste target lost focus");
    let mods = if terminal { "CTRL SHIFT" } else { "CTRL" };
    run(&format!(
        "dispatch hl.dsp.send_shortcut({{ mods = \"{mods}\", key = \"V\", window = \"address:{}\" }})",
        target_address(original)?
    ))
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
    let reply = request("keyword apparent_probe 1")?;
    let syntax = if reply.contains("non-legacy") {
        Syntax::Lua
    } else {
        Syntax::Hyprlang
    };
    *cached = Some(syntax);
    Ok(syntax)
}

/// Hyprland's `SUPER` mask for Apparent's Carbon modifier bits (⌥ is Alt, ⌘ is Super).
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

/// Apparent's key names (`Enter`, `Backspace`) as xkb names, which Hyprland takes.
fn key_name(key: &str) -> String {
    match key {
        "Enter" => "Return".into(),
        "Backspace" => "BackSpace".into(),
        // The dedicated Copilot key arrives as evdev code 201 on Omarchy. Keep its
        // display name in config while asking Hyprland for the physical key.
        "Copilot" => "code:201".into(),
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
    format!("Apparent: {id}")
}

/// Omarchy's Lua-backed binds hide their physical key in `hyprctl binds -j`. Read the
/// narrow, user-owned source definition and require its description in the live table;
/// neither an empty `key` nor a description alone identifies a working Copilot bind.
fn external_record_from(source: &str, live: &str) -> Option<(ShortcutConfig, PathBuf)> {
    let matches: Vec<_> = source
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            let command = line
                .strip_prefix("o.bind(\"SUPER + SHIFT + code:201\", \"Apparent: record\", \"")?
                .strip_suffix("\")")?;
            let parts: Vec<_> = command.split_whitespace().collect();
            let (prefix, path, action) = match parts.as_slice() {
                [prefix @ .., path, "shortcut", "record"] => (prefix, path, true),
                _ => (&[][..], &"", false),
            };
            let env = prefix.first().copied() == Some("env");
            let assignments = if env { &prefix[1..] } else { prefix };
            let safe_env = assignments.iter().all(|value| {
                ["LD_PRELOAD", "GST_PLUGIN_PATH", "GPUI_X11_SCALE_FACTOR"]
                    .iter()
                    .any(|name| value.starts_with(&format!("{name}=")))
                    && !value.contains([';', '&', '|', '$', '`', '(', ')', '<', '>', '\\'])
            });
            (action
                && (env || assignments.is_empty())
                && safe_env
                && path.starts_with('/')
                && path.ends_with("/apparent")
                && !path.contains([';', '&', '|', '$', '`', '(', ')', '<', '>', '\\']))
            .then(|| PathBuf::from(path))
        })
        .collect();
    if matches.len() != 1 {
        return None;
    }
    let binds: Vec<serde_json::Value> = serde_json::from_str(live).ok()?;
    let live_matches: Vec<_> = binds
        .iter()
        .filter(|bind| {
            bind["description"].as_str() == Some("Apparent: record")
                && bind["modmask"].as_u64() == Some(u64::from(MOD_SUPER | MOD_SHIFT))
                && bind["key"].as_str() == Some("")
                && bind["keycode"].as_u64() == Some(0)
        })
        .collect();
    (live_matches.len() == 1).then(|| {
        (
            ShortcutConfig {
                key_code: 201,
                carbon_modifiers: CARBON_CMD_KEY | CARBON_SHIFT_KEY,
                label: "Super+Shift+Copilot".into(),
            },
            matches[0].clone(),
        )
    })
}

/// A permanent Omarchy recording binding overrides Apparent's built-in default.
/// Re-evaluated at launch; edit/reload Hyprland and restart Apparent after changing it.
pub fn external_record() -> Option<ShortcutConfig> {
    static RECORD: std::sync::OnceLock<Option<ShortcutConfig>> = std::sync::OnceLock::new();
    RECORD.get_or_init(external_record_live).clone()
}

fn external_record_live() -> Option<ShortcutConfig> {
    if !running() {
        return None;
    }
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))?;
    let source = std::fs::read_to_string(config_home.join("hypr/bindings.lua")).ok()?;
    let live = request("j/binds").ok()?;
    let (record, command) = external_record_from(&source, &live)?;
    use std::os::unix::fs::PermissionsExt;
    let executable = std::fs::metadata(command).ok()?;
    if !executable.is_file() || executable.permissions().mode() & 0o111 == 0 {
        return None;
    }
    // An old Apparent process may have registered the built-in default in this same
    // compositor session. Remove only that described chord, never a different owner's S.
    if remove_obsolete_default_in(&live).is_err() {
        return None;
    }
    Some(record)
}

fn remove_obsolete_default_in(live: &str) -> Result<()> {
    if obsolete_default_in(live) {
        unbind(CARBON_OPTION_KEY | CARBON_SHIFT_KEY, "S")?;
    }
    Ok(())
}

fn obsolete_default_in(live: &str) -> bool {
    serde_json::from_str::<Vec<serde_json::Value>>(live)
        .ok()
        .is_some_and(|binds| {
            binds.iter().any(|bind| {
                bind["description"].as_str() == Some("Apparent: record")
                    && bind["modmask"].as_u64() == Some(u64::from(MOD_ALT | MOD_SHIFT))
                    && bind["key"]
                        .as_str()
                        .is_some_and(|key| key.eq_ignore_ascii_case("S"))
            })
        })
}

fn command(id: &str) -> Result<String> {
    let exe = super::super::apparent_exe()?;
    Ok(format!(
        "{} shortcut {id}",
        shell_quote(&exe.display().to_string())
    ))
}

/// What already sits on a chord, from `hyprctl binds -j`.
#[derive(Debug, PartialEq, Eq)]
enum Existing {
    Free,
    /// A bind with Apparent's description: ours from before, or one the user made permanent
    /// from the snippet in `capture/desktop/docs/linux/linux.md`.
    Apparent,
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
            .is_some_and(|k| k.eq_ignore_ascii_case(&key_name(key)))
            || (key == "Copilot"
                && bind["key"].as_str() == Some("")
                && bind["keycode"].as_u64() == Some(0));
        if !(same_mask && same_key) {
            continue;
        }
        // Lua-backed code binds are opaque here. A description cannot prove which code
        // was pressed; only external_record_from can establish the matching source.
        if key != "Copilot" && bind["description"].as_str() == Some(ours.as_str()) {
            return Existing::Apparent;
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

/// XWayland's override-redirect keeps the capsule above windows, but Hyprland still
/// assigns it to one workspace. Pin only the exact capsule, never the whole app.
pub fn pin_pill(xid: u32) -> Result<()> {
    pin_pill_for_process(xid, std::process::id(), || true)
}

/// The Electron presentation has its own parent-owned process, with the same
/// XID-specific title and exact compositor identity contract as the native pill.
pub fn pin_pill_for_process(xid: u32, pid: u32, still_current: impl FnOnce() -> bool) -> Result<()> {
    if let Some(command) = pill_pin_command(
        &request("j/clients")?,
        pid,
        xid,
        syntax()?,
    )? {
        // Discovery/syntax RPCs may have waited while the child was retired.
        // Recheck its caller-owned lifetime immediately before any mutation.
        if !still_current() {
            return Ok(());
        }
        run(&command)?;
    }
    Ok(())
}

pub fn pill_title(xid: u32) -> String {
    format!("Apparent Recording Controls {xid:x}")
}

pub fn note_title(xid: u32) -> String { format!("Apparent notes {xid:x}") }

pub fn note_is_active(xid: u32, pid: u32) -> bool {
    request("j/activewindow").ok().and_then(|reply| serde_json::from_str::<serde_json::Value>(&reply).ok())
        .is_some_and(|window| active_note_matches(&window, xid, pid))
}

fn active_note_matches(window: &serde_json::Value, xid: u32, pid: u32) -> bool {
    window["pid"].as_u64() == Some(u64::from(pid))
        && window["title"].as_str() == Some(&note_title(xid))
        && window["xwayland"].as_bool() == Some(true)
}

pub fn pin_note(xid: u32, pid: u32) -> Result<()> {
    if let Some(command) = owned_pin_command(&request("j/clients")?, pid, &note_title(xid), syntax()?)? {
        run(&command)?;
    }
    Ok(())
}

/// X11 geometry can diverge from Hyprland's scene for override-redirect Electron
/// windows. Reconcile the exact registered child surface in the compositor too.
pub fn place_note(xid: u32, pid: u32, rect: [i32; 4], focus: bool, still_current: impl Fn() -> bool) -> Result<()> {
    pin_note(xid, pid)?;
    let clients: Vec<serde_json::Value> = serde_json::from_str(&request("j/clients")?)?;
    let title = note_title(xid);
    let mut owned = clients.iter().filter(|w| w["pid"].as_u64() == Some(u64::from(pid)) && w["title"].as_str() == Some(&title) && w["xwayland"].as_bool() == Some(true));
    let window = owned.next().context("Notes surface not mapped")?;
    anyhow::ensure!(owned.next().is_none(), "Ambiguous notes surface");
    let address = window["address"].as_str().context("Notes surface has no address")?;
    anyhow::ensure!(address.starts_with("0x") && address.len() > 2 && address[2..].bytes().all(|b| b.is_ascii_hexdigit()), "Invalid notes address");
    let layout: Vec<Monitor> = serde_json::from_str(&request("j/monitors")?)?;
    let outputs: Vec<_> = super::x11::monitors().into_iter().map(|m| (m.name, m.rect)).collect();
    let [x,y,width,height] = note_layout_rect(rect, &layout, &outputs).context("Notes output unavailable")?;
    let syntax = syntax()?;
    place_then_focus(
        || match syntax {
            Syntax::Lua => run(&format!("eval local w = hl.get_windows({{ title = '{title}' }})[1]; assert(w and w.address == '{address}', 'Notes surface changed'); hl.dispatch(hl.dsp.window.resize({{ window = w, x = {width}, y = {height}, relative = false }})); hl.dispatch(hl.dsp.window.move({{ window = w, x = {x}, y = {y}, relative = false }}))")),
            Syntax::Hyprlang => {
                run(&format!("dispatch resizewindowpixel exact {width} {height},address:{address}"))?;
                run(&format!("dispatch movewindowpixel exact {x} {y},address:{address}"))
            }
        },
        || focus && still_current() && note_is_active(xid, pid),
        || match syntax {
            Syntax::Lua => run(&format!("eval local w = hl.get_windows({{ title = '{title}' }})[1]; assert(w and w.address == '{address}', 'Notes surface changed'); hl.dispatch(hl.dsp.focus({{ window = w }}))")),
            Syntax::Hyprlang => run(&format!("dispatch focuswindow address:{address}")),
        },
    )
}

// Placement RPCs can outlive the user's focus intent. Check it after the final
// geometry operation, before either compositor focus backend can take focus.
fn place_then_focus(
    place: impl FnOnce() -> Result<()>,
    owns_focus: impl FnOnce() -> bool,
    focus: impl FnOnce() -> Result<()>,
) -> Result<()> {
    place()?;
    if owns_focus() { focus()?; }
    Ok(())
}

fn pill_pin_command(clients: &str, pid: u32, xid: u32, syntax: Syntax) -> Result<Option<String>> {
    owned_pin_command(clients, pid, &pill_title(xid), syntax)
}

fn owned_pin_command(clients: &str, pid: u32, title: &str, syntax: Syntax) -> Result<Option<String>> {
    let clients: Vec<serde_json::Value> = serde_json::from_str(clients)?;
    let mut matching = clients.iter().filter(|client| {
        client["pid"].as_u64() == Some(u64::from(pid))
            && client["xwayland"].as_bool() == Some(true)
            && client["title"].as_str() == Some(title)
    });
    let pill = matching.next().context("Recording pill not mapped in Hyprland yet")?;
    anyhow::ensure!(matching.next().is_none(), "Recording pill identity is ambiguous");
    if pill["pinned"].as_bool() == Some(true) {
        return Ok(None);
    }
    let address = pill["address"].as_str().context("Recording pill has no address")?;
    anyhow::ensure!(
        address.starts_with("0x") && address.len() > 2
            && address[2..].bytes().all(|byte| byte.is_ascii_hexdigit()),
        "Invalid Hyprland pill address"
    );
    Ok(Some(match syntax {
        Syntax::Lua => format!(
            "eval hl.dispatch(hl.dsp.window.pin({{ action = 'set', window = 'address:{address}' }}))"
        ),
        // Legacy pin toggles; only send it after observing this exact window unpinned.
        Syntax::Hyprlang => format!("dispatch pin address:{address}"),
    }))
}

/// A normal GPUI window is tiled by Hyprland. Float only the whiteboard, not Apparent's
/// picker or capsule, so the browser can stay beside a movable, resizable paper window.
pub fn float_whiteboard() -> Result<()> {
    let clients: Vec<serde_json::Value> = serde_json::from_str(&request("j/clients")?)?;
    let Some(board) = clients.iter().find(|client| {
        client["title"].as_str() == Some("Apparent Whiteboard")
            && client["pid"].as_u64() == Some(u64::from(std::process::id()))
    }) else {
        bail!("Whiteboard window not mapped yet");
    };
    let floating = board["floating"].as_bool() == Some(true);
    let fullscreen = board["fullscreen"].as_u64().unwrap_or(0) != 0;
    if floating && !fullscreen {
        return Ok(());
    }
    let address = board["address"]
        .as_str()
        .context("Whiteboard window has no address")?;
    match syntax()? {
        Syntax::Lua => {
            let unset_fullscreen = if fullscreen {
                "hl.dispatch(hl.dsp.window.fullscreen_state({ internal = 0, client = 0, action = 'set', window = w })); "
            } else {
                ""
            };
            run(&format!(
                "eval local w = hl.get_windows({{ title = 'Apparent Whiteboard' }})[1]; \
                 assert(w and w.address == '{}', 'Whiteboard window changed'); \
                 {}hl.dispatch(hl.dsp.window.float({{ action = 'set', window = w }})); \
                 hl.dispatch(hl.dsp.window.resize({{ x = 560, y = 460, window = w }}))",
                address, unset_fullscreen
            ))?;
        }
        Syntax::Hyprlang => {
            if fullscreen {
                // The old dispatcher targets the focused window, not the address in its
                // arguments. Never risk changing another window's fullscreen state.
                bail!("Cannot safely unfullscreen the whiteboard on this Hyprland version");
            }
            run(&format!("dispatch setfloating address:{address}"))?;
            run(&format!(
                "dispatch resizewindowpixel exact 560 460,address:{address}"
            ))?;
        }
    }
    Ok(())
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
    if key == "Copilot"
        && external_record().is_some_and(|record| record.carbon_modifiers == modifiers)
        && id == "record"
    {
        // A config reload restores permanent binds, including the old default.
        // external_record() is cached, so recheck the source and live bind before
        // treating it as working or removing the obsolete chord after a reload.
        if external_record_live().is_none() {
            bail!(
                "Hyprland-managed Record shortcut is no longer active; restart Apparent after changing its binding"
            );
        }
        return Ok(());
    }
    match existing(&request("j/binds")?, mask, key, id) {
        Existing::Apparent => return Ok(()),
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

/// A point in Hyprland's layout as XWayland's coordinates, which Apparent's windows and captures
/// use. XWayland lays the monitors out by itself (`xrandr` can put a monitor at y 0 where
/// Hyprland has it at y 1080), so the point is taken to the same place on the same output,
/// found by name, and scaled by how big XWayland draws it. `x_monitors` is each output's name
/// and X11 rectangle.
fn note_layout_rect(rect: [i32; 4], layout: &[Monitor], outputs: &[(String, [f64; 4])]) -> Option<[i32; 4]> {
    let [x, y, w, h] = rect.map(f64::from);
    let (name, r) = outputs.iter().find(|(_, r)| x >= r[0] && x < r[0]+r[2] && y >= r[1] && y < r[1]+r[3])?;
    let m = layout.iter().find(|m| &m.name == name)?;
    let factor = m.layout_size().0 / r[2];
    Some([m.x + (x-r[0])*factor, m.y + (y-r[1])*factor, w*factor, h*factor].map(|v| v.round() as i32))
}

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

/// Place this process's uniquely titled overlay after XWayland has mapped it.
/// Raw X geometry is reinterpreted at map time; dispatch in compositor units.
pub fn place_overlay(xid: u32, rect: [f64; 4]) -> Result<()> {
    let mut last = None;
    // X mapping and Hyprland IPC use separate queues. Reconcile only this owned
    // window, within a fixed 100ms budget, after each remap.
    for _ in 0..10 {
        match place_overlay_once(xid, rect) {
            Ok(()) => return Ok(()),
            Err(error) => last = Some(error),
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    Err(last.unwrap())
}

fn place_overlay_once(xid: u32, rect: [f64; 4]) -> Result<()> {
    let clients: Vec<serde_json::Value> = serde_json::from_str(&request("j/clients")?)?;
    let title = format!("Apparent Ink {xid:x}");
    let mut matches = clients.iter().filter(|c| c["pid"].as_u64() == Some(std::process::id() as u64)
        && c["xwayland"].as_bool() == Some(true) && c["title"].as_str() == Some(title.as_str()));
    let client = matches.next().context("Ink overlay not mapped yet")?;
    anyhow::ensure!(matches.next().is_none(), "Ambiguous ink overlay");
    let address = client["address"].as_str().context("Ink overlay has no address")?;
    anyhow::ensure!(address.starts_with("0x") && address.len() > 2 && address[2..].bytes().all(|b| b.is_ascii_hexdigit()), "Invalid overlay address");
    let [x,y,w,h] = rect.map(|n| n.round() as i32);
    match syntax()? {
        Syntax::Lua => run(&format!("eval hl.dispatch(hl.dsp.window.resize({{ x = {w}, y = {h}, relative = false, window = 'address:{address}' }})); hl.dispatch(hl.dsp.window.move({{ x = {x}, y = {y}, relative = false, window = 'address:{address}' }})); hl.dispatch(hl.dsp.window.pin({{ action = 'set', window = 'address:{address}' }}))"))?,
        Syntax::Hyprlang => {
            run(&format!("dispatch resizewindowpixel exact {w} {h},address:{address}"))?;
            run(&format!("dispatch movewindowpixel exact {x} {y},address:{address}"))?;
            if client["pinned"].as_bool() != Some(true) { run(&format!("dispatch pin address:{address}"))?; }
        }
    }
    Ok(())
}

/// Override-redirect surfaces are placed in compositor layout units on Hyprland.
/// Keep this separate from the X output rectangle used by telemetry and capture.
pub fn overlay_rect(name: &str) -> Option<[f64; 4]> {
    let layout: Vec<Monitor> = serde_json::from_str(&request("j/monitors").ok()?).ok()?;
    let m = layout.iter().find(|m| m.name == name)?;
    let (w, h) = m.layout_size();
    Some([m.x, m.y, w, h])
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
        .name("apparent-hyprland-events".into())
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
    #[test]
    fn note_app_switch_during_placement_prevents_compositor_focus() {
        use std::cell::{Cell, RefCell};
        let active = RefCell::new(serde_json::json!({"pid":10,"title":"Apparent notes 14","xwayland":true}));
        let focus_calls = Cell::new(0);
        place_then_focus(
            || { *active.borrow_mut() = serde_json::json!({"pid":99,"title":"Other app","xwayland":false}); Ok(()) },
            || active_note_matches(&active.borrow(), 20, 10),
            || { focus_calls.set(focus_calls.get() + 1); Ok(()) },
        ).unwrap();
        assert_eq!(focus_calls.get(), 0, "placement must not override an intervening app switch");
        place_then_focus(|| Ok(()), || true,
            || { focus_calls.set(focus_calls.get() + 1); Ok(()) }).unwrap();
        assert_eq!(focus_calls.get(), 1, "a still-owned note retains compositor activation");
    }
    #[test]
    fn note_focus_requires_the_exact_active_xwayland_surface() {
        let note = serde_json::json!({"pid": 10, "title": "Apparent notes 14", "xwayland": true});
        assert!(super::active_note_matches(&note, 20, 10));
        assert!(!super::active_note_matches(&note, 21, 10));
        assert!(!super::active_note_matches(&note, 20, 11));
        assert!(!super::active_note_matches(&serde_json::json!({}), 20, 10));
        let mut native = note;
        native["xwayland"] = serde_json::json!(false);
        assert!(!super::active_note_matches(&native, 20, 10));
    }
    use super::*;

    const ALT_SHIFT: u32 = CARBON_OPTION_KEY | CARBON_SHIFT_KEY;

    #[test]
    fn pill_pin_targets_only_its_xid_and_process() {
        let clients = r#"[
            {"pid":42,"xwayland":true,"title":"Apparent Recording Controls 18000005","stableId":"18000007","address":"0x111","pinned":false},
            {"pid":43,"xwayland":true,"title":"Apparent Recording Controls 18000007","address":"0x222","pinned":false},
            {"pid":42,"xwayland":false,"title":"Apparent Recording Controls 18000007","address":"0x333","pinned":false},
            {"pid":42,"xwayland":true,"title":"Apparent Recording Controls 18000007","stableId":"7","address":"0x444","pinned":false}
        ]"#;
        assert_eq!(
            pill_pin_command(clients, 42, 0x18000007, Syntax::Lua).unwrap(),
            Some("eval hl.dispatch(hl.dsp.window.pin({ action = 'set', window = 'address:0x444' }))".into())
        );
        assert_eq!(
            pill_pin_command(clients, 42, 0x18000007, Syntax::Hyprlang).unwrap(),
            Some("dispatch pin address:0x444".into())
        );
        assert!(pill_pin_command(clients, 42, 0x18000009, Syntax::Lua).is_err());
        assert!(pill_pin_command("[]", 42, 0x18000007, Syntax::Lua).is_err());
    }

    #[test]
    fn pill_pin_is_idempotent_and_rejects_invalid_identity() {
        let clients = r#"[{"pid":42,"xwayland":true,"title":"Apparent Recording Controls 18000007","address":"0x444","pinned":true}]"#;
        for syntax in [Syntax::Lua, Syntax::Hyprlang] {
            assert_eq!(pill_pin_command(clients, 42, 0x18000007, syntax).unwrap(), None);
        }
        let unpinned = clients.replace("true}]", "false}]");
        assert!(pill_pin_command(&unpinned.replace("0x444", "0x444';oops"), 42, 0x18000007, Syntax::Lua).is_err());
        assert!(pill_pin_command(&unpinned.replace("18000007", "unknown"), 42, 0x18000007, Syntax::Lua).is_err());
        let duplicate = format!("[{},{}]", &unpinned[1..unpinned.len()-1], &unpinned[1..unpinned.len()-1]);
        assert!(pill_pin_command(&duplicate, 42, 0x18000007, Syntax::Lua).is_err());
    }

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
        assert_eq!(
            lua_chord(MOD_SUPER | MOD_SHIFT, "Copilot"),
            "SUPER + SHIFT + code:201"
        );
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
    fn notes_map_inverted_outputs_by_identity() {
        let layout: Vec<Monitor> = serde_json::from_value(serde_json::json!([
            {"name":"left","x":-1280,"y":0,"width":1280,"height":800,"scale":1.0},
            {"name":"right","x":0,"y":0,"width":1280,"height":800,"scale":1.25}
        ])).unwrap();
        let outputs = vec![("left".into(), [1280.0,0.0,1280.0,800.0]), ("right".into(), [0.0,0.0,1280.0,800.0])];
        assert_eq!(note_layout_rect([1500,100,332,220], &layout, &outputs), Some([-1060,100,332,220]));
        assert_eq!(note_layout_rect([250,100,400,200], &layout, &outputs), Some([200,80,320,160]));
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
            {"modmask": 9, "key": "TAB", "description": "Apparent: record"},
            {"modmask": 9, "key": "V", "description": ""}
        ]"#;
        assert_eq!(existing(binds, 72, "S", "record"), Existing::Other);
        assert_eq!(existing(binds, 9, "Tab", "record"), Existing::Apparent);
        assert_eq!(existing(binds, 9, "v", "paste"), Existing::Other);
        assert_eq!(existing(binds, 9, "S", "record"), Existing::Free);
        assert_eq!(existing("not json", 9, "S", "record"), Existing::Free);
        // Hyprland reports a code:201 Lua binding with an empty key and keycode 0.
        let copilot = r#"[{"modmask":65,"key":"","keycode":0,"description":"Apparent: record"}]"#;
        assert_eq!(existing(copilot, 65, "Copilot", "record"), Existing::Other);
        assert_eq!(existing(copilot, 65, "S", "record"), Existing::Free);
    }

    #[test]
    fn external_copilot_requires_source_and_live_binding() {
        let source = "o.bind(\"SUPER + SHIFT + code:201\", \"Apparent: record\", \"env LD_PRELOAD=/usr/lib/libcblas.so.3 GST_PLUGIN_PATH=/home/simon/.local/share/scrawl-setup/gstreamer-1.28.8 GPUI_X11_SCALE_FACTOR=1.6666667 /home/simon/.local/bin/apparent shortcut record\")";
        let live = r#"[{"modmask":65,"key":"","keycode":0,"description":"Apparent: record"}]"#;
        assert_eq!(external_record_from(source, live).unwrap().0.key_code, 201);
        assert!(external_record_from(source, "[]").is_none());
        assert!(external_record_from("", live).is_none());
        assert!(external_record_from(&source.replacen("env ", "false && ", 1), live).is_none());
        assert!(
            external_record_from(&source.replace("shortcut record", "shortcut paste"), live)
                .is_none()
        );
        assert!(
            external_record_from(source, &format!("[{0},{0}]", &live[1..live.len() - 1])).is_none()
        );
        assert!(!obsolete_default_in(live));
        assert!(obsolete_default_in(
            r#"[{"description":"Apparent: record","modmask":9,"key":"S"}]"#
        ));
        let both = format!(
            "[{},{{\"description\":\"Apparent: record\",\"modmask\":9,\"key\":\"S\"}}]",
            &live[1..live.len() - 1]
        );
        // Hyprland restores both permanent binds on reload. Reconciliation must
        // recognize the obsolete chord even while the Copilot bind is still present.
        assert!(obsolete_default_in(&both));
        assert!(!obsolete_default_in(
            r#"[{"description":"Other app","modmask":9,"key":"S"}]"#
        ));
    }
}

/// Return to the exact window behind a note, only while its process still owns it.
pub fn restore_note_focus(address: u64, pid: u32) -> Result<()> {
    let target = format!("0x{address:x}");
    let clients: Vec<serde_json::Value> = serde_json::from_str(&request("j/clients")?)?;
    anyhow::ensure!(clients.iter().any(|w| w["address"].as_str() == Some(&target) && w["pid"].as_u64() == Some(u64::from(pid))), "Note target no longer exists");
    match syntax()? {
        Syntax::Lua => run(&format!("eval hl.dispatch(hl.dsp.focus({{ window = \"address:{target}\" }}))")),
        Syntax::Hyprlang => run(&format!("dispatch focuswindow address:{target}")),
    }
}
