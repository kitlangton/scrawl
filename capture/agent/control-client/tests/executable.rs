#![cfg(feature = "client")]

use std::io::{BufRead, BufReader, Write};
use std::os::unix::{fs::PermissionsExt, net::UnixListener};
use std::process::Command;

#[test]
fn executable_receipts_and_exit_codes() {
    let id = "a".repeat(32);
    for (args, reply, success) in [
        (vec!["state"], r#"{"state":{"phase":"idle","capture_id":null}}"#, true),
        (vec!["state"], "{}", false),
        (vec!["state"], r#"{"state":{"phase":"future"}}"#, false),
        (vec!["state"], "not-json", false),
        (vec!["state"], r#"{"error":"app_busy"}"#, false),
        (vec!["stop", &id], r#"{"capture_id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","status":"recording","outcome":"wrong_capture"}"#, false),
        (vec!["inspect", &id], r#"{"capture_id":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","status":"recording"}"#, true),
    ] {
        // Darwin's default TMPDIR plus the production socket suffix exceeds SUN_LEN.
        // Keep the private fixture short; never alter the production rendezvous path.
        let home = std::path::PathBuf::from("/tmp").join(format!("ac-e-{}", apparent_capture::new_id().unwrap()));
        let dir = home.join(".config/apparent/agent-control");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let socket = dir.join("capture.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600)).unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(&stream).read_line(&mut request).unwrap();
            writeln!(stream, "{reply}").unwrap();
        });
        let output = Command::new(env!("CARGO_BIN_EXE_apparent-capture"))
            .args(args).env("HOME", &home).output().unwrap();
        server.join().unwrap();
        assert_eq!(output.status.success(), success, "{}", String::from_utf8_lossy(&output.stderr));
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert!(value.is_object());
        std::fs::remove_dir_all(home).unwrap();
    }
}
