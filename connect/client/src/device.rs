use super::*;
use std::sync::{Arc, Mutex};

const DEVICE_LIFETIME: Duration = Duration::from_secs(15 * 60);

struct DeviceState {
    device_code: Option<String>,
    approved: Option<(u64, Value)>,
}

/// One bounded device authorization transaction. Raw service replies are returned to
/// the caller and are never formatted or logged by this type.
pub struct DeviceLogin {
    client: Client,
    server: String,
    scope: String,
    login_id: String,
    login_path: PathBuf,
    cancellation: Option<String>,
    allowed: Arc<dyn Fn() -> bool + Send + Sync>,
    created: Instant,
    identity: Option<(String, Value)>,
    state: Mutex<DeviceState>,
}

impl DeviceLogin {
    pub fn server(&self) -> &str {
        &self.server
    }

    fn current(&self) -> bool {
        (self.allowed)()
            && !self.client.revocation_pending()
            && self
                .client
                .cancellation()
                .is_ok_and(|epoch| epoch == self.cancellation)
            && read_private(&self.login_path, 128)
                .ok()
                .and_then(|bytes| serde_json::from_slice::<String>(&bytes).ok())
                .as_deref()
                == Some(self.login_id.as_str())
            && self.created.elapsed() < DEVICE_LIFETIME
    }

    fn ensure_current(&self) -> Result<()> {
        ensure!(self.current(), "cancelled");
        Ok(())
    }

    fn remaining(&self) -> Result<Duration> {
        self.ensure_current()?;
        let remaining = DEVICE_LIFETIME.saturating_sub(self.created.elapsed());
        ensure!(!remaining.is_zero(), "Device authorization expired");
        Ok(remaining)
    }

    /// Start device authorization and return the unmodified service response.
    pub fn start(&self) -> Result<Value> {
        self.ensure_current()?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("device transaction unavailable"))?;
        ensure!(
            state.device_code.is_none(),
            "Device authorization already started"
        );
        let mut body = json!({"scope": self.scope});
        if let Some((_, public_key)) = &self.identity {
            body["encryptionKey"] = public_key.clone().into();
        }
        let reply = self.client.json_request(
            &self.server,
            None,
            "POST",
            "/v1/device/start",
            Some(body),
            self.remaining()?.min(Duration::from_secs(30)),
            &|| self.current(),
        )?;
        self.ensure_current()?;
        let code = reply["deviceCode"]
            .as_str()
            .context("Missing device code")?;
        ensure!(
            !code.is_empty() && code.len() <= 8192,
            "Invalid device response"
        );
        state.device_code = Some(code.to_owned());
        Ok(reply)
    }

    /// Poll once and return the raw response. Only an approved response is retained
    /// privately for a subsequent one-shot commit.
    pub fn poll(&self, device_code: &str, timeout_ms: u64) -> Result<Value> {
        ensure!(
            (1..=30_000).contains(&timeout_ms),
            "Invalid device poll timeout"
        );
        ensure!(
            !device_code.is_empty()
                && device_code.len() <= 8192
                && device_code.bytes().all(|byte| byte.is_ascii_graphic()),
            "Invalid device code"
        );
        self.ensure_current()?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| anyhow::anyhow!("device transaction unavailable"))?;
        ensure!(
            state.device_code.as_deref() == Some(device_code),
            "Invalid device code"
        );
        let issued = now();
        let reply = self.client.json_request(
            &self.server,
            None,
            "POST",
            "/v1/device/poll",
            Some(json!({"deviceCode": device_code})),
            self.remaining()?.min(Duration::from_millis(timeout_ms)),
            &|| self.current(),
        )?;
        self.ensure_current()?;
        if reply["status"].as_str() == Some("approved") {
            state.approved = Some((issued, reply.clone()));
        }
        Ok(reply)
    }

    /// Consume the most recent approved poll reply and publish its credential under
    /// the existing queue/config transaction used by CLI login.
    pub fn commit(&self, site_origin: &str) -> Result<()> {
        self.ensure_current()?;
        let site = server_url(site_origin)?;
        let (issued, reply) = {
            let mut state = self
                .state
                .lock()
                .map_err(|_| anyhow::anyhow!("device transaction unavailable"))?;
            let approved = state
                .approved
                .take()
                .context("No approved device response")?;
            ensure!(
                reply_is_approved(&approved.1),
                "No approved device response"
            );
            approved
        };
        ensure!(
            reply["scope"].as_str() == Some(self.scope.as_str()),
            "Device authorization returned a different scope"
        );
        let token = reply["accessToken"]
            .as_str()
            .context("Missing access token")?;
        let mut credential = Credential {
            server: self.server.clone(),
            site: Some(site),
            scope: self.scope.clone(),
            token: token.to_owned(),
            generation: random_id()?,
            refresh_token: None,
            expires_at: None,
            refresh_expires_at: None,
            grant_id: None,
            auth_required: false,
            encryption_identity: self.identity.as_ref().map(|(id, _)| id.clone()),
            account_initial: None,
        };
        credential.apply_reply(&reply, issued, false)?;

        private_dir(&self.client.state)?;
        let _lock = Lock::acquire_until(
            &self.client.state.join("queue.lock"),
            false,
            Some(Instant::now() + Duration::from_secs(3)),
        )?;
        ensure!(self.current(), "cancelled");
        let mut settings = self.client.settings();
        ensure!(
            self.scope == "upload" || !settings.enabled || settings.server == self.server,
            "Disable uploads before switching servers"
        );
        if self.scope == "upload" {
            settings = CloudConfig {
                enabled: true,
                upload_consent: true,
                server: self.server.clone(),
            };
            self.client.provider.set_experiment(HOSTED_SYNC, true)?;
        } else {
            settings.server = self.server.clone();
        }
        let still_current = || self.current();
        self.client
            .provider
            .set_cloud_allowed(&settings, &still_current)?;
        ensure!(still_current(), "cancelled");
        self.client.clear_renewal(&self.scope)?;
        atomic_json(
            &self.client.state.join(format!("{}.credential", self.scope)),
            &credential,
        )?;
        fs::remove_file(&self.login_path)?;
        Ok(())
    }
}

