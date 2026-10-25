//! Where a Mac capture of a browser tab goes. When the capture ends on a browser showing a dev
//! site, the bundle is sent to that project's OpenCode session instead of only the clipboard:
//!
//! 1. The page registered itself through Scrawl's `/sdk.js` (`web`): its session, or the session
//!    for the directory it named.
//! 2. The tab is on a loopback host: the process listening on that port, its working directory,
//!    that directory's git root, and the most recently active root session in it.
//! 3. Otherwise nothing: the capture stays on the clipboard.
//!
//! A `routing.sites` rule covering the tab's URL comes first ([`site_rule`]).
//!
//! With `"experiments": {"smartRouting": true}`, [`smart`] decides instead, from clues
//! [`look`] gathers (`docs/routing.md`); with `jevRouting` too, [`jev`] weighs them instead.

pub mod jev;
pub mod look;
pub mod named;
pub mod policy;
pub mod router;
pub mod served;
pub mod smart;

/// The experiment that routes Mac captures with [`smart`] (`handoff::experiment`).
pub const SMART_ROUTING: &str = "smartRouting";

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::Result;

use crate::capture::scrawl_log;
use crate::handoff::{ApiGet, OpenCodeSession};
use crate::model::{Destination, ScrawlBundle};
use crate::web::Registration;

/// The parts of an `http(s)` URL routing needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WebUrl {
    pub scheme: String,
    pub host: String,
    pub port: u16,
}

impl WebUrl {
    /// `scheme://host[:port]`, the port left out when it's the scheme's default (as browsers send
    /// `Origin`).
    pub fn origin(&self) -> String {
        let default = if self.scheme == "https" { 443 } else { 80 };
        let host = if self.host.contains(':') {
            format!("[{}]", self.host)
        } else {
            self.host.clone()
        };
        if self.port == default {
            format!("{}://{host}", self.scheme)
        } else {
            format!("{}://{host}:{}", self.scheme, self.port)
        }
    }

    pub fn is_loopback(&self) -> bool {
        is_loopback_host(&self.host)
    }
}

/// Parses `http://` and `https://` URLs (and bare origins). Hosts are lowercased; IPv6 hosts
/// lose their brackets.
pub fn parse_url(url: &str) -> Option<WebUrl> {
    let url = url.trim();
    let (scheme, rest) = url.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if scheme != "http" && scheme != "https" {
        return None;
    }
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    let (host, port) = if let Some(v6) = authority.strip_prefix('[') {
        let (host, after) = v6.split_once(']')?;
        let port = match after.strip_prefix(':') {
            Some(port) => Some(port.parse().ok()?),
            None if after.is_empty() => None,
            None => return None,
        };
        (host.to_owned(), port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (host.to_owned(), Some(port.parse().ok()?)),
            None => (authority.to_owned(), None),
        }
    };
    if host.is_empty()
        || !host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_:".contains(&b))
    {
        return None;
    }
    let port = port.unwrap_or(if scheme == "https" { 443 } else { 80 });
    Some(WebUrl {
        scheme,
        host: host.to_ascii_lowercase(),
        port,
    })
}

/// `localhost`, `*.localhost`, `127.0.0.0/8`, `::1`, and `0.0.0.0` (which browsers send to this
/// machine).
pub fn is_loopback_host(host: &str) -> bool {
    let host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    if host == "localhost" || host.ends_with(".localhost") || host == "::1" || host == "0.0.0.0" {
        return true;
    }
    host.parse::<std::net::Ipv4Addr>()
        .is_ok_and(|ip| ip.is_loopback())
}

// MARK: Site rules

/// Where a `routing.sites` rule sends a site's captures.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SiteTarget {
    Session(String),
    /// The newest open session anywhere in it (in its repository, when it's in one).
    Folder(PathBuf),
}

/// The `routing.sites` rule covering `url`, the most specific first: its prefix and target.
pub fn site_rule(url: &str, sites: &[(String, PathBuf)]) -> Option<(String, SiteTarget)> {
    let parsed = parse_url(url)?;
    let path = url_path(url);
    sites
        .iter()
        .filter_map(|(prefix, to)| Some((covers(prefix, &parsed, path)?, prefix, to)))
        .max_by_key(|(rank, ..)| *rank)
        .map(|(_, prefix, to)| {
            let text = to.to_string_lossy();
            let target = if text.starts_with("ses_") && !text.contains('/') {
                SiteTarget::Session(text.into_owned())
            } else {
                SiteTarget::Folder(to.clone())
            };
            (prefix.clone(), target)
        })
}

/// The path of `url`, without its query or fragment (`""` for none).
fn url_path(url: &str) -> &str {
    let url = url.trim();
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    let rest = rest.split(['?', '#']).next().unwrap_or_default();
    rest.find('/').map_or("", |at| &rest[at..])
}

