//! Where a capture or dictation goes among OpenCode sessions, sending it there, and taking it
//! back. Scrawl's `scrawl router …` commands and Hex call the same [`Router`]; the host owns
//! the settings ([`RouterConfig`]) and the folders ([`RouterDirs`]). See `README.md`.

pub mod agent_context;
pub mod bundle;
pub mod config;
pub mod delivery;
pub mod env;
pub mod handoff;
pub mod log;
pub mod model;
pub mod opencode;
pub mod page;
pub mod routing;
pub mod sketch;
pub mod util;
pub mod web;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};

pub use config::{
    DeliveryMode, RouterConfig, RouterDirs, parse_scrawl_config, parse_scrawl_config_with_warnings,
    parse_scrawl_router_config, parse_scrawl_router_config_with_warnings,
    router_config_from_scrawl,
};
pub use handoff::{
    EngineConfig, MicChoice, enable_scrawl_host_defaults, engine_config_from_scrawl,
};
pub use log::{Sink, enable_file_log, scrawl_log, set_sink, set_sink_if_unset, start_metric};
pub use routing::jev::{Action, Past, Screen, Thresholds};
pub use routing::policy::{AutoSend, OfferRule, RuleKind};
pub use routing::router::{
    DeliverRequest, Delivered, Output as Decision, Target, Target as CandidateTarget,
};
pub use routing::undo::{Report as UndoReport, Sent, Undone};

use env::Env;

/// Host-supplied [`RouterConfig`] source: either a closure (`Arc<dyn Fn() -> RouterConfig + Send + Sync>`
/// or `impl Fn() -> RouterConfig`) asked on each router call so live settings changes take effect
/// immediately, or a static [`RouterConfig`] value.
pub type ConfigSource = Arc<dyn Fn() -> RouterConfig + Send + Sync>;

pub trait IntoRouterConfig {
    fn into_config_source(self) -> ConfigSource;
}

impl IntoRouterConfig for ConfigSource {
    fn into_config_source(self) -> ConfigSource {
        self
    }
}

impl<F> IntoRouterConfig for F
where
    F: Fn() -> RouterConfig + Send + Sync + 'static,
{
    fn into_config_source(self) -> ConfigSource {
        Arc::new(self)
    }
}

impl IntoRouterConfig for RouterConfig {
    fn into_config_source(self) -> ConfigSource {
        Arc::new(move || self.clone())
    }
}

/// Recent session deliveries and pastes within a 15-minute sliding window, used by Hex and
/// Scrawl to give Jev recent routing history (`sent`, `taken_back`, `redirected`, and pasted
/// items).
#[derive(Clone, Default)]
pub struct Recents {
    items: Arc<Mutex<VecDeque<RecentEntry>>>,
}

#[derive(Clone)]
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

