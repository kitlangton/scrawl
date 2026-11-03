//! Native admission belongs to one visible presentation, not the child lifetime.
#[derive(Default)]
pub(super) struct Admission {
    epoch: u64,
    presentation: Option<u64>,
    failed: bool,
}

impl Admission {
    pub fn update(&mut self, visible: bool, generation: u64) {
        let next = visible.then_some(generation);
        if next != self.presentation {
            self.epoch += 1;
            self.presentation = next;
            self.failed = false;
        }
    }

    pub fn begin(&self) -> Option<u64> {
        self.presentation.filter(|_| !self.failed).map(|_| self.epoch)
    }

    pub fn current(&self, epoch: u64) -> bool {
        self.begin() == Some(epoch)
    }

    pub fn fail(&mut self, epoch: u64) -> bool {
        if !self.current(epoch) { return false; }
        self.failed = true;
        true
    }

    pub fn healthy(&self) -> bool { !self.failed }
}

/// No admission lock is held across compositor IPC or the retry delay.
pub(super) fn reconcile<E>(
    admission: &std::sync::Mutex<Admission>,
    epoch: u64,
    current: impl Fn() -> bool,
    mut attempt: impl FnMut() -> Result<(), E>,
    mut pause: impl FnMut(),
) -> Option<E> {
    let mut error = None;
    for _ in 0..8 {
        if !current() || !admission.lock().unwrap().current(epoch) { return None; }
        match attempt() {
            Ok(()) => return None,
            Err(failure) => error = Some(failure),
        }
        pause();
    }
    if current() && admission.lock().unwrap().fail(epoch) { error } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancelled_pin_failure_cannot_retire_hidden_or_next_capture() {
        let mut admission = Admission::default();
        admission.update(true, 1);
        let pending = admission.begin().unwrap();
        admission.update(false, 1);
        assert!(!admission.current(pending));
        assert!(!admission.fail(pending));
        assert!(admission.healthy());
        admission.update(true, 2);
        assert!(!admission.fail(pending));
        assert!(admission.healthy());
        let next = admission.begin().unwrap();
        assert!(admission.fail(next), "a genuine new admission failure still falls back");
        assert!(!admission.healthy());
    }

    #[test]
    fn hide_show_same_capture_invalidates_inflight_discovery() {
        let mut admission = Admission::default();
        admission.update(true, 7);
        let pending = admission.begin().unwrap();
        admission.update(false, 7);
        admission.update(true, 7);
        assert!(!admission.fail(pending));
        assert!(admission.begin().is_some());
    }

    #[test]
    fn clock_updates_do_not_extend_native_admission() {
        let mut admission = Admission::default();
        admission.update(true, 3);
        let pending = admission.begin().unwrap();
        for _ in 0..20 { admission.update(true, 3); }
        assert!(admission.fail(pending));
        admission.update(true, 3);
        assert!(!admission.healthy());
        // GUI reconciliation may observe cancellation after the RPC completed,
        // but before it consumes the failure. That result is now obsolete too.
        admission.update(false, 3);
        assert!(admission.healthy());
    }

    #[test]
    fn cancellation_during_last_native_rpc_discards_its_failure() {
        use std::sync::Mutex;
        let state = Mutex::new(Admission::default());
        state.lock().unwrap().update(true, 1);
        let epoch = state.lock().unwrap().begin().unwrap();
        let mut calls = 0;
        let failure = reconcile(&state, epoch, || true, || {
            calls += 1;
            if calls == 8 {
                // Simulate the GUI cancelling/restarting while clients IPC waits.
                let mut state = state.lock().unwrap();
                state.update(false, 1);
                state.update(true, 2);
            }
            Err("not mapped")
        }, || {});
        assert_eq!(calls, 8);
        assert_eq!(failure, None);
        assert!(state.lock().unwrap().healthy());
        let next = state.lock().unwrap().begin().unwrap();
        assert_eq!(reconcile(&state, next, || true, || Ok::<_, &str>(()), || {}), None);
        assert!(state.lock().unwrap().healthy());
        assert_eq!(reconcile(&state, next, || true, || Err("not mapped"), || {}), Some("not mapped"));
        assert!(!state.lock().unwrap().healthy());
    }
}