/// How specifically `prefix` covers a URL, or `None` when it doesn't. A prefix without a scheme
/// covers both, and without a port any port.
fn covers(prefix: &str, url: &WebUrl, path: &str) -> Option<usize> {
    let prefix = prefix.trim();
    let schemed = prefix.contains("://");
    let full = if schemed {
        prefix.to_owned()
    } else {
        format!("https://{prefix}")
    };
    let rule = parse_url(&full)?;
    if rule.host != url.host {
        return None;
    }
    let authority = full.split_once("://").map_or("", |(_, rest)| {
        rest.split(['/', '?', '#']).next().unwrap_or_default()
    });
    let ported = authority
        .rsplit_once(':')
        .is_some_and(|(_, port)| port.parse::<u16>().is_ok());
    if (schemed && rule.scheme != url.scheme) || ((schemed || ported) && rule.port != url.port) {
        return None;
    }
    let within = url_path(&full).trim_end_matches('/');
    let inside = within.is_empty()
        || path.trim_end_matches('/') == within
        || path.starts_with(&format!("{within}/"));
    inside.then_some(within.len() * 4 + usize::from(schemed) * 2 + usize::from(ported))
}

/// [`route`], with a `routing.sites` rule covering `url` obeyed first.
pub fn route_with_sites(
    url: &str,
    sites: &[(String, PathBuf)],
    hooks: &Hooks,
) -> std::result::Result<Destination, String> {
    let Some((prefix, target)) = site_rule(url, sites) else {
        return route(url, hooks);
    };
    let mut api = within(hooks.api, SESSIONS_BUDGET);
    let ruled = match target {
        SiteTarget::Session(id) => {
            let title = crate::handoff::get_session(&mut api, &id)
                .map_or_else(|_| id.clone(), |s| s.label().to_owned());
            Some(Destination {
                session: id,
                title,
                via: "rule".into(),
                url: Some(url.to_owned()),
                directory: None,
            })
        }
        SiteTarget::Folder(dir) => {
            let (root, _) = project_root(&dir);
            session_for_directory(&mut api, &root, true)
                .ok()
                .flatten()
                .map(|s| Destination {
                    session: s.id.clone(),
                    title: s.label().to_owned(),
                    via: "rule".into(),
                    url: Some(url.to_owned()),
                    directory: Some(root.display().to_string()),
                })
        }
    };
    match ruled {
        Some(dest) => Ok(dest),
        None => {
            scrawl_log(format!(
                "routing: {prefix} rule found no open session; routing as usual"
            ));
            route(url, hooks)
        }
    }
}

// MARK: The browser tab

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Browser {
    /// Chrome, Brave, Arc, Edge, Vivaldi, Chromium: `URL of active tab of front window`.
    Chromium,
    /// Safari: `URL of current tab of front window`.
    Safari,
}

pub fn browser_kind(bundle_id: &str) -> Option<Browser> {
    const CHROMIUM: [&str; 6] = [
        "com.google.chrome",
        "com.brave.browser",
        "company.thebrowser.browser",
        "com.microsoft.edgemac",
        "com.vivaldi.vivaldi",
        "org.chromium.chromium",
    ];
    let id = bundle_id.to_ascii_lowercase();
    if id == "com.apple.safari" || id == "com.apple.safaritechnologypreview" {
        return Some(Browser::Safari);
    }
    // Beta, dev, nightly, and canary channels add a suffix.
    CHROMIUM
        .iter()
        .any(|base| id == *base || id.strip_prefix(base).is_some_and(|s| s.starts_with('.')))
        .then_some(Browser::Chromium)
}

fn safe_bundle_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-_".contains(&b))
}

/// The front tab's URL in the browser `bundle_id`, through AppleScript. The first time macOS
/// asks whether Scrawl may control the browser; while that prompt is up this waits a few
/// seconds, then gives up for this capture. A denial just returns `None`.
pub fn browser_tab_url(bundle_id: &str) -> Option<String> {
    let kind = browser_kind(bundle_id)?;
    if !safe_bundle_id(bundle_id) {
        return None;
    }
    let tab = match kind {
        Browser::Chromium => "active tab",
        Browser::Safari => "current tab",
    };
    let script = format!(
        "tell application id \"{bundle_id}\"\nif (count of windows) is 0 then return \"\"\nreturn URL of {tab} of front window\nend tell"
    );
    let output = run_with_timeout(
        Command::new("/usr/bin/osascript").args(["-e", &script]),
        Duration::from_secs(6),
    )?;
    if !output.status.success() {
        scrawl_log(format!(
            "routing: couldn't read the {bundle_id} tab ({})",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
        return None;
    }
    let url = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (!url.is_empty()).then_some(url)
}

fn run_with_timeout(command: &mut Command, timeout: Duration) -> Option<std::process::Output> {
    use std::io::Read as _;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    // Read as it's written: output larger than a pipe's buffer would otherwise stall the child.
    let drain = |pipe: Option<Box<dyn std::io::Read + Send>>| {
        std::thread::spawn(move || {
            let mut out = Vec::new();
            if let Some(mut pipe) = pipe {
                let _ = pipe.read_to_end(&mut out);
            }
            out
        })
    };
    let stdout = drain(child.stdout.take().map(|p| Box::new(p) as _));
    let stderr = drain(child.stderr.take().map(|p| Box::new(p) as _));
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return Some(std::process::Output {
                    status,
                    stdout: stdout.join().ok()?,
                    stderr: stderr.join().ok()?,
                });
            }
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                scrawl_log("routing: gave up waiting on a subprocess");
                return None;
            }
        }
    }
}

