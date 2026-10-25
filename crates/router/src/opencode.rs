//! OpenCode's session API, as the router uses it: the local server over HTTP (found through
//! OpenCode's own `service.json`), or `opencode2 api` over SSH on the agent host. Sessions are
//! listed and looked up, prompts sent (steer or queue), and the inbox, interrupt, revert and
//! messages reached through [`api_within`].

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::bundle::{load_bundle, resolve_bundle_dir};
use crate::model::ScrawlBundle;

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

/// The Mac whose OpenCode server holds Kit's sessions when it isn't this one: an SSH host from
/// the host's [`RouterConfig::agent_host`](crate::RouterConfig).
pub fn agent_host() -> Option<String> {
    crate::env::get().config.agent_host().map(str::to_owned)
}

/// The `opencode2` command to run on the agent host (`agentOpencode`, default `opencode2`).
fn agent_opencode() -> String {
    crate::env::get().config.agent_opencode().to_owned()
}

pub fn shell_quote(text: &str) -> String {
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

pub fn ssh(host: &str) -> Command {
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
/// Present in a bundle on this Mac while its heavy media is being copied to the agent host.
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
        crate::env::spawn(move || {
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
    if agent_host().is_some() {
        return opencode_api(&method.to_ascii_lowercase(), path, data);
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

pub fn normalize_slug(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
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
fn session_idle(session: &str) -> Option<bool> {
    let body = api_get_within("/api/session/active", IDLE_CHECK).ok()?;
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

fn post_prompt(session: &str, mut body: serde_json::Value) -> Result<Posted> {
    let delivery = crate::env::get().config.delivery.label();
    body["delivery"] = serde_json::json!(delivery);
    let was_idle = session_idle(session);
    let answer = opencode_api(
        "post",
        &format!("/api/session/{session}/prompt"),
        Some(&body),
    )?;
    Ok(Posted {
        message_id: posted_id(&answer),
        delivery: delivery.to_owned(),
        was_idle,
    })
}

/// [`send_bundle_to_session`] without remembering it.
pub fn send_bundle(bundle_dir: &Path, session: &str) -> Result<Posted> {
    let (directory, bundle) = load_bundle(bundle_dir)?;
    let summary = summary_of(&bundle, &directory);
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
    post_prompt(
        session,
        serde_json::json!({
            "text": text,
            "files": [{
                "uri": file_uri(&contact_sheet),
                "name": "contact-sheet.png",
                "description": "Scrawl contact sheet: the capture's keyframes in order",
            }],
        }),
    )
}

/// The host's summary of a bundle ([`crate::env::Summary`]), else its `summary.md`.
fn summary_of(bundle: &ScrawlBundle, directory: &Path) -> String {
    match crate::env::get().summary {
        Some(summary) => summary(bundle, directory),
        None => fs::read_to_string(directory.join("summary.md")).unwrap_or_default(),
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

/// A stand-in for OpenCode's session API, for tests here and in hosts.
#[doc(hidden)]
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
        use crate::util::percent_decode;
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

#[cfg(test)]
mod session_lookup {
    use super::fake_api::{serve, session};
    use super::*;

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
        let dir =
            std::env::temp_dir().join(format!("scrawl-service-{}", crate::util::make_bundle_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let state_path = dir.join("state-service.json");
        let config_path = dir.join("config-service.json");
        std::fs::write(
            &state_path,
            r#"{"id":"s1","url":"http://127.0.0.1:49374","pid":123,"password":"secret-state"}"#,
        )
        .unwrap();
        std::fs::write(
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
        let _ = std::fs::remove_dir_all(&dir);
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

    #[test]
    fn opencode_api_statuses_survive_the_cli() {
        assert_eq!(super::cli_status("HTTP 409 Conflict\n"), Some(409));
        assert_eq!(super::cli_status("warn: x\nHTTP 404 Not Found"), Some(404));
        assert_eq!(super::cli_status("error: connect ECONNREFUSED"), None);
        let err = anyhow::Error::new(super::HttpStatus(404)).context("OpenCode API failed");
        assert_eq!(super::http_status(&err), Some(404));
        assert_eq!(super::http_status(&anyhow::anyhow!("down")), None);
    }

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
