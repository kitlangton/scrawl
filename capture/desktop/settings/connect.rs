//! Shared Connect Settings policy; native renderers only draw `View` and send `Command`.
//!
//! The parent gates construction/rendering with the default-on `connectSettings` flag.
//! An explicit false hides the controls without changing upload consent. A Session
//! owns one controller and routes emitted events back to that same controller on its UI
//! thread. Call `refresh()` on Settings Tick for fresh **local-only** state; it never
//! probes the network. `Check` alone starts a background health request. Call `close()`
//! when the window closes (Drop also cancels); create a new controller on reopen.
//!
//! `receive()` authorizes the parent's existing upload worker exactly once, only for
//! the current login after an authoritative local read verifies the committed state.
//! It does not upload, scan old captures, create shares, write config, or open browsers.
//! `command(OpenBrowser)` returns a URI only while its owning login is current. The
//! Session must open it synchronously as that user action, not queue it for later.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use serde::{Deserialize, Serialize};

mod presentation;
pub use presentation::{Presentation, Tone};

pub const CONNECT_SETTINGS: &str = "connectSettings";
pub const DEFAULT_SERVER: &str = "https://hosted.tryapparent.com";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Command {
    Connect,
    Allow,
    Cancel,
    Check,
    Disable,
    Disconnect,
    OpenBrowser,
    RequestDisable,
    RequestDisconnect,
    OpenSharedLinks,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Button {
    pub command: Command,
    pub title: String,
    pub enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub title: String,
    pub status: String,
    pub detail: String,
    pub server: Option<String>,
    pub verification_uri: Option<String>,
    pub user_code: Option<String>,
    pub buttons: Vec<Button>,
    pub note: String,
    #[serde(default)]
    pub presentation: Presentation,
}

/// Local, authoritative state, not cached network health. The adapter must validate
/// a configured server with the existing client's validator; invalid configuration
/// is an error, never `None`. `None` means no server was configured. `connected`
/// includes a usable read-only grant. `uploads_enabled` requires *all* current upload
/// gates plus a usable upload grant for this server, not just cloud.enabled.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub server: Option<String>,
    pub connected: bool,
    pub uploads_enabled: bool,
    pub reauthorization_required: bool,
    pub renewal_required: bool,
    pub account_initial: Option<String>,
}

/// Only public device-flow presentation data. The adapter must use the client's
/// existing verification-URI validation (HTTPS, or test loopback HTTP), reject embedded
/// credentials/device secrets and control characters, and supply only the public code.
/// Never pass a deviceCode, access token, raw response or error into this module.
#[derive(Clone)]
pub struct Approval {
    pub verification_uri: String,
    pub user_code: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Health {
    Online,
    Offline,
    AuthRequired,
}

/// Closed error vocabulary prevents raw protocol responses/credentials reaching UI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SafeError {
    Cancelled,
    Offline,
    AuthRequired,
    Rejected,
    Expired,
    LocalState,
    InvalidServer,
    UnverifiedCommit,
    Unavailable,
}

impl SafeError {
    fn message(self) -> &'static str {
        match self {
            Self::Cancelled => {
                "Cancelled. Check the current local upload settings before reconnecting."
            }
            Self::Offline => "Could not reach Connect. Check your connection and try again.",
            Self::AuthRequired => "Sign in again to authorize this device.",
            Self::Rejected => "Sign-in was not approved. You can try again.",
            Self::Expired => "The sign-in code expired. Connect again for a new code.",
            Self::LocalState => {
                "Apparent could not read or save Connect state. Check local storage and try again."
            }
            Self::InvalidServer => {
                "The configured Connect server is invalid. Correct it before connecting."
            }
            Self::UnverifiedCommit => {
                "Sign-in finished, but Apparent could not verify uploads are enabled for this server. Check local state before trying again."
            }
            Self::Unavailable => "Connect is unavailable. Try again.",
        }
    }
}

