//! Gathering [`Evidence`] for smart routing: the browser tab, the process serving it, the
//! terminal's folder, the open sessions, and what the bundle says. All the I/O lives here;
//! [`smart::decide`](super::smart::decide) only reads what this returns.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;
use std::time::Duration;

use anyhow::Result;

use super::served;
use super::smart::{Address, Candidate, Clue, Evidence, Folder, Pointer, Source, words};
use super::{Hooks, WebUrl, parse_url, project_root, run_with_timeout};
use crate::handoff::{
    ApiGet, OpenCodeSession, Scan, find_root_session, get_session, normalize_slug, query_escape,
};
use crate::model::ScrawlBundle;

/// Pages of newest root sessions read (100 each), besides the exact-folder and by-ID reads.
pub const SESSION_PAGES: usize = 2;
/// OpenCode Desktop counts as showing the session viewed last if that was this recently.
const VIEWED_WITHIN_MS: u64 = 30 * 60_000;
const GHOSTTY: &str = "com.mitchellh.ghostty";

/// The outside steps of looking, swappable in tests.
#[derive(Clone, Copy)]
pub struct Senses {
    pub hooks: Hooks,
    /// The port on this Mac that serves `url`, when its host is this Mac.
    pub own_port: fn(&WebUrl) -> Option<u16>,
    /// The folder and title of a terminal app's focused pane.
    pub terminal: fn(&str) -> Option<(PathBuf, String)>,
    /// URL prefixes and the folders they serve (`routing.servers`).
    pub servers: fn() -> Vec<(String, PathBuf)>,
    /// URL prefixes and the folder or session their captures go to (`routing.sites`).
    pub sites: fn() -> Vec<(String, PathBuf)>,
    /// Bundle IDs and the folder or session their captures go to (`routing.apps`).
    pub apps: fn() -> Vec<(String, PathBuf)>,
    /// Spoken names and the title or folder they mean (`routing.aliases`).
    pub aliases: fn() -> Vec<(String, String)>,
    /// Tool calls in OpenCode's history that named a folder around the time a file in it changed.
    pub writes: fn(&str, u64, u64) -> Vec<served::Write>,
    /// The executable a process runs, for a dev build's checkout.
    pub executable: fn(i32) -> Option<PathBuf>,
}

impl Default for Senses {
    fn default() -> Self {
        Self {
            hooks: Hooks::default(),
            own_port,
            terminal: terminal_pane,
            servers: crate::handoff::routing_servers,
            sites: crate::handoff::routing_sites,
            apps: crate::handoff::routing_apps,
            aliases: crate::handoff::routing_aliases,
            writes: served::opencode_writes,
            executable: super::policy::executable_of,
        }
    }
}

/// What can be seen when a capture finishes, before its bundle exists.
#[derive(Clone, Debug, Default)]
pub struct Live {
    pub url: Option<String>,
    pub clues: Vec<Clue>,
    pub candidates: Vec<Candidate>,
    /// Why the sessions couldn't be read, when they couldn't.
    pub error: Option<String>,
}

/// Looks at the browser tab or terminal in front (`front`) or captured (`target`), and reads
/// the open sessions. Runs while the bundle is built. `pid` is the captured app's process, whose
/// executable may be a dev build (Jev routing).
pub fn look_live(
    front: Option<&str>,
    target: Option<&str>,
    pid: Option<i32>,
    senses: &Senses,
) -> Live {
    let url = super::capture_url(front, target);
    let mut clues = url
        .as_deref()
        .map(|url| url_clues(url, senses, true))
        .unwrap_or_default();
    if super::jev::enabled()
        && let Some(exe) = pid.and_then(senses.executable)
    {
        clues.extend(dev_build_clue(target.or(front), &exe));
    }
    if let Some((cwd, title)) = [front, target]
        .into_iter()
        .flatten()
        .find_map(|id| (senses.terminal)(id))
    {
        clues.push(Clue::new(
            Source::Terminal,
            Pointer::Folder(folder_of(&cwd)),
            format!("the terminal's focused pane is in {}", cwd.display()),
        ));
        let title_words = words(&title);
        if !title_words.is_empty() {
            clues.push(Clue::new(
                Source::Title,
                Pointer::Words { words: title_words },
                format!("{TERMINAL_TITLE} \"{title}\""),
            ));
        }
    }
    let (mut candidates, error) = read_for(&clues, None, senses);
    look_at_page(url.as_deref(), &mut clues, &mut candidates, senses);
    Live {
        url,
        clues,
        candidates,
        error,
    }
}

/// How long finding a page's publisher may spend on OpenCode's API (resolving a marker's
/// subagent session to its root, reading sessions not read yet).
const PUBLISHER_BUDGET: Duration = Duration::from_secs(1);

