use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::model::ScrawlBundle;

pub use crate::bundle::plain_path;
pub use crate::opencode::*;

/// The `microphone` key: a device the user picked in Settings.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MicChoice {
    pub id: String,
    pub name: String,
}

/// Default memory ceiling for candidate keyframes during recording (384 MB).
pub const CANDIDATE_BUDGET_BYTES: usize = 384 << 20;

/// Default Hex language code.
pub const DEFAULT_LANGUAGE: &str = "en";

pub fn language_code(raw: &str) -> Option<String> {
    let s = raw.trim().to_ascii_lowercase();
    if s.is_empty() || s.len() > 16 {
        return None;
    }
    if s.bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    {
        Some(s)
    } else {
        None
    }
}

pub fn speech_locale_id(raw: &str) -> Option<String> {
    let s = raw.trim();
    if s.is_empty() || s.eq_ignore_ascii_case("auto") || s.len() > 32 {
        return None;
    }
    if s.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        Some(s.to_owned())
    } else {
        None
    }
}

/// Where captures go: `SCRAWL_CAPTURES_DIR`, else `"capturesDir"` in `EngineConfig` (`config.json`,
/// Settings › Storage), else `builtin_captures_dir`. Tests never see the config file.
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
    crate::bundle::resolve_bundle_dir_in(Some(&default_captures_dir()), path_or_id)
}

pub fn load_bundle(bundle_dir: &Path) -> Result<(PathBuf, ScrawlBundle)> {
    crate::bundle::load_bundle_in(Some(&default_captures_dir()), bundle_dir)
}

