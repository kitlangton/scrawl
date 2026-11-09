//! The ink overlay on Linux: a click-through GPUI window over the display being recorded,
//! drawing `crate::capture::desktop::ink_view::OverlayView`. It's an override-redirect X11 window (through
//! XWayland on Wayland) with an empty input shape, so every click reaches the app below.
//!
//! Neither X11 nor Wayland lets an app leave its windows out of a screen recording, so the ink
//! shows in `recording.mp4` too; the keyframes draw the same strokes over it at the same place.
//!
//! On Wayland Apparent can't see where the pointer is over other apps' windows, so while ⇧ is held
//! (read from the input devices, `super::input`) the overlay takes the pointer itself
//! (`take_pointer`): the ink follows it, and clicks land on the overlay rather than the app.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use gpui::{
    Bounds, Context, WindowBackgroundAppearance, WindowBounds, WindowKind, WindowOptions, point,
    prelude::*, px, size,
};

use super::x11;
use crate::capture::desktop::ink_view::{OverlayView, Placement};
use crate::capture::engine::runtime::SharedCaptureTelemetry;

/// The open overlay's window, for `take_pointer`.
static OVERLAY: AtomicU32 = AtomicU32::new(0);
static HIDE_BETWEEN_STROKES: AtomicBool = AtomicBool::new(false);
static POINTER_TAKEN: AtomicBool = AtomicBool::new(false);
static NEXT_LIFETIME: AtomicU64 = AtomicU64::new(1);
/// The input poller and the UI tick both change this window's mapping and input shape.
#[derive(Default)]
struct OverlayState {
    generation: u64,
    lifetime: u64,
    compositor_fade: bool,
    faulted: bool,
    pending: bool,
    failed_request: bool,
    rule_ready: bool,
}

impl OverlayState {
    fn acknowledge(
        &mut self,
        lifetime: u64,
        generation: u64,
        success: bool,
        take: bool,
    ) -> Option<bool> {
        if self.lifetime != lifetime {
            return None;
        }
        if !success {
            self.faulted = true;
            self.failed_request = true;
            self.generation = self.generation.wrapping_add(1);
            return Some(false);
        }
        if self.generation == generation && (!self.faulted || take) {
            self.failed_request = false;
            Some(true)
        } else {
            None
        }
    }
}

static OVERLAY_STATE: Mutex<OverlayState> = Mutex::new(OverlayState {
    generation: 0,
    lifetime: 0,
    compositor_fade: false,
    faulted: false,
    pending: false,
    failed_request: false,
    rule_ready: false,
});

/// Intended pointer capture, not proof of mapping or acquisition. Coordinates
/// must come from fresh events on this exact window after this generation.
pub fn pointer_request() -> Option<(u32, u64)> {
    let state = OVERLAY_STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let id = OVERLAY.load(Ordering::Acquire);
    (id != 0 && POINTER_TAKEN.load(Ordering::Acquire) && !state.failed_request)
        .then_some((id, state.generation))
}

enum InkWork {
    Current,
    Cleanup(u64),
}

