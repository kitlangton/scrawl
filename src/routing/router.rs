//! `scrawl router decide`, `deliver`, `correct` and `eval`: Jev routing for any item (Hex calls
//! it for dictation), sending a decided item, logging Kit's corrections, and Jev's accuracy on
//! past captures whose real destination is known. Only `deliver` sends. The contract is in
//! `docs/routing.md`.

use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail, ensure};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};

use super::jev::{self, Action, Asking, Band, Past, Record, Subject, Thresholds};
use super::look;
use super::smart::{
    self, Basis, Candidate, Clue, Decision, Evidence, Judgement, Pointer, Policy, Source,
};
use crate::delivery::{self, Delivery, Forward};
use crate::model::ScrawlBundle;

/// Hex's deadline is 3 s: `decide` answers within this whatever happens...
const DEADLINE: Duration = Duration::from_millis(2500);
/// ...and reading sessions and asking Jev must be done by this.
const JEV_DEADLINE: Duration = Duration::from_millis(2300);
/// Saved decisions are kept this long for `deliver` and `correct`.
const KEEP_FOR: Duration = Duration::from_secs(7 * 24 * 3600);
/// The longest selection passed to Jev.
const SELECTION_CHARS: usize = 2000;
/// Candidates listed when Jev is sure or lost.
const LISTED: usize = 5;

/// `scrawl router decide`'s input. Unknown fields are ignored.
#[derive(Clone, Debug, Deserialize)]
struct Input {
    item: Item,
    /// Sessions pinned in Hex: always candidates, never winners on their own.
    #[serde(default, deserialize_with = "one_or_many")]
    pinned: Vec<String>,
    /// Recent items and where they went, passed to Jev as state.
    #[serde(default)]
    history: Vec<Past>,
}

#[derive(Clone, Debug, Default, Deserialize)]
struct Item {
    /// The caller's ID for the item (Hex's UUID), echoed in the reply and the log.
    #[serde(default, alias = "id")]
    item_id: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(alias = "transcript")]
    text: String,
    #[serde(default)]
    app: Option<App>,
    #[serde(default, alias = "window_title")]
    title: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    selected_text: Option<String>,
    /// Hex's mode.
    #[serde(default)]
    mode: Option<String>,
    /// A capture's bundle, which `deliver` sends the normal way.
    #[serde(default)]
    bundle: Option<PathBuf>,
    /// What was on screen ([`jev::Screen`]), read leniently and capped.
    #[serde(default)]
    windows: Value,
    #[serde(default)]
    focused: Value,
    #[serde(default)]
    visible_text: Value,
    #[serde(default)]
    opencode_tab: Value,
    #[serde(default)]
    recent: Value,
}

impl Item {
    fn kind(&self) -> &str {
        present(&self.kind).unwrap_or("dictation")
    }

    fn screen(&self) -> jev::Screen {
        jev::Screen::from_values(
            &self.windows,
            &self.focused,
            &self.visible_text,
            &self.opencode_tab,
            &self.recent,
        )
    }

    fn clear_screen(&mut self) {
        for field in [
            &mut self.windows,
            &mut self.focused,
            &mut self.visible_text,
            &mut self.opencode_tab,
            &mut self.recent,
        ] {
            *field = Value::Null;
        }
    }

    fn app_name(&self) -> Option<String> {
        self.app.as_ref().and_then(App::name)
    }
}

/// A bundle ID or name, or both.
#[derive(Clone, Debug, Deserialize)]
#[serde(untagged)]
enum App {
    Text(String),
    Both {
        #[serde(default)]
        id: Option<String>,
        #[serde(default)]
        name: Option<String>,
    },
}

impl App {
    /// The name, else the bundle ID's last part (`com.mitchellh.ghostty` → `ghostty`).
    fn name(&self) -> Option<String> {
        let (id, name) = match self {
            Self::Text(text) if text.contains('.') && !text.contains(' ') => (Some(text), None),
            Self::Text(text) => (None, Some(text)),
            Self::Both { id, name } => (id.as_ref(), name.as_ref()),
        };
        name.map(|n| n.trim().to_owned())
            .filter(|n| !n.is_empty())
            .or_else(|| id.and_then(|id| id.rsplit('.').next()).map(str::to_owned))
            .filter(|n| !n.is_empty())
    }
}

fn one_or_many<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match Option::<OneOrMany>::deserialize(d)? {
        None => Vec::new(),
        Some(OneOrMany::One(id)) => vec![id],
        Some(OneOrMany::Many(ids)) => ids,
    })
}

/// `scrawl router decide`'s output.
#[derive(Clone, Debug, Serialize)]
pub struct Output {
    pub decision_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item_id: Option<String>,
    /// `send`, `ask`, `fallback` (HOME) or `keep`.
    pub decision: &'static str,
    pub target: Option<Target>,
    /// Jev's probabilities, best first (for `ask` the top 3 plus HOME); without Jev, the sessions
    /// smart routing would show, unscored.
    pub candidates: Vec<Target>,
    pub confidence: Option<f64>,
    pub any_fits: Option<f64>,
    pub model: Option<String>,
    pub ms: Option<u64>,
    pub why: String,
}

