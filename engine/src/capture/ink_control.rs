//! Capture-owned dispatch and bounded pre-material ink barrier. No native UI policy.
use super::*;
pub fn handle(req: &Value, shared: &Shared, live_owner: bool) -> Result<Value, &'static str> {
    let mut j = shared.journal.lock().map_err(|_| "internal")?;
    if req["ownerToken"] != j.owner_token {
        return Err("permission_denied");
    }
    // Same durable transaction as Stop/Cancel: an external control peer may
    // already have ended admission even while this owner's memory is recording.
    let _transaction = shared
        .runtime
        .capture_transaction()
        .map_err(|_| "internal")?;
    let durable = read_journal(&shared.directory, &j.capture_id, &j.owner_token)?;
    if !["recording", "stopping"].contains(&durable.state.as_str()) {
        return Err("unavailable");
    }
    j.cancel_requested |= durable.cancel_requested;
    j.ink_cutoff_ns = durable.ink_cutoff_ns.or(j.ink_cutoff_ns);
    if durable.state == "stopping" {
        j.state = "stopping".into();
    }
    if !live_owner || !["recording", "stopping"].contains(&j.state.as_str()) || j.cancel_requested {
        return Err("unavailable");
    }
    let now = monotonic_ns().map_err(|_| "internal")?;
    let source = j.request["source"].clone();
    if req["ingressVersion"] != 1 {
        return Err("unsupported_version");
    }
    if req["method"] == "registerInkProducer" {
        if req["source"] != json!({"id":source["id"], "generation":source["generation"]}) {
            return Err("source_changed");
        }
        if j.state != "recording" || j.ink_cutoff_ns.is_some() {
            return Err("unavailable");
        }
        if let Some(r) = &j.ink_registration {
            if r.matches(req) {
                return Ok(r.receipt(&j.capture_id));
            }
            return Err("identity_conflict");
        }
        let boot =
            fs::read_to_string("/proc/sys/kernel/random/boot_id").map_err(|_| "unavailable")?;
        let registration = ink::Registration::new(
            req,
            &source,
            j.admitted_monotonic_ns.ok_or("unavailable")?,
            boot.trim(),
        )?;
        let result = registration.receipt(&j.capture_id);
        j.ink_registration = Some(registration);
        write_locked(shared, &j)?;
        return Ok(result);
    }
    let cutoff = j.ink_cutoff_ns.unwrap_or(now);
    let expired = j
        .ink_cutoff_ns
        .is_some_and(|n| now > n.saturating_add(ink::GRACE_NS));
    let r = j.ink_registration.as_mut().ok_or("unavailable")?;
    // Authenticate before invalidating a producer: a foreign secret is not an
    // instruction to discard accepted history. The mapper repeats this check.
    r.authenticate(req)?;
    if expired && r.collecting() {
        r.invalidate()
    }
    let result = if req["method"] == "submitInkEvents" {
        r.submit(req, cutoff, &source)
    } else {
        r.finish(req, now)
    };
    if result.is_err() {
        r.invalidate();
    }
    // Persist both acceptance and authenticated validation failure before reply.
    // On failed persistence the connection outcome is unknown, never an ACK.
    write_locked(shared, &j)?;
    result
}
fn write_locked(shared: &Shared, j: &Journal) -> Result<(), &'static str> {
    apparent_capture_engine::SecureStore::open(&shared.directory)
        .map_err(|_| "internal")?
        .replace_checked(
            &path_id(&j.capture_id),
            &serde_json::to_vec(j).map_err(|_| "internal")?,
        )
        .map_err(|_| "internal")
}
pub fn close_input(shared: &Shared) -> Result<(), &'static str> {
    let mut j = shared.journal.lock().map_err(|_| "internal")?;
    let _transaction = shared
        .runtime
        .capture_transaction()
        .map_err(|_| "internal")?;
    let current = read_journal(&shared.directory, &j.capture_id, &j.owner_token)?;
    j.ink_cutoff_ns = current
        .ink_cutoff_ns
        .or(j.ink_cutoff_ns)
        .or(Some(monotonic_ns().map_err(|_| "internal")?));
    j.cancel_requested |= current.cancel_requested;
    if current.state == "stopping" {
        j.state = "stopping".into();
    }
    // Never replace a terminal journal if an external owner ended the capture.
    if !["recording", "stopping"].contains(&current.state.as_str()) {
        return Err("unavailable");
    }
    write_locked(shared, &j)
}
pub fn drain(shared: &Shared) -> Result<(), &'static str> {
    close_input(shared)?;
    loop {
        let mut j = shared.journal.lock().map_err(|_| "internal")?;
        let end = j
            .ink_cutoff_ns
            .ok_or("internal")?
            .saturating_add(ink::GRACE_NS);
        let Some(r) = j.ink_registration.as_mut() else {
            return Ok(());
        };
        if !r.collecting() {
            return Ok(());
        }
        if shared.cancel.load(Ordering::Acquire) || monotonic_ns().map_err(|_| "internal")? >= end {
            r.invalidate();
            persist_journal(shared, &j).map_err(|_| "internal")?;
            return Ok(());
        }
        drop(j);
        thread::sleep(Duration::from_millis(5));
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (PathBuf, Shared, Value) {
        let (base, directory, _, mut j) = super::super::unsealed_cleanup_tests::fixture();
        let now = monotonic_ns().unwrap();
        let source = json!({"id":"display:1","generation":"g","kind":"display","name":"DP-1","bounds":{"x":0,"y":0,"width":640,"height":360,"scale":1}});
        j.request["source"] = source.clone();
        j.cancel_requested = false;
        j.admitted_monotonic_ns = Some(now - 1_000_000);
        let rect = json!({"x":0,"y":0,"width":640,"height":360});
        let req = json!({"method":"registerInkProducer","ingressVersion":1,"ownerToken":j.owner_token,"captureId":j.capture_id,"source":{"id":"display:1","generation":"g"},"observation":{"host":{"processEpoch":"epoch","revision":0,"pid":1,"startTicks":"1"},"producerId":"c".repeat(32),"generation":1,"bootId":fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap().trim(),"clock":"linux-clock-monotonic-observation","output":{"name":"DP-1","physical":rect,"compositor":source["bounds"]},"surfaces":[{"surfaceId":"screen","transformId":"t","nativeIncarnation":"i","target":"capture-rectangle","pointSpace":"desktop-physical","contentRectangle":rect,"plane":{"width":640,"height":360}}]}});
        let shared = Shared {
            journal: Mutex::new(j),
            directory,
            runtime: Runtime::open(&base).unwrap(),
            stop: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
            epoch: Instant::now(),
        };
        persist_new(&shared).unwrap();
        (base, shared, req)
    }
    #[test]
    fn durable_owner_register_finish_original_fact_and_exact_cleanup() {
        let (base, shared, req) = fixture();
        let mut wrong = req.clone();
        wrong["ownerToken"] = json!("f".repeat(64));
        assert_eq!(handle(&wrong, &shared, true), Err("permission_denied"));
        assert_eq!(handle(&req, &shared, false), Err("unavailable"));
        let registration = handle(&req, &shared, true).unwrap();
        assert_eq!(handle(&req, &shared, true).unwrap(), registration);
        let n = shared
            .journal
            .lock()
            .unwrap()
            .admitted_monotonic_ns
            .unwrap();
        let events = json!([
            {"kind":"begin","gestureId":"d".repeat(32),"surfaceId":"screen","transformId":"t","nativeIncarnation":"i","timeNs":n.to_string(),"x":100,"y":100,"rgb":[12,34,56]},
            {"kind":"points","gestureId":"d".repeat(32),"points":[{"timeNs":(n+10).to_string(),"x":200,"y":100}]},
            {"kind":"commit","gestureId":"d".repeat(32),"acceptedAtNs":(n+20).to_string()}
        ]);
        let raw = serde_json::to_string(&events).unwrap();
        let submit = json!({"method":"submitInkEvents","ingressVersion":1,"ownerToken":req["ownerToken"],"captureId":req["captureId"],"registrationId":registration["registrationId"],"producerToken":registration["producerToken"],"sequence":0,"eventsJson":raw,"batchSha256":sha(raw.as_bytes())});
        assert_eq!(
            handle(&submit, &shared, true).unwrap()["outcome"],
            "accepted"
        );
        assert_eq!(
            handle(&submit, &shared, true).unwrap()["outcome"],
            "replayed"
        );
        let now = monotonic_ns().unwrap();
        let finish = json!({"method":"finishInkProducer","ingressVersion":1,"ownerToken":req["ownerToken"],"captureId":req["captureId"],"registrationId":registration["registrationId"],"producerToken":registration["producerToken"],"finalSequence":0,"seal":{"state":"sealed-and-drained","observedAtNs":now.to_string(),"nativeRevision":0}});
        assert_eq!(handle(&finish, &shared, true).unwrap()["state"], "complete");
        let cold = read_journal(
            &shared.directory,
            req["captureId"].as_str().unwrap(),
            req["ownerToken"].as_str().unwrap(),
        )
        .unwrap();
        let fact = cold
            .ink_registration
            .unwrap()
            .facts(req["captureId"].as_str().unwrap(), &"e".repeat(32))
            .unwrap();
        let sealed = seal_fact(&shared, "accepted-strokes.json", &fact).unwrap();
        let bytes = operation_dir(&shared)
            .unwrap()
            .read("accepted-strokes.json")
            .unwrap()
            .unwrap();
        assert_eq!(sha(&bytes), sealed["sha256"]);
        assert_eq!(fact["strokes"].as_array().unwrap().len(), 1);
        if let Some(path) = std::env::var_os("APPARENT_TEST_INK_READER_FIXTURE") {
            use std::os::unix::fs::OpenOptionsExt;
            let mut output = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)
                .unwrap();
            output
                .write_all(
                    &serde_json::to_vec(
                        &json!({"seal":sealed,"bytes":String::from_utf8(bytes.clone()).unwrap()}),
                    )
                    .unwrap(),
                )
                .unwrap();
            output.sync_all().unwrap();
        }
        let cold = read_journal(
            &shared.directory,
            req["captureId"].as_str().unwrap(),
            req["ownerToken"].as_str().unwrap(),
        )
        .unwrap();
        assert_eq!(cold.fact_files["accepted-strokes.json"], sealed);
        remove_unsealed(&shared.directory, &cold).unwrap();
        drop(shared);
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn durable_stop_cutoff_and_expired_drain_never_fake_collected_empty() {
        let (base, shared, req) = fixture();
        let registration = handle(&req, &shared, true).unwrap();
        let mut cold = read_journal(
            &shared.directory,
            req["captureId"].as_str().unwrap(),
            req["ownerToken"].as_str().unwrap(),
        )
        .unwrap();
        cold.state = "stopping".into();
        cold.ink_cutoff_ns = Some(monotonic_ns().unwrap() - ink::GRACE_NS - 1);
        persist_journal(&shared, &cold).unwrap();
        let finish = json!({"method":"finishInkProducer","ingressVersion":1,"ownerToken":req["ownerToken"],"captureId":req["captureId"],"registrationId":registration["registrationId"],"producerToken":registration["producerToken"],"finalSequence":null,"seal":{"state":"sealed-and-drained","observedAtNs":monotonic_ns().unwrap().to_string(),"nativeRevision":0}});
        assert_eq!(
            handle(&finish, &shared, true).unwrap()["state"],
            "unavailable"
        );
        drain(&shared).unwrap();
        assert!(
            shared
                .journal
                .lock()
                .unwrap()
                .ink_registration
                .as_ref()
                .unwrap()
                .facts("c", "m")
                .is_none()
        );
        let cold = shared.journal.lock().unwrap().clone();
        remove_unsealed(&shared.directory, &cold).unwrap();
        drop(shared);
        fs::remove_dir_all(base).unwrap();
    }
}
