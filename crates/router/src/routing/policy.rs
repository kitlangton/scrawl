//! Who may send on their own (`routing.autoSend`), the apps no rule may name, and the rules
//! Scrawl offers once picks agree (`router/learned.json`). See `docs/routing.md`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::jev::{Band, Record};
use super::smart::{Basis, Candidate, Decision, Judgement, Scored, Source};

// MARK: Auto-send

/// What may send without asking (`"routing": {"autoSend": …}`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutoSend {
    /// Only deterministic sources ([`obeyed`]); Jev's pick and smart weighing suggest.
    #[default]
    Rules,
    /// Jev's send band too, and smart routing's weighed sends, after the undo countdown.
    Confident,
    /// Nothing but an address: rules only order the list.
    Never,
}

impl AutoSend {
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "rules" => Some(Self::Rules),
            "confident" => Some(Self::Confident),
            "never" => Some(Self::Never),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Rules => "rules",
            Self::Confident => "confident",
            Self::Never => "never",
        }
    }

    /// The host's setting ([`RouterConfig::auto_send`](crate::RouterConfig)).
    pub fn configured() -> Self {
        crate::env::get().config.auto_send
    }

    /// Whether a send to `to` may go on its own.
    fn sends(self, to: &Scored) -> bool {
        match self {
            Self::Confident => true,
            Self::Rules => obeyed(to),
            Self::Never => addressed(to),
        }
    }
}

/// Whether a send rests on a source that's obeyed rather than guessed: an address (a choice, a
/// recorded destination), a session named at the start, a `routing.sites` or `routing.apps`
/// rule, a signed page or its publisher, the localhost port, or a dev build's checkout.
pub fn obeyed(to: &Scored) -> bool {
    matches!(
        to.lead().map(|r| r.basis),
        Some(
            Basis::Chosen
                | Basis::Recorded
                | Basis::Clue(
                    Source::Named
                        | Source::Rule
                        | Source::AppRule
                        | Source::Page
                        | Source::Port
                        | Source::Publisher
                        | Source::DevBuild
                )
        )
    )
}

fn addressed(to: &Scored) -> bool {
    matches!(
        to.lead().map(|r| r.basis),
        Some(Basis::Chosen | Basis::Recorded)
    )
}

/// The most sessions a suggestion lists.
const LISTED: usize = super::jev::CHIPS;

/// `judgement` under `mode`: a send that may not go on its own becomes an offer with what it
/// would have sent first (pre-selected), then its runners-up, the rest of the ranking and HOME.
/// Returns whether it was turned into a suggestion.
pub fn apply(mode: AutoSend, judgement: Judgement, home: Option<&Candidate>) -> (Judgement, bool) {
    let Decision::Send { to, instead } = &judgement.decision else {
        return (judgement, false);
    };
    if mode.sends(to) {
        return (judgement, false);
    }
    let mut choices: Vec<Scored> = Vec::new();
    let fresh = |choices: &[Scored], id: &str| !choices.iter().any(|s| s.candidate.id == id);
    for s in std::iter::once(to).chain(instead).chain(
        judgement
            .ranking
            .iter()
            .filter(|s| s.clues() > 0 || s.score > 0),
    ) {
        if choices.len() < LISTED && fresh(&choices, &s.candidate.id) {
            choices.push(s.clone());
        }
    }
    if let Some(home) = home.filter(|h| fresh(&choices, &h.id)) {
        if choices.len() == LISTED {
            choices.pop();
        }
        choices.push(Scored {
            candidate: home.clone(),
            score: 0,
            reasons: vec![super::smart::Reason {
                basis: Basis::JevHome,
                points: 0,
                why: "HOME".into(),
                directory: None,
            }],
        });
    }
    (
        Judgement {
            decision: Decision::Offer { choices },
            ranking: judgement.ranking,
        },
        true,
    )
}

