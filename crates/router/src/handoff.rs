use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::model::ScrawlBundle;

/// Where captures go: `SCRAWL_CAPTURES_DIR`, else `"capturesDir"` in `config.json` (Settings ›
/// Storage), else `builtin_captures_dir`. Tests never see the config file.
pub fn default_captures_dir() -> PathBuf {
    if cfg!(test) {
        if let Some(dir) = std::env::var_os("SCRAWL_CAPTURES_DIR") {
            return PathBuf::from(dir);
        }
        return builtin_captures_dir();
    }
    config().resolved_captures_dir()
}

/// A chosen folder that exists (or can be made) and takes writes; otherwise it's logged once and the
/// built-in folder is used, so a capture is never lost to a missing or unplugged drive.
fn usable_captures_dir(dir: &Path) -> bool {
    let usable = fs::create_dir_all(dir).is_ok()
        && fs::metadata(dir).is_ok_and(|meta| meta.is_dir() && !meta.permissions().readonly());
    if !usable {
        log_config_once(format!(
            "config.json: \"capturesDir\" {} can't be used (missing or not writable); captures go to {}",
            dir.display(),
            builtin_captures_dir().display()
        ));
    }
    usable
}

/// A `"capturesDir"` value as a folder: `~` is the home folder, and only an absolute path counts
/// (anything else is logged and skipped, so captures never land relative to the app's cwd).
pub fn configured_captures_dir(value: &str) -> Option<PathBuf> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    let path = match value.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
            dirs::home_dir()?.join(rest.trim_start_matches(['/', '\\']))
        }
        _ => PathBuf::from(value),
    };
    if path.is_absolute() {
        Some(path)
    } else {
        log_config_once(format!(
            "config.json: ignoring \"capturesDir\": {value} is not an absolute path"
        ));
        None
    }
}

/// `~/Library/Application Support/scrawl/captures` on macOS, `%LOCALAPPDATA%\scrawl\captures`
/// on Windows (local, not roaming: the movies are large).
pub fn builtin_captures_dir() -> PathBuf {
    if cfg!(target_os = "windows") {
        dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("scrawl")
            .join("captures")
    } else {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("Library/Application Support/scrawl/captures")
    }
}

pub fn resolve_bundle_dir(path_or_id: &Path) -> Result<PathBuf> {
    resolve_bundle_dir_in(path_or_id, &default_captures_dir())
}

pub fn resolve_bundle_dir_in(path_or_id: &Path, captures_dir: &Path) -> Result<PathBuf> {
    if path_or_id == Path::new("latest") {
        return newest_bundle(captures_dir);
    }
    let mut path = path_or_id.to_owned();
    if path.file_name().is_some_and(|name| name == "scrawl.json") {
        path = path
            .parent()
            .context("Missing bundle directory")?
            .to_owned();
        if path.as_os_str().is_empty() {
            path = PathBuf::from(".");
        }
    }
    if !path.exists() && path.components().count() == 1 {
        path = captures_dir.join(&path);
        if path.extension().is_none() {
            path.set_extension("scrawl");
        }
    }
    ensure!(
        path.is_dir(),
        "Bundle directory not found: {}",
        path.display()
    );
    let path = plain_path(
        path.canonicalize()
            .with_context(|| format!("Could not resolve {}", path.display()))?,
    );
    ensure!(
        path.extension().is_some_and(|ext| ext == "scrawl"),
        "Expected a .scrawl directory: {}",
        path.display()
    );
    Ok(path)
}

