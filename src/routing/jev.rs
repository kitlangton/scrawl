//! Jev routing (`"experiments": {"jevRouting": true}`, on top of smart routing): instead of
//! weighing words, Scrawl asks Jev, TypeSafe's System One model behind OpenCode's gateway, which
//! of the recent open root sessions a capture is about (`docs/routing.md`).
//!
//! Deterministic sources still decide first: an address, a `routing.sites` rule, a page's
//! `/sdk.js` session, a publisher marker and a localhost port ([`settled`]). Jev replaces only the
//! weighing of titles, speech, the app, the viewed session and the terminal, which reach it as
//! sentences in `hints`. Every question, the pinned model and the thresholds live here.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::smart::{
    self, Basis, Candidate, Decision, Evidence, Judgement, Pointer, Reason, Scored, Source,
};
use crate::capture::scrawl_log;
use crate::model::{InteractionEvent, ScrawlBundle};

/// The experiment that asks Jev (only read when `smartRouting` is on too).
pub const JEV_ROUTING: &str = "jevRouting";

/// Pinned: thresholds tuned on one version can shift on the next.
pub const MODEL: &str = "jev-1.13";
const ENDPOINT: &str = "https://opencode.ai/inference/systemone/v1/systemone";
/// Cloudflare answers 403 (error 1010) to requests without a user agent.
const USER_AGENT: &str = concat!("scrawl/", env!("CARGO_PKG_VERSION"));
/// Dollars per million input tokens (`jev-1.13`).
const PRICE_PER_M: f64 = 0.042;
/// The whole HTTPS round trip, connecting included.
const TIMEOUT: Duration = Duration::from_secs(3);
/// Not worth asking with less time than this left before a deadline.
const MIN_ASK: Duration = Duration::from_millis(400);

/// Send when the target's confidence and its lead over the runner-up both clear these. Jev's
/// target choice is well calibrated; its yes/no answers aren't (real positives land between
/// 0.3 and 0.65), so `any_fits` never blocks a confident choice.
pub const SEND_AT: f64 = 0.85;
pub const MARGIN_AT: f64 = 0.3;
/// Below this confidence Jev doesn't know: the item goes to HOME (or stays on the clipboard).
/// Between it and a send it's offered, unless `any_fits` is under [`NO_FIT_AT`], which picks
/// HOME over the weak choice.
pub const OFFER_AT: f64 = 0.5;
pub const NO_FIT_AT: f64 = 0.2;
/// The `any_fits` gate on sends before 2026-10-24, for `scrawl router eval`'s demotion count.
pub const OLD_FITS_AT: f64 = 0.4;
/// A runner-up worth a chip beside "Sending to …".
const RUNNER_UP_AT: f64 = 0.1;

/// At most this many sessions are put to Jev, newest first.
const MAX_TARGETS: usize = 20;
/// The latest earlier items passed as `history`, each trimmed.
const HISTORY: usize = 10;
const TRANSCRIPT_CHARS: usize = 2000;
const RECENT_CHARS: usize = 300;
const NOTES: usize = 12;
/// How long reading the targets' last messages may take, all together.
pub const RECENT_BUDGET: Duration = Duration::from_millis(1500);

/// The open root session things go to when Jev can't tell.
const HOME_TITLE: &str = "HOME";

pub fn enabled() -> bool {
    crate::handoff::experiment(super::SMART_ROUTING) && crate::handoff::experiment(JEV_ROUTING)
}

// MARK: Thresholds

/// What happens when Jev doesn't know.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Low {
    /// Send to HOME, with the same undo window.
    Home,
    /// Clipboard only.
    Keep,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Thresholds {
    pub send: f64,
    pub margin: f64,
    pub offer: f64,
    pub no_fit: f64,
    pub low: Low,
}

impl Default for Thresholds {
    fn default() -> Self {
        Self {
            send: SEND_AT,
            margin: MARGIN_AT,
            offer: OFFER_AT,
            no_fit: NO_FIT_AT,
            low: Low::Home,
        }
    }
}

impl Thresholds {
    /// `"jevRouting": {"send": 0.85, "margin": 0.3, "offer": 0.5, "noFit": 0.2, "low": "home"}`
    /// in `config.json`; anything missing or out of range keeps its default.
    pub fn from_config(value: Option<&Value>) -> Self {
        let mut t = Self::default();
        let Some(value) = value else {
            return t;
        };
        let num = |key: &str| {
            value
                .get(key)
                .and_then(Value::as_f64)
                .filter(|v| (0.0..=1.0).contains(v))
        };
        t.send = num("send").unwrap_or(t.send);
        t.margin = num("margin").unwrap_or(t.margin);
        t.offer = num("offer").unwrap_or(t.offer);
        t.no_fit = num("noFit").unwrap_or(t.no_fit);
        if value.get("low").and_then(Value::as_str) == Some("keep") {
            t.low = Low::Keep;
        }
        t
    }

    pub fn configured() -> Self {
        Self::from_config(crate::handoff::jev_routing_config().as_ref())
    }

    pub fn band(&self, answer: &Answer) -> Band {
        self.band_of(answer.confidence, answer.margin(), answer.fits)
    }

    /// The band for a confidence, its margin over the runner-up and `any_fits`.
    pub fn band_of(&self, confidence: f64, margin: f64, fits: f64) -> Band {
        if confidence >= self.send && margin >= self.margin {
            Band::Send
        } else if confidence < self.offer || fits < self.no_fit {
            match self.low {
                Low::Home => Band::Fallback,
                Low::Keep => Band::Keep,
            }
        } else {
            Band::Ask
        }
    }
}

/// The decision, in the words `routing.json`, the decisions log and `scrawl router decide` share.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Band {
    /// Confident: sent after the undo countdown.
    Send,
    /// Offered on chips.
    Ask,
    /// Jev couldn't tell: sent to HOME after the countdown.
    Fallback,
    /// Clipboard only.
    #[default]
    Keep,
}

impl Band {
    pub const ALL: [Self; 4] = [Self::Send, Self::Ask, Self::Fallback, Self::Keep];

    pub fn label(self) -> &'static str {
        match self {
            Self::Send => "send",
            Self::Ask => "ask",
            Self::Fallback => "fallback",
            Self::Keep => "keep",
        }
    }

    /// Smart routing's own decision in these words.
    pub fn of(decision: &Decision) -> Self {
        match decision {
            Decision::Send { .. } => Self::Send,
            Decision::Offer { .. } => Self::Ask,
            Decision::Keep { .. } => Self::Keep,
        }
    }
}

// MARK: Hooks

/// The outside steps, swappable in tests, and the time they may take.
#[derive(Clone, Copy)]
pub struct Hooks {
    /// POSTs a request body to Jev within a time and returns the response body.
    pub ask: fn(&[u8], Duration) -> Result<Vec<u8>>,
    /// The last user message and assistant text of each session within a time: from OpenCode's
    /// API now (`None`), or from its history as it was before a time (a replay).
    pub recent: fn(&[String], Option<u64>, Duration) -> Vec<Recent>,
    /// When the answer is due (`scrawl router decide`); reading and asking share what's left.
    pub deadline: Option<Instant>,
}

impl Default for Hooks {
    fn default() -> Self {
        Self {
            ask: gateway,
            recent: read_recent,
            deadline: None,
        }
    }
}

impl Hooks {
    fn left(&self) -> Option<Duration> {
        self.deadline
            .map(|d| d.saturating_duration_since(Instant::now()))
    }

    /// The time reading recent lines may take: at most a third of what's left.
    fn recent_budget(&self) -> Duration {
        self.left()
            .map_or(RECENT_BUDGET, |left| RECENT_BUDGET.min(left / 3))
    }

    /// The time asking Jev may take, or `None` when too little is left.
    fn ask_budget(&self) -> Option<Duration> {
        match self.left() {
            None => Some(TIMEOUT),
            Some(left) => {
                let left = left.saturating_sub(Duration::from_millis(50));
                (left >= MIN_ASK).then(|| TIMEOUT.min(left))
            }
        }
    }
}

/// Jev's error when the deadline came first; `scrawl router decide` answers `keep` for it.
pub const TIMED_OUT: &str = "timeout";

// MARK: Deciding

/// Whether smart routing's decision rests on a source that's obeyed rather than weighed.
pub fn settled(judgement: &Judgement) -> bool {
    let Decision::Send { to, .. } = &judgement.decision else {
        return false;
    };
    matches!(
        to.lead().map(|r| r.basis),
        Some(
            Basis::Chosen
                | Basis::Recorded
                | Basis::Clue(
                    Source::Named | Source::Rule | Source::Page | Source::Port | Source::Publisher
                )
        )
    )
}