/// [`apply`] to a decision Jev or smart routing made, noting a suggestion in `record`: its
/// band becomes `ask`, `would` keeps the band it had (`send` or `fallback`) and `suggested`
/// is set, so the eval can count what would have been sent.
pub fn suggest(
    mode: AutoSend,
    judgement: Judgement,
    record: &mut Record,
    candidates: &[Candidate],
) -> Judgement {
    let home = super::jev::home_session(candidates);
    let (judgement, suggested) = apply(mode, judgement, home.as_ref());
    if suggested {
        record.suggested = true;
        record.would = Some(record.band);
        record.band = Band::Ask;
        record.chips = super::jev::chips_of(&judgement.decision);
        record.reason = format!(
            "{} · suggested, autoSend is {}",
            record.reason,
            mode.label()
        );
    }
    judgement
}

// MARK: General-purpose apps

/// Apps whose captures are about anything: no `routing.apps` rule may name them, and Scrawl
/// never offers one. An entry covers its own bundle ID and any below it (`ai.opencode.desktop`
/// covers `ai.opencode.desktop.beta`).
pub const GENERAL_APPS: &[&str] = &[
    // Browsers
    "com.apple.safari",
    "com.apple.safaritechnologypreview",
    "com.google.chrome",
    "com.brave.browser",
    "company.thebrowser.browser",
    "com.microsoft.edgemac",
    "com.vivaldi.vivaldi",
    "org.chromium.chromium",
    "org.mozilla.firefox",
    "com.operasoftware.opera",
    "app.zen-browser.zen",
    // Terminals
    "com.apple.terminal",
    "com.mitchellh.ghostty",
    "com.googlecode.iterm2",
    "dev.warp.warp-stable",
    "net.kovidgoyal.kitty",
    "org.alacritty",
    "com.github.wez.wezterm",
    // OpenCode, and the Mac's own
    "ai.opencode.desktop",
    "com.apple.finder",
    "com.apple.mobilesms",
    "com.apple.mail",
    "com.tinyspeck.slackmacgap",
];

/// Whether `bundle_id` is, falls under or covers a general-purpose app (a rule for `com.apple`
/// would cover Finder).
pub fn is_general_app(bundle_id: &str) -> bool {
    let id = bundle_id.trim().to_ascii_lowercase();
    if id.is_empty() {
        return false;
    }
    GENERAL_APPS.iter().any(|g| {
        id == *g
            || id.strip_prefix(g).is_some_and(|rest| rest.starts_with('.'))
            || g.strip_prefix(id.as_str())
                .is_some_and(|rest| rest.starts_with('.'))
    })
}

/// The `routing.apps` rule covering `bundle_id`, the longest key first: its key and target.
pub fn app_rule(bundle_id: &str, apps: &[(String, PathBuf)]) -> Option<(String, PathBuf)> {
    let id = bundle_id.trim().to_ascii_lowercase();
    if id.is_empty() || is_general_app(&id) {
        return None;
    }
    apps.iter()
        .filter(|(key, _)| {
            let key = key.trim().to_ascii_lowercase();
            !key.is_empty()
                && !is_general_app(&key)
                && (id == key || id.strip_prefix(&key).is_some_and(|r| r.starts_with('.')))
        })
        .max_by_key(|(key, _)| key.len())
        .cloned()
}

// MARK: Dev builds

/// How many folders above an executable are checked for `.git`.
const DEV_BUILD_DEPTH: usize = 12;

/// The git working tree an app's executable sits in (`~/code/scrawl/target/release/scrawl` →
/// `~/code/scrawl`), found by checking parents for `.git` (no processes). Never the home folder
/// or `/`.
pub fn dev_build_root(executable: &Path) -> Option<PathBuf> {
    let mut current = executable.parent();
    for _ in 0..DEV_BUILD_DEPTH {
        let dir = current?;
        if super::smart::is_home_or_root(dir) {
            return None;
        }
        if dir.join(".git").exists() {
            return Some(dir.to_path_buf());
        }
        current = dir.parent();
    }
    None
}

/// The executable of a running process.
pub fn executable_of(pid: i32) -> Option<PathBuf> {
    if pid <= 0 {
        return None;
    }
    crate::util::process_executable(pid)
}

// MARK: Rule offers

/// What a rule would cover: a site (a URL's origin) or an app (a bundle ID).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RuleKind {
    Site,
    App,
}

