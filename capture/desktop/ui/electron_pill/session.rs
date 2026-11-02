//! Bounded, parent-owned stdio transport. Rendering cannot own capture lifetime.
use super::{Command as Action, View};
use std::{
    io::{BufRead, Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Default)]
struct Mailbox {
    state: Option<serde_json::Value>,
    place: Option<[f64; 2]>,
    activate: bool,
    closed: bool,
}

pub type Selection = (Option<PathBuf>, Option<PathBuf>);

pub struct Session {
    mailbox: Arc<(Mutex<Mailbox>, Condvar)>,
    activated: Arc<AtomicBool>,
    activation: Option<Instant>,
    actions: flume::Receiver<(Instant, Action)>,
    alive: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    hovered: Arc<AtomicBool>,
    moved: Arc<AtomicBool>,
    child: Option<crate::capture::ui_process::UiChild>,
    opened: Instant,
    sent: Instant,
    pub view: View,
    generation: u64,
    selection: Selection,
    window_id: Arc<AtomicU32>,
}

fn assets() -> Option<(PathBuf, PathBuf)> {
    let executable = std::env::current_exe().ok()?;
    let parent = executable.parent()?;
    #[cfg(target_os = "macos")]
    let root = parent.parent()?.join("Resources/settings");
    #[cfg(target_os = "linux")]
    let root = parent.parent()?.join("share/apparent/settings");
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("capture/desktop/ui/frontend");
    let mut candidates = vec![(root.join("app"), root.join("runtime"))];
    if executable.starts_with(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target")) {
        candidates.push((source.clone(), source.join("node_modules/electron/dist")));
    }
    for (app, runtime) in candidates {
        #[cfg(target_os = "macos")]
        let runtime = runtime.join("Electron.app/Contents/MacOS/Electron");
        #[cfg(target_os = "linux")]
        let runtime = runtime.join("electron");
        if runtime.is_file()
            && app.join("dist/pill.html").is_file()
            && app.join("electron/pill/main.cjs").is_file()
        {
            return Some((app, runtime));
        }
    }
    None
}

