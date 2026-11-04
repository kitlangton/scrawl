//! Owned presentation contract. Settings retains action and persistence authority.
use super::*;
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct View {
    pub version: String,
    pub connect: Option<crate::capture::desktop::settings::connect::View>,
    pub hosted: Option<crate::connect::hosted::AccountView>,
    pub login: LoginView,
    pub error_reporting: bool,
    pub agent_demo_video: bool,
    pub shortcuts: Vec<ShortcutView>,
    pub microphone: MicrophoneView,
    pub transcriber: TranscriberView,
    pub lower_playback: bool,
    #[serde(skip_serializing, default = "playback_available")]
    pub playback_available: bool,
    #[serde(skip_serializing, default = "playback_note")]
    pub playback_note: String,
    pub playback_level: u8,
    pub video_export: bool,
    pub retention: RetentionView,
    /// How much the captures folder takes, once measured.
    pub captures_size: Option<String>,
    pub captures_dir: CapturesDirView,
    pub permissions: Vec<PermissionView>,
    pub updates: bool,
    pub update_check: UpdateCheck,
    pub update_channel: String,
    pub main_updates: bool,
    pub command_line: CommandLineView,
    pub skill_command: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
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
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct CommandLineView {
    /// Where `apparent` is linked, once it is.
    pub path: Option<String>,
    /// "install", "uninstall", or none.
    pub button: Option<String>,
    pub enabled: bool,
    pub caption: String,
    pub error: bool,
}

impl CommandLineView {
    pub(super) fn new(state: &crate::composition::command_line::State, error: Option<&str>) -> Self {
        use crate::composition::command_line::{State, tilde, usually_on_path};
        let folder = |link: &std::path::Path| link.parent().map_or_else(String::new, tilde);
        let mut view = Self {
            path: None,
            button: Some("install".into()),
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
                    view.button = Some("uninstall".into());
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct LoginView {
    pub on: bool,
    pub available: bool,
    pub needs_approval: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutView {
    pub kind: ShortcutKind,
    pub title: String,
    pub label: String,
    pub key: String,
    pub shift: bool,
    pub control: bool,
    pub option: bool,
    pub command: bool,
    pub is_default: bool,
    /// A live system binding owns this chord; change it in the compositor instead.
    #[serde(default)]
    pub externally_managed: bool,
    /// It can be turned off (paste); `label` is empty while it is.
    #[serde(default)]
    pub clearable: bool,
    /// The recorder is waiting for a new combination.
    pub recording: bool,
    /// Why the last combination was turned down.
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MenuOptionView {
    pub id: String,
    pub title: String,
    pub enabled: bool,
}

pub(super) fn microphone_view(
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct TranscriberView {
    /// What recordings use now: "apple", "hex" or "command".
    pub engine: String,
    pub hex_title: String,
    pub hex_enabled: bool,
    pub language: String,
    pub apple_locale: String,
    pub models: Vec<ModelView>,
    /// The program's file name while `transcribeCommand` is usable (read, never run); Command
    /// is offered only then.
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speech: Option<Speech>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ModelView {
    pub id: String,
    pub title: String,
    pub chosen: bool,
}

/// Settings › Storage › Location: where new captures go (`"capturesDir"`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
#[serde(default)]
pub struct CapturesDirView {
    /// The folder, with the home folder as `~`.
    pub path: String,
    /// Chosen in Settings rather than the built-in folder, so Reset shows.
    pub custom: bool,
}

impl CapturesDirView {
    pub(super) fn new(config: &crate::shared::handoff::Config) -> Self {
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

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct RetentionView {
    /// The chosen preset, or `None` for a policy written by hand.
    pub preset: Option<String>,
    pub options: Vec<OptionView>,
    pub note: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OptionView {
    pub id: String,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionView {
    pub permission: Permission,
    pub status: Status,
    pub required: bool,
    pub requested: bool,
    /// Granted, but on only after Apparent reopens.
    pub relaunch: bool,
}

// This is permission to request the optional feature, not a live audio check.
pub(super) fn playback_available() -> bool {
    cfg!(target_os = "macos") || cfg!(target_os = "linux") && crate::shared::handoff::experiment("linux-playback-ducking")
}

pub(super) fn playback_note() -> String {
    if cfg!(target_os = "linux") {
        if playback_available() {
            "Experimental · manual setup required. Mono/stereo, one capture. No live effectiveness check."
        } else {
            "Linux playback lowering is opt-in. Setup: capture/desktop/docs/linux/linux-playback.md."
        }
    } else {
        "Temporarily lower other audio while recording."
    }.into()
}


impl View {
    pub fn recording(&self) -> Option<ShortcutKind> { self.shortcuts.iter().find(|s| s.recording).map(|s| s.kind) }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Speech {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub locales: Vec<String>,
    pub progress: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}


impl From<crate::capture::engine::runtime::audio::transcription::speech_model::Status> for Speech {
    fn from(status: crate::capture::engine::runtime::audio::transcription::speech_model::Status) -> Self {
        use crate::capture::engine::runtime::audio::transcription::speech_model::State;
        Self {
            state: match status.state {
                State::Unknown => "unknown", State::Checking => "checking",
                State::Downloading => "downloading", State::Ready => "ready",
                State::Unavailable => "unavailable", State::NeedsPermission => "needsPermission",
                State::Failed => "failed",
            }.into(),
            engine: status.engine, locale: status.locale, locales: status.locales,
            progress: status.progress, error: status.error,
        }
    }
}