fn newest_bundle(captures: &Path) -> Result<PathBuf> {
    let mut candidates = Vec::new();
    for entry in
        fs::read_dir(captures).with_context(|| format!("Could not read {}", captures.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "scrawl") && path.is_dir() {
            // Bundle creation time is stable even after later zoom-frame extraction.
            let created = fs::read(path.join("scrawl.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<ScrawlBundle>(&bytes).ok())
                .map(|bundle| bundle.created_at_ms)
                .unwrap_or_else(|| {
                    entry
                        .metadata()
                        .ok()
                        .and_then(|m| m.created().or_else(|_| m.modified()).ok())
                        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                        .map_or(0, |time| time.as_millis() as u64)
                });
            candidates.push((created, path));
        }
    }
    candidates.sort_unstable();
    candidates
        .pop()
        .context("No .scrawl captures found")?
        .1
        .canonicalize()
        .map(plain_path)
        .map_err(Into::into)
}

/// `canonicalize` on Windows returns `\\?\C:\…`; agents and terminals want `C:\…`.
pub fn plain_path(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|p| p.strip_prefix(r"\\?\")) {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}

pub fn load_bundle(bundle_dir: &Path) -> Result<(PathBuf, ScrawlBundle)> {
    let directory = resolve_bundle_dir(bundle_dir)?;
    let path = directory.join("scrawl.json");
    let bundle = serde_json::from_slice(
        &fs::read(&path).with_context(|| format!("Could not read {}", path.display()))?,
    )
    .with_context(|| format!("Invalid bundle manifest: {}", path.display()))?;
    Ok((directory, bundle))
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct OpenCodeSession {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, rename = "parentID")]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub time: OpenCodeSessionTime,
    /// V2 puts the session's working directory here; V1 used a top-level `directory`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<OpenCodeSessionLocation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<String>,
}

impl OpenCodeSession {
    pub fn directory(&self) -> Option<&str> {
        self.location
            .as_ref()
            .and_then(|l| l.directory.as_deref())
            .or(self.directory.as_deref())
            .filter(|d| !d.is_empty())
    }

    pub fn is_open_root(&self) -> bool {
        self.parent_id.is_none() && self.time.archived.is_none()
    }

    /// The title, or the ID when it has none.
    pub fn label(&self) -> &str {
        if self.title.is_empty() {
            &self.id
        } else {
            &self.title
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct OpenCodeSessionLocation {
    #[serde(default)]
    pub directory: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct OpenCodeSessionTime {
    #[serde(default)]
    pub updated: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewed: Option<u64>,
    #[serde(default)]
    pub archived: Option<u64>,
}

fn executable(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// `name` on `PATH`. On Windows `name.exe` and `name.cmd` (npm and bun shims) count too.
pub fn on_path(name: &str) -> Option<PathBuf> {
    let names: Vec<String> = if cfg!(windows) && Path::new(name).extension().is_none() {
        vec![
            format!("{name}.exe"),
            format!("{name}.cmd"),
            name.to_owned(),
        ]
    } else {
        vec![name.to_owned()]
    };
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .flat_map(|path| names.iter().map(move |name| path.join(name)))
            .find(|path| executable(path))
    })
}

fn opencode_binary() -> Result<PathBuf> {
    if let Some(bin) = std::env::var_os("OPENCODE_BIN").filter(|bin| !bin.is_empty()) {
        let path = PathBuf::from(bin);
        if executable(&path) {
            return Ok(path);
        }
        if let Some(path) = path.to_str().and_then(on_path) {
            return Ok(path);
        }
        bail!("OPENCODE_BIN is not an executable");
    }
    if let Some(path) = on_path("opencode2") {
        return Ok(path);
    }
    if let Some(path) = dirs::home_dir()
        .map(|home| home.join("bin/opencode2"))
        .filter(|path| executable(path))
    {
        return Ok(path);
    }
    bail!("Could not find opencode2; set OPENCODE_BIN or add it to PATH")
}

/// The `microphone` key: a device the user picked in Settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MicChoice {
    pub id: String,
    pub name: String,
}

/// Default memory ceiling for candidate keyframes during recording (384 MB).
pub const CANDIDATE_BUDGET_BYTES: usize = 384 << 20;

/// How a prompt enters a busy OpenCode session.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeliveryMode {
    #[default]
    Steer,
    Queue,
}

impl DeliveryMode {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "steer" => Some(Self::Steer),
            "queue" => Some(Self::Queue),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Steer => "steer",
            Self::Queue => "queue",
        }
    }
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EngineConfig {
    #[serde(default)]
    pub agent_host: Option<String>,
    #[serde(default)]
    pub agent_opencode: Option<String>,
    #[serde(default)]
    pub ingest_token: Option<String>,
    #[serde(default, alias = "tetherApps")]
    pub usb_apps: Vec<String>,
    /// `false` turns Screen-capture auto-crop off; an object overrides its settings.
    #[serde(default)]
    pub autocrop: Option<serde_json::Value>,
    /// `false` keeps Mac captures of a dev site on the clipboard instead of sending them to the
    /// site's OpenCode session (`routing`).
    #[serde(default)]
    pub auto_send: Option<bool>,
    /// `"steer"` (default) or `"queue"`: whether a capture sent to a busy session joins its
    /// current turn or waits until the turn ends (`delivery_mode`). Read as a value so a bad
    /// one can't reset this struct.
    #[serde(default)]
    pub delivery: Option<serde_json::Value>,
    /// `"apple"` (default), `"hex"` or `"command"`: which engine transcribes recordings
    /// (`transcribe`). The command's own keys are read by `transcribe_command`, not here, so a
    /// malformed one can't reset this struct.
    #[serde(default)]
    pub transcriber: Option<String>,
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
    /// Behaviour Kit tries locally before it ships (`{"agentContext": true}`); see `experiment`.
    /// Read leniently (`lenient_experiments`), so a stray value never fails the file.
    #[serde(default, deserialize_with = "lenient_experiments")]
    pub experiments: std::collections::BTreeMap<String, bool>,
    /// The microphone Mac captures record from (`mic`); absent follows the macOS default.
    #[serde(default)]
    pub microphone: Option<MicChoice>,
    /// Smart routing's settings (`routing::look`). Read key by key (`lenient_routing`), so a
    /// bad `undoWindowSec` doesn't take `autoSend` with it.
    #[serde(default, deserialize_with = "lenient_routing")]
    pub routing: Option<RoutingConfig>,
    /// Jev routing's thresholds (`routing::jev::Thresholds::from_config`).
    #[serde(default)]
    pub jev_routing: Option<serde_json::Value>,
    /// The OpenCode key Jev routing calls the gateway with; absent uses `$OPENCODE_API_KEY` or
    /// OpenCode's own credential.
    #[serde(default)]
    pub jev_api_key: Option<String>,
    /// Where new captures go (`default_captures_dir`); absent keeps the built-in folder.
    #[serde(default)]
    pub captures_dir: Option<String>,
    /// Optional host override for the frame candidate buffer ceiling in bytes (defaults to
    /// `capture::CANDIDATE_BUDGET_BYTES`, 384 MB).
    #[serde(skip)]
    candidate_budget_bytes: Option<usize>,
    /// Optional host override for an owner-only (`0600`) ingest token file. When set, the first
    /// read imports an existing `ingestToken` from `~/.config/scrawl/config.json` so paired
    /// ScrawlKit phones keep working without re-pairing.
    #[serde(skip)]
    ingest_token_path: Option<PathBuf>,
}

pub type Config = EngineConfig;

impl EngineConfig {
    /// Whether an experiment is on in this config (with process-level overrides winning).
    pub fn experiment(&self, name: &str) -> bool {
        let overridden = EXPERIMENT_OVERRIDES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .any(|n| n == name);
        if overridden {
            return true;
        }
        self.experiments.get(name).copied().unwrap_or(false)
    }

    pub fn auto_send(&self) -> bool {
        self.auto_send.unwrap_or(true)
    }

    pub fn delivery_mode(&self) -> DeliveryMode {
        match delivery_mode_of(self.delivery.as_ref()) {
            "queue" => DeliveryMode::Queue,
            _ => DeliveryMode::Steer,
        }
    }

    pub fn hex_model(&self) -> Option<String> {
        self.hex_model
            .as_deref()
            .map(str::trim)
            .filter(|m| !m.is_empty())
            .map(str::to_owned)
    }

    pub fn resolved_captures_dir(&self) -> PathBuf {
        if let Some(dir) = std::env::var_os("SCRAWL_CAPTURES_DIR") {
            return PathBuf::from(dir);
        }
        self.captures_dir
            .as_deref()
            .and_then(configured_captures_dir)
            .filter(|dir| usable_captures_dir(dir))
            .unwrap_or_else(builtin_captures_dir)
    }

    pub fn with_candidate_budget_bytes(mut self, bytes: usize) -> Self {
        self.candidate_budget_bytes = Some(bytes);
        self
    }

    pub fn set_candidate_budget_bytes(&mut self, bytes: Option<usize>) {
        self.candidate_budget_bytes = bytes;
    }

    pub fn candidate_budget_bytes(&self) -> usize {
        self.candidate_budget_bytes
            .unwrap_or(CANDIDATE_BUDGET_BYTES)
            .max(16 << 20)
    }

    pub fn with_ingest_token_path(mut self, path: PathBuf) -> Self {
        self.ingest_token_path = Some(path);
        self
    }

    pub fn ingest_token_path(&self) -> Option<&Path> {
        self.ingest_token_path.as_deref()
    }

    pub fn routing_apps(&self) -> Vec<(String, PathBuf)> {
        self.routing_apps_any()
    }

    pub(crate) fn routing_apps_any(&self) -> Vec<(String, PathBuf)> {
        let mut apps = routing_paths_in_any(self, |routing| &routing.apps);
        apps.retain(|(id, _)| {
            let general = crate::routing::policy::is_general_app(id);
            if general {
                log_config_once(format!(
                    "config.json: ignoring routing.apps \"{id}\": a general-purpose app's captures are about anything"
                ));
            }
            !general
        });
        apps
    }

    pub fn routing_servers(&self) -> Vec<(String, PathBuf)> {
        routing_paths_in_any(self, |routing| &routing.servers)
    }

    pub fn routing_sites(&self) -> Vec<(String, PathBuf)> {
        routing_paths_in_any(self, |routing| &routing.sites)
    }

    pub fn routing_aliases(&self) -> Vec<(String, String)> {
        self.routing
            .as_ref()
            .map(|r| {
                r.aliases
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn routing_refused_rules(&self) -> Vec<String> {
        match self.routing.as_ref().and_then(|r| r.refused_rules.as_ref()) {
            Some(serde_json::Value::Array(list)) => list
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect(),
            _ => Vec::new(),
        }
    }

    pub fn routing_auto_send(&self) -> crate::routing::policy::AutoSend {
        match self.routing.as_ref().and_then(|r| r.auto_send.as_ref()) {
            None => crate::routing::policy::AutoSend::default(),
            Some(serde_json::Value::String(text)) => {
                match crate::routing::policy::AutoSend::parse(text) {
                    Some(mode) => mode,
                    None => {
                        log_config_once(format!(
                            "config.json: ignoring routing.autoSend {text:?} (expected \"rules\", \"confident\" or \"never\")"
                        ));
                        crate::routing::policy::AutoSend::default()
                    }
                }
            }
            Some(other) => {
                log_config_once(format!(
                    "config.json: ignoring routing.autoSend {other} (expected \"rules\", \"confident\" or \"never\")"
                ));
                crate::routing::policy::AutoSend::default()
            }
        }
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
    /// Where captures of a site always go, ahead of weighing: a URL prefix (`https://` optional;
    /// any path and query below it) to a folder, whose newest open session gets them, or to a
    /// session ID (`{"https://problems.example.com": "~/code/team-board"}`).
    #[serde(default)]
    pub sites: std::collections::BTreeMap<String, String>,
    /// Spoken names for sessions, to the title or folder name they mean
    /// (`{"scroll": "SCRAWL"}`), for names speech recognition gets wrong.
    #[serde(default)]
    pub aliases: std::collections::BTreeMap<String, String>,
    /// What may send without asking: `"rules"` (default), `"confident"`
    /// or `"never"` (`routing::policy::AutoSend`).
    #[serde(default)]
    pub auto_send: Option<serde_json::Value>,
    /// Sites and bundle IDs Scrawl never offers a rule for again.
    #[serde(default)]
    pub refused_rules: Option<serde_json::Value>,
    /// How long after a send `scrawl router undo` still takes it back, in seconds.
    #[serde(default)]
    pub undo_window_sec: Option<u64>,
}

/// `true`/`false`, `"yes"`/`"on"`/`"1"` and their opposites, or a number (non-zero is on). Other
/// values are skipped and logged once, as is an `experiments` that isn't an object.
fn lenient_experiments<'de, D>(
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

/// `"routing"` without the keys or map entries that don't fit, each dropped and logged on its own.
fn lenient_routing<'de, D>(deserializer: D) -> std::result::Result<Option<RoutingConfig>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde_json::Value;
    let map = match Value::deserialize(deserializer)? {
        Value::Null => return Ok(None),
        Value::Object(map) => map,
        other => {
            log_config_once(format!(
                "config.json: ignoring \"routing\": {other} (expected an object)"
            ));
            return Ok(None);
        }
    };
    let fits = |map: &serde_json::Map<String, Value>| {
        serde_json::from_value::<RoutingConfig>(Value::Object(map.clone()))
    };
    let mut kept = serde_json::Map::new();
    for (key, value) in map {
        kept.insert(key.clone(), value.clone());
        let Err(error) = fits(&kept) else { continue };
        kept.remove(&key);
        let Value::Object(entries) = value else {
            log_config_once(format!("config.json: ignoring \"routing.{key}\": {error}"));
            continue;
        };
        // A map (`sites`, `apps`): keep the entries that fit.
        let mut good = serde_json::Map::new();
        for (name, entry) in entries {
            good.insert(name.clone(), entry.clone());
            kept.insert(key.clone(), Value::Object(good.clone()));
            if fits(&kept).is_err() {
                good.remove(&name);
                log_config_once(format!(
                    "config.json: ignoring \"routing.{key}\" entry \"{name}\": {entry}"
                ));
            }
        }
        kept.insert(key, Value::Object(good));
    }
    Ok(Some(fits(&kept).unwrap_or_default()))
}

/// Config problems are logged once per process, not on every read.
fn log_config_once(message: String) {
    static SEEN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let mut seen = SEEN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !seen.contains(&message) {
        crate::log::scrawl_log(&message);
        seen.push(message);
    }
}

/// More of what Scrawl already knows in the summary: marks, shortcuts, drags and windows inline
/// at their time, numbered marks on shared frames, where a screen changed.
pub const AGENT_CONTEXT: &str = "agentContext";

/// Transcribe Mac recordings while they record, so finishing only waits for the last words
/// (`live_transcribe`).
pub const LIVE_TRANSCRIBE: &str = "liveTranscribe";

static EXPERIMENT_OVERRIDES: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// Turns an experiment on for this process (`scrawl reprocess --experiment <name>`).
pub fn enable_experiment(name: &str) {
    EXPERIMENT_OVERRIDES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .push(name.to_owned());
}

/// Whether an experiment is on: `"experiments": {"<name>": true}` in `config.json`, or
/// `enable_experiment` in this process. Off by default; tests see only `enable_experiment`.
pub fn experiment(name: &str) -> bool {
    let overridden = EXPERIMENT_OVERRIDES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .any(|n| n == name);
    if overridden || cfg!(test) {
        return overridden;
    }
    config().experiments.get(name).copied().unwrap_or(false)
}

static SCRAWL_HOST_DEFAULTS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Enables Scrawl's default host paths (`~/.config/scrawl/config.json`,
/// `~/Library/Application Support/scrawl`, and `~/Library/Logs/Scrawl/scrawl.log`).
/// Disabled by default in library mode so embedding hosts such as Hex never implicitly read or
/// write Scrawl's files.
pub fn enable_scrawl_host_defaults() {
    SCRAWL_HOST_DEFAULTS.store(true, std::sync::atomic::Ordering::Release);
    crate::log::enable_file_log();
}

pub fn scrawl_host_defaults_enabled() -> bool {
    SCRAWL_HOST_DEFAULTS.load(std::sync::atomic::Ordering::Acquire)
}

pub fn default_config_path() -> PathBuf {
    std::env::var_os("SCRAWL_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config/scrawl/config.json")
        })
}

pub fn parse_config_from<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .map(parse_config_value)
        .unwrap_or_default()
}

/// Pure parser for a Scrawl `config.json` string, returning both `(EngineConfig, RouterConfig)`
/// for a host's one-time import without touching the filesystem.
pub fn parse_scrawl_config(json: &str) -> (EngineConfig, crate::RouterConfig) {
    let engine: EngineConfig = serde_json::from_str::<serde_json::Value>(json)
        .ok()
        .map(parse_config_value)
        .unwrap_or_default();
    let router = crate::RouterConfig::from_engine_config(&engine);
    (engine, router)
}

pub fn load_config_from(path: &Path) -> EngineConfig {
    parse_config_from(path)
}

pub fn engine_config() -> EngineConfig {
    if !scrawl_host_defaults_enabled() && std::env::var_os("SCRAWL_CONFIG").is_none() {
        return EngineConfig::default();
    }
    load_config_from(&default_config_path())
}

/// The config in `value`. A key whose value doesn't fit (`"autoSend": "maybe"`) is dropped and
/// logged on its own, so one bad setting never resets the others to their defaults.
pub fn parse_config_value<T: for<'de> Deserialize<'de> + Default>(value: serde_json::Value) -> T {
    let serde_json::Value::Object(map) = value else {
        return T::default();
    };
    if let Ok(config) = serde_json::from_value(serde_json::Value::Object(map.clone())) {
        return config;
    }
    // Add keys one at a time, keeping each only if everything so far still reads.
    let mut kept = serde_json::Map::new();
    for (key, value) in map {
        kept.insert(key.clone(), value.clone());
        if let Err(error) = serde_json::from_value::<T>(serde_json::Value::Object(kept.clone())) {
            kept.remove(&key);
            log_config_once(format!("config.json: ignoring \"{key}\": {error}"));
        }
    }
    serde_json::from_value(serde_json::Value::Object(kept)).unwrap_or_default()
}

thread_local! {
    static SCOPED_ENGINE_CONFIG: std::cell::RefCell<Option<EngineConfig>> =
        const { std::cell::RefCell::new(None) };
}

pub fn scoped_engine_config() -> Option<EngineConfig> {
    SCOPED_ENGINE_CONFIG.with(|slot| slot.borrow().clone())
}

pub fn with_scoped_engine_config<R>(config: &EngineConfig, f: impl FnOnce() -> R) -> R {
    struct Guard(Option<EngineConfig>);
    impl Drop for Guard {
        fn drop(&mut self) {
            SCOPED_ENGINE_CONFIG.with(|slot| *slot.borrow_mut() = self.0.take());
        }
    }
    let prev = SCOPED_ENGINE_CONFIG.with(|slot| slot.borrow_mut().replace(config.clone()));
    let _guard = Guard(prev);
    f()
}

pub fn with_router_config<R>(config: &crate::RouterConfig, f: impl FnOnce() -> R) -> R {
    with_scoped_engine_config(&config.to_engine_config(), f)
}

fn config() -> EngineConfig {
    scoped_engine_config().unwrap_or_else(engine_config)
}

fn random_bytes_32() -> Result<[u8; 32]> {
    let mut bytes = [0_u8; 32];
    #[cfg(windows)]
    {
        #[link(name = "bcrypt")]
        unsafe extern "system" {
            fn BCryptGenRandom(
                hAlgorithm: *mut std::ffi::c_void,
                pbBuffer: *mut u8,
                cbBuffer: u32,
                dwFlags: u32,
            ) -> i32;
        }
        const BCRYPT_USE_SYSTEM_PREFERRED_RNG: u32 = 0x0000_0002;
        let status = unsafe {
            BCryptGenRandom(
                std::ptr::null_mut(),
                bytes.as_mut_ptr(),
                bytes.len() as u32,
                BCRYPT_USE_SYSTEM_PREFERRED_RNG,
            )
        };
        ensure!(status == 0, "BCryptGenRandom failed: {status}");
    }
    #[cfg(not(windows))]
    {
        use std::io::Read as _;
        let mut file = fs::File::open("/dev/urandom").context("Could not open /dev/urandom")?;
        file.read_exact(&mut bytes)
            .context("Could not read 32 bytes from /dev/urandom")?;
    }
    Ok(bytes)
}

fn random_hex_token_32() -> Result<String> {
    use std::fmt::Write as _;
    let bytes = random_bytes_32()?;
    let mut token = String::with_capacity(64);
    for byte in bytes {
        let _ = write!(&mut token, "{byte:02x}");
    }
    Ok(token)
}

/// Returns `ingestToken` from `path`, generating a 32-byte hex token and writing `path` back with
/// mode `0600` (while preserving all other JSON keys) if `ingestToken` is absent or empty.
pub fn ensure_ingest_token_in(path: &Path) -> Result<String> {
    let mut map = match fs::read(path) {
        Ok(bytes) if !bytes.iter().all(u8::is_ascii_whitespace) => {
            match serde_json::from_slice::<serde_json::Value>(&bytes)
                .with_context(|| format!("Invalid JSON in {}", path.display()))?
            {
                serde_json::Value::Object(map) => map,
                _ => bail!("Expected a JSON object in {}", path.display()),
            }
        }
        Ok(_) | Err(_) => serde_json::Map::new(),
    };

    if let Some(existing) = map
        .get("ingestToken")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Ok(existing.to_owned());
    }

    let token = random_hex_token_32()?;
    map.insert(
        "ingestToken".to_owned(),
        serde_json::Value::String(token.clone()),
    );
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .with_context(|| format!("Could not create {}", parent.display()))?;
    }
    let mut body = serde_json::to_vec_pretty(&serde_json::Value::Object(map))?;
    body.push(b'\n');
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(path)
            .with_context(|| format!("Could not write {}", path.display()))?;
        file.write_all(&body)?;
        file.sync_all()?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        fs::write(path, &body).with_context(|| format!("Could not write {}", path.display()))?;
    }
    Ok(token)
}

