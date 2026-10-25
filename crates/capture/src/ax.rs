//! What's under a click or a mark, from the Accessibility tree (the `agentContext` experiment):
//! the element's role, label, identifier and the document its window shows. Lookups run on one
//! background thread with a short messaging timeout, so a hung app never holds up input. Secure
//! text fields are never read, input fields' values are never read, and text is cut short.

#[cfg(target_os = "macos")]
use core_foundation::base::{CFType, CFTypeRef, TCFType};
#[cfg(target_os = "macos")]
use core_foundation::string::{CFString, CFStringRef};
use std::collections::HashMap;
#[cfg(target_os = "macos")]
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

#[cfg(target_os = "macos")]
type AxElement = *const c_void;

#[cfg(target_os = "macos")]
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateSystemWide() -> AxElement;
    fn AXUIElementCreateApplication(pid: i32) -> AxElement;
    fn AXUIElementCopyElementAtPosition(
        element: AxElement,
        x: f32,
        y: f32,
        out: *mut AxElement,
    ) -> i32;
    fn AXUIElementCopyAttributeValue(
        element: AxElement,
        attribute: CFStringRef,
        out: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementSetMessagingTimeout(element: AxElement, seconds: f32) -> i32;
    fn AXUIElementGetPid(element: AxElement, pid: *mut i32) -> i32;
    fn AXUIElementGetTypeID() -> usize;
}

/// Longest label, identifier or file name kept.
const MAX_CHARS: usize = 60;

/// Each Accessibility message to an app gives up after this.
#[cfg(target_os = "macos")]
const MESSAGE_TIMEOUT_S: f32 = 0.1;

/// A lookup stops reading more attributes after this, whatever it has by then.
const LOOKUP_BUDGET: Duration = Duration::from_millis(250);

/// How long finishing a capture waits for lookups still running.
const DRAIN_WAIT: Duration = Duration::from_millis(400);

/// The element under a point.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hit {
    /// Human role, as the app describes it (`button`, `text field`, `tab`).
    pub role: String,
    pub label: Option<String>,
    pub identifier: Option<String>,
    /// The file name of the document the element's window shows (`AXDocument`).
    pub document: Option<String>,
}

impl Hit {
    /// `“New Tab” #tab-new in main.rs`: what `Click.label` holds (the role goes in `Click.role`).
    pub fn label_text(&self) -> Option<String> {
        let mut parts = Vec::new();
        if let Some(label) = &self.label {
            parts.push(format!("“{label}”"));
        }
        if let Some(id) = &self.identifier {
            parts.push(format!("#{id}"));
        }
        if let Some(doc) = &self.document {
            parts.push(format!("in {doc}"));
        }
        (!parts.is_empty()).then(|| parts.join(" "))
    }

    /// `button “Area” #area`: what `Stroke.on` holds. `None` when the element has nothing to name
    /// it by but its role.
    pub fn on_text(&self) -> Option<String> {
        self.label_text()
            .map(|text| format!("{} {text}", self.role))
    }
}

/// One line, at most `MAX_CHARS`.
pub fn clip(raw: &str) -> Option<String> {
    let compact = raw.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.is_empty() {
        return None;
    }
    Some(if compact.chars().count() <= MAX_CHARS {
        compact
    } else {
        let cut: String = compact.chars().take(MAX_CHARS - 1).collect();
        format!("{}…", cut.trim_end())
    })
}

/// Roles whose value is what the user typed; their value is never read.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn is_input(role: &str, subrole: &str) -> bool {
    subrole == "AXSecureTextField"
        || matches!(
            role,
            "AXSecureTextField" | "AXTextField" | "AXTextArea" | "AXComboBox" | "AXSearchField"
        )
}

/// Roles that are usually a piece of a control (its text or icon), so the control names it.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn is_part(role: &str) -> bool {
    matches!(role, "AXStaticText" | "AXImage" | "AXGroup" | "AXUnknown")
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn is_control(role: &str) -> bool {
    matches!(
        role,
        "AXButton"
            | "AXLink"
            | "AXMenuItem"
            | "AXMenuBarItem"
            | "AXCheckBox"
            | "AXRadioButton"
            | "AXTab"
            | "AXPopUpButton"
            | "AXMenuButton"
            | "AXDisclosureTriangle"
            | "AXCell"
            | "AXRow"
            | "AXSlider"
    )
}

