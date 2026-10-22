//! Displays, and the coordinate spaces Scrawl moves between on a Mac with more than one.
//!
//! - **Global** points, top-left origin at the primary display's top-left corner: `CGEvent`
//!   locations, `CGDisplayBounds`, `SCDisplay`/`SCWindow` frames and every `TargetFrame`. A
//!   display left of or above the primary one has negative coordinates.
//! - **AppKit** points, bottom-left origin at the primary display's bottom-left corner, y up:
//!   `NSScreen` and `NSWindow` frames, `NSEvent.mouseLocation`.
//! - **Local** points, top-left origin at one display's top-left corner: what an overlay
//!   covering that display draws in, and `SCStreamConfiguration.sourceRect`.
//!
//! On Windows, global coordinates are physical pixels (per-monitor DPI aware), top-left origin
//! at the primary display's top-left, and `Screen::scale` is the display's DPI scale.
//!
//! Capture pixels are a target's normalized coordinates (`SharedCaptureTelemetry::normalize_point`)
//! times its recording's size (`capture::capture_size`, at most the display's own scale).
//!
use crate::model::TargetFrame;

/// `[x, y, width, height]`.
pub type Rect = [f64; 4];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Screen {
    pub id: u32,
    /// Global points.
    pub frame: Rect,
    /// The frame less the menu bar and the Dock, global points.
    pub visible: Rect,
    /// Backing pixels per point (2 for Retina, 1 for most external displays).
    pub scale: f64,
    /// The safe area's top inset (the notch), points.
    pub safe_top: f64,
}

/// AppKit rect → global rect (and back: the flip is its own inverse). `primary_h` is the
/// primary display's height.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn flip(r: Rect, primary_h: f64) -> Rect {
    [r[0], primary_h - r[1] - r[3], r[2], r[3]]
}

pub fn contains(r: Rect, p: [f64; 2]) -> bool {
    p[0] >= r[0] && p[0] < r[0] + r[2] && p[1] >= r[1] && p[1] < r[1] + r[3]
}

fn overlap(a: Rect, b: Rect) -> f64 {
    let w = (a[0] + a[2]).min(b[0] + b[2]) - a[0].max(b[0]);
    let h = (a[1] + a[3]).min(b[1] + b[3]) - a[1].max(b[1]);
    w.max(0.0) * h.max(0.0)
}

/// The screen holding `p`, else the nearest one (a point can sit in the gap between displays
/// of different sizes). `None` only without screens.
pub fn index_at(screens: &[Screen], p: [f64; 2]) -> Option<usize> {
    screens
        .iter()
        .position(|s| contains(s.frame, p))
        .or_else(|| {
            let distance = |r: Rect| {
                let dx = (r[0] - p[0]).max(p[0] - (r[0] + r[2])).max(0.0);
                let dy = (r[1] - p[1]).max(p[1] - (r[1] + r[3])).max(0.0);
                dx.hypot(dy)
            };
            (0..screens.len())
                .min_by(|&a, &b| distance(screens[a].frame).total_cmp(&distance(screens[b].frame)))
        })
}

/// The screen a rect belongs to: the one holding its centre, else the one it overlaps most.
pub fn index_for_rect(screens: &[Screen], r: Rect) -> Option<usize> {
    let centre = [r[0] + r[2] / 2.0, r[1] + r[3] / 2.0];
    screens
        .iter()
        .position(|s| contains(s.frame, centre))
        .or_else(|| {
            (0..screens.len())
                .filter(|&i| overlap(screens[i].frame, r) > 0.0)
                .max_by(|&a, &b| {
                    overlap(screens[a].frame, r).total_cmp(&overlap(screens[b].frame, r))
                })
        })
        .or_else(|| index_at(screens, centre))
}

/// Global point → points from `screen`'s top-left corner.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn to_local(screen: Rect, p: [f64; 2]) -> [f64; 2] {
    [p[0] - screen[0], p[1] - screen[1]]
}

/// The part of `r` on `screen`; `r` itself when they don't meet.
pub fn clamp_to(r: Rect, screen: Rect) -> Rect {
    let x0 = r[0].max(screen[0]);
    let y0 = r[1].max(screen[1]);
    let x1 = (r[0] + r[2]).min(screen[0] + screen[2]);
    let y1 = (r[1] + r[3]).min(screen[1] + screen[3]);
    if x1 > x0 && y1 > y0 {
        [x0, y0, x1 - x0, y1 - y0]
    } else {
        r
    }
}

