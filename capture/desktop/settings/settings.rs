// The Settings window. It is SwiftUI in `apparent-helper settings` and, like onboarding, only
// draws what this module sends it (`View`) and reports what was clicked or typed (`Action`).
// Everything is decided and written here: `~/.config/apparent/config.json` keeps its other keys
// and its `0600` mode.
//
// Panes: General (launch at login, the two shortcuts), Recording
// (transcriber, video export), Storage (how long recordings are kept), Permissions (the
// onboarding checklist, to fix things later), Agents (`apparent` on PATH, the skill's install
// command) and About.
//
// Shortcuts live under `"shortcuts"` (`{"record": {"keyCode", "modifiers", "label"}}`, Carbon
// modifier bits; absent means the default, and `"paste": null` turns paste off). While the window records one, Apparent's hotkeys are
// paused (`crate::capture::engine::runtime::pause_shortcuts`) so the current combo reaches the recorder.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::capture::engine::runtime::{
    CARBON_CMD_KEY, CARBON_CONTROL_KEY, CARBON_OPTION_KEY, CARBON_SHIFT_KEY, ShortcutConfig,
    ShortcutKind, modifier_symbols,
};
use crate::capture::desktop::settings::onboarding::{Grants, Permission, Status};

// MARK: Shortcuts in the config

/// Apparent's two shortcuts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shortcuts {
    pub record: ShortcutConfig,
    pub paste: ShortcutConfig,
}

impl Default for Shortcuts {
    fn default() -> Self {
        Self {
            record: ShortcutConfig::default_record(),
            paste: ShortcutConfig::default_paste(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    key_code: u32,
    modifiers: u32,
    label: String,
}

impl Shortcuts {
    pub fn get(&self, kind: ShortcutKind) -> &ShortcutConfig {
        match kind {
            ShortcutKind::Record => &self.record,
            ShortcutKind::Paste => &self.paste,
        }
    }

    fn set(&mut self, kind: ShortcutKind, cfg: ShortcutConfig) {
        match kind {
            ShortcutKind::Record => self.record = cfg,
            ShortcutKind::Paste => self.paste = cfg,
        }
    }

    /// From `~/.config/apparent/config.json`, moving a shortcut saved by 0.3.x (its own file
    /// under Application Support) across the first time.
    pub fn load() -> Self {
        let path = crate::shared::handoff::default_config_path();
        let legacy = legacy_path();
        let (shortcuts, migrated) = Self::load_from(&path, &legacy);
        if migrated && save_shortcut_in(&path, ShortcutKind::Record, &shortcuts.record).is_ok() {
            let _ = std::fs::remove_file(&legacy);
        }
        #[cfg(target_os = "linux")]
        if let Some(record) = crate::capture::desktop::linux::hyprland::external_record() {
            return Self {
                record,
                ..shortcuts
            };
        }
        shortcuts
    }

    /// The shortcuts in `path`, else a custom record shortcut from the old file at `legacy`
    /// (returning `true` when that one should be moved across).
    pub fn load_from(path: &Path, legacy: &Path) -> (Self, bool) {
        let config = read_json(path);
        let mut shortcuts = Self::default();
        let stored = config.get("shortcuts");
        for kind in ShortcutKind::ALL {
            let value = stored.and_then(|s| s.get(kind_key(kind)));
            if kind.can_unset() && value.is_some_and(serde_json::Value::is_null) {
                shortcuts.set(kind, ShortcutConfig::unset());
            } else if let Some(cfg) = value
                .and_then(|v| serde_json::from_value::<Stored>(v.clone()).ok())
                .and_then(|s| from_stored(kind, s))
            {
                shortcuts.set(kind, cfg);
            }
        }
        if stored.is_some() {
            return (shortcuts, false);
        }
        let old = read_json(legacy);
        let old = (
            old.get("key_code").and_then(serde_json::Value::as_u64),
            old.get("carbon_modifiers")
                .and_then(serde_json::Value::as_u64),
            old.get("label").and_then(serde_json::Value::as_str),
        );
        if let (Some(key_code), Some(modifiers), Some(label)) = old
            && let Some(cfg) = from_stored(
                ShortcutKind::Record,
                Stored {
                    key_code: key_code as u32,
                    modifiers: modifiers as u32,
                    label: label.to_owned(),
                },
            )
            // 0.3.x saved its default too, and ⌃⌥⇧S was an older default.
            && !cfg.is_default_for(ShortcutKind::Record)
            && cfg.label != "⌃⌥⇧S"
        {
            shortcuts.record = cfg;
            return (shortcuts, true);
        }
        (shortcuts, false)
    }
}

fn kind_key(kind: ShortcutKind) -> &'static str {
    match kind {
        ShortcutKind::Record => "record",
        ShortcutKind::Paste => "paste",
    }
}

/// A stored default is read as the default, so it follows the active layout's key.
fn from_stored(kind: ShortcutKind, stored: Stored) -> Option<ShortcutConfig> {
    let cfg = ShortcutConfig {
        key_code: stored.key_code,
        carbon_modifiers: stored.modifiers,
        label: stored.label,
    };
    if cfg.label.is_empty() || cfg.key_label().is_empty() {
        return None;
    }
    Some(if cfg.is_default_for(kind) {
        kind.default_config()
    } else {
        cfg
    })
}

fn legacy_path() -> PathBuf {
    crate::shared::handoff::builtin_captures_dir()
        .parent()
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
        .join("config.json")
}

fn read_json(path: &Path) -> serde_json::Value {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}))
}

/// Saves one shortcut; a default is removed rather than written, and one turned off is `null`.
pub fn save_shortcut(kind: ShortcutKind, cfg: &ShortcutConfig) {
    if let Err(error) = save_shortcut_in(&crate::shared::handoff::default_config_path(), kind, cfg) {
        crate::capture::engine::runtime::apparent_log(format!("Saving the {kind:?} shortcut failed: {error:#}"));
    }
}

pub fn save_shortcut_in(
    path: &Path,
    kind: ShortcutKind,
    cfg: &ShortcutConfig,
) -> anyhow::Result<()> {
    let mut shortcuts = read_json(path)
        .get("shortcuts")
        .cloned()
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    let map = shortcuts.as_object_mut().expect("an object");
    if cfg.is_default_for(kind) {
        map.remove(kind_key(kind));
    } else if cfg.is_unset() {
        anyhow::ensure!(
            kind.can_unset(),
            "the {kind:?} shortcut can't be turned off"
        );
        map.insert(kind_key(kind).to_owned(), serde_json::Value::Null);
    } else {
        map.insert(
            kind_key(kind).to_owned(),
            serde_json::to_value(Stored {
                key_code: cfg.key_code,
                modifiers: cfg.carbon_modifiers,
                label: cfg.label.clone(),
            })?,
        );
    }
    crate::shared::handoff::set_config_value_in(path, "shortcuts", shortcuts)
}

/// The label of the paste shortcut, for hints (`⌥⇧V`, or `Alt+Shift+V`, unless changed; "the
/// paste shortcut" while it's off).
pub fn paste_label() -> String {
    let label = PASTE_LABEL
        .lock()
        .ok()
        .and_then(|label| label.clone())
        .unwrap_or_else(|| ShortcutConfig::default_paste().label);
    if label.is_empty() {
        "the paste shortcut".to_owned()
    } else {
        label
    }
}

static PASTE_LABEL: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

pub fn note_paste_label(label: &str) {
    if let Ok(mut slot) = PASTE_LABEL.lock() {
        *slot = Some(label.to_owned());
    }
}

// MARK: Recording a shortcut

/// A key combination pressed in the recorder.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pressed {
    pub key_code: u32,
    /// Carbon modifier bits (⌘ 1<<8, ⇧ 1<<9, ⌥ 1<<11, ⌃ 1<<12).
    pub modifiers: u32,
    /// The key as the active layout types it, without modifiers ("r", "5", "/").
    #[serde(default)]
    pub key: String,
    /// macOS has this combination as one of its own shortcuts (Keyboard Shortcuts settings).
    #[serde(default)]
    pub reserved: bool,
}

impl Pressed {
    /// Delete or forward delete with no modifier, which turns a shortcut off.
    pub fn is_delete(&self) -> bool {
        let modifiers = self.modifiers
            & (CARBON_CMD_KEY | CARBON_CONTROL_KEY | CARBON_OPTION_KEY | CARBON_SHIFT_KEY);
        modifiers == 0 && DELETE_KEYS.contains(&self.key_code)
    }
}

/// Windows virtual keys (Linux stores shortcuts the same way): Backspace, Delete.
#[cfg(not(target_os = "macos"))]
const DELETE_KEYS: [u32; 2] = [0x08, 0x2E];

/// `kVK_Delete`, `kVK_ForwardDelete`.
#[cfg(target_os = "macos")]
const DELETE_KEYS: [u32; 2] = [51, 117];

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rejection {
    /// Only ⇧, or nothing, with a key that isn't a function key.
    NoModifier,
    /// A key Apparent can't name or that the recorder uses (Esc).
    UnsupportedKey,
    /// macOS or every app already uses it.
    Reserved(String),
    /// Apparent's other shortcut.
    Taken(ShortcutKind),
    /// `RegisterEventHotKey` refused it.
    Refused,
}

impl Rejection {
    pub fn message(&self) -> String {
        match self {
            Self::NoModifier if cfg!(target_os = "windows") => {
                "Hold Ctrl, Alt or Win with the key.".to_owned()
            }
            Self::NoModifier if cfg!(target_os = "linux") => {
                "Hold Ctrl, Alt or Super with the key.".to_owned()
            }
            Self::NoModifier => "Hold ⌘, ⌥ or ⌃ with the key.".to_owned(),
            Self::UnsupportedKey => "Apparent can't use that key. Try a letter or number.".to_owned(),
            Self::Reserved(label) if cfg!(not(target_os = "macos")) => {
                let system = if cfg!(target_os = "windows") {
                    "Windows"
                } else {
                    "Your desktop"
                };
                if label.starts_with("Ctrl+")
                    && !label.contains("Alt+")
                    && !label.contains("Win+")
                    && !label.contains("Super+")
                {
                    format!("Apps use {label}. Add Alt.")
                } else {
                    format!("{system} uses {label}.")
                }
            }
            Self::Reserved(label) if label.starts_with('⌘') && !label.contains(['⌃', '⌥']) => {
                format!("Apps use {label}. Add ⌥ or ⌃.")
            }
            Self::Reserved(label) => format!("macOS uses {label}."),
            Self::Taken(kind) => format!("{} already uses this.", kind.name()),
            Self::Refused => "Another app has this shortcut.".to_owned(),
        }
    }
}

/// Virtual-key codes (Linux stores shortcuts as them too): F1–F16, then the keys named rather
/// than typed.
#[cfg(not(target_os = "macos"))]
const FUNCTION_KEYS: [(u32, &str); 16] = [
    (0x70, "F1"),
    (0x71, "F2"),
    (0x72, "F3"),
    (0x73, "F4"),
    (0x74, "F5"),
    (0x75, "F6"),
    (0x76, "F7"),
    (0x77, "F8"),
    (0x78, "F9"),
    (0x79, "F10"),
    (0x7A, "F11"),
    (0x7B, "F12"),
    (0x7C, "F13"),
    (0x7D, "F14"),
    (0x7E, "F15"),
    (0x7F, "F16"),
];

#[cfg(not(target_os = "macos"))]
const NAMED_KEYS: [(u32, &str); 8] = [
    (0x20, "Space"),
    (0x0D, "Enter"),
    (0x09, "Tab"),
    (0x08, "Backspace"),
    (0x25, "Left"),
    (0x27, "Right"),
    (0x28, "Down"),
    (0x26, "Up"),
];

#[cfg(not(target_os = "macos"))]
const KEY_ESCAPE: u32 = 0x1B;

/// macOS virtual key codes (`kVK_*`).
#[cfg(target_os = "macos")]
const FUNCTION_KEYS: [(u32, &str); 16] = [
    (122, "F1"),
    (120, "F2"),
    (99, "F3"),
    (118, "F4"),
    (96, "F5"),
    (97, "F6"),
    (98, "F7"),
    (100, "F8"),
    (101, "F9"),
    (109, "F10"),
    (103, "F11"),
    (111, "F12"),
    (105, "F13"),
    (107, "F14"),
    (113, "F15"),
    (106, "F16"),
];

#[cfg(target_os = "macos")]
const NAMED_KEYS: [(u32, &str); 8] = [
    (49, "Space"),
    (36, "↩"),
    (48, "⇥"),
    (51, "⌫"),
    (123, "←"),
    (124, "→"),
    (125, "↓"),
    (126, "↑"),
];

#[cfg(target_os = "macos")]
const KEY_ESCAPE: u32 = 53;

/// The name a key is shown with: a function key, a named key, or the typed character in capitals.
pub fn key_label(key_code: u32, typed: &str) -> Option<String> {
    if let Some((_, name)) = FUNCTION_KEYS
        .iter()
        .chain(NAMED_KEYS.iter())
        .find(|(code, _)| *code == key_code)
    {
        return Some((*name).to_owned());
    }
    if key_code == KEY_ESCAPE {
        return None;
    }
    let mut chars = typed
        .chars()
        .filter(|c| !c.is_control() && !c.is_whitespace());
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    Some(ch.to_uppercase().collect())
}

fn is_function_key(key_code: u32) -> bool {
    FUNCTION_KEYS.iter().any(|(code, _)| *code == key_code)
}