/// What Jev was asked and said, saved in `routing.json` as `jev`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    /// `"jev"`, or `"smart"` when smart routing's own decision stands.
    pub by: String,
    #[serde(rename = "decision")]
    pub band: Band,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<Pick>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    #[serde(default, rename = "anyFits", skip_serializing_if = "Option::is_none")]
    pub fits: Option<f64>,
    /// Every target Jev gave at least 1%, likeliest first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub probabilities: Vec<Pick>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pick {
    /// The target's ID in the question (`t3`).
    pub t: String,
    pub session: String,
    pub title: String,
    pub p: f64,
}

impl Record {
    fn smart(judgement: &Judgement, reason: impl Into<String>) -> Self {
        Self {
            by: "smart".into(),
            band: Band::of(&judgement.decision),
            reason: reason.into(),
            ..Self::default()
        }
    }
}

/// An earlier item and where it went (`scrawl router decide`'s `history`).
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
pub struct Past {
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub to: Option<String>,
    /// Where Kit moved it instead.
    #[serde(default)]
    pub corrected_to: Option<String>,
}

/// What a caller adds to the question: earlier items, and sessions to always put to Jev (pinned
/// in Hex; they don't win on their own).
#[derive(Clone, Debug, Default)]
pub struct Asking {
    pub history: Vec<Past>,
    pub pinned: Vec<String>,
}

/// Jev's decision for `evidence` in place of smart routing's `smart`, unless that rests on a
/// settled source or Jev can't be asked (then `smart` stands, and the record says why).
/// `capture` is the state's `capture`; `before_ms` reads the sessions' last messages as they were
/// then (a replay) instead of now.
pub fn judge(
    evidence: &Evidence,
    smart: Judgement,
    capture: &Value,
    asking: &Asking,
    before_ms: Option<u64>,
    hooks: &Hooks,
    thresholds: &Thresholds,
) -> (Judgement, Record) {
    if settled(&smart) {
        let via = match &smart.decision {
            Decision::Send { to, .. } => smart::destination_of(to, None).via,
            _ => String::new(),
        };
        return (
            smart.clone(),
            Record::smart(&smart, format!("settled by {via}; Jev not asked")),
        );
    }
    let targets = pick_targets(evidence, &smart, asking);
    if targets.is_empty() {
        return (
            smart.clone(),
            Record::smart(&smart, "no open sessions to ask Jev about"),
        );
    }
    let ids: Vec<String> = targets.iter().map(|c| c.id.clone()).collect();
    let recent = (hooks.recent)(&ids, before_ms, hooks.recent_budget());
    let hints = hints(evidence, &targets);
    let history = history_state(&asking.history, &targets, &evidence.candidates);
    let body = request_body(
        capture,
        &hints,
        &history,
        &targets,
        &recent,
        evidence.now_ms,
    );
    let started = Instant::now();
    let answer = match hooks.ask_budget() {
        Some(budget) => (hooks.ask)(&body, budget).and_then(|reply| parse_answer(&reply)),
        None => Err(anyhow::anyhow!("{TIMED_OUT}: no time left to ask Jev")),
    };
    let ms = started.elapsed().as_millis() as u64;
    let mut answer = match answer {
        Ok(answer) => answer,
        Err(err) => {
            let error = format!("{err:#}");
            scrawl_log(format!(
                "routing (jev): failed after {ms} ms, smart routing decides: {error}"
            ));
            let mut record = Record::smart(&smart, "Jev failed; smart routing's decision stands");
            record.ms = Some(ms);
            record.error = Some(error);
            return (smart, record);
        }
    };
    answer.ms = ms;
    let band = thresholds.band(&answer);
    let home = home_session(&evidence.candidates);
    let (decision, reason) = decision_for(band, &answer, &targets, home.as_ref());
    let pick = |t: &str, p: f64| {
        index_of(t).and_then(|i| targets.get(i)).map(|c| Pick {
            t: t.to_owned(),
            session: c.id.clone(),
            title: c.label().to_owned(),
            p: round3(p),
        })
    };
    let record = Record {
        by: "jev".into(),
        band,
        reason,
        model: Some(answer.model.clone()),
        ms: Some(ms),
        target: pick(&answer.target, answer.p(&answer.target)),
        confidence: Some(round3(answer.confidence)),
        fits: Some(round3(answer.fits)),
        probabilities: answer
            .ranked()
            .into_iter()
            .filter(|(_, p)| *p >= 0.01)
            .filter_map(|(t, p)| pick(&t, p))
            .collect(),
        input_tokens: answer.input_tokens,
        error: None,
    };
    (
        Judgement {
            decision,
            ranking: smart.ranking,
        },
        record,
    )
}

fn round3(v: f64) -> f64 {
    (v * 1000.0).round() / 1000.0
}

/// The sessions put to Jev: those pinned, named by a clue, ranked by smart routing or that
/// earlier items went to, HOME, then the newest, up to [`MAX_TARGETS`], newest first.
pub fn pick_targets(evidence: &Evidence, smart: &Judgement, asking: &Asking) -> Vec<Candidate> {
    let mut named: Vec<&str> = asking.pinned.iter().map(String::as_str).collect();
    named.extend(
        evidence
            .clues
            .iter()
            .filter_map(|clue| match &clue.pointer {
                Pointer::Session { id, .. } => Some(id.as_str()),
                _ => None,
            }),
    );
    named.extend(
        smart
            .ranking
            .iter()
            .take(3)
            .map(|s| s.candidate.id.as_str()),
    );
    for past in asking.history.iter().rev().take(HISTORY) {
        named.extend(past.corrected_to.as_deref());
        named.extend(past.to.as_deref());
    }
    let mut picked: Vec<Candidate> = Vec::new();
    let mut add = |c: &Candidate| {
        if picked.len() < MAX_TARGETS && !picked.iter().any(|p| p.id == c.id) {
            picked.push(c.clone());
        }
    };
    for id in named {
        if let Some(c) = evidence.candidates.iter().find(|c| c.id == id) {
            add(c);
        }
    }
    if let Some(home) = home_session(&evidence.candidates) {
        add(&home);
    }
    let mut newest: Vec<&Candidate> = evidence.candidates.iter().collect();
    newest.sort_by(|a, b| b.updated_ms.cmp(&a.updated_ms).then(a.id.cmp(&b.id)));
    for c in newest {
        add(c);
    }
    picked.sort_by(|a, b| b.updated_ms.cmp(&a.updated_ms).then(a.id.cmp(&b.id)));
    picked
}

/// The open root session titled HOME, the newest if there are several.
pub fn home_session(candidates: &[Candidate]) -> Option<Candidate> {
    candidates
        .iter()
        .filter(|c| c.title.trim().eq_ignore_ascii_case(HOME_TITLE))
        .max_by_key(|c| c.updated_ms)
        .cloned()
}

fn scored(candidate: &Candidate, basis: Basis, p: f64, why: String) -> Scored {
    let points = (p * 100.0).round() as u32;
    Scored {
        candidate: candidate.clone(),
        score: points,
        reasons: vec![Reason {
            basis,
            points,
            why,
            directory: None,
        }],
    }
}

/// The decision a band makes, and why, in words.
fn decision_for(
    band: Band,
    answer: &Answer,
    targets: &[Candidate],
    home: Option<&Candidate>,
) -> (Decision, String) {
    let ranked: Vec<(&Candidate, f64)> = answer
        .ranked()
        .into_iter()
        .filter_map(|(t, p)| Some((targets.get(index_of(&t)?)?, p)))
        .collect();
    let numbers = format!(
        "confidence {:.2}, fits {:.2}",
        answer.confidence, answer.fits
    );
    let Some(&(best, best_p)) = ranked.first() else {
        return (
            Decision::Keep {
                reason: "Jev named no target".into(),
            },
            "Jev named no target".into(),
        );
    };
    let jev = |c: &Candidate, p: f64| scored(c, Basis::Jev, p, format!("Jev gave it {p:.2}"));
    match band {
        Band::Send => {
            let instead = ranked
                .get(1)
                .filter(|(_, p)| *p >= RUNNER_UP_AT)
                .map(|&(c, p)| vec![jev(c, p)])
                .unwrap_or_default();
            (
                Decision::Send {
                    to: jev(best, best_p),
                    instead,
                },
                format!("Jev is sure it's {} ({numbers})", best.label()),
            )
        }
        Band::Ask => {
            // Two chips fit beside Paste: the best, then HOME when it isn't the best, so Kit can
            // always send it there; the runners-up follow for the record.
            let mut choices = vec![jev(best, best_p)];
            if let Some(home) = home.filter(|h| h.id != best.id) {
                choices.push(scored(home, Basis::JevHome, 0.0, "HOME".into()));
            }
            for &(c, p) in ranked.iter().skip(1).take(2) {
                if !choices.iter().any(|s| s.candidate.id == c.id) {
                    choices.push(jev(c, p));
                }
            }
            (
                Decision::Offer { choices },
                format!("Jev leans to {} but isn't sure ({numbers})", best.label()),
            )
        }
        Band::Fallback => match home {
            Some(home) => {
                let instead = (best.id != home.id)
                    .then(|| vec![jev(best, best_p)])
                    .unwrap_or_default();
                (
                    Decision::Send {
                        to: scored(
                            home,
                            Basis::JevHome,
                            0.0,
                            "Jev couldn't tell, so HOME".into(),
                        ),
                        instead,
                    },
                    format!(
                        "Jev couldn't tell (best {}, {numbers}), so HOME",
                        best.label()
                    ),
                )
            }
            None => {
                let reason = format!(
                    "Jev couldn't tell (best {}, {numbers}) and no session is titled HOME",
                    best.label()
                );
                (
                    Decision::Keep {
                        reason: reason.clone(),
                    },
                    reason,
                )
            }
        },
        Band::Keep => {
            let reason = format!("Jev couldn't tell (best {}, {numbers})", best.label());
            (
                Decision::Keep {
                    reason: reason.clone(),
                },
                reason,
            )
        }
    }
}