impl Session {
    pub fn open(view: View, generation: u64, selection: Selection) -> anyhow::Result<Self> {
        let (app, runtime) =
            assets().ok_or_else(|| anyhow::anyhow!("Packaged recording pill unavailable"))?;
        let mut launch = Command::new(&runtime);
        launch.arg(app.join("electron/pill/main.cjs"));
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::{MetadataExt, PermissionsExt};
            let helper = std::fs::metadata(runtime.parent().unwrap().join("chrome-sandbox"))?;
            if helper.uid() != 0 || helper.permissions().mode() & 0o4000 == 0 {
                // User-owned AppImages cannot provide a privileged SUID helper.
                // Chromium must use its kernel namespace sandbox or refuse launch;
                // this never disables renderer/seccomp sandboxing.
                launch.arg("--disable-setuid-sandbox");
            }
        }
        let mut child = launch
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()?;
        let mut stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let alive = Arc::new(AtomicBool::new(true));
        let ready = Arc::new(AtomicBool::new(false));
        let window_id = Arc::new(AtomicU32::new(0));
        let identified = window_id.clone();
        let mailbox = Arc::new((Mutex::new(Mailbox::default()), Condvar::new()));
        let outgoing = mailbox.clone();
        let writing = alive.clone();
        std::thread::spawn(move || {
            loop {
                let (lock, wake) = &*outgoing;
                let mut pending = lock.lock().unwrap();
                while !pending.closed
                    && pending.state.is_none()
                    && pending.place.is_none()
                    && !pending.activate
                {
                    pending = wake.wait(pending).unwrap();
                }
                if pending.closed {
                    break;
                }
                let state = pending.state.take();
                let place = pending.place.take();
                let activate = std::mem::take(&mut pending.activate);
                drop(pending);
                let messages = place
                    .map(|point| serde_json::json!({"type":"place","point":point}))
                    .into_iter()
                    .chain(state)
                    .chain(activate.then(|| serde_json::json!({"type":"activate"})));
                let mut failed = false;
                for message in messages {
                    if serde_json::to_writer(&mut stdin, &message).is_err()
                        || stdin.write_all(b"\n").is_err()
                        || stdin.flush().is_err()
                    {
                        failed = true;
                        break;
                    }
                }
                if failed {
                    break;
                }
            }
            writing.store(false, Ordering::Release);
        });
        let activated = Arc::new(AtomicBool::new(false));
        let acknowledged = activated.clone();
        let (commands, actions) = flume::bounded(16);
        let hovered = Arc::new(AtomicBool::new(false));
        let hovering = hovered.clone();
        let moved = Arc::new(AtomicBool::new(false));
        let moving = moved.clone();
        let reading = alive.clone();
        let mounted = ready.clone();
        std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(stdout);
            loop {
                let mut line = Vec::new();
                // Bound each read before allocation; a malformed child cannot grow this queue.
                let result = reader.by_ref().take(1025).read_until(b'\n', &mut line);
                if !matches!(result, Ok(1..=1024)) || !line.ends_with(b"\n") {
                    break;
                }
                if line == b"{\"activated\":true}\n" {
                    acknowledged.store(true, Ordering::Release);
                    continue;
                }
                if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&line) {
                    if value.get("ready").and_then(|v| v.as_bool()) == Some(true) {
                        let Some(id) = value
                            .get("windowId")
                            .and_then(|v| v.as_u64())
                            .and_then(|v| u32::try_from(v).ok())
                            .filter(|v| *v != 0)
                        else {
                            break;
                        };
                        identified.store(id, Ordering::Release);
                        mounted.store(true, Ordering::Release);
                        continue;
                    }
                    if value.get("moved").and_then(|v| v.as_bool()) == Some(true) {
                        moving.store(true, Ordering::Release);
                        continue;
                    }
                    if let Some(value) = value.get("hovered").and_then(|v| v.as_bool()) {
                        hovering.store(value, Ordering::Release);
                        continue;
                    }
                }
                let Ok(command) = serde_json::from_slice::<Action>(&line) else {
                    break;
                };
                if commands.try_send((Instant::now(), command)).is_err() {
                    break;
                }
            }
            reading.store(false, Ordering::Release);
        });
        let mut session = Self {
            mailbox,
            activated,
            activation: None,
            actions,
            alive,
            ready,
            hovered,
            moved,
            child: Some(crate::capture::ui_process::UiChild::new(child)),
            opened: Instant::now(),
            sent: Instant::now() - Duration::from_secs(1),
            view,
            generation,
            selection,
            window_id,
        };
        session.send();
        Ok(session)
    }
    /// Reported by Electron main's native API before readiness/activation.
    /// Never derived from onscreen discovery or a renderer argument.
    pub fn window_number(&self) -> Option<u32> {
        match self.window_id.load(Ordering::Acquire) {
            0 => None,
            id => Some(id),
        }
    }
    pub fn healthy(&self) -> bool {
        self.alive.load(Ordering::Acquire)
            && (self.ready() || self.opened.elapsed() < Duration::from_secs(5))
            && self.activation.is_none_or(|since| {
                self.activated.load(Ordering::Acquire) || since.elapsed() < Duration::from_secs(1)
            })
    }
    pub fn ready(&self) -> bool {
        self.ready.load(Ordering::Acquire)
    }
    pub fn moved(&self) -> bool {
        self.moved.load(Ordering::Acquire)
    }
    pub fn hovered(&self) -> bool {
        self.hovered.load(Ordering::Acquire)
    }
    pub fn update(&mut self, mut view: View, generation: u64, selection: Selection) {
        view.revision = self.view.revision;
        if view.phase == self.view.phase
            && generation == self.generation
            && selection == self.selection
        {
            view.notice = self.view.notice.clone();
        }
        let mut semantic = view.clone();
        semantic.clock.clear();
        semantic.input_level = None;
        let mut previous = self.view.clone();
        previous.clock.clear();
        previous.input_level = None;
        let changed =
            semantic != previous || self.generation != generation || self.selection != selection;
        if changed {
            view.revision += 1;
        }
        self.generation = generation;
        self.selection = selection;
        self.view = view;
        if changed || self.sent.elapsed() >= Duration::from_millis(100) {
            self.send();
        }
    }
    fn send(&mut self) {
        let (lock, wake) = &*self.mailbox;
        lock.lock().unwrap().state = Some(serde_json::json!({"type":"state","view":self.view}));
        self.sent = Instant::now();
        wake.notify_one();
    }
    pub fn activate(&mut self) {
        self.activation = Some(Instant::now());
        let (lock, wake) = &*self.mailbox;
        lock.lock().unwrap().activate = true;
        wake.notify_one();
    }
    pub fn place(&self, point: [f64; 2]) {
        self.moved.store(false, Ordering::Release);
        let (lock, wake) = &*self.mailbox;
        lock.lock().unwrap().place = Some(point);
        wake.notify_one();
    }
    pub fn actions(&mut self) -> Vec<crate::capture::desktop::native::pill::PillAction> {
        let mut admitted = vec![];
        let before = self.view.revision;
        while let Ok((received, command)) = self.actions.try_recv() {
            if let Some(action) = self
                .view
                .consume(command, received.elapsed() < Duration::from_secs(1))
            {
                admitted.push(action);
            }
        }
        if self.view.revision != before {
            self.send();
        }
        admitted
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        let (lock, wake) = &*self.mailbox;
        lock.lock().unwrap().closed = true;
        wake.notify_one();
        if let Some(mut child) = self.child.take() {
            std::thread::spawn(move || {
                for _ in 0..20 {
                    if !matches!(child.try_wait(), Ok(None)) {
                        return;
                    }
                    std::thread::sleep(Duration::from_millis(50));
                }
                let _ = child.kill();
                let _ = child.wait();
            });
        }
    }
}
