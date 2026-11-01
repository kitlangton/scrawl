use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::model::ScrawlBundle;

/// Where captures go: `SCRAWL_CAPTURES_DIR`, else `"capturesDir"` in `config.json` (Settings ›
/// Storage), else `builtin_captures_dir`. Tests never see the config file.
pub fn default_captures_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("SCRAWL_CAPTURES_DIR") {
        return PathBuf::from(dir);
    }
    let configured = if cfg!(test) {
        None
    } else {
        config().captures_dir
    };
    configured
        .as_deref()
        .and_then(configured_captures_dir)
        .filter(|dir| usable_captures_dir(dir))
        .unwrap_or_else(builtin_captures_dir)
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

/// Opens a file, folder or URL with its default app (Finder, Explorer, the browser).
pub fn open_path(target: &Path) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        crate::win::shell::open(target)
    }
    #[cfg(not(target_os = "windows"))]
    {
        let status = Command::new("/usr/bin/open").arg(target).status()?;
        ensure!(status.success(), "open {} failed", target.display());
        Ok(())
    }
}

/// Shows a file selected in Finder or Explorer.
pub fn reveal_path(target: &Path) -> Result<()> {
    #[cfg(target_os = "windows")]
    {
        crate::win::shell::reveal(target)
    }
    #[cfg(not(target_os = "windows"))]
    {
        Command::new("/usr/bin/open")
            .arg("-R")
            .arg(target)
            .spawn()?;
        Ok(())
    }
}