/// Owns a CoreFoundation value returned under the create rule.
#[cfg(target_os = "macos")]
struct Owned(CFTypeRef);

#[cfg(target_os = "macos")]
impl Drop for Owned {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { core_foundation::base::CFRelease(self.0) };
        }
    }
}

#[cfg(target_os = "macos")]
fn attribute(element: AxElement, name: &'static str) -> Option<Owned> {
    let key = CFString::from_static_string(name);
    let mut out: CFTypeRef = std::ptr::null();
    let status =
        unsafe { AXUIElementCopyAttributeValue(element, key.as_concrete_TypeRef(), &raw mut out) };
    (status == 0 && !out.is_null()).then_some(Owned(out))
}

#[cfg(target_os = "macos")]
fn string_attribute(element: AxElement, name: &'static str) -> Option<String> {
    let value = attribute(element, name)?;
    let value = unsafe { CFType::wrap_under_get_rule(value.0) };
    value
        .downcast::<CFString>()
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty())
}

#[cfg(target_os = "macos")]
fn element_attribute(element: AxElement, name: &'static str) -> Option<Owned> {
    let value = attribute(element, name)?;
    let is_element =
        unsafe { core_foundation::base::CFGetTypeID(value.0) } == unsafe { AXUIElementGetTypeID() };
    is_element.then_some(value)
}

#[cfg(target_os = "macos")]
fn pid_of(element: AxElement) -> Option<i32> {
    let mut pid = 0;
    (unsafe { AXUIElementGetPid(element, &raw mut pid) } == 0).then_some(pid)
}

#[cfg(target_os = "macos")]
fn element_at(root: AxElement, x: f64, y: f64) -> Option<Owned> {
    let mut out: AxElement = std::ptr::null();
    let status =
        unsafe { AXUIElementCopyElementAtPosition(root, x as f32, y as f32, &raw mut out) };
    (status == 0 && !out.is_null()).then_some(Owned(out))
}

/// The topmost element at a global top-left point, looking past Scrawl's own windows. Blocking;
/// call it off the main thread.
#[cfg(target_os = "macos")]
pub fn hit_test(x: f64, y: f64) -> Option<Hit> {
    let started = Instant::now();
    let own = std::process::id() as i32;
    let system = Owned(unsafe { AXUIElementCreateSystemWide() });
    unsafe { AXUIElementSetMessagingTimeout(system.0, MESSAGE_TIMEOUT_S) };
    let mut element = element_at(system.0, x, y)?;
    if pid_of(element.0) == Some(own) {
        let pid = objc2::rc::autoreleasepool(|_| crate::windows::raw_windows())
            .into_iter()
            .find(|w| {
                let [wx, wy, ww, wh] = w.bounds;
                w.pid != own && w.alpha > 0.01 && x >= wx && x <= wx + ww && y >= wy && y <= wy + wh
            })?
            .pid;
        let app = Owned(unsafe { AXUIElementCreateApplication(pid) });
        unsafe { AXUIElementSetMessagingTimeout(app.0, MESSAGE_TIMEOUT_S) };
        element = element_at(app.0, x, y)?;
    }
    let over = || started.elapsed() > LOOKUP_BUDGET;

    let mut role = string_attribute(element.0, "AXRole").unwrap_or_default();
    // A button's text or icon names the button.
    if is_part(&role) {
        let mut parent = element_attribute(element.0, "AXParent");
        for _ in 0..2 {
            let Some(candidate) = parent.take() else {
                break;
            };
            if over() {
                break;
            }
            let parent_role = string_attribute(candidate.0, "AXRole").unwrap_or_default();
            if is_control(&parent_role) {
                element = candidate;
                role = parent_role;
                break;
            }
            parent = element_attribute(candidate.0, "AXParent");
        }
    }
    let subrole = string_attribute(element.0, "AXSubrole").unwrap_or_default();
    let input = is_input(&role, &subrole);
    let role_description = string_attribute(element.0, "AXRoleDescription")
        .or_else(|| role.strip_prefix("AX").map(str::to_lowercase))
        .unwrap_or_default();
    let mut hit = Hit {
        role: clip(&role_description).unwrap_or_else(|| "element".into()),
        ..Hit::default()
    };
    for name in ["AXTitle", "AXDescription", "AXValue", "AXPlaceholderValue"] {
        if over() || hit.label.is_some() {
            break;
        }
        if name == "AXValue" && input {
            continue;
        }
        hit.label = string_attribute(element.0, name).and_then(|s| clip(&s));
    }
    if !over() {
        hit.identifier = string_attribute(element.0, "AXIdentifier")
            .or_else(|| string_attribute(element.0, "AXDOMIdentifier"))
            .and_then(|s| clip(&s));
    }
    if !over()
        && let Some(window) = element_attribute(element.0, "AXWindow")
    {
        hit.document = string_attribute(window.0, "AXDocument")
            .and_then(|url| document_name(&url))
            .and_then(|s| clip(&s));
    }
    Some(hit)
}

