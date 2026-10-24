//! Settings and the welcome window live as long as Scrawl, but each `Session` that drives one
//! has its own reader thread. A replaced session's reader can still be working through its
//! "close" after the new session has attached. So each attach takes a new id, and a reader's
//! updates, opens and hides reach the window only while its id is the window's.

use std::marker::PhantomData;
use std::sync::atomic::{AtomicU64, Ordering};

use rune::{AppHandle, Tx};

/// What a reader drives: the window's `AppHandle`, or a recorder in tests.
pub(crate) trait Host<S>: Clone + Send + 'static {
    fn update(&self, f: impl FnOnce(&mut S, &mut Tx) + Send + 'static);
    fn open(&self);
    fn hide(&self);
}

impl<S: 'static> Host<S> for AppHandle<S> {
    fn update(&self, f: impl FnOnce(&mut S, &mut Tx) + Send + 'static) {
        AppHandle::update(self, f);
    }

    fn open(&self) {
        AppHandle::open(self);
    }

    fn hide(&self) {
        AppHandle::hide(self);
    }
}

/// A window state that records which session it belongs to.
pub(crate) trait Attached {
    fn session(&self) -> u64;
    fn set_session(&mut self, id: u64);
}

static NEXT: AtomicU64 = AtomicU64::new(1);

/// One session's hold on its window.
pub(crate) struct Link<S, H> {
    host: H,
    id: u64,
    state: PhantomData<fn(S)>,
}

impl<S: Attached + 'static, H: Host<S>> Link<S, H> {
    /// Takes the window for a new session. `attach` runs before anything the session sends,
    /// and from then on calls through an older link are dropped.
    pub(crate) fn attach(host: H, attach: impl FnOnce(&mut S, &mut Tx) + Send + 'static) -> Self {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        host.update(move |s, tx| {
            s.set_session(id);
            attach(s, tx);
        });
        Self { host, id, state: PhantomData }
    }

    pub(crate) fn update(&self, f: impl FnOnce(&mut S, &mut Tx) + Send + 'static) {
        let id = self.id;
        self.host.update(move |s, tx| {
            if s.session() == id {
                f(s, tx);
            }
        });
    }

    /// Asked from inside an update, so it keeps its place among them: AppHandle keeps only the
    /// last open or hide it's asked for, whichever thread asked.
    pub(crate) fn open(&self) {
        let host = self.host.clone();
        self.update(move |_, _| host.open());
    }

    pub(crate) fn hide(&self) {
        let host = self.host.clone();
        self.update(move |_, _| host.hide());
    }
}

#[cfg(test)]
pub(crate) mod test_host {
    use std::collections::VecDeque;
    use std::sync::{Arc, Mutex};

    use rune::Tx;

    type Op<S> = Box<dyn FnOnce(&mut S, &mut Tx, &mut bool) + Send>;

    /// Records what readers ask for and plays it back on a state in order, as the UI thread
    /// would: `visible` follows the opens and hides.
    pub(crate) struct Recorder<S>(Arc<Mutex<VecDeque<Op<S>>>>);

    impl<S> Clone for Recorder<S> {
        fn clone(&self) -> Self {
            Self(Arc::clone(&self.0))
        }
    }

    impl<S> Default for Recorder<S> {
        fn default() -> Self {
            Self(Arc::default())
        }
    }

    impl<S: 'static> Recorder<S> {
        fn push(&self, op: Op<S>) {
            self.0.lock().unwrap().push_back(op);
        }

        pub(crate) fn run(&self, s: &mut S, visible: &mut bool) {
            loop {
                let Some(op) = self.0.lock().unwrap().pop_front() else { return };
                op(s, &mut Tx::default(), visible);
            }
        }
    }

    impl<S: 'static> super::Host<S> for Recorder<S> {
        fn update(&self, f: impl FnOnce(&mut S, &mut Tx) + Send + 'static) {
            self.push(Box::new(move |s, tx, _| f(s, tx)));
        }

        fn open(&self) {
            self.push(Box::new(|_, _, visible| *visible = true));
        }

        fn hide(&self) {
            self.push(Box::new(|_, _, visible| *visible = false));
        }
    }
}
