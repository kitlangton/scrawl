use crate::Runtime;
use std::io;
#[derive(Clone, Copy)]
pub enum LeaseKind {
    Read,
    Write,
    Finalize,
    Maintenance,
}
/// Identity is a persistent bundle ID, never a pathname. Read/write/finalize coexist;
/// this is a maintenance exclusion mechanism, not a concurrent-writer coordinator.
pub struct BundleLease {
    pub(crate) lock: crate::locking::Lock,
    store: std::sync::Arc<crate::secure::Store>,
}
impl BundleLease {
    pub(crate) fn check(&self) -> io::Result<()> {
        self.lock.check(&self.store)
    }
}
impl Runtime {
    pub fn lease(&self, id: &str, kind: LeaseKind) -> io::Result<Option<BundleLease>> {
        crate::identity::validate_bundle(id)?;
        let _tx = self.transaction()?;
        self.lease_locked(id, kind)
    }
    pub(crate) fn lease_locked(
        &self,
        id: &str,
        kind: LeaseKind,
    ) -> io::Result<Option<BundleLease>> {
        // Preserve pre-existing random-ID lock anchors exactly. Other identities
        // use an injective, reversible encoding in a disjoint permanent namespace.
        // Chunking supports full-length stems without exceeding NAME_MAX.
        let (store, key) = if crate::identity::validate(id, 32).is_ok() {
            (self.store.clone(), format!("bundle-{id}.lock"))
        } else {
            let encoded = crate::identity::bundle_hex(id)?;
            let mut store = self.store.child("bundle-v1")?;
            for chunk in encoded.as_bytes().chunks(120) {
                store = store
                    .child(std::str::from_utf8(chunk).map_err(|_| crate::secure::invalid())?)?;
            }
            (store, "identity.lock".into())
        };
        let lock = crate::locking::Lock::open(&store, &key)?;
        Ok(lock
            .attempt(&store, !matches!(kind, LeaseKind::Maintenance))?
            .then_some(BundleLease { lock, store }))
    }
}