impl RuleKind {
    fn config_key(self) -> &'static str {
        match self {
            Self::Site => "sites",
            Self::App => "apps",
        }
    }
}

/// A rule Scrawl offers once two picks agree: the route result's `offer_rule`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OfferRule {
    pub kind: RuleKind,
    pub key: String,
    pub folder: String,
}

/// The rule key a pick teaches: the tab's origin (never a loopback one), else the app's bundle
/// ID (never a general-purpose app's).
pub fn rule_key(url: Option<&str>, bundle_id: Option<&str>) -> Option<(RuleKind, String)> {
    if let Some(parsed) = url.and_then(super::parse_url) {
        // A dev server's port already says where it goes, and the next one gets another port.
        return (!parsed.is_loopback()).then(|| (RuleKind::Site, parsed.origin()));
    }
    let id = bundle_id.map(str::trim).filter(|id| !id.is_empty())?;
    (!is_general_app(id)).then(|| (RuleKind::App, id.to_owned()))
}

/// The folder a rule for `candidate` would name: its repository's main checkout, else its
/// project. Never the home folder.
pub fn rule_folder(candidate: &Candidate) -> Option<PathBuf> {
    match (&candidate.repo, &candidate.directory) {
        (Some(repo), _) => Some(repo.clone()).filter(|f| !super::smart::is_home_or_root(f)),
        (None, Some(dir)) => rule_folder_of(dir),
        (None, None) => None,
    }
}

/// [`rule_folder`] for a session folder alone.
pub fn rule_folder_of(dir: &Path) -> Option<PathBuf> {
    let folder = super::look::folder_of(dir);
    let folder = folder.repo.unwrap_or(folder.root);
    (!super::smart::is_home_or_root(&folder)).then_some(folder)
}

/// Picks counted per key, in `learned.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Learned {
    #[serde(default)]
    picks: BTreeMap<String, Picks>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Picks {
    folder: String,
    count: u32,
}

/// Picks that must agree before a rule is offered.
const AGREEING: u32 = 2;

/// A pick [`learn`] counted (`learned.json`'s key and folder), so an undo can take it back.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counted {
    pub key: String,
    pub folder: String,
}

/// Counts a pick of `folder` for `key` in `file` and returns the rule to offer once
/// [`AGREEING`] picks in a row agree. Nothing is offered for a key that already has a rule
/// (`ruled`) or that Kit refused (`refused`).
#[cfg(test)]
pub fn learn(
    file: &Path,
    kind: RuleKind,
    key: &str,
    folder: &Path,
    refused: &[String],
    ruled: bool,
) -> Result<Option<OfferRule>> {
    Ok(learn_counted(file, kind, key, folder, refused, ruled)?.1)
}

/// [`learn`], also saying what it counted.
pub fn learn_counted(
    file: &Path,
    kind: RuleKind,
    key: &str,
    folder: &Path,
    refused: &[String],
    ruled: bool,
) -> Result<(Option<Counted>, Option<OfferRule>)> {
    if ruled || refused.iter().any(|r| r.trim() == key) {
        return Ok((None, None));
    }
    if kind == RuleKind::App && is_general_app(key) {
        return Ok((None, None));
    }
    let mut learned: Learned = std::fs::read(file)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default();
    let folder = folder.display().to_string();
    let counted = Counted {
        key: format!("{}:{key}", config_kind(kind)),
        folder: folder.clone(),
    };
    let entry = learned
        .picks
        .entry(counted.key.clone())
        .or_insert_with(|| Picks {
            folder: folder.clone(),
            count: 0,
        });
    if entry.folder == folder {
        entry.count += 1;
    } else {
        *entry = Picks {
            folder: folder.clone(),
            count: 1,
        };
    }
    let count = entry.count;
    write_learned(file, &learned)?;
    let offer = (count >= AGREEING).then(|| OfferRule {
        kind,
        key: key.to_owned(),
        folder,
    });
    Ok((Some(counted), offer))
}

fn write_learned(file: &Path, learned: &Learned) -> Result<()> {
    if let Some(parent) = file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(learned)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, file)?;
    Ok(())
}