pub fn newest_bundle(root: &Path) -> Result<PathBuf> {
    crate::bundle::newest_bundle(root)
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct EngineConfig {
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
    /// Where new captures go (`default_captures_dir`); absent keeps the built-in folder.
    #[serde(default)]
    pub captures_dir: Option<String>,
    /// Optional host override for the frame candidate buffer ceiling in bytes (defaults to
    /// `CANDIDATE_BUDGET_BYTES`, 384 MB).
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

/// Config problems are logged once per process, not on every read.
pub(crate) fn log_config_once(message: String) {
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
static SCRAWL_HOST_DEFAULTS: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Enables Scrawl's own `~/.config/scrawl/config.json` and `~/Library/Logs/Scrawl/scrawl.log`
/// defaults. Called only by the Scrawl CLI/app binary (`src/main.rs`). Library consumers (such as
/// Hex) never call this, so library code never implicitly reads `~/.config/scrawl/config.json` or
/// writes under `~/Library/Application Support/scrawl`.
pub fn enable_scrawl_host_defaults() {
    SCRAWL_HOST_DEFAULTS.store(true, std::sync::atomic::Ordering::Release);
    crate::log::enable_file_log();
}

pub fn scrawl_host_defaults_enabled() -> bool {
    SCRAWL_HOST_DEFAULTS.load(std::sync::atomic::Ordering::Acquire)
}

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

pub fn engine_config_from_scrawl(raw: &str) -> EngineConfig {
    serde_json::from_str::<serde_json::Value>(raw)
        .ok()
        .map(parse_config_value::<EngineConfig>)
        .unwrap_or_default()
}

pub fn load_config_from(path: &Path) -> EngineConfig {
    parse_config_from(path)
}

pub fn engine_config() -> EngineConfig {
    if let Some(cfg) = scoped_engine_config() {
        return cfg;
    }
    if scrawl_host_defaults_enabled() {
        load_config_from(&default_config_path())
    } else {
        EngineConfig::default()
    }
}

/// The config in `value`. A key whose value doesn't fit (`"autoSend": "maybe"`) is dropped and
/// logged on its own, so one bad setting never resets the others to their defaults.
pub(crate) fn parse_config_value<T: for<'de> Deserialize<'de> + Default>(
    value: serde_json::Value,
) -> T {
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

fn config() -> EngineConfig {
    engine_config()
}

fn random_bytes_32() -> Result<[u8; 32]> {
    let mut bytes = [0_u8; 32];
    #[cfg(windows)]
    {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = ((n >> ((i % 16) * 8)) ^ (std::process::id() as u128).wrapping_mul(i as u128 + 1))
                as u8;
        }
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

/// What the router reads (installed as its fallback by `main`): `config.json`'s routing settings
/// with this process's `--experiment`s, Scrawl's folders, and the app's log, summaries and
/// `/sdk.js` pages. Rebuilt when `config.json` changes.
pub fn router_env() -> std::sync::Arc<crate::env::Env> {
    use std::sync::{Arc, Mutex, PoisonError};
    type Key = (Option<std::time::SystemTime>, u64, usize);
    static CACHE: Mutex<Option<(Key, Arc<crate::env::Env>)>> = Mutex::new(None);
    let path = default_config_path();
    let meta = fs::metadata(&path).ok();
    let overrides = EXPERIMENT_OVERRIDES
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .len();
    let key = (
        meta.as_ref().and_then(|m| m.modified().ok()),
        meta.map_or(0, |m| m.len()),
        overrides,
    );
    let mut cache = CACHE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((cached, env)) = cache.as_ref()
        && *cached == key
    {
        return env.clone();
    }
    let env = Arc::new(router_env_from(&path));
    *cache = Some((key, env.clone()));
    env
}

fn router_env_from(path: &Path) -> crate::env::Env {
    let text = fs::read_to_string(path).unwrap_or_default();
    let (mut config, warnings) = crate::config::parse_scrawl_router_config_with_warnings(&text);
    for warning in warnings {
        log_config_once(warning);
    }
    config.smart_routing = experiment(crate::routing::SMART_ROUTING);
    config.jev_routing = experiment(crate::routing::jev::JEV_ROUTING);
    if let Ok(host) = std::env::var("SCRAWL_AGENT_HOST") {
        config.agent_host = Some(host);
    }
    let support = dirs::home_dir().map(|home| home.join("Library/Application Support/scrawl"));
    let router = std::env::var_os("SCRAWL_ROUTER_DIR")
        .map(PathBuf::from)
        .or_else(|| support.as_ref().map(|s| s.join("router")));
    let log = std::env::var_os("SCRAWL_ROUTING_LOG")
        .map(PathBuf::from)
        .or_else(|| support.map(|s| s.join("routing-decisions.jsonl")));
    let captures = Some(default_captures_dir());
    let dirs = match router {
        Some(router) => crate::config::RouterDirs::under(&router, log, captures),
        None => crate::config::RouterDirs {
            log,
            captures,
            ..Default::default()
        },
    };
    crate::env::Env {
        config,
        dirs,
        log: Some(std::sync::Arc::new(|line: &str| {
            crate::log::scrawl_log(line)
        })),
        summary: Some(crate::sketch::summary_markdown),
        pages: Some(crate::web::lookup),
        live: true,
    }
}

pub fn auto_send() -> bool {
    config().auto_send()
}

/// The configured transcriber name (`transcriber`) as written, unvalidated.
pub fn transcriber() -> Option<String> {
    config().transcriber
}

/// The configured Hex model id (`hexModel`), if any.
pub fn hex_model() -> Option<String> {
    config().hex_model()
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
    config.apple_locale.as_deref().and_then(speech_locale_id)
}

/// Changes `config.json`'s `routing` object with `change` and saves it, other keys kept (`scrawl
/// router rule …`).
pub fn update_routing(
    change: impl FnOnce(&mut serde_json::Map<String, serde_json::Value>) -> Result<()>,
) -> Result<()> {
    let path = default_config_path();
    let mut routing = fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
        .and_then(|config| match config.get("routing") {
            Some(serde_json::Value::Object(map)) => Some(map.clone()),
            _ => None,
        })
        .unwrap_or_default();
    change(&mut routing)?;
    set_config_value_in(&path, "routing", serde_json::Value::Object(routing))
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
        .filter(|s| crate::config::safe_word(s))
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

#[cfg(test)]
mod tests {
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
            let keyframes: Vec<_> = [0, 1_900, 2_500, 4_000, 5_100, 6_000]
                .into_iter()
                .enumerate()
                .map(|(index, t)| {
                    let file = format!("frames/frame-{index:03}.png");
                    fs::write(path.join(&file), b"fixture").unwrap();
                    serde_json::json!({"index": index, "t": t, "reason": "initial", "file": file, "event": ""})
                })
                .collect();
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
        let dir =
            std::env::temp_dir().join(format!("scrawl-dest-{}", crate::util::make_bundle_id()));
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

        let second = ensure_ingest_token_in(&config_path).unwrap();
        assert_eq!(generated, second);

        let parsed = load_config_from(&config_path);
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
        assert_eq!(raw["agentHost"], "studio");

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
            r#"{"agentHost": "studio", "autoSend": "maybe", "usbApps": "not-a-list", "transcriber": "hex", "unknown": {"x": 1}}"#,
        )
        .unwrap();
        let cfg = load_config_from(&path);
        assert_eq!(cfg.transcriber.as_deref(), Some("hex"));
        assert_eq!(cfg.auto_send, None);
        assert!(cfg.usb_apps.is_empty());
        fs::write(
            &path,
            r#"{"usbApps": ["a"], "tetherApps": ["b"], "agentHost": "pro"}"#,
        )
        .unwrap();
        let cfg = load_config_from(&path);
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
        assert!(!experiment("neverEnabled"));
        enable_experiment("enabledForThisTest");
        assert!(experiment("enabledForThisTest"));
    }
}