fn ink_worker() -> Option<&'static flume::Sender<InkWork>> {
    static WORKER: OnceLock<Option<flume::Sender<InkWork>>> = OnceLock::new();
    WORKER
        .get_or_init(|| {
            let (tx, rx) = flume::unbounded();
            std::thread::Builder::new()
                .name("ink-compositor".into())
                .spawn(move || {
                    'requests: while let Ok(work) = rx.recv() {
                        if let InkWork::Cleanup(lifetime) = work {
                            if let Err(error) = super::hyprland::disable_ink_rule(lifetime) {
                                crate::capture::engine::runtime::apparent_log(format!(
                                    "Ink runtime rule cleanup failed: {error:#}"
                                ));
                            }
                            continue;
                        }
                        let (id, lifetime, generation, needs_rule) = {
                            let mut state = OVERLAY_STATE
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            if !state.pending || !state.compositor_fade {
                                continue;
                            }
                            state.pending = false;
                            let take = POINTER_TAKEN.load(Ordering::Acquire);
                            if state.faulted && !take {
                                continue;
                            }
                            (
                                OVERLAY.load(Ordering::Acquire),
                                state.lifetime,
                                state.generation,
                                !state.rule_ready && !state.faulted,
                            )
                        };
                        if id == 0 {
                            continue;
                        }
                        let mut result = if needs_rule {
                            super::hyprland::prepare_ink_rule(id, lifetime)
                        } else {
                            Ok(())
                        };
                        let take = {
                            let mut state = OVERLAY_STATE
                                .lock()
                                .unwrap_or_else(std::sync::PoisonError::into_inner);
                            if OVERLAY.load(Ordering::Acquire) != id || state.lifetime != lifetime {
                                continue;
                            }
                            if result.is_ok() && needs_rule {
                                state.rule_ready = true;
                            }
                            if state.generation != generation && result.is_ok() {
                                continue;
                            }
                            let take = POINTER_TAKEN.load(Ordering::Acquire);
                            if result.is_ok() {
                                x11::accept_pointer(id);
                                x11::set_title(id, &super::hyprland::ink_title(id, lifetime, take));
                            }
                            take
                        };
                        if result.is_ok() {
                            result = Err(anyhow::anyhow!("Ink title did not become ready"));
                            for _ in 0..20 {
                                {
                                    let state = OVERLAY_STATE
                                        .lock()
                                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                                    if state.lifetime != lifetime || state.generation != generation
                                    {
                                        continue 'requests;
                                    }
                                }
                                match super::hyprland::ink_title_ready(id, lifetime, take) {
                                    Ok(true) => {
                                        result = Ok(());
                                        break;
                                    }
                                    Ok(false) => {
                                        std::thread::sleep(std::time::Duration::from_millis(5))
                                    }
                                    Err(error) => {
                                        result = Err(error);
                                        break;
                                    }
                                }
                            }
                        }
                        let mut state = OVERLAY_STATE
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        if OVERLAY.load(Ordering::Acquire) != id {
                            continue;
                        }
                        let take_now = POINTER_TAKEN.load(Ordering::Acquire);
                        if let Some(visible) =
                            state.acknowledge(lifetime, generation, result.is_ok(), take_now)
                        {
                            x11::set_visible(id, visible);
                        }
                        if let Err(error) = result {
                            crate::capture::engine::runtime::apparent_log(format!(
                                "Ink fade unavailable; released overlay stays hidden: {error:#}"
                            ));
                        }
                    }
                })
                .ok()?;
            Some(tx)
        })
        .as_ref()
}

/// At most one current-state request is queued, alongside lifetime cleanup.
fn reconcile_ink(state: &mut OverlayState) {
    if !state.pending {
        state.pending = ink_worker().is_some_and(|worker| worker.send(InkWork::Current).is_ok());
        if !state.pending {
            state.faulted = true;
            state.failed_request = true;
            state.generation = state.generation.wrapping_add(1);
            crate::capture::engine::runtime::apparent_log(
                "Ink compositor worker unavailable; live overlay stays hidden",
            );
        }
    }
}

/// The overlay for one display. Dropping it closes the window.
pub struct InkOverlay {
    id: u32,
    placement: Arc<Mutex<Placement>>,
}