/// The element at a global point (physical pixels) from UI Automation, looking past Scrawl's own
/// windows. Blocking; call it off the main thread.
#[cfg(target_os = "windows")]
pub fn hit_test(x: f64, y: f64) -> Option<Hit> {
    crate::win::uia::hit_test(x, y, LOOKUP_BUDGET)
}

/// `file:///Users/kit/src/main.rs` → `main.rs`.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn document_name(url: &str) -> Option<String> {
    let path = url.strip_prefix("file://").unwrap_or(url);
    let name = path.trim_end_matches('/').rsplit('/').next()?;
    let decoded = percent_decode(name);
    (!decoded.is_empty()).then_some(decoded)
}

pub(crate) fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(byte) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// What a lookup was for, so its result can be put on the right click or stroke at finish.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// The click or drag that began at this capture time.
    Pointer(u64),
    /// The stroke that began at this capture time.
    Stroke(u64),
}

type Job = (u64, Target, f64, f64);

pub const MAX_PENDING_JOBS: usize = 64;

/// Runs lookups for one capture at a time on a background thread and keeps their results until
/// the capture finishes.
pub struct Prober {
    jobs: flume::Sender<Job>,
    session: Arc<AtomicU64>,
    pending: Arc<AtomicUsize>,
    dropped: Arc<AtomicUsize>,
    hits: Arc<Mutex<HashMap<Target, Hit>>>,
}

impl Prober {
    pub fn shared() -> &'static Prober {
        static PROBER: OnceLock<Prober> = OnceLock::new();
        PROBER.get_or_init(|| Prober::start(hit_test))
    }

    fn start(lookup: fn(f64, f64) -> Option<Hit>) -> Self {
        Self::start_with_capacity(MAX_PENDING_JOBS, lookup)
    }

    fn start_with_capacity(capacity: usize, lookup: fn(f64, f64) -> Option<Hit>) -> Self {
        let (jobs, queue) = flume::bounded::<Job>(capacity.max(1));
        let session = Arc::new(AtomicU64::new(0));
        let pending = Arc::new(AtomicUsize::new(0));
        let dropped = Arc::new(AtomicUsize::new(0));
        let hits = Arc::new(Mutex::new(HashMap::new()));
        let (worker_session, worker_pending, worker_hits) = (
            Arc::clone(&session),
            Arc::clone(&pending),
            Arc::clone(&hits),
        );
        let _ = std::thread::Builder::new()
            .name("scrawl-ax".into())
            .spawn(move || {
                for (job_session, target, x, y) in queue {
                    if job_session == worker_session.load(Ordering::Acquire)
                        && let Some(hit) = lookup(x, y)
                    {
                        worker_hits
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner)
                            .insert(target, hit);
                    }
                    worker_pending.fetch_sub(1, Ordering::AcqRel);
                }
            });
        Self {
            jobs,
            session,
            pending,
            dropped,
            hits,
        }
    }

    /// Forgets the last capture's results; lookups still queued for it are skipped.
    pub fn begin(&self) {
        self.session.fetch_add(1, Ordering::AcqRel);
        self.dropped.store(0, Ordering::Release);
        self.hits
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
    }

    pub fn dropped(&self) -> usize {
        self.dropped.load(Ordering::Acquire)
    }

    /// Looks up the element at a global top-left point for `target`, in the background.
    /// Drops the lookup without blocking if the queue is full.
    pub fn probe(&self, target: Target, x: f64, y: f64) {
        self.pending.fetch_add(1, Ordering::AcqRel);
        if self
            .jobs
            .try_send((self.session.load(Ordering::Acquire), target, x, y))
            .is_err()
        {
            self.pending.fetch_sub(1, Ordering::AcqRel);
            if self.dropped.fetch_add(1, Ordering::AcqRel) == 0 {
                crate::log::scrawl_log("ax: lookup queue full, dropping probe");
            }
        }
    }

    /// Waits briefly for running lookups, then takes this capture's results.
    pub fn finish(&self) -> HashMap<Target, Hit> {
        let deadline = Instant::now() + DRAIN_WAIT;
        while self.pending.load(Ordering::Acquire) > 0 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        std::mem::take(
            &mut *self
                .hits
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }
}

