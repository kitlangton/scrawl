//! Kernel-held, close-on-exec locks. Lock names and their identity anchors are never removed.
use crate::secure::{Store, invalid};
use std::fs::{File, TryLockError};
use std::io;
use std::os::unix::fs::MetadataExt;

pub(crate) struct Lock {
    file: File,
    key: String,
}
impl Lock {
    pub fn open(store: &Store, key: &str) -> io::Result<Self> {
        let file = match store.file(key, true) {
            Ok(f) => {
                // Incomplete bootstrap fails closed rather than repairing a possibly live lock.
                store.link(key, &format!("anchor-{key}"))?;
                f
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => store.file(key, false)?,
            Err(e) => return Err(e),
        };
        let lock = Self {
            file,
            key: key.into(),
        };
        lock.check(store)?;
        Ok(lock)
    }
    pub fn check(&self, store: &Store) -> io::Result<()> {
        let m = self.file.metadata()?;
        for key in [&self.key, &format!("anchor-{}", self.key)] {
            let f = store.file(key, false)?;
            let n = f.metadata()?;
            if m.dev() != n.dev() || m.ino() != n.ino() || n.nlink() != 2 {
                return Err(invalid());
            }
        }
        Ok(())
    }
    pub fn exclusive(&self, store: &Store) -> io::Result<()> {
        self.file.lock()?;
        self.check(store)
    }
    pub fn attempt(&self, store: &Store, shared: bool) -> io::Result<bool> {
        let result = if shared {
            self.file.try_lock_shared()
        } else {
            self.file.try_lock()
        };
        match result {
            Ok(()) => {
                self.check(store)?;
                Ok(true)
            }
            Err(TryLockError::WouldBlock) => {
                self.check(store)?;
                Ok(false)
            }
            Err(TryLockError::Error(e)) => Err(e),
        }
    }
}
// No explicit unlock: close drops only this open file description. A forked copy intentionally
// keeps the kernel lock until *all* copies close. Consumers must not fork a media child without exec.
