
    use super::*;

    #[cfg(target_os = "macos")]
    #[test]
    fn notes_hit_testing_limits_the_dock_to_its_visible_strip() {
        let (sender, _receiver) = flume::unbounded();
        let telemetry = SharedCaptureTelemetry::new(sender);
        telemetry.note_ui_pid.store(700, Ordering::Release);
        *telemetry.dock_bounds.lock().unwrap() = Some([0.0, 816.0, 1440.0, 84.0]);
        let windows = [
            crate::shared::windows::RawWindow { id: 1, pid: 600, owner: "Dock".into(), title: String::new(), bounds: [0.0, 0.0, 1440.0, 900.0], layer: 20, alpha: 1.0 },
            crate::shared::windows::RawWindow { id: 2, pid: 700, owner: "Apparent Notes".into(), title: String::new(), bounds: [100.0, 100.0, 332.0, 220.0], layer: 3, alpha: 1.0 },
        ];
        assert!(telemetry.notes_window_in(&windows, 200.0, 200.0));
        assert!(!telemetry.notes_window_in(&windows, 200.0, 850.0));
    }

    /// Clicks on `apparent-helper`'s windows (the SwiftUI capsule, Settings, onboarding) are
    /// Apparent's own, except on the iPhone mirror, which is what's being recorded.
    #[test]
    fn inverted_mixed_scale_outputs_never_bridge_x_space_jump() {
        let frame = TargetFrame { x: 0.0, y: 0.0, width: 1280.0, height: 800.0, scale: 1.0 };
        // Hyprland left output -1280..0 maps to X 1280..2560; the right
        // output 0..1024 at scale 1.25 maps to X 0..1280.
        let stroke = Stroke { t0: 10, t1: 50, fw: 1280, fh: 800, on: None, pin: None, laser: false,
            rgb: None,
            pts: [ [2500.0,420.0], [2550.0,420.0], [10.0,525.0], [50.0,525.0] ].into_iter().enumerate().flat_map(|(i,p)|
                [(p[0] / 1280.0 * SCALE as f64) as i32, (p[1] / 800.0 * SCALE as f64) as i32, i as i32 * 10]).collect() };
        let parts = ink_output_parts(&stroke, frame, &[[0.0,0.0,1280.0,800.0],[1280.0,0.0,1280.0,800.0]]);
        assert_eq!(parts.len(), 2);
        let saved: Vec<_> = parts.iter().flat_map(clip_recording_ink).collect();
        assert_eq!(saved.len(), 1);
        assert!(saved[0].normalized_points().iter().all(|p| p[0] < 0.05), "no phantom full-width crossing");
    }

    #[test]
    fn helper_windows_are_apparent_s_except_the_capture_source() {
        let (sender, _receiver) = flume::unbounded();
        let telemetry = SharedCaptureTelemetry::new(sender);
        let window = |id, pid, bounds| crate::shared::windows::RawWindow {
            id,
            pid,
            owner: String::new(),
            title: String::new(),
            bounds,
            layer: 0,
            alpha: 1.0,
        };
        let own = std::process::id() as i32;
        let (settings, mirror, app) = (own + 1, own + 2, own + 3);
        let windows = [
            window(1, own, [0.0, 0.0, 1728.0, 1117.0]), // the ink overlay, click-through
            window(2, settings, [100.0, 100.0, 600.0, 400.0]),
            window(3, mirror, [1300.0, 100.0, 300.0, 650.0]),
            window(4, app, [0.0, 0.0, 1728.0, 1117.0]),
        ];
        telemetry.passthrough_window_ids.lock().unwrap().push(1);
        telemetry.set_capture_source_pid(mirror as u32);
        let helper = |pid| pid == settings || pid == mirror;
        let at = |x, y| telemetry.apparent_window_in(&windows, x, y, helper);
        assert!(at(300.0, 300.0), "Settings");
        assert!(!at(1400.0, 400.0), "the iPhone mirror");
        assert!(!at(1000.0, 900.0), "the app under the ink overlay");
        telemetry.note_ui_pid.store(app as u64, Ordering::Release);
        assert!(at(1000.0, 900.0), "the separate Electron note UI is owned");
        assert!(telemetry.notes_window_in(&windows, 1000.0, 900.0), "notes clicks stay inside");
        let notes_and_capsule = [window(10, own, [800.0, 0.0, 500.0, 100.0]), window(11, app, [1000.0, 30.0, 150.0, 44.0])];
        assert!(!telemetry.notes_window_in(&notes_and_capsule, 850.0, 50.0), "capsule outside notes dismisses");
        #[cfg(target_os = "linux")]
        assert!(telemetry.notes_window_in(&notes_and_capsule, 1050.0, 50.0), "pinned notes remain inside despite stale X11 stacking");
        telemetry.note_ui_pid.store(0, Ordering::Release);
        assert!(!at(1000.0, 900.0), "closing notes releases that process identity");
        telemetry.set_capture_source_pid(0);
        assert!(at(1400.0, 400.0), "a helper that isn't the source");
    }

    #[test]
    fn separate_settings_window_is_owned_only_while_its_exact_child_is_alive() {
        use crate::capture::ui_process::UiChild;
        let (sender, _receiver) = flume::unbounded();
        let telemetry = SharedCaptureTelemetry::new(sender);
        let spawn = || std::process::Command::new("sh").args(["-c", "exec sleep 30"]).spawn().unwrap();
        let raw = spawn();
        let pid = raw.id() as i32;
        let windows = [crate::shared::windows::RawWindow {
            id: 42, pid, owner: "Electron".into(), title: "Apparent Settings".into(),
            bounds: [0.0, 0.0, 600.0, 400.0], layer: 0, alpha: 1.0,
        }];
        let at = || telemetry.apparent_window_in(&windows, 100.0, 100.0, |_| false);
        assert!(!at(), "an unregistered Electron process is not ours");
        let mut child = UiChild::new(raw);
        assert!(at(), "the separate Settings window excludes presses and scrolls");
        telemetry.set_capture_source_pid(pid as u32);
        assert!(!at(), "explicit capture sources retain their interaction telemetry");
        telemetry.set_capture_source_pid(0);
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(!at(), "a reaped PID is not owned even before its registration is dropped");
        let next = spawn();
        let next_pid = next.id() as i32;
        let mut next = UiChild::new(next);
        drop(child);
        assert!(crate::capture::ui_process::owns(next_pid), "old cleanup cannot retire a new session");
        next.kill().unwrap();
        next.wait().unwrap();
        drop(next);
        assert!(!crate::capture::ui_process::owns(next_pid));
    }

    /// Kit's Screen capture on the Pro: the old listen-only event tap was off (no Input
    /// Monitoring), so his clicks on OpenCode's composer never reached the bundle. Clicks now
    /// come from the mouse monitor; polled button state stands in if it couldn't be installed.
    #[test]
    fn note_editor_dismisses_only_on_an_actual_external_button_press() {
        let (sender, receiver) = flume::unbounded();
        let telemetry = SharedCaptureTelemetry::new(sender);
        telemetry.notes_editing.store(true, Ordering::Release);
        // A live global monitor does not receive clicks on the native capsule.
        telemetry.mouse_live.store(true, Ordering::Release);
        let mut prev = [false; 2];
        // Typing and clicks inside the editor never become captured input.
        poll_clicks(&telemetry, &mut prev, [true, false], (20.0, 20.0), false, |_, _| true);
        poll_clicks(&telemetry, &mut prev, [false, false], (20.0, 20.0), false, |_, _| true);
        assert!(receiver.try_recv().is_err());
        // An external Shift-click dismisses once on press; its release adds no pin.
        poll_clicks(&telemetry, &mut prev, [true, false], (500.0, 500.0), true, |_, _| false);
        assert!(matches!(receiver.try_recv(), Ok(LiveOverlayEvent::NoteOutside)));
        poll_clicks(&telemetry, &mut prev, [true, false], (500.0, 500.0), true, |_, _| false);
        poll_clicks(&telemetry, &mut prev, [false, false], (500.0, 500.0), true, |_, _| false);
        assert!(receiver.try_recv().is_err());
        assert!(telemetry.events.lock().unwrap().is_empty());
    }

    #[test]
    fn clicks_are_recorded_from_polled_buttons_while_the_mouse_monitor_is_missing() {
        let (sender, _receiver) = flume::unbounded();
        let telemetry = SharedCaptureTelemetry::new(sender);
        telemetry.reset_for_session(TargetFrame {
            x: 0.0,
            y: 0.0,
            width: 1728.0,
            height: 1117.0,
            scale: 2.0,
        });
        assert!(!telemetry.mouse_live.load(Ordering::Acquire));
        let mut prev = [false; 2];
        let not_apparent = |_: f64, _: f64| false;
        let composer = (778.0, 1050.0);
        // A ⇧-click is a pin, not a click.
        poll_clicks(
            &telemetry,
            &mut prev,
            [true, false],
            composer,
            true,
            not_apparent,
        );
        poll_clicks(
            &telemetry,
            &mut prev,
            [false, false],
            composer,
            true,
            not_apparent,
        );
        // A click on the composer.
        poll_clicks(
            &telemetry,
            &mut prev,
            [true, false],
            composer,
            false,
            not_apparent,
        );
        poll_clicks(
            &telemetry,
            &mut prev,
            [true, false],
            composer,
            false,
            not_apparent,
        );
        poll_clicks(
            &telemetry,
            &mut prev,
            [false, false],
            composer,
            false,
            not_apparent,
        );
        // Then the stop button, which is Apparent's own.
        let stop = (860.0, 20.0);
        poll_clicks(&telemetry, &mut prev, [true, false], stop, false, |_, _| {
            true
        });
        poll_clicks(
            &telemetry,
            &mut prev,
            [false, false],
            stop,
            false,
            |_, _| true,
        );
        // While the mouse monitor is installed it reports clicks itself.
        telemetry.mouse_live.store(true, Ordering::Release);
        poll_clicks(
            &telemetry,
            &mut prev,
            [true, false],
            composer,
            false,
            not_apparent,
        );
        poll_clicks(
            &telemetry,
            &mut prev,
            [false, false],
            composer,
            false,
            not_apparent,
        );

        let (events, _) = telemetry.finish_session();
        assert_eq!(events.len(), 1, "{events:?}");
        let InteractionEvent::Click { x, y, button, .. } = &events[0] else {
            panic!("{events:?}");
        };
        assert!(
            (x - 0.45).abs() < 0.01 && (y - 0.94).abs() < 0.01,
            "{x}, {y}"
        );
        assert_eq!(button.as_deref(), Some("left"));
    }

    #[test]
    fn the_capture_shortcut_matches_its_aliases_and_a_custom_one() {
        let s = 1_u16; // the S key on this layout
        let default = ShortcutConfig {
            key_code: u32::from(s),
            carbon_modifiers: CARBON_OPTION_KEY | CARBON_SHIFT_KEY,
            label: "⌥⇧S".into(),
        };
        // (cmd, ctrl, alt, shift)
        let hit = |cfg: &ShortcutConfig, key, (cmd, ctrl, alt, shift)| {
            is_capture_shortcut(cfg, s, key, cmd, ctrl, alt, shift)
        };
        assert!(hit(&default, s, (false, false, true, true)), "⌥⇧S");
        assert!(hit(&default, s, (true, false, true, true)), "⌥⇧⌘S");
        assert!(hit(&default, s, (false, true, true, true)), "⌃⌥⇧S");
        assert!(hit(&default, s, (true, true, true, false)), "⌃⌥⌘S");
        assert!(hit(&default, s, (true, true, true, true)), "hyper S");
        assert!(hit(&default, KEY_S, (false, false, true, true)), "ANSI S");
        assert!(!hit(&default, s, (false, false, false, true)), "⇧S");
        assert!(!hit(&default, s, (true, false, false, false)), "⌘S");
        assert!(!hit(&default, 2, (false, false, true, true)), "⌥⇧D");

        let custom = ShortcutConfig {
            key_code: 15,
            carbon_modifiers: CARBON_CONTROL_KEY | CARBON_OPTION_KEY,
            label: "⌃⌥R".into(),
        };
        assert!(hit(&custom, 15, (false, true, true, false)));
        assert!(!hit(&custom, 15, (true, true, true, false)));
        assert!(!hit(&custom, 15, (false, false, true, false)));
        // Once changed, ⌥⇧S and its aliases are free for other apps.
        assert!(!hit(&custom, s, (false, false, true, true)), "⌥⇧S");
        assert!(!hit(&custom, KEY_S, (true, true, true, true)), "hyper S");
        assert_eq!(
            hotkey_combos(ShortcutKind::Record, &custom),
            [(15, custom.carbon_modifiers)]
        );
        assert_eq!(
            hotkey_combos(ShortcutKind::Record, &default).len(),
            if u32::from(KEY_S) == default.key_code { 4 } else { 8 },
            "⌥⇧S and three aliases, on the layout's key and the ANSI one"
        );

        // The paste shortcut works the same way.
        let v = 9_u16;
        let paste = ShortcutConfig {
            key_code: u32::from(v),
            carbon_modifiers: CARBON_OPTION_KEY | CARBON_SHIFT_KEY,
            label: "⌥⇧V".into(),
        };
        let paste_hit = |cfg: &ShortcutConfig, key, (cmd, ctrl, alt, shift)| {
            shortcut_matches(ShortcutKind::Paste, cfg, v, key, cmd, ctrl, alt, shift)
        };
        assert!(paste_hit(&paste, v, (false, false, true, true)), "⌥⇧V");
        assert!(paste_hit(&paste, v, (true, true, true, false)), "⌃⌥⌘V");
        assert!(
            !paste_hit(&paste, s, (false, false, true, true)),
            "⌥⇧S isn't paste"
        );
        let paste_custom = ShortcutConfig {
            key_code: 35,
            carbon_modifiers: CARBON_CONTROL_KEY | CARBON_OPTION_KEY,
            label: "⌃⌥P".into(),
        };
        assert!(paste_hit(&paste_custom, 35, (false, true, true, false)));
        assert!(!paste_hit(&paste_custom, v, (false, false, true, true)));
        assert!(
            paste.is_default_for(ShortcutKind::Paste)
                && !paste.is_default_for(ShortcutKind::Record)
        );
        #[cfg(target_os = "macos")]
        assert_eq!(
            modifier_symbols(CARBON_CMD_KEY | CARBON_CONTROL_KEY | CARBON_SHIFT_KEY),
            "⌃⇧⌘"
        );
        #[cfg(target_os = "windows")]
        assert_eq!(
            modifier_symbols(CARBON_CMD_KEY | CARBON_CONTROL_KEY | CARBON_SHIFT_KEY),
            "Ctrl+Win+Shift+"
        );
        #[cfg(target_os = "linux")]
        assert_eq!(
            modifier_symbols(CARBON_CMD_KEY | CARBON_CONTROL_KEY | CARBON_SHIFT_KEY),
            "Ctrl+Super+Shift+"
        );

        assert_eq!(shortcut_parts("⌥⇧S"), ("⌥⇧", "S"));
        assert_eq!(shortcut_parts("⌃⌥⌘R"), ("⌃⌥⌘", "R"));
        assert_eq!(shortcut_parts("⌥⇧É"), ("⌥⇧", "É"));
        assert_eq!(shortcut_parts("Alt+Shift+S"), ("Alt+Shift", "S"));
        assert_eq!(shortcut_parts("Ctrl+Alt+Ä"), ("Ctrl+Alt", "Ä"));
        assert_eq!(shortcut_parts("Ctrl++"), ("Ctrl", "+"));
        assert_eq!(shortcut_parts(""), ("", ""));
    }

    #[test]
    fn recordings_use_the_display_under_the_cursor() {
        let displays = [[0.0, 0.0, 1728.0, 1117.0], [1728.0, -200.0, 2560.0, 1440.0]];
        assert_eq!(pick_display(&displays, Some([2000.0, 300.0]), None), 1);
        assert_eq!(pick_display(&displays, Some([100.0, 100.0]), None), 0);
        // No cursor: the display holding the focused window's centre.
        let focused = Some([1900.0, 0.0, 800.0, 600.0]);
        assert_eq!(pick_display(&displays, None, focused), 1);
        assert_eq!(pick_display(&displays, Some([-50.0, 0.0]), focused), 1);
        assert_eq!(pick_display(&displays, None, None), 0);
        // A display left of the laptop, at negative x.
        let left = [[0.0, 0.0, 1728.0, 1117.0], [-1920.0, 37.0, 1920.0, 1080.0]];
        assert_eq!(pick_display(&left, Some([-5.0, 600.0]), None), 1);
        // One above it, at negative y.
        let above = [[0.0, 0.0, 1728.0, 1117.0], [0.0, -1440.0, 2560.0, 1440.0]];
        assert_eq!(pick_display(&above, Some([10.0, -1.0]), None), 1);
        assert_eq!(pick_display(&above, Some([10.0, 0.0]), None), 0);
    }

    /// Clicks and ink on a display left of, right of or above the laptop normalize into that
    /// display, and land on the recording's pixels at its own scale.
    #[test]
    fn input_on_an_external_display_normalizes_into_it() {
        let (sender, _receiver) = flume::unbounded();
        let telemetry = SharedCaptureTelemetry::new(sender);
        let left = [-1920.0, 37.0, 1920.0, 1080.0];
        let right = [1728.0, 0.0, 1920.0, 1080.0];
        let above = [0.0, -1440.0, 2560.0, 1440.0];
        let small_left = [-1280.0, -100.0, 1280.0, 800.0];
        let below = [0.0, 1117.0, 1920.0, 1080.0];
        // (display, scale, global point, expected normalized point, recording size)
        let cases = [
            // 1x displays are recorded at their own pixels
            (left, 1.0, [-960.0, 577.0], (0.5, 0.5), (1920, 1080)),
            (right, 1.0, [2208.0, 270.0], (0.25, 0.25), (1920, 1080)),
            (below, 1.0, [480.0, 1387.0], (0.25, 0.25), (1920, 1080)),
            // 2x ones at the 2560 px long edge
            (above, 2.0, [640.0, -360.0], (0.25, 0.75), (2560, 1440)),
            (small_left, 2.0, [-320.0, 500.0], (0.75, 0.75), (2560, 1600)),
        ];
        for (rect, scale, point, expected, size) in cases {
            let frame = crate::capture::engine::runtime::screens::with_scale(rect, scale);
            let target = TargetInfo {
                mode: CaptureMode::Display,
                app: None,
                bundle_id: None,
                window_title: None,
                window_id: None,
                display_id: 2,
                frame,
            };
            // Windows frames are pixels already and never upscaled.
            if cfg!(target_os = "macos") {
                assert_eq!(capture_size(&target, 2560.0), size, "{rect:?}");
            }
            telemetry.reset_for_session(frame);
            assert!(telemetry.contains_screen_point(point[0], point[1]));
            // The laptop's centre is outside this target.
            assert!(!telemetry.contains_screen_point(864.0, 558.0));
            let (nx, ny, fw, fh) = telemetry.normalize_point(point[0], point[1]);
            assert_eq!((nx, ny), expected, "{rect:?}");
            assert_eq!((fw, fh), (rect[2] as u32, rect[3] as u32));
            // A click there is recorded at those coordinates.
            telemetry.pointer_down(10, point[0], point[1]);
            telemetry.pointer_up(20, point[0], point[1], false);
            let (events, _) = telemetry.finish_session();
            assert!(
                matches!(events[..], [InteractionEvent::Click { x, y, .. }] if (x, y) == (nx, ny)),
                "{events:?}"
            );
        }
        // In recording pixels: the left 1x display's centre is (960, 540).
        let frame = crate::capture::engine::runtime::screens::with_scale(
            [-1920.0, 37.0, 1920.0, 1080.0],
            1.0,
        );
        telemetry.reset_for_session(frame);
        let (nx, ny, _, _) = telemetry.normalize_point(-960.0, 577.0);
        assert_eq!((nx * 1920.0, ny * 1080.0), (960.0, 540.0));
        // Above 2x: (640, -360) is a quarter in → (640, 1080) px of 2560×1440.
        let frame = crate::capture::engine::runtime::screens::with_scale(
            [0.0, -1440.0, 2560.0, 1440.0],
            2.0,
        );
        telemetry.reset_for_session(frame);
        let (nx, ny, _, _) = telemetry.normalize_point(640.0, -360.0);
        assert_eq!((nx * 2560.0, ny * 1440.0), (640.0, 1080.0));
        telemetry.finish_session();
    }

    fn session(width: f64, height: f64) -> Arc<SharedCaptureTelemetry> {
        let (sender, _receiver) = flume::unbounded();
        let telemetry = SharedCaptureTelemetry::new(sender);
        telemetry.reset_for_session(TargetFrame {
            x: 0.0,
            y: 0.0,
            width,
            height,
            scale: 2.0,
        });
        telemetry
    }


    #[test]
    fn a_long_still_shift_press_drops_no_pin() {
        let down = (1_000, 10.0, 10.0);
        let slop = f64::from(CLICK_SLOP_PT);
        assert!(is_pin_click(down, (1_150, 14.0, 13.0)));
        assert!(is_pin_click(down, (1_000 + PIN_MAX_PRESS_MS, 10.0, 10.0)));
        assert!(!is_pin_click(down, (2_500, 10.0, 10.0)));
        assert!(!is_pin_click(down, (1_100, 10.0, 10.0 + slop)));
    }

    /// The poller draws ink while ⇧ is held. A ⇧-click after ink was drawn in the same hold is
    /// part of the drawing: no pin, and the stroke carries on whole. A ⇧-click with nothing drawn
    /// first (only its own jitter) drops a pin and leaves no stroke.
    #[test]
    fn a_shift_click_mid_drawing_drops_no_pin() {
        let telemetry = session(1000.0, 1000.0);
        let ink = |pts: &[(i32, i32, i32)]| Stroke {
            t0: 0,
            t1: 0,
            fw: 1000,
            fh: 1000,
            on: None,
            pin: None,
            laser: false,
            rgb: None,
            pts: pts.iter().flat_map(|&(x, y, t)| [x, y, t]).collect(),
        };
        let press = telemetry.elapsed_ms() + 50;
        let p = press as i32;
        let s = SCALE;
        // A line drawn while ⇧ was held, then a click at its end.
        let line = ink(&[
            (s / 10, s / 2, 0),
            (s / 5, s / 2, p - 20),
            (s / 5 + s / 500, s / 2, p + 5),
        ]);
        *telemetry.live_ink.lock().unwrap() = Some(line.clone());
        telemetry.shift_down(press, 200.0, 500.0);
        assert!(telemetry.shift_up(press + 80, 201.0, 500.0));
        assert_eq!(telemetry.live_ink.lock().unwrap().as_ref(), Some(&line));
        // Letting go of ⇧ commits it as one stroke.
        let at = (200.0, 500.0);
        track_shift_ink(&telemetry, at, false, true);

        // Drawn long before the press still counts: the hold is a drawing.
        *telemetry.live_ink.lock().unwrap() = Some(ink(&[(s / 10, s / 10, 0), (s / 5, s / 10, 1)]));
        let t = telemetry.elapsed_ms() + 5_000;
        telemetry.shift_down(t, 200.0, 100.0);
        assert!(telemetry.shift_up(t + 60, 200.0, 100.0));
        *telemetry.live_ink.lock().unwrap() = None;

        // A ⇧-click with only its own few points of jitter is a pin.
        let jitter = ink(&[(s / 2, s / 2, 0), (s / 2 + 2, s / 2, 30)]);
        *telemetry.live_ink.lock().unwrap() = Some(jitter);
        let t = telemetry.elapsed_ms();
        telemetry.shift_down(t, 500.0, 500.0);
        assert!(telemetry.shift_up(t + 60, 500.0, 500.0));
        assert!(telemetry.live_ink.lock().unwrap().is_none());

        let (events, strokes) = telemetry.finish_session();
        assert!(events.is_empty(), "{events:?}");
        let kinds: Vec<_> = strokes.iter().map(|s| (s.pin, s.pts.len() / 3)).collect();
        assert_eq!(kinds, vec![(None, 3), (Some(1), 1)]);
    }

    #[test]
    fn shift_clicks_drop_pins_from_polled_buttons_too() {
        let telemetry = session(1000.0, 1000.0);
        let mut prev = [false; 2];
        // ⇧-clicks at a point, the second on Apparent's own window.
        for (at, on_apparent) in [((250.0, 750.0), false), ((400.0, 400.0), true)] {
            for buttons in [[true, false], [false, false]] {
                poll_clicks(&telemetry, &mut prev, buttons, at, true, |_, _| on_apparent);
            }
        }
        let (events, strokes) = telemetry.finish_session();
        assert!(events.is_empty(), "{events:?}");
        assert_eq!(strokes.len(), 1);
        assert_eq!(strokes[0].pin, Some(1));
        assert_eq!(strokes[0].normalized_points(), vec![[0.25, 0.75]]);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn settings_notes_preserve_control_editor_and_ordinary_click_exclusions() {
        use crate::shared::windows::RawWindow;
        let telemetry = session(1000.0, 1000.0);
        let pid = std::process::id() as i32;
        let window = |id, pid, bounds| RawWindow { id, pid, bounds, owner: "synthetic".into(), title: String::new(), layer: 0, alpha: 1.0 };
        let settings = window(0xfeed0200, pid, [0.0, 100.0, 800.0, 600.0]);
        let pill = window(0xfeed0201, pid, [300.0, 10.0, 300.0, 60.0]);
        let windows = [pill.clone(), settings.clone()];
        let registered = crate::capture::ui_process::SettingsWindow::new(settings.id);
        assert!(telemetry.apparent_window_in(&windows, 400.0, 400.0, |_| false));
        assert!(!telemetry.note_input_ui_in(&windows, 400.0, 400.0));
        assert!(telemetry.note_input_ui_in(&windows, 400.0, 30.0), "pill is never a note subject");
        let mut previous = [false; 2];
        for shift in [false, true] {
            for buttons in [[true, false], [false, false]] {
                poll_clicks(&telemetry, &mut previous, buttons, (400.0, 400.0), shift, |x, y| {
                    if shift { telemetry.note_input_ui_in(&windows, x, y) }
                    else { telemetry.apparent_window_in(&windows, x, y, |_| false) }
                });
            }
        }
        telemetry.note_ui_pid.store(700, Ordering::Release);
        let editor = window(0xfeed0202, 700, [350.0, 350.0, 300.0, 200.0]);
        assert!(telemetry.note_input_ui_in(&[settings.clone(), editor.clone()], 400.0, 400.0), "pinned editor stays protected even with stale stacking");
        let covering_control = window(0xfeed0203, pid, settings.bounds);
        assert!(telemetry.note_input_ui_in(&[covering_control, settings.clone()], 400.0, 400.0), "another own surface cannot inherit the Settings role");
        telemetry.notes_editing.store(true, Ordering::Release);
        assert!(telemetry.note_input_ui_in(&[editor], 400.0, 400.0));
        telemetry.notes_editing.store(false, Ordering::Release);
        drop(registered);
        assert!(telemetry.note_input_ui_in(&windows, 400.0, 400.0));
        let (events, strokes) = telemetry.finish_session();
        assert!(events.is_empty());
        assert_eq!(strokes.len(), 1);
        assert_eq!(strokes[0].pin, Some(1));
    }


    #[test]
    fn candidates_are_small_and_recordings_keep_full_size() {
        let target = |mode, width, height| TargetInfo {
            mode,
            app: None,
            bundle_id: None,
            window_title: None,
            window_id: None,
            display_id: 1,
            frame: TargetFrame {
                x: 0.0,
                y: 0.0,
                width,
                height,
                scale: 2.0,
            },
        };
        let display = target(CaptureMode::Display, 1728.0, 1117.0);
        assert_eq!(capture_size(&display, CANDIDATE_EDGE), (480, 310));
        let region = target(CaptureMode::Region, 480.0, 320.0);
        assert_eq!(capture_size(&region, CANDIDATE_EDGE), (480, 320));
        if cfg!(not(target_os = "macos")) {
            // Frames are in pixels: never upscaled past the screen's own.
            assert_eq!(capture_size(&display, 2560.0), (1728, 1116));
            assert_eq!(capture_size(&region, 1280.0), (480, 320));
            let wide = target(CaptureMode::Display, 3440.0, 1440.0);
            assert_eq!(capture_size(&wide, 2560.0), (2560, 1072));
        } else {
            assert_eq!(capture_size(&display, 2560.0), (2560, 1654));
            assert_eq!(capture_size(&region, 1280.0), (960, 640));
        }

        // A 1x external display is recorded at its own pixels, never upscaled.
        let external = |mode, width, height| TargetInfo {
            frame: crate::capture::engine::runtime::screens::with_scale(
                [-1920.0, 37.0, width, height],
                1.0,
            ),
            ..target(mode, width, height)
        };
        let screen = external(CaptureMode::Display, 1920.0, 1080.0);
        assert_eq!(capture_size(&screen, 2560.0), (1920, 1080));
        assert_eq!(capture_size(&screen, CANDIDATE_EDGE), (480, 270));
        let region = external(CaptureMode::Region, 480.0, 320.0);
        assert_eq!(capture_size(&region, 1280.0), (480, 320));
        // A 2x one above the laptop keeps the 2560 px long edge.
        let above = TargetInfo {
            frame: crate::capture::engine::runtime::screens::with_scale(
                [0.0, -1440.0, 2560.0, 1440.0],
                2.0,
            ),
            ..target(CaptureMode::Display, 2560.0, 1440.0)
        };
        assert_eq!(capture_size(&above, 2560.0), (2560, 1440));
    }

    #[test]
    fn cached_window_hit_preserves_front_to_back_order_and_screen_coordinates() {
        let window = |id, x, y| TargetInfo {
            mode: CaptureMode::Window,
            app: Some(format!("App {id}")),
            bundle_id: None,
            window_title: Some(format!("Window {id}")),
            window_id: Some(id),
            display_id: 1,
            frame: TargetFrame {
                x,
                y,
                width: 200.0,
                height: 100.0,
                scale: 2.0,
            },
        };
        let windows = [window(1, -100.0, 20.0), window(2, 0.0, 0.0)];
        assert_eq!(
            selectable_window_at(&windows, 50.0, 50.0)
                .unwrap()
                .window_id,
            Some(1)
        );
        assert_eq!(
            selectable_window_at(&windows, 150.0, 50.0)
                .unwrap()
                .window_id,
            Some(2)
        );
        assert_eq!(
            selectable_window_at(&windows, -100.0, 20.0)
                .unwrap()
                .window_id,
            Some(1)
        );
        assert!(selectable_window_at(&windows, -101.0, 50.0).is_none());
        assert!(selectable_window_at(&[], 0.0, 0.0).is_none());
    }
