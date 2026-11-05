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
    if wait_before_input { assert_eq!(write_result.as_ref().unwrap_err().kind(), std::io::ErrorKind::BrokenPipe); }
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