/// `SCStreamConfiguration.sourceRect` for an area: points from its display's top-left corner,
/// not global ones.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn source_rect(area: Rect, display: Rect) -> Rect {
    let [x, y, w, h] = clamp_to(area, display);
    let [lx, ly] = to_local(display, [x, y]);
    [lx, ly, w, h]
}

/// The smallest area Scrawl records, points a side.
pub const MIN_AREA: f64 = 32.0;

/// Where an area dragged in the picker is recorded: on the display holding its centre (the
/// picker already keeps the drag on one display), trimmed to it and grown to `MIN_AREA` a side
/// without leaving it, so the `TargetFrame` and the display-local `sourceRect` are the same
/// rect. Returns the display's index and the rect in global points.
pub fn area_on_display(screens: &[Screen], area: Rect) -> Option<(usize, Rect)> {
    let i = index_for_rect(screens, area)?;
    let [dx, dy, dw, dh] = screens[i].frame;
    let [cx, cy, cw, ch] = clamp_to(area, screens[i].frame);
    let grow = |start: f64, len: f64, origin: f64, extent: f64| {
        let len = len.max(MIN_AREA).min(extent);
        (start.min(origin + extent - len).max(origin), len)
    };
    let (x, w) = grow(cx, cw, dx, dw);
    let (y, h) = grow(cy, ch, dy, dh);
    Some((i, [x, y, w, h]))
}

/// `frame` with its display's scale.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn with_scale(frame: Rect, scale: f64) -> TargetFrame {
    TargetFrame {
        x: frame[0],
        y: frame[1],
        width: frame[2].max(1.0),
        height: frame[3].max(1.0),
        scale: if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            2.0
        },
    }
}

/// Where the capsule's panel (`width` points wide) goes on `screen`: centred, its top
/// `PANEL_Y` below the screen's top, and lower when a taller menu bar or the notch needs it.
pub fn pill_origin(screen: &Screen, width: f64, panel_y: f64) -> [f64; 2] {
    let menu_bar = screen.visible[1] - screen.frame[1];
    let top = panel_y.max(menu_bar - 3.0).max(screen.safe_top + 2.0);
    [
        screen.frame[0] + ((screen.frame[2] - width) / 2.0).round(),
        screen.frame[1] + top,
    ]
}

/// The GPUI pill window's rect on `screen` (Windows, global pixels): `w`×`h` points at the
/// screen's scale, placed as `pill_origin` places the capsule (below a taskbar on top).
#[cfg_attr(target_os = "macos", allow(dead_code))]
pub fn pill_window_rect(screen: &Screen, w: f64, h: f64, top: f64) -> Rect {
    let width = (w * screen.scale).round();
    let [x, y] = pill_origin(screen, width, top * screen.scale);
    [x, y, width, (h * screen.scale).round()]
}

/// Every active display from CoreGraphics (any thread). `visible` is the whole frame and
/// `safe_top` 0: only `NSScreen` knows the menu bar and the notch (`current`).
#[cfg(target_os = "macos")]
pub fn displays() -> Vec<Screen> {
    use core_graphics::display::CGDisplay;
    let ids = CGDisplay::active_displays().unwrap_or_default();
    let mut out: Vec<Screen> = ids
        .into_iter()
        .map(|id| {
            let display = CGDisplay::new(id);
            let b = display.bounds();
            let frame = [b.origin.x, b.origin.y, b.size.width, b.size.height];
            (display, frame)
        })
        // A display unplugged while this runs reports zero bounds.
        .filter(|(_, frame)| frame[2] > 0.0 && frame[3] > 0.0)
        .map(|(display, frame)| {
            let id = display.id;
            let scale = display
                .display_mode()
                .map(|m| m.pixel_width() as f64 / (m.width() as f64).max(1.0))
                .filter(|s| s.is_finite() && *s >= 1.0)
                .unwrap_or(2.0);
            Screen {
                id,
                frame,
                visible: frame,
                scale,
                safe_top: 0.0,
            }
        })
        .collect();
    // The primary display (origin 0,0) first, like `NSScreen.screens`.
    out.sort_by_key(|s| !(s.frame[0] == 0.0 && s.frame[1] == 0.0));
    out
}