fn reply_is_approved(reply: &Value) -> bool {
    reply["status"].as_str() == Some("approved")
}

impl Client {
    pub fn begin_device_login(
        &self,
        server: &str,
        scope: &str,
        consent: bool,
        allowed: Arc<dyn Fn() -> bool + Send + Sync>,
    ) -> Result<DeviceLogin> {
        ensure!(allowed(), "cancelled");
        let server = server_url(server)?;
        ensure!(matches!(scope, "upload" | "read"), "Invalid scope");
        ensure!(scope != "upload" || consent, "Upload consent is required");
        private_dir(&self.state)?;
        ensure!(!self.revocation_pending(), "cancelled: revocation pending");
        let cancellation = self.cancellation()?;
        let login_id = random_id()?;
        let login_path = self.state.join(format!("{scope}.login"));
        {
            let _queue = Lock::acquire_until(
                &self.state.join("queue.lock"),
                false,
                Some(Instant::now() + Duration::from_secs(3)),
            )?;
            ensure!(
                allowed() && !self.revocation_pending() && self.cancellation()? == cancellation,
                "cancelled"
            );
            atomic_json(&login_path, &login_id)?;
        }

        let identity = if scope == "read" || self.provider.experiment(HOSTED_ENCRYPTION) {
            let identity_id = login_id.clone();
            let value = encryption::load_or_create_identity(
                &self.state.join(format!("{identity_id}.identity")),
            )?;
            Some((identity_id, value.public_key()))
        } else {
            None
        };

        Ok(DeviceLogin {
            client: self.clone(),
            server,
            scope: scope.to_owned(),
            login_id,
            login_path,
            cancellation,
            allowed,
            created: Instant::now(),
            identity,
            state: Mutex::new(DeviceState {
                device_code: None,
                approved: None,
            }),
        })
    }
}