impl InkOverlay {
    /// Covers the display holding the centre of `on` (global pixels), else the primary.
    pub fn open<T: 'static>(
        telemetry: Arc<SharedCaptureTelemetry>,
        on: Option<[f64; 4]>,
        cx: &mut Context<T>,
    ) -> Result<Self, String> {
        let monitor = on
            .and_then(|[x, y, w, h]| super::super::monitor_at(x + w / 2.0, y + h / 2.0))
            .or_else(super::super::primary_monitor)
            .ok_or("No display")?;
        let scale = monitor.scale as f32;
        let rect = monitor.rect;
        let bounds = Bounds {
            origin: point(px(rect[0] as f32 / scale), px(rect[1] as f32 / scale)),
            size: size(px(rect[2] as f32 / scale), px(rect[3] as f32 / scale)),
        };
        let placement = Arc::new(Mutex::new(Placement { rect, scale }));
        let view_placement = Arc::clone(&placement);
        // KWin 5's XWayland overlay intercepts native Wayland clicks even with an empty
        // X input shape, and so does Hyprland's, which also sends the pill's and the picker's
        // clicks to the focused app when a full-screen notification window sits above them.
        // Keep it mapped only while drawing, so the sharing dialog, the pill and the app
        // underneath stay usable. The bundle still contains the complete ink.
        let hide_between_strokes = hide_between_strokes(
            super::super::kde(),
            super::super::hyprland(),
            super::super::wayland_session(),
        );
        let handle = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    titlebar: None,
                    focus: false,
                    show: true,
                    kind: WindowKind::PopUp,
                    is_movable: false,
                    is_resizable: false,
                    is_minimizable: false,
                    window_background: WindowBackgroundAppearance::Transparent,
                    app_id: Some("dev.kitlangton.apparent".into()),
                    ..Default::default()
                },
                move |_, cx| cx.new(|_| OverlayView::new(telemetry, view_placement)),
            )
            .map_err(|error| format!("{error:#}"))?;
        let id = handle
            .update(cx, |_, window, _| x11::window_id(window))
            .ok()
            .flatten()
            .ok_or("The overlay has no X11 window")?;
        let compositor_fade = super::super::hyprland() && super::hyprland::ink_fade_available();
        let lifetime = NEXT_LIFETIME.fetch_add(1, Ordering::Relaxed);
        if compositor_fade {
            x11::set_title(id, &super::hyprland::ink_title(id, lifetime, true));
            x11::set_class(id);
        }
        x11::float(id, rect);
        // KWin rebuilds the input region when a managed window becomes override-redirect.
        let mut state = OVERLAY_STATE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        x11::set_click_through(id, true);
        HIDE_BETWEEN_STROKES.store(hide_between_strokes, Ordering::Release);
        POINTER_TAKEN.store(false, Ordering::Release);
        if hide_between_strokes {
            x11::set_visible(id, false);
        }
        OVERLAY.store(id, Ordering::Release);
        state.generation = state.generation.wrapping_add(1);
        state.lifetime = lifetime;
        state.compositor_fade = compositor_fade;
        state.faulted = false;
        state.pending = false;
        state.rule_ready = false;
        state.failed_request = false;
        if compositor_fade {
            reconcile_ink(&mut state);
        }
        Ok(Self { id, placement })
    }

    pub fn window_number(&self) -> u32 {
        self.id
    }

    /// The display it covers, global pixels.
    pub fn screen(&self) -> [f64; 4] {
        self.placement
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .rect
    }

    /// Moves to the display holding the centre of `rect` (global pixels). Returns whether it
    /// moved: false when it's on that display already or there's none there.
    pub fn move_to(&mut self, rect: [f64; 4]) -> bool {
        let [x, y, w, h] = rect;
        let Some(monitor) = super::super::monitor_at(x + w / 2.0, y + h / 2.0) else {
            return false;
        };
        let mut state = OVERLAY_STATE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        {
            let mut placement = self
                .placement
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if (0..4).all(|i| (placement.rect[i] - monitor.rect[i]).abs() < 1.0) {
                return false;
            }
            *placement = Placement {
                rect: monitor.rect,
                scale: monitor.scale as f32,
            };
        }
        if state.compositor_fade {
            x11::place_unmapped(self.id, monitor.rect);
            state.generation = state.generation.wrapping_add(1);
            state.failed_request = false;
            reconcile_ink(&mut state);
        } else {
            state.generation = state.generation.wrapping_add(1);
            x11::place(self.id, monitor.rect);
        }
        // Hyprland remaps moved override-redirect windows. Restore the same input shape and
        // visibility they had before the move; an idle overlay must not swallow pill clicks.
        let taken = POINTER_TAKEN.load(Ordering::Acquire);
        x11::set_click_through(self.id, !taken);
        if !visible_after_move(HIDE_BETWEEN_STROKES.load(Ordering::Acquire), taken) {
            x11::set_visible(self.id, false);
        }
        true
    }
}

impl Drop for InkOverlay {
    fn drop(&mut self) {
        let mut state = OVERLAY_STATE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if OVERLAY
            .compare_exchange(self.id, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            POINTER_TAKEN.store(false, Ordering::Release);
            state.generation = state.generation.wrapping_add(1);
            if state.compositor_fade {
                x11::set_visible(self.id, false);
                if let Some(worker) = ink_worker() {
                    let _ = worker.send(InkWork::Cleanup(state.lifetime));
                }
            }
            state.compositor_fade = false;
            state.lifetime = 0;
        }
        x11::request_close(self.id);
    }
}