/// When the server behind the tab serves the page from outside its own project (a shared
/// artifact gallery), or runs inside a folder sessions publish into (`served::content_root`),
/// the port's clue drops (to [`served::SERVER_ONLY`] or [`served::SERVER_IN_CONTENT`]) and the
/// page's files give the clues instead: who published them, and the words of their path.
/// Sessions those name that weren't read are added to `candidates`. Does nothing twice.
pub fn look_at_page(
    url: Option<&str>,
    clues: &mut Vec<Clue>,
    candidates: &mut Vec<Candidate>,
    senses: &Senses,
) {
    if clues
        .iter()
        .any(|c| matches!(c.source, Source::Publisher | Source::Document))
    {
        return;
    }
    let Some(port) = clues
        .iter_mut()
        .find(|c| c.source == Source::Port && c.points.is_none())
    else {
        return;
    };
    let Pointer::Folder(folder) = &port.pointer else {
        return;
    };
    let dir = folder.path.clone();
    let servers = (senses.servers)();
    let mut roots: Vec<PathBuf> = candidates
        .iter()
        .flat_map(|c| [c.directory.clone(), c.repo.clone()])
        .flatten()
        .collect();
    roots.sort();
    roots.dedup();
    let elsewhere = url.and_then(|url| match served::locate(url, &dir, &servers, &roots) {
        served::Served::Elsewhere(page) => Some(page),
        _ => None,
    });
    let (page, changed, what) = if let Some(page) = elsewhere {
        port.points = Some(served::SERVER_ONLY);
        port.note = format!(
            "{}, but the page is {}, from outside it",
            port.note,
            page.file.display()
        );
        let changed: Vec<u64> = page.modified_ms().into_iter().collect();
        let what = page.file.file_name().map_or_else(
            || page.file.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        (page, changed, what)
    } else if let Some(root) = served::content_root(&dir, &servers)
        && !candidates
            .iter()
            .any(|c| c.directory.as_deref().is_some_and(|d| d.starts_with(&dir)))
    {
        port.points = Some(served::SERVER_IN_CONTENT);
        port.note = format!(
            "{}, inside {}, where sessions publish: its repository says nothing about who made it",
            port.note,
            root.display()
        );
        let what = format!(
            "{}/",
            dir.file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
        );
        let page = served::Page {
            file: dir.join("index.html"),
            root,
        };
        (page, served::changes(&dir), what)
    } else {
        return;
    };
    let mut api = super::within(senses.hooks.api, PUBLISHER_BUDGET);
    let mut found = Vec::new();
    if let Some((id, how)) = served::marker(&page) {
        let id = root_session(&mut api, &id);
        found.push(
            Clue::new(
                Source::Publisher,
                Pointer::Session {
                    id,
                    directory: None,
                },
                how,
            )
            .worth(served::MARKED),
        );
    } else if let (Some(needle), Some((from, to))) = (page.needle(), served::window(&changed)) {
        found.extend(served::writer_clues(
            &changed,
            &what,
            &(senses.writes)(&needle, from, to),
        ));
    }
    found.extend(served::path_clue(&page));
    let missing: Vec<String> = found
        .iter()
        .filter_map(|clue| match &clue.pointer {
            Pointer::Session { id, .. } if !candidates.iter().any(|c| &c.id == id) => {
                Some(id.clone())
            }
            _ => None,
        })
        .collect();
    if !missing.is_empty()
        && let Ok(more) = read_candidates(&mut api, &[], &missing, 0)
    {
        candidates.extend(more);
    }
    clues.extend(found);
}

/// The root session of `id` (a subagent's session names its parent), or `id` when that can't
/// be read.
fn root_session(get: &mut ApiGet, id: &str) -> String {
    let mut current = id.to_owned();
    for _ in 0..5 {
        let Some(parent) = get_session(get, &current).ok().and_then(|s| s.parent_id) else {
            break;
        };
        current = parent;
    }
    current
}

/// Reads the sessions the clues and address could name.
pub(crate) fn read_for(
    clues: &[Clue],
    address: Option<&Address>,
    senses: &Senses,
) -> (Vec<Candidate>, Option<String>) {
    read_with(clues, address, &[], senses)
}

/// [`read_for`], also reading the sessions `also` names.
pub(crate) fn read_with(
    clues: &[Clue],
    address: Option<&Address>,
    also: &[String],
    senses: &Senses,
) -> (Vec<Candidate>, Option<String>) {
    let mut roots = Vec::new();
    let mut ids = Vec::new();
    for clue in clues {
        match &clue.pointer {
            Pointer::Folder(folder) => roots.push(folder.root.clone()),
            Pointer::Session { id, .. } => ids.push(id.clone()),
            Pointer::Words { .. } => {}
        }
    }
    ids.extend(address.map(|a| a.session().to_owned()));
    ids.extend(also.iter().cloned());
    let mut api = super::within(senses.hooks.api, super::SESSIONS_BUDGET);
    match read_candidates(&mut api, &roots, &ids, SESSION_PAGES) {
        Ok(candidates) => (candidates, None),
        Err(err) => (Vec::new(), Some(format!("couldn't list sessions: {err:#}"))),
    }
}

/// The clues a tab gives: its `/sdk.js` registration, and the folder of the process serving
/// its port when the tab is on this Mac (a loopback host, or with `own_host` this Mac's own
/// name).
pub fn url_clues(url: &str, senses: &Senses, own_host: bool) -> Vec<Clue> {
    let mut clues = Vec::new();
    clues.extend(rule_clue(url, senses));
    if let Some(reg) = (senses.hooks.sdk)(url) {
        if let Some(id) = reg.session.clone() {
            clues.push(Clue::new(
                Source::Page,
                Pointer::Session {
                    id,
                    directory: reg.directory.as_deref().map(PathBuf::from),
                },
                format!("{} registered with data-session", reg.origin),
            ));
        } else if let Some(dir) = reg.directory.as_deref() {
            clues.push(Clue::new(
                Source::Page,
                Pointer::Folder(folder_of(Path::new(dir))),
                format!("{} registered with data-directory", reg.origin),
            ));
        }
    }
    let Some(parsed) = parse_url(url) else {
        return clues;
    };
    let port = if parsed.is_loopback() {
        Some(parsed.port)
    } else if own_host {
        (senses.own_port)(&parsed)
    } else {
        None
    };
    if let Some(port) = port
        && let Some(dir) = (senses.hooks.listener_dir)(port)
    {
        let via = if port == parsed.port || parsed.is_loopback() {
            format!("port {port}")
        } else {
            format!("{} (port {port} here)", parsed.origin())
        };
        clues.push(Clue::new(
            Source::Port,
            Pointer::Folder(folder_of(&dir)),
            format!("{via} is served from {}", dir.display()),
        ));
    }
    clues
}

/// The clue a `routing.sites` rule covering `url` gives.
fn rule_clue(url: &str, senses: &Senses) -> Option<Clue> {
    let (prefix, target) = super::site_rule(url, &(senses.sites)())?;
    let (pointer, to) = match target {
        super::SiteTarget::Session(id) => {
            let to = id.clone();
            (Pointer::Session { id, directory: None }, to)
        }
        super::SiteTarget::Folder(dir) => (
            Pointer::Folder(Folder {
                tree: true,
                ..folder_of(&dir)
            }),
            dir.display().to_string(),
        ),
    };
    Some(Clue::new(
        Source::Rule,
        pointer,
        format!("routing.sites sends {prefix} to {to}"),
    ))
}

/// The clue a dev build gives: the git working tree `executable` runs from, when the app isn't a
/// general-purpose one (a browser or terminal built from source is still about anything).
pub fn dev_build_clue(bundle_id: Option<&str>, executable: &Path) -> Option<Clue> {
    if bundle_id.is_some_and(super::policy::is_general_app) {
        return None;
    }
    let root = super::policy::dev_build_root(executable)?;
    let name = executable
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    Some(Clue::new(
        Source::DevBuild,
        Pointer::Folder(Folder {
            tree: true,
            ..folder_of(&root)
        }),
        format!("the app {name} runs from {}", root.display()),
    ))
}

/// The clue a `routing.apps` rule covering `bundle_id` gives (Jev routing).
pub fn app_rule_clue(bundle_id: &str, apps: &[(String, PathBuf)]) -> Option<Clue> {
    let (key, to) = super::policy::app_rule(bundle_id, apps)?;
    let text = to.to_string_lossy();
    let pointer = if text.starts_with("ses_") && !text.contains('/') {
        Pointer::Session {
            id: text.into_owned(),
            directory: None,
        }
    } else {
        Pointer::Folder(Folder {
            tree: true,
            ..folder_of(&to)
        })
    };
    Some(Clue::new(
        Source::AppRule,
        pointer,
        format!("routing.apps sends {key} to {}", to.display()),
    ))
}

/// The session `text` opens by addressing ("tell scrawl …"), as a [`Source::Named`] clue.
pub fn named_clue(
    text: &str,
    candidates: &[Candidate],
    aliases: &[(String, String)],
) -> Option<Clue> {
    let named = super::named::addressed(text, candidates, aliases)?;
    Some(Clue::new(
        Source::Named,
        Pointer::Session {
            id: named.id,
            directory: None,
        },
        format!(
            "opens by naming \u{201c}{}\u{201d} ({})",
            named.spoken, named.how
        ),
    ))
}

/// A named-address clue for a capture's speech, with Jev routing on.
fn spoken_address(bundle: &ScrawlBundle, candidates: &[Candidate]) -> Option<Clue> {
    if !super::jev::enabled() {
        return None;
    }
    named_clue(
        &bundle.transcript.text,
        candidates,
        &crate::handoff::routing_aliases(),
    )
}

/// The rest of the evidence, from the finished bundle: its title, what was said, the session
/// OpenCode Desktop showed, the checkout the captured app is built from, and the destination the
/// recording carries.
pub fn evidence_for(bundle: &ScrawlBundle, dir: &Path, live: Live) -> Evidence {
    let mut clues = live.clues;
    clues.extend(bundle_clues(
        bundle,
        &live.candidates,
        &crate::handoff::routing_apps(),
    ));
    clues.extend(spoken_address(bundle, &live.candidates));
    Evidence {
        address: crate::handoff::recorded_destination(dir).map(Address::Recorded),
        clues,
        candidates: live.candidates,
        now_ms: bundle.created_at_ms,
        url: live.url,
    }
}

/// A saved `routing.json`'s evidence with the clues the bundle gives read again under today's
/// rules: what was seen live (the tab, the port, the terminal) and the sessions are kept as saved.
pub fn replay(saved: Evidence, bundle: &ScrawlBundle, apps: &[(String, PathBuf)]) -> Evidence {
    let mut clues: Vec<Clue> = saved
        .clues
        .into_iter()
        .filter(|clue| match clue.source {
            Source::Rule
            | Source::DevBuild
            | Source::Page
            | Source::Port
            | Source::Publisher
            | Source::Terminal
            | Source::Document => true,
            Source::Title => clue.note.starts_with(TERMINAL_TITLE),
            Source::Named
            | Source::AppRule
            | Source::Viewed
            | Source::App
            | Source::Speech
            | Source::CursorEdit => false,
        })
        .collect();
    clues.extend(bundle_clues(bundle, &saved.candidates, apps));
    clues.extend(spoken_address(bundle, &saved.candidates));
    Evidence { clues, ..saved }
}

/// The note of a terminal pane's title clue, which a replay keeps.
const TERMINAL_TITLE: &str = "terminal title";

/// Points for the captured app's own checkout, and for the session OpenCode Desktop showed, by
/// what an OpenCode capture is about. Another app's checkout is always [`APP_REPO`].
const APP_REPO: u32 = 70;
fn opencode_points(about: &About) -> (u32, u32) {
    match about {
        About::Interface(_) => (APP_REPO, 15),
        About::Unclear => (60, 30),
        About::Session(_) => (30, 60),
    }
}

/// The clues a finished bundle gives. `apps` maps bundle IDs to the checkouts they're built from
/// (`routing.apps` in config.json); otherwise an open session's repository named like the app is
/// its checkout.
pub fn bundle_clues(
    bundle: &ScrawlBundle,
    candidates: &[Candidate],
    apps: &[(String, PathBuf)],
) -> Vec<Clue> {
    let mut clues = Vec::new();
    let target = &bundle.target;
    if let Some(title) = target.window_title.as_deref() {
        let app = words(target.app.as_deref().unwrap_or(""));
        let title_words: Vec<String> = words(title)
            .into_iter()
            .filter(|w| !app.contains(w))
            .collect();
        if !title_words.is_empty() {
            clues.push(Clue::new(
                Source::Title,
                Pointer::Words { words: title_words },
                format!("window title \"{title}\""),
            ));
        }
    }
    let bundle_id = target.bundle_id.as_deref().unwrap_or("");
    let app = target.app.as_deref().unwrap_or(bundle_id);
    if !app.is_empty() && !is_tool(bundle_id) {
        let last = bundle_id.rsplit('.').next().unwrap_or("");
        let app_words = words(&format!("{app} {last}"));
        if !app_words.is_empty() {
            clues.push(Clue::new(
                Source::App,
                Pointer::Words { words: app_words },
                format!("captured app {app}"),
            ));
        }
    }
    let opencode = is_opencode(bundle_id);
    let about = opencode.then(|| about(bundle));
    let app_rule = super::jev::enabled()
        .then(|| app_rule_clue(bundle_id, apps))
        .flatten();
    let ruled = app_rule.is_some();
    clues.extend(app_rule);
    if !ruled && let Some((folder, configured)) = app_checkout(target, candidates, apps) {
        let (points, why) = match &about {
            Some(about) => (opencode_points(about).0, about.why()),
            None => (APP_REPO, String::new()),
        };
        let note = format!(
            "{app} is built from {}{}{why}",
            folder.path.display(),
            if configured { " (routing.apps)" } else { "" }
        );
        clues.push(Clue::new(Source::App, Pointer::Folder(folder), note).worth(points));
    }
    let said = words(&bundle.transcript.text);
    if !said.is_empty() {
        clues.push(Clue::new(
            Source::Speech,
            Pointer::Words { words: said },
            "said",
        ));
    }
    if let Some(about) = &about
        && let Some((viewed, ms)) = last_viewed(candidates, bundle.created_at_ms)
    {
        let note = format!(
            "OpenCode Desktop showed {} last ({} before the capture ended){}",
            viewed.label(),
            super::smart::ago_text(bundle.created_at_ms.saturating_sub(ms)),
            about.short()
        );
        clues.push(
            Clue::new(
                Source::Viewed,
                Pointer::Session {
                    id: viewed.id.clone(),
                    directory: None,
                },
                note,
            )
            .worth(opencode_points(about).1),
        );
    }
    clues
}

fn is_opencode(bundle_id: &str) -> bool {
    bundle_id
        .to_ascii_lowercase()
        .starts_with("ai.opencode.desktop")
}

/// Browsers, terminals, OpenCode and the system's own apps: their names say nothing about the
/// project.
fn is_tool(bundle_id: &str) -> bool {
    const PREFIXES: [&str; 8] = [
        "com.apple.",
        "ai.opencode.",
        "com.mitchellh.ghostty",
        "com.googlecode.iterm2",
        "dev.zed.",
        "com.microsoft.vscode",
        "com.todesktop.",
        "com.tinyspeck.",
    ];
    let id = bundle_id.to_ascii_lowercase();
    super::browser_kind(&id).is_some() || PREFIXES.iter().any(|p| id.starts_with(p))
}

/// The checkout the captured app is built from, and whether config named it: `apps`' entry for
/// the bundle ID (or a prefix of it), else the one open session repository whose folder is
/// named like the app ("ai.opencode.desktop.beta" → `…/opencode`). Browsers, terminals and
/// system apps have none, OpenCode aside.
fn app_checkout(
    target: &crate::model::TargetInfo,
    candidates: &[Candidate],
    apps: &[(String, PathBuf)],
) -> Option<(Folder, bool)> {
    let id = target.bundle_id.as_deref()?.to_ascii_lowercase();
    let configured = apps
        .iter()
        .filter(|(key, _)| {
            let key = key.to_ascii_lowercase();
            id == key || id.starts_with(&format!("{key}."))
        })
        .max_by_key(|(key, _)| key.len());
    if let Some((_, path)) = configured {
        return Some((folder_of(path), true));
    }
    if is_tool(&id) && !is_opencode(&id) {
        return None;
    }
    let names = app_names(target.app.as_deref().unwrap_or(""), &id);
    let mut found: Vec<(&Path, bool)> = Vec::new();
    for candidate in candidates {
        let (path, repo) = match (&candidate.repo, &candidate.directory) {
            (Some(repo), _) => (repo.as_path(), true),
            (None, Some(dir)) => (dir.as_path(), false),
            (None, None) => continue,
        };
        let named = path
            .file_name()
            .is_some_and(|name| names.contains(&normalize_slug(&name.to_string_lossy())));
        if named && !super::smart::is_home_or_root(path) && !found.iter().any(|(p, _)| *p == path) {
            found.push((path, repo));
        }
    }
    // Two checkouts named like the app: neither is known to be it.
    let [(path, repo)] = found.as_slice() else {
        return None;
    };
    Some((
        Folder {
            path: path.to_path_buf(),
            root: path.to_path_buf(),
            tree: *repo,
            repo: repo.then(|| path.to_path_buf()),
        },
        false,
    ))
}

/// The names an app's checkout could have: the app's name run together, and its words and bundle
/// ID's parts beyond the generic ones.
fn app_names(app: &str, bundle_id: &str) -> Vec<String> {
    const GENERIC: [&str; 18] = [
        "com", "org", "net", "io", "ai", "dev", "app", "apps", "desktop", "beta", "alpha",
        "nightly", "canary", "preview", "mac", "macos", "helper", "client",
    ];
    let mut names = Vec::new();
    let parts = std::iter::once(normalize_slug(app))
        .chain(app.split_whitespace().map(normalize_slug))
        .chain(bundle_id.split('.').map(normalize_slug));
    for name in parts {
        if name.len() >= 3 && !GENERIC.contains(&name.as_str()) && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// What a capture of OpenCode Desktop is about: OpenCode's own interface (the composer, a
/// picker, the sidebar), the session it shows, or it can't be told.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum About {
    /// What gave it away.
    Interface(Vec<String>),
    Session(Vec<String>),
    Unclear,
}

impl About {
    /// What it's about, without what gave it away (the app's clue says that).
    fn short(&self) -> &'static str {
        match self {
            Self::Interface(_) => "; the capture is about OpenCode's own interface",
            Self::Session(_) => "; the capture is about the session",
            Self::Unclear => {
                "; whether the capture is about the session or OpenCode itself is unclear"
            }
        }
    }

    fn why(&self) -> String {
        let quote = |seen: &[String]| {
            seen.iter()
                .take(4)
                .map(|w| format!("\u{201c}{w}\u{201d}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        match self {
            Self::Interface(seen) => {
                format!(
                    "; the capture is about OpenCode's own interface ({})",
                    quote(seen)
                )
            }
            Self::Session(seen) => format!("; the capture is about the session ({})", quote(seen)),
            Self::Unclear => {
                "; whether the capture is about the session or OpenCode itself is unclear".into()
            }
        }
    }
}

/// Words about an interface rather than about work in a session.
const INTERFACE_WORDS: &str = "button buttons spacing padding margin margins align aligned alignment \
     composer picker pickers dropdown menu menus sidebar toolbar titlebar tab tabs icon icons font \
     fonts layout animation animations animate animates hover hovering scroll scrolling scrollbar \
     search searching typeahead flashing flash flicker flickering spinner spinners loading focus \
     focused shortcut shortcuts keyboard component components pill chip chips modal dialog popover \
     popup tooltip panel header footer style styling glass blur shadow border rounded corner \
     corners rename click clicking drag resize placeholder textbox checkbox toggle badge avatar \
     theme laggy janky jank polish pixel pixels highlight cursor selected selection sidebar \
     window windows";

/// Phrases about what the session's agent did or said.
const SESSION_PHRASES: [&str; 22] = [
    "you said",
    "you wrote",
    "you did",
    "you made",
    "you changed",
    "you added",
    "you broke",
    "you were",
    "youre doing",
    "your plan",
    "your answer",
    "your response",
    "your output",
    "your code",
    "your change",
    "this plan",
    "this diff",
    "the diff",
    "this answer",
    "this response",
    "it said",
    "the agent said",
];

/// Accessibility roles of OpenCode's controls, as clicks record them.
const INTERFACE_ROLES: [&str; 12] = [
    "button",
    "pop up button",
    "menu button",
    "menu item",
    "text field",
    "combo box",
    "search field",
    "toolbar",
    "tab",
    "radio button",
    "checkbox",
    "slider",
];

/// Reads what an OpenCode capture is about from what was said, clicked and marked. Only clear
/// talk about the session's work counts for the session; it's unclear otherwise.
pub fn about(bundle: &ScrawlBundle) -> About {
    let text = bundle
        .transcript
        .text
        .to_lowercase()
        .replace(['\'', '\u{2019}'], "");
    let spoken: Vec<&str> = text
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let mut interface: Vec<String> = Vec::new();
    let mut note = |word: &str| {
        if !interface.iter().any(|w| w == word) {
            interface.push(word.to_owned());
        }
    };
    for word in &spoken {
        if INTERFACE_WORDS.split_whitespace().any(|w| w == *word) {
            note(word);
        }
    }
    for event in &bundle.events {
        if let crate::model::InteractionEvent::Click {
            role: Some(role), ..
        } = event
            && INTERFACE_ROLES.contains(&role.as_str())
        {
            note(role);
        }
    }
    for mark in &bundle.marks {
        if let Some(on) = mark.on.as_deref() {
            let on = on.to_lowercase();
            if let Some(word) = on
                .split(|c: char| !c.is_alphanumeric())
                .find(|w| INTERFACE_WORDS.split_whitespace().any(|i| i == *w))
            {
                note(word);
            }
        }
    }
    let joined = format!(" {} ", spoken.join(" "));
    let session: Vec<String> = SESSION_PHRASES
        .iter()
        .filter(|p| joined.contains(&format!(" {p} ")))
        .map(|p| (*p).to_owned())
        .collect();
    if !session.is_empty() && session.len() >= interface.len() {
        About::Session(session)
    } else if session.is_empty() && interface.len() >= 2 {
        About::Interface(interface)
    } else {
        About::Unclear
    }
}

/// The session viewed most recently before `end`, if within [`VIEWED_WITHIN_MS`].
fn last_viewed(candidates: &[Candidate], end: u64) -> Option<(&Candidate, u64)> {
    candidates
        .iter()
        .filter_map(|c| Some((c, c.viewed_ms?)))
        .filter(|(_, ms)| *ms <= end + 5_000 && end.saturating_sub(*ms) <= VIEWED_WITHIN_MS)
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.id.cmp(&a.0.id)))
}

/// Evidence for an existing bundle: its saved `routing.json` when it has one (the sessions and
/// live clues as they were, the bundle's clues read again), else what can be gathered now, with
/// `url` standing in for the tab.
pub fn evidence_for_bundle(
    dir: &Path,
    bundle: &ScrawlBundle,
    url: Option<&str>,
    senses: &Senses,
) -> (Evidence, Option<String>, bool) {
    let recorded_url = || bundle.destination.as_ref().and_then(|d| d.url.clone());
    if url.is_none()
        && let Some(saved) = super::read_record(dir)
    {
        let mut evidence = replay(saved, bundle, &crate::handoff::routing_apps());
        // Captures saved before pages from file servers were looked at get that look now.
        evidence.url = evidence.url.take().or_else(recorded_url);
        let url = evidence.url.clone();
        // Rules are read as they are today.
        evidence.clues.retain(|c| c.source != Source::Rule);
        if let Some(clue) = url.as_deref().and_then(|url| rule_clue(url, senses)) {
            evidence.clues.insert(0, clue);
        }
        look_at_page(
            url.as_deref(),
            &mut evidence.clues,
            &mut evidence.candidates,
            senses,
        );
        return (evidence, None, true);
    }
    let url = url.map(str::to_owned).or_else(recorded_url);
    let mut clues = url
        .as_deref()
        .map(|url| url_clues(url, senses, true))
        .unwrap_or_default();
    let address = crate::handoff::recorded_destination(dir).map(Address::Recorded);
    let (mut candidates, error) = read_for(&clues, address.as_ref(), senses);
    look_at_page(url.as_deref(), &mut clues, &mut candidates, senses);
    let live = Live {
        url,
        clues,
        candidates,
        error: error.clone(),
    };
    (evidence_for(bundle, dir, live), error, false)
}

// MARK: Folders

/// A folder with its project root and repository.
pub fn folder_of(path: &Path) -> Folder {
    let (root, tree) = project_root(path);
    Folder {
        path: path.to_path_buf(),
        repo: repo_of(&root),
        root,
        tree,
    }
}

/// The main checkout of the repository `dir` is in: the folder holding `.git`, or for a
/// worktree (a `.git` file) the checkout its `commondir` belongs to.
pub fn repo_of(dir: &Path) -> Option<PathBuf> {
    let mut current = Some(dir);
    while let Some(path) = current {
        let git = path.join(".git");
        if git.is_dir() {
            return Some(path.to_path_buf());
        }
        if git.is_file() {
            let text = std::fs::read_to_string(&git).ok()?;
            let gitdir = PathBuf::from(text.trim().strip_prefix("gitdir:")?.trim());
            let gitdir = if gitdir.is_absolute() {
                gitdir
            } else {
                path.join(gitdir)
            };
            let common = std::fs::read_to_string(gitdir.join("commondir"))
                .map(|c| gitdir.join(c.trim()))
                .unwrap_or(gitdir);
            let common = common.canonicalize().unwrap_or(common);
            return common.parent().map(Path::to_path_buf);
        }
        current = path.parent();
    }
    None
}

// MARK: Sessions

fn candidate(session: OpenCodeSession, repos: &mut HashMap<PathBuf, Option<PathBuf>>) -> Candidate {
    let directory = session.directory().map(PathBuf::from);
    let repo = directory
        .as_ref()
        .and_then(|d| repos.entry(d.clone()).or_insert_with(|| repo_of(d)).clone());
    Candidate {
        id: session.id,
        title: session.title,
        directory,
        repo,
        updated_ms: session.time.updated,
        viewed_ms: session.time.viewed,
    }
}

/// Open root sessions: the newest `pages` pages, the newest page in each of `roots`, and each
/// of `ids` (whatever its state, since something named it).
pub fn read_candidates(
    get: &mut ApiGet,
    roots: &[PathBuf],
    ids: &[String],
    pages: usize,
) -> Result<Vec<Candidate>> {
    let mut raw: Vec<OpenCodeSession> = Vec::new();
    let add = |raw: &mut Vec<OpenCodeSession>, session: OpenCodeSession| {
        if !raw.iter().any(|s| s.id == session.id) {
            raw.push(session);
        }
    };
    find_root_session(get, "", pages, |session| {
        add(&mut raw, session.clone());
        Scan::Skip
    })?;
    for root in roots {
        if super::smart::is_home_or_root(root) {
            continue;
        }
        let query = format!("&directory={}", query_escape(&root.to_string_lossy()));
        find_root_session(get, &query, 1, |session| {
            add(&mut raw, session.clone());
            Scan::Skip
        })?;
    }
    for id in ids {
        if !raw.iter().any(|s| &s.id == id)
            && let Ok(session) = get_session(get, id)
        {
            add(&mut raw, session);
        }
    }
    let mut repos = HashMap::new();
    Ok(raw
        .into_iter()
        .map(|s| candidate(s, &mut repos))
        .collect())
}

// MARK: This Mac's own names

/// The port on this Mac serving `url`: its own port on a loopback host, else, when the host is
/// one of this Mac's names, the loopback port `tailscale serve` proxies it to, or the same
/// port for plain `http`.
pub fn own_port(url: &WebUrl) -> Option<u16> {
    if url.is_loopback() {
        return Some(url.port);
    }
    if !own_names().iter().any(|name| *name == url.host) {
        return None;
    }
    tailscale(&["serve", "status", "--json"])
        .and_then(|json| serve_target(&json, &url.host, url.port))
        .or_else(|| (url.scheme == "http").then_some(url.port))
}

/// The loopback port `tailscale serve status --json` proxies `host:port`'s root to.
pub fn serve_target(status: &serde_json::Value, host: &str, port: u16) -> Option<u16> {
    let handlers = status
        .get("Web")?
        .as_object()?
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(&format!("{host}:{port}")))?
        .1
        .get("Handlers")?
        .as_object()?;
    let proxy = handlers
        .get("/")
        .or_else(|| handlers.values().next())?
        .get("Proxy")?
        .as_str()?;
    let target = parse_url(proxy).or_else(|| parse_url(&format!("http://{proxy}")))?;
    target.is_loopback().then_some(target.port)
}

