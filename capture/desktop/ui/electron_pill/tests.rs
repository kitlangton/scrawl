use super::*;
use crate::capture::desktop::native::pill::PillStyle;
fn snapshot(pres: Pres) -> PillSnapshot {
    PillSnapshot {
        pres,
        clock: "00:12".into(),
        mic_warning: None,
        reduced_motion: false,
        style: PillStyle::Black,
    }
}
fn command(revision: u64, action: &str) -> Command {
    Command {
        revision,
        action: action.into(),
        target: None,
    }
}
#[test]
#[cfg(target_os = "linux")]
fn electron_pill_drag_admission_stays_with_current_recording() {
    let mut view = View::new(&snapshot(Pres::Recording { chips: false, has_iphone: false }), 12, true);
    let mut drag = command(12, "drag");
    drag.target = Some([300.0, 60.0]);
    assert_eq!(view.drag_point(&drag, true), drag.target);
    assert_eq!(view.drag_point(&drag, false), None);
    drag.revision = 11;
    assert_eq!(view.drag_point(&drag, true), None);
    drag.revision = 12;
    drag.target = Some([f32::NAN, 60.0]);
    assert_eq!(view.drag_point(&drag, true), None);
    drag.target = Some([300.0, 60.0]);
    view.draggable = false;
    assert_eq!(view.drag_point(&drag, true), None);
    view.draggable = true;
    view.phase = "selecting";
    assert_eq!(view.drag_point(&drag, true), None);
    assert!(view.admit(drag).is_none(), "Geometry cannot dispatch a capture action");
}
#[test]
fn electron_pill_refuses_stale_and_unavailable_mutations() {
    let view = View::new(
        &snapshot(Pres::Recording {
            chips: false,
            has_iphone: false,
        }),
        12,
        true,
    );
    assert!(matches!(
        view.admit(command(12, "finish")),
        Some(PillAction::Finish)
    ));
    for (revision, action) in [
        (11, "finish"),
        (12, "paste"),
        (12, "record"),
        (12, "select-window"),
        (12, "switch-iphone"),
    ] {
        assert!(view.admit(command(revision, action)).is_none());
    }
    let value = serde_json::json!({"revision":12,"action":"finish","path":"/tmp/foreign"});
    assert!(serde_json::from_value::<Command>(value).is_err());
}
#[test]
fn electron_pill_preserves_capabilities_and_one_primary_action() {
    let states = vec![
        Pres::Hidden,
        Pres::Selecting {
            mode: SelectionMode::Area,
            has_iphone: false,
        },
        Pres::Selecting {
            mode: SelectionMode::Screen,
            has_iphone: true,
        },
        Pres::Recording {
            chips: false,
            has_iphone: false,
        },
        Pres::Recording {
            chips: true,
            has_iphone: true,
        },
        Pres::RecordingWarning {
            chips: false,
            has_iphone: false,
        },
        Pres::Processing,
        Pres::Completed {
            label: "Saved".into(),
            routes: vec![],
            warning: false,
            share: Some(false),
        },
    ];
    for state in states {
        let view = View::new(&snapshot(state), 1, true);
        assert!(view.actions.iter().filter(|a| a.primary).count() <= 1);
        assert!(!view.actions.iter().any(|a| a.id.contains("window")));
        if view.phase != "recording" {
            assert!(!view.draggable);
        }
        let json = serde_json::to_string(&view).unwrap();
        for private in ["bundle_path", "capture_id", "config", "credential"] {
            assert!(!json.contains(private));
        }
    }
}
#[test]
fn electron_pill_warning_keeps_finish_without_competing_accent() {
    let view = View::new(
        &snapshot(Pres::RecordingWarning {
            chips: false,
            has_iphone: false,
        }),
        1,
        true,
    );
    assert_eq!(view.warning.as_deref(), Some("No words recognized yet"));
    assert!(
        view.actions
            .iter()
            .any(|a| a.id == "finish" && a.enabled && !a.primary)
    );
    let view = View::new(
        &snapshot(Pres::Completed {
            label: "Saved".into(),
            routes: vec![],
            warning: false,
            share: Some(false),
        }),
        1,
        false,
    );
    assert!(view.admit(command(1, "copy-link")).is_none());
}

