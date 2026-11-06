use std::path::PathBuf;

use anyhow::Result;

use crate::shared::handoff;
use crate::shared::ids::make_bundle_id;
use crate::shared::model::{ApparentBundle, CaptureMode, InteractionEvent, SCALE, Stroke, TargetFrame, TargetInfo, TranscriptData, TranscriptSegment, WordTiming};
use crate::capture::engine::render::render_synthetic_demo_frame;
use crate::capture::engine::bundle::bundle_processing::{build_bundle_from_parts, SourceExtras};

/// Generates a rich, deterministic multi-frame demo bundle showcasing clicks, circles, arrows, pins, and aligned speech.
pub(crate) fn generate_demo_bundle(out: Option<PathBuf>) -> Result<(PathBuf, ApparentBundle, String)> {
    let bundle_dir = out.unwrap_or_else(|| {
        handoff::default_captures_dir().join(format!("{}.apparent", make_bundle_id()))
    });
    let (fw, fh) = (1280_u32, 800_u32);
    let target = TargetInfo {
        mode: CaptureMode::Window,
        app: Some("OpenCode".into()),
        bundle_id: Some("ai.opencode.desktop".into()),
        window_title: Some("session/tool-running — OpenCode".into()),
        window_id: Some(4210),
        display_id: 1,
        frame: TargetFrame {
            x: 100.0,
            y: 60.0,
            width: f64::from(fw),
            height: f64::from(fh),
            scale: 2.0,
        },
    };

    let frames = vec![
        render_synthetic_demo_frame(fw, fh, 400, 0),
        render_synthetic_demo_frame(fw, fh, 1_500, 1),
        render_synthetic_demo_frame(fw, fh, 3_600, 1),
        render_synthetic_demo_frame(fw, fh, 6_400, 2),
        render_synthetic_demo_frame(fw, fh, 9_200, 2),
    ];

    // 1. Circle around the shifted header band (cx=0.52, cy=0.14, r_x=0.14, r_y=0.06)
    let mut circle_pts = Vec::new();
    for i in 0..=36 {
        let angle = (i as f32 / 36.0) * std::f32::consts::TAU;
        let x = ((0.52 + 0.13 * angle.cos()) * SCALE as f32).round() as i32;
        let y = ((0.15 + 0.055 * angle.sin()) * SCALE as f32).round() as i32;
        circle_pts.extend_from_slice(&[x, y, i * 18]);
    }
    let circle_stroke = Stroke {
        t0: 2_900,
        t1: 2_900 + 36 * 18,
        fw,
        fh,
        on: Some("Band header".into()),
        pin: None,
        laser: false,
        rgb: None,
        pts: circle_pts,
    };

    // 2. Arrow pointing from sidebar item to the tool card (shaft + arrowhead)
    let mut shaft_pts = Vec::new();
    for i in 0..=12 {
        let t = i as f32 / 12.0;
        let x = ((0.22 + (0.44 - 0.22) * t) * SCALE as f32).round() as i32;
        let y = ((0.48 + (0.36 - 0.48) * t) * SCALE as f32).round() as i32;
        shaft_pts.extend_from_slice(&[x, y, i * 16]);
    }
    let arrow_shaft = Stroke {
        t0: 5_800,
        t1: 5_800 + 12 * 16,
        fw,
        fh,
        on: Some("Used 12 tools".into()),
        pin: None,
        laser: false,
        rgb: None,
        pts: shaft_pts,
    };
    let arrow_head = Stroke {
        t0: 6_100,
        t1: 6_160,
        fw,
        fh,
        on: Some("Used 12 tools".into()),
        pin: None,
        laser: false,
        rgb: None,
        pts: vec![
            (0.41 * SCALE as f32) as i32,
            (0.35 * SCALE as f32) as i32,
            0,
            (0.44 * SCALE as f32) as i32,
            (0.36 * SCALE as f32) as i32,
            30,
            (0.42 * SCALE as f32) as i32,
            (0.40 * SCALE as f32) as i32,
            60,
        ],
    };

    // 3. Numbered pin on the composer send button
    let pin_stroke = Stroke {
        t0: 8_900,
        t1: 8_900,
        fw,
        fh,
        on: Some("Send button".into()),
        pin: Some(1),
        laser: false,
        rgb: None,
        pts: vec![
            (0.88 * SCALE as f32) as i32,
            (0.88 * SCALE as f32) as i32,
            0,
        ],
    };

    let strokes = vec![circle_stroke, arrow_shaft, arrow_head, pin_stroke];
    let events = vec![
        InteractionEvent::Click {
            t: 1_250,
            x: 0.46,
            y: 0.36,
            button: Some("left".into()),
            label: Some("Used 12 tools".into()),
            role: Some("button".into()),
        },
        InteractionEvent::Scroll {
            t: 4_800,
            x: 0.55,
            y: 0.55,
            dx: 0.0,
            dy: -240.0,
            label: Some("Transcript".into()),
        },
    ];

    let transcript = TranscriptData {
        source: "speech-analyzer".into(),
        model: Some("SpeechTranscriber en_US".into()),
        text: "When I click Used 12 tools this band header jumps down by four pixels, and the row spacing under the card feels too tight.".into(),
        segments: vec![
            TranscriptSegment {
                t0: 600,
                t1: 4_100,
                text: "When I click Used 12 tools this band header jumps down by four pixels,".into(),
            },
            TranscriptSegment {
                t0: 5_400,
                t1: 9_400,
                text: "and the row spacing under the card feels too tight.".into(),
            },
        ],
        words: vec![
            WordTiming { t0: 600, t1: 850, text: "When".into() },
            WordTiming { t0: 850, t1: 980, text: "I".into() },
            WordTiming { t0: 980, t1: 1_320, text: "click".into() },
            WordTiming { t0: 1_320, t1: 1_650, text: "Used".into() },
            WordTiming { t0: 1_650, t1: 1_900, text: "12".into() },
            WordTiming { t0: 1_900, t1: 2_300, text: "tools".into() },
            WordTiming { t0: 2_450, t1: 2_700, text: "this".into() },
            WordTiming { t0: 2_700, t1: 3_050, text: "band".into() },
            WordTiming { t0: 3_050, t1: 3_400, text: "header".into() },
            WordTiming { t0: 3_400, t1: 3_700, text: "jumps".into() },
            WordTiming { t0: 3_700, t1: 3_900, text: "down".into() },
            WordTiming { t0: 3_900, t1: 4_100, text: "by four pixels,".into() },
            WordTiming { t0: 5_400, t1: 5_800, text: "and the".into() },
            WordTiming { t0: 5_800, t1: 6_300, text: "row spacing".into() },
            WordTiming { t0: 6_300, t1: 7_200, text: "under the card".into() },
            WordTiming { t0: 7_800, t1: 9_400, text: "feels too tight.".into() },
        ],
        estimated: false,
        missing: None,
    };

    build_bundle_from_parts(
        &bundle_dir,
        target,
        9_800,
        frames,
        strokes,
        events,
        transcript,
        None,
        Some("Header inset shifts on expand".into()),
        SourceExtras::default(),
    )
}