impl Output {
    fn timeout() -> Self {
        Self {
            decision_id: String::new(),
            item_id: None,
            decision: Band::Keep.label(),
            target: None,
            candidates: Vec::new(),
            confidence: None,
            any_fits: None,
            model: None,
            ms: None,
            why: jev::TIMED_OUT.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Target {
    pub id: String,
    pub title: String,
    pub directory: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    /// On an ask's chips: `jev`, `smart` or `home` ([`jev::Chip`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<&'static str>,
}

// MARK: Deciding

/// Where `input`'s item goes: site rules and signed pages first, then Jev, within [`DEADLINE`].
/// Logs the decision and saves it for `deliver` and `correct`. Sends nothing.
pub fn decide(input: &str) -> Result<Output> {
    let started = Instant::now();
    let request: Value = serde_json::from_str(input).context("expected JSON on stdin")?;
    let parsed: Input = serde_json::from_value(request.clone())
        .context("expected {\"item\": {\"text\": …}, \"pinned\"?, \"history\"?}")?;
    let hooks = jev::Hooks {
        deadline: Some(started + JEV_DEADLINE),
        ..jev::Hooks::default()
    };
    let thresholds = Thresholds::configured();
    let now = jev::now_ms();
    let (tx, rx) = mpsc::channel();
    let input = parsed.clone();
    std::thread::spawn(move || {
        let _ = tx.send(decide_with(
            &input,
            &look::Senses::default(),
            &hooks,
            &thresholds,
            now,
        ));
    });
    let id = new_id(now);
    let item = &parsed.item;
    let app = item.app_name();
    let (mut output, mut line) = match rx.recv_timeout(DEADLINE.saturating_sub(started.elapsed())) {
        Ok((output, judgement, record)) => {
            let line = jev::decision_line(
                item.kind(),
                Subject::Decision(&id),
                &judgement.decision,
                &record,
                app.as_deref(),
                now,
            );
            (output, line)
        }
        Err(_) => {
            let mut line = jev::decision_line(
                item.kind(),
                Subject::Decision(&id),
                &Decision::Keep {
                    reason: jev::TIMED_OUT.into(),
                },
                &Record::default(),
                app.as_deref(),
                now,
            );
            line["by"] = json!(jev::TIMED_OUT);
            (Output::timeout(), line)
        }
    };
    output.decision_id.clone_from(&id);
    output.item_id = present(&item.item_id).map(str::to_owned);
    if output.why == jev::TIMED_OUT {
        line_timeout(&mut output);
    }
    if let Some(item_id) = &output.item_id {
        line["item_id"] = json!(item_id);
    }
    jev::log_line(&line);
    if let Some(dir) = decisions_dir() {
        if let Err(err) = save(&dir, &id, &request, &output, now) {
            crate::capture::scrawl_log(format!("router: couldn't save decision {id}: {err:#}"));
        }
        prune(&dir, KEEP_FOR);
        if let Some(outbox) = outbox_dir() {
            prune(&outbox, KEEP_FOR);
        }
    }
    Ok(output)
}

/// A timed-out answer names no target; Hex shows its picker.
fn line_timeout(output: &mut Output) {
    output.decision = Band::Keep.label();
    output.target = None;
}

fn present(text: &Option<String>) -> Option<&str> {
    text.as_deref().map(str::trim).filter(|t| !t.is_empty())
}

fn decide_with(
    input: &Input,
    senses: &look::Senses,
    hooks: &jev::Hooks,
    thresholds: &Thresholds,
    now_ms: u64,
) -> (Output, Judgement, Record) {
    let item = &input.item;
    let url = present(&item.url);
    let app = item.app_name();
    let mut clues = url
        .map(|url| look::url_clues(url, senses, true))
        .unwrap_or_default();
    for (source, text) in [
        (Source::Title, present(&item.title)),
        (Source::App, app.as_deref()),
    ] {
        let Some(text) = text else { continue };
        let words = smart::words(text);
        if !words.is_empty() {
            let note = match source {
                Source::Title => format!("window title \u{201c}{text}\u{201d}"),
                _ => format!("the app in front is {text}"),
            };
            clues.push(Clue::new(source, Pointer::Words { words }, note));
        }
    }
    let pinned: Vec<String> = input
        .pinned
        .iter()
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty())
        .collect();
    let (mut candidates, error) = look::read_with(&clues, None, &pinned, senses);
    look::look_at_page(url, &mut clues, &mut candidates, senses);
    clues.extend(look::named_clue(
        &item.text,
        &candidates,
        &(senses.aliases)(),
    ));
    let evidence = Evidence {
        address: None,
        clues,
        candidates,
        now_ms,
        url: url.map(str::to_owned),
    };
    let smart = smart::decide(&evidence, Policy::Weigh);
    let mut capture = serde_json::Map::new();
    capture.insert("kind".into(), json!(item.kind()));
    capture.insert("text".into(), json!(jev::trim_middle(&item.text, 2000)));
    if let Some(selected) = present(&item.selected_text) {
        capture.insert(
            "selected_text".into(),
            json!(jev::trim_middle(selected, SELECTION_CHARS)),
        );
    }
    if let Some(app) = &app {
        capture.insert("app".into(), json!(app));
    }
    for (key, value) in [("window_title", &item.title), ("mode", &item.mode)] {
        if let Some(value) = present(value) {
            capture.insert(key.into(), json!(value));
        }
    }
    if let Some(url) = url {
        capture.insert("url".into(), json!(url));
    }
    let asking = Asking {
        history: input.history.clone(),
        pinned,
        screen: item.screen(),
    };
    let (judgement, record) = jev::judge(
        &evidence,
        smart,
        &Value::Object(capture),
        &asking,
        None,
        hooks,
        thresholds,
    );
    let output = output_for(&evidence, &judgement, &record, error.as_deref());
    (output, judgement, record)
}

fn target(c: &Candidate, score: Option<f64>) -> Target {
    Target {
        id: c.id.clone(),
        title: c.label().to_owned(),
        directory: c.directory.as_ref().map(|d| d.display().to_string()),
        score,
        from: None,
    }
}

fn output_for(
    evidence: &Evidence,
    judgement: &Judgement,
    record: &Record,
    read_error: Option<&str>,
) -> Output {
    let asked = record.by == "jev";
    let band = if asked {
        record.band
    } else {
        Band::of(&judgement.decision)
    };
    let to = match &judgement.decision {
        Decision::Send { to, .. } => Some(target(&to.candidate, None)),
        Decision::Offer { choices } => choices.first().map(|c| target(&c.candidate, None)),
        Decision::Keep { .. } => None,
    };
    let find = |id: &str| evidence.candidates.iter().find(|c| c.id == id);
    let candidates = if asked {
        match &judgement.decision {
            // The chips the capsule would show, each with Jev's probability for it.
            Decision::Offer { choices } if band == Band::Ask => choices
                .iter()
                .map(|s| {
                    let p = record
                        .probabilities
                        .iter()
                        .find(|pick| pick.session == s.candidate.id)
                        .map_or(0.0, |pick| pick.p);
                    Target {
                        from: Some(jev::chip_source(s)),
                        ..target(&s.candidate, Some(p))
                    }
                })
                .collect(),
            _ => record
                .probabilities
                .iter()
                .filter_map(|pick| Some(target(find(&pick.session)?, Some(pick.p))))
                .take(LISTED)
                .collect(),
        }
    } else {
        match &judgement.decision {
            Decision::Send { to, instead } => std::iter::once(to)
                .chain(instead)
                .map(|s| target(&s.candidate, None))
                .collect(),
            Decision::Offer { choices } => {
                choices.iter().map(|s| target(&s.candidate, None)).collect()
            }
            Decision::Keep { .. } => Vec::new(),
        }
    };
    let timed_out = record
        .error
        .as_deref()
        .is_some_and(|e| e.starts_with(jev::TIMED_OUT));
    let why = if timed_out {
        jev::TIMED_OUT.to_owned()
    } else if asked {
        match band {
            Band::Send => "jev: confident".to_owned(),
            Band::Ask => "jev: unsure".to_owned(),
            Band::Fallback => "jev: can't tell, so HOME".to_owned(),
            Band::Keep if jev::home_session(&evidence.candidates).is_none() => {
                "jev: can't tell, and there's no HOME session".to_owned()
            }
            Band::Keep => "jev: can't tell".to_owned(),
        }
    } else {
        let mut why = settled_why(judgement).unwrap_or_else(|| record.reason.clone());
        if let Some(err) = record.error.as_deref().or(read_error) {
            let err: String = err.chars().take(120).collect();
            why = format!("{why} ({err})");
        }
        why
    };
    let mut output = Output {
        decision_id: String::new(),
        item_id: None,
        decision: band.label(),
        target: to,
        candidates,
        confidence: record.confidence,
        any_fits: record.fits,
        model: record.model.clone(),
        ms: record.ms,
        why,
    };
    if timed_out {
        line_timeout(&mut output);
    }
    output
}

/// The source that settled a decision without Jev, in a few words.
fn settled_why(judgement: &Judgement) -> Option<String> {
    if !jev::settled(judgement) {
        return None;
    }
    let Decision::Send { to, .. } = &judgement.decision else {
        return None;
    };
    Some(
        match to.lead()?.basis {
            Basis::Chosen => "chosen",
            Basis::Recorded => "recorded destination",
            Basis::Clue(Source::Named) => "named at the start",
            Basis::Clue(Source::Rule) => "routing.sites rule",
            Basis::Clue(Source::Page) => "signed page",
            Basis::Clue(Source::Port) => "localhost port",
            Basis::Clue(Source::Publisher) => "publisher marker",
            _ => return None,
        }
        .to_owned(),
    )
}

// MARK: Saved decisions

/// `~/Library/Application Support/scrawl/router` (`$SCRAWL_ROUTER_DIR`); never in tests.
fn router_dir() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    if let Some(dir) = std::env::var_os("SCRAWL_ROUTER_DIR") {
        return Some(PathBuf::from(dir));
    }
    Some(dirs::home_dir()?.join("Library/Application Support/scrawl/router"))
}

/// Each decision's request and reply, `<id>.json`, for `deliver` and `correct`.
pub fn decisions_dir() -> Option<PathBuf> {
    router_dir().map(|d| d.join("decisions"))
}

/// Text prompts `deliver` couldn't send yet, a folder each, retried by the app.
pub fn outbox_dir() -> Option<PathBuf> {
    router_dir().map(|d| d.join("outbox"))
}

/// A short ID, unique enough: the time in ms and some of its nanoseconds, in hex.
fn new_id(now_ms: u64) -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let salt = (nanos ^ std::process::id().rotate_left(16)) & 0xffff;
    format!("{now_ms:x}{salt:04x}")
}

fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

#[derive(Debug, Deserialize, Serialize)]
struct Saved {
    time: u64,
    request: Value,
    reply: Value,
}

impl Saved {
    fn input(&self) -> Result<Input> {
        serde_json::from_value(self.request.clone()).context("the saved request doesn't parse")
    }

    fn target(&self) -> Option<(String, String)> {
        let t = self.reply.get("target")?;
        Some((
            t.get("id")?.as_str()?.to_owned(),
            t.get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
        ))
    }

    /// A session's title as the reply listed it.
    fn title_of(&self, id: &str) -> Option<String> {
        let reply = &self.reply;
        reply
            .get("target")
            .into_iter()
            .chain(
                reply
                    .get("candidates")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten(),
            )
            .find(|t| t.get("id").and_then(Value::as_str) == Some(id))
            .and_then(|t| t.get("title")?.as_str().map(str::to_owned))
    }
}