// MARK: The request

/// The last user message and the last assistant text of a session, each trimmed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Recent {
    pub user: Option<String>,
    pub assistant: Option<String>,
}

/// `capture` in the state for a finished bundle: what was said (start and end when long), what
/// was marked or clicked, and where.
pub fn capture_state(bundle: &ScrawlBundle, url: Option<&str>) -> Value {
    let target = &bundle.target;
    let mut notes = Vec::new();
    for mark in &bundle.marks {
        let shape = serde_json::to_value(mark.shape)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "mark".into());
        notes.push((
            mark.t0,
            format!(
                "{} {shape}{}",
                clock(mark.t0),
                mark.on
                    .as_deref()
                    .map(|on| format!(" on {on}"))
                    .unwrap_or_default()
            ),
        ));
    }
    for event in &bundle.events {
        let note = match event {
            InteractionEvent::Click { t, label, role, .. } => Some((
                *t,
                format!(
                    "{} clicked {}{}",
                    clock(*t),
                    label.as_deref().unwrap_or("something"),
                    role.as_deref()
                        .map(|r| format!(" ({r})"))
                        .unwrap_or_default()
                ),
            )),
            InteractionEvent::AppChange {
                t,
                app,
                window_title,
            } => Some((
                *t,
                format!(
                    "{} switched to {app}{}",
                    clock(*t),
                    window_title
                        .as_deref()
                        .map(|w| format!(" \u{201c}{w}\u{201d}"))
                        .unwrap_or_default()
                ),
            )),
            InteractionEvent::AppEvent { t, label } => Some((*t, format!("{} {label}", clock(*t)))),
            _ => None,
        };
        notes.extend(note);
    }
    notes.sort_by_key(|(t, _)| *t);
    let notes: Vec<String> = notes.into_iter().take(NOTES).map(|(_, n)| n).collect();
    let mut capture = serde_json::Map::new();
    capture.insert("kind".into(), json!("capture"));
    capture.insert(
        "transcript".into(),
        json!(trim_middle(&bundle.transcript.text, TRANSCRIPT_CHARS)),
    );
    if let Some(note) = bundle.note.as_deref().filter(|n| !n.trim().is_empty()) {
        capture.insert("note".into(), json!(note));
    }
    if !notes.is_empty() {
        capture.insert("marks_and_clicks".into(), json!(notes));
    }
    if let Some(app) = &target.app {
        capture.insert("app".into(), json!(app));
    }
    if let Some(title) = &target.window_title {
        capture.insert("window_title".into(), json!(title));
    }
    if let Some(url) = url {
        capture.insert("url".into(), json!(url));
    }
    capture.insert("mode".into(), json!(target.mode));
    capture.insert("duration".into(), json!(clock(bundle.duration_ms)));
    Value::Object(capture)
}

fn clock(ms: u64) -> String {
    format!("{}:{:02}", ms / 60_000, ms / 1000 % 60)
}

/// `text` cut to about `max` characters, keeping its start and its end.
pub fn trim_middle(text: &str, max: usize) -> String {
    let text = text.trim();
    let count = text.chars().count();
    if count <= max {
        return text.to_owned();
    }
    let head = max * 3 / 5;
    let tail = max - head;
    let start: String = text.chars().take(head).collect();
    let end: String = text.chars().skip(count - tail).collect();
    format!("{start} … {end}")
}

fn trim_end(text: &str, max: usize) -> String {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() <= max {
        return flat;
    }
    let cut: String = flat.chars().take(max).collect();
    format!("{cut}…")
}

/// The clues besides what was said, as sentences, with the targets they point at.
pub fn hints(evidence: &Evidence, targets: &[Candidate]) -> Vec<String> {
    let t_of = |id: &str| {
        targets
            .iter()
            .position(|c| c.id == id)
            .map(|i| format!("targets.t{}", i + 1))
    };
    let mut hints = Vec::new();
    for clue in &evidence.clues {
        if clue.source == Source::Speech {
            continue;
        }
        let mut hint = sentence(&tilde(&clue.note));
        match &clue.pointer {
            Pointer::Session { id, .. } => {
                if let Some(t) = t_of(id) {
                    hint = format!("{hint} That session is `{t}`.");
                }
            }
            Pointer::Folder(folder) => {
                let inside: Vec<String> = targets
                    .iter()
                    .enumerate()
                    .filter(|(_, c)| {
                        let dir = c.directory.as_deref();
                        let repo = c.repo.as_deref();
                        dir.is_some_and(|d| d.starts_with(&folder.root))
                            || (folder.repo.is_some() && repo == folder.repo.as_deref())
                    })
                    .map(|(i, _)| format!("`targets.t{}`", i + 1))
                    .collect();
                if !inside.is_empty() && !smart::is_home_or_root(&folder.root) {
                    hint = format!("{hint} Sessions in that project: {}.", inside.join(", "));
                }
            }
            Pointer::Words { .. } => {}
        }
        hints.push(hint);
    }
    hints
}

fn sentence(note: &str) -> String {
    let mut chars = note.trim().chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut s: String = first.to_uppercase().chain(chars).collect();
    if !s.ends_with(['.', '!', '?']) {
        s.push('.');
    }
    s
}

fn tilde(text: &str) -> String {
    match dirs::home_dir() {
        Some(home) => text.replace(&home.display().to_string(), "~"),
        None => text.to_owned(),
    }
}

/// `history` in the state, latest last, each session named by its target when it's one.
fn history_state(history: &[Past], targets: &[Candidate], candidates: &[Candidate]) -> Vec<Value> {
    let name = |id: &str| {
        if let Some(i) = targets.iter().position(|c| c.id == id) {
            return format!("targets.t{} ({})", i + 1, targets[i].label());
        }
        candidates
            .iter()
            .find(|c| c.id == id)
            .map_or_else(|| id.to_owned(), |c| c.label().to_owned())
    };
    let skip = history.len().saturating_sub(HISTORY);
    history
        .iter()
        .skip(skip)
        .map(|past| {
            let mut entry = serde_json::Map::new();
            entry.insert("text".into(), json!(trim_end(&past.text, RECENT_CHARS)));
            if let Some(to) = &past.to {
                entry.insert("went_to".into(), json!(name(to)));
            }
            if let Some(to) = &past.corrected_to {
                entry.insert("corrected_to".into(), json!(name(to)));
            }
            Value::Object(entry)
        })
        .collect()
}

/// Serializes its pairs in order (`serde_json::Map` would sort `t10` before `t2`).
struct Ordered<'a>(&'a [(String, Value)]);

impl Serialize for Ordered<'_> {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap as _;
        let mut map = s.serialize_map(Some(self.0.len()))?;
        for (k, v) in self.0 {
            map.serialize_entry(k, v)?;
        }
        map.end()
    }
}

const TARGET_INSTRUCTIONS: &str = "Kit recorded `capture` (his screen, voice and pointer) and it goes \
to the OpenCode session it is about. `targets` are his open sessions, most recently active first: each \
has a title, its project folder, when it was last active, and its latest exchange (`recent`). `hints` \
are facts Scrawl observed while recording. Which target is `capture` about?";
const HISTORY_INSTRUCTIONS: &str = " `history` lists Kit's earlier items, latest last, and where they \
went; `corrected_to` is where he moved one by hand.";

const FITS_INSTRUCTIONS: &str = "Is `capture` clearly about the work in one of `targets`?";
const FITS_TRUE: &str = "What Kit says or shows clearly matches one target's title, project folder or \
recent exchange: the same app, feature, page, bug or conversation.";
const FITS_FALSE: &str = "It is about something new or general, or none of the targets in particular, \
or it fits several targets equally.";

