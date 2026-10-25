//! What became of a ScrawlKit recording once the Mac took it in, what the phone is told about
//! it, and the retries that finish the job when OpenCode didn't take the bundle.
//!
//! The phone uploads once. When the bundle is built but queuing it into the session fails, the
//! Mac keeps a `.deliver.json` in the bundle and tries again with backoff (15 s doubling to
//! 30 min) for a day, then renames it `.deliver-failed.json`. Retention leaves a bundle with a
//! pending delivery alone.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::env::log as scrawl_log;

/// A bundle whose forward to OpenCode is still being retried.
pub const PENDING: &str = ".deliver.json";
/// A bundle whose forward was given up on.
pub const FAILED: &str = ".deliver-failed.json";
/// A text prompt waiting in `scrawl router`'s outbox (a folder of its own, retried like a bundle).
pub const PROMPT: &str = "prompt.json";

const GIVE_UP_AFTER_MS: u64 = 24 * 60 * 60 * 1000;
const SCAN_EVERY: Duration = Duration::from_secs(15);

/// What happened once a received recording's bundle was built.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Delivery {
    /// Queued into the OpenCode session `title`.
    Sent { title: String },
    /// Saved on the Mac; queuing it into `title` failed and is retried (`PENDING`).
    Queued { title: String, error: String },
    /// Saved on the Mac and sent nowhere (no session resolved, or routing is off).
    Saved,
}

impl Delivery {
    /// The HUD's line for it; `saved` says what happened locally ("Copied to clipboard").
    pub fn status(&self, saved: &str) -> String {
        match self {
            Self::Sent { title } => format!("Sent to {title}"),
            Self::Queued { title, .. } => format!("{saved} · will retry sending to {title}"),
            Self::Saved => saved.to_owned(),
        }
    }
}

/// What the Mac can tell the phone when it answers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Done(Delivery),
    /// Still building or sending when the wait ran out; it carries on (a failed send is retried).
    Processing,
    /// The bundle couldn't be built; the recording wasn't kept, so the phone should send it again.
    Failed(String),
}

/// The JSON body for an outcome. `title` is the session the recording is meant for ("Mac"
/// when it isn't routed). Phones from before `delivered` treat any 2xx as sent.
pub fn reply_json(outcome: &Outcome, title: &str) -> serde_json::Value {
    let (delivered, state, error) = match outcome {
        Outcome::Done(Delivery::Sent { .. }) => (true, "sent", None),
        Outcome::Done(Delivery::Queued { error, .. }) => (false, "queued", Some(error.as_str())),
        Outcome::Done(Delivery::Saved) => (false, "saved", None),
        Outcome::Processing => (false, "processing", None),
        Outcome::Failed(error) => {
            return serde_json::json!({ "ok": false, "error": short(error) });
        }
    };
    let mut body = serde_json::json!({
        "ok": true,
        "delivered": delivered,
        "state": state,
        "sessionTitle": title,
    });
    if let Some(error) = error {
        body["error"] = short(error).into();
    }
    body
}

/// The first line of an error, at most 160 characters, for the phone.
pub fn short(error: &str) -> String {
    let line = error.lines().next().unwrap_or("").trim();
    if line.chars().count() > 160 {
        format!("{}…", line.chars().take(159).collect::<String>())
    } else {
        line.to_owned()
    }
}

/// `PENDING`: where to send a bundle and how the tries have gone. Times are epoch ms.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pending {
    pub session: String,
    pub title: String,
    /// The recording's ingest key (`recordedAt`), so a later duplicate upload hears it was sent.
    #[serde(default)]
    pub key: Option<String>,
    pub attempts: u32,
    pub first_at: u64,
    pub next_at: u64,
    pub last_error: String,
}

impl Pending {
    /// After the first failed send at `now`.
    pub fn new(session: &str, title: &str, key: Option<&str>, error: &str, now: u64) -> Self {
        Self {
            session: session.to_owned(),
            title: title.to_owned(),
            key: key.map(str::to_owned),
            attempts: 1,
            first_at: now,
            next_at: now + backoff_ms(1),
            last_error: short(error),
        }
    }