fn save(dir: &Path, id: &str, request: &Value, output: &Output, now: u64) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let saved = Saved {
        time: now,
        request: request.clone(),
        reply: serde_json::to_value(output)?,
    };
    let tmp = dir.join(format!(".{id}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec(&saved)?)?;
    std::fs::rename(&tmp, dir.join(format!("{id}.json")))?;
    Ok(())
}

fn load(dir: &Path, id: &str) -> Result<Saved> {
    ensure!(valid_id(id), "{id:?} isn't a decision ID");
    let path = dir.join(format!("{id}.json"));
    let bytes = std::fs::read(&path)
        .with_context(|| format!("no decision {id} (decisions are kept for 7 days)"))?;
    serde_json::from_slice(&bytes).with_context(|| format!("{} is unreadable", path.display()))
}

/// The newest saved decision about the caller's item `item_id`, with its decision ID.
fn find_by_item(dir: &Path, item_id: &str) -> Option<(String, Saved)> {
    std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let id = path.file_stem()?.to_str()?.to_owned();
            if path.extension()? != "json" || !valid_id(&id) {
                return None;
            }
            let saved: Saved = serde_json::from_slice(&std::fs::read(&path).ok()?).ok()?;
            let item = saved.request.get("item")?;
            let theirs = item.get("item_id").or_else(|| item.get("id"))?.as_str()?;
            (theirs == item_id).then_some((id, saved))
        })
        .max_by_key(|(_, saved)| saved.time)
}

/// Removes entries of `dir` not modified within `keep`.
fn prune(dir: &Path, keep: Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let old = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age > keep);
        if old {
            let path = entry.path();
            let _ = if path.is_dir() {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
        }
    }
}

// MARK: Delivering

/// `scrawl router deliver`'s output.
#[derive(Debug, PartialEq, Serialize)]
pub struct Delivered {
    pub ok: bool,
    /// `sent`, or `queued` (the app retries it).
    pub state: &'static str,
    pub session: String,
}

/// The sends, swappable in tests.
#[derive(Clone, Copy)]
pub struct DeliverHooks {
    /// Sends the prompt in an outbox folder.
    pub text: Forward,
    /// Sends a capture's bundle.
    pub bundle: Forward,
}

impl Default for DeliverHooks {
    fn default() -> Self {
        Self {
            text: send_prompt,
            bundle: crate::handoff::send_bundle_to_session,
        }
    }
}

/// Sends the prompt saved in an outbox folder to `session` (the app's retries call this too).
pub fn send_prompt(dir: &Path, session: &str) -> Result<()> {
    let prompt: Value = serde_json::from_slice(&std::fs::read(dir.join(delivery::PROMPT))?)?;
    let text = prompt
        .get("text")
        .and_then(Value::as_str)
        .context("the prompt has no text")?;
    crate::handoff::send_text_to_session(session, text)
}

/// What `scrawl router deliver` is told: a decision by its ID or the caller's item ID, the
/// session, and the text from stdin, which wins over the decision's.
#[derive(Debug, Default)]
pub struct DeliverRequest {
    pub decision: Option<String>,
    pub item_id: Option<String>,
    pub to: String,
    pub text: Option<String>,
}

/// `scrawl router deliver`.
pub fn deliver(request: DeliverRequest) -> Result<Delivered> {
    let dir = decisions_dir().context("no decisions folder")?;
    let outbox = outbox_dir().context("no outbox folder")?;
    deliver_with(&dir, request, &outbox, &DeliverHooks::default())
}

fn deliver_with(
    decisions: &Path,
    request: DeliverRequest,
    outbox: &Path,
    hooks: &DeliverHooks,
) -> Result<Delivered> {
    let to = request.to.trim();
    ensure!(
        to.starts_with("ses")
            && to
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-'),
        "{to:?} isn't an OpenCode session ID"
    );
    let item_id = request
        .item_id
        .as_deref()
        .map(str::trim)
        .filter(|i| !i.is_empty());
    let found = match (&request.decision, item_id) {
        (Some(id), _) => Some((id.clone(), load(decisions, id)?)),
        (None, Some(item_id)) => find_by_item(decisions, item_id),
        (None, None) => bail!("give --decision <id>, or --item-id <id> with the text on stdin"),
    };
    let text = request
        .text
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());
    let mut item = match &found {
        Some((_, saved)) => saved.input()?.item,
        None => Item::default(),
    };
    match text {
        Some(text) => {
            item.text = text.to_owned();
            // Hex's text is the whole prompt; the saved selection may no longer apply.
            item.selected_text = None;
        }
        None if found.is_none() => bail!("no saved decision for that item, and no text on stdin"),
        None => {}
    }
    let title = found
        .as_ref()
        .and_then(|(_, saved)| saved.title_of(to))
        .unwrap_or_else(|| to.to_owned());
    let key: String = match (&found, item_id) {
        (Some((id, _)), _) => id.clone(),
        (None, Some(item_id)) => item_id
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .take(32)
            .collect(),
        (None, None) => unreachable!(),
    };
    ensure!(!key.is_empty(), "the item ID has no letters or digits");
    let bundle = item
        .bundle
        .as_ref()
        .filter(|b| item.kind() == "capture" && text.is_none() && b.is_dir());
    let delivery = match bundle {
        Some(bundle) => delivery::deliver(bundle, to, &title, None, hooks.bundle),
        None => {
            let folder = outbox.join(&key);
            std::fs::create_dir_all(&folder)?;
            let _ = std::fs::remove_file(folder.join(delivery::PENDING));
            std::fs::write(
                folder.join(delivery::PROMPT),
                serde_json::to_vec(&json!({"text": prompt_text(&item)}))?,
            )?;
            let delivery = delivery::deliver(&folder, to, &title, None, hooks.text);
            if matches!(delivery, Delivery::Sent { .. }) {
                let _ = std::fs::remove_dir_all(&folder);
            }
            delivery
        }
    };
    let state = match delivery {
        Delivery::Sent { .. } => "sent",
        _ => "queued",
    };
    Ok(Delivered {
        ok: true,
        state,
        session: to.to_owned(),
    })
}

/// The prompt for an item: a bracketed first line saying where it came from
/// (`[voice via Hex · from Ghostty]`), then the text and any selection.
fn prompt_text(item: &Item) -> String {
    if item.text.trim_start().starts_with("[voice via Hex") {
        return item.text.trim().to_owned();
    }
    let mut head = match item.kind() {
        "capture" => "[scrawl capture".to_owned(),
        _ => "[voice via Hex".to_owned(),
    };
    if let Some(app) = item.app_name() {
        head.push_str(&format!(" · from {app}"));
    }
    head.push(']');
    let mut text = format!("{head}\n{}", item.text.trim());
    if let Some(selected) = present(&item.selected_text) {
        let quoted: Vec<String> = selected.lines().map(|l| format!("> {l}")).collect();
        text.push_str(&format!("\n\nSelected text:\n{}", quoted.join("\n")));
    }
    text
}

// MARK: Correcting

/// `scrawl router correct --decision <id> --action <action> [--to <session>]`: from the
/// decision's target to `to`.
pub fn correct(id: &str, action: Action, to: Option<&str>) -> Result<()> {
    let dir = decisions_dir().context("no decisions folder")?;
    let saved = load(&dir, id)?;
    jev::write_correction(&correction(&saved, id, action, to, jev::now_ms())?)
}

fn correction(
    saved: &Saved,
    id: &str,
    action: Action,
    to: Option<&str>,
    now: u64,
) -> Result<Value> {
    let to = to.map(str::trim).filter(|t| !t.is_empty());
    let item = saved.input()?.item;
    let from = saved.target();
    let to_title = to.map(|t| saved.title_of(t).unwrap_or_default());
    let app = item.app_name();
    let mut line = jev::Correction {
        kind: item.kind(),
        subject: Subject::Decision(id),
        action,
        detail: None,
        from: from
            .as_ref()
            .map(|(id, title)| (id.as_str(), title.as_str())),
        to: to.zip(to_title.as_deref()),
        app: app.as_deref(),
    }
    .line(now);
    if let Some(item_id) = present(&item.item_id) {
        line["item_id"] = json!(item_id);
    }
    Ok(line)
}

