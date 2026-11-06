//! Public media/2 controls. Expensive native work stays in the supervised child.
use super::protocol::{self, Authority, Request, Result};
use crate::{
    materials_media::{Checkpoint, JobReference, JobRegistry, StoredJob, StoredJobState},
    media2_worker::Active,
    retained, transport,
};
use apparent_capture_engine::SecureStore;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashSet, VecDeque},
    os::fd::AsRawFd,
    path::Path,
    sync::{Arc, mpsc::RecvTimeoutError},
    time::Duration,
};

fn native_error(error: Box<dyn std::error::Error + Send + Sync>) -> &'static str {
    if let Some(io) = error.downcast_ref::<std::io::Error>() {
        if io.kind() == std::io::ErrorKind::NotFound {
            return "job_not_found";
        }
    }
    match error.to_string().as_str() {
        "job reference mismatch" => "identity_conflict",
        "cancelled" => "cancelled",
        "deadline_exceeded" => "deadline_exceeded",
        "owner_lost" => "owner_lost",
        _ => "source_mismatch",
    }
}
fn reference(job: &StoredJob) -> JobReference {
    JobReference {
        capture_id: job.intent.capture_id.clone(),
        materials: job.intent.materials.clone(),
        lease_id: job.intent.lease_id.clone(),
        job_id: job.intent.job_id.clone(),
        request_sha256: job.intent.request_sha256.clone(),
    }
}
fn status(job: StoredJob) -> Result<Value> {
    let mut result = json!({"jobId":job.intent.job_id,"requestSha256":job.intent.request_sha256,"state":"running"});
    match job.state {
        StoredJobState::Preparing | StoredJobState::Running => (),
        StoredJobState::Cancelled => result["state"] = json!("cancelled"),
        StoredJobState::DeadlineExceeded | StoredJobState::Failed | StoredJobState::OwnerLost => {
            result["state"] = json!("failed");
            result["error"] = json!({"code":match job.state {
                StoredJobState::DeadlineExceeded => "deadline_exceeded",
                StoredJobState::OwnerLost => "owner_lost",
                _ => "decode_failed",
            }});
        }
        StoredJobState::Complete => {
            let receipt = job.receipt.ok_or("source_mismatch")?;
            let bytes = job.envelope.ok_or("source_mismatch")?;
            let envelope: Value = serde_json::from_slice(&bytes).map_err(|_| "source_mismatch")?;
            // Native payloads are emitted as serde_json Values. Re-encode that
            // exact detached payload and verify its native seal before readback.
            let payload = serde_json::to_vec(envelope.get("payload").ok_or("source_mismatch")?)
                .map_err(|_| "source_mismatch")?;
            if payload.len() as u64 != receipt.payload_file.identity.size
                || format!("{:x}", Sha256::digest(&payload)) != receipt.payload_file.sha256
            {
                return Err("source_mismatch");
            }
            result["state"] = json!("complete");
            result["derivative"] = json!({"envelope":envelope,"receiptJson":String::from_utf8(payload).map_err(|_| "source_mismatch")?});
        }
    }
    Ok(result)
}
fn lookup(registry: &JobRegistry, authority: &Authority) -> Result<StoredJob> {
    let job = registry
        .lookup_authenticated(
            &authority.capture_id,
            &authority.job_id,
            &authority.request_sha256,
        )
        .map_err(native_error)?;
    if matches!(
        job.state,
        StoredJobState::Preparing | StoredJobState::Running
    ) {
        registry
            .reconcile_if_ownerless(&reference(&job))
            .map_err(native_error)
    } else {
        Ok(job)
    }
}