pub fn ensure_ingest_token() -> Result<String> {
    ensure_ingest_token_in(&default_config_path())
}

/// Reads an ingest token from a host's owner-only (`0600`) token file `token_file`. If the file
/// is missing or empty, imports an existing `ingestToken` from `legacy_scrawl_config` (when
/// present) or generates a fresh 32-byte hex token, writing `token_file` with mode `0600` (and its
/// parent directory with `0700`) so paired ScrawlKit phones keep working without re-pairing.
pub fn import_or_ensure_ingest_token_file(
    token_file: &Path,
    legacy_scrawl_config: Option<&Path>,
) -> Result<String> {
    if let Ok(raw) = fs::read_to_string(token_file) {
        let trimmed = raw.trim();
        if !trimmed.is_empty() {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(token_file, fs::Permissions::from_mode(0o600));
            }
            return Ok(trimmed.to_owned());
        }
    }

    let imported = legacy_scrawl_config.and_then(|cfg_path| {
        let bytes = fs::read(cfg_path).ok()?;
        let val = serde_json::from_slice::<serde_json::Value>(&bytes).ok()?;
        val.get("ingestToken")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    });
    let token = match imported {
        Some(token) => token,
        None => random_hex_token_32()?,
    };

    if let Some(parent) = token_file.parent()
        && !parent.as_os_str().is_empty()
    {
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)
                .with_context(|| format!("Could not create {}", parent.display()))?;
            let _ = fs::set_permissions(parent, fs::Permissions::from_mode(0o700));
        }
        #[cfg(not(unix))]
        {
            fs::create_dir_all(parent)
                .with_context(|| format!("Could not create {}", parent.display()))?;
        }
    }

    let body = format!("{token}\n");
    #[cfg(unix)]
    {
        use std::io::Write as _;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let mut file = fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o600)
            .open(token_file)
            .with_context(|| format!("Could not write {}", token_file.display()))?;
        file.write_all(body.as_bytes())?;
        file.sync_all()?;
        fs::set_permissions(token_file, fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    {
        fs::write(token_file, body.as_bytes())
            .with_context(|| format!("Could not write {}", token_file.display()))?;
    }
    Ok(token)
}

pub fn import_ingest_token_from_scrawl(token_file: &Path) -> Result<String> {
    import_or_ensure_ingest_token_file(token_file, Some(&default_config_path()))
}

fn skip_in_test() -> bool {
    cfg!(test) && scoped_engine_config().is_none()
}

/// `routing.apps` from config.json, `~` expanded. Tests see none.
pub fn routing_apps() -> Vec<(String, PathBuf)> {
    if skip_in_test() {
        return Vec::new();
    }
    config().routing_apps()
}

/// `routing.autoSend` in config.json, when it's text; anything else is logged once.
pub fn routing_auto_send() -> Option<String> {
    if skip_in_test() {
        return None;
    }
    match config().routing?.auto_send? {
        serde_json::Value::String(text) => {
            if crate::routing::policy::AutoSend::parse(&text).is_none() {
                log_config_once(format!(
                    "config.json: ignoring routing.autoSend {text:?} (expected \"rules\", \"confident\" or \"never\")"
                ));
            }
            Some(text)
        }
        other => {
            log_config_once(format!(
                "config.json: ignoring routing.autoSend {other} (expected \"rules\", \"confident\" or \"never\")"
            ));
            None
        }
    }
}

/// `routing.refusedRules` in config.json: sites and bundle IDs never offered a rule again.
/// `routing.undoWindowSec` (never read in tests).
pub fn routing_undo_window_sec() -> Option<u64> {
    if skip_in_test() {
        return None;
    }
    config().routing.and_then(|r| r.undo_window_sec)
}

pub fn routing_refused_rules() -> Vec<String> {
    if skip_in_test() {
        return Vec::new();
    }
    config().routing_refused_rules()
}

/// `routing.aliases` in config.json: spoken names and the title or folder they mean.
pub fn routing_aliases() -> Vec<(String, String)> {
    if skip_in_test() {
        return Vec::new();
    }
    config().routing_aliases()
}

/// `routing.servers` in config.json: URL prefixes and the folders they serve, `~` expanded.
pub fn routing_servers() -> Vec<(String, PathBuf)> {
    if skip_in_test() {
        return Vec::new();
    }
    config().routing_servers()
}

/// `routing.sites` in config.json: URL prefixes and the folder (`~` expanded) or session ID their
/// captures go to.
pub fn routing_sites() -> Vec<(String, PathBuf)> {
    if skip_in_test() {
        return Vec::new();
    }
    config().routing_sites()
}

fn routing_paths_in_any(
    config: &EngineConfig,
    pick: fn(&RoutingConfig) -> &std::collections::BTreeMap<String, String>,
) -> Vec<(String, PathBuf)> {
    let home = dirs::home_dir().unwrap_or_default();
    config
        .routing
        .as_ref()
        .map(pick)
        .into_iter()
        .flatten()
        .map(|(id, path)| {
            let path = match path.strip_prefix("~/") {
                Some(rest) => home.join(rest),
                None => PathBuf::from(path),
            };
            (id.clone(), path)
        })
        .collect()
}

/// Whether Mac captures of a dev site go straight to its session (`autoSend`, default on).
/// `jevRouting` in config.json (never read in tests).
pub fn jev_routing_config() -> Option<serde_json::Value> {
    if skip_in_test() {
        return None;
    }
    config().jev_routing
}

/// `jevApiKey` in config.json (never read in tests).
pub fn jev_api_key() -> Option<String> {
    if skip_in_test() {
        return None;
    }
    config().jev_api_key
}

pub fn auto_send() -> bool {
    config().auto_send()
}

/// How a capture enters a busy OpenCode session (`"delivery"`): `"steer"` joins its current
/// turn, `"queue"` waits until the turn ends. Anything else is logged once and steers.
pub fn delivery_mode() -> &'static str {
    if skip_in_test() {
        return "steer";
    }
    delivery_mode_of(config().delivery.as_ref())
}

fn delivery_mode_of(value: Option<&serde_json::Value>) -> &'static str {
    match value.and_then(serde_json::Value::as_str) {
        None if value.is_none() => "steer",
        Some("steer") => "steer",
        Some("queue") => "queue",
        _ => {
            log_config_once(format!(
                "config.json: delivery {} isn't \"steer\" or \"queue\"; steering",
                value.map(ToString::to_string).unwrap_or_default()
            ));
            "steer"
        }
    }
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

pub const DEFAULT_LANGUAGE: &str = "en";

pub fn language_code(value: &str) -> Option<String> {
    let value = value.trim();
    if value.is_empty() {
        return Some(DEFAULT_LANGUAGE.into());
    }
    (value.len() <= 35
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-')
        && value.as_bytes()[0].is_ascii_alphabetic())
    .then(|| value.to_ascii_lowercase())
}

pub fn locale_id(value: &str) -> Option<String> {
    let value = value.trim();
    let shaped = value.starts_with(|c: char| c.is_ascii_alphabetic())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    (shaped && !value.eq_ignore_ascii_case("auto")).then(|| value.replace('_', "-"))
}

/// The configured Hex language, defaulting to the previous English behavior.
pub fn hex_language() -> String {
    hex_language_from(&config())
}

pub fn hex_language_from(config: &EngineConfig) -> String {
    config
        .hex_language
        .as_deref()
        .and_then(language_code)
        .unwrap_or_else(|| DEFAULT_LANGUAGE.into())
}

/// The configured Apple Speech locale, or the Mac's own language when unset.
pub fn apple_locale() -> Option<String> {
    apple_locale_from(&config())
}

pub fn apple_locale_from(config: &EngineConfig) -> Option<String> {
    config.apple_locale.as_deref().and_then(locale_id)
}

/// Sets one top-level key in `path` (`null` removes it), keeping the rest (and its `0600` mode,
/// since it holds the ingest token).
pub fn set_config_value_in(path: &Path, key: &str, value: serde_json::Value) -> Result<()> {
    let mut config = fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .filter(serde_json::Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}));
    match (value, config.as_object_mut()) {
        (serde_json::Value::Null, Some(map)) => {
            map.remove(key);
        }
        (value, _) => config[key] = value,
    }
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("json.tmp");
    let mut body = serde_json::to_vec_pretty(&config)?;
    body.push(b'\n');
    fs::write(&tmp, body)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&tmp, fs::Permissions::from_mode(0o600))?;
    }
    fs::rename(&tmp, path).with_context(|| format!("Could not write {}", path.display()))
}

/// Bundle IDs whose `Documents/Scrawl` (and legacy `Documents/Tether`) container the USB watcher
/// checks (`usbApps` in config, with `tetherApps` accepted as a fallback alias; default empty so
/// USB polling is off unless configured).
pub fn usb_apps() -> Vec<String> {
    config()
        .usb_apps
        .into_iter()
        .map(|s| s.trim().to_owned())
        .filter(|s| safe_word(s))
        .collect()
}

/// Safe to put unquoted in a remote shell command (host names, `~/bin/tool` paths).
fn safe_word(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.@/~".contains(&b))
}

/// The Mac whose OpenCode server holds Kit's sessions when it isn't this one: an SSH host from
/// `SCRAWL_AGENT_HOST` or `~/.config/scrawl/config.json` (`{"agentHost": "studio"}`).
pub fn agent_host() -> Option<String> {
    std::env::var("SCRAWL_AGENT_HOST")
        .ok()
        .or_else(|| config().agent_host)
        .filter(|h| safe_word(h))
}