/// Combinations macOS or nearly every app keeps for itself, as (Carbon modifiers, key label).
/// Turned-off system shortcuts are allowed; `Pressed::reserved` covers the enabled ones,
/// including any Kit set up himself.
fn reserved(modifiers: u32, key: &str) -> bool {
    const CMD: u32 = CARBON_CMD_KEY;
    const SHIFT: u32 = CARBON_SHIFT_KEY;
    const OPT: u32 = CARBON_OPTION_KEY;
    const CTRL: u32 = CARBON_CONTROL_KEY;
    if cfg!(target_os = "windows") {
        // Win (⌘'s bit) with a key, alone or with Shift, is Windows' own (Win+R, Win+Shift+S…);
        // Ctrl with one key is an app's (Ctrl+C, Ctrl+Shift+T…).
        return modifiers & CMD != 0 && modifiers & (CTRL | OPT) == 0
            || modifiers & (CTRL | CMD | OPT) == CTRL
            || [
                (OPT, "Tab"),
                (OPT | SHIFT, "Tab"),
                (OPT, "F4"),
                (OPT, "Space"),
                (CTRL | OPT, "Tab"),
                (CTRL | CMD | OPT, "Space"),
            ]
            .contains(&(modifiers, key));
    }
    if cfg!(target_os = "linux") {
        // Super (⌘'s bit) with a key, alone or with Shift, is the desktop's (Super+A, the
        // workspaces…); Ctrl with one key is an app's. Then Alt+Tab, Alt+F2, Ctrl+Alt+T…
        return modifiers & CMD != 0 && modifiers & (CTRL | OPT) == 0
            || modifiers & (CTRL | CMD | OPT) == CTRL
            || [
                (OPT, "Tab"),
                (OPT | SHIFT, "Tab"),
                (OPT, "F2"),
                (OPT, "F4"),
                (OPT, "Space"),
                (OPT, "F7"),
                (OPT, "F8"),
                (CTRL | OPT, "Tab"),
                (CTRL | OPT, "T"),
                (CTRL | OPT, "Left"),
                (CTRL | OPT, "Right"),
                (CTRL | OPT, "Up"),
                (CTRL | OPT, "Down"),
            ]
            .contains(&(modifiers, key));
    }
    // ⌘ with one key is an app's menu shortcut somewhere (⌘Q, ⌘W, ⌘R…).
    if modifiers == CMD || modifiers == CMD | SHIFT && ["3", "4", "5", "6", "Q", "/"].contains(&key)
    {
        return true;
    }
    [
        (CTRL | CMD, "Q"),
        (CTRL | CMD, "F"),
        (CTRL | CMD, "Space"),
        (CTRL | CMD, "D"),
        (CTRL, "Space"),
        (CTRL | OPT, "Space"),
        (OPT | CMD, "Space"),
        (CMD | SHIFT, "Space"),
        (OPT | CMD, "D"),
        (OPT | CMD, "H"),
        (OPT | CMD, "M"),
        (OPT | CMD, "W"),
        (CMD | SHIFT, "⇥"),
        (CTRL, "⇥"),
    ]
    .contains(&(modifiers, key))
}

/// Checks a pressed combination for `kind` and names it. `other` is Apparent's other shortcut.
pub fn validate(
    kind: ShortcutKind,
    pressed: &Pressed,
    other: &ShortcutConfig,
) -> Result<ShortcutConfig, Rejection> {
    let modifiers = pressed.modifiers
        & (CARBON_CMD_KEY | CARBON_CONTROL_KEY | CARBON_OPTION_KEY | CARBON_SHIFT_KEY);
    let key = key_label(pressed.key_code, &pressed.key).ok_or(Rejection::UnsupportedKey)?;
    let strong = modifiers & (CARBON_CMD_KEY | CARBON_CONTROL_KEY | CARBON_OPTION_KEY) != 0;
    if !strong && !is_function_key(pressed.key_code) {
        return Err(Rejection::NoModifier);
    }
    let label = format!("{}{key}", modifier_symbols(modifiers));
    if pressed.reserved || reserved(modifiers, &key) {
        return Err(Rejection::Reserved(label));
    }
    let cfg = ShortcutConfig {
        key_code: pressed.key_code,
        carbon_modifiers: modifiers,
        label,
    };
    let other_kind = match kind {
        ShortcutKind::Record => ShortcutKind::Paste,
        ShortcutKind::Paste => ShortcutKind::Record,
    };
    if cfg.key_code == other.key_code && cfg.carbon_modifiers == other.carbon_modifiers
        || cfg.label == other.label
    {
        return Err(Rejection::Taken(other_kind));
    }
    Ok(if cfg.is_default_for(kind) {
        kind.default_config()
    } else {
        cfg
    })
}

// MARK: Launch at login (SMAppService on macOS 13+, the Run key on Windows)

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoginItem {
    Off,
    On,
    /// Registered, waiting for the user in System Settings › Login Items (on Windows: switched
    /// off in Settings › Apps › Startup).
    NeedsApproval,
    /// Not running from Apparent.app (a development build).
    Unavailable,
}

/// Apply an explicit file preference to OS registration, without fighting approval switches.
fn reconcile_login_item_in(
    path: &Path,
    current: LoginItem,
    stale_command: bool,
    mut set: impl FnMut(bool) -> anyhow::Result<()>,
) -> anyhow::Result<()> {
    let Some(wanted) = crate::shared::handoff::load_config_from(path).launch_at_login else {
        return Ok(());
    };
    if current == LoginItem::Unavailable || current == LoginItem::NeedsApproval && wanted {
        return Ok(());
    }
    if wanted != (current == LoginItem::On) || wanted && stale_command {
        set(wanted)?;
    }
    Ok(())
}

/// Reconcile at launch and on Settings refresh, so agent edits take effect without restarting.
pub fn reconcile_login_item() {
    if option_env!("APPARENT_PREVIEW_BUILD").is_some() {
        return;
    }
    if let Err(error) = reconcile_login_item_in(
        &crate::shared::handoff::default_config_path(), login_item(), login_command_stale(), set_login_item,
    ) {
        crate::capture::engine::runtime::apparent_log(format!("Launch at login: {error:#}"));
    }
}

#[cfg(target_os = "macos")]
fn login_command_stale() -> bool { false }
#[cfg(target_os = "windows")]
fn login_command_stale() -> bool { crate::capture::engine::runtime::platform::legacy_windows::platform::login::command_stale() }
#[cfg(target_os = "linux")]
fn login_command_stale() -> bool { crate::capture::desktop::linux::login::command_stale() }

#[cfg(target_os = "macos")]
pub use self::login::{login_item, open_login_items_settings, set_login_item};

#[cfg(target_os = "macos")]
mod login {
    use super::LoginItem;

    #[link(name = "ServiceManagement", kind = "framework")]
    unsafe extern "C" {}

    fn main_app_service() -> Option<objc2::rc::Retained<objc2::runtime::AnyObject>> {
        // Shared config preferences cannot register or remove a production login item.
        if option_env!("APPARENT_PREVIEW_BUILD").is_some() {
            return None;
        }
        use objc2::runtime::{AnyClass, AnyObject};
        let class = AnyClass::get(c"SMAppService")?;
        let exe = std::env::current_exe().ok()?;
        // SMAppService registers the bundle this executable is in; outside one it can't.
        exe.ancestors()
            .find(|p| p.extension().is_some_and(|ext| ext == "app"))?;
        unsafe {
            let service: *mut AnyObject = objc2::msg_send![class, mainAppService];
            objc2::rc::Retained::retain(service)
        }
    }

    fn login_item_from_status(status: isize) -> LoginItem {
        match status {
            1 => LoginItem::On,
            2 => LoginItem::NeedsApproval,
            _ => LoginItem::Off,
        }
    }

    pub fn login_item() -> LoginItem {
        let Some(service) = main_app_service() else {
            return LoginItem::Unavailable;
        };
        let status: isize = unsafe { objc2::msg_send![&*service, status] };
        login_item_from_status(status)
    }

    pub fn set_login_item(on: bool) -> anyhow::Result<()> {
        use objc2::runtime::Bool;
        use objc2_foundation::NSError;
        let service =
            main_app_service().ok_or_else(|| anyhow::anyhow!("not running from Apparent.app"))?;
        let mut error: Option<objc2::rc::Retained<NSError>> = None;
        let ok: Bool = unsafe {
            if on {
                objc2::msg_send![&*service, registerAndReturnError: &mut error]
            } else {
                objc2::msg_send![&*service, unregisterAndReturnError: &mut error]
            }
        };
        if ok.as_bool() {
            Ok(())
        } else {
            anyhow::bail!(
                "{}",
                error.map_or_else(
                    || "unknown error".to_owned(),
                    |e| e.localizedDescription().to_string()
                )
            )
        }
    }

    pub fn open_login_items_settings() {
        use objc2::runtime::AnyClass;
        if let Some(class) = AnyClass::get(c"SMAppService") {
            let _: () = unsafe { objc2::msg_send![class, openSystemSettingsLoginItems] };
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn login_item_statuses_keep_the_toggle_available() {
            assert_eq!(login_item_from_status(0), LoginItem::Off);
            assert_eq!(login_item_from_status(1), LoginItem::On);
            assert_eq!(login_item_from_status(2), LoginItem::NeedsApproval);
            assert_eq!(login_item_from_status(3), LoginItem::Off);
            assert_eq!(login_item_from_status(99), LoginItem::Off);
        }
    }
}

#[cfg(target_os = "windows")]
pub fn login_item() -> LoginItem {
    match crate::capture::engine::runtime::platform::legacy_windows::platform::login::status() {
        (false, _) => LoginItem::Off,
        (true, false) => LoginItem::On,
        (true, true) => LoginItem::NeedsApproval,
    }
}

#[cfg(target_os = "windows")]
pub fn set_login_item(on: bool) -> anyhow::Result<()> {
    crate::capture::engine::runtime::platform::legacy_windows::platform::login::set(on)
}

#[cfg(target_os = "windows")]
fn open_login_items_settings() {
    crate::capture::engine::runtime::platform::legacy_windows::platform::login::open_startup_settings();
}

#[cfg(target_os = "linux")]
pub fn login_item() -> LoginItem {
    match crate::capture::desktop::linux::login::status() {
        (false, _) => LoginItem::Off,
        (true, false) => LoginItem::On,
        (true, true) => LoginItem::NeedsApproval,
    }
}

#[cfg(target_os = "linux")]
pub fn set_login_item(on: bool) -> anyhow::Result<()> {
    crate::capture::desktop::linux::login::set(on)
}

#[cfg(target_os = "linux")]
fn open_login_items_settings() {
    crate::capture::desktop::linux::login::open_startup_settings();
}

// MARK: What the window shows

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Pane {
    General,
    Recording,
    Storage,
    Permissions,
    Agents,
    About,
}

/// The capture-reading skill source is `capture/agent/skills/apparent/SKILL.md`.
pub const SKILL_COMMAND: &str = "bunx skills add https://github.com/tryapparent/apparent/tree/main/capture/agent/skills/apparent";

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub version: String,
    pub hosted: Option<crate::connect::hosted::AccountView>,
    pub login: LoginView,
    pub agent_demo_video: bool,
    pub shortcuts: Vec<ShortcutView>,
    pub microphone: MicrophoneView,
    pub transcriber: TranscriberView,
    pub lower_playback: bool,
    pub playback_level: u8,
    pub video_export: bool,
    pub retention: RetentionView,
    /// How much the captures folder takes, once measured.
    pub captures_size: Option<String>,
    pub captures_dir: CapturesDirView,
    pub permissions: Vec<PermissionView>,
    pub updates: bool,
    pub update_check: UpdateCheck,
    pub update_channel: &'static str,
    pub main_updates: bool,
    pub command_line: CommandLineView,
    pub skill_command: &'static str,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    pub checking: bool,
    pub result: Option<UpdateResult>,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateResult {
    Available,
    Current,
    Failed,
}

impl UpdateResult {
    pub fn label(self) -> &'static str {
        match self {
            Self::Available => "Update available",
            Self::Current => "No update available",
            Self::Failed => "Update check failed",
        }
    }
}

/// The "Command-line tool" row.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandLineView {
    /// Where `apparent` is linked, once it is.
    pub path: Option<String>,
    /// "install", "uninstall", or none.
    pub button: Option<&'static str>,
    pub enabled: bool,
    pub caption: String,
    pub error: bool,
}