/// The URL a capture was of: the browser in front when it finished, else the captured window's
/// browser.
pub fn capture_url(
    front_bundle_id: Option<&str>,
    target_bundle_id: Option<&str>,
) -> Option<String> {
    [front_bundle_id, target_bundle_id]
        .into_iter()
        .flatten()
        .filter(|id| browser_kind(id).is_some())
        .find_map(browser_tab_url)
}

// MARK: Port → project

/// PIDs in `lsof -F p` output (`p1234` lines), in order, without repeats.
pub fn parse_lsof_pids(output: &str) -> Vec<i32> {
    let mut pids = Vec::new();
    for pid in output
        .lines()
        .filter_map(|line| line.strip_prefix('p')?.trim().parse::<i32>().ok())
    {
        if pid > 0 && !pids.contains(&pid) {
            pids.push(pid);
        }
    }
    pids
}

/// The directory in `lsof -a -p PID -d cwd -Fn` output (the `n` line).
pub fn parse_lsof_cwd(output: &str) -> Option<PathBuf> {
    output
        .lines()
        .find_map(|line| line.strip_prefix('n'))
        .map(str::trim)
        .filter(|path| path.starts_with('/'))
        .map(PathBuf::from)
}

fn lsof(args: &[&str]) -> Option<String> {
    let output = run_with_timeout(
        Command::new("/usr/sbin/lsof").args(args),
        Duration::from_secs(3),
    )?;
    // lsof exits 1 when some of what it was asked about wasn't found.
    let text = String::from_utf8_lossy(&output.stdout).into_owned();
    (!text.trim().is_empty()).then_some(text)
}

/// The working directory of the process listening on TCP `port` on this Mac.
pub fn listener_directory(port: u16) -> Option<PathBuf> {
    let pids = parse_lsof_pids(&lsof(&[
        "-nP",
        &format!("-iTCP:{port}"),
        "-sTCP:LISTEN",
        "-Fp",
    ])?);
    pids.into_iter().find_map(|pid| {
        parse_lsof_cwd(&lsof(&["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"])?)
            .filter(|dir| dir != Path::new("/"))
    })
}

/// The project a directory belongs to: its git root (a `.git` directory or worktree file), or
/// the directory itself with `false` when it isn't in a repository.
pub fn project_root(dir: &Path) -> (PathBuf, bool) {
    let mut current = Some(dir);
    while let Some(path) = current {
        if path.join(".git").exists() {
            return (path.to_path_buf(), true);
        }
        current = path.parent();
    }
    (dir.to_path_buf(), false)
}

/// The most recently active open root session for `root`. Inside a repository a session in any
/// folder of it counts; outside one, only a session in that very folder, so a server started
/// from the home folder doesn't claim every session.
///
/// The server filters by exact directory, so that takes one request. A session in a subfolder
/// only wins if it's newer, so the newest-first scan for one stops at the exact match's age (or
/// after [`ROOT_SEARCH_PAGES`](crate::handoff::ROOT_SEARCH_PAGES)).
pub fn session_for_directory(
    get: &mut ApiGet,
    root: &Path,
    whole_tree: bool,
) -> Result<Option<OpenCodeSession>> {
    use crate::handoff::{ROOT_SEARCH_PAGES, Scan, find_root_session, query_escape};
    if smart::is_home_or_root(root) {
        return Ok(None);
    }
    let directory = format!("&directory={}", query_escape(&root.to_string_lossy()));
    let exact = find_root_session(get, &directory, ROOT_SEARCH_PAGES, |_| Scan::Take)?;
    if !whole_tree {
        return Ok(exact);
    }
    let floor = exact.as_ref().map(|s| s.time.updated);
    let nested = find_root_session(get, "", ROOT_SEARCH_PAGES, |s| {
        if floor.is_some_and(|floor| s.time.updated <= floor) {
            Scan::Stop
        } else if s
            .directory()
            .is_some_and(|dir| Path::new(dir).starts_with(root))
        {
            Scan::Take
        } else {
            Scan::Skip
        }
    })?;
    Ok(nested.or(exact))
}

// MARK: Deciding

/// The outside steps of a decision, swappable in tests.
#[derive(Clone, Copy)]
pub struct Hooks {
    pub sdk: fn(&str) -> Option<Registration>,
    pub listener_dir: fn(u16) -> Option<PathBuf>,
    /// One `GET` against OpenCode's API, given up after the time given.
    pub api: fn(&str, Duration) -> Result<Vec<u8>>,
}

impl Default for Hooks {
    fn default() -> Self {
        Self {
            sdk: crate::web::lookup,
            listener_dir: listener_directory,
            api: crate::handoff::api_get_within,
        }
    }
}

/// How long one capture's routing may spend reading sessions, all requests together.
pub const SESSIONS_BUDGET: Duration = Duration::from_secs(3);

