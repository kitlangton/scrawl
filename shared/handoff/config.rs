//! Config schema and read access; durable writes remain in config_store.
use super::{AGENT_DEMO_VIDEO, default_config_path, update_config_in};
use std::{fs, path::{Path, PathBuf}};
use anyhow::{Context, Result};
use serde::Deserialize;

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Config {
    /// Hosted sync preference; credentials are separate private artifacts, not settings.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[serde(default)]
    pub cloud: Option<crate::connect::hosted::CloudConfig>,
    /// Desired OS login registration. Missing leaves the OS's existing choice alone.
    #[serde(default)]
    pub launch_at_login: Option<bool>,
    #[serde(default)]
    pub agent_host: Option<String>,
    #[serde(default)]
    pub agent_opencode: Option<String>,
    #[serde(default)]
    pub update_channel: Option<String>,
    /// `false` turns Screen-capture auto-crop off; an object overrides its settings.
    #[serde(default)]
    pub autocrop: Option<serde_json::Value>,
    /// `false` keeps Mac captures of a dev site on the clipboard instead of sending them to the
    /// site's OpenCode session (`routing`).
    #[serde(default)]
    pub auto_send: Option<bool>,
    /// Feature flag: shows **Export Last Capture as Video…** in the menu (default on).
    #[serde(default)]
    pub video_export: Option<bool>,
    /// `"apple"` (default), `"hex"` or `"command"`: which engine transcribes recordings
    /// (`transcribe`). The command's own keys are read by `transcribe_command`, not here, so a
    /// malformed one can't reset this struct.
    #[serde(default)]
    pub transcriber: Option<String>,
    /// Reuse Linux Parakeet across captures for 30 idle minutes (default true).
    #[serde(default)]
    pub warm_recognizer: Option<bool>,
    /// Lower the output volume while recording (`playback`) to `playbackLevel` percent of it.
    #[serde(default)]
    pub lower_playback: Option<bool>,
    #[serde(default)]
    pub playback_level: Option<u8>,
    /// The Hex model to use when `transcriber` is `"hex"` (default: the first ready one of
    /// `hex::PREFERRED_MODELS`).
    #[serde(default)]
    pub hex_model: Option<String>,
    /// Hex language code (`"en"` by default, `"auto"` for model language detection).
    #[serde(default)]
    pub hex_language: Option<String>,
    /// Apple Speech locale (BCP-47, e.g. "zh-TW"); missing means the Mac's own language.
    #[serde(default)]
    pub apple_locale: Option<String>,
    /// Whether to use the native Metal recording pill instead of GPUI's `HudPillWindow` (default `true`).
    #[serde(default)]
    pub native_pill: Option<bool>,
    /// Visual style of the native recording pill: `"black"` (default) or `"glass"` (`NSGlassEffectView`).
    #[serde(default)]
    pub pill_style: Option<String>,
    /// Behaviour Kit tries locally before it ships (`{"agentContext": true}`); see `experiment`.
    /// Read leniently (`lenient_experiments`), so a stray value never fails the file.
    #[serde(default, deserialize_with = "lenient_experiments")]
    pub experiments: std::collections::BTreeMap<String, bool>,
    /// The microphone Mac captures record from (`mic`); absent follows the macOS default.
    #[serde(default)]
    pub microphone: Option<crate::capture::engine::runtime::audio::mic::Choice>,
    /// Explicit off allows typed captures without microphone access; applies at next start.
    #[serde(default)]
    pub microphone_enabled: Option<bool>,
    /// Smart routing's settings (`routing::look`).
    #[serde(default)]
    pub routing: Option<RoutingConfig>,
    /// Where new captures go (`default_captures_dir`); absent keeps the built-in folder.
    #[serde(default)]
    pub captures_dir: Option<String>,
}

impl Config {
    pub fn video_export_enabled(&self) -> bool {
        self.video_export.unwrap_or(true)
    }

    pub fn warm_recognizer_enabled(&self) -> bool {
        self.warm_recognizer.unwrap_or(true)
    }

