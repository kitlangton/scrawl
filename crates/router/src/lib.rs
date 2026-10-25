pub mod agent_context;
pub mod delivery;
pub mod handoff;
pub mod log;
pub mod model;
pub mod page;
pub mod routing;
pub mod sketch;
pub mod web;

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::Result;
use serde::{Deserialize, Serialize};

pub use handoff::{
    CANDIDATE_BUDGET_BYTES, Config, DeliveryMode, EngineConfig, MicChoice, RoutingConfig,
    enable_scrawl_host_defaults, parse_scrawl_config,
};
pub use log::{Sink, enable_file_log, scrawl_log, set_sink, set_sink_if_unset, start_metric};
pub use routing::jev::{Action, Band, Past, Screen, Thresholds};
pub use routing::policy::{AutoSend, OfferRule, RuleKind};
pub use routing::router::{
    DeliverHooks, DeliverRequest, Delivered, Output as Decision, Target as CandidateTarget,
};
pub use routing::undo::{Origin as DeliveryOrigin, Report as UndoReport, Sent, Undone};

/// Default port for `scrawl serve` and `/web/*` page registrations.
pub const DEFAULT_INGEST_PORT: u16 = 47_771;

static BUNDLE_NONCE: AtomicU64 = AtomicU64::new(0);

pub fn now_epoch_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

pub fn make_bundle_id() -> String {
    let millis = now_epoch_ms();
    let pid = std::process::id();
    let seq = BUNDLE_NONCE.fetch_add(1, Ordering::Relaxed) & 0xffff;
    format!("{millis:x}-{pid:x}-{seq:04x}")
}

/// Constant-time string comparison for bearer tokens.
pub fn constant_time_eq(a: &str, b: &str) -> bool {
    let a = a.as_bytes();
    let b = b.as_bytes();
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0_u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

pub fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2]))
        {
            out.push((hi << 4) | lo);
            i += 3;
            continue;
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}

/// Returns the executable path for a running process ID (`proc_pidpath` on macOS).
#[cfg(target_os = "macos")]
pub fn process_executable(pid: i32) -> Option<PathBuf> {
    if pid <= 0 {
        return None;
    }
    unsafe extern "C" {
        fn proc_pidpath(pid: i32, buffer: *mut libc::c_void, buffersize: u32) -> i32;
    }
    let mut buf = vec![0_u8; 4096];
    let len = unsafe { proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
    if len <= 0 {
        return None;
    }
    buf.truncate(len as usize);
    let path = PathBuf::from(String::from_utf8_lossy(&buf).into_owned());
    path.is_absolute().then_some(path)
}

#[cfg(not(target_os = "macos"))]
pub fn process_executable(_pid: i32) -> Option<PathBuf> {
    None
}

/// Host-supplied routing configuration (`autoSend`, `undoWindowSec`, rules, `delivery`
/// steer/queue, `jevRouting` thresholds, and `jevApiKey`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouterConfig {
    #[serde(default)]
    pub auto_send: AutoSend,
    #[serde(default = "default_undo_window_sec")]
    pub undo_window_sec: u64,
    #[serde(default)]
    pub delivery: DeliveryMode,
    #[serde(default)]
    pub sites: BTreeMap<String, String>,
    #[serde(default)]
    pub apps: BTreeMap<String, String>,
    #[serde(default)]
    pub servers: BTreeMap<String, String>,
    #[serde(default)]
    pub aliases: BTreeMap<String, String>,
    #[serde(default)]
    pub refused_rules: Vec<String>,
    #[serde(default)]
    pub jev_routing: Option<serde_json::Value>,
    #[serde(default)]
    pub jev_api_key: Option<String>,
    #[serde(default)]
    pub agent_host: Option<String>,
    #[serde(default)]
    pub agent_opencode: Option<String>,
}

fn default_undo_window_sec() -> u64 {
    routing::undo::UNDO_WINDOW_SEC
}

impl Default for RouterConfig {
    fn default() -> Self {
        Self {
            auto_send: AutoSend::default(),
            undo_window_sec: routing::undo::UNDO_WINDOW_SEC,
            delivery: DeliveryMode::default(),
            sites: BTreeMap::new(),
            apps: BTreeMap::new(),
            servers: BTreeMap::new(),
            aliases: BTreeMap::new(),
            refused_rules: Vec::new(),
            jev_routing: None,
            jev_api_key: None,
            agent_host: None,
            agent_opencode: None,
        }
    }
}

