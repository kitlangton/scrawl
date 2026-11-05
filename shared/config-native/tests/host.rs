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
