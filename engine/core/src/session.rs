use crate::{Receipt, Status, identity, locking::Lock, receipt::Record, secure::Store};
use std::{io, path::Path, sync::Arc};
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub bundle_path: String,
    pub screen: bool,
}
#[derive(Clone)]
pub struct Runtime {
    pub(crate) store: Arc<Store>,
}
/// Shared exclusion for native live capture across capture protocol versions.
/// The lock anchor is the same one used by v1 admission.
pub struct LiveCapturePermit {
    store: Arc<Store>,
    lock: Lock,
}
/// Process-independent owner lock for one capture/2 journal. The lock file and
/// its anchor are permanent; dropping this value is the operation's death signal.
pub struct CaptureOwnerPermit {
    _lock: Lock,
}
/// Short metadata transaction used by capture/2's owner-local journal adapter.
pub struct CaptureTransaction {
    _lock: Lock,
}
impl LiveCapturePermit {
    pub fn check(&self) -> io::Result<()> {
        self.lock.check(&self.store)
    }
}
/// Keep alive from before any media side effect through finalizer completion.
/// Dropping releases admission, but does not manufacture a successful receipt.
pub struct Session {
    runtime: Runtime,
    owner: Lock,
    recording: Lock,
    bundle: crate::BundleLease,
    record: Record,
}
pub enum Admission {
    Acquired(Session),
    Existing,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Control {
    Stop,
    Cancel,
}
impl Session {
    fn load(&self) -> io::Result<Record> {
        self.owner.check(&self.runtime.store)?;
        self.recording.check(&self.runtime.store)?;
        self.bundle.check()?;
        let record = Record::read(&self.runtime.store, &self.record.receipt.request_id)?
            .ok_or_else(crate::secure::invalid)?;
        if !identity::equal_secret(&record.token, &self.record.token)
            || record.receipt.bundle_id != self.record.receipt.bundle_id
            || !record.receipt.status.active()
        {
            return Err(crate::secure::invalid());
        }
        Ok(record)
    }
    fn transition(&mut self, from: Status, to: Status) -> io::Result<()> {
        let _tx = self.runtime.transaction()?;
        let mut record = self.load()?;
        if record.receipt.status != from {
            return Err(crate::secure::invalid());
        }
        record.receipt.status = to;
        record.write(&self.runtime.store)?;
        self.record = record;
        Ok(())
    }
    /// Call only after the media adapter reports its first real frame.
    pub fn recording(&mut self) -> io::Result<()> {
        self.transition(Status::Starting, Status::Recording)
    }
    pub fn processing(&mut self) -> io::Result<()> {
        let current = self.load()?.receipt.status;
        match current {
            Status::Stopping => self.transition(Status::Stopping, Status::Processing),
            // Retain the staged package's existing caller contract until its consumers migrate.
            Status::Recording => self.transition(Status::Recording, Status::Processing),
            _ => Err(crate::secure::invalid()),
        }
    }
    /// Persist the stop boundary after the native source has drained its first frame stream.
    pub fn stopping(&mut self) -> io::Result<()> {
        self.transition(Status::Recording, Status::Stopping)
    }
    pub fn control(&self) -> io::Result<Option<Control>> {
        let _tx = self.runtime.transaction()?;
        Ok(self.load()?.control)
    }
    pub fn finish(self, result: Result<crate::FinalizerSuccess, ()>) -> io::Result<Receipt> {
        self.finish_inner(result, "finalization_failed")
    }
    /// Persist a failed admitted attempt when a required native source is unavailable.
    /// No media side effect may have started before using this outcome.
    pub fn finish_unavailable(self) -> io::Result<Receipt> {
        self.finish_inner(Err(()), "native_unavailable")
    }
    fn finish_inner(
        self,
        result: Result<crate::FinalizerSuccess, ()>,
        failure: &str,
    ) -> io::Result<Receipt> {
        let _tx = self.runtime.transaction()?;
        let mut record = self.load()?;
        if record.control == Some(Control::Cancel) {
            record.receipt.status = Status::Cancelled;
            record.receipt.error = Some("cancelled".into());
        } else if let Ok(output) = result {
            if record.receipt.status == Status::Processing
                && output.request == record.receipt.request_id
                && output.bundle == record.receipt.bundle_id
                && output.check().is_ok()
            {
                record.receipt.status = Status::Saved;
                record.receipt.bundle_path = Some(output.path);
            } else {
                record.receipt.status = Status::Failed;
                record.receipt.error = Some("invalid_finalizer_result".into());
            }
        } else {
            record.receipt.status = Status::Failed;
            record.receipt.error = Some(failure.into());
        }
        record.write(&self.runtime.store)?;
        Ok(record.receipt)
    }
}
impl Runtime {
    /// Claim the per-operation owner lock. `None` means another live process
    /// owns this operation; it says nothing about other captures.
    pub fn acquire_capture_owner(&self, id: &str) -> io::Result<Option<CaptureOwnerPermit>> {
        identity::validate(id, 32)?;
        let _tx = self.transaction()?;
        let lock = Lock::open(&self.store, &format!("capture2-owner-{id}.lock"))?;
        if !lock.attempt(&self.store, false)? {
            return Ok(None);
        }
        Ok(Some(CaptureOwnerPermit { _lock: lock }))
    }
    /// Serialize capture/2 journal reads and writes with v1 receipt metadata.
    pub fn capture_transaction(&self) -> io::Result<CaptureTransaction> {
        Ok(CaptureTransaction {
            _lock: self.transaction()?,
        })
    }
    /// Acquire the process-independent live-capture slot without creating a v1 receipt.
    /// Media decoding and other offline jobs must not call this method.
    pub fn acquire_live_capture(&self) -> io::Result<Option<LiveCapturePermit>> {
        let _tx = self.transaction()?;
        let lock = Lock::open(&self.store, "recording.lock")?;
        if !lock.attempt(&self.store, false)? {
            return Ok(None);
        }
        Ok(Some(LiveCapturePermit {
            store: self.store.clone(),
            lock,
        }))
    }