fn dispatch_identity(request: &Request, authority: &Authority) -> Result<()> {
    let bytes = request.request_json.as_ref().ok_or("malformed_request")?;
    let value: Value = serde_json::from_str(bytes).map_err(|_| "malformed_request")?;
    if value["materials"]["captureId"].as_str() != Some(&authority.capture_id)
        || value["jobId"].as_str() != Some(&authority.job_id)
    {
        return Err("identity_conflict");
    }
    Ok(())
}
fn handle(
    request: &Request,
    runtime: &Path,
    registry: &JobRegistry,
    active: &mut Option<Active>,
) -> Result<Value> {
    if !cfg!(target_os = "linux") {
        return Err("unavailable");
    }
    let authority = request.authority.as_ref().ok_or("malformed_request")?;
    // Even cold inspection and cancellation authenticate the capture owner
    // before job lookup or disclosure of stored paths/materials/receipt bytes.
    retained::read_authenticated_journal(
        &runtime.join("capture-v2"),
        &authority.capture_id,
        &authority.owner_token,
    )?;
    if request.method == "derive" {
        dispatch_identity(request, authority)?;
        match lookup(registry, authority) {
            Ok(job) => return status(job),
            Err("job_not_found") => (),
            Err(error) => return Err(error),
        }
        if active.is_some() {
            return Err("busy");
        }
        *active = Some(Active::start(
            runtime,
            &authority.capture_id,
            &authority.owner_token,
            request.request_json.as_deref().ok_or("malformed_request")?,
        )?);
    } else if request.method == "cancel" {
        let job = lookup(registry, authority)?;
        let original = reference(&job);
        registry.request_cancel(&original).map_err(native_error)?;
        if let Some(worker) = active
            .as_mut()
            .filter(|worker| worker.reference() == &original)
        {
            worker.stop(Checkpoint::Cancel)?;
            *active = None;
        }
    }
    status(lookup(registry, authority)?)
}

pub fn run(runtime: &Path) -> Result<()> {
    let store = Arc::new(SecureStore::open(runtime).map_err(|_| "source_mismatch")?);
    let registry = JobRegistry::open(store).map_err(native_error)?;
    let fd = std::io::stdout().as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err("internal");
    }
    let inputs = transport::reader(protocol::MAX_LINE);
    let mut client = None;
    let mut seen = HashSet::new();
    let mut order = VecDeque::new();
    let mut active: Option<Active> = None;
    let outcome = (|| loop {
        if let Some(worker) = active.as_mut() {
            if worker.poll()? {
                active = None;
            }
        }
        let bytes = match inputs.recv_timeout(Duration::from_millis(5)) {
            Ok(transport::Input::Line(bytes)) => bytes,
            Ok(transport::Input::Closed) | Err(RecvTimeoutError::Disconnected) => break Ok(()),
            Ok(transport::Input::Invalid) => break Err("malformed_request"),
            Err(RecvTimeoutError::Timeout) => continue,
        };
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| "malformed_request")?;
        let rpc = value
            .get("rpcId")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let request = match protocol::parse(value) {
            Ok(request) => request,
            Err(error) => {
                transport::send(protocol::response(&rpc, Err(error)), protocol::MAX_LINE)?;
                continue;
            }
        };
        let result = if !seen.insert(request.rpc.clone()) {
            Err("malformed_request")
        } else {
            order.push_back(request.rpc.clone());
            if order.len() > 4096 {
                if let Some(old) = order.pop_front() {
                    seen.remove(&old);
                }
            }
            if client.as_ref().is_some_and(|id| id != &request.client) {
                Err("identity_conflict")
            } else if request.method == "handshake" {
                client = Some(request.client.clone());
                Ok(
                    json!({"channel":"media","protocolVersion":2,"implementation":"native","capabilities":{"retainedFrames":cfg!(target_os="linux"),"contactSheet":cfg!(target_os="linux"),"nativeJobAuthority":cfg!(target_os="linux")}}),
                )
            } else if client.is_none() {
                Err("malformed_request")
            } else {
                handle(&request, runtime, &registry, &mut active)
            }
        };
        transport::send(protocol::response(&request.rpc, result), protocol::MAX_LINE)?;
    })();
    if let Some(mut worker) = active {
        worker.stop(Checkpoint::OwnerLost)?;
    }
    outcome
}
