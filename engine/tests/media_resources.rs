#![cfg(all(feature = "media", target_os = "linux"))]
use apparent_capture_engine::{Admission, LeaseKind, Runtime, new_id};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::process::{Command, Stdio};

#[test]
fn real_media_child_holds_read_lease_without_consuming_capture_admission() {
    let root = std::env::temp_dir()
        .canonicalize()
        .unwrap()
        .join(format!("engine-media-resources-{}", new_id().unwrap()));
    std::fs::create_dir(&root).unwrap();
    std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    let runtime = root.join("runtime");
    let bundle = root.join("source.apparent");
    for p in [&runtime, &bundle] {
        std::fs::create_dir(p).unwrap();
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let video = bundle.join("recording.mov");
    assert!(
        Command::new("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                "testsrc2=size=640x360:rate=10:duration=2",
                "-an",
                "-c:v",
                "mjpeg",
                "-threads",
                "1"
            ])
            .arg(&video)
            .status()
            .unwrap()
            .success()
    );
    std::fs::write(
        bundle.join("apparent.json"),
        serde_json::to_vec(
            &json!({"version":1,"id":"source","durationMs":2000,"video":"recording.mov"}),
        )
        .unwrap(),
    )
    .unwrap();
    for p in [&video, &bundle.join("apparent.json")] {
        std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
    let m = std::fs::metadata(&video).unwrap();
    let request = json!({"channel":"media","protocolVersion":1,"clientId":"a".repeat(32),"rpcId":"c".repeat(32),"method":"decode","jobId":"d".repeat(32),
        "source":{"bundleId":"source","bundlePath":bundle,"videoPath":video,"videoOffsetMs":0,"identity":{"device":m.dev().to_string(),"inode":m.ino().to_string(),"size":m.len(),"modifiedNs":(i128::from(m.mtime())*1_000_000_000+i128::from(m.mtime_nsec())).to_string()}},
        "requestedBundleMs":vec![0;48],"outputDirectory":root.join("output"),"limits":{"maxFrames":48,"maxEdge":640,"maxTotalPixels":64000000,"deadlineMs":10000}});
    let rt = Runtime::open(&runtime).unwrap();
    let Admission::Acquired(_capture) = rt
        .admit(&"e".repeat(32), &"f".repeat(64), "unrelated-live-capture")
        .unwrap()
    else {
        panic!("capture admission not held")
    };
    let mut child = Command::new(env!("CARGO_BIN_EXE_apparent-engine-frames"))
        .args([
            "--media-runtime-directory",
            runtime.to_str().unwrap(),
            "--media-channel",
            "media",
            "--media-protocol-version",
            "1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    writeln!(stdin, "{}", json!({"channel":"media","protocolVersion":1,"clientId":"a".repeat(32),"rpcId":"b".repeat(32),"method":"handshake"})).unwrap();
    let mut line = String::new();
    stdout.read_line(&mut line).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap()["result"]["implementation"],
        "native"
    );
    writeln!(stdin, "{request}").unwrap();
    writeln!(stdin, "{}", json!({"channel":"media","protocolVersion":1,"clientId":"a".repeat(32),"rpcId":"9".repeat(32),"method":"inspect","jobId":"d".repeat(32)})).unwrap();
    line.clear();
    stdout.read_line(&mut line).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap()["result"]["state"],
        "decoding"
    );
    assert!(
        rt.lease("source", LeaseKind::Maintenance)
            .unwrap()
            .is_none()
    );
    // EOF must kill/reap the owned decoder before releasing the read lease.
    drop(stdin);
    assert!(child.wait().unwrap().success());
    assert!(
        rt.lease("source", LeaseKind::Maintenance)
            .unwrap()
            .is_some()
    );
    assert_eq!(std::fs::read_dir(root.join("output")).unwrap().count(), 0);
    assert!(matches!(
        rt.admit(&"8".repeat(32), &"7".repeat(64), "other-capture")
            .unwrap(),
        Admission::Existing
    ));
    drop(_capture);
    std::fs::remove_dir_all(root).unwrap();
}

#[cfg(feature = "capture")]
mod recovery_peer_tests {

    use apparent_capture_engine::{Admission, Runtime};
    use serde::Serialize;
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use std::{
        io::{BufRead, BufReader, Write},
        os::unix::fs::{MetadataExt, PermissionsExt},
        process::{Child, Command, Stdio},
        thread,
        time::{Duration, Instant},
    };

    fn runtime() -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "engine-recovery-peer-{}",
            apparent_capture_engine::new_id().unwrap()
        ));
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        root
    }

    fn wait(child: &mut Child) {
        let until = Instant::now() + Duration::from_secs(15);
        loop {
            if child.try_wait().unwrap().is_some() {
                return;
            }
            assert!(Instant::now() < until, "native recovery peer did not exit");
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn lifecycle(
        stderr: &mut BufReader<std::process::ChildStderr>,
        event: &str,
        peer: &str,
        channel: &str,
    ) {
        let mut line = String::new();
        stderr.read_line(&mut line).unwrap();
        let value: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(value["schema"], "apparent.engine-recovery-peer/1");
        assert_eq!(value["peerId"], peer);
        assert_eq!(value["channel"], channel);
        assert_eq!(value["implementation"], "native");
        assert_eq!(value["role"], "recovery");
        assert_eq!(value["startsFreshCapture"], false);
        assert_eq!(value["onInputEof"], "retain-admitted-finite-work");
        assert_eq!(value["event"], event);
    }

    fn sha(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn seal(
        root: &apparent_capture_engine::SecureStore,
        path: &std::path::Path,
        leaf: &str,
    ) -> Value {
        use std::os::unix::fs::MetadataExt;
        let file = root.file(leaf, false).unwrap();
        let meta = file.metadata().unwrap();
        json!({"relativePath":leaf,"sha256":sha(&std::fs::read(path.join(leaf)).unwrap()),"identity":{"device":meta.dev().to_string(),"inode":meta.ino().to_string(),"size":meta.len(),"modifiedNs":(meta.mtime() as i128 * 1_000_000_000 + meta.mtime_nsec() as i128).to_string()}})
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct MediaKey {
        capture_id: String,
        materials_id: String,
        revision: String,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct MediaLimits {
        max_frames: u64,
        max_edge: u32,
        max_total_pixels: u64,
        deadline_ms: u64,
    }

    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct MediaRequest {
        schema: &'static str,
        materials: MediaKey,
        lease_id: String,
        job_id: String,
        output_directory: String,
        requested_bundle_ms: Vec<u64>,
        limits: MediaLimits,
        contact_sheet: Option<Value>,
    }

    #[test]
    fn capture_recovery_rejects_start_and_idle_release_is_bounded() {
        let root = runtime();
        let runtime = Runtime::open(&root).unwrap();
        let foreign_capture = "9".repeat(32);
        let foreign_owner = "8".repeat(64);
        let Admission::Acquired(_foreign) = runtime
            .admit(&foreign_capture, &foreign_owner, "foreign-live-capture")
            .unwrap()
        else {
            panic!("foreign capture fixture must acquire its original admission")
        };
        let peer = "a1".repeat(16);
        let mut child = Command::new(env!("CARGO_BIN_EXE_apparent-engine-capture"))
            .args([
                "--runtime-directory",
                root.to_str().unwrap(),
                "--capture-channel",
                "capture",
                "--capture-protocol-version",
                "2",
                "--recovery-peer-v1",
                &peer,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stderr = BufReader::new(child.stderr.take().unwrap());
        lifecycle(&mut stderr, "ready", &peer, "capture/2");
        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        writeln!(
            stdin,
            "{}",
            json!({"channel":"capture","protocolVersion":2,"clientId":"b".repeat(32),"rpcId":"a".repeat(32),"method":"handshake"})
        )
        .unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let handshake: Value = serde_json::from_str::<Value>(&line).unwrap()["result"].clone();
        assert_eq!(handshake["readiness"], "unavailable");
        assert_eq!(handshake["capabilities"]["displayCapture"], false);
        assert_eq!(handshake["capabilities"]["sourceDiscovery"], false);
        let request = json!({"channel":"capture","protocolVersion":2,"clientId":"b".repeat(32),"rpcId":"c".repeat(32),"method":"start","ownerToken":"d".repeat(64),"captureId":"e".repeat(32),"bundleId":"recovery-test.apparent","bundlePath":root.join("recovery-test.apparent"),"source":{},"options":{"screen":true},"invocation":{}});
        writeln!(stdin, "{request}").unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["error"]["code"], "unavailable");
        assert!(
            !root
                .join("capture-v2")
                .join(format!("{}.json", "e".repeat(32)))
                .exists()
        );
        drop(stdin);
        lifecycle(&mut stderr, "input-closed", &peer, "capture/2");
        wait(&mut child);
        assert!(child.wait().unwrap().success());
        assert!(matches!(
            runtime
                .admit(&foreign_capture, &foreign_owner, "foreign-live-capture")
                .unwrap(),
            Admission::Existing
        ));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn media2_recovery_role_and_malformed_nonce_are_enforced() {
        let root = runtime();
        let peer = "f1".repeat(16);
        let mut child = Command::new(env!("CARGO_BIN_EXE_apparent-engine-frames"))
            .args([
                "--media-runtime-directory",
                root.to_str().unwrap(),
                "--media-channel",
                "media",
                "--media-protocol-version",
                "2",
                "--recovery-peer-v1",
                &peer,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stderr = BufReader::new(child.stderr.take().unwrap());
        lifecycle(&mut stderr, "ready", &peer, "media/2");
        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        writeln!(stdin, "{}", json!({"channel":"media","protocolVersion":2,"clientId":"1".repeat(32),"rpcId":"2".repeat(32),"method":"handshake"})).unwrap();
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        assert_eq!(
            serde_json::from_str::<Value>(&line).unwrap()["result"]["channel"],
            "media"
        );
        drop(stdout);
        // A caller may relinquish output handles before EOF is observed. A late
        // response failure must not tear down the native recovery process.
        writeln!(stdin, "{}", json!({"channel":"media","protocolVersion":2,"clientId":"1".repeat(32),"rpcId":"3".repeat(32),"method":"handshake"})).unwrap();
        drop(stdin);
        lifecycle(&mut stderr, "input-closed", &peer, "media/2");
        wait(&mut child);
        assert!(child.wait().unwrap().success());

        let bad = Command::new(env!("CARGO_BIN_EXE_apparent-engine-frames"))
            .args([
                "--media-runtime-directory",
                root.to_str().unwrap(),
                "--media-channel",
                "media",
                "--media-protocol-version",
                "2",
                "--recovery-peer-v1",
                "A".repeat(32).as_str(),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .unwrap();
        assert!(!bad.success());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn media2_finite_job_is_reaped_and_reconciled_after_output_disconnect_and_eof() {
        finite_job_after_disconnection(b"");
    }

    #[test]
    fn media2_truncated_recovery_input_does_not_cancel_admitted_work() {
        finite_job_after_disconnection(b"{\"rpcId\":");
    }

    #[test]
    fn media2_invalid_recovery_json_does_not_cancel_admitted_work() {
        finite_job_after_disconnection(b"{broken}\n");
    }

    fn finite_job_after_disconnection(tail: &[u8]) {
        use apparent_capture_engine::SecureStore;

        let base = runtime();
        let runtime = base.join("runtime");
        std::fs::create_dir(&runtime).unwrap();
        std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
        let _runtime = apparent_capture_engine::Runtime::open(&runtime).unwrap();
        let capture_id = "a".repeat(32);
        let materials_id = "b".repeat(32);
        let lease_id = "c".repeat(32);
        let owner = "d".repeat(64);
        let job_id = "e".repeat(32);
        let capture_path = runtime.join("capture-v2");
        let captures = SecureStore::create(&capture_path).unwrap();
        let materials_parent = SecureStore::create(&capture_path.join("materials")).unwrap();
        let materials_path = capture_path.join("materials").join(&capture_id);
        let materials = SecureStore::create(&materials_path).unwrap();
        materials
            .write_new(
                "recording.mp4",
                include_bytes!("../core/tests/fixtures/tiny.mp4"),
            )
            .unwrap();
        let video = seal(&materials, &materials_path, "recording.mp4");
        let payload = json!({
            "schema":"apparent.capture-materials-payload/1",
            "identity":{"captureId":capture_id,"materialsId":materials_id},
            "bundleId":"synthetic-recovery.apparent","root":materials_path,
            "leaseId":lease_id,
            "clock":{"epoch":"capture-admission-monotonic","units":"integer-milliseconds","durationMs":2000,"videoOffsetMs":100},
            "files":[video],"video":{"file":"recording.mp4","width":16,"height":16,"durationMs":200},
            "safety":{"safeForProcessing":true}
        });
        let payload_bytes = serde_json::to_vec(&payload).unwrap();
        materials
            .write_new("materials.json", &payload_bytes)
            .unwrap();
        let payload_file = seal(&materials, &materials_path, "materials.json");
        let root_meta = materials.directory_metadata().unwrap();
        let manifest = json!({
            "schema":"apparent.capture-materials/1",
            "key":{"captureId":capture_id,"materialsId":materials_id,"revision":sha(&payload_bytes)},
            "root":materials_path,"leaseId":lease_id,"payloadFile":payload_file
        });
        captures
            .write_new(
                &format!("{capture_id}.json"),
                &serde_json::to_vec(&json!({
                    "capture_id":capture_id,"owner_token":owner,"request":{},
                    "state":"materials-ready","materials":manifest,"root":materials_path,
                    "materials_id":materials_id,"lease_id":lease_id,
                    "root_device":root_meta.dev(),"root_inode":root_meta.ino(),
                    "release_phase":null,"release_movie":video["identity"],
                    "release_payload":payload_file["identity"],"payload_digest":sha(&payload_bytes)
                }))
                .unwrap(),
            )
            .unwrap();
        drop(materials_parent);

        let output = base.with_extension("derivative");
        let request_json = serde_json::to_string(&MediaRequest {
            schema: "apparent.retained-frame-request/1",
            materials: MediaKey {
                capture_id: capture_id.clone(),
                materials_id: materials_id.clone(),
                revision: sha(&payload_bytes),
            },
            lease_id: lease_id.clone(),
            job_id: job_id.clone(),
            output_directory: output.to_string_lossy().into_owned(),
            requested_bundle_ms: vec![0, 150, 150, 2000],
            limits: MediaLimits {
                max_frames: 4,
                max_edge: 8,
                max_total_pixels: 64000,
                deadline_ms: 10000,
            },
            contact_sheet: None,
        })
        .unwrap();
        let request_sha = sha(request_json.as_bytes());
        let mut child = Command::new(env!("CARGO_BIN_EXE_apparent-engine-frames"))
            .args([
                "--media-runtime-directory",
                runtime.to_str().unwrap(),
                "--media-channel",
                "media",
                "--media-protocol-version",
                "2",
                "--recovery-peer-v1",
                "7a".repeat(16).as_str(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut stderr = BufReader::new(child.stderr.take().unwrap());
        lifecycle(&mut stderr, "ready", &"7a".repeat(16), "media/2");
        let mut stdin = child.stdin.take().unwrap();
        let mut stdout = BufReader::new(child.stdout.take().unwrap());
        writeln!(
        stdin,
        "{}",
        json!({"channel":"media","protocolVersion":2,"clientId":"8".repeat(32),"rpcId":"6".repeat(32),"method":"handshake"})
    )
    .unwrap();
        let mut handshake_line = String::new();
        stdout.read_line(&mut handshake_line).unwrap();
        assert!(serde_json::from_str::<Value>(&handshake_line).unwrap()["result"].is_object());
        writeln!(
        stdin,
        "{}",
        json!({
            "channel":"media","protocolVersion":2,"clientId":"8".repeat(32),"rpcId":"9".repeat(32),
            "method":"derive","captureId":capture_id,"ownerToken":owner,"jobId":job_id,
            "requestSha256":request_sha,"requestJson":request_json
        })
    )
    .unwrap();
        stdin.flush().unwrap();
        let mut initial_line = String::new();
        stdout.read_line(&mut initial_line).unwrap();
        let initial: Value = serde_json::from_str(&initial_line).unwrap();
        assert_eq!(initial["result"]["state"], "running", "{initial}");
        drop(stdout);
        drop(stderr);
        stdin.write_all(tail).unwrap();
        stdin.flush().unwrap();
        drop(stdin);
        wait(&mut child);
        assert!(child.wait().unwrap().success());

        let mut observer = Command::new(env!("CARGO_BIN_EXE_apparent-engine-frames"))
            .args([
                "--media-runtime-directory",
                runtime.to_str().unwrap(),
                "--media-channel",
                "media",
                "--media-protocol-version",
                "2",
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut input = observer.stdin.take().unwrap();
        let mut output_reader = BufReader::new(observer.stdout.take().unwrap());
        for (rpc, method) in [("1".repeat(32), "handshake"), ("2".repeat(32), "inspect")] {
            let mut request = json!({"channel":"media","protocolVersion":2,"clientId":"3".repeat(32),"rpcId":rpc,"method":method});
            if method == "inspect" {
                request["captureId"] = json!(capture_id);
                request["ownerToken"] = json!(owner);
                request["jobId"] = json!(job_id);
                request["requestSha256"] = json!(request_sha);
            }
            writeln!(input, "{request}").unwrap();
            let mut line = String::new();
            output_reader.read_line(&mut line).unwrap();
            let response: Value = serde_json::from_str(&line).unwrap();
            if method == "inspect" {
                assert_eq!(response["result"]["jobId"], job_id, "{response}");
                assert_eq!(response["result"]["requestSha256"], request_sha);
                assert_eq!(response["result"]["state"], "complete", "{response}");
            }
        }
        drop(input);
        wait(&mut observer);
        assert!(observer.wait().unwrap().success());
        assert!(output.join("receipt.json").is_file());
        std::fs::remove_dir_all(base).unwrap();
        std::fs::remove_dir_all(output).unwrap();
    }
}