/// The request for `targets`. They are listed most recently active first, in the state and as
/// the choice's options; Jev's answer shouldn't depend on that order (the skill warns option order
/// can sway a choice), and recency order only means a true tie leans to the newest.
pub fn request_body(
    capture: &Value,
    hints: &[String],
    history: &[Value],
    targets: &[Candidate],
    recent: &[Recent],
    now_ms: u64,
) -> Vec<u8> {
    let entries: Vec<(String, Value)> = targets
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let mut t = serde_json::Map::new();
            t.insert("title".into(), json!(c.label()));
            if let Some(dir) = &c.directory {
                t.insert("directory".into(), json!(tilde(&dir.display().to_string())));
            }
            t.insert(
                "last_active".into(),
                json!(last_active(c.updated_ms, now_ms)),
            );
            if let Some(r) = recent.get(i) {
                let mut last = serde_json::Map::new();
                if let Some(user) = &r.user {
                    last.insert("user".into(), json!(user));
                }
                if let Some(assistant) = &r.assistant {
                    last.insert("assistant".into(), json!(assistant));
                }
                if !last.is_empty() {
                    t.insert("recent".into(), Value::Object(last));
                }
            }
            (format!("t{}", i + 1), Value::Object(t))
        })
        .collect();
    let criteria: Vec<(String, Value)> = entries
        .iter()
        .zip(targets)
        .map(|((t, _), c)| (t.clone(), json!(format!("`targets.{t}` ({})", c.label()))))
        .collect();
    #[derive(Serialize)]
    struct State<'a> {
        capture: &'a Value,
        #[serde(skip_serializing_if = "<[String]>::is_empty")]
        hints: &'a [String],
        #[serde(skip_serializing_if = "<[Value]>::is_empty")]
        history: &'a [Value],
        targets: Ordered<'a>,
    }
    #[derive(Serialize)]
    struct Choice<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        instructions: String,
        criteria: Ordered<'a>,
    }
    #[derive(Serialize)]
    struct Questions<'a> {
        target: Choice<'a>,
        any_fits: Value,
    }
    #[derive(Serialize)]
    struct Body<'a> {
        model: &'static str,
        state: State<'a>,
        questions: Questions<'a>,
    }
    let body = Body {
        model: MODEL,
        state: State {
            capture,
            hints,
            history,
            targets: Ordered(&entries),
        },
        questions: Questions {
            target: Choice {
                kind: "choice",
                instructions: if history.is_empty() {
                    TARGET_INSTRUCTIONS.to_owned()
                } else {
                    format!("{TARGET_INSTRUCTIONS}{HISTORY_INSTRUCTIONS}")
                },
                criteria: Ordered(&criteria),
            },
            any_fits: json!({
                "type": "noul",
                "instructions": FITS_INSTRUCTIONS,
                "criteria": {"true": FITS_TRUE, "false": FITS_FALSE},
            }),
        },
    };
    serde_json::to_vec(&body).unwrap_or_default()
}

fn last_active(updated_ms: u64, now_ms: u64) -> String {
    if updated_ms == 0 {
        return "unknown".into();
    }
    if updated_ms > now_ms {
        return "just now".into();
    }
    format!("{} ago", smart::ago_text(now_ms - updated_ms))
}

fn index_of(t: &str) -> Option<usize> {
    t.strip_prefix('t')?.parse::<usize>().ok()?.checked_sub(1)
}

// MARK: The answer

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Answer {
    pub model: String,
    pub ms: u64,
    pub target: String,
    pub confidence: f64,
    pub probabilities: BTreeMap<String, f64>,
    pub fits: f64,
    pub input_tokens: Option<u64>,
}

impl Answer {
    fn p(&self, t: &str) -> f64 {
        self.probabilities.get(t).copied().unwrap_or(0.0)
    }

    /// The choice's confidence less the likeliest other target's probability.
    pub fn margin(&self) -> f64 {
        let second = self
            .probabilities
            .iter()
            .filter(|(t, _)| **t != self.target)
            .map(|(_, p)| *p)
            .fold(0.0, f64::max);
        self.confidence - second
    }

    /// Targets by probability, the chosen one first.
    fn ranked(&self) -> Vec<(String, f64)> {
        let mut ranked: Vec<(String, f64)> = self
            .probabilities
            .iter()
            .map(|(t, p)| (t.clone(), *p))
            .collect();
        ranked.sort_by(|a, b| {
            (b.0 == self.target)
                .cmp(&(a.0 == self.target))
                .then(b.1.total_cmp(&a.1))
                .then(index_of(&a.0).cmp(&index_of(&b.0)))
        });
        if !ranked.iter().any(|(t, _)| *t == self.target) {
            ranked.insert(0, (self.target.clone(), self.confidence));
        }
        ranked
    }
}

pub fn parse_answer(body: &[u8]) -> Result<Answer> {
    let reply: Value = serde_json::from_slice(body).context("Jev's reply isn't JSON")?;
    let answers = reply.get("answers").context("Jev's reply has no answers")?;
    let target = answers.get("target").context("no target answer")?;
    let choice = target
        .get("choice")
        .and_then(Value::as_str)
        .context("the target answer has no choice")?;
    let probabilities: BTreeMap<String, f64> = target
        .get("probabilities")
        .and_then(Value::as_object)
        .map(|m| {
            m.iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_f64()?)))
                .collect()
        })
        .unwrap_or_default();
    let fits = answers
        .get("any_fits")
        .and_then(|a| a.get("noul").or_else(|| a.get("probability")))
        .and_then(Value::as_f64)
        .context("no any_fits answer")?;
    Ok(Answer {
        model: reply
            .get("model")
            .and_then(Value::as_str)
            .unwrap_or(MODEL)
            .to_owned(),
        ms: 0,
        target: choice.to_owned(),
        confidence: target
            .get("confidence")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        probabilities,
        fits,
        input_tokens: reply.pointer("/usage/input_tokens").and_then(Value::as_u64),
    })
}

// MARK: The gateway

/// POSTs `body` to Jev with `/usr/bin/curl`. The key reaches curl through a config on stdin, so
/// it's never in an argument list or a log.
pub fn gateway(body: &[u8], timeout: Duration) -> Result<Vec<u8>> {
    let key = api_key()?;
    let body = std::str::from_utf8(body).context("request isn't UTF-8")?;
    with_retry(timeout, |left| post(&key, body, left), std::thread::sleep)
}

/// One HTTP answer: its status, a `Retry-After` in seconds, and its body.
#[derive(Debug)]
struct Reply {
    status: u16,
    retry_after: Option<Duration>,
    body: String,
}

/// The wait before the one retry after a 429: `Retry-After` when it's short, else this.
const RETRY_WAIT: Duration = Duration::from_millis(400);
const RETRY_AFTER_MAX: Duration = Duration::from_secs(1);

/// Posts within `timeout`, retrying a 429 once after [`RETRY_WAIT`] (or a short `Retry-After`)
/// when enough time is left for a second try.
fn with_retry(
    timeout: Duration,
    mut post: impl FnMut(Duration) -> Result<Reply>,
    mut sleep: impl FnMut(Duration),
) -> Result<Vec<u8>> {
    let started = Instant::now();
    let mut reply = post(timeout)?;
    if reply.status == 429 {
        let wait = reply
            .retry_after
            .filter(|w| *w <= RETRY_AFTER_MAX)
            .unwrap_or(RETRY_WAIT);
        let left = timeout.saturating_sub(started.elapsed());
        if left >= wait + MIN_ASK {
            sleep(wait);
            reply = post(left - wait)?;
        }
    }
    if !(200..300).contains(&reply.status) {
        bail!(
            "HTTP {}: {}",
            reply.status,
            reply.body.chars().take(200).collect::<String>()
        );
    }
    Ok(reply.body.into_bytes())
}

