//! What the host tells the router: its settings ([`RouterConfig`]) and where it keeps its files
//! ([`RouterDirs`]). The router never reads a config file or picks a folder of its own.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::routing::policy::{self, AutoSend, RuleKind};

/// How a prompt enters a busy session: `steer` joins its current turn, `queue` waits for it
/// to end.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeliveryMode {
    #[default]
    Steer,
    Queue,
}

impl DeliveryMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Steer => "steer",
            Self::Queue => "queue",
        }
    }
}

/// The router's settings. Scrawl keeps them in `~/.config/scrawl/config.json`
/// ([`parse_scrawl_config`] reads that file's text); a host may keep them anywhere.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RouterConfig {
    /// What may send without asking (`routing.autoSend`).
    pub auto_send: AutoSend,
    /// How long after a send an undo still takes it back, in seconds (`routing.undoWindowSec`,
    /// default 600).
    pub undo_window_sec: Option<u64>,
    /// URL prefix → folder (`~` allowed) or `ses_` ID: where a site's captures always go
    /// (`routing.sites`).
    pub sites: BTreeMap<String, String>,
    /// Bundle ID (or a prefix of one) → folder or `ses_` ID (`routing.apps`). General-purpose
    /// apps are ignored whatever this says.
    pub apps: BTreeMap<String, String>,
    /// URL prefix → the folder a file server serves (`routing.servers`).
    pub servers: BTreeMap<String, String>,
    /// Spoken name → the title or folder it means (`routing.aliases`).
    pub aliases: BTreeMap<String, String>,
    /// Sites and bundle IDs never offered a rule again (`routing.refusedRules`).
    pub refused_rules: Vec<String>,
    /// `delivery`.
    pub delivery: DeliveryMode,
    /// The `smartRouting` experiment: weigh the evidence of a capture rather than follow a
    /// dev site's port alone.
    pub smart_routing: bool,
    /// The `jevRouting` experiment: ask Jev (with `smart_routing`).
    pub jev_routing: bool,
    /// Jev's thresholds and options (`jevRouting` when it's an object).
    pub jev: Option<Value>,
    /// The OpenCode key Jev is called with; absent uses `$OPENCODE_API_KEY` or OpenCode's own
    /// credential (`jevApiKey`).
    pub jev_api_key: Option<String>,
    /// The SSH host whose OpenCode server holds the sessions, when it isn't this Mac
    /// (`agentHost`).
    pub agent_host: Option<String>,
    /// The `opencode2` command on the agent host (`agentOpencode`, default `opencode2`).
    pub agent_opencode: Option<String>,
    /// Whether a capture of a dev site goes straight to its session (top-level `autoSend`,
    /// default on).
    pub send_captures: bool,
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            auto_send: AutoSend::default(),
            undo_window_sec: None,
            sites: BTreeMap::new(),
            apps: BTreeMap::new(),
            servers: BTreeMap::new(),
            aliases: BTreeMap::new(),
            refused_rules: Vec::new(),
            delivery: DeliveryMode::default(),
            smart_routing: false,
            jev_routing: false,
            jev: None,
            jev_api_key: None,
            agent_host: None,
            agent_opencode: None,
            send_captures: true,
        }
    }
}

impl RouterConfig {
    /// `sites`, `~` expanded.
    pub fn site_rules(&self) -> Vec<(String, PathBuf)> {
        expand(&self.sites)
    }

    /// `apps`, `~` expanded, without general-purpose apps.
    pub fn app_rules(&self) -> Vec<(String, PathBuf)> {
        let mut apps = expand(&self.apps);
        apps.retain(|(id, _)| !policy::is_general_app(id));
        apps
    }

    /// `servers`, `~` expanded.
    pub fn server_folders(&self) -> Vec<(String, PathBuf)> {
        expand(&self.servers)
    }

