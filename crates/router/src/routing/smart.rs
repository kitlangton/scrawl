//! Deciding where a capture goes from what was seen around it (`docs/routing.md`).
//!
//! `look` gathers [`Evidence`]; [`decide`] turns it into a [`Judgement`] without reading the
//! clock or doing I/O, so the same evidence always gives the same decision.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::Destination;

/// A send needs at least this score from clues alone (recency doesn't count toward it)...
pub const SEND_AT: u32 = 95;
/// ...and this lead over the second candidate.
pub const LEAD: u32 = 15;
/// Below this a candidate isn't worth suggesting.
pub const OFFER_AT: u32 = 40;
/// A runner-up needs this much from clues to be offered one click away.
pub const ALT_AT: u32 = 15;
/// At most this many runners-up are offered beside the best.
const ALTERNATIVES: usize = 2;

/// Everything [`decide`] needs. Plain data, saved as the bundle's `routing.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<Address>,
    #[serde(default)]
    pub clues: Vec<Clue>,
    #[serde(default)]
    pub candidates: Vec<Candidate>,
    /// The capture's end, epoch ms; recency is measured from here.
    pub now_ms: u64,
    /// The browser tab's URL, when there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// A destination someone stated. Obeyed, never weighed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "session")]
pub enum Address {
    /// The user picked it (`--session`, a chip).
    Chosen(String),
    /// The recording carries it (a ScrawlKit timeline's `session`, an earlier routing).
    Recorded(String),
}

impl Clue {
    /// The project this clue names, if any.
    fn directory(&self) -> Option<PathBuf> {
        match &self.pointer {
            Pointer::Session { directory, .. } => directory.clone(),
            Pointer::Folder(folder) => Some(folder.root.clone()),
            Pointer::Words { .. } => None,
        }
    }
}

impl Address {
    pub fn session(&self) -> &str {
        match self {
            Self::Chosen(id) | Self::Recorded(id) => id,
        }
    }
}

/// An observed fact pointing at sessions.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Clue {
    pub source: Source,
    pub pointer: Pointer,
    /// What was seen, for explanations.
    pub note: String,
    /// Full points when this clue is worth more or less than its source usually is (the viewed
    /// session when the capture is about OpenCode's own interface, say).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub points: Option<u32>,
}

impl Clue {
    pub fn new(source: Source, pointer: Pointer, note: impl Into<String>) -> Self {
        Self {
            source,
            pointer,
            note: note.into(),
            points: None,
        }
    }

    pub fn worth(mut self, points: u32) -> Self {
        self.points = Some(points);
        self
    }

    fn full(&self) -> u32 {
        self.points.unwrap_or_else(|| self.source.points())
    }
}

/// Where a clue came from. Each source counts once per candidate.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Source {
    /// A session named at the start of what was said ("tell scrawl …"; `named`). Obeyed ahead
    /// of everything but an address.
    Named,
    /// A `routing.sites` rule covering the tab's URL. Obeyed ahead of weighing when it finds a
    /// session.
    Rule,
    /// A `routing.apps` rule covering the captured app (Jev routing). Obeyed like a site rule.
    AppRule,
    /// The git working tree the captured app's executable runs from (Jev routing): a dev build
    /// names its project like a localhost port does.
    DevBuild,
    /// The tab registered through `/sdk.js`.
    Page,
    /// The process serving the tab's port on this Mac.
    Port,
    /// Who published the file the tab shows, when a server serves it from outside its own
    /// project: the session the page names, or the one whose tool call wrote the file.
    Publisher,
    /// The captured terminal's focused pane.
    Terminal,
    /// The file the tab shows, when it's from outside the server's project: its path's words.
    Document,
    /// The session OpenCode Desktop showed last.
    Viewed,
    /// The captured app: its name (an app being built names its project), and the checkout it's
    /// built from when one is open.
    App,
    /// Words in the window or tab title.
    Title,
    /// Words that were said.
    Speech,
}

impl Source {
    /// The `via` written into `scrawl.json`'s `destination`. The ladder's two keep their names.
    pub fn via(self) -> &'static str {
        match self {
            Self::Page => "sdk",
            Self::Port => "localhost",
            other => other.label(),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Named => "named",
            Self::Rule => "rule",
            Self::AppRule => "app-rule",
            Self::DevBuild => "dev-build",
            Self::Page => "page",
            Self::Port => "port",
            Self::Publisher => "publisher",
            Self::Terminal => "terminal",
            Self::Document => "document",
            Self::Viewed => "viewed",
            Self::App => "app",
            Self::Title => "title",
            Self::Speech => "speech",
        }
    }

    /// Full points for a clue of this source that names a candidate.
    fn points(self) -> u32 {
        match self {
            Self::Named => 100,
            Self::Rule
            | Self::AppRule
            | Self::DevBuild
            | Self::Page
            | Self::Port
            | Self::Publisher => 95,
            Self::Terminal => 70,
            Self::Viewed => 60,
            Self::App => 50,
            Self::Title | Self::Document => 40,
            Self::Speech => 25,
        }
    }
}

/// A page naming its session outright is worth a little more than a folder.
const PAGE_SESSION: u32 = 100;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Pointer {
    /// `directory` is a folder the page also named, kept for the record.
    Session {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        directory: Option<PathBuf>,
    },
    Folder(Folder),
    Words {
        words: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub path: PathBuf,
    /// Its project: the git root, or `path` outside a repository.
    pub root: PathBuf,
    /// Whether sessions anywhere under `root` count (inside a repository).
    pub tree: bool,
    /// The repository's main checkout, shared by its worktrees.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<PathBuf>,
}

/// An open root session.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Candidate {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub directory: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<PathBuf>,
    #[serde(default)]
    pub updated_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub viewed_ms: Option<u64>,
}

impl Candidate {
    pub fn label(&self) -> &str {
        if self.title.is_empty() {
            &self.id
        } else {
            &self.title
        }
    }

    /// The words that name it, and how: its title's words, its folder's or repository's name
    /// when that's one word, and the pieces of its folder's name split on `-`, `_` and `.`.
    fn names(&self) -> BTreeMap<String, Naming> {
        let mut names = BTreeMap::new();
        let mut add = |word: String, naming: Naming| {
            let entry = names.entry(word).or_insert(naming);
            *entry = (*entry).min(naming);
        };
        let title = words(&self.title);
        if let [whole] = title.as_slice() {
            add(whole.clone(), Naming::Whole);
        } else {
            for word in title {
                add(word, Naming::Title);
            }
        }
        let base = |path: &Path| {
            path.file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        for path in [self.directory.as_deref(), self.repo.as_deref()]
            .into_iter()
            .flatten()
        {
            if let [whole] = words(&base(path)).as_slice()
                && base(path).eq_ignore_ascii_case(whole)
            {
                add(whole.clone(), Naming::Whole);
            }
        }
        if let Some(dir) = self.directory.as_deref() {
            // OpenCode's own worktrees get made-up names ("stellar-star"): they name nothing.
            let made_up = dir.components().any(|c| c.as_os_str() == "worktree");
            let worktree = self.repo.as_deref().is_some_and(|repo| repo != dir);
            if !made_up {
                for word in words(&base(dir)) {
                    add(
                        word,
                        if worktree {
                            Naming::Worktree
                        } else {
                            Naming::Folder
                        },
                    );
                }
            }
        }
        names
    }
}

/// How a word names a candidate, strongest first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Naming {
    /// Its whole title, or its folder's or repository's whole name.
    Whole,
    /// A word of its title.
    Title,
    /// A piece of its folder's name.
    Folder,
    /// A piece of a worktree's name, worth half: worktrees are named for a task's words.
    Worktree,
}

