//! OpenCode session discovery, local transport and SSH bundle delivery.
use super::{config, load_bundle, resolve_bundle_dir};
use std::{fs, path::{Path, PathBuf}, process::Command};
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use crate::shared::model::ApparentBundle;

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

pub(super) fn executable(path: &Path) -> bool {
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

pub(super) fn opencode_binary() -> Result<PathBuf> {
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

/// Safe to put unquoted in a remote shell command (host names, `~/bin/tool` paths).
pub(super) fn safe_word(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.@/~".contains(&b))
}

/// The Mac whose OpenCode server holds Kit's sessions when it isn't this one: an SSH host from
/// `APPARENT_AGENT_HOST` or `~/.config/apparent/config.json` (`{"agentHost": "studio"}`).
pub fn agent_host() -> Option<String> {
    std::env::var("APPARENT_AGENT_HOST")
        .ok()
        .or_else(|| config().agent_host)
        .filter(|h| safe_word(h))
}

/// The `opencode2` command to run on the agent host (`agentOpencode`, default `opencode2`).
pub(super) fn agent_opencode() -> String {
    config()
        .agent_opencode
        .filter(|c| safe_word(c))
        .unwrap_or_else(|| "opencode2".to_owned())
}

pub(super) fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// A system command: `/usr/bin/<name>` on macOS; on Windows the copy in System32 (OpenSSH and
/// bsdtar ship with Windows 10 and later), and on Linux the one on `PATH`.
pub(super) fn system_tool(name: &str) -> PathBuf {
    if cfg!(not(target_os = "macos")) {
        on_path(name).unwrap_or_else(|| PathBuf::from(name))
    } else {
        Path::new("/usr/bin").join(name)
    }
}

pub(super) fn ssh(host: &str) -> Command {
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
/// in the background for `apparent zoom`, marked by [`MEDIA_PENDING`] until it lands.
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
    // The marker goes up with the light files, before the session is prompted, so `apparent zoom`
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
        let sending = bundle_dir.join(crate::capture::engine::bundle::retention::MEDIA_SENDING);
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

pub(super) struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

pub(super) fn opencode_api(method: &str, path: &str, data: Option<&serde_json::Value>) -> Result<Vec<u8>> {
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
pub(super) struct ServiceFile {
    url: Option<String>,
    hostname: Option<String>,
    port: Option<u16>,
    password: Option<String>,
}

impl ServiceFile {
    pub(super) fn load(path: &Path) -> Option<Self> {
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

pub(super) fn non_empty(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// The socket address in `http://127.0.0.1:4096…` when its host is an IP or `localhost` (no
/// lookups).
pub(super) fn server_address(url: &str) -> Option<std::net::SocketAddr> {
    let parsed = crate::shared::routing::parse_url(url.split_whitespace().next()?)?;
    let ip = if parsed.host == "localhost" {
        std::net::IpAddr::from([127, 0, 0, 1])
    } else {
        parsed.host.parse().ok()?
    };
    Some(std::net::SocketAddr::new(ip, parsed.port))
}

pub(super) fn resolve_server(
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
pub(super) fn local_server() -> (std::net::SocketAddr, Option<String>) {
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

pub(super) fn base64(bytes: &[u8]) -> String {
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

pub(super) fn http_get(
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

/// The session an older imported recording was addressed to: its timeline's top-level `session`,
/// else the `destination` a Mac capture of a dev site was routed to (`apparent.json`).
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
        .or_else(|| read("apparent.json").and_then(|b| session(b.pointer("/destination/session"))))
}

pub(super) fn choose_destination<'a>(
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
    opencode_api(
        "post",
        &format!("/api/session/{session}/prompt"),
        Some(&session_prompt(&directory, &bundle)),
    )?;
    Ok(())
}

pub(super) fn session_prompt(directory: &Path, bundle: &ApparentBundle) -> serde_json::Value {
    let summary = crate::capture::engine::sketch::summary_markdown(bundle, directory);
    let quoted_directory = shell_quote(&directory.to_string_lossy());
    let contact_sheet = directory.join(&bundle.contact_sheet);
    let clicked = bundle.has_screen_click();
    let visual_note = if clicked {
        format!("The contact sheet is attached ({}).", contact_sheet.display())
    } else {
        format!("No image is attached; the recorded visuals remain in {}. Look them up only if needed.", directory.display())
    };
    let text = format!(
        "{}\n\n{} Read {} for timestamped transcript, marks, and interactions.\nTo look closely at a moment (an animation, a glitch, what the narration points at), run:\napparent zoom {} --at <M:SS or \"a phrase that was said\"> [--span 2] [--frames 12]\nIt saves a dense contact sheet of that moment with each frame's time and prints what was said. This capture is {} ms long.",
        summary.trim_end(),
        visual_note,
        directory.join("apparent.json").display(),
        quoted_directory,
        bundle.duration_ms,
    );
    // The server reads `file://` attachments from its own disk; the bundle is at the same path
    // there (copied above when the agents run on another Mac).
    let files = if clicked {
        serde_json::json!([{
                "uri": file_uri(&contact_sheet),
                "name": "contact-sheet.png",
                "description": "Apparent contact sheet: the capture's keyframes in order",
            }])
    } else {
        serde_json::json!([])
    };
    serde_json::json!({ "text": text, "files": files, "delivery": "queue" })
}

/// A `file://` URL for an absolute path, percent-encoding everything but unreserved
/// characters and `/` ("Application Support" → "Application%20Support"). A Windows path
/// (`C:\Users\…`, or `\\?\C:\…` after `canonicalize`) becomes `file:///C:/Users/…`.
pub(crate) fn file_uri(path: &Path) -> String {
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
