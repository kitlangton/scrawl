//! One lazy, capture-leased recognizer. Native state stays on its owner thread.
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::sync::{OnceLock, mpsc};
use std::time::{Duration, Instant};

use crate::shared::model::TranscriptData;
use anyhow::{Context, Result, ensure};

const IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60);

// Include file identity, not just its name: replacing a model in place must reload it.
#[derive(Clone, Debug, Eq, PartialEq)]
struct ModelKey {
    path: PathBuf,
    identity: (u64, u64, u64, i64, i64, i64, i64),
}

impl ModelKey {
    fn selected() -> Result<Self> {
        let path = crate::capture::engine::runtime::audio::transcription::speech::installed_models()
            .into_iter()
            .next()
            .context("No Parakeet model installed")?;
        Self::at(path)
    }
    fn at(path: PathBuf) -> Result<Self> {
        let meta = path.metadata()?;
        Ok(Self {
            path,
            identity: (
                meta.dev(),
                meta.ino(),
                meta.len(),
                meta.mtime(),
                meta.mtime_nsec(),
                meta.ctime(),
                meta.ctime_nsec(),
            ),
        })
    }

    fn check_current(&self) -> Result<()> {
        ensure!(
            Self::at(self.path.clone())? == *self,
            "Selected Parakeet model changed during recording"
        );
        Ok(())
    }

    fn load(&self) -> Result<crate::capture::engine::runtime::audio::transcription::speech::Incremental> {
        // Reopening an old lease after another capture changed the single cache must
        // not load replacement bytes under the old identity. No native state escapes
        // if the file changed while it was loading, either.
        self.check_current()?;
        let recognizer = crate::capture::engine::runtime::audio::transcription::speech::Incremental::load(&self.path)?;
        self.check_current()?;
        Ok(recognizer)
    }
}

struct Cache<K, R> {
    recognizer: Option<(K, R)>,
    active: usize,
    idle_since: Option<Instant>,
    discard: bool,
}

impl<K: Eq, R> Cache<K, R> {
    fn new() -> Self {
        Self {
            recognizer: None,
            active: 0,
            idle_since: None,
            discard: false,
        }
    }
    fn begin(&mut self) {
        self.active += 1;
        self.idle_since = None;
    }
    fn end(&mut self, now: Instant) {
        self.active -= 1;
        if self.active == 0 {
            self.idle_since = Some(now);
            if self.discard {
                self.recognizer = None;
                self.discard = false;
            }
        }
    }
    fn clear(&mut self) {
        if self.active == 0 {
            self.recognizer = None;
        } else {
            self.discard = true;
        }
    }
    fn wait(&self, now: Instant) -> Option<Duration> {
        self.recognizer.as_ref()?;
        self.idle_since
            .map(|since| IDLE_TIMEOUT.saturating_sub(now.saturating_duration_since(since)))
    }
    fn expire(&mut self, now: Instant) {
        if self.active == 0 && self.wait(now) == Some(Duration::ZERO) {
            self.recognizer = None;
            self.idle_since = None;
            crate::capture::engine::runtime::apparent_log("Warm recognizer: expired after 1800000 ms idle");
        }
    }
    fn run<T>(
        &mut self,
        key: K,
        load: impl FnOnce(&K) -> Result<R>,
        run: impl FnOnce(&mut R) -> Result<T>,
    ) -> Result<T> {
        let started = Instant::now();
        if self.recognizer.as_ref().is_some_and(|(old, _)| *old != key) {
            self.recognizer = None;
        }
        if self.recognizer.is_none() {
            let recognizer = load(&key)?;
            self.recognizer = Some((key, recognizer));
            crate::capture::engine::runtime::apparent_log(format!(
                "Warm recognizer: loaded in {} ms",
                started.elapsed().as_millis()
            ));
        } else {
            crate::capture::engine::runtime::apparent_log("Warm recognizer: reused");
        }
        let started = Instant::now();
        let result = run(&mut self.recognizer.as_mut().unwrap().1);
        if result.is_err() {
            self.recognizer = None;
            crate::capture::engine::runtime::apparent_log("Warm recognizer: recognition failed; discarded session");
        }
        crate::capture::engine::runtime::apparent_log(format!(
            "Warm recognizer: recognition completed in {} ms",
            started.elapsed().as_millis()
        ));
        result
    }
}

enum Command<K> {
    Begin,
    End,
    Clear,
    Run(K, Vec<f32>, u64, mpsc::Sender<Result<TranscriptData>>),
    #[cfg(test)]
    Sweep(mpsc::Sender<()>),
    #[cfg(test)]
    Active(mpsc::Sender<usize>),
}