/// Takes back a pick an undone send counted: one fewer for its key, while the key still
/// points at the same folder. Whether it changed anything.
pub fn unlearn(file: &Path, counted: &Counted) -> Result<bool> {
    let Some(mut learned) = std::fs::read(file)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Learned>(&bytes).ok())
    else {
        return Ok(false);
    };
    let Some(entry) = learned.picks.get_mut(&counted.key) else {
        return Ok(false);
    };
    if entry.folder != counted.folder || entry.count == 0 {
        return Ok(false);
    }
    entry.count -= 1;
    if entry.count == 0 {
        learned.picks.remove(&counted.key);
    }
    write_learned(file, &learned)?;
    Ok(true)
}

/// [`unlearn`] in the learned file; errors are logged.
pub fn unlearn_pick(counted: &Counted) {
    let Some(file) = learned_file() else { return };
    if let Err(err) = unlearn(&file, counted) {
        crate::env::log(format!("routing: couldn't take back a pick: {err:#}"));
    }
}

fn config_kind(kind: RuleKind) -> &'static str {
    match kind {
        RuleKind::Site => "site",
        RuleKind::App => "app",
    }
}

/// The host's learned-picks file ([`RouterDirs::learned`](crate::RouterDirs)).
pub fn learned_file() -> Option<PathBuf> {
    crate::env::get().dirs.learned.clone()
}

/// Whether config already has a rule for `key`.
pub fn has_rule(kind: RuleKind, key: &str) -> bool {
    let config = &crate::env::get().config;
    let rules = match kind {
        RuleKind::Site => config.site_rules(),
        RuleKind::App => config.app_rules(),
    };
    rules
        .iter()
        .any(|(k, _)| k.trim().eq_ignore_ascii_case(key.trim()))
}

/// Counts a pick of `folder` (a [`rule_folder`]) for `url` or `bundle_id` in the learned file,
/// with config's refusals and rules: what it counted, and the rule to offer, when there is one.
/// Errors are logged.
pub fn learn_pick(
    url: Option<&str>,
    bundle_id: Option<&str>,
    folder: &Path,
) -> (Option<Counted>, Option<OfferRule>) {
    let Some((kind, key)) = rule_key(url, bundle_id) else {
        return (None, None);
    };
    let Some(file) = learned_file() else {
        return (None, None);
    };
    let refused = crate::env::get().config.refused_rules.clone();
    match learn_counted(&file, kind, &key, folder, &refused, has_rule(kind, &key)) {
        Ok((counted, offer)) => {
            if let Some(offer) = &offer {
                crate::env::log(format!(
                    "routing: picks agree, offering a {} rule",
                    config_kind(offer.kind)
                ));
            }
            (counted, offer)
        }
        Err(err) => {
            crate::env::log(format!("routing: couldn't count a pick: {err:#}"));
            (None, None)
        }
    }
}

// MARK: Writing rules

/// A rule's key and target, checked: an app rule needs a bundle ID that isn't a
/// general-purpose app's, and a folder (other than `~/…`) is made absolute against the working
/// directory and must exist. A `ses_` ID stays as it is.
pub fn rule_target(kind: RuleKind, key: &str, target: &str) -> Result<(String, String)> {
    let key = key.trim();
    let target = target.trim();
    ensure!(!key.is_empty(), "the rule needs a key");
    ensure!(!target.is_empty(), "the rule needs a folder or session");
    if kind == RuleKind::App {
        ensure!(
            !key.contains('/') && !key.contains(' '),
            "{key:?} isn't a bundle ID"
        );
        if is_general_app(key) {
            bail!("{key} is a general-purpose app; its captures are about anything, so no rule")
        }
    }
    let target = if target.starts_with("ses_") || target.starts_with('~') {
        target.to_owned()
    } else {
        let path = PathBuf::from(target);
        let path = if path.is_absolute() {
            path
        } else {
            std::env::current_dir()?.join(path)
        };
        path.canonicalize()
            .with_context(|| format!("{} doesn't exist", path.display()))?
            .display()
            .to_string()
    };
    Ok((key.to_owned(), target))
}