    pub fn alias_list(&self) -> Vec<(String, String)> {
        self.aliases
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// `agent_host` when it's safe to pass to `ssh`.
    pub fn agent_host(&self) -> Option<&str> {
        self.agent_host.as_deref().filter(|h| safe_word(h))
    }

    /// The `opencode2` command on the agent host.
    pub fn agent_opencode(&self) -> &str {
        self.agent_opencode
            .as_deref()
            .filter(|c| safe_word(c))
            .unwrap_or("opencode2")
    }

    /// Whether Jev is asked: both experiments on.
    pub fn jev_enabled(&self) -> bool {
        self.smart_routing && self.jev_routing
    }

    /// `rule add`: this config with `key` → `target` in `sites` or `apps` (a folder, `~` kept, or
    /// a `ses_` ID; a relative folder is made absolute against the working directory). A
    /// refusal of `key` is lifted. The host saves it.
    pub fn with_rule(&self, kind: RuleKind, key: &str, target: &str) -> Result<Self> {
        let (key, target) = policy::rule_target(kind, key, target)?;
        let mut next = self.clone();
        match kind {
            RuleKind::Site => next.sites.insert(key.clone(), target),
            RuleKind::App => next.apps.insert(key.clone(), target),
        };
        next.refused_rules.retain(|k| *k != key);
        Ok(next)
    }

    /// `rule refuse`: this config with `key` never offered a rule again. The host saves it.
    pub fn with_refused(&self, key: &str) -> Result<Self> {
        let key = key.trim();
        ensure!(
            !key.is_empty(),
            "give the site or bundle ID to stop offering"
        );
        let mut next = self.clone();
        if !next.refused_rules.iter().any(|k| k == key) {
            next.refused_rules.push(key.to_owned());
        }
        Ok(next)
    }
}

/// Safe to put unquoted in a remote shell command (host names, `~/bin/tool` paths).
pub fn safe_word(text: &str) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.@/~".contains(&b))
}

fn expand(map: &BTreeMap<String, String>) -> Vec<(String, PathBuf)> {
    let home = dirs::home_dir().unwrap_or_default();
    map.iter()
        .map(|(id, path)| {
            let path = match path.strip_prefix("~/") {
                Some(rest) => home.join(rest),
                None => PathBuf::from(path),
            };
            (id.clone(), path)
        })
        .collect()
}

/// The router's settings in the text of Scrawl's `config.json`. Lenient like Scrawl: a key (or a
/// `sites`/`apps` entry) that doesn't fit is dropped on its own; the rest stand. Unreadable
/// text is the defaults.
pub fn parse_scrawl_config(text: &str) -> RouterConfig {
    parse_scrawl_config_with_warnings(text).0
}

/// [`parse_scrawl_config`] and what it ignored, one line each.
pub fn parse_scrawl_config_with_warnings(text: &str) -> (RouterConfig, Vec<String>) {
    let mut warnings = Vec::new();
    let Ok(Value::Object(root)) = serde_json::from_str::<Value>(text) else {
        return (RouterConfig::default(), warnings);
    };
    (router_config_from_value(&root, &mut warnings), warnings)
}

/// [`parse_scrawl_config`] from the parsed file.
pub fn router_config_from_scrawl(
    root: &serde_json::Map<String, Value>,
    warnings: &mut Vec<String>,
) -> RouterConfig {
    router_config_from_value(root, warnings)
}

fn router_config_from_value(
    root: &serde_json::Map<String, Value>,
    warnings: &mut Vec<String>,
) -> RouterConfig {
    let mut config = RouterConfig::default();
    let experiments = experiments(root.get("experiments"), warnings);
    config.smart_routing = experiments
        .get(crate::routing::SMART_ROUTING)
        .copied()
        .unwrap_or(false);
    config.jev_routing = experiments
        .get(crate::routing::jev::JEV_ROUTING)
        .copied()
        .unwrap_or(false);
    match root.get("delivery") {
        None | Some(Value::Null) => {}
        Some(Value::String(s)) if s == "steer" => {}
        Some(Value::String(s)) if s == "queue" => config.delivery = DeliveryMode::Queue,
        Some(other) => warnings.push(format!(
            "config.json: delivery {other} isn't \"steer\" or \"queue\"; steering"
        )),
    }
    if let Some(value) = root.get("jevRouting").filter(|v| !v.is_null()) {
        config.jev = Some(value.clone());
    }
    config.jev_api_key = text(root, "jevApiKey", warnings);
    config.agent_host = text(root, "agentHost", warnings);
    config.agent_opencode = text(root, "agentOpencode", warnings);
    match root.get("autoSend") {
        None | Some(Value::Null) => {}
        Some(Value::Bool(on)) => config.send_captures = *on,
        Some(other) => warnings.push(format!(
            "config.json: ignoring \"autoSend\": {other} (expected true or false)"
        )),
    }
    match root.get("routing") {
        None | Some(Value::Null) => {}
        Some(Value::Object(routing)) => read_routing(routing, &mut config, warnings),
        Some(other) => warnings.push(format!(
            "config.json: ignoring \"routing\": {other} (expected an object)"
        )),
    }
    config
}