/// Adapter over the sole owned Connect client; no protocol or config implementation here.
///
/// `snapshot` must be local-only and fresh. `login` is upload login after explicit
/// consent; it MUST NOT print device data or open a browser. It must check `allowed`
/// during transport AND under the existing queue lock immediately before publishing
/// credentials/consent. `disable` uses the same queue -> config transaction to revoke
/// gates and durably cancel jobs; logout additionally removes credentials. Neither
/// operation may revive old jobs. Those client guarantees apply across controllers,
/// CLI processes and a close/reopen while a durable disable finishes. The controller's
/// UI guard cannot replace that transaction. All calls must have bounded waits.
pub trait Backend: Send + Sync + 'static {
    fn snapshot(&self) -> Result<Snapshot, SafeError>;
    fn shared_links_url(&self) -> Option<String> { None }
    fn login(
        &self,
        server: &str,
        allowed: Arc<dyn Fn() -> bool + Send + Sync>,
        on_approval: &dyn Fn(Approval),
    ) -> Result<(), SafeError>;
    fn health(&self, allowed: Arc<dyn Fn() -> bool + Send + Sync>) -> Health;
    fn disable(&self, logout: bool) -> Result<(), SafeError>;
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Operation {
    Login,
    Probe,
    Disable { logout: bool },
}

struct Request {
    active: AtomicBool,
    operation: Operation,
    server: String,
}

impl Request {
    fn allowed(self: &Arc<Self>) -> Arc<dyn Fn() -> bool + Send + Sync> {
        let request = self.clone();
        Arc::new(move || request.active.load(Ordering::SeqCst))
    }
}

/// Opaque, cloneable UI-thread receipt. Do not log it or route to another Session.
#[derive(Clone)]
pub struct Event {
    request: Arc<Request>,
    outcome: Outcome,
}

impl std::fmt::Debug for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ConnectEvent")
    }
}

#[derive(Clone)]
enum Outcome {
    Approval(Approval),
    Login(Result<(), SafeError>),
    Health(Health),
    Disabled(Result<(), SafeError>),
}

pub struct Controller {
    backend: Arc<dyn Backend>,
    emit: Arc<dyn Fn(Event) + Send + Sync>,
    snapshot: Option<Snapshot>,
    local_error: Option<SafeError>,
    error: Option<SafeError>,
    health: Option<Health>,
    consent: bool,
    approval: Option<Approval>,
    request: Option<Arc<Request>>,
    closed: bool,
    // A failed durable cancellation must remain actionable even if snapshot fails.
    retry_disable: Option<bool>,
    confirmation: Option<bool>,
}

impl Controller {
    pub fn new(backend: Arc<dyn Backend>, emit: Arc<dyn Fn(Event) + Send + Sync>) -> Self {
        let mut controller = Self {
            backend,
            emit,
            snapshot: None,
            local_error: None,
            error: None,
            health: None,
            consent: false,
            approval: None,
            request: None,
            closed: false,
            retry_disable: None,
            confirmation: None,
        };
        controller.refresh();
        controller
    }

    /// Safe for each Settings Tick: reads local state only, never starts a worker.
    /// Errors discard the old snapshot rather than presenting cached success.
    pub fn refresh(&mut self) {
        match self.backend.snapshot() {
            Ok(snapshot) => {
                if self.snapshot.as_ref() != Some(&snapshot) {
                    self.health = None;
                }
                self.snapshot = Some(snapshot);
                self.local_error = None;
            }
            Err(error) => {
                self.snapshot = None;
                self.local_error = Some(error);
                self.health = None;
            }
        }
    }

    fn cancel_request(&mut self) {
        if let Some(request) = self.request.take() {
            request.active.store(false, Ordering::SeqCst);
        }
        self.approval = None;
        self.consent = false;
    }

    /// Closing cancels ephemeral login/probes but cannot interrupt durable revocation.
    /// Continue routing its receipt if Session survives close; Drop does not stop the
    /// detached disable worker, and the next Session reads its durable result locally.
    pub fn close(&mut self) {
        if self
            .request
            .as_ref()
            .is_some_and(|r| r.operation == Operation::Login)
        {
            // A click can race a committed login whose UI receipt has not arrived.
            // Serialize cancellation through the same durable client transaction.
            self.cancel_request();
            self.start(Operation::Disable { logout: false }, String::new());
        }
        self.closed = true;
        self.approval = None;
        self.consent = false;
        if !self.disabling() {
            self.cancel_request();
        }
    }