/// Every screen with its menu bar, Dock and notch, from `NSScreen` (main thread only; falls
/// back to `displays` elsewhere).
#[cfg(target_os = "macos")]
pub fn current() -> Vec<Screen> {
    use objc2::MainThreadMarker;
    use objc2_app_kit::NSScreen;
    let mut out = displays();
    let Some(mtm) = MainThreadMarker::new() else {
        return out;
    };
    let screens = NSScreen::screens(mtm);
    let Some(primary_h) = screens.firstObject().map(|s| s.frame().size.height) else {
        return out;
    };
    let rect = |r: objc2_foundation::NSRect| [r.origin.x, r.origin.y, r.size.width, r.size.height];
    for ns in screens.iter() {
        let frame = flip(rect(ns.frame()), primary_h);
        let scale = ns.backingScaleFactor();
        let visible = flip(rect(ns.visibleFrame()), primary_h);
        let safe_top = ns.safeAreaInsets().top;
        match out
            .iter_mut()
            .find(|s| (0..4).all(|i| (s.frame[i] - frame[i]).abs() < 1.0))
        {
            Some(s) => {
                s.visible = visible;
                s.safe_top = safe_top;
                s.scale = scale;
            }
            None => out.push(Screen {
                id: 0,
                frame,
                visible,
                scale,
                safe_top,
            }),
        }
    }
    out
}

/// The global pointer, from `CGEvent` (any thread).
#[cfg(target_os = "macos")]
pub fn pointer() -> Option<[f64; 2]> {
    crate::capture::current_mouse_screen().map(|[x, y]| [f64::from(x), f64::from(y)])
}

/// Every display, the primary first, in global pixels; `id` is its index (`TargetInfo::display_id`).
#[cfg(not(target_os = "macos"))]
pub fn displays() -> Vec<Screen> {
    #[cfg(target_os = "windows")]
    use crate::win as os;
    #[cfg(target_os = "linux")]
    use crate::linux as os;
    os::monitors()
        .into_iter()
        .enumerate()
        .map(|(i, m)| Screen {
            id: i as u32,
            frame: m.rect,
            visible: m.work,
            scale: m.scale,
            safe_top: 0.0,
        })
        .collect()
}

#[cfg(not(target_os = "macos"))]
pub fn current() -> Vec<Screen> {
    displays()
}

/// The global pointer, physical pixels.
#[cfg(target_os = "windows")]
pub fn pointer() -> Option<[f64; 2]> {
    crate::win::cursor_position().map(|(x, y)| [x, y])
}