    pub fn write(&self, bundle: &Path) -> Result<()> {
        let tmp = bundle.join(".deliver.json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self)?)?;
        std::fs::rename(&tmp, bundle.join(PENDING))
            .with_context(|| format!("Could not write {}", bundle.join(PENDING).display()))
    }

    fn read(bundle: &Path) -> Result<Self> {
        let path = bundle.join(PENDING);
        serde_json::from_slice(&std::fs::read(&path)?)
            .with_context(|| format!("Invalid {}", path.display()))
    }
}

/// Wait before the next try after `attempts` tries: 15 s, 30 s, 1 min, … up to 30 min.
pub fn backoff_ms(attempts: u32) -> u64 {
    let doublings = attempts.saturating_sub(1).min(16);
    (15_000_u64 << doublings).min(30 * 60 * 1000)
}

pub type Forward = fn(&Path, &str) -> Result<()>;

/// Queues the bundle at `dir` into `session` through `forward`. If that fails, leaves a `PENDING`
/// marker so the port owner retries it, and says so.
pub fn deliver(
    dir: &Path,
    session: &str,
    title: &str,
    key: Option<&str>,
    forward: Forward,
) -> Delivery {
    deliver_by(dir, session, title, key, forward).0
}

/// [`deliver`] through any send, keeping what a successful one returned.
pub fn deliver_by<T>(
    dir: &Path,
    session: &str,
    title: &str,
    key: Option<&str>,
    forward: impl FnOnce(&Path, &str) -> Result<T>,
) -> (Delivery, Option<T>) {
    match forward(dir, session) {
        Ok(sent) => (
            Delivery::Sent {
                title: title.to_owned(),
            },
            Some(sent),
        ),
        Err(err) => {
            let error = format!("{err:#}");
            scrawl_log(format!(
                "deliver: OpenCode send to {title} failed ({error}); retrying from the Mac"
            ));
            let pending = Pending::new(session, title, key, &error, crate::util::now_epoch_ms());
            if let Err(err) = pending.write(dir) {
                scrawl_log(format!("deliver: {err:#}"));
            }
            (
                Delivery::Queued {
                    title: title.to_owned(),
                    error,
                },
                None,
            )
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Report {
    /// Bundles that went through, with their markers.
    pub sent: Vec<(PathBuf, Pending)>,
    pub retrying: usize,
    pub given_up: usize,
}

/// Tries every pending delivery in `captures` that is due at `now`.
pub fn retry_due(captures: &Path, now: u64, forward: Forward) -> Report {
    let mut report = Report::default();
    let Ok(entries) = std::fs::read_dir(captures) else {
        return report;
    };
    let mut bundles: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            (p.extension().is_some_and(|ext| ext == "scrawl") || p.join(PROMPT).is_file())
                && p.join(PENDING).is_file()
        })
        .collect();
    bundles.sort();
    for dir in bundles {
        let mut pending = match Pending::read(&dir) {
            Ok(pending) => pending,
            Err(err) => {
                scrawl_log(format!("deliver: {err:#}; giving up on {}", dir.display()));
                let _ = std::fs::rename(dir.join(PENDING), dir.join(FAILED));
                report.given_up += 1;
                continue;
            }
        };
        if pending.next_at > now {
            continue;
        }
        match forward(&dir, &pending.session) {
            Ok(()) => {
                let _ = std::fs::remove_file(dir.join(PENDING));
                scrawl_log(format!(
                    "deliver: {} sent to {} on try {}",
                    dir.display(),
                    pending.title,
                    pending.attempts + 1
                ));
                report.sent.push((dir, pending));
            }
            Err(err) => {
                pending.attempts += 1;
                pending.last_error = short(&format!("{err:#}"));
                if now.saturating_sub(pending.first_at) >= GIVE_UP_AFTER_MS {
                    scrawl_log(format!(
                        "deliver: giving up on {} after {} tries over a day ({})",
                        dir.display(),
                        pending.attempts,
                        pending.last_error
                    ));
                    let _ = pending
                        .write(&dir)
                        .and_then(|()| Ok(std::fs::rename(dir.join(PENDING), dir.join(FAILED))?));
                    report.given_up += 1;
                } else {
                    let wait = backoff_ms(pending.attempts);
                    pending.next_at = now + wait;
                    scrawl_log(format!(
                        "deliver: {} try {} failed ({}); next in {} s",
                        dir.display(),
                        pending.attempts,
                        pending.last_error,
                        wait / 1000
                    ));
                    if let Err(err) = pending.write(&dir) {
                        scrawl_log(format!("deliver: {err:#}"));
                    }
                    report.retrying += 1;
                }
            }
        }
    }
    report
}

/// Retries pending deliveries in `captures` every 15 s for as long as the process runs;
/// `on_sent` hears about each that went through.
pub fn spawn_retries(captures: PathBuf, forward: Forward, on_sent: fn(&Path, &Pending)) {
    let _ = crate::env::spawn_named(Some("scrawl-deliver"), move || {
        loop {
            std::thread::sleep(SCAN_EVERY);
            for (dir, pending) in retry_due(&captures, crate::util::now_epoch_ms(), forward).sent {
                on_sent(&dir, &pending);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    fn temp_captures() -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("scrawl-deliver-{}", crate::util::make_bundle_id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn replies_say_whether_the_recording_reached_its_session() {
        let sent = reply_json(
            &Outcome::Done(Delivery::Sent {
                title: "UP DOWN".into(),
            }),
            "UP DOWN",
        );
        assert_eq!(
            sent,
            serde_json::json!({"ok": true, "delivered": true, "state": "sent", "sessionTitle": "UP DOWN"})
        );
        let queued = reply_json(
            &Outcome::Done(Delivery::Queued {
                title: "UP DOWN".into(),
                error: "OpenCode API failed: connection refused\nmore".into(),
            }),
            "UP DOWN",
        );
        assert_eq!(queued["delivered"], false);
        assert_eq!(queued["state"], "queued");
        assert_eq!(queued["error"], "OpenCode API failed: connection refused");
        assert_eq!(
            reply_json(&Outcome::Done(Delivery::Saved), "Mac")["state"],
            "saved"
        );
        let processing = reply_json(&Outcome::Processing, "UP DOWN");
        assert_eq!(
            (processing["delivered"].clone(), processing["state"].clone()),
            (false.into(), "processing".into())
        );
        let failed = reply_json(&Outcome::Failed("no video track".into()), "UP DOWN");
        assert_eq!(failed["ok"], false);
        assert!(failed.get("delivered").is_none());
    }

    #[test]
    fn backoff_doubles_from_15_seconds_to_30_minutes() {
        assert_eq!(backoff_ms(0), 15_000);
        assert_eq!(backoff_ms(1), 15_000);
        assert_eq!(backoff_ms(2), 30_000);
        assert_eq!(backoff_ms(5), 240_000);
        assert_eq!(backoff_ms(8), 30 * 60 * 1000);
        assert_eq!(backoff_ms(40), 30 * 60 * 1000);
    }

    static UP: AtomicBool = AtomicBool::new(false);
    static TRIES: AtomicUsize = AtomicUsize::new(0);

    fn flaky_forward(_: &Path, session: &str) -> Result<()> {
        assert_eq!(session, "ses_retry");
        TRIES.fetch_add(1, Ordering::SeqCst);
        anyhow::ensure!(UP.load(Ordering::SeqCst), "OpenCode API failed: offline");
        Ok(())
    }

    #[test]
    fn retries_when_due_then_clears_the_marker() {
        let captures = temp_captures();
        let bundle = captures.join("scrawl-1-a.scrawl");
        std::fs::create_dir_all(&bundle).unwrap();
        Pending::new("ses_retry", "Retry", Some("k"), "offline", 1_000)
            .write(&bundle)
            .unwrap();

        // Not due yet.
        let report = retry_due(&captures, 1_000 + 14_999, flaky_forward);
        assert_eq!((report.sent.len(), report.retrying), (0, 0));
        assert_eq!(TRIES.load(Ordering::SeqCst), 0);

        // Due and still failing: backs off 30 s.
        let report = retry_due(&captures, 16_000, flaky_forward);
        assert_eq!(report.retrying, 1);
        let pending = Pending::read(&bundle).unwrap();
        assert_eq!((pending.attempts, pending.next_at), (2, 46_000));
        assert_eq!(pending.last_error, "OpenCode API failed: offline");

        UP.store(true, Ordering::SeqCst);
        let report = retry_due(&captures, 46_000, flaky_forward);
        assert_eq!(report.sent.len(), 1);
        assert_eq!(report.sent[0].1.key.as_deref(), Some("k"));
        assert!(!bundle.join(PENDING).exists());
        assert_eq!(TRIES.load(Ordering::SeqCst), 2);
        std::fs::remove_dir_all(captures).unwrap();
    }

    fn down(_: &Path, _: &str) -> Result<()> {
        anyhow::bail!("OpenCode API failed: offline")
    }

    #[test]
    fn gives_up_after_a_day() {
        let captures = temp_captures();
        let bundle = captures.join("scrawl-2-b.scrawl");
        std::fs::create_dir_all(&bundle).unwrap();
        let mut pending = Pending::new("ses_x", "X", None, "offline", 0);
        pending.next_at = GIVE_UP_AFTER_MS;
        pending.write(&bundle).unwrap();
        let report = retry_due(&captures, GIVE_UP_AFTER_MS, down);
        assert_eq!(report.given_up, 1);
        assert!(!bundle.join(PENDING).exists());
        let failed: Pending =
            serde_json::from_slice(&std::fs::read(bundle.join(FAILED)).unwrap()).unwrap();
        assert_eq!(failed.attempts, 2);
        std::fs::remove_dir_all(captures).unwrap();
    }
}