/// `scrawl router log`: one correction line from stdin (Hex's spelling), joined to its
/// decision by `decision_id` or `item_id`, then written like every other correction.
pub fn log(input: &str) -> Result<()> {
    let line: Value =
        serde_json::from_str(input.trim()).context("expected one JSON object on stdin")?;
    let line = complete(line, decisions_dir().as_deref(), jev::now_ms())?;
    jev::write_correction(&line)
}

/// Fills in what the decision knows: `time`, `decision_id` from `item_id` (or the reverse),
/// `from` when missing, titles for bare session IDs, and `app`.
fn complete(mut line: Value, decisions: Option<&Path>, now: u64) -> Result<Value> {
    let obj = line.as_object_mut().context("expected a JSON object")?;
    obj.entry("time").or_insert(json!(now));
    let text = |obj: &serde_json::Map<String, Value>, key: &str| {
        obj.get(key).and_then(Value::as_str).map(str::to_owned)
    };
    let found = match (text(obj, "decision_id"), text(obj, "item_id"), decisions) {
        (Some(id), _, Some(dir)) => load(dir, &id).ok().map(|saved| (id, saved)),
        (None, Some(item_id), Some(dir)) => find_by_item(dir, &item_id),
        _ => None,
    };
    if let Some((id, saved)) = &found {
        obj.entry("decision_id").or_insert(json!(id));
        if let Ok(input) = saved.input() {
            if let Some(item_id) = present(&input.item.item_id) {
                obj.entry("item_id").or_insert(json!(item_id));
            }
            if let Some(app) = input.item.app_name() {
                obj.entry("app").or_insert(json!(app));
            }
        }
        if obj.get("from").is_none_or(Value::is_null) {
            if let Some((id, title)) = saved.target() {
                obj.insert("from".into(), json!({"id": id, "title": title}));
            }
        }
    }
    for key in ["from", "to"] {
        if let Some(Value::String(id)) = obj.get(key).cloned() {
            let title = found
                .as_ref()
                .and_then(|(_, saved)| saved.title_of(&id))
                .unwrap_or_default();
            obj.insert(key.into(), json!({"id": id, "title": title}));
        }
    }
    Ok(line)
}

/// `7d`, `36h`, `90m`.
pub fn parse_since(text: &str) -> Result<Duration> {
    let text = text.trim();
    let (number, unit) = text.split_at(text.len().saturating_sub(1));
    let n: u64 = number
        .parse()
        .with_context(|| format!("{text}: expected a number then d, h or m"))?;
    Ok(Duration::from_secs(
        n * match unit {
            "d" => 86_400,
            "h" => 3_600,
            "m" => 60,
            _ => bail!("{text}: expected a number then d, h or m"),
        },
    ))
}

/// A past capture with a known destination.
struct Case {
    dir: PathBuf,
    bundle: ScrawlBundle,
    evidence: Evidence,
    smart: Judgement,
    truth: String,
    /// `chosen` (picked on a chip), `delivered` (found in OpenCode's history) or `sent`.
    how: &'static str,
}

struct Outcome {
    band: Band,
    pick: Option<String>,
    confidence: f64,
    margin: f64,
    fits: f64,
    /// Where the band would have put it: sent to, or the first chip.
    landed: Option<String>,
    /// Whether the truth was on one of the two chips an offer shows.
    on_chips: bool,
}

/// About this many input tokens per question, for the estimate printed first.
const TOKENS_PER_CASE: u64 = 5_000;

/// `lines: true` puts the targets' recent lines in every question whatever the config says,
/// and `screen: false` leaves the `screen` out, to compare.
pub fn eval(since: Duration, limit: usize, lines: bool, screen: bool) -> Result<()> {
    let cutoff = jev::now_ms().saturating_sub(since.as_millis() as u64);
    let captures = crate::handoff::default_captures_dir();
    let mut dirs: Vec<(u64, PathBuf)> = std::fs::read_dir(&captures)
        .with_context(|| format!("couldn't read {}", captures.display()))?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "scrawl") && p.join(super::RECORD).is_file())
        .filter_map(|p| {
            let (_, bundle) = crate::handoff::load_bundle(&p).ok()?;
            (bundle.created_at_ms >= cutoff).then_some((bundle.created_at_ms, p))
        })
        .collect();
    dirs.sort_by(|a, b| b.0.cmp(&a.0));
    let senses = look::Senses::default();
    let (mut settled, mut unknown, mut absent) = (0, 0, 0);
    let mut cases = Vec::new();
    for (_, dir) in dirs {
        if cases.len() >= limit {
            break;
        }
        let Ok((_, bundle)) = crate::handoff::load_bundle(&dir) else {
            continue;
        };
        let destination = bundle.destination.clone();
        let truth = match &destination {
            Some(d) if d.via == "chosen" => Some((d.session.clone(), "chosen")),
            _ => jev::delivered_to(&dir, bundle.created_at_ms)
                .map(|id| (id, "delivered"))
                .or_else(|| destination.map(|d| (d.session, "sent"))),
        };
        let Some((truth, how)) = truth else {
            unknown += 1;
            continue;
        };
        let (evidence, _, _) = look::evidence_for_bundle(&dir, &bundle, None, &senses);
        if !evidence.candidates.iter().any(|c| c.id == truth) {
            absent += 1;
            continue;
        }
        let smart = smart::decide(&evidence, Policy::Weigh);
        if jev::settled(&smart) {
            settled += 1;
            continue;
        }
        cases.push(Case {
            dir,
            bundle,
            evidence,
            smart,
            truth,
            how,
        });
    }
    println!(
        "Asking Jev ({}) about {} captures: about {} input tokens, ~${:.4}",
        jev::MODEL,
        cases.len(),
        cases.len() as u64 * TOKENS_PER_CASE,
        jev::cost(cases.len() as u64 * TOKENS_PER_CASE)
    );
    println!(
        "Skipped: {settled} settled without Jev (address, rule, page, port, publisher), \
         {unknown} with no known destination, {absent} whose destination wasn't among the sessions read"
    );
    let mut thresholds = Thresholds::configured();
    thresholds.lines |= lines;
    let hooks = jev::Hooks::default();
    if thresholds.lines {
        println!("Targets with their recent lines (--lines or \"lines\": true)");
    }
    if !screen {
        println!("Without `screen` (--no-screen)");
    }
    let mut results: Vec<(&Case, Outcome)> = Vec::new();
    let (mut tokens, mut ms, mut failed) = (0_u64, Vec::new(), 0);
    let mut model = String::new();
    for case in &cases {
        let capture = jev::capture_state(&case.bundle, case.evidence.url.as_deref());
        let (judgement, record) = jev::judge(
            &case.evidence,
            case.smart.clone(),
            &capture,
            &if screen {
                jev::capture_asking(&case.bundle, &case.evidence.candidates)
            } else {
                Asking::default()
            },
            Some(case.bundle.created_at_ms),
            &hooks,
            &thresholds,
        );
        if record.by != "jev" {
            failed += 1;
            println!(
                "  {}  failed: {}",
                short(&case.dir),
                record.error.as_deref().unwrap_or(&record.reason)
            );
            continue;
        }
        tokens += record.input_tokens.unwrap_or(0);
        ms.extend(record.ms);
        if let Some(m) = &record.model {
            model.clone_from(m);
        }
        let (landed, on_chips) = landed(&judgement.decision, &case.truth, jev::CHIPS);
        let outcome = Outcome {
            band: record.band,
            pick: record.target.as_ref().map(|t| t.session.clone()),
            confidence: record.confidence.unwrap_or(0.0),
            margin: margin_of(&record),
            fits: record.fits.unwrap_or(0.0),
            landed,
            on_chips,
        };
        let title = |id: &str| {
            case.evidence
                .candidates
                .iter()
                .find(|c| c.id == id)
                .map_or_else(|| id.to_owned(), |c| c.label().to_owned())
        };
        println!(
            "  {}  {:<5} {} conf {:.2} margin {:.2} fits {:.2} · Jev {} · truth {} ({})",
            short(&case.dir),
            outcome.band.label(),
            if outcome.pick.as_deref() == Some(&case.truth) {
                "✓"
            } else {
                "✗"
            },
            outcome.confidence,
            outcome.margin,
            outcome.fits,
            outcome.pick.as_deref().map(title).unwrap_or_default(),
            title(&case.truth),
            case.how
        );
        results.push((case, outcome));
    }
    report(&results, &thresholds, tokens, &mut ms, failed, &model);
    let mut samples: Vec<Sample> = results
        .iter()
        .map(|(case, o)| Sample {
            kind: "capture",
            confidence: o.confidence,
            margin: o.margin,
            fits: o.fits,
            right: o.pick.as_deref() == Some(case.truth.as_str()),
        })
        .collect();
    let captures = samples.len();
    if let Some(dir) = router_dir().map(|d| d.join("eval")) {
        samples.extend(eval_items(&dir, limit, &hooks, &thresholds, screen));
    }
    println!();
    println!(
        "Gates, one at a time with the others at their settings ({captures} captures, {} items):",
        samples.len() - captures
    );
    print!("{}", sweep(&samples, &thresholds));
    print!("{}", demotions(&samples, &thresholds));
    Ok(())
}