impl Client {
    pub fn login_server(&self, explicit: Option<&str>) -> Result<String> {
        let configured = self.settings().server;
        server_url(explicit.unwrap_or(if configured.is_empty() {
            settings::DEFAULT_SERVER
        } else {
            &configured
        }))
    }
    pub(super) fn read_login_command(&self) -> Result<String> {
        let server = self.login_server(None)?;
        Ok(if server == settings::DEFAULT_SERVER {
            "apparent connect login".to_owned()
        } else {
            format!("apparent connect login --server '{}'", server.replace('\'', "'\\''"))
        })
    }
    pub fn login(&self, server: &str, scope: &str, consent: bool, browser: bool) -> Result<()> {
        self.login_allowed(server, scope, consent, browser, &|| true)
    }
    pub fn login_allowed(
        &self,
        server: &str,
        scope: &str,
        consent: bool,
        browser: bool,
        allowed: &dyn Fn() -> bool,
    ) -> Result<()> {
        self.login_presented(
            server,
            scope,
            consent,
            allowed,
            &|approval| {
                println!(
                    "Open {}\nVerification code: {}\nRequested scope: {scope}",
                    approval.verification_uri, approval.user_code
                );
                if browser {
                    self.provider
                        .open_browser(Path::new(&approval.verification_uri));
                }
            },
            true,
        )
    }
    pub fn login_presented(
        &self,
        server: &str,
        scope: &str,
        consent: bool,
        allowed: &dyn Fn() -> bool,
        on_approval: &dyn Fn(settings::Approval),
        cli: bool,
    ) -> Result<()> {
        ensure!(allowed(), "cancelled");
        let server = server_url(server)?;
        ensure!(matches!(scope, "upload" | "read"), "Invalid scope");
        ensure!(
            scope != "upload" || consent,
            "Upload shares screen, voice, transcript and all bundle media with the hosted service. Repeat with --consent-upload to explicitly agree"
        );
        private_dir(&self.state)?;
        ensure!(!self.revocation_pending(), "cancelled: revocation pending");
        let cancellation = self.cancellation()?;
        let login_id = random_id()?;
        let login_path = self.state.join(format!("{scope}.login"));
        {
            let _queue = Lock::acquire_until(
                &self.state.join("queue.lock"),
                false,
                (!cli).then(|| Instant::now() + Duration::from_secs(3)),
            )?;
            ensure!(
                allowed() && !self.revocation_pending() && self.cancellation()? == cancellation,
                "cancelled"
            );
            atomic_json(&login_path, &login_id)?;
        }
        let caller_allowed = allowed;
        let still_current = || {
            caller_allowed()
                && !self.revocation_pending()
                && self.cancellation().is_ok_and(|epoch| epoch == cancellation)
                && read_private(&login_path, 128)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<String>(&b).ok())
                    .as_deref()
                    == Some(&login_id)
        };
        let allowed: &dyn Fn() -> bool = &still_current;
        // Public keys are bound to this same approval, never to its server-known
        // user code. Secret material stays in a separate private endpoint artifact.
        let encryption_identity = if (cli && scope == "read") || self.provider.experiment(HOSTED_ENCRYPTION) {
            let identity_path = self.state.join(format!("{login_id}.identity"));
            let identity = encryption::load_or_create_identity(&identity_path)?;
            Some((login_id.clone(), identity.public_key()))
        } else {
            None
        };
        let mut start_body = json!({"scope":scope});
        if let Some((_, public_key)) = &encryption_identity {
            start_body["encryptionKey"] = public_key.clone();
        }
        let start = self.json_request(
            &server,
            None,
            "POST",
            "/v1/device/start",
            Some(start_body),
            Duration::from_secs(30),
            allowed,
        )?;
        let device_code = start["deviceCode"]
            .as_str()
            .context("Missing device code")?;
        let user_code = start["userCode"].as_str().context("Missing user code")?;
        let uri = start["verificationUri"]
            .as_str()
            .context("Missing verification URI")?;
        ensure!(
            !device_code.is_empty()
                && device_code.len() <= 8192
                && !user_code.is_empty()
                && user_code.len() <= 128
                && !user_code.contains(device_code)
                && user_code
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                && !user_code.chars().any(char::is_control),
            "Invalid device response"
        );
        // The API and device approval site can have distinct origins. Only open a
        // normal HTTPS link (loopback HTTP for a local test server), never a custom scheme.
        let (origin, _) = uri
            .split_once("://")
            .and_then(|(scheme, rest)| {
                rest.split_once('/')
                    .map(|(host, path)| (format!("{scheme}://{host}"), path))
            })
            .context("Invalid verification URI")?;
        server_url(&origin)?;
        ensure!(
            uri.len() <= 4096
                && !uri.contains(device_code)
                && !uri.contains(['#', '%'])
                && uri
                    .split_once('?')
                    .is_none_or(|(_, query)| query == format!("code={user_code}"))
                && !uri.bytes().any(|b| b.is_ascii_control()
                    || b.is_ascii_whitespace()
                    || b"\\\"'".contains(&b)),
            "Invalid verification URI"
        );
        ensure!(allowed(), "cancelled");
        on_approval(settings::Approval {
            verification_uri: uri.into(),
            user_code: user_code.into(),
        });
        ensure!(allowed(), "cancelled");
        let deadline = Instant::now()
            + Duration::from_secs(start["expiresIn"].as_u64().unwrap_or(300).min(900));
        let interval = Duration::from_secs(start["interval"].as_u64().unwrap_or(5).clamp(1, 30));
        let credential = loop {
            ensure!(allowed(), "cancelled");
            ensure!(Instant::now() < deadline, "Device authorization expired");
            std::thread::sleep(interval.min(deadline.saturating_duration_since(Instant::now())));
            ensure!(Instant::now() < deadline, "Device authorization expired");
            let issued = now();
            let reply = self.json_request(
                &server,
                None,
                "POST",
                "/v1/device/poll",
                Some(json!({"deviceCode": device_code})),
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_secs(30)),
                allowed,
            )?;
            match reply["status"].as_str() {
                Some("pending") => continue,
                Some("approved") => {
                    ensure!(
                        reply["scope"].as_str() == Some(scope),
                        "Device authorization returned a different scope"
                    );
                    let token = reply["accessToken"]
                        .as_str()
                        .context("Missing access token")?;
                    ensure!(
                        !token.is_empty()
                            && token.len() <= 8192
                            && token.bytes().all(|b| b.is_ascii_graphic()),
                        "Invalid access token"
                    );
                    let mut credential = Credential {
                        server: server.clone(),
                        site: Some(origin.clone()),
                        scope: scope.into(),
                        token: token.into(),
                        generation: random_id()?,
                        refresh_token: None,
                        expires_at: None,
                        refresh_expires_at: None,
                        grant_id: None,
                        auth_required: false,
                        encryption_identity: encryption_identity.as_ref().map(|(id, _)| id.clone()),
                        account_initial: None,
                    };
                    credential.apply_reply(&reply, issued, false)?;
                    break credential;
                }
                _ => bail!("Device authorization rejected or invalid"),
            }
        };
        private_dir(&self.state)?;
        let _lock = Lock::acquire_until(
            &self.state.join("queue.lock"),
            false,
            (!cli).then(|| Instant::now() + Duration::from_secs(3)),
        )?;
        // Read-only login must not silently enable upload or switch an active uploader's server.
        ensure!(allowed(), "cancelled");
        let mut settings = self.settings();
        #[cfg(any(test, feature = "test-support"))]
        test_support::run_hook("login_snapshot", &self.config);
        ensure!(
            scope == "upload" || !settings.enabled || settings.server == server,
            "Disable uploads before switching servers"
        );
        if scope == "upload" {
            settings = CloudConfig {
                enabled: true,
                upload_consent: true,
                server: server.clone(),
            };
            self.provider.set_experiment(HOSTED_SYNC, true)?;
        } else {
            settings.server = server;
        }
        self.provider.set_cloud_allowed(&settings, allowed)?;
        ensure!(allowed(), "cancelled");
        self.clear_renewal(scope)?;
        atomic_json(&self.state.join(format!("{scope}.credential")), &credential)?;
        fs::remove_file(login_path)?;
        if cli {
            println!("Signed in for {scope}; tokens remain private on this machine.");
        }
        Ok(())
    }
}
