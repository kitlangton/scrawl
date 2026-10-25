# scrawl-router

Where a capture or dictation goes among OpenCode sessions, sending it there, and taking it back. Scrawl's `scrawl router …` commands and Hex call the same code: Scrawl through its CLI and app, Hex in-process through `Router`.

The host owns the settings and the folders. The library never reads Scrawl's `~/.config/scrawl/config.json` and never picks a folder of its own. It builds without gpui, ScreenCaptureKit, the Swift helper, cpal or objc2: its dependencies are `anyhow`, `dirs`, `libc`, `serde` and `serde_json`.

## Using it

```rust
use std::time::Duration;
use scrawl_router::{Action, Router, RouterConfig, RouterDirs, parse_scrawl_config};

let config: RouterConfig = parse_scrawl_config(&std::fs::read_to_string(config_path)?);
let dirs = RouterDirs::under(&router_dir, Some(log_path), Some(captures_dir));
let router = Router::new(config, dirs).with_log(|line| eprintln!("{line}"));

let decision = router.decide(&serde_json::json!({"item": {"kind": "dictation", "text": "…"}}), Duration::from_millis(2500))?;
let sent = router.deliver(&decision.decision_id, "ses_…", Duration::from_secs(10))?;
let undone = router.undo(&sent.sent.unwrap().delivery_id, Duration::from_secs(10))?;
router.correct(&decision.decision_id, Action::Redirect, Some("ses_other"))?;
```

Every call blocks the calling thread until it's done or its deadline passes. Serializing a result with `serde_json` gives exactly what the matching `scrawl router` command prints (`docs/routing.md`).

## The API

```rust
impl Router {
    pub fn new(config: RouterConfig, dirs: RouterDirs) -> Self;
    pub fn with_log(self, log: impl Fn(&str) + Send + Sync + 'static) -> Self;
    pub fn with_summary(self, summary: env::Summary) -> Self;   // fn(&ScrawlBundle, &Path) -> String
    pub fn with_pages(self, pages: env::Pages) -> Self;         // fn(&str) -> Option<Registration>
    pub fn with_live(self, live: bool) -> Self;
    pub fn config(&self) -> &RouterConfig;
    pub fn dirs(&self) -> &RouterDirs;

    pub fn decide(&self, request: &serde_json::Value, deadline: Duration) -> Result<Decision>;
    pub fn deliver(&self, decision_id: &str, to: &str, deadline: Duration) -> Result<Delivered>;
    pub fn deliver_request(&self, request: DeliverRequest, deadline: Duration) -> Result<Delivered>;
    pub fn undo(&self, delivery_id: &str, deadline: Duration) -> Result<UndoReport>;
    pub fn correct(&self, decision_id: &str, action: Action, to: Option<&str>) -> Result<()>;
    pub fn log_correction(&self, line: &str) -> Result<()>;
    pub fn retry_outbox(&self) -> delivery::Report;
    pub fn eval(&self, options: EvalOptions) -> Result<()>;

    pub fn run<T>(&self, f: impl FnOnce() -> T) -> T;
    pub fn install_as_fallback(&self);
}
```