/// `scrawl router rule add`: `routing.sites` or `routing.apps` in Scrawl's `routing` object
/// gets `key` → `target` (see [`rule_target`]), other keys kept. A refusal of `key` is lifted.
/// The host saves it.
pub fn add_rule(
    routing: &mut serde_json::Map<String, Value>,
    kind: RuleKind,
    key: &str,
    target: &str,
) -> Result<()> {
    let (key, target) = rule_target(kind, key, target)?;
    let rules = routing
        .entry(kind.config_key())
        .or_insert_with(|| json!({}));
    if !rules.is_object() {
        *rules = json!({});
    }
    rules[key.as_str()] = json!(target);
    unrefuse(routing, &key);
    Ok(())
}

/// `scrawl router rule refuse`: never offer a rule for `key` again (`routing.refusedRules` in
/// Scrawl's `routing` object). The host saves it.
pub fn refuse_rule(routing: &mut serde_json::Map<String, Value>, key: &str) -> Result<()> {
    let key = key.trim();
    ensure!(
        !key.is_empty(),
        "give the site or bundle ID to stop offering"
    );
    let list = routing.entry("refusedRules").or_insert_with(|| json!([]));
    if !list.is_array() {
        *list = json!([]);
    }
    let list = list.as_array_mut().expect("an array");
    if !list.iter().any(|v| v.as_str() == Some(key)) {
        list.push(json!(key));
    }
    Ok(())
}

