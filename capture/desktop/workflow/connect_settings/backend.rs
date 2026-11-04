//! Finite main-process RPC bridge for the TypeScript Connect controller.
//! It exposes only the existing local backend operations; it owns no policy or state.
use crate::capture::desktop::settings::connect::{
    Approval, Backend, DEFAULT_SERVER, Health, SafeError, Snapshot,
};
use serde::Deserialize;
use std::{
    io::{Read, Write},
    os::{fd::AsRawFd, unix::net::UnixStream},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

const MAX_FRAME: usize = 32768;
const MAX_OPENING: usize = 4096;
const REQUEST_MS: u64 = 3000;
const LEASE: Duration = Duration::from_secs(10);

#[derive(Deserialize)]
#[serde(tag = "connect", rename_all = "kebab-case", deny_unknown_fields)]
enum Opening {
    Backend { deadline: u64, version: u32 },
}
#[derive(Deserialize)]
#[serde(tag = "connect", rename_all = "kebab-case", deny_unknown_fields)]
enum Request {
    BackendRead {
        deadline: u64,
    },
    BackendCommand {
        command: Verb,
        deadline: u64,
        #[serde(default)]
        operation: Option<u64>,
        #[serde(default)]
        origin: Option<String>,
        #[serde(default, rename = "deviceCode")]
        device_code: Option<String>,
        #[serde(default, rename = "timeoutMs")]
        timeout_ms: Option<u64>,
        #[serde(default)]
        site: Option<String>,
    },
}
#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Verb {
    Login,
    DeviceLogin,
    DevicePoll,
    DeviceCommit,
    Probe,
    Cancel,
    Disable,
    Disconnect,
    Activate,
}

pub(super) fn opening_deadline(line: &str) -> Option<u64> {
    if line.len() > MAX_OPENING {
        return None;
    }
    match serde_json::from_str(line).ok()? {
        Opening::Backend {
            deadline,
            version: 1,
        } => Some(deadline),
        Opening::Backend { .. } => None,
    }
}
fn enabled(config: &Path) -> bool {
    super::enabled(config).is_ok()
}
fn clock_ms() -> u64 {
    super::super::agent_capture::storage_clock_ms()
}
fn safe_error(error: SafeError) -> &'static str {
    match error {
        SafeError::Cancelled => "cancelled",
        SafeError::Offline => "offline",
        SafeError::AuthRequired => "auth-required",
        SafeError::Rejected => "rejected",
        SafeError::Expired => "expired",
        SafeError::LocalState => "local-state",
        SafeError::InvalidServer => "invalid-server",
        SafeError::UnverifiedCommit => "unverified-commit",
        SafeError::Unavailable => "unavailable",
    }
}
fn snapshot(value: Option<Snapshot>) -> serde_json::Value {
    value.map(|s| serde_json::json!({
    "server":s.server,"connected":s.connected,"uploadsEnabled":s.uploads_enabled,
    "reauthorizationRequired":s.reauthorization_required,"renewalRequired":s.renewal_required,"accountInitial":s.account_initial,
})).unwrap_or(serde_json::Value::Null)
}
fn envelope(
    backend: &dyn Backend,
    events: Vec<serde_json::Value>,
    operation: Option<u64>,
) -> serde_json::Value {
    let (snap, error) = match backend.snapshot() {
        Ok(s) => (snapshot(Some(s)), serde_json::Value::Null),
        Err(e) => (serde_json::Value::Null, safe_error(e).into()),
    };
    let mut account = serde_json::json!({"version":1,"snapshot":snap,"error":error,"sharedLinksUrl":backend.shared_links_url(),"events":events});
    if let Some(id) = operation {
        account["operation"] = id.into();
    }
    serde_json::json!({"account":account})
}
fn device_result(
    result: anyhow::Result<serde_json::Value>,
    allowed: &dyn Fn() -> bool,
) -> (serde_json::Value, Option<&'static str>) {
    match result {
        Ok(reply) => (reply, None),
        Err(error) => (
            serde_json::Value::Null,
            Some(safe_error(crate::connect::hosted::device_error(
                &error, allowed,
            ))),
        ),
    }
}
fn write(stream: &mut UnixStream, value: &serde_json::Value) -> std::io::Result<()> {
    serde_json::to_writer(&mut *stream, value)?;
    stream.write_all(b"\n")
}