#[test]
#[ignore = "requires the built Electron frontend and an isolated Xvfb/disposable desktop"]
fn electron_pill_real_pipe_roundtrip() {
    use std::{os::unix::fs::PermissionsExt, process::Command as Process};
    let directory = std::env::temp_dir().join(format!(
        "pill-pipe-{}",
        crate::capture::desktop::workflow::agent_capture::new_id().unwrap()
    ));
    std::fs::create_dir(&directory).unwrap();
    std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700)).unwrap();
    let output = Process::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "capture::desktop::ui::electron_pill::tests::electron_pill_real_pipe_child",
            "--ignored",
            "--nocapture",
        ])
        .env("APPARENT_PILL_PIPE_CHILD", "1")
        .env("HOME", &directory)
        .env("XDG_CONFIG_HOME", directory.join(".config"))
        .env("XDG_CACHE_HOME", directory.join(".cache"))
        .env("XDG_DATA_HOME", directory.join(".local/share"))
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&directory);
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
#[ignore = "subprocess fixture; invoked only by electron_pill_real_pipe_roundtrip"]
fn electron_pill_real_pipe_child() {
    use std::time::{Duration, Instant};
    assert_eq!(
        std::env::var("APPARENT_PILL_PIPE_CHILD").as_deref(),
        Ok("1")
    );
    let view = View::new(
        &snapshot(Pres::Recording {
            chips: false,
            has_iphone: false,
        }),
        1,
        true,
    );
    let mut session = Session::open(view.clone(), 1, Default::default()).unwrap();
    // A legitimate placement may precede Electron's app/window readiness.
    session.place([100.0, 100.0]);
    let deadline = Instant::now() + Duration::from_secs(8);
    while !session.ready() && Instant::now() < deadline {
        session.update(view.clone(), 1, Default::default());
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        session.ready() && session.healthy(),
        "real Electron did not report readiness"
    );
    // Flood latest-state replacement around activation: control messages must survive.
    for _ in 0..2000 {
        session.update(view.clone(), 1, Default::default());
    }
    session.activate();
    for _ in 0..2000 {
        session.update(view.clone(), 1, Default::default());
    }
    std::thread::sleep(Duration::from_millis(1200));
    assert!(
        session.healthy(),
        "activation acknowledgement was lost under state load"
    );
    session.update(
        View::new(&snapshot(Pres::Hidden), 0, false),
        1,
        Default::default(),
    );
    assert!(
        session.actions().is_empty(),
        "rendering alone must never dispatch a capture action"
    );
}

#[test]
fn electron_pill_expired_command_returns_fresh_actionable_readback_without_dispatch() {
    let mut view = View::new(
        &snapshot(Pres::Recording {
            chips: false,
            has_iphone: false,
        }),
        9,
        true,
    );
    assert!(view.consume(command(9, "finish"), false).is_none());
    assert_eq!(view.revision, 10);
    assert!(view.notice.is_some());
    assert!(view.consume(command(9, "cancel"), true).is_none());
    assert_eq!(
        view.revision, 10,
        "old commands cannot consume a new capture's revision"
    );
    assert!(matches!(
        view.consume(command(10, "finish"), true),
        Some(PillAction::Finish)
    ));
    assert!(view.notice.is_none());
}

#[test]
fn electron_pill_requires_an_explicit_capture_for_handoff() {
    let view = View::new(
        &snapshot(Pres::Completed {
            label: "Saved".into(),
            routes: vec![],
            warning: false,
            share: Some(true),
        }),
        1,
        false,
    )
    .with_handoff(false, false);
    for action in ["paste", "copy-link", "inspect"] {
        assert!(view.admit(command(1, action)).is_none());
    }
    assert!(matches!(
        view.admit(command(1, "dismiss")),
        Some(PillAction::DismissCompleted)
    ));
}