impl RouterConfig {
    pub fn from_engine_config(engine: &EngineConfig) -> Self {
        let routing = engine.routing.as_ref();
        Self {
            auto_send: engine.routing_auto_send(),
            undo_window_sec: routing
                .and_then(|r| r.undo_window_sec)
                .unwrap_or(routing::undo::UNDO_WINDOW_SEC),
            delivery: engine.delivery_mode(),
            sites: routing.map(|r| r.sites.clone()).unwrap_or_default(),
            apps: routing
                .map(|r| {
                    r.apps
                        .iter()
                        .filter(|(id, _)| !routing::policy::is_general_app(id))
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect()
                })
                .unwrap_or_default(),
            servers: routing.map(|r| r.servers.clone()).unwrap_or_default(),
            aliases: routing.map(|r| r.aliases.clone()).unwrap_or_default(),
            refused_rules: engine.routing_refused_rules(),
            jev_routing: engine.jev_routing.clone(),
            jev_api_key: engine.jev_api_key.clone(),
            agent_host: engine.agent_host.clone(),
            agent_opencode: engine.agent_opencode.clone(),
        }
    }

    pub fn to_engine_config(&self) -> EngineConfig {
        let mut cfg = EngineConfig::default();
        cfg.agent_host = self.agent_host.clone();
        cfg.agent_opencode = self.agent_opencode.clone();
        cfg.delivery = Some(serde_json::json!(self.delivery.as_str()));
        cfg.routing = Some(RoutingConfig {
            auto_send: Some(serde_json::json!(self.auto_send.label())),
            undo_window_sec: Some(self.undo_window_sec),
            apps: self.apps.clone(),
            servers: self.servers.clone(),
            sites: self.sites.clone(),
            aliases: self.aliases.clone(),
            refused_rules: Some(serde_json::json!(self.refused_rules)),
        });
        cfg.jev_routing = self.jev_routing.clone();
        cfg.jev_api_key = self.jev_api_key.clone();
        cfg
    }

    pub fn thresholds(&self) -> Thresholds {
        Thresholds::from_config(self.jev_routing.as_ref())
    }
}

/// Host-owned directories for router state (`decisions/`, `deliveries/`, `outbox/`,
/// `learned.json`) and the append-only `routing-decisions.jsonl` log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RouterDirs {
    pub state_dir: PathBuf,
    pub decisions_log: Option<PathBuf>,
}

impl RouterDirs {
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        Self {
            state_dir: state_dir.into(),
            decisions_log: None,
        }
    }

    pub fn with_decisions_log(mut self, path: impl Into<PathBuf>) -> Self {
        self.decisions_log = Some(path.into());
        self
    }

    pub fn decisions_dir(&self) -> PathBuf {
        self.state_dir.join("decisions")
    }

    pub fn deliveries_dir(&self) -> PathBuf {
        self.state_dir.join("deliveries")
    }

    pub fn outbox_dir(&self) -> PathBuf {
        self.state_dir.join("outbox")
    }

    pub fn learned_file(&self) -> PathBuf {
        self.state_dir.join("learned.json")
    }

    pub fn decisions_log_path(&self) -> PathBuf {
        self.decisions_log
            .clone()
            .unwrap_or_else(|| self.state_dir.join("routing-decisions.jsonl"))
    }
}

/// Recent session deliveries and pastes within a 15-minute sliding window, used by Hex and
/// Scrawl to give Jev recent routing history (`sent`, `taken_back`, `redirected`, and pasted
/// items).
pub struct Recents {
    items: Mutex<VecDeque<RecentEntry>>,
}

struct RecentEntry {
    at: Instant,
    item_id: Option<String>,
    delivery_id: Option<String>,
    item: RecentItem,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentItem {
    pub text_excerpt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub routed_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub corrected_to: Option<String>,
}

const RECENT_LIMIT: usize = 3;
const RECENT_MAX_AGE: Duration = Duration::from_secs(15 * 60);
const RECENT_EXCERPT_CHARS: usize = 160;

impl Default for Recents {
    fn default() -> Self {
        Self::new()
    }
}

impl Recents {
    pub const fn new() -> Self {
        Self {
            items: Mutex::new(VecDeque::new()),
        }
    }

    /// Record a session delivery (`to` is the session id, e.g. `ses_…`).
    pub fn sent(
        &self,
        item_id: Option<&str>,
        delivery_id: Option<&str>,
        text: &str,
        app: Option<&str>,
        to: &str,
    ) {
        self.push(
            item_id.map(str::to_owned),
            delivery_id.map(str::to_owned),
            text,
            app.map(str::to_owned),
            Some(to.to_owned()),
        );
    }