impl Naming {
    /// Whether `word` names a candidate this way. Ordinary words ("loading", "search", "chat")
    /// are in names for what the work is about, and said for everything else, so for them only a
    /// whole name counts.
    fn counts(self, word: &str) -> bool {
        self == Self::Whole || !is_common(word)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Policy {
    /// Today's ladder (the app still calls `routing::route_capture` when smart routing is off;
    /// this reproduces it, which the tests check).
    /// The first source with a clue decides, and its newest session wins.
    #[cfg_attr(not(test), allow(dead_code))]
    Ladder,
    /// Weigh every clue and send only when one candidate is clearly ahead.
    Weigh,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Judgement {
    pub decision: Decision,
    /// Supported candidates, best first.
    pub ranking: Vec<Scored>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum Decision {
    /// `instead`: runners-up the undo window offers one click away.
    Send {
        to: Scored,
        #[serde(skip_serializing_if = "Vec::is_empty")]
        instead: Vec<Scored>,
    },
    /// The suggestion first, then its runners-up. Nothing is sent until one is picked.
    Offer {
        choices: Vec<Scored>,
    },
    Keep {
        reason: String,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scored {
    pub candidate: Candidate,
    pub score: u32,
    pub reasons: Vec<Reason>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Reason {
    pub basis: Basis,
    pub points: u32,
    pub why: String,
    /// The project the clue named, recorded as the destination's `directory`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub directory: Option<PathBuf>,
}

/// What a reason rests on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind", content = "source")]
pub enum Basis {
    Chosen,
    Recorded,
    Clue(Source),
    Recent,
    /// Jev's pick (`jev`).
    Jev,
    /// HOME, where captures go when Jev can't tell.
    JevHome,
}

impl Basis {
    fn label(self) -> &'static str {
        match self {
            Self::Chosen => "chosen",
            Self::Recorded => "recorded",
            Self::Clue(source) => source.label(),
            Self::Recent => "recent",
            Self::Jev => "jev",
            Self::JevHome => "jev-home",
        }
    }
}

impl Scored {
    /// The score from clues alone, without recency.
    pub fn clues(&self) -> u32 {
        self.reasons
            .iter()
            .filter(|r| r.basis != Basis::Recent)
            .fold(0, |sum, r| sum.saturating_add(r.points))
    }

    /// The reason that gave the most points (the first on a tie), which names how it was found.
    pub(crate) fn lead(&self) -> Option<&Reason> {
        self.reasons
            .iter()
            .filter(|r| r.basis != Basis::Recent)
            .fold(None, |best: Option<&Reason>, r| match best {
                Some(b) if b.points >= r.points => Some(b),
                _ => Some(r),
            })
    }
}

/// Where the evidence says the capture goes.
pub fn decide(evidence: &Evidence, policy: Policy) -> Judgement {
    if let Some(address) = &evidence.address {
        let to = addressed(evidence, address);
        return Judgement {
            decision: Decision::Send {
                to: to.clone(),
                instead: Vec::new(),
            },
            ranking: vec![to],
        };
    }
    if let Some(judgement) = ruled(evidence, Source::Named) {
        return judgement;
    }
    for source in [Source::Rule, Source::AppRule, Source::DevBuild] {
        if let Some(judgement) = ruled(evidence, source) {
            return judgement;
        }
    }
    match policy {
        Policy::Ladder => ladder(evidence),
        Policy::Weigh => weigh(evidence),
    }
}

fn addressed(evidence: &Evidence, address: &Address) -> Scored {
    let id = address.session();
    let candidate = find(&evidence.candidates, id).unwrap_or_else(|| bare(id));
    let (basis, why) = match address {
        Address::Chosen(_) => (Basis::Chosen, "you chose it"),
        Address::Recorded(_) => (Basis::Recorded, "the recording was addressed to it"),
    };
    Scored {
        candidate,
        score: u32::MAX,
        reasons: vec![Reason {
            basis,
            points: u32::MAX,
            why: why.into(),
            directory: None,
        }],
    }
}

/// The session a named address, a `routing.sites` or `routing.apps` rule, or a dev build
/// (`source`) names, or the newest in its
/// folder (any folder of the folder's repository counts), sent to without weighing. `None` when
/// it names none open.
fn ruled(evidence: &Evidence, source: Source) -> Option<Judgement> {
    let clue = evidence.clues.iter().find(|c| c.source == source)?;
    let candidate = match &clue.pointer {
        Pointer::Session { id, .. } => find(&evidence.candidates, id).unwrap_or_else(|| bare(id)),
        // Sessions in the folder named come before the rest of its project.
        Pointer::Folder(folder) => evidence
            .candidates
            .iter()
            .filter(|c| in_project(c, folder) || same_repo(c, folder))
            .max_by(|a, b| {
                let named = |c: &Candidate| {
                    c.directory
                        .as_deref()
                        .is_some_and(|d| d.starts_with(&folder.path))
                };
                named(a)
                    .cmp(&named(b))
                    .then(a.updated_ms.cmp(&b.updated_ms))
                    .then(b.id.cmp(&a.id))
            })
            .cloned()?,
        Pointer::Words { .. } => return None,
    };
    let to = Scored {
        candidate,
        score: clue.full(),
        reasons: vec![Reason {
            basis: Basis::Clue(source),
            points: clue.full(),
            why: clue.note.clone(),
            directory: clue.directory(),
        }],
    };
    Some(Judgement {
        decision: Decision::Send {
            to: to.clone(),
            instead: Vec::new(),
        },
        ranking: vec![to],
    })
}

fn find(candidates: &[Candidate], id: &str) -> Option<Candidate> {
    candidates.iter().find(|c| c.id == id).cloned()
}

/// A session known only by its ID (not among the candidates read).
fn bare(id: &str) -> Candidate {
    Candidate {
        id: id.to_owned(),
        title: String::new(),
        directory: None,
        repo: None,
        updated_ms: 0,
        viewed_ms: None,
    }
}

// MARK: The ladder

/// The rungs, in order. Only the first source that has a clue is consulted.
const RUNGS: [Source; 2] = [Source::Page, Source::Port];

fn ladder(evidence: &Evidence) -> Judgement {
    let keep = |reason: String| Judgement {
        decision: Decision::Keep { reason },
        ranking: Vec::new(),
    };
    let Some(clue) = RUNGS
        .iter()
        .find_map(|rung| evidence.clues.iter().find(|c| c.source == *rung))
    else {
        return keep("no page registration or local dev server".into());
    };
    let pick = match &clue.pointer {
        Pointer::Session { id, .. } => {
            Some(find(&evidence.candidates, id).unwrap_or_else(|| bare(id)))
        }
        Pointer::Folder(folder) => newest_in(&evidence.candidates, folder),
        Pointer::Words { .. } => None,
    };
    match pick {
        Some(candidate) => {
            let to = Scored {
                candidate,
                score: clue.source.points(),
                reasons: vec![Reason {
                    basis: Basis::Clue(clue.source),
                    points: clue.source.points(),
                    why: clue.note.clone(),
                    directory: clue.directory(),
                }],
            };
            Judgement {
                decision: Decision::Send {
                    to: to.clone(),
                    instead: Vec::new(),
                },
                ranking: vec![to],
            }
        }
        None => keep(match &clue.pointer {
            Pointer::Folder(folder) => {
                format!("no open OpenCode session in {}", folder.root.display())
            }
            _ => clue.note.clone(),
        }),
    }
}

/// The most recently updated candidate in `folder`'s project.
fn newest_in(candidates: &[Candidate], folder: &Folder) -> Option<Candidate> {
    candidates
        .iter()
        .filter(|c| in_project(c, folder))
        .max_by(|a, b| a.updated_ms.cmp(&b.updated_ms).then(b.id.cmp(&a.id)))
        .cloned()
}

pub(crate) fn is_home_or_root(path: &Path) -> bool {
    path == Path::new("/") || dirs::home_dir().is_some_and(|home| path == home)
}

fn in_project(candidate: &Candidate, folder: &Folder) -> bool {
    if is_home_or_root(&folder.root) {
        return false;
    }
    candidate
        .directory
        .as_deref()
        .is_some_and(|dir| dir == folder.root || (folder.tree && dir.starts_with(&folder.root)))
}

// MARK: Weighing

fn weigh(evidence: &Evidence) -> Judgement {
    let candidates = with_named_sessions(evidence);
    let frequency = word_frequency(&candidates);
    // For each folder clue, how close its closest candidate is.
    let closest: Vec<Option<usize>> = evidence
        .clues
        .iter()
        .map(|clue| match &clue.pointer {
            Pointer::Folder(folder) => candidates
                .iter()
                .filter_map(|c| match fit(c, folder) {
                    Fit::Along(d) => Some(d),
                    _ => None,
                })
                .min(),
            _ => None,
        })
        .collect();
    let mut ranking: Vec<Scored> = candidates
        .iter()
        .filter_map(|candidate| score(candidate, evidence, &frequency, &closest))
        .collect();
    // Clues rank; recency only breaks their ties.
    ranking.sort_by(|a, b| {
        b.clues()
            .cmp(&a.clues())
            .then_with(|| b.score.cmp(&a.score))
            .then_with(|| a.candidate.id.cmp(&b.candidate.id))
    });

    let decision = match ranking.as_slice() {
        [] => Decision::Keep {
            reason: if evidence.clues.is_empty() {
                "nothing seen points at a session".into()
            } else {
                "no open session matches what was seen".into()
            },
        },
        [best, rest @ ..] => {
            let lead = best
                .score
                .saturating_sub(rest.iter().map(|s| s.score).max().unwrap_or(0));
            // Recency counts toward the lead between sessions the clues put forward equally, but
            // never toward SEND_AT.
            if best.clues() >= SEND_AT && lead >= LEAD {
                Decision::Send {
                    to: best.clone(),
                    instead: alternatives(&ranking),
                }
            } else if best.score >= OFFER_AT {
                let mut choices = vec![best.clone()];
                choices.extend(alternatives(&ranking));
                Decision::Offer { choices }
            } else {
                Decision::Keep {
                    reason: format!(
                        "the best match, {}, scores {} (an offer needs {OFFER_AT})",
                        best.candidate.label(),
                        best.score
                    ),
                }
            }
        }
    };
    Judgement { decision, ranking }
}

/// The runners-up worth one click: close behind the best, or resting on a different clue than
/// it (the session a misread clue would have meant), one per clue source.
fn alternatives(ranking: &[Scored]) -> Vec<Scored> {
    let Some((best, rest)) = ranking.split_first() else {
        return Vec::new();
    };
    let basis = |s: &Scored| s.lead().map(|r| r.basis);
    let mut picked: Vec<Scored> = Vec::new();
    for s in rest {
        if picked.len() == ALTERNATIVES {
            break;
        }
        let close = best.score.saturating_sub(s.score) < LEAD && s.score >= OFFER_AT;
        let elsewhere = basis(s) != basis(best)
            && s.clues() >= ALT_AT
            && !picked.iter().any(|p| basis(p) == basis(s));
        if close || elsewhere {
            picked.push(s.clone());
        }
    }
    picked
}

/// The candidates, plus a bare one for each session a page or its publisher names that wasn't
/// read.
fn with_named_sessions(evidence: &Evidence) -> Vec<Candidate> {
    let mut candidates = evidence.candidates.clone();
    for clue in &evidence.clues {
        if let (Source::Page | Source::Publisher, Pointer::Session { id, .. }) =
            (clue.source, &clue.pointer)
            && !candidates.iter().any(|c| &c.id == id)
        {
            candidates.push(bare(id));
        }
    }
    candidates
}

/// How many candidates each word names.
fn word_frequency(candidates: &[Candidate]) -> BTreeMap<String, u32> {
    let mut frequency = BTreeMap::new();
    for candidate in candidates {
        for (word, naming) in candidate.names() {
            if naming.counts(&word) {
                *frequency.entry(word).or_insert(0) += 1;
            }
        }
    }
    frequency
}

fn score(
    candidate: &Candidate,
    evidence: &Evidence,
    frequency: &BTreeMap<String, u32>,
    closest: &[Option<usize>],
) -> Option<Scored> {
    // The best clue of each source.
    let mut best: BTreeMap<Source, Reason> = BTreeMap::new();
    let names = candidate.names();
    for (clue, closest) in evidence.clues.iter().zip(closest) {
        let Some((points, why)) = support(clue, candidate, &names, frequency, *closest) else {
            continue;
        };
        if points == 0 {
            continue;
        }
        let better = best
            .get(&clue.source)
            .is_none_or(|r| points > r.points || (points == r.points && why < r.why));
        if better {
            best.insert(
                clue.source,
                Reason {
                    basis: Basis::Clue(clue.source),
                    points,
                    why,
                    directory: clue.directory(),
                },
            );
        }
    }
    if best.is_empty() {
        return None;
    }
    let mut reasons: Vec<Reason> = best.into_values().collect();
    reasons.sort_by(|a, b| b.points.cmp(&a.points));
    if let Some((points, why)) = recency(candidate, evidence.now_ms) {
        reasons.push(Reason {
            basis: Basis::Recent,
            points,
            why,
            directory: None,
        });
    }
    Some(Scored {
        candidate: candidate.clone(),
        score: reasons.iter().map(|r| r.points).sum(),
        reasons,
    })
}

/// The points `clue` gives `candidate`, and why.
fn support(
    clue: &Clue,
    candidate: &Candidate,
    names: &BTreeMap<String, Naming>,
    frequency: &BTreeMap<String, u32>,
    closest: Option<usize>,
) -> Option<(u32, String)> {
    match &clue.pointer {
        Pointer::Session { id, .. } if *id == candidate.id => {
            let points = match (clue.source, clue.points) {
                (_, Some(points)) => points,
                (Source::Page, None) => PAGE_SESSION,
                (source, None) => source.points(),
            };
            Some((points, clue.note.clone()))
        }
        Pointer::Session { .. } => None,
        Pointer::Folder(folder) => {
            let full = clue.full();
            match fit(candidate, folder) {
                Fit::Along(d) => {
                    let further = d.saturating_sub(closest.unwrap_or(d));
                    let points = full.saturating_sub(FURTHER * further as u32).max(full / 2);
                    let why = if further == 0 {
                        clue.note.clone()
                    } else {
                        format!(
                            "{} ({further} folder{} further than the closest session)",
                            clue.note,
                            if further == 1 { "" } else { "s" }
                        )
                    };
                    Some((points, why))
                }
                Fit::Aside => Some((
                    full / 2,
                    format!("{} (elsewhere in the same project)", clue.note),
                )),
                Fit::Repo => Some((
                    full / 2,
                    format!("{} (another checkout of the same repository)", clue.note),
                )),
                Fit::None => None,
            }
        }
        Pointer::Words { words } => {
            let (points, word, n, naming) = words
                .iter()
                .filter_map(|w| {
                    let naming = *names.get(w)?;
                    naming.counts(w).then_some(())?;
                    let n = frequency.get(w).copied().unwrap_or(1).max(1);
                    let points = clue.full() / n;
                    let points = if naming == Naming::Worktree {
                        points / 2
                    } else {
                        points
                    };
                    Some((points, w, n, naming))
                })
                .max_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(a.1)))?;
            let share = if n == 1 {
                "only this session".to_owned()
            } else {
                format!("{n} sessions")
            };
            let how = match naming {
                Naming::Worktree => " (in its worktree's name, half)",
                Naming::Whole | Naming::Title | Naming::Folder => "",
            };
            Some((
                points,
                format!("{}: \"{word}\" names {share}{how}", clue.note),
            ))
        }
    }
}

/// Points a folder clue loses for each folder between a session and the clue's folder, beyond
/// the closest session's distance.
const FURTHER: u32 = 15;

/// How a session's directory sits relative to a folder clue.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fit {
    /// On the line from the project root to the folder, or inside the folder: this many
    /// folders away from it.
    Along(usize),
    /// Elsewhere in the project.
    Aside,
    /// In another checkout of the same repository.
    Repo,
    None,
}

fn fit(candidate: &Candidate, folder: &Folder) -> Fit {
    if in_project(candidate, folder) {
        let dir = candidate.directory.as_deref().unwrap_or(Path::new("/"));
        let depth = |p: &Path| p.components().count();
        if folder.path.starts_with(dir) {
            Fit::Along(depth(&folder.path) - depth(dir))
        } else if dir.starts_with(&folder.path) {
            Fit::Along(depth(dir) - depth(&folder.path))
        } else {
            Fit::Aside
        }
    } else if same_repo(candidate, folder) {
        Fit::Repo
    } else {
        Fit::None
    }
}

fn same_repo(candidate: &Candidate, folder: &Folder) -> bool {
    match (&candidate.repo, &folder.repo) {
        (Some(a), Some(b)) => a == b && !is_home_or_root(a),
        _ => false,
    }
}

/// Points for having been active shortly before the capture ended. Activity well after it (a
/// bundle explained later, against today's sessions) says nothing about that moment.
fn recency(candidate: &Candidate, now_ms: u64) -> Option<(u32, String)> {
    const MINUTE: u64 = 60_000;
    let active = [Some(candidate.updated_ms), candidate.viewed_ms]
        .into_iter()
        .flatten()
        .filter(|ms| *ms > 0 && *ms <= now_ms + MINUTE)
        .max()?;
    let ago = now_ms.saturating_sub(active);
    let points = match ago {
        a if a <= 10 * MINUTE => 20,
        a if a <= 60 * MINUTE => 12,
        a if a <= 24 * 60 * MINUTE => 5,
        _ => return None,
    };
    Some((
        points,
        format!("active {} before the capture ended", ago_text(ago)),
    ))
}

pub fn ago_text(ms: u64) -> String {
    let s = ms / 1000;
    match s {
        0..60 => format!("{s} s"),
        60..3600 => format!("{} min", s / 60),
        _ if s < 172_800 => format!("{} h", s / 3600),
        _ => format!("{} days", s / 86_400),
    }
}

// MARK: Words

/// Words too common to name a session, whitespace-separated.
const STOP: &str = "the and for but not you your with this that these those there here what when where which who \
     why how just like yeah okay um uh also really maybe should would could can will its are was \
     were been have has had does did doing get got make makes made see look looks thing things \
     some all any one two too very much more most now then than into onto from out off over under \
     about again still even kind sort think know want need going way right left new old app fix \
     fixes work worktree main test tests feat pass they them our let lets bit lot little good \
     nice better weird mean probably actually because something everything page window tab \
     session sessions review preview index html www com http https localhost src code open source \
     users kit only down each different next none quite lots whole often around starts start \
     feels feel sometimes happen happens gonna wanna guys dont doesnt isnt cant wont didnt thats \
     whats theres youre shit fuck damn same other another first last every while after before \
     through being point part parts show shows says said tell take give keep put try use used \
     using time times day back able sure yes delete read write free tier settings setting";

/// Ordinary words, and the words of software and its interfaces: in a folder's name they say what
/// the folder does ("opencode-ios-oc380-loading"), so saying one names nothing. A session's title
/// or a whole folder name still counts.
const COMMON: &str = "able above across add added adding after again against ago ahead all almost alone \
     along already alright always amount answer anyway anything anywhere appear apply area arent \
     around ask asked asking away awesome bad bar base basic basically become begin behind below \
     best between big black blank block blue body boring both bottom box break bring broke broken \
     brown build building built busy button buttons call called calls care case cases center change \
     changed changes changing check checked checking choose clean clear clearly click clicked \
     clicking close closed closes come comes coming common complete completely component components \
     content control controls copy correct could couldnt couple crazy current cut dark data dead \
     deal default definitely design detail details didnt display does doing done double draw drawn \
     drop easy edge edit else empty end enough entire error errors etc exactly example except \
     expected extra fact fail failed fair fake far fast faster feature features fill final finally \
     find fine finish fixed flash flashing flicker focus focused follow font force form forward full \
     fun function funny further gets give given glass goes gone goodness gray great green grid group \
     guess half hand handle hard header hell help hidden hide high hold hope hover huge idea ideas \
     ill image images important inside instead issue issues item items keep kinds label large later \
     layout least less level light line lines list little live load loaded loader loading local long \
     loop lose lost loud low mad maybe menu message messages middle might mind minute missing mode \
     move moved moving multiple must name names near nearly need never next nice nothing number \
     obviously odd okay once ones open opened opening option options order others otherwise outside \
     own pad padding panel past pause perfect pick picked picker picks piece place placed places \
     plain play please plus polish pop popup possible pretty problem problems pull push quick quickly \
     random rather ready real reason red remove removed rename render rendering reset rest result \
     return run running safe save saved scroll scrolling search searching second section seems seen \
     select selected send sent set sets shape short should side simple since single size slow small \
     smooth sorry space spacing special spinner spot stack state stay step stop stuff style super \
     support switch system take taken talk text that their theme thats theyre thing think third \
     though thought three tiny title today together top total touch toward track true turn type \
     types ugly under understand unless until upon usual value view visible wait walk wanted wants \
     ways weve white whatever whether whole wide will without wonder word words working works world \
     worse worst wrong yet zero having chat library model models server audit check typo quick \
     game engine infinite algorithm screenshots";

fn is_common(word: &str) -> bool {
    COMMON.split_whitespace().any(|common| common == word)
}

/// Lowercased words of three letters or more that aren't on the stop list or numbers.
pub fn words(text: &str) -> Vec<String> {
    let mut seen = BTreeSet::new();
    // "don't" is one word.
    let text = text.replace(['\'', '’'], "");
    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 3)
        .filter(|w| !w.chars().all(|c| c.is_ascii_digit()))
        .filter(|w| !STOP.split_whitespace().any(|stop| stop == w))
        .filter(|w| seen.insert(w.clone()))
        .collect()
}

// MARK: Effects

/// The `destination` a send records, or `None` when nothing is sent.
#[cfg_attr(not(test), allow(dead_code))]
pub fn destination(judgement: &Judgement, url: Option<&str>) -> Option<Destination> {
    let Decision::Send { to, .. } = &judgement.decision else {
        return None;
    };
    Some(destination_of(to, url))
}

/// The `destination` sending to `to` records, `via` the reason that gave it the most points.
pub fn destination_of(to: &Scored, url: Option<&str>) -> Destination {
    let lead = to.lead();
    let via = match lead.map(|r| r.basis) {
        Some(Basis::Chosen) => "chosen",
        Some(Basis::Recorded) => "recorded",
        Some(Basis::Clue(source)) => source.via(),
        Some(Basis::Jev) => "jev",
        Some(Basis::JevHome) => "jev-home",
        Some(Basis::Recent) | None => "weighed",
    };
    Destination {
        session: to.candidate.id.clone(),
        title: to.candidate.label().to_owned(),
        via: via.to_owned(),
        url: url.map(str::to_owned),
        directory: lead
            .and_then(|r| r.directory.as_ref())
            .map(|d| d.display().to_string()),
    }
}

// MARK: Explaining

/// A plain-text account of the evidence, the scores and the decision.
pub fn explain(evidence: &Evidence, judgement: &Judgement) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    match &evidence.address {
        Some(Address::Chosen(id)) => {
            let _ = writeln!(out, "Address   chosen: {id}");
        }
        Some(Address::Recorded(id)) => {
            let _ = writeln!(out, "Address   recorded: {id}");
        }
        None => {
            let _ = writeln!(out, "Address   none");
        }
    }
    if evidence.clues.is_empty() {
        let _ = writeln!(out, "Clues     none");
    } else {
        let _ = writeln!(out, "Clues");
        for clue in &evidence.clues {
            let target = match &clue.pointer {
                Pointer::Session { id, .. } => format!("session {id}"),
                Pointer::Folder(f) if f.tree => format!("project {}", f.root.display()),
                Pointer::Folder(f) => format!("folder {}", f.root.display()),
                Pointer::Words { words } => {
                    let shown: Vec<&str> = words.iter().take(12).map(String::as_str).collect();
                    let more = words.len().saturating_sub(shown.len());
                    if more > 0 {
                        format!("words {} (+{more})", shown.join(", "))
                    } else {
                        format!("words {}", shown.join(", "))
                    }
                }
            };
            let _ = writeln!(out, "  {:<9} {} → {target}", clue.source.label(), clue.note);
        }
    }
    let _ = writeln!(
        out,
        "Sessions  {} read, {} supported",
        evidence.candidates.len(),
        judgement.ranking.len()
    );
    for scored in judgement.ranking.iter().take(6) {
        let score = if scored.score == u32::MAX {
            "  ∞".to_owned()
        } else {
            format!("{:>4}", scored.score)
        };
        let dir = scored
            .candidate
            .directory
            .as_ref()
            .map(|d| format!("  {}", d.display()))
            .unwrap_or_default();
        let _ = writeln!(
            out,
            "{score}  {}  {}{dir}",
            scored.candidate.label(),
            scored.candidate.id
        );
        for reason in &scored.reasons {
            let label = reason.basis.label();
            let points = if reason.points == u32::MAX {
                String::new()
            } else {
                format!("+{} ", reason.points)
            };
            let _ = writeln!(out, "        {points}{label} {}", reason.why);
        }
    }
    let _ = match &judgement.decision {
        Decision::Send { to, instead } => {
            let _ = writeln!(
                out,
                "Decision  send to {} ({})",
                to.candidate.label(),
                to.candidate.id
            );
            if instead.is_empty() {
                Ok(())
            } else {
                let names: Vec<&str> = instead.iter().map(|c| c.candidate.label()).collect();
                writeln!(
                    out,
                    "          one click away while it waits: {}",
                    names.join(", ")
                )
            }
        }
        Decision::Offer { choices } => {
            let best = choices.first();
            let rest: Vec<&str> = choices
                .iter()
                .skip(1)
                .map(|c| c.candidate.label())
                .collect();
            let _ = writeln!(
                out,
                "Decision  ask: send to {}? ({})",
                best.map_or("?", |c| c.candidate.label()),
                best.map_or_else(String::new, |b| why_not_sent(
                    b,
                    choices,
                    &judgement.ranking
                ))
            );
            if rest.is_empty() {
                Ok(())
            } else {
                writeln!(out, "          or one click: {}", rest.join(", "))
            }
        }
        Decision::Keep { reason } => writeln!(out, "Decision  clipboard only: {reason}"),
    };
    out
}

/// Why an offered best wasn't sent, in the explanation's words.
fn why_not_sent(best: &Scored, choices: &[Scored], ranking: &[Scored]) -> String {
    let clues = best.clues();
    if clues < SEND_AT {
        let recent = best.score - clues;
        return if recent > 0 {
            format!(
                "its clues give {clues}, a send needs {SEND_AT} from clues; recency only breaks ties"
            )
        } else {
            format!("its clues give {clues}, a send needs {SEND_AT}")
        };
    }
    let second = ranking.get(1).or_else(|| choices.get(1));
    match second {
        Some(s) if s.clues() > clues => {
            format!(
                "{} has stronger clues ({} to {clues})",
                s.candidate.label(),
                s.clues()
            )
        }
        Some(s) => format!(
            "{} is {} behind; a send needs a lead of {LEAD}",
            s.candidate.label(),
            best.score.saturating_sub(s.score)
        ),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: u64 = 1_800_000_000_000;
    const MIN: u64 = 60_000;

    fn cand(id: &str, title: &str, dir: &str, ago_min: u64) -> Candidate {
        Candidate {
            id: id.into(),
            title: title.into(),
            directory: Some(dir.into()),
            repo: None,
            updated_ms: NOW - ago_min * MIN,
            viewed_ms: None,
        }
    }

    fn folder(source: Source, root: &str) -> Clue {
        Clue {
            source,
            pointer: Pointer::Folder(Folder {
                path: root.into(),
                root: root.into(),
                tree: true,
                repo: None,
            }),
            note: format!("{source:?} in {root}"),
            points: None,
        }
    }

    fn named(source: Source, id: &str) -> Clue {
        Clue {
            source,
            pointer: Pointer::Session {
                id: id.into(),
                directory: None,
            },
            note: format!("{source:?} names {id}"),
            points: None,
        }
    }

    fn said(source: Source, text: &str) -> Clue {
        Clue {
            source,
            pointer: Pointer::Words { words: words(text) },
            note: format!("{source:?}"),
            points: None,
        }
    }

    fn sessions() -> Vec<Candidate> {
        vec![
            cand("ses_scrawl", "SCRAWL", "/p/scrawl", 2),
            cand("ses_ios", "OPENCODE IOS", "/p/opencode-ios", 5),
            cand("ses_console", "CONSOLE VORTEX", "/p/opencode-console", 30),
            cand("ses_old", "SCRAWL OLD", "/p/scrawl", 60 * 24 * 5),
        ]
    }

    fn ev(clues: Vec<Clue>) -> Evidence {
        Evidence {
            address: None,
            clues,
            candidates: sessions(),
            now_ms: NOW,
            url: None,
        }
    }

    fn sent_to(j: &Judgement) -> Option<&str> {
        match &j.decision {
            Decision::Send { to, .. } => Some(to.candidate.id.as_str()),
            _ => None,
        }
    }

    fn offered(j: &Judgement) -> Vec<&str> {
        match &j.decision {
            Decision::Offer { choices } => {
                choices.iter().map(|c| c.candidate.id.as_str()).collect()
            }
            _ => Vec::new(),
        }
    }

    #[test]
    fn a_site_rule_sends_ahead_of_weighing() {
        // The tab's title names CONSOLE; the rule names the folder SCRAWL works in.
        let mut e = ev(vec![
            folder(Source::Rule, "/p/scrawl"),
            said(Source::Title, "OpenCode Console · Problems"),
            said(Source::Speech, "console vortex"),
        ]);
        for policy in [Policy::Ladder, Policy::Weigh] {
            let j = decide(&e, policy);
            assert_eq!(sent_to(&j), Some("ses_scrawl"));
            let Decision::Send { to, .. } = &j.decision else {
                unreachable!()
            };
            let dest = destination_of(to, Some("https://problems.example.com/tasks/1"));
            assert_eq!(dest.via, "rule");
            assert_eq!(dest.directory.as_deref(), Some("/p/scrawl"));
        }
        // A session ID is obeyed even when it wasn't read.
        e.clues[0] = named(Source::Rule, "ses_pinned");
        assert_eq!(sent_to(&decide(&e, Policy::Weigh)), Some("ses_pinned"));
        // A folder no open session works in leaves it to weighing.
        e.clues[0] = folder(Source::Rule, "/p/nobody");
        let j = decide(&e, Policy::Weigh);
        assert_ne!(sent_to(&j), Some("ses_scrawl"));
        // The user's own choice still beats a rule.
        e.clues[0] = folder(Source::Rule, "/p/scrawl");
        e.address = Some(Address::Chosen("ses_ios".into()));
        assert_eq!(sent_to(&decide(&e, Policy::Weigh)), Some("ses_ios"));
    }

    #[test]
    fn app_rules_and_dev_builds_send_ahead_of_weighing() {
        for (source, via) in [
            (Source::AppRule, "app-rule"),
            (Source::DevBuild, "dev-build"),
        ] {
            let mut e = ev(vec![
                folder(source, "/p/scrawl"),
                said(Source::Title, "OpenCode Console · Problems"),
            ]);
            let j = decide(&e, Policy::Weigh);
            assert_eq!(sent_to(&j), Some("ses_scrawl"), "{source:?}");
            let Decision::Send { to, .. } = &j.decision else {
                unreachable!()
            };
            assert_eq!(destination_of(to, None).via, via);
            assert!(crate::routing::policy::obeyed(to));
            // A site rule comes first.
            e.clues.push(named(Source::Rule, "ses_pinned"));
            assert_eq!(sent_to(&decide(&e, Policy::Weigh)), Some("ses_pinned"));
        }
        // An app rule's session ID is obeyed as is.
        let e = ev(vec![named(Source::AppRule, "ses_pinned")]);
        assert_eq!(sent_to(&decide(&e, Policy::Weigh)), Some("ses_pinned"));
    }

    #[test]
    fn a_site_rule_folder_reaches_its_repositorys_worktrees() {
        let mut worktree = cand("ses_wt", "SEER", "/elsewhere/team-board-task", 1);
        worktree.repo = Some("/p/team-board".into());
        let mut e = ev(vec![Clue {
            source: Source::Rule,
            pointer: Pointer::Folder(Folder {
                path: "/p/team-board".into(),
                root: "/p/team-board".into(),
                tree: true,
                repo: Some("/p/team-board".into()),
            }),
            note: "rule".into(),
            points: None,
        }]);
        e.candidates.push(worktree);
        assert_eq!(sent_to(&decide(&e, Policy::Weigh)), Some("ses_wt"));
        // A session in the very folder named beats a newer one elsewhere in its project.
        e.candidates
            .push(cand("ses_here", "HERE", "/p/team-board/docs", 90));
        if let Pointer::Folder(folder) = &mut e.clues[0].pointer {
            folder.path = "/p/team-board/docs".into();
        }
        assert_eq!(sent_to(&decide(&e, Policy::Weigh)), Some("ses_here"));
    }

    #[test]
    fn an_address_always_wins() {
        let mut e = ev(vec![
            named(Source::Page, "ses_ios"),
            folder(Source::Port, "/p/scrawl"),
        ]);
        e.address = Some(Address::Recorded("ses_console".into()));
        for policy in [Policy::Ladder, Policy::Weigh] {
            assert_eq!(sent_to(&decide(&e, policy)), Some("ses_console"));
        }
        // An unknown session is still obeyed.
        e.address = Some(Address::Chosen("ses_gone".into()));
        let j = decide(&e, Policy::Weigh);
        assert_eq!(sent_to(&j), Some("ses_gone"));
        assert_eq!(destination(&j, None).unwrap().via, "chosen");
    }

    #[test]
    fn a_page_or_port_sends_on_its_own() {
        let j = decide(&ev(vec![named(Source::Page, "ses_ios")]), Policy::Weigh);
        assert_eq!(sent_to(&j), Some("ses_ios"));
        assert_eq!(destination(&j, Some("http://x")).unwrap().via, "sdk");
        // The port names /p/scrawl: the recent session wins over the five-day-old one.
        let j = decide(&ev(vec![folder(Source::Port, "/p/scrawl")]), Policy::Weigh);
        assert_eq!(sent_to(&j), Some("ses_scrawl"));
        assert_eq!(destination(&j, None).unwrap().via, "localhost");
    }

    #[test]
    fn two_close_sessions_are_offered_not_sent() {
        let mut e = ev(vec![folder(Source::Port, "/p/scrawl")]);
        e.candidates[3].updated_ms = NOW - 3 * MIN;
        let j = decide(&e, Policy::Weigh);
        assert_eq!(sent_to(&j), None);
        assert_eq!(offered(&j), vec!["ses_old", "ses_scrawl"]);
        // The ladder sends to the newest regardless.
        assert_eq!(sent_to(&decide(&e, Policy::Ladder)), Some("ses_scrawl"));
    }

    #[test]
    fn whats_on_screen_is_a_hint_not_an_address() {
        let mut e = ev(vec![named(Source::Viewed, "ses_ios")]);
        e.candidates[1].viewed_ms = Some(NOW);
        let j = decide(&e, Policy::Weigh);
        assert_eq!(offered(&j), vec!["ses_ios"]);
        // Saying its name and having just used it still only asks: recency never lifts a
        // session to SEND_AT.
        e.clues
            .push(said(Source::Speech, "the iOS composer feels slow"));
        let j = decide(&e, Policy::Weigh);
        assert_eq!(j.ranking[0].score, 60 + 25 + 20);
        assert_eq!(offered(&j), vec!["ses_ios"]);
        // Clues that reach it on their own send.
        e.clues.push(said(Source::Title, "OpenCode iOS"));
        assert_eq!(sent_to(&decide(&e, Policy::Weigh)), Some("ses_ios"));
    }

    #[test]
    fn ordinary_words_dont_match_pieces_of_folder_names() {
        let mut e = ev(vec![said(
            Source::Speech,
            "when I search, loading, the flashing",
        )]);
        e.candidates.push(Candidate {
            repo: Some("/p/opencode-ios".into()),
            ..cand("ses_wt", "OPENCODE IOS", "/p/opencode-ios-oc380-loading", 1)
        });
        assert!(decide(&e, Policy::Weigh).ranking.is_empty());
        // A whole title still counts; a word of a longer one doesn't.
        e.candidates
            .push(cand("ses_states", "LOADING STATES", "/p/states", 1));
        assert!(decide(&e, Policy::Weigh).ranking.is_empty());
        e.candidates
            .push(cand("ses_loading", "LOADING", "/p/load", 1));
        let j = decide(&e, Policy::Weigh);
        assert_eq!(j.ranking.len(), 1);
        assert_eq!(j.ranking[0].candidate.id, "ses_loading");
        // A distinctive piece of a worktree's name counts for half.
        let e = Evidence {
            clues: vec![said(Source::Speech, "the oc380 spinner")],
            ..e
        };
        let j = decide(&e, Policy::Weigh);
        assert_eq!(j.ranking[0].candidate.id, "ses_wt");
        assert_eq!(j.ranking[0].clues(), 25 / 2);
        // OpenCode's made-up worktree names name nothing.
        let mut e = ev(vec![said(Source::Speech, "a lucky rocket")]);
        e.candidates.push(cand(
            "ses_made_up",
            "AUDIT",
            "/data/opencode/worktree/012780/lucky-rocket",
            1,
        ));
        assert!(decide(&e, Policy::Weigh).ranking.is_empty());
    }

    #[test]
    fn recency_only_breaks_ties_between_sessions_the_clues_put_forward() {
        // A port names two sessions equally: the recent one is sent to.
        let j = decide(&ev(vec![folder(Source::Port, "/p/scrawl")]), Policy::Weigh);
        assert_eq!(sent_to(&j), Some("ses_scrawl"));
        // A stronger clue for a stale session isn't overtaken by a weaker one's recency.
        let mut e = ev(vec![
            named(Source::Page, "ses_old"),
            folder(Source::Port, "/p/opencode-ios"),
        ]);
        e.candidates[3].directory = Some("/p/elsewhere".into());
        let j = decide(&e, Policy::Weigh);
        assert_eq!(j.ranking[0].candidate.id, "ses_old");
        assert_eq!(sent_to(&j), None);
        assert_eq!(offered(&j), vec!["ses_old", "ses_ios"]);
    }

    #[test]
    fn an_offer_keeps_the_runner_up_from_another_clue_one_click_away() {
        let mut e = ev(vec![
            folder(Source::App, "/p/scrawl"),
            named(Source::Viewed, "ses_ios"),
        ]);
        e.clues[0].points = Some(70);
        e.clues[1].points = Some(15);
        let j = decide(&e, Policy::Weigh);
        assert_eq!(offered(&j), vec!["ses_scrawl", "ses_ios"]);
        // Another session behind the same clue at half isn't offered.
        assert!(!offered(&j).contains(&"ses_old"));
        // A send keeps its runner-up for the undo window.
        e.clues[0].points = None;
        e.clues[0].source = Source::Port;
        let j = decide(&e, Policy::Weigh);
        match &j.decision {
            Decision::Send { to, instead } => {
                assert_eq!(to.candidate.id, "ses_scrawl");
                let ids: Vec<&str> = instead.iter().map(|s| s.candidate.id.as_str()).collect();
                assert_eq!(ids, vec!["ses_ios"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_medium_clue_alone_never_sends() {
        for source in [
            Source::Terminal,
            Source::Document,
            Source::Viewed,
            Source::App,
            Source::Title,
            Source::Speech,
        ] {
            let clue = match source {
                Source::App | Source::Title | Source::Speech => said(source, "scrawl"),
                Source::Viewed => named(source, "ses_scrawl"),
                _ => folder(source, "/p/scrawl"),
            };
            let mut e = ev(vec![clue]);
            e.candidates.truncate(3);
            assert_eq!(sent_to(&decide(&e, Policy::Weigh)), None, "{source:?}");
        }
    }

    #[test]
    fn recency_never_nominates() {
        let j = decide(&ev(vec![folder(Source::Port, "/p/nowhere")]), Policy::Weigh);
        assert!(j.ranking.is_empty());
        assert!(matches!(j.decision, Decision::Keep { .. }));
        let j = decide(&ev(Vec::new()), Policy::Weigh);
        assert!(matches!(j.decision, Decision::Keep { .. }));
    }

    #[test]
    fn decisions_ignore_order_and_repeats() {
        let clues = vec![
            folder(Source::Terminal, "/p/opencode-ios"),
            said(Source::Speech, "composer in opencode ios"),
            named(Source::Viewed, "ses_ios"),
            said(Source::Title, "Scrawl"),
        ];
        let base = decide(&ev(clues.clone()), Policy::Weigh);
        let mut reversed = ev(clues.iter().rev().cloned().collect());
        reversed.candidates.reverse();
        assert_eq!(decide(&reversed, Policy::Weigh), base);
        let mut doubled = clues.clone();
        doubled.extend(clues);
        assert_eq!(decide(&ev(doubled), Policy::Weigh), base);
    }

    #[test]
    fn support_never_hurts_the_leader() {
        let mut e = ev(vec![folder(Source::Port, "/p/opencode-ios")]);
        assert_eq!(sent_to(&decide(&e, Policy::Weigh)), Some("ses_ios"));
        for extra in [
            named(Source::Viewed, "ses_ios"),
            said(Source::Speech, "ios"),
            folder(Source::Terminal, "/p/opencode-ios"),
        ] {
            e.clues.push(extra);
            assert_eq!(sent_to(&decide(&e, Policy::Weigh)), Some("ses_ios"));
        }
    }

    #[test]
    fn home_is_not_a_project() {
        let home = dirs::home_dir().unwrap();
        let mut e = ev(vec![folder(Source::Port, &home.to_string_lossy())]);
        e.candidates
            .push(cand("ses_home", "HOME", &home.to_string_lossy(), 1));
        assert!(decide(&e, Policy::Weigh).ranking.is_empty());
        assert!(
            decide(&ev(vec![folder(Source::Port, "/")]), Policy::Weigh)
                .ranking
                .is_empty()
        );
    }

    #[test]
    fn rare_words_count_more_than_common_ones() {
        let e = ev(vec![said(Source::Speech, "opencode console is broken")]);
        let j = decide(&e, Policy::Weigh);
        // "console" names one session (25 + 5 recency); "opencode" names two.
        assert_eq!(j.ranking[0].candidate.id, "ses_console");
        assert_eq!(j.ranking[0].score, 25 + 12);
        assert!(
            matches!(j.decision, Decision::Offer { .. })
                || matches!(j.decision, Decision::Keep { .. })
        );
    }

    #[test]
    fn the_session_closest_to_the_served_folder_wins() {
        // A dev server in a subfolder of a big repository with many sessions at its root.
        let mut e = ev(Vec::new());
        e.candidates = vec![
            cand("ses_root_a", "HOME", "/h", 3),
            cand("ses_root_b", "NOTES", "/h", 4),
            cand("ses_motion", "KIT MOTION", "/h/research/motion", 30),
            cand("ses_tools", "TOOLS", "/h/tools", 1),
        ];
        e.clues.push(Clue {
            source: Source::Port,
            pointer: Pointer::Folder(Folder {
                path: "/h/research/motion".into(),
                root: "/h".into(),
                tree: true,
                repo: Some("/h".into()),
            }),
            note: "port 5203".into(),
            points: None,
        });
        let j = decide(&e, Policy::Weigh);
        assert_eq!(sent_to(&j), Some("ses_motion"));
        let points = |id: &str| {
            j.ranking
                .iter()
                .find(|s| s.candidate.id == id)
                .unwrap()
                .score
        };
        assert_eq!(points("ses_motion"), 95 + 12);
        assert_eq!(points("ses_root_a"), 65 + 20);
        assert_eq!(points("ses_tools"), 47 + 20);
        // With only root sessions, the root is the closest and sends as today.
        e.candidates.retain(|c| c.id == "ses_root_a");
        assert_eq!(sent_to(&decide(&e, Policy::Weigh)), Some("ses_root_a"));
    }

    #[test]
    fn another_checkout_of_the_repo_gets_half() {
        let mut e = ev(Vec::new());
        e.candidates[0].repo = Some("/p/scrawl".into());
        e.clues.push(Clue {
            source: Source::Port,
            pointer: Pointer::Folder(Folder {
                path: "/w/scrawl-routing/site".into(),
                root: "/w/scrawl-routing".into(),
                tree: true,
                repo: Some("/p/scrawl".into()),
            }),
            note: "port 5173".into(),
            points: None,
        });
        let j = decide(&e, Policy::Weigh);
        assert_eq!(offered(&j), vec!["ses_scrawl"]);
        assert_eq!(j.ranking[0].score, 47 + 20);
    }

    #[test]
    fn the_ladder_takes_the_first_rung_with_a_clue() {
        // A page folder with no session doesn't fall through to the port.
        let e = ev(vec![
            folder(Source::Port, "/p/scrawl"),
            folder(Source::Page, "/p/none"),
        ]);
        let j = decide(&e, Policy::Ladder);
        assert!(matches!(&j.decision, Decision::Keep { reason } if reason.contains("/p/none")));
        // Other sources are ignored.
        let e = ev(vec![named(Source::Viewed, "ses_ios")]);
        assert!(matches!(
            decide(&e, Policy::Ladder).decision,
            Decision::Keep { .. }
        ));
    }

    #[test]
    fn words_skip_filler_and_keep_names() {
        assert_eq!(
            words("Why don't we fix the Scrawl pill, who knows? OpenCode iOS 26"),
            vec!["scrawl", "pill", "knows", "opencode", "ios"]
        );
    }

    #[test]
    fn evidence_round_trips_through_json() {
        let mut e = ev(vec![
            folder(Source::Port, "/p/scrawl"),
            named(Source::Page, "ses_ios"),
            said(Source::Speech, "hello scrawl"),
        ]);
        e.address = Some(Address::Recorded("ses_ios".into()));
        let json = serde_json::to_string(&e).unwrap();
        assert_eq!(serde_json::from_str::<Evidence>(&json).unwrap(), e);
    }
}