/// The confidence less the likeliest other target's probability.
fn margin_of(record: &Record) -> f64 {
    let chosen = record.target.as_ref().map(|t| t.t.as_str());
    let second = record
        .probabilities
        .iter()
        .filter(|p| Some(p.t.as_str()) != chosen)
        .map(|p| p.p)
        .fold(0.0, f64::max);
    record.confidence.unwrap_or(0.0) - second
}

/// One answer for the sweep: its numbers, and whether sending to Jev's choice would be right.
#[derive(Clone, Copy, Debug)]
struct Sample {
    /// `capture` or `dictation`.
    kind: &'static str,
    confidence: f64,
    margin: f64,
    fits: f64,
    right: bool,
}

/// For each send gate (confidence, margin), what each value sends, and how many right and
/// wrong sends it blocks compared with no gate there, holding the other at `thresholds`.
fn sweep(samples: &[Sample], thresholds: &Thresholds) -> String {
    use std::fmt::Write as _;
    let gates: [(&str, f64, &[f64]); 2] = [
        (
            "confidence",
            thresholds.send,
            &[0.6, 0.7, 0.8, 0.85, 0.9, 0.95],
        ),
        ("margin", thresholds.margin, &[0.0, 0.1, 0.2, 0.3, 0.4, 0.5]),
    ];
    let at = |s: &Sample, gate: usize, value: f64| {
        let mut v = [thresholds.send, thresholds.margin];
        v[gate] = value;
        s.confidence >= v[0] && s.margin >= v[1]
    };
    let mut out = String::new();
    for (i, (name, setting, values)) in gates.iter().enumerate() {
        let open: Vec<&Sample> = samples
            .iter()
            .filter(|s| at(s, i, f64::NEG_INFINITY))
            .collect();
        let _ = writeln!(
            out,
            "  {name} (set {setting}): value  sends  wrong  blocks right  blocks wrong"
        );
        for &value in *values {
            let sent: Vec<&&Sample> = open.iter().filter(|s| at(s, i, value)).collect();
            let wrong = sent.iter().filter(|s| !s.right).count();
            let blocked_right = open.iter().filter(|s| s.right && !at(s, i, value)).count();
            let blocked_wrong = open.iter().filter(|s| !s.right && !at(s, i, value)).count();
            let _ = writeln!(
                out,
                "  {:>10}  {value:>15.2}  {:>5}  {:>5}  {:>12}  {:>12}{}",
                "",
                sent.len(),
                wrong,
                blocked_right,
                blocked_wrong,
                if (value - setting).abs() < 1e-9 {
                    "  ←"
                } else {
                    ""
                }
            );
        }
    }
    out
}

/// How often `any_fits` alone would have demoted a send (it no longer gates one): of the sends
/// confidence and margin allow, the right ones under each `any_fits` value, by kind, and the
/// wrong ones it would have stopped.
fn demotions(samples: &[Sample], thresholds: &Thresholds) -> String {
    use std::fmt::Write as _;
    let sends: Vec<&Sample> = samples
        .iter()
        .filter(|s| s.confidence >= thresholds.send && s.margin >= thresholds.margin)
        .collect();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "any_fits alone as a send gate: right sends it would demote (rate), by kind"
    );
    let _ = writeln!(
        out,
        "  any_fits  capture          dictation        wrong sends stopped"
    );
    for gate in [0.2, 0.3, jev::OLD_FITS_AT, 0.5, 0.7] {
        let cell = |kind: &str| {
            let right: Vec<_> = sends.iter().filter(|s| s.kind == kind && s.right).collect();
            let demoted = right.iter().filter(|s| s.fits < gate).count();
            if right.is_empty() {
                "—".to_owned()
            } else {
                format!(
                    "{demoted}/{} ({:.0}%)",
                    right.len(),
                    100.0 * demoted as f64 / right.len() as f64
                )
            }
        };
        let wrong = sends.iter().filter(|s| !s.right).count();
        let stopped = sends.iter().filter(|s| !s.right && s.fits < gate).count();
        let _ = writeln!(
            out,
            "  {gate:<8.2}  {:<15}  {:<15}  {stopped}/{wrong}",
            cell("capture"),
            cell("dictation")
        );
    }
    out
}

/// What an eval item should come to: a session, or a decision word.
#[derive(Debug, Deserialize)]
struct Expect {
    #[serde(default)]
    to: Option<String>,
    #[serde(default)]
    decision: Option<String>,
}

#[derive(Debug, Deserialize)]
struct EvalItem {
    #[serde(flatten)]
    input: Input,
    expect: Expect,
}

/// Each line of `dir/*.jsonl` (`{"item": {…}, "expect": {"to": "ses_…"} | {"decision": …}}`),
/// in file order. Lines that don't parse are reported by number.
fn eval_cases(dir: &Path) -> Vec<(String, Result<EvalItem>)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .collect();
    files.sort();
    let mut cases = Vec::new();
    for file in files {
        let name = file
            .file_stem()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        for (n, line) in text.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            cases.push((
                format!("{name}:{}", n + 1),
                serde_json::from_str(line).map_err(anyhow::Error::from),
            ));
        }
    }
    cases
}

/// Whether `output` meets `expect`: the decision word when one is given, and the session it
/// lands on (sent to, or the first choice offered) when one is.
fn meets(output: &Output, expect: &Expect) -> bool {
    let decision = expect
        .decision
        .as_deref()
        .is_none_or(|d| d == output.decision);
    let to = expect.to.as_deref().is_none_or(|to| {
        matches!(output.decision, "send" | "fallback" | "ask")
            && output.target.as_ref().is_some_and(|t| t.id == to)
    });
    decision && to
}

/// Kit's own items (private: their text stays in his Application Support folder), asked live
/// and reported as their own rows by file and line.
fn eval_items(
    dir: &Path,
    limit: usize,
    hooks: &jev::Hooks,
    thresholds: &Thresholds,
    screen: bool,
) -> Vec<Sample> {
    let cases = eval_cases(dir);
    let mut samples = Vec::new();
    if cases.is_empty() {
        return samples;
    }
    println!();
    println!("Items in {} ({} lines):", dir.display(), cases.len());
    let senses = look::Senses::default();
    let (mut right, mut asked, mut wrong_sends) = (0, 0, 0);
    for (name, case) in cases.into_iter().take(limit) {
        let mut case = match case {
            Ok(case) => case,
            Err(err) => {
                println!("  {name:<16} unreadable: {err:#}");
                continue;
            }
        };
        if !screen {
            case.input.item.clear_screen();
        }
        let (output, _, record) =
            decide_with(&case.input, &senses, hooks, thresholds, jev::now_ms());
        asked += 1;
        if record.by == "jev" {
            let pick = record.target.as_ref().map(|t| t.session.as_str());
            samples.push(Sample {
                kind: if case.input.item.kind() == "capture" {
                    "capture"
                } else {
                    "dictation"
                },
                confidence: record.confidence.unwrap_or(0.0),
                margin: margin_of(&record),
                fits: record.fits.unwrap_or(0.0),
                right: case.expect.to.is_some()
                    && pick == case.expect.to.as_deref()
                    && case.expect.decision.as_deref().is_none_or(|d| d == "send"),
            });
        }
        let ok = meets(&output, &case.expect);
        right += usize::from(ok);
        let to = output.target.as_ref().map_or("—", |t| t.title.as_str());
        let expected = match (&case.expect.decision, &case.expect.to) {
            (Some(d), Some(to)) => format!("{d} to {}", title_in(&output, to)),
            (Some(d), None) => d.clone(),
            (None, Some(to)) => title_in(&output, to),
            (None, None) => "anything".into(),
        };
        if !ok && output.decision == "send" {
            wrong_sends += 1;
        }
        println!(
            "  {name:<16} {:<8} {} {to} · conf {} margin {:.2} fits {} · expected {expected} · {}",
            output.decision,
            if ok { "✓" } else { "✗" },
            output.confidence.map_or("—".into(), |c| format!("{c:.2}")),
            margin_of(&record),
            output.any_fits.map_or("—".into(), |f| format!("{f:.2}")),
            output.why,
        );
    }
    println!("Items right: {right}/{asked}, wrong automatic sends: {wrong_sends}");
    samples
}

