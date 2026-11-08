//! Secret-free presentation boundary over the existing device/credential transport.
use super::*;

pub const DEFAULT_SERVER: &str = "https://hosted.tryapparent.com";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub server: Option<String>,
    pub connected: bool,
    pub uploads_enabled: bool,
    pub reauthorization_required: bool,
    pub account_initial: Option<String>,
}
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SafeError {
    Cancelled,
    Offline,
    AuthRequired,
    Rejected,
    Expired,
    LocalState,
    InvalidServer,
    Unavailable,
}

// Inspect only known internal diagnostics, never copy remote text, paths or secrets.
fn safe_error(error: &anyhow::Error, allowed: &dyn Fn() -> bool) -> SafeError {
    if !allowed() {
        return SafeError::Cancelled;
    }
    let message = error.to_string();
    if message == "cancelled" || message.starts_with("cancelled:") {
        SafeError::Cancelled
    } else if message.starts_with("auth_required:") {
        SafeError::AuthRequired
    } else if message == "Device authorization expired" || message == "Hosted API: device_expired" {
        SafeError::Expired
    } else if message == "Device authorization rejected or invalid"
        || message == "Hosted API: device_rejected"
        || message == "Hosted API: rejected"
    {
        SafeError::Rejected
    } else if message.starts_with("Hosted transport")
        || message.starts_with("Hosted request")
        || message == "Hosted API temporarily unavailable; retry later"
    {
        SafeError::Offline
    } else if error.downcast_ref::<std::io::Error>().is_some()
        || error.downcast_ref::<serde_json::Error>().is_some()
    {
        SafeError::LocalState
    } else {
        SafeError::Unavailable
    }
}