struct Owner<K>(mpsc::Sender<Command<K>>);

impl<K: Eq + Send + 'static> Owner<K> {
    fn spawn<R: 'static>(
        mut load: impl FnMut(&K) -> Result<R> + Send + 'static,
        mut run: impl FnMut(&mut R, &[f32], u64) -> Result<TranscriptData> + Send + 'static,
        now: impl Fn() -> Instant + Send + 'static,
    ) -> Result<Self> {
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("apparent-warm-recognizer".into())
            .spawn(move || {
                // R is constructed and destroyed here; it need not be Send or Sync.
                let mut cache = Cache::new();
                loop {
                    let command = match cache.wait(now()) {
                        Some(wait) => match receiver.recv_timeout(wait) {
                            Ok(command) => command,
                            Err(mpsc::RecvTimeoutError::Timeout) => {
                                cache.expire(now());
                                continue;
                            }
                            Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        },
                        None => match receiver.recv() {
                            Ok(command) => command,
                            Err(_) => break,
                        },
                    };
                    match command {
                        Command::Begin => {
                            cache.expire(now());
                            cache.begin();
                        }
                        Command::End => cache.end(now()),
                        Command::Clear => cache.clear(),
                        Command::Run(key, samples, offset, reply) => {
                            let result = cache.run(key, &mut load, |recognizer| {
                                run(recognizer, &samples, offset)
                            });
                            let _ = reply.send(result);
                        }
                        #[cfg(test)]
                        Command::Sweep(reply) => {
                            cache.expire(now());
                            let _ = reply.send(());
                        }
                        #[cfg(test)]
                        Command::Active(reply) => {
                            let _ = reply.send(cache.active);
                        }
                    }
                }
            })
            .context("Could not start warm recognizer owner")?;
        Ok(Self(sender))
    }
    fn begin(&self) -> Result<Lease<K>> {
        self.0
            .send(Command::Begin)
            .map_err(|_| anyhow::anyhow!("Warm recognizer owner stopped"))?;
        Ok(Lease(self.0.clone()))
    }
}

// Lives in the pause worker's closure, not in Pause: cancellation may detach that worker.
// Its End reaches the owner only AFTER the final native run and all pause processing end.
struct Lease<K>(mpsc::Sender<Command<K>>);
impl<K> Lease<K> {
    fn transcribe(&self, key: K, samples: &[f32], offset: u64) -> Result<TranscriptData> {
        let (reply, result) = mpsc::channel();
        self.0
            .send(Command::Run(key, samples.to_vec(), offset, reply))
            .map_err(|_| anyhow::anyhow!("Warm recognizer owner stopped"))?;
        result.recv().context("Warm recognizer owner stopped")?
    }
}
impl<K: Clone + Send + 'static> Lease<K> {
    fn with_selection(
        self,
        mut select: impl FnMut() -> Result<K> + Send + 'static,
    ) -> impl FnMut(&[f32], u64) -> Result<TranscriptData> + Send {
        let mut selected = None;
        move |samples: &[f32], offset| {
            if selected.is_none() {
                selected = Some(select().inspect_err(|_| {
                    let _ = self.0.send(Command::Clear);
                })?);
            }
            self.transcribe(selected.as_ref().unwrap().clone(), samples, offset)
        }
    }
}
impl<K> Drop for Lease<K> {
    fn drop(&mut self) {
        let _ = self.0.send(Command::End);
    }
}

static OWNER: OnceLock<Owner<ModelKey>> = OnceLock::new();

/// No thread or model allocation at app launch. Called at the next recording start.
pub fn clear_if_disabled() {
    if (!crate::shared::handoff::warm_recognizer() || !crate::capture::engine::runtime::audio::transcription::pause_transcribe::wanted())
        && let Some(owner) = OWNER.get()
    {
        let _ = owner.0.send(Command::Clear);
    }
}

