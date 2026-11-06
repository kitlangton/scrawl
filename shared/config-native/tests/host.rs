use serde_json::{Value, json};
use std::{fs, io::Write, os::unix::fs::DirBuilderExt, path::PathBuf, process::{Command, Stdio}, sync::{Arc, Barrier, atomic::{AtomicU64, Ordering}}};

static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

fn temp() -> PathBuf {
    temp_at(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos())
}
fn temp_at(stamp: u128) -> PathBuf {
    let sequence = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("apparent-config-host-{}-{stamp}-{sequence}", std::process::id()));
    fs::DirBuilder::new().mode(0o700).create(&p).unwrap();
    p.canonicalize().unwrap()
}

#[test]
fn fixtures_never_share_state_when_the_clock_repeats() {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let a = temp_at(stamp);
    let b = temp_at(stamp);
    assert_ne!(a, b, "independent tests must never share config or lock files");
    fs::remove_dir_all(a).unwrap();
    fs::remove_dir_all(b).unwrap();
}
fn host(path: &PathBuf, owner: &str, request: &str) -> (i32, String) {
    host_ordered(path, owner, request, false)
}
fn host_ordered(path: &PathBuf, owner: &str, request: &str, wait_before_input: bool) -> (i32, String) {
    let mut child = Command::new(env!("CARGO_BIN_EXE_apparent-config-host"))
        .args(["--config", path.to_str().unwrap(), "--owner", owner])
        .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let mut input = child.stdin.take().unwrap();
    if wait_before_input { child.wait().unwrap(); }
    let write_result = input.write_all(request.as_bytes());
    drop(input);
    let out = child.wait_with_output().unwrap();
    if let Err(error) = write_result {
        let intentional_early_close = error.kind() == std::io::ErrorKind::BrokenPipe
            && (out.status.code() == Some(2) || request.len() > 64 * 1024);
        assert!(intentional_early_close, "host closed stdin for a valid request: {error}");
    }
    (out.status.code().unwrap_or(-1), String::from_utf8(out.stdout).unwrap())
}

#[test]
fn argument_rejection_can_close_the_pipe_before_the_fixture_sends_input() {
    let dir = temp();
    let (status, output) = host_ordered(&dir.join("config.json"), "InvalidOwner", &req("x", "read", ""), true);
    assert_eq!(status, 2);
    assert!(output.is_empty());
    fs::remove_dir_all(dir).unwrap();
}
fn req(id: &str, operation: &str, tail: &str) -> String { format!("{{\"protocol\":1,\"id\":\"{id}\",\"operation\":\"{operation}\"{tail}}}\n") }
fn response(s: &str) -> Value { serde_json::from_str::<Value>(s.lines().next().unwrap()).unwrap()["result"].clone() }