- **`decide`** (`router decide`): `request` is `{"item": {"kind", "text", "app", "title", "url", "selected_text", "mode"}, "pinned"?, "history"?}`. The `Decision` has `decision_id`, `decision` (`send`, `ask`, `fallback` or `keep`), `target`, `candidates`, `confidence`, `any_fits`, `model`, `ms`, `why`, and `suggested`, `would` and `offer_rule` when they apply. It answers within `deadline` whatever happens: a timeout is a `keep` whose `why` says so. Sessions are read and Jev asked until 200 ms before the deadline. The decision is logged and saved for `deliver` and `correct`. Nothing is sent.
- **`deliver`** (`router deliver --decision <id> --to <session>`): sends the decision's text (dictation) or bundle (capture). `Delivered` is `{"ok", "state": "sent" | "queued", "session", "offer_rule"?}`, plus `delivery_id`, `message_id`, `delivery` (`steer` or `queue`) and `was_idle` once sent. A send OpenCode didn't take is queued in the outbox; call `retry_outbox` every few seconds to retry it (Scrawl's ingest server does this for its own outbox). `deliver_request` takes the caller's `item_id` instead of a decision, or new text.
- **`undo`** (`router undo <id> --json`): `UndoReport` is `{"undone", "session", "message_id"}`, where `undone` is `withdrawn` (removed before it was read), `reverted` (the session reverted to before it), `disowned` (the session was told to ignore it), `gone` (the send or its session no longer exists) or `too-old` (past `undo_window_sec`; nothing done, not an error). It errors only for a malformed ID, a server that can't be reached, or the deadline.
- **`correct`** (`router correct --decision <id> --action <cancel|redirect|paste|edit> [--to <session>]`): logs that a decision was corrected, joined to it in the log. A redirect names where the item went instead. Picks are counted toward a rule offer by `deliver`, not here.
- **`log_correction`** (`router log`): one correction line in Hex's spelling, joined to its decision.
- **`eval`** (`router eval`): replays past captures and the items in `dirs.eval` through Jev and prints the accuracy.
- **`run`** runs any module function (`routing::…`) with this router's settings. **`install_as_fallback`** makes them what the modules use outside a call, for a host that calls the modules directly.

Rules are part of the config: `config.with_rule(RuleKind::Site | RuleKind::App, key, target)` and `config.with_refused(key)` return a new `RouterConfig` for the host to save. Scrawl, which keeps its rules as JSON, uses `routing::policy::add_rule` and `refuse_rule` on its `routing` object instead.

## The settings (`RouterConfig`)

Serde camelCase, every field optional. `parse_scrawl_config(&str)` reads them from the text of Scrawl's `config.json`, where a key (or a `sites`/`apps` entry) that doesn't fit is dropped on its own; `parse_scrawl_config_with_warnings` also says what it dropped.

| Field | Scrawl's key | Meaning |
|---|---|---|
| `auto_send` | `routing.autoSend` | What may send without asking: `rules` (default), `confident` or `never`. |
| `undo_window_sec` | `routing.undoWindowSec` | How long an undo still takes a send back (default 600). |
| `sites` | `routing.sites` | URL prefix → folder (`~` allowed) or `ses_` ID: where a site's items always go. |
| `apps` | `routing.apps` | Bundle ID (or a prefix) → folder or `ses_` ID. General-purpose apps (browsers, terminals, editors) are ignored. |
| `servers` | `routing.servers` | URL prefix → the folder a file server serves. |
| `aliases` | `routing.aliases` | Spoken name → the session title or folder it means. |
| `refused_rules` | `routing.refusedRules` | Sites and bundle IDs never offered a rule again. |
| `delivery` | `delivery` | `steer` (join a busy session's turn, default) or `queue` (wait for it to end). |
| `smart_routing` | `experiments.smartRouting` | Weigh a capture's evidence rather than follow a dev site's port alone. |
| `jev_routing` | `experiments.jevRouting` | Ask Jev (needs `smart_routing`). |
| `jev` | `jevRouting` (an object) | Jev's thresholds and options. |
| `jev_api_key` | `jevApiKey` | The key Jev is called with; absent uses `$OPENCODE_API_KEY`, then OpenCode's own credential. |
| `agent_host` | `agentHost` | The SSH host whose OpenCode server holds the sessions, when it isn't this Mac. |
| `agent_opencode` | `agentOpencode` | The `opencode2` command on that host. |
| `send_captures` | `autoSend` | Whether a capture of a dev site goes straight to its session (default on). |

## The folders (`RouterDirs`)

Each is `Option<PathBuf>`; `None` turns off what needs it. `RouterDirs::under(router, log, captures)` gives Scrawl's layout. To share decisions and undo records with the `scrawl router` CLI, use Scrawl's own: `RouterDirs::under("~/Library/Application Support/scrawl/router", Some(".../scrawl/routing-decisions.jsonl"), Some(captures))`.

| Field | Scrawl's layout | Holds |
|---|---|---|
| `decisions` | `router/decisions/` | Each decision's request and reply (`<id>.json`, 0600, kept 7 days), for `deliver` and `correct`. |
| `outbox` | `router/outbox/` | Prompts waiting for a retry, a folder each. |
| `deliveries` | `router/deliveries/` | Sends that can be undone (`<delivery_id>.json`, 0600, kept 7 days). |
| `learned` | `router/learned.json` | Each site's and app's delivered picks, counted toward a rule offer. |
| `log` | `routing-decisions.jsonl` | One JSON line per decision and correction (0600). |
| `eval` | `router/eval/` | Kit's eval items (`*.jsonl`), read by `eval`. |
| `captures` | Scrawl's captures folder | `.scrawl` bundles: bundle IDs resolve here, and `eval` replays them. |

## What it reads and never touches

It reads OpenCode's own files to reach the server and its history: `service.json` under `$XDG_STATE_HOME/opencode`, `~/.local/state/opencode` and `$OPENCODE_CONFIG_DIR` / `$XDG_CONFIG_HOME/opencode`; OpenCode's database read-only (`$OPENCODE_DB`, else found through the server); and `$PATH`, `$OPENCODE_BIN`, `$OPENCODE_PASSWORD` / `$OPENCODE_SERVER_PASSWORD`, `$OPENCODE_SESSION_ID`, `$OPENCODE_API_KEY` and `$SCRAWL_NO_OPENCODE`.

It never reads `~/.config/scrawl/config.json` or `$SCRAWL_CONFIG`, never writes under `~/Library/Application Support/scrawl` unless the host's `RouterDirs` point there, and never reads `$SCRAWL_ROUTER_DIR`, `$SCRAWL_ROUTING_LOG` or `$SCRAWL_AGENT_HOST` (Scrawl reads those and passes the result in). Outside a `Router` call with no fallback installed it is inert: default settings, no folders, no log, and no OpenCode history read. Tests rely on that.
