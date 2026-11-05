//! One app-owned permissions-only helper shared by the legacy onboarding fallback and the
//! neutral native-host port. The worker keeps process startup and pipe writes off the GUI thread.
use std::os::fd::AsRawFd;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{SyncSender, TrySendError, sync_channel};
use std::sync::{Arc, OnceLock};

use super::Permission;

struct DispatchRequest {
    action: &'static str,
    permission: Permission,
    deadline: Option<u64>,
    reply: Option<SyncSender<String>>,
}

struct Service {
    sender: SyncSender<DispatchRequest>,
    busy: Arc<AtomicBool>,
}

struct HelperProcess {
    child: Child,
    stdin: Option<ChildStdin>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Dispatch {
    Queued,
    Busy,
    Unavailable,
}

static SERVICE: OnceLock<Option<Service>> = OnceLock::new();

fn service() -> Option<&'static Service> {
    SERVICE.get_or_init(|| {
        let (sender, receiver) = sync_channel::<DispatchRequest>(1);
        let busy = Arc::new(AtomicBool::new(false));
        let worker_busy = Arc::clone(&busy);
        std::thread::Builder::new()
            .name("apparent-permission-helper".into())
            .spawn(move || run(receiver, worker_busy))
            .ok()?;
        Some(Service { sender, busy })
    }).as_ref()
}

pub(crate) fn is_busy() -> bool {
    SERVICE.get().and_then(Option::as_ref)
        .is_some_and(|service| service.busy.load(Ordering::Acquire))
}

pub(crate) fn dispatch(
    action: &str,
    permission: Permission,
    deadline: Option<u64>,
    reply: Option<SyncSender<String>>,
) -> Dispatch {
    let Some(action) = normalize_action(action) else { return Dispatch::Unavailable; };
    if deadline.is_some_and(|deadline| storage_clock_ms() >= deadline) {
        return Dispatch::Unavailable;
    }
    let Some(service) = service() else { return Dispatch::Unavailable; };
    if service.busy.compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire).is_err() {
        return Dispatch::Busy;
    }
    let request = DispatchRequest { action, permission, deadline, reply };
    match service.sender.try_send(request) {
        Ok(()) => Dispatch::Queued,
        Err(TrySendError::Full(_)) => {
            service.busy.store(false, Ordering::Release);
            Dispatch::Busy
        }
        Err(TrySendError::Disconnected(_)) => {
            service.busy.store(false, Ordering::Release);
            Dispatch::Unavailable
        }
    }
}

fn normalize_action(action: &str) -> Option<&'static str> {
    match action {
        "request" => Some("request"),
        "open-settings" => Some("open-settings"),
        "close-guide" => Some("close-guide"),
        _ => None,
    }
}

pub(crate) fn close_guide(permission: Permission) {
    let _ = dispatch("close-guide", permission, None, None);
}

fn run(receiver: std::sync::mpsc::Receiver<DispatchRequest>, busy: Arc<AtomicBool>) {
    let mut helper = None;
    while let Ok(request) = receiver.recv() {
        let result = dispatch_one(&mut helper, &request);
        if let Some(reply) = request.reply {
            let _ = reply.send(result.to_string());
        }
        busy.store(false, Ordering::Release);
    }
    if let Some(helper) = helper.as_mut() {
        retire(helper);
    }
}

fn dispatch_one(helper: &mut Option<HelperProcess>, request: &DispatchRequest) -> serde_json::Value {
    if expired(request) {
        return refused_unavailable();
    }
    if helper.as_mut().is_some_and(|helper| matches!(helper.child.try_wait(), Ok(Some(_)) | Err(_))) {
        if let Some(mut old) = helper.take() { retire(&mut old); }
    }
    if helper.is_none() {
        *helper = match start_helper() {
            Ok(helper) => Some(helper),
            Err(_) => return refused_unavailable(),
        };
    }
    if expired(request) {
        return refused_unavailable();
    }
    let message = serde_json::json!({"type":request.action,"permission":request.permission}).to_string();
    let mut frame = message.into_bytes();
    frame.push(b'\n');
    if frame.len() > 256 {
        return serde_json::json!({"status":"outcome-unknown"});
    }
    let stdin = helper.as_mut().expect("helper was started").stdin.as_ref().expect("helper stdin is present");
    let result = unsafe { libc::write(stdin.as_raw_fd(), frame.as_ptr().cast(), frame.len()) };
    if result == frame.len() as isize {
        serde_json::json!({"status":"accepted"})
    } else if result < 0 {
        if let Some(mut dead) = helper.take() { retire(&mut dead); }
        refused_unavailable()
    } else {
        if let Some(mut uncertain) = helper.take() { retire(&mut uncertain); }
        serde_json::json!({"status":"outcome-unknown"})
    }
}

fn start_helper() -> anyhow::Result<HelperProcess> {
    let mut command = Command::new(crate::capture::desktop::helper::path()?);
    crate::capture::ui_process::configure_build_identity(&mut command);
    command.args(["onboarding", "--permissions-only"])
        .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
    let mut child = command.spawn()?;
    let Some(stdin) = child.stdin.take() else {
        let _ = child.kill();
        let _ = child.wait();
        anyhow::bail!("permission helper stdin unavailable");
    };
    let fd = stdin.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } != 0 {
        drop(stdin);
        let _ = child.kill();
        let _ = child.wait();
        anyhow::bail!("permission helper pipe could not be made nonblocking");
    }
    Ok(HelperProcess { child, stdin: Some(stdin) })
}

fn retire(helper: &mut HelperProcess) {
    drop(helper.stdin.take());
    if matches!(helper.child.try_wait(), Ok(None)) { let _ = helper.child.kill(); }
    let _ = helper.child.wait();
}

fn expired(request: &DispatchRequest) -> bool {
    request.deadline.is_some_and(|deadline| storage_clock_ms() >= deadline)
}

fn storage_clock_ms() -> u64 {
    crate::capture::desktop::workflow::agent_capture::storage_clock_ms()
}

fn refused_unavailable() -> serde_json::Value {
    serde_json::json!({"status":"refused","reason":"unavailable"})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_actions_admit_only_known_operations_before_queueing() {
        assert_eq!(normalize_action("request"), Some("request"));
        assert_eq!(normalize_action("open-settings"), Some("open-settings"));
        assert_eq!(normalize_action("close-guide"), Some("close-guide"));
        assert_eq!(normalize_action("unknown"), None);
    }
}