/// The global pointer, X root-window pixels (on Wayland, where XWayland last saw it).
#[cfg(target_os = "linux")]
pub fn pointer() -> Option<[f64; 2]> {
    crate::linux::cursor_position().map(|(x, y)| [x, y])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 1728×1117 pt Retina laptop as the primary display.
    const LAPTOP: Rect = [0.0, 0.0, 1728.0, 1117.0];

    fn screen(id: u32, frame: Rect, scale: f64) -> Screen {
        Screen {
            id,
            frame,
            visible: frame,
            scale,
            safe_top: 0.0,
        }
    }

    /// The laptop plus one external display at `frame`.
    fn pair(frame: Rect, scale: f64) -> [Screen; 2] {
        [screen(1, LAPTOP, 2.0), screen(2, frame, scale)]
    }

    #[test]
    fn appkit_and_global_rects_flip_against_the_primary_height() {
        // A 1920×1080 display left of the laptop, bottoms aligned: AppKit y 0, global y 37.
        let left = [-1920.0, 0.0, 1920.0, 1080.0];
        assert_eq!(flip(left, 1117.0), [-1920.0, 37.0, 1920.0, 1080.0]);
        // One above the laptop: AppKit y above 1117, global y negative.
        let above = [0.0, 1117.0, 2560.0, 1440.0];
        assert_eq!(flip(above, 1117.0), [0.0, -1440.0, 2560.0, 1440.0]);
        assert_eq!(flip(flip(above, 1117.0), 1117.0), above);
        assert_eq!(flip(LAPTOP, 1117.0), LAPTOP);
        // One below it: AppKit y negative, global y past the laptop's bottom.
        let below = [0.0, -1080.0, 1920.0, 1080.0];
        assert_eq!(flip(below, 1117.0), [0.0, 1117.0, 1920.0, 1080.0]);
        assert_eq!(flip([0.0, 1117.0, 1920.0, 1080.0], 1117.0), below);
    }

    #[test]
    fn points_on_each_display_pick_that_display() {
        let cases = [
            // left, 1x, negative x
            ([-1920.0, 37.0, 1920.0, 1080.0], 1.0, [-10.0, 500.0]),
            // right, 1x
            ([1728.0, 0.0, 1920.0, 1080.0], 1.0, [1800.0, 40.0]),
            // above, 2x, negative y
            ([0.0, -1440.0, 2560.0, 1440.0], 2.0, [1000.0, -1.0]),
            // left, 2x
            ([-2560.0, -200.0, 2560.0, 1440.0], 2.0, [-2559.0, -199.0]),
            // below, 1x
            ([0.0, 1117.0, 1920.0, 1080.0], 1.0, [1800.0, 1117.0]),
        ];
        for (frame, scale, p) in cases {
            let screens = pair(frame, scale);
            assert_eq!(index_at(&screens, p), Some(1), "{frame:?} {p:?}");
            assert_eq!(index_at(&screens, [864.0, 558.0]), Some(0));
            // The edge belongs to the display that starts there.
            assert_eq!(index_at(&screens, [0.0, 0.0]), Some(0));
        }
        // In the gap below a short external display: the nearest one.
        let screens = pair([1728.0, 0.0, 1280.0, 720.0], 1.0);
        assert_eq!(index_at(&screens, [2000.0, 900.0]), Some(1));
        assert_eq!(index_at(&[], [0.0, 0.0]), None);
    }

    #[test]
    fn rects_belong_to_the_display_holding_their_centre_or_most_of_them() {
        let screens = pair([-1920.0, 37.0, 1920.0, 1080.0], 1.0);
        assert_eq!(
            index_for_rect(&screens, [-900.0, 100.0, 400.0, 300.0]),
            Some(1)
        );
        // A window straddling the seam, mostly on the laptop.
        assert_eq!(
            index_for_rect(&screens, [-100.0, 100.0, 800.0, 300.0]),
            Some(0)
        );
    }

    #[test]
    fn global_points_become_local_to_their_display() {
        let left = [-1920.0, 37.0, 1920.0, 1080.0];
        assert_eq!(to_local(left, [-1920.0, 37.0]), [0.0, 0.0]);
        assert_eq!(to_local(left, [-960.0, 577.0]), [960.0, 540.0]);
        let above = [0.0, -1440.0, 2560.0, 1440.0];
        assert_eq!(to_local(above, [1280.0, -720.0]), [1280.0, 720.0]);
        let right = [1728.0, 0.0, 1920.0, 1080.0];
        assert_eq!(to_local(right, [1828.0, 50.0]), [100.0, 50.0]);
        assert_eq!(to_local(LAPTOP, [864.0, 558.5]), [864.0, 558.5]);
    }

    #[test]
    fn area_source_rects_are_local_to_their_display() {
        let left = [-1920.0, 37.0, 1920.0, 1080.0];
        assert_eq!(
            source_rect([-1000.0, 100.0, 400.0, 300.0], left),
            [920.0, 63.0, 400.0, 300.0]
        );
        let above = [0.0, -1440.0, 2560.0, 1440.0];
        assert_eq!(
            source_rect([100.0, -400.0, 300.0, 200.0], above),
            [100.0, 1040.0, 300.0, 200.0]
        );
        // An area dragged past the display's edge keeps the part on it.
        assert_eq!(
            source_rect([-100.0, 100.0, 300.0, 200.0], left),
            [1820.0, 63.0, 100.0, 200.0]
        );
        // The primary display: global and local agree.
        assert_eq!(
            source_rect([10.0, 20.0, 300.0, 200.0], LAPTOP),
            [10.0, 20.0, 300.0, 200.0]
        );
    }

    /// Kit's review: a drag that starts on a left display and overshoots onto the laptop. The
    /// picker keeps the drag on the display it started on (`ui::area_rect`), so the area is the
    /// left display's part and is recorded there.
    #[test]
    fn an_area_dragged_across_the_seam_stays_on_its_display() {
        let left = [-1920.0, 37.0, 1920.0, 1080.0];
        let screens = pair(left, 1.0);
        // The picker pulled the overshoot (x 160) back to the seam (x 0).
        let (i, rect) = area_on_display(&screens, [-150.0, 200.0, 150.0, 200.0]).unwrap();
        assert_eq!((i, rect), (1, [-150.0, 200.0, 150.0, 200.0]));
        // Without that, the centre (x 5) is on the laptop, and only the laptop's part of the
        // area is recorded there: the frame never spans the seam.
        let (i, rect) = area_on_display(&screens, [-150.0, 200.0, 310.0, 200.0]).unwrap();
        assert_eq!((i, rect), (0, [0.0, 200.0, 160.0, 200.0]));
    }

    /// A sliver at a display's edge grows to `MIN_AREA` inward, so the target frame and the
    /// display-local `sourceRect` describe the same pixels.
    #[test]
    fn small_areas_grow_inside_their_display() {
        let left = [-1920.0, 37.0, 1920.0, 1080.0];
        let screens = pair(left, 1.0);
        let (i, rect) = area_on_display(&screens, [-20.0, 1100.0, 10.0, 10.0]).unwrap();
        assert_eq!((i, rect), (1, [-32.0, 1085.0, 32.0, 32.0]));
        let [x, y, w, h] = rect;
        assert_eq!(source_rect(rect, left), [x - left[0], y - left[1], w, h]);
        assert_eq!(source_rect(rect, left), [1888.0, 1048.0, 32.0, 32.0]);
        // Top-left corner of the laptop.
        let (_, rect) = area_on_display(&screens, [0.0, 0.0, 5.0, 40.0]).unwrap();
        assert_eq!(rect, [0.0, 0.0, 32.0, 40.0]);
        assert_eq!(source_rect(rect, LAPTOP), rect);
    }

    #[test]
    fn target_frames_carry_their_display_scale() {
        let f = with_scale([-1920.0, 37.0, 1920.0, 1080.0], 1.0);
        assert_eq!(
            (f.x, f.y, f.width, f.height, f.scale),
            (-1920.0, 37.0, 1920.0, 1080.0, 1.0)
        );
        assert_eq!(with_scale(LAPTOP, f64::NAN).scale, 2.0);
    }

    #[test]
    fn the_capsule_sits_below_each_displays_menu_bar_and_notch() {
        // The notched laptop: a 37 pt menu bar, a 32 pt notch.
        let laptop = Screen {
            visible: [0.0, 37.0, 1728.0, 1080.0],
            safe_top: 32.0,
            ..screen(1, LAPTOP, 2.0)
        };
        assert_eq!(pill_origin(&laptop, 640.0, 34.0), [544.0, 34.0]);
        // An external display left of it with a 24 pt menu bar: same 34 pt from its top.
        let left = Screen {
            visible: [-1920.0, 61.0, 1920.0, 1056.0],
            ..screen(2, [-1920.0, 37.0, 1920.0, 1080.0], 1.0)
        };
        assert_eq!(pill_origin(&left, 640.0, 34.0), [-1280.0, 71.0]);
        // Above, with no menu bar of its own.
        let above = screen(3, [0.0, -1440.0, 2560.0, 1440.0], 2.0);
        assert_eq!(pill_origin(&above, 640.0, 34.0), [960.0, -1406.0]);
        // A notched display with a taller menu bar pushes it down.
        let tall = Screen {
            visible: [0.0, 44.0, 1512.0, 938.0],
            safe_top: 40.0,
            ..screen(4, [0.0, 0.0, 1512.0, 982.0], 2.0)
        };
        assert_eq!(pill_origin(&tall, 640.0, 34.0)[1], 42.0);
    }

    #[test]
    fn the_windows_pill_window_lands_on_its_own_monitor_in_pixels() {
        // A 150% monitor left of the primary, taskbar at the bottom: centred, 14 pt down.
        let left = screen(1, [-2880.0, 0.0, 2880.0, 1620.0], 1.5);
        assert_eq!(
            pill_window_rect(&left, 400.0, 100.0, 14.0),
            [-1740.0, 21.0, 600.0, 150.0]
        );
        // The primary at 100% with the taskbar on top (48 px): below it.
        let top_bar = Screen {
            visible: [0.0, 48.0, 1920.0, 1032.0],
            ..screen(0, [0.0, 0.0, 1920.0, 1080.0], 1.0)
        };
        assert_eq!(
            pill_window_rect(&top_bar, 400.0, 100.0, 14.0),
            [760.0, 45.0, 400.0, 100.0]
        );
    }
}
