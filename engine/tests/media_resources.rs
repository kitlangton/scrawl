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
