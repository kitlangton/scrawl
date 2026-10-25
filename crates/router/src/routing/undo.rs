//! Taking a send back (`scrawl router undo`). Every send to a session (`router deliver`, the
//! app's capture sends and their retries) is remembered in `router/deliveries/<id>.json`; an
//! undo then does the least that removes it:
//!
//! - **withdrawn**: still in the session's inbox, so it's cancelled there.
//! - **reverted**: it started a turn and nothing was said after it, so the turn is interrupted
//!   and a revert to it is staged with its file changes (like the TUI's /undo; not committed).
//! - **disowned**: it joined running work, or more came after it, so a short follow-up tells
//!   the session to ignore it instead of undoing the session's own work.
//! - **gone**: no record, or the session or message no longer exists.
//!
//! The undo is logged as a correction (`event: "undo"`) and takes back any rule-offer pick the
//! send counted. See `docs/routing.md`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{jev, policy, router};
use crate::handoff::{self, Posted};
use crate::log::scrawl_log;

/// How long after a send it can be undone, unless `routing.undoWindowSec` says otherwise.
pub const UNDO_WINDOW_SEC: u64 = 600;
/// The follow-up that disowns a send.
pub const DISOWN: &str = "Ignore my previous message: it was sent to this session by mistake.";

/// Records older than this are removed.
const KEEP_FOR: Duration = Duration::from_secs(7 * 24 * 60 * 60);
/// How long each request of an undo may take.
const REQUEST: Duration = Duration::from_secs(3);
/// Newest messages read per page, and pages read, looking for the sent one.
const PAGE: usize = 100;
const PAGES: usize = 5;
/// Tries at staging the revert while the interrupted turn winds down, and the wait between.
const STAGE_TRIES: usize = 10;
const STAGE_WAIT: Duration = Duration::from_millis(200);

/// Where a send came from, for the correction an undo logs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Origin {
    /// `dictation` or `capture`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    /// The rule-offer pick the send counted, taken back on undo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub learned: Option<policy::Counted>,
}

/// `router/deliveries/<delivery_id>.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Record {
    pub delivery_id: String,
    /// When it was sent, epoch ms.
    pub time: u64,
    pub session: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub message_id: Option<String>,
    pub delivery: String,
    pub was_idle: Option<bool>,
    #[serde(flatten)]
    pub origin: Origin,
    /// How it was undone, once it was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub undone: Option<Undone>,
}

/// What `router deliver` prints about a send.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Sent {
    pub delivery_id: String,
    pub message_id: Option<String>,
    pub delivery: String,
    pub was_idle: Option<bool>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Undone {
    Withdrawn,
    Reverted,
    Disowned,
    Gone,
    /// Past the window; nothing was done.
    TooOld,
}

impl Undone {
    pub fn label(self) -> &'static str {
        match self {
            Self::Withdrawn => "withdrawn",
            Self::Reverted => "reverted",
            Self::Disowned => "disowned",
            Self::Gone => "gone",
            Self::TooOld => "too-old",
        }
    }
}

/// `scrawl router undo`'s answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Report {
    pub undone: Undone,
    pub session: Option<String>,
    pub message_id: Option<String>,
}

impl Report {
    fn of(undone: Undone, record: Option<&Record>) -> Self {
        Self {
            undone,
            session: record.map(|r| r.session.clone()),
            message_id: record.and_then(|r| r.message_id.clone()),
        }
    }

    /// One line for a person.
    pub fn text(&self) -> String {
        let session = self.session.as_deref().unwrap_or("its session");
        match self.undone {
            Undone::Withdrawn => format!("Withdrawn from {session} before it was read"),
            Undone::Reverted => format!("Reverted {session} to before it (redo with /redo)"),
            Undone::Disowned => format!("Told {session} to ignore it"),
            Undone::Gone => "Nothing to undo: the send or its session is gone".to_owned(),
            Undone::TooOld => "Too old to undo (routing.undoWindowSec)".to_owned(),
        }
    }
}

/// `router/deliveries`; never in tests.
pub fn deliveries_dir() -> Option<PathBuf> {
    router::router_dir().map(|d| d.join("deliveries"))
}

/// Remembers a send in [`deliveries_dir`] and says how to undo it. Failures are logged.
pub fn remember(session: &str, title: Option<&str>, posted: &Posted, origin: Origin) -> Sent {
    remember_in(deliveries_dir().as_deref(), session, title, posted, origin)
}

/// Remembers a send in `dir` and says how to undo it. Failures are logged.
pub fn remember_in(
    dir: Option<&Path>,
    session: &str,
    title: Option<&str>,
    posted: &Posted,
    origin: Origin,
) -> Sent {
    let now = jev::now_ms();
    let record = Record {
        delivery_id: router::new_id(now),
        time: now,
        session: session.to_owned(),
        title: title.map(str::to_owned),
        message_id: posted.message_id.clone(),
        delivery: posted.delivery.clone(),
        was_idle: posted.was_idle,
        origin,
        undone: None,
    };
    if let Some(dir) = dir {
        if let Err(err) = write(dir, &record) {
            scrawl_log(format!("undo: couldn't remember a send: {err:#}"));
        }
        router::prune(dir, KEEP_FOR);
    }
    Sent {
        delivery_id: record.delivery_id,
        message_id: record.message_id,
        delivery: record.delivery,
        was_idle: record.was_idle,
    }
}