fn text(
    root: &serde_json::Map<String, Value>,
    key: &str,
    warnings: &mut Vec<String>,
) -> Option<String> {
    match root.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Null => None,
        other => {
            warnings.push(format!(
                "config.json: ignoring \"{key}\": {other} (expected text)"
            ));
            None
        }
    }
}

/// `true`/`false`, `"yes"`/`"on"`/`"1"` and their opposites, or a number (non-zero is on).
fn experiments(value: Option<&Value>, warnings: &mut Vec<String>) -> BTreeMap<String, bool> {
    let mut out = BTreeMap::new();
    let map = match value {
        None | Some(Value::Null) => return out,
        Some(Value::Object(map)) => map,
        Some(other) => {
            warnings.push(format!(
                "config.json: ignoring \"experiments\": {other} (expected an object)"
            ));
            return out;
        }
    };
    for (name, value) in map {
        match experiment_value(value) {
            Some(on) => {
                out.insert(name.clone(), on);
            }
            None => warnings.push(format!(
                "config.json: ignoring experiment \"{name}\": {value} (expected true or false)"
            )),
        }
    }
    out
}

/// An experiment's value as on or off; `None` when it's neither.
pub fn experiment_value(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(b) => Some(*b),
        Value::Number(n) => n.as_f64().map(|n| n != 0.0),
        Value::String(s) => match s.trim().to_ascii_lowercase().as_str() {
            "true" | "yes" | "on" | "1" => Some(true),
            "false" | "no" | "off" | "0" | "" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn read_routing(
    routing: &serde_json::Map<String, Value>,
    config: &mut RouterConfig,
    warnings: &mut Vec<String>,
) {
    for (key, value) in routing {
        match key.as_str() {
            "sites" => config.sites = map_of(key, value, warnings),
            "apps" => {
                config.apps = map_of(key, value, warnings);
                config.apps.retain(|id, _| {
                    let general = policy::is_general_app(id);
                    if general {
                        warnings.push(format!(
                            "config.json: ignoring routing.apps \"{id}\": a general-purpose app's captures are about anything"
                        ));
                    }
                    !general
                });
            }
            "servers" => config.servers = map_of(key, value, warnings),
            "aliases" => config.aliases = map_of(key, value, warnings),
            "autoSend" => match value.as_str().and_then(AutoSend::parse) {
                Some(mode) => config.auto_send = mode,
                None => warnings.push(format!(
                    "config.json: ignoring routing.autoSend {value} (expected \"rules\", \"confident\" or \"never\")"
                )),
            },
            "undoWindowSec" => match value.as_u64() {
                Some(sec) => config.undo_window_sec = Some(sec),
                None => warnings.push(format!(
                    "config.json: ignoring \"routing.undoWindowSec\": {value} (expected whole seconds)"
                )),
            },
            "refusedRules" => match value {
                Value::Array(list) => {
                    config.refused_rules = list
                        .iter()
                        .filter_map(|v| v.as_str().map(str::to_owned))
                        .collect();
                }
                other => warnings.push(format!(
                    "config.json: ignoring \"routing.refusedRules\": {other} (expected a list)"
                )),
            },
            _ => {}
        }
    }
}

/// A map of text, without the entries that aren't text.
fn map_of(key: &str, value: &Value, warnings: &mut Vec<String>) -> BTreeMap<String, String> {
    let Value::Object(entries) = value else {
        warnings.push(format!(
            "config.json: ignoring \"routing.{key}\": {value} (expected an object)"
        ));
        return BTreeMap::new();
    };
    entries
        .iter()
        .filter_map(|(name, entry)| match entry {
            Value::String(s) => Some((name.clone(), s.clone())),
            other => {
                warnings.push(format!(
                    "config.json: ignoring \"routing.{key}\" entry \"{name}\": {other}"
                ));
                None
            }
        })
        .collect()
}

/// Where the router keeps its files. Each is the host's choice; `None` turns off what needs it
/// (no decisions saved means nothing to deliver or correct later).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RouterDirs {
    /// Saved decisions (`<decision_id>.json`, 0600, kept 7 days), for `deliver` and `correct`.
    pub decisions: Option<PathBuf>,
    /// Prompts waiting for a retry (`<key>/prompt.json` + `.deliver.json`).
    pub outbox: Option<PathBuf>,
    /// Sends that can be undone (`<delivery_id>.json`, 0600, kept 7 days).
    pub deliveries: Option<PathBuf>,
    /// Picks counted toward a rule offer (a file, 0600).
    pub learned: Option<PathBuf>,
    /// The decision and correction log (JSON lines, 0600).
    pub log: Option<PathBuf>,
    /// Kit's eval items (`*.jsonl`), read by `eval`.
    pub eval: Option<PathBuf>,
    /// Captures (`*.scrawl` bundles): bundle IDs resolve here, and `eval` replays them.
    pub captures: Option<PathBuf>,
}

impl RouterDirs {
    /// Scrawl's layout under `router`: `decisions/`, `outbox/`, `deliveries/`, `learned.json`
    /// and `eval/`, with the log and captures where the host says.
    pub fn under(router: &Path, log: Option<PathBuf>, captures: Option<PathBuf>) -> Self {
        Self {
            decisions: Some(router.join("decisions")),
            outbox: Some(router.join("outbox")),
            deliveries: Some(router.join("deliveries")),
            learned: Some(router.join("learned.json")),
            log,
            eval: Some(router.join("eval")),
            captures,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bad_key_drops_only_itself() {
        let (config, warnings) = parse_scrawl_config_with_warnings(
            r#"{"routing": {"autoSend": "never", "undoWindowSec": "15",
                "sites": {"https://a.example": "~/a", "https://b.example": 3},
                "apps": {"com.apple.Safari": "~/x", "com.example.tool": "ses_t"}},
                "delivery": 3, "agentHost": "studio",
                "experiments": {"smartRouting": "on", "jevRouting": "maybe"}}"#,
        );
        assert_eq!(config.auto_send, AutoSend::Never);
        assert_eq!(config.undo_window_sec, None);
        assert_eq!(
            config.sites.into_iter().collect::<Vec<_>>(),
            vec![("https://a.example".to_owned(), "~/a".to_owned())]
        );
        assert_eq!(config.apps.len(), 1, "general-purpose apps are dropped");
        assert_eq!(config.delivery, DeliveryMode::Steer);
        assert_eq!(config.agent_host.as_deref(), Some("studio"));
        assert!(config.smart_routing && !config.jev_routing);
        assert_eq!(warnings.len(), 5, "{warnings:#?}");
        let config = parse_scrawl_config(r#"{"routing": 3, "autoSend": false}"#);
        assert!(!config.send_captures);
        assert_eq!(config.auto_send, AutoSend::Rules);
        assert_eq!(parse_scrawl_config("not json"), RouterConfig::default());
    }

    #[test]
    fn rules_are_added_and_refused_on_a_copy() {
        let config = RouterConfig {
            refused_rules: vec!["https://a.example".into()],
            ..RouterConfig::default()
        };
        let next = config
            .with_rule(RuleKind::Site, "https://a.example", "~/a")
            .unwrap();
        assert_eq!(next.sites["https://a.example"], "~/a");
        assert!(next.refused_rules.is_empty(), "adding lifts the refusal");
        assert!(config.sites.is_empty(), "the original is untouched");
        assert!(
            config
                .with_rule(RuleKind::App, "com.apple.Safari", "~/a")
                .is_err()
        );
        let next = next.with_refused("com.example.tool").unwrap();
        assert_eq!(
            next.with_refused("com.example.tool").unwrap().refused_rules,
            vec!["com.example.tool".to_owned()]
        );
    }

    #[test]
    fn the_config_round_trips() {
        let config = RouterConfig {
            auto_send: AutoSend::Confident,
            delivery: DeliveryMode::Queue,
            jev_routing: true,
            ..RouterConfig::default()
        };
        let text = serde_json::to_string(&config).unwrap();
        assert!(text.contains("\"autoSend\":\"confident\""), "{text}");
        assert_eq!(serde_json::from_str::<RouterConfig>(&text).unwrap(), config);
        assert_eq!(
            serde_json::from_str::<RouterConfig>("{}").unwrap(),
            RouterConfig::default()
        );
    }
}
