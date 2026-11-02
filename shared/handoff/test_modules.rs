use super::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn image_offer_is_opt_in_and_falls_back_to_text_if_unavailable() {
        use std::cell::RefCell;
        let calls = RefCell::new(Vec::new());
        let offer = |enabled, image: Result<Vec<u8>>| {
            offer_image_handoff(
                enabled,
                || {
                    calls.borrow_mut().push("read".into());
                    image
                },
                "summary",
                |png, text| {
                    calls
                        .borrow_mut()
                        .push(format!("image:{}:{text}", png.len()));
                    Ok(true)
                },
                |text| {
                    calls.borrow_mut().push(format!("text:{text}"));
                    Ok(())
                },
            )
        };
        offer(false, Ok(vec![1, 2, 3])).unwrap();
        offer(true, Ok(vec![1, 2, 3])).unwrap();
        offer(true, Err(anyhow::anyhow!("missing sheet"))).unwrap();
        offer_image_handoff(
            true,
            || Ok(vec![1]),
            "summary",
            |_, _| {
                calls.borrow_mut().push("unsupported".into());
                Ok(false)
            },
            |text| {
                calls.borrow_mut().push(format!("text:{text}"));
                Ok(())
            },
        )
        .unwrap();
        let contended = offer_image_handoff(
            true,
            || Ok(vec![1]),
            "summary",
            |_, _| {
                calls.borrow_mut().push("ownership-lost".into());
                anyhow::bail!("clipboard changed after image publication")
            },
            |_| panic!("must not overwrite the new clipboard owner"),
        );
        assert!(contended.is_err());
        assert_eq!(
            *calls.borrow(),
            [
                "text:summary",
                "read",
                "image:3:summary",
                "read",
                "text:summary",
                "unsupported",
                "text:summary",
                "ownership-lost"
            ]
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_receiver_refuses_before_clipboard_or_paste() {
        let target = crate::capture::desktop::linux::hyprland::FocusedWindow {
            address: "0x123".into(),
            pid: 123,
            class: "t3code".into(),
        };
        let error = run_draft_handoff_at(
            b"image",
            "text",
            Path::new("contact-sheet.png"),
            false,
            &target,
            &mut LinuxDraftHandoff,
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("No verified attachment observer"));
        assert!(
            format!("{:#}", draft_receiver_preflight(&target).unwrap_err())
                .contains("No verified attachment observer")
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn draft_handoff_attaches_image_and_text_without_sending_and_restores_clipboard() {
        struct Draft {
            clipboard: Vec<u8>,
            draft_image: Option<Vec<u8>>,
            draft_text: String,
            pastes: usize,
            fail_at: Option<usize>,
            focus_lost_after_first: bool,
            focus: (u8, u8), // window and focused editor
            supported: bool,
            attachment_ready_after: usize,
            attachment_checks: usize,
            editable_checks: usize,
            editable_after: usize,
            clipboard_contended_after_first: bool,
        }
        impl DraftHandoffOps for Draft {
            type Snapshot = Vec<u8>;
            type Target = (u8, u8);
            fn target(&mut self) -> Result<(u8, u8)> {
                Ok(self.focus)
            }
            fn supported(&mut self, _: &(u8, u8)) -> Result<()> {
                ensure!(self.supported, "Unsupported composer");
                Ok(())
            }
            fn editor_ready(&mut self, _: &(u8, u8)) -> Result<bool> {
                self.editable_checks += 1;
                Ok(self.editable_checks >= self.editable_after)
            }
            fn still_ours(&mut self, _: &str, bytes: &[u8]) -> Result<bool> {
                Ok(self.clipboard == bytes)
            }
            fn attachment_visible(&mut self, _: &(u8, u8), image: &[u8]) -> Result<bool> {
                self.attachment_checks += 1;
                Ok(self.attachment_checks >= self.attachment_ready_after
                    && self.draft_image.as_deref() == Some(image))
            }
            fn text_visible(&mut self, _: &(u8, u8), text: &str) -> Result<bool> {
                Ok(self.draft_text == text)
            }
            fn snapshot(&mut self) -> Result<Vec<u8>> {
                Ok(self.clipboard.clone())
            }
            fn restore(&mut self, bytes: Vec<u8>, offered: Option<(&str, &[u8])>) -> Result<()> {
                if let Some((_, expected)) = offered {
                    ensure!(self.clipboard == expected, "clipboard changed");
                    self.clipboard = bytes;
                }
                Ok(())
            }
            fn image(&mut self, image: &[u8]) -> Result<()> {
                ensure!(
                    self.editable_checks >= self.editable_after,
                    "no editable field was selected"
                );
                self.clipboard = image.to_vec();
                Ok(())
            }
            fn text(&mut self, text: &str) -> Result<()> {
                self.clipboard = text.as_bytes().to_vec();
                Ok(())
            }
            fn paste(&mut self) -> Result<()> {
                self.pastes += 1;
                if self.fail_at == Some(self.pastes) {
                    bail!("paste refused");
                }
                if self.pastes == 1 {
                    self.draft_image = Some(self.clipboard.clone());
                    if self.focus_lost_after_first {
                        self.focus.1 = 2;
                    }
                    if self.clipboard_contended_after_first {
                        self.clipboard = b"other app".to_vec();
                    }
                } else {
                    self.draft_text = String::from_utf8(self.clipboard.clone())?;
                }
                Ok(())
            }
        }
        let mut image = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut image, 1, 1);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&[255, 0, 0, 255])
                .unwrap();
        }
        let mut receiver = Draft {
            clipboard: b"original clipboard".to_vec(),
            draft_image: None,
            draft_text: String::new(),
            pastes: 0,
            fail_at: None,
            focus_lost_after_first: false,
            focus: (1, 1),
            supported: true,
            attachment_ready_after: 3,
            attachment_checks: 0,
            editable_checks: 0,
            editable_after: 3,
            clipboard_contended_after_first: false,
        };
        run_draft_handoff(
            &image,
            "Synthetic transcript",
            Path::new("contact-sheet.png"),
            false,
            &mut receiver,
        )
        .unwrap();
        assert_eq!(receiver.draft_image.as_deref(), Some(image.as_slice()));
        assert_eq!(receiver.draft_text, "Synthetic transcript");
        assert_eq!(receiver.clipboard, b"original clipboard");
        assert_eq!(receiver.pastes, 2);
        assert_eq!(
            receiver.attachment_checks, 3,
            "confirmation retries must not repaste"
        );
        assert!(
            receiver.editable_checks >= 3,
            "wait for a focused writable field"
        );

        receiver.pastes = 0;
        receiver.supported = false;
        assert!(
            run_draft_handoff(
                &image,
                "Synthetic transcript",
                Path::new("contact-sheet.png"),
                false,
                &mut receiver
            )
            .is_err()
        );
        assert_eq!(receiver.pastes, 0);
        assert_eq!(receiver.clipboard, b"original clipboard");
        receiver.supported = true;

        receiver.pastes = 0;
        receiver.editable_checks = 0;
        receiver.editable_after = usize::MAX;
        let error = run_draft_handoff(
            &image,
            "Synthetic transcript",
            Path::new("contact-sheet.png"),
            false,
            &mut receiver,
        )
        .unwrap_err();
        assert!(format!("{error:#}").contains("No writable field selected"));
        assert_eq!(receiver.pastes, 0, "no field must mean no paste");
        assert_eq!(receiver.clipboard, b"original clipboard");
        receiver.editable_after = 3;

        receiver.pastes = 0;
        receiver.attachment_checks = 0;
        receiver.attachment_ready_after = 99;
        assert!(
            run_draft_handoff(
                &image,
                "Synthetic transcript",
                Path::new("contact-sheet.png"),
                false,
                &mut receiver
            )
            .is_err()
        );
        assert_eq!(receiver.pastes, 1, "missing attachment must not paste text");
        receiver.attachment_ready_after = 1;

        receiver.pastes = 0;
        receiver.fail_at = None;
        receiver.focus_lost_after_first = true;
        assert!(
            run_draft_handoff(
                &image,
                "Synthetic transcript",
                Path::new("contact-sheet.png"),
                false,
                &mut receiver
            )
            .is_err()
        );
        assert_eq!(
            receiver.pastes, 1,
            "focus loss must prevent the second paste"
        );
        receiver.focus = (1, 1);
        receiver.focus_lost_after_first = false;

        receiver.pastes = 0;
        receiver.clipboard_contended_after_first = true;
        assert!(
            run_draft_handoff(
                &image,
                "Synthetic transcript",
                Path::new("contact-sheet.png"),
                false,
                &mut receiver
            )
            .is_err()
        );
        assert_eq!(
            receiver.pastes, 1,
            "clipboard contention must prevent text paste"
        );
        assert_eq!(receiver.clipboard, b"other app");
        receiver.clipboard_contended_after_first = false;
        receiver.clipboard = b"original clipboard".to_vec();

        receiver.pastes = 0;
        receiver.fail_at = Some(1);
        assert!(
            run_draft_handoff(
                &image,
                "Synthetic transcript",
                Path::new("contact-sheet.png"),
                false,
                &mut receiver
            )
            .is_err()
        );
        assert_eq!(
            receiver.pastes, 1,
            "refused first paste must not advance to text"
        );
        assert_eq!(receiver.clipboard, b"original clipboard");

        receiver.pastes = 0;
        receiver.fail_at = Some(2);
        assert!(
            run_draft_handoff(
                &image,
                "Synthetic transcript",
                Path::new("contact-sheet.png"),
                false,
                &mut receiver
            )
            .is_err()
        );
        assert_eq!(receiver.clipboard, b"original clipboard");
        assert_eq!(receiver.pastes, 2);

        receiver.pastes = 0;
        receiver.focus = (2, 1);
        assert!(
            run_draft_handoff_at(
                &image,
                "Synthetic transcript",
                Path::new("contact-sheet.png"),
                false,
                &(1, 1),
                &mut receiver,
            )
            .is_err()
        );
        assert_eq!(
            receiver.pastes, 0,
            "a changed window must not receive the first paste"
        );
        receiver.focus = (1, 1);
    }

    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static COUNTER: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "apparent-handoff-{}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn bundle(&self, name: &str, created: u64) -> PathBuf {
            let path = self.0.join(format!("{name}.apparent"));
            fs::create_dir_all(path.join("frames")).unwrap();
            let keyframes: Vec<_> = [0, 1_900, 2_500, 4_000, 5_100, 6_000].into_iter().enumerate().map(|(index, t)| {
                let file = format!("frames/frame-{index:03}.png");
                fs::write(path.join(&file), b"fixture").unwrap();
                serde_json::json!({"index": index, "t": t, "reason": "initial", "file": file, "event": ""})
            }).collect();
            let bundle = serde_json::json!({
                "version": 1, "id": name, "createdAtMs": created, "durationMs": 7_000,
                "target": {"mode": "display", "displayId": 1,
                    "frame": {"x": 0.0, "y": 0.0, "width": 100.0, "height": 100.0, "scale": 1.0}},
                "transcript": {"source": "none", "text": "", "segments": []},
                "strokes": [], "marks": [], "events": [], "keyframes": keyframes,
                "contactSheet": "contact-sheet.png"
            });
            fs::write(
                path.join("apparent.json"),
                serde_json::to_vec(&bundle).unwrap(),
            )
            .unwrap();
            path
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn session_prompt_attaches_visuals_only_after_a_recorded_click() {
        let fixture = Fixture::new();
        let dir = fixture.bundle("speech", 100);
        let (_, mut bundle) = load_bundle(&dir).unwrap();
        bundle.events.push(crate::shared::model::InteractionEvent::Drag {
            t: 500, t1: 800, x0: 0.1, y0: 0.1, x1: 0.9, y1: 0.9,
            label: None, button: None, path: vec![],
        });
        let text_only = session_prompt(&dir, &bundle);
        assert_eq!(text_only["files"], serde_json::json!([]));
        assert!(text_only["text"].as_str().unwrap().contains("apparent zoom"));
        assert!(!text_only["text"].as_str().unwrap().contains(crate::capture::engine::sketch::LOOK_AT_IMAGES));
        assert!(!text_only["text"].as_str().unwrap().contains("contact sheet is attached"));
        assert!(dir.join("frames/frame-000.png").exists());

        bundle.events.push(crate::shared::model::InteractionEvent::Click {
            t: 1_000, x: 0.5, y: 0.5, button: None, label: None, role: None,
        });
        let with_click = session_prompt(&dir, &bundle);
        assert_eq!(with_click["files"][0]["name"], "contact-sheet.png");
        assert!(with_click["text"].as_str().unwrap().contains("contact sheet is attached"));
    }

    #[test]
    fn resolves_directory_manifest_and_newest_capture() {
        let fixture = Fixture::new();
        let older = fixture.bundle("old", 100);
        let newer = fixture.bundle("new", 200);
        assert_eq!(
            resolve_bundle_dir(&older).unwrap(),
            plain_path(older.canonicalize().unwrap())
        );
        assert_eq!(
            resolve_bundle_dir(&older.join("apparent.json")).unwrap(),
            plain_path(older.canonicalize().unwrap())
        );
        assert_eq!(
            newest_bundle(&fixture.0).unwrap(),
            plain_path(newer.canonicalize().unwrap())
        );
        assert!(resolve_bundle_dir(&fixture.0).is_err());
        assert!(resolve_bundle_dir(&fixture.0.join("missing.apparent")).is_err());
    }

    #[test]
    fn latest_skips_cancelled_and_partial_captures() {
        let fixture = Fixture::new();
        let cancelled = fixture.0.join("cancelled.apparent");
        fs::create_dir(&cancelled).unwrap();
        fs::write(cancelled.join("audio.wav"), b"partial recording").unwrap();
        let partial = fixture.bundle("partial", 300);
        fs::write(partial.join("apparent.json"), b"{\"version\":").unwrap();
        assert!(newest_bundle(&fixture.0).is_err());

        let completed = fixture.bundle("completed", 100);
        assert_eq!(
            newest_bundle(&fixture.0).unwrap(),
            plain_path(completed.canonicalize().unwrap())
        );
    }

    #[test]
    fn capture_history_is_newest_first_and_skips_incomplete_bundles() {
        let fixture = Fixture::new();
        let older = fixture.bundle("older", 100);
        let newer = fixture.bundle("newer", 300);
        let same_time = fixture.bundle("same-time", 300);
        let cancelled = fixture.0.join("cancelled.apparent");
        fs::create_dir(&cancelled).unwrap();
        let broken = fixture.bundle("broken", 400);
        fs::write(broken.join("apparent.json"), b"invalid").unwrap();
        let history = capture_history(&fixture.0).unwrap();
        assert_eq!(history.len(), 3);
        assert_eq!(history[0], plain_path(same_time.canonicalize().unwrap()));
        assert_eq!(history[1], plain_path(newer.canonicalize().unwrap()));
        assert_eq!(history[2], plain_path(older.canonicalize().unwrap()));
    }

    #[test]
    fn capture_history_reflects_new_and_removed_bundles() {
        let fixture = Fixture::new();
        let older = fixture.bundle("older", 100);
        assert_eq!(capture_history(&fixture.0).unwrap().len(), 1);

        let newer = fixture.bundle("newer", 200);
        let history = capture_history(&fixture.0).unwrap();
        assert_eq!(history[0], plain_path(newer.canonicalize().unwrap()));
        assert_eq!(history.len(), 2);

        fs::remove_dir_all(&newer).unwrap();
        assert_eq!(
            capture_history(&fixture.0).unwrap(),
            vec![plain_path(older.canonicalize().unwrap())]
        );
    }

    #[test]
    fn file_uris_are_percent_encoded() {
        assert_eq!(
            file_uri(Path::new(
                "/Users/kit/Library/Application Support/apparent/captures/a b#1.apparent/contact-sheet.png"
            )),
            "file:///Users/kit/Library/Application%20Support/apparent/captures/a%20b%231.apparent/contact-sheet.png"
        );
    }

    #[test]
    fn loads_manifest_and_reports_invalid_json() {
        let fixture = Fixture::new();
        let path = fixture.bundle("example", 123);
        let (resolved, bundle) = load_bundle(&path.join("apparent.json")).unwrap();
        assert_eq!(resolved, plain_path(path.canonicalize().unwrap()));
        assert_eq!(bundle.id, "example");
        assert_eq!(bundle.created_at_ms, 123);
        fs::write(path.join("apparent.json"), b"not json").unwrap();
        assert!(load_bundle(&path).is_err());
    }

    #[test]
    fn frame_fallback_uses_manifest_times_not_file_indices() {
        let fixture = Fixture::new();
        let path = fixture.bundle("example", 123);
        let frames = extract_frames_range(&path, 2_000, 5_000, 4).unwrap();
        let names: Vec<_> = frames
            .iter()
            .map(|frame| frame.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(
            names,
            vec![
                "frame-001.png",
                "frame-002.png",
                "frame-003.png",
                "frame-004.png"
            ]
        );
        assert!(
            extract_frames_range(&path, 10_000, 11_000, 4)
                .unwrap()
                .is_empty()
        );
        assert!(extract_frames_range(&path, 5_000, 2_000, 4).is_err());
        assert!(extract_frames_range(&path, 0, 2_000, 0).is_err());
    }

    #[test]
    fn recordings_go_to_their_addressed_session_not_the_viewed_one() {
        let dir = std::env::temp_dir().join(format!("apparent-dest-{}", crate::shared::ids::make_bundle_id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("timeline.json"),
            r#"{"session":"ses_ios","context":{"session":"ses_motion"}}"#,
        )
        .unwrap();
        let addressed = recorded_destination(&dir);
        assert_eq!(addressed.as_deref(), Some("ses_ios"));
        let addressed = addressed.as_deref();
        assert_eq!(
            choose_destination(None, addressed, false).unwrap(),
            Some("ses_ios")
        );
        assert!(choose_destination(Some("ses_motion"), addressed, false).is_err());
        assert_eq!(
            choose_destination(Some("ses_motion"), addressed, true).unwrap(),
            Some("ses_motion")
        );
        assert_eq!(
            choose_destination(Some("ses_ios"), addressed, false).unwrap(),
            Some("ses_ios")
        );
        assert_eq!(
            choose_destination(Some("ses_x"), None, false).unwrap(),
            Some("ses_x")
        );
        fs::remove_dir_all(&dir).unwrap();
        assert_eq!(recorded_destination(&dir), None);
    }

    #[test]
    fn apple_locale_round_trips_and_keeps_other_keys() {
        let fixture = Fixture::new();
        let path = fixture.0.join("config.json");
        fs::write(&path, br#"{"ingestToken":"abc","autoSend":false}"#).unwrap();
        let config = load_config_from(&path);
        assert_eq!(apple_locale_from(&config), None);
        assert_eq!(config.auto_send, Some(false));

        for (value, expected) in [
            ("zh-TW".into(), Some("zh-TW")),
            ("bad value".into(), None),
            (5.into(), None),
            (serde_json::Value::Null, None),
        ] {
            set_config_value_in(&path, "appleLocale", value).unwrap();
            let config = load_config_from(&path);
            assert_eq!(apple_locale_from(&config).as_deref(), expected);
            assert_eq!(serde_json::from_slice::<serde_json::Value>(&fs::read(&path).unwrap()).unwrap()["ingestToken"], "abc");
            assert_eq!(config.auto_send, Some(false));
        }
        let raw: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert!(raw.get("appleLocale").is_none());
    }

    #[test]
    fn transcriber_preference_round_trips_and_keeps_other_keys() {
        let fixture = Fixture::new();
        let path = fixture.0.join("config.json");
        fs::write(&path, br#"{"ingestToken":"abc","autoSend":false}"#).unwrap();
        let config = load_config_from(&path);
        assert_eq!((&config.transcriber, &config.hex_model), (&None, &None));
        assert_eq!(hex_language_from(&config), "en");
        set_config_value_in(&path, "transcriber", "hex".into()).unwrap();
        set_config_value_in(&path, "hexModel", "parakeet_v3".into()).unwrap();
        set_config_value_in(&path, "hexLanguage", "nl".into()).unwrap();
        let config = load_config_from(&path);
        assert_eq!(config.transcriber.as_deref(), Some("hex"));
        assert_eq!(config.hex_model.as_deref(), Some("parakeet_v3"));
        assert_eq!(hex_language_from(&config), "nl");
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&fs::read(&path).unwrap()).unwrap()["ingestToken"], "abc");
        assert_eq!(config.auto_send, Some(false));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }

    #[test]
    fn malformed_command_keys_never_reset_the_rest_of_the_config() {
        let fixture = Fixture::new();
        let path = fixture.0.join("config.json");
        fs::write(
            &path,
            br#"{"transcriber":"command","autoSend":false,"transcribeCommand":"whisper {audio}","transcribeTimeoutSeconds":"soon"}"#,
        )
        .unwrap();
        let config = load_config_from(&path);
        assert_eq!(config.transcriber.as_deref(), Some("command"));
        assert_eq!(config.auto_send, Some(false));
    }

    #[test]
    fn auto_send_defaults_on_and_the_toggle_keeps_other_keys() {
        let fixture = Fixture::new();
        let path = fixture.0.join("config.json");
        fs::write(
            &path,
            br#"{"ingestToken":"abc","retention":{"mediaDays":7}}"#,
        )
        .unwrap();
        assert_eq!(load_config_from(&path).auto_send, None);
        set_config_value_in(&path, "autoSend", serde_json::Value::Bool(false)).unwrap();
        let config = load_config_from(&path);
        assert_eq!(config.auto_send, Some(false));
        assert_eq!(serde_json::from_slice::<serde_json::Value>(&fs::read(&path).unwrap()).unwrap()["ingestToken"], "abc");
        let raw: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["retention"]["mediaDays"], 7);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn setting_a_key_never_replaces_an_invalid_existing_config() {
        let fixture = Fixture::new();
        let path = fixture.0.join("config.json");
        for bytes in [b"{\"ingestToken\":\"private\",".as_slice(), b"[]".as_slice()] {
            fs::write(&path, bytes).unwrap();
            assert!(set_config_value_in(&path, "autoSend", false.into()).is_err());
            assert_eq!(fs::read(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn warm_recognizer_defaults_on_and_config_can_disable_it() {
        let fixture = Fixture::new();
        let path = fixture.0.join("config.json");
        assert!(load_config_from(&path).warm_recognizer_enabled());
        fs::write(&path, br#"{"unrelated":"kept","autoSend":false}"#).unwrap();
        assert!(load_config_from(&path).warm_recognizer_enabled());
        set_config_value_in(&path, "warmRecognizer", false.into()).unwrap();
        let config = load_config_from(&path);
        assert!(!config.warm_recognizer_enabled());
        assert_eq!(config.auto_send, Some(false));
        let raw: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["unrelated"], "kept");
        set_config_value_in(&path, "warmRecognizer", true.into()).unwrap();
        assert!(load_config_from(&path).warm_recognizer_enabled());
        // An invalid value drops only this key, leaving the default and other settings.
        fs::write(&path, br#"{"warmRecognizer":"off","autoSend":false}"#).unwrap();
        let config = load_config_from(&path);
        assert!(config.warm_recognizer_enabled());
        assert_eq!(config.auto_send, Some(false));
        // The retired experiment cannot override an explicit top-level opt-out.
        fs::write(&path, br#"{"warmRecognizer":false,"experiments":{"warmRecognizer":true}}"#).unwrap();
        assert!(!load_config_from(&path).warm_recognizer_enabled());
    }

    #[test]
    fn video_export_defaults_on_and_preserves_explicit_opt_out() {
        let fixture = Fixture::new();
        let path = fixture.0.join("config.json");
        assert!(load_config_from(&path).video_export_enabled());
        fs::write(&path, br#"{"autoSend":false}"#).unwrap();
        assert!(load_config_from(&path).video_export_enabled());
        update_config_in(&path, |config| {
            config["videoExport"] = false.into();
            Ok(())
        }).unwrap();
        assert!(!load_config_from(&path).video_export_enabled());
        assert_eq!(load_config_from(&path).auto_send, Some(false));
        fs::write(&path, br#"{"videoExport":true}"#).unwrap();
        assert!(load_config_from(&path).video_export_enabled());
    }

    #[test]
    fn promoted_features_default_on_without_activating_other_experiments() {
        let fixture = Fixture::new();
        let path = fixture.0.join("config.json");
        let config = load_config_from(&path);
        for name in ["nativeLaunch", "movableRecordingControls", "exportNarration"] {
            assert!(experiment_enabled_in(&config, name), "{name}");
            set_experiment_in(&path, name, false).unwrap();
            assert!(!experiment_enabled_in(&load_config_from(&path), name), "{name}");
            set_experiment_in(&path, name, true).unwrap();
            assert!(experiment_enabled_in(&load_config_from(&path), name), "{name}");
        }
        for name in ["hostedSync", "hostedSharing", "hostedEncryption", "clipboardImage", "autoPasteDraft", "unknownFutureFeature"] {
            assert!(!experiment_enabled_in(&config, name), "{name}");
        }
        assert!(config.cloud.is_none());
    }

    #[test]
    fn sessions_read_their_directory_from_v2_location_or_v1_field() {
        let v2: OpenCodeSession = serde_json::from_str(
            r#"{"id":"ses_a","title":"A","time":{"updated":1},"location":{"directory":"/p/a"}}"#,
        )
        .unwrap();
        assert_eq!(v2.directory(), Some("/p/a"));
        assert!(v2.is_open_root());
        let v1: OpenCodeSession =
            serde_json::from_str(r#"{"id":"ses_b","parentID":"ses_a","directory":"/p/b"}"#)
                .unwrap();
        assert_eq!(v1.directory(), Some("/p/b"));
        assert!(!v1.is_open_root());
        assert_eq!(v1.label(), "ses_b");
    }

    #[test]
    fn config_defaults_when_missing_or_minimal() {
        let fixture = Fixture::new();
        let missing = fixture.0.join("missing.json");
        let cfg = load_config_from(&missing);
        assert_eq!(cfg, Config::default());

    }

    #[test]
    fn an_unusable_captures_folder_is_refused() {
        let root = std::env::temp_dir().join(format!("apparent-usable-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let file = root.join("file");
        fs::write(&file, "x").unwrap();
        assert!(!usable_captures_dir(&file.join("inside")));
        assert!(usable_captures_dir(&root.join("new folder")));
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn captures_dir_takes_absolute_and_home_paths_only() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(configured_captures_dir("~"), Some(home.clone()));
        assert_eq!(
            configured_captures_dir(" ~/Apparent Captures "),
            Some(home.join("Apparent Captures"))
        );
        let absolute = std::env::temp_dir().join("captures");
        assert_eq!(
            configured_captures_dir(&absolute.display().to_string()),
            Some(absolute)
        );
        assert_eq!(configured_captures_dir("captures"), None);
        assert_eq!(configured_captures_dir("~someone/captures"), None);
        assert_eq!(configured_captures_dir("  "), None);
    }

    #[test]
    fn one_bad_setting_never_resets_the_others() {
        let fixture = Fixture::new();
        let path = fixture.0.join("config.json");
        fs::write(
            &path,
            r#"{"agentHost": "studio", "autoSend": "maybe", "usbApps": "not-a-list", "videoExport": true, "nativePill": 3, "pillStyle": "glass", "unknown": {"x": 1}}"#,
        )
        .unwrap();
        let cfg = load_config_from(&path);
        assert_eq!(cfg.agent_host.as_deref(), Some("studio"));
        assert_eq!(cfg.video_export, Some(true));
        assert_eq!(cfg.pill_style.as_deref(), Some("glass"));
        assert_eq!(cfg.auto_send, None);
        assert_eq!(cfg.native_pill, None);
    }

    #[test]
    fn experiments_are_read_from_config_and_off_by_default() {
        let fixture = Fixture::new();
        let path = fixture.0.join("config.json");
        fs::write(
            &path,
            r#"{"experiments": {"agentContext": true, "other": false}}"#,
        )
        .unwrap();
        let cfg = load_config_from(&path);
        assert_eq!(cfg.experiments.get(AGENT_CONTEXT), Some(&true));
        assert_eq!(cfg.experiments.get("other"), Some(&false));
        assert!(Config::default().experiments.is_empty());
        // A bad value skips that experiment, never the file.
        fs::write(
            &path,
            r#"{"autoSend": false, "experiments": {"agentContext": "yes", "a": 1, "b": "maybe", "c": [1], "d": "off"}}"#,
        )
        .unwrap();
        let cfg = load_config_from(&path);
        assert_eq!(cfg.auto_send, Some(false));
        assert_eq!(cfg.experiments.get(AGENT_CONTEXT), Some(&true));
        assert_eq!(cfg.experiments.get("a"), Some(&true));
        assert_eq!(cfg.experiments.get("d"), Some(&false));
        assert!(!cfg.experiments.contains_key("b") && !cfg.experiments.contains_key("c"));
        fs::write(&path, r#"{"autoSend": false, "experiments": true}"#).unwrap();
        let cfg = load_config_from(&path);
        assert_eq!(cfg.auto_send, Some(false));
        assert!(cfg.experiments.is_empty());
        // Tests never read Kit's config, only what this process turned on.
        assert!(!experiment("neverEnabled"));
        enable_experiment("enabledForThisTest");
        assert!(experiment("enabledForThisTest"));
    }

    #[test]
    fn connect_settings_defaults_on_without_enabling_uploads_and_preserves_opt_out() {
        let fixture = Fixture::new();
        let path = fixture.0.join("config.json");
        let missing = load_config_from(&path);
        assert!(experiment_enabled_in(&missing, "connectSettings"));
        assert!(!experiment_enabled_in(&missing, "hostedSync"));
        assert!(!experiment_enabled_in(&missing, "hostedSharing"));
        assert!(missing.cloud.is_none());
        fs::write(&path, r#"{"autoSend":false,"experiments":{"other":true}}"#).unwrap();
        assert!(experiment_enabled_in(&load_config_from(&path), "connectSettings"));
        set_experiment_in(&path, "connectSettings", false).unwrap();
        let opted_out = load_config_from(&path);
        assert!(!experiment_enabled_in(&opted_out, "connectSettings"));
        assert_eq!(opted_out.auto_send, Some(false));
        assert_eq!(opted_out.experiments.get("other"), Some(&true));
        assert!(opted_out.cloud.is_none());
    }

    #[test]
    fn copy_capture_id_defaults_on_and_preserves_explicit_opt_out() {
        let fixture = Fixture::new();
        let path = fixture.0.join("config.json");
        assert!(experiment_enabled_in(
            &load_config_from(&path),
            "copyCaptureId"
        ));
        fs::write(&path, r#"{"autoSend":false,"experiments":{"other":true}}"#).unwrap();
        let cfg = load_config_from(&path);
        assert!(experiment_enabled_in(&cfg, "copyCaptureId"));
        assert!(!experiment_enabled_in(&cfg, "unrelatedNewExperiment"));
        set_experiment_in(&path, "copyCaptureId", false).unwrap();
        let cfg = load_config_from(&path);
        assert!(!experiment_enabled_in(&cfg, "copyCaptureId"));
        assert_eq!(cfg.auto_send, Some(false));
        assert_eq!(cfg.experiments.get("other"), Some(&true));
        set_experiment_in(&path, "copyCaptureId", true).unwrap();
        assert!(experiment_enabled_in(
            &load_config_from(&path),
            "copyCaptureId"
        ));
    }

    #[test]
    fn clipboard_templates_put_access_first_and_transcript_last() {
        let fixture = Fixture::new();
        let directory = fixture.bundle("a path's capture", 100);
        let (_, mut bundle) = load_bundle(&directory).unwrap();
        bundle.keyframes[0].said = Some("Please change this control".into());
        bundle.keyframes.last_mut().unwrap().said = Some("This is the last instruction".into());
        bundle.note = Some("Keep this note".into());
        bundle.image_quality_warning = Some("Keep this warning".into());
        bundle.autocrop = Some(crate::shared::model::AutocropInfo {
            label: "the activity".into(),
            full_frames: "frames-full".into(),
            ..Default::default()
        });
        let archive = crate::capture::engine::sketch::summary_markdown(&bundle, &directory);
        let commands = "apparent fetch 0123456789abcdef0123456789abcdef --out ./0123456789abcdef0123456789abcdef.apparent --wait 120\napparent inspect ./0123456789abcdef0123456789abcdef.apparent\n";
        for hosted in [None, Some(commands)] {
            for video in [false, true] {
                let prompt = clipboard_summary_with_access(&bundle, &directory, video, hosted);
                assert!(prompt.starts_with(hosted.unwrap_or("apparent inspect ")));
                assert!(prompt.ends_with("“This is the last instruction”\n"));
                assert!(prompt.find(CLI_INSTALL).unwrap() < prompt.find("[apparent ").unwrap());
                assert_eq!(prompt.contains(DEMO_VIDEO_REQUEST), video);
                if video {
                    assert!(
                        prompt.find(DEMO_VIDEO_REQUEST).unwrap()
                            < prompt.find("[apparent ").unwrap()
                    );
                }
                assert!(prompt.contains("Keep this note") && prompt.contains("Keep this warning"));
                assert!(prompt.contains("#0 (0:00) “Please change this control”"));
                assert!(!prompt.contains("connect login") && !prompt.contains("Cropped to:"));
                assert!(!prompt.contains(crate::capture::engine::sketch::LOOK_AT_IMAGES));
                assert_eq!(
                    prompt.contains("Uncropped images are in frames-full/"),
                    hosted.is_none()
                );
                if hosted.is_some() {
                    assert!(prompt.contains("[apparent 0:07]"));
                    assert!(!prompt.contains(&directory.to_string_lossy().to_string()));
                } else {
                    assert!(prompt.contains(&shell_quote(&directory.to_string_lossy())));
                }
            }
        }
        assert!(archive.contains("Cropped to: the activity"));
        assert!(archive.contains(&directory.to_string_lossy().to_string()));
        assert!(!archive.contains(CLI_INSTALL) && !archive.contains(DEMO_VIDEO_REQUEST));
    }

    #[test]
    fn demo_video_setting_preserves_other_experiments_and_rejects_bad_config() {
        let fixture = Fixture::new();
        let path = fixture.0.join("config.json");
        assert!(experiment_enabled_in(&load_config_from(&path), AGENT_DEMO_VIDEO));
        fs::write(&path, r#"{"ingestToken":"private","experiments":{"other":true}}"#).unwrap();
        assert!(experiment_enabled_in(&load_config_from(&path), AGENT_DEMO_VIDEO));
        set_experiment_in(&path, AGENT_DEMO_VIDEO, true).unwrap();
        let raw: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["ingestToken"], "private");
        assert_eq!(raw["experiments"]["other"], true);
        assert_eq!(load_config_from(&path).experiments.get(AGENT_DEMO_VIDEO), Some(&true));
        set_experiment_in(&path, AGENT_DEMO_VIDEO, false).unwrap();
        assert_eq!(load_config_from(&path).experiments.get(AGENT_DEMO_VIDEO), Some(&false));
        assert!(!experiment_enabled_in(&load_config_from(&path), AGENT_DEMO_VIDEO));
        fs::write(&path, b"{").unwrap();
        assert!(set_experiment_in(&path, AGENT_DEMO_VIDEO, true).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"{");
    }

    #[test]
    fn server_addresses_parse_without_lookups() {
        let addr = |s: &str| server_address(s).map(|a| a.to_string());
        assert_eq!(
            addr("http://127.0.0.1:4096\nmore").as_deref(),
            Some("127.0.0.1:4096")
        );
        assert_eq!(
            addr("http://localhost:4096/").as_deref(),
            Some("127.0.0.1:4096")
        );
        assert_eq!(addr("http://[::1]:5000").as_deref(), Some("[::1]:5000"));
        assert_eq!(addr("https://10.0.0.2").as_deref(), Some("10.0.0.2:443"));
        assert_eq!(addr("http://studio.example.ts.net:4096"), None);
        assert_eq!(addr("nonsense"), None);
    }

    #[test]
    fn server_endpoint_reads_state_and_config_service_files() {
        let fixture = Fixture::new();
        let state_path = fixture.0.join("state-service.json");
        let config_path = fixture.0.join("config-service.json");
        fs::write(
            &state_path,
            r#"{"id":"s1","url":"http://127.0.0.1:49374","pid":123,"password":"secret-state"}"#,
        )
        .unwrap();
        fs::write(
            &config_path,
            r#"{"hostname":"127.0.0.1","port":4096,"password":"secret-config","env":{"API_KEY":"ignored"}}"#,
        )
        .unwrap();
        let state = ServiceFile::load(&state_path).unwrap();
        let config = ServiceFile::load(&config_path).unwrap();

        // State registration wins over service config when both exist.
        let (addr, pw) = resolve_server(&[state, config], None);
        assert_eq!(addr, "127.0.0.1:49374".parse().unwrap());
        assert_eq!(pw.as_deref(), Some("secret-state"));

        // When state registration is absent, config's hostname/port and password are used.
        let config = ServiceFile::load(&config_path).unwrap();
        let (addr, pw) = resolve_server(&[config], None);
        assert_eq!(addr, "127.0.0.1:4096".parse().unwrap());
        assert_eq!(pw.as_deref(), Some("secret-config"));

        // With no address in files, the default port and OPENCODE_PASSWORD are used.
        let (addr, pw) = resolve_server(&[], Some("env-pw"));
        assert_eq!(addr, "127.0.0.1:49374".parse().unwrap());
        assert_eq!(pw.as_deref(), Some("env-pw"));
    }

    #[test]
    fn http_get_authenticates_and_times_out_when_slow() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::time::{Duration, Instant};

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            for i in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut req = Vec::new();
                let mut buf = [0_u8; 256];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = stream.read(&mut buf).unwrap();
                    if n == 0 {
                        break;
                    }
                    req.extend_from_slice(&buf[..n]);
                }
                let head = String::from_utf8_lossy(&req);
                match i {
                    0 => {
                        assert!(
                            head.contains("Authorization: Basic b3BlbmNvZGU6c2VjcmV0\r\n"),
                            "{head}"
                        );
                        let body = br#"{"data":[]}"#;
                        write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .unwrap();
                        stream.write_all(body).unwrap();
                    }
                    1 => {
                        stream
                            .write_all(
                                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                            )
                            .unwrap();
                    }
                    _ => {
                        std::thread::sleep(Duration::from_millis(400));
                    }
                }
            }
        });

        let body = http_get(
            address,
            Some("secret"),
            "/api/session",
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(body, br#"{"data":[]}"#);

        let err = http_get(
            address,
            Some("secret"),
            "/api/session/missing",
            Duration::from_secs(1),
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("HTTP 404"), "{err:#}");

        let started = Instant::now();
        let err = http_get(
            address,
            Some("secret"),
            "/api/session",
            Duration::from_millis(100),
        )
        .unwrap_err();
        assert!(format!("{err:#}").contains("didn't answer"), "{err:#}");
        assert!(started.elapsed() < Duration::from_secs(1));
        let _ = server.join();
    }
}

#[cfg(test)]
mod sync_live_check {
    /// Manual: `cargo test sync_to_pro -- --ignored` copies a dummy bundle to `pro` and checks the
    /// heavy media lands atomically and the marker clears.
    #[test]
    #[ignore]
    fn sync_to_pro() {
        let base = std::path::PathBuf::from(std::env::var("HOME").unwrap())
            .join("Library/Caches/apparent-sync-live");
        let _ = std::fs::remove_dir_all(&base);
        let bundle = base.join("apparent-synctest.apparent");
        std::fs::create_dir_all(bundle.join("frames-full")).unwrap();
        std::fs::write(bundle.join("summary.md"), "hi").unwrap();
        std::fs::write(bundle.join("recording.mp4"), vec![7u8; 8_000_000]).unwrap();
        std::fs::write(bundle.join("frames-full/frame-01.png"), b"png").unwrap();
        super::sync_bundle_to("pro", &bundle).unwrap();
        let q = super::shell_quote(&bundle.to_string_lossy());
        let check = format!("ls -a {q}; ls {q}/frames-full; wc -c < {q}/recording.mp4");
        let mut out = String::new();
        for _ in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(250));
            let o = super::ssh("pro").arg(&check).output().unwrap();
            out = String::from_utf8_lossy(&o.stdout).into_owned();
            if !out.contains(super::MEDIA_PENDING) && out.contains("8000000") {
                break;
            }
        }
        let _ = super::ssh("pro")
            .arg(format!(
                "rm -rf {}",
                super::shell_quote(&base.to_string_lossy())
            ))
            .status();
        assert!(
            out.contains("8000000")
                && out.contains("frame-01.png")
                && !out.contains(super::MEDIA_PENDING),
            "{out}"
        );
    }
}

/// A stand-in for OpenCode's session API.
#[cfg(test)]
pub(crate) mod fake_api {
    use super::*;

    pub fn session(id: &str, title: &str, dir: &str, updated: u64) -> OpenCodeSession {
        OpenCodeSession {
            id: id.into(),
            title: title.into(),
            parent_id: None,
            time: OpenCodeSessionTime {
                updated,
                viewed: None,
                archived: None,
            },
            location: Some(OpenCodeSessionLocation {
                directory: Some(dir.into()),
            }),
            directory: None,
        }
    }

    /// Answers `GET /api/session/{id}` and `GET /api/session?…` like OpenCode 2: newest first,
    /// `parentID=null`, exact `directory`, `limit` (default 50), and a cursor carrying the filters.
    pub fn serve(sessions: &[OpenCodeSession], path: &str) -> Result<Vec<u8>> {
        use crate::capture::engine::accessibility::ax::percent_decode;
        let (route, query) = path.split_once('?').unwrap_or((path, ""));
        if let Some(id) = route.strip_prefix("/api/session/") {
            let session = sessions
                .iter()
                .find(|s| s.id == percent_decode(id))
                .context("404 Not Found")?;
            return Ok(serde_json::to_vec(&serde_json::json!({ "data": session }))?);
        }
        assert_eq!(route, "/api/session");
        let mut params: std::collections::HashMap<String, String> = query
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .map(|(k, v)| (k.to_owned(), percent_decode(v)))
            .collect();
        let mut offset = 0;
        if let Some(cursor) = params.remove("cursor") {
            let cursor: serde_json::Value = serde_json::from_str(&cursor)?;
            offset = cursor["offset"].as_u64().unwrap() as usize;
            for key in ["parentID", "directory"] {
                if let Some(v) = cursor[key].as_str() {
                    params.insert(key.into(), v.into());
                }
            }
        }
        let limit: usize = params.get("limit").map_or(50, |l| l.parse().unwrap());
        let mut matching: Vec<_> = sessions
            .iter()
            .filter(|s| params.get("parentID").is_none_or(|_| s.parent_id.is_none()))
            .filter(|s| {
                params
                    .get("directory")
                    .is_none_or(|d| s.directory() == Some(d.as_str()))
            })
            .collect();
        matching.sort_by(|a, b| b.time.updated.cmp(&a.time.updated));
        let page: Vec<_> = matching.iter().skip(offset).take(limit).collect();
        let next = (offset + limit < matching.len()).then(|| {
            serde_json::json!({
                "offset": offset + limit,
                "parentID": params.get("parentID"),
                "directory": params.get("directory"),
            })
            .to_string()
        });
        Ok(serde_json::to_vec(
            &serde_json::json!({ "data": page, "cursor": { "next": next } }),
        )?)
    }
}

#[cfg(test)]
mod session_lookup {
    use super::fake_api::{serve, session};
    use super::*;

    fn many(count: usize) -> Vec<OpenCodeSession> {
        (0..count)
            .map(|i| {
                session(
                    &format!("ses_{i:05}"),
                    &format!("Chat {i}"),
                    "/p/x",
                    100_000 - i as u64,
                )
            })
            .collect()
    }

    /// Runs `f` against `sessions` and returns its result with the paths it requested.
    fn counting<T>(
        sessions: &[OpenCodeSession],
        f: impl FnOnce(&mut ApiGet) -> T,
    ) -> (T, Vec<String>) {
        let mut paths = Vec::new();
        let result = f(&mut |path: &str| {
            paths.push(path.to_owned());
            serve(sessions, path)
        });
        (result, paths)
    }

    #[test]
    fn an_explicit_id_is_one_request() {
        let mut sessions = many(5_000);
        sessions[4_000].title = "THE ONE".into();
        let (result, paths) = counting(&sessions, |get| {
            resolve_opencode_session_with(get, Some("ses_04000"), None, None).unwrap()
        });
        assert_eq!(result, ("ses_04000".into(), "THE ONE".into()));
        assert_eq!(paths, ["/api/session/ses_04000"]);
        // Unknown to the server: still sent, titled by its ID.
        let (result, _) = counting(&sessions, |get| {
            resolve_opencode_session_with(get, Some("ses_gone"), None, None).unwrap()
        });
        assert_eq!(result, ("ses_gone".into(), "ses_gone".into()));
    }

    #[test]
    fn the_newest_open_root_is_one_page() {
        let mut sessions = many(5_000);
        sessions[0].time.archived = Some(1);
        sessions[1].parent_id = Some("ses_00002".into());
        let (result, paths) = counting(&sessions, |get| {
            resolve_opencode_session_with(get, None, None, None).unwrap()
        });
        assert_eq!(result.0, "ses_00002");
        assert_eq!(paths, ["/api/session?parentID=null&limit=100"]);
    }

    #[test]
    fn an_app_match_stops_at_the_first_hit_and_the_search_is_capped() {
        let mut sessions = many(5_000);
        sessions[150].title = "UP DOWN APP".into();
        sessions[300].title = "updown older".into();
        let (result, paths) = counting(&sessions, |get| {
            resolve_opencode_session_with(get, None, Some("com.kitlangton.UpDown"), None).unwrap()
        });
        assert_eq!(result, ("ses_00150".into(), "UP DOWN APP".into()));
        assert_eq!(paths.len(), 2);

        // No match: OPENCODE_SESSION_ID if it's an open root, else the newest; never every page.
        let (result, paths) = counting(&sessions, |get| {
            resolve_opencode_session_with(get, None, Some("Nothing"), Some("ses_04321")).unwrap()
        });
        assert_eq!(result.0, "ses_04321");
        assert_eq!(paths.len(), ROOT_SEARCH_PAGES + 1);
        let (result, _) = counting(&sessions, |get| {
            resolve_opencode_session_with(get, None, Some("Nothing"), None).unwrap()
        });
        assert_eq!(result.0, "ses_00000");
    }
}