/// The `opencode2` command to run on the agent host (`agentOpencode`, default `opencode2`).
fn agent_opencode() -> String {
    config()
        .agent_opencode
        .filter(|c| safe_word(c))
        .unwrap_or_else(|| "opencode2".to_owned())
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// A system command: `/usr/bin/<name>` on macOS; on Windows the copy in System32 (OpenSSH and
/// bsdtar ship with Windows 10 and later), found through `PATH`.
fn system_tool(name: &str) -> PathBuf {
    if cfg!(windows) {
        on_path(name).unwrap_or_else(|| PathBuf::from(name))
    } else {
        Path::new("/usr/bin").join(name)
    }
}

fn ssh(host: &str) -> Command {
    let mut command = Command::new(system_tool("ssh"));
    command.args(["-o", "BatchMode=yes", "-o", "ConnectTimeout=8", host]);
    command
}

/// Bundle files too big to hold up queueing the session; they follow in the background.
pub const HEAVY_MEDIA: [&str; 5] = [
    "recording.mp4",
    "recording.mov",
    "audio.wav",
    "frames-full",
    "contact-sheet-full.png",
];
/// Present in a copied bundle while its heavy media is still arriving.
pub const MEDIA_PENDING: &str = ".media-pending";
/// Marks a local bundle while its heavy media is being streamed to a remote host.
pub const MEDIA_SENDING: &str = ".media-sending";

/// Copies a bundle to the same path on `host`, so the agent there can open its files.
/// Syncs the contact sheet, keyframe PNGs, manifest, and summary synchronously so the session
/// can be queued immediately, then streams heavy media (`recording.mp4`/`.mov`, `frames-full`)
/// in the background for `scrawl zoom`, marked by [`MEDIA_PENDING`] until it lands.
pub fn sync_bundle_to(host: &str, bundle_dir: &Path) -> Result<()> {
    use std::process::Stdio;
    let parent = bundle_dir.parent().context("Bundle has no parent")?;
    let name = bundle_dir.file_name().context("Bundle has no name")?;
    let mut tar = Command::new(system_tool("tar"))
        .arg("-C")
        .arg(parent)
        .args([
            "--exclude=*/recording.mp4",
            "--exclude=*/recording.mov",
            "--exclude=*/audio.wav",
            "--exclude=*/frames-full",
            "--exclude=*/contact-sheet-full.png",
            "--exclude=*/.media-sending",
        ])
        .arg("-cf")
        .arg("-")
        .arg(name)
        .stdout(Stdio::piped())
        .spawn()
        .context("Could not start tar")?;
    let parent_q = shell_quote(&parent.to_string_lossy());
    let name_str = name.to_string_lossy().into_owned();
    let heavy: Vec<String> = HEAVY_MEDIA
        .iter()
        .filter(|rel| bundle_dir.join(rel).exists())
        .map(|rel| format!("{name_str}/{rel}"))
        .collect();
    let bundle_q = shell_quote(&parent.join(&name_str).to_string_lossy());
    // The marker goes up with the light files, before the session is prompted, so `scrawl zoom`
    // there knows the video is still on its way rather than missing.
    let mark_pending = if heavy.is_empty() {
        String::new()
    } else {
        format!(" && touch {bundle_q}/{MEDIA_PENDING}")
    };
    let output = ssh(host)
        .arg(format!(
            "mkdir -p {parent_q} && tar -C {parent_q} -xf -{mark_pending}"
        ))
        .stdin(tar.stdout.take().context("No tar output")?)
        .output()
        .context("Could not start ssh")?;
    let _ = tar.wait();
    ensure!(
        output.status.success(),
        "Copying the bundle to {host} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );

    if !heavy.is_empty() {
        let host_owned = host.to_owned();
        let parent_owned = parent.to_path_buf();
        // Unpack into a staging folder and move into the bundle only once complete, so nothing on
        // the other side ever reads a half-copied video; then clear the marker.
        let staging_q = shell_quote(
            &parent
                .join(format!(".{name_str}.incoming"))
                .to_string_lossy(),
        );
        let remote = format!(
            "rm -rf {staging_q} && mkdir -p {staging_q} && tar -C {staging_q} -xf - && \
             for f in {staging_q}/*/*; do rm -rf {bundle_q}/\"$(basename \"$f\")\" && mv \"$f\" {bundle_q}/; done; \
             rm -rf {staging_q} {bundle_q}/{MEDIA_PENDING}"
        );
        // Held until the media lands, so the iPhone video isn't re-encoded or cleaned up under
        // the copy (see `retention`).
        let sending = bundle_dir.join(MEDIA_SENDING);
        let _ = fs::write(&sending, host);
        std::thread::spawn(move || {
            let _done = RemoveOnDrop(sending);
            let Ok(mut bg_tar) = Command::new(system_tool("tar"))
                .arg("-C")
                .arg(&parent_owned)
                .arg("-cf")
                .arg("-")
                .args(&heavy)
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            else {
                return;
            };
            if let Some(stdout) = bg_tar.stdout.take() {
                let _ = ssh(&host_owned)
                    .arg(remote)
                    .stdin(stdout)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
            let _ = bg_tar.wait();
        });
    }
    Ok(())
}

struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

fn opencode_api(method: &str, path: &str, data: Option<&serde_json::Value>) -> Result<Vec<u8>> {
    opencode_api_for(
        method,
        path,
        data,
        agent_host().as_deref(),
        &agent_opencode(),
        None,
    )
}

pub fn opencode_api_with_config(
    method: &str,
    path: &str,
    data: Option<&serde_json::Value>,
    config: Option<&crate::RouterConfig>,
    timeout: Option<std::time::Duration>,
) -> Result<Vec<u8>> {
    let host = config
        .and_then(|c| c.agent_host.clone())
        .or_else(|| std::env::var("SCRAWL_AGENT_HOST").ok())
        .filter(|h| safe_word(h));
    let opencode = config
        .and_then(|c| c.agent_opencode.clone())
        .filter(|c| safe_word(c))
        .unwrap_or_else(|| "opencode2".to_owned());
    opencode_api_for(method, path, data, host.as_deref(), &opencode, timeout)
}

fn opencode_api_for(
    method: &str,
    path: &str,
    data: Option<&serde_json::Value>,
    host: Option<&str>,
    opencode: &str,
    timeout: Option<std::time::Duration>,
) -> Result<Vec<u8>> {
    let mut command = if let Some(host) = host {
        let mut remote = format!(
            "PATH=\"$HOME/bin:$HOME/.opencode/bin:/opt/homebrew/bin:$PATH\" {} api {} {}",
            opencode,
            shell_quote(method),
            shell_quote(path)
        );
        if let Some(data) = data {
            remote.push_str(&format!(
                " --data {}",
                shell_quote(&serde_json::to_string(data)?)
            ));
        }
        let mut command = ssh(host);
        command.arg(remote);
        command
    } else {
        let mut command = Command::new(opencode_binary()?);
        command.args(["api", method, path]);
        if let Some(data) = data {
            command.args(["--data", &serde_json::to_string(data)?]);
        }
        command
    };
    let output = match timeout {
        Some(timeout) => crate::routing::run_with_timeout(&mut command, timeout)
            .context("OpenCode API timed out or could not launch opencode2")?,
        None => command.output().context("Could not launch opencode2")?,
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let message = format!("OpenCode API failed: {}", stderr.trim());
        // `opencode api` prints `HTTP <status> <text>` when the server answered.
        return Err(match cli_status(&stderr) {
            Some(status) => anyhow::Error::new(HttpStatus(status)).context(message),
            None => anyhow::anyhow!(message),
        });
    }
    Ok(output.stdout)
}

/// The status in `opencode api`'s `HTTP 409 Conflict` line.
fn cli_status(stderr: &str) -> Option<u16> {
    stderr.lines().find_map(|line| {
        let code = line.trim().strip_prefix("HTTP ")?.get(..3)?;
        code.parse().ok().filter(|c| (100..600).contains(c))
    })
}

pub fn query_escape(value: &str) -> String {
    let mut result = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            result.push(char::from(byte));
        } else {
            result.push_str(&format!("%{byte:02X}"));
        }
    }
    result
}

/// One `GET` against OpenCode's API, returning the body. Swapped for a fake in tests.
pub type ApiGet<'a> = dyn FnMut(&str) -> Result<Vec<u8>> + 'a;

pub fn api_get(path: &str) -> Result<Vec<u8>> {
    if agent_host().is_none()
        && let Ok(body) = api_get_within(path, std::time::Duration::from_secs(5))
    {
        return Ok(body);
    }
    opencode_api("get", path, None)
}

/// How long a TCP connect to the local OpenCode server may take before giving up.
pub const SERVER_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(400);
const SERVER_RESPONSE_LIMIT: u64 = 16 * 1024 * 1024;
const DEFAULT_SERVER_PORT: u16 = 0xc0de;

/// Only the connection fields of OpenCode's `service.json` (never its `env` block).
#[derive(Default, Deserialize)]
struct ServiceFile {
    url: Option<String>,
    hostname: Option<String>,
    port: Option<u16>,
    password: Option<String>,
}

impl ServiceFile {
    fn load(path: &Path) -> Option<Self> {
        serde_json::from_slice(&fs::read(path).ok()?).ok()
    }

    fn address(&self) -> Option<std::net::SocketAddr> {
        self.url.as_deref().and_then(server_address).or_else(|| {
            let host = self.hostname.as_deref().unwrap_or("127.0.0.1");
            server_address(&format!("http://{host}:{}", self.port?))
        })
    }

    fn password(&self) -> Option<String> {
        non_empty(self.password.as_deref())
    }
}

fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// The socket address in `http://127.0.0.1:4096…` when its host is an IP or `localhost` (no
/// lookups).
fn server_address(url: &str) -> Option<std::net::SocketAddr> {
    let parsed = crate::routing::parse_url(url.split_whitespace().next()?)?;
    let ip = if parsed.host == "localhost" {
        std::net::IpAddr::from([127, 0, 0, 1])
    } else {
        parsed.host.parse().ok()?
    };
    Some(std::net::SocketAddr::new(ip, parsed.port))
}

fn resolve_server(
    files: &[ServiceFile],
    env_password: Option<&str>,
) -> (std::net::SocketAddr, Option<String>) {
    let (address, matched_password) = files
        .iter()
        .find_map(|f| Some((f.address()?, f.password())))
        .unwrap_or_else(|| {
            (
                std::net::SocketAddr::from(([127, 0, 0, 1], DEFAULT_SERVER_PORT)),
                None,
            )
        });
    let password = matched_password
        .or_else(|| non_empty(env_password))
        .or_else(|| files.iter().find_map(ServiceFile::password));
    (address, password)
}

/// Finds the local OpenCode V2 server from its state registration (`$XDG_STATE_HOME/opencode/service.json`
/// or `~/.local/state/opencode/service.json`), service config (`$OPENCODE_CONFIG_DIR/service.json`
/// or `~/.config/opencode/service.json`), and `OPENCODE_PASSWORD`.
fn local_server() -> (std::net::SocketAddr, Option<String>) {
    let env_dir = |name: &str| {
        std::env::var_os(name)
            .filter(|dir| !dir.is_empty())
            .map(PathBuf::from)
    };
    let home = dirs::home_dir();
    let files: Vec<ServiceFile> = [
        env_dir("XDG_STATE_HOME").map(|d| d.join("opencode/service.json")),
        home.as_ref()
            .map(|h| h.join(".local/state/opencode/service.json")),
        env_dir("OPENCODE_CONFIG_DIR").map(|d| d.join("service.json")),
        env_dir("XDG_CONFIG_HOME").map(|d| d.join("opencode/service.json")),
        home.as_ref()
            .map(|h| h.join(".config/opencode/service.json")),
    ]
    .into_iter()
    .flatten()
    .filter_map(|p| ServiceFile::load(&p))
    .collect();
    let env_password = std::env::var("OPENCODE_PASSWORD")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| std::env::var("OPENCODE_SERVER_PASSWORD").ok());
    resolve_server(&files, env_password.as_deref())
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for c in bytes.chunks(3) {
        let n = u32::from_be_bytes([0, c[0], *c.get(1).unwrap_or(&0), *c.get(2).unwrap_or(&0)]);
        let ch = |shift: u32| char::from(TABLE[((n >> shift) & 63) as usize]);
        out.extend([
            ch(18),
            ch(12),
            if c.len() > 1 { ch(6) } else { '=' },
            if c.len() > 2 { ch(0) } else { '=' },
        ]);
    }
    out
}

fn http_get(
    address: std::net::SocketAddr,
    password: Option<&str>,
    path: &str,
    timeout: std::time::Duration,
) -> Result<Vec<u8>> {
    http_request(address, password, "GET", path, None, timeout)
}

/// A non-2xx answer from OpenCode, so callers can tell a missing session (404) from a server
/// that can't be reached.
#[derive(Debug)]
pub struct HttpStatus(pub u16);

impl std::fmt::Display for HttpStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OpenCode API failed: HTTP {}", self.0)
    }
}