pub fn resolve_bundle_dir(path_or_id: &Path) -> Result<PathBuf> {
    if path_or_id == Path::new("latest") {
        return newest_bundle(&default_captures_dir());
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
        path = default_captures_dir().join(&path);
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
fn plain_path(path: PathBuf) -> PathBuf {
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

#[cfg(target_os = "macos")]
pub fn copy_string_to_clipboard(content: &str) -> Result<()> {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypeString};
    use objc2_foundation::{NSArray, NSString};

    let text = NSString::from_str(content);
    let pasteboard = NSPasteboard::generalPasteboard();
    unsafe {
        let types = NSArray::from_slice(&[NSPasteboardTypeString]);
        pasteboard.clearContents();
        pasteboard.declareTypes_owner(&types, None);
        ensure!(
            pasteboard.setString_forType(&text, NSPasteboardTypeString),
            "Could not copy text to clipboard"
        );
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn copy_png_to_clipboard(png_bytes: &[u8]) -> Result<()> {
    use objc2_app_kit::{NSPasteboard, NSPasteboardTypePNG};
    use objc2_foundation::{NSArray, NSData};

    let data = NSData::with_bytes(png_bytes);
    let pasteboard = NSPasteboard::generalPasteboard();
    unsafe {
        let types = NSArray::from_slice(&[NSPasteboardTypePNG]);
        pasteboard.clearContents();
        pasteboard.declareTypes_owner(&types, None);
        ensure!(
            pasteboard.setData_forType(Some(&data), NSPasteboardTypePNG),
            "Could not copy PNG image to clipboard"
        );
    }
    Ok(())
}

#[cfg(target_os = "windows")]
pub fn copy_string_to_clipboard(content: &str) -> Result<()> {
    crate::win::clipboard::set_text(content)
}

#[cfg(target_os = "windows")]
pub fn copy_png_to_clipboard(png_bytes: &[u8]) -> Result<()> {
    crate::win::clipboard::set_png(png_bytes)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn copy_bundle_to_clipboard(bundle_dir: &Path) -> Result<()> {
    let (directory, bundle) = load_bundle(bundle_dir)?;
    let summary = crate::sketch::summary_markdown(&bundle, &directory);
    #[cfg(target_os = "windows")]
    if experiment("clipboardImage")
        && let Ok(image) = fs::read(directory.join(&bundle.contact_sheet))
        && crate::win::clipboard::set_png_and_text(&image, Some(&summary)).is_ok()
    {
        return Ok(());
    }
    copy_string_to_clipboard(&summary)
}

/// Pastes both the contact-sheet image and the transcript/keyframe text into the active app
/// as a two-step sequence (`⌘V` image, then `⌘V` text; Ctrl+V on Windows). If `click_target` is
/// provided (from dragging the Paste chip onto a window/input), clicks at that screen coordinate
/// first to focus it.
#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn paste_bundle_at_target(bundle_dir: &Path, click_target: Option<[f32; 2]>) -> Result<()> {
    if let Some([x, y]) = click_target {
        crate::capture::post_synthetic_click(f64::from(x), f64::from(y));
        std::thread::sleep(std::time::Duration::from_millis(65));
    } else {
        crate::capture::ensure_external_app_active();
    }
    paste_bundle_image_and_text(bundle_dir)
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
pub fn paste_bundle_image_and_text(bundle_dir: &Path) -> Result<()> {
    let (directory, bundle) = load_bundle(bundle_dir)?;
    let sheet_path = directory.join(&bundle.contact_sheet);
    let image = fs::read(&sheet_path).context("Could not read contact sheet")?;
    let summary = crate::sketch::summary_markdown(&bundle, &directory);

    crate::capture::wait_for_modifiers_released(350);

    if crate::capture::is_frontmost_terminal() {
        // In terminals (Ghostty / OpenCode TUI / Claude Code), pasting the image path first
        // resolves and attaches `[Image 1]`, and the second paste inserts the text summary.
        copy_string_to_clipboard(&sheet_path.display().to_string())?;
    } else {
        // In GUI apps (OpenCode Desktop, Slack, Linear, browsers), PNG-only attaches the image.
        copy_png_to_clipboard(&image)?;
    }
    std::thread::sleep(std::time::Duration::from_millis(20));
    crate::capture::post_synthetic_cmd_v();

    std::thread::sleep(std::time::Duration::from_millis(160));
    copy_string_to_clipboard(&summary)?;
    std::thread::sleep(std::time::Duration::from_millis(20));
    crate::capture::post_synthetic_cmd_v();
    Ok(())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn copy_bundle_to_clipboard(_bundle_dir: &Path) -> Result<()> {
    bail!("Image and text clipboard handoff requires macOS or Windows")
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn paste_bundle_image_and_text(_bundle_dir: &Path) -> Result<()> {
    bail!("Image and text paste handoff requires macOS or Windows")
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn paste_bundle_at_target(_bundle_dir: &Path, _click_target: Option<[f32; 2]>) -> Result<()> {
    bail!("Image and text paste handoff requires macOS or Windows")
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
pub(crate) fn on_path(name: &str) -> Option<PathBuf> {
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

#[derive(Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Config {
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
    /// Feature flag: shows **Export Last Capture as Video…** in the menu (default off).
    #[serde(default)]
    pub video_export: Option<bool>,
    /// `"apple"` (default), `"hex"` or `"command"`: which engine transcribes recordings
    /// (`transcribe`). The command's own keys are read by `transcribe_command`, not here, so a
    /// malformed one can't reset this struct.
    #[serde(default)]
    pub transcriber: Option<String>,
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
    pub microphone: Option<crate::mic::Choice>,
    /// Smart routing's settings (`routing::look`).
    #[serde(default)]
    pub routing: Option<RoutingConfig>,
    /// Where new captures go (`default_captures_dir`); absent keeps the built-in folder.
    #[serde(default)]
    pub captures_dir: Option<String>,
}

impl Config {
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
    /// Where captures of a site always go, ahead of weighing: a URL prefix (`https://` optional;
    /// any path and query below it) to a folder, whose newest open session gets them, or to a
    /// session ID (`{"https://problems.example.com": "~/code/team-board"}`).
    #[serde(default)]
    pub sites: std::collections::BTreeMap<String, String>,
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
fn log_config_once(message: String) {
    static SEEN: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    let mut seen = SEEN
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !seen.contains(&message) {
        crate::capture::scrawl_log(&message);
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

pub fn default_config_path() -> PathBuf {
    std::env::var_os("SCRAWL_CONFIG")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".config/scrawl/config.json")
        })
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
fn config_from_value(value: serde_json::Value) -> Config {
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

fn config() -> Config {
    load_config_from(&default_config_path())
}

fn random_bytes_32() -> Result<[u8; 32]> {
    let mut bytes = [0_u8; 32];
    #[cfg(windows)]
    {
        crate::win::shell::random_bytes(&mut bytes)?;
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

/// `routing.apps` from config.json, `~` expanded. Tests see none.
pub fn routing_apps() -> Vec<(String, PathBuf)> {
    routing_paths(|routing| routing.apps)
}

/// `routing.servers` in config.json: URL prefixes and the folders they serve, `~` expanded.
pub fn routing_servers() -> Vec<(String, PathBuf)> {
    routing_paths(|routing| routing.servers)
}

/// `routing.sites` in config.json: URL prefixes and the folder (`~` expanded) or session ID their
/// captures go to.
pub fn routing_sites() -> Vec<(String, PathBuf)> {
    routing_paths(|routing| routing.sites)
}

fn routing_paths(
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

/// Whether the menu offers video export (`videoExport`, default off). `scrawl export` works
/// either way.
pub fn video_export() -> bool {
    config().video_export.unwrap_or(false)
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
        .and_then(crate::hex::language_code)
        .unwrap_or_else(|| crate::hex::DEFAULT_LANGUAGE.into())
}

/// The configured Apple Speech locale, or the Mac's own language when unset.
pub fn apple_locale() -> Option<String> {
    apple_locale_from(&config())
}

pub fn apple_locale_from(config: &Config) -> Option<String> {
    config
        .apple_locale
        .as_deref()
        .and_then(crate::speech_model::locale_id)
}

/// The command transcriber's program (`transcribeCommand`, `transcribeTimeoutSeconds`), read
/// on its own from the config file.
pub fn transcribe_command()
-> Result<crate::command_transcriber::Program, crate::command_transcriber::Unusable> {
    crate::command_transcriber::Program::load_from(&default_config_path())
}

/// Whether the native AppKit + Metal recording pill is enabled (`SCRAWL_NATIVE_PILL` or
/// `nativePill` in `config.json`, default `true`). macOS only: elsewhere the GPUI capsule draws.
pub fn native_pill() -> bool {
    if cfg!(not(target_os = "macos")) {
        return false;
    }
    if let Ok(val) = std::env::var("SCRAWL_NATIVE_PILL") {
        let s = val.trim().to_ascii_lowercase();
        if matches!(s.as_str(), "0" | "false" | "gpui" | "off" | "no") {
            return false;
        }
        if matches!(s.as_str(), "1" | "true" | "native" | "metal" | "on" | "yes") {
            return true;
        }
    }
    config().native_pill.unwrap_or(true)
}

/// Configured capsule style (`pillStyle` in `config.json`, default `"black"`).
pub fn pill_style() -> crate::native::pill::PillStyle {
    config()
        .pill_style
        .as_deref()
        .and_then(crate::native::pill::PillStyle::from_str_opt)
        .unwrap_or(crate::native::pill::PillStyle::Black)
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
        let sending = bundle_dir.join(crate::retention::MEDIA_SENDING);
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
    let mut command = if let Some(host) = agent_host() {
        let mut remote = format!(
            "PATH=\"$HOME/bin:$HOME/.opencode/bin:/opt/homebrew/bin:$PATH\" {} api {} {}",
            agent_opencode(),
            shell_quote(method),
            shell_quote(path)
        );
        if let Some(data) = data {
            remote.push_str(&format!(
                " --data {}",
                shell_quote(&serde_json::to_string(data)?)
            ));
        }
        let mut command = ssh(&host);
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
    let output = command.output().context("Could not launch opencode2")?;
    ensure!(
        output.status.success(),
        "OpenCode API failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    );
    Ok(output.stdout)
}

pub(crate) fn query_escape(value: &str) -> String {
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
    let request =
        format!("GET {path} HTTP/1.1\r\nHost: {address}\r\n{auth}Connection: close\r\n\r\n");
    stream
        .write_all(request.as_bytes())
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
    ensure!(
        (200..300).contains(&status),
        "OpenCode API failed: HTTP {status}"
    );
    Ok(body)
}

/// [`api_get`] for routing: talks HTTP directly to the local OpenCode server with a
/// [`SERVER_CONNECT_TIMEOUT`] connect cap and `timeout` read deadline, instead of spawning
/// `opencode2 api` per request.
pub fn api_get_within(path: &str, timeout: std::time::Duration) -> Result<Vec<u8>> {
    if agent_host().is_some() {
        return opencode_api("get", path, None);
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

pub(crate) fn normalize_slug(text: &str) -> String {
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

/// Queues a bundle's summary and pointers into an OpenCode session.
pub fn send_bundle_to_session(bundle_dir: &Path, session: &str) -> Result<()> {
    let (directory, bundle) = load_bundle(bundle_dir)?;
    let summary = crate::sketch::summary_markdown(&bundle, &directory);
    ensure!(
        session.starts_with("ses")
            && session
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
        "Invalid OpenCode session ID"
    );
    if let Some(host) = agent_host() {
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
    // The server reads `file://` attachments from its own disk; the bundle is at the same path
    // there (copied above when the agents run on another Mac).
    opencode_api(
        "post",
        &format!("/api/session/{session}/prompt"),
        Some(&serde_json::json!({
            "text": text,
            "files": [{
                "uri": file_uri(&contact_sheet),
                "name": "contact-sheet.png",
                "description": "Scrawl contact sheet: the capture's keyframes in order",
            }],
            "delivery": "queue",
        })),
    )?;
    Ok(())
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

pub fn extract_frames_range(
    bundle_dir: &Path,
    from_ms: u64,
    to_ms: u64,
    fps: u32,
) -> Result<Vec<PathBuf>> {
    ensure!(from_ms <= to_ms, "Frame range ends before it starts");
    ensure!(fps > 0 && fps <= 240, "FPS must be between 1 and 240");
    let directory = resolve_bundle_dir(bundle_dir)?;
    let loaded = load_bundle(&directory).ok().map(|(_, bundle)| bundle);
    let offset_ms = loaded.as_ref().map_or(0, |b| b.video_offset_ms);
    let video = loaded
        .and_then(|bundle| bundle.video)
        .map_or_else(|| directory.join("recording.mp4"), |v| directory.join(v));
    if video.is_file() && from_ms < to_ms {
        let output_dir = directory
            .join("zoom")
            .join(format!("range-{from_ms}-{to_ms}"));
        if let Some(ffmpeg) = on_path("ffmpeg") {
            fs::create_dir_all(&output_dir)?;
            // Only remove our generated files, so repeating at a lower FPS cannot
            // return leftover frames from a previous extraction.
            for entry in fs::read_dir(&output_dir)? {
                let entry = entry?;
                if generated_frame(&entry.path()) {
                    fs::remove_file(entry.path())?;
                }
            }
            let output = Command::new(ffmpeg)
                .args([
                    "-hide_banner",
                    "-loglevel",
                    "error",
                    "-nostdin",
                    "-y",
                    "-ss",
                ])
                .arg(format!("{:.3}", from_ms as f64 / 1_000.0))
                .args(["-threads", "1", "-filter_threads", "1"])
                .arg("-i")
                .arg(&video)
                .arg("-t")
                .arg(format!("{:.3}", (to_ms - from_ms) as f64 / 1_000.0))
                .arg("-vf")
                .arg(format!("fps={fps}"))
                .args(["-threads", "1"])
                .arg(output_dir.join("frame-%03d.png"))
                .output()
                .context("Could not start ffmpeg")?;
            ensure!(
                output.status.success(),
                "ffmpeg extraction failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            let mut frames = Vec::new();
            for entry in fs::read_dir(&output_dir)? {
                let path = entry?.path();
                if generated_frame(&path) {
                    frames.push(path);
                }
            }
            frames.sort();
            return Ok(frames);
        }

        let step_ms = (1_000 / u64::from(fps)).max(1);
        let times: Vec<u64> = (from_ms..=to_ms).step_by(step_ms as usize).collect();
        if !times.is_empty()
            && let Ok(decoded) = crate::movie_frames_at(&video, &times, offset_ms, 0)
            && !decoded.is_empty()
        {
            fs::create_dir_all(&output_dir)?;
            for entry in fs::read_dir(&output_dir)? {
                let entry = entry?;
                if generated_frame(&entry.path()) {
                    fs::remove_file(entry.path())?;
                }
            }
            let mut written = Vec::with_capacity(decoded.len());
            for (idx, frame) in decoded.iter().enumerate() {
                let out_path = output_dir.join(format!("frame-{:03}.png", idx + 1));
                crate::render::save_png(frame, &out_path)?;
                written.push(out_path);
            }
            return Ok(written);
        }
    }
    existing_keyframes(&directory, from_ms, to_ms, fps)
}

fn generated_frame(path: &Path) -> bool {
    path.is_file()
        && path.extension().is_some_and(|ext| ext == "png")
        && path
            .file_stem()
            .and_then(|name| name.to_str())
            .and_then(|name| name.strip_prefix("frame-"))
            .is_some_and(|digits| {
                !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
            })
}

fn existing_keyframes(
    directory: &Path,
    from_ms: u64,
    to_ms: u64,
    fps: u32,
) -> Result<Vec<PathBuf>> {
    let (_, bundle) = load_bundle(directory)?;
    let margin = 1_000_u64.div_ceil(u64::from(fps)) / 2;
    let mut frames = Vec::new();
    for frame in bundle.keyframes {
        if frame.t < from_ms.saturating_sub(margin) || frame.t > to_ms.saturating_add(margin) {
            continue;
        }
        let relative = Path::new(&frame.file);
        ensure!(
            !relative.is_absolute()
                && relative.components().all(|part| matches!(
                    part,
                    std::path::Component::Normal(_) | std::path::Component::CurDir
                )),
            "Invalid keyframe path"
        );
        let path = directory.join(relative);
        if generated_frame(&path) {
            let canonical = plain_path(path.canonicalize()?);
            ensure!(
                canonical.starts_with(directory.join("frames")),
                "Keyframe is outside bundle frames directory"
            );
            frames.push((frame.t, canonical));
        }
    }
    frames.sort_unstable();
    frames.dedup_by(|a, b| a.1 == b.1);
    Ok(frames.into_iter().map(|(_, path)| path).collect())
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
    fn frame_fallback_uses_manifest_times_not_file_indices() {
        let fixture = Fixture::new();
        let path = fixture.bundle("example", 123);
        let frames = extract_frames_range(&path, 2_000, 5_000, 4).unwrap();
        let names: Vec<_> = frames
            .iter()
            .map(|frame| frame.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(
            names,
            vec![
                "frame-001.png",
                "frame-002.png",
                "frame-003.png",
                "frame-004.png"
            ]
        );
        assert!(
            extract_frames_range(&path, 10_000, 11_000, 4)
                .unwrap()
                .is_empty()
        );
        assert!(extract_frames_range(&path, 5_000, 2_000, 4).is_err());
        assert!(extract_frames_range(&path, 0, 2_000, 0).is_err());
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
    fn video_export_flag_defaults_off() {
        let fixture = Fixture::new();
        let path = fixture.0.join("config.json");
        fs::write(&path, br#"{"autoSend":false}"#).unwrap();
        assert_eq!(load_config_from(&path).video_export, None);
        fs::write(&path, br#"{"videoExport":true}"#).unwrap();
        assert_eq!(load_config_from(&path).video_export, Some(true));
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
        assert_eq!(cfg.video_export, Some(true));
        assert_eq!(cfg.pill_style.as_deref(), Some("glass"));
        assert_eq!(cfg.auto_send, None);
        assert_eq!(cfg.native_pill, None);
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
#[cfg(test)]
pub(crate) mod fake_api {
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
        use crate::ax::percent_decode;
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
}
