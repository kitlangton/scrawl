use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Pluggable logging and metric sink for hosts (such as Hex routing to `tracing`).
///
/// Both methods must be non-blocking. [`Sink::metric`] is only called on one-shot capture
/// lifecycle milestones (`sck_ready_ms`, `audio_ready_ms`, `bundle_ready_ms`), never on
/// per-frame, per-event, or per-audio-chunk hot paths.
///
/// In library mode, no log file is written unless [`enable_file_log`] is called by the Scrawl
/// binary.
pub trait Sink: Send + Sync + 'static {
    fn log(&self, line: &str);
    fn metric(&self, name: &str, ms: f64) {
        let _ = (name, ms);
    }
}

static SINK: RwLock<Option<Arc<dyn Sink>>> = RwLock::new(None);
static FILE_LOG_ENABLED: AtomicBool = AtomicBool::new(false);

/// Installs or clears the process-global log/metric sink. Call this once during host
/// initialization before `Engine::start` (or `Engine::new` installs a bridge to `Host::log` /
/// `Host::metric` when none is set).
pub fn set_sink(sink: Option<Arc<dyn Sink>>) {
    *SINK.write().unwrap_or_else(|e| e.into_inner()) = sink;
}

/// Installs `sink` only if no sink is currently set. Used by `Engine::new` so a host's `Host::log`
/// and `Host::metric` receive `scrawl_log` and `scrawl_metric` automatically unless an explicit
/// [`set_sink`] was installed first.
pub fn set_sink_if_unset(sink: Arc<dyn Sink>) {
    let mut guard = SINK.write().unwrap_or_else(|e| e.into_inner());
    if guard.is_none() {
        *guard = Some(sink);
    }
}

pub fn has_sink() -> bool {
    SINK.read().unwrap_or_else(|e| e.into_inner()).is_some()
}

/// Enables writing to `~/Library/Logs/Scrawl/scrawl.log` when no custom [`Sink`] is set.
/// Disabled by default in library mode; only the Scrawl binary enables it.
pub fn enable_file_log() {
    FILE_LOG_ENABLED.store(true, Ordering::Release);
}

pub fn set_file_log_enabled(enabled: bool) {
    FILE_LOG_ENABLED.store(enabled, Ordering::Release);
}

pub fn file_log_enabled() -> bool {
    FILE_LOG_ENABLED.load(Ordering::Acquire)
}

pub fn log_dir() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        dirs::home_dir().map(|home| home.join("Library/Logs/Scrawl"))
    }
    #[cfg(target_os = "windows")]
    {
        Some(
            dirs::data_local_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("scrawl")
                .join("Logs"),
        )
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

thread_local! {
    static IN_SINK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

struct SinkGuard;
impl Drop for SinkGuard {
    fn drop(&mut self) {
        IN_SINK.with(|in_sink| in_sink.set(false));
    }
}

pub fn scrawl_log(msg: impl AsRef<str>) {
    use std::io::Write;
    let msg = msg.as_ref();
    let already_in_sink = IN_SINK.with(|in_sink| in_sink.replace(true));
    if !already_in_sink {
        let _guard = SinkGuard;
        if let Some(sink) = SINK.read().unwrap_or_else(|e| e.into_inner()).clone() {
            sink.log(msg);
            return;
        }
    }
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let line = format!("[{ts}] {msg}\n");
    if cfg!(test) {
        eprint!("{line}");
        return;
    }
    if !file_log_enabled() {
        return;
    }
    eprint!("{line}");
    let Some(dir) = log_dir() else {
        return;
    };
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("scrawl.log"))
    {
        let _ = f.write_all(line.as_bytes());
    }
}

pub fn record_metric(name: &str, ms: f64) {
    let already_in_sink = IN_SINK.with(|in_sink| in_sink.replace(true));
    if !already_in_sink {
        let _guard = SinkGuard;
        if let Some(sink) = SINK.read().unwrap_or_else(|e| e.into_inner()).clone() {
            sink.metric(name, ms);
        }
    }
    if std::env::var_os("SCRAWL_BENCH_START").is_some() {
        eprintln!("START_METRIC {name}={ms:.6}");
    }
}

pub fn start_metric(name: &str, started: Instant) {
    record_metric(name, started.elapsed().as_secs_f64() * 1_000.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct TestSink {
        logs: Mutex<Vec<String>>,
        metrics: Mutex<Vec<String>>,
    }

    impl Sink for TestSink {
        fn log(&self, line: &str) {
            self.logs.lock().unwrap().push(line.to_owned());
        }
        fn metric(&self, name: &str, _ms: f64) {
            self.metrics.lock().unwrap().push(name.to_owned());
        }
    }

    #[test]
    fn pluggable_sink_receives_logs_and_metrics() {
        let sink = Arc::new(TestSink {
            logs: Mutex::new(Vec::new()),
            metrics: Mutex::new(Vec::new()),
        });
        set_sink(Some(sink.clone()));
        scrawl_log("test_pluggable_sink_marker");
        start_metric("test_metric_marker", Instant::now());
        set_sink(None);

        assert!(
            sink.logs
                .lock()
                .unwrap()
                .iter()
                .any(|l| l == "test_pluggable_sink_marker")
        );
        assert!(
            sink.metrics
                .lock()
                .unwrap()
                .iter()
                .any(|m| m == "test_metric_marker")
        );
    }
}