impl std::error::Error for HttpStatus {}

/// The status of a failed request, when the server answered.
pub fn http_status(err: &anyhow::Error) -> Option<u16> {
    err.downcast_ref::<HttpStatus>().map(|s| s.0)
}

fn http_request(
    address: std::net::SocketAddr,
    password: Option<&str>,
    method: &str,
    path: &str,
    body: Option<&[u8]>,
    timeout: std::time::Duration,
) -> Result<Vec<u8>> {
    use std::io::{Read, Write};
    let connect = SERVER_CONNECT_TIMEOUT
        .min(timeout)
        .max(std::time::Duration::from_millis(1));
    let timeout = timeout.max(std::time::Duration::from_millis(1));
    let mut stream = std::net::TcpStream::connect_timeout(&address, connect)
        .with_context(|| format!("Could not reach server at http://{address}"))?;
    stream.set_write_timeout(Some(timeout))?;
    stream.set_read_timeout(Some(timeout))?;
    let auth = match password {
        Some(password) if !password.contains(['\r', '\n']) => format!(
            "Authorization: Basic {}\r\n",
            base64(format!("opencode:{password}").as_bytes())
        ),
        _ => String::new(),
    };
    let content = match body {
        Some(body) => format!(
            "Content-Type: application/json\r\nContent-Length: {}\r\n",
            body.len()
        ),
        None if method != "GET" => "Content-Length: 0\r\n".to_owned(),
        None => String::new(),
    };
    let mut request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {address}\r\n{auth}{content}Connection: close\r\n\r\n"
    )
    .into_bytes();
    request.extend_from_slice(body.unwrap_or_default());
    stream
        .write_all(&request)
        .with_context(|| format!("Could not write to server at http://{address}"))?;
    let mut bytes = Vec::new();
    stream
        .take(SERVER_RESPONSE_LIMIT + 1)
        .read_to_end(&mut bytes)
        .with_context(|| format!("OpenCode at http://{address} didn't answer in time"))?;
    ensure!(
        bytes.len() as u64 <= SERVER_RESPONSE_LIMIT,
        "OpenCode response too large"
    );
    let split = bytes
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .context("Invalid OpenCode HTTP response")?;
    let header = std::str::from_utf8(&bytes[..split]).context("Invalid OpenCode HTTP header")?;
    let status: u16 = header
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .context("Missing OpenCode HTTP status")?
        .parse()
        .context("Invalid OpenCode HTTP status")?;
    let body = bytes[split + 4..].to_vec();
    if let Some(length) = header
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, value)| value.trim().parse::<usize>().ok())
    {
        ensure!(body.len() == length, "Incomplete OpenCode HTTP response");
    }
    if !(200..300).contains(&status) {
        return Err(HttpStatus(status).into());
    }
    Ok(body)
}

/// One request to the local OpenCode server (`method` is `GET`, `POST`, `DELETE`…) within
/// `timeout`, through `opencode2 api` when the agents run on another Mac. A 4xx or 5xx answer is
/// an [`HttpStatus`] error.
pub fn api_within(
    method: &str,
    path: &str,
    data: Option<&serde_json::Value>,
    timeout: std::time::Duration,
) -> Result<Vec<u8>> {
    api_within_with_config(method, path, data, timeout, None)
}

pub fn api_within_with_config(
    method: &str,
    path: &str,
    data: Option<&serde_json::Value>,
    timeout: std::time::Duration,
    config: Option<&crate::RouterConfig>,
) -> Result<Vec<u8>> {
    let has_remote_host = config
        .and_then(|c| c.agent_host.as_ref())
        .filter(|h| safe_word(h))
        .is_some()
        || (config.is_none() && agent_host().is_some());
    if has_remote_host {
        return opencode_api_with_config(
            &method.to_ascii_lowercase(),
            path,
            data,
            config,
            Some(timeout),
        );
    }
    let body = data.map(serde_json::to_vec).transpose()?;
    let (address, password) = local_server();
    http_request(
        address,
        password.as_deref(),
        method,
        path,
        body.as_deref(),
        timeout,
    )
}

/// [`api_get`] for routing: talks HTTP directly to the local OpenCode server with a
/// [`SERVER_CONNECT_TIMEOUT`] connect cap and `timeout` read deadline, instead of spawning
/// `opencode2 api` per request.
pub fn api_get_within(path: &str, timeout: std::time::Duration) -> Result<Vec<u8>> {
    api_get_within_with_config(path, timeout, None)
}

pub fn api_get_within_with_config(
    path: &str,
    timeout: std::time::Duration,
    config: Option<&crate::RouterConfig>,
) -> Result<Vec<u8>> {
    let has_remote_host = config
        .and_then(|c| c.agent_host.as_ref())
        .filter(|h| safe_word(h))
        .is_some()
        || (config.is_none() && agent_host().is_some());
    if has_remote_host {
        return opencode_api_with_config("get", path, None, config, Some(timeout));
    }
    let (address, password) = local_server();
    http_get(address, password.as_deref(), path, timeout)
}

/// How many pages of root sessions (100 each, newest first) a search reads before giving up.
pub const ROOT_SEARCH_PAGES: usize = 5;
const ROOT_PAGE: usize = 100;

/// One session by ID (`GET /api/session/{id}`).
pub fn get_session(get: &mut ApiGet, id: &str) -> Result<OpenCodeSession> {
    let response: serde_json::Value =
        serde_json::from_slice(&get(&format!("/api/session/{}", query_escape(id)))?)
            .context("Invalid OpenCode session response")?;
    let session = response.get("data").unwrap_or(&response).clone();
    Ok(serde_json::from_value(session)?)
}

/// What a root-session search does with each open root session it reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scan {
    Skip,
    Take,
    Stop,
}

/// Reads open root sessions newest first (`query` adds filters, e.g. `&directory=…`) and returns
/// the first that `scan` takes, reading at most `max_pages` pages.
pub fn find_root_session(
    get: &mut ApiGet,
    query: &str,
    max_pages: usize,
    mut scan: impl FnMut(&OpenCodeSession) -> Scan,
) -> Result<Option<OpenCodeSession>> {
    let mut path = format!("/api/session?parentID=null&limit={ROOT_PAGE}{query}");
    let mut cursors = std::collections::HashSet::new();
    for _ in 0..max_pages {
        let response: serde_json::Value =
            serde_json::from_slice(&get(&path)?).context("Invalid OpenCode session response")?;
        let data = response
            .as_array()
            .or_else(|| response.get("data").and_then(|data| data.as_array()))
            .context("Missing OpenCode session list")?;
        for session in data
            .iter()
            .filter_map(|s| serde_json::from_value::<OpenCodeSession>(s.clone()).ok())
        {
            // The API has no archived filter.
            if !session.is_open_root() {
                continue;
            }
            match scan(&session) {
                Scan::Skip => {}
                Scan::Take => return Ok(Some(session)),
                Scan::Stop => return Ok(None),
            }
        }
        let Some(next) = response.pointer("/cursor/next").and_then(|v| v.as_str()) else {
            break;
        };
        if data.is_empty() || !cursors.insert(next.to_owned()) {
            break;
        }
        // The cursor carries the filters.
        path = format!(
            "/api/session?cursor={}&limit={ROOT_PAGE}",
            query_escape(next)
        );
    }
    Ok(None)
}

pub fn normalize_slug(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Sends a bundle to `session_id`, else to the session its recording was addressed to, else the
/// session the app resolves to. Sending an addressed recording elsewhere needs `reroute`.
pub fn send_to_opencode(
    bundle_dir: &Path,
    session_id: Option<&str>,
    reroute: bool,
) -> Result<String> {
    let addressed = recorded_destination(&resolve_bundle_dir(bundle_dir)?);
    let session_id = choose_destination(session_id, addressed.as_deref(), reroute)?;
    let (id, _) = send_to_opencode_for_app(bundle_dir, session_id, None)?;
    Ok(id)
}

/// The session a recording was addressed to: a ScrawlKit `timeline.json`'s top-level `session`,
/// else the `destination` a Mac capture of a dev site was routed to (`scrawl.json`).
/// `context.session` only names the session that was on screen, and never chooses a destination.
pub fn recorded_destination(bundle_dir: &Path) -> Option<String> {
    let read = |name: &str| -> Option<serde_json::Value> {
        serde_json::from_slice(&fs::read(bundle_dir.join(name)).ok()?).ok()
    };
    let session = |value: Option<&serde_json::Value>| {
        value
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    };
    read("timeline.json")
        .and_then(|timeline| session(timeline.get("session")))
        .or_else(|| read("scrawl.json").and_then(|b| session(b.pointer("/destination/session"))))
}

fn choose_destination<'a>(
    requested: Option<&'a str>,
    addressed: Option<&'a str>,
    reroute: bool,
) -> Result<Option<&'a str>> {
    match (requested, addressed) {
        (Some(requested), Some(addressed)) if requested != addressed && !reroute => bail!(
            "This recording was addressed to {addressed}; pass --reroute to send it to {requested} instead"
        ),
        (Some(requested), _) => Ok(Some(requested)),
        (None, addressed) => Ok(addressed),
    }
}

pub fn send_to_opencode_for_app(
    bundle_dir: &Path,
    session_id: Option<&str>,
    app_name: Option<&str>,
) -> Result<(String, String)> {
    let (_, bundle) = load_bundle(bundle_dir)?;
    let (session, title) =
        resolve_opencode_session(session_id, app_name.or(bundle.target.app.as_deref()))?;
    send_bundle_to_session(bundle_dir, &session)?;
    Ok((session, title))
}

/// Picks the session a capture goes to, and its title: an explicit ID, else the root session
/// whose title matches the app ("UpDown" → "UP DOWN APP"), else `OPENCODE_SESSION_ID`, else the
/// most recently updated root session.
pub fn resolve_opencode_session(
    session_id: Option<&str>,
    app_name: Option<&str>,
) -> Result<(String, String)> {
    let env_session = std::env::var("OPENCODE_SESSION_ID").ok();
    resolve_opencode_session_with(&mut api_get, session_id, app_name, env_session.as_deref())
}

/// [`resolve_opencode_session`] with the API and `OPENCODE_SESSION_ID` passed in. An explicit ID
/// costs one request; an app title match reads the newest [`ROOT_SEARCH_PAGES`] pages of root
/// sessions at most.
pub fn resolve_opencode_session_with(
    get: &mut ApiGet,
    session_id: Option<&str>,
    app_name: Option<&str>,
    env_session: Option<&str>,
) -> Result<(String, String)> {
    if let Some(id) = session_id.filter(|s| !s.trim().is_empty()) {
        let title = get_session(get, id)
            .ok()
            .map(|s| s.title)
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| id.to_owned());
        return Ok((id.to_owned(), title));
    }

    // Match against the last component if a bundle identifier was passed (com.kitlangton.UpDown -> updown).
    let short = app_name
        .and_then(|a| a.rsplit('.').next())
        .map(normalize_slug)
        .filter(|s| !s.is_empty())
        .or_else(|| app_name.map(normalize_slug).filter(|s| !s.is_empty()));
    let mut newest = None;
    let matched = find_root_session(get, "", ROOT_SEARCH_PAGES, |s| {
        if newest.is_none() {
            newest = Some(s.clone());
        }
        let Some(short) = short.as_deref() else {
            return Scan::Stop;
        };
        let title_slug = normalize_slug(&s.title);
        if !title_slug.is_empty() && (title_slug.contains(short) || short.contains(&title_slug)) {
            Scan::Take
        } else {
            Scan::Skip
        }
    })?;
    let matched = matched
        .or_else(|| {
            let id = env_session.filter(|id| !id.trim().is_empty())?;
            get_session(get, id)
                .ok()
                .filter(OpenCodeSession::is_open_root)
        })
        .or(newest)
        .context("No root OpenCode session found; pass a session ID")?;
    let title = matched.label().to_owned();
    Ok((matched.id, title))
}