/// `api` limited to `budget` in all, counted from its first request: each request gets what's
/// left, and none starts after.
pub fn within(
    api: fn(&str, Duration) -> Result<Vec<u8>>,
    budget: Duration,
) -> impl FnMut(&str) -> Result<Vec<u8>> {
    let mut deadline = None;
    move |path| {
        let deadline = *deadline.get_or_insert_with(|| Instant::now() + budget);
        let left = deadline.saturating_duration_since(Instant::now());
        anyhow::ensure!(
            !left.is_zero(),
            "OpenCode took over {} ms to list sessions",
            budget.as_millis()
        );
        api(path, left)
    }
}

/// Where a capture of `url` goes, or why it goes nowhere.
pub fn route(url: &str, hooks: &Hooks) -> std::result::Result<Destination, String> {
    let parsed = parse_url(url).ok_or_else(|| format!("{url} isn't an http(s) URL"))?;
    let destination = |session: &OpenCodeSession, via: &str, dir: Option<&Path>| Destination {
        session: session.id.clone(),
        title: session.label().to_owned(),
        via: via.to_owned(),
        url: Some(url.to_owned()),
        directory: dir.map(|d| d.display().to_string()),
    };
    let mut api = within(hooks.api, SESSIONS_BUDGET);
    let by_directory = |api: &mut ApiGet, dir: &Path, via: &str| {
        let (root, git) = project_root(dir);
        session_for_directory(api, &root, git)
            .map_err(|err| format!("couldn't list sessions: {err:#}"))?
            .map(|s| destination(&s, via, Some(&root)))
            .ok_or_else(|| format!("no open OpenCode session in {}", root.display()))
    };

    if let Some(reg) = (hooks.sdk)(url) {
        if let Some(id) = reg.session.as_deref() {
            let title = crate::handoff::get_session(&mut api, id)
                .map_or_else(|_| id.to_owned(), |s| s.label().to_owned());
            return Ok(Destination {
                session: id.to_owned(),
                title,
                via: "sdk".into(),
                url: Some(url.to_owned()),
                directory: reg.directory,
            });
        }
        if let Some(dir) = reg.directory.as_deref() {
            return by_directory(&mut api, Path::new(dir), "sdk");
        }
    }
    if !parsed.is_loopback() {
        return Err(format!("{} isn't a local dev server", parsed.origin()));
    }
    let dir = (hooks.listener_dir)(parsed.port)
        .ok_or_else(|| format!("no process found listening on port {}", parsed.port))?;
    by_directory(&mut api, &dir, "localhost")
}

fn routing_enabled() -> bool {
    crate::handoff::auto_send() && std::env::var_os("SCRAWL_NO_OPENCODE").is_none()
}

/// Decides where a capture that just finished goes; `None` (logged) keeps it on the clipboard.
pub fn route_capture(
    front_bundle_id: Option<&str>,
    target_bundle_id: Option<&str>,
) -> Option<Destination> {
    if !routing_enabled() {
        return None;
    }
    let url = capture_url(front_bundle_id, target_bundle_id)?;
    match route_with_sites(&url, &crate::handoff::routing_sites(), &Hooks::default()) {
        Ok(dest) => {
            scrawl_log(format!(
                "routing: {url} → {} ({}) via {}",
                dest.title, dest.session, dest.via
            ));
            Some(dest)
        }
        Err(reason) => {
            scrawl_log(format!("routing: {url} stays on the clipboard: {reason}"));
            None
        }
    }
}

/// Smart routing's first half for a capture that just finished: the tab, terminal and sessions,
/// or `None` when routing is off.
pub fn look_capture(
    front_bundle_id: Option<&str>,
    target_bundle_id: Option<&str>,
    app_pid: Option<i32>,
) -> Option<look::Live> {
    routing_enabled().then(|| {
        look::look_live(
            front_bundle_id,
            target_bundle_id,
            app_pid,
            &look::Senses::default(),
        )
    })
}

/// Smart routing's second half, once the bundle is built: decides, saves the evidence beside
/// the bundle (`RECORD`), and logs the outcome.
pub fn judge_capture(
    dir: &Path,
    bundle: &ScrawlBundle,
    live: look::Live,
) -> (smart::Judgement, Option<String>) {
    let url = live.url.clone();
    if let Some(error) = &live.error {
        scrawl_log(format!("routing: {error}"));
    }
    let evidence = look::evidence_for(bundle, dir, live);
    let judgement = smart::decide(&evidence, smart::Policy::Weigh);
    let (judgement, jev) = if jev::enabled() {
        let (judgement, record) = jev::judge(
            &evidence,
            judgement,
            &jev::capture_state(bundle, url.as_deref()),
            &jev::capture_asking(bundle, &evidence.candidates),
            None,
            &jev::Hooks::default(),
            &jev::Thresholds::configured(),
        );
        let mut record = record;
        let judgement = policy::suggest(
            policy::AutoSend::configured(),
            judgement,
            &mut record,
            &evidence.candidates,
        );
        scrawl_log(format!(
            "routing (jev): {} · {}{}",
            record.band.label(),
            record.reason,
            record
                .ms
                .map(|ms| format!(" · {ms} ms"))
                .unwrap_or_default()
        ));
        jev::log_decision(
            dir,
            &judgement.decision,
            &record,
            bundle.target.app.as_deref(),
        );
        (judgement, Some(record))
    } else {
        (judgement, None)
    };
    if let Err(err) = write_record(dir, &evidence, &judgement, jev.as_ref()) {
        scrawl_log(format!("routing: couldn't save {RECORD}: {err:#}"));
    }
    let outcome = match &judgement.decision {
        smart::Decision::Send { to, .. } => {
            format!("send to {} ({})", to.candidate.label(), to.candidate.id)
        }
        smart::Decision::Offer { choices } => format!(
            "offer {}",
            choices
                .iter()
                .map(|c| c.candidate.label())
                .collect::<Vec<_>>()
                .join(" or ")
        ),
        smart::Decision::Keep { reason } => format!("clipboard only: {reason}"),
    };
    scrawl_log(format!(
        "routing (smart): {} clues, {} sessions → {outcome}",
        evidence.clues.len(),
        evidence.candidates.len()
    ));
    (judgement, url)
}

