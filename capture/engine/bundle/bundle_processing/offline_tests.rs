//! Synthetic acquisition inputs through the real selector, renderer and publisher.
use super::*;
use crate::shared::{
    handoff,
    model::{TranscriptSegment, WordTiming},
};

#[test]
fn offline_pipeline_preserves_speech_context_and_visual_change() {
    let evidence = std::env::var_os("APPARENT_OFFLINE_EVIDENCE");
    let root = evidence.as_ref().map(PathBuf::from).unwrap_or_else(|| {
        std::env::temp_dir().join(format!("{}.apparent", crate::shared::ids::make_bundle_id()))
    });
    assert!(!root.exists(), "fixture destination must be fresh");
    let target: TargetInfo = serde_json::from_value(serde_json::json!({
        "mode":"window", "app":"Synthetic settings", "windowTitle":"Offline fixture",
        "displayId":1, "frame":{"x":0,"y":0,"width":320,"height":200,"scale":1}
    }))
    .unwrap();
    let transcript = TranscriptData {
        source: "synthetic-measured".into(),
        text: "The panel is gray. The panel is blue.".into(),
        segments: vec![
            TranscriptSegment {
                t0: 100,
                t1: 900,
                text: "The panel is gray.".into(),
            },
            TranscriptSegment {
                t0: 2100,
                t1: 2900,
                text: "The panel is blue.".into(),
            },
        ],
        words: [
            (100, "The"),
            (300, "panel"),
            (500, "is"),
            (700, "gray."),
            (2100, "The"),
            (2300, "panel"),
            (2500, "is"),
            (2700, "blue."),
        ]
        .into_iter()
        .map(|(t, text)| WordTiming {
            t0: t,
            t1: t + 200,
            text: text.into(),
        })
        .collect(),
        ..Default::default()
    };
    let frames = (0..=3000)
        .step_by(100)
        .map(|t| {
            let color = if t < 2000 {
                [180, 180, 180, 255]
            } else {
                [20, 50, 220, 255]
            };
            RgbaFrame::new(t, 320, 200, color.repeat(320 * 200))
        })
        .collect();
    let events = vec![
        InteractionEvent::Click {
            t: 0,
            x: 0.5,
            y: 0.5,
            button: Some("left".into()),
            label: Some("Gray panel".into()),
            role: Some("button".into()),
        },
        InteractionEvent::Click {
            t: 2000,
            x: 0.5,
            y: 0.5,
            button: Some("left".into()),
            label: Some("Blue panel".into()),
            role: Some("button".into()),
        },
    ];
    let (_, _, summary) = build_bundle_from_parts(
        &root,
        target,
        3100,
        frames,
        vec![],
        events.clone(),
        transcript.clone(),
        None,
        Some("Synthetic offline acceptance".into()),
        SourceExtras {
            context: [("session".into(), "offline-session".into())].into(),
            autocrop: Some(None),
            ..Default::default()
        },
    )
    .unwrap();
    let (_, saved) = handoff::load_bundle(&root).unwrap();
    assert_eq!(saved.transcript, transcript);
    assert_eq!(saved.events, events);
    assert_eq!(saved.context.get("session").unwrap(), "offline-session");
    assert_eq!(
        std::fs::read_to_string(root.join("summary.md")).unwrap(),
        summary
    );
    assert!(summary.contains("gray.") && summary.contains("blue."));
    assert!(summary.contains("Synthetic offline acceptance"));
    assert!(summary.contains(&root.display().to_string()));
    assert!(
        saved.keyframes.len() >= 2,
        "the changed panel must remain visible"
    );
    let aligned: Vec<_> = saved
        .keyframes
        .iter()
        .filter_map(|key| key.said.as_deref())
        .collect();
    assert_eq!(aligned, ["The panel is gray.", "The panel is blue."]);
    let mut gray = false;
    let mut blue = false;
    for key in &saved.keyframes {
        let image = render::decode_png(std::io::Cursor::new(
            std::fs::read(root.join(&key.file)).unwrap(),
        ))
        .unwrap();
        assert_eq!((image.width, image.height), (320, 200));
        let i = (180 * 320 + 280) * 4; // away from the number chip and click
        if key.said.as_deref() == Some("The panel is gray.") {
            assert_eq!(&image.rgba[i..i + 3], &[180, 180, 180]);
        }
        if key.said.as_deref() == Some("The panel is blue.") {
            assert!(u16::from(image.rgba[i + 2]) > u16::from(image.rgba[i]) + 100);
        }
        gray |= image.rgba[i..i + 3] == [180, 180, 180];
        blue |= u16::from(image.rgba[i + 2]) > u16::from(image.rgba[i]) + 100;
    }
    assert!(gray && blue, "both source scenes survive rendering");
    render::decode_png(std::io::Cursor::new(
        std::fs::read(root.join("contact-sheet.png")).unwrap(),
    ))
    .unwrap();
    if evidence.is_none() {
        std::fs::remove_dir_all(root).unwrap();
    }
}