/// Queues a bundle's summary and pointers into an OpenCode session, and remembers the send so
/// it can be undone (`scrawl router undo`).
pub fn send_bundle_to_session(bundle_dir: &Path, session: &str) -> Result<()> {
    let posted = send_bundle(bundle_dir, session)?;
    let app = load_bundle(bundle_dir)
        .ok()
        .and_then(|(_, bundle)| bundle.target.app);
    crate::routing::undo::remember(
        session,
        None,
        &posted,
        crate::routing::undo::Origin {
            kind: "capture".into(),
            bundle: resolve_bundle_dir(bundle_dir).ok(),
            app,
            ..Default::default()
        },
    );
    Ok(())
}

/// What OpenCode said about a prompt it took.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Posted {
    /// The inbox item's ID, which the message keeps once delivered (`msg_…`).
    pub message_id: Option<String>,
    /// `steer` or `queue`.
    pub delivery: String,
    /// Whether the session was idle just before (unknown when the check failed).
    pub was_idle: Option<bool>,
}

/// How long the idle check before a send may take.
const IDLE_CHECK: std::time::Duration = std::time::Duration::from_millis(600);

/// Whether `session` has no foreground drain running (`GET /api/session/active`).
fn session_idle(
    session: &str,
    config: Option<&crate::RouterConfig>,
    timeout: std::time::Duration,
) -> Option<bool> {
    let body = api_get_within_with_config("/api/session/active", timeout, config).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&body).ok()?;
    let active = value.get("data").unwrap_or(&value).as_object()?;
    Some(!active.contains_key(session))
}

/// The inbox item's ID in `POST /api/session/:id/prompt`'s answer (`{"data": {"id"}}`).
pub fn posted_id(body: &[u8]) -> Option<String> {
    let value: serde_json::Value = serde_json::from_slice(body).ok()?;
    let id = value.get("data").unwrap_or(&value).get("id")?.as_str()?;
    id.starts_with("msg_").then(|| id.to_owned())
}

fn post_prompt(session: &str, body: serde_json::Value) -> Result<Posted> {
    post_prompt_for(session, body, delivery_mode(), None, None)
}

pub fn post_prompt_with_config(
    session: &str,
    body: serde_json::Value,
    config: &crate::RouterConfig,
    timeout: Option<std::time::Duration>,
) -> Result<Posted> {
    post_prompt_for(
        session,
        body,
        config.delivery.as_str(),
        Some(config),
        timeout,
    )
}

fn post_prompt_for(
    session: &str,
    mut body: serde_json::Value,
    delivery: &str,
    config: Option<&crate::RouterConfig>,
    timeout: Option<std::time::Duration>,
) -> Result<Posted> {
    body["delivery"] = serde_json::json!(delivery);
    let idle_budget = timeout.map_or(IDLE_CHECK, |t| {
        IDLE_CHECK
            .min(t / 3)
            .max(std::time::Duration::from_millis(50))
    });
    let was_idle = session_idle(session, config, idle_budget);
    let answer = opencode_api_with_config(
        "post",
        &format!("/api/session/{session}/prompt"),
        Some(&body),
        config,
        timeout,
    )?;
    Ok(Posted {
        message_id: posted_id(&answer),
        delivery: delivery.to_owned(),
        was_idle,
    })
}

/// [`send_bundle_to_session`] without remembering it.
pub fn send_bundle(bundle_dir: &Path, session: &str) -> Result<Posted> {
    send_bundle_for(bundle_dir, session, None, None)
}

pub fn send_bundle_with_config(
    bundle_dir: &Path,
    session: &str,
    config: &crate::RouterConfig,
    timeout: Option<std::time::Duration>,
) -> Result<Posted> {
    send_bundle_for(bundle_dir, session, Some(config), timeout)
}

fn send_bundle_for(
    bundle_dir: &Path,
    session: &str,
    config: Option<&crate::RouterConfig>,
    timeout: Option<std::time::Duration>,
) -> Result<Posted> {
    let (directory, bundle) = load_bundle(bundle_dir)?;
    let summary = crate::sketch::summary_markdown(&bundle, &directory);
    ensure!(
        session.starts_with("ses")
            && session
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
        "Invalid OpenCode session ID"
    );
    let remote_host = config
        .and_then(|c| c.agent_host.clone())
        .filter(|h| safe_word(h))
        .or_else(|| if config.is_none() { agent_host() } else { None });
    if let Some(host) = remote_host {
        sync_bundle_to(&host, &directory)?;
    }
    let quoted_directory = shell_quote(&directory.to_string_lossy());
    let contact_sheet = directory.join(&bundle.contact_sheet);
    let text = format!(
        "{}\n\nThe contact sheet is attached ({}); read {} for timestamped transcript, marks, and interactions.\nTo look closely at a moment (an animation, a glitch, what the narration points at), run:\nscrawl zoom {} --at <M:SS or \"a phrase that was said\"> [--span 2] [--frames 12]\nIt saves a dense contact sheet of that moment with each frame's time and prints what was said; open the PNG before acting on the feedback. This capture is {} ms long.",
        summary.trim_end(),
        contact_sheet.display(),
        directory.join("scrawl.json").display(),
        quoted_directory,
        bundle.duration_ms,
    );
    let body = serde_json::json!({
        "text": text,
        "files": [{
            "uri": file_uri(&contact_sheet),
            "name": "contact-sheet.png",
            "description": "Scrawl contact sheet: the capture's keyframes in order",
        }],
    });
    match config {
        Some(cfg) => post_prompt_with_config(session, body, cfg, timeout),
        None => post_prompt(session, body),
    }
}

/// Sends `text` into an OpenCode session as a prompt (`scrawl router deliver`), steering or
/// queueing per `delivery_mode`.
pub fn send_text_to_session(session: &str, text: &str) -> Result<Posted> {
    ensure!(
        session.starts_with("ses")
            && session
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
        "Invalid OpenCode session ID"
    );
    post_prompt(session, serde_json::json!({ "text": text }))
}

pub fn send_text_with_config(
    session: &str,
    text: &str,
    config: &crate::RouterConfig,
    timeout: Option<std::time::Duration>,
) -> Result<Posted> {
    ensure!(
        session.starts_with("ses")
            && session
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
        "Invalid OpenCode session ID"
    );
    post_prompt_with_config(
        session,
        serde_json::json!({ "text": text }),
        config,
        timeout,
    )
}