/// `scrawl route --explain`: what smart routing sees for a bundle and what it decides, beside
/// what the ladder (smart routing off) does, and what Jev said at capture time. `ask_jev` asks
/// Jev again now (one call). Read-only.
pub fn explain_bundle(bundle: &Path, url: Option<&str>, ask_jev: bool) -> Result<String> {
    use std::fmt::Write as _;
    let dir = crate::handoff::resolve_bundle_dir(bundle)?;
    let (_, loaded) = crate::handoff::load_bundle(&dir)?;
    let (evidence, error, replayed) =
        look::evidence_for_bundle(&dir, &loaded, url, &look::Senses::default());
    let judgement = smart::decide(&evidence, smart::Policy::Weigh);
    let mut out = String::new();
    let target = &loaded.target;
    let _ = writeln!(
        out,
        "Capture   {} · {}{} · {}:{:02}",
        dir.file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
        target.app.as_deref().unwrap_or("?"),
        target
            .window_title
            .as_deref()
            .map(|t| format!(" “{t}”"))
            .unwrap_or_default(),
        loaded.duration_ms / 60_000,
        loaded.duration_ms / 1000 % 60
    );
    let tab = url
        .map(str::to_owned)
        .or_else(|| loaded.destination.as_ref().and_then(|d| d.url.clone()))
        .or_else(|| evidence.url.clone());
    if replayed {
        let _ = writeln!(
            out,
            "Evidence  saved at capture time ({RECORD}); the bundle's clues read again with today's rules"
        );
    } else {
        let _ = writeln!(
            out,
            "Evidence  gathered now: sessions as they are today{}",
            if tab.is_none() {
                "; no tab URL (pass --url)"
            } else {
                ""
            }
        );
    }
    if let Some(error) = error {
        let _ = writeln!(out, "Warning   {error}");
    }
    out.push_str(&smart::explain(&evidence, &judgement));
    if let Some(saved) = jev::read_record(&dir) {
        let _ = writeln!(out, "At capture (saved)");
        out.push_str(&jev::explain(&saved));
    }
    if ask_jev {
        // The sessions' last messages as they were when the capture finished.
        let (now, mut record) = jev::judge(
            &evidence,
            judgement.clone(),
            &jev::capture_state(&loaded, evidence.url.as_deref()),
            &jev::capture_asking(&loaded, &evidence.candidates),
            Some(loaded.created_at_ms),
            &jev::Hooks::default(),
            &jev::Thresholds::configured(),
        );
        policy::suggest(
            policy::AutoSend::configured(),
            now,
            &mut record,
            &evidence.candidates,
        );
        let _ = writeln!(out, "Asked now");
        out.push_str(&jev::explain(&record));
    }
    let today = match tab.as_deref() {
        Some(tab) => {
            match route_with_sites(tab, &crate::handoff::routing_sites(), &Hooks::default()) {
                Ok(dest) => format!("send to {} ({}) via {}", dest.title, dest.session, dest.via),
                Err(reason) => format!("clipboard only: {reason}"),
            }
        }
        None => "clipboard only: not a browser tab".into(),
    };
    let _ = writeln!(out, "Flag off  {today}");
    Ok(out)
}

/// The evidence and decision behind a capture's routing, saved beside it so
/// `scrawl route --explain` replays the same decision.
pub const RECORD: &str = "routing.json";

pub fn write_record(
    dir: &Path,
    evidence: &smart::Evidence,
    judgement: &smart::Judgement,
    jev: Option<&jev::Record>,
) -> Result<()> {
    let mut body = serde_json::json!({ "evidence": evidence, "decision": judgement.decision });
    if let Some(jev) = jev {
        body["jev"] = serde_json::to_value(jev)?;
    }
    std::fs::write(dir.join(RECORD), serde_json::to_vec_pretty(&body)?)?;
    Ok(())
}