/// A session's title as `output` listed it, else its ID.
fn title_in(output: &Output, id: &str) -> String {
    output
        .target
        .iter()
        .chain(&output.candidates)
        .find(|t| t.id == id)
        .map_or_else(|| id.to_owned(), |t| t.title.clone())
}

fn short(dir: &std::path::Path) -> String {
    dir.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
        .trim_start_matches("scrawl-")
        .trim_end_matches(".scrawl")
        .to_owned()
}

/// The chips the capsule shows for smart routing's own offer.
const SMART_CHIPS: usize = 2;

/// Where a decision puts a capture, and whether `truth` is on one of the `chips` it shows.
fn landed(decision: &Decision, truth: &str, chips: usize) -> (Option<String>, bool) {
    match decision {
        Decision::Send { to, instead } => (
            Some(to.candidate.id.clone()),
            to.candidate.id == truth || instead.first().is_some_and(|s| s.candidate.id == truth),
        ),
        Decision::Offer { choices } => (
            choices.first().map(|c| c.candidate.id.clone()),
            choices.iter().take(chips).any(|c| c.candidate.id == truth),
        ),
        Decision::Keep { .. } => (None, false),
    }
}

fn report(
    results: &[(&Case, Outcome)],
    thresholds: &Thresholds,
    tokens: u64,
    ms: &mut [u64],
    failed: usize,
    model: &str,
) {
    ms.sort_unstable();
    let median = ms.get(ms.len() / 2).copied().unwrap_or(0);
    println!();
    println!(
        "{} answered by {model}, {failed} failed · median {median} ms, max {} ms · {tokens} input tokens (${:.4})",
        results.len(),
        ms.last().copied().unwrap_or(0),
        jev::cost(tokens)
    );
    println!(
        "Thresholds: send ≥ {} with margin ≥ {}, offer ≥ {} (HOME instead when fits < {}), low below that",
        thresholds.send, thresholds.margin, thresholds.offer, thresholds.no_fit
    );
    let right = |o: &Outcome, truth: &str| o.pick.as_deref() == Some(truth);
    let top = results.iter().filter(|(c, o)| right(o, &c.truth)).count();
    let chosen: Vec<_> = results.iter().filter(|(c, _)| c.how == "chosen").collect();
    let chosen_right = chosen.iter().filter(|(c, o)| right(o, &c.truth)).count();
    println!(
        "Jev's top pick right: {top}/{} (chosen on a chip: {chosen_right}/{})",
        results.len(),
        chosen.len()
    );
    println!("Band    n  landed right  wrong  truth on a chip");
    for band in Band::ALL {
        let in_band: Vec<_> = results.iter().filter(|(_, o)| o.band == band).collect();
        if in_band.is_empty() {
            continue;
        }
        let landed_right = in_band
            .iter()
            .filter(|(c, o)| o.landed.as_deref() == Some(c.truth.as_str()))
            .count();
        let wrong = in_band
            .iter()
            .filter(|(c, o)| o.landed.is_some() && o.landed.as_deref() != Some(c.truth.as_str()))
            .count();
        let chips = in_band.iter().filter(|(_, o)| o.on_chips).count();
        println!(
            "{:<6} {:>2}  {:>12}  {:>5}  {:>15}{}",
            band.label(),
            in_band.len(),
            landed_right,
            wrong,
            chips,
            if band == Band::Send && wrong > 0 {
                "  ← wrong automatic sends"
            } else {
                ""
            }
        );
    }
    let (mut s_sent, mut s_wrong, mut s_offer, mut s_chips, mut s_keep) = (0, 0, 0, 0, 0);
    for (case, _) in results {
        let (landed, on_chips) = landed(&case.smart.decision, &case.truth, SMART_CHIPS);
        match &case.smart.decision {
            Decision::Send { .. } => {
                s_sent += 1;
                if landed.as_deref() != Some(case.truth.as_str()) {
                    s_wrong += 1;
                }
            }
            Decision::Offer { .. } => {
                s_offer += 1;
                if on_chips {
                    s_chips += 1;
                }
            }
            Decision::Keep { .. } => s_keep += 1,
        }
    }
    println!(
        "Smart routing alone on these: sent {s_sent} ({s_wrong} wrong), offered {s_offer} (truth on a chip {s_chips}), kept {s_keep}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_sdk(_: &str) -> Option<crate::web::Registration> {
        None
    }
    fn no_dir(_: u16) -> Option<PathBuf> {
        None
    }
    fn sessions(path: &str, _: Duration) -> Result<Vec<u8>> {
        use crate::handoff::fake_api;
        // Too old to be listed, but readable by ID.
        if path == "/api/session/ses_old" {
            let old = fake_api::session("ses_old", "OLD", "/tmp/old", 1_000);
            return fake_api::serve(&[old], path);
        }
        fake_api::serve(
            &[
                fake_api::session("ses_hex", "HEX", "/tmp/hex", 10_000),
                fake_api::session("ses_a", "A", "/tmp/a", 9_500),
                fake_api::session("ses_home", "HOME", "/tmp/home", 9_000),
                fake_api::session("ses_b", "B", "/tmp/b", 8_500),
                fake_api::session("ses_c", "C", "/tmp/c", 8_000),
            ],
            path,
        )
    }
    fn no_port(_: &crate::routing::WebUrl) -> Option<u16> {
        None
    }
    fn no_terminal(_: &str) -> Option<(PathBuf, String)> {
        None
    }
    fn nothing() -> Vec<(String, PathBuf)> {
        Vec::new()
    }
    fn no_writes(_: &str, _: u64, _: u64) -> Vec<crate::routing::served::Write> {
        Vec::new()
    }
    // Targets newest first: t1 HEX, t2 A, t3 HOME, t4 B, t5 C.
    fn sure(body: &[u8], _: Duration) -> Result<Vec<u8>> {
        ASKED.with(|a| *a.borrow_mut() = serde_json::from_slice(body).ok());
        Ok(
            br#"{"model":"jev-1.13","answers":{"target":{"choice":"t1","confidence":0.95,
            "probabilities":{"t1":0.97,"t2":0.03}},"any_fits":{"noul":0.9}}}"#
                .to_vec(),
        )
    }
    fn unsure(_: &[u8], _: Duration) -> Result<Vec<u8>> {
        Ok(br#"{"model":"jev-1.13","answers":{"target":{"choice":"t4","confidence":0.6,
            "probabilities":{"t1":0.2,"t2":0.1,"t3":0.04,"t4":0.6,"t5":0.06}},"any_fits":{"noul":0.6}}}"#
            .to_vec())
    }
    fn slow(_: &[u8], _: Duration) -> Result<Vec<u8>> {
        bail!("{}: no answer within 2000 ms", jev::TIMED_OUT)
    }
    fn recent(ids: &[String], _: Option<u64>, _: Duration) -> Vec<jev::Recent> {
        vec![jev::Recent::default(); ids.len()]
    }

    thread_local! {
        static ASKED: std::cell::RefCell<Option<Value>> = const { std::cell::RefCell::new(None) };
        static SENT: std::cell::RefCell<Vec<(String, String)>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    fn senses() -> look::Senses {
        look::Senses {
            hooks: crate::routing::Hooks {
                sdk: no_sdk,
                listener_dir: no_dir,
                api: sessions,
            },
            own_port: no_port,
            terminal: no_terminal,
            servers: nothing,
            sites: nothing,
            aliases: no_aliases,
            writes: no_writes,
        }
    }

    fn no_aliases() -> Vec<(String, String)> {
        Vec::new()
    }

    fn run(input: Value, ask: fn(&[u8], Duration) -> Result<Vec<u8>>) -> Value {
        let input: Input = serde_json::from_value(input).unwrap();
        let hooks = jev::Hooks {
            ask,
            recent,
            deadline: None,
        };
        let (out, _, _) = decide_with(&input, &senses(), &hooks, &Thresholds::default(), 100_000);
        serde_json::to_value(out).unwrap()
    }

    #[test]
    fn a_confident_answer_is_a_send() {
        let out = run(
            json!({"item": {"kind": "dictation", "text": "the hex icon is wrong",
                            "app": {"id": "com.kitlangton.Hex", "name": "Hex"}, "mode": "Code", "extra": 1},
                   "history": [{"text": "the menu", "to": "ses_hex"}]}),
            sure,
        );
        assert_eq!(out["decision"], "send", "{out}");
        assert_eq!(out["target"]["id"], "ses_hex");
        assert_eq!(out["target"]["directory"], "/tmp/hex");
        assert_eq!(out["candidates"][0]["score"], 0.97);
        assert_eq!(out["model"], "jev-1.13");
        assert_eq!(out["confidence"], 0.95);
        assert_eq!(out["any_fits"], 0.9);
        assert_eq!(out["why"], "jev: confident");
        let asked = ASKED.with(|a| a.borrow().clone()).unwrap();
        assert_eq!(asked["state"]["capture"]["app"], "Hex");
        assert_eq!(asked["state"]["capture"]["mode"], "Code");
    }

    #[test]
    fn hexs_screen_fields_reach_jev_as_screen() {
        let out = run(
            json!({"item": {"text": "make it bigger", "app": "Editor",
                            "windows": [{"app": "Editor", "title": "notes.txt"}, {"app": "Chat"}],
                            "focused": {"role": "AXTextArea", "value_excerpt": "the margin is"},
                            "visible_text": "v".repeat(5000),
                            "opencode_tab": "HEX",
                            "recent": [{"text_excerpt": "rename it", "routed_to": "ses_a"}],
                            "future_field": {"x": 1}}}),
            sure,
        );
        assert_eq!(out["decision"], "send", "{out}");
        let asked = ASKED.with(|a| a.borrow().clone()).unwrap();
        let screen = &asked["state"]["screen"];
        assert_eq!(screen["windows"][1]["app"], "Chat");
        assert_eq!(screen["focused"]["value_excerpt"], "the margin is");
        assert!(screen["visible_text"].as_str().unwrap().chars().count() <= 1501);
        assert_eq!(screen["opencode_tab"], "HEX");
        // Targets newest first: t1 HEX, t2 A.
        assert_eq!(screen["recent"][0]["routed_to"], "targets.t2");
        assert!(screen.get("future_field").is_none());
        let hints = asked["state"]["hints"].as_array().unwrap();
        assert!(
            hints
                .iter()
                .any(|h| h.as_str().unwrap().contains("`targets.t1`"))
        );
    }

    #[test]
    fn a_session_named_at_the_start_is_sent_to_without_jev() {
        ASKED.with(|a| *a.borrow_mut() = None);
        let out = run(
            json!({"item": {"text": "For home: file the hex icon bug", "app": "Ghostty"}}),
            sure,
        );
        assert_eq!(out["decision"], "send", "{out}");
        assert_eq!(out["target"]["id"], "ses_home");
        assert_eq!(out["why"], "named at the start");
        assert!(out["model"].is_null());
        assert!(ASKED.with(|a| a.borrow().is_none()), "Jev wasn't asked");
        // A name later on is only a clue for Jev.
        let out = run(json!({"item": {"text": "file the home bug"}}), sure);
        assert_eq!(out["target"]["id"], "ses_hex");
        assert_eq!(out["why"], "jev: confident");
    }

    #[test]
    fn an_unsure_answer_asks_with_jevs_top_two_and_home() {
        let out = run(json!({"item": {"transcript": "rename it"}}), unsure);
        assert_eq!(out["decision"], "ask", "{out}");
        assert_eq!(out["target"]["id"], "ses_b");
        let ids: Vec<&str> = out["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, ["ses_b", "ses_hex", "ses_home"]);
        assert_eq!(out["candidates"][2]["score"], 0.04);
        let from: Vec<&str> = out["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| c["from"].as_str().unwrap())
            .collect();
        assert_eq!(from, ["jev", "jev", "home"]);
    }

    #[test]
    fn pinned_sessions_are_put_to_jev_but_dont_win() {
        // ses_old isn't among the recent sessions; pinning reads it anyway.
        let out = run(json!({"item": {"text": "x"}, "pinned": ["ses_old"]}), sure);
        let asked = ASKED.with(|a| a.borrow().clone()).unwrap();
        let titles: Vec<&str> = asked["state"]["targets"]
            .as_object()
            .unwrap()
            .values()
            .map(|t| t["title"].as_str().unwrap())
            .collect();
        assert!(titles.contains(&"OLD"), "{titles:?}");
        assert_eq!(out["target"]["id"], "ses_hex", "Jev still decides");
    }

    #[test]
    fn running_out_of_time_keeps_it() {
        let out = run(json!({"item": {"text": "x"}}), slow);
        assert_eq!(out["decision"], "keep", "{out}");
        assert_eq!(out["why"], "timeout");
        assert!(out["target"].is_null());
    }

    #[test]
    fn the_text_is_required_and_the_app_can_be_a_bundle_id() {
        assert!(serde_json::from_value::<Input>(json!({"item": {"app": "Hex"}})).is_err());
        let input: Input =
            serde_json::from_value(json!({"item": {"text": "x", "app": "com.mitchellh.ghostty",
                "window_title": "zsh"}, "pinned": "ses_a"}))
            .unwrap();
        assert_eq!(input.item.app_name().as_deref(), Some("ghostty"));
        assert_eq!(input.item.title.as_deref(), Some("zsh"));
        assert_eq!(input.pinned, ["ses_a"]);
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("scrawl-router-{name}-{}", crate::make_bundle_id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn saved_dictation(dir: &Path) -> Saved {
        let request = json!({"item": {"kind": "dictation", "item_id": "A1B2-C3", "text": "make the icon bigger",
            "app": "Ghostty", "selected_text": "icon.png"}});
        let input: Input = serde_json::from_value(request.clone()).unwrap();
        let hooks = jev::Hooks {
            ask: unsure,
            recent,
            deadline: None,
        };
        let (mut out, _, _) =
            decide_with(&input, &senses(), &hooks, &Thresholds::default(), 100_000);
        out.decision_id = "abc123".into();
        save(dir, "abc123", &request, &out, 1).unwrap();
        load(dir, "abc123").unwrap()
    }

    fn sends(dir: &Path, session: &str) -> Result<()> {
        let prompt: Value = serde_json::from_slice(&std::fs::read(dir.join(delivery::PROMPT))?)?;
        SENT.with(|s| {
            s.borrow_mut()
                .push((session.into(), prompt["text"].as_str().unwrap().into()))
        });
        Ok(())
    }
    fn fails(_: &Path, _: &str) -> Result<()> {
        bail!("OpenCode is down")
    }
    fn no_bundle(_: &Path, _: &str) -> Result<()> {
        panic!("not a capture")
    }
    fn last_sent() -> (String, String) {
        SENT.with(|s| s.borrow().last().cloned().unwrap())
    }

    #[test]
    fn deliver_sends_the_text_tagged_with_its_app() {
        let dir = temp_dir("deliver");
        let decisions = dir.join("decisions");
        saved_dictation(&decisions);
        let outbox = dir.join("outbox");
        let ok = DeliverHooks {
            text: sends,
            bundle: no_bundle,
        };
        let by_decision = |to: &str, hooks: &DeliverHooks| {
            let request = DeliverRequest {
                decision: Some("abc123".into()),
                to: to.into(),
                ..DeliverRequest::default()
            };
            deliver_with(&decisions, request, &outbox, hooks)
        };
        let out = by_decision("ses_hex", &ok).unwrap();
        assert_eq!(
            out,
            Delivered {
                ok: true,
                state: "sent",
                session: "ses_hex".into()
            }
        );
        let (session, text) = last_sent();
        assert_eq!(session, "ses_hex");
        assert!(
            text.starts_with("[voice via Hex · from Ghostty]\nmake the icon bigger"),
            "{text}"
        );
        assert!(text.ends_with("Selected text:\n> icon.png"), "{text}");
        assert!(
            !outbox.join("abc123").exists(),
            "sent prompts leave the outbox"
        );

        // Hex's spelling: the item ID and the text on stdin, which wins.
        let request = DeliverRequest {
            item_id: Some("A1B2-C3".into()),
            to: "ses_b".into(),
            text: Some("make it smaller\n".into()),
            ..DeliverRequest::default()
        };
        deliver_with(&decisions, request, &outbox, &ok).unwrap();
        assert_eq!(
            last_sent(),
            (
                "ses_b".into(),
                "[voice via Hex · from Ghostty]\nmake it smaller".into()
            )
        );

        // An item Scrawl never decided still goes, without the "from".
        let request = DeliverRequest {
            item_id: Some("zz-9".into()),
            to: "ses_b".into(),
            text: Some("hello".into()),
            ..DeliverRequest::default()
        };
        deliver_with(&decisions, request, &outbox, &ok).unwrap();
        assert_eq!(last_sent().1, "[voice via Hex]\nhello");

        // Text that already carries the prefix isn't given a second one.
        let request = DeliverRequest {
            item_id: Some("zz-10".into()),
            to: "ses_b".into(),
            text: Some("[voice via Hex · from Zed]\nhello".into()),
            ..DeliverRequest::default()
        };
        deliver_with(&decisions, request, &outbox, &ok).unwrap();
        assert_eq!(last_sent().1, "[voice via Hex · from Zed]\nhello");

        // A failed send waits in the outbox for the app's retries.
        let down = DeliverHooks {
            text: fails,
            bundle: no_bundle,
        };
        assert_eq!(by_decision("ses_b", &down).unwrap().state, "queued");
        let pending = outbox.join("abc123");
        assert!(
            pending.join(delivery::PENDING).is_file() && pending.join(delivery::PROMPT).is_file()
        );
        let report = delivery::retry_due(&outbox, u64::MAX / 2, sends);
        assert_eq!(report.sent.len(), 1, "the retry loop picks prompts up");
        assert!(by_decision("not a session", &down).is_err());
        let nothing = DeliverRequest {
            item_id: Some("zz-9".into()),
            to: "ses_b".into(),
            ..DeliverRequest::default()
        };
        assert!(
            deliver_with(&decisions, nothing, &outbox, &ok).is_err(),
            "no decision and no text"
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn corrections_go_from_the_decisions_target() {
        let dir = temp_dir("correct");
        let saved = saved_dictation(&dir);
        let line = correction(&saved, "abc123", Action::Redirect, Some("ses_home"), 7).unwrap();
        assert_eq!(line["event"], "correction");
        assert_eq!(line["kind"], "dictation");
        assert_eq!(line["decision_id"], "abc123");
        assert_eq!(line["item_id"], "A1B2-C3");
        assert_eq!(line["action"], "redirect");
        assert_eq!(line["from"]["id"], "ses_b");
        assert_eq!(line["to"], json!({"id": "ses_home", "title": "HOME"}));
        assert_eq!(line["app"], "Ghostty");
        jev::check(&line).unwrap();
        let bad = correction(&saved, "abc123", Action::Redirect, None, 7).unwrap();
        assert!(jev::check(&bad).is_err(), "a redirect needs to");
        assert!(load(&dir, "../etc").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn logged_lines_join_their_decision_by_item_id() {
        let dir = temp_dir("log");
        saved_dictation(&dir);
        let line = json!({"kind": "dictation", "event": "correction", "item_id": "A1B2-C3",
                          "action": "redirect", "to": "ses_home"});
        let line = complete(line, Some(&dir), 9).unwrap();
        assert_eq!(line["time"], 9);
        assert_eq!(line["decision_id"], "abc123");
        assert_eq!(line["from"], json!({"id": "ses_b", "title": "B"}));
        assert_eq!(line["to"], json!({"id": "ses_home", "title": "HOME"}));
        assert_eq!(line["app"], "Ghostty");
        jev::check(&line).unwrap();
        for bad in [
            json!({"kind": "dictation", "event": "correction", "item_id": "x", "action": "undo"}),
            json!({"kind": "email", "event": "correction", "item_id": "x", "action": "cancel"}),
            json!({"kind": "dictation", "event": "correction", "action": "cancel"}),
            json!({"kind": "dictation", "event": "correction", "item_id": "x", "action": "cancel", "to": 3}),
        ] {
            assert!(
                jev::check(&complete(bad.clone(), Some(&dir), 9).unwrap()).is_err(),
                "{bad}"
            );
        }
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn eval_items_are_read_by_line_and_judged_by_expectation() {
        let dir = temp_dir("eval");
        std::fs::write(
            dir.join("set.jsonl"),
            "{\"item\": {\"text\": \"a\"}, \"expect\": {\"to\": \"ses_hex\"}}\n\nnot json\n\
             {\"item\": {\"text\": \"b\"}, \"expect\": {\"decision\": \"fallback\"}}\n",
        )
        .unwrap();
        let cases = eval_cases(&dir);
        let names: Vec<&str> = cases.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["set:1", "set:3", "set:4"]);
        assert!(cases[1].1.is_err());
        let first = cases[0].1.as_ref().unwrap();
        let input = &first.input;
        let hooks = jev::Hooks {
            ask: sure,
            recent,
            deadline: None,
        };
        let (out, _, _) = decide_with(input, &senses(), &hooks, &Thresholds::default(), 100_000);
        assert!(meets(&out, &first.expect));
        assert!(!meets(&out, &cases[2].1.as_ref().unwrap().expect));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn the_sweep_counts_what_each_gate_blocks() {
        let sample = |kind, confidence, margin, fits, right| Sample {
            kind,
            confidence,
            margin,
            fits,
            right,
        };
        let samples = [
            sample("capture", 0.95, 0.9, 0.5, true),
            sample("dictation", 0.9, 0.2, 0.6, false),
            sample("dictation", 0.9, 0.5, 0.3, true),
        ];
        let out = sweep(&samples, &Thresholds::default());
        // The margin gate at 0.3 blocks the wrong one only.
        let row = |out: &str, gate: &str, value: &str| {
            let section = &out[out.find(gate).unwrap()..];
            let line = section
                .lines()
                .find(|l| l.split_whitespace().next() == Some(value))
                .unwrap();
            line.split_whitespace()
                .skip(1)
                .take(4)
                .collect::<Vec<_>>()
                .join(" ")
        };
        assert_eq!(row(&out, "margin (set", "0.30"), "2 0 0 1");
        // any_fits at 0.4 would have demoted the right dictation send, and stopped nothing.
        let out = demotions(&samples, &Thresholds::default());
        let line = out
            .lines()
            .find(|l| l.trim_start().starts_with("0.40"))
            .unwrap();
        assert_eq!(
            line.split_whitespace().collect::<Vec<_>>(),
            ["0.40", "0/1", "(0%)", "1/1", "(100%)", "0/0"]
        );
    }

    #[test]
    fn old_decisions_are_pruned() {
        let dir = temp_dir("prune");
        std::fs::write(dir.join("a.json"), "{}").unwrap();
        prune(&dir, Duration::from_secs(3600));
        assert!(dir.join("a.json").exists());
        prune(&dir, Duration::ZERO);
        std::thread::sleep(Duration::from_millis(5));
        prune(&dir, Duration::ZERO);
        assert!(!dir.join("a.json").exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn since_reads_days_hours_and_minutes() {
        assert_eq!(parse_since("7d").unwrap(), Duration::from_secs(7 * 86_400));
        assert_eq!(parse_since("36h").unwrap(), Duration::from_secs(36 * 3_600));
        assert!(parse_since("soon").is_err());
    }

    #[test]
    fn landing_counts_the_chips_an_offer_shows() {
        let scored = |id: &str| smart::Scored {
            candidate: Candidate {
                id: id.into(),
                title: id.into(),
                directory: None,
                repo: None,
                updated_ms: 0,
                viewed_ms: None,
            },
            score: 50,
            reasons: Vec::new(),
        };
        let offer = Decision::Offer {
            choices: vec![scored("a"), scored("b"), scored("c")],
        };
        assert_eq!(landed(&offer, "b", 2), (Some("a".into()), true));
        assert_eq!(landed(&offer, "c", 2), (Some("a".into()), false));
    }
}