/// A `file://` URL for an absolute path, percent-encoding everything but unreserved
/// characters and `/` ("Application Support" → "Application%20Support"). A Windows path
/// (`C:\Users\…`, or `\\?\C:\…` after `canonicalize`) becomes `file:///C:/Users/…`.
fn file_uri(path: &Path) -> String {
    let mut text = path.to_string_lossy().into_owned();
    let mut uri = String::from("file://");
    let windows_path = cfg!(windows) || text.as_bytes().get(1) == Some(&b':');
    if windows_path {
        if let Some(rest) = text.strip_prefix(r"\\?\") {
            text = rest.to_owned();
        }
        text = text.replace('\\', "/");
        if !text.starts_with('/') {
            uri.push('/');
        }
    }
    for (index, byte) in text.bytes().enumerate() {
        let drive_colon = windows_path && index == 1 && byte == b':';
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) || drive_colon {
            uri.push(char::from(byte));
        } else {
            uri.push_str(&format!("%{byte:02X}"));
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    #[test]
    fn opencode_api_statuses_survive_the_cli() {
        assert_eq!(super::cli_status("HTTP 409 Conflict\n"), Some(409));
        assert_eq!(super::cli_status("warn: x\nHTTP 404 Not Found"), Some(404));
        assert_eq!(super::cli_status("error: connect ECONNREFUSED"), None);
        let err = anyhow::Error::new(super::HttpStatus(404)).context("OpenCode API failed");
        assert_eq!(super::http_status(&err), Some(404));
        assert_eq!(super::http_status(&anyhow::anyhow!("down")), None);
    }

    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "scrawl-handoff-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn bundle(&self, name: &str, created: u64) -> PathBuf {
            let path = self.0.join(format!("{name}.scrawl"));
            fs::create_dir_all(path.join("frames")).unwrap();
            let keyframes: Vec<_> = [0, 1_900, 2_500, 4_000, 5_100, 6_000].into_iter().enumerate().map(|(index, t)| {
                let file = format!("frames/frame-{index:03}.png");
                fs::write(path.join(&file), b"fixture").unwrap();
                serde_json::json!({"index": index, "t": t, "reason": "initial", "file": file, "event": ""})
            }).collect();
            let bundle = serde_json::json!({
                "version": 1, "id": name, "createdAtMs": created, "durationMs": 7_000,
                "target": {"mode": "display", "displayId": 1,
                    "frame": {"x": 0.0, "y": 0.0, "width": 100.0, "height": 100.0, "scale": 1.0}},
                "transcript": {"source": "none", "text": "", "segments": []},
                "strokes": [], "marks": [], "events": [], "keyframes": keyframes,
                "contactSheet": "contact-sheet.png"
            });
            fs::write(
                path.join("scrawl.json"),
                serde_json::to_vec(&bundle).unwrap(),
            )
            .unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn resolves_directory_manifest_and_newest_capture() {
        let fixture = Fixture::new();
        let older = fixture.bundle("old", 100);
        let newer = fixture.bundle("new", 200);
        assert_eq!(
            resolve_bundle_dir(&older).unwrap(),
            plain_path(older.canonicalize().unwrap())
        );
        assert_eq!(
            resolve_bundle_dir(&older.join("scrawl.json")).unwrap(),
            plain_path(older.canonicalize().unwrap())
        );
        assert_eq!(
            newest_bundle(&fixture.0).unwrap(),
            plain_path(newer.canonicalize().unwrap())
        );
        assert!(resolve_bundle_dir(&fixture.0).is_err());
        assert!(resolve_bundle_dir(&fixture.0.join("missing.scrawl")).is_err());
    }

    #[test]
    fn file_uris_are_percent_encoded() {
        assert_eq!(
            file_uri(Path::new(
                "/Users/kit/Library/Application Support/scrawl/captures/a b#1.scrawl/contact-sheet.png"
            )),
            "file:///Users/kit/Library/Application%20Support/scrawl/captures/a%20b%231.scrawl/contact-sheet.png"
        );
    }

    #[test]
    fn loads_manifest_and_reports_invalid_json() {
        let fixture = Fixture::new();
        let path = fixture.bundle("example", 123);
        let (resolved, bundle) = load_bundle(&path.join("scrawl.json")).unwrap();
        assert_eq!(resolved, plain_path(path.canonicalize().unwrap()));
        assert_eq!(bundle.id, "example");
        assert_eq!(bundle.created_at_ms, 123);
        fs::write(path.join("scrawl.json"), b"not json").unwrap();
        assert!(load_bundle(&path).is_err());
    }

    #[test]
    fn recordings_go_to_their_addressed_session_not_the_viewed_one() {
        let dir = std::env::temp_dir().join(format!("scrawl-dest-{}", crate::make_bundle_id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("timeline.json"),
            r#"{"session":"ses_ios","context":{"session":"ses_motion"}}"#,
        )
        .unwrap();
        let addressed = recorded_destination(&dir);
        assert_eq!(addressed.as_deref(), Some("ses_ios"));
        let addressed = addressed.as_deref();
        assert_eq!(
            choose_destination(None, addressed, false).unwrap(),
            Some("ses_ios")
        );
        assert!(choose_destination(Some("ses_motion"), addressed, false).is_err());
        assert_eq!(
            choose_destination(Some("ses_motion"), addressed, true).unwrap(),
            Some("ses_motion")
        );
        assert_eq!(
            choose_destination(Some("ses_ios"), addressed, false).unwrap(),
            Some("ses_ios")
        );
        assert_eq!(
            choose_destination(Some("ses_x"), None, false).unwrap(),
            Some("ses_x")
        );
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(recorded_destination(&dir), None);
    }

    #[test]
    fn token_generation_persists_with_0600_and_preserves_other_keys() {
        let fixture = Fixture::new();
        let config_path = fixture.0.join("nested/config.json");
        fs::create_dir_all(config_path.parent().unwrap()).unwrap();
        fs::write(
            &config_path,
            br#"{"agentHost":"studio","agentOpencode":"~/bin/opencode2","usbApps":["com.kitlangton.UpDown"],"customKey":42}"#,
        )
        .unwrap();

        let generated = ensure_ingest_token_in(&config_path).unwrap();
        assert_eq!(generated.len(), 64);
        assert!(generated.bytes().all(|b| b.is_ascii_hexdigit()));

        // Calling a second time returns the same token without regenerating.
        let second = ensure_ingest_token_in(&config_path).unwrap();
        assert_eq!(generated, second);

        // Other keys are preserved and parsed properly.
        let parsed = load_config_from(&config_path);
        assert_eq!(parsed.agent_host.as_deref(), Some("studio"));
        assert_eq!(parsed.agent_opencode.as_deref(), Some("~/bin/opencode2"));
        assert_eq!(parsed.ingest_token.as_deref(), Some(generated.as_str()));
        assert_eq!(parsed.usb_apps, vec!["com.kitlangton.UpDown"]);

        let legacy_path = fixture.0.join("nested/legacy.json");
        fs::write(&legacy_path, br#"{"tetherApps":["com.kitlangton.UpDown"]}"#).unwrap();
        assert_eq!(
            load_config_from(&legacy_path).usb_apps,
            vec!["com.kitlangton.UpDown"]
        );

        let raw: serde_json::Value =
            serde_json::from_slice(&fs::read(&config_path).unwrap()).unwrap();
        assert_eq!(raw["customKey"], serde_json::json!(42));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&config_path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }

    #[test]
    fn apple_locale_round_trips_and_keeps_other_keys() {
        let fixture = Fixture::new();
        let path = fixture.0.join("config.json");
        fs::write(&path, br#"{"ingestToken":"abc","autoSend":false}"#).unwrap();
        let config = load_config_from(&path);
        assert_eq!(apple_locale_from(&config), None);
        assert_eq!(config.auto_send, Some(false));

        for (value, expected) in [
            ("zh-TW".into(), Some("zh-TW")),
            ("bad value".into(), None),
            (5.into(), None),
            (serde_json::Value::Null, None),
        ] {
            set_config_value_in(&path, "appleLocale", value).unwrap();
            let config = load_config_from(&path);
            assert_eq!(apple_locale_from(&config).as_deref(), expected);
            assert_eq!(config.ingest_token.as_deref(), Some("abc"));
            assert_eq!(config.auto_send, Some(false));
        }
        let raw: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(raw.get("appleLocale").is_none());
    }

    #[test]
    fn transcriber_preference_round_trips_and_keeps_other_keys() {
        let fixture = Fixture::new();
        let path = fixture.0.join("config.json");
        fs::write(&path, br#"{"ingestToken":"abc","autoSend":false}"#).unwrap();
        let config = load_config_from(&path);
        assert_eq!((&config.transcriber, &config.hex_model), (&None, &None));
        assert_eq!(hex_language_from(&config), "en");
        set_config_value_in(&path, "transcriber", "hex".into()).unwrap();
        set_config_value_in(&path, "hexModel", "parakeet_v3".into()).unwrap();
        set_config_value_in(&path, "hexLanguage", "nl".into()).unwrap();
        let config = load_config_from(&path);
        assert_eq!(config.transcriber.as_deref(), Some("hex"));
        assert_eq!(config.hex_model.as_deref(), Some("parakeet_v3"));
        assert_eq!(hex_language_from(&config), "nl");
        assert_eq!(config.ingest_token.as_deref(), Some("abc"));
        assert_eq!(config.auto_send, Some(false));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn malformed_command_keys_never_reset_the_rest_of_the_config() {
        let fixture = Fixture::new();
        let path = fixture.0.join("config.json");
        fs::write(
            &path,
            br#"{"transcriber":"command","autoSend":false,"transcribeCommand":"whisper {audio}","transcribeTimeoutSeconds":"soon"}"#,
        )
        .unwrap();
        let config = load_config_from(&path);
        assert_eq!(config.transcriber.as_deref(), Some("command"));
        assert_eq!(config.auto_send, Some(false));
    }

    #[test]
    fn captures_steer_unless_delivery_says_queue() {
        use serde_json::json;
        assert_eq!(delivery_mode_of(None), "steer");
        assert_eq!(delivery_mode_of(Some(&json!("steer"))), "steer");
        assert_eq!(delivery_mode_of(Some(&json!("queue"))), "queue");
        assert_eq!(delivery_mode_of(Some(&json!("later"))), "steer");
        assert_eq!(delivery_mode_of(Some(&json!(true))), "steer");
        let config: Config = serde_json::from_str(r#"{"delivery": 3, "autoSend": false}"#).unwrap();
        assert_eq!(
            config.auto_send,
            Some(false),
            "a bad delivery keeps the other keys"
        );
    }

    #[test]
    fn a_bad_routing_key_drops_only_itself() {
        let config: Config = serde_json::from_str(
            r#"{"routing": {"autoSend": "never", "undoWindowSec": "15",
                "sites": {"https://a.example": "~/a", "https://b.example": 3}}}"#,
        )
        .unwrap();
        let routing = config.routing.unwrap();
        assert_eq!(routing.auto_send, Some(serde_json::json!("never")));
        assert_eq!(routing.undo_window_sec, None);
        assert_eq!(
            routing.sites.into_iter().collect::<Vec<_>>(),
            vec![("https://a.example".to_owned(), "~/a".to_owned())]
        );
        let config: Config = serde_json::from_str(r#"{"routing": 3, "autoSend": false}"#).unwrap();
        assert_eq!((config.routing, config.auto_send), (None, Some(false)));
    }

    #[test]
    fn auto_send_defaults_on_and_the_toggle_keeps_other_keys() {
        let fixture = Fixture::new();
        let path = fixture.0.join("config.json");
        fs::write(
            &path,
            br#"{"ingestToken":"abc","retention":{"mediaDays":7}}"#,
        )
        .unwrap();
        assert_eq!(load_config_from(&path).auto_send, None);
        set_config_value_in(&path, "autoSend", serde_json::Value::Bool(false)).unwrap();
        let config = load_config_from(&path);
        assert_eq!(config.auto_send, Some(false));
        assert_eq!(config.ingest_token.as_deref(), Some("abc"));
        let raw: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["retention"]["mediaDays"], 7);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn sessions_read_their_directory_from_v2_location_or_v1_field() {
        let v2: OpenCodeSession = serde_json::from_str(
            r#"{"id":"ses_a","title":"A","time":{"updated":1},"location":{"directory":"/p/a"}}"#,
        )
        .unwrap();
        assert_eq!(v2.directory(), Some("/p/a"));
        assert!(v2.is_open_root());
        let v1: OpenCodeSession =
            serde_json::from_str(r#"{"id":"ses_b","parentID":"ses_a","directory":"/p/b"}"#)
                .unwrap();
        assert_eq!(v1.directory(), Some("/p/b"));
        assert!(!v1.is_open_root());
        assert_eq!(v1.label(), "ses_b");
    }

    #[test]
    fn config_defaults_when_missing_or_minimal() {
        let fixture = Fixture::new();
        let missing = fixture.0.join("missing.json");
        let cfg = load_config_from(&missing);
        assert_eq!(cfg, Config::default());
        assert!(cfg.usb_apps.is_empty());

        let token = ensure_ingest_token_in(&missing).unwrap();
        assert_eq!(token.len(), 64);
        let cfg_after = load_config_from(&missing);
        assert_eq!(cfg_after.ingest_token.as_deref(), Some(token.as_str()));
        assert!(cfg_after.usb_apps.is_empty());
    }

    #[test]
    fn an_unusable_captures_folder_is_refused() {
        let root = std::env::temp_dir().join(format!("scrawl-usable-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let file = root.join("file");
        fs::write(&file, "x").unwrap();
        assert!(!usable_captures_dir(&file.join("inside")));
        assert!(usable_captures_dir(&root.join("new folder")));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn captures_dir_takes_absolute_and_home_paths_only() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(configured_captures_dir("~"), Some(home.clone()));
        assert_eq!(
            configured_captures_dir(" ~/Scrawl Captures "),
            Some(home.join("Scrawl Captures"))
        );
        let absolute = std::env::temp_dir().join("captures");
        assert_eq!(
            configured_captures_dir(&absolute.display().to_string()),
            Some(absolute)
        );
        assert_eq!(configured_captures_dir("captures"), None);
        assert_eq!(configured_captures_dir("~someone/captures"), None);
        assert_eq!(configured_captures_dir("  "), None);
    }

    #[test]
    fn one_bad_setting_never_resets_the_others() {
        let fixture = Fixture::new();
        let path = fixture.0.join("config.json");
        fs::write(
            &path,
            r#"{"agentHost": "studio", "autoSend": "maybe", "usbApps": "not-a-list", "videoExport": true, "nativePill": 3, "pillStyle": "glass", "unknown": {"x": 1}}"#,
        )
        .unwrap();
        let cfg = load_config_from(&path);
        assert_eq!(cfg.agent_host.as_deref(), Some("studio"));
        assert_eq!(cfg.auto_send, None);
        assert!(cfg.usb_apps.is_empty());
        // Both spellings of one field: one is kept rather than neither.
        fs::write(
            &path,
            r#"{"usbApps": ["a"], "tetherApps": ["b"], "agentHost": "pro"}"#,
        )
        .unwrap();
        let cfg = load_config_from(&path);
        assert_eq!(cfg.agent_host.as_deref(), Some("pro"));
        assert_eq!(cfg.usb_apps.len(), 1);
    }

    #[test]
    fn experiments_are_read_from_config_and_off_by_default() {
        let fixture = Fixture::new();
        let path = fixture.0.join("config.json");
        fs::write(
            &path,
            r#"{"experiments": {"agentContext": true, "other": false}}"#,
        )
        .unwrap();
        let cfg = load_config_from(&path);
        assert_eq!(cfg.experiments.get(AGENT_CONTEXT), Some(&true));
        assert_eq!(cfg.experiments.get("other"), Some(&false));
        assert!(Config::default().experiments.is_empty());
        // A bad value skips that experiment, never the file.
        fs::write(
            &path,
            r#"{"autoSend": false, "experiments": {"agentContext": "yes", "a": 1, "b": "maybe", "c": [1], "d": "off"}}"#,
        )
        .unwrap();
        let cfg = load_config_from(&path);
        assert_eq!(cfg.auto_send, Some(false));
        assert_eq!(cfg.experiments.get(AGENT_CONTEXT), Some(&true));
        assert_eq!(cfg.experiments.get("a"), Some(&true));
        assert_eq!(cfg.experiments.get("d"), Some(&false));
        assert!(!cfg.experiments.contains_key("b") && !cfg.experiments.contains_key("c"));
        fs::write(&path, r#"{"autoSend": false, "experiments": true}"#).unwrap();
        let cfg = load_config_from(&path);
        assert_eq!(cfg.auto_send, Some(false));
        assert!(cfg.experiments.is_empty());
        // Tests never read Kit's config, only what this process turned on.
        assert!(!experiment("neverEnabled"));
        enable_experiment("enabledForThisTest");
        assert!(experiment("enabledForThisTest"));
    }

    #[test]
    fn server_addresses_parse_without_lookups() {
        let addr = |s: &str| server_address(s).map(|a| a.to_string());
        assert_eq!(
            addr("http://127.0.0.1:4096\nmore").as_deref(),
            Some("127.0.0.1:4096")
        );
        assert_eq!(
            addr("http://localhost:4096/").as_deref(),
            Some("127.0.0.1:4096")
        );
        assert_eq!(addr("http://[::1]:5000").as_deref(), Some("[::1]:5000"));
        assert_eq!(addr("https://10.0.0.2").as_deref(), Some("10.0.0.2:443"));
        assert_eq!(addr("http://studio.example.ts.net:4096"), None);
        assert_eq!(addr("nonsense"), None);
    }

    #[test]
    fn server_endpoint_reads_state_and_config_service_files() {
        let fixture = Fixture::new();
        let state_path = fixture.0.join("state-service.json");
        let config_path = fixture.0.join("config-service.json");
        fs::write(
            &state_path,
            r#"{"id":"s1","url":"http://127.0.0.1:49374","pid":123,"password":"secret-state"}"#,
        )
        .unwrap();
        fs::write(
            &config_path,
            r#"{"hostname":"127.0.0.1","port":4096,"password":"secret-config","env":{"API_KEY":"ignored"}}"#,
        )
        .unwrap();
        let state = ServiceFile::load(&state_path).unwrap();
        let config = ServiceFile::load(&config_path).unwrap();

        // State registration wins over service config when both exist.
        let (addr, pw) = resolve_server(&[state, config], None);
        assert_eq!(addr, "127.0.0.1:49374".parse().unwrap());
        assert_eq!(pw.as_deref(), Some("secret-state"));

        // When state registration is absent, config's hostname/port and password are used.
        let config = ServiceFile::load(&config_path).unwrap();
        let (addr, pw) = resolve_server(&[config], None);
        assert_eq!(addr, "127.0.0.1:4096".parse().unwrap());
        assert_eq!(pw.as_deref(), Some("secret-config"));

        // With no address in files, the default port and OPENCODE_PASSWORD are used.
        let (addr, pw) = resolve_server(&[], Some("env-pw"));
        assert_eq!(addr, "127.0.0.1:49374".parse().unwrap());
        assert_eq!(pw.as_deref(), Some("env-pw"));
    }

    #[test]
    fn http_get_authenticates_and_times_out_when_slow() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::time::{Duration, Instant};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for i in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut req = Vec::new();
                let mut buf = [0_u8; 256];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = stream.read(&mut buf).unwrap();
                    if n == 0 {
                        break;
                    }
                    req.extend_from_slice(&buf[..n]);
                }
                let head = String::from_utf8_lossy(&req);
                match i {
                    0 => {
                        assert!(
                            head.contains("Authorization: Basic b3BlbmNvZGU6c2VjcmV0\r\n"),
                            "{head}"
                        );
                        let body = br#"{"data":[]}"#;
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .unwrap();
                        stream.write_all(body).unwrap();
                    }
                    1 => {
                        stream
                            .write_all(
                                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                            )
                            .unwrap();
                    }
                    _ => {
                        std::thread::sleep(Duration::from_millis(400));
                    }
                }
            }
        });

        let body = http_get(
            address,
            Some("secret"),
            "/api/session",
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(body, br#"{"data":[]}"#);

        let err = http_get(
            address,
            Some("secret"),
            "/api/session/missing",
            Duration::from_secs(1),
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("HTTP 404"), "{err:#}");

        let started = Instant::now();
        let err = http_get(
            address,
            Some("secret"),
            "/api/session",
            Duration::from_millis(100),
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("didn't answer"), "{err:#}");
        assert!(started.elapsed() < Duration::from_secs(1));
        let _ = server.join();
    }
}

#[cfg(test)]
mod sync_live_check {
    /// Manual: `cargo test sync_to_pro -- --ignored` copies a dummy bundle to `pro` and checks the
    /// heavy media lands atomically and the marker clears.
    #[test]
    #[ignore]
    fn sync_to_pro() {
        let base = std::path::PathBuf::from(std::env::var("HOME").unwrap())
            .join("Library/Caches/scrawl-sync-live");
        let _ = std::fs::remove_dir_all(&base);
        let bundle = base.join("scrawl-synctest.scrawl");
        std::fs::create_dir_all(bundle.join("frames-full")).unwrap();
        std::fs::write(bundle.join("summary.md"), "hi").unwrap();
        std::fs::write(bundle.join("recording.mp4"), vec![7u8; 8_000_000]).unwrap();
        std::fs::write(bundle.join("frames-full/frame-01.png"), b"png").unwrap();
        super::sync_bundle_to("pro", &bundle).unwrap();
        let q = super::shell_quote(&bundle.to_string_lossy());
        let check = format!("ls -a {q}; ls {q}/frames-full; wc -c < {q}/recording.mp4");
        let mut out = String::new();
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let o = super::ssh("pro").arg(&check).output().unwrap();
            out = String::from_utf8_lossy(&o.stdout).into_owned();
            if !out.contains(super::MEDIA_PENDING) && out.contains("8000000") {
                break;
            }
        }
        let _ = super::ssh("pro")
            .arg(format!(
                "rm -rf {}",
                super::shell_quote(&base.to_string_lossy())
            ))
            .status();
        assert!(
            out.contains("8000000")
                && out.contains("frame-01.png")
                && !out.contains(super::MEDIA_PENDING),
            "{out}"
        );
    }
}

/// A stand-in for OpenCode's session API.
pub mod fake_api {
    use super::*;

    pub fn session(id: &str, title: &str, dir: &str, updated: u64) -> OpenCodeSession {
        OpenCodeSession {
            id: id.into(),
            title: title.into(),
            parent_id: None,
            time: OpenCodeSessionTime {
                updated,
                viewed: None,
                archived: None,
            },
            location: Some(OpenCodeSessionLocation {
                directory: Some(dir.into()),
            }),
            directory: None,
        }
    }

    /// Answers `GET /api/session/{id}` and `GET /api/session?…` like OpenCode 2: newest first,
    /// `parentID=null`, exact `directory`, `limit` (default 50), and a cursor carrying the filters.
    pub fn serve(sessions: &[OpenCodeSession], path: &str) -> Result<Vec<u8>> {
        use crate::percent_decode;
        let (route, query) = path.split_once('?').unwrap_or((path, ""));
        if let Some(id) = route.strip_prefix("/api/session/") {
            let session = sessions
                .iter()
                .find(|s| s.id == percent_decode(id))
                .context("404 Not Found")?;
            return Ok(serde_json::to_vec(&serde_json::json!({ "data": session }))?);
        }
        assert_eq!(route, "/api/session");
        let mut params: std::collections::HashMap<String, String> = query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .map(|(k, v)| (k.to_owned(), percent_decode(v)))
            .collect();
        let mut offset = 0;
        if let Some(cursor) = params.remove("cursor") {
            let cursor: serde_json::Value = serde_json::from_str(&cursor)?;
            offset = cursor["offset"].as_u64().unwrap() as usize;
            for key in ["parentID", "directory"] {
                if let Some(v) = cursor[key].as_str() {
                    params.insert(key.into(), v.into());
                }
            }
        }
        let limit: usize = params.get("limit").map_or(50, |l| l.parse().unwrap());
        let mut matching: Vec<_> = sessions
            .iter()
            .filter(|s| params.get("parentID").is_none_or(|_| s.parent_id.is_none()))
            .filter(|s| {
                params
                    .get("directory")
                    .is_none_or(|d| s.directory() == Some(d.as_str()))
            })
            .collect();
        matching.sort_by(|a, b| b.time.updated.cmp(&a.time.updated));
        let page: Vec<_> = matching.iter().skip(offset).take(limit).collect();
        let next = (offset + limit < matching.len()).then(|| {
            serde_json::json!({
                "offset": offset + limit,
                "parentID": params.get("parentID"),
                "directory": params.get("directory"),
            })
            .to_string()
        });
        Ok(serde_json::to_vec(
            &serde_json::json!({ "data": page, "cursor": { "next": next } }),
        )?)
    }
}

#[cfg(test)]
mod session_lookup {
    use super::fake_api::{serve, session};
    use super::*;

    fn many(count: usize) -> Vec<OpenCodeSession> {
        (0..count)
            .map(|i| {
                session(
                    &format!("ses_{i:05}"),
                    &format!("Chat {i}"),
                    "/p/x",
                    100_000 - i as u64,
                )
            })
            .collect()
    }

    /// Runs `f` against `sessions` and returns its result with the paths it requested.
    fn counting<T>(
        sessions: &[OpenCodeSession],
        f: impl FnOnce(&mut ApiGet) -> T,
    ) -> (T, Vec<String>) {
        let mut paths = Vec::new();
        let result = f(&mut |path: &str| {
            paths.push(path.to_owned());
            serve(sessions, path)
        });
        (result, paths)
    }

    #[test]
    fn an_explicit_id_is_one_request() {
        let mut sessions = many(5_000);
        sessions[4_000].title = "THE ONE".into();
        let (result, paths) = counting(&sessions, |get| {
            resolve_opencode_session_with(get, Some("ses_04000"), None, None).unwrap()
        });
        assert_eq!(result, ("ses_04000".into(), "THE ONE".into()));
        assert_eq!(paths, ["/api/session/ses_04000"]);
        // Unknown to the server: still sent, titled by its ID.
        let (result, _) = counting(&sessions, |get| {
            resolve_opencode_session_with(get, Some("ses_gone"), None, None).unwrap()
        });
        assert_eq!(result, ("ses_gone".into(), "ses_gone".into()));
    }

    #[test]
    fn the_newest_open_root_is_one_page() {
        let mut sessions = many(5_000);
        sessions[0].time.archived = Some(1);
        sessions[1].parent_id = Some("ses_00002".into());
        let (result, paths) = counting(&sessions, |get| {
            resolve_opencode_session_with(get, None, None, None).unwrap()
        });
        assert_eq!(result.0, "ses_00002");
        assert_eq!(paths, ["/api/session?parentID=null&limit=100"]);
    }

    #[test]
    fn an_app_match_stops_at_the_first_hit_and_the_search_is_capped() {
        let mut sessions = many(5_000);
        sessions[150].title = "UP DOWN APP".into();
        sessions[300].title = "updown older".into();
        let (result, paths) = counting(&sessions, |get| {
            resolve_opencode_session_with(get, None, Some("com.kitlangton.UpDown"), None).unwrap()
        });
        assert_eq!(result, ("ses_00150".into(), "UP DOWN APP".into()));
        assert_eq!(paths.len(), 2);

        // No match: OPENCODE_SESSION_ID if it's an open root, else the newest; never every page.
        let (result, paths) = counting(&sessions, |get| {
            resolve_opencode_session_with(get, None, Some("Nothing"), Some("ses_04321")).unwrap()
        });
        assert_eq!(result.0, "ses_04321");
        assert_eq!(paths.len(), ROOT_SEARCH_PAGES + 1);
        let (result, _) = counting(&sessions, |get| {
            resolve_opencode_session_with(get, None, Some("Nothing"), None).unwrap()
        });
        assert_eq!(result.0, "ses_00000");
    }

    #[test]
    fn engine_config_candidate_budget_defaults_to_384mb_and_clamps_minimum() {
        let cfg = EngineConfig::default();
        assert_eq!(cfg.candidate_budget_bytes(), CANDIDATE_BUDGET_BYTES);
        let custom = EngineConfig::default().with_candidate_budget_bytes(96 << 20);
        assert_eq!(custom.candidate_budget_bytes(), 96 << 20);
        let tiny = EngineConfig::default().with_candidate_budget_bytes(1024);
        assert_eq!(tiny.candidate_budget_bytes(), 16 << 20);
    }

    #[test]
    fn import_or_ensure_ingest_token_file_imports_scrawl_token_with_0600() {
        let dir =
            std::env::temp_dir().join(format!("scrawl-token-import-{}", crate::make_bundle_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let scrawl_cfg = dir.join("config.json");
        std::fs::write(&scrawl_cfg, r#"{"ingestToken": "paired-phone-token-123"}"#).unwrap();
        let hex_token_file = dir.join("hex/ingest_token");

        let imported =
            import_or_ensure_ingest_token_file(&hex_token_file, Some(&scrawl_cfg)).unwrap();
        assert_eq!(imported, "paired-phone-token-123");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&hex_token_file)
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }

        // Subsequent reads keep the imported token even if Scrawl's config changes or disappears.
        std::fs::remove_file(&scrawl_cfg).unwrap();
        let again = import_or_ensure_ingest_token_file(&hex_token_file, Some(&scrawl_cfg)).unwrap();
        assert_eq!(again, "paired-phone-token-123");

        // Fresh install with no legacy config generates a 64-char hex token.
        let fresh_file = dir.join("fresh/ingest_token");
        let fresh = import_or_ensure_ingest_token_file(&fresh_file, Some(&scrawl_cfg)).unwrap();
        assert_eq!(fresh.len(), 64);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