/// This Mac's host names, lowercased: `hostname`, its Bonjour name, and its Tailscale name.
fn own_names() -> &'static [String] {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        let mut names = Vec::new();
        let mut push = |name: &str| {
            let name = name.trim().trim_end_matches('.').to_ascii_lowercase();
            if !name.is_empty() && !names.contains(&name) {
                names.push(name);
            }
        };
        if let Some(out) = run(&mut Command::new("/bin/hostname")) {
            push(&out);
        }
        if let Some(out) = run(Command::new("/usr/sbin/scutil").args(["--get", "LocalHostName"])) {
            push(&format!("{}.local", out.trim()));
        }
        if let Some(status) = tailscale(&["status", "--json"])
            && let Some(dns) = status.pointer("/Self/DNSName").and_then(|v| v.as_str())
        {
            push(dns);
        }
        names
    })
}

fn run(command: &mut Command) -> Option<String> {
    let output = run_with_timeout(command, Duration::from_secs(2))?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).into_owned())
}

fn tailscale(args: &[&str]) -> Option<serde_json::Value> {
    const APP: &str = "/Applications/Tailscale.app/Contents/MacOS/Tailscale";
    let binary = if Path::new(APP).exists() {
        APP
    } else {
        "tailscale"
    };
    serde_json::from_str(&run(Command::new(binary).args(args))?).ok()
}