fn post(key: &str, body: &str, timeout: Duration) -> Result<Reply> {
    let quote = |s: &str| s.replace('\\', "\\\\").replace('"', "\\\"");
    let config = format!(
        "header = \"Authorization: Bearer {}\"\ndata-binary = \"{}\"\n",
        quote(key),
        quote(body)
    );
    let mut child = Command::new("/usr/bin/curl")
        .args([
            "-sS",
            "-K",
            "-",
            "--max-time",
            &format!("{:.2}", timeout.as_secs_f64()),
            "-X",
            "POST",
            "-H",
            "Content-Type: application/json",
            "-H",
            &format!("User-Agent: {USER_AGENT}"),
            "-w",
            "\n%{http_code} %header{retry-after}",
            ENDPOINT,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("couldn't start curl")?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(config.as_bytes())
            .context("couldn't hand curl the request")?;
    }
    let output = child.wait_with_output().context("curl didn't finish")?;
    // 28: curl's "operation timed out".
    if output.status.code() == Some(28) {
        bail!("{TIMED_OUT}: no answer within {} ms", timeout.as_millis());
    }
    if !output.status.success() {
        bail!(
            "curl failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let (body, trailer) = text.rsplit_once('\n').unwrap_or(("", &text));
    let mut trailer = trailer.split_whitespace();
    let status = trailer.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    let retry_after = trailer
        .next()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|s| s.is_finite() && *s >= 0.0)
        .map(Duration::from_secs_f64);
    Ok(Reply {
        status,
        retry_after,
        body: body.to_owned(),
    })
}

/// The OpenCode key: `jevApiKey` in config.json, `$OPENCODE_API_KEY`, or the active `opencode`
/// credential in OpenCode's database (read-only). Never printed or logged.
fn api_key() -> Result<String> {
    if let Some(key) = crate::handoff::jev_api_key().filter(|k| !k.trim().is_empty()) {
        return Ok(key.trim().to_owned());
    }
    if let Ok(key) = std::env::var("OPENCODE_API_KEY")
        && !key.trim().is_empty()
    {
        return Ok(key.trim().to_owned());
    }
    let db = opencode_db().context("no OpenCode database found for a key")?;
    let rows = sqlite_json(
        &db,
        "select value from credential where integration_id='opencode' and active=1 limit 1",
    )
    .context("couldn't read OpenCode's credentials")?;
    let value = rows
        .first()
        .and_then(|r| r.get("value"))
        .and_then(Value::as_str)
        .context("no active opencode credential")?;
    let credential: Value =
        serde_json::from_str(value).context("the opencode credential isn't JSON")?;
    if let Some(expires) = credential.get("expires").and_then(Value::as_u64)
        && expires <= now_ms()
    {
        bail!("the opencode credential has expired");
    }
    credential
        .get("access")
        .or_else(|| credential.get("key"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .context("the opencode credential has no key")
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// `$OPENCODE_DB`, the database the local server has open, or the usual places.
fn opencode_db() -> Option<PathBuf> {
    if let Some(db) = std::env::var_os("OPENCODE_DB").map(PathBuf::from)
        && db.is_file()
    {
        return Some(db);
    }
    if let Some(db) = super::served::opencode_db() {
        return Some(db);
    }
    let home = dirs::home_dir()?;
    [
        ".local/share/opencode-studio-migration/opencode.db",
        ".local/share/opencode/opencode.db",
    ]
    .into_iter()
    .map(|p| home.join(p))
    .find(|p| p.is_file())
}

fn sqlite_json(db: &Path, sql: &str) -> Option<Vec<Value>> {
    let output = super::run_with_timeout(
        Command::new("/usr/bin/sqlite3")
            .args(["-readonly", "-json"])
            .arg(db)
            .arg(sql),
        Duration::from_secs(3),
    )?;
    if !output.status.success() {
        return None;
    }
    if output.stdout.trim_ascii().is_empty() {
        return Some(Vec::new());
    }
    serde_json::from_slice(&output.stdout).ok()
}

// MARK: Sessions' last messages

/// Each session's last messages: through OpenCode's API (within [`RECENT_BUDGET`], a session
/// that doesn't fit gets none), or from its history before `before_ms`.
pub fn read_recent(ids: &[String], before_ms: Option<u64>, budget: Duration) -> Vec<Recent> {
    if cfg!(test) {
        return vec![Recent::default(); ids.len()];
    }
    match before_ms {
        Some(before) => recent_from_history(ids, before),
        None => recent_from_api(ids, crate::handoff::api_get_within, budget),
    }
}

pub fn recent_from_api(
    ids: &[String],
    api: fn(&str, Duration) -> Result<Vec<u8>>,
    budget: Duration,
) -> Vec<Recent> {
    let deadline = Instant::now() + budget;
    let get = move |path: &str| -> Option<Vec<Value>> {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        let body = api(path, left).ok()?;
        let value: Value = serde_json::from_slice(&body).ok()?;
        value
            .get("data")
            .or(Some(&value))
            .and_then(Value::as_array)
            .cloned()
    };
    std::thread::scope(|scope| {
        let handles: Vec<_> = ids
            .iter()
            .map(|id| {
                let id = crate::handoff::query_escape(id);
                scope.spawn(move || {
                    let user = get(&format!(
                        "/api/session/{id}/message?limit=1&order=desc&type=user"
                    ))
                    .and_then(|m| m.first().and_then(user_text));
                    let assistant = get(&format!(
                        "/api/session/{id}/message?limit=4&order=desc&type=assistant"
                    ))
                    .and_then(|m| m.iter().find_map(assistant_text));
                    Recent { user, assistant }
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap_or_default())
            .collect()
    })
}

fn user_text(message: &Value) -> Option<String> {
    let text = message.get("text")?.as_str()?;
    (!text.trim().is_empty()).then(|| trim_end(text, RECENT_CHARS))
}

fn assistant_text(message: &Value) -> Option<String> {
    let text: Vec<&str> = message
        .get("content")?
        .as_array()?
        .iter()
        .filter(|c| c.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|c| c.get("text").and_then(Value::as_str))
        .collect();
    let text = text.join(" ");
    (!text.trim().is_empty()).then(|| trim_end(&text, RECENT_CHARS))
}

fn safe_id(id: &str) -> bool {
    !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The last messages each session had before `before_ms`, from OpenCode's database, so a replay
/// doesn't see the capture itself or what came of it.
pub fn recent_from_history(ids: &[String], before_ms: u64) -> Vec<Recent> {
    let Some(db) = opencode_db() else {
        return vec![Recent::default(); ids.len()];
    };
    let parts: Vec<String> = ids
        .iter()
        .filter(|id| safe_id(id))
        .flat_map(|id| {
            [
                format!(
                    "select * from (select '{id}' as s, type, data from session_message \
                     where session_id='{id}' and type='user' and time_created < {before_ms} \
                     order by time_created desc limit 1)"
                ),
                format!(
                    "select * from (select '{id}' as s, type, data from session_message \
                     where session_id='{id}' and type='assistant' and time_created < {before_ms} \
                     and instr(data, '\"type\":\"text\"') > 0 order by time_created desc limit 1)"
                ),
            ]
        })
        .collect();
    let rows = if parts.is_empty() {
        Vec::new()
    } else {
        sqlite_json(&db, &parts.join(" union all ")).unwrap_or_default()
    };
    ids.iter()
        .map(|id| {
            let mut recent = Recent::default();
            for row in &rows {
                if row.get("s").and_then(Value::as_str) != Some(id) {
                    continue;
                }
                let Some(data) = row
                    .get("data")
                    .and_then(Value::as_str)
                    .and_then(|d| serde_json::from_str::<Value>(d).ok())
                else {
                    continue;
                };
                match row.get("type").and_then(Value::as_str) {
                    Some("user") => recent.user = user_text(&data),
                    Some("assistant") => recent.assistant = assistant_text(&data),
                    _ => {}
                }
            }
            recent
        })
        .collect()
}

/// The root session a bundle was actually delivered to or pasted into: the first user message
/// within six hours that names the bundle's folder, followed up to its root.
pub fn delivered_to(bundle: &Path, created_ms: u64) -> Option<String> {
    let name = bundle.file_name()?.to_string_lossy().into_owned();
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    {
        return None;
    }
    let db = opencode_db()?;
    let until = created_ms + 6 * 3_600_000;
    let sql = format!(
        "with first as (select session_id from session_message where type='user' \
         and time_created between {created_ms} and {until} and instr(data, '{name}') > 0 \
         order by time_created limit 1), \
         up(id, parent) as (select s.id, s.parent_id from session_v2 s join first f on s.id = f.session_id \
         union all select s.id, s.parent_id from session_v2 s join up on s.id = up.parent) \
         select id from up where parent is null limit 1"
    );
    sqlite_json(&db, &sql)?
        .first()?
        .get("id")?
        .as_str()
        .map(str::to_owned)
}

// MARK: The decisions log

/// One line per decision and per correction, across captures:
/// `~/Library/Application Support/scrawl/routing-decisions.jsonl` (`$SCRAWL_ROUTING_LOG`).
pub fn log_path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    if let Some(path) = std::env::var_os("SCRAWL_ROUTING_LOG") {
        return Some(PathBuf::from(path));
    }
    Some(dirs::home_dir()?.join("Library/Application Support/scrawl/routing-decisions.jsonl"))
}

fn append(line: &Value) {
    if let Err(err) = try_append(line) {
        scrawl_log(format!("routing (jev): couldn't log a decision: {err:#}"));
    }
}

fn try_append(line: &Value) -> Result<()> {
    let Some(path) = log_path() else {
        return Ok(());
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("couldn't open {}", path.display()))?;
    writeln!(file, "{line}")?;
    Ok(())
}

fn bundle_name(dir: &Path) -> String {
    dir.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

fn session_json(c: &Candidate) -> Value {
    json!({"id": c.id, "title": c.label()})
}

/// What a log line is about: a capture's bundle, or a `scrawl router decide` decision.
#[derive(Clone, Copy, Debug)]
pub enum Subject<'a> {
    Bundle(&'a str),
    Decision(&'a str),
}

impl Subject<'_> {
    fn stamp(self, line: &mut Value) {
        match self {
            Self::Bundle(name) => line["bundle"] = json!(name),
            Self::Decision(id) => line["decision_id"] = json!(id),
        }
    }
}

/// The line logging a decision (schema in `docs/routing.md`).
pub fn decision_line(
    kind: &str,
    subject: Subject,
    decision: &Decision,
    record: &Record,
    app: Option<&str>,
    time_ms: u64,
) -> Value {
    let to = match decision {
        Decision::Send { to, .. } => Some(session_json(&to.candidate)),
        Decision::Offer { choices } => choices.first().map(|c| session_json(&c.candidate)),
        Decision::Keep { .. } => None,
    };
    let band = match record.by.as_str() {
        "jev" => record.band,
        _ => Band::of(decision),
    };
    let mut line = json!({
        "time": time_ms,
        "kind": kind,
        "event": "decision",
        "decision": band.label(),
        "by": record.by,
        "to": to,
        "confidence": record.confidence,
        "any_fits": record.fits,
        "model": record.model,
        "ms": record.ms,
        "app": app,
    });
    subject.stamp(&mut line);
    line
}

pub fn log_decision(dir: &Path, decision: &Decision, record: &Record, app: Option<&str>) {
    let name = bundle_name(dir);
    append(&decision_line(
        "capture",
        Subject::Bundle(&name),
        decision,
        record,
        app,
        now_ms(),
    ));
}

/// Appends a line to the decisions log (`scrawl router decide`'s own lines).
pub fn log_line(line: &Value) {
    append(line);
}

/// How Kit corrected a route.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub enum Action {
    /// Not sent after all (Esc or ✕ during the countdown; Hex's cancel).
    Cancel,
    /// Sent somewhere else (a chip, `scrawl send --reroute`, Hex's picker).
    Redirect,
    /// Pasted instead.
    Paste,
    /// Edited before it went.
    Edit,
}

impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cancel => "cancel",
            Self::Redirect => "redirect",
            Self::Paste => "paste",
            Self::Edit => "edit",
        }
    }
}

/// A correction to a route. The app's captures and `scrawl router correct` both log through
/// [`Correction::log`] (schema in `docs/routing.md`).
#[derive(Clone, Copy, Debug)]
pub struct Correction<'a> {
    pub kind: &'a str,
    pub subject: Subject<'a>,
    pub action: Action,
    /// How, in the app: `escape`, `dismiss`, `paste`, `chip`, `reroute`.
    pub detail: Option<&'a str>,
    /// What routing chose: `(id, title)`.
    pub from: Option<(&'a str, &'a str)>,
    /// Where Kit put it instead.
    pub to: Option<(&'a str, &'a str)>,
    pub app: Option<&'a str>,
}

impl Correction<'_> {
    pub fn line(&self, time_ms: u64) -> Value {
        let session =
            |s: Option<(&str, &str)>| s.map(|(id, title)| json!({"id": id, "title": title}));
        let mut line = json!({
            "time": time_ms,
            "kind": self.kind,
            "event": "correction",
            "action": self.action.label(),
            "from": session(self.from),
            "to": session(self.to),
            "app": self.app,
        });
        if let Some(detail) = self.detail {
            line["detail"] = json!(detail);
        }
        self.subject.stamp(&mut line);
        line
    }

    pub fn log(&self) {
        if let Err(err) = write_correction(&self.line(now_ms())) {
            scrawl_log(format!("routing (jev): correction not logged: {err:#}"));
        }
    }
}

/// Checks a correction line against the schema in `docs/routing.md` and appends it. Every
/// correction comes through here: the app's, `scrawl router correct` and `scrawl router log`.
pub fn write_correction(line: &Value) -> Result<()> {
    check(line)?;
    append(line);
    Ok(())
}

/// The schema check [`write_correction`] makes.
pub fn check(line: &Value) -> Result<()> {
    let text = |key: &str| line.get(key).and_then(Value::as_str);
    if !line.is_object() {
        bail!("a correction is a JSON object");
    }
    if !matches!(text("kind"), Some("capture" | "dictation")) {
        bail!("kind must be capture or dictation");
    }
    if text("event") != Some("correction") {
        bail!("event must be correction");
    }
    let action = text("action").unwrap_or_default();
    if <Action as clap::ValueEnum>::from_str(action, false).is_err() {
        bail!("action must be cancel, redirect, paste or edit, not {action:?}");
    }
    if ["decision_id", "item_id", "bundle"]
        .iter()
        .all(|key| text(key).is_none_or(str::is_empty))
    {
        bail!("a correction needs a decision_id, an item_id or a bundle");
    }
    if !line.get("time").is_some_and(Value::is_u64) {
        bail!("time must be epoch milliseconds");
    }
    for key in ["from", "to"] {
        match line.get(key) {
            None | Some(Value::Null) => {}
            Some(session) if session.get("id").and_then(Value::as_str).is_some() => {}
            Some(_) => bail!("{key} must be null or {{\"id\", \"title\"}}"),
        }
    }
    if action == "redirect" && line.get("to").is_none_or(Value::is_null) {
        bail!("a redirect needs to");
    }
    Ok(())
}

/// What routing suggested for a bundle: the session a send went or an offer pointed to.
fn suggested(dir: &Path) -> Option<(String, String)> {
    let value: Value =
        serde_json::from_slice(&std::fs::read(dir.join(super::RECORD)).ok()?).ok()?;
    let decision = value.get("decision")?;
    let candidate = match decision.get("kind")?.as_str()? {
        "send" => decision.pointer("/to/candidate")?,
        "offer" => decision.pointer("/choices/0/candidate")?,
        _ => return None,
    };
    Some((
        candidate.get("id")?.as_str()?.to_owned(),
        candidate
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
    ))
}

/// Logs Kit undoing or overriding a capture's route (with Jev routing on): `detail` is how
/// (`escape`, `dismiss`, `paste`, `chip`, `reroute`). A chip that matches the suggestion is a
/// redirect whose `from` and `to` match: a confirmation.
pub fn correct_capture(dir: &Path, action: Action, detail: &str, to: Option<(&str, &str)>) {
    if !enabled() {
        return;
    }
    let from = suggested(dir);
    let app = crate::handoff::load_bundle(dir)
        .ok()
        .and_then(|(_, bundle)| bundle.target.app);
    Correction {
        kind: "capture",
        subject: Subject::Bundle(&bundle_name(dir)),
        action,
        detail: Some(detail),
        from: from
            .as_ref()
            .map(|(id, title)| (id.as_str(), title.as_str())),
        to,
        app: app.as_deref(),
    }
    .log();
}

// MARK: Explaining

pub fn explain(record: &Record) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = write!(out, "Jev       {} · {}", record.band.label(), record.reason);
    if let Some(model) = &record.model {
        let _ = write!(out, " · {model}");
    }
    if let Some(ms) = record.ms {
        let _ = write!(out, " · {ms} ms");
    }
    if let Some(tokens) = record.input_tokens {
        let _ = write!(
            out,
            " · {tokens} tokens (${:.5})",
            tokens as f64 * PRICE_PER_M / 1e6
        );
    }
    out.push('\n');
    if let Some(error) = &record.error {
        let _ = writeln!(out, "          error: {error}");
    }
    if let (Some(c), Some(f)) = (record.confidence, record.fits) {
        let _ = writeln!(out, "          confidence {c:.2} · fits {f:.2}");
    }
    for pick in record.probabilities.iter().take(5) {
        let _ = writeln!(
            out,
            "          {:>5.2}  {:<4} {} ({})",
            pick.p, pick.t, pick.title, pick.session
        );
    }
    out
}

/// The `jev` section of a bundle's `routing.json`, when it has one.
pub fn read_record(dir: &Path) -> Option<Record> {
    let value: Value =
        serde_json::from_slice(&std::fs::read(dir.join(super::RECORD)).ok()?).ok()?;
    serde_json::from_value(value.get("jev")?.clone()).ok()
}

pub fn cost(tokens: u64) -> f64 {
    tokens as f64 * PRICE_PER_M / 1e6
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::smart::{Clue, Policy};
    use std::cell::RefCell;

    fn candidate(id: &str, title: &str, dir: &str, updated: u64) -> Candidate {
        Candidate {
            id: id.into(),
            title: title.into(),
            directory: Some(PathBuf::from(dir)),
            repo: None,
            updated_ms: updated,
            viewed_ms: None,
        }
    }

    fn evidence(clues: Vec<Clue>) -> Evidence {
        Evidence {
            address: None,
            clues,
            candidates: vec![
                candidate("ses_home", "HOME", "/Users/k", 9_000),
                candidate("ses_hex", "HEX", "/Users/k/code/hex", 10_000),
                candidate("ses_board", "PROBLEMS", "/Users/k/code/board", 8_000),
            ],
            now_ms: 100_000,
            url: None,
        }
    }

    thread_local! {
        static SENT: RefCell<Option<Value>> = const { RefCell::new(None) };
        static REPLY: RefCell<Option<Value>> = const { RefCell::new(None) };
    }

    fn reply(target: &str, confidence: f64, fits: f64, probabilities: Value) {
        REPLY.with(|r| {
            *r.borrow_mut() = Some(json!({
                "model": "jev-1.13.0",
                "answers": {
                    "target": {"type": "choice", "choice": target, "confidence": confidence,
                               "probabilities": probabilities},
                    "any_fits": {"type": "noul", "noul": fits},
                },
                "usage": {"input_tokens": 1234},
            }))
        });
    }

    fn fake_ask(body: &[u8], _: Duration) -> Result<Vec<u8>> {
        SENT.with(|s| *s.borrow_mut() = serde_json::from_slice(body).ok());
        REPLY.with(|r| match r.borrow().as_ref() {
            Some(reply) => Ok(serde_json::to_vec(reply).unwrap()),
            None => bail!("HTTP 529: overloaded"),
        })
    }

    fn fake_recent(ids: &[String], _: Option<u64>, _: Duration) -> Vec<Recent> {
        ids.iter()
            .map(|id| Recent {
                user: Some(format!("last words in {id}")),
                assistant: None,
            })
            .collect()
    }

    const HOOKS: Hooks = Hooks {
        ask: fake_ask,
        recent: fake_recent,
        deadline: None,
    };

    fn run(evidence: &Evidence, thresholds: &Thresholds) -> (Judgement, Record) {
        let smart = smart::decide(evidence, Policy::Weigh);
        judge(
            evidence,
            smart,
            &json!({"kind": "capture", "transcript": "the hex icon"}),
            &Asking::default(),
            None,
            &HOOKS,
            thresholds,
        )
    }

    fn sent_to(judgement: &Judgement) -> Option<crate::model::Destination> {
        smart::destination(judgement, None)
    }

    #[test]
    fn a_sure_answer_sends_with_the_runner_up_one_click_away() {
        // Targets newest first: t1 HEX, t2 HOME, t3 PROBLEMS.
        reply("t1", 0.93, 0.9, json!({"t1": 0.95, "t2": 0.01, "t3": 0.04}));
        let (judgement, record) = run(&evidence(vec![]), &Thresholds::default());
        let dest = sent_to(&judgement).unwrap();
        assert_eq!(
            (dest.session.as_str(), dest.via.as_str()),
            ("ses_hex", "jev")
        );
        let Decision::Send { instead, .. } = &judgement.decision else {
            unreachable!()
        };
        assert!(instead.is_empty(), "4% isn't worth a chip");
        assert_eq!(record.band, Band::Send);
        assert_eq!(record.by, "jev");
        assert_eq!(record.target.as_ref().unwrap().title, "HEX");
        assert_eq!(record.input_tokens, Some(1234));
        assert_eq!(record.model.as_deref(), Some("jev-1.13.0"));
    }

    #[test]
    fn a_middling_answer_offers_the_best_then_home() {
        reply("t3", 0.62, 0.6, json!({"t1": 0.3, "t2": 0.05, "t3": 0.65}));
        let (judgement, record) = run(&evidence(vec![]), &Thresholds::default());
        let Decision::Offer { choices } = &judgement.decision else {
            panic!("{:?}", judgement.decision)
        };
        let ids: Vec<&str> = choices.iter().map(|c| c.candidate.id.as_str()).collect();
        assert_eq!(ids, ["ses_board", "ses_home", "ses_hex"]);
        assert_eq!(record.band, Band::Ask);
    }

    #[test]
    fn a_lost_answer_sends_to_home_or_keeps_it() {
        reply("t1", 0.3, 0.2, json!({"t1": 0.4, "t2": 0.3, "t3": 0.3}));
        let (judgement, record) = run(&evidence(vec![]), &Thresholds::default());
        let dest = sent_to(&judgement).unwrap();
        assert_eq!(
            (dest.session.as_str(), dest.via.as_str()),
            ("ses_home", "jev-home")
        );
        assert_eq!(record.band, Band::Fallback);
        let Decision::Send { instead, .. } = &judgement.decision else {
            unreachable!()
        };
        assert_eq!(
            instead[0].candidate.id, "ses_hex",
            "Jev's guess one click away"
        );

        let keep = Thresholds {
            low: Low::Keep,
            ..Thresholds::default()
        };
        let (judgement, record) = run(&evidence(vec![]), &keep);
        assert!(matches!(judgement.decision, Decision::Keep { .. }));
        assert_eq!(record.band, Band::Keep);
    }

    #[test]
    fn a_429_is_retried_once_after_a_short_wait() {
        let reply = |status, retry_after: Option<f64>| Reply {
            status,
            retry_after: retry_after.map(Duration::from_secs_f64),
            body: "ok".into(),
        };
        let run = |replies: Vec<Reply>, timeout_ms| {
            let mut replies = replies.into_iter();
            let mut posts = 0;
            let mut waits = Vec::new();
            let result = with_retry(
                Duration::from_millis(timeout_ms),
                |_| {
                    posts += 1;
                    Ok(replies.next().unwrap())
                },
                |w| waits.push(w.as_millis()),
            );
            (result.is_ok(), posts, waits)
        };
        assert_eq!(
            run(vec![reply(429, None), reply(200, None)], 2300),
            (true, 2, vec![400])
        );
        // A short Retry-After is honoured, a long one isn't, and there's only one retry.
        assert_eq!(
            run(vec![reply(429, Some(0.2)), reply(200, None)], 2300),
            (true, 2, vec![200])
        );
        assert_eq!(
            run(vec![reply(429, Some(30.0)), reply(429, None)], 2300),
            (false, 2, vec![400])
        );
        // Without time for a second try it fails open at once.
        assert_eq!(run(vec![reply(429, None)], 600), (false, 1, vec![]));
    }

    #[test]
    fn any_fits_only_sends_a_weak_choice_home() {
        // any_fits never blocks a confident choice...
        reply("t1", 0.95, 0.1, json!({"t1": 0.97, "t2": 0.02, "t3": 0.01}));
        let (_, record) = run(&evidence(vec![]), &Thresholds::default());
        assert_eq!(record.band, Band::Send);
        // ...but picks HOME over a weak one.
        reply("t1", 0.7, 0.35, json!({"t1": 0.7, "t2": 0.2, "t3": 0.1}));
        let (judgement, _) = run(&evidence(vec![]), &Thresholds::default());
        assert!(matches!(judgement.decision, Decision::Offer { .. }));
        reply("t1", 0.7, 0.1, json!({"t1": 0.7, "t2": 0.2, "t3": 0.1}));
        let (judgement, record) = run(&evidence(vec![]), &Thresholds::default());
        assert_eq!(record.band, Band::Fallback);
        assert_eq!(sent_to(&judgement).unwrap().session, "ses_home");
    }

    #[test]
    fn a_close_runner_up_is_not_a_send() {
        reply("t1", 0.88, 0.9, json!({"t1": 0.88, "t2": 0.7}));
        let (judgement, record) = run(&evidence(vec![]), &Thresholds::default());
        assert_eq!(record.band, Band::Ask);
        assert!(matches!(judgement.decision, Decision::Offer { .. }));
    }

    #[test]
    fn settled_sources_win_without_asking() {
        reply("t2", 0.99, 0.99, json!({"t2": 1.0}));
        SENT.with(|s| *s.borrow_mut() = None);
        let page = Clue::new(
            Source::Page,
            Pointer::Session {
                id: "ses_board".into(),
                directory: None,
            },
            "http://localhost:5173 registered its session",
        );
        let (judgement, record) = run(&evidence(vec![page]), &Thresholds::default());
        assert_eq!(sent_to(&judgement).unwrap().session, "ses_board");
        assert_eq!(record.by, "smart");
        assert!(SENT.with(|s| s.borrow().is_none()), "Jev wasn't asked");

        let mut addressed = evidence(vec![]);
        addressed.address = Some(smart::Address::Chosen("ses_hex".into()));
        let (judgement, record) = run(&addressed, &Thresholds::default());
        assert_eq!(sent_to(&judgement).unwrap().via, "chosen");
        assert_eq!(record.by, "smart");
    }

    #[test]
    fn a_failure_leaves_smart_routings_decision() {
        REPLY.with(|r| *r.borrow_mut() = None);
        let ev = evidence(vec![]);
        let smart = smart::decide(&ev, Policy::Weigh);
        let (judgement, record) = run(&ev, &Thresholds::default());
        assert_eq!(judgement.decision, smart.decision);
        assert_eq!(record.by, "smart");
        assert!(record.error.as_deref().unwrap().contains("529"));
    }

    #[test]
    fn the_request_lists_targets_newest_first_with_hints_and_recent_lines() {
        reply("t1", 0.9, 0.9, json!({"t1": 0.9}));
        let terminal = Clue::new(
            Source::Terminal,
            Pointer::Folder(smart::Folder {
                path: "/Users/k/code/hex/src".into(),
                root: "/Users/k/code/hex".into(),
                tree: true,
                repo: None,
            }),
            "the terminal's focused pane is in /Users/k/code/hex/src",
        );
        let said = Clue::new(
            Source::Speech,
            Pointer::Words {
                words: vec!["icon".into()],
            },
            "said",
        );
        run(&evidence(vec![terminal, said]), &Thresholds::default());
        let sent = SENT.with(|s| s.borrow().clone()).unwrap();
        assert_eq!(sent["model"], MODEL);
        let targets = sent["state"]["targets"].as_object().unwrap();
        assert_eq!(targets["t1"]["title"], "HEX");
        assert_eq!(targets["t1"]["last_active"], "1 min ago");
        assert_eq!(targets["t1"]["recent"]["user"], "last words in ses_hex");
        assert_eq!(targets["t2"]["title"], "HOME");
        assert_eq!(
            sent["questions"]["target"]["criteria"]["t3"],
            "`targets.t3` (PROBLEMS)"
        );
        let hints = sent["state"]["hints"].as_array().unwrap();
        assert_eq!(hints.len(), 1, "speech is in the transcript, not a hint");
        assert!(
            hints[0]
                .as_str()
                .unwrap()
                .ends_with("Sessions in that project: `targets.t1`."),
            "{hints:?}"
        );
        assert_eq!(sent["questions"]["any_fits"]["type"], "noul");
    }

    #[test]
    fn targets_keep_named_sessions_within_the_cap() {
        let mut ev = evidence(vec![]);
        for i in 0..30 {
            ev.candidates.push(candidate(
                &format!("ses_{i:02}"),
                &format!("S{i}"),
                "/tmp",
                20_000 + i,
            ));
        }
        ev.candidates
            .push(candidate("ses_old", "OLD", "/tmp/old", 1));
        ev.clues.push(Clue::new(
            Source::Viewed,
            Pointer::Session {
                id: "ses_old".into(),
                directory: None,
            },
            "OpenCode Desktop showed OLD last",
        ));
        let smart = smart::decide(&ev, Policy::Weigh);
        let targets = pick_targets(&ev, &smart, &Asking::default());
        assert_eq!(targets.len(), MAX_TARGETS);
        assert!(targets.iter().any(|c| c.id == "ses_old"));
        assert!(targets.iter().any(|c| c.id == "ses_home"));
        assert_eq!(targets[0].id, "ses_29");
        assert_eq!(targets.last().unwrap().id, "ses_old");
    }

    #[test]
    fn long_transcripts_keep_their_start_and_end() {
        let text = format!("{}{}", "a".repeat(3000), "END");
        let trimmed = trim_middle(&text, 100);
        assert!(trimmed.starts_with("aaa") && trimmed.ends_with("END"));
        assert!(trimmed.chars().count() <= 103);
        assert_eq!(trim_middle(" short ", 100), "short");
    }

    #[test]
    fn thresholds_read_from_config_with_defaults() {
        let t = Thresholds::from_config(Some(
            &json!({"send": 0.9, "margin": 0.2, "offer": 2, "low": "keep"}),
        ));
        assert_eq!(t.send, 0.9);
        assert_eq!(t.margin, 0.2);
        assert_eq!(t.offer, OFFER_AT, "out of range keeps the default");
        assert_eq!(t.low, Low::Keep);
        assert_eq!(Thresholds::from_config(None), Thresholds::default());
    }

    #[test]
    fn recent_lines_come_from_the_api_and_skip_what_doesnt_fit() {
        fn api(path: &str, _: Duration) -> Result<Vec<u8>> {
            if path.contains("ses_slow") {
                bail!("timed out");
            }
            Ok(if path.contains("type=user") {
                br#"{"data":[{"type":"user","text":"From HOME:\n please   look"}]}"#.to_vec()
            } else {
                br#"{"data":[{"type":"assistant","content":[{"type":"reasoning","text":"x"}]},
                    {"type":"assistant","content":[{"type":"text","text":"Done."}]}]}"#
                    .to_vec()
            })
        }
        let recent = recent_from_api(
            &["ses_a".into(), "ses_slow".into()],
            api,
            Duration::from_secs(1),
        );
        assert_eq!(recent[0].user.as_deref(), Some("From HOME: please look"));
        assert_eq!(recent[0].assistant.as_deref(), Some("Done."));
        assert_eq!(recent[1], Recent::default());
    }

    #[test]
    fn decision_lines_follow_the_schema() {
        reply("t1", 0.93, 0.9, json!({"t1": 0.95}));
        let (judgement, record) = run(&evidence(vec![]), &Thresholds::default());
        let line = decision_line(
            "capture",
            Subject::Bundle("b.scrawl"),
            &judgement.decision,
            &record,
            Some("Ghostty"),
            5,
        );
        assert_eq!(line["decision"], "send");
        assert_eq!(line["to"]["id"], "ses_hex");
        assert_eq!(line["to"]["title"], "HEX");
        assert_eq!(
            (line["kind"].as_str(), line["event"].as_str()),
            (Some("capture"), Some("decision"))
        );
        assert_eq!(line["bundle"], "b.scrawl");
        assert_eq!(line["confidence"], 0.93);
        assert_eq!(line["any_fits"], 0.9);
        assert_eq!(line["app"], "Ghostty");
        assert_eq!(line["time"], 5);
        let fix = Correction {
            kind: "dictation",
            subject: Subject::Decision("d1"),
            action: Action::Redirect,
            detail: None,
            from: Some(("ses_hex", "HEX")),
            to: Some(("ses_home", "HOME")),
            app: Some("Ghostty"),
        }
        .line(6);
        assert_eq!(fix["event"], "correction");
        assert_eq!(fix["action"], "redirect");
        assert_eq!(fix["decision_id"], "d1");
        assert_eq!(
            (fix["from"]["id"].as_str(), fix["to"]["id"].as_str()),
            (Some("ses_hex"), Some("ses_home"))
        );
        assert!(fix.get("bundle").is_none() && fix.get("detail").is_none());
    }

    #[test]
    fn a_deadline_too_close_skips_jev_as_a_timeout() {
        reply("t1", 0.93, 0.9, json!({"t1": 0.95}));
        SENT.with(|s| *s.borrow_mut() = None);
        let hooks = Hooks {
            deadline: Some(Instant::now() + Duration::from_millis(100)),
            ..HOOKS
        };
        let ev = evidence(vec![]);
        let smart = smart::decide(&ev, Policy::Weigh);
        let (_, record) = judge(
            &ev,
            smart,
            &json!({}),
            &Asking::default(),
            None,
            &hooks,
            &Thresholds::default(),
        );
        assert_eq!(record.by, "smart");
        assert!(record.error.as_deref().unwrap().starts_with(TIMED_OUT));
        assert!(SENT.with(|s| s.borrow().is_none()), "Jev wasn't asked");
    }

    #[test]
    fn history_names_targets_and_brings_their_sessions_in() {
        reply("t1", 0.9, 0.9, json!({"t1": 0.9}));
        let mut ev = evidence(vec![]);
        for i in 0..25 {
            ev.candidates.push(candidate(
                &format!("ses_{i:02}"),
                &format!("S{i}"),
                "/tmp",
                20_000 + i,
            ));
        }
        ev.candidates
            .push(candidate("ses_old", "OLD", "/tmp/old", 1));
        let asking = Asking {
            history: vec![Past {
                text: "move the button".into(),
                to: Some("ses_hex".into()),
                corrected_to: Some("ses_old".into()),
            }],
            pinned: Vec::new(),
        };
        let smart = smart::decide(&ev, Policy::Weigh);
        judge(
            &ev,
            smart,
            &json!({"transcript": "and the label"}),
            &asking,
            None,
            &HOOKS,
            &Thresholds::default(),
        );
        let sent = SENT.with(|s| s.borrow().clone()).unwrap();
        let past = &sent["state"]["history"][0];
        assert_eq!(past["text"], "move the button");
        assert!(
            past["went_to"].as_str().unwrap().ends_with("(HEX)"),
            "{past}"
        );
        let corrected = past["corrected_to"].as_str().unwrap();
        assert!(
            corrected.starts_with("targets.t") && corrected.ends_with("(OLD)"),
            "{past}"
        );
        assert!(
            sent["questions"]["target"]["instructions"]
                .as_str()
                .unwrap()
                .contains("history")
        );
    }

    #[test]
    fn answers_parse_and_rank() {
        let answer = parse_answer(
            br#"{"answers":{"target":{"choice":"t2","confidence":0.5,"probabilities":{"t1":0.3,"t2":0.6,"t3":0.1}},
                "any_fits":{"type":"noul","noul":0.8}}}"#,
        )
        .unwrap();
        assert_eq!(answer.model, MODEL);
        let ranked: Vec<String> = answer.ranked().into_iter().map(|(t, _)| t).collect();
        assert_eq!(ranked, ["t2", "t1", "t3"]);
        assert!(parse_answer(b"{}").is_err());
    }
}
