use apparent_capture_engine::{
    Admission, Binding, Control, Receipt as DurableReceipt, Runtime, Status,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashSet, VecDeque};
use std::io::{self, BufRead, Write};
use std::path::{Component, Path, PathBuf};

const MAX_LINE: usize = 64 * 1024;
const RPC_REPLAY_WINDOW: usize = 4096;
const VERSION: u8 = 1;

fn remember_rpc(seen: &mut HashSet<String>, order: &mut VecDeque<String>, id: &str) -> bool {
    if seen.contains(id) {
        return false;
    }
    if order.len() == RPC_REPLAY_WINDOW {
        if let Some(oldest) = order.pop_front() {
            seen.remove(&oldest);
        }
    }
    seen.insert(id.to_owned());
    order.push_back(id.to_owned());
    true
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Envelope {
    protocol_version: u8,
    client_id: String,
    rpc_id: String,
    #[serde(flatten)]
    body: Body,
}
#[derive(Debug, Deserialize)]
#[serde(tag = "method")]
#[serde(rename_all = "camelCase")]
#[serde(rename_all_fields = "camelCase")]
enum Body {
    #[serde(rename = "handshake")]
    Handshake,
    #[serde(rename = "start")]
    Start {
        owner_token: String,
        capture_id: String,
        bundle_id: String,
        bundle_path: String,
        options: Options,
    },
    #[serde(rename = "inspect")]
    Inspect {
        owner_token: String,
        capture_id: String,
    },
    #[serde(rename = "stop")]
    Stop {
        owner_token: String,
        capture_id: String,
    },
    #[serde(rename = "cancel")]
    Cancel {
        owner_token: String,
        capture_id: String,
    },
}
#[derive(Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
struct Options {
    screen: bool,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "snake_case")]