    /// The output volume while recording when `lowerPlayback` is on, in percent of what it was.
    pub fn recording_volume(&self) -> u8 {
        self.playback_level.unwrap_or(20).min(100)
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RoutingConfig {
    /// The checkout each app is built from, by bundle ID or a prefix of one
    /// (`{"ai.opencode.desktop": "~/code/opencode"}`), for apps whose checkout isn't named like
    /// them.
    #[serde(default)]
    pub apps: std::collections::BTreeMap<String, String>,
    /// The folder a URL prefix serves, for file servers whose pages aren't their own project's
    /// (`{"https://studio.example.ts.net:8444/raw/": "~/code/home/artifacts"}`).
    #[serde(default)]
    pub servers: std::collections::BTreeMap<String, String>,
}

/// `true`/`false`, `"yes"`/`"on"`/`"1"` and their opposites, or a number (non-zero is on). Other
/// values are skipped and logged once, as is an `experiments` that isn't an object.
pub(super) fn lenient_experiments<'de, D>(
    deserializer: D,
) -> std::result::Result<std::collections::BTreeMap<String, bool>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde_json::Value;
    let value = Value::deserialize(deserializer)?;
    let mut out = std::collections::BTreeMap::new();
    let Value::Object(map) = value else {
        if !value.is_null() {
            log_config_once(format!(
                "config.json: ignoring \"experiments\": {value} (expected an object)"
            ));
        }
        return Ok(out);
    };
    for (name, value) in map {
        let on = match &value {
            Value::Bool(b) => Some(*b),
            Value::Number(n) => n.as_f64().map(|n| n != 0.0),
            Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
                "true" | "yes" | "on" | "1" => Some(true),
                "false" | "no" | "off" | "0" | "" => Some(false),
                _ => None,
            },
            _ => None,
        };
        match on {
            Some(on) => {
                out.insert(name, on);
            }
            None => log_config_once(format!(
                "config.json: ignoring experiment \"{name}\": {value} (expected true or false)"
            )),
        }
    }
    Ok(out)
}

/// Config problems are logged once per process, not on every read.
pub(super) fn log_config_once(message: String) {
    static SEEN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let mut seen = SEEN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !seen.contains(&message) {
        crate::capture::engine::runtime::apparent_log(&message);
        seen.push(message);
    }
}

/// More of what Apparent already knows in the summary: marks, shortcuts, drags and windows inline
/// at their time, numbered marks on shared frames, where a screen changed.
pub const AGENT_CONTEXT: &str = "agentContext";

/// Transcribe Mac recordings while they record, so finishing only waits for the last words
/// (`live_transcribe`).
pub const LIVE_TRANSCRIBE: &str = "liveTranscribe";

static EXPERIMENT_OVERRIDES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Turns an experiment on for this process (`apparent reprocess --experiment <name>`).
pub fn enable_experiment(name: &str) {
    EXPERIMENT_OVERRIDES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(name.to_owned());
}

pub(super) fn experiment_overridden(name: &str) -> bool {
    EXPERIMENT_OVERRIDES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .any(|n| n == name)
}

/// Whether an experiment is on: `"experiments": {"<name>": true}` in `config.json`, or
/// `enable_experiment` in this process. Most are off by default; tests see only overrides.
pub fn experiment(name: &str) -> bool {
    let overridden = experiment_overridden(name);
    if overridden || cfg!(test) {
        return overridden;
    }
    experiment_enabled_in(&config(), name)
}

/// Most experiments are opt-in; these features are on unless explicitly disabled.
pub(crate) fn experiment_enabled_in(config: &Config, name: &str) -> bool {
    config
        .experiments
        .get(name)
        .copied()
        .unwrap_or(matches!(
            name,
            AGENT_DEMO_VIDEO | "copyCaptureId" | "connectSettings" | "nativeLaunch" | "electronCaptureLocation"
                | "movableRecordingControls" | "exportNarration" | "onboardingFirstCapture"
                | "hostedSync" | "hostedEncryption" | "hostedSharing"
        ))
}

pub fn load_config_from(path: &Path) -> Config {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .map(config_from_value)
        .unwrap_or_default()
}

