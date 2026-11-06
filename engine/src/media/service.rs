use super::transport::Input;
use super::{
    job::Active,
    protocol::{self, Decode, Result},
};
use apparent_capture_engine::Runtime;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet, VecDeque};
use std::os::fd::AsRawFd;
use std::path::Path;
use std::sync::mpsc::RecvTimeoutError;
use std::time::{Duration, Instant};

struct Job {
    input: Decode,
    status: Value,
}
fn send(value: Value) -> Result<()> {
    super::transport::send(value, 65536)
}
fn reply(rpc: &str, result: Result<Value>) -> Result<()> {
    send(protocol::response(rpc, result))
}

pub fn run(runtime_path: &Path) -> Result<()> {
    let runtime = Runtime::open(runtime_path).map_err(|_| "source_mismatch")?;
    // A blocked consumer must not prevent supervising the native child forever.
    let fd = std::io::stdout().as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err("internal");
    }
    let inputs = super::transport::reader(65536);
    let mut client = None;
    let mut seen = HashSet::new();
    let mut order = VecDeque::new();
    let mut jobs: HashMap<String, Job> = HashMap::new();
    let mut active: Option<Active> = None;
    let outcome = (|| loop {
        if let Some(a) = active.as_mut() {
            let expired = a.started.elapsed().as_millis() >= u128::from(a.input.limits.deadline_ms);
            if a.cancelled || expired {
                a.stop();
            }
            let exit = a.child.try_wait().map_err(|_| "internal")?;
            if let Some(exit) = exit {
                let mut a = active.take().ok_or("internal")?;
                let result = if a.cancelled {
                    a.cleanup();
                    Err("cancelled")
                } else if expired {
                    a.cleanup();
                    Err("deadline_exceeded")
                } else {
                    a.finish(exit.success())
                };
                let status = match &result {
                    Ok(v) => v.clone(),
                    Err(e) => protocol::failed(&a.input.job_id, e),
                };
                jobs.get_mut(&a.input.job_id).ok_or("internal")?.status = status;
                reply(&a.rpc, result)?;
            }
        }
        let input = match inputs.recv_timeout(Duration::from_millis(5)) {
            Ok(Input::Line(b)) => b,
            Ok(Input::Closed) | Err(RecvTimeoutError::Disconnected) => break Ok(()),
            Ok(Input::Invalid) => break Err("malformed_request"),
            Err(RecvTimeoutError::Timeout) => continue,
        };
        let raw: Value = match serde_json::from_slice(&input) {
            Ok(v) => v,
            Err(_) => break Err("malformed_request"),
        };
        let fallback = raw
            .get("rpcId")
            .and_then(Value::as_str)
            .filter(|s| protocol::hex(s))
            .unwrap_or("")
            .to_owned();
        let r = match protocol::parse(raw) {
            Ok(r) => r,
            Err(e) => {
                if fallback.is_empty() {
                    break Err(e);
                }
                reply(&fallback, Err(e))?;
                continue;
            }
        };
        if !seen.insert(r.rpc.clone()) {
            reply(&r.rpc, Err("malformed_request"))?;
            continue;
        }
        order.push_back(r.rpc.clone());
        if order.len() > 4096 {
            if let Some(old) = order.pop_front() {
                seen.remove(&old);
            }
        }
        if let Some(expected) = &client {
            if expected != &r.client {
                reply(&r.rpc, Err("identity_conflict"))?;
                continue;
            }
        }
        if r.method == "handshake" {
            client = Some(r.client);
            reply(
                &r.rpc,
                Ok(
                    json!({"channel":"media","protocolVersion":1,"implementation":"native","capabilities":{"decodeFrames":super::job::decode_available()}}),
                ),
            )?;
            continue;
        }
        if client.is_none() {
            reply(&r.rpc, Err("malformed_request"))?;
            continue;
        }
        let id = r.job.as_deref().ok_or("malformed_request")?;
        match r.method.as_str() {
            "decode" => {
                let d = r.decode.ok_or("malformed_request")?;
                if let Some(previous) = jobs.get(id) {
                    reply(
                        &r.rpc,
                        Err(if previous.input == d {
                            "busy"
                        } else {
                            "identity_conflict"
                        }),
                    )?;
                } else if jobs.len() >= 32 || active.is_some() {
                    reply(&r.rpc, Err("busy"))?;
                } else {
                    let status = json!({"jobId":id,"state":"decoding"});
                    jobs.insert(
                        id.into(),
                        Job {
                            input: d.clone(),
                            status,
                        },
                    );
                    match Active::start(d, r.rpc.clone(), runtime_path, &runtime) {
                        Ok(a) => active = Some(a),
                        Err(e) => {
                            jobs.get_mut(id).ok_or("internal")?.status = protocol::failed(id, e);
                            reply(&r.rpc, Err(e))?;
                        }
                    }
                }
            }
            "inspect" => {
                reply(
                    &r.rpc,
                    jobs.get(id)
                        .map(|j| j.status.clone())
                        .ok_or("job_not_found"),
                )?;
            }
            "cancel" => {
                if let Some(j) = jobs.get_mut(id) {
                    if let Some(a) = active.as_mut().filter(|a| a.input.job_id == id) {
                        a.cancelled = true;
                        j.status = json!({"jobId":id,"state":"cancelling"});
                    }
                    reply(&r.rpc, Ok(json!({"jobId":id,"state":j.status["state"]})))?;
                } else {
                    reply(&r.rpc, Err("job_not_found"))?;
                }
            }
            _ => unreachable!(),
        }
    })();
    if let Some(mut a) = active {
        a.stop();
        a.cleanup();
    }
    outcome
}