    /// Remove a delivery that was cancelled or undone so a mistaken send is not fed to Jev as
    /// precedent.
    pub fn taken_back(&self, item_id: Option<&str>, delivery_id: Option<&str>) {
        let mut items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        items.retain(|entry| {
            let same_item =
                item_id.is_some_and(|id| !id.is_empty() && entry.item_id.as_deref() == Some(id));
            let same_delivery = delivery_id
                .is_some_and(|id| !id.is_empty() && entry.delivery_id.as_deref() == Some(id));
            !(same_item || same_delivery)
        });
    }

    /// Update an earlier item when the user redirected it to another session (`to_session`), or
    /// insert a new entry if it wasn't recorded yet.
    pub fn redirected(
        &self,
        item_id: Option<&str>,
        delivery_id: Option<&str>,
        text: Option<&str>,
        app: Option<&str>,
        from_session: Option<&str>,
        to_session: &str,
    ) {
        let mut items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = Instant::now();
        prune_recents(&mut items, now);
        for entry in items.iter_mut().rev() {
            let same_item =
                item_id.is_some_and(|id| !id.is_empty() && entry.item_id.as_deref() == Some(id));
            let same_delivery = delivery_id
                .is_some_and(|id| !id.is_empty() && entry.delivery_id.as_deref() == Some(id));
            if same_item || same_delivery {
                entry.item.corrected_to = Some(to_session.to_owned());
                return;
            }
        }
        if let Some(text) = text {
            let excerpt = routing::jev::trim_end(text, RECENT_EXCERPT_CHARS);
            if !excerpt.is_empty() {
                if items.len() == RECENT_LIMIT {
                    items.pop_front();
                }
                items.push_back(RecentEntry {
                    at: now,
                    item_id: item_id.map(str::to_owned),
                    delivery_id: delivery_id.map(str::to_owned),
                    item: RecentItem {
                        text_excerpt: excerpt,
                        app: app.filter(|s| !s.is_empty()).map(str::to_owned),
                        routed_to: from_session
                            .filter(|s| !s.is_empty())
                            .map(str::to_owned)
                            .or_else(|| Some(to_session.to_owned())),
                        corrected_to: Some(to_session.to_owned()),
                    },
                });
            }
        }
    }

    /// Reserve a slot when a paste begins so overlapping dictations stay in submission order.
    pub fn paste_submitted(&self, id: &str, app: Option<String>) {
        let mut items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = Instant::now();
        prune_recents(&mut items, now);
        if items.len() == RECENT_LIMIT {
            items.pop_front();
        }
        items.push_back(RecentEntry {
            at: now,
            item_id: Some(id.to_owned()),
            delivery_id: None,
            item: RecentItem {
                text_excerpt: String::new(),
                app: app.filter(|s| !s.is_empty()),
                routed_to: Some("pasted".to_owned()),
                corrected_to: None,
            },
        });
    }

    /// Fill in a reserved paste slot once `paste_text` succeeds.
    pub fn paste_completed(&self, id: &str, text: &str, app: Option<String>) {
        let excerpt = routing::jev::trim_end(text, RECENT_EXCERPT_CHARS);
        let mut items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = Instant::now();
        prune_recents(&mut items, now);
        if excerpt.is_empty() {
            items.retain(|entry| entry.item_id.as_deref() != Some(id));
            return;
        }
        let app = app.filter(|s| !s.is_empty());
        if let Some(entry) = items
            .iter_mut()
            .find(|entry| entry.item_id.as_deref() == Some(id))
        {
            entry.item.text_excerpt = excerpt;
            if app.is_some() {
                entry.item.app = app;
            }
            return;
        }
        if items.len() == RECENT_LIMIT {
            items.pop_front();
        }
        items.push_back(RecentEntry {
            at: now,
            item_id: Some(id.to_owned()),
            delivery_id: None,
            item: RecentItem {
                text_excerpt: excerpt,
                app,
                routed_to: Some("pasted".to_owned()),
                corrected_to: None,
            },
        });
    }

    /// Drop a reserved slot when a paste fails or is skipped.
    pub fn forget(&self, id: &str) {
        self.taken_back(Some(id), None);
    }

    /// Oldest-first snapshot of completed items within the 15-minute window.
    pub fn snapshot(&self) -> Vec<RecentItem> {
        let mut items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        prune_recents(&mut items, Instant::now());
        items
            .iter()
            .filter(|e| !e.item.text_excerpt.is_empty())
            .map(|e| e.item.clone())
            .collect()
    }