/// The config in `value`. A key whose value doesn't fit (`"autoSend": "maybe"`) is dropped and
/// logged on its own, so one bad setting never resets the others to their defaults.
pub(super) fn config_from_value(value: serde_json::Value) -> Config {
    let serde_json::Value::Object(map) = value else {
        return Config::default();
    };
    if let Ok(config) = serde_json::from_value(serde_json::Value::Object(map.clone())) {
        return config;
    }
    // Add keys one at a time, keeping each only if everything so far still reads.
    let mut kept = serde_json::Map::new();
    for (key, value) in map {
        kept.insert(key.clone(), value.clone());
        if let Err(error) =
            serde_json::from_value::<Config>(serde_json::Value::Object(kept.clone()))
        {
            kept.remove(&key);
            log_config_once(format!("config.json: ignoring \"{key}\": {error}"));
        }
    }
    serde_json::from_value(serde_json::Value::Object(kept)).unwrap_or_default()
}

pub(super) fn config() -> Config {
    load_config_from(&default_config_path())
}

/// Read at recording start; edits never interrupt an active capture. Tests use defaults.
pub fn warm_recognizer() -> bool {
    if cfg!(test) {
        Config::default().warm_recognizer_enabled()
    } else {
        config().warm_recognizer_enabled()
    }
}



/// `routing.apps` from config.json, `~` expanded. Tests see none.
pub fn routing_apps() -> Vec<(String, PathBuf)> {
    routing_paths(|routing| routing.apps)
}

/// `routing.servers` in config.json: URL prefixes and the folders they serve, `~` expanded.
pub fn routing_servers() -> Vec<(String, PathBuf)> {
    routing_paths(|routing| routing.servers)
}

pub(super) fn routing_paths(
    pick: fn(RoutingConfig) -> std::collections::BTreeMap<String, String>,
) -> Vec<(String, PathBuf)> {
    if cfg!(test) {
        return Vec::new();
    }
    let home = dirs::home_dir().unwrap_or_default();
    config()
        .routing
        .map(pick)
        .unwrap_or_default()
        .into_iter()
        .map(|(id, path)| {
            let path = match path.strip_prefix("~/") {
                Some(rest) => home.join(rest),
                None => PathBuf::from(path),
            };
            (id, path)
        })
        .collect()
}

/// Whether Mac captures of a dev site go straight to its session (`autoSend`, default on).
pub fn auto_send() -> bool {
    config().auto_send.unwrap_or(true)
}

/// Whether the menu offers video export (`videoExport`, default on). `apparent export` works
/// either way.
pub fn video_export() -> bool {
    config().video_export_enabled()
}

/// The configured transcriber name (`transcriber`) as written, unvalidated.
pub fn transcriber() -> Option<String> {
    config().transcriber
}

/// The configured Hex model id (`hexModel`), if any.
pub fn hex_model() -> Option<String> {
    config()
        .hex_model
        .map(|model| model.trim().to_owned())
        .filter(|model| !model.is_empty())
}

/// The configured Hex language, defaulting to the previous English behavior.
pub fn hex_language() -> String {
    hex_language_from(&config())
}

pub fn hex_language_from(config: &Config) -> String {
    config
        .hex_language
        .as_deref()
        .and_then(crate::shared::hex::language_code)
        .unwrap_or_else(|| crate::shared::hex::DEFAULT_LANGUAGE.into())
}

/// The configured Apple Speech locale, or the Mac's own language when unset.
pub fn apple_locale() -> Option<String> {
    apple_locale_from(&config())
}

pub fn apple_locale_from(config: &Config) -> Option<String> {
    config
        .apple_locale
        .as_deref()
        .and_then(crate::capture::engine::runtime::audio::transcription::speech_model::locale_id)
}

/// The command transcriber's program (`transcribeCommand`, `transcribeTimeoutSeconds`), read
/// on its own from the config file.
pub fn transcribe_command()
-> Result<crate::capture::engine::runtime::audio::transcription::command_transcriber::Program, crate::capture::engine::runtime::audio::transcription::command_transcriber::Unusable> {
    crate::capture::engine::runtime::audio::transcription::command_transcriber::Program::load_from(&default_config_path())
}

/// Change one experiment without replacing unrelated flags or secret-bearing config keys.
pub(crate) fn set_experiment_in(path: &Path, name: &str, on: bool) -> Result<()> {
    update_config_in(path, |config| {
        let experiments = config
            .as_object_mut()
            .unwrap()
            .entry("experiments")
            .or_insert_with(|| serde_json::json!({}));
        let experiments = experiments
            .as_object_mut()
            .with_context(|| format!("Invalid experiments at {}", path.display()))?;
        experiments.insert(name.to_owned(), on.into());
        Ok(())
    })
}