/// Counts Kit's pick of `session` for the capture in `dir` (a chip, or a send left to stand)
/// toward a `routing.sites` or `routing.apps` rule, with Jev routing on. Once two agree, the
/// rule to offer is saved in its `routing.json` as `offerRule` and returned.
pub fn learn_capture(dir: &Path, session: &str) -> Option<policy::OfferRule> {
    if !jev::enabled() {
        return None;
    }
    let path = dir.join(RECORD);
    let mut body: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
    let evidence: smart::Evidence = serde_json::from_value(body.get("evidence")?.clone()).ok()?;
    // A rule, port or dev build that sent it already says where it goes.
    if let Some(smart::Decision::Send { to, .. }) = body
        .get("decision")
        .and_then(|d| serde_json::from_value(d.clone()).ok())
        && to.candidate.id == session
        && policy::obeyed(&to)
    {
        return None;
    }
    let candidate = evidence.candidates.iter().find(|c| c.id == session)?;
    let folder = policy::rule_folder(candidate)?;
    let bundle_id = crate::handoff::load_bundle(dir)
        .ok()
        .and_then(|(_, bundle)| bundle.target.bundle_id);
    let offer = policy::learn_pick(evidence.url.as_deref(), bundle_id.as_deref(), &folder)?;
    body["offerRule"] = serde_json::to_value(&offer).ok()?;
    if let Ok(bytes) = serde_json::to_vec_pretty(&body) {
        let _ = std::fs::write(&path, bytes);
    }
    Some(offer)
}

pub fn read_record(dir: &Path) -> Option<smart::Evidence> {
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join(RECORD)).ok()?).ok()?;
    serde_json::from_value(value.get("evidence")?.clone()).ok()
}

