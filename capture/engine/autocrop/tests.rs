
    use super::*;
    use crate::shared::model::{CaptureMode, KeyframeReason, TargetInfo};
    use crate::shared::windows::WindowEntry;

    const FRAME: TargetFrame = TargetFrame {
        x: 0.0,
        y: 0.0,
        width: 1728.0,
        height: 1117.0,
        scale: 2.0,
    };

    fn solid(w: u32, h: u32, v: u8) -> RgbaFrame {
        RgbaFrame::new(0, w, h, [v, v, v, 255].repeat((w * h) as usize))
    }

    fn keyframe(index: usize, t: u64) -> Keyframe {
        Keyframe {
            index,
            t,
            reason: KeyframeReason::Click,
            file: format!("frames/frame-{index:02}.png"),
            event: String::new(),
            said: None,
            mark_id: None,
            cursor: Some([0.5, 0.5]),
        }
    }

    #[test]
    fn grouped_ink_belongs_to_its_keyframe_alone() {
        let stroke = |t0: u64, x: i32| Stroke {
            t0,
            t1: t0 + 100,
            fw: 1000,
            fh: 800,
            on: None,
            pin: None,
            laser: false,
            rgb: None,
            pts: vec![x, 5000, 0, x + 500, 5000, 100],
        };
        let strokes = [stroke(1_000, 1000), stroke(9_000, 8000)];
        let mark = |id: &str, stroke: usize| Mark {
            id: id.into(),
            strokes: vec![stroke],
            t0: strokes[stroke].t0,
            t1: strokes[stroke].t1,
            box_norm: [0.0, 0.0, 1.0, 1.0],
            shape: crate::shared::model::Shape::Line,
            from: None,
            to: None,
            pin: None,
            on: None,
            description: String::new(),
            frame: Some("frames/frame-02.png".into()),
        };
        let keyframes = [keyframe(1, 500), keyframe(2, 9_100)];
        let mut input = Input::new(FRAME, 10_000, &[500, 9_100], &strokes, &[], &[], &[], &[]);
        let before = input.items.len();
        input.pin_ink(&keyframes, &strokes, &[mark("m1", 0), mark("m2", 1)]);
        assert_eq!(input.items.len(), before);
        let ink: Vec<_> = input
            .items
            .iter()
            .filter(|item| item.kind == Kind::Stroke)
            .map(|item| item.keyframe)
            .collect();
        assert_eq!(ink, [Some(1), Some(1)]);
    }

    fn window(id: u32, app: &str, bounds: [i32; 4], frontmost: bool) -> WindowEntry {
        WindowEntry {
            id,
            app: app.into(),
            bundle_id: None,
            regular: true,
            title: "Secret title".into(),
            bounds,
            layer: 0,
            frontmost,
        }
    }

    fn click(t: u64, x: f32, y: f32) -> InteractionEvent {
        InteractionEvent::Click {
            t,
            x,
            y,
            button: None,
            label: None,
            role: None,
        }
    }

    #[test]
    fn resting_cursor_cannot_hide_the_frontmost_window() {
        let frame = TargetFrame {
            width: 1440.0,
            height: 900.0,
            ..FRAME
        };
        let windows = [
            WindowSnapshot {
                t: 0,
                windows: vec![window(1, "Finder", [0, 30, 700, 600], true)],
            },
            WindowSnapshot {
                t: 173_535,
                windows: vec![window(104, "TextEdit", [141, 64, 603, 505], true)],
            },
        ];
        let cursor = [
            CursorSample(0, 0.95, 0.05),
            CursorSample(172_000, 0.95, 0.05),
        ];
        let times = [172_995, 173_926];
        for scope in [Scope::Keyframe, Scope::Grouped, Scope::Whole] {
            let settings = Settings {
                scope,
                ..Settings::default()
            };
            let input = Input::new(frame, 197_481, &times, &[], &[], &windows, &cursor, &[]);
            let crops = compute(&input, &settings);
            for (crop, bounds) in crops
                .iter()
                .zip([[0.0, 30.0, 700.0, 630.0], [141.0, 64.0, 744.0, 569.0]])
            {
                let win = [
                    bounds[0] / 1440.0,
                    bounds[1] / 900.0,
                    bounds[2] / 1440.0,
                    bounds[3] / 900.0,
                ];
                assert!(contains(crop.rect, win), "{scope:?}: {crop:?}");
            }
        }

        // Exercise the real image path, with target pixels far from the idle pointer.
        let mut raw = solid(1440, 900, 30);
        for y in 100..500 {
            for x in 180..700 {
                let i = (y * 1440 + x) * 4;
                raw.rgba[i..i + 3].copy_from_slice(&[10, 220, 80]);
            }
        }
        let target = TargetInfo {
            mode: CaptureMode::Display,
            app: None,
            bundle_id: None,
            window_title: None,
            window_id: None,
            display_id: 1,
            frame,
        };
        let selected = vec![
            (keyframe(1, times[0]), raw.clone()),
            (keyframe(2, times[1]), raw),
        ];
        let (frames, _) = annotate_and_crop(
            selected,
            None,
            Some(&Settings::default()),
            &target,
            197_481,
            &[],
            &[],
            &[],
            &windows,
            &cursor,
        );
        let has_target = |f: &RgbaFrame| f.rgba.chunks_exact(4).any(|p| p[..3] == [10, 220, 80]);
        assert!(
            frames.iter().all(|(_, f)| has_target(f)),
            "agent frames lost target pixels"
        );
        let sheet = crate::capture::engine::render::render_contact_sheet_for_duration(197_481, &frames);
        assert!(has_target(&sheet), "contact sheet lost target pixels");
    }

    #[test]
    fn resting_cursor_keeps_edge_content_with_or_without_window_metadata() {
        let frame = TargetFrame {
            width: 1280.0,
            height: 800.0,
            scale: 1.0,
            ..FRAME
        };
        let target = TargetInfo {
            mode: CaptureMode::Display,
            app: None,
            bundle_id: None,
            window_title: None,
            window_id: None,
            display_id: 1,
            frame,
        };
        let baseline = solid(1280, 800, 30);
        let mut selected = Vec::new();
        for (index, t, color) in [(1, 1_000, [10, 220, 80]), (2, 5_000, [220, 220, 80])] {
            let mut raw = baseline.clone();
            for y in 144..224 {
                for x in 48..240 {
                    let i = (y * 1280 + x) * 4;
                    raw.rgba[i..i + 3].copy_from_slice(&color);
                }
            }
            selected.push((keyframe(index, t), raw));
        }
        assert!(!change_regions(&baseline, &selected[0].1).is_empty());
        assert!(!change_regions(&selected[0].1, &selected[1].1).is_empty());
        let cursor = [CursorSample(0, 0.5, 0.5)];
        let has_subject = |f: &RgbaFrame| {
            f.rgba
                .chunks_exact(4)
                .any(|p| p[..3] == [10, 220, 80] || p[..3] == [220, 220, 80])
        };
        // A maximized active window used to permit a tight Focus crop; missing
        // window metadata used to bypass the passive safety check altogether.
        for windows in [
            Vec::new(),
            vec![WindowSnapshot {
                t: 0,
                windows: vec![window(1, "Editor", [0, 0, 1280, 800], true)],
            }],
        ] {
            for scope in [Scope::Keyframe, Scope::Grouped, Scope::Whole] {
                let settings = Settings {
                    scope,
                    ..Settings::default()
                };
                let (frames, _) = annotate_and_crop(
                    selected.clone(),
                    Some(&baseline),
                    Some(&settings),
                    &target,
                    6_000,
                    &[],
                    &[],
                    &[],
                    &windows,
                    &cursor,
                );
                assert!(
                    frames.iter().all(|(_, f)| has_subject(f)),
                    "{scope:?}, {} window snapshots: lost edge content",
                    windows.len()
                );
                assert!(
                    frames
                        .iter()
                        .all(|(_, f)| (f.width, f.height) == (1280, 800))
                );
                let sheet = crate::capture::engine::render::render_contact_sheet_for_duration(
                    6_000, &frames,
                );
                assert!(has_subject(&sheet), "{scope:?}: sheet lost edge content");
            }
        }
    }

    #[test]
    fn passive_crop_cannot_keep_only_a_blank_sliver_of_the_active_window() {
        let frame = TargetFrame {
            width: 1280.0,
            height: 800.0,
            ..FRAME
        };
        let windows = [WindowSnapshot {
            t: 73,
            windows: vec![window(1, "Editor", [320, 203, 640, 420], true)],
        }];
        let cursor = [
            CursorSample(73, 0.5, 0.5),
            CursorSample(1_306, 0.9492, 0.0488),
        ];
        let changes = [vec![
            [0.25, 0.0, 1.0, 0.225],
            [0.25, 0.75, 0.765625, 0.825],
            [0.3125, 0.475, 0.609375, 0.575],
            [0.3125, 0.375, 0.578125, 0.45],
            [0.3125, 0.6, 0.6875, 0.65],
            [0.84375, 0.075, 0.875, 0.1],
        ]];
        let active = [0.25, 203.0 / 800.0, 0.75, 623.0 / 800.0];
        let input = Input::new(
            frame,
            13_207,
            &[6_734],
            &[],
            &[],
            &windows,
            &cursor,
            &changes,
        );
        let settings = Settings::default();
        let mut crop = compute(&input, &settings).remove(0);
        // The final content-trimmed geometry observed in the normal-decoder GUI capture.
        // It intersects blank window chrome, but excludes every word in the editor.
        crop.rect = [869.0 / 1280.0, 0.0, 1.0, 306.0 / 800.0];
        crop.kind = CropKind::Activity;
        let ctx = Ctx {
            input: &input,
            s: &settings,
            sw: 1280.0,
            sh: 800.0,
        };
        assert!(area(intersect(crop.rect, active)) > 0.0);
        ctx.keep_frontmost(&mut crop, 6_734);
        assert!(contains(crop.rect, active), "{crop:?}");
    }

    #[test]
    fn deliberate_input_still_selects_a_nonfrontmost_window() {
        let windows = [WindowSnapshot {
            t: 0,
            windows: vec![
                window(1, "Editor", [100, 100, 600, 400], true),
                window(2, "Browser", [1100, 100, 500, 600], false),
            ],
        }];
        let input = Input::new(
            FRAME,
            2_000,
            &[1_000],
            &[],
            &[click(1_000, 0.8, 0.3)],
            &windows,
            &[CursorSample(0, 0.95, 0.05)],
            &[],
        );
        let crops = compute(&input, &Settings::default());
        assert_eq!(crops[0].kind, CropKind::Window);
        assert_eq!(crops[0].apps, ["Browser"]);
        assert_eq!(label(&crops).as_deref(), Some("Browser"));
    }

    #[test]
    fn activity_label_does_not_name_an_excluded_window() {
        let windows = [WindowSnapshot {
            t: 0,
            windows: vec![window(1, "Editor", [100, 100, 600, 400], true)],
        }];
        let input = Input::new(
            FRAME,
            2_000,
            &[1_000],
            &[],
            &[click(1_000, 0.95, 0.1)],
            &windows,
            &[],
            &[vec![[0.1, 0.1, 0.3, 0.3]]],
        );
        let settings = Settings {
            snap: false,
            cause_reach: 0.0,
            ..Settings::default()
        };
        let crops = compute(&input, &settings);
        assert!(crops[0].apps.is_empty(), "{crops:?}");
        assert_eq!(label(&crops).as_deref(), Some("the activity"));
    }

    #[test]
    fn window_snapshots_do_not_arrive_before_their_timestamp() {
        let windows = [WindowSnapshot {
            t: 1_100,
            windows: vec![window(1, "Editor", [100, 100, 600, 400], true)],
        }];
        let input = Input::new(FRAME, 2_000, &[1_000, 1_200], &[], &[], &windows, &[], &[]);
        assert!(input.windows_at(1_000, &Settings::default()).is_empty());
        assert_eq!(input.windows_at(1_200, &Settings::default()).len(), 1);
    }

    #[test]
    fn crop_view_moves_ink_and_clicks_into_the_crop() {
        let r = [0.25, 0.5, 0.75, 1.0];
        let stroke = Stroke {
            t0: 0,
            t1: 10,
            fw: 1000,
            fh: 800,
            on: None,
            pin: None,
            laser: false,
            rgb: None,
            pts: vec![(0.5 * SCALE as f32) as i32, (0.75 * SCALE as f32) as i32, 0],
        };
        let view = crop_view(r, &keyframe(1, 0), &[stroke], &[], &[click(0, 0.25, 1.0)]);
        let p = view.strokes[0].timed_points()[0];
        assert!(
            (p.0 - 0.5).abs() < 1e-3 && (p.1 - 0.5).abs() < 1e-3,
            "{p:?}"
        );
        assert_eq!((view.strokes[0].fw, view.strokes[0].fh), (500, 400));
        assert!(
            matches!(view.events[0], InteractionEvent::Click { x, y, .. } if x == 0.0 && y == 1.0)
        );
        assert_eq!(view.keyframe.cursor, Some([0.5, 0.0]));
    }

    #[test]
    fn crop_frame_takes_the_pixels_inside_the_rect() {
        let mut frame = solid(100, 50, 0);
        for y in 10..20 {
            for x in 40..60 {
                frame.rgba[(y * 100 + x) * 4] = 255;
            }
        }
        let out = crop_frame(&frame, [0.4, 0.2, 0.6, 0.4]);
        assert_eq!((out.width, out.height), (20, 10));
        assert!(out.rgba.chunks(4).all(|p| p[0] == 255));
    }

    #[test]
    fn change_regions_compare_frames_of_different_sizes_by_position() {
        let small = solid(640, 414, 30);
        let mut big = solid(2560, 1654, 30);
        for y in 400..800 {
            for x in 1280..1920 {
                let i = (y * 2560 + x) * 4;
                big.rgba[i..i + 3].copy_from_slice(&[230, 230, 230]);
            }
        }
        let regions = change_regions(&small, &big);
        assert_eq!(regions.len(), 1, "{regions:?}");
        let r = regions[0];
        assert!(
            r[0] <= 0.5 && r[2] >= 0.75 && r[1] <= 0.25 && r[3] >= 0.48,
            "{r:?}"
        );
        assert!(r[0] >= 0.45 && r[2] <= 0.8, "{r:?}");
        assert!(change_regions(&small, &solid(1280, 828, 30)).is_empty());
    }

    #[test]
    fn content_snapping_moves_an_edge_out_of_a_line_of_text() {
        // A text block from x 0.30 to 0.70 of the screen; a crop whose right edge cuts through
        // it moves out to the white space after it.
        let mut frame = solid(1728, 1117, 255);
        for y in (300..600).step_by(12) {
            for x in 518..1210 {
                if x % 7 < 4 {
                    for dy in 0..4 {
                        let i = ((y + dy) * 1728 + x) * 4;
                        frame.rgba[i..i + 3].copy_from_slice(&[20, 20, 20]);
                    }
                }
            }
        }
        let content = Content::from_frame(&frame, [1728.0, 1117.0]);
        let input = Input {
            screen: [1728.0, 1117.0],
            ..Default::default()
        };
        let s = Settings::default();
        let ctx = Ctx {
            input: &input,
            s: &s,
            sw: 1728.0,
            sh: 1117.0,
        };
        let r = ctx.snap([0.28, 0.25, 0.6, 0.55], &content, [0.4, 0.3, 0.5, 0.4]);
        assert!(r[2] >= 0.70 && r[2] < 0.76, "right edge {r:?}");
        assert!(r[0] <= 0.30 && r[0] > 0.26, "left edge {r:?}");
    }

    /// A Linux capture without a movie: 480x270 candidates of a 1920x1080 screen, fewer
    /// pixels than 3 pt cells, read past the frame's last row.
    #[test]
    fn content_of_a_frame_smaller_than_its_cells() {
        let frame = solid(480, 270, 255);
        let content = Content::from_frame(&frame, [1920.0, 1080.0]);
        assert_eq!((content.cols, content.rows), (640, 360));
    }

    /// Kit's capture: a tall Focus crop whose right edge ran through a ragged paragraph,
    /// cutting "…instead of 5 GB, a" mid-line, because each column there is only a few
    /// percent text.
    #[test]
    fn crop_edges_finish_the_lines_they_cross() {
        let mut frame = RgbaFrame::new(0, 1728, 1117, vec![30; 1728 * 1117 * 4]);
        let mut fill = |x0: usize, x1: usize, y0: usize, h: usize| {
            for y in y0..y0 + h {
                for x in x0..x1 {
                    // Letters with a 4 px space between words.
                    if x % 48 < 44 {
                        let i = (y * 1728 + x) * 4;
                        frame.rgba[i..i + 3].copy_from_slice(&[220, 220, 220]);
                    }
                }
            }
        };
        // Paragraph lines from x 646 to between 1180 and 1300, every 26 pt.
        for (i, end) in [1300, 1250, 1290, 1180, 1300, 1270, 1240, 1300]
            .iter()
            .enumerate()
        {
            fill(646, *end, 120 + i * 26, 12);
        }
        // A rule across the whole window.
        fill(310, 1700, 400, 2);
        let content = Content::from_frame(&frame, [1728.0, 1117.0]);
        let input = Input {
            screen: [1728.0, 1117.0],
            ..Default::default()
        };
        let s = Settings::default();
        let ctx = Ctx {
            input: &input,
            s: &s,
            sw: 1728.0,
            sh: 1117.0,
        };
        let crop = [
            621.0 / 1728.0,
            27.0 / 1117.0,
            1248.0 / 1728.0,
            832.0 / 1117.0,
        ];
        let window = [310.0 / 1728.0, 38.0 / 1117.0, 1990.0 / 1728.0, 1.0];
        let r = ctx.finish_lines(crop, &content, window);
        let right = r[2] * 1728.0;
        assert!((1300.0..1330.0).contains(&right), "right edge {right}");
        assert_eq!(r[0], crop[0], "nothing crosses the left edge");
        assert_eq!((r[1], r[3]), (crop[1], crop[3]));
        // Never past the window; lines that run to (or nearly to) its edge are left alone,
        // like the rule.
        let narrow = [310.0 / 1728.0, 0.0, 1270.0 / 1728.0, 1.0];
        let right = ctx.finish_lines(crop, &content, narrow)[2] * 1728.0;
        assert!((1245.0..=1270.0).contains(&right), "{right}");
    }

    #[test]
    fn config_turns_it_off_or_overrides_settings() {
        assert_eq!(Settings::from_config(None), Some(Settings::default()));
        assert_eq!(
            Settings::from_config(Some(&serde_json::json!(true))),
            Some(Settings::default())
        );
        assert_eq!(Settings::from_config(Some(&serde_json::json!(false))), None);
        let s = Settings::from_config(Some(
            &serde_json::json!({ "padding": 32, "scope": "whole" }),
        ))
        .unwrap();
        assert_eq!((s.padding, s.scope), (32.0, Scope::Whole));
        assert_eq!(s.min_w, Settings::default().min_w);
    }

    #[test]
    fn screen_captures_crop_to_the_window_used_and_keep_the_full_frames() {
        let target = TargetInfo {
            mode: CaptureMode::Display,
            app: None,
            bundle_id: None,
            window_title: None,
            window_id: None,
            display_id: 1,
            frame: FRAME,
        };
        let windows = vec![WindowSnapshot {
            t: 0,
            windows: vec![
                window(1, "Finder", [100, 100, 600, 400], true),
                window(2, "Safari", [900, 100, 700, 900], false),
            ],
        }];
        let events = vec![click(1_000, 0.25, 0.3), click(1_300, 0.3, 0.35)];
        let selected = vec![(keyframe(1, 1_000), solid(1728, 1117, 40))];
        let (frames, cropped) = annotate_and_crop(
            selected.clone(),
            None,
            Some(&Settings::default()),
            &target,
            2_000,
            &[],
            &[],
            &events,
            &windows,
            &[],
        );
        let (info, full) = cropped.expect("cropped");
        assert_eq!((frames[0].1.width, frames[0].1.height), (600, 400));
        assert_eq!((full[0].1.width, full[0].1.height), (1728, 1117));
        let c = &info.crops[0];
        assert_eq!((c.x, c.y, c.width, c.height), (100.0, 100.0, 600.0, 400.0));
        assert_eq!((c.kind, info.label.as_str()), (CropKind::Window, "Finder"));
        assert!(!serde_json::to_string(&info).unwrap().contains("Secret"));

        // Other capture modes and auto-crop off leave the frames alone.
        for (mode, settings) in [
            (CaptureMode::Window, Some(Settings::default())),
            (CaptureMode::Display, None),
        ] {
            let target = TargetInfo {
                mode,
                ..target.clone()
            };
            let (frames, cropped) = annotate_and_crop(
                selected.clone(),
                None,
                settings.as_ref(),
                &target,
                2_000,
                &[],
                &[],
                &events,
                &windows,
                &[],
            );
            assert!(cropped.is_none());
            assert_eq!(frames[0].1.width, 1728);
        }
    }
