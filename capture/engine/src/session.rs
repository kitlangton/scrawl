use crate::{Receipt, Status, identity, locking::Lock, receipt::Record, secure::Store};
use std::{io, path::Path, sync::Arc};
#[derive(Clone)]
pub struct Runtime {
    pub(crate) store: Arc<Store>,
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
        self.transition(Status::Recording, Status::Processing)
    }
    pub fn control(&self) -> io::Result<Option<Control>> {
        let _tx = self.runtime.transaction()?;
        Ok(self.load()?.control)
    }
    pub fn finish(self, result: Result<crate::FinalizerSuccess, ()>) -> io::Result<Receipt> {
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
            record.receipt.error = Some("finalization_failed".into());
        }
        record.write(&self.runtime.store)?;
        Ok(record.receipt)
    }
}
impl Runtime {
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
        identity::validate(id, 32)?;
        identity::validate(token, 64)?;
        identity::validate_bundle(bundle_id)?;
        let _tx = self.transaction()?;
        if let Some(record) = Record::read(&self.store, id)? {
            if !identity::equal_secret(token, &record.token)
                || record.receipt.bundle_id != bundle_id
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "request identity or authority mismatch",
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