// MARK: Terminals

/// Ghostty's focused pane: its working directory and title (AppleScript, Ghostty 1.3+). Other
/// terminals give nothing yet.
pub fn terminal_pane(bundle_id: &str) -> Option<(PathBuf, String)> {
    if !bundle_id.eq_ignore_ascii_case(GHOSTTY) {
        return None;
    }
    let script = format!(
        "tell application id \"{GHOSTTY}\"\nif (count of windows) is 0 then return \"\"\nset t to focused terminal of selected tab of front window\nreturn (working directory of t) & linefeed & (name of t)\nend tell"
    );
    let output = run(Command::new("/usr/bin/osascript").args(["-e", &script]))?;
    let (dir, title) = output
        .trim_end()
        .split_once('\n')
        .unwrap_or((output.trim(), ""));
    let dir = PathBuf::from(dir.trim());
    (dir.is_absolute() && dir != Path::new("/")).then(|| (dir, title.trim().to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handoff::fake_api::session;
    use crate::web::Registration;

    #[test]
    fn tailscale_serve_maps_this_macs_name_to_a_loopback_port() {
        let status = serde_json::json!({
            "TCP": {"4948": {"HTTPS": true}},
            "Web": {
                "studio.example.ts.net:4948": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:4948"}}},
                "studio.example.ts.net:8443": {"Handlers": {"/": {"Proxy": "http://127.0.0.1:3000"}}},
                "studio.example.ts.net:9000": {"Handlers": {"/": {"Proxy": "https://example.com"}}}
            }
        });
        assert_eq!(
            serve_target(&status, "studio.example.ts.net", 4948),
            Some(4948)
        );
        assert_eq!(
            serve_target(&status, "studio.example.ts.net", 8443),
            Some(3000)
        );
        assert_eq!(serve_target(&status, "studio.example.ts.net", 9000), None);
        assert_eq!(serve_target(&status, "studio.example.ts.net", 1), None);
        assert_eq!(serve_target(&serde_json::json!({}), "x", 1), None);
    }

    #[test]
    fn worktrees_share_their_repository() {
        let base = std::env::temp_dir().join(format!("scrawl-repo-{}", crate::make_bundle_id()));
        let main = base.join("main");
        std::fs::create_dir_all(main.join(".git/worktrees/wt")).unwrap();
        std::fs::write(main.join(".git/worktrees/wt/commondir"), "../..\n").unwrap();
        let wt = base.join("wt");
        std::fs::create_dir_all(wt.join("site")).unwrap();
        std::fs::write(
            wt.join(".git"),
            format!("gitdir: {}\n", main.join(".git/worktrees/wt").display()),
        )
        .unwrap();
        let main = main.canonicalize().unwrap();
        assert_eq!(repo_of(&wt.join("site")), Some(main.clone()));
        assert_eq!(repo_of(&main), Some(main.clone()));
        let folder = folder_of(&wt.join("site"));
        assert_eq!((folder.root, folder.tree), (wt.clone(), true));
        std::fs::remove_dir_all(base).unwrap();
    }

    fn no_sdk(_: &str) -> Option<Registration> {
        None
    }
    fn listener(port: u16) -> Option<PathBuf> {
        (port == 4948).then(|| PathBuf::from("/p/board"))
    }
    fn tailnet(url: &WebUrl) -> Option<u16> {
        (url.host == "studio.example.ts.net").then_some(url.port)
    }
    fn no_terminal(_: &str) -> Option<(PathBuf, String)> {
        None
    }
    fn api(path: &str, _: Duration) -> Result<Vec<u8>> {
        crate::handoff::fake_api::serve(
            &[
                session("ses_board", "BOARD", "/p/board", 10),
                session("ses_other", "OTHER", "/q", 20),
            ],
            path,
        )
    }

    /// The world one ladder case sees, read by the plain-`fn` hooks below.
    #[derive(Clone, Default)]
    struct World {
        reg: Option<Registration>,
        listeners: Vec<(u16, PathBuf)>,
        sessions: Vec<crate::handoff::OpenCodeSession>,
    }

    thread_local! {
        static WORLD: std::cell::RefCell<World> = std::cell::RefCell::default();
    }

    fn world_sdk(url: &str) -> Option<Registration> {
        WORLD.with(|w| {
            let reg = w.borrow().reg.clone()?;
            (parse_url(url)?.origin() == reg.origin).then_some(reg)
        })
    }
    fn world_listener(port: u16) -> Option<PathBuf> {
        WORLD.with(|w| {
            w.borrow()
                .listeners
                .iter()
                .find(|(p, _)| *p == port)
                .map(|(_, d)| d.clone())
        })
    }
    fn world_api(path: &str, _: Duration) -> Result<Vec<u8>> {
        let sessions = WORLD.with(|w| w.borrow().sessions.clone());
        crate::handoff::fake_api::serve(&sessions, path)
    }
    fn world_own_port(url: &WebUrl) -> Option<u16> {
        Some(url.port)
    }

    /// Flag off is today: `Policy::Ladder` over the clues `url_clues` gathers (without this
    /// Mac's own names) decides exactly what `routing::route` does, case by case.
    #[test]
    fn the_ladder_policy_reproduces_route() {
        use super::super::smart::{Policy, decide, destination};
        let base = std::env::temp_dir().join(format!("scrawl-ladder-{}", crate::make_bundle_id()));
        let repo = base.join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(repo.join("site")).unwrap();
        let repo_s = repo.to_string_lossy().into_owned();
        let site_s = repo.join("site").to_string_lossy().into_owned();
        let home = dirs::home_dir().unwrap().to_string_lossy().into_owned();

        let reg = |origin: &str, session: Option<&str>, directory: Option<&str>| Registration {
            origin: origin.into(),
            url: format!("{origin}/"),
            session: session.map(Into::into),
            directory: directory.map(Into::into),
            ..Registration::default()
        };
        let sessions = vec![
            session("ses_sdk", "SDK", "/elsewhere", 5),
            session("ses_app", "APP", "/p/app", 10),
            session("ses_app_old", "APP OLD", "/p/app", 3),
            session("ses_repo", "REPO", &repo_s, 20),
            session("ses_site", "SITE", &site_s, 30),
            session("ses_home", "HOME", &home, 40),
            session("ses_untitled", "", "/p/untitled", 50),
        ];
        let cases: Vec<(&str, World)> = vec![
            (
                "http://localhost:5173/page",
                World {
                    reg: Some(reg("http://localhost:5173", Some("ses_sdk"), None)),
                    ..World::default()
                },
            ),
            (
                "http://localhost:5173/page",
                World {
                    reg: Some(reg(
                        "http://localhost:5173",
                        Some("ses_sdk"),
                        Some("/p/app"),
                    )),
                    listeners: vec![(5173, "/p/app".into())],
                    ..World::default()
                },
            ),
            (
                "http://localhost:5173/",
                World {
                    reg: Some(reg("http://localhost:5173", Some("ses_missing"), None)),
                    ..World::default()
                },
            ),
            (
                "https://studio.ts.net:5173/",
                World {
                    reg: Some(reg("https://studio.ts.net:5173", None, Some("/p/app"))),
                    ..World::default()
                },
            ),
            (
                "http://localhost:4321/",
                World {
                    reg: Some(reg("http://localhost:4321", None, Some("/p/none"))),
                    listeners: vec![(4321, "/p/app".into())],
                    ..World::default()
                },
            ),
            (
                "http://localhost:4321/",
                World {
                    reg: Some(reg("http://localhost:4321", None, None)),
                    listeners: vec![(4321, "/p/app".into())],
                    ..World::default()
                },
            ),
            (
                "http://127.0.0.1:4321/",
                World {
                    listeners: vec![(4321, "/p/app".into())],
                    ..World::default()
                },
            ),
            (
                "http://[::1]:4000/x",
                World {
                    listeners: vec![(4000, repo.join("site/src"))],
                    ..World::default()
                },
            ),
            (
                "http://localhost:4001/",
                World {
                    listeners: vec![(4001, repo.clone())],
                    ..World::default()
                },
            ),
            (
                "http://localhost:4002/",
                World {
                    listeners: vec![(4002, home.clone().into())],
                    ..World::default()
                },
            ),
            (
                "http://localhost:4003/",
                World {
                    listeners: vec![(4003, "/p/untitled".into())],
                    ..World::default()
                },
            ),
            ("http://localhost:9999/", World::default()),
            (
                "https://studio.ts.net:4321/",
                World {
                    listeners: vec![(4321, "/p/app".into())],
                    ..World::default()
                },
            ),
            ("https://example.com/", World::default()),
            ("chrome://newtab", World::default()),
        ];
        let senses = Senses {
            hooks: Hooks {
                sdk: world_sdk,
                listener_dir: world_listener,
                api: world_api,
            },
            own_port: world_own_port,
            terminal: no_terminal,
            ..Senses::default()
        };
        let mut sent = 0;
        for (url, mut world) in cases {
            world.sessions = sessions.clone();
            WORLD.with(|w| *w.borrow_mut() = world);
            let today = super::super::route(url, &senses.hooks).ok();
            let clues = url_clues(url, &senses, false);
            let (candidates, _) = read_for(&clues, None, &senses);
            let evidence = Evidence {
                address: None,
                clues,
                candidates,
                now_ms: 0,
                url: None,
            };
            let ladder = destination(&decide(&evidence, Policy::Ladder), Some(url));
            assert_eq!(ladder, today, "{url}");
            sent += usize::from(today.is_some());
        }
        assert_eq!(sent, 9);
        std::fs::remove_dir_all(base).unwrap();
    }

    // MARK: Real captures, replayed

    use super::super::smart::{Decision, Judgement, Policy, decide, explain};

    const OPENCODE_INTERFACE: &str = include_str!("fixtures/opencode-interface.json");
    const ARTIFACTS: [&str; 3] = [
        include_str!("fixtures/artifact-hex-shout2.json"),
        include_str!("fixtures/artifact-hex-motion.json"),
        include_str!("fixtures/artifact-lot.json"),
    ];
    const DEV_SITE_KNOBS: &str = include_str!("fixtures/dev-site-knobs.json");
    const DEVICE_HUB: [&str; 2] = [
        include_str!("fixtures/device-hub-ios.json"),
        include_str!("fixtures/device-hub-ios-2.json"),
    ];
    const OPENCODE_IOS: &str = "ses_f501e10c4ffedeySM4jnNIe3AD";
    const DESKTOP: &str = "ses_edb79c155ffeZ8eqFja5zoWOlX";

    /// A capture's bundle and the evidence its `routing.json` saved (sessions trimmed), and the
    /// decision it got then (`"send TITLE"`, `"offer"`, `"keep"`).
    fn fixture(json: &str) -> (ScrawlBundle, Evidence, String) {
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        (
            serde_json::from_value(value["bundle"].clone()).unwrap(),
            serde_json::from_value(value["evidence"].clone()).unwrap(),
            value["savedDecision"].as_str().unwrap().to_owned(),
        )
    }

    fn replayed(bundle: &ScrawlBundle, saved: Evidence) -> (Evidence, Judgement) {
        let evidence = replay(saved, bundle, &[]);
        let judgement = decide(&evidence, Policy::Weigh);
        (evidence, judgement)
    }

    fn ids(choices: &[super::super::smart::Scored]) -> Vec<&str> {
        choices.iter().map(|c| c.candidate.id.as_str()).collect()
    }

    /// Kit recorded OpenCode Beta to complain about its composer, branch picker and type-ahead
    /// search. It was sent to OPENCODE IOS, the session last open in the window, because
    /// "loading" is in that session's worktree name and it had just been active.
    #[test]
    fn a_capture_of_opencodes_own_interface_asks_about_the_opencode_repo() {
        let (bundle, saved, then) = fixture(OPENCODE_INTERFACE);
        assert_eq!(then, "send OPENCODE IOS");
        assert!(matches!(about(&bundle), About::Interface(_)));
        let (evidence, judgement) = replayed(&bundle, saved);
        match &judgement.decision {
            Decision::Offer { choices } => {
                assert_eq!(ids(choices)[0], DESKTOP);
                assert!(ids(choices).contains(&OPENCODE_IOS), "{:?}", ids(choices));
            }
            other => panic!("expected an offer of DESKTOP, got {other:?}"),
        }
        let text = explain(&evidence, &judgement);
        assert!(
            text.contains("is built from /Users/kit/code/open-source/opencode"),
            "{text}"
        );
        assert!(text.contains("OpenCode's own interface"), "{text}");
        assert!(text.contains("Decision  ask: send to DESKTOP?"), "{text}");
        assert!(!text.contains("\"loading\" names"), "{text}");
    }

    #[test]
    fn a_capture_about_the_session_on_screen_suggests_that_session() {
        let (mut bundle, saved, _) = fixture(OPENCODE_INTERFACE);
        bundle.transcript.text = "Your plan here says we keep the old reducer, but you said the \
            tests pass. Why did you change the cache in this diff?"
            .into();
        bundle.events.clear();
        assert!(matches!(about(&bundle), About::Session(_)));
        let (_, judgement) = replayed(&bundle, saved);
        match &judgement.decision {
            Decision::Offer { choices } => {
                assert_eq!(ids(choices)[0], OPENCODE_IOS);
                assert!(ids(choices).contains(&DESKTOP), "{:?}", ids(choices));
            }
            other => panic!("expected an offer of OPENCODE IOS, got {other:?}"),
        }
        // Unclear: the app's repo still leads, and the viewed session is one click away.
        bundle.transcript.text = "Hmm, what about this one?".into();
        assert_eq!(about(&bundle), About::Unclear);
        let (bundle, saved, _) = (bundle, fixture(OPENCODE_INTERFACE).1, ());
        let (_, judgement) = replayed(&bundle, saved);
        match &judgement.decision {
            Decision::Offer { choices } => {
                assert_eq!(ids(choices)[0], DESKTOP);
                assert!(ids(choices).contains(&OPENCODE_IOS));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn dev_sites_and_device_hub_captures_decide_as_before() {
        for json in [DEV_SITE_KNOBS] {
            let (bundle, saved, then) = fixture(json);
            let (_, judgement) = replayed(&bundle, saved);
            match &judgement.decision {
                Decision::Send { to, .. } => {
                    assert_eq!(format!("send {}", to.candidate.label()), then);
                }
                other => panic!("{then}: {other:?}"),
            }
        }
        for json in DEVICE_HUB {
            let (bundle, saved, then) = fixture(json);
            assert_eq!(then, "keep");
            let (_, judgement) = replayed(&bundle, saved);
            assert!(
                matches!(judgement.decision, Decision::Keep { .. }),
                "{:?}",
                judgement.decision
            );
        }
    }

    #[test]
    fn the_captured_apps_checkout_is_found_by_name_or_config() {
        let (bundle, saved, _) = fixture(OPENCODE_INTERFACE);
        let found = |target: &crate::model::TargetInfo, apps: &[(String, PathBuf)]| {
            app_checkout(target, &saved.candidates, apps)
                .map(|(f, configured)| (f.root, configured))
        };
        let opencode = PathBuf::from("/Users/kit/code/open-source/opencode");
        assert_eq!(found(&bundle.target, &[]), Some((opencode, false)));
        let mut target = bundle.target.clone();
        // Kit's own apps: their checkout is named like them.
        target.app = Some("Hex".into());
        target.bundle_id = Some("com.kitlangton.Hex".into());
        assert_eq!(
            found(&target, &[]).map(|f| f.0),
            Some(PathBuf::from("/Users/kit/code/open-source/hex"))
        );
        // A browser's name says nothing.
        target.app = Some("Brave Browser".into());
        target.bundle_id = Some("com.brave.Browser".into());
        assert_eq!(found(&target, &[]), None);
        // Config names a checkout for an app named otherwise, by bundle ID prefix.
        target.app = Some("Problems".into());
        target.bundle_id = Some("com.kitlangton.problems.mac".into());
        let apps = vec![(
            "com.kitlangton.problems".to_owned(),
            PathBuf::from("/p/team-board"),
        )];
        assert_eq!(
            found(&target, &apps),
            Some((PathBuf::from("/p/team-board"), true))
        );
    }

    #[test]
    fn what_an_opencode_capture_is_about() {
        let (mut bundle, _, _) = fixture(OPENCODE_INTERFACE);
        let mut said = |text: &str| {
            bundle.transcript.text = text.into();
            bundle.events.clear();
            about(&bundle)
        };
        assert!(matches!(
            said("The composer padding is off and this button flickers"),
            About::Interface(_)
        ));
        assert!(matches!(
            said("You said the migration was done, why?"),
            About::Session(_)
        ));
        assert_eq!(said("Look at this."), About::Unclear);
        assert_eq!(
            said("The button. You said it would work."),
            About::Session(vec!["you said".into()])
        );
    }

    #[test]
    fn a_down_server_leaves_no_candidates_at_once() {
        fn down(_: &str, _: Duration) -> Result<Vec<u8>> {
            anyhow::bail!("Could not reach server at http://127.0.0.1:4096")
        }
        let senses = Senses {
            hooks: Hooks {
                sdk: no_sdk,
                listener_dir: listener,
                api: down,
            },
            own_port: tailnet,
            terminal: no_terminal,
            ..Senses::default()
        };
        let started = std::time::Instant::now();
        let live = look_live(None, None, None, &senses);
        assert!(live.candidates.is_empty());
        assert!(live.error.unwrap().contains("couldn't list sessions"));
        assert!(started.elapsed() < Duration::from_millis(100));
    }

    #[test]
    fn a_tab_on_this_macs_tailnet_name_is_a_local_dev_site() {
        let senses = Senses {
            hooks: Hooks {
                sdk: no_sdk,
                listener_dir: listener,
                api,
            },
            own_port: tailnet,
            terminal: no_terminal,
            ..Senses::default()
        };
        let url = "https://studio.example.ts.net:4948/board";
        assert!(url_clues(url, &senses, false).is_empty());
        let clues = url_clues(url, &senses, true);
        assert_eq!(clues.len(), 1);
        assert_eq!(clues[0].source, Source::Port);
        let (candidates, error) = read_for(&clues, None, &senses);
        assert_eq!(error, None);
        let evidence = Evidence {
            address: None,
            clues,
            candidates,
            now_ms: 15,
            url: None,
        };
        let j = super::super::smart::decide(&evidence, super::super::smart::Policy::Weigh);
        let dest = super::super::smart::destination(&j, Some(url)).unwrap();
        assert_eq!(
            (dest.session.as_str(), dest.via.as_str()),
            ("ses_board", "localhost")
        );
        assert_eq!(dest.directory.as_deref(), Some("/p/board"));
    }

    // MARK: Pages from a file server

    /// What a fixture's page needs: the folder `routing.servers` maps its URL prefix to, and the
    /// tool calls OpenCode's history holds for it.
    #[derive(Clone, Default)]
    struct PageWorld {
        servers: Vec<(String, PathBuf)>,
        writes: Vec<served::Write>,
    }

    thread_local! {
        static PAGE: std::cell::RefCell<PageWorld> = std::cell::RefCell::default();
    }

    fn page_servers() -> Vec<(String, PathBuf)> {
        PAGE.with(|p| p.borrow().servers.clone())
    }
    fn page_writes(_: &str, _: u64, _: u64) -> Vec<served::Write> {
        PAGE.with(|p| p.borrow().writes.clone())
    }
    fn page_senses() -> Senses {
        fn down(_: &str, _: Duration) -> Result<Vec<u8>> {
            anyhow::bail!("no server in tests")
        }
        Senses {
            hooks: Hooks {
                sdk: no_sdk,
                listener_dir: listener,
                api: down,
            },
            own_port: tailnet,
            terminal: no_terminal,
            servers: page_servers,
            sites: Vec::new,
            apps: Vec::new,
            aliases: Vec::new,
            writes: page_writes,
            executable: |_| None,
        }
    }

    /// Replays an artifact fixture: its file laid out under a temporary served folder with the
    /// modification time it had, and `writes` as OpenCode's history.
    fn replay_page(json: &str, with_history: bool) -> (Evidence, Judgement) {
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        let page = &value["page"];
        let (bundle, saved, _) = fixture(json);
        let root = std::env::temp_dir().join(format!("scrawl-page-{}", crate::make_bundle_id()));
        let file = root.join(page["file"].as_str().unwrap());
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let handle = std::fs::File::create(&file).unwrap();
        let modified =
            std::time::UNIX_EPOCH + Duration::from_millis(page["modifiedMs"].as_u64().unwrap());
        handle.set_modified(modified).unwrap();
        let writes = if with_history {
            serde_json::from_value(page["writes"].clone()).unwrap()
        } else {
            Vec::new()
        };
        PAGE.with(|p| {
            *p.borrow_mut() = PageWorld {
                servers: vec![(page["served"].as_str().unwrap().to_owned(), root.clone())],
                writes,
            }
        });
        let url = page["url"].as_str().unwrap();
        let mut evidence = replay(saved, &bundle, &[]);
        look_at_page(
            Some(url),
            &mut evidence.clues,
            &mut evidence.candidates,
            &page_senses(),
        );
        std::fs::remove_dir_all(root).unwrap();
        let judgement = decide(&evidence, Policy::Weigh);
        (evidence, judgement)
    }

    const CANVAS: &str = "ses_ec318a614ffeQvPht0qntUnIcP";
    const HEX: &str = "ses_ec3d5c5d4ffeYKYSkmxIUdMuzn";
    const GAME_ENGINE: &str = "ses_ee2d10e0bffeibMGwlBmCwZoQI";

    /// Three captures of pages on the shared artifact gallery (`:8444/raw/…`, served by a server
    /// whose working directory is gpu-gallery) were all sent to CANVAS, the session in
    /// gpu-gallery, on the port's 95 points. Their pages were published by HEX, HEX and GAME
    /// ENGINE.
    #[test]
    fn artifact_gallery_pages_go_to_whoever_published_them() {
        for (json, publisher) in ARTIFACTS.iter().zip([HEX, HEX, GAME_ENGINE]) {
            let (_, _, then) = fixture(json);
            assert_eq!(then, "send CANVAS");
            let (evidence, judgement) = replay_page(json, true);
            let text = explain(&evidence, &judgement);
            match &judgement.decision {
                Decision::Send { to, instead } => {
                    assert_eq!(to.candidate.id, publisher, "{text}");
                    // The server's session is one click away during the undo window.
                    assert!(ids(instead).contains(&CANVAS), "{text}");
                }
                other => panic!("expected a send to {publisher}, got {other:?}\n{text}"),
            }
            assert!(text.contains("call was running when"), "{text}");
            assert!(text.contains("from outside it"), "{text}");
        }
    }

    /// Without OpenCode's history the page's words still lead, and nothing goes to CANVAS.
    #[test]
    fn without_history_an_artifact_page_never_goes_to_the_servers_session() {
        for (json, publisher) in ARTIFACTS.iter().zip([Some(HEX), Some(HEX), None]) {
            let (evidence, judgement) = replay_page(json, false);
            let text = explain(&evidence, &judgement);
            match (&judgement.decision, publisher) {
                (Decision::Offer { choices }, Some(publisher)) => {
                    assert_eq!(ids(choices)[0], publisher, "{text}");
                }
                (Decision::Keep { .. }, None) => {}
                (other, _) => panic!("{other:?}\n{text}"),
            }
        }
    }

    /// A page that names its session (a subagent's, here) goes there, whatever the history says.
    #[test]
    fn a_marked_page_names_its_publisher() {
        fn api(path: &str, _: Duration) -> Result<Vec<u8>> {
            let body = match path {
                "/api/session/ses_child" => {
                    r#"{"id":"ses_child","title":"sub","parentID":"ses_ec3d5c5d4ffeYKYSkmxIUdMuzn"}"#
                }
                _ => r#"{"id":"ses_ec3d5c5d4ffeYKYSkmxIUdMuzn","title":"HEX"}"#,
            };
            Ok(body.as_bytes().to_vec())
        }
        let json = ARTIFACTS[2];
        let value: serde_json::Value = serde_json::from_str(json).unwrap();
        let (bundle, saved, _) = fixture(json);
        let root = std::env::temp_dir().join(format!("scrawl-mark-{}", crate::make_bundle_id()));
        let file = root.join(value["page"]["file"].as_str().unwrap());
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "").unwrap();
        std::fs::write(
            file.parent().unwrap().join(".opencode-session"),
            "ses_child\n",
        )
        .unwrap();
        PAGE.with(|p| {
            *p.borrow_mut() = PageWorld {
                servers: vec![(
                    value["page"]["served"].as_str().unwrap().to_owned(),
                    root.clone(),
                )],
                writes: serde_json::from_value(value["page"]["writes"].clone()).unwrap(),
            }
        });
        let senses = Senses {
            hooks: Hooks {
                api,
                ..page_senses().hooks
            },
            ..page_senses()
        };
        let mut evidence = replay(saved, &bundle, &[]);
        let url = value["page"]["url"].as_str().unwrap();
        look_at_page(
            Some(url),
            &mut evidence.clues,
            &mut evidence.candidates,
            &senses,
        );
        std::fs::remove_dir_all(root).unwrap();
        let judgement = decide(&evidence, Policy::Weigh);
        match &judgement.decision {
            Decision::Send { to, .. } => assert_eq!(to.candidate.id, HEX),
            other => panic!("{other:?}"),
        }
    }

    const ONBOARDING: &str = include_str!("fixtures/artifact-server-onboarding.json");
    const HOME: &str = "ses_efcd8972fffeT5zZbj9YtIl3LU";
    const KNOBS: &str = "ses_ec91ede28ffeFpn6L7FZtGoka0";

    /// Windows opens a folder only with `FILE_FLAG_BACKUP_SEMANTICS`, and setting its time needs
    /// write access.
    fn set_modified(path: &std::path::Path, at: std::time::SystemTime) {
        #[cfg(windows)]
        let file = {
            use std::os::windows::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .write(true)
                .custom_flags(0x0200_0000)
                .open(path)
        };
        #[cfg(not(windows))]
        let file = std::fs::File::open(path);
        file.unwrap().set_modified(at).unwrap();
    }

    /// Replays the onboarding-story fixture: its server's folder laid out inside a temporary
    /// repository's `artifacts/`, with the entries' times it had.
    fn replay_server(with_history: bool) -> (Evidence, Judgement) {
        let value: serde_json::Value = serde_json::from_str(ONBOARDING).unwrap();
        let server = &value["server"];
        let (bundle, mut saved, _) = fixture(ONBOARDING);
        let repo = std::env::temp_dir().join(format!("scrawl-story-{}", crate::make_bundle_id()));
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        let dir = repo.join(server["folder"].as_str().unwrap());
        std::fs::create_dir_all(&dir).unwrap();
        for entry in server["entries"].as_array().unwrap() {
            let path = dir.join(entry["name"].as_str().unwrap());
            if entry["kind"] == "dir" {
                std::fs::create_dir_all(&path).unwrap();
            } else {
                std::fs::write(&path, "").unwrap();
            }
        }
        for entry in server["entries"].as_array().unwrap() {
            let at = std::time::UNIX_EPOCH
                + Duration::from_millis(entry["modifiedMs"].as_u64().unwrap());
            set_modified(&dir.join(entry["name"].as_str().unwrap()), at);
        }
        let port = saved
            .clues
            .iter_mut()
            .find(|c| c.source == Source::Port)
            .unwrap();
        port.pointer = Pointer::Folder(folder_of(&dir));
        PAGE.with(|p| {
            *p.borrow_mut() = PageWorld {
                servers: Vec::new(),
                writes: if with_history {
                    serde_json::from_value(server["writes"].clone()).unwrap()
                } else {
                    Vec::new()
                },
            }
        });
        let mut evidence = replay(saved, &bundle, &[]);
        look_at_page(
            Some("https://studio.taila8e7c7.ts.net:4660/"),
            &mut evidence.clues,
            &mut evidence.candidates,
            &page_senses(),
        );
        std::fs::remove_dir_all(repo).unwrap();
        let judgement = decide(&evidence, Policy::Weigh);
        (evidence, judgement)
    }

    /// Nine captures of the onboarding story (`:4660`, a server running in
    /// `~/code/home/artifacts/2026-10-20-onboarding/story`) went to HOME, the newest session in
    /// the repository around it. KNOBS wrote the story; HOME's subagent only added its assets.
    #[test]
    fn a_server_running_inside_artifacts_goes_to_whoever_wrote_it() {
        let (_, _, then) = fixture(ONBOARDING);
        assert_eq!(then, "send HOME");
        let (evidence, judgement) = replay_server(true);
        let text = explain(&evidence, &judgement);
        match &judgement.decision {
            Decision::Send { to, .. } => assert_eq!(to.candidate.id, KNOBS, "{text}"),
            Decision::Offer { choices } => assert_eq!(ids(choices)[0], KNOBS, "{text}"),
            other => panic!("{other:?}\n{text}"),
        }
        assert!(text.contains("where sessions publish"), "{text}");
        let (evidence, judgement) = replay_server(false);
        let text = explain(&evidence, &judgement);
        assert!(
            !matches!(&judgement.decision, Decision::Send { to, .. } if to.candidate.id == HOME),
            "{text}"
        );
    }
}