    fn push(
        &self,
        item_id: Option<String>,
        delivery_id: Option<String>,
        text: &str,
        app: Option<String>,
        routed_to: Option<String>,
    ) {
        let excerpt = routing::jev::trim_end(text, RECENT_EXCERPT_CHARS);
        if excerpt.is_empty() {
            return;
        }
        let mut items = self
            .items
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let now = Instant::now();
        prune_recents(&mut items, now);
        if items.len() == RECENT_LIMIT {
            items.pop_front();
        }
        items.push_back(RecentEntry {
            at: now,
            item_id,
            delivery_id,
            item: RecentItem {
                text_excerpt: excerpt,
                app: app.filter(|s| !s.is_empty()),
                routed_to,
                corrected_to: None,
            },
        });
    }
}

fn prune_recents(items: &mut VecDeque<RecentEntry>, now: Instant) {
    while items
        .front()
        .is_some_and(|e| now.saturating_duration_since(e.at) > RECENT_MAX_AGE)
    {
        items.pop_front();
    }
}

/// Host-configured session router. All state (`decisions/`, `deliveries/`, `outbox/`,
/// `learned.json`, and `routing-decisions.jsonl`) lives under the caller's `RouterDirs`, and all
/// settings come from `RouterConfig`.
pub struct Router {
    config: Mutex<RouterConfig>,
    dirs: RouterDirs,
    recents: Recents,
}

impl Router {
    pub fn new(config: RouterConfig, dirs: RouterDirs) -> Self {
        Self {
            config: Mutex::new(config),
            dirs,
            recents: Recents::new(),
        }
    }