pub fn write(dir: &Path, record: &Record) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{}.tmp", record.delivery_id));
    write_private(&tmp, &serde_json::to_vec_pretty(record)?)?;
    std::fs::rename(&tmp, dir.join(format!("{}.json", record.delivery_id)))?;
    Ok(())
}

/// Writes `bytes` to `path`, readable by Kit alone (0600) even if it was there before.
pub fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut file = private(
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true),
    )
    .open(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    }
    file.write_all(bytes)
}

/// `options` creating files as 0600.
pub fn private(options: &mut std::fs::OpenOptions) -> &mut std::fs::OpenOptions {
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(options, 0o600);
    options
}

fn read(dir: &Path, id: &str) -> Option<Record> {
    serde_json::from_slice(&std::fs::read(dir.join(format!("{id}.json"))).ok()?).ok()
}

/// The requests an undo makes, swappable in tests. A 4xx or 5xx answer is a
/// [`handoff::HttpStatus`] error; anything else means the server couldn't be reached.
#[derive(Clone, Copy)]
pub struct Api {
    pub get: fn(&str) -> Result<Vec<u8>>,
    pub post: fn(&str, &Value) -> Result<Vec<u8>>,
    pub delete: fn(&str) -> Result<Vec<u8>>,
    pub pause: fn(Duration),
}

struct ApiFns<'a> {
    get: &'a dyn Fn(&str) -> Result<Vec<u8>>,
    post: &'a dyn Fn(&str, &Value) -> Result<Vec<u8>>,
    delete: &'a dyn Fn(&str) -> Result<Vec<u8>>,
    pause: &'a dyn Fn(Duration),
}

impl Default for Api {
    fn default() -> Self {
        Self {
            get: |path| handoff::api_within("GET", path, None, REQUEST),
            post: |path, body| handoff::api_within("POST", path, Some(body), REQUEST),
            delete: |path| handoff::api_within("DELETE", path, None, REQUEST),
            pause: std::thread::sleep,
        }
    }
}

/// The undo window: `routing.undoWindowSec`, else ten minutes.
pub fn window() -> Duration {
    Duration::from_secs(handoff::routing_undo_window_sec().unwrap_or(UNDO_WINDOW_SEC))
}

/// `scrawl router undo <delivery_id>`.
pub fn undo(id: &str) -> Result<Report> {
    let dir = deliveries_dir().context("no deliveries folder")?;
    undo_with(&dir, id, &Api::default(), window(), jev::now_ms())
}

/// Undoes the send `id` remembered in `dir`. Errors only for a malformed ID or a server that
/// can't be reached; an undo already made answers the same again.
pub fn undo_with(dir: &Path, id: &str, api: &Api, window: Duration, now: u64) -> Result<Report> {
    let fns = ApiFns {
        get: &api.get,
        post: &api.post,
        delete: &api.delete,
        pause: &api.pause,
    };
    undo_in(
        dir,
        policy::learned_file().as_deref(),
        jev::log_path().as_deref(),
        id,
        &fns,
        window,
        now,
    )
}

pub fn undo_with_config(
    dirs: &crate::RouterDirs,
    config: &crate::RouterConfig,
    id: &str,
    deadline: Duration,
) -> Result<Report> {
    let started = std::time::Instant::now();
    let req_timeout = || {
        let left = deadline.saturating_sub(started.elapsed());
        REQUEST.min(left).max(Duration::from_millis(100))
    };
    let get = |path: &str| {
        handoff::api_within_with_config("GET", path, None, req_timeout(), Some(config))
    };
    let post = |path: &str, body: &Value| {
        handoff::api_within_with_config("POST", path, Some(body), req_timeout(), Some(config))
    };
    let delete = |path: &str| {
        handoff::api_within_with_config("DELETE", path, None, req_timeout(), Some(config))
    };
    let pause = |dur: Duration| {
        let left = deadline.saturating_sub(started.elapsed());
        if !left.is_zero() {
            std::thread::sleep(dur.min(left));
        }
    };
    let fns = ApiFns {
        get: &get,
        post: &post,
        delete: &delete,
        pause: &pause,
    };
    undo_in(
        &dirs.deliveries_dir(),
        Some(&dirs.learned_file()),
        Some(&dirs.decisions_log_path()),
        id,
        &fns,
        Duration::from_secs(config.undo_window_sec),
        jev::now_ms(),
    )
}

fn undo_in(
    dir: &Path,
    learned_file: Option<&Path>,
    log_file: Option<&Path>,
    id: &str,
    api: &ApiFns<'_>,
    window: Duration,
    now: u64,
) -> Result<Report> {
    let id = id.trim();
    ensure!(router::valid_id(id), "{id:?} isn't a delivery ID");
    let Some(mut record) = read(dir, id) else {
        return Ok(Report::of(Undone::Gone, None));
    };
    if let Some(undone) = record.undone {
        return Ok(Report::of(undone, Some(&record)));
    }
    if now.saturating_sub(record.time) > window.as_millis() as u64 {
        return Ok(Report::of(Undone::TooOld, Some(&record)));
    }
    let undone = take_back(&record, api)?;
    record.undone = Some(undone);
    if let Err(err) = write(dir, &record) {
        scrawl_log(format!("undo: couldn't save the undo: {err:#}"));
    }
    after(&record, undone, now, learned_file, log_file);
    Ok(Report::of(undone, Some(&record)))
}

