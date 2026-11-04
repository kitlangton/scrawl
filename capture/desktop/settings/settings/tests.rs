use super::*;

#[test]
fn retired_windows_cannot_close_or_restart_a_reopened_settings_session() {
    let old_session = super::super::presentation::generation();
    let old_storage = super::super::presentation::generation();
    let reopened_session = super::super::presentation::generation();
    let reopened_storage = super::super::presentation::generation();
    for retired in [old_session, old_storage] {
        for current in [reopened_session, reopened_storage] {
            for event in [Event::Exited, Event::Action(Action::Close), Event::Action(Action::UpdateNow)] {
                assert!(Event::Presentation { generation: retired, event: Box::new(event) }
                    .for_presentation(current).is_none());
            }
        }
    }
    assert!(matches!(Event::Presentation { generation: reopened_storage,
        event: Box::new(Event::Action(Action::Close)) }.for_presentation(reopened_storage),
        Some(Event::Action(Action::Close))));
    assert!(matches!(Event::Tick.for_presentation(reopened_storage), Some(Event::Tick)));
}

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
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
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
fn hosted_sharing_settings_default_on_preserve_opt_out_and_explicit_account_actions() {
    let dir = TempDir::new("hosted-sharing-settings");
    let host = fake(&dir);
    let panel = Panel::default();
    assert!(!serde_json::to_value(panel.view(&host)).unwrap()["hosted"].is_null());
    assert!(crate::shared::handoff::load_config_from(&host.path).cloud.is_none());
    std::fs::write(&host.path, r#"{"experiments":{"hostedSharing":false}}"#).unwrap();
    assert!(serde_json::to_value(panel.view(&host)).unwrap()["hosted"].is_null());
    std::fs::write(&host.path, r#"{"cloud":{"server":"https://synthetic.example","enabled":false,"uploadConsent":false}}"#).unwrap();
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
fn error_reporting_switch_round_trips_without_losing_config() {
    let dir = TempDir::new("error-reporting");
    let mut host = fake(&dir);
    let mut panel = Panel::default();
    assert!(panel.view(&host).error_reporting, "missing config defaults on");
    let unrelated = serde_json::json!({
        "ingestToken": "synthetic-private-token",
        "experiments": {"other": true},
        "lowerPlayback": true
    });
    std::fs::write(&host.path, serde_json::to_vec(&unrelated).unwrap()).unwrap();
    assert!(panel.view(&host).error_reporting, "missing preference defaults on");
    for on in [false, true, false] {
        let action = serde_json::from_value::<Action>(serde_json::json!({
            "action": "set", "key": "errorReporting", "value": on
        })).unwrap();
        assert_eq!(panel.on(action, &mut host), Effect::None);
        let reopened = Panel::default().view(&host);
        assert_eq!(reopened.error_reporting, on);
        let serialized = serde_json::to_value(reopened).unwrap();
        assert_eq!(serialized["errorReporting"], on);
        assert!(serialized.get("error_reporting").is_none());
        let saved = read_json(&host.path);
        for (key, value) in unrelated.as_object().unwrap() {
            assert_eq!(&saved[key], value, "unrelated config key {key} must survive");
        }
    }
}

#[test]
fn invalid_config_keeps_error_reporting_off_and_is_not_replaced() {
    let dir = TempDir::new("invalid-error-reporting-config");
    let mut host = fake(&dir);
    let mut panel = Panel::default();
    std::fs::write(&host.path, b"{").unwrap();
    assert!(!panel.view(&host).error_reporting);
    panel.on(Action::Set { key: "errorReporting".into(), value: true.into() }, &mut host);
    assert_eq!(std::fs::read(&host.path).unwrap(), b"{");
    assert!(!panel.view(&host).error_reporting);
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
    assert!(panel.view(&host).video_export);
    set(&mut panel, &mut host, "videoExport", false.into());
    assert!(!Panel::default().view(&host).video_export);
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

#[test]
fn shortcut_transaction_preserves_interleaved_other_shortcut() {
    use crate::shared::handoff::set_config_write_hook;
    for kind in ShortcutKind::ALL {
        let dir = TempDir::new(&format!("race-{kind:?}"));
        let path = dir.0.join("config.json");
        std::fs::write(&path, r#"{"shortcuts":{"extension":"keep"},"cloud":{"token":"synthetic-secret"},"unrelated":42}"#).unwrap();
        let other_kind = match kind {
            ShortcutKind::Record => ShortcutKind::Paste,
            ShortcutKind::Paste => ShortcutKind::Record,
        };
        let alternate = ShortcutConfig {
            key_code: R,
            carbon_modifiers: CTRL_OPT,
            label: label(CTRL_OPT, "R"),
        };
        let expected = alternate.clone();
        let mut interleaved = false;
        set_config_write_hook(Some(Box::new(move |point, path| {
            if point == "before_lock" && !interleaved {
                interleaved = true;
                save_shortcut_in(path, other_kind, &alternate).unwrap();
            }
        })));
        // The old implementation already cloned `shortcuts` before reaching this hook.
        let result = save_shortcut_in(&path, kind, &kind.default_config());
        set_config_write_hook(None);
        result.unwrap();
        let (loaded, _) = Shortcuts::load_from(&path, &dir.0.join("absent-legacy"));
        assert_eq!(loaded.get(other_kind), &expected);
        assert_eq!(loaded.get(kind), &kind.default_config());
        let config = read_json(&path);
        assert_eq!(config["shortcuts"]["extension"], "keep");
        assert_eq!(config["cloud"]["token"], "synthetic-secret");
        assert_eq!(config["unrelated"], 42);
    }
}