    pub fn config(&self) -> RouterConfig {
        self.config
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub fn set_config(&self, config: RouterConfig) {
        *self
            .config
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = config;
    }

    pub fn dirs(&self) -> &RouterDirs {
        &self.dirs
    }

    pub fn recents(&self) -> &Recents {
        &self.recents
    }

    /// Decide where an item should go within `deadline`, saving the decision under
    /// `dirs.decisions_dir()` and appending a decision line to `dirs.decisions_log_path()`.
    pub fn decide(&self, request: serde_json::Value, deadline: Duration) -> Result<Decision> {
        let cfg = self.config();
        routing::router::decide_in(
            Some(&self.dirs.decisions_dir()),
            Some(&self.dirs.outbox_dir()),
            Some(&self.dirs.decisions_log_path()),
            Some(&cfg),
            request,
            deadline,
        )
    }

    pub fn decide_json(&self, request_json: &str, deadline: Duration) -> Result<Decision> {
        let value: serde_json::Value = serde_json::from_str(request_json)?;
        self.decide(value, deadline)
    }

    /// Deliver a decided (or direct) item to `request.to` within `deadline`, returning a
    /// `delivery_id` when sent or `state == "queued"` when saved in `dirs.outbox_dir()`.
    pub fn deliver(&self, request: DeliverRequest, _deadline: Duration) -> Result<Delivered> {
        let cfg = self.config();
        routing::router::deliver_in(&self.dirs, &cfg, request, &DeliverHooks::default())
    }

    pub fn deliver_with_hooks(
        &self,
        request: DeliverRequest,
        hooks: &DeliverHooks,
    ) -> Result<Delivered> {
        let cfg = self.config();
        routing::router::deliver_in(&self.dirs, &cfg, request, hooks)
    }

    /// Undo a previous delivery (`withdrawn` / `reverted` / `disowned` / `gone` / `too-old`)
    /// within `deadline`.
    pub fn undo(&self, delivery_id: &str, deadline: Duration) -> Result<UndoReport> {
        let cfg = self.config();
        routing::undo::undo_with_config(&self.dirs, &cfg, delivery_id, deadline)
    }

    /// Record a correction (`cancel` / `redirect` / `paste` / `edit`) for a saved `decision_id`.
    pub fn correct(&self, decision_id: &str, action: Action, to: Option<&str>) -> Result<()> {
        routing::router::correct_in(
            &self.dirs.decisions_dir(),
            Some(&self.dirs.decisions_log_path()),
            decision_id,
            action,
            to,
        )
    }

    /// Record a correction JSON object (matching `scrawl router log`), joining it to its saved
    /// decision by `decision_id` or `item_id`.
    pub fn log_correction(&self, line: serde_json::Value) -> Result<()> {
        routing::router::log_in(
            Some(&self.dirs.decisions_dir()),
            Some(&self.dirs.decisions_log_path()),
            line,
        )
    }

    pub fn log_correction_json(&self, line_json: &str) -> Result<()> {
        let line: serde_json::Value = serde_json::from_str(line_json.trim())?;
        self.log_correction(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn parse_scrawl_config_extracts_engine_and_router_configs_purely() {
        let raw = r#"{
            "transcriber": "hex",
            "hexModel": "parakeet-tdt-0.6b-v3",
            "delivery": "queue",
            "jevApiKey": "sk-test-123",
            "jevRouting": { "send": 0.85 },
            "routing": {
                "autoSend": "confident",
                "undoWindowSec": 120,
                "sites": { "localhost:3000": "~/code/app" },
                "apps": { "dev.kitlangton.Hex": "~/code/hex", "com.brave.Browser": "~/code/bad" },
                "aliases": { "hex": "Hex voice" },
                "refusedRules": ["localhost:5173"]
            }
        }"#;
        let (engine, router) = parse_scrawl_config(raw);
        assert_eq!(engine.transcriber.as_deref(), Some("hex"));
        assert_eq!(engine.hex_model().as_deref(), Some("parakeet-tdt-0.6b-v3"));
        assert_eq!(router.auto_send, AutoSend::Confident);
        assert_eq!(router.undo_window_sec, 120);
        assert_eq!(router.delivery, DeliveryMode::Queue);
        assert_eq!(
            router.sites.get("localhost:3000").map(String::as_str),
            Some("~/code/app")
        );
        // General-purpose browsers are filtered out of routing.apps.
        assert_eq!(
            router.apps.get("dev.kitlangton.Hex").map(String::as_str),
            Some("~/code/hex")
        );
        assert!(!router.apps.contains_key("com.brave.Browser"));
        assert_eq!(
            router.aliases.get("hex").map(String::as_str),
            Some("Hex voice")
        );
        assert_eq!(router.refused_rules, vec!["localhost:5173".to_owned()]);
        assert_eq!(router.jev_api_key.as_deref(), Some("sk-test-123"));
        assert!((router.thresholds().send - 0.85).abs() < 1e-6);
    }

    #[test]
    fn router_writes_only_under_caller_dirs_and_respects_recents() {
        let tmp = std::env::temp_dir().join(format!("scrawl-router-lib-{}", make_bundle_id()));
        let dirs = RouterDirs::new(&tmp);
        let mut cfg = RouterConfig::default();
        cfg.sites
            .insert("localhost:4321".into(), "ses_sitetarget".into());
        let router = Router::new(cfg, dirs.clone());

        let decision = router
            .decide(
                serde_json::json!({
                    "item": {
                        "id": "item-abc",
                        "kind": "dictation",
                        "text": "fix the header alignment",
                        "url": "http://localhost:4321/settings",
                        "app": "Brave"
                    }
                }),
                Duration::from_secs(2),
            )
            .unwrap();
        assert!(!decision.decision_id.is_empty());
        assert!(
            dirs.decisions_dir()
                .join(format!("{}.json", decision.decision_id))
                .is_file()
        );
        assert!(dirs.decisions_log_path().is_file());

        // Deliver using fake hook and verify delivery record lands in dirs.deliveries_dir().
        fn fake_send(_dir: &Path, _session: &str) -> Result<handoff::Posted> {
            Ok(handoff::Posted {
                message_id: Some("msg_123".into()),
                delivery: "queue".into(),
                was_idle: Some(true),
            })
        }
        let delivered = router
            .deliver_with_hooks(
                DeliverRequest {
                    decision: Some(decision.decision_id.clone()),
                    item_id: None,
                    to: "ses_sitetarget".into(),
                    text: None,
                },
                &DeliverHooks {
                    text: fake_send,
                    bundle: fake_send,
                },
            )
            .unwrap();
        assert_eq!(delivered.state, "sent");
        let sent = delivered.sent.expect("sent record");
        assert!(
            dirs.deliveries_dir()
                .join(format!("{}.json", sent.delivery_id))
                .is_file()
        );

        // Recents tracking: sent -> redirected -> taken_back
        router.recents().sent(
            Some("item-abc"),
            Some(&sent.delivery_id),
            "fix the header alignment",
            Some("Brave"),
            "ses_sitetarget",
        );
        assert_eq!(router.recents().snapshot().len(), 1);
        router.recents().redirected(
            Some("item-abc"),
            None,
            None,
            None,
            Some("ses_sitetarget"),
            "ses_other",
        );
        assert_eq!(
            router.recents().snapshot()[0].corrected_to.as_deref(),
            Some("ses_other")
        );
        router
            .recents()
            .taken_back(Some("item-abc"), Some(&sent.delivery_id));
        assert!(router.recents().snapshot().is_empty());

        router
            .correct(&decision.decision_id, Action::Redirect, Some("ses_other"))
            .unwrap();
        let log_text = std::fs::read_to_string(dirs.decisions_log_path()).unwrap();
        assert_eq!(log_text.lines().count(), 2);

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