/// A request's answer: `None` when OpenCode says the session or message doesn't exist (404).
fn found(result: Result<Vec<u8>>) -> Result<Option<Vec<u8>>> {
    match result {
        Ok(body) => Ok(Some(body)),
        Err(err) if handoff::http_status(&err) == Some(404) => Ok(None),
        Err(err) => Err(err),
    }
}

/// Times the inbox and messages are read before giving up on a send that keeps moving.
const ROUNDS: usize = 3;

fn take_back(record: &Record, api: &ApiFns<'_>) -> Result<Undone> {
    let session = handoff::query_escape(&record.session);
    let Some(message) = record.message_id.as_deref() else {
        // Sent, but OpenCode's answer had no ID to find it by.
        return disown(&session, api);
    };
    let mut cancelled = false;
    for round in 0..ROUNDS {
        let Some(now) = standing(&session, message, api)? else {
            return Ok(Undone::Gone);
        };
        if now.waiting {
            if cancelled {
                // Still waiting after a cancel: say so rather than leave it to be read.
                return disown(&session, api);
            }
            let path = format!(
                "/api/session/{session}/inbox/{}",
                handoff::query_escape(message)
            );
            match found((api.delete)(&path)) {
                Ok(Some(_)) => cancelled = true,
                Ok(None) => return Ok(Undone::Gone),
                Err(err) if handoff::http_status(&err).is_some() => {
                    scrawl_log(format!("undo: couldn't withdraw it ({err:#}); disowning"));
                    return disown(&session, api);
                }
                Err(err) => return Err(err),
            }
            // A cancel of an item delivered in the meantime does nothing, so look again.
            continue;
        }
        if !now.timeline.found {
            if cancelled {
                return Ok(Undone::Withdrawn);
            }
            // In neither place: read the inbox once more, in case it was on its way.
            if round == 0 {
                continue;
            }
            return Ok(Undone::Gone);
        }
        if now.may_revert() {
            return revert(&session, message, api);
        }
        return disown(&session, api);
    }
    disown(&session, api)
}

/// The session's waiting inbox items, by ID; `None` when the session is gone.
fn inbox(session: &str, api: &ApiFns<'_>) -> Result<Option<Vec<String>>> {
    let Some(body) = found((api.get)(&format!("/api/session/{session}/inbox")))? else {
        return Ok(None);
    };
    let value: Value = serde_json::from_slice(&body).context("Invalid OpenCode inbox")?;
    let items = value
        .get("data")
        .unwrap_or(&value)
        .as_array()
        .context("Invalid OpenCode inbox")?;
    Ok(Some(
        items
            .iter()
            .map(|item| item["id"].as_str().unwrap_or_default().to_owned())
            .collect(),
    ))
}

/// Where a send stands: waiting, or among the messages, and whether anything else waits.
#[derive(Debug, Default)]
struct Standing {
    waiting: bool,
    /// Other input waits (a follow-up typed during the turn, a compaction): a revert would
    /// leave it to land on the reverted session.
    others_waiting: bool,
    timeline: Timeline,
}

impl Standing {
    /// It started the turn, and nothing was said or is waiting after it.
    fn may_revert(&self) -> bool {
        !self.waiting
            && self.timeline.found
            && self.timeline.started
            && !self.timeline.later
            && !self.others_waiting
    }
}

/// Reads the inbox, then (unless the send still waits) the messages. `None` when the session
/// is gone.
fn standing(session: &str, message: &str, api: &ApiFns<'_>) -> Result<Option<Standing>> {
    let Some(waiting) = inbox(session, api)? else {
        return Ok(None);
    };
    let mut now = Standing {
        waiting: waiting.iter().any(|id| id == message),
        others_waiting: waiting.iter().any(|id| id != message),
        timeline: Timeline::default(),
    };
    if !now.waiting {
        let Some(timeline) = timeline(session, message, api)? else {
            return Ok(None);
        };
        now.timeline = timeline;
    }
    Ok(Some(now))
}

/// Where the sent message sits among the session's messages.
#[derive(Debug, Default, PartialEq, Eq)]
struct Timeline {
    found: bool,
    /// It began a turn: nothing but an idle marker (or the start) before it.
    started: bool,
    /// Something was said after it (a prompt, a synthetic message, a shell command).
    later: bool,
}

/// Messages that don't end or begin a turn.
const QUIET: &[&str] = &[
    "agent-switched",
    "model-switched",
    "location-switched",
    "system",
];
/// Messages someone said.
const SAID: &[&str] = &["user", "synthetic", "shell", "skill"];

