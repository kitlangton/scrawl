//! The host's settings, folders and hooks as the router's code sees them while it runs.
//!
//! A [`Router`](crate::Router) call makes its own the current one on its thread (and the
//! threads it starts, through [`spawn`] and [`scope_spawn`]). Outside a call, a host that
//! calls into the modules directly (Scrawl's app) installs a fallback with [`set_fallback`].
//! With neither, the router is inert: default settings, no folders, no log, and nothing read
//! from this Mac's OpenCode history.

use std::cell::RefCell;
use std::path::Path;
use std::sync::{Arc, RwLock};

use crate::config::{RouterConfig, RouterDirs};
use crate::model::ScrawlBundle;
use crate::routing::Registration;

/// Writes one line to the host's log.
pub type Log = Arc<dyn Fn(&str) + Send + Sync>;

/// A bundle's summary, as sent to a session.
pub type Summary = fn(&ScrawlBundle, &Path) -> String;

/// The `/sdk.js` registration for a tab's URL, when the host serves `/sdk.js`.
pub type Pages = fn(&str) -> Option<Registration>;

#[derive(Clone, Default)]
pub struct Env {
    pub config: RouterConfig,
    pub dirs: RouterDirs,
    pub log: Option<Log>,
    pub summary: Option<Summary>,
    pub pages: Option<Pages>,
    /// Configured by a host: reads this Mac's OpenCode history and sessions' last lines. Off
    /// when nothing configured the router (tests).
    pub live: bool,
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<Env>>> = const { RefCell::new(None) };
}

type Fallback = Arc<dyn Fn() -> Arc<Env> + Send + Sync>;

static FALLBACK: RwLock<Option<Fallback>> = RwLock::new(None);

/// What the router uses outside a [`Router`](crate::Router) call: `f` is asked each time, so
/// it can follow the host's settings as they change.
pub fn set_fallback(f: impl Fn() -> Arc<Env> + Send + Sync + 'static) {
    *FALLBACK.write().unwrap_or_else(|e| e.into_inner()) = Some(Arc::new(f));
}

/// The settings in force on this thread.
pub fn get() -> Arc<Env> {
    if let Some(env) = current() {
        return env;
    }
    let fallback = FALLBACK.read().unwrap_or_else(|e| e.into_inner()).clone();
    match fallback {
        Some(f) => f(),
        None => Arc::new(Env::default()),
    }
}

/// Makes `env` current on this thread until the guard drops.
pub fn enter(env: Arc<Env>) -> Entered {
    let previous = CURRENT.with(|c| c.borrow_mut().replace(env));
    Entered { previous }
}

pub struct Entered {
    previous: Option<Arc<Env>>,
}

impl Drop for Entered {
    fn drop(&mut self) {
        let previous = self.previous.take();
        CURRENT.with(|c| *c.borrow_mut() = previous);
    }
}

/// The settings made current on this thread by a call, if any (not the fallback).
fn current() -> Option<Arc<Env>> {
    CURRENT.with(|c| c.borrow().clone())
}

fn enter_if(env: Option<Arc<Env>>) -> Option<Entered> {
    env.map(enter)
}

/// `std::thread::spawn` that keeps this thread's settings. Without a current call the new thread
/// follows the fallback, as this one does.
pub fn spawn<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> std::thread::JoinHandle<T> {
    spawn_named(None, f).expect("spawn a thread")
}

/// [`spawn`] with a thread name.
pub fn spawn_named<T: Send + 'static>(
    name: Option<&str>,
    f: impl FnOnce() -> T + Send + 'static,
) -> std::io::Result<std::thread::JoinHandle<T>> {
    let env = current();
    let mut builder = std::thread::Builder::new();
    if let Some(name) = name {
        builder = builder.name(name.to_owned());
    }
    builder.spawn(move || {
        let _entered = enter_if(env);
        f()
    })
}

/// `Scope::spawn` that keeps this thread's settings.
pub fn scope_spawn<'scope, 'env, T: Send + 'scope>(
    scope: &'scope std::thread::Scope<'scope, 'env>,
    f: impl FnOnce() -> T + Send + 'scope,
) -> std::thread::ScopedJoinHandle<'scope, T> {
    let env = current();
    scope.spawn(move || {
        let _entered = enter_if(env);
        f()
    })
}

/// One line in the host's log, if it gave one.
pub fn log(message: impl AsRef<str>) {
    if let Some(log) = &get().log {
        log(message.as_ref());
    }
}
