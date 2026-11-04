use std::io::{BufRead, BufReader, Read, Write};
#[cfg(test)]
use std::os::fd::AsRawFd;
#[cfg(test)]
use std::os::unix::fs::PermissionsExt;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
#[cfg(test)]
use std::os::unix::net::UnixStream;
#[cfg(test)]
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};

use crate::wire::{Action, DeadlineStream, clock_ms, connect_until, same_user, socket_path, valid_id};

pub fn new_id() -> Result<String> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}
pub fn command(action: Option<(Action, &str)>) -> Result<String> {
    command_for(action, Duration::from_secs(4))
}
fn command_for(action: Option<(Action, &str)>, budget: Duration) -> Result<String> {
    if let Some((_, id)) = action {
        ensure!(valid_id(id), "capture ID must be 32 hex characters");
    }
    let timeout = budget
        .min(Duration::from_secs(4))
        .max(Duration::from_millis(1));
    let deadline_ms = clock_ms() + timeout.as_millis() as u64;
    let path = socket_path(false)?;
    command_at(action, timeout, deadline_ms, &path)
}

pub fn command_at(
    action: Option<(Action, &str)>,
    timeout: Duration,
    deadline_ms: u64,
    path: &std::path::Path,
) -> Result<String> {
    let deadline = Instant::now()
        + timeout.min(Duration::from_millis(
            deadline_ms.saturating_sub(clock_ms()),
        ));
    let meta = std::fs::symlink_metadata(&path).context("no running GUI control socket")?;
    ensure!(
        meta.file_type().is_socket()
            && meta.uid() == unsafe { libc::geteuid() }
            && meta.mode() & 0o777 == 0o600,
        "unsafe control socket"
    );
    let stream = connect_until(path, deadline).context("GUI capture control unavailable")?;
    same_user(&stream)?;
    let mut stream = DeadlineStream { stream, deadline };
    let request = match action {
        None => "state".to_owned(),
        Some((action, id)) => format!(
            "{} {id} {deadline_ms}",
            match action {
                Action::Start => "start",
                Action::Inspect => "inspect",
                Action::Stop => "stop",
                Action::Cancel => "cancel",
                Action::DrawingRead
                | Action::DrawingUndo
                | Action::ReferenceRead
                | Action::ReferenceCopy
                | Action::PublicShareStart
                | Action::PublicShareRead
                | Action::PublicShareDeliver
                | Action::PublicShareCancel => bail!("Typed GUI actions require the GUI protocol"),
            }
        ),
    };
    writeln!(stream, "{request}")?;
    let mut response = String::new();
    BufReader::new(stream).take(4096).read_line(&mut response)?;
    ensure!(
        !response.is_empty(),
        "GUI capture control closed without a reply"
    );
    Ok(response.trim().to_owned())
}

/// One mutation, then read-only polling by the original ID. Never infer ownership
/// from global state and never resend Stop as a toggle after a timeout.
pub fn execute(action: Option<(Action, &str)>, wait: bool, timeout: Duration) -> serde_json::Value {
    execute_with(action, wait, timeout, command_for)
}