#[test]
fn host_cas_is_serialized_persistent_and_preserves_legacy_keys() {
    let dir = temp(); let path = dir.join("config.json");
    fs::write(&path, br#"{"unrelated":{"secret":"synthetic"},"streamState":{"agent":{"opaque":"preserved"}}}"#).unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let mut joins = vec![];
    for (id, value) in [("left", "L"), ("right", "R")] {
        let path = path.clone(); let barrier = barrier.clone();
        joins.push(std::thread::spawn(move || {
            barrier.wait();
            host(&path, "desktop", &req(id, "compareAndSwap", &format!(",\"expected\":0,\"value\":\"{value}\""))).1
        }));
    }
    barrier.wait();
    let outputs: Vec<_> = joins.into_iter().map(|j| j.join().unwrap()).collect(); let results: Vec<_> = outputs.iter().map(|s| response(s)).collect();
    assert_eq!(results.iter().filter(|v| v["status"] == "saved").count(), 1, "{results:?}");
    assert_eq!(results.iter().filter(|v| v["status"] == "conflict").count(), 1, "{results:?}");
    let read = response(&host(&path, "desktop", &req("read", "read", "")).1);
    assert_eq!(read["revision"], 1);
    let mut config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(config["unrelated"]["secret"], "synthetic");
    assert_eq!(config["streamState"]["agent"], json!({"opaque":"preserved"}));
    config["legacy"] = json!(true);
    fs::write(&path, serde_json::to_vec(&config).unwrap()).unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let left_path = path.clone(); let left_barrier = barrier.clone();
    let legacy = std::thread::spawn(move || { left_barrier.wait(); apparent_config_native::set_config_value_in(&left_path, "legacy", json!(false)).unwrap(); });
    let right_path = path.clone(); let right_barrier = barrier.clone();
    let stream = std::thread::spawn(move || { right_barrier.wait(); host(&right_path, "desktop", &req("next", "compareAndSwap", ",\"expected\":1,\"value\":null")).1 });
    barrier.wait(); legacy.join().unwrap();
    assert_eq!(response(&stream.join().unwrap())["status"], "saved");
    let config: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(config["unrelated"]["secret"], "synthetic");
    assert_eq!(config["streamState"]["desktop"]["value"], Value::Null);
    assert_eq!(config["streamState"]["desktop"]["revision"], 2);
    let restart = response(&host(&path, "desktop", &req("again", "read", "")).1);
    assert_eq!(restart["revision"], 2);
    fs::remove_dir_all(dir).unwrap();
}

fn adoption_req(id: &str, operation: &str, tail: &str) -> String {
    format!("{{\"protocol\":1,\"id\":\"{id}\",\"operation\":\"{operation}\"{tail}}}\n")
}

#[test]
fn adoption_presence_cas_and_preservation_are_authoritative() {
    let dir = temp(); let path = dir.join("config.json");
    fs::write(&path, br#"{"unrelated":{"keep":true},"desktop":null,"streamState":{"agent":{"opaque":1}}}"#).unwrap();
    let (code, out) = host(&path, "desktop", &adoption_req("r", "readForAdoption", ""));
    assert_eq!(code, 0);
    let snapshot = response(&out);
    assert_eq!(snapshot, json!({"owner":{"present":false,"revision":0,"value":null},"legacy":{"present":true,"value":null}}));
    let before = fs::read(&path).unwrap();
    assert_eq!(response(&host(&path, "desktop", &adoption_req("r2", "readForAdoption", "" )).1), snapshot);
    assert_eq!(fs::read(&path).unwrap(), before, "read must not rewrite config");

    // A legacy writer changes the selected top-level key after the observation.
    apparent_config_native::set_config_value_in(&path, "desktop", json!("new legacy")).unwrap();
    let stale = adoption_req("a", "adoptLegacy", ",\"expectedLegacy\":{\"present\":true,\"value\":null},\"value\":{\"adopted\":true}");
    let conflict = response(&host(&path, "desktop", &stale).1);
    assert_eq!(conflict["status"], "conflict");
    assert_eq!(conflict["snapshot"]["legacy"], json!({"present":true,"value":"new legacy"}));
    assert_eq!(conflict["snapshot"]["owner"], json!({"present":false,"revision":0,"value":null}));

    // Missing and null are distinct, and a matching successful adoption preserves legacy.
    let missing = temp().join("config.json");
    let dir2 = missing.parent().unwrap().to_path_buf();
    fs::write(&missing, br#"{"unrelated":{"keep":true},"streamState":{"agent":{"opaque":1}}}"#).unwrap();
    let wrongly_null = adoption_req("n", "adoptLegacy", ",\"expectedLegacy\":{\"present\":true,\"value\":null},\"value\":1");
    assert_eq!(response(&host(&missing, "desktop", &wrongly_null).1)["status"], "conflict");
    let correct = adoption_req("ok", "adoptLegacy", ",\"expectedLegacy\":{\"present\":false,\"value\":null},\"value\":{\"from\":\"missing\"}");
    assert_eq!(response(&host(&missing, "desktop", &correct).1), json!({"status":"saved","revision":1}));
    let saved: Value = serde_json::from_slice(&fs::read(&missing).unwrap()).unwrap();
    assert_eq!(saved["streamState"]["desktop"], json!({"revision":1,"value":{"from":"missing"}}));
    assert_eq!(saved["streamState"]["agent"], json!({"opaque":1}));
    assert_eq!(saved["unrelated"]["keep"], true);
    assert!(!saved.as_object().unwrap().contains_key("desktop"));
    fs::remove_dir_all(dir).unwrap(); fs::remove_dir_all(dir2).unwrap();
}

#[test]
fn adoption_refuses_owner_cas_winner_including_null_and_malformed_selected_owner() {
    let dir = temp(); let path = dir.join("config.json");
    apparent_config_native::compare_and_swap_stream_state_in(&path, "desktop", 0, Value::Null).unwrap();
    let req = adoption_req("a", "adoptLegacy", ",\"expectedLegacy\":{\"present\":false,\"value\":null},\"value\":1");
    assert_eq!(response(&host(&path, "desktop", &req).1)["status"], "conflict");
    assert_eq!(response(&host(&path, "desktop", &adoption_req("r", "readForAdoption", "")).1)["owner"], json!({"present":true,"revision":1,"value":null}));
    let malformed = dir.join("bad.json");
    fs::write(&malformed, br#"{"streamState":{"desktop":{"revision":1}}}"#).unwrap();
    let (_, out) = host(&malformed, "desktop", &adoption_req("bad", "readForAdoption", ""));
    assert_eq!(response(&out)["status"], "unknown");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn onboarding_adoption_atomically_retires_real_legacy_progress_writer_and_preserves_other_config() {
    let dir = temp(); let path = dir.join("config.json");
    fs::write(&path, br#"{"onboarding":{"resume":"permissions"},"secret":"synthetic","streamState":{"agent":{"revision":2,"value":{"keep":true}}}}"#).unwrap();
    let snapshot = apparent_config_native::read_for_adoption_in(&path, "onboarding").unwrap();
    let adopted = apparent_config_native::adopt_legacy_in(&path, "onboarding", snapshot.legacy, json!({"schema":1,"resume":"permissions"})).unwrap();
    assert_eq!(adopted, apparent_config_native::AdoptResult::Saved { revision: 1 });
    assert!(apparent_config_native::set_config_value_in(&path, "onboarding", json!({"resume":"welcome"})).is_err());
    assert!(apparent_config_native::set_config_value_in(&path, "onboardingLegacyWriter", Value::Null).is_err());
    assert!(apparent_config_native::update_config_in(&path, |config| { config.as_object_mut().unwrap().remove("onboardingLegacyWriter"); Ok(()) }).is_err());
    let before = fs::read(&path).unwrap();
    assert!(apparent_config_native::update_config_in(&path, |config| { config["onboarding"] = json!({"resume":"welcome"}); Ok(()) }).is_err());
    assert!(apparent_config_native::update_config_allowed_in(&path, &|| true, |config| { config.as_object_mut().unwrap().remove("onboarding"); Ok(()) }).is_err());
    assert_eq!(fs::read(&path).unwrap(), before, "rejected generic edits must not publish any bytes");
    apparent_config_native::update_config_in(&path, |config| { config["anotherSetting"] = json!(true); Ok(()) }).unwrap();
    apparent_config_native::set_config_value_in(&path, "unrelated", json!(false)).unwrap();
    let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["onboarding"], json!({"resume":"permissions"}));
    assert_eq!(saved["streamState"]["onboarding"]["value"]["resume"], "permissions");
    assert_eq!(saved["streamState"]["agent"], json!({"revision":2,"value":{"keep":true}}));
    assert_eq!(saved["secret"], "synthetic");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn explicit_onboarding_retirement_is_restart_safe_owner_bound_and_does_not_block_other_keys() {
    let dir = temp(); let path = dir.join("config.json");
    fs::write(&path, br#"{"onboarding":{"resume":"try-it"},"secret":"synthetic"}"#).unwrap();
    assert_eq!(response(&host(&path, "record", &adoption_req("wrong", "retireOnboardingLegacyWriter", "")).1)["status"], "unknown");
    assert_eq!(response(&host(&path, "onboarding", &adoption_req("retire", "retireOnboardingLegacyWriter", "")).1), json!({"status":"retired"}));
    assert!(apparent_config_native::set_config_value_in(&path, "onboarding", json!({"resume":"welcome"})).is_err());
    apparent_config_native::set_config_value_in(&path, "microphoneEnabled", json!(false)).unwrap();
    let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["onboarding"]["resume"], "try-it");
    assert_eq!(saved["secret"], "synthetic");
    assert_eq!(saved["microphoneEnabled"], false);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn malformed_onboarding_ownership_marker_refuses_adoption_and_legacy_writes() {
    let dir = temp(); let path = dir.join("config.json");
    fs::write(&path, br#"{"onboarding":{"resume":"permissions"},"onboardingLegacyWriter":{"state":"maybe"}}"#).unwrap();
    let snapshot = apparent_config_native::read_for_adoption_in(&path, "onboarding").unwrap();
    assert!(apparent_config_native::adopt_legacy_in(&path, "onboarding", snapshot.legacy, json!({"schema":1})).is_err());
    assert!(apparent_config_native::set_config_value_in(&path, "onboarding", json!({"resume":"welcome"})).is_err());
    let unchanged: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(unchanged["onboarding"]["resume"], "permissions");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn onboarding_owner_presence_alone_does_not_retire_writer_and_pre_adoption_save_conflicts() {
    let dir = temp(); let path = dir.join("config.json");
    fs::write(&path, br#"{"onboarding":{"resume":"permissions"},"streamState":{"onboarding":{"revision":7,"value":{"schema":1,"resume":"welcome"}}}}"#).unwrap();
    apparent_config_native::set_config_value_in(&path, "onboarding", json!({"resume":"try-it"})).unwrap();
    let stale = apparent_config_native::adopt_legacy_in(&path, "onboarding", apparent_config_native::LegacyValue { present: true, value: json!({"resume":"permissions"}) }, json!({"schema":1,"resume":"permissions"})).unwrap();
    assert!(matches!(stale, apparent_config_native::AdoptResult::Conflict(_)));
    let after = apparent_config_native::read_for_adoption_in(&path, "onboarding").unwrap();
    assert_eq!(after.legacy.value, json!({"resume":"try-it"}));
    assert_eq!(after.owner.revision, 7);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn record_pen_color_presence_cas_is_locked_bounded_and_owner_denied() {
    let dir = temp(); let path = dir.join("config.json");
    fs::write(&path, br#"{"penColor":null,"secret":"synthetic","streamState":{"agent":{"opaque":1}}}"#).unwrap();
    let read = req("r", "readPenColor", "");
    assert_eq!(response(&host(&path, "record", &read).1), json!({"present":true,"value":null}));
    assert_eq!(response(&host(&path, "onboarding", &read).1)["status"], "unknown");
    let wrong = req("w", "compareAndSwapPenColor", ",\"expectedLegacy\":{\"present\":false,\"value\":null},\"next\":{\"present\":true,\"value\":\"red\"}");
    assert_eq!(response(&host(&path, "record", &wrong).1), json!({"status":"conflict","current":{"present":true,"value":null}}));

    let cas = |id: &'static str, value: &'static str| {
        let path = path.clone();
        std::thread::spawn(move || host(&path, "record", &req(id, "compareAndSwapPenColor", &format!(",\"expectedLegacy\":{{\"present\":true,\"value\":null}},\"next\":{{\"present\":true,\"value\":\"{value}\"}}"))).1)
    };
    let results = [cas("a", "blue"), cas("b", "green")].into_iter().map(|j| response(&j.join().unwrap())).collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|v| v["status"] == "saved").count(), 1);
    let conflict = results.iter().find(|v| v["status"] == "conflict").unwrap();
    assert!(matches!(conflict["current"]["value"].as_str(), Some("blue" | "green")));
    let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["secret"], "synthetic");
    assert_eq!(saved["streamState"]["agent"], json!({"opaque":1}));

    let missing = dir.join("missing.json");
    let absent = json!({"present":false,"value":null});
    let set_null = req("n", "compareAndSwapPenColor", ",\"expectedLegacy\":{\"present\":false,\"value\":null},\"next\":{\"present\":true,\"value\":null}");
    assert_eq!(response(&host(&missing, "record", &set_null).1), json!({"status":"saved"}));
    assert_eq!(response(&host(&missing, "record", &read).1), json!({"present":true,"value":null}));
    let delete = req("d", "compareAndSwapPenColor", ",\"expectedLegacy\":{\"present\":true,\"value\":null},\"next\":{\"present\":false,\"value\":null}");
    assert_eq!(response(&host(&missing, "record", &delete).1), json!({"status":"saved"}));
    assert_eq!(response(&host(&missing, "record", &read).1), absent);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn pen_color_protocol_rejects_bad_presence_and_oversized_values() {
    let dir = temp(); let path = dir.join("config.json");
    let malformed = req("m", "compareAndSwapPenColor", ",\"expectedLegacy\":{\"present\":false,\"value\":1},\"next\":{\"present\":true,\"value\":1}");
    assert_eq!(response(&host(&path, "record", &malformed).1)["status"], "unknown");
    let oversized = "x".repeat(16 * 1024 + 1);
    let large = format!("{{\"protocol\":1,\"id\":\"l\",\"operation\":\"compareAndSwapPenColor\",\"expectedLegacy\":{{\"present\":false,\"value\":null}},\"next\":{{\"present\":true,\"value\":\"{oversized}\"}}}}\n");
    assert_eq!(response(&host(&path, "record", &large).1)["status"], "unknown");
    assert!(!path.exists(), "invalid requests must not create config");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn guarded_pen_color_effect_skips_stale_snapshots_and_keeps_absent_distinct_from_null() {
    use apparent_config_native::{PenColorEffectResult, compare_and_swap_pen_color_in, with_pen_color_current_in};
    use std::sync::atomic::{AtomicBool, Ordering};

    let dir = temp(); let path = dir.join("config.json");
    fs::write(&path, br#"{"secret":"preserve","streamState":{"agent":{"revision":4,"value":"opaque"}}}"#).unwrap();
    let absent = apparent_config_native::LegacyValue { present: false, value: Value::Null };
    let null = apparent_config_native::LegacyValue { present: true, value: Value::Null };
    let called = AtomicBool::new(false);

    // A newer top-level CAS completes first; a stale apply must not run.
    assert_eq!(compare_and_swap_pen_color_in(&path, absent.clone(), null.clone()).unwrap(),
        apparent_config_native::LegacyCasResult::Saved);
    let stale = with_pen_color_current_in(&path, absent, || {
        called.store(true, Ordering::SeqCst);
        Ok("must not apply")
    }).unwrap();
    assert_eq!(stale, PenColorEffectResult::Conflict(null.clone()));
    assert!(!called.load(Ordering::SeqCst));

    let applied = with_pen_color_current_in(&path, null.clone(), || Ok("synthetic effect".to_owned())).unwrap();
    assert_eq!(applied, PenColorEffectResult::Applied("synthetic effect".to_owned()));
    let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["penColor"], Value::Null);
    assert_eq!(saved["secret"], "preserve");
    assert_eq!(saved["streamState"]["agent"], json!({"revision":4,"value":"opaque"}));
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn guarded_pen_color_effect_holds_lock_until_interprocess_writer_can_commit() {
    use apparent_config_native::{PenColorEffectResult, with_pen_color_current_in};
    use std::sync::mpsc;
    use std::time::Duration;

    let dir = temp(); let path = dir.join("config.json");
    fs::write(&path, br#"{"penColor":"blue","keep":true}"#).unwrap();
    let expected = apparent_config_native::LegacyValue { present: true, value: json!("blue") };
    let guard_path = path.clone();
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let effect = std::thread::spawn(move || with_pen_color_current_in(&guard_path, expected, || {
        entered_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        Ok("native callback completed")
    }));
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();

    use std::os::fd::AsRawFd;
    let probe_path = dir.join("config.json.lock");
    let probe = fs::OpenOptions::new().read(true).write(true).open(probe_path).unwrap();
    let probe_result = unsafe { libc::flock(probe.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    assert_eq!(probe_result, -1, "effect callback must retain the shared config lock");
    assert_eq!(std::io::Error::last_os_error().kind(), std::io::ErrorKind::WouldBlock);

    let writer_path = path.clone();
    let (writer_started_tx, writer_started_rx) = mpsc::channel();
    let (writer_done_tx, writer_done_rx) = mpsc::channel();
    let writer = std::thread::spawn(move || {
        writer_started_tx.send(()).unwrap();
        let result = host(&writer_path, "record", &req("later", "compareAndSwapPenColor",
            ",\"expectedLegacy\":{\"present\":true,\"value\":\"blue\"},\"next\":{\"present\":true,\"value\":\"green\"}")).1;
        writer_done_tx.send(response(&result)).unwrap();
    });
    writer_started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(writer_done_rx.recv_timeout(Duration::from_millis(150)).is_err(),
        "interprocess config writer must remain blocked during the effect callback");

    release_tx.send(()).unwrap();
    assert_eq!(effect.join().unwrap().unwrap(), PenColorEffectResult::Applied("native callback completed"));
    assert_eq!(writer_done_rx.recv_timeout(Duration::from_secs(2)).unwrap()["status"], "saved");
    writer.join().unwrap();
    let saved: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    assert_eq!(saved["penColor"], "green");
    assert_eq!(saved["keep"], true);
    assert_eq!(unsafe { libc::flock(probe.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) }, 0,
        "writer lock must be released when the effect callback finishes");
    assert_eq!(unsafe { libc::flock(probe.as_raw_fd(), libc::LOCK_UN) }, 0);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn guarded_pen_color_effect_cancels_while_writer_is_still_held() {
    use apparent_config_native::{with_pen_color_current_in, with_pen_color_current_allowed_in};
    use std::sync::{Arc, mpsc, atomic::{AtomicBool, Ordering}};
    use std::time::Duration;

    let dir = temp(); let path = dir.join("config.json");
    fs::write(&path, br#"{"penColor":"blue","keep":true}"#).unwrap();
    let before = fs::read(&path).unwrap();
    let expected = apparent_config_native::LegacyValue { present: true, value: json!("blue") };
    let (held_tx, held_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let holder_path = path.clone(); let holder_expected = expected.clone();
    let holder = std::thread::spawn(move || with_pen_color_current_in(&holder_path, holder_expected, || {
        held_tx.send(()).unwrap();
        release_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        Ok(())
    }));
    held_rx.recv_timeout(Duration::from_secs(1)).unwrap();

    let admitted = Arc::new(AtomicBool::new(true));
    let effect_called = Arc::new(AtomicBool::new(false));
    let (checking_tx, checking_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker_path = path.clone(); let worker_admitted = admitted.clone();
    let worker_called = effect_called.clone();
    let worker = std::thread::spawn(move || {
        let result = with_pen_color_current_allowed_in(&worker_path, expected, &|| {
            let _ = checking_tx.send(());
            worker_admitted.load(Ordering::SeqCst)
        }, || { worker_called.store(true, Ordering::SeqCst); Ok(()) });
        done_tx.send(result.is_err()).unwrap();
    });
    checking_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    admitted.store(false, Ordering::SeqCst);
    let cancelled_before_release = done_rx.recv_timeout(Duration::from_secs(1));
    release_tx.send(()).unwrap();
    holder.join().unwrap().unwrap();
    worker.join().unwrap();
    assert_eq!(cancelled_before_release.unwrap(), true, "cancellation must not wait for the holder");
    assert!(!effect_called.load(Ordering::SeqCst));
    assert_eq!(fs::read(&path).unwrap(), before);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn guarded_pen_color_effect_failure_and_invalid_paths_never_claim_applied() {
    use apparent_config_native::{PenColorEffectResult, with_pen_color_current_in};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::os::unix::fs::symlink;

    let dir = temp(); let path = dir.join("config.json");
    fs::write(&path, br#"{"penColor":"blue","secret":"unchanged"}"#).unwrap();
    let expected = apparent_config_native::LegacyValue { present: true, value: json!("blue") };
    let before = fs::read(&path).unwrap();
    let failed = with_pen_color_current_in(&path, expected.clone(), || -> anyhow::Result<()> {
        anyhow::bail!("synthetic native effect failure")
    });
    assert!(failed.is_err());
    assert!(!matches!(failed, Ok(PenColorEffectResult::Applied(_))));
    assert_eq!(fs::read(&path).unwrap(), before);

    let invalid = dir.join("link.json");
    symlink(&path, &invalid).unwrap();
    let called = AtomicBool::new(false);
    assert!(with_pen_color_current_in(&invalid, expected, || {
        called.store(true, Ordering::SeqCst);
        Ok(())
    }).is_err());
    assert!(!called.load(Ordering::SeqCst));
    assert_eq!(fs::read(&path).unwrap(), before);
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn malformed_and_unsafe_lock_inputs_are_rejected_without_rewriting_config() {
    let dir = temp(); let path = dir.join("config.json");
    let malformed = b"{broken"; fs::write(&path, malformed).unwrap();
    assert!(apparent_config_native::compare_and_swap_stream_state_in(&path, "desktop", 0, json!(1)).is_err());
    assert_eq!(fs::read(&path).unwrap(), malformed);
    fs::remove_file(dir.join("config.json.lock")).unwrap();
    fs::write(&path, b"{}").unwrap();
    let lock = dir.join("config.json.lock"); let target = dir.join("lock-target");
    fs::write(&target, b"").unwrap(); fs::hard_link(&target, &lock).unwrap();
    assert!(apparent_config_native::set_config_value_in(&path, "x", json!(1)).is_err());
    fs::remove_file(&lock).unwrap(); std::os::unix::fs::symlink(&target, &lock).unwrap();
    assert!(apparent_config_native::set_config_value_in(&path, "x", json!(1)).is_err());
    assert_eq!(fs::read(&path).unwrap(), b"{}");
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn host_binds_owner_and_path_and_rejects_malformed_protocol() {
    let dir = temp(); let path = dir.join("config.json");
    let (code, _) = host(&path, "BadOwner", &req("x", "read", "")); assert_eq!(code, 2);
    let (code, _) = host(&PathBuf::from("relative-config.json"), "desktop", &req("x", "read", "")); assert_eq!(code, 2);
    let bad = "{\"protocol\":1,\"id\":\"x\",\"operation\":\"read\",\"owner\":\"other\"}\n";
    let (_, out) = host(&path, "desktop", bad);
    assert_eq!(response(&out)["status"], "unknown");
    let mut oversized = "x".repeat(65_537); oversized.push('\n');
    let (_, out) = host(&path, "desktop", &oversized);
    assert!(out.is_empty());
    fs::remove_dir_all(dir).unwrap();
}


#[test]
fn repeated_concurrent_first_lock_creation_keeps_one_cas_winner() {
    let dir = temp();
    for iteration in 0..32 {
        let path = dir.join(format!("cold-{iteration}.json"));
        fs::write(&path, br#"{"legacy":"synthetic"}"#).unwrap();
        let barrier = Arc::new(Barrier::new(3));
        let joins: Vec<_> = ["left", "right"].into_iter().map(|id| {
            let path = path.clone(); let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                response(&host(&path, "desktop", &req(id, "compareAndSwap", ",\"expected\":0,\"value\":true")).1)
            })
        }).collect();
        barrier.wait();
        let results: Vec<_> = joins.into_iter().map(|join| join.join().unwrap()).collect();
        assert_eq!(results.iter().filter(|result| result["status"] == "saved").count(), 1, "iteration {iteration}: {results:?}");
        assert_eq!(results.iter().filter(|result| result["status"] == "conflict").count(), 1, "iteration {iteration}: {results:?}");
        assert_eq!(apparent_config_native::read_stream_state_in(&path, "desktop").unwrap(), (1, json!(true)));
    }
    fs::remove_dir_all(dir).unwrap();
}