/// Writes `dest` into the bundle's `scrawl.json` and `summary.md`.
pub fn record_destination(dir: &Path, bundle: &mut ScrawlBundle, dest: Destination) -> Result<()> {
    bundle.destination = Some(dest);
    std::fs::write(dir.join("scrawl.json"), serde_json::to_vec_pretty(bundle)?)?;
    std::fs::write(
        dir.join("summary.md"),
        crate::sketch::summary_markdown(bundle, dir),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, dir: &str, updated: u64) -> OpenCodeSession {
        crate::handoff::fake_api::session(id, &id.to_uppercase(), dir, updated)
    }

    #[test]
    fn urls_parse_to_origins() {
        let u = parse_url("http://localhost:5173/a/b?c#d").unwrap();
        assert_eq!((u.host.as_str(), u.port), ("localhost", 5173));
        assert_eq!(u.origin(), "http://localhost:5173");
        assert!(u.is_loopback());
        assert_eq!(
            parse_url("HTTPS://User:pw@App.Example.com/x")
                .unwrap()
                .origin(),
            "https://app.example.com"
        );
        assert_eq!(parse_url("http://example.com").unwrap().port, 80);
        let v6 = parse_url("http://[::1]:3000/").unwrap();
        assert_eq!((v6.host.as_str(), v6.port), ("::1", 3000));
        assert_eq!(v6.origin(), "http://[::1]:3000");
        assert!(v6.is_loopback());
        assert!(parse_url("http://127.0.0.1:8080").unwrap().is_loopback());
        assert!(
            parse_url("http://my-app.localhost:1355/")
                .unwrap()
                .is_loopback()
        );
        assert!(
            !parse_url("http://studio.example.ts.net:5173/")
                .unwrap()
                .is_loopback()
        );
        assert!(
            !parse_url("http://localhost.evil.com/")
                .unwrap()
                .is_loopback()
        );
        assert!(parse_url("file:///tmp/x.html").is_none());
        assert!(parse_url("chrome://newtab").is_none());
        assert!(parse_url("http://localhost:notaport/").is_none());
        assert!(parse_url("http:///nohost").is_none());
    }

    #[test]
    fn browsers_are_recognized_by_bundle_id() {
        assert_eq!(browser_kind("com.brave.Browser"), Some(Browser::Chromium));
        assert_eq!(
            browser_kind("com.brave.Browser.beta"),
            Some(Browser::Chromium)
        );
        assert_eq!(
            browser_kind("com.google.Chrome.canary"),
            Some(Browser::Chromium)
        );
        assert_eq!(
            browser_kind("company.thebrowser.Browser"),
            Some(Browser::Chromium)
        );
        assert_eq!(browser_kind("com.apple.Safari"), Some(Browser::Safari));
        assert_eq!(browser_kind("com.mitchellh.ghostty"), None);
        assert_eq!(browser_kind("com.google.Chromecast"), None);
    }

    #[test]
    fn lsof_output_parses() {
        let listen = "p48211\ncnode\np48211\np50002\n";
        assert_eq!(parse_lsof_pids(listen), vec![48211, 50002]);
        assert!(parse_lsof_pids("").is_empty());
        let cwd = "p48211\nfcwd\nn/Users/kit/code/open-source/scrawl/site\n";
        assert_eq!(
            parse_lsof_cwd(cwd),
            Some(PathBuf::from("/Users/kit/code/open-source/scrawl/site"))
        );
        assert_eq!(parse_lsof_cwd("p1\nfcwd\n"), None);
    }

    #[test]
    fn project_root_walks_up_to_git() {
        let base = std::env::temp_dir().join(format!("scrawl-root-{}", crate::make_bundle_id()));
        let site = base.join("repo/site/src");
        std::fs::create_dir_all(&site).unwrap();
        std::fs::create_dir_all(base.join("repo/.git")).unwrap();
        assert_eq!(project_root(&site), (base.join("repo"), true));
        let loose = base.join("loose");
        std::fs::create_dir_all(&loose).unwrap();
        // The temp folder isn't in a repository.
        assert_eq!(project_root(&loose), (loose.clone(), false));
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn site_rules_cover_their_host_and_paths_below() {
        let sites = vec![
            (
                "https://problems.example.com".to_owned(),
                PathBuf::from("/p/board"),
            ),
            (
                "problems.example.com/docs/".to_owned(),
                PathBuf::from("ses_docs"),
            ),
            (
                "http://localhost:4000".to_owned(),
                PathBuf::from("/p/local"),
            ),
        ];
        let rule = |url: &str| site_rule(url, &sites).map(|(_, to)| to);
        let board = Some(SiteTarget::Folder("/p/board".into()));
        assert_eq!(
            rule("https://problems.example.com/tasks/OCN-16?project=x"),
            board
        );
        assert_eq!(rule("https://PROBLEMS.example.com"), board);
        assert_eq!(rule("https://problems.example.com/docsy"), board);
        // The longer prefix wins, scheme or not.
        assert_eq!(
            rule("http://problems.example.com/docs/a#b"),
            Some(SiteTarget::Session("ses_docs".into()))
        );
        assert_eq!(
            rule("https://problems.example.com/docs"),
            Some(SiteTarget::Session("ses_docs".into()))
        );
        // A scheme given pins it and the port.
        assert_eq!(rule("http://problems.example.com/"), None);
        assert_eq!(rule("https://problems.example.com:8443/"), None);
        assert_eq!(rule("https://other.example.com/"), None);
        assert_eq!(rule("https://sub.problems.example.com/"), None);
        assert_eq!(
            rule("http://localhost:4000/x"),
            Some(SiteTarget::Folder("/p/local".into()))
        );
        assert_eq!(rule("http://localhost:4001/x"), None);
    }

    #[test]
    fn sessions_are_chosen_by_directory() {
        let mut child = session("ses_child", "/p/app", 900);
        child.parent_id = Some("ses_app".into());
        let mut archived = session("ses_old", "/p/app", 800);
        archived.time.archived = Some(1);
        let mut sessions = vec![
            session("ses_app", "/p/app", 100),
            session("ses_site", "/p/app/site", 200),
            session("ses_other", "/p/application", 300),
            child,
            archived,
        ];
        // Thousands of newer sessions elsewhere.
        sessions.extend((0..3_000).map(|i| session(&format!("ses_x{i}"), "/q", 1_000 + i)));
        let pick = |sessions: &[OpenCodeSession], root: &str, tree| {
            let mut requests = 0;
            let found = session_for_directory(
                &mut |path: &str| {
                    requests += 1;
                    crate::handoff::fake_api::serve(sessions, path)
                },
                Path::new(root),
                tree,
            )
            .unwrap()
            .map(|s| s.id);
            (found, requests)
        };
        // Exact folder: one filtered request.
        assert_eq!(pick(&sessions, "/p/app", false), (Some("ses_app".into()), 1));
        // A subfolder needs a newest-first scan, which is capped: past it the exact match stands.
        let (found, requests) = pick(&sessions, "/p/app", true);
        assert_eq!(found.as_deref(), Some("ses_app"));
        assert_eq!(requests, 1 + crate::handoff::ROOT_SEARCH_PAGES);
        assert_eq!(pick(&sessions, "/p/none", false), (None, 1));
        assert_eq!(pick(&sessions, "/", true), (None, 0));

        // Recent work in a subfolder beats an older exact match; a sibling with a common prefix
        // doesn't count.
        sessions.truncate(5);
        let (found, requests) = pick(&sessions, "/p/app", true);
        assert_eq!(found.as_deref(), Some("ses_site"));
        assert_eq!(requests, 2);
        // An exact match newer than everything else stops the scan at once.
        sessions[0].time.updated = 1_000;
        assert_eq!(pick(&sessions, "/p/app", true), (Some("ses_app".into()), 2));
    }

    fn no_sdk(_: &str) -> Option<Registration> {
        None
    }
    fn sdk_session(url: &str) -> Option<Registration> {
        url.starts_with("http://localhost:5173")
            .then(|| Registration {
                origin: "http://localhost:5173".into(),
                url: url.into(),
                session: Some("ses_sdk".into()),
                ..Registration::default()
            })
    }
    fn sdk_directory(_: &str) -> Option<Registration> {
        Some(Registration {
            origin: "https://studio.ts.net:5173".into(),
            directory: Some("/p/app".into()),
            ..Registration::default()
        })
    }
    fn listener(port: u16) -> Option<PathBuf> {
        (port == 4321).then(|| PathBuf::from("/p/app"))
    }
    fn api(path: &str, _: Duration) -> Result<Vec<u8>> {
        crate::handoff::fake_api::serve(
            &[
                session("ses_sdk", "/elsewhere", 5),
                session("ses_app", "/p/app", 10),
            ],
            path,
        )
    }

    fn down(_: &str, _: Duration) -> Result<Vec<u8>> {
        anyhow::bail!("OpenCode API failed: Error: Could not reach server at http://127.0.0.1:4096")
    }
    /// Answers each request after 200 ms, or fails once the time it's given runs out.
    fn slow(path: &str, left: Duration) -> Result<Vec<u8>> {
        let delay = Duration::from_millis(200);
        std::thread::sleep(delay.min(left));
        anyhow::ensure!(left >= delay, "no answer");
        let sessions: Vec<_> = (0..250)
            .map(|i| session(&format!("ses_{i}"), "/q", 1_000 + i))
            .chain([session("ses_app", "/p/app", 10)])
            .collect();
        crate::handoff::fake_api::serve(&sessions, path)
    }

    #[test]
    fn a_down_or_slow_server_costs_routing_a_bounded_time() {
        let hooks = Hooks {
            sdk: no_sdk,
            listener_dir: listener,
            api: down,
        };
        let started = Instant::now();
        let err = route("http://127.0.0.1:4321/", &hooks).unwrap_err();
        assert!(err.contains("Could not reach server"), "{err}");
        assert!(started.elapsed() < Duration::from_millis(100));

        // Every request shares one budget, counted from the first: reading pages stops when
        // it's spent instead of waiting out each request in turn.
        let budget = Duration::from_millis(500);
        let mut requests = 0;
        let mut api = within(slow, budget);
        let started = Instant::now();
        let result = crate::routing::look::read_candidates(
            &mut |path: &str| {
                requests += 1;
                api(path)
            },
            &[PathBuf::from("/p/app"), PathBuf::from("/p/other")],
            &[],
            look::SESSION_PAGES,
        );
        assert!(result.is_err());
        // Two pages answer, the first folder's read gets the last 100 ms and fails, and the
        // second is never asked: 500 ms, not the 800 ms four requests take.
        let took = started.elapsed();
        assert!((2..4).contains(&requests), "{requests}");
        assert!(took < Duration::from_millis(700), "{took:?}");

        // A server that answers in time routes as usual.
        let hooks = Hooks { api: slow, ..hooks };
        let local = route("http://127.0.0.1:4321/", &hooks).unwrap();
        assert_eq!(local.session, "ses_app");
    }

    #[test]
    fn route_prefers_sdk_then_localhost_port() {
        let hooks = Hooks {
            sdk: sdk_session,
            listener_dir: listener,
            api,
        };
        let sdk = route("http://localhost:5173/page", &hooks).unwrap();
        assert_eq!((sdk.session.as_str(), sdk.via.as_str()), ("ses_sdk", "sdk"));
        assert_eq!(sdk.title, "SES_SDK");

        let hooks = Hooks {
            sdk: no_sdk,
            ..hooks
        };
        // `/p/app` isn't a repository here, so only a session in exactly that folder matches.
        let local = route("http://127.0.0.1:4321/", &hooks).unwrap();
        assert_eq!(
            (
                local.session.as_str(),
                local.via.as_str(),
                local.directory.as_deref()
            ),
            ("ses_app", "localhost", Some("/p/app"))
        );
        assert!(route("http://localhost:9999/", &hooks).is_err());
        assert!(route("https://example.com/", &hooks).is_err());

        let hooks = Hooks {
            sdk: sdk_directory,
            ..hooks
        };
        let by_dir = route("https://studio.ts.net:5173/", &hooks).unwrap();
        assert_eq!(
            (by_dir.session.as_str(), by_dir.via.as_str()),
            ("ses_app", "sdk")
        );
    }

    #[test]
    fn destination_is_recorded_and_preferred_by_send() {
        let dir =
            std::env::temp_dir().join(format!("scrawl-route-{}.scrawl", crate::make_bundle_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut bundle: ScrawlBundle = serde_json::from_value(serde_json::json!({
            "version": 1, "id": "t", "createdAtMs": 0, "durationMs": 1_000,
            "target": {"mode": "display", "displayId": 1,
                "frame": {"x": 0.0, "y": 0.0, "width": 10.0, "height": 10.0, "scale": 1.0}},
            "transcript": {"source": "none", "text": "", "segments": []},
            "strokes": [], "marks": [], "events": [], "keyframes": [], "contactSheet": "c.png"
        }))
        .unwrap();
        assert_eq!(crate::handoff::recorded_destination(&dir), None);
        record_destination(
            &dir,
            &mut bundle,
            Destination {
                session: "ses_app".into(),
                title: "APP".into(),
                via: "localhost".into(),
                url: Some("http://localhost:4321/".into()),
                directory: None,
            },
        )
        .unwrap();
        let summary = std::fs::read_to_string(dir.join("summary.md")).unwrap();
        assert!(
            summary.contains("For session: APP (ses_app) · page http://localhost:4321/"),
            "{summary}"
        );
        let (_, loaded) = crate::handoff::load_bundle(&dir).unwrap();
        assert_eq!(loaded.destination.unwrap().via, "localhost");
        assert_eq!(
            crate::handoff::recorded_destination(&dir).as_deref(),
            Some("ses_app")
        );
        std::fs::remove_dir_all(dir).unwrap();
    }
}
