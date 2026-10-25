//! Where a capture or dictation goes among OpenCode sessions, sending it there, and taking it
//! back. Scrawl's `scrawl router …` commands and Hex call the same [`Router`]; the host owns
//! the settings ([`RouterConfig`]) and the folders ([`RouterDirs`]). See `README.md`.

pub mod bundle;
pub mod config;
pub mod delivery;
pub mod env;
pub mod model;
pub mod opencode;
pub mod routing;
pub mod util;

use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use anyhow::{Result, anyhow};

pub use config::{
    DeliveryMode, RouterConfig, RouterDirs, parse_scrawl_config, parse_scrawl_config_with_warnings,
    router_config_from_scrawl,
};
pub use routing::jev::Action;
pub use routing::policy::{AutoSend, OfferRule, RuleKind};
pub use routing::router::{DeliverRequest, Delivered, Output as Decision, Target};
pub use routing::undo::{Report as UndoReport, Sent, Undone};

use env::Env;

/// The router with a host's settings and folders. Cheap to clone; every call runs with these
/// settings, on the caller's thread and the threads it starts.
#[derive(Clone)]
pub struct Router {
    env: Arc<Env>,
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
    /// A router that reads this Mac's OpenCode sessions and history.
    pub fn new(config: RouterConfig, dirs: RouterDirs) -> Self {
        Self {
            env: Arc::new(Env {
                config,
                dirs,
                live: true,
                ..Env::default()
            }),
        }
    }

    fn with(mut self, f: impl FnOnce(&mut Env)) -> Self {
        f(Arc::make_mut(&mut self.env));
        self
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

    pub fn config(&self) -> &RouterConfig {
        &self.env.config
    }

    pub fn dirs(&self) -> &RouterDirs {
        &self.env.dirs
    }

    /// Runs `f` with this router's settings, for hosts that call the modules directly.
    pub fn run<T>(&self, f: impl FnOnce() -> T) -> T {
        let _entered = env::enter(self.env.clone());
        f()
    }

    /// Makes this router what the modules use outside a call (Scrawl's app).
    pub fn install_as_fallback(&self) {
        let env = self.env.clone();
        env::set_fallback(move || env.clone());
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
        let env = self.env.clone();
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
        self.run(|| routing::router::decide_within(request.clone(), deadline))
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