/// Reads the session's messages newest first until the sent one and the one before it.
fn timeline(session: &str, message: &str, api: &ApiFns<'_>) -> Result<Option<Timeline>> {
    let mut path = format!("/api/session/{session}/message?order=desc&limit={PAGE}");
    let mut timeline = Timeline::default();
    for _ in 0..PAGES {
        let Some(body) = found((api.get)(&path))? else {
            return Ok(None);
        };
        let value: Value = serde_json::from_slice(&body).context("Invalid OpenCode messages")?;
        let messages = value
            .get("data")
            .unwrap_or(&value)
            .as_array()
            .context("Invalid OpenCode messages")?;
        for m in messages {
            let kind = m["type"].as_str().unwrap_or_default();
            if !timeline.found {
                if m["id"] == message {
                    timeline.found = true;
                } else if SAID.contains(&kind) {
                    timeline.later = true;
                }
                continue;
            }
            if QUIET.contains(&kind) {
                continue;
            }
            timeline.started = kind == "idle";
            return Ok(Some(timeline));
        }
        let Some(next) = value.pointer("/cursor/next").and_then(Value::as_str) else {
            // The start of the session.
            timeline.started = timeline.found;
            return Ok(Some(timeline));
        };
        if messages.is_empty() {
            break;
        }
        path = format!(
            "/api/session/{session}/message?cursor={}&limit={PAGE}",
            handoff::query_escape(next)
        );
    }
    // Ran out of pages: it's long past, or not here.
    Ok(Some(Timeline {
        started: false,
        ..timeline
    }))
}

/// Stops the turn it started and stages a revert to before it, files too. Once the turn is
/// stopped, anything but a staged revert disowns it, which also wakes the session.
fn revert(session: &str, message: &str, api: &ApiFns<'_>) -> Result<Undone> {
    let interrupt = format!("/api/session/{session}/interrupt?resume=false");
    if found((api.post)(&interrupt, &json!({})))?.is_none() {
        return Ok(Undone::Gone);
    }
    let stage = format!("/api/session/{session}/revert/stage");
    let body = json!({"messageID": message, "files": true});
    for _ in 0..STAGE_TRIES {
        // Something may have come in before the interrupt landed.
        match standing(session, message, api) {
            Ok(None) => return Ok(Undone::Gone),
            Ok(Some(now)) if !now.waiting && !now.timeline.found => return Ok(Undone::Gone),
            Ok(Some(now)) if !now.may_revert() => {
                scrawl_log("undo: more came in after it; disowning".to_owned());
                return disown(session, api);
            }
            Ok(Some(_)) => {}
            Err(err) => return disown_or(session, api, err),
        }
        match (api.post)(&stage, &body) {
            Ok(_) => return Ok(Undone::Reverted),
            // Busy while the interrupted turn winds down.
            Err(err) if handoff::http_status(&err) == Some(409) => (api.pause)(STAGE_WAIT),
            Err(err) if handoff::http_status(&err) == Some(404) => return Ok(Undone::Gone),
            Err(err) if handoff::http_status(&err).is_some() => {
                scrawl_log(format!(
                    "undo: couldn't stage the revert ({err:#}); disowning"
                ));
                return disown(session, api);
            }
            Err(err) => return disown_or(session, api, err),
        }
    }
    scrawl_log("undo: the session stayed busy; disowning".to_owned());
    disown(session, api)
}

/// After an interrupt, a failed request still owes the session a message: disown, or give
/// back `err` when even that fails.
fn disown_or(session: &str, api: &ApiFns<'_>, err: anyhow::Error) -> Result<Undone> {
    scrawl_log(format!("undo: {err:#} after the interrupt; disowning"));
    disown(session, api).map_err(|_| err)
}

fn disown(session: &str, api: &ApiFns<'_>) -> Result<Undone> {
    let path = format!("/api/session/{session}/prompt");
    let body = json!({"text": DISOWN, "delivery": "steer"});
    Ok(match found((api.post)(&path, &body))? {
        Some(_) => Undone::Disowned,
        None => Undone::Gone,
    })
}

/// Logs the undo as a correction, takes back the pick the send counted, and marks a capture.
fn after(
    record: &Record,
    undone: Undone,
    now: u64,
    learned_file: Option<&Path>,
    log_file: Option<&Path>,
) {
    if let Some(line) = correction(record, undone, now)
        && let Err(err) = jev::write_correction_to(log_file, &line)
    {
        scrawl_log(format!("undo: correction not logged: {err:#}"));
    }
    if let Some(counted) = &record.origin.learned {
        policy::unlearn_pick_in(learned_file, counted);
    }
    if let Some(bundle) = &record.origin.bundle {
        mark_capture(bundle, record, undone, now, learned_file);
    }
}

/// The correction line for an undo, joined to the decision or the bundle.
pub fn correction(record: &Record, undone: Undone, now: u64) -> Option<Value> {
    let origin = &record.origin;
    let bundle = origin
        .bundle
        .as_ref()
        .and_then(|b| b.file_name())
        .map(|n| n.to_string_lossy().into_owned());
    if origin.decision_id.is_none() && origin.item_id.is_none() && bundle.is_none() {
        return None;
    }
    let mut line = json!({
        "time": now,
        "kind": if origin.kind.is_empty() { "dictation" } else { origin.kind.as_str() },
        "event": "undo",
        "action": "cancel",
        "undone": undone.label(),
        "delivery_id": record.delivery_id,
        "from": {"id": record.session, "title": record.title.clone().unwrap_or_default()},
        "to": null,
        "app": origin.app,
    });
    for (key, value) in [
        ("decision_id", origin.decision_id.clone()),
        ("item_id", origin.item_id.clone()),
        ("bundle", bundle),
    ] {
        if let Some(value) = value {
            line[key] = json!(value);
        }
    }
    Some(line)
}