enum ErrorCode {
    MalformedRequest,
    UnsupportedVersion,
    IdentityConflict,
    NotReady,
    NotFound,
    Busy,
    PermissionDenied,
    Unavailable,
    Internal,
}
#[derive(Serialize)]
struct Error {
    code: ErrorCode,
    message: &'static str,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Response<T: Serialize> {
    protocol_version: u8,
    rpc_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Error>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Handshake {
    protocol_version: u8,
    implementation: &'static str,
    capabilities: Capabilities,
    readiness: &'static str,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Capabilities {
    screen_capture: bool,
    audio_capture: bool,
    window_capture: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WireReceipt {
    protocol_version: u8,
    rpc_id: String,
    capture_id: String,
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    bundle_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    artifact_version: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    artifact_provenance: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<Error>,
}

fn ok<T: Serialize>(rpc_id: String, result: T) -> Response<T> {
    Response {
        protocol_version: VERSION,
        rpc_id,
        result: Some(result),
        error: None,
    }
}
fn err<T: Serialize>(rpc_id: String, code: ErrorCode, message: &'static str) -> Response<T> {
    Response {
        protocol_version: VERSION,
        rpc_id,
        result: None,
        error: Some(Error { code, message }),
    }
}
fn valid_hex(s: &str, n: usize) -> bool {
    s.len() == n
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn valid_bundle_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 255
        && s != "."
        && s != ".."
        && !s.chars().any(|c| c.is_control() || c == '/' || c == '\\')
}
fn normalized_absolute(s: &str) -> bool {
    let p = Path::new(s);
    if !p.is_absolute() || s.contains('\0') {
        return false;
    }
    let mut normalized = PathBuf::new();
    for component in p.components() {
        match component {
            Component::RootDir => normalized.push("/"),
            Component::Normal(part) => normalized.push(part),
            _ => return false,
        }
    }
    normalized.to_str() == Some(s)
}
fn state(s: &Status) -> &'static str {
    match s {
        Status::Starting => "starting",
        Status::Recording => "recording",
        Status::Stopping => "stopping",
        Status::Processing => "finalizing",
        Status::Saved => "saved",
        Status::Cancelled => "cancelled",
        Status::Failed => "failed",
        Status::Rejected => "rejected",
    }
}
fn wire_receipt(rpc_id: &str, id: &str, receipt: DurableReceipt) -> WireReceipt {
    let failure = receipt.error.as_deref();
    let status = receipt.status;
    let (error, bundle_path, version, provenance) = match status {
        Status::Failed => (
            Some(Error {
                code: if failure == Some("native_unavailable") {
                    ErrorCode::Unavailable
                } else {
                    ErrorCode::Internal
                },
                message: if failure == Some("native_unavailable") {
                    "Native screen capture is unavailable"
                } else {
                    "Engine operation failed"
                },
            }),
            None,
            None,
            None,
        ),
        Status::Rejected => (
            Some(Error {
                code: ErrorCode::Busy,
                message: "Capture admission was rejected",
            }),
            None,
            None,
            None,
        ),
        Status::Saved => (
            None,
            receipt
                .bundle_path
                .map(|p| p.to_string_lossy().into_owned()),
            Some(1),
            Some("native"),
        ),
        _ => (None, None, None, None),
    };
    WireReceipt {
        protocol_version: VERSION,
        rpc_id: rpc_id.into(),
        capture_id: id.into(),
        state: state(&status),
        bundle_path,
        artifact_version: version,
        artifact_provenance: provenance,
        error,
    }
}

fn process(runtime: &Runtime, envelope: Envelope) -> Response<Value> {
    let rpc_id = envelope.rpc_id.clone();
    if envelope.protocol_version != VERSION {
        return err(
            rpc_id,
            ErrorCode::UnsupportedVersion,
            "Unsupported protocol version",
        );
    }
    if !valid_hex(&envelope.client_id, 32) || !valid_hex(&envelope.rpc_id, 32) {
        return err(
            rpc_id,
            ErrorCode::MalformedRequest,
            "Malformed Engine request",
        );
    }
    match envelope.body {
        Body::Handshake => ok(
            rpc_id,
            serde_json::to_value(Handshake {
                protocol_version: 1,
                implementation: "native",
                capabilities: Capabilities {
                    screen_capture: false,
                    audio_capture: false,
                    window_capture: false,
                },
                readiness: "unavailable",
            })
            .unwrap_or(Value::Null),
        ),
        Body::Start {
            owner_token,
            capture_id,
            bundle_id,
            bundle_path,
            options,
        } => {
            if !valid_hex(&capture_id, 32)
                || !valid_hex(&owner_token, 64)
                || !valid_bundle_id(&bundle_id)
                || !normalized_absolute(&bundle_path)
                || options != (Options { screen: true })
            {
                return err(
                    rpc_id,
                    ErrorCode::MalformedRequest,
                    "Malformed Engine request",
                );
            }
            let binding = Binding {
                bundle_path,
                screen: options.screen,
            };
            match runtime.admit_bound(&capture_id, &owner_token, &bundle_id, binding) {
                Ok(Admission::Acquired(session)) => match session.finish_unavailable() {
                    Ok(receipt) => receipt_response(&rpc_id, &capture_id, receipt),
                    Err(_) => err(rpc_id, ErrorCode::Internal, "Engine operation failed"),
                },
                Ok(Admission::Existing) => {
                    match runtime.inspect_authorized(&capture_id, &owner_token) {
                        Ok(Some(receipt)) => receipt_response(&rpc_id, &capture_id, receipt),
                        Ok(None) => err(rpc_id, ErrorCode::NotFound, "Capture was not found"),
                        Err(error) => map_runtime_error(rpc_id, error),
                    }
                }
                Err(error) => map_runtime_error(rpc_id, error),
            }
        }
        Body::Inspect {
            owner_token,
            capture_id,
        } => control(runtime, &rpc_id, &capture_id, &owner_token, None),
        Body::Stop {
            owner_token,
            capture_id,
        } => control(
            runtime,
            &rpc_id,
            &capture_id,
            &owner_token,
            Some(Control::Stop),
        ),
        Body::Cancel {
            owner_token,
            capture_id,
        } => control(
            runtime,
            &rpc_id,
            &capture_id,
            &owner_token,
            Some(Control::Cancel),
        ),
    }
}
fn receipt_response(rpc: &str, id: &str, receipt: DurableReceipt) -> Response<Value> {
    ok(
        rpc.into(),
        serde_json::to_value(wire_receipt(rpc, id, receipt)).unwrap_or(Value::Null),
    )
}
fn map_runtime_error(rpc: String, error: io::Error) -> Response<Value> {
    match error.kind() {
        io::ErrorKind::PermissionDenied => {
            err(rpc, ErrorCode::PermissionDenied, "Capture authority denied")
        }
        io::ErrorKind::WouldBlock => err(rpc, ErrorCode::NotReady, "Capture is not ready"),
        io::ErrorKind::InvalidInput => err(
            rpc,
            ErrorCode::IdentityConflict,
            "Capture identity conflicts with its original request",
        ),
        _ => err(rpc, ErrorCode::Internal, "Engine operation failed"),
    }
}
fn control(
    runtime: &Runtime,
    rpc: &str,
    id: &str,
    token: &str,
    command: Option<Control>,
) -> Response<Value> {
    if !valid_hex(id, 32) || !valid_hex(token, 64) {
        return err(
            rpc.into(),
            ErrorCode::MalformedRequest,
            "Malformed Engine request",
        );
    }
    let result = match command {
        Some(c) => runtime.request_control(id, token, c).map(Some),
        None => runtime.inspect_authorized(id, token),
    };
    match result {
        Ok(Some(receipt)) => ok(
            rpc.into(),
            serde_json::to_value(wire_receipt(rpc, id, receipt)).unwrap_or(Value::Null),
        ),
        Ok(None) => err(rpc.into(), ErrorCode::NotFound, "Capture was not found"),
        Err(error) => map_runtime_error(rpc.into(), error),
    }
}

fn main() {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.len() != 2 || args[0] != "--runtime-directory" {
        eprintln!("usage: apparent-engine --runtime-directory <absolute-private-directory>");
        std::process::exit(2);
    }
    let path = PathBuf::from(&args[1]);
    if !path.is_absolute() {
        eprintln!("runtime directory must be absolute");
        std::process::exit(2);
    }
    let runtime = match Runtime::open(&path) {
        Ok(r) => r,
        Err(_) => {
            eprintln!("Engine runtime directory is unavailable or unsafe");
            std::process::exit(1);
        }
    };
    let stdin = io::stdin();
    let mut reader = stdin.lock();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    let mut seen = HashSet::new();
    let mut rpc_order = VecDeque::new();
    loop {
        let mut line = Vec::with_capacity(1024);
        let mut terminated = false;
        loop {
            let buf = match reader.fill_buf() {
                Ok(b) => b,
                Err(_) => return,
            };
            if buf.is_empty() {
                break;
            }
            if let Some(pos) = buf.iter().position(|b| *b == b'\n') {
                let chunk = &buf[..pos];
                if line.len() + chunk.len() > MAX_LINE {
                    return;
                }
                line.extend_from_slice(chunk);
                reader.consume(pos + 1);
                terminated = true;
                break;
            }
            if line.len() + buf.len() > MAX_LINE {
                return;
            }
            let n = buf.len();
            line.extend_from_slice(buf);
            reader.consume(n);
        }
        if line.is_empty() && !terminated {
            return;
        }
        if line.last() == Some(&b'\r') {
            line.pop();
        }
        if line.contains(&b'\r') {
            return;
        }
        let raw: Value = match serde_json::from_slice(&line) {
            Ok(v) => v,
            Err(_) => return,
        };
        let rpc = raw
            .get("rpcId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        if !valid_hex(&rpc, 32) || !remember_rpc(&mut seen, &mut rpc_order, &rpc) {
            return;
        }
        let method = raw
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or_default();
        let allowed: &[&str] = match method {
            "handshake" => &["protocolVersion", "clientId", "rpcId", "method"],
            "start" => &[
                "protocolVersion",
                "clientId",
                "rpcId",
                "method",
                "ownerToken",
                "captureId",
                "bundleId",
                "bundlePath",
                "options",
            ],
            "inspect" | "stop" | "cancel" => &[
                "protocolVersion",
                "clientId",
                "rpcId",
                "method",
                "ownerToken",
                "captureId",
            ],
            _ => &[],
        };
        if !raw.as_object().is_some_and(|o| {
            !allowed.is_empty()
                && o.keys().all(|key| allowed.contains(&key.as_str()))
                && allowed.iter().all(|key| o.contains_key(*key))
        }) {
            let response =
                err::<Value>(rpc, ErrorCode::MalformedRequest, "Malformed Engine request");
            if serde_json::to_writer(&mut stdout, &response).is_err()
                || stdout.write_all(b"\n").is_err()
                || stdout.flush().is_err()
            {
                return;
            }
            continue;
        }
        let request: Envelope = match serde_json::from_value(raw) {
            Ok(v) => v,
            Err(_) => {
                let response =
                    err::<Value>(rpc, ErrorCode::MalformedRequest, "Malformed Engine request");
                if serde_json::to_writer(&mut stdout, &response).is_err()
                    || stdout.write_all(b"\n").is_err()
                    || stdout.flush().is_err()
                {
                    return;
                }
                continue;
            }
        };
        let response = process(&runtime, request);
        if serde_json::to_writer(&mut stdout, &response).is_err()
            || stdout.write_all(b"\n").is_err()
            || stdout.flush().is_err()
        {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polling_keeps_a_bounded_replay_window_and_rejects_recent_duplicates() {
        let mut seen = HashSet::new();
        let mut order = VecDeque::new();
        for n in 0..RPC_REPLAY_WINDOW * 3 {
            assert!(remember_rpc(&mut seen, &mut order, &format!("{n:032x}")));
            assert!(seen.len() <= RPC_REPLAY_WINDOW);
            assert!(order.len() <= RPC_REPLAY_WINDOW);
        }
        assert!(!remember_rpc(
            &mut seen,
            &mut order,
            &format!("{:032x}", RPC_REPLAY_WINDOW * 3 - 1)
        ));
        assert!(remember_rpc(&mut seen, &mut order, &format!("{:032x}", 0)));
        assert_eq!(seen.len(), RPC_REPLAY_WINDOW);
    }
}
