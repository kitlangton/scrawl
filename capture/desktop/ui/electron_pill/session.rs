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
    position: Option<(u64, [f64; 2])>,
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
    #[cfg(target_os = "linux")]
    child_pid: u32,
    #[cfg(target_os = "linux")]
    pin: Arc<AtomicBool>,
    #[cfg(target_os = "linux")]
    pin_admission: Arc<Mutex<super::pin::Admission>>,
    #[cfg(target_os = "linux")]
    drag_position: Arc<Mutex<Option<[f64; 4]>>>,
    frame: Arc<Mutex<Option<(u64, [f64; 4])>>>,
}

pub(super) fn assets() -> Option<(PathBuf, PathBuf)> {
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
            && app.join("dist/pill/index.html").is_file()
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
        crate::capture::desktop::workflow::error_reporting::configure_child(&mut launch);
        // Ozone initializes before main.cjs runs. Appending this switch in JS
        // leaves current Electron on Wayland, where our X11 shape/ID contract
        // is invalid. Select XWayland at process startup instead.
        #[cfg(target_os = "linux")]
        launch.arg("--ozone-platform=x11");
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
        #[cfg(target_os = "linux")]
        let child_pid = child.id();
        let alive = Arc::new(AtomicBool::new(true));
        let ready = Arc::new(AtomicBool::new(false));
        let window_id = Arc::new(AtomicU32::new(0));
        let identified = window_id.clone();
        let frame = Arc::new(Mutex::new(None));
        let frame_readback = frame.clone();
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
                    && pending.position.is_none()
                    && !pending.activate
                {
                    pending = wake.wait(pending).unwrap();
                }
                if pending.closed {
                    break;
                }
                let state = pending.state.take();
                let place = pending.place.take();
                let position = pending.position.take();
                let activate = std::mem::take(&mut pending.activate);
                drop(pending);
                let messages = position
                    .map(|(revision, point)| serde_json::json!({"type":"position","revision":revision,"point":point}))
                    .into_iter()
                    .chain(place
                    .map(|point| serde_json::json!({"type":"place","point":point}))
                    .into_iter())
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
        let pin_needed = Arc::new(AtomicBool::new(false));
        let pin_context = Arc::new(Mutex::new(None::<String>));
        #[cfg(target_os = "linux")]
        let pin_request = pin_needed.clone();
        #[cfg(target_os = "linux")]
        let pin_diagnostics = pin_context.clone();
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
                    if let Some(frame_value) = value.get("frame") {
                        let Some(revision) = value.get("revision").and_then(|v| v.as_u64()) else { break; };
                        let Ok(rect) = serde_json::from_value::<[f64; 4]>(frame_value.clone()) else { break; };
                        if !rect.iter().all(|v| v.is_finite()) || rect[2] <= 0.0 || rect[3] <= 0.0 { break; }
                        *frame_readback.lock().unwrap() = Some((revision, rect));
                        continue;
                    }
                    if value.get("pin").and_then(|v| v.as_bool()) == Some(true) {
                        let phase = value.get("phase").and_then(|v| v.as_str())
                            .filter(|phase| matches!(*phase, "hidden" | "selecting" | "recording" | "processing" | "completed"))
                            .unwrap_or("unknown");
                        *pin_context.lock().unwrap_or_else(std::sync::PoisonError::into_inner) = Some(format!(
                            "phase={phase} child_activated={:?} renderer_ready={:?} visible={:?}",
                            value.get("activated").and_then(|v| v.as_bool()),
                            value.get("rendererReady").and_then(|v| v.as_bool()),
                            value.get("visible").and_then(|v| v.as_bool()),
                        ));
                        pin_needed.store(true, Ordering::Release);
                        continue;
                    }
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
            #[cfg(target_os = "linux")]
            child_pid,
            #[cfg(target_os = "linux")]
            pin: pin_request.clone(),
            #[cfg(target_os = "linux")]
            pin_admission: Arc::new(Mutex::new(super::pin::Admission::default())),
            #[cfg(target_os = "linux")]
            drag_position: Arc::new(Mutex::new(None)),
            frame,
        };
        #[cfg(target_os = "linux")]
        if crate::capture::desktop::linux::hyprland::running() {
            let alive = session.alive.clone();
            let id = session.window_id.clone();
            let activated = session.activated.clone();
            let admission = session.pin_admission.clone();
            std::thread::spawn(move || {
                // Compositor requests can block; neither GUI ticks nor the state
                // writer may wait for them. Coalesce remap/drag notifications.
                while alive.load(Ordering::Acquire) {
                    std::thread::sleep(Duration::from_millis(120));
                    // Preserve an early/coalesced request until main acknowledges
                    // activation. A hidden surface cannot satisfy native admission.
                    if !activated.load(Ordering::Acquire)
                        || !pin_request.swap(false, Ordering::AcqRel)
                    {
                        continue;
                    }
                    let Some(epoch) = admission.lock().unwrap().begin() else { continue; };
                    let context = pin_diagnostics.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
                        .clone().unwrap_or_else(|| "presentation context unavailable".into());
                    let current = || alive.load(Ordering::Acquire)
                        && crate::capture::ui_process::owns(child_pid as i32);
                    let error = super::pin::reconcile(&admission, epoch, current, || {
                        let xid = id.load(Ordering::Acquire);
                        anyhow::ensure!(xid != 0, "Recording pill has no native window identity");
                        crate::capture::desktop::linux::hyprland::pin_pill_for_process(xid, child_pid, || {
                            current() && admission.lock().unwrap().current(epoch)
                        })
                    }, || std::thread::sleep(Duration::from_millis(120)));
                    if let Some(error) = error {
                        crate::capture::engine::runtime::apparent_log(format!(
                            "Electron recording pill: could not pin on Hyprland: {error:#} ({context}; activation_ack={})",
                            activated.load(Ordering::Acquire),
                        ));
                        // Electron's visibility/activation acknowledgement does not
                        // prove that XWayland supplied a compositor surface. Do not
                        // leave capture without usable controls after bounded native
                        // admission fails; the coordinator restores its native pill.
                        // The GUI consumes this presentation's failure and restores
                        // native controls if it is still current at its next tick.
                    }
                }
            });
        }
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
    pub fn note_badge_anchor(&self) -> Option<[f64; 4]> {
        if !self.healthy() || !self.ready() { return None; }
        let (revision, rect) = (*self.frame.lock().ok()?)?;
        (revision == self.view.revision).then_some(rect)
    }
    pub fn healthy(&self) -> bool {
        #[cfg(target_os = "linux")]
        if !self.pin_admission.lock().unwrap().healthy() { return false; }
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
        #[cfg(target_os = "linux")]
        {
            let position = self.drag_position.lock().unwrap().take();
            if let Some(rect) = position.filter(|_| generation == self.generation) {
                self.moved.store(true, Ordering::Release);
                let (lock, wake) = &*self.mailbox;
                lock.lock().unwrap().position = Some((self.view.revision, [rect[0], rect[1]]));
                wake.notify_one();
            }
        }
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
        #[cfg(target_os = "linux")]
        self.pin_admission.lock().unwrap().update(self.view.phase != "hidden", self.generation);
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
        self.frame.lock().unwrap().take();
        #[cfg(target_os = "linux")]
        self.drag_position.lock().unwrap().take();
        let (lock, wake) = &*self.mailbox;
        let mut pending = lock.lock().unwrap();
        pending.position = None;
        pending.place = Some(point);
        drop(pending);
        wake.notify_one();
    }
    pub fn actions(&mut self) -> Vec<crate::capture::desktop::native::pill::PillAction> {
        let mut admitted = vec![];
        let before = self.view.revision;
        while let Ok((received, command)) = self.actions.try_recv() {
            // A drag changes only this owned window. Rust rechecks the current
            // capture's revision/policy before the native adapter reads the OS
            // button state and geometry. Main supplies the OS press point.
            #[cfg(target_os = "linux")]
            if command.action == "drag" {
                if let (Some(id), Some(point)) = (
                    self.window_number(),
                    self.view.drag_point(&command, received.elapsed() < Duration::from_secs(1)),
                )
                {
                    super::super::linux::begin_electron_pill_drag(
                        id, self.child_pid, self.alive.clone(), self.pin.clone(), self.drag_position.clone(), point.map(f64::from),
                    );
                }
                continue;
            }
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
        self.alive.store(false, Ordering::Release);
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

#[cfg(all(test, target_os = "linux"))]
mod native_tests {
    use super::*;

    #[test]
    #[ignore = "requires owned disposable Hyprland guest and packaged Electron assets"]
    fn cancelled_native_pin_keeps_child_for_next_capture() {
        assert!(crate::capture::desktop::linux::hyprland::running());
        let view = View {
            revision: 1, phase: "recording", clock: "00:01".into(), label: "Synthetic pin test".into(),
            warning: None, notice: None, pocket: None, reduced_motion: true,
            draggable: false, input_level: None, actions: vec![], note_count: 0,
        };
        let mut session = Session::open(view.clone(), 1, Default::default()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(8);
        while !session.ready() && Instant::now() < deadline {
            session.update(view.clone(), 1, Default::default());
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(session.ready() && session.healthy());
        session.activate();
        std::thread::sleep(Duration::from_millis(600));
        let mut hidden = view.clone();
        hidden.phase = "hidden";
        session.update(hidden, 1, Default::default());
        // Model a show/resize notification already in flight when Cancel hides
        // the surface. The real worker, child and compositor handle the request.
        session.pin.store(true, Ordering::Release);
        std::thread::sleep(Duration::from_millis(1800));
        assert!(session.healthy(), "cancelled native pin retired the reusable Electron child");
        session.update(view, 2, Default::default());
        std::thread::sleep(Duration::from_millis(600));
        assert!(session.healthy());
        crate::capture::desktop::linux::hyprland::pin_pill_for_process(
            session.window_number().unwrap(), session.child_pid, || true,
        ).expect("next capture must map the same owned Electron child");
    }
}