/// Lets the overlay take the pointer (`true`), so XWayland reports where it is, or lets every
/// click through again.
pub fn take_pointer(take: bool) {
    let mut state = OVERLAY_STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let id = OVERLAY.load(Ordering::Acquire);
    if id != 0 {
        state.generation = state.generation.wrapping_add(1);
        state.failed_request = false;
        if state.compositor_fade {
            // Release input locally before waiting for the compositor. Both
            // press and release remain unmapped until their policy is accepted.
            x11::set_visible(id, false);
        }
        POINTER_TAKEN.store(take, Ordering::Release);
        x11::set_click_through(id, !take);
        if state.compositor_fade {
            reconcile_ink(&mut state);
        } else if HIDE_BETWEEN_STROKES.load(Ordering::Acquire) {
            x11::set_visible(id, take);
        }
    }
}

/// A Lua config reload discards runtime rules. Hide first, then establish a new
/// policy lifetime so old IPC replies cannot expose this surface unprotected.
pub fn compositor_reloaded() {
    let mut state = OVERLAY_STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let id = OVERLAY.load(Ordering::Acquire);
    if id == 0 || !state.compositor_fade {
        return;
    }
    x11::set_visible(id, false);
    state.generation = state.generation.wrapping_add(1);
    state.lifetime = NEXT_LIFETIME.fetch_add(1, Ordering::Relaxed);
    state.rule_ready = false;
    state.faulted = false;
    state.failed_request = false;
    reconcile_ink(&mut state);
}

fn visible_after_move(hide_between_strokes: bool, pointer_taken: bool) -> bool {
    !hide_between_strokes || pointer_taken
}

fn hide_between_strokes(kde: bool, hyprland: bool, wayland: bool) -> bool {
    (kde || hyprland) && wayland
}

#[cfg(test)]
mod tests {
    #[test]
    fn delayed_compositor_ack_cannot_remap_a_new_shift_transition() {
        let mut state = super::OverlayState {
            lifetime: 1,
            generation: 2,
            ..Default::default()
        };
        assert_eq!(state.acknowledge(1, 1, true, true), None);
        assert_eq!(
            state.acknowledge(1, state.generation, true, true),
            Some(true)
        );
        state.generation = 3;
        assert_eq!(state.acknowledge(1, 2, true, false), None);
        assert_eq!(state.acknowledge(1, 2, true, true), None);
    }

    #[test]
    fn unknown_compositor_mutation_disables_released_remaps_until_reopen() {
        let mut state = super::OverlayState {
            lifetime: 1,
            generation: 2,
            ..Default::default()
        };
        assert_eq!(state.acknowledge(1, 1, false, false), Some(false));
        assert_eq!(state.generation, 3);
        assert!(state.failed_request);
        assert_eq!(state.acknowledge(1, 3, true, false), None);
        assert_eq!(state.acknowledge(1, 3, true, true), Some(true));
        assert!(!state.failed_request);
        state = super::OverlayState {
            lifetime: 2,
            generation: 4,
            ..Default::default()
        };
        assert_eq!(state.acknowledge(1, 2, false, false), None);
        assert!(!state.faulted);
        assert_eq!(state.acknowledge(2, 4, true, false), Some(true));
    }

    #[test]
    fn picker_overlay_stays_unmapped_when_carried_into_recording() {
        // The picker creates the same overlay before telemetry.active becomes true.
        assert!(super::hide_between_strokes(false, true, true));
        assert!(super::hide_between_strokes(true, false, true));
        assert!(!super::hide_between_strokes(false, true, false));
    }

    #[test]
    fn moving_a_hyprland_overlay_does_not_unhide_it_between_strokes() {
        assert!(!super::visible_after_move(true, false));
        assert!(super::visible_after_move(true, true));
        assert!(super::visible_after_move(false, false));
    }

    #[test]
    fn shift_transition_cannot_interrupt_an_overlay_move() {
        use std::sync::mpsc;
        use std::time::Duration;

        let move_guard = super::OVERLAY_STATE.lock().unwrap();
        let (started, ready) = mpsc::channel();
        let (finished, done) = mpsc::channel();
        let input = std::thread::spawn(move || {
            started.send(()).unwrap();
            super::take_pointer(true);
            finished.send(()).unwrap();
        });
        ready.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(done.recv_timeout(Duration::from_millis(20)).is_err());
        drop(move_guard);
        done.recv_timeout(Duration::from_secs(1)).unwrap();
        input.join().unwrap();
    }
}
