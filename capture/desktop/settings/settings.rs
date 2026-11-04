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

mod shortcut_config;
pub use shortcut_config::*;
use shortcut_config::{read_json, reserved};

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

mod models;
pub use models::*;

pub(crate) fn preset_id(preset: crate::capture::engine::bundle::retention::Preset) -> &'static str {
    match preset {
        crate::capture::engine::bundle::retention::Preset::Week => "week",
        crate::capture::engine::bundle::retention::Preset::Month => "month",
        crate::capture::engine::bundle::retention::Preset::Forever => "forever",
    }
}

pub(crate) fn retention_view(policy: &crate::capture::engine::bundle::retention::Policy) -> RetentionView {
    RetentionView {
        preset: policy.preset().map(|p| preset_id(p).into()),
        options: crate::capture::engine::bundle::retention::Preset::ALL.into_iter()
            .map(|p| OptionView { id: preset_id(p).into(), label: p.label().into() }).collect(),
        note: retention_note(policy),
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
        title: kind.name().into(),
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
    Navigate { pane: Pane },
    Connect { command: super::connect::Command },
    /// A settings control: `launchAtLogin`, `errorReporting`, `videoExport`, `lowerPlayback` (bool),
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
    connect: Option<super::connect::View>,
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
            // Session owns the asynchronous controller; forged actions cannot bypass it.
            Action::Connect { .. } => Effect::None,
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
            Action::Navigate { .. } => Effect::None, // Session owns presentation routing.
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
            ("errorReporting", serde_json::Value::Bool(on)) => {
                if let Err(error) = crate::capture::desktop::workflow::error_reporting::set_enabled_in(&path, *on) {
                    crate::capture::engine::runtime::apparent_log(format!("Saving {key} failed: {error:#}"));
                }
            }
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
            connect: self.connect.clone(),
            hosted: crate::shared::handoff::experiment_enabled_in(&config, crate::connect::hosted::HOSTED_SHARING)
                .then(|| crate::connect::hosted::account_view_in(&host.config_path())),
            login: LoginView {
                on: matches!(login, LoginItem::On | LoginItem::NeedsApproval),
                available: login != LoginItem::Unavailable,
                needs_approval: login == LoginItem::NeedsApproval,
            },
            error_reporting: crate::capture::desktop::workflow::error_reporting::enabled_in(&host.config_path()),
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
            playback_available: models::playback_available(),
            playback_note: models::playback_note(),
            playback_level: config.recording_volume(),
            transcriber: TranscriberView {
                engine: match engine {
                    crate::capture::engine::runtime::audio::transcription::transcribe::Engine::Command if command.is_some() => "command",
                    _ if menu.hex_checked => "hex",
                    _ => "apple",
                }.into(),
                hex_title: menu.hex_title.into(),
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
                speech: None,
            },
            video_export: config.video_export_enabled(),
            retention: retention_view(&policy),
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
            }.into(),
            main_updates: cfg!(target_os = "linux"),
            command_line: CommandLineView::new(
                &crate::composition::command_line::state(&host.command_line()),
                self.cli_error.as_deref(),
            ),
            skill_command: SKILL_COMMAND.into(),
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
    Presentation { generation: u64, event: Box<Event> },
    Connect(super::connect::Event),
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

impl Event {
    pub(super) fn for_presentation(self, current: u64) -> Option<Self> {
        match self {
            Self::Presentation { generation, event } if generation == current => Some(*event),
            Self::Presentation { .. } => None,
            event => Some(event),
        }
    }
}

/// GPUI receives owned state directly; existing control envelopes retain their wire adapter.
pub(crate) enum PresentationMessage {
    State(View),
    Control(serde_json::Value),
}

/// Native or Electron presentation. Separate children register their exact live
/// process with capture's own-window exclusion; all use the same bounded actions.
pub(super) enum Transport {
    Helper {
        child: crate::capture::ui_process::UiChild,
        stdin: Option<std::process::ChildStdin>,
    },
    InProcess(Option<flume::Sender<PresentationMessage>>),
}

/// The open Settings window.
pub struct Session {
    generation: u64,
    storage: bool,
    connect_presentation: bool,
    panel: Panel,
    connect: Option<super::connect::Controller>,
    transport: Transport,
    polling: std::sync::Arc<std::sync::atomic::AtomicBool>,
    events: std::sync::Arc<dyn Fn(Event) + Send + Sync>,
    last_view: Option<View>,
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
        let generation = super::presentation::generation();
        let (transport, storage) = Self::open_presentation(pane, generation, &events, true)?;

        let polling = std::sync::Arc::new(AtomicBool::new(true));
        let alive = std::sync::Arc::clone(&polling);
        let poll = std::sync::Arc::clone(&events);
        std::thread::spawn(move || {
            let mut n = 0_u32;
            while alive.load(Ordering::Acquire) {
                // Refresh model metadata every 30 s; grants/login state every
                // second. Capture history is measured only on a Storage request.
                if n % 30 == 0 {
                    crate::shared::hex::refresh_in_background();
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
        let config_path = crate::shared::handoff::default_config_path();
        let connect = crate::shared::handoff::experiment(super::connect::CONNECT_SETTINGS)
            .then(|| {
                let events = events.clone();
                super::connect::Controller::new(
                    crate::connect::hosted::settings_backend(&config_path),
                    std::sync::Arc::new(move |event| events(Event::Connect(event))),
                )
            });
        panel.connect = connect.as_ref().map(|controller| controller.view());
        Ok(Self {
            generation,
            storage,
            connect_presentation: false,
            panel,
            connect,
            transport,
            polling,
            events,
            last_view: None,
            _microphones: microphones,
        })
    }

    fn open_presentation(
        pane: Option<Pane>, generation: u64,
        events: &std::sync::Arc<dyn Fn(Event) + Send + Sync>,
        allow_storage: bool,
    ) -> anyhow::Result<(Transport, bool)> {
        let target = events.clone();
        let events: std::sync::Arc<dyn Fn(Event) + Send + Sync> = std::sync::Arc::new(move |event| {
            target(Event::Presentation { generation, event: Box::new(event) });
        });
        if allow_storage && pane == Some(Pane::Storage) && crate::shared::handoff::experiment("electronCaptureLocation") {
            match super::presentation::open(events.clone()) {
                Ok(transport) => return Ok((transport, true)),
                Err(error) => crate::capture::engine::runtime::apparent_log(format!("Storage Settings fallback: {error:#}")),
            }
        }
        let transport = if cfg!(not(target_os = "macos")) {
            let (sender, receiver) = flume::unbounded();
            crate::capture::desktop::settings::settings_window::request_open(receiver, pane, events.clone());
            Transport::InProcess(Some(sender))
        } else {
            Self::spawn_helper(pane, events.clone())?
        };
        if pane == Some(Pane::Storage) {
            Self::measure_native_storage(events);
        }
        Ok((transport, false))
    }

    // Native fallback has no bounded Electron usage request. Measure only after
    // an explicit Storage visit, never on General/Connect open or a timer.
    fn measure_native_storage(events: std::sync::Arc<dyn Fn(Event) + Send + Sync>) {
        std::thread::spawn(move || {
            let size = crate::capture::engine::bundle::retention::captures_size(
                &crate::shared::handoff::default_captures_dir(),
            );
            events(Event::CapturesSize(size));
        });
    }

    /// `apparent-helper settings`, reading its actions off stdout.
    fn spawn_helper(
        pane: Option<Pane>,
        events: std::sync::Arc<dyn Fn(Event) + Send + Sync>,
    ) -> anyhow::Result<Transport> {
        use std::io::BufRead;
        use std::process::{Command, Stdio};

        let mut command = Command::new(crate::capture::desktop::helper::path()?);
        crate::capture::ui_process::configure_build_identity(&mut command);
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
        Ok(Transport::Helper { child: crate::capture::ui_process::UiChild::new(child), stdin })
    }

    /// Feeds an event in, redraws, and returns what the app must do (`Relaunch`, `Tour`,
    /// `Close`); the rest is done here.
    /// Filter retired windows before coordinator safety/permission checks too.
    pub fn current_event(&self, event: Event) -> Option<Event> {
        event.for_presentation(self.generation)
    }

    pub fn event(&mut self, event: Event, host: &mut dyn Host) -> Effect {
        let Some(event) = self.current_event(event) else { return Effect::None; };
        let effect = match event {
            Event::Presentation { .. } => return Effect::None,
            Event::Action(Action::Navigate { pane }) => {
                self.panel.on(Action::CancelShortcut, host);
                self.show(Some(pane));
                Effect::None
            }
            Event::Connect(event) => {
                if self.connect.as_mut().is_some_and(|controller| controller.receive(event)) {
                    crate::connect::hosted::start_worker();
                }
                Effect::None
            }
            Event::Action(Action::Connect { command }) => {
                let generation = super::presentation::generation();
                let target = self.events.clone();
                let events = std::sync::Arc::new(move |event| {
                    target(Event::Presentation { generation, event: Box::new(event) });
                });
                if let Ok(transport) = super::presentation::open_connect(events, command) {
                    if let Some(controller) = &mut self.connect { controller.close(); }
                    self.connect = None;
                    self.generation = generation;
                    self.storage = false;
                    self.connect_presentation = true;
                    self.last_view = None;
                    super::presentation::retire(std::mem::replace(&mut self.transport, transport));
                    return Effect::None;
                }
                if let Some(controller) = &mut self.connect {
                    if let Some(uri) = controller.command(command) {
                        let _ = crate::shared::handoff::open_path(Path::new(&uri));
                    }
                }
                Effect::None
            }
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
            Event::Exited if self.connect_presentation => {
                self.show(Some(Pane::General));
                return Effect::None;
            }
            Event::Exited if self.storage => {
                // EOF without a normal Close means Electron crashed or failed to
                // launch. Keep Settings usable without touching saved policy.
                let generation = super::presentation::generation();
                if let Ok((transport, storage)) = Self::open_presentation(Some(Pane::Storage), generation, &self.events, false) {
                    self.generation = generation;
                    self.storage = storage;
                    self.last_view = None;
                    super::presentation::retire(std::mem::replace(&mut self.transport, transport));
                    self.sync(host);
                    return Effect::None;
                }
                return Effect::Close;
            }
            Event::Exited => {
                if let Some(controller) = &mut self.connect { controller.close(); }
                self.panel.on(Action::Close, host);
                return Effect::Close;
            }
        };
        if let Some(controller) = &mut self.connect {
            if effect == Effect::Close { controller.close(); }
            controller.refresh();
            self.panel.connect = Some(controller.view());
        }
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
        let mut replaced = false;
        if let Some(pane) = pane {
            let storage = pane == Pane::Storage && crate::shared::handoff::experiment("electronCaptureLocation");
            if self.connect_presentation || storage != self.storage {
                let generation = super::presentation::generation();
                match Self::open_presentation(Some(pane), generation, &self.events, true) {
                    Ok((transport, storage)) => {
                        replaced = true;
                        self.send(&serde_json::json!({"type": "close"}));
                        self.generation = generation;
                        self.storage = storage;
                        self.connect_presentation = false;
                        self.last_view = None;
                        super::presentation::retire(std::mem::replace(&mut self.transport, transport));
                    }
                    Err(error) => crate::capture::engine::runtime::apparent_log(format!("Settings navigation failed: {error:#}")),
                }
            }
        }
        if !replaced && !self.storage && pane == Some(Pane::Storage) {
            let generation = self.generation;
            let events = self.events.clone();
            Self::measure_native_storage(std::sync::Arc::new(move |event| {
                events(Event::Presentation { generation, event: Box::new(event) });
            }));
        }
        if !self.storage && !self.connect_presentation && self.connect.is_none()
            && crate::shared::handoff::experiment(super::connect::CONNECT_SETTINGS)
        {
            let events = self.events.clone();
            self.connect = Some(super::connect::Controller::new(
                crate::connect::hosted::settings_backend(&crate::shared::handoff::default_config_path()),
                std::sync::Arc::new(move |event| events(Event::Connect(event))),
            ));
        }
        self.send(&serde_json::json!({"type": "show", "pane": pane}));
    }

    pub fn sync(&mut self, host: &dyn Host) {
        // Storage reads its narrow projected socket contract. Do not forward the
        // other pages' state to a renderer which does not present those pages.
        if self.storage || self.connect_presentation { return; }
        let mut view = self.panel.view(host);
        // Switched from Hex to Apple Speech: get its model now, not at the first capture.
        let speech = crate::capture::engine::runtime::audio::transcription::speech_model::current();
        if view.transcriber.engine == "apple"
            && speech.state == crate::capture::engine::runtime::audio::transcription::speech_model::State::Unknown
        {
            crate::capture::engine::runtime::audio::transcription::speech_model::ensure();
        }
        view.transcriber.speech = Some(speech.into());
        if self.last_view.as_ref() != Some(&view) {
            if let Transport::InProcess(sender) = &mut self.transport {
                if let Some(channel) = sender.as_ref() {
                    if channel.send(PresentationMessage::State(view.clone())).is_err() { *sender = None; }
                }
            } else {
                #[derive(Serialize)]
                struct State<'a> { r#type: &'static str, #[serde(flatten)] view: &'a View }
                self.send(&serde_json::to_value(State { r#type: "state", view: &view }).unwrap());
            }
            self.last_view = Some(view);
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
                    && channel.send(PresentationMessage::Control(message.clone())).is_err()
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
        let transport = std::mem::replace(&mut self.transport, Transport::InProcess(None));
        super::presentation::retire(transport);
    }
}

#[cfg(test)]
mod tests;