impl Client {
    // A renewable but expired access grant is not a completed renewal. Only the
    // explicit health/network path can establish that; local presentation cannot.
    fn settings_usable(credential: &Credential) -> bool {
        credential.usable() && credential.expires_at.is_none_or(|expiry| expiry > now())
    }
    // Strict read for presentation: unlike the application's tolerant preferences
    // reader, corrupt relevant state must not be displayed as a successful sign-out.
    // This is read-only; the application provider remains the sole config writer.
    fn settings_local_config(&self) -> std::result::Result<CloudConfig, SafeError> {
        let bytes = match OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(self.provider.config_path())
        {
            Ok(file) => {
                if !file
                    .metadata()
                    .map_err(|_| SafeError::LocalState)?
                    .is_file()
                {
                    return Err(SafeError::LocalState);
                }
                let mut bytes = Vec::new();
                file.take(JSON_LIMIT + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| SafeError::LocalState)?;
                if bytes.len() as u64 > JSON_LIMIT {
                    return Err(SafeError::LocalState);
                }
                bytes
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(CloudConfig::default());
            }
            Err(_) => return Err(SafeError::LocalState),
        };
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| SafeError::LocalState)?;
        let object = value.as_object().ok_or(SafeError::LocalState)?;
        if let Some(experiments) = object.get("experiments") {
            let experiments = experiments.as_object().ok_or(SafeError::LocalState)?;
            if experiments
                .get(HOSTED_SYNC)
                .is_some_and(|flag| !flag.is_boolean())
            {
                return Err(SafeError::LocalState);
            }
        }
        match object.get("cloud") {
            None => Ok(CloudConfig::default()),
            Some(value) => serde_json::from_value(value.clone()).map_err(|_| SafeError::LocalState),
        }
    }
    fn settings_grant(
        &self,
        scope: &str,
        server: &str,
    ) -> std::result::Result<Option<Credential>, SafeError> {
        match fs::symlink_metadata(self.state.join(format!("{scope}.credential"))) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(SafeError::LocalState),
            Ok(_) => (),
        }
        let credential = self
            .stored_credential(scope)
            .map_err(|_| SafeError::LocalState)?;
        Ok((credential.server == server).then_some(credential))
    }
    /// Fresh disk-only state; never renews credentials or probes the server.
    pub fn settings_snapshot(&self) -> std::result::Result<Snapshot, SafeError> {
        if self.revocation_pending() {
            return Err(SafeError::LocalState);
        }
        let config = self.settings_local_config()?;
        let server = if config.server.is_empty() {
            None
        } else {
            Some(server_url(&config.server).map_err(|_| SafeError::InvalidServer)?)
        };
        let origin = server.as_deref().unwrap_or(DEFAULT_SERVER);
        let upload = self.settings_grant("upload", origin)?;
        let read = self.settings_grant("read", origin)?;
        let connected = upload.as_ref().is_some_and(Self::settings_usable)
            || read.as_ref().is_some_and(Self::settings_usable);
        // Prefer the upload grant even when it cannot supply identity: a read
        // grant may belong to a different account. Never substitute its initial.
        let account_initial = upload.as_ref().or(read.as_ref())
            .filter(|grant| grant.usable())
            .and_then(|grant| grant.account_initial.as_deref())
            .filter(|value| renewal::valid_account_initial(value)).map(str::to_owned);
        Ok(Snapshot {
            account_initial,
            uploads_enabled: server.is_some()
                && !self.revocation_pending()
                && config.enabled
                && config.upload_consent
                && self.provider.experiment(HOSTED_SYNC)
                && upload.as_ref().is_some_and(Self::settings_usable),
            reauthorization_required: upload
                .as_ref()
                .or(read.as_ref())
                .is_some_and(|grant| !Self::settings_usable(grant)),
            server,
            connected,
        })
    }
    /// The sole device-login protocol, with no stdout or automatic browser opening.
    pub fn login_with_approval(
        &self,
        server: &str,
        scope: &str,
        consent: bool,
        allowed: &dyn Fn() -> bool,
        on_approval: &dyn Fn(Approval),
    ) -> std::result::Result<(), SafeError> {
        if !allowed() {
            return Err(SafeError::Cancelled);
        }
        server_url(server).map_err(|_| SafeError::InvalidServer)?;
        self.settings_local_config()?;
        self.login_presented(server, scope, consent, allowed, on_approval, false)
            .map_err(|error| safe_error(&error, allowed))
    }
    /// Bounded authenticated account probe, sharing renewal and cancellation logic.
    /// No queue/config lock is retained across transport or renewal waits.
    pub fn settings_health(&self, allowed: &dyn Fn() -> bool) -> Health {
        if !allowed() {
            return Health::Offline;
        }
        let snapshot = match self.settings_snapshot() {
            Ok(snapshot) => snapshot,
            Err(SafeError::AuthRequired) => return Health::AuthRequired,
            Err(_) => return Health::Offline,
        };
        let server = snapshot.server.as_deref().unwrap_or(DEFAULT_SERVER);
        let credential = ["upload", "read"].into_iter().find_map(|scope| {
            self.settings_grant(scope, server)
                .ok()
                .flatten()
                .filter(Credential::usable)
        });
        let Some(credential) = credential else {
            return Health::AuthRequired;
        };
        let epoch = match self.cancellation() {
            Ok(epoch) => epoch,
            Err(_) => return Health::Offline,
        };
        let current = || {
            allowed()
                && !self.revocation_pending()
                && self.cancellation().is_ok_and(|value| value == epoch)
        };
        let deadline = Instant::now() + Duration::from_secs(3);
        match self.json_request_until(
            server,
            Some(&credential),
            "GET",
            "/v1/account",
            None,
            Duration::from_secs(3),
            &current,
            Some(deadline),
        ) {
            Ok(_) if current() => Health::Online,
            Err(error) if current() && safe_error(&error, &current) == SafeError::AuthRequired => {
                Health::AuthRequired
            }
            _ => Health::Offline,
        }
    }
    pub fn settings_disable(&self, logout: bool) -> std::result::Result<(), SafeError> {
        self.disable_until(logout, Some(Instant::now() + Duration::from_secs(3)))
            .map_err(|_| SafeError::LocalState)
    }
}