impl Recents {
    pub fn new() -> Self {
        Self {
            items: Arc::new(Mutex::new(VecDeque::new())),
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

/// The router with a host's settings and folders. Cheap to clone; every call runs with these
/// settings, on the caller's thread and the threads it starts.
#[derive(Clone)]
pub struct Router {
    config: ConfigSource,
    env: Arc<Env>,
    recents: Recents,
}

/// What [`Router::eval`] replays.
#[derive(Clone, Copy, Debug)]
pub struct EvalOptions {
    /// How far back to look.
    pub since: Duration,
    /// At most this many captures.
    pub limit: usize,
    /// Put the targets' recent lines in every question.
    pub lines: bool,
    /// Show Jev the screen (windows, focused text, the OpenCode tab).
    pub screen: bool,
}

impl Default for EvalOptions {
    fn default() -> Self {
        Self {
            since: Duration::from_secs(7 * 24 * 3600),
            limit: 40,
            lines: false,
            screen: true,
        }
    }
}

impl Router {
    /// A router that reads this Mac's OpenCode sessions and history. `config` may be a closure
    /// (`Arc<dyn Fn() -> RouterConfig + Send + Sync>` or `|| ...`) or a [`RouterConfig`].
    pub fn new(config: impl IntoRouterConfig, dirs: RouterDirs) -> Self {
        let config = config.into_config_source();
        let initial = config();
        Self {
            config,
            env: Arc::new(Env {
                config: initial,
                dirs,
                live: true,
                ..Env::default()
            }),
            recents: Recents::new(),
        }
    }

    fn with(mut self, f: impl FnOnce(&mut Env)) -> Self {
        f(Arc::make_mut(&mut self.env));
        self
    }

    fn current_env(&self) -> Arc<Env> {
        let mut env = (*self.env).clone();
        env.config = (self.config)();
        Arc::new(env)
    }

    /// Where the router's log lines go (none by default).
    pub fn with_log(self, log: impl Fn(&str) + Send + Sync + 'static) -> Self {
        let log: env::Log = Arc::new(log);
        self.with(|env| env.log = Some(log))
    }

    /// The summary sent with a capture and rewritten when its destination is recorded (default:
    /// the bundle's own `summary.md`, never rewritten).
    pub fn with_summary(self, summary: env::Summary) -> Self {
        self.with(|env| env.summary = Some(summary))
    }

    /// The `/sdk.js` registration for a tab, when the host serves `/sdk.js`.
    pub fn with_pages(self, pages: env::Pages) -> Self {
        self.with(|env| env.pages = Some(pages))
    }

    /// Whether OpenCode's history and sessions' last lines are read (on by default).
    pub fn with_live(self, live: bool) -> Self {
        self.with(|env| env.live = live)
    }

    pub fn config(&self) -> RouterConfig {
        (self.config)()
    }

    pub fn dirs(&self) -> &RouterDirs {
        &self.env.dirs
    }

    pub fn recents(&self) -> &Recents {
        &self.recents
    }

    /// Runs `f` with this router's settings, for hosts that call the modules directly.
    pub fn run<T>(&self, f: impl FnOnce() -> T) -> T {
        let _entered = env::enter(self.current_env());
        f()
    }

    /// Makes this router what the modules use outside a call (Scrawl's app).
    pub fn install_as_fallback(&self) {
        let router = self.clone();
        env::set_fallback(move || router.current_env());
    }

    /// Runs `f` on its own thread with this router's settings and waits up to `deadline`.
    /// Past it, `f` carries on alone and the call fails.
    fn within<T: Send + 'static>(
        &self,
        what: &str,
        deadline: Duration,
        f: impl FnOnce() -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let (tx, rx) = mpsc::channel();
        let env = self.current_env();
        std::thread::spawn(move || {
            let _entered = env::enter(env);
            let _ = tx.send(f());
        });
        match rx.recv_timeout(deadline) {
            Ok(result) => result,
            Err(_) => Err(anyhow!(
                "router {what} timed out after {} ms",
                deadline.as_millis()
            )),
        }
    }

    /// `router decide`: where `request`'s item goes, without sending. `request` is
    /// `{"item": {"kind", "text", "app", "title", "url", "selected_text", "mode", …}, "pinned"?,
    /// "history"?}`. Answers within `deadline` whatever happens: a timeout is a `keep`.
    /// Logged, and saved for [`Router::deliver`] and [`Router::correct`].
    pub fn decide(&self, request: &serde_json::Value, deadline: Duration) -> Result<Decision> {
        let mut request = request.clone();
        if let Some(obj) = request.as_object_mut()
            && !obj.contains_key("history")
        {
            let recent = self.recents.snapshot();
            if !recent.is_empty()
                && let Ok(val) = serde_json::to_value(recent)
            {
                obj.insert("history".to_owned(), val);
            }
        }
        self.run(|| routing::router::decide_within(request, deadline))
    }

    /// `router deliver --decision <id> --to <session>`: sends the decision's text (dictation) or
    /// bundle (capture). A send OpenCode didn't take is queued and retried by the host.
    pub fn deliver(&self, decision_id: &str, to: &str, deadline: Duration) -> Result<Delivered> {
        self.deliver_request(
            DeliverRequest {
                decision: Some(decision_id.to_owned()),
                to: to.to_owned(),
                ..DeliverRequest::default()
            },
            deadline,
        )
    }

    /// [`Router::deliver`] by the caller's item ID, or with new text.
    pub fn deliver_request(
        &self,
        request: DeliverRequest,
        deadline: Duration,
    ) -> Result<Delivered> {
        self.within("deliver", deadline, move || {
            routing::router::deliver(request)
        })
    }

    /// `router undo <delivery_id>`: takes a send back. Errors only for a malformed ID, a server
    /// that can't be reached, or `deadline`.
    pub fn undo(&self, delivery_id: &str, deadline: Duration) -> Result<UndoReport> {
        let id = delivery_id.to_owned();
        self.within("undo", deadline, move || routing::undo::undo(&id))
    }

    /// `router correct`: logs that a decision was corrected, from its target to `to` (a
    /// [`Action::Redirect`] names where it went instead).
    pub fn correct(&self, decision_id: &str, action: Action, to: Option<&str>) -> Result<()> {
        self.run(|| routing::router::correct(decision_id, action, to))
    }

    /// `router log`: one correction line in Hex's spelling, joined to its decision.
    pub fn log_correction(&self, line: &str) -> Result<()> {
        self.run(|| routing::router::log(line))
    }

    /// One pass over the outbox: sends the queued prompts that are due and drops those that
    /// went. A host that doesn't run Scrawl's ingest server calls this every few seconds.
    pub fn retry_outbox(&self) -> delivery::Report {
        self.run(|| {
            let Some(outbox) = routing::router::outbox_dir() else {
                return delivery::Report::default();
            };
            let report =
                delivery::retry_due(&outbox, util::now_epoch_ms(), routing::router::send_prompt);
            for (dir, _) in &report.sent {
                let _ = std::fs::remove_dir_all(dir);
            }
            report
        })
    }

    /// `router eval`: replays past captures through Jev and prints the accuracy.
    pub fn eval(&self, options: EvalOptions) -> Result<()> {
        self.run(|| {
            routing::router::eval(options.since, options.limit, options.lines, options.screen)
        })
    }
}
