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

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

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
/// The input poller and the UI tick both change this window's mapping and input shape.
static OVERLAY_STATE: Mutex<()> = Mutex::new(());

/// The overlay for one display. Dropping it closes the window.
pub struct InkOverlay {
    id: u32,
    placement: Arc<Mutex<Placement>>,
    telemetry: Arc<SharedCaptureTelemetry>,
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
        let view_telemetry = Arc::clone(&telemetry);
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
                move |_, cx| cx.new(|_| OverlayView::new(view_telemetry, view_placement)),
            )
            .map_err(|error| format!("{error:#}"))?;
        let id = handle
            .update(cx, |_, window, _| x11::window_id(window))
            .ok()
            .flatten()
            .ok_or("The overlay has no X11 window")?;
        x11::float(id, rect);
        // KWin rebuilds the input region when a managed window becomes override-redirect.
        let _state = OVERLAY_STATE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        x11::set_click_through(id, true);
        HIDE_BETWEEN_STROKES.store(hide_between_strokes, Ordering::Release);
        POINTER_TAKEN.store(false, Ordering::Release);
        if hide_between_strokes {
            x11::set_visible(id, false);
        }
        OVERLAY.store(id, Ordering::Release);
        Ok(Self {
            id,
            placement,
            telemetry,
        })
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
        let rect = if spans_desktop(self.telemetry.active.load(Ordering::Acquire),
            self.telemetry.capture_mode.load(Ordering::Acquire)) {
            desktop_bounds(&crate::capture::engine::runtime::screens::current())
                .unwrap_or(monitor.rect)
        } else {
            monitor.rect
        };
        let _state = OVERLAY_STATE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        {
            let mut placement = self
                .placement
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if (0..4).all(|i| (placement.rect[i] - rect[i]).abs() < 1.0) {
                return false;
            }
            *placement = Placement {
                rect,
                scale: monitor.scale as f32,
            };
        }
        x11::place(self.id, rect);
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
        let _state = OVERLAY_STATE
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if OVERLAY
            .compare_exchange(self.id, 0, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
        {
            POINTER_TAKEN.store(false, Ordering::Release);
        }
        x11::request_close(self.id);
    }
}

/// Lets the overlay take the pointer (`true`), so XWayland reports where it is, or lets every
/// click through again.
pub fn take_pointer(take: bool) {
    let _state = OVERLAY_STATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let id = OVERLAY.load(Ordering::Acquire);
    if id != 0 {
        POINTER_TAKEN.store(take, Ordering::Release);
        x11::set_click_through(id, !take);
        if HIDE_BETWEEN_STROKES.load(Ordering::Acquire) {
            x11::set_visible(id, take);
        }
    }
}

fn spans_desktop(active: bool, mode: u64) -> bool { active && mode <= 1 }

/// One input region keeps Shift pointer ownership continuous at monitor boundaries.
fn desktop_bounds(
    screens: &[crate::capture::engine::runtime::screens::Screen],
) -> Option<[f64; 4]> {
    let first = screens.first()?.frame;
    let (mut x, mut y, mut right, mut bottom) =
        (first[0], first[1], first[0] + first[2], first[1] + first[3]);
    for screen in &screens[1..] {
        let r = screen.frame;
        x = x.min(r[0]);
        y = y.min(r[1]);
        right = right.max(r[0] + r[2]);
        bottom = bottom.max(r[1] + r[3]);
    }
    Some([x, y, right - x, bottom - y])
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
    fn desktop_region_preserves_negative_origins_and_global_scale_units() {
        use crate::capture::engine::runtime::screens::Screen;
        let screen = |frame, scale| Screen {
            id: 1,
            frame,
            visible: frame,
            scale,
            safe_top: 0.0,
        };
        assert!(!super::spans_desktop(false, 0));
        assert!(super::spans_desktop(true, 0));
        assert!(super::spans_desktop(true, 1));
        assert!(!super::spans_desktop(true, 3), "phone mirror stays local");
        assert_eq!(super::desktop_bounds(&[]), None);
        assert_eq!(
            super::desktop_bounds(&[
                screen([-1920.0, -200.0, 1920.0, 1080.0], 1.0),
                screen([0.0, 0.0, 2560.0, 1440.0], 2.0),
            ]),
            Some([-1920.0, -200.0, 4480.0, 1640.0])
        );
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
