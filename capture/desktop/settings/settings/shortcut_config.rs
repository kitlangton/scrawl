use super::*;

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

    pub(super) fn set(&mut self, kind: ShortcutKind, cfg: ShortcutConfig) {
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

pub(super) fn read_json(path: &Path) -> serde_json::Value {
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
    crate::shared::handoff::update_config_in(path, |config| {
        let object = config.as_object_mut().expect("config store returns object");
        let shortcuts = object.entry("shortcuts").or_insert_with(|| serde_json::json!({}));
        if !shortcuts.is_object() {
            *shortcuts = serde_json::json!({});
        }
        let map = shortcuts.as_object_mut().expect("an object");
        if cfg.is_default_for(kind) {
            map.remove(kind_key(kind));
        } else if cfg.is_unset() {
            anyhow::ensure!(kind.can_unset(), "the {kind:?} shortcut can't be turned off");
            map.insert(kind_key(kind).to_owned(), serde_json::Value::Null);
        } else {
            map.insert(kind_key(kind).to_owned(), serde_json::to_value(Stored {
                key_code: cfg.key_code,
                modifiers: cfg.carbon_modifiers,
                label: cfg.label.clone(),
            })?);
        }
        Ok(())
    })
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
pub(super) fn reserved(modifiers: u32, key: &str) -> bool {
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