    fn disabling(&self) -> bool {
        self.request
            .as_ref()
            .is_some_and(|r| matches!(r.operation, Operation::Disable { .. }))
    }

    fn start(&mut self, operation: Operation, server: String) {
        self.cancel_request();
        self.error = None;
        self.health = None;
        let request = Arc::new(Request {
            active: AtomicBool::new(true),
            operation,
            server,
        });
        self.request = Some(request.clone());
        let backend = self.backend.clone();
        let emit = self.emit.clone();
        std::thread::spawn(move || {
            let outcome = match operation {
                Operation::Login => {
                    let approval_request = request.clone();
                    let approval_emit = emit.clone();
                    let result =
                        backend.login(&request.server, request.allowed(), &move |approval| {
                            if approval_request.active.load(Ordering::SeqCst) {
                                approval_emit(Event {
                                    request: approval_request.clone(),
                                    outcome: Outcome::Approval(approval),
                                });
                            }
                        });
                    Outcome::Login(result)
                }
                Operation::Probe => Outcome::Health(backend.health(request.allowed())),
                Operation::Disable { logout } => Outcome::Disabled(backend.disable(logout)),
            };
            emit(Event { request, outcome });
        });
    }

    /// Returns a safe browser URI only for this immediate, explicit user action.
    /// Busy controls reject forged/duplicate renderer commands, not just button clicks.
    pub fn command(&mut self, command: Command) -> Option<String> {
        if self.closed || self.disabling() {
            return None;
        }
        if matches!(command, Command::RequestDisable | Command::RequestDisconnect) {
            self.confirmation = Some(command == Command::RequestDisconnect);
            return None;
        }
        if command == Command::Cancel && self.confirmation.take().is_some() {
            return None;
        }
        if command != Command::Cancel { self.confirmation = None; }
        if command == Command::OpenSharedLinks { return self.backend.shared_links_url(); }
        if command == Command::OpenBrowser {
            return self
                .request
                .as_ref()
                .filter(|r| r.operation == Operation::Login && r.active.load(Ordering::SeqCst))
                .and(self.approval.as_ref())
                .map(|a| a.verification_uri.clone());
        }
        if matches!(command, Command::Disable | Command::Disconnect) {
            // Invalidate login/probe BEFORE calling the client's durable operation.
            self.cancel_request();
            let logout = command == Command::Disconnect;
            self.retry_disable = Some(logout);
            self.start(Operation::Disable { logout }, String::new());
            return None;
        }
        if command == Command::Cancel && self.consent {
            self.cancel_request();
            self.refresh();
            return None;
        }
        if command == Command::Cancel {
            if self
                .request
                .as_ref()
                .is_some_and(|r| r.operation == Operation::Login)
            {
                self.cancel_request();
                self.retry_disable = Some(false);
                self.start(Operation::Disable { logout: false }, String::new());
                return None;
            }
            self.cancel_request();
            self.error = Some(SafeError::Cancelled);
            self.refresh();
            return None;
        }
        if self.request.is_some() {
            return None;
        }
        match command {
            Command::Connect => {
                self.refresh();
                if self.snapshot.is_some() && self.retry_disable.is_none() {
                    self.error = None;
                    self.consent = true;
                }
            }
            Command::Allow if self.consent && self.retry_disable.is_none() => {
                self.refresh();
                if let Some(snapshot) = &self.snapshot {
                    let server = snapshot
                        .server
                        .clone()
                        .unwrap_or_else(|| DEFAULT_SERVER.into());
                    self.start(Operation::Login, server);
                }
            }
            Command::Check if !self.consent => {
                self.refresh();
                if self.snapshot.is_some() {
                    self.start(Operation::Probe, String::new());
                }
            }
            _ => {}
        }
        None
    }

