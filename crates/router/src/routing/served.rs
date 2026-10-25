//! Pages a server on this Mac serves from outside its own project, such as a shared artifact
//! gallery: which file the tab shows, and which session published it (`docs/routing.md`, "Pages
//! from a file server"). The server's folder says nothing about who owns such a page.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use serde::Deserialize;

use super::smart::{Clue, Pointer, Source, words};

/// Where the page's file is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Served {
    /// A file of the server's own project (or nothing file-like): the port says who it is.
    Project,
    /// A file outside the server's project.
    Elsewhere(Page),
    /// Not found anywhere.
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Page {
    pub file: PathBuf,
    /// The folder the URL's path was found under (`~/code/home/artifacts`).
    pub root: PathBuf,
}

impl Page {
    /// The file's path below `root`, as segments.
    fn below_root(&self) -> Vec<String> {
        self.file
            .strip_prefix(&self.root)
            .map(|rel| {
                rel.components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// What its publisher's tool calls would name: the folder it was published in (the first
    /// below `root`), or the file itself when it sits in `root`. `None` when that's too
    /// ordinary to tell sessions apart ("index.html").
    pub fn needle(&self) -> Option<String> {
        let first = self.below_root().into_iter().next()?;
        (first.chars().count() >= 8
            && !first.starts_with("index.")
            && !first.chars().any(|c| c.is_control() || c == '\''))
        .then_some(first)
    }

    pub fn modified_ms(&self) -> Option<u64> {
        modified_ms(&self.file)
    }
}

/// The decoded segments of a URL's path when it names a file: a last segment with an extension,
/// or a folder (a trailing `/`, read as its `index.html`). `None` for `/` and app routes.
pub fn file_path(url: &str) -> Option<Vec<String>> {
    let rest = url.split_once("://")?.1;
    let path = rest.find('/').map_or("", |i| &rest[i..]);
    let path = path.split(['?', '#']).next().unwrap_or("");
    let mut segments: Vec<String> = path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(crate::util::percent_decode)
        .collect();
    if segments
        .iter()
        .any(|s| s == "." || s == ".." || s.contains('/'))
    {
        return None;
    }
    let last = segments.last()?;
    if path.ends_with('/') {
        segments.push("index.html".into());
    } else if !last.chars().skip(1).any(|c| c == '.') {
        return None;
    }
    Some(segments)
}

/// Where `url`'s file is. `listener` is the folder of the process serving it; `servers` maps URL
/// prefixes to the folders they serve (`routing.servers`); `roots` are the folders open sessions
/// work in, searched with their immediate subfolders.
pub fn locate(
    url: &str,
    listener: &Path,
    servers: &[(String, PathBuf)],
    roots: &[PathBuf],
) -> Served {
    if let Some((prefix, folder)) = servers
        .iter()
        .filter(|(prefix, _)| url.starts_with(prefix.as_str()))
        .max_by_key(|(prefix, _)| prefix.len())
    {
        let rest = &url[prefix.len()..];
        let file = file_path(&format!("http://x/{rest}"))
            .map(|segments| join(folder, &segments))
            .unwrap_or_else(|| folder.clone());
        return Served::Elsewhere(Page {
            file,
            root: folder.clone(),
        });
    }
    let Some(segments) = file_path(url) else {
        return Served::Project;
    };
    let (project, _) = super::project_root(listener);
    if find(&segments, std::slice::from_ref(&project)).is_some() {
        return Served::Project;
    }
    let others: Vec<PathBuf> = roots
        .iter()
        .filter(|r| !r.starts_with(&project) && !super::smart::is_home_or_root(r))
        .cloned()
        .collect();
    match find(&segments, &others) {
        Some(page) => Served::Elsewhere(page),
        None => Served::Unknown,
    }
}

fn join(base: &Path, segments: &[String]) -> PathBuf {
    segments.iter().fold(base.to_path_buf(), |p, s| p.join(s))
}

/// The file at the longest tail of `segments` (at least the file and its folder) under one of
/// `roots` or their immediate subfolders. Two different files found at the same length are no
/// answer.
fn find(segments: &[String], roots: &[PathBuf]) -> Option<Page> {
    let shortest = if segments.len() >= 2 {
        2
    } else if segments.first()?.starts_with("index.") {
        return None;
    } else {
        1
    };
    let mut bases: Vec<PathBuf> = Vec::new();
    let mut add = |base: PathBuf| {
        if !bases.contains(&base) {
            bases.push(base);
        }
    };
    for root in roots {
        add(root.clone());
        let Ok(entries) = std::fs::read_dir(root) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || matches!(name.as_str(), "node_modules" | "target") {
                continue;
            }
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                add(entry.path());
            }
        }
    }
    for len in (shortest..=segments.len()).rev() {
        let tail = &segments[segments.len() - len..];
        let mut found: Vec<Page> = Vec::new();
        for base in &bases {
            let file = join(base, tail);
            if file.is_file() && !found.iter().any(|p| p.file == file) {
                found.push(Page {
                    file,
                    root: base.clone(),
                });
            }
        }
        match found.len() {
            0 => continue,
            1 => return found.pop(),
            _ => return None,
        }
    }
    None
}

/// The folder sessions publish into, when a server runs inside it rather than in a project of
/// its own (`~/code/home/artifacts/2026-10-20-onboarding/story`): a `routing.servers` folder or
/// a folder named `artifacts` above `dir`, else the folder above the nearest one holding an
/// `.opencode-session`, within `dir`'s project.
pub fn content_root(dir: &Path, servers: &[(String, PathBuf)]) -> Option<PathBuf> {
    if let Some(folder) = servers
        .iter()
        .map(|(_, folder)| folder)
        .filter(|folder| dir.starts_with(folder) && dir != folder.as_path())
        .max_by_key(|folder| folder.components().count())
    {
        return Some(folder.clone());
    }
    let (project, _) = super::project_root(dir);
    let mut current = dir.parent();
    while let Some(d) = current {
        if d.file_name().is_some_and(|n| n == "artifacts") {
            return Some(d.to_path_buf());
        }
        if d == project {
            break;
        }
        current = d.parent();
    }
    let mut current = Some(dir);
    while let Some(d) = current {
        if d == project {
            break;
        }
        if d.join(".opencode-session").is_file() {
            return d.parent().map(Path::to_path_buf);
        }
        current = d.parent();
    }
    None
}

/// When the entries directly in `dir` last changed (folders too, `node_modules` and hidden ones
/// aside), newest first: at most [`CHANGES`], within [`CHANGES_SPAN_MS`] of the newest.
pub fn changes(dir: &Path) -> Vec<u64> {
    let mut times: Vec<u64> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            !name.starts_with('.') && name != "node_modules"
        })
        .filter_map(|e| modified_ms(&e.path()))
        .collect();
    times.sort_unstable_by(|a, b| b.cmp(a));
    let newest = times.first().copied().unwrap_or(0);
    times
        .into_iter()
        .filter(|t| newest - t <= CHANGES_SPAN_MS)
        .take(CHANGES)
        .collect()
}