/// A capture's `routing.json` gets `undone`, so evals don't take the session it went to as
/// right, and loses the pick `learn_capture` counted.
fn mark_capture(
    bundle: &Path,
    record: &Record,
    undone: Undone,
    now: u64,
    learned_file: Option<&Path>,
) {
    let path = bundle.join(super::RECORD);
    let Some(mut body) = std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
    else {
        return;
    };
    if let Some(counted) = body
        .get("learned")
        .and_then(|c| serde_json::from_value::<policy::Counted>(c.clone()).ok())
        && record.origin.learned.is_none()
    {
        policy::unlearn_pick_in(learned_file, &counted);
    }
    if let Some(map) = body.as_object_mut() {
        map.remove("learned");
        map.remove("offerRule");
    }
    body["undone"] = json!({"session": record.session, "how": undone.label(), "time": now});
    if let Ok(bytes) = serde_json::to_vec_pretty(&body) {
        let _ = std::fs::write(&path, bytes);
    }
}

/// Whether a capture's send was undone (its `routing.json`).
pub fn capture_undone(bundle: &Path) -> bool {
    std::fs::read(bundle.join(super::RECORD))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .is_some_and(|body| body.get("undone").is_some_and(|u| !u.is_null()))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use anyhow::anyhow;

    use super::*;

    /// A pretend OpenCode session: its inbox and messages (oldest first).
    #[derive(Default)]
    struct Fake {
        inbox: Vec<String>,
        messages: Vec<Value>,
        gone: bool,
        down: bool,
        /// The item is delivered between the inbox read and the cancel, which does nothing.
        delivered_on_cancel: bool,
        /// Stage answers 409 this many times first.
        busy: usize,
        /// Turns up in the inbox on the second read.
        late: Option<String>,
        inbox_reads: usize,
        /// The cancel answers 500.
        refuses_cancel: bool,
        /// Arrives as the interrupt lands: a waiting item, or a message.
        on_interrupt: Option<Arrival>,
        /// The server stops answering stage requests.
        stage_unreachable: bool,
        calls: Vec<String>,
    }

    enum Arrival {
        Waiting(&'static str),
        Said(&'static str),
    }

    thread_local! {
        static FAKE: RefCell<Fake> = RefCell::new(Fake::default());
    }

    fn status(code: u16) -> anyhow::Error {
        handoff::HttpStatus(code).into()
    }

    fn get(path: &str) -> Result<Vec<u8>> {
        FAKE.with(|f| {
            let mut f = f.borrow_mut();
            f.calls.push(format!("GET {path}"));
            if f.down {
                return Err(anyhow!("Could not connect"));
            }
            if f.gone {
                return Err(status(404));
            }
            let body = if path.ends_with("/inbox") {
                f.inbox_reads += 1;
                if f.inbox_reads == 2
                    && let Some(late) = f.late.take()
                {
                    f.inbox.push(late);
                }
                let items: Vec<Value> = f.inbox.iter().map(|id| json!({"id": id})).collect();
                json!({ "data": items })
            } else {
                assert!(path.contains("/message?order=desc"), "{path}");
                let newest_first: Vec<Value> = f.messages.iter().rev().cloned().collect();
                json!({"data": newest_first, "cursor": {}})
            };
            Ok(serde_json::to_vec(&body).unwrap())
        })
    }

    fn post(path: &str, body: &Value) -> Result<Vec<u8>> {
        FAKE.with(|f| {
            let mut f = f.borrow_mut();
            f.calls.push(format!("POST {path} {body}"));
            if f.down {
                return Err(anyhow!("Could not connect"));
            }
            if f.gone {
                return Err(status(404));
            }
            if path.ends_with("/revert/stage") && f.stage_unreachable {
                return Err(anyhow!("Could not connect"));
            }
            if path.ends_with("/revert/stage") && f.busy > 0 {
                f.busy -= 1;
                return Err(status(409));
            }
            if path.contains("/interrupt") {
                match f.on_interrupt.take() {
                    Some(Arrival::Waiting(id)) => f.inbox.push(id.into()),
                    Some(Arrival::Said(id)) => f.messages.push(json!({"id": id, "type": "user"})),
                    None => {}
                }
            }
            if path.ends_with("/prompt") {
                let id = format!("msg_{}", f.messages.len());
                f.messages.push(json!({"id": id, "type": "user"}));
            }
            Ok(b"{}".to_vec())
        })
    }

    fn delete(path: &str) -> Result<Vec<u8>> {
        FAKE.with(|f| {
            let mut f = f.borrow_mut();
            f.calls.push(format!("DELETE {path}"));
            if f.gone {
                return Err(status(404));
            }
            if f.refuses_cancel {
                return Err(status(500));
            }
            let id = path.rsplit('/').next().unwrap().to_owned();
            if f.delivered_on_cancel {
                f.inbox.retain(|i| *i != id);
                f.messages.push(json!({"id": id, "type": "user"}));
            } else {
                f.inbox.retain(|i| *i != id);
            }
            Ok(b"{}".to_vec())
        })
    }

    const API: Api = Api {
        get,
        post,
        delete,
        pause: |_| {},
    };

    fn set(fake: Fake) {
        FAKE.with(|f| *f.borrow_mut() = fake);
    }

    fn calls() -> Vec<String> {
        FAKE.with(|f| f.borrow().calls.clone())
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("scrawl-undo-{name}-{}", crate::make_bundle_id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const SENT_AT: u64 = 1_000_000;

    /// A remembered send of `msg_mine` to `ses_hex`, from decision `d1`.
    fn sent(dir: &Path, was_idle: bool) -> String {
        let record = Record {
            delivery_id: "abc1".into(),
            time: SENT_AT,
            session: "ses_hex".into(),
            title: Some("HEX".into()),
            message_id: Some("msg_mine".into()),
            delivery: "steer".into(),
            was_idle: Some(was_idle),
            origin: Origin {
                kind: "dictation".into(),
                decision_id: Some("d1".into()),
                item_id: Some("A1B2".into()),
                ..Default::default()
            },
            undone: None,
        };
        write(dir, &record).unwrap();
        record.delivery_id
    }

    fn undo_now(dir: &Path, id: &str) -> Result<Report> {
        undo_with(dir, id, &API, Duration::from_secs(600), SENT_AT + 5_000)
    }

    fn idle() -> Value {
        json!({"id": "msg_idle", "type": "idle", "outcome": "succeeded"})
    }
    fn mine() -> Value {
        json!({"id": "msg_mine", "type": "user"})
    }
    fn reply() -> Value {
        json!({"id": "msg_reply", "type": "assistant"})
    }

    fn posted(path_end: &str) -> Vec<String> {
        calls()
            .into_iter()
            .filter(|c| c.starts_with("POST") && c.split(' ').nth(1).unwrap().ends_with(path_end))
            .collect()
    }

    #[test]
    fn a_waiting_send_is_withdrawn_from_the_inbox() {
        let dir = temp_dir("withdraw");
        let id = sent(&dir, false);
        set(Fake {
            inbox: vec!["msg_other".into(), "msg_mine".into()],
            messages: vec![idle()],
            ..Fake::default()
        });
        let report = undo_now(&dir, &id).unwrap();
        assert_eq!(
            report,
            Report {
                undone: Undone::Withdrawn,
                session: Some("ses_hex".into()),
                message_id: Some("msg_mine".into()),
            }
        );
        assert!(calls().contains(&"DELETE /api/session/ses_hex/inbox/msg_mine".to_owned()));
        assert!(
            calls().iter().all(|c| !c.starts_with("POST")),
            "{:?}",
            calls()
        );
        FAKE.with(|f| assert_eq!(f.borrow().inbox, vec!["msg_other".to_owned()]));
        // Asking again answers the same, without touching the session.
        set(Fake::default());
        assert_eq!(undo_now(&dir, &id).unwrap().undone, Undone::Withdrawn);
        assert!(calls().is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_send_that_started_a_turn_is_interrupted_and_reverted() {
        let dir = temp_dir("revert");
        let id = sent(&dir, true);
        set(Fake {
            messages: vec![reply(), idle(), mine(), reply()],
            busy: 2,
            ..Fake::default()
        });
        assert_eq!(undo_now(&dir, &id).unwrap().undone, Undone::Reverted);
        let interrupts = posted("/interrupt?resume=false");
        assert_eq!(interrupts.len(), 1);
        let stages = posted("/revert/stage");
        assert_eq!(stages.len(), 3, "retried while busy");
        assert!(
            stages[2].contains(r#""messageID":"msg_mine""#)
                && stages[2].contains(r#""files":true"#),
            "{stages:?}"
        );
        assert!(
            posted("/revert/commit").is_empty(),
            "staged only, so it can be redone"
        );
        assert!(posted("/prompt").is_empty());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_first_message_of_a_session_counts_as_starting_a_turn() {
        let dir = temp_dir("first");
        let id = sent(&dir, true);
        set(Fake {
            messages: vec![
                json!({"id": "msg_a", "type": "agent-switched"}),
                mine(),
                reply(),
            ],
            ..Fake::default()
        });
        assert_eq!(undo_now(&dir, &id).unwrap().undone, Undone::Reverted);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_send_into_running_work_or_with_later_messages_is_disowned() {
        for (name, messages) in [
            (
                "joined",
                vec![
                    idle(),
                    json!({"id": "msg_k", "type": "user"}),
                    reply(),
                    mine(),
                    reply(),
                ],
            ),
            (
                "later",
                vec![
                    idle(),
                    mine(),
                    reply(),
                    json!({"id": "msg_k", "type": "user"}),
                ],
            ),
        ] {
            let dir = temp_dir(name);
            let id = sent(&dir, true);
            set(Fake {
                messages,
                ..Fake::default()
            });
            assert_eq!(
                undo_now(&dir, &id).unwrap().undone,
                Undone::Disowned,
                "{name}"
            );
            assert!(
                posted("/interrupt?resume=false").is_empty(),
                "{name}: no abort"
            );
            assert!(posted("/revert/stage").is_empty(), "{name}: no revert");
            let prompts = posted("/api/session/ses_hex/prompt");
            assert_eq!(prompts.len(), 1, "{name}");
            assert!(prompts[0].contains(DISOWN) && prompts[0].contains(r#""delivery":"steer""#));
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn a_session_that_stays_busy_is_disowned_rather_than_left() {
        let dir = temp_dir("busy");
        let id = sent(&dir, true);
        set(Fake {
            messages: vec![idle(), mine(), reply()],
            busy: STAGE_TRIES + 1,
            ..Fake::default()
        });
        assert_eq!(undo_now(&dir, &id).unwrap().undone, Undone::Disowned);
        assert_eq!(posted("/revert/stage").len(), STAGE_TRIES);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn delivered_between_the_inbox_read_and_the_cancel_goes_on_to_revert_or_disown() {
        // Delivered into an idle session: it started the turn.
        let dir = temp_dir("race-revert");
        let id = sent(&dir, true);
        set(Fake {
            inbox: vec!["msg_mine".into()],
            messages: vec![reply(), idle()],
            delivered_on_cancel: true,
            ..Fake::default()
        });
        assert_eq!(undo_now(&dir, &id).unwrap().undone, Undone::Reverted);
        assert!(calls().iter().any(|c| c.starts_with("DELETE")));
        let _ = std::fs::remove_dir_all(dir);

        // Delivered into running work: disowned.
        let dir = temp_dir("race-disown");
        let id = sent(&dir, false);
        set(Fake {
            inbox: vec!["msg_mine".into()],
            messages: vec![idle(), json!({"id": "msg_k", "type": "user"}), reply()],
            delivered_on_cancel: true,
            ..Fake::default()
        });
        assert_eq!(undo_now(&dir, &id).unwrap().undone, Undone::Disowned);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn other_waiting_input_is_never_reverted_over() {
        // Kit typed a follow-up while the sent turn ran: reverting would leave it to land on
        // the reverted session.
        let dir = temp_dir("others");
        let id = sent(&dir, true);
        set(Fake {
            inbox: vec!["msg_followup".into()],
            messages: vec![idle(), mine(), reply()],
            ..Fake::default()
        });
        assert_eq!(undo_now(&dir, &id).unwrap().undone, Undone::Disowned);
        assert!(posted("/interrupt?resume=false").is_empty());
        assert!(posted("/revert/stage").is_empty());
        FAKE.with(|f| assert_eq!(f.borrow().inbox, vec!["msg_followup".to_owned()]));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn input_arriving_with_the_interrupt_turns_the_revert_into_a_disown() {
        for (name, arrival) in [
            ("waiting", Arrival::Waiting("msg_k")),
            ("said", Arrival::Said("msg_k")),
        ] {
            let dir = temp_dir(name);
            let id = sent(&dir, true);
            set(Fake {
                messages: vec![idle(), mine(), reply()],
                on_interrupt: Some(arrival),
                ..Fake::default()
            });
            assert_eq!(
                undo_now(&dir, &id).unwrap().undone,
                Undone::Disowned,
                "{name}"
            );
            assert_eq!(posted("/interrupt?resume=false").len(), 1, "{name}");
            assert!(posted("/revert/stage").is_empty(), "{name}: no revert");
            assert_eq!(
                posted("/prompt").len(),
                1,
                "{name}: the interrupted session gets a message"
            );
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn a_send_seen_waiting_on_a_second_look_is_still_withdrawn() {
        let dir = temp_dir("late");
        let id = sent(&dir, false);
        set(Fake {
            late: Some("msg_mine".into()),
            ..Fake::default()
        });
        assert_eq!(undo_now(&dir, &id).unwrap().undone, Undone::Withdrawn);
        assert!(calls().contains(&"DELETE /api/session/ses_hex/inbox/msg_mine".to_owned()));
        FAKE.with(|f| assert!(f.borrow().inbox.is_empty()));
        let _ = std::fs::remove_dir_all(dir);

        // A cancel the server refuses: disowned rather than left to be read.
        let dir = temp_dir("late-refused");
        let id = sent(&dir, false);
        set(Fake {
            late: Some("msg_mine".into()),
            refuses_cancel: true,
            ..Fake::default()
        });
        assert_eq!(undo_now(&dir, &id).unwrap().undone, Undone::Disowned);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_failure_after_the_interrupt_still_leaves_a_message() {
        let dir = temp_dir("after-interrupt");
        let id = sent(&dir, true);
        set(Fake {
            messages: vec![idle(), mine(), reply()],
            stage_unreachable: true,
            ..Fake::default()
        });
        assert_eq!(undo_now(&dir, &id).unwrap().undone, Undone::Disowned);
        assert_eq!(posted("/interrupt?resume=false").len(), 1);
        assert_eq!(posted("/prompt").len(), 1);
        let _ = std::fs::remove_dir_all(dir);

        // `opencode api`'s status (agentHost) reads like a direct request's: 409 retries.
        let dir = temp_dir("cli-status");
        let id = sent(&dir, true);
        set(Fake {
            messages: vec![idle(), mine(), reply()],
            ..Fake::default()
        });
        fn busy_once_via_cli(path: &str, body: &Value) -> Result<Vec<u8>> {
            let first = FAKE.with(|f| {
                f.borrow()
                    .calls
                    .iter()
                    .all(|c| !c.contains("/revert/stage"))
            });
            if path.ends_with("/revert/stage") && first {
                FAKE.with(|f| f.borrow_mut().calls.push(format!("POST {path} {body}")));
                return Err(anyhow::Error::new(handoff::HttpStatus(409))
                    .context("OpenCode API failed: HTTP 409 Conflict"));
            }
            post(path, body)
        }
        let api = Api {
            post: busy_once_via_cli,
            ..API
        };
        let report = undo_with(&dir, &id, &api, Duration::from_secs(600), SENT_AT + 5_000);
        assert_eq!(report.unwrap().undone, Undone::Reverted);
        assert_eq!(posted("/revert/stage").len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_sessions_records_and_messages_are_gone() {
        let dir = temp_dir("gone");
        set(Fake::default());
        assert_eq!(
            undo_now(&dir, "nope1").unwrap(),
            Report {
                undone: Undone::Gone,
                session: None,
                message_id: None,
            }
        );
        let id = sent(&dir, true);
        set(Fake {
            gone: true,
            ..Fake::default()
        });
        assert_eq!(undo_now(&dir, &id).unwrap().undone, Undone::Gone);

        let id = sent(&dir, true);
        set(Fake {
            messages: vec![idle()],
            ..Fake::default()
        });
        assert_eq!(
            undo_now(&dir, &id).unwrap().undone,
            Undone::Gone,
            "reverted away already"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bad_ids_and_unreachable_servers_are_errors_and_old_sends_too_old() {
        let dir = temp_dir("errors");
        assert!(undo_now(&dir, "../etc").is_err());
        assert!(undo_now(&dir, "").is_err());
        let id = sent(&dir, true);
        set(Fake {
            down: true,
            ..Fake::default()
        });
        assert!(undo_now(&dir, &id).is_err());
        let record = read(&dir, &id).unwrap();
        assert_eq!(record.undone, None, "a failed undo can be tried again");

        set(Fake::default());
        let late = undo_with(&dir, &id, &API, Duration::from_secs(600), SENT_AT + 601_000).unwrap();
        assert_eq!(late.undone, Undone::TooOld);
        assert!(calls().is_empty());
        let wider = undo_with(&dir, &id, &API, Duration::from_secs(900), SENT_AT + 601_000);
        assert_ne!(
            wider.unwrap().undone,
            Undone::TooOld,
            "the window is configurable"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn a_send_without_a_message_id_is_disowned() {
        let dir = temp_dir("no-id");
        let mut record = read(&dir, &sent(&dir, true)).unwrap();
        record.message_id = None;
        write(&dir, &record).unwrap();
        set(Fake::default());
        assert_eq!(
            undo_now(&dir, &record.delivery_id).unwrap().undone,
            Undone::Disowned
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn records_are_private_and_print_the_documented_shapes() {
        let dir = temp_dir("shape");
        let id = sent(&dir, true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(dir.join(format!("{id}.json")))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        let report = Report::of(Undone::Withdrawn, read(&dir, &id).as_ref());
        assert_eq!(
            serde_json::to_value(&report).unwrap(),
            json!({"undone": "withdrawn", "session": "ses_hex", "message_id": "msg_mine"})
        );
        assert_eq!(serde_json::to_value(Undone::TooOld).unwrap(), "too-old");
        let sent = remember(
            "ses_hex",
            None,
            &Posted {
                message_id: Some("msg_x".into()),
                delivery: "queue".into(),
                was_idle: Some(false),
            },
            Origin::default(),
        );
        let printed = serde_json::to_value(&sent).unwrap();
        assert_eq!(printed["message_id"], "msg_x");
        assert_eq!(printed["delivery"], "queue");
        assert_eq!(printed["was_idle"], false);
        assert!(router::valid_id(printed["delivery_id"].as_str().unwrap()));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn an_undo_is_logged_as_a_correction_joined_to_its_decision() {
        let dir = temp_dir("log");
        let record = read(&dir, &sent(&dir, true)).unwrap();
        let line = correction(&record, Undone::Reverted, 9).unwrap();
        jev::check(&line).unwrap();
        assert_eq!(line["event"], "undo");
        assert_eq!(line["decision_id"], "d1");
        assert_eq!(line["item_id"], "A1B2");
        assert_eq!(line["undone"], "reverted");
        assert_eq!(line["from"], json!({"id": "ses_hex", "title": "HEX"}));
        assert_eq!(line["to"], Value::Null);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn undoing_a_capture_marks_it_and_takes_back_its_pick() {
        let dir = temp_dir("capture");
        let bundle = dir.join("b.scrawl");
        std::fs::create_dir_all(&bundle).unwrap();
        let counted = json!({"key": "site:example.com", "folder": "/code/x"});
        std::fs::write(
            bundle.join(super::super::RECORD),
            serde_json::to_vec(&json!({"evidence": {}, "learned": counted, "offerRule": {}}))
                .unwrap(),
        )
        .unwrap();
        let record = Record {
            delivery_id: "c1".into(),
            time: SENT_AT,
            session: "ses_hex".into(),
            title: None,
            message_id: Some("msg_mine".into()),
            delivery: "steer".into(),
            was_idle: Some(true),
            origin: Origin {
                kind: "capture".into(),
                bundle: Some(bundle.clone()),
                ..Default::default()
            },
            undone: None,
        };
        let line = correction(&record, Undone::Disowned, 9).unwrap();
        jev::check(&line).unwrap();
        assert_eq!(line["bundle"], "b.scrawl");
        assert_eq!(line["kind"], "capture");
        assert!(!capture_undone(&bundle));
        mark_capture(&bundle, &record, Undone::Disowned, 9, None);
        assert!(capture_undone(&bundle));
        let body: Value =
            serde_json::from_slice(&std::fs::read(bundle.join(super::super::RECORD)).unwrap())
                .unwrap();
        assert!(body.get("learned").is_none() && body.get("offerRule").is_none());
        assert_eq!(body["undone"]["how"], "disowned");
        let _ = std::fs::remove_dir_all(dir);
    }
}
