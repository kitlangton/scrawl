//! Native child admission and terminal receipts. No renderer acknowledgement is fabricated.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FinishOutcome {
    Drained,
    UnavailableBeforeAdmission,
}
pub type FinishResult = Result<FinishOutcome, String>;

#[derive(Default)]
pub(super) struct Lifecycle {
    sealed: bool,
    ready: bool,
    admitted: bool,
    startup_protocol_failed: bool,
    terminal: Option<FinishResult>,
    waiters: Vec<flume::Sender<FinishResult>>,
}

impl Lifecycle {
    pub fn admit_open(&mut self) -> bool {
        if self.sealed {
            return false;
        }
        // Conservative even when the bounded actor queue rejects the request.
        self.admitted = true;
        true
    }
    pub fn observed_ready(&mut self) {
        self.ready = true;
    }
    pub fn observed_note_request(&mut self) {
        self.admitted = true;
    }
    pub fn observed_actor_result(&mut self, failed: bool, actor_ready: bool) {
        // A valid JSON message can still violate the startup protocol. Later EOF
        // cannot turn that uncertainty into evidence that no editor started.
        self.startup_protocol_failed |= failed && !actor_ready;
    }
    pub fn seal(&mut self) {
        self.sealed = true;
    }

    pub fn finish(&mut self) -> flume::Receiver<FinishResult> {
        self.seal();
        let (reply, receipt) = flume::bounded(1);
        if let Some(result) = &self.terminal {
            let _ = reply.try_send(result.clone());
        } else {
            self.waiters.push(reply);
        }
        receipt
    }

    pub fn complete(&mut self, drained: bool, failed_writes: bool, startup_exit: bool) {
        self.seal();
        let result = if drained && self.ready && !failed_writes && !self.startup_protocol_failed {
            Ok(FinishOutcome::Drained)
        } else if startup_exit && !self.ready && !self.admitted && !failed_writes && !self.startup_protocol_failed {
            Ok(FinishOutcome::UnavailableBeforeAdmission)
        } else {
            Err("Notes did not finish saving; recording assets were retained.".into())
        };
        for waiter in self.waiters.drain(..) {
            let _ = waiter.try_send(result.clone());
        }
        self.terminal = Some(result);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_exit_without_admission_has_an_explicit_receipt_before_or_after_finish() {
        for finish_first in [false, true] {
            let mut state = Lifecycle::default();
            let receipt = finish_first.then(|| state.finish());
            state.seal();
            assert!(!state.admit_open());
            state.complete(false, false, true);
            let receipt = receipt.unwrap_or_else(|| state.finish());
            assert_eq!(
                receipt.recv().unwrap(),
                Ok(FinishOutcome::UnavailableBeforeAdmission)
            );
        }
    }
    #[test]
    fn admitted_or_ready_or_uncertain_failures_retain_assets() {
        let mut opened = Lifecycle::default();
        assert!(opened.admit_open());
        opened.complete(false, false, true);
        assert!(opened.finish().recv().unwrap().is_err());
        let mut ready = Lifecycle::default();
        ready.observed_ready();
        ready.complete(false, false, true);
        assert!(ready.finish().recv().unwrap().is_err());
        let mut request = Lifecycle::default();
        request.observed_note_request();
        request.complete(false, false, true);
        assert!(request.finish().recv().unwrap().is_err());
        let mut unknown = Lifecycle::default();
        unknown.complete(false, false, false);
        assert!(unknown.finish().recv().unwrap().is_err());
        let mut failed_write = Lifecycle::default();
        failed_write.complete(false, true, true);
        assert!(failed_write.finish().recv().unwrap().is_err());
    }
    #[test]
    fn normal_barrier_and_failed_save_remain_distinct() {
        for failed in [false, true] {
            let mut state = Lifecycle::default();
            state.observed_ready();
            state.observed_note_request();
            let receipt = state.finish();
            assert!(receipt.try_recv().is_err());
            state.complete(true, failed, false);
            assert_eq!(receipt.recv().unwrap().is_ok(), !failed);
        }
    }
    #[test]
    fn open_racing_exit_is_either_rejected_or_blocks_startup_fallback() {
        for _ in 0..64 {
            let state = std::sync::Arc::new(std::sync::Mutex::new(Lifecycle::default()));
            let opening = state.clone();
            let open = std::thread::spawn(move || opening.lock().unwrap().admit_open());
            state.lock().unwrap().complete(false, false, true);
            let admitted = open.join().unwrap();
            let result = state.lock().unwrap().finish().recv().unwrap();
            assert_eq!(result.is_err(), admitted);
        }
    }
    #[test]
    fn an_old_sessions_terminal_result_cannot_complete_its_successor() {
        let mut old = Lifecycle::default();
        let mut new = Lifecycle::default();
        let receipt = new.finish();
        old.complete(false, false, true);
        assert!(receipt.try_recv().is_err());
        new.observed_ready();
        new.complete(true, false, false);
        assert_eq!(receipt.recv().unwrap(), Ok(FinishOutcome::Drained));
    }
}
