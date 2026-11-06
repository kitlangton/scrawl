    #[test]
    fn first_capture_close_resumes_until_success_without_trapping_the_user() {
        let mut progress = Progress::default();
        let mut flow = Flow::tour(Step::Shortcut, Some(grants(&Permission::REQUIRED)))
            .with_first_capture(true, false, false);
        assert_eq!(flow.on(Input::Closed), Effect::Close);
        progress.leave(&flow);
        assert!(!progress.seen);
        assert_eq!(progress.resume, Some(Step::Shortcut));
        flow.on(Input::ShortcutChosen);
        flow.on(Input::Continue);
        assert_eq!(flow.on(Input::Later), Effect::Close);
        progress.leave(&flow);
        assert!(!progress.seen);
        assert_eq!(progress.resume, Some(Step::TryIt));
        flow.on(Input::ShortcutTriggered);
        flow.on(Input::CaptureStarted);
        flow.on(Input::CaptureEnded { completed: true });
        progress.leave(&flow);
        assert!(progress.seen);
        assert_eq!(progress.resume, None);
    }

    #[test]
    fn first_capture_resume_rechecks_milestones_and_preserves_legacy_and_repairs() {
        let ready = Some(grants(&Permission::REQUIRED));
        let missing = Flow::tour(Step::TryIt, ready.clone()).with_first_capture(true, false, false);
        assert_eq!(missing.step(), Step::Shortcut);
        let unfinished = Flow::tour(Step::Done, ready.clone()).with_first_capture(true, true, false);
        assert_eq!(unfinished.step(), Step::TryIt);
        let completed = Flow::tour(Step::TryIt, ready.clone()).with_first_capture(true, true, true);
        assert_eq!(completed.step(), Step::Done);
        let legacy = Flow::tour(Step::Shortcut, ready.clone()).with_first_capture(false, false, false);
        let mut progress = Progress::default();
        progress.leave(&legacy);
        assert!(progress.seen, "the default flow still allows dismissal");
        let repair = Flow::fix(Permission::REQUIRED.to_vec(), ready).with_first_capture(true, false, false);
        assert!(!repair.view().first_capture_required);
        let before = progress.clone();
        progress.leave(&repair);
        assert_eq!(progress, before, "permission repair does not complete or reset a tour");
    }

    #[test]
    fn first_capture_requires_explicit_shortcut_choice() {
        let mut flow = Flow::tour(Step::Shortcut, Some(grants(&Permission::REQUIRED)))
            .with_first_capture(true, false, false);
        assert_eq!(flow.on(Input::Continue), Effect::None);
        assert_eq!(flow.step(), Step::Shortcut, "an existing shortcut is not a choice");
        flow.on(Input::ChangeShortcut);
        flow.on(Input::Continue);
        assert_eq!(flow.step(), Step::Shortcut, "opening the recorder is not a saved choice");
        flow.on(Input::ShortcutChosen);
        flow.on(Input::Continue);
        assert_eq!(flow.step(), Step::TryIt);
    }

    #[test]
    fn first_capture_requires_shortcut_start_and_successful_finish() {
        let mut flow = Flow::tour(Step::TryIt, Some(grants(&Permission::REQUIRED)))
            .with_first_capture(true, true, false);
        assert_eq!(flow.on(Input::Continue), Effect::None, "Try it cannot finish the tour");
        flow.on(Input::CaptureEnded { completed: true });
        assert_eq!(flow.step(), Step::TryIt, "a stale completion cannot count");
        flow.on(Input::CaptureStarted);
        flow.on(Input::CaptureEnded { completed: true });
        assert_eq!(flow.step(), Step::TryIt, "menu recordings do not prove the shortcut works");
        flow.on(Input::ShortcutTriggered);
        flow.on(Input::CaptureStarted);
        flow.on(Input::CaptureEnded { completed: false });
        assert_eq!(flow.step(), Step::TryIt, "cancelled captures do not count");
        flow.on(Input::CaptureStarted);
        flow.on(Input::CaptureEnded { completed: true });
        assert_eq!(flow.step(), Step::TryIt, "cancel clears the shortcut attempt");
        flow.on(Input::ShortcutTriggered);
        flow.on(Input::CaptureStarted);
        flow.on(Input::CaptureEnded { completed: true });
        assert_eq!(flow.step(), Step::Done);
    }

    use super::*;

    use Permission::{Accessibility, Camera, InputMonitoring, Microphone, ScreenRecording};

    const ALL: [Permission; 3] = [ScreenRecording, Microphone, Accessibility];

    fn grants(granted: &[Permission]) -> Grants {
        Grants {
            statuses: granted.iter().map(|p| (*p, Status::Granted)).collect(),
            screen_needs_relaunch: false,
        }
    }

    /// The helper says Screen Recording is granted, but this copy can't capture yet.
    fn stale(granted: &[Permission]) -> Grants {
        Grants {
            screen_needs_relaunch: true,
            ..grants(granted)
        }
    }

    fn tick(flow: &mut Flow, g: &Grants, now_ms: u64) -> Effect {
        flow.on(Input::Grants {
            grants: g.clone(),
            now_ms,
        })
    }

    fn rows(flow: &Flow) -> Vec<Permission> {
        flow.view().rows.iter().map(|r| r.permission).collect()
    }

    #[test]
    fn shortcut_step_edits_inline_and_continue_leaves_the_editor() {
        let mut flow = Flow::tour(Step::Welcome, Some(grants(&[])));
        assert_eq!(flow.view().count, 5);
        flow.on(Input::Continue);
        flow.on(Input::Continue);
        flow.on(Input::Grants {
            grants: grants(&ALL),
            now_ms: 0,
        });
        flow.on(Input::Continue);
        assert_eq!(flow.step(), Step::Shortcut);
        assert_eq!(flow.resume_point(), Some(Step::Shortcut));
        assert_eq!(flow.on(Input::ChangeShortcut), Effect::EditShortcut);
        assert_eq!(flow.step(), Step::Shortcut);
        assert!(flow.view().shortcut_editing);
        flow.on(Input::Continue);
        assert_eq!(flow.step(), Step::TryIt);
        assert!(!flow.view().shortcut_editing);
        flow.on(Input::Back);
        assert_eq!(flow.step(), Step::Shortcut);

        let mut fix = Flow::fix(vec![Permission::Microphone], Some(grants(&[])));
        assert_eq!(fix.on(Input::ChangeShortcut), Effect::None);
    }

    #[test]
    fn the_tour_has_a_shortcut_screen_with_every_permission_on_one() {
        let mut flow = Flow::tour(Step::Welcome, Some(grants(&[])));
        assert_eq!(flow.view().count, 5);
        assert_eq!(flow.on(Input::Continue), Effect::None);
        assert_eq!(flow.step(), Step::HowItWorks);
        assert_eq!(flow.on(Input::Continue), Effect::None);
        assert_eq!(flow.step(), Step::Permissions);
        assert_eq!(rows(&flow), Permission::REQUIRED);
        assert!(flow.view().rows.iter().all(|r| r.required));
        // Camera is asked for when used, never in the tour.
        assert!(!rows(&flow).contains(&Camera));
        // Continue does nothing until the required ones are on.
        assert!(!flow.view().ready);
        assert_eq!(flow.on(Input::Continue), Effect::None);
        assert_eq!(flow.step(), Step::Permissions);
        assert_eq!(flow.on(Input::Back), Effect::None);
        assert_eq!(flow.step(), Step::HowItWorks);
    }

    #[test]
    fn speech_recognition_is_an_optional_row_only_where_the_helper_reports_it() {
        use Permission::SpeechRecognition;
        // macOS 26: the helper doesn't report it, so it isn't on the screen or in Settings.
        assert!(!SpeechRecognition.asked(Some(&grants(&[]))));
        assert!(!SpeechRecognition.asked(None));
        let flow = Flow::tour(Step::Permissions, Some(grants(&[])));
        assert!(!rows(&flow).contains(&SpeechRecognition));

        // macOS 15: a fourth row after the required ones, asked for with Apple's prompt.
        let mut asking = grants(&[]);
        asking
            .statuses
            .insert(SpeechRecognition, Status::NotDetermined);
        let mut flow = Flow::tour(Step::Permissions, Some(asking));
        assert_eq!(
            rows(&flow),
            [
                ScreenRecording,
                Microphone,
                Accessibility,
                SpeechRecognition
            ]
        );
        let row = flow.view().rows[3].clone();
        assert!(!row.required);
        assert_eq!(
            flow.on(Input::Grant(SpeechRecognition)),
            Effect::Request(SpeechRecognition)
        );
        assert_eq!(
            flow.on(Input::Grant(SpeechRecognition)),
            Effect::OpenSettings(SpeechRecognition)
        );

        // It never holds up Continue.
        let mut required = grants(&ALL);
        required.statuses.insert(SpeechRecognition, Status::Denied);
        tick(&mut flow, &required, 0);
        assert!(flow.view().ready);
        assert_eq!(flow.on(Input::Continue), Effect::None);
        assert_eq!(flow.step(), Step::Shortcut);
        flow.on(Input::Continue);
        assert_eq!(flow.step(), Step::TryIt);
    }

    #[test]
    fn the_demo_can_be_skipped_and_adds_no_screen() {
        let mut flow = Flow::tour(Step::HowItWorks, Some(grants(&[])));
        assert_eq!(flow.view().count, 5);
        assert_eq!(flow.view().tutorial, None);
        // Skip goes straight on, like Continue, and is remembered.
        assert_eq!(flow.on(Input::Skip), Effect::None);
        assert_eq!(flow.step(), Step::Permissions);
        assert_eq!(flow.tutorial(), Some(Tutorial::Skipped));
        // Skip, Watched and Practiced mean nothing on another screen.
        assert_eq!(flow.on(Input::Skip), Effect::None);
        assert_eq!(flow.on(Input::Practiced), Effect::None);
        assert_eq!(flow.step(), Step::Permissions);
        assert_eq!(flow.tutorial(), Some(Tutorial::Skipped));
        // Back to it and watched through: the button reads Continue from then on.
        flow.on(Input::Back);
        assert_eq!(flow.step(), Step::HowItWorks);
        flow.on(Input::Watched);
        assert_eq!(flow.view().tutorial, Some(Tutorial::Watched));
        assert_eq!(flow.step(), Step::HowItWorks, "watching doesn't move on");
        // A mark drawn on the demo wins over watching, and watching again doesn't undo it.
        flow.on(Input::Practiced);
        flow.on(Input::Watched);
        assert_eq!(flow.tutorial(), Some(Tutorial::Practiced));
        // Skip after watching moves on without forgetting the mark.
        flow.on(Input::Skip);
        assert_eq!(flow.step(), Step::Permissions);
        assert_eq!(flow.tutorial(), Some(Tutorial::Practiced));
        // With every permission on, Skip passes the permissions screen too.
        let mut flow = Flow::tour(Step::HowItWorks, Some(grants(&ALL)));
        flow.on(Input::Skip);
        assert_eq!(flow.step(), Step::Shortcut);
        // A fix never shows the demo.
        let mut fix = Flow::fix(Permission::REQUIRED.to_vec(), Some(grants(&[])));
        assert_eq!(fix.on(Input::Skip), Effect::None);
        assert_eq!(fix.tutorial(), None);
    }

    #[test]
    fn the_demo_outcome_comes_back_after_a_quit() {
        let progress = Progress {
            resume: Some(Step::HowItWorks),
            tutorial: Some(Tutorial::Watched),
            ..Progress::default()
        };
        let flow = launch_plan(&progress, &grants(&[]))
            .expect("the tour")
            .with_tutorial(progress.tutorial);
        assert_eq!(flow.step(), Step::HowItWorks);
        assert_eq!(flow.view().tutorial, Some(Tutorial::Watched));
        // The view carries it to the helper as kebab-case, or null before anything happened.
        let json = serde_json::to_value(flow.view()).unwrap();
        assert_eq!(json["tutorial"], "watched");
        let fresh = serde_json::to_value(Flow::tour(Step::Welcome, None).view()).unwrap();
        assert!(fresh["tutorial"].is_null());
        // An older config without the key loads as nothing yet.
        let old: Progress = serde_json::from_str(r#"{"seen":false,"resume":"how-it-works"}"#).unwrap();
        assert_eq!(old.tutorial, None);
    }

    #[test]
    fn screen_recording_is_the_first_row_and_accessibility_the_third() {
        assert_eq!(Permission::REQUIRED, ALL);
        let flow = Flow::tour(Step::Permissions, Some(grants(&[])));
        assert_eq!(rows(&flow), ALL);
        let fix = Flow::fix(Permission::REQUIRED.to_vec(), Some(grants(&[])));
        assert_eq!(rows(&fix)[0], ScreenRecording);
        assert_eq!(rows(&fix)[2], Accessibility);
        assert!(Accessibility.required() && Accessibility.draggable());
    }

    #[test]
    fn input_monitoring_is_never_requested() {
        assert!(!InputMonitoring.required());
        assert!(!InputMonitoring.draggable());
        let mut flow = Flow::tour(Step::Permissions, Some(grants(&[])));
        assert!(!rows(&flow).contains(&InputMonitoring));
        assert_eq!(flow.on(Input::Grant(InputMonitoring)), Effect::None);
        assert!(missing_to_record(Some(&grants(&ALL)), true, true).is_empty());
        // Nothing in Apparent or its helper asks macOS for it.
        let sources = [
            include_str!("../../mod.rs"),
            include_str!("../../../engine/runtime/platform/macos.rs"),
            include_str!("../../ui/macos.rs"),
            include_str!("../onboarding.rs"),
            include_str!("../settings.rs"),
            include_str!("../mod.rs"),
            include_str!("../../native/Onboarding.swift"),
            include_str!("../../native/Settings.swift"),
            include_str!("../../native/PermissionPanel.swift"),
        ];
        let calls = [
            concat!("CGRequest", "ListenEventAccess"),
            concat!("CGPreflight", "ListenEventAccess"),
            concat!("IOHIDRequest", "Access"),
            concat!("IOHIDCheck", "Access"),
            concat!("kCGEventTapOption", "ListenOnly"),
        ];
        for source in sources {
            for call in calls {
                assert!(!source.contains(call), "{call} is still called");
            }
        }
        // A 0.3.x config that remembers it granted still loads.
        let old: Progress = serde_json::from_str(
            r#"{"seen":true,"granted":["screen-recording","microphone","input-monitoring"]}"#,
        )
        .unwrap();
        assert!(old.seen);
    }

    #[test]
    fn permissions_are_granted_in_any_order_then_continue_moves_on() {
        let mut flow = Flow::tour(Step::Permissions, Some(grants(&[])));
        // A list the app can be dragged into goes straight to System Settings and the panel.
        assert_eq!(
            flow.on(Input::Grant(Accessibility)),
            Effect::OpenSettings(Accessibility)
        );
        assert_eq!(
            flow.on(Input::Grant(Microphone)),
            Effect::Request(Microphone)
        );
        // A second press opens System Settings.
        assert_eq!(
            flow.on(Input::Grant(Microphone)),
            Effect::OpenSettings(Microphone)
        );
        let some = grants(&[Microphone, Accessibility]);
        tick(&mut flow, &some, 0);
        let view = flow.view();
        assert_eq!(view.rows[1].status, Status::Granted);
        assert!(!view.rows[0].requested && !view.ready);
        // No auto-advance in the tour, even long after.
        let all = grants(&ALL);
        tick(&mut flow, &all, 1);
        tick(&mut flow, &all, 10_000);
        assert_eq!(flow.step(), Step::Permissions);
        assert!(flow.view().ready);
        // A granted row's button does nothing.
        assert_eq!(flow.on(Input::Grant(Microphone)), Effect::None);
        assert_eq!(flow.on(Input::Continue), Effect::None);
        assert_eq!(flow.step(), Step::Shortcut);
        flow.on(Input::Continue);
        assert_eq!(flow.step(), Step::TryIt);
        // Back visits Shortcut, then passes the finished permissions screen.
        flow.on(Input::Back);
        assert_eq!(flow.step(), Step::Shortcut);
        flow.on(Input::Back);
        assert_eq!(flow.step(), Step::HowItWorks);
    }

    #[test]
    fn a_denied_permission_goes_straight_to_settings() {
        let mut denied = grants(&[]);
        denied.statuses.insert(Microphone, Status::Denied);
        let mut flow = Flow::tour(Step::Permissions, Some(denied));
        assert_eq!(
            flow.on(Input::Grant(Microphone)),
            Effect::OpenSettings(Microphone)
        );
    }

    #[test]
    fn granted_permissions_pass_the_screen_and_leave_the_dots() {
        let all = grants(&ALL);
        let mut flow = Flow::tour(Step::HowItWorks, Some(all.clone()));
        assert_eq!(flow.view().count, 4);
        flow.on(Input::Continue);
        assert_eq!(flow.step(), Step::Shortcut);
        flow.on(Input::Continue);
        assert_eq!(flow.step(), Step::TryIt);
        assert_eq!(flow.view().index, 3);
        // A tour resumed past the screen goes back to it while something is missing.
        let some = grants(&[Microphone]);
        assert_eq!(
            Flow::tour(Step::TryIt, Some(some)).step(),
            Step::Permissions
        );
        // Resumed on the screen itself, it stays there with every check showing.
        let resumed = Flow::tour(Step::Permissions, Some(all));
        assert_eq!(resumed.step(), Step::Permissions);
        assert!(resumed.view().ready);
    }

    #[test]
    fn a_screen_recording_grant_this_copy_cant_use_relaunches_by_itself_once() {
        let mut flow = Flow::tour(Step::Permissions, Some(grants(&[])));
        // The user drags Apparent into Screen Recording; macOS offers Quit & Reopen and the user
        // picks Later (or nothing): the helper says granted, this process still can't capture.
        let granted = stale(&[ScreenRecording]);
        assert_eq!(tick(&mut flow, &granted, 1_000), Effect::None);
        let row = flow.view().rows[0].clone();
        assert_eq!(row.status, Status::Granted);
        assert!(row.reopening && !row.relaunch);
        // Not ready: Continue can't skip past a grant that doesn't work yet.
        assert_eq!(flow.on(Input::Continue), Effect::None);
        // The check shows a moment, then Apparent relaunches, back on this screen, even though
        // the other two rows are still off.
        assert_eq!(tick(&mut flow, &granted, 1_500), Effect::None);
        let relaunch = Effect::Relaunch {
            resume: Some(Step::Permissions),
        };
        assert_eq!(tick(&mut flow, &granted, 1_000 + RELAUNCH_AFTER_MS), relaunch);
        // Never twice from one window.
        assert_eq!(tick(&mut flow, &granted, 9_000), Effect::None);
        assert_eq!(flow.on(Input::Grant(ScreenRecording)), Effect::None);
        // A grant this copy can use (ScreenCaptureKit works without a relaunch, or macOS's
        // Quit & Reopen already restarted it) relaunches nothing.
        let mut flow = Flow::tour(Step::Permissions, Some(grants(&[])));
        assert_eq!(
            tick(&mut flow, &grants(&[ScreenRecording]), 0),
            Effect::ConfirmScreen
        );
        for t in [5_000, 60_000] {
            assert_eq!(tick(&mut flow, &grants(&[ScreenRecording]), t), Effect::None);
        }
        assert!(!flow.view().rows[0].reopening);
    }

    #[test]
    fn once_screen_recording_works_the_window_picker_question_comes_up_explained() {
        let mut flow = Flow::tour(Step::Permissions, Some(grants(&[])));
        assert_eq!(tick(&mut flow, &grants(&[Microphone]), 0), Effect::None);
        assert!(!flow.view().picker_note);
        // Not while the grant needs a relaunch to work: listing the screen would fail.
        assert_eq!(
            tick(&mut flow, &stale(&[ScreenRecording]), 100),
            Effect::None
        );
        assert_eq!(
            tick(&mut flow, &grants(&[ScreenRecording]), 200),
            Effect::ConfirmScreen
        );
        assert!(flow.view().picker_note);
        assert_eq!(
            tick(&mut flow, &grants(&[ScreenRecording]), 300),
            Effect::None
        );
        // Answered by the time Try it shows: no note there.
        flow.on(Input::Grants {
            grants: grants(&Permission::REQUIRED),
            now_ms: 400,
        });
        flow.on(Input::Continue);
        assert_eq!(flow.step(), Step::Shortcut);
        flow.on(Input::Continue);
        assert_eq!(flow.step(), Step::TryIt);
        assert!(!flow.view().picker_note);
        // Everything already on: it comes up on Try it instead, before the first capture.
        let mut flow = Flow::tour(Step::TryIt, Some(grants(&Permission::REQUIRED)));
        assert_eq!(flow.step(), Step::TryIt);
        assert_eq!(
            tick(&mut flow, &grants(&Permission::REQUIRED), 0),
            Effect::ConfirmScreen
        );
        assert!(flow.view().picker_note);
        // Gone from the last screen.
        flow.on(Input::CaptureEnded { completed: true });
        assert_eq!(flow.step(), Step::Done);
        assert!(!flow.view().picker_note);
    }

    #[test]
    fn a_fix_relaunches_too_and_reopens_after() {
        let mut flow = Flow::fix(Permission::REQUIRED.to_vec(), Some(grants(&[Microphone])));
        let granted = stale(&[ScreenRecording, Microphone]);
        tick(&mut flow, &granted, 0);
        assert_eq!(
            tick(&mut flow, &granted, RELAUNCH_AFTER_MS),
            Effect::Relaunch { resume: None }
        );
        let mut progress = Progress {
            seen: true,
            ..Progress::default()
        };
        progress.relaunching(None, 42);
        assert!(progress.reopen_fix && progress.seen);
        let flow = launch_plan(&progress, &grants(&[ScreenRecording, Microphone])).expect("fix");
        assert_eq!((flow.mode(), flow.step()), (Mode::Fix, Step::Permissions));
        assert_eq!(rows(&flow), Permission::REQUIRED);
    }

    #[test]
    fn right_after_a_relaunch_it_never_relaunches_by_itself_again() {
        let now = 1_000_000;
        let mut progress = Progress::default();
        assert!(relaunch_allowed(&progress, now));
        progress.relaunching(Some(Step::Permissions), now);
        assert!(!relaunch_allowed(&progress, now + 5_000));
        assert!(!relaunch_allowed(&progress, now + RELAUNCH_GUARD_MS - 1));
        assert!(relaunch_allowed(&progress, now + RELAUNCH_GUARD_MS));
        // The fresh copy still can't capture (something else is wrong): no loop. The row offers
        // Reopen, and only pressing it relaunches.
        let mut flow = Flow::tour(Step::Permissions, Some(grants(&[]))).with_auto_relaunch(false);
        let granted = stale(&[ScreenRecording]);
        for t in [0, RELAUNCH_AFTER_MS, 60_000] {
            assert_eq!(tick(&mut flow, &granted, t), Effect::None);
        }
        let row = flow.view().rows[0].clone();
        assert!(row.relaunch && !row.reopening);
        assert_eq!(
            flow.on(Input::Grant(ScreenRecording)),
            Effect::Relaunch {
                resume: Some(Step::Permissions)
            }
        );
        assert_eq!(flow.on(Input::Grant(ScreenRecording)), Effect::None);
    }

    #[test]
    fn after_either_kind_of_relaunch_the_tour_resumes_on_the_permissions_screen() {
        // Apparent's own relaunch (the user picked Later, or nothing).
        let mut progress = Progress::default();
        progress.relaunching(Some(Step::Permissions), 7);
        assert_eq!(progress.resume, Some(Step::Permissions));
        assert!(!progress.seen);
        let fresh = grants(&[ScreenRecording]);
        let flow = launch_plan(&progress, &fresh).expect("the tour");
        assert_eq!((flow.mode(), flow.step()), (Mode::Tour, Step::Permissions));
        let view = flow.view();
        assert_eq!(view.rows[0].status, Status::Granted);
        assert!(!view.rows[0].reopening && !view.rows[0].relaunch);
        // The next row to do is Microphone.
        assert_eq!(
            view.rows.iter().find(|r| r.status != Status::Granted).map(|r| r.permission),
            Some(Microphone)
        );
        // macOS's Quit & Reopen: Apparent never noted a relaunch, but the step it saved while
        // the window was open brings it back to the same place.
        let progress = Progress {
            resume: Flow::tour(Step::Permissions, Some(grants(&[]))).resume_point(),
            ..Progress::default()
        };
        assert_eq!(progress.relaunched_at, None);
        let flow = launch_plan(&progress, &fresh).expect("the tour");
        assert_eq!((flow.mode(), flow.step()), (Mode::Tour, Step::Permissions));
        // Screen Recording was the last one: the screen shows every check and Continue goes on.
        let mut flow = launch_plan(&progress, &grants(&ALL)).expect("the tour");
        assert_eq!(flow.step(), Step::Permissions);
        assert!(flow.view().ready);
        flow.on(Input::Continue);
        assert_eq!(flow.step(), Step::Shortcut);
        flow.on(Input::Continue);
        assert_eq!(flow.step(), Step::TryIt);
    }

    #[test]
    fn a_tour_quit_midway_resumes_where_it_was() {
        let mut flow = Flow::tour(Step::Welcome, Some(grants(&[])));
        assert_eq!(flow.resume_point(), Some(Step::Welcome));
        flow.on(Input::Continue);
        flow.on(Input::Continue);
        assert_eq!(flow.resume_point(), Some(Step::Permissions));
        let progress = Progress {
            resume: flow.resume_point(),
            ..Progress::default()
        };
        let flow = launch_plan(&progress, &grants(&[ScreenRecording])).expect("the tour");
        assert_eq!(flow.step(), Step::Permissions);
        assert_eq!(
            Flow::fix(vec![Camera], None).resume_point(),
            None,
            "a fix has nothing to resume"
        );
    }

    #[test]
    fn set_up_later_and_closing_end_the_tour() {
        let mut flow = Flow::tour(Step::Permissions, Some(grants(&[])));
        assert_eq!(flow.on(Input::Later), Effect::Close);
        assert_eq!(flow.on(Input::Closed), Effect::Close);
    }

    #[test]
    fn the_shortcut_with_a_permission_off_starts_nothing_and_opens_the_window_on_release() {
        // Mic and Accessibility on, Screen Recording not usable in this process.
        let some = grants(&[Microphone, Accessibility]);
        let missing = missing_to_record(Some(&some), false, true);
        assert_eq!(missing, [ScreenRecording]);
        // Accessibility is read live in this process, not from the last helper read.
        assert_eq!(
            missing_to_record(Some(&grants(&ALL)), true, false),
            [Accessibility]
        );
        let mut gate = Gate::default();
        assert!(!gate.press(missing.clone()), "nothing starts");
        // Pressing again while it waits starts nothing either.
        assert!(!gate.press(Vec::new()));
        // Nothing shows while ⌥⇧ is held.
        assert_eq!(gate.tick(true), None);
        let asks = gate.tick(false).expect("the window, once the keys are up");
        assert_eq!(gate.tick(false), None);
        assert!(!gate.is_pending());
        // The window lists every required permission so the user sees what's left.
        let mut flow = Flow::fix(Permission::REQUIRED.to_vec(), Some(some.clone()));
        assert_eq!(asks, missing);
        assert_eq!(flow.step(), Step::Permissions);
        assert_eq!(
            flow.on(Input::Grant(ScreenRecording)),
            Effect::OpenSettings(ScreenRecording)
        );
        // With everything on, the shortcut goes through.
        assert!(missing_to_record(Some(&grants(&ALL)), true, true).is_empty());
        assert!(gate.press(Vec::new()));
        // A tour that's still explaining jumps to the permissions screen.
        let mut tour = Flow::tour(Step::Welcome, Some(some));
        assert_eq!(tour.on(Input::Blocked), Effect::Show);
        assert_eq!(tour.step(), Step::Permissions);
        // Unknown grants (the launch read failed): only what's read here blocks.
        assert!(missing_to_record(None, true, true).is_empty());
    }

    #[test]
    fn camera_is_asked_for_on_its_own_when_first_used() {
        let mut flow = Flow::fix(vec![Camera], Some(grants(&[ScreenRecording])));
        assert_eq!(rows(&flow), [Camera]);
        let view = flow.view();
        assert!(view.fix && !view.ready && !view.rows[0].required);
        assert_eq!(flow.on(Input::Grant(Camera)), Effect::Request(Camera));
        // A permission that isn't on this screen can't be asked for from it.
        assert_eq!(flow.on(Input::Grant(Microphone)), Effect::None);
        let camera = grants(&[Camera]);
        assert_eq!(tick(&mut flow, &camera, 100), Effect::None);
        assert!(flow.view().ready);
        assert_eq!(
            tick(&mut flow, &camera, 100 + AUTO_ADVANCE_MS),
            Effect::Close
        );
        // ⌥⇧V after Accessibility was turned off asks for just that row.
        let mut flow = Flow::fix(vec![Accessibility], Some(grants(&[])));
        assert_eq!(flow.on(Input::Later), Effect::Close);
        // A fix never steps back into the tour.
        assert_eq!(flow.on(Input::Back), Effect::None);
        assert_eq!(flow.step(), Step::Permissions);
    }

    #[test]
    fn a_capture_hides_the_window_and_finishing_from_try_it_shows_done() {
        let mut flow = Flow::tour(Step::TryIt, None);
        assert_eq!(flow.on(Input::CaptureStarted), Effect::Hide);
        assert_eq!(flow.on(Input::CaptureStarted), Effect::None);
        assert_eq!(
            flow.on(Input::CaptureEnded { completed: true }),
            Effect::Show
        );
        assert_eq!(flow.step(), Step::Done);
        assert_eq!(flow.on(Input::Settings), Effect::Settings);
        assert_eq!(flow.on(Input::Continue), Effect::Close);
        // A cancelled capture comes back to the same step.
        let mut flow = Flow::tour(Step::TryIt, None);
        flow.on(Input::CaptureStarted);
        assert_eq!(
            flow.on(Input::CaptureEnded { completed: false }),
            Effect::Show
        );
        assert_eq!(flow.step(), Step::TryIt);
    }

    #[test]
    fn launch_shows_the_tour_once_then_only_a_lost_permission() {
        let all = grants(&ALL);
        let mut progress = Progress::default();
        assert_eq!(
            launch_plan(&progress, &all).map(|f| (f.mode(), f.step())),
            Some((Mode::Tour, Step::Welcome))
        );
        progress.seen = true;
        assert!(progress.note(&all));
        assert!(!progress.note(&all));
        assert!(launch_plan(&progress, &all).is_none());
        // Accessibility went away: the permissions screen, as a fix.
        let lost = grants(&[ScreenRecording, Microphone]);
        let flow = launch_plan(&progress, &lost).expect("a fix");
        assert_eq!((flow.mode(), flow.step()), (Mode::Fix, Step::Permissions));
        assert_eq!(rows(&flow), Permission::REQUIRED);
        // Never granted (put off in the tour): no nagging at launch.
        progress.granted.remove(&Accessibility);
        assert!(launch_plan(&progress, &lost).is_none());
    }

    #[test]
    fn progress_is_kept_beside_the_rest_of_the_config() {
        let dir = std::env::temp_dir().join(format!("apparent-onboarding-{}", std::process::id()));
        let path = dir.join("config.json");
        std::fs::create_dir_all(&dir).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        std::fs::write(&path, r#"{"agentHost":"studio","autocrop":false}"#).unwrap();
        assert_eq!(Progress::load_from(&path), Progress::default());
        let progress = Progress {
            seen: true,
            resume: Some(Step::Permissions),
            granted: [Microphone].into_iter().collect(),
            relaunched_at: Some(1_792_000_000_000),
            reopen_fix: true,
            tutorial: Some(Tutorial::Practiced),
            shortcut_chosen: true,
            first_capture_completed: true,
        };
        progress.save_to(&path).unwrap();
        assert_eq!(Progress::load_from(&path), progress);
        let config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(config["agentHost"], "studio");
        assert_eq!(config["autocrop"], false);
        assert_eq!(config["onboarding"]["resume"], "permissions");
        assert_eq!(config["onboarding"]["relaunchedAt"], 1_792_000_000_000_u64);
        assert_eq!(config["onboarding"]["reopenFix"], true);
        assert_eq!(config["onboarding"]["tutorial"], "practiced");
        // A resume point saved by 0.3.2's one-permission-per-step tour still loads.
        std::fs::write(
            &path,
            r#"{"onboarding":{"seen":false,"resume":"microphone","granted":[]}}"#,
        )
        .unwrap();
        assert_eq!(Progress::load_from(&path).resume, Some(Step::Permissions));
        std::fs::write(&path, r#"{"onboarding":{"resume":"draw"}}"#).unwrap();
        assert_eq!(Progress::load_from(&path).resume, Some(Step::HowItWorks));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn helper_statuses_parse_into_grants() {
        let statuses: BTreeMap<Permission, Status> = serde_json::from_str(
            r#"{"screen-recording":"granted","microphone":"denied","camera":"not-determined"}"#,
        )
        .unwrap();
        let g = Grants {
            statuses,
            screen_needs_relaunch: false,
        };
        assert!(g.granted(ScreenRecording));
        assert_eq!(g.status(Microphone), Status::Denied);
        assert_eq!(g.status(Accessibility), Status::NotDetermined);
    }

    #[test]
    fn accessibility_matches_the_app_without_hiding_screen_revocation() {
        let helper = BTreeMap::from([
            (Accessibility, Status::NotDetermined),
            (ScreenRecording, Status::NotDetermined),
            (Microphone, Status::Granted),
        ]);
        let statuses = reconcile_accessibility(helper, true);
        assert_eq!(statuses[&Accessibility], Status::Granted);
        // A cached positive screen preflight is not evidence against a fresh denial.
        assert_eq!(statuses[&ScreenRecording], Status::NotDetermined);
        assert_eq!(statuses[&Microphone], Status::Granted);

        let helper = BTreeMap::from([
            (Accessibility, Status::Granted),
            (ScreenRecording, Status::Granted),
            (Microphone, Status::Granted),
        ]);
        let statuses = reconcile_accessibility(helper, false);
        assert_eq!(statuses[&Accessibility], Status::NotDetermined);
        // The helper may see a new screen grant before the running app does.
        assert_eq!(statuses[&ScreenRecording], Status::Granted);
        assert_eq!(statuses[&Microphone], Status::Granted);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn screen_revocation_blocks_recording_despite_a_cached_success() {
        let mut grants = Grants {
            statuses: BTreeMap::from([
                (Accessibility, Status::Granted),
                (ScreenRecording, Status::Granted),
                (Microphone, Status::Granted),
            ]),
            screen_needs_relaunch: false,
        };
        assert!(missing_to_record(Some(&grants), true, true).is_empty());
        for status in [Status::NotDetermined, Status::Denied] {
            grants.statuses.insert(ScreenRecording, status);
            assert_eq!(
                missing_to_record(Some(&grants), true, true),
                vec![ScreenRecording],
                "a fresh screen denial must override a cached successful probe"
            );
        }
        grants.statuses.insert(ScreenRecording, Status::Granted);
        assert!(missing_to_record(Some(&grants), true, true).is_empty());
        grants.screen_needs_relaunch = true;
        assert_eq!(
            missing_to_record(Some(&grants), true, true),
            vec![ScreenRecording]
        );
        assert_eq!(
            missing_to_record(Some(&grants), false, true),
            vec![ScreenRecording]
        );
    }

    #[test]
    fn app_permission_refresh_observes_revocation_without_a_window() {
        let mut reads = vec![
            Some(grants(&ALL)),
            None, // A failed helper read must not end the app-wide worker.
            Some(Grants {
                statuses: BTreeMap::from([
                    (Accessibility, Status::Granted),
                    (ScreenRecording, Status::Denied),
                    (Microphone, Status::Granted),
                ]),
                screen_needs_relaunch: false,
            }),
            Some(grants(&ALL)),
        ]
        .into_iter();
        let mut admission = Vec::new();
        let mut pauses = 0;
        // No Session/Settings exists here: the application owns the refresh and delivery.
        poll_grants(
            || reads.next().expect("the receiver must stop the worker"),
            |fresh| {
                admission.push(missing_to_record(Some(&fresh), true, true));
                admission.len() < 3
            },
            || pauses += 1,
        );
        assert_eq!(pauses, 3);
        assert_eq!(admission.len(), 3);
        assert!(admission[0].is_empty());
        if cfg!(target_os = "macos") {
            assert_eq!(admission[1], vec![ScreenRecording]);
        } else {
            // Linux admission remains portal-owned, even with a cached helper denial.
            assert!(admission[1].is_empty());
        }
        assert!(admission[2].is_empty());
    }