fn modified_ms(path: &Path) -> Option<u64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(
        modified
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_millis() as u64,
    )
}

// MARK: Who published it

/// The session a page names itself: `<meta name="opencode-session" content="ses_…">` in an HTML
/// file, else an `.opencode-session` file in its folder or one above it, below `root`. Returns
/// the ID and where it was found.
pub fn marker(page: &Page) -> Option<(String, String)> {
    let html = page
        .file
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"));
    if html && let Some(id) = read_head(&page.file).as_deref().and_then(meta_session) {
        return Some((
            id,
            format!("{} names it in a meta tag", page.file.display()),
        ));
    }
    let mut dir = page.file.parent();
    while let Some(d) = dir {
        if d == page.root || !d.starts_with(&page.root) {
            break;
        }
        let file = d.join(".opencode-session");
        if let Some(id) = std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| session_id(text.split_whitespace().next()?))
        {
            return Some((id, format!("{} names it", file.display())));
        }
        dir = d.parent();
    }
    None
}

fn read_head(file: &Path) -> Option<String> {
    use std::io::Read as _;
    let mut head = Vec::new();
    std::fs::File::open(file)
        .ok()?
        .take(64 * 1024)
        .read_to_end(&mut head)
        .ok()?;
    Some(String::from_utf8_lossy(&head).into_owned())
}

/// The session in `<meta name="opencode-session" content="…">`, attributes in either order.
pub fn meta_session(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let mut from = 0;
    while let Some(start) = lower[from..].find("<meta").map(|i| from + i) {
        let end = lower[start..].find('>').map_or(lower.len(), |i| start + i);
        let tag = &html[start..end];
        if attribute(tag, "name").is_some_and(|n| n.eq_ignore_ascii_case("opencode-session"))
            && let Some(id) = attribute(tag, "content").and_then(session_id)
        {
            return Some(id);
        }
        from = end;
    }
    None
}

fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find(name).map(|i| from + i) {
        from = i + name.len();
        let before = lower[..i].chars().next_back();
        if !before.is_some_and(char::is_whitespace) {
            continue;
        }
        let rest = tag[from..].trim_start().strip_prefix('=')?.trim_start();
        let quote = rest.chars().next()?;
        return if quote == '"' || quote == '\'' {
            rest[1..].split(quote).next()
        } else {
            rest.split(|c: char| c.is_whitespace() || c == '/').next()
        };
    }
    None
}

fn session_id(text: &str) -> Option<String> {
    let text = text.trim();
    (text.starts_with("ses_")
        && text.len() <= 64
        && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
    .then(|| text.to_owned())
}

/// A tool call whose input named the page's folder, from OpenCode's history.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, serde::Serialize)]
pub struct Write {
    /// The session the call ran in (a subagent's, often).
    pub session: String,
    /// Its root session.
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub tool: String,
    #[serde(default)]
    pub ran: Option<u64>,
    #[serde(default)]
    pub done: Option<u64>,
}

/// History read before the first change looked at, and after the last.
pub const BEFORE_MS: u64 = 30 * 60_000;
pub const AFTER_MS: u64 = 60_000;
/// For a server's own folder: how many of its newest entries are looked at, all changed within
/// this long of the newest.
const CHANGES: usize = 5;
const CHANGES_SPAN_MS: u64 = 2 * 60 * 60_000;

/// The window of history to read for `changed`.
pub fn window(changed: &[u64]) -> Option<(u64, u64)> {
    let first = changed.iter().min()?;
    let last = changed.iter().max()?;
    Some((first.saturating_sub(BEFORE_MS), last + AFTER_MS))
}
/// A call wrote the file if the file changed while it ran, give or take this.
const SLACK_MS: u64 = 2_000;
const READ_ONLY_TOOLS: [&str; 5] = ["read", "grep", "glob", "list", "webfetch"];

/// Points for a session whose tool call wrote the file, and for one that only used its folder.
pub const WROTE: u32 = 95;
pub const USED: u32 = 60;
/// Points for a page that names its session.
pub const MARKED: u32 = 100;
/// The serving process's folder, when the page is a file from elsewhere.
pub const SERVER_ONLY: u32 = 15;
/// The serving process's folder, when it's inside a folder sessions publish into: its project
/// (the enclosing repository) says nothing about who made the page.
pub const SERVER_IN_CONTENT: u32 = 0;

/// One [`Source::Publisher`] clue per root session among `writes`, given when the page's files
/// last `changed` (epoch ms) and what to call them. A change was written by the session whose
/// call (not a read) was running then. The session that wrote the most of them gets [`WROTE`]
/// (each, on a tie); the others that wrote some, or only named the folder, get [`USED`].
pub fn writer_clues(changed: &[u64], what: &str, writes: &[Write]) -> Vec<Clue> {
    let wrote = |write: &Write, at: u64| {
        !READ_ONLY_TOOLS.contains(&write.tool.as_str())
            && matches!((write.ran, write.done), (Some(ran), Some(done))
                if ran.saturating_sub(SLACK_MS) <= at && at <= done + SLACK_MS)
    };
    // Per root: how many changes it wrote, and a call to name (a writing one if any).
    let mut by_root: BTreeMap<String, (usize, &Write)> = BTreeMap::new();
    for write in writes {
        let root = write.root.clone().unwrap_or_else(|| write.session.clone());
        by_root.entry(root).or_insert((0, write));
    }
    for at in changed {
        let mut roots: Vec<String> = writes
            .iter()
            .filter(|w| wrote(w, *at))
            .map(|w| w.root.clone().unwrap_or_else(|| w.session.clone()))
            .collect();
        roots.sort();
        roots.dedup();
        for root in roots {
            if let Some(entry) = by_root.get_mut(&root) {
                entry.0 += 1;
                if let Some(w) = writes
                    .iter()
                    .find(|w| wrote(w, *at) && w.root.as_ref().unwrap_or(&w.session) == &root)
                {
                    entry.1 = w;
                }
            }
        }
    }
    let most = by_root.values().map(|(n, _)| *n).max().unwrap_or(0);
    by_root
        .into_iter()
        .map(|(root, (count, write))| {
            let by = if write.session == root {
                "its"
            } else {
                "its subagent's"
            };
            let (points, note) = match (count, changed.len()) {
                (0, _) => (
                    USED,
                    format!("{by} tool calls named the folder of {what} shortly before it changed"),
                ),
                (n, 1) if n == most => (
                    WROTE,
                    format!(
                        "{by} {} call was running when {what} last changed",
                        write.tool
                    ),
                ),
                (n, total) => (
                    if n == most { WROTE } else { USED },
                    format!("{by} tool calls wrote {n} of the last {total} changes to {what}"),
                ),
            };
            Clue::new(
                Source::Publisher,
                Pointer::Session {
                    id: root,
                    directory: None,
                },
                note,
            )
            .worth(points)
        })
        .collect()
}

/// The words of the page's path below its root (folders and the file's name), as a
/// [`Source::Document`] clue.
pub fn path_clue(page: &Page) -> Option<Clue> {
    let mut segments = page.below_root();
    if let Some(last) = segments.last_mut()
        && let Some((stem, _)) = last.rsplit_once('.')
    {
        *last = stem.to_owned();
    }
    let found = words(&segments.join(" "));
    (!found.is_empty()).then(|| {
        Clue::new(
            Source::Document,
            Pointer::Words { words: found },
            format!("the page is {}", page.file.display()),
        )
    })
}

// MARK: OpenCode's history

/// The tool calls in OpenCode's history whose input names `needle`, in assistant messages started
/// between `from` and `to` (epoch ms), with their root sessions. Reads the local server's
/// database read-only (found through the process `/api/info` names), with a time limit; any
/// failure is no answer.
pub fn opencode_writes(needle: &str, from: u64, to: u64) -> Vec<Write> {
    if !crate::env::get().live || crate::opencode::agent_host().is_some() {
        return Vec::new();
    }
    let Some(db) = opencode_db() else {
        return Vec::new();
    };
    let Some(output) = super::run_with_timeout(
        Command::new("/usr/bin/sqlite3").args([
            "-readonly",
            "-json",
            &db.to_string_lossy(),
            &writes_query(needle, from, to),
        ]),
        Duration::from_secs(3),
    ) else {
        return Vec::new();
    };
    if !output.status.success() || output.stdout.trim_ascii().is_empty() {
        return Vec::new();
    }
    serde_json::from_slice(&output.stdout).unwrap_or_default()
}

/// The database the local OpenCode server has open.
pub(crate) fn opencode_db() -> Option<PathBuf> {
    let info = crate::opencode::api_get_within("/api/info", Duration::from_secs(1)).ok()?;
    let info: serde_json::Value = serde_json::from_slice(&info).ok()?;
    let pid = info.get("pid")?.as_u64().filter(|pid| *pid > 0)?;
    let open = super::lsof(&["-nP", "-a", "-p", &pid.to_string(), "-Fn"])?;
    open.lines()
        .filter_map(|line| line.strip_prefix('n'))
        .find(|path| path.starts_with('/') && path.ends_with(".db"))
        .map(PathBuf::from)
}

/// Tool calls (`session_message.data.content[]` parts of type `tool`) in assistant messages
/// started in the window, whose `state.input` contains `needle`, with each one's root session.
pub fn writes_query(needle: &str, from: u64, to: u64) -> String {
    let needle = needle.replace('\'', "''");
    format!(
        "WITH calls AS MATERIALIZED (
  SELECT m.session_id AS sid, json_extract(c.value, '$.name') AS tool,
    coalesce(json_extract(c.value, '$.time.ran'), json_extract(c.value, '$.time.created')) AS ran,
    json_extract(c.value, '$.time.completed') AS done
  FROM session_message m, json_each(m.data, '$.content') c
  WHERE m.type = 'assistant' AND m.time_created BETWEEN {from} AND {to}
    AND instr(m.data, '{needle}') > 0
    AND json_extract(c.value, '$.type') = 'tool'
    AND instr(json_extract(c.value, '$.state.input'), '{needle}') > 0
), up(sid, id, parent, depth) AS (
  SELECT s.id, s.id, s.parent_id, 0 FROM session_v2 s WHERE s.id IN (SELECT sid FROM calls)
  UNION ALL
  SELECT up.sid, s.id, s.parent_id, up.depth + 1 FROM up JOIN session_v2 s ON s.id = up.parent
  WHERE up.depth < 8
)
SELECT calls.sid AS session,
  (SELECT id FROM up WHERE up.sid = calls.sid AND up.parent IS NULL) AS root,
  tool, ran, done
FROM calls ORDER BY ran;"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "scrawl-served-{name}-{}",
            crate::util::make_bundle_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn touch(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn a_urls_file_path_is_its_decoded_segments() {
        let path = |url: &str| file_path(url);
        assert_eq!(
            path("https://h:8444/raw/2026-10-19-hex-shout2/index.html?x=1#y"),
            Some(vec![
                "raw".into(),
                "2026-10-19-hex-shout2".into(),
                "index.html".into()
            ])
        );
        assert_eq!(
            path("http://h/raw/a%20b/"),
            Some(vec!["raw".into(), "a b".into(), "index.html".into()])
        );
        assert_eq!(path("http://h/board/motion"), None);
        assert_eq!(path("http://h/"), None);
        assert_eq!(path("http://h/.env"), None);
        assert_eq!(path("http://h/raw/../x.html"), None);
    }

    /// A gallery server in its own repository serves `/raw/<path>` from a folder elsewhere that
    /// an open session works in.
    #[test]
    fn a_file_from_outside_the_servers_project_is_found_where_sessions_work() {
        let base = temp("locate");
        let gallery = base.join("gallery");
        std::fs::create_dir_all(gallery.join(".git")).unwrap();
        touch(&gallery.join("public/logo.png"), "");
        touch(&gallery.join("index.html"), "");
        let home = base.join("home");
        let artifacts = home.join("artifacts");
        touch(&artifacts.join("2026-10-19-hex/index.html"), "");
        // A session in the folder above `home` lists it before `home`'s own session does.
        let roots = vec![base.clone(), home.clone(), gallery.clone()];
        let url = "https://studio:8444/raw/2026-10-19-hex/index.html";
        assert_eq!(
            locate(url, &gallery.join("server"), &[], &roots),
            Served::Elsewhere(Page {
                file: artifacts.join("2026-10-19-hex/index.html"),
                root: artifacts.clone(),
            })
        );
        // The trailing slash is its index.html.
        assert!(matches!(
            locate("https://studio:8444/2026-10-19-hex/", &gallery, &[], &roots),
            Served::Elsewhere(_)
        ));
        // The server's own files and app routes are its project's.
        assert_eq!(
            locate("http://localhost:8460/logo.png", &gallery, &[], &roots),
            Served::Project
        );
        assert_eq!(
            locate("http://localhost:8460/board/motion", &gallery, &[], &roots),
            Served::Project
        );
        // An index.html alone names nothing.
        assert_eq!(
            locate(
                "http://localhost:8460/missing/index.html",
                &gallery,
                &[],
                &roots
            ),
            Served::Unknown
        );
        // Config maps a prefix outright.
        let servers = vec![("https://studio:8444/raw/".to_owned(), artifacts.clone())];
        assert_eq!(
            locate(
                "https://studio:8444/raw/lot/iteration-2.jpg",
                &gallery,
                &servers,
                &[]
            ),
            Served::Elsewhere(Page {
                file: artifacts.join("lot/iteration-2.jpg"),
                root: artifacts.clone(),
            })
        );
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn a_page_names_its_session_by_meta_tag_or_folder_file() {
        assert_eq!(
            meta_session(
                r#"<head><meta charset="utf-8"><META content='ses_abc123' name="opencode-session"></head>"#
            ),
            Some("ses_abc123".into())
        );
        assert_eq!(
            meta_session(r#"<meta name="opencode-session" content="nope">"#),
            None
        );
        let base = temp("marker");
        let page = Page {
            file: base.join("hex/stills/a.png"),
            root: base.clone(),
        };
        touch(&page.file, "");
        assert_eq!(marker(&page), None);
        // A marker in the root itself would claim everything published there.
        touch(&base.join(".opencode-session"), "ses_root\n");
        assert_eq!(marker(&page), None);
        touch(&base.join("hex/.opencode-session"), "ses_hex\n");
        assert_eq!(marker(&page).map(|m| m.0), Some("ses_hex".into()));
        let html = Page {
            file: base.join("hex/index.html"),
            root: base.clone(),
        };
        touch(
            &html.file,
            r#"<meta name="opencode-session" content="ses_page">"#,
        );
        assert_eq!(marker(&html).map(|m| m.0), Some("ses_page".into()));
        assert_eq!(html.needle(), None);
        std::fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn the_call_running_when_the_file_changed_wrote_it() {
        let page = Page {
            file: PathBuf::from("/a/lot-2026-10-20/iteration-2.jpg"),
            root: PathBuf::from("/a"),
        };
        assert_eq!(page.needle().as_deref(), Some("lot-2026-10-20"));
        let call = |session: &str, root: &str, tool: &str, ran: u64, done: u64| Write {
            session: session.into(),
            root: Some(root.into()),
            tool: tool.into(),
            ran: Some(ran),
            done: Some(done),
        };
        let writes = vec![
            call("ses_child", "ses_game", "shell", 9_000, 10_500),
            call("ses_child", "ses_game", "read", 1_000, 1_100),
            call("ses_canvas", "ses_canvas", "shell", 2_000, 2_100),
            call("ses_desk", "ses_desk", "read", 9_500, 10_000),
        ];
        let clues = writer_clues(&[10_000], "iteration-2.jpg", &writes);
        let points: Vec<(String, u32)> = clues
            .iter()
            .map(|c| match &c.pointer {
                Pointer::Session { id, .. } => (id.clone(), c.points.unwrap()),
                _ => unreachable!(),
            })
            .collect();
        assert_eq!(
            points,
            vec![
                ("ses_canvas".into(), USED),
                ("ses_desk".into(), USED),
                ("ses_game".into(), WROTE),
            ]
        );
        assert!(clues[2].note.contains("subagent's shell call was running"));
    }

    #[test]
    fn the_history_query_quotes_its_needle() {
        let sql = writes_query("it's", 600_000, 2_460_000);
        assert!(sql.contains("instr(m.data, 'it''s')"));
        assert!(sql.contains("BETWEEN 600000 AND 2460000"));
    }

    /// Several changes: the session that wrote most of them is the publisher, a session that
    /// wrote fewer is a runner-up, and a tie is a tie.
    #[test]
    fn the_session_that_wrote_most_of_a_folder_published_it() {
        let call = |root: &str, tool: &str, ran: u64| Write {
            session: format!("{root}_child"),
            root: Some(root.into()),
            tool: tool.into(),
            ran: Some(ran),
            done: Some(ran + 500),
        };
        let writes = vec![
            call("ses_knobs", "edit", 1_000),
            call("ses_knobs", "shell", 5_000),
            call("ses_home", "shell", 9_000),
            call("ses_home", "read", 13_000),
        ];
        let points = |changed: &[u64]| -> Vec<(String, u32)> {
            writer_clues(changed, "story", &writes)
                .iter()
                .map(|c| match &c.pointer {
                    Pointer::Session { id, .. } => (id.clone(), c.points.unwrap()),
                    _ => unreachable!(),
                })
                .collect()
        };
        assert_eq!(
            points(&[13_000, 9_000, 5_000, 1_000]),
            vec![("ses_home".into(), USED), ("ses_knobs".into(), WROTE)]
        );
        assert_eq!(
            points(&[9_000, 5_000]),
            vec![("ses_home".into(), WROTE), ("ses_knobs".into(), WROTE)]
        );
    }

    #[test]
    fn a_server_inside_a_publishing_folder_is_content() {
        let base = temp("content");
        std::fs::create_dir_all(base.join("home/.git")).unwrap();
        let story = base.join("home/artifacts/2026-10-20-onboarding/story");
        std::fs::create_dir_all(&story).unwrap();
        assert_eq!(content_root(&story, &[]), Some(base.join("home/artifacts")));
        // The publishing folder itself is a gallery, not content.
        assert_eq!(content_root(&base.join("home/artifacts"), &[]), None);
        let app = base.join("home/tools/app");
        std::fs::create_dir_all(&app).unwrap();
        assert_eq!(content_root(&app, &[]), None);
        // A marker makes a folder content.
        touch(&base.join("home/tools/.opencode-session"), "ses_x");
        assert_eq!(content_root(&app, &[]), Some(base.join("home")));
        // Config names a publishing folder.
        let servers = vec![("http://x/".to_owned(), base.join("home/tools"))];
        assert_eq!(content_root(&app, &servers), Some(base.join("home/tools")));
        touch(&story.join("index.html"), "");
        std::fs::create_dir_all(story.join("node_modules")).unwrap();
        assert_eq!(changes(&story).len(), 1);
        std::fs::remove_dir_all(base).unwrap();
    }
}