/// Puts lookup results on the clicks, drags and strokes they were for.
pub fn apply(
    hits: &HashMap<Target, Hit>,
    events: &mut [crate::model::InteractionEvent],
    strokes: &mut [crate::model::Stroke],
) {
    use crate::model::InteractionEvent;
    for event in events.iter_mut() {
        match event {
            InteractionEvent::Click { t, label, role, .. } => {
                if let Some(hit) = hits.get(&Target::Pointer(*t)) {
                    *role = Some(hit.role.clone());
                    *label = hit.label_text().or(label.take());
                }
            }
            InteractionEvent::Drag { t, label, .. } => {
                if let Some(text) = hits.get(&Target::Pointer(*t)).and_then(Hit::on_text) {
                    *label = Some(text);
                }
            }
            _ => {}
        }
    }
    for stroke in strokes.iter_mut().filter(|s| !s.laser && s.on.is_none()) {
        if let Some(text) = hits.get(&Target::Stroke(stroke.t0)).and_then(Hit::on_text) {
            stroke.on = Some(text);
        }
    }
}

/// Where to look for what a stroke marks: the middle of its box, or just above a flat stroke
/// (an underline sits under the thing it marks). Global top-left points.
pub fn stroke_probe_point(
    stroke: &crate::model::Stroke,
    frame: crate::model::TargetFrame,
) -> Option<(f64, f64)> {
    let points = stroke.normalized_points();
    if points.is_empty() {
        return None;
    }
    let [x0, y0, x1, y1] = crate::sketch::bounds(&points);
    let (w, h) = (
        f64::from(x1 - x0) * frame.width,
        f64::from(y1 - y0) * frame.height,
    );
    let cx = frame.x + f64::from(x0 + x1) / 2.0 * frame.width;
    let top = frame.y + f64::from(y0) * frame.height;
    if h < 24.0 && w > 3.0 * h {
        Some((cx, top - 8.0))
    } else {
        Some((cx, top + h / 2.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    #[test]
    fn labels_are_one_short_line_and_name_the_element() {
        assert_eq!(clip("  New\n  Tab "), Some("New Tab".into()));
        assert_eq!(clip("   "), None);
        let long = "x".repeat(200);
        let clipped = clip(&long).unwrap();
        assert_eq!(clipped.chars().count(), MAX_CHARS);
        assert!(clipped.ends_with('…'));

        let hit = Hit {
            role: "button".into(),
            label: Some("New Tab".into()),
            identifier: Some("tab-new".into()),
            document: Some("main.rs".into()),
        };
        assert_eq!(
            hit.label_text().as_deref(),
            Some("“New Tab” #tab-new in main.rs")
        );
        assert_eq!(
            hit.on_text().as_deref(),
            Some("button “New Tab” #tab-new in main.rs")
        );
        let bare = Hit {
            role: "group".into(),
            ..Hit::default()
        };
        assert_eq!(bare.label_text(), None);
        assert_eq!(bare.on_text(), None, "a role alone names nothing");
        assert_eq!(
            document_name("file:///Users/k/My%20Notes.md").as_deref(),
            Some("My Notes.md")
        );
    }

    #[test]
    fn typed_values_are_never_read() {
        assert!(is_input("AXTextField", "AXSecureTextField"));
        assert!(is_input("AXSecureTextField", ""));
        assert!(is_input("AXTextArea", ""));
        assert!(!is_input("AXStaticText", ""));
        assert!(!is_input("AXButton", ""));
    }

    #[test]
    fn results_land_on_the_click_or_stroke_they_were_for() {
        let mut hits = HashMap::new();
        hits.insert(
            Target::Pointer(100),
            Hit {
                role: "button".into(),
                label: Some("Area".into()),
                ..Hit::default()
            },
        );
        hits.insert(
            Target::Stroke(500),
            Hit {
                role: "tab".into(),
                label: Some("Window".into()),
                ..Hit::default()
            },
        );
        let mut events = vec![
            InteractionEvent::Click {
                t: 100,
                x: 0.1,
                y: 0.1,
                button: None,
                label: None,
                role: None,
            },
            InteractionEvent::Click {
                t: 200,
                x: 0.1,
                y: 0.1,
                button: None,
                label: None,
                role: None,
            },
        ];
        let stroke = |t0| Stroke {
            t0,
            t1: t0 + 10,
            fw: 100,
            fh: 100,
            on: None,
            pin: None,
            laser: false,
            pts: vec![0, 0, 0],
        };
        let mut strokes = vec![stroke(500), stroke(600)];
        apply(&hits, &mut events, &mut strokes);
        assert!(
            matches!(&events[0], InteractionEvent::Click { label: Some(l), role: Some(r), .. } if l == "“Area”" && r == "button")
        );
        assert!(matches!(
            &events[1],
            InteractionEvent::Click {
                label: None,
                role: None,
                ..
            }
        ));
        assert_eq!(strokes[0].on.as_deref(), Some("tab “Window”"));
        assert_eq!(strokes[1].on, None);
    }

    #[test]
    fn the_prober_keeps_only_this_captures_results() {
        fn fake(x: f64, _y: f64) -> Option<Hit> {
            (x > 0.0).then(|| Hit {
                role: "button".into(),
                label: Some(format!("{x}")),
                ..Hit::default()
            })
        }
        let prober = Prober::start(fake);
        prober.begin();
        prober.probe(Target::Pointer(1), 5.0, 0.0);
        prober.probe(Target::Pointer(2), -1.0, 0.0);
        let hits = prober.finish();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[&Target::Pointer(1)].label.as_deref(), Some("5"));
        prober.begin();
        assert!(prober.finish().is_empty());
    }

    #[test]
    fn prober_queue_is_bounded_and_drops_without_blocking() {
        static BLOCK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
        fn slow(x: f64, _y: f64) -> Option<Hit> {
            while BLOCK.load(Ordering::Acquire) {
                std::thread::sleep(Duration::from_millis(2));
            }
            Some(Hit {
                role: "button".into(),
                label: Some(format!("{x}")),
                ..Hit::default()
            })
        }
        BLOCK.store(true, Ordering::Release);
        let prober = Prober::start_with_capacity(2, slow);
        prober.begin();
        for i in 0..10 {
            prober.probe(Target::Pointer(i), i as f64, 0.0);
        }
        assert!(prober.dropped() >= 7, "dropped={}", prober.dropped());
        BLOCK.store(false, Ordering::Release);
        let hits = prober.finish();
        assert!(!hits.is_empty());
    }

    #[test]
    fn underlines_are_probed_just_above() {
        let frame = TargetFrame {
            x: 100.0,
            y: 50.0,
            width: 1000.0,
            height: 1000.0,
            scale: 2.0,
        };
        let flat = Stroke {
            t0: 0,
            t1: 10,
            fw: 1000,
            fh: 1000,
            on: None,
            pin: None,
            laser: false,
            pts: vec![1000, 5000, 0, 3000, 5050, 5],
        };
        let (x, y) = stroke_probe_point(&flat, frame).unwrap();
        assert!(
            (x - 300.0).abs() < 0.5 && (y - 542.0).abs() < 0.5,
            "{x} {y}"
        );
        let circle = Stroke {
            pts: vec![1000, 1000, 0, 3000, 3000, 5],
            ..flat
        };
        let (x, y) = stroke_probe_point(&circle, frame).unwrap();
        assert!(
            (x - 300.0).abs() < 0.5 && (y - 250.0).abs() < 0.5,
            "{x} {y}"
        );
    }
}