#[derive(Clone)]
struct Receipt {
    id: u64,
    event: serde_json::Value,
}
type FactsReader = Arc<dyn Fn() -> Result<serde_json::Value, SafeError> + Send + Sync>;
type DeviceFactory = Arc<
    dyn Fn(
            &str,
            Arc<dyn Fn() -> bool + Send + Sync>,
        ) -> anyhow::Result<apparent_connect_client::DeviceLogin>
        + Send
        + Sync,
>;

struct Active {
    id: u64,
    verb: Verb,
    active: Arc<AtomicBool>,
    terminal: bool,
    success: bool,
}

pub(crate) fn serve(stream: UnixStream, deadline: u64, config: &Path) {
    run(
        stream,
        deadline,
        config,
        || crate::connect::hosted::settings_backend(config),
        crate::connect::hosted::start_worker,
        LEASE,
        Some(Arc::new({
            let config = config.to_owned();
            move |server, allowed| {
                crate::connect::hosted::begin_device_login(&config, server, allowed)
            }
        })),
        Some(Arc::new({
            let config = config.to_owned();
            move || crate::connect::hosted::settings_facts(&config)
        })),
    )
}
fn run(
    mut stream: UnixStream,
    deadline: u64,
    config: &Path,
    backend_fn: impl FnOnce() -> Arc<dyn Backend>,
    mut worker: impl FnMut(),
    lease_duration: Duration,
    device_factory: Option<DeviceFactory>,
    facts_reader: Option<FactsReader>,
) {
    use std::io::ErrorKind;
    if stream
        .set_read_timeout(Some(Duration::from_millis(50)))
        .is_err()
        || stream
            .set_write_timeout(Some(Duration::from_millis(200)))
            .is_err()
    {
        return;
    }
    if clock_ms() >= deadline {
        super::refuse(&mut stream, "admission_expired");
        return;
    }
    if let Err(code) = super::enabled(config) {
        super::refuse(&mut stream, code);
        return;
    }
    let backend = Arc::new(super::FeatureBackend {
        inner: backend_fn(),
        config: config.to_owned(),
    });
    // The legacy snapshot remains only for older installed client compatibility.
    // Current TypeScript consumers derive their own local state from private facts.
    let envelope = |backend: &dyn Backend, events, operation| {
        let mut reply = envelope(backend, events, operation);
        if let Some(read) = &facts_reader {
            reply["account"]["stateFacts"] = 1.into();
            match read() {
                Ok(facts) => reply["account"]["facts"] = facts,
                Err(error) => {
                    reply["account"]["facts"] = serde_json::Value::Null;
                    reply["account"]["error"] = safe_error(error).into();
                    reply["account"]["snapshot"] = serde_json::Value::Null;
                }
            }
        }
        reply
    };
    let (tx, rx) = mpsc::sync_channel::<Receipt>(64);
    let mut buffer = Vec::new();
    let mut frame_until = None;
    let mut lease = Instant::now() + lease_duration;
    let mut next_gate = Instant::now();
    let mut next_id = 0u64;
    let mut active: Option<Active> = None;
    let mut pending_login: Option<(u64, String)> = None;
    let device_session = Arc::new(std::sync::Mutex::new(
        None::<Arc<apparent_connect_client::DeviceLogin>>,
    ));
    let mut device_busy = false;
    let mut initial = envelope(&*backend, vec![], None);
    if device_factory.is_some() {
        initial["account"]["deviceFlow"] = 1.into();
    }
    if clock_ms() >= deadline {
        super::refuse(&mut stream, "admission_expired");
        return;
    }
    if write(&mut stream, &initial).is_err() {
        return;
    }
    loop {
        if Instant::now() >= lease || frame_until.is_some_and(|d| clock_ms() >= d) {
            break;
        }
        if Instant::now() >= next_gate {
            if !enabled(config) {
                break;
            }
            next_gate = Instant::now() + Duration::from_millis(500);
        }
        let mut fd = libc::pollfd {
            fd: stream.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        let ready = unsafe { libc::poll(&mut fd, 1, 50) };
        if ready < 0 {
            if std::io::Error::last_os_error().kind() == ErrorKind::Interrupted {
                continue;
            }
            break;
        }
        if fd.revents & (libc::POLLERR | libc::POLLNVAL | libc::POLLHUP) != 0 {
            break;
        }
        if ready > 0 {
            let mut bytes = [0u8; 1024];
            match stream.read(&mut bytes) {
                Ok(0) => break,
                Ok(n) => {
                    let frame_deadline =
                        *frame_until.get_or_insert_with(|| clock_ms() + REQUEST_MS);
                    if buffer.len() + n > MAX_FRAME {
                        break;
                    }
                    buffer.extend_from_slice(&bytes[..n]);
                    if let Some(end) = buffer.iter().position(|b| *b == b'\n') {
                        if end + 1 != buffer.len() {
                            break;
                        }
                        let Ok(req) = serde_json::from_slice::<Request>(&buffer[..end]) else {
                            break;
                        };
                        buffer.clear();
                        frame_until = None;
                        let (req_deadline, verb, op, origin, device_code, timeout_ms, site) =
                            match req {
                                Request::BackendRead { deadline } => {
                                    (deadline, None, None, None, None, None, None)
                                }
                                Request::BackendCommand {
                                    command,
                                    deadline,
                                    operation,
                                    origin,
                                    device_code,
                                    timeout_ms,
                                    site,
                                } => (
                                    deadline,
                                    Some(command),
                                    operation,
                                    origin,
                                    device_code,
                                    timeout_ms,
                                    site,
                                ),
                            };
                        if verb.is_some_and(|v| {
                            matches!(v, Verb::Activate | Verb::DevicePoll | Verb::DeviceCommit)
                        }) != op.is_some()
                        {
                            break;
                        }
                        if verb.is_some_and(|v| matches!(v, Verb::Login | Verb::DeviceLogin))
                            != origin.is_some()
                        {
                            break;
                        }
                        let is_poll = verb.is_some_and(|v| matches!(v, Verb::DevicePoll));
                        let is_commit = verb.is_some_and(|v| matches!(v, Verb::DeviceCommit));
                        if is_poll != (device_code.is_some() && timeout_ms.is_some())
                            || is_commit != site.is_some()
                            || (!is_poll && (device_code.is_some() || timeout_ms.is_some()))
                        {
                            break;
                        }
                        let req_deadline = req_deadline.min(frame_deadline);
                        if clock_ms() >= req_deadline || !enabled(config) {
                            break;
                        }
                        let mut events = Vec::new();
                        // Only a live admitted read/command may drain completion receipts.
                        while let Ok(receipt) = rx.try_recv() {
                            if active.as_ref().is_some_and(|a| a.id == receipt.id) {
                                if matches!(
                                    receipt.event["kind"].as_str(),
                                    Some("login" | "probe" | "disable" | "disconnect")
                                ) {
                                    let ok = receipt.event["error"].is_null();
                                    if let Some(a) = active.as_mut() {
                                        a.terminal = true;
                                        a.success = ok;
                                    }
                                }
                                if matches!(
                                    receipt.event["kind"].as_str(),
                                    Some("device-start" | "device-poll" | "device-commit")
                                ) {
                                    device_busy = false;
                                    if !receipt.event["error"].is_null() {
                                        if let Some(a) = active.as_mut() {
                                            a.terminal = true;
                                            a.success = false;
                                        }
                                    }
                                }
                                if receipt.event["kind"] == "device-commit"
                                    && receipt.event["error"].is_null()
                                {
                                    if let Some(a) = active.as_mut() {
                                        a.terminal = true;
                                        a.success = true;
                                    }
                                }
                                events.push(receipt.event);
                            }
                        }
                        // A failed authorization has no activation to acknowledge.
                        // Its receipt is still returned, while later explicit retries
                        // may allocate a new operation on this same owned stream.
                        if active.as_ref().is_some_and(|a| {
                            a.terminal
                                && !a.success
                                && matches!(a.verb, Verb::Login | Verb::DeviceLogin)
                        }) {
                            if let Some(a) = active.take() {
                                a.active.store(false, Ordering::SeqCst);
                            }
                            pending_login = None;
                        }
                        let mut returned_op = None;
                        let mut activated = None;
                        if let Some(verb) = verb {
                            match verb {
                                Verb::Login
                                | Verb::DeviceLogin
                                | Verb::Probe
                                | Verb::Disable
                                | Verb::Disconnect => {
                                    if active.as_ref().is_some_and(|a| {
                                        !a.terminal
                                            && matches!(a.verb, Verb::Disable | Verb::Disconnect)
                                    }) {
                                        // A live durable revocation cannot be cancelled
                                        // or replaced by a duplicate renderer request.
                                        let same = active.as_ref().is_some_and(|a| {
                                            std::mem::discriminant(&a.verb)
                                                == std::mem::discriminant(&verb)
                                        });
                                        let mut reply = envelope(
                                            &*backend,
                                            events,
                                            if same {
                                                active.as_ref().map(|a| a.id)
                                            } else {
                                                None
                                            },
                                        );
                                        if !same {
                                            reply["account"]["error"] = "unavailable".into();
                                        }
                                        if clock_ms() >= req_deadline
                                            || write(&mut stream, &reply).is_err()
                                        {
                                            break;
                                        }
                                        lease = Instant::now() + lease_duration;
                                        continue;
                                    }
                                    let login_server =
                                        if matches!(verb, Verb::Login | Verb::DeviceLogin) {
                                            let snap = backend.snapshot();
                                            if !snap.as_ref().is_ok_and(|s| {
                                                origin.as_deref()
                                                    == Some(
                                                        s.server
                                                            .as_deref()
                                                            .unwrap_or(DEFAULT_SERVER),
                                                    )
                                            }) {
                                                let mut reply = envelope(&*backend, events, None);
                                                if snap.is_ok() {
                                                    reply["account"]["error"] =
                                                        "invalid-server".into();
                                                }
                                                if clock_ms() >= req_deadline
                                                    || write(&mut stream, &reply).is_err()
                                                {
                                                    break;
                                                }
                                                lease = Instant::now() + lease_duration;
                                                continue;
                                            }
                                            origin.clone()
                                        } else {
                                            None
                                        };
                                    if clock_ms() >= req_deadline || !enabled(config) {
                                        break;
                                    }
                                    if matches!(verb, Verb::Disable | Verb::Disconnect) {
                                        if let Some(a) = active.take() {
                                            a.active.store(false, Ordering::SeqCst);
                                        }
                                        pending_login = None;
                                    } else if active.as_ref().is_some_and(|a| {
                                        !a.terminal
                                            || matches!(a.verb, Verb::Login | Verb::DeviceLogin)
                                    }) {
                                        break;
                                    } else {
                                        active = None;
                                    }
                                    let Some(id) = next_id
                                        .checked_add(1)
                                        .filter(|id| *id <= 9_007_199_254_740_991)
                                    else {
                                        break;
                                    };
                                    next_id = id;
                                    returned_op = Some(id);
                                    match verb {
                                        Verb::Login => {
                                            // Pin the origin shown at this explicit
                                            // consent action; never silently redirect
                                            // approval after a concurrent config edit.
                                            let server =
                                                login_server.expect("validated login origin");
                                            pending_login = Some((id, server.clone()));
                                            let active_flag = Arc::new(AtomicBool::new(true));
                                            let allowed_flag = active_flag.clone();
                                            let txc = tx.clone();
                                            let b = backend.clone();
                                            std::thread::spawn(move || {
                                                let allowed = Arc::new(move || {
                                                    allowed_flag.load(Ordering::SeqCst)
                                                });
                                                let approval_tx = txc.clone();
                                                let approval = move |a: Approval| {
                                                    let _=approval_tx.try_send(Receipt{id,event:serde_json::json!({"operation":id,"kind":"approval","approval":{"verificationUri":a.verification_uri,"userCode":a.user_code}})});
                                                };
                                                let result = b.login(&server, allowed, &approval);
                                                let _=txc.try_send(Receipt{id,event:serde_json::json!({"operation":id,"kind":"login","error":result.err().map(safe_error)})});
                                            });
                                            active = Some(Active {
                                                id,
                                                verb,
                                                active: active_flag,
                                                terminal: false,
                                                success: false,
                                            });
                                        }
                                        Verb::DeviceLogin => {
                                            let Some(factory) = device_factory.clone() else {
                                                break;
                                            };
                                            let server =
                                                login_server.expect("validated device origin");
                                            pending_login = Some((id, server.clone()));
                                            let flag = Arc::new(AtomicBool::new(true));
                                            let allowed_flag = flag.clone();
                                            let session = device_session.clone();
                                            *session.lock().unwrap() = None;
                                            let txc = tx.clone();
                                            let config = config.to_owned();
                                            device_busy = true;
                                            std::thread::spawn(move || {
                                                let allowed: Arc<dyn Fn() -> bool + Send + Sync> =
                                                    Arc::new(move || {
                                                        allowed_flag.load(Ordering::SeqCst)
                                                            && enabled(&config)
                                                    });
                                                let result = factory(&server, allowed.clone())
                                                    .and_then(|flow| {
                                                        let flow = Arc::new(flow);
                                                        let reply = flow.start()?;
                                                        *session.lock().unwrap() = Some(flow);
                                                        Ok(reply)
                                                    });
                                                let (reply, error) =
                                                    device_result(result, &*allowed);
                                                let _ = txc.try_send(Receipt { id, event: serde_json::json!({"operation":id,"kind":"device-start","reply":reply,"error":error}) });
                                            });
                                            active = Some(Active {
                                                id,
                                                verb,
                                                active: flag,
                                                terminal: false,
                                                success: false,
                                            });
                                        }
                                        Verb::Probe => {
                                            let flag = Arc::new(AtomicBool::new(true));
                                            let f = flag.clone();
                                            let txc = tx.clone();
                                            let b = backend.clone();
                                            std::thread::spawn(move || {
                                                let h = b.health(Arc::new(move || {
                                                    f.load(Ordering::SeqCst)
                                                }));
                                                let health = match h {
                                                    Health::Online => "online",
                                                    Health::Offline => "offline",
                                                    Health::AuthRequired => "auth-required",
                                                };
                                                let _=txc.try_send(Receipt{id,event:serde_json::json!({"operation":id,"kind":"probe","health":health})});
                                            });
                                            active = Some(Active {
                                                id,
                                                verb,
                                                active: flag,
                                                terminal: false,
                                                success: false,
                                            });
                                        }
                                        Verb::Disable | Verb::Disconnect => {
                                            let logout = matches!(verb, Verb::Disconnect);
                                            let b = backend.clone();
                                            let txc = tx.clone();
                                            std::thread::spawn(move || {
                                                let e = b.disable(logout).err().map(safe_error);
                                                let _=txc.try_send(Receipt{id,event:serde_json::json!({"operation":id,"kind":if logout{"disconnect"}else{"disable"},"error":e})});
                                            });
                                            active = Some(Active {
                                                id,
                                                verb,
                                                active: Arc::new(AtomicBool::new(true)),
                                                terminal: false,
                                                success: false,
                                            });
                                        }
                                        Verb::Cancel => unreachable!(),
                                        Verb::Activate | Verb::DevicePoll | Verb::DeviceCommit => {
                                            unreachable!()
                                        }
                                    }
                                }
                                Verb::DevicePoll | Verb::DeviceCommit => {
                                    if device_busy
                                        || !active.as_ref().is_some_and(|a| {
                                            Some(a.id) == op
                                                && matches!(a.verb, Verb::DeviceLogin)
                                                && !a.terminal
                                        })
                                    {
                                        break;
                                    }
                                    let Some(flow) = device_session.lock().unwrap().clone() else {
                                        break;
                                    };
                                    returned_op = op;
                                    let a = active.as_ref().unwrap();
                                    let id = a.id;
                                    let flag = a.active.clone();
                                    let config = config.to_owned();
                                    let txc = tx.clone();
                                    let b = backend.clone();
                                    device_busy = true;
                                    std::thread::spawn(move || {
                                        let allowed =
                                            || flag.load(Ordering::SeqCst) && enabled(&config);
                                        let result = if matches!(verb, Verb::DevicePoll) {
                                            flow.poll(
                                                device_code.as_deref().unwrap(),
                                                timeout_ms.unwrap(),
                                            )
                                        } else {
                                            flow.commit(site.as_deref().unwrap())
                                                .map(|_| serde_json::Value::Null)
                                        };
                                        // An uncertain or partial credential/config commit must
                                        // settle durable off before returning a failed login.
                                        if matches!(verb, Verb::DeviceCommit) && result.is_err() {
                                            let _ = b.disable(false);
                                        }
                                        let (reply, error) = device_result(result, &allowed);
                                        let kind = if matches!(verb, Verb::DevicePoll) {
                                            "device-poll"
                                        } else {
                                            "device-commit"
                                        };
                                        let _ = txc.try_send(Receipt { id, event: serde_json::json!({"operation":id,"kind":kind,"reply":reply,"error":error}) });
                                    });
                                }
                                Verb::Cancel => {
                                    if let Some(a) = active.take() {
                                        if matches!(
                                            a.verb,
                                            Verb::Login | Verb::DeviceLogin | Verb::Probe
                                        ) {
                                            a.active.store(false, Ordering::SeqCst);
                                        } else {
                                            active = Some(a)
                                        }
                                    }
                                }
                                Verb::Activate => {
                                    if op.is_none() {
                                        break;
                                    }
                                    let valid =
                                        active.as_ref().is_some_and(|a| {
                                            Some(a.id) == op
                                                && matches!(a.verb, Verb::Login | Verb::DeviceLogin)
                                                && a.terminal
                                                && a.success
                                        }) && pending_login.as_ref().is_some_and(|(id, server)| {
                                            Some(*id) == op
                                                && enabled(config)
                                                && backend.snapshot().is_ok_and(|s| {
                                                    s.connected
                                                        && s.uploads_enabled
                                                        && s.server.as_deref()
                                                            == Some(server.as_str())
                                                })
                                        });
                                    let valid =
                                        valid && clock_ms() < req_deadline && enabled(config);
                                    activated = Some(valid);
                                    if !valid {
                                        // Explicit negative acknowledgement; do not
                                        // synthesize another login terminal receipt.
                                    } else {
                                        worker();
                                        if let Some(a) = active.take() {
                                            a.active.store(false, Ordering::SeqCst);
                                        }
                                        pending_login = None;
                                    }
                                }
                            }
                        }
                        // Include only receipts collected at the start of this admitted turn.
                        let mut reply = envelope(&*backend, events, returned_op);
                        if device_factory.is_some() {
                            reply["account"]["deviceFlow"] = 1.into();
                        }
                        if let Some(activated) = activated {
                            reply["account"]["activated"] = activated.into();
                        }
                        if clock_ms() >= req_deadline {
                            break;
                        }
                        if write(&mut stream, &reply).is_err() {
                            break;
                        }
                        lease = Instant::now() + lease_duration;
                    }
                }
                Err(e)
                    if matches!(
                        e.kind(),
                        ErrorKind::TimedOut | ErrorKind::WouldBlock | ErrorKind::Interrupted
                    ) => {}
                Err(_) => break,
            }
        }
    }
    // A login remains revocable until explicit activation, including after its terminal receipt.
    if let Some(a) = active.take() {
        if matches!(a.verb, Verb::Login | Verb::DeviceLogin | Verb::Probe) {
            a.active.store(false, Ordering::SeqCst);
        }
    }
    if pending_login.is_some() {
        let b = backend.clone();
        std::thread::spawn(move || {
            let _ = b.disable(false);
        });
    }
    // Durable revocations run detached and are never cancelled on EOF.
}

#[cfg(test)]
#[path = "backend_tests.rs"]
mod lifecycle_tests;

#[cfg(test)]
mod tests {
    use super::{MAX_FRAME, opening_deadline};
    #[test]
    fn backend_opening_is_exact_and_bounded() {
        assert_eq!(
            opening_deadline(r#"{"connect":"backend","version":1,"deadline":123}"#),
            Some(123)
        );
        assert_eq!(
            opening_deadline(r#"{"connect":"backend","version":1,"deadline":123,"path":"/tmp/x"}"#),
            None
        );
        assert_eq!(
            opening_deadline(r#"{"connect":"backend","version":2,"deadline":123}"#),
            None
        );
        assert_eq!(
            opening_deadline(r#"{"connect":"backend","deadline":123}"#),
            None
        );
        assert_eq!(
            opening_deadline(r#"{"connect":"settings","deadline":123}"#),
            None
        );
        assert_eq!(
            opening_deadline(&format!(
                "{{\"connect\":\"backend\",\"version\":1,\"deadline\":123}}{}",
                " ".repeat(MAX_FRAME)
            )),
            None
        );
    }
}