impl CommandLineView {
    fn new(state: &crate::composition::command_line::State, error: Option<&str>) -> Self {
        use crate::composition::command_line::{State, tilde, usually_on_path};
        let folder = |link: &std::path::Path| link.parent().map_or_else(String::new, tilde);
        let mut view = Self {
            path: None,
            button: Some("install"),
            enabled: true,
            caption: String::new(),
            error: false,
        };
        match state {
            State::Installed { link, homebrew } => {
                view.path = Some(tilde(link));
                view.caption = "Agents and your terminal can run apparent to read captures, zoom \
                                into a moment and export video."
                    .to_owned();
                if *homebrew {
                    view.button = None;
                    view.caption = "Installed with Homebrew. Agents and your terminal can run \
                                    apparent to read captures, zoom into a moment and export video."
                        .to_owned();
                } else {
                    view.button = Some("uninstall");
                    if !usually_on_path(link) {
                        view.caption = format!(
                            "If your shell can't find apparent, add {} to your PATH.",
                            folder(link)
                        );
                    }
                }
            }
            State::Missing { link } => {
                view.caption = format!(
                    "Links apparent into {}, so agents and your terminal can run it.",
                    folder(link)
                );
            }
            State::Stale { link } => {
                view.caption = format!(
                    "{} points to another copy of Apparent. Install links it to this one.",
                    tilde(link)
                );
            }
            State::Taken { link } => {
                view.button = None;
                view.error = true;
                view.caption = format!(
                    "{} is another program, so Apparent leaves it alone.",
                    tilde(link)
                );
            }
            State::Nowhere => {
                view.enabled = false;
                view.error = true;
                view.caption =
                    "There's no folder on your PATH that Apparent can write to.".to_owned();
            }
            State::Unavailable => {
                view.enabled = false;
                view.caption = if cfg!(target_os = "linux") {
                    "The command-line tool links to Apparent's AppImage; run Apparent from one."
                } else {
                    "The command-line tool works in the installed app."
                }
                .to_owned();
            }
        }
        if let Some(error) = error {
            view.error = true;
            view.caption = error.to_owned();
        }
        view
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginView {
    pub on: bool,
    pub available: bool,
    pub needs_approval: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutView {
    pub kind: ShortcutKind,
    pub title: &'static str,
    pub label: String,
    pub key: String,
    pub shift: bool,
    pub control: bool,
    pub option: bool,
    pub command: bool,
    pub is_default: bool,
    /// A live system binding owns this chord; change it in the compositor instead.
    pub externally_managed: bool,
    /// It can be turned off (paste); `label` is empty while it is.
    pub clearable: bool,
    /// The recorder is waiting for a new combination.
    pub recording: bool,
    /// Why the last combination was turned down.
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MicrophoneView {
    /// `"default"` or the chosen device's id.
    pub selected: String,
    /// What the popup says.
    pub value: String,
    /// "System default (…)", a separator (`id` "-"), then each input; a chosen device that
    /// isn't connected comes last, disabled.
    pub options: Vec<MenuOptionView>,
    /// Why another device is recording.
    pub note: Option<String>,
    /// The device whose level the window shows, only once Microphone is allowed.
    pub meter: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct MenuOptionView {
    pub id: String,
    pub title: String,
    pub enabled: bool,
}

fn microphone_view(
    choice: Option<&crate::capture::engine::runtime::audio::mic::Choice>,
    inputs: Option<&crate::capture::engine::runtime::audio::mic::Inputs>,
    allowed: bool,
) -> MicrophoneView {
    use crate::capture::engine::runtime::audio::mic::Pick;
    let option = |id: &str, title: String, enabled: bool| MenuOptionView {
        id: id.to_owned(),
        title,
        enabled,
    };
    let empty = crate::capture::engine::runtime::audio::mic::Inputs::default();
    let pick = Pick::new(choice, inputs.unwrap_or(&empty));
    let default_title = match inputs.and_then(crate::capture::engine::runtime::audio::mic::Inputs::default_input) {
        Some(input) => format!("System default ({})", input.name),
        None => "System default".to_owned(),
    };
    let mut options = vec![option("default", default_title, true)];
    if let Some(inputs) = inputs.filter(|i| !i.devices.is_empty()) {
        options.push(option("-", String::new(), false));
        options.extend(
            inputs
                .devices
                .iter()
                .map(|input| option(&input.id, input.name.clone(), true)),
        );
    }
    // Until the first list arrives, a choice is shown as it was saved, not as missing.
    let loaded = inputs.is_some();
    if let (Some(choice), true, Pick::Missing { .. }) = (choice, loaded, &pick) {
        options.push(option(
            &choice.id,
            format!("{} (not connected)", choice.name),
            false,
        ));
    }
    MicrophoneView {
        selected: choice.map_or_else(|| "default".to_owned(), |c| c.id.clone()),
        value: match (choice, &pick) {
            (_, Pick::Chosen(input)) => input.name.clone(),
            (Some(choice), _) => choice.name.clone(),
            (None, _) => "System default".to_owned(),
        },
        options,
        note: if loaded { pick.note() } else { None },
        meter: pick
            .device()
            .filter(|_| allowed && loaded)
            .map(|input| input.id.clone()),
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriberView {
    /// What recordings use now: "apple", "hex" or "command".
    pub engine: &'static str,
    pub hex_title: &'static str,
    pub hex_enabled: bool,
    pub language: String,
    pub apple_locale: String,
    pub models: Vec<ModelView>,
    /// The program's file name while `transcribeCommand` is usable (read, never run); Command
    /// is offered only then.
    pub command: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ModelView {
    pub id: String,
    pub title: String,
    pub chosen: bool,
}

/// Settings › Storage › Location: where new captures go (`"capturesDir"`).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CapturesDirView {
    /// The folder, with the home folder as `~`.
    pub path: String,
    /// Chosen in Settings rather than the built-in folder, so Reset shows.
    pub custom: bool,
}

impl CapturesDirView {
    fn new(config: &crate::shared::handoff::Config) -> Self {
        let chosen = config
            .captures_dir
            .as_deref()
            .and_then(crate::shared::handoff::configured_captures_dir);
        Self {
            custom: chosen.is_some(),
            path: crate::composition::command_line::tilde(
                &chosen.unwrap_or_else(crate::shared::handoff::builtin_captures_dir),
            ),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RetentionView {
    /// The chosen preset, or `None` for a policy written by hand.
    pub preset: Option<&'static str>,
    pub options: Vec<OptionView>,
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct OptionView {
    pub id: &'static str,
    pub label: &'static str,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionView {
    pub permission: Permission,
    pub status: Status,
    pub required: bool,
    pub requested: bool,
    /// Granted, but on only after Apparent reopens.
    pub relaunch: bool,
}

fn preset_id(preset: crate::capture::engine::bundle::retention::Preset) -> &'static str {
    match preset {
        crate::capture::engine::bundle::retention::Preset::Week => "week",
        crate::capture::engine::bundle::retention::Preset::Month => "month",
        crate::capture::engine::bundle::retention::Preset::Forever => "forever",
    }
}

fn retention_note(policy: &crate::capture::engine::bundle::retention::Policy) -> String {
    let days = |n: u32| {
        if n == 1 {
            "1 day".to_owned()
        } else {
            format!("{n} days")
        }
    };
    let bin = if cfg!(target_os = "windows") {
        "Recycle Bin"
    } else {
        "Trash"
    };
    match (policy.media_days, policy.bundle_days) {
        (None, None) => String::new(),
        (Some(media), Some(bundle)) => format!(
            "Video and audio go to the {bin} after {}, the rest of each capture after {}.",
            days(media),
            days(bundle)
        ),
        (Some(media), None) => format!("Video and audio go to the {bin} after {}.", days(media)),
        (None, Some(bundle)) => format!("Captures go to the {bin} after {}.", days(bundle)),
    }
}

fn shortcut_view(
    kind: ShortcutKind,
    cfg: &ShortcutConfig,
    recording: bool,
    error: Option<&String>,
) -> ShortcutView {
    let mods = cfg.carbon_modifiers;
    ShortcutView {
        kind,
        title: kind.name(),
        label: cfg.label.clone(),
        key: cfg.key_label().to_owned(),
        shift: mods & CARBON_SHIFT_KEY != 0,
        control: mods & CARBON_CONTROL_KEY != 0,
        option: mods & CARBON_OPTION_KEY != 0,
        command: mods & CARBON_CMD_KEY != 0,
        is_default: cfg.is_default_for(kind),
        externally_managed: external_record_owned(kind),
        clearable: kind.can_unset(),
        recording,
        error: error.cloned(),
    }
}

pub(crate) fn external_record_owned(kind: ShortcutKind) -> bool {
    #[cfg(target_os = "linux")]
    {
        kind == ShortcutKind::Record && crate::capture::desktop::linux::hyprland::external_record().is_some()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = kind;
        false
    }
}

// MARK: The window's logic

/// What the window can ask for.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(tag = "action", rename_all = "kebab-case")]
pub enum Action {
    /// A settings control: `launchAtLogin`, `videoExport`, `lowerPlayback` (bool),
    /// `playbackLevel` (u8), `microphone` ("default" or a device id), `transcriber`
    /// ("apple"/"hex"/"command"), `hexModel` (an id), `hexLanguage` (a code), `appleLocale`
    /// (a locale id or "auto"), `retention` (a preset id).
    Set {
        key: String,
        value: serde_json::Value,
    },
    /// The recorder for `kind` was clicked.
    RecordShortcut {
        kind: ShortcutKind,
    },
    /// A combination was pressed while recording.
    Shortcut {
        kind: ShortcutKind,
        #[serde(flatten)]
        pressed: Pressed,
    },
    /// Esc, a click elsewhere, or the window losing focus.
    CancelShortcut,
    ResetShortcut {
        kind: ShortcutKind,
    },
    /// The × beside the paste shortcut, or Delete while it records: no shortcut at all.
    ClearShortcut {
        kind: ShortcutKind,
    },
    /// A permission row's button.
    Grant {
        permission: Permission,
    },
    /// Reopen Apparent so a Screen Recording grant takes.
    Reopen,
    ShowCaptures,
    CheckForUpdates,
    DismissUpdateCheck,
    UpdateNow,
    UpdateLater,
    /// "Show Welcome Tour" on About.
    Tour,
    /// Link `apparent` into a folder on PATH, or remove that link.
    InstallCli,
    UninstallCli,
    ConnectHosted,
    ManageSharedLinks,
    Close,
}

/// What the app does with an action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    None,
    ConnectHosted,
    ManageSharedLinks,
    /// Ask macOS (the system prompt): the first press on a row that was never asked.
    Request(Permission),
    /// Already asked or denied: its System Settings pane.
    OpenSettings(Permission),
    OpenLoginItems,
    ShowCaptures,
    CheckForUpdates,
    RelaunchUpdate,
    QuitForUpdate,
    /// The captures folder may have changed size.
    Measure,
    /// The Apple Speech language changed.
    SpeechModel,
    Relaunch,
    Tour,
    Close,
}

/// The outside world the window changes: Apparent's hotkeys and the config file.
pub trait Host {
    fn shortcuts(&self) -> Shortcuts;
    /// Registers both, keeping the old ones if macOS refuses one.
    fn set_shortcuts(&mut self, shortcuts: &Shortcuts) -> Result<(), ShortcutKind>;
    fn pause_shortcuts(&mut self, paused: bool);
    fn config_path(&self) -> PathBuf;
    fn set_login_item(&mut self, on: bool) -> anyhow::Result<()>;
    /// Apparent's binary and the folders `apparent` may be linked into.
    fn command_line(&self) -> crate::composition::command_line::Places;
}

/// The window's own state: what it's recording, what failed, what was asked for.
#[derive(Debug, Default)]
pub struct Panel {
    recording: Option<ShortcutKind>,
    error: Option<(ShortcutKind, String)>,
    /// Why the last Install or Uninstall failed.
    cli_error: Option<String>,
    requested: BTreeSet<Permission>,
    grants: Option<Grants>,
    captures_size: Option<u64>,
    /// The connected microphones, once listed (`mic::Watch` keeps them fresh).
    inputs: Option<crate::capture::engine::runtime::audio::mic::Inputs>,
    update_check: UpdateCheck,
}

impl Panel {
    /// The same Record recorder state for Settings and the onboarding shortcut page.
    pub fn record_shortcut(&self, host: &dyn Host) -> ShortcutView {
        let kind = ShortcutKind::Record;
        let error = self
            .error
            .as_ref()
            .filter(|(k, _)| *k == kind)
            .map(|(_, e)| e);
        shortcut_view(
            kind,
            host.shortcuts().get(kind),
            self.recording == Some(kind),
            error,
        )
    }

    #[cfg(test)]
    pub fn recording(&self) -> Option<ShortcutKind> {
        self.recording
    }

    pub fn set_grants(&mut self, grants: Grants) {
        self.grants = Some(grants);
    }

    pub fn set_captures_size(&mut self, bytes: u64) {
        self.captures_size = Some(bytes);
    }

    pub fn set_inputs(&mut self, inputs: crate::capture::engine::runtime::audio::mic::Inputs) {
        self.inputs = Some(inputs);
    }

    #[cfg(target_os = "linux")]
    fn finish_update_check(&mut self, result: Result<&'static str, String>) {
        self.update_check.checking = false;
        self.update_check.result = Some(match result {
            // The current Linux feed contract checks AND installs. "Available" includes
            // a newer image already staged for the next launch; this is not discovery-only.
            Ok("installed; restart Apparent to use it") | Ok("already installed; restart Apparent to use it") =>
                UpdateResult::Available,
            Ok("up to date") => UpdateResult::Current,
            Ok(other) => {
                crate::capture::engine::runtime::apparent_log(format!("Unexpected update check result: {other}"));
                UpdateResult::Failed
            }
            Err(error) => {
                crate::capture::engine::runtime::apparent_log(format!("Could not check for updates: {error}"));
                UpdateResult::Failed
            }
        });
    }

    pub fn on(&mut self, action: Action, host: &mut dyn Host) -> Effect {
        match action {
            Action::ConnectHosted => Effect::ConnectHosted,
            Action::ManageSharedLinks => Effect::ManageSharedLinks,
            Action::Set { key, value } => self.set(&key, &value, host),
            Action::RecordShortcut { kind } => {
                if external_record_owned(kind) {
                    return Effect::None;
                }
                self.error = None;
                if self.recording.replace(kind).is_none() {
                    host.pause_shortcuts(true);
                }
                Effect::None
            }
            Action::CancelShortcut => {
                self.stop_recording(host);
                Effect::None
            }
            Action::Shortcut { kind, pressed } => {
                if external_record_owned(kind) {
                    return Effect::None;
                }
                if self.recording != Some(kind) {
                    return Effect::None;
                }
                self.stop_recording(host);
                let mut shortcuts = host.shortcuts();
                if kind.can_unset() && pressed.is_delete() {
                    self.apply(kind, ShortcutConfig::unset(), &mut shortcuts, host);
                    return Effect::None;
                }
                let other = match kind {
                    ShortcutKind::Record => &shortcuts.paste,
                    ShortcutKind::Paste => &shortcuts.record,
                };
                match validate(kind, &pressed, other) {
                    Ok(cfg) => self.apply(kind, cfg, &mut shortcuts, host),
                    Err(rejection) => self.error = Some((kind, rejection.message())),
                }
                Effect::None
            }
            Action::ResetShortcut { kind } => {
                if external_record_owned(kind) {
                    return Effect::None;
                }
                self.stop_recording(host);
                let mut shortcuts = host.shortcuts();
                self.apply(kind, kind.default_config(), &mut shortcuts, host);
                Effect::None
            }
            Action::ClearShortcut { kind } => {
                if external_record_owned(kind) {
                    return Effect::None;
                }
                self.stop_recording(host);
                if kind.can_unset() {
                    let mut shortcuts = host.shortcuts();
                    self.apply(kind, ShortcutConfig::unset(), &mut shortcuts, host);
                }
                Effect::None
            }
            Action::Grant { permission } => self.grant(permission),
            Action::Reopen => {
                if self.screen_relaunch() {
                    Effect::Relaunch
                } else {
                    Effect::None
                }
            }
            Action::ShowCaptures => Effect::ShowCaptures,
            Action::CheckForUpdates => {
                if cfg!(target_os = "linux") && self.update_check.checking {
                    return Effect::None;
                }
                if cfg!(target_os = "linux") {
                    self.update_check = UpdateCheck { checking: true, result: None };
                }
                Effect::CheckForUpdates
            }
            Action::DismissUpdateCheck => {
                self.update_check.result = None;
                Effect::None
            }
            Action::UpdateLater => {
                self.update_check.result = None;
                Effect::None
            }
            Action::UpdateNow => {
                if cfg!(target_os = "linux") && self.update_check.result == Some(UpdateResult::Available) {
                    Effect::RelaunchUpdate
                } else {
                    Effect::None
                }
            }
            Action::Tour => Effect::Tour,
            Action::InstallCli => {
                let result = crate::composition::command_line::install(&host.command_line());
                self.note_cli("install", result.map(drop));
                Effect::None
            }
            Action::UninstallCli => {
                let result = crate::composition::command_line::uninstall(&host.command_line());
                self.note_cli("uninstall", result);
                Effect::None
            }
            Action::Close => {
                self.stop_recording(host);
                Effect::Close
            }
        }
    }

    fn note_cli(&mut self, verb: &str, result: anyhow::Result<()>) {
        self.cli_error = result.err().map(|error| {
            crate::capture::engine::runtime::apparent_log(format!("Command-line tool: {verb}: {error:#}"));
            format!("Couldn't {verb} it: {error}.")
        });
    }

    fn stop_recording(&mut self, host: &mut dyn Host) {
        if self.recording.take().is_some() {
            host.pause_shortcuts(false);
        }
    }

    fn apply(
        &mut self,
        kind: ShortcutKind,
        cfg: ShortcutConfig,
        shortcuts: &mut Shortcuts,
        host: &mut dyn Host,
    ) {
        if shortcuts.get(kind) == &cfg {
            self.error = None;
            return;
        }
        shortcuts.set(kind, cfg.clone());
        match host.set_shortcuts(shortcuts) {
            Ok(()) => {
                self.error = None;
                if let Err(error) = save_shortcut_in(&host.config_path(), kind, &cfg) {
                    crate::capture::engine::runtime::apparent_log(format!("Saving the shortcut failed: {error:#}"));
                }
            }
            Err(refused) => self.error = Some((refused, Rejection::Refused.message())),
        }
    }

    fn set(&mut self, key: &str, value: &serde_json::Value, host: &mut dyn Host) -> Effect {
        let path = host.config_path();
        let write = |key: &str, value: serde_json::Value| {
            if let Err(error) = crate::shared::handoff::set_config_value_in(&path, key, value) {
                crate::capture::engine::runtime::apparent_log(format!("Saving {key} failed: {error:#}"));
            }
        };
        match (key, value) {
            ("agentDemoVideo", serde_json::Value::Bool(on)) => {
                if let Err(error) = crate::shared::handoff::set_experiment_in(&path, "agentDemoVideo", *on) {
                    crate::capture::engine::runtime::apparent_log(format!("Saving {key} failed: {error:#}"));
                }
            }
            ("launchAtLogin", serde_json::Value::Bool(on)) => {
                let previous = std::fs::read(&path)
                    .ok()
                    .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
                    .and_then(|config| config.get(key).cloned())
                    .unwrap_or(serde_json::Value::Null);
                if let Err(error) = crate::shared::handoff::set_config_value_in(&path, key, (*on).into()) {
                    crate::capture::engine::runtime::apparent_log(format!("Saving {key} failed: {error:#}"));
                    return Effect::None;
                }
                if let Err(error) = host.set_login_item(*on) {
                    crate::capture::engine::runtime::apparent_log(format!("Launch at login: {error:#}"));
                    if let Err(rollback) = crate::shared::handoff::set_config_value_in(&path, key, previous) {
                        crate::capture::engine::runtime::apparent_log(format!("Restoring {key} failed: {rollback:#}"));
                    }
                    return Effect::None;
                }
                if *on && login_item() == LoginItem::NeedsApproval {
                    return Effect::OpenLoginItems;
                }
            }

            ("videoExport" | "lowerPlayback", serde_json::Value::Bool(on)) => {
                write(key, (*on).into())
            }
            ("updateChannel", serde_json::Value::String(channel))
                if matches!(channel.as_str(), "stable" | "main") =>
            {
                write(key, value.clone());
            }
            ("playbackLevel", serde_json::Value::Number(level))
                if level.as_u64().is_some_and(|n| n <= 100) =>
            {
                write(key, value.clone());
            }
            ("microphone", serde_json::Value::String(id)) => {
                let inputs = self.inputs.clone().unwrap_or_default();
                if let Err(error) = crate::capture::engine::runtime::audio::mic::save_in(&path, id, &inputs) {
                    crate::capture::engine::runtime::apparent_log(format!("Saving microphone failed: {error:#}"));
                }
            }
            ("transcriber", serde_json::Value::String(name)) => {
                if let Some(engine) = crate::capture::engine::runtime::audio::transcription::transcribe::Engine::parse(name) {
                    write("transcriber", engine.name().into());
                }
            }
            ("hexModel", serde_json::Value::String(model)) if !model.trim().is_empty() => {
                // Picking a Hex model is choosing Hex.
                write("hexModel", model.trim().into());
                write("transcriber", "hex".into());
            }
            ("hexLanguage", serde_json::Value::String(language)) => {
                if let Some(language) = crate::shared::hex::language_code(language) {
                    write("hexLanguage", language.into());
                }
            }
            ("appleLocale", serde_json::Value::String(locale)) => {
                let locale = locale.trim();
                if locale.is_empty() || locale.eq_ignore_ascii_case("auto") {
                    write("appleLocale", serde_json::Value::Null);
                    return Effect::SpeechModel;
                }
                if let Some(locale) = crate::capture::engine::runtime::audio::transcription::speech_model::locale_id(locale) {
                    write("appleLocale", locale.into());
                    return Effect::SpeechModel;
                }
            }
            // An absolute folder from the helper's open panel; "" or null is the built-in one.
            // Captures already made stay where they are.
            ("capturesDir", serde_json::Value::Null | serde_json::Value::String(_)) => {
                match crate::shared::handoff::capture_location::set_in(&path, value.as_str(), None, || true) {
                    Ok(()) => return Effect::Measure,
                    Err(code) => crate::capture::engine::runtime::apparent_log(format!("Saving capturesDir failed: {code}")),
                }
            }
            ("retention", serde_json::Value::String(id)) => {
                if let Some(preset) = crate::capture::engine::bundle::retention::Preset::ALL
                    .into_iter()
                    .find(|p| preset_id(*p) == id)
                {
                    if let Err(error) = preset.policy().save_to(&path) {
                        crate::capture::engine::runtime::apparent_log(format!("Saving retention failed: {error:#}"));
                    }
                    return Effect::Measure;
                }
            }
            _ => crate::capture::engine::runtime::apparent_log(format!("Settings: ignoring {key}={value}")),
        }
        Effect::None
    }

    fn status(&self, permission: Permission) -> Status {
        self.grants
            .as_ref()
            .map_or(Status::NotDetermined, |g| g.status(permission))
    }

    fn screen_relaunch(&self) -> bool {
        self.grants
            .as_ref()
            .is_some_and(|g| g.granted(Permission::ScreenRecording) && g.screen_needs_relaunch)
    }

    /// Like onboarding: the first press asks macOS, later ones (or a denied one) open the
    /// System Settings pane.
    fn grant(&mut self, permission: Permission) -> Effect {
        let status = self.status(permission);
        if status == Status::Granted {
            return Effect::None;
        }
        if self.requested.insert(permission) && status != Status::Denied {
            Effect::Request(permission)
        } else {
            Effect::OpenSettings(permission)
        }
    }

    pub fn view(&self, host: &dyn Host) -> View {
        let config = crate::shared::handoff::load_config_from(&host.config_path());
        let shortcuts = host.shortcuts();
        let hex = crate::shared::hex::cached_status();
        let engine = config
            .transcriber
            .as_deref()
            .and_then(crate::capture::engine::runtime::audio::transcription::transcribe::Engine::parse)
            .unwrap_or_default();
        let menu = crate::capture::engine::runtime::audio::transcription::transcribe::menu_state(engine, hex.as_ref(), config.hex_model.as_deref());
        let command = crate::capture::engine::runtime::audio::transcription::command_transcriber::Program::load_from(&host.config_path())
            .ok()
            .map(|program| program.name());
        let policy = crate::capture::engine::bundle::retention::Policy::load_from(&host.config_path());
        let login = login_item();
        // Windows asks for no screen or accessibility grant, and has no cabled iPhone. Linux
        // asks for the input devices on Wayland only (the Accessibility row).
        let order: &[Permission] = if cfg!(target_os = "windows") {
            &[Permission::Microphone]
        } else if cfg!(target_os = "linux") {
            if wayland() {
                &[Permission::Microphone, Permission::Accessibility]
            } else {
                &[Permission::Microphone]
            }
        } else {
            &[
                Permission::ScreenRecording,
                Permission::Microphone,
                Permission::Accessibility,
                Permission::Camera,
                Permission::SpeechRecognition,
            ]
        };
        View {
            version: env!("APPARENT_VERSION_LABEL").to_owned(),
            hosted: crate::shared::handoff::experiment_enabled_in(&config, crate::connect::hosted::HOSTED_SHARING)
                .then(|| crate::connect::hosted::account_view_in(&host.config_path())),
            login: LoginView {
                on: matches!(login, LoginItem::On | LoginItem::NeedsApproval),
                available: login != LoginItem::Unavailable,
                needs_approval: login == LoginItem::NeedsApproval,
            },
            agent_demo_video: crate::shared::handoff::experiment_enabled_in(&config, "agentDemoVideo"),
            shortcuts: ShortcutKind::ALL
                .into_iter()
                .map(|kind| {
                    let error = self
                        .error
                        .as_ref()
                        .filter(|(k, _)| *k == kind)
                        .map(|(_, e)| e);
                    shortcut_view(
                        kind,
                        shortcuts.get(kind),
                        self.recording == Some(kind),
                        error,
                    )
                })
                .collect(),
            microphone: microphone_view(
                crate::capture::engine::runtime::audio::mic::choice_in(&host.config_path()).as_ref(),
                self.inputs.as_ref(),
                self.status(Permission::Microphone) == Status::Granted,
            ),
            lower_playback: config.lower_playback.unwrap_or(false),
            playback_level: config.recording_volume(),
            transcriber: TranscriberView {
                engine: match engine {
                    crate::capture::engine::runtime::audio::transcription::transcribe::Engine::Command if command.is_some() => "command",
                    _ if menu.hex_checked => "hex",
                    _ => "apple",
                },
                hex_title: menu.hex_title,
                hex_enabled: menu.hex_enabled,
                language: crate::shared::handoff::hex_language_from(&config),
                apple_locale: crate::shared::handoff::apple_locale_from(&config)
                    .unwrap_or_else(|| "auto".into()),
                models: menu
                    .models
                    .into_iter()
                    .map(|(id, title, chosen)| ModelView { id, title, chosen })
                    .collect(),
                command,
            },
            video_export: config.video_export.unwrap_or(false),
            retention: RetentionView {
                preset: policy.preset().map(preset_id),
                options: crate::capture::engine::bundle::retention::Preset::ALL
                    .into_iter()
                    .map(|p| OptionView {
                        id: preset_id(p),
                        label: p.label(),
                    })
                    .collect(),
                note: retention_note(&policy),
            },
            captures_size: self.captures_size.map(crate::capture::engine::bundle::retention::format_bytes),
            captures_dir: CapturesDirView::new(&config),
            permissions: order
                .iter()
                .filter(|p| p.asked(self.grants.as_ref()))
                .map(|&permission| PermissionView {
                    permission,
                    status: self.status(permission),
                    required: permission.required(),
                    requested: self.requested.contains(&permission),
                    relaunch: permission == Permission::ScreenRecording && self.screen_relaunch(),
                })
                .collect(),
            updates: updates_available(),
            update_check: self.update_check.clone(),
            update_channel: if config.update_channel.as_deref() == Some("main") {
                "main"
            } else {
                "stable"
            },
            main_updates: cfg!(target_os = "linux"),
            command_line: CommandLineView::new(
                &crate::composition::command_line::state(&host.command_line()),
                self.cli_error.as_deref(),
            ),
            skill_command: SKILL_COMMAND,
        }
    }
}

#[cfg(target_os = "linux")]
fn wayland() -> bool {
    crate::capture::desktop::linux::wayland_session()
}

#[cfg(not(target_os = "linux"))]
fn wayland() -> bool {
    false
}

/// Whether this copy can check for updates: Sparkle in Apparent.app; on Windows the releases
/// page always opens.
fn updates_available() -> bool {
    #[cfg(target_os = "macos")]
    return crate::capture::desktop::sparkle::available();
    #[cfg(target_os = "linux")]
    return std::env::var_os("APPIMAGE").is_some();
    #[cfg(target_os = "windows")]
    true
}

#[cfg(not(target_os = "linux"))]
fn check_for_updates() {
    #[cfg(target_os = "macos")]
    crate::capture::desktop::sparkle::check_for_updates();
    #[cfg(target_os = "windows")]
    let _ = crate::shared::handoff::open_path(Path::new(RELEASES_URL));
}

#[cfg(target_os = "windows")]
const RELEASES_URL: &str = "https://github.com/tryapparent/apparent/releases/latest";

// MARK: The live window

/// The real hotkeys and config file.
pub struct AppHost<'a> {
    pub telemetry: &'a crate::capture::engine::runtime::SharedCaptureTelemetry,
}

impl Host for AppHost<'_> {
    fn shortcuts(&self) -> Shortcuts {
        let (record, paste) = self.telemetry.shortcuts();
        Shortcuts { record, paste }
    }

    fn set_shortcuts(&mut self, shortcuts: &Shortcuts) -> Result<(), ShortcutKind> {
        crate::capture::engine::runtime::set_shortcuts(
            self.telemetry,
            shortcuts.record.clone(),
            shortcuts.paste.clone(),
        )?;
        note_paste_label(&shortcuts.paste.label);
        Ok(())
    }

    fn pause_shortcuts(&mut self, paused: bool) {
        crate::capture::engine::runtime::pause_shortcuts(self.telemetry, paused);
    }

    fn config_path(&self) -> PathBuf {
        crate::shared::handoff::default_config_path()
    }

    fn set_login_item(&mut self, on: bool) -> anyhow::Result<()> {
        set_login_item(on)
    }

    fn command_line(&self) -> crate::composition::command_line::Places {
        crate::composition::command_line::Places::system()
    }
}

/// From the window's helper process and its pollers.
#[derive(Clone, Debug)]
pub enum Event {
    Action(Action),
    Grants(Grants),
    CapturesSize(u64),
    Microphones(crate::capture::engine::runtime::audio::mic::Inputs),
    #[cfg(target_os = "linux")]
    UpdateChecked(Result<&'static str, String>),
    /// Something outside the window may have changed (Hex, the login item): redraw.
    Tick,
    Exited,
}

/// Where the window runs: the SwiftUI helper on macOS, a GPUI window in this process on Windows
/// (`settings_window`). Both take the same JSON messages and report the same `Action`s.
enum Transport {
    Helper {
        child: std::process::Child,
        stdin: Option<std::process::ChildStdin>,
    },
    InProcess(Option<flume::Sender<serde_json::Value>>),
}

/// The open Settings window.
pub struct Session {
    panel: Panel,
    transport: Transport,
    polling: std::sync::Arc<std::sync::atomic::AtomicBool>,
    events: std::sync::Arc<dyn Fn(Event) + Send + Sync>,
    last_view: String,
    _microphones: crate::capture::engine::runtime::audio::mic::Watch,
}

impl Session {
    pub fn open(
        pane: Option<Pane>,
        grants: Option<Grants>,
        events: impl Fn(Event) + Send + Sync + 'static,
    ) -> anyhow::Result<Self> {
        use std::sync::atomic::{AtomicBool, Ordering};

        let events: std::sync::Arc<dyn Fn(Event) + Send + Sync> = std::sync::Arc::new(events);
        let transport = if cfg!(not(target_os = "macos")) {
            let (sender, receiver) = flume::unbounded();
            crate::capture::desktop::settings::settings_window::request_open(receiver, pane, std::sync::Arc::clone(&events));
            Transport::InProcess(Some(sender))
        } else {
            Self::spawn_helper(pane, std::sync::Arc::clone(&events))?
        };

        let polling = std::sync::Arc::new(AtomicBool::new(true));
        let alive = std::sync::Arc::clone(&polling);
        let poll = std::sync::Arc::clone(&events);
        std::thread::spawn(move || {
            let mut n = 0_u32;
            while alive.load(Ordering::Acquire) {
                // Hex and the size on open (`hex::watch` keeps Hex fresh) and every 30 s; the
                // grants and the login item every second.
                if n % 30 == 0 {
                    crate::shared::hex::refresh_in_background();
                    let size =
                        crate::capture::engine::bundle::retention::captures_size(&crate::shared::handoff::default_captures_dir());
                    poll(Event::CapturesSize(size));
                }
                // macOS receives the app-wide permission refresh, including with this
                // window closed. Keep the existing local polling on other platforms.
                #[cfg(not(target_os = "macos"))]
                if let Some(grants) = crate::capture::desktop::settings::onboarding::read_grants()
                    && alive.load(Ordering::Acquire)
                {
                    poll(Event::Grants(grants));
                } else {
                    poll(Event::Tick);
                }
                #[cfg(target_os = "macos")]
                poll(Event::Tick);
                n += 1;
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        });

        let list = std::sync::Arc::clone(&events);
        let microphones =
            crate::capture::engine::runtime::audio::mic::Watch::start(move || list(Event::Microphones(crate::capture::engine::runtime::audio::mic::inputs())));

        let mut panel = Panel::default();
        if let Some(grants) = grants {
            panel.set_grants(grants);
        }
        Ok(Self {
            panel,
            transport,
            polling,
            events,
            last_view: String::new(),
            _microphones: microphones,
        })
    }

    /// `apparent-helper settings`, reading its actions off stdout.
    fn spawn_helper(
        pane: Option<Pane>,
        events: std::sync::Arc<dyn Fn(Event) + Send + Sync>,
    ) -> anyhow::Result<Transport> {
        use std::io::BufRead;
        use std::process::{Command, Stdio};

        let mut command = Command::new(crate::capture::desktop::helper::path()?);
        command.arg("settings");
        if let Some(serde_json::Value::String(pane)) = pane.map(serde_json::to_value).transpose()? {
            command.args(["--pane", &pane]);
        }
        let dev_icon = concat!(env!("CARGO_MANIFEST_DIR"), "/shared/assets/brand/apparent-a.png");
        if Path::new(dev_icon).is_file() {
            command.args(["--icon", dev_icon]);
        }
        let mut child = command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();

        std::thread::spawn(move || {
            if let Some(stdout) = stdout {
                for line in std::io::BufReader::new(stdout)
                    .lines()
                    .map_while(Result::ok)
                {
                    match serde_json::from_str::<Action>(&line) {
                        Ok(action) => events(Event::Action(action)),
                        Err(error) => {
                            crate::capture::engine::runtime::apparent_log(format!("settings: {error}: {line}"))
                        }
                    }
                }
            }
            events(Event::Exited);
        });
        Ok(Transport::Helper { child, stdin })
    }

    /// Feeds an event in, redraws, and returns what the app must do (`Relaunch`, `Tour`,
    /// `Close`); the rest is done here.
    pub fn event(&mut self, event: Event, host: &mut dyn Host) -> Effect {
        let effect = match event {
            Event::Action(action) => {
                crate::capture::engine::runtime::apparent_log(format!("settings: {action:?}"));
                self.panel.on(action, host)
            }
            Event::Grants(grants) => {
                self.panel.set_grants(grants);
                Effect::None
            }
            Event::CapturesSize(bytes) => {
                self.panel.set_captures_size(bytes);
                Effect::None
            }
            Event::Microphones(inputs) => {
                self.panel.set_inputs(inputs);
                Effect::None
            }
            #[cfg(target_os = "linux")]
            Event::UpdateChecked(result) => {
                self.panel.finish_update_check(result);
                Effect::None
            }
            Event::Tick => Effect::None,
            Event::Exited => {
                self.panel.on(Action::Close, host);
                return Effect::Close;
            }
        };
        let result = match effect {
            // Attributed to whichever process asks, so Apparent itself asks.
            Effect::Request(Permission::ScreenRecording) => {
                crate::capture::engine::runtime::request_screen_capture();
                Effect::None
            }
            Effect::Request(permission) => {
                self.send(&serde_json::json!({"type": "request", "permission": permission}));
                Effect::None
            }
            Effect::OpenSettings(permission) => {
                self.send(&serde_json::json!({"type": "open-settings", "permission": permission}));
                Effect::None
            }
            Effect::OpenLoginItems => {
                open_login_items_settings();
                Effect::None
            }
            Effect::ShowCaptures => {
                let dir = crate::shared::handoff::default_captures_dir();
                let _ = std::fs::create_dir_all(&dir);
                let _ = crate::shared::handoff::open_path(&dir);
                Effect::None
            }
            Effect::CheckForUpdates => {
                #[cfg(target_os = "linux")]
                {
                    let events = std::sync::Arc::clone(&self.events);
                    std::thread::spawn(move || {
                        let result = crate::capture::desktop::linux::update::check_cli().map_err(|error| format!("{error:#}"));
                        events(Event::UpdateChecked(result));
                    });
                }
                #[cfg(not(target_os = "linux"))]
                check_for_updates();
                Effect::None
            }
            Effect::RelaunchUpdate => {
                #[cfg(target_os = "linux")]
                {
                    match crate::capture::desktop::linux::update::relaunch_staged() {
                        Ok(()) => return Effect::QuitForUpdate,
                        Err(error) => {
                            crate::capture::engine::runtime::apparent_log(format!("Could not relaunch updated AppImage: {error:#}"));
                            self.panel.update_check.result = Some(UpdateResult::Failed);
                        }
                    }
                }
                Effect::None
            }
            Effect::Measure => {
                crate::capture::engine::bundle::retention::nudge();
                let events = std::sync::Arc::clone(&self.events);
                std::thread::spawn(move || {
                    let size =
                        crate::capture::engine::bundle::retention::captures_size(&crate::shared::handoff::default_captures_dir());
                    events(Event::CapturesSize(size));
                });
                Effect::None
            }
            Effect::SpeechModel => {
                crate::capture::engine::runtime::audio::transcription::speech_model::restart();
                Effect::None
            }
            other => other,
        };
        self.sync(host);
        result
    }

    pub fn show(&mut self, pane: Option<Pane>) {
        self.send(&serde_json::json!({"type": "show", "pane": pane}));
    }

    pub fn sync(&mut self, host: &dyn Host) {
        let mut view = serde_json::to_value(self.panel.view(host)).unwrap_or_default();
        view["type"] = "state".into();
        // Switched from Hex to Apple Speech: get its model now, not at the first capture.
        let speech = crate::capture::engine::runtime::audio::transcription::speech_model::current();
        if view["transcriber"]["engine"] == "apple"
            && speech.state == crate::capture::engine::runtime::audio::transcription::speech_model::State::Unknown
        {
            crate::capture::engine::runtime::audio::transcription::speech_model::ensure();
        }
        view["transcriber"]["speech"] = serde_json::to_value(speech).unwrap_or_default();
        let text = view.to_string();
        if text != self.last_view {
            self.send(&view);
            self.last_view = text;
        }
    }

    fn send(&mut self, message: &serde_json::Value) {
        use std::io::Write;
        match &mut self.transport {
            Transport::Helper { stdin, .. } => {
                if let Some(pipe) = stdin.as_mut()
                    && writeln!(pipe, "{message}")
                        .and_then(|()| pipe.flush())
                        .is_err()
                {
                    *stdin = None;
                }
            }
            Transport::InProcess(sender) => {
                if let Some(channel) = sender.as_ref()
                    && channel.send(message.clone()).is_err()
                {
                    *sender = None;
                }
            }
        }
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.polling
            .store(false, std::sync::atomic::Ordering::Release);
        self.send(&serde_json::json!({"type": "close"}));
        match &mut self.transport {
            Transport::Helper { child, stdin } => {
                *stdin = None;
                let _ = child.wait();
            }
            Transport::InProcess(sender) => *sender = None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Key codes: macOS `kVK_*` (ANSI), Windows virtual keys.
    /// Windows and Linux store virtual keys.
    const WIN: bool = cfg!(not(target_os = "macos"));
    const R: u32 = if WIN { 0x52 } else { 15 };
    const V: u32 = if WIN { 0x56 } else { 9 };
    const F1: u32 = if WIN { 0x70 } else { 122 };
    const F5: u32 = if WIN { 0x74 } else { 96 };
    const ESC: u32 = if WIN { 0x1B } else { 53 };
    const SPACE: u32 = if WIN { 0x20 } else { 49 };
    const SLASH: u32 = if WIN { 0xBF } else { 44 };
    const OPT_SHIFT: u32 = CARBON_OPTION_KEY | CARBON_SHIFT_KEY;
    const CTRL_OPT: u32 = CARBON_CONTROL_KEY | CARBON_OPTION_KEY;

    /// A shortcut's label on this platform: `⌃⌥R` on macOS, `Ctrl+Alt+R` on Windows.
    fn label(modifiers: u32, key: &str) -> String {
        crate::capture::engine::runtime::shortcut_label(modifiers, key)
    }

    struct Fake {
        shortcuts: Shortcuts,
        path: PathBuf,
        paused: Vec<bool>,
        refuse: Option<String>,
        refuse_login: bool,
        login_changes: Vec<bool>,
        cli: crate::composition::command_line::Places,
    }

    impl Host for Fake {
        fn shortcuts(&self) -> Shortcuts {
            self.shortcuts.clone()
        }
        fn set_shortcuts(&mut self, shortcuts: &Shortcuts) -> Result<(), ShortcutKind> {
            for kind in ShortcutKind::ALL {
                if self.refuse.as_deref() == Some(shortcuts.get(kind).label.as_str()) {
                    return Err(kind);
                }
            }
            self.shortcuts = shortcuts.clone();
            Ok(())
        }
        fn pause_shortcuts(&mut self, paused: bool) {
            self.paused.push(paused);
        }
        fn config_path(&self) -> PathBuf {
            self.path.clone()
        }
        fn set_login_item(&mut self, on: bool) -> anyhow::Result<()> {
            self.login_changes.push(on);
            anyhow::ensure!(!self.refuse_login, "login registration refused");
            Ok(())
        }
        fn command_line(&self) -> crate::composition::command_line::Places {
            self.cli.clone()
        }
    }

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "apparent-settings-{name}-{}-{}",
                std::process::id(),
                crate::shared::clock::now_epoch_ms()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn pressed(key_code: u32, modifiers: u32, key: &str) -> Pressed {
        Pressed {
            key_code,
            modifiers,
            key: key.to_owned(),
            reserved: false,
        }
    }

    fn fake(dir: &TempDir) -> Fake {
        Fake {
            shortcuts: Shortcuts::default(),
            path: dir.0.join("config.json"),
            paused: Vec::new(),
            refuse: None,
            refuse_login: false,
            login_changes: Vec::new(),
            cli: crate::composition::command_line::Places {
                target: None,
                install: vec![dir.0.join("bin")],
                also: Vec::new(),
            },
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn available_update_accepts_now_and_later_actions() {
        let dir = TempDir::new("update-choices");
        let mut host = fake(&dir);
        let mut panel = Panel::default();
        assert_eq!(panel.on(Action::UpdateNow, &mut host), Effect::None);
        panel.finish_update_check(Ok("installed; restart Apparent to use it"));
        assert_eq!(panel.on(serde_json::from_str("{\"action\":\"update-later\"}").unwrap(), &mut host), Effect::None);
        assert!(panel.view(&host).update_check.result.is_none());
        assert_eq!(panel.on(Action::UpdateNow, &mut host), Effect::None);
        panel.finish_update_check(Ok("already installed; restart Apparent to use it"));
        assert_eq!(panel.on(serde_json::from_str("{\"action\":\"update-now\"}").unwrap(), &mut host), Effect::RelaunchUpdate);
        panel.finish_update_check(Err("network failure".into()));
        assert_eq!(panel.on(Action::UpdateNow, &mut host), Effect::None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn manual_update_check_serializes_distinct_results_and_preserves_progress() {
        let dir = TempDir::new("update-results");
        let mut host = fake(&dir);
        let mut panel = Panel::default();
        for (response, expected) in [
            (Ok("up to date"), "current"),
            (Ok("installed; restart Apparent to use it"), "available"),
            (Ok("already installed; restart Apparent to use it"), "available"),
            (Err("feed unavailable: private diagnostic".into()), "failed"),
            (Ok("unrecognized worker response"), "failed"),
        ] {
            assert_eq!(panel.on(Action::CheckForUpdates, &mut host), Effect::CheckForUpdates);
            let progress = serde_json::to_value(panel.view(&host)).unwrap();
            assert_eq!(progress["updateCheck"]["checking"], true);
            assert!(progress["updateCheck"]["result"].is_null());
            assert_eq!(panel.on(Action::CheckForUpdates, &mut host), Effect::None);
            panel.finish_update_check(response);
            let finished = serde_json::to_value(panel.view(&host)).unwrap();
            assert_eq!(finished["updateCheck"], serde_json::json!({
                "checking": false, "result": expected
            }), "the public result must identify the outcome without diagnostics or extra copy");
            assert_eq!(panel.on(Action::DismissUpdateCheck, &mut host), Effect::None);
            let dismissed = serde_json::to_value(panel.view(&host)).unwrap();
            assert_eq!(dismissed["updateCheck"], serde_json::json!({
                "checking": false, "result": null
            }));
        }
    }

    #[test]
    fn hosted_sharing_settings_are_off_by_default_and_have_explicit_account_actions() {
        let dir = TempDir::new("hosted-sharing-settings");
        let host = fake(&dir);
        let panel = Panel::default();
        assert!(serde_json::to_value(panel.view(&host)).unwrap()["hosted"].is_null());
        std::fs::write(&host.path, r#"{"experiments":{"hostedSharing":true},"cloud":{"server":"https://synthetic.example","enabled":false,"uploadConsent":false}}"#).unwrap();
        let view = serde_json::to_value(panel.view(&host)).unwrap();
        assert_eq!(view["hosted"]["server"], "https://synthetic.example");
        assert!(serde_json::from_value::<Action>(serde_json::json!({"action":"manage-shared-links"})).is_ok());
        assert!(serde_json::from_value::<Action>(serde_json::json!({"action":"connect-hosted"})).is_ok());
    }

    #[test]
    fn login_preference_is_saved_after_registration_succeeds() {
        let dir = TempDir::new("login-preference");
        let mut host = fake(&dir);
        let mut panel = Panel::default();
        panel.on(Action::Set { key: "launchAtLogin".into(), value: true.into() }, &mut host);
        assert_eq!(read_json(&host.path)["launchAtLogin"], true);
        assert_eq!(crate::shared::handoff::load_config_from(&host.path).launch_at_login, Some(true));
        host.refuse_login = true;
        panel.on(Action::Set { key: "launchAtLogin".into(), value: false.into() }, &mut host);
        assert_eq!(read_json(&host.path)["launchAtLogin"], true, "a failed OS change must not claim success in the file");
    }

    #[test]
    fn agent_demo_video_switch_round_trips_without_losing_config() {
        let dir = TempDir::new("agent-demo-video");
        let mut host = fake(&dir);
        let mut panel = Panel::default();
        assert!(panel.view(&host).agent_demo_video, "missing config defaults on");
        std::fs::write(&host.path, r#"{"ingestToken":"private","experiments":{"other":true}}"#).unwrap();
        assert!(panel.view(&host).agent_demo_video, "missing flag defaults on");
        panel.on(Action::Set { key: "agentDemoVideo".into(), value: true.into() }, &mut host);
        assert!(panel.view(&host).agent_demo_video);
        assert_eq!(read_json(&host.path)["experiments"]["other"], true);
        assert_eq!(read_json(&host.path)["ingestToken"], "private");
        panel.on(Action::Set { key: "agentDemoVideo".into(), value: false.into() }, &mut host);
        assert!(!panel.view(&host).agent_demo_video);
        assert_eq!(read_json(&host.path)["experiments"]["agentDemoVideo"], false);
    }

    #[test]
    fn invalid_config_blocks_login_registration() {
        let dir = TempDir::new("invalid-login-config");
        let mut host = fake(&dir);
        std::fs::write(&host.path, b"{").unwrap();
        let mut panel = Panel::default();
        panel.on(Action::Set { key: "launchAtLogin".into(), value: true.into() }, &mut host);
        assert_eq!(std::fs::read(&host.path).unwrap(), b"{");
        assert!(host.login_changes.is_empty());
    }

    #[test]
    fn external_login_preference_applies_only_when_it_differs_from_the_os() {
        let dir = TempDir::new("login-reconcile");
        let path = dir.0.join("config.json");
        let mut applied = Vec::new();
        reconcile_login_item_in(&path, LoginItem::Off, false, |on| { applied.push(on); Ok(()) }).unwrap();
        assert!(applied.is_empty(), "an absent preference must not change existing OS state");
        crate::shared::handoff::set_config_value_in(&path, "launchAtLogin", true.into()).unwrap();
        reconcile_login_item_in(&path, LoginItem::Off, false, |on| { applied.push(on); Ok(()) }).unwrap();
        reconcile_login_item_in(&path, LoginItem::On, false, |on| { applied.push(on); Ok(()) }).unwrap();
        reconcile_login_item_in(&path, LoginItem::On, true, |on| { applied.push(on); Ok(()) }).unwrap();
        reconcile_login_item_in(&path, LoginItem::NeedsApproval, true, |on| { applied.push(on); Ok(()) }).unwrap();
        assert_eq!(applied, [true, true]);
        crate::shared::handoff::set_config_value_in(&path, "launchAtLogin", false.into()).unwrap();
        reconcile_login_item_in(&path, LoginItem::On, false, |on| { applied.push(on); Ok(()) }).unwrap();
        assert_eq!(applied, [true, true, false]);
    }

    #[test]
    fn a_shortcut_needs_a_real_modifier_and_a_free_key() {
        let paste = ShortcutConfig::default_paste();
        let ok = validate(
            ShortcutKind::Record,
            &pressed(R, CTRL_OPT, "r"),
            &paste,
        )
        .unwrap();
        assert_eq!(ok.label, label(CTRL_OPT, "R"));
        assert_eq!(ok.key_label(), "R");
        // Labels follow the platform's order whatever order the bits come in.
        let ok = validate(
            ShortcutKind::Record,
            &pressed(
                R,
                CARBON_CMD_KEY | CARBON_SHIFT_KEY | CARBON_CONTROL_KEY | CARBON_OPTION_KEY,
                "r",
            ),
            &paste,
        )
        .unwrap();
        assert_eq!(
            ok.label,
            if cfg!(target_os = "linux") {
                "Ctrl+Super+Alt+Shift+R"
            } else if WIN {
                "Ctrl+Win+Alt+Shift+R"
            } else {
                "⌃⌥⇧⌘R"
            }
        );
        let no = |p: Pressed| validate(ShortcutKind::Record, &p, &paste).unwrap_err();
        assert_eq!(no(pressed(R, 0, "r")), Rejection::NoModifier);
        assert_eq!(no(pressed(R, CARBON_SHIFT_KEY, "r")), Rejection::NoModifier);
        // A function key may stand alone.
        assert_eq!(
            validate(ShortcutKind::Record, &pressed(F5, 0, ""), &paste)
                .unwrap()
                .label,
            "F5"
        );
        assert_eq!(
            no(pressed(ESC, CARBON_OPTION_KEY, "\u{1b}")),
            Rejection::UnsupportedKey
        );
        let mut system = pressed(R, CTRL_OPT, "r");
        system.reserved = true;
        assert_eq!(no(system), Rejection::Reserved(label(CTRL_OPT, "R")));
        // Apparent's other shortcut.
        assert_eq!(
            no(pressed(V, OPT_SHIFT, "v")),
            Rejection::Taken(ShortcutKind::Paste)
        );
        assert_eq!(
            Rejection::Taken(ShortcutKind::Paste).message(),
            "Paste last capture already uses this."
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn apps_and_macos_keep_their_shortcuts() {
        let paste = ShortcutConfig::default_paste();
        let no = |p: Pressed| validate(ShortcutKind::Record, &p, &paste).unwrap_err();
        // ⌘ and one key belongs to apps; macOS's own combinations are refused.
        assert_eq!(
            no(pressed(R, CARBON_CMD_KEY, "r")),
            Rejection::Reserved("⌘R".into())
        );
        assert_eq!(
            no(pressed(R, CARBON_CMD_KEY, "r")).message(),
            "Apps use ⌘R. Add ⌥ or ⌃."
        );
        assert_eq!(
            no(pressed(21, CARBON_CMD_KEY | CARBON_SHIFT_KEY, "4")),
            Rejection::Reserved("⇧⌘4".into())
        );
        assert_eq!(
            no(pressed(SPACE, CARBON_CONTROL_KEY, " ")).message(),
            "macOS uses ⌃Space."
        );
        assert_eq!(
            Rejection::NoModifier.message(),
            "Hold ⌘, ⌥ or ⌃ with the key."
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn apps_and_windows_keep_their_shortcuts() {
        let paste = ShortcutConfig::default_paste();
        let no = |p: Pressed| validate(ShortcutKind::Record, &p, &paste).unwrap_err();
        // Ctrl and one key belongs to apps; Win and a key, and Alt+Tab and friends, to Windows.
        assert_eq!(
            no(pressed(R, CARBON_CONTROL_KEY, "r")),
            Rejection::Reserved("Ctrl+R".into())
        );
        assert_eq!(
            no(pressed(R, CARBON_CONTROL_KEY | CARBON_SHIFT_KEY, "r")).message(),
            "Apps use Ctrl+Shift+R. Add Alt."
        );
        assert_eq!(
            no(pressed(R, CARBON_CMD_KEY, "r")).message(),
            "Windows uses Win+R."
        );
        assert_eq!(
            no(pressed(0x53, CARBON_CMD_KEY | CARBON_SHIFT_KEY, "s")).message(),
            "Windows uses Win+Shift+S."
        );
        assert_eq!(
            no(pressed(0x73, CARBON_OPTION_KEY, "")).message(),
            "Windows uses Alt+F4."
        );
        assert_eq!(
            no(pressed(SPACE, CARBON_OPTION_KEY, " ")),
            Rejection::Reserved("Alt+Space".into())
        );
        // Win with Ctrl or Alt is free (Ctrl+Win+Alt+S is one of the default's aliases).
        assert!(
            validate(
                ShortcutKind::Record,
                &pressed(R, CARBON_CMD_KEY | CARBON_OPTION_KEY, "r"),
                &paste
            )
            .is_ok()
        );
        assert_eq!(
            Rejection::NoModifier.message(),
            "Hold Ctrl, Alt or Win with the key."
        );
    }

    #[test]
    fn keys_are_named_for_the_layout_in_use() {
        assert_eq!(
            key_label(41, "s").as_deref(),
            Some("S"),
            "the typed letter names the key (Dvorak S sits on ANSI ;)"
        );
        assert_eq!(key_label(SPACE, " ").as_deref(), Some("Space"));
        assert_eq!(key_label(F1, "").as_deref(), Some("F1"));
        assert_eq!(key_label(SLASH, "/").as_deref(), Some("/"));
        assert_eq!(key_label(ESC, "\u{1b}"), None);
        assert_eq!(key_label(0, ""), None);
    }

    #[test]
    fn shortcuts_round_trip_through_the_config_keeping_other_keys_and_0600() {
        let dir = TempDir::new("round-trip");
        let path = dir.0.join("config.json");
        let legacy = dir.0.join("legacy.json");
        std::fs::write(&path, r#"{"ingestToken":"secret","autoSend":false}"#).unwrap();
        assert_eq!(
            Shortcuts::load_from(&path, &legacy),
            (Shortcuts::default(), false)
        );
        let custom = ShortcutConfig {
            key_code: R,
            carbon_modifiers: CARBON_CONTROL_KEY | CARBON_OPTION_KEY,
            label: "⌃⌥R".into(),
        };
        save_shortcut_in(&path, ShortcutKind::Record, &custom).unwrap();
        let (loaded, _) = Shortcuts::load_from(&path, &legacy);
        assert_eq!(loaded.record, custom);
        assert_eq!(loaded.paste, ShortcutConfig::default_paste());
        let config = read_json(&path);
        assert_eq!(config["ingestToken"], "secret");
        assert_eq!(config["autoSend"], false);
        assert_eq!(config["shortcuts"]["record"]["label"], "⌃⌥R");
        assert!(
            config["shortcuts"].get("paste").is_none(),
            "defaults aren't written"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // Back to the default removes it.
        save_shortcut_in(
            &path,
            ShortcutKind::Record,
            &ShortcutConfig::default_record(),
        )
        .unwrap();
        assert!(read_json(&path)["shortcuts"].get("record").is_none());
        assert_eq!(read_json(&path)["ingestToken"], "secret");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn copilot_record_shortcut_from_config_is_the_only_record_shortcut_in_settings() {
        let dir = TempDir::new("copilot-record");
        let mut host = fake(&dir);
        let cfg = ShortcutConfig {
            key_code: 201,
            carbon_modifiers: CARBON_CMD_KEY | CARBON_SHIFT_KEY,
            label: "Super+Shift+Copilot".into(),
        };
        save_shortcut_in(&host.path, ShortcutKind::Record, &cfg).unwrap();
        host.shortcuts = Shortcuts::load_from(&host.path, &dir.0.join("legacy.json")).0;
        let view = Panel::default().view(&host);
        let record: Vec<_> = view.shortcuts.iter().filter(|s| s.kind == ShortcutKind::Record).collect();
        assert_eq!(record.len(), 1);
        assert_eq!(record[0].label, "Super+Shift+Copilot");
        assert_eq!(record[0].key, "Copilot");
        assert!(record[0].command && record[0].shift);
        assert!(!record[0].is_default);
    }

    #[test]
    fn a_custom_shortcut_from_0_3_moves_across_once() {
        let dir = TempDir::new("legacy");
        let path = dir.0.join("config.json");
        let legacy = dir.0.join("legacy.json");
        // 0.3.x wrote its default too: that stays the default (on the active layout's S).
        std::fs::write(
            &legacy,
            r#"{"key_code":41,"carbon_modifiers":2560,"label":"⌥⇧S"}"#,
        )
        .unwrap();
        assert_eq!(
            Shortcuts::load_from(&path, &legacy),
            (Shortcuts::default(), false)
        );
        std::fs::write(
            &legacy,
            r#"{"key_code":15,"carbon_modifiers":6144,"label":"⌃⌥R"}"#,
        )
        .unwrap();
        let (loaded, migrate) = Shortcuts::load_from(&path, &legacy);
        assert!(migrate);
        assert_eq!(loaded.record.label, "⌃⌥R");
        // Once the new config has shortcuts, the old file is ignored.
        save_shortcut_in(
            &path,
            ShortcutKind::Record,
            &ShortcutConfig::default_record(),
        )
        .unwrap();
        assert_eq!(
            Shortcuts::load_from(&path, &legacy),
            (Shortcuts::default(), false)
        );
    }

    #[test]
    fn recording_pauses_the_hotkeys_and_a_good_combo_is_saved() {
        let dir = TempDir::new("record");
        let mut host = fake(&dir);
        let mut panel = Panel::default();
        panel.on(
            Action::RecordShortcut {
                kind: ShortcutKind::Record,
            },
            &mut host,
        );
        assert_eq!(panel.recording(), Some(ShortcutKind::Record));
        assert_eq!(host.paused, [true]);
        // A key for the other recorder does nothing.
        panel.on(
            Action::Shortcut {
                kind: ShortcutKind::Paste,
                pressed: pressed(R, CARBON_CONTROL_KEY | CARBON_OPTION_KEY, "r"),
            },
            &mut host,
        );
        assert_eq!(host.shortcuts, Shortcuts::default());
        panel.on(
            Action::Shortcut {
                kind: ShortcutKind::Record,
                pressed: pressed(R, CARBON_CONTROL_KEY | CARBON_OPTION_KEY, "r"),
            },
            &mut host,
        );
        assert_eq!(host.paused, [true, false]);
        assert_eq!(panel.recording(), None);
        assert_eq!(host.shortcuts.record.label, label(CTRL_OPT, "R"));
        assert_eq!(
            read_json(&host.path)["shortcuts"]["record"]["label"],
            label(CTRL_OPT, "R")
        );
        let view = panel.view(&host);
        let record = &view.shortcuts[0];
        assert_eq!(
            (
                record.key.as_str(),
                record.control,
                record.option,
                record.shift
            ),
            ("R", true, true, false)
        );
        assert!(!record.is_default && record.error.is_none());
        // Reset puts the default back and removes it from the config.
        panel.on(
            Action::ResetShortcut {
                kind: ShortcutKind::Record,
            },
            &mut host,
        );
        assert!(host.shortcuts.record.is_default_for(ShortcutKind::Record));
        assert!(read_json(&host.path)["shortcuts"].get("record").is_none());
    }

    #[test]
    fn a_bad_or_refused_combo_keeps_the_old_one_and_says_why() {
        let dir = TempDir::new("refused");
        let mut host = fake(&dir);
        let mut panel = Panel::default();
        let press = |panel: &mut Panel, host: &mut Fake, p: Pressed| {
            panel.on(
                Action::RecordShortcut {
                    kind: ShortcutKind::Paste,
                },
                host,
            );
            panel.on(
                Action::Shortcut {
                    kind: ShortcutKind::Paste,
                    pressed: p,
                },
                host,
            );
        };
        press(&mut panel, &mut host, pressed(R, CARBON_SHIFT_KEY, "r"));
        assert_eq!(host.shortcuts, Shortcuts::default());
        assert_eq!(
            panel.view(&host).shortcuts[1].error,
            Some(Rejection::NoModifier.message())
        );
        host.refuse = Some(label(CTRL_OPT, "R"));
        press(
            &mut panel,
            &mut host,
            pressed(R, CARBON_CONTROL_KEY | CARBON_OPTION_KEY, "r"),
        );
        assert_eq!(host.shortcuts, Shortcuts::default());
        assert_eq!(
            panel.view(&host).shortcuts[1].error.as_deref(),
            Some("Another app has this shortcut.")
        );
        assert!(!dir.0.join("config.json").exists(), "nothing saved");
        // Esc cancels and resumes the hotkeys; the old message goes when recording starts again.
        panel.on(
            Action::RecordShortcut {
                kind: ShortcutKind::Paste,
            },
            &mut host,
        );
        assert!(panel.view(&host).shortcuts[1].error.is_none());
        panel.on(Action::CancelShortcut, &mut host);
        assert_eq!(host.paused.last(), Some(&false));
        assert_eq!(panel.recording(), None);
        // Closing the window while recording resumes them too.
        panel.on(
            Action::RecordShortcut {
                kind: ShortcutKind::Record,
            },
            &mut host,
        );
        assert_eq!(panel.on(Action::Close, &mut host), Effect::Close);
        assert_eq!(host.paused.last(), Some(&false));
    }

    #[test]
    fn the_paste_shortcut_can_be_turned_off_and_back_on() {
        let dir = TempDir::new("clear");
        let mut host = fake(&dir);
        let mut panel = Panel::default();
        std::fs::write(&host.path, r#"{"ingestToken":"secret"}"#).unwrap();
        panel.on(
            Action::ClearShortcut {
                kind: ShortcutKind::Paste,
            },
            &mut host,
        );
        assert!(host.shortcuts.paste.is_unset());
        assert!(host.shortcuts.record.is_default_for(ShortcutKind::Record));
        let config = read_json(&host.path);
        assert!(config["shortcuts"]["paste"].is_null());
        assert!(config["shortcuts"].get("paste").is_some(), "stored as null");
        assert_eq!(config["ingestToken"], "secret");
        let (loaded, _) = Shortcuts::load_from(&host.path, &dir.0.join("legacy.json"));
        assert!(loaded.paste.is_unset());
        assert!(loaded.record.is_default_for(ShortcutKind::Record));

        let view = panel.view(&host);
        let paste = &view.shortcuts[1];
        assert_eq!((paste.label.as_str(), paste.key.as_str()), ("", ""));
        assert!(paste.clearable && !paste.is_default);
        assert!(!(paste.shift || paste.control || paste.option || paste.command));
        assert!(!view.shortcuts[0].clearable);

        // Nothing is registered for it and no key press matches it.
        assert!(crate::capture::engine::runtime::hotkey_combos(ShortcutKind::Paste, &loaded.paste).is_empty());
        assert!(!crate::capture::engine::runtime::shortcut_matches(
            ShortcutKind::Paste,
            &loaded.paste,
            V as u16,
            V as u16,
            false,
            false,
            true,
            true,
        ));

        // With paste off, any combination is free for Record, and Reset brings ⌥⇧V back.
        let free = validate(
            ShortcutKind::Record,
            &pressed(R, CARBON_CONTROL_KEY | CARBON_OPTION_KEY, "r"),
            &host.shortcuts.paste,
        );
        assert_eq!(free.map(|c| c.label), Ok(label(CTRL_OPT, "R")));
        panel.on(
            Action::ResetShortcut {
                kind: ShortcutKind::Paste,
            },
            &mut host,
        );
        assert!(host.shortcuts.paste.is_default_for(ShortcutKind::Paste));
        assert!(read_json(&host.path)["shortcuts"].get("paste").is_none());
    }

    #[test]
    fn delete_while_recording_turns_paste_off_but_never_record() {
        let dir = TempDir::new("clear-delete");
        let mut host = fake(&dir);
        let mut panel = Panel::default();
        let delete = if WIN { 0x08 } else { 51 };
        let press = |panel: &mut Panel, host: &mut Fake, kind: ShortcutKind, p: Pressed| {
            panel.on(Action::RecordShortcut { kind }, host);
            panel.on(Action::Shortcut { kind, pressed: p }, host);
        };
        press(
            &mut panel,
            &mut host,
            ShortcutKind::Record,
            pressed(delete, 0, ""),
        );
        assert_eq!(host.shortcuts, Shortcuts::default());
        assert_eq!(
            panel.view(&host).shortcuts[0].error,
            Some(Rejection::NoModifier.message())
        );
        panel.on(
            Action::ClearShortcut {
                kind: ShortcutKind::Record,
            },
            &mut host,
        );
        assert_eq!(host.shortcuts, Shortcuts::default());
        assert!(!host.path.exists(), "nothing saved");
        assert!(
            save_shortcut_in(&host.path, ShortcutKind::Record, &ShortcutConfig::unset()).is_err()
        );
        // A `null` record shortcut written by hand is read as the default.
        std::fs::write(&host.path, r#"{"shortcuts":{"record":null}}"#).unwrap();
        let (loaded, _) = Shortcuts::load_from(&host.path, &dir.0.join("legacy.json"));
        assert_eq!(loaded, Shortcuts::default());
        std::fs::remove_file(&host.path).unwrap();

        press(
            &mut panel,
            &mut host,
            ShortcutKind::Paste,
            pressed(delete, CARBON_OPTION_KEY, ""),
        );
        assert!(!host.shortcuts.paste.is_unset(), "⌥⌫ is a combination");
        press(
            &mut panel,
            &mut host,
            ShortcutKind::Paste,
            pressed(delete, 0, ""),
        );
        assert!(host.shortcuts.paste.is_unset());
        assert_eq!(panel.recording(), None);
        assert_eq!(host.paused.last(), Some(&false));
        assert!(panel.view(&host).shortcuts[1].error.is_none());
    }

    #[test]
    fn toggles_and_popups_write_their_keys() {
        let dir = TempDir::new("set");
        let mut host = fake(&dir);
        std::fs::write(&host.path, r#"{"ingestToken":"secret"}"#).unwrap();
        let mut panel = Panel::default();
        let set = |panel: &mut Panel, host: &mut Fake, key: &str, value: serde_json::Value| {
            panel.on(
                Action::Set {
                    key: key.into(),
                    value,
                },
                host,
            )
        };
        #[cfg(target_os = "linux")]
        assert!(panel.view(&host).main_updates, "Linux must show the channel choice");
        assert_eq!(serde_json::to_value(panel.view(&host)).unwrap()["updateChannel"], "stable");
        set(&mut panel, &mut host, "updateChannel", "main".into());
        assert_eq!(serde_json::to_value(panel.view(&host)).unwrap()["updateChannel"], "main");
        set(&mut panel, &mut host, "updateChannel", "untrusted".into());
        assert_eq!(serde_json::to_value(panel.view(&host)).unwrap()["updateChannel"], "main");
        assert!(!panel.view(&host).lower_playback);
        assert_eq!(panel.view(&host).playback_level, 20);
        set(&mut panel, &mut host, "lowerPlayback", true.into());
        set(&mut panel, &mut host, "playbackLevel", 35.into());
        set(&mut panel, &mut host, "playbackLevel", 101.into());
        set(&mut panel, &mut host, "playbackLevel", (-1).into());
        set(&mut panel, &mut host, "playbackLevel", 0.5.into());
        assert!(Panel::default().view(&host).lower_playback);
        assert_eq!(Panel::default().view(&host).playback_level, 35);
        set(&mut panel, &mut host, "videoExport", true.into());
        set(&mut panel, &mut host, "transcriber", "hex".into());
        assert_eq!(
            set(&mut panel, &mut host, "retention", "month".into()),
            Effect::Measure
        );
        set(&mut panel, &mut host, "transcriber", "nonsense".into());
        let view = panel.view(&host);
        assert!(view.video_export);
        assert!(serde_json::to_value(&view).unwrap().get("autoSend").is_none());
        set(&mut panel, &mut host, "autoSend", true.into());
        assert!(read_json(&host.path).get("autoSend").is_none());
        assert_eq!(view.retention.preset, Some("month"));
        assert_eq!(
            view.retention.note,
            format!(
                "Video and audio go to the {} after 30 days, the rest of each capture after 90 days.",
                if cfg!(target_os = "windows") { "Recycle Bin" } else { "Trash" }
            )
        );
        let config = read_json(&host.path);
        assert_eq!(config["transcriber"], "hex");
        assert_eq!(config["ingestToken"], "secret");
        set(&mut panel, &mut host, "hexModel", "parakeet".into());
        assert_eq!(read_json(&host.path)["hexModel"], "parakeet");
        assert_eq!(panel.view(&host).transcriber.language, "en");
        set(&mut panel, &mut host, "hexLanguage", " NL ".into());
        assert_eq!(panel.view(&host).transcriber.language, "nl");
        set(
            &mut panel,
            &mut host,
            "hexLanguage",
            "nl&language=en".into(),
        );
        assert_eq!(read_json(&host.path)["hexLanguage"], "nl");
        set(&mut panel, &mut host, "hexLanguage", "auto".into());
        assert_eq!(panel.view(&host).transcriber.language, "auto");
        set(&mut panel, &mut host, "hexLanguage", "".into());
        assert_eq!(panel.view(&host).transcriber.language, "en");
        assert_eq!(panel.view(&host).transcriber.apple_locale, "auto");
        assert_eq!(
            set(&mut panel, &mut host, "appleLocale", "zh-TW".into()),
            Effect::SpeechModel
        );
        assert_eq!(read_json(&host.path)["appleLocale"], "zh-TW");
        assert_eq!(panel.view(&host).transcriber.apple_locale, "zh-TW");
        assert_eq!(
            set(&mut panel, &mut host, "appleLocale", "zh-TW --install".into()),
            Effect::None
        );
        assert_eq!(read_json(&host.path)["appleLocale"], "zh-TW");
        assert_eq!(
            set(&mut panel, &mut host, "appleLocale", "auto".into()),
            Effect::SpeechModel
        );
        assert!(read_json(&host.path).get("appleLocale").is_none());
        assert_eq!(panel.view(&host).transcriber.apple_locale, "auto");
        set(&mut panel, &mut host, "retention", "forever".into());
        // "Forever" says it all; no caption.
        assert_eq!(panel.view(&host).retention.note, "");
    }

    #[test]
    fn the_captures_folder_can_be_chosen_and_reset() {
        let dir = TempDir::new("captures-dir");
        let mut host = fake(&dir);
        let mut panel = Panel::default();
        let set = |panel: &mut Panel, host: &mut Fake, value: serde_json::Value| {
            panel.on(
                Action::Set {
                    key: "capturesDir".into(),
                    value,
                },
                host,
            )
        };
        let chosen = dir.0.join("My Captures").display().to_string();

        assert_eq!(
            set(&mut panel, &mut host, chosen.clone().into()),
            Effect::Measure
        );
        assert_eq!(read_json(&host.path)["capturesDir"], chosen.as_str());
        // A relative folder is turned down and the chosen one kept.
        assert_eq!(set(&mut panel, &mut host, "captures".into()), Effect::None);
        assert_eq!(read_json(&host.path)["capturesDir"], chosen.as_str());

        let view = panel.view(&host).captures_dir;
        assert!(view.custom);
        assert_eq!(
            view.path,
            crate::composition::command_line::tilde(&dir.0.join("My Captures"))
        );

        assert_eq!(set(&mut panel, &mut host, "".into()), Effect::Measure);
        assert!(read_json(&host.path).get("capturesDir").is_none());
        let view = panel.view(&host).captures_dir;
        assert!(!view.custom);
        assert_eq!(
            view.path,
            crate::composition::command_line::tilde(&crate::shared::handoff::builtin_captures_dir())
        );
    }

    #[test]
    fn the_microphone_popup_follows_what_is_plugged_in() {
        use crate::capture::engine::runtime::audio::mic::{Input, Inputs};
        let dir = TempDir::new("mic");
        let mut host = fake(&dir);
        let mut panel = Panel::default();
        let built_in = Input {
            id: "BuiltInMicrophoneDevice".into(),
            name: "MacBook Pro Microphone".into(),
        };
        let shure = Input {
            id: "usb-1".into(),
            name: "Shure MV7".into(),
        };
        let both = Inputs {
            devices: vec![built_in.clone(), shure.clone()],
            default: Some(built_in.id.clone()),
        };
        let pick = |panel: &mut Panel, host: &mut Fake, id: &str| {
            panel.on(
                Action::Set {
                    key: "microphone".into(),
                    value: id.into(),
                },
                host,
            )
        };

        let mic = panel.view(&host).microphone;
        assert_eq!(
            (mic.selected.as_str(), mic.value.as_str()),
            ("default", "System default")
        );
        assert_eq!((mic.note, mic.meter), (None, None), "nothing listed yet");

        panel.set_inputs(both.clone());
        let mic = panel.view(&host).microphone;
        let titles: Vec<_> = mic.options.iter().map(|o| o.title.as_str()).collect();
        assert_eq!(
            titles,
            [
                "System default (MacBook Pro Microphone)",
                "",
                "MacBook Pro Microphone",
                "Shure MV7"
            ]
        );
        assert_eq!(mic.options[1].id, "-");
        assert_eq!(mic.meter, None, "no level before Microphone is allowed");

        let mut grants = Grants::default();
        grants
            .statuses
            .insert(Permission::Microphone, Status::Granted);
        panel.set_grants(grants);
        assert_eq!(
            panel.view(&host).microphone.meter.as_deref(),
            Some("BuiltInMicrophoneDevice")
        );

        pick(&mut panel, &mut host, "usb-1");
        assert_eq!(
            read_json(&host.path)["microphone"],
            serde_json::json!({"id": "usb-1", "name": "Shure MV7"})
        );
        let mic = panel.view(&host).microphone;
        assert_eq!(
            (mic.selected.as_str(), mic.value.as_str()),
            ("usb-1", "Shure MV7")
        );
        assert_eq!(mic.meter.as_deref(), Some("usb-1"));

        // Unplugged: still chosen, said once in the caption, and the default stands in.
        panel.set_inputs(Inputs {
            devices: vec![built_in.clone()],
            default: Some(built_in.id.clone()),
        });
        let mic = panel.view(&host).microphone;
        assert_eq!(mic.value, "Shure MV7");
        assert_eq!(
            mic.note.as_deref(),
            Some("Shure MV7 isn't connected. Using MacBook Pro Microphone.")
        );
        let last = mic.options.last().unwrap();
        assert_eq!(
            (last.id.as_str(), last.title.as_str(), last.enabled),
            ("usb-1", "Shure MV7 (not connected)", false)
        );
        assert_eq!(mic.meter.as_deref(), Some("BuiltInMicrophoneDevice"));

        panel.set_inputs(both);
        assert_eq!(panel.view(&host).microphone.note, None, "back again");
        pick(&mut panel, &mut host, "default");
        assert_eq!(read_json(&host.path).get("microphone"), None);
        assert_eq!(panel.view(&host).microphone.selected, "default");
    }

    #[test]
    #[cfg(unix)]
    fn command_shows_only_while_it_is_configured() {
        let dir = TempDir::new("command");
        let mut host = fake(&dir);
        let mut panel = Panel::default();
        std::fs::write(&host.path, r#"{"transcriber":"command"}"#).unwrap();
        let view = panel.view(&host).transcriber;
        assert_eq!((view.engine, view.command), ("apple", None));

        std::fs::write(
            &host.path,
            r#"{"transcriber":"apple","transcribeCommand":["/opt/stt/whisper-apparent","{audio}"]}"#,
        )
        .unwrap();
        let view = panel.view(&host).transcriber;
        assert_eq!(view.engine, "apple");
        assert_eq!(view.command.as_deref(), Some("whisper-apparent"));

        panel.on(
            Action::Set {
                key: "transcriber".into(),
                value: "command".into(),
            },
            &mut host,
        );
        assert_eq!(read_json(&host.path)["transcriber"], "command");
        assert_eq!(panel.view(&host).transcriber.engine, "command");
    }

    #[test]
    fn permission_rows_ask_once_then_open_settings() {
        let dir = TempDir::new("grant");
        let mut host = fake(&dir);
        let mut panel = Panel::default();
        let mut grants = Grants::default();
        grants
            .statuses
            .insert(Permission::Microphone, Status::Granted);
        grants.statuses.insert(Permission::Camera, Status::Denied);
        panel.set_grants(grants.clone());
        let grant = |panel: &mut Panel, host: &mut Fake, permission| {
            panel.on(Action::Grant { permission }, host)
        };
        assert_eq!(
            grant(&mut panel, &mut host, Permission::Microphone),
            Effect::None
        );
        assert_eq!(
            grant(&mut panel, &mut host, Permission::Accessibility),
            Effect::Request(Permission::Accessibility)
        );
        assert_eq!(
            grant(&mut panel, &mut host, Permission::Accessibility),
            Effect::OpenSettings(Permission::Accessibility)
        );
        assert_eq!(
            grant(&mut panel, &mut host, Permission::Camera),
            Effect::OpenSettings(Permission::Camera)
        );
        let view = panel.view(&host);
        if WIN {
            // Only the microphone has a switch on Windows (and on Linux under X11; the tests
            // run without a Wayland session).
            assert_eq!(view.permissions.len(), 1);
            assert_eq!(view.permissions[0].permission, Permission::Microphone);
        } else {
            let order: Vec<_> = view.permissions.iter().map(|p| p.permission).collect();
            assert_eq!(
                order,
                [
                    Permission::ScreenRecording,
                    Permission::Microphone,
                    Permission::Accessibility,
                    Permission::Camera
                ],
                "no Input Monitoring"
            );
            assert!(view.permissions[..3].iter().all(|p| p.required));
            assert!(!view.permissions[3].required);
        }
        // Screen Recording granted but not yet usable: Reopen relaunches.
        assert_eq!(panel.on(Action::Reopen, &mut host), Effect::None);
        grants
            .statuses
            .insert(Permission::ScreenRecording, Status::Granted);
        grants.screen_needs_relaunch = true;
        panel.set_grants(grants);
        assert_eq!(panel.view(&host).permissions[0].relaunch, !WIN);
        assert_eq!(panel.on(Action::Reopen, &mut host), Effect::Relaunch);
    }

    #[test]
    fn helper_lines_parse_into_actions() {
        let parse = |line: &str| serde_json::from_str::<Action>(line).unwrap();
        assert_eq!(
            parse(
                r#"{"action":"shortcut","kind":"paste","keyCode":15,"modifiers":6144,"key":"r","reserved":false}"#
            ),
            Action::Shortcut {
                kind: ShortcutKind::Paste,
                pressed: pressed(15, CARBON_CONTROL_KEY | CARBON_OPTION_KEY, "r")
            }
        );
        assert_eq!(
            parse(r#"{"action":"set","key":"autoSend","value":true}"#),
            Action::Set {
                key: "autoSend".into(),
                value: true.into()
            }
        );
        assert_eq!(
            parse(r#"{"action":"record-shortcut","kind":"record"}"#),
            Action::RecordShortcut {
                kind: ShortcutKind::Record
            }
        );
        assert_eq!(
            parse(r#"{"action":"clear-shortcut","kind":"paste"}"#),
            Action::ClearShortcut {
                kind: ShortcutKind::Paste
            }
        );
        assert_eq!(
            parse(r#"{"action":"grant","permission":"camera"}"#),
            Action::Grant {
                permission: Permission::Camera
            }
        );
        assert_eq!(
            parse(r#"{"action":"check-for-updates"}"#),
            Action::CheckForUpdates
        );
        assert_eq!(parse(r#"{"action":"install-cli"}"#), Action::InstallCli);
        assert_eq!(parse(r#"{"action":"uninstall-cli"}"#), Action::UninstallCli);
    }

    #[test]
    fn the_command_line_row_abbreviates_only_paths_inside_home() {
        let home = PathBuf::from(std::env::var_os("HOME").unwrap());
        // A sibling with the same string prefix is not inside HOME.
        let mut outside = home.as_os_str().to_os_string();
        outside.push("-outside");
        let outside = PathBuf::from(outside).join("bin/apparent");
        for (link, expected) in [
            (
                home.join(".local/bin/apparent"),
                "~/.local/bin/apparent".to_owned(),
            ),
            (outside.clone(), outside.display().to_string()),
        ] {
            let row = CommandLineView::new(
                &crate::composition::command_line::State::Installed {
                    link,
                    homebrew: false,
                },
                None,
            );
            assert_eq!(row.path, Some(expected));
        }
    }

    /// Links are a unix thing; Windows has no Apparent.app to link (`State::Unavailable`).
    #[cfg(unix)]
    #[test]
    fn the_command_line_row_installs_uninstalls_and_says_why_not() {
        let dir = TempDir::new("cli");
        let mut host = fake(&dir);
        let mut panel = Panel::default();
        // A development build has nothing to link.
        let row = panel.view(&host).command_line;
        assert_eq!((row.button, row.enabled), (Some("install"), false));
        assert_eq!(
            row.caption,
            if cfg!(target_os = "linux") {
                "The command-line tool links to Apparent's AppImage; run Apparent from one."
            } else {
                "The command-line tool works in the installed app."
            }
        );
        assert_eq!(
            panel.view(&host).skill_command,
            "bunx skills add https://github.com/tryapparent/apparent/tree/main/capture/agent/skills/apparent"
        );

        let macos = dir.0.join("Apparent.app/Contents/MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        std::fs::write(macos.join("apparent"), "").unwrap();
        host.cli.target = Some(std::fs::canonicalize(macos.join("apparent")).unwrap());
        let row = panel.view(&host).command_line;
        assert_eq!(
            (row.button, row.enabled, row.path),
            (Some("install"), true, None)
        );
        assert!(
            row.caption.starts_with("Links apparent into "),
            "{}",
            row.caption
        );

        panel.on(Action::InstallCli, &mut host);
        let link = dir.0.join("bin/apparent");
        assert_eq!(
            std::fs::read_link(&link).unwrap(),
            host.cli.target.clone().unwrap()
        );
        let row = panel.view(&host).command_line;
        assert_eq!(row.button, Some("uninstall"));
        assert_eq!(row.path, Some(crate::composition::command_line::tilde(&link)));
        // Not Homebrew's or /usr/local's bin, so it may not be on PATH.
        assert!(row.caption.contains("to your PATH"), "{}", row.caption);
        assert!(!row.error);

        panel.on(Action::UninstallCli, &mut host);
        assert!(link.symlink_metadata().is_err());
        assert_eq!(panel.view(&host).command_line.button, Some("install"));

        // Someone else's apparent stays, and the row says so.
        std::fs::write(&link, "other").unwrap();
        let row = panel.view(&host).command_line;
        assert_eq!((row.button, row.error), (None, true));
        assert!(
            row.caption
                .ends_with("is another program, so Apparent leaves it alone.")
        );
        panel.on(Action::InstallCli, &mut host);
        assert_eq!(std::fs::read_to_string(&link).unwrap(), "other");
        let row = panel.view(&host).command_line;
        assert!(
            row.error && row.caption.starts_with("Couldn't install it: "),
            "{}",
            row.caption
        );
    }
}