    /// Authenticate a durable operation before recovery can mutate or disclose its receipt.
    pub fn inspect_authorized(&self, id: &str, token: &str) -> io::Result<Option<Receipt>> {
        identity::validate(id, 32)?;
        identity::validate(token, 64)?;
        let _tx = self.transaction()?;
        let Some(record) = Record::read(&self.store, id)? else {
            return Ok(None);
        };
        if !identity::equal_secret(token, &record.token) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "wrong owner authority",
            ));
        }
        Ok(self.read_recover(id)?.map(|r| r.receipt))
    }
    pub fn inspect(&self, id: &str) -> io::Result<Option<Receipt>> {
        identity::validate(id, 32)?;
        let _tx = self.transaction()?;
        Ok(self.read_recover(id)?.map(|r| r.receipt))
    }
    fn read_recover(&self, id: &str) -> io::Result<Option<Record>> {
        let Some(mut record) = Record::read(&self.store, id)? else {
            return Ok(None);
        };
        if record.receipt.status.active() {
            let owner = Lock::open(&self.store, &format!("owner-{id}.lock"))?;
            if owner.attempt(&self.store, false)? {
                record.receipt.status = if record.control == Some(Control::Cancel) {
                    Status::Cancelled
                } else {
                    Status::Failed
                };
                record.receipt.error = Some(
                    if record.control == Some(Control::Cancel) {
                        "cancelled"
                    } else {
                        "owner_interrupted"
                    }
                    .into(),
                );
                record.write(&self.store)?;
            }
        }
        Ok(Some(record))
    }
    /// Persist a command for the live owner to observe. Does not signal a PID or stop media.
    /// Cancellation is sticky. An inspection/readiness timeout never calls this method.
    pub fn request_control(&self, id: &str, token: &str, control: Control) -> io::Result<Receipt> {
        identity::validate(id, 32)?;
        identity::validate(token, 64)?;
        let _tx = self.transaction()?;
        let mut record = Record::read(&self.store, id)?.ok_or_else(crate::secure::invalid)?;
        if !identity::equal_secret(token, &record.token) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "wrong owner authority",
            ));
        }
        if record.receipt.status.active() {
            if control == Control::Stop && record.receipt.status == Status::Starting {
                return Err(io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "capture not ready",
                ));
            }
            if record.control != Some(Control::Cancel) {
                record.control = Some(control);
                if control == Control::Stop && record.receipt.status == Status::Recording {
                    record.receipt.status = Status::Stopping;
                }
            }
            record.write(&self.store)?;
        }
        Ok(record.receipt)
    }
    /// All products MUST use the same stable, caller-provisioned 0700 directory.
    /// It must survive app restarts; do not use a temp directory in production.
    pub fn open(path: &Path) -> io::Result<Self> {
        Ok(Self {
            store: Arc::new(Store::open(path)?),
        })
    }
    pub(crate) fn transaction(&self) -> io::Result<Lock> {
        let lock = Lock::open(&self.store, "metadata.lock")?;
        lock.exclusive(&self.store)?;
        Ok(lock)
    }
    pub fn admit(&self, id: &str, token: &str, bundle_id: &str) -> io::Result<Admission> {
        self.admit_inner(id, token, bundle_id, None)
    }
    /// Provider admission binds every v1 start field before any native side effect.
    pub fn admit_bound(
        &self,
        id: &str,
        token: &str,
        bundle_id: &str,
        binding: Binding,
    ) -> io::Result<Admission> {
        if !binding.screen || !Path::new(&binding.bundle_path).is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid start binding",
            ));
        }
        self.admit_inner(id, token, bundle_id, Some(binding))
    }
    fn admit_inner(
        &self,
        id: &str,
        token: &str,
        bundle_id: &str,
        binding: Option<Binding>,
    ) -> io::Result<Admission> {
        identity::validate(id, 32)?;
        identity::validate(token, 64)?;
        identity::validate_bundle(bundle_id)?;
        let _tx = self.transaction()?;
        if let Some(record) = Record::read(&self.store, id)? {
            if !identity::equal_secret(token, &record.token) {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "wrong owner authority",
                ));
            }
            if record.receipt.bundle_id != bundle_id || record.binding != binding {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "identity conflict",
                ));
            }
            self.read_recover(id)?;
            return Ok(Admission::Existing);
        }
        let owner = Lock::open(&self.store, &format!("owner-{id}.lock"))?;
        if !owner.attempt(&self.store, false)? {
            return Err(crate::secure::invalid());
        }
        let mut record = Record {
            token: token.into(),
            control: None,
            binding,
            receipt: Receipt {
                version: 1,
                request_id: id.into(),
                bundle_id: bundle_id.into(),
                status: Status::Starting,
                bundle_path: None,
                error: None,
            },
        };
        // Durable intent precedes taking admission; a crash cannot retry this ID into a second take.
        record.write(&self.store)?;
        let recording = Lock::open(&self.store, "recording.lock")?;
        if !recording.attempt(&self.store, false)? {
            record.receipt.status = Status::Rejected;
            record.receipt.error = Some("occupied".into());
            record.write(&self.store)?;
            return Ok(Admission::Existing);
        }
        let Some(bundle) = self.lease_locked(bundle_id, crate::LeaseKind::Write)? else {
            record.receipt.status = Status::Rejected;
            record.receipt.error = Some("bundle_busy".into());
            record.write(&self.store)?;
            return Ok(Admission::Existing);
        };
        Ok(Admission::Acquired(Session {
            runtime: self.clone(),
            owner,
            recording,
            bundle,
            record,
        }))
    }
}