fn unrefuse(routing: &mut serde_json::Map<String, Value>, key: &str) {
    if let Some(Value::Array(list)) = routing.get_mut("refusedRules") {
        list.retain(|v| v.as_str() != Some(key));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::routing::smart::Reason;

    fn candidate(id: &str, dir: &str) -> Candidate {
        Candidate {
            id: id.into(),
            title: id.to_uppercase(),
            directory: Some(PathBuf::from(dir)),
            repo: None,
            updated_ms: 0,
            viewed_ms: None,
        }
    }

    fn scored(id: &str, basis: Basis, points: u32) -> Scored {
        Scored {
            candidate: candidate(id, &format!("/tmp/{id}")),
            score: points,
            reasons: vec![Reason {
                basis,
                points,
                why: String::new(),
                directory: None,
            }],
        }
    }

    fn send(to: Scored, instead: Vec<Scored>, ranking: Vec<Scored>) -> Judgement {
        Judgement {
            decision: Decision::Send { to, instead },
            ranking,
        }
    }

    fn offered(j: &Judgement) -> Vec<&str> {
        match &j.decision {
            Decision::Offer { choices } => {
                choices.iter().map(|c| c.candidate.id.as_str()).collect()
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn auto_send_reads_its_three_words() {
        assert_eq!(AutoSend::parse(" Rules"), Some(AutoSend::Rules));
        assert_eq!(AutoSend::parse("confident"), Some(AutoSend::Confident));
        assert_eq!(AutoSend::parse("never"), Some(AutoSend::Never));
        assert_eq!(AutoSend::parse("always"), None);
        assert_eq!(AutoSend::default(), AutoSend::Rules);
    }

    #[test]
    fn under_rules_a_guess_suggests_and_a_rule_sends() {
        let home = candidate("ses_home", "/tmp/home");
        let jev = send(
            scored("ses_a", Basis::Jev, 95),
            vec![scored("ses_b", Basis::Jev, 3)],
            vec![scored("ses_c", Basis::Clue(Source::Terminal), 70)],
        );
        let (j, suggested) = apply(AutoSend::Rules, jev.clone(), Some(&home));
        assert!(suggested);
        assert_eq!(offered(&j), ["ses_a", "ses_b", "ses_c", "ses_home"]);
        let (j, suggested) = apply(AutoSend::Confident, jev, Some(&home));
        assert!(!suggested);
        assert!(matches!(j.decision, Decision::Send { .. }));
        for source in [
            Source::Named,
            Source::Rule,
            Source::AppRule,
            Source::Page,
            Source::Port,
            Source::Publisher,
            Source::DevBuild,
        ] {
            let rule = send(scored("ses_r", Basis::Clue(source), 95), vec![], vec![]);
            let (j, suggested) = apply(AutoSend::Rules, rule.clone(), Some(&home));
            assert!(!suggested, "{source:?}");
            assert!(matches!(j.decision, Decision::Send { .. }));
            // Never: the rule only goes first.
            let (j, suggested) = apply(AutoSend::Never, rule, Some(&home));
            assert!(suggested, "{source:?}");
            assert_eq!(offered(&j), ["ses_r", "ses_home"]);
        }
        let weighed = send(
            scored("ses_w", Basis::Clue(Source::Terminal), 99),
            vec![],
            vec![],
        );
        assert!(apply(AutoSend::Rules, weighed, None).1);
        // An address always sends.
        let chosen = send(scored("ses_x", Basis::Chosen, u32::MAX), vec![], vec![]);
        assert!(!apply(AutoSend::Never, chosen, Some(&home)).1);
    }

    #[test]
    fn a_suggestion_lists_at_most_four_with_home_last() {
        let home = candidate("ses_home", "/tmp/home");
        let ranking: Vec<Scored> = ["a", "b", "c", "d", "e"]
            .iter()
            .map(|id| scored(&format!("ses_{id}"), Basis::Clue(Source::Title), 40))
            .collect();
        let j = send(ranking[0].clone(), vec![], ranking);
        let (j, _) = apply(AutoSend::Rules, j, Some(&home));
        assert_eq!(offered(&j), ["ses_a", "ses_b", "ses_c", "ses_home"]);
    }

    #[test]
    fn general_purpose_apps_never_get_rules() {
        for id in [
            "com.brave.Browser",
            "com.google.Chrome.canary",
            "com.mitchellh.ghostty",
            "ai.opencode.desktop.beta",
            "com.apple.finder",
            "com.apple.MobileSMS",
            "com.apple.mail",
            "com.tinyspeck.slackmacgap",
            "com.apple",
            "ai.opencode",
        ] {
            assert!(is_general_app(id), "{id}");
        }
        assert!(!is_general_app("com.example.mytool"));
        assert!(!is_general_app("com.apple.safarix"));
        let apps = vec![
            ("com.example".to_owned(), PathBuf::from("/code/example")),
            ("com.example.tool".to_owned(), PathBuf::from("/code/tool")),
            (
                "ai.opencode.desktop".to_owned(),
                PathBuf::from("/code/opencode"),
            ),
        ];
        assert_eq!(
            app_rule("com.example.tool.beta", &apps).map(|(k, _)| k),
            Some("com.example.tool".to_owned())
        );
        assert_eq!(
            app_rule("com.example.other", &apps).unwrap().0,
            "com.example"
        );
        assert_eq!(app_rule("ai.opencode.desktop", &apps), None);
        assert_eq!(app_rule("com.examples", &apps), None);
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "scrawl-policy-{name}-{}",
            crate::util::make_bundle_id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_dev_build_is_found_in_its_working_tree() {
        let base = temp("dev");
        let exe = base.join("repo/target/release/tool");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::create_dir_all(base.join("repo/.git")).unwrap();
        std::fs::write(&exe, b"").unwrap();
        assert_eq!(dev_build_root(&exe), Some(base.join("repo")));
        let clue = crate::routing::look::dev_build_clue(Some("com.example.tool"), &exe).unwrap();
        assert_eq!(clue.source, Source::DevBuild);
        // A browser or terminal built from source is still about anything.
        assert!(
            crate::routing::look::dev_build_clue(Some("com.mitchellh.ghostty"), &exe).is_none()
        );
        let elsewhere = base.join("apps/Tool.app/Contents/MacOS/tool");
        std::fs::create_dir_all(elsewhere.parent().unwrap()).unwrap();
        assert_eq!(dev_build_root(&elsewhere), None);
        assert_eq!(dev_build_root(Path::new("/tool")), None);
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn the_second_agreeing_pick_offers_a_rule() {
        let base = temp("learn");
        let file = base.join("learned.json");
        let folder = Path::new("/code/board");
        let site = "https://board.example.com";
        let learn = |folder: &Path, refused: &[String], ruled: bool| {
            learn(&file, RuleKind::Site, site, folder, refused, ruled).unwrap()
        };
        assert_eq!(learn(folder, &[], false), None);
        let offer = learn(folder, &[], false).unwrap();
        assert_eq!(
            serde_json::to_value(&offer).unwrap(),
            json!({"kind": "site", "key": site, "folder": "/code/board"})
        );
        // A different folder starts over.
        assert_eq!(learn(Path::new("/code/other"), &[], false), None);
        assert_eq!(
            learn(Path::new("/code/other"), &[], false).unwrap().folder,
            "/code/other"
        );
        // Refused or already a rule: never offered.
        assert_eq!(
            learn(Path::new("/code/other"), &[site.to_owned()], false),
            None
        );
        assert_eq!(learn(Path::new("/code/other"), &[], true), None);
        // General-purpose apps are never learned.
        assert_eq!(
            super::learn(&file, RuleKind::App, "com.apple.finder", folder, &[], false).unwrap(),
            None
        );
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn an_undone_send_takes_its_pick_back() {
        let base = temp("unlearn");
        let file = base.join("learned.json");
        let folder = Path::new("/code/board");
        let site = "https://board.example.com";
        let (counted, offer) =
            learn_counted(&file, RuleKind::Site, site, folder, &[], false).unwrap();
        assert_eq!(offer, None);
        let counted = counted.unwrap();
        assert_eq!(counted.key, format!("site:{site}"));
        assert!(unlearn(&file, &counted).unwrap());
        assert!(
            !unlearn(&file, &counted).unwrap(),
            "nothing left to take back"
        );
        // So the next pick is the first again, not the second.
        assert_eq!(
            learn(&file, RuleKind::Site, site, folder, &[], false).unwrap(),
            None
        );
        // A key since pointed elsewhere is left alone.
        learn(
            &file,
            RuleKind::Site,
            site,
            Path::new("/code/other"),
            &[],
            false,
        )
        .unwrap();
        assert!(!unlearn(&file, &counted).unwrap());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn rule_keys_are_a_tabs_origin_or_a_specific_app() {
        assert_eq!(
            rule_key(
                Some("https://board.example.com/x/y?z=1"),
                Some("com.brave.Browser")
            ),
            Some((RuleKind::Site, "https://board.example.com".to_owned()))
        );
        assert_eq!(
            rule_key(None, Some("com.example.tool")),
            Some((RuleKind::App, "com.example.tool".to_owned()))
        );
        assert_eq!(rule_key(None, Some("com.mitchellh.ghostty")), None);
        let home = dirs::home_dir().unwrap();
        let mut c = candidate("ses_h", &home.display().to_string());
        assert_eq!(rule_folder(&c), None, "never the home folder");
        c.repo = Some(PathBuf::from("/code/main"));
        assert_eq!(rule_folder(&c), Some(PathBuf::from("/code/main")));
    }

    #[test]
    fn rules_are_written_into_config_keeping_the_rest() {
        let mut routing =
            json!({"aliases": {"scroll": "SCRAWL"}, "refusedRules": ["https://x.example"]})
                .as_object()
                .unwrap()
                .clone();
        add_rule(
            &mut routing,
            RuleKind::Site,
            "https://x.example",
            "~/code/x",
        )
        .unwrap();
        add_rule(&mut routing, RuleKind::App, "com.example.tool", "ses_abc").unwrap();
        assert!(add_rule(&mut routing, RuleKind::App, "com.brave.Browser", "~/code/x").is_err());
        refuse_rule(&mut routing, "com.example.other").unwrap();
        refuse_rule(&mut routing, "com.example.other").unwrap();
        assert_eq!(routing["aliases"]["scroll"], "SCRAWL");
        assert_eq!(routing["sites"]["https://x.example"], "~/code/x");
        assert_eq!(routing["apps"]["com.example.tool"], "ses_abc");
        assert_eq!(routing["refusedRules"], json!(["com.example.other"]));
    }
}