pub fn execute_with(
    action: Option<(Action, &str)>,
    wait: bool,
    timeout: Duration,
    mut send: impl FnMut(Option<(Action, &str)>, Duration) -> Result<String>,
) -> serde_json::Value {
    let deadline = Instant::now() + timeout;
    let id = action.map(|(_, id)| id);
    let mut next = action;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return serde_json::json!({"capture_id": id, "error": "timeout", "message": "Inspect the same ID; an admitted capture is not cancelled on timeout."});
        }
        let response = send(next, remaining)
            .and_then(|text| Ok(serde_json::from_str::<serde_json::Value>(&text)?));
        if Instant::now() >= deadline
            || response.as_ref().err().is_some_and(|error| {
                error
                    .downcast_ref::<std::io::Error>()
                    .is_some_and(|error| error.kind() == std::io::ErrorKind::TimedOut)
            })
        {
            return serde_json::json!({"capture_id": id, "error": "timeout", "message": "Inspect the same ID; an admitted capture is not cancelled on timeout."});
        }
        let mut value = match response {
            Ok(value) => value,
            Err(error) => {
                return serde_json::json!({"capture_id": id, "error": "control_unavailable", "message": format!("{error:#}"), "recovery": "Inspect the same ID before retrying; admission may have succeeded."});
            }
        };
        if !value.is_object() {
            return serde_json::json!({"capture_id": id, "error": "invalid_response"});
        }
        if value["error"].is_string() {
            // Listener/transport errors may not yet have reached the GUI thread.
            value["capture_id"] = serde_json::json!(id);
            return value;
        }
        if let Some((action, id)) = action {
            // A response for a different resource is never a successful receipt.
            let status = value["status"].as_str();
            if value["capture_id"].as_str() != Some(id)
                || !matches!(
                    status,
                    Some(
                        "starting"
                            | "recording"
                            | "processing"
                            | "saved"
                            | "cancelled"
                            | "failed"
                            | "rejected"
                            | "unknown"
                    )
                )
                || (status == Some("saved")
                    && !value["bundle_path"].as_str().is_some_and(|p| !p.is_empty()))
            {
                return serde_json::json!({"capture_id": id, "error": "invalid_response"});
            }
            if !wait
                || matches!(
                    value["outcome"].as_str(),
                    Some("wrong_capture" | "not_ready")
                )
            {
                return value;
            }
            if (action == Action::Start && status == Some("recording"))
                || (action == Action::Stop
                    && status == Some("saved")
                    && value["bundle_path"].is_string())
            {
                return value;
            }
            if matches!(
                status,
                Some("cancelled" | "failed" | "rejected" | "unknown" | "saved")
            ) {
                value["error"] = serde_json::json!(match status {
                    Some("cancelled") => "cancelled",
                    Some("unknown") => "unknown_capture",
                    _ => "ended_before_ready",
                });
                return value;
            }
            next = Some((Action::Inspect, id));
        } else {
            if !matches!(value["state"]["phase"].as_str(), Some("idle" | "selecting" | "starting" | "recording" | "processing" | "completed")) {
                return serde_json::json!({"error": "invalid_response"});
            }
            return value;
        }
        std::thread::sleep(
            Duration::from_millis(50).min(deadline.saturating_duration_since(Instant::now())),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::net::UnixListener;
    fn transport_fixture() -> (PathBuf, UnixListener) {
        // Short enough for sun_path, private and unrelated to the live GUI socket.
        let dir = std::env::temp_dir().join(format!("ac-{}", new_id().unwrap()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let path = dir.join("s");
        let listener = UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        (path, listener)
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn transport_full_backlog_obeys_absolute_deadline() {
        let (path, listener) = transport_fixture();
        assert_eq!(unsafe { libc::listen(listener.as_raw_fd(), 1) }, 0);
        let queued = [
            UnixStream::connect(&path).unwrap(),
            UnixStream::connect(&path).unwrap(),
        ];
        // Bound even the buggy test: unblock its blocking connect only after the assertion's
        // budget has been exceeded. No timed-out worker is abandoned.
        let server = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(450));
            drop(queued);
            drop(listener);
        });
        let started = Instant::now();
        let result = command_at(None, Duration::from_millis(80), clock_ms() + 80, &path);
        let elapsed = started.elapsed();
        server.join().unwrap();
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        assert!(result.is_err());
        assert!(
            elapsed < Duration::from_millis(300),
            "connect exceeded budget: {elapsed:?}"
        );
    }

    #[test]
    fn transport_trickled_reply_obeys_absolute_deadline() {
        let (path, listener) = transport_fixture();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = String::new();
            BufReader::new(&stream).read_line(&mut request).unwrap();
            for byte in b"{\"state\":\"idle\"}\n" {
                if stream.write_all(&[*byte]).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(30));
            }
        });
        let started = Instant::now();
        let result = command_at(None, Duration::from_millis(80), clock_ms() + 80, &path);
        let elapsed = started.elapsed();
        server.join().unwrap();
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        assert!(
            result.is_err(),
            "trickle must not restart the timeout: {result:?}"
        );
        assert!(
            elapsed < Duration::from_millis(300),
            "read exceeded budget: {elapsed:?}"
        );
    }

    #[test]
    fn transport_normal_reply_preserves_request_and_peer_checks() {
        let (path, listener) = transport_fixture();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(1)))
                .unwrap();
            let mut request = String::new();
            BufReader::new(&stream).read_line(&mut request).unwrap();
            assert_eq!(request, "state\n");
            stream
                .write_all(b"{\"state\":{\"phase\":\"idle\"}}\n")
                .unwrap();
        });
        let response = command_at(None, Duration::from_secs(1), clock_ms() + 1000, &path);
        server.join().unwrap();
        std::fs::remove_dir_all(path.parent().unwrap()).unwrap();
        assert_eq!(response.unwrap(), "{\"state\":{\"phase\":\"idle\"}}");
    }

    #[test]
    fn transport_stalled_write_obeys_absolute_deadline() {
        let (stream, _peer) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        let started = Instant::now();
        let mut stream = DeadlineStream {
            stream,
            deadline: started + Duration::from_millis(80),
        };
        // More than the socket send queue; peer deliberately never drains it.
        let result = stream.write_all(&vec![0; 4 * 1024 * 1024]);
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_millis(300));
    }

    #[test]
    fn late_reply_cannot_turn_an_expired_wait_into_success() {
        let value = execute_with(
            Some((Action::Start, "owned")),
            true,
            Duration::from_millis(1),
            |_, _| {
                std::thread::sleep(Duration::from_millis(5));
                Ok(serde_json::json!({"capture_id":"owned", "status":"recording"}).to_string())
            },
        );
        assert_eq!(value["error"], "timeout");
        assert_eq!(value["capture_id"], "owned");
    }
    #[test]
    fn wait_timeout_preserves_owned_id_and_retry_reads_terminal_result() {
        let mut stops = 0;
        let value = execute_with(
            Some((Action::Stop, "owned")),
            true,
            Duration::from_millis(5),
            |action, _| {
                stops += usize::from(action.unwrap().0 == Action::Stop);
                Ok(serde_json::json!({"capture_id":"owned", "status":"processing", "outcome":"stopping"}).to_string())
            },
        );
        assert_eq!(value["error"], "timeout");
        assert_eq!(value["capture_id"], "owned");
        assert_eq!(stops, 1);
        let saved = execute_with(
            Some((Action::Stop, "owned")),
            true,
            Duration::from_secs(1),
            |_, _| {
                Ok(serde_json::json!({"capture_id":"owned", "status":"saved", "outcome":"already_ended", "bundle_path":"/exact"}).to_string())
            },
        );
        assert_eq!(saved["bundle_path"], "/exact");
        assert!(saved["error"].is_null());
    }
    #[test]
    fn wait_returns_failures_and_rejects_unknown_or_incomplete_receipts() {
        for (reply, expected) in [
            (
                serde_json::json!({"capture_id":"owned", "status":"failed", "error":"start_failed"}),
                "start_failed",
            ),
            (
                serde_json::json!({"capture_id":"owned", "status":"failed", "error":"finalization_failed"}),
                "finalization_failed",
            ),
            (
                serde_json::json!({"capture_id":"owned", "status":"cancelled"}),
                "cancelled",
            ),
            (
                serde_json::json!({"capture_id":"owned", "status":"unknown", "outcome":"already_ended"}),
                "unknown_capture",
            ),
            (
                serde_json::json!({"capture_id":"foreign", "status":"saved", "bundle_path":"/foreign"}),
                "invalid_response",
            ),
            (serde_json::json!({}), "invalid_response"),
            (
                serde_json::json!({"capture_id":"owned", "status":"saved"}),
                "invalid_response",
            ),
        ] {
            let value = execute_with(
                Some((Action::Stop, "owned")),
                true,
                Duration::from_millis(10),
                |_, _| Ok(reply.to_string()),
            );
            assert_eq!(value["error"], expected, "{reply}");
        }
    }
    #[test]
    fn start_wait_requires_real_readiness_receipt_for_the_owned_id() {
        let mut calls = 0;
        let value = execute_with(
            Some((Action::Start, "owned")),
            true,
            Duration::from_secs(1),
            |action, _| {
                calls += 1;
                assert_eq!(
                    action.unwrap().0,
                    if calls == 1 {
                        Action::Start
                    } else {
                        Action::Inspect
                    }
                );
                Ok(serde_json::json!({"capture_id":"owned", "status":if calls == 1 {"starting"} else {"recording"}}).to_string())
            },
        );
        assert_eq!(value["status"], "recording");
        assert_eq!(calls, 2);
    }
    #[test]
    fn stop_wait_sends_one_stop_then_inspects_only_that_id_until_saved() {
        let mut calls = vec![];
        let replies = [
            serde_json::json!({"capture_id":"owned", "status":"processing", "outcome":"stopping"}),
            serde_json::json!({"capture_id":"owned", "status":"processing"}),
            serde_json::json!({"capture_id":"owned", "status":"saved", "bundle_path":"/captures/exact.apparent"}),
        ];
        let value = execute_with(
            Some((Action::Stop, "owned")),
            true,
            Duration::from_secs(1),
            |action, _| {
                calls.push(action.map(|(a, id)| (a, id.to_owned())));
                Ok(replies[calls.len() - 1].to_string())
            },
        );
        assert_eq!(value["bundle_path"], "/captures/exact.apparent");
        assert_eq!(
            calls,
            vec![
                Some((Action::Stop, "owned".into())),
                Some((Action::Inspect, "owned".into())),
                Some((Action::Inspect, "owned".into()))
            ]
        );
    }
}
