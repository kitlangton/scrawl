#[cfg(test)]
fn wrap_text(text: &str, size: f32, width: f32, bold: bool) -> Vec<String> {
    wrap_text_indented(text, size, width, width, bold)
}

#[cfg(test)]
fn wrap_text_indented(
    text: &str,
    size: f32,
    width: f32,
    first_width: f32,
    bold: bool,
) -> Vec<String> {
    let mut lines = Vec::new();
    for paragraph in text.lines() {
        let mut line = String::new();
        for word in paragraph.split_whitespace() {
            let candidate = if line.is_empty() {
                word.to_owned()
            } else {
                format!("{line} {word}")
            };
            let available = if lines.is_empty() { first_width } else { width };
            if text_width(&candidate, size, bold) <= available {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            // Character boundaries avoid slicing inside UTF-8, including emoji.
            for ch in word.chars() {
                let mut candidate = line.clone();
                candidate.push(ch);
                let available = if lines.is_empty() { first_width } else { width };
                if !line.is_empty() && text_width(&candidate, size, bold) > available {
                    lines.push(std::mem::take(&mut line));
                }
                line.push(ch);
            }
        }
        if !line.is_empty() {
            lines.push(line);
        }
    }
    lines
}


    use super::*;
    use std::io::Cursor;

    fn key(index: usize) -> Keyframe {
        Keyframe {
            index,
            t: 4000,
            reason: KeyframeReason::Mark,
            file: format!("frames/{index}.png"),
            event: "✎ circled region x 0.42–0.58, y 0.26–0.34 over “Band header”".into(),
            said: Some("When I click Used 12 tools…".into()),
            mark_id: None,
            cursor: None,
        }
    }

    #[test]
    fn opaque_rounded_spans_match_source_over_edges_and_clipping() {
        for rect in [
            [0.0, 0.0, 32.0, 24.0],
            [-4.25, -2.5, 21.75, 16.25],
            [8.75, 5.25, 31.5, 24.75],
            [2.25, 3.75, 0.5, 1.25],
            [-50.0, 0.0, 5.0, 5.0],
            [40.0, 0.0, 5.0, 5.0],
        ] {
            for radius in [0.0, 0.25, 2.5, 10.0, 100.0] {
                for alpha in [0, 127, 255] {
                    let color = [59, 130, 243, alpha];
                    let mut expected = solid(32, 24, [90, 60, 30, 137]);
                    let mut actual = expected.clone();
                    rounded(&mut actual, rect, radius, color);
                    let [x, y, w, h] = rect;
                    for py in (y.floor() as i32 - 1).max(0)..((y + h).ceil() as i32 + 1).min(24) {
                        for px in (x.floor() as i32 - 1).max(0)..((x + w).ceil() as i32 + 1).min(32)
                        {
                            let r = radius.max(0.0).min(w * 0.5).min(h * 0.5);
                            let qx = (px as f32 + 0.5 - x - w * 0.5).abs() - w * 0.5 + r;
                            let qy = (py as f32 + 0.5 - y - h * 0.5).abs() - h * 0.5 + r;
                            let distance = qx.max(0.0).hypot(qy.max(0.0)) + qx.max(qy).min(0.0) - r;
                            let coverage = (0.5 - distance).clamp(0.0, 1.0);
                            blend(&mut expected, px, py, color, coverage);
                        }
                    }
                    assert_eq!(
                        actual.rgba, expected.rgba,
                        "rect={rect:?} radius={radius} alpha={alpha}"
                    );
                }
            }
        }
    }

    #[test]
    fn swipes_leave_a_trail_on_phone_keyframes() {
        let base = solid(400, 860, [240, 240, 240, 255]);
        let swipe = InteractionEvent::Drag {
            t: 3000,
            t1: 3400,
            x0: 0.5,
            y0: 0.8,
            x1: 0.5,
            y1: 0.3,
            label: None,
            button: Some("touch".into()),
            path: vec![[0.5, 0.8], [0.52, 0.55], [0.5, 0.3]],
        };
        let blue = |frame: &RgbaFrame| {
            frame
                .rgba
                .chunks_exact(4)
                .filter(|p| p[2] > 200 && p[0] < 120)
                .count()
        };
        let after = render_annotated_keyframe(&base, &key(1), &[], &[], &[swipe.clone()]);
        assert!(blue(&after) > 500, "trail missing");
        let mut later = key(1);
        later.t = 6000;
        let stale = render_annotated_keyframe(&base, &later, &[], &[], &[swipe]);
        assert_eq!(blue(&stale), 0, "an old swipe should not be drawn");
    }

    /// A click is marked by a thin ring and a small dot: what was clicked stays visible.
    #[test]
    fn click_marks_leave_what_they_mark_visible() {
        let base = solid(1280, 800, [240, 240, 240, 255]);
        let mut keyframe = key(1);
        keyframe.reason = KeyframeReason::Click;
        keyframe.cursor = Some([0.5, 0.5]);
        let after = render_annotated_keyframe(&base, &keyframe, &[], &[], &[]);
        let changed: Vec<(usize, usize)> = (0..800)
            .flat_map(|y| (0..1280).map(move |x| (x, y)))
            .filter(|&(x, y)| {
                let i = (y * 1280 + x) * 4;
                after.rgba[i..i + 4] != base.rgba[i..i + 4]
            })
            .filter(|&(x, y)| x > 600 && x < 680 && y > 360 && y < 440)
            .collect();
        assert!(!changed.is_empty(), "click mark missing");
        // A quarter of the filled 18 px disc it replaces, and the band between dot and ring is
        // left alone.
        assert!(changed.len() < 260, "{} px painted", changed.len());
        for (x, y) in [(645, 400), (640, 395), (636, 400)] {
            let i = (y * 1280 + x) * 4;
            assert_eq!(
                after.rgba[i..i + 4],
                base.rgba[i..i + 4],
                "({x}, {y}) covered"
            );
        }
        // Nothing is opaque: the screen shows through every pixel of the mark.
        for (x, y) in changed {
            let i = (y * 1280 + x) * 4;
            assert!(after.rgba[i] > 20, "({x}, {y}) hides the screen");
        }
    }

    #[test]
    fn png_round_trip_preserves_rgba_including_alpha() {
        let frame = RgbaFrame::new(
            123,
            2,
            2,
            vec![
                255, 59, 48, 255, 3, 5, 7, 0, 23, 42, 91, 128, 255, 255, 255, 255,
            ],
        );
        let result = decode_png(Cursor::new(encode_png(&frame).unwrap())).unwrap();
        assert_eq!((result.width, result.height), (2, 2));
        assert_eq!(result.rgba, frame.rgba);
    }

    #[test]
    fn recorded_stroke_rgb_reaches_keyframe_png_and_export_pixels() {
        let base = solid(320, 200, [20, 20, 20, 255]);
        let stroke: Stroke = serde_json::from_value(serde_json::json!({
            "t0":3500,"t1":3900,"fw":320,"fh":200,"rgb":[17,203,81],
            "pts":[1500,6000,0,5000,6000,200,8500,6000,400]
        })).unwrap();
        let image = render_annotated_keyframe(&base, &key(1), &[stroke.clone()], &[], &[]);
        let png = encode_png(&image).unwrap();
        let decoded = decode_png(std::io::Cursor::new(&png)).unwrap();
        assert!(decoded.rgba.chunks_exact(4).filter(|p| *p == [17,203,81,255]).count() > 80);
        #[cfg(target_os = "linux")]
        {
            let mut bundle: ApparentBundle = serde_json::from_value(serde_json::json!({
                "version":1,"id":"rgb-synthetic","createdAtMs":0,"durationMs":5000,
                "target":{"mode":"display","displayId":0,"frame":{"x":0,"y":0,"width":320,"height":200,"scale":1}},
                "transcript":{"source":"synthetic","text":"","segments":[]},"strokes":[],"marks":[],"events":[],"keyframes":[],"contactSheet":"contact.png"
            })).unwrap();
            bundle.strokes.push(stroke);
            let plan = crate::capture::engine::export::ink::plan(&bundle, (320,200), crate::capture::engine::export::ink::Layers { ink:true, clicks:false });
            let mut exported = base.clone();
            draw_export_annotations(&mut exported, 3900.0, [0.0,0.0,1.0,1.0], (320,200), &plan);
            assert!(exported.rgba.chunks_exact(4).filter(|p| *p == [17,203,81,255]).count() > 80);
        }
    }

    #[test]
    fn active_ink_paints_red_but_future_ink_is_hidden() {
        let base = solid(320, 200, [20, 20, 20, 255]);
        let stroke = Stroke {
            t0: 3500,
            t1: 3900,
            fw: 320,
            fh: 200,
            on: None,
            pin: None,
            laser: false,
            rgb: None,
            pts: vec![1500, 6000, 0, 5000, 6000, 200, 8500, 6000, 400],
        };
        let result = render_annotated_keyframe(&base, &key(1), &[stroke.clone()], &[], &[]);
        let red = result
            .rgba
            .chunks_exact(4)
            .enumerate()
            .filter(|(i, p)| i / 320 > 100 && p[0] == 255 && p[3] == 255)
            .count();
        assert!(
            red > 80,
            "Expected opaque red stroke pixels below the badge, got {red}"
        );
        let future = Stroke {
            t0: 5000,
            t1: 5400,
            ..stroke
        };
        let result = render_annotated_keyframe(&base, &key(1), &[future], &[], &[]);
        assert_eq!(&result.rgba[100 * 320 * 4..], &base.rgba[100 * 320 * 4..]);
        assert!(base.rgba.chunks_exact(4).all(|p| p == [20, 20, 20, 255]));
    }

    #[test]
    fn a_grouped_keyframe_shows_every_mark_drawn_on_its_screen() {
        let base = solid(320, 200, [20, 20, 20, 255]);
        // Three strokes across the lower half, drawn long before the keyframe.
        let stroke = |y: i32| Stroke {
            t0: 100,
            t1: 300,
            fw: 320,
            fh: 200,
            on: None,
            pin: None,
            laser: false,
            rgb: None,
            pts: vec![1500, y, 0, 5000, y, 100, 8500, y, 200],
        };
        let strokes = [stroke(6000), stroke(7500), stroke(9000)];
        let mark = |id: &str, stroke: usize, frame: &str| Mark {
            id: id.into(),
            strokes: vec![stroke],
            t0: 100,
            t1: 300,
            box_norm: [0.15, 0.6, 0.85, 0.9],
            shape: Shape::Underline,
            from: None,
            to: None,
            pin: None,
            on: None,
            description: String::new(),
            frame: Some(frame.into()),
        };
        let mut keyframe = key(1);
        keyframe.mark_id = Some("m1".into());
        let marks = [
            mark("m1", 0, &keyframe.file),
            mark("m2", 1, &keyframe.file),
            mark("m3", 2, "frames/2.png"),
        ];
        let result = render_annotated_keyframe(&base, &keyframe, &strokes, &marks, &[]);
        let red_rows = |rows: std::ops::Range<usize>| {
            rows.filter(|y| {
                result.rgba[y * 320 * 4..(y + 1) * 320 * 4]
                    .chunks_exact(4)
                    .any(|p| p[0] == 255 && p[1] < 80)
            })
            .count()
        };
        assert!(red_rows(115..125) > 0 && red_rows(145..155) > 0);
        assert_eq!(red_rows(175..185), 0, "another keyframe's mark");

        // No numbered badges beside the marks (the text numbers them): below the corner badge,
        // every pixel is the backdrop or red ink.
        let foreign = result.rgba[60 * 320 * 4..]
            .chunks_exact(4)
            .filter(|p| p[..3] != [20, 20, 20] && !(p[0] > p[1] && p[0] > p[2]))
            .count();
        assert_eq!(foreign, 0, "pixels that are neither backdrop nor ink");
        // Tests run with experiments off, so also check the images never depend on agentContext.
        assert!(!include_str!("../render.rs").contains(concat!("handoff::", "AGENT_CONTEXT")));
    }

    /// The box of pixels in `frame` that differ from `base`, as `[x0, y0, x1, y1]` inclusive.
    fn changed_box(frame: &RgbaFrame, base: &RgbaFrame) -> Option<[usize; 4]> {
        let w = frame.width as usize;
        let mut found: Option<[usize; 4]> = None;
        for (i, (a, b)) in frame
            .rgba
            .chunks_exact(4)
            .zip(base.rgba.chunks_exact(4))
            .enumerate()
        {
            if a != b {
                let (x, y) = (i % w, i / w);
                let r = found.get_or_insert([x, y, x, y]);
                *r = [r[0].min(x), r[1].min(y), r[2].max(x), r[3].max(y)];
            }
        }
        found
    }

    #[test]
    fn a_keyframe_is_labelled_by_its_number_alone_in_a_small_white_chip() {
        let base = solid(1280, 800, [120, 130, 140, 255]);
        let mut frame = base.clone();
        stamp_keyframe_badge_in_place(&mut frame, &key(7));
        let [x0, y0, x1, y1] = changed_box(&frame, &base).expect("no chip");
        // k = 800 / 480: the chip sits 5k from the corner (its ring just outside) and is square
        // for one digit: no `#`, no time beside it.
        let k = 800.0 / 480.0;
        assert!((x0 as f32 - 3.6 * k).abs() <= 1.5 && (y0 as f32 - 3.6 * k).abs() <= 1.5);
        let (w, h) = ((x1 - x0 + 1) as f32, (y1 - y0 + 1) as f32);
        assert!((h - 26.8 * k).abs() <= 2.0, "{w}x{h}");
        assert!(
            (w - h).abs() <= 1.0,
            "a one-digit chip should be square, got {w}x{h}"
        );
        let at = |x: usize, y: usize| &frame.rgba[(y * 1280 + x) * 4..][..4];
        // A white fill with dark digit strokes inside it, and a dark ring around it.
        let inner: Vec<_> = (y0 + 4..y1 - 3)
            .flat_map(|y| (x0 + 4..x1 - 3).map(move |x| (x, y)))
            .map(|(x, y)| at(x, y)[0])
            .collect();
        assert!(inner.iter().filter(|&&v| v > 240).count() > inner.len() / 2);
        assert!(inner.iter().filter(|&&v| v < 40).count() > 20, "no digit");
        assert!(at(x0 + 1, (y0 + y1) / 2)[0] < 40, "no ring");

        // Two digits widen it but keep it small.
        let mut wide = base.clone();
        stamp_keyframe_badge_in_place(&mut wide, &key(12));
        let [a0, _, a1, _] = changed_box(&wide, &base).unwrap();
        let w12 = (a1 - a0 + 1) as f32;
        assert!(w12 > w && w12 < 2.0 * h, "{w12}");
    }

    #[test]
    fn the_chip_avoids_top_right_controls_when_ink_or_cursor_is_top_left() {
        let base = solid(1280, 800, [120, 130, 140, 255]);
        let mut inked = base.clone();
        inked.badge_bottom_left = true;
        stamp_keyframe_badge_in_place(&mut inked, &key(3));
        let mut pointed = base.clone();
        let mut keyframe = key(3);
        keyframe.cursor = Some([0.1, 0.05]);
        stamp_keyframe_badge_in_place(&mut pointed, &keyframe);
        for frame in [&inked, &pointed] {
            let [x0, _, _, y1] = changed_box(frame, &base).unwrap();
            let k = 800.0 / 480.0;
            assert!(x0 < 30, "chip at x {x0}");
            assert!((799.0 - y1 as f32 - 3.6 * k).abs() <= 1.5);
        }
    }

    #[test]
    fn contact_sheet_chips_are_one_readable_size_in_every_tile() {
        // Twelve tiles of mixed shapes: every chip is about 24 px whatever the tile's size.
        let dims = [(1280, 800), (736, 1600), (930, 975), (1054, 327)];
        let frames: Vec<_> = (1..=12)
            .map(|i| {
                let (w, h) = dims[i % dims.len()];
                let mut frame = solid(w, h, [120, 130, 140, 255]);
                frame.badge_bottom_left = i % 3 == 0;
                (key(i), frame)
            })
            .collect();
        let sheet = render_contact_sheet_for_duration(8000, &frames);
        // Chip pixels are the near-white fill; group them by tile through the gray around them.
        let w = sheet.width as usize;
        let white: Vec<(usize, usize)> = sheet
            .rgba
            .chunks_exact(4)
            .enumerate()
            .filter(|(_, p)| p[0] > 240 && p[1] > 240 && p[2] > 240)
            .map(|(i, _)| (i % w, i / w))
            .collect();
        let mut chips: Vec<[usize; 4]> = Vec::new();
        for (x, y) in white {
            match chips
                .iter_mut()
                .find(|c| x + 4 >= c[0] && x <= c[2] + 4 && y + 4 >= c[1] && y <= c[3] + 4)
            {
                Some(c) => *c = [c[0].min(x), c[1].min(y), c[2].max(x), c[3].max(y)],
                None => chips.push([x, y, x, y]),
            }
        }
        assert_eq!(chips.len(), 12, "{chips:?}");
        for c in chips {
            // The white fill is the chip less its ring: 21.6 px, so about 24 px with the ring.
            let h = c[3] - c[1] + 1;
            assert!((20..=23).contains(&h), "chip fill {h} px tall: {c:?}");
        }
    }

    #[test]
    fn contact_sheets_lay_out_up_to_thirteen_frames() {
        // Twelve selected screen frames plus the finished whiteboard paper.
        for count in [1, 2, 3, 4, 7, 9, 12, 13] {
            for size in [(1280, 800), (736, 1600), (930, 975)] {
                let frames: Vec<_> = (1..=count)
                    .map(|i| {
                        (
                            key(i),
                            solid(size.0, size.1, [40 + i as u8 * 10, 60, 80, 255]),
                        )
                    })
                    .collect();
                let sheet = render_contact_sheet_for_duration(8000, &frames);
                assert!(
                    sheet.width <= SHEET_MAX_EDGE as u32 && sheet.height <= SHEET_MAX_EDGE as u32,
                    "Sheet {}x{} exceeded max edge for count={count} size={size:?}",
                    sheet.width,
                    sheet.height
                );
                assert!(
                    (sheet.width as f32 * sheet.height as f32) <= SHEET_MAX_PIXELS * 1.03,
                    "Sheet {}x{} exceeded pixel budget for count={count} size={size:?}",
                    sheet.width,
                    sheet.height
                );
                let mut centroids = Vec::with_capacity(count);
                for i in 1..=count {
                    let color = [40 + i as u8 * 10, 60, 80, 255];
                    let mut sum_x = 0usize;
                    let mut sum_y = 0usize;
                    let mut n_px = 0usize;
                    for (idx, p) in sheet.rgba.chunks_exact(4).enumerate() {
                        if p == color {
                            sum_x += idx % sheet.width as usize;
                            sum_y += idx / sheet.width as usize;
                            n_px += 1;
                        }
                    }
                    assert!(n_px > 500, "Missing card {i} of {count}");
                    centroids.push((sum_x as f32 / n_px as f32, sum_y as f32 / n_px as f32));
                }
                // Reading order: every frame is either to the right on the same row, or on a lower row.
                for pair in centroids.windows(2) {
                    let (x0, y0) = pair[0];
                    let (x1, y1) = pair[1];
                    let same_row = (y1 - y0).abs() < 20.0;
                    assert!(
                        (same_row && x1 > x0) || (!same_row && y1 > y0),
                        "Reading order violated for count={count} size={size:?}: {centroids:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn contact_sheets_pack_mixed_aspects_and_fill_width_without_letterboxing() {
        // Mixed aspect ratios like an auto-cropped screen capture: wide strip, landscape, square, tall portrait.
        let dims = [
            (1054, 327),
            (1183, 837),
            (1183, 837),
            (685, 648),
            (694, 1416),
            (898, 1416),
            (898, 1416),
        ];
        let frames: Vec<_> = dims
            .iter()
            .enumerate()
            .map(|(idx, &(w, h))| {
                let i = idx + 1;
                (key(i), solid(w, h, [50 + i as u8 * 15, 90, 130, 255]))
            })
            .collect();
        let sheet = render_contact_sheet_for_duration(8000, &frames);
        let w = sheet.width as usize;
        let h = sheet.height as usize;
        let pad = SHEET_PAD as usize;
        assert_eq!(sheet.rgba.len(), w * h * 4);
        assert!(sheet.rgba.chunks_exact(4).all(|p| p[3] == 255));

        // Every non-gutter scanline inside [pad..h-pad] must start with a frame pixel at x=pad
        // and end with a frame pixel at x=w-pad-1 (every row fills the inner width completely).
        let is_card = |p: &[u8]| p != SHEET_BG;
        let mut content_rows = 0usize;
        for y in pad..h - pad {
            let left = &sheet.rgba[(y * w + pad) * 4..(y * w + pad + 1) * 4];
            let right = &sheet.rgba[(y * w + w - pad - 1) * 4..(y * w + w - pad) * 4];
            if is_card(left) || is_card(right) {
                assert!(
                    is_card(left) && is_card(right),
                    "Row y={y} did not fill full inner width"
                );
                content_rows += 1;
                // Count non-gutter pixels in this scanline: only thin horizontal gutters may be SHEET_BG.
                let bg_px = (pad..w - pad)
                    .filter(|&x| !is_card(&sheet.rgba[(y * w + x) * 4..(y * w + x + 1) * 4]))
                    .count();
                assert!(
                    bg_px <= 5 * SHEET_GAP as usize,
                    "Unexpected blank space ({bg_px} px) on scanline y={y}"
                );
            }
        }
        assert!(content_rows > h * 4 / 5, "Too many empty scanlines");
    }

    #[test]
    fn contact_sheet_balances_per_frame_area_across_wide_and_tall_crops() {
        // Aspect mix from 17e3c-9: 5 landscape/square crops followed by 3 tall portrait crops.
        let dims = [
            (685, 648),
            (1183, 837),
            (1183, 837),
            (694, 726),
            (1183, 1095),
            (694, 1416),
            (898, 1416),
            (898, 1416),
        ];
        let aspects: Vec<f32> = dims.iter().map(|&(w, h)| w as f32 / h as f32).collect();
        let rows = plan_contact_sheet_rows(&aspects);
        let mut areas: Vec<f32> = rows
            .iter()
            .flat_map(|row| {
                let a_row: f32 = aspects[row.clone()].iter().sum();
                aspects[row.clone()]
                    .iter()
                    .map(move |&a_i| a_i / (a_row * a_row))
            })
            .collect();
        areas.sort_by(f32::total_cmp);
        let n = areas.len();
        let median = 0.5 * (areas[(n - 1) / 2] + areas[n / 2]);
        let min_ratio = areas[0] / median;
        assert!(
            min_ratio >= 0.55,
            "Tall crops were crushed relative to median frame area: min/median = {min_ratio:.2} (rows={rows:?})"
        );
    }

    #[test]
    fn portrait_frames_scale_without_smearing_lower_rows() {
        // Rows past the frame's width once blended with row `width - 1`.
        let mut frame = solid(100, 400, [200, 0, 0, 255]);
        frame.rgba[200 * 100 * 4..]
            .chunks_exact_mut(4)
            .for_each(|p| p.copy_from_slice(&[0, 0, 200, 255]));
        let mut sheet = solid(100, 400, [0, 0, 0, 255]);
        blit_scaled(&mut sheet, &frame, [0.0, 0.0, 100.0, 400.0], 0.0);
        let at = |x: usize, y: usize| &sheet.rgba[(y * 100 + x) * 4..][..4];
        assert_eq!(at(50, 150), [200, 0, 0, 255]);
        assert_eq!(at(50, 300), [0, 0, 200, 255]);
        assert_eq!(at(50, 399), [0, 0, 200, 255]);
    }

    #[test]
    fn unicode_wrapping_and_empty_sheet_are_safe() {
        let lines = wrap_text(
            "✎ · – “hello” 🦀 漢字 longwordwithoutspaces",
            16.0,
            80.0,
            false,
        );
        assert!(lines.len() > 2);
        assert!(lines.iter().all(|s| text_width(s, 16.0, false) <= 81.0));
        let sheet = render_contact_sheet_for_duration(8000, &[]);
        assert!(sheet.width > 0 && sheet.height > 0);
    }