    /// True ONLY for one exact current successful login, after a fresh local snapshot
    /// proves the upload grant and consent committed for the requested server. The
    /// parent may then start its existing worker (which rechecks consent itself).
    pub fn receive(&mut self, event: Event) -> bool {
        let current = self.request.as_ref().is_some_and(|current| {
            Arc::ptr_eq(current, &event.request) && current.active.load(Ordering::SeqCst)
        });
        if !current || (self.closed && !self.disabling()) {
            return false;
        }
        if let Outcome::Approval(approval) = event.outcome {
            if event.request.operation == Operation::Login && !self.closed {
                self.approval = Some(approval);
            }
            return false;
        }
        // Terminal receipts retire ONLY their own request; duplicate delivery is inert.
        self.cancel_request();
        self.refresh();
        match event.outcome {
            Outcome::Login(result) => match result {
                Ok(()) => {
                    let verified = self.snapshot.as_ref().is_some_and(|snapshot| {
                        snapshot.connected
                            && snapshot.uploads_enabled
                            && snapshot.server.as_deref().unwrap_or(DEFAULT_SERVER)
                                == event.request.server
                    });
                    if verified && self.local_error.is_none() {
                        self.error = None;
                        return true;
                    }
                    self.error = Some(SafeError::UnverifiedCommit);
                }
                Err(error) => self.error = Some(error),
            },
            Outcome::Health(health) => self.health = Some(health),
            Outcome::Disabled(result) => match result {
                Ok(()) => {
                    self.retry_disable = None;
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            },
            Outcome::Approval(_) => unreachable!(),
        }
        false
    }

    pub fn view(&self) -> View {
        let mut view = View {
            title: "Apparent Connect".into(),
            status: "Not connected".into(),
            detail: "Sign in to send new captures to your Connect account.".into(),
            server: self.snapshot.as_ref().map(|s| s.server.clone().unwrap_or_else(|| DEFAULT_SERVER.into())),
            verification_uri: None,
            user_code: None,
            buttons: Vec::new(),
            presentation: Presentation::default(),
            note: "Existing captures are not uploaded. No public links are created. Disabling cancels unfinished uploads; disconnecting also signs out this computer. Already uploaded data is not deleted.".into(),
        };
        if let Some(snapshot) = &self.snapshot {
            if snapshot.reauthorization_required {
                view.status = "Sign-in required".into();
                view.detail = "This computer's authorization has expired or been revoked. Sign in again to reconnect; local captures still work.".into();
            } else if snapshot.renewal_required {
                view.status = "Connection needs a refresh".into();
                view.detail = "You're still signed in. Check connection to renew this computer's access; local captures still work.".into();
            } else if snapshot.uploads_enabled {
                view.status = "Uploads on".into();
                view.detail = "New captures can upload privately to your Connect account.".into();
            } else if snapshot.connected {
                view.status = "Connected · uploads off".into();
                view.detail =
                    "This device is signed in. Uploads require your explicit permission.".into();
            }
        }
        match self.health {
            Some(Health::Offline) => {
                view.status = if self.snapshot.as_ref().is_some_and(|s| s.uploads_enabled) {
                    "Uploads on · offline"
                } else {
                    "Offline"
                }
                .into();
                view.detail = "Connect could not be reached. Local captures still work; upload consent has not changed.".into();
            }
            Some(Health::AuthRequired) => {
                view.status = "Sign-in required".into();
                view.detail =
                    "Sign in again to authorize this device. Local captures still work.".into();
            }
            Some(Health::Online) => view.detail.push_str(" Connect is reachable."),
            None => {}
        }
        if let Some(error) = self.error {
            view.detail = error.message().into();
            if error == SafeError::UnverifiedCommit {
                view.status = "Sign-in not verified".into();
            }
        }
        if let Some(error) = self.local_error {
            view.status = "Local state unavailable".into();
            view.detail = error.message().into();
        }
        if let Some(request) = &self.request {
            match request.operation {
                Operation::Login => {
                    view.status = "Signing in…".into();
                    view.detail =
                        "Approve upload access in your browser. You can cancel at any time.".into();
                    if let Some(approval) = &self.approval {
                        view.verification_uri = Some(approval.verification_uri.clone());
                        view.user_code = Some(approval.user_code.clone());
                        view.buttons
                            .push(button(Command::OpenBrowser, "Open browser", true));
                    }
                    view.buttons
                        .push(button(Command::Cancel, "Cancel sign-in", true));
                    view.buttons
                        .push(button(Command::Disable, "Disable uploads", true));
                    view.buttons
                        .push(button(Command::Disconnect, "Disconnect", true));
                }
                Operation::Probe => {
                    view.status = "Checking connection…".into();
                    view.buttons
                        .push(button(Command::Cancel, "Cancel check", true));
                    view.buttons
                        .push(button(Command::Disable, "Disable uploads", true));
                    view.buttons
                        .push(button(Command::Disconnect, "Disconnect", true));
                }
                Operation::Disable { logout } => {
                    view.status = if logout {
                        "Disconnecting…"
                    } else {
                        "Disabling uploads…"
                    }
                    .into();
                    view.detail = "Saving your choice and cancelling unfinished uploads. Wait for confirmation before reconnecting.".into();
                    view.buttons.push(button(
                        if logout {
                            Command::Disconnect
                        } else {
                            Command::Disable
                        },
                        "Please wait…",
                        false,
                    ));
                }
            }
        } else if self.consent {
            view.status = "Allow uploads?".into();
            view.detail = "New captures will upload privately to your account, including screen images, recordings, voice, transcripts, context and other supported assets. Website sign-in alone does not connect this computer. Allow uploads to continue?".into();
            view.buttons.push(button(
                Command::Allow,
                "Allow and continue",
                self.snapshot.is_some(),
            ));
            view.buttons.push(button(Command::Cancel, "Not now", true));
        } else {
            let uploads = self.snapshot.as_ref().is_some_and(|s| s.uploads_enabled);
            let connected = self.snapshot.as_ref().is_some_and(|s| s.connected);
            let renewal = self.snapshot.as_ref().is_some_and(|s| s.renewal_required);
            let reauthorize = self
                .snapshot
                .as_ref()
                .is_some_and(|s| s.reauthorization_required)
                || self.health == Some(Health::AuthRequired);
            if (!uploads && !renewal) || reauthorize {
                view.buttons.push(button(
                    Command::Connect,
                    if reauthorize {
                        "Sign in again…"
                    } else if connected {
                        "Enable uploads…"
                    } else {
                        "Sign in to Connect…"
                    },
                    self.snapshot.is_some() && self.retry_disable.is_none(),
                ));
            }
            if connected || renewal || reauthorize || self.error.is_some() || self.local_error.is_some() {
                view.buttons.push(button(
                    Command::Check,
                    "Check connection",
                    self.snapshot.is_some(),
                ));
            }
            if uploads || renewal || self.retry_disable == Some(false) || self.local_error.is_some() {
                view.buttons.push(button(
                    Command::Disable,
                    if self.retry_disable == Some(false) {
                        "Retry disabling uploads"
                    } else {
                        "Disable uploads"
                    },
                    true,
                ));
            }
            if connected
                || renewal
                || reauthorize
                || self.retry_disable == Some(true)
                || self.local_error.is_some()
            {
                view.buttons.push(button(
                    Command::Disconnect,
                    if self.retry_disable == Some(true) {
                        "Retry disconnecting"
                    } else {
                        "Disconnect"
                    },
                    true,
                ));
            }
        }
        view.presentation = self.presentation(&view);
        if self.closed {
            view.verification_uri = None;
            view.user_code = None;
            for button in &mut view.presentation.buttons { button.enabled = false; }
            for button in &mut view.buttons {
                button.enabled = false;
            }
        }
        view
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.close();
    }
}

fn button(command: Command, title: &str, enabled: bool) -> Button {
    Button {
        command,
        title: title.into(),
        enabled,
    }
}

#[cfg(test)]
mod tests;