pub fn transcriber() -> Result<impl FnMut(&[f32], u64) -> Result<TranscriptData> + Send> {
    // Serialise initialization too: OnceLock::get_or_try_init is not stable.
    static INIT: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _init = INIT
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if OWNER.get().is_none() {
        let owner = Owner::spawn(
            ModelKey::load,
            |recognizer, samples, offset| recognizer.transcribe(samples, offset),
            Instant::now,
        )?;
        let _ = OWNER.set(owner);
    }
    let lease = OWNER.get().unwrap().begin()?;
    Ok(lease.with_selection(ModelKey::selected))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn selected_symlink_name_is_preserved_and_replacement_changes_identity() {
        let dir = std::env::temp_dir().join(format!("warm-model-key-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let target = dir.join("target.gguf");
        let alias = dir.join("selected.gguf");
        std::fs::write(&target, b"first").unwrap();
        let _ = std::fs::remove_file(&alias);
        std::os::unix::fs::symlink(&target, &alias).unwrap();
        let first = ModelKey::at(alias.clone()).unwrap();
        assert_eq!(
            first.path, alias,
            "transcript metadata keeps the selected name"
        );
        let replacement = dir.join("replacement.gguf");
        std::fs::write(&replacement, b"second").unwrap();
        std::fs::rename(replacement, &target).unwrap();
        assert!(
            first.check_current().is_err(),
            "a pinned replaced file must not be reloaded"
        );
        assert_ne!(first, ModelKey::at(alias).unwrap());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn selection_is_lazy_and_pinned_for_each_capture_not_each_chunk() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        let selection = Arc::new(AtomicUsize::new(1));
        let selected = Arc::clone(&selection);
        let owner = Owner::spawn(
            |key: &usize| Ok(*key),
            |key, _, _| {
                Ok(TranscriptData {
                    text: key.to_string(),
                    ..Default::default()
                })
            },
            Instant::now,
        )
        .unwrap();
        let mut transcribe = owner
            .begin()
            .unwrap()
            .with_selection(move || Ok(selected.load(Ordering::SeqCst)));
        selection.store(2, Ordering::SeqCst);
        assert_eq!(
            transcribe(&[0.1], 0).unwrap().text,
            "2",
            "selection waits for recognition"
        );
        selection.store(3, Ordering::SeqCst);
        assert_eq!(
            transcribe(&[0.1], 100).unwrap().text,
            "2",
            "a capture cannot mix models"
        );
        drop(transcribe);
        let mut next = owner
            .begin()
            .unwrap()
            .with_selection(move || Ok(selection.load(Ordering::SeqCst)));
        assert_eq!(next(&[0.1], 0).unwrap().text, "3");
    }
    #[test]
    fn warm_recognizer_reuses_across_capture_leases() {
        let mut cache = Cache::new();
        let mut loads = 0;
        for _ in 0..2 {
            cache.begin();
            cache
                .run(
                    "model",
                    |_| {
                        loads += 1;
                        Ok(())
                    },
                    |_| Ok(()),
                )
                .unwrap();
            cache.end(Instant::now());
        }
        assert_eq!(loads, 1, "the model/session must survive the first capture");
    }

    #[test]
    fn expiry_is_idle_only_and_restarts_after_the_last_active_lease() {
        let now = Instant::now();
        let mut cache = Cache::new();
        cache.begin();
        cache.run("model", |_| Ok(()), |_| Ok(())).unwrap();
        cache.end(now);
        cache.expire(now + IDLE_TIMEOUT - Duration::from_nanos(1));
        assert!(cache.recognizer.is_some());
        cache.begin();
        cache.begin();
        cache.expire(now + IDLE_TIMEOUT * 2);
        assert!(cache.recognizer.is_some());
        cache.end(now + IDLE_TIMEOUT * 2);
        cache.expire(now + IDLE_TIMEOUT * 3);
        assert!(
            cache.recognizer.is_some(),
            "a second recording still owns a lease"
        );
        cache.end(now + IDLE_TIMEOUT * 3);
        cache.expire(now + IDLE_TIMEOUT * 4 - Duration::from_nanos(1));
        assert!(cache.recognizer.is_some());
        cache.expire(now + IDLE_TIMEOUT * 4);
        assert!(cache.recognizer.is_none());
    }

    #[test]
    fn changed_models_and_failed_loads_or_runs_cannot_reuse_old_state() {
        let mut cache = Cache::new();
        cache.begin();
        let mut loads = 0;
        for key in ["first", "second"] {
            let value = cache
                .run(
                    key,
                    |key| {
                        loads += 1;
                        Ok(key.to_string())
                    },
                    |r| Ok(r.clone()),
                )
                .unwrap();
            assert_eq!(value, key);
        }
        assert_eq!(loads, 2);
        assert!(
            cache
                .run("missing", |_| anyhow::bail!("load failed"), |_| Ok(()))
                .is_err()
        );
        assert!(cache.recognizer.is_none());
        assert!(
            cache
                .run(
                    "second",
                    |_| Ok("new".into()),
                    |_| -> Result<()> { anyhow::bail!("run failed") }
                )
                .is_err()
        );
        assert!(cache.recognizer.is_none());
        assert_eq!(
            cache
                .run("second", |_| Ok("fresh".into()), |r| Ok(r.clone()))
                .unwrap(),
            "fresh"
        );
    }

    #[test]
    fn config_invalidation_waits_for_active_workers_then_discards() {
        let mut cache = Cache::new();
        cache.begin();
        cache.run("model", |_| Ok(()), |_| Ok(())).unwrap();
        cache.clear();
        assert!(cache.recognizer.is_some());
        cache.end(Instant::now());
        assert!(cache.recognizer.is_none());
        cache.begin();
        cache.run("model", |_| Ok(()), |_| Ok(())).unwrap();
        cache.end(Instant::now());
        cache.clear();
        assert!(cache.recognizer.is_none());
    }

    // Exercise the real owner/channel/lease/clock and pause worker, replacing only
    // native compute. R deliberately is !Send, just as a thread-bound engine may be.
    struct Fake {
        drops: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        _thread_bound: std::rc::Rc<()>,
    }
    impl Drop for Fake {
        fn drop(&mut self) {
            self.drops.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    }
    fn sweep(owner: &Owner<&'static str>) {
        let (tx, rx) = mpsc::channel();
        owner.0.send(Command::Sweep(tx)).unwrap();
        rx.recv_timeout(Duration::from_secs(3)).unwrap();
    }

    #[test]
    fn cancelled_pause_keeps_lease_until_native_work_ends_then_reuses_and_expires() {
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        };
        let ticks = Arc::new(AtomicUsize::new(0));
        let clock = Arc::clone(&ticks);
        let origin = Instant::now();
        let loads = Arc::new(AtomicUsize::new(0));
        let loaded = Arc::clone(&loads);
        let drops = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::clone(&drops);
        let (entered, running) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let mut first = true;
        let owner = Owner::spawn(
            move |_| {
                loaded.fetch_add(1, Ordering::SeqCst);
                Ok(Fake {
                    drops: Arc::clone(&dropped),
                    _thread_bound: std::rc::Rc::new(()),
                })
            },
            move |_, samples, offset| {
                if first {
                    first = false;
                    entered.send(()).unwrap();
                    gate.recv().unwrap();
                }
                Ok(TranscriptData {
                    text: format!("{}:{offset}", samples[0]),
                    ..Default::default()
                })
            },
            move || origin + Duration::from_secs(clock.load(Ordering::SeqCst) as u64),
        )
        .unwrap();
        let lease = owner.begin().unwrap();
        sweep(&owner);
        assert_eq!(loads.load(Ordering::SeqCst), 0, "capture start is lazy");
        let mut pcm = vec![0.2; 4 * 16_000];
        pcm.extend(vec![0.0; 16_000]);
        let pause = crate::capture::engine::runtime::audio::transcription::pause_transcribe::Pause::start_with(
            Arc::new(Mutex::new(pcm.into())),
            move |samples, offset| lease.transcribe("model", samples, offset),
        )
        .unwrap();
        running.recv_timeout(Duration::from_secs(3)).unwrap();
        drop(pause); // Must not end the lease while the native run is held.
        ticks.store(4_000, Ordering::SeqCst);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        // A new capture can queue behind the cancelled worker without a second model.
        let next = owner.begin().unwrap();
        release.send(()).unwrap();
        assert_eq!(next.transcribe("model", &[0.7], 0).unwrap().text, "0.7:0");
        sweep(&owner);
        ticks.store(8_000, Ordering::SeqCst);
        sweep(&owner);
        assert_eq!(loads.load(Ordering::SeqCst), 1);
        assert_eq!(
            drops.load(Ordering::SeqCst),
            0,
            "new capture prevents expiry"
        );
        drop(next);
        // Wait for the cancelled pause worker to release its lease, without sleeps.
        // Its thread may still be unwinding after the owner answered its run.
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            sweep(&owner);
            // A test-only snapshot of the lease count is requested below.
            let (tx, rx) = mpsc::channel();
            owner.0.send(Command::Active(tx)).unwrap();
            if rx.recv_timeout(Duration::from_secs(3)).unwrap() == 0 {
                break;
            }
            assert!(Instant::now() < deadline, "cancelled worker must end");
            std::thread::yield_now();
        }
        ticks.store(9_799, Ordering::SeqCst);
        sweep(&owner);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        ticks.store(9_800, Ordering::SeqCst);
        sweep(&owner);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
        let last = owner.begin().unwrap();
        assert_eq!(last.transcribe("model", &[0.9], 0).unwrap().text, "0.9:0");
        assert_eq!(loads.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn default_on_policy_needs_no_experiment_override() {
        const CASE: &str = "APPARENT_WARM_POLICY_TEST";
        if let Ok(case) = std::env::var(CASE) {
            assert!(!crate::shared::handoff::experiment("warmRecognizer"));
            assert_eq!(case, "default");
            let pause = crate::capture::engine::runtime::audio::transcription::pause_transcribe::Pause::start(std::sync::Arc::new(
                std::sync::Mutex::new(crate::capture::engine::runtime::audio::persistence::Samples::default()),
            ))
            .unwrap();
            let wav = crate::capture::engine::runtime::audio::audio::encode_wav_16k(&[]).unwrap();
            assert!(pause.finish(&wav).unwrap().text.is_empty());
            assert!(OWNER.get().is_some(), "an unset config must use the warm owner");
            return;
        }
        for case in ["default"] {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "capture::engine::runtime::audio::transcription::warm_recognizer::tests::default_on_policy_needs_no_experiment_override",
                    "--exact",
                ])
                .env(CASE, case)
                .output()
                .unwrap();
            let stdout = String::from_utf8_lossy(&output.stdout);
            assert!(output.status.success(), "warm policy for {case}: {stdout}");
            assert!(stdout.lines().any(|line| line == "running 1 test"), "exact warm policy selector must run one test for {case}: {stdout}");
            assert!(stdout.contains("capture::engine::runtime::audio::transcription::warm_recognizer::tests::default_on_policy_needs_no_experiment_override ... ok"), "the policy case must pass for {case}: {stdout}");
        }
    }

    #[test]
    fn finished_pause_reuses_session_but_not_text_words_or_offsets() {
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        };
        let loads = Arc::new(AtomicUsize::new(0));
        let loaded = Arc::clone(&loads);
        let owner = Owner::spawn(
            move |_| {
                loaded.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            |_, samples, offset| {
                let text = samples[0].to_string();
                Ok(TranscriptData {
                    text: text.clone(),
                    words: vec![crate::shared::model::WordTiming {
                        t0: offset,
                        t1: offset + 100,
                        text,
                    }],
                    ..Default::default()
                })
            },
            Instant::now,
        )
        .unwrap();
        for value in [0.2, 0.7] {
            let lease = owner.begin().unwrap();
            let pause = crate::capture::engine::runtime::audio::transcription::pause_transcribe::Pause::start_with(
                Arc::new(Mutex::new(crate::capture::engine::runtime::audio::persistence::Samples::default())),
                move |samples, offset| lease.transcribe("model", samples, offset),
            )
            .unwrap();
            let pcm = vec![value; 16_000];
            let wav = crate::capture::engine::runtime::audio::audio::encode_wav_16k(&pcm).unwrap();
            let result = pause.finish(&wav).unwrap();
            // WAV quantization changes the float slightly; compare with its decoded value.
            let decoded = crate::capture::engine::runtime::audio::audio::decode_wav_16k(&wav).unwrap()[0].to_string();
            assert_eq!(result.text, decoded);
            assert_eq!(result.words.len(), 1);
            assert_eq!(result.words[0].text, decoded);
            assert_eq!(result.words[0].t0, 0);
        }
        assert_eq!(loads.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn native_error_reaches_pause_fallback_and_next_capture_loads_fresh() {
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        };
        let loads = Arc::new(AtomicUsize::new(0));
        let loaded = Arc::clone(&loads);
        let owner = Owner::spawn(
            move |_| Ok(loaded.fetch_add(1, Ordering::SeqCst)),
            |id, _, _| {
                if *id == 0 {
                    anyhow::bail!("synthetic native failure");
                }
                Ok(TranscriptData {
                    text: "fresh".into(),
                    ..Default::default()
                })
            },
            Instant::now,
        )
        .unwrap();
        for succeeds in [false, true] {
            let lease = owner.begin().unwrap();
            let pause = crate::capture::engine::runtime::audio::transcription::pause_transcribe::Pause::start_with(
                Arc::new(Mutex::new(crate::capture::engine::runtime::audio::persistence::Samples::default())),
                move |samples, offset| lease.transcribe("model", samples, offset),
            )
            .unwrap();
            let wav = crate::capture::engine::runtime::audio::audio::encode_wav_16k(&vec![0.2; 16_000]).unwrap();
            let result = pause.finish(&wav);
            assert_eq!(
                result.is_ok(),
                succeeds,
                "errors must reach the existing whole-WAV fallback"
            );
            if succeeds {
                assert_eq!(result.unwrap().text, "fresh");
            }
        }
        assert_eq!(loads.load(Ordering::SeqCst), 2);
    }
}
