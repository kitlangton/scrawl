//! Renewable transport credentials. Renewal locks never nest inside queue.lock.
//! The response journal survives a crash between a rotating response and publication.
use super::*;

const AUTH_REQUIRED: &str = "auth_required: approve this device again with apparent connect login";
pub(super) fn valid_account_initial(value: &str) -> bool {
    let count = value.chars().take(4).count();
    (1..=3).contains(&count) && value.chars().all(char::is_alphanumeric)
}
// Display metadata must never make otherwise valid authorization unusable.
pub(super) fn deserialize_account_initial<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<String>, D::Error> {
    let value = Value::deserialize(deserializer)?;
    Ok(value.as_str().filter(|value| valid_account_initial(value)).map(str::to_owned))
}
fn opaque(value: &str) -> bool {
    !value.is_empty() && value.len() <= 8192 && value.bytes().all(|b| b.is_ascii_graphic())
}
impl Credential {
    pub(super) fn validate(&self) -> Result<()> {
        match (
            &self.refresh_token,
            self.expires_at,
            self.refresh_expires_at,
            &self.grant_id,
        ) {
            (None, _, None, None) => Ok(()), // Legacy grants remain readable.
            (Some(refresh), Some(access_expiry), Some(refresh_expiry), Some(grant)) => {
                ensure!(
                    opaque(refresh)
                        && opaque(grant)
                        && grant.len() <= 512
                        && access_expiry > 0
                        && refresh_expiry > 0,
                    "Invalid credential lifecycle"
                );
                Ok(())
            }
            _ => bail!("Invalid credential lifecycle"),
        }
    }
    pub(super) fn usable(&self) -> bool {
        !self.auth_required
            && self.refresh_expires_at.is_none_or(|expiry| expiry > now())
            && (self.refresh_token.is_some() || self.expires_at.is_none_or(|expiry| expiry > now()))
    }
    pub(super) fn apply_reply(&mut self, reply: &Value, issued: u64, renewal: bool) -> Result<()> {
        ensure!(
            reply["scope"].as_str() == Some(&self.scope),
            "auth_required: credential scope mismatch"
        );
        let access = reply["accessToken"]
            .as_str()
            .filter(|s| opaque(s))
            .context("Invalid access token")?;
        let refresh = reply["refreshToken"]
            .as_str()
            .filter(|s| opaque(s))
            .context("Invalid refresh token")?;
        let grant = reply["grantId"]
            .as_str()
            .filter(|s| opaque(s) && s.len() <= 512)
            .context("Invalid grant ID")?;
        ensure!(
            !renewal || self.grant_id.as_deref() == Some(grant),
            "auth_required: credential grant mismatch"
        );
        let access_life = reply["expiresIn"]
            .as_u64()
            .filter(|n| *n > 0)
            .context("Invalid access lifetime")?;
        let refresh_life = reply["refreshExpiresIn"]
            .as_u64()
            .filter(|n| *n > 0)
            .context("Invalid refresh lifetime")?;
        self.expires_at = Some(
            issued
                .checked_add(access_life)
                .context("Invalid access lifetime")?,
        );
        self.refresh_expires_at = Some(
            issued
                .checked_add(refresh_life)
                .context("Invalid refresh lifetime")?,
        );
        self.token = access.into();
        self.refresh_token = Some(refresh.into());
        self.grant_id = Some(grant.into());
        self.account_initial = reply["accountInitial"].as_str()
            .filter(|value| valid_account_initial(value)).map(str::to_owned);
        self.auth_required = false;
        self.validate()
    }
}

#[derive(Deserialize, Serialize)]
struct Renewal {
    base: Credential,
    issued: u64,
    #[serde(default)]
    completed: bool,
}
impl Client {
    pub(super) fn auth_status(&self, scope: &str, server: &str) -> Value {
        let Ok(c) = self.credential(scope, server) else {
            return json!({"status":"signed_out"});
        };
        let status = if !c.usable() {
            "auth_required"
        } else if c.refresh_token.is_none() {
            "legacy_reapproval_on_expiry"
        } else if c
            .expires_at
            .is_some_and(|expiry| expiry <= now().saturating_add(30))
        {
            "renewal_due"
        } else {
            "signed_in"
        };
        json!({"status":status,"expiresAt":c.expires_at,"refreshExpiresAt":c.refresh_expires_at,
            "action": if status == "auth_required" { Some(if scope == "upload" { "Run apparent connect login --scope upload --consent-upload for this server" } else { "Run apparent connect login --scope read for this server" }) } else { None }})
    }
    fn same_login(&self, credential: &Credential) -> bool {
        self.credential(&credential.scope, &credential.server)
            .is_ok_and(|current| current.generation == credential.generation)
    }
    pub(super) fn cancellation(&self) -> Result<Option<String>> {
        let path = self.state.join("cancellation");
        match fs::symlink_metadata(&path) {
            Ok(_) => Ok(Some(serde_json::from_slice(&read_private(&path, 128)?)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn renewal_reply(
        &self,
        path: &Path,
        expected: &Credential,
        allowed: &dyn Fn() -> bool,
        queue_deadline: Option<Instant>,
    ) -> Result<Value> {
        match read_private(path, JSON_LIMIT).and_then(|b| Ok(serde_json::from_slice(&b)?)) {
            Ok(reply) => Ok(reply),
            Err(_) => {
                self.mark_auth_required(expected, allowed, queue_deadline)?;
                bail!("{AUTH_REQUIRED}: invalid renewal response");
            }
        }
    }
    fn current_credential(&self, expected: &Credential) -> Result<Credential> {
        let current = self
            .credential(&expected.scope, &expected.server)
            .map_err(|_| anyhow::anyhow!(AUTH_REQUIRED))?;
        ensure!(
            current.generation == expected.generation,
            "cancelled: device login changed"
        );
        ensure!(!current.auth_required, "{AUTH_REQUIRED}");
        Ok(current)
    }
    // Settings opts into cancellable queue waits; CLI/worker retain blocking waits.
    fn renewal_queue(&self, allowed: &dyn Fn() -> bool, deadline: Option<Instant>) -> Result<Lock> {
        let check = || -> Result<()> {
            if let Some(end) = deadline {
                ensure!(allowed(), "cancelled");
                ensure!(Instant::now() < end, "Hosted request deadline exceeded");
            }
            Ok(())
        };
        let lock = if deadline.is_some() {
            loop {
                check()?;
                match Lock::acquire(&self.state.join("queue.lock"), true) {
                    Ok(lock) => break lock,
                    Err(_) => std::thread::sleep(Duration::from_millis(20)),
                }
            }
        } else {
            Lock::acquire(&self.state.join("queue.lock"), false)?
        };
        check()?;
        Ok(lock)
    }
    fn mark_auth_required(
        &self,
        expected: &Credential,
        allowed: &dyn Fn() -> bool,
        queue_deadline: Option<Instant>,
    ) -> Result<()> {
        #[cfg(any(test, feature = "test-support"))]
        test_support::run_hook("renewal_auth_required", &self.state);
        let _queue = self.renewal_queue(allowed, queue_deadline)?;
        if let Ok(mut current) = self.credential(&expected.scope, &expected.server) {
            if current == *expected {
                current.auth_required = true;
                atomic_json(
                    &self.state.join(format!("{}.credential", current.scope)),
                    &current,
                )?;
            }
        }
        Ok(())
    }
    pub(super) fn clear_renewal(&self, scope: &str) -> Result<()> {
        for suffix in ["renewal", "response"] {
            match fs::remove_file(self.state.join(format!("{scope}.{suffix}"))) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
        }
        File::open(&self.state)?.sync_all()?;
        Ok(())
    }
    fn renew(
        &self,
        expected: &Credential,
        force: bool,
        allowed: &dyn Fn() -> bool,
        deadline: Instant,
        queue_deadline: Option<Instant>,
    ) -> Result<Credential> {
        // Poll a nonblocking flock so cancellation also bounds a competing process's wait.
        let _renewal = loop {
            ensure!(allowed() && self.same_login(expected), "cancelled");
            match Lock::acquire(
                &self.state.join(format!("{}.renewal.lock", expected.scope)),
                true,
            ) {
                Ok(lock) => break lock,
                Err(_) if Instant::now() < deadline => {
                    #[cfg(any(test, feature = "test-support"))]
                    test_support::run_hook("renewal_waiting", &self.state);
                    std::thread::sleep(Duration::from_millis(20))
                }
                Err(_) => bail!("Credential renewal busy; retry later"),
            }
        };
        let mut current = self.current_credential(expected)?;
        ensure!(
            allowed() && queue_deadline.is_none_or(|end| Instant::now() < end),
            "cancelled"
        );
        // A different process has already renewed this exact access incarnation.
        let cancellation = self.cancellation()?;
        if current != *expected
            || (!force
                && current
                    .expires_at
                    .is_none_or(|t| t > now().saturating_add(30)))
        {
            return Ok(current);
        }
        if current.refresh_token.is_none() {
            self.mark_auth_required(&current, allowed, queue_deadline)?;
            bail!("{AUTH_REQUIRED}");
        }
        let journal = self.state.join(format!("{}.renewal", current.scope));
        let response = self.state.join(format!("{}.response", current.scope));
        let recovered = read_private(&journal, 65536)
            .ok()
            .and_then(|b| serde_json::from_slice::<Renewal>(&b).ok())
            .filter(|r| r.base == current);
        let mut record = if let Some(record) = recovered {
            record
        } else {
            #[cfg(any(test, feature = "test-support"))]
            test_support::run_hook("renewal_preparation", &self.state);
            let _queue = self.renewal_queue(allowed, queue_deadline)?;
            ensure!(
                self.credential(&current.scope, &current.server)? == current,
                "cancelled"
            );
            self.clear_renewal(&current.scope)?;
            let record = Renewal {
                base: current.clone(),
                issued: now(),
                completed: false,
            };
            atomic_json(&journal, &record)?;
            record
        };
        let mut reply = if record.completed {
            Some(self.renewal_reply(&response, &current, allowed, queue_deadline)?)
        } else {
            None
        };
        if reply.is_none() {
            ensure!(allowed(), "cancelled");
            if current.refresh_expires_at.is_some_and(|t| t <= now()) {
                self.mark_auth_required(&current, allowed, queue_deadline)?;
                bail!("{AUTH_REQUIRED}");
            }
            {
                let _queue = self.renewal_queue(allowed, queue_deadline)?;
                ensure!(
                    self.credential(&current.scope, &current.server)? == current,
                    "cancelled"
                );
                record.issued = now();
                record.completed = false;
                atomic_json(&journal, &record)?;
            }
            match fs::remove_file(&response) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
            let scratch = Scratch::new(&self.state)?;
            let body = scratch.0.join("refresh");
            atomic_json(&body, &json!({"refreshToken":current.refresh_token}))?;
            // Keep a completed rotation recoverable even when its upload is cancelled.
            // Logout/new login can stop the request; neither can publish the old response.
            let result = self.raw_request(
                &current.server,
                None,
                "POST",
                "/v1/token/refresh",
                Some(&body),
                &response,
                JSON_LIMIT,
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_secs(30)),
                &|| {
                    self.same_login(&current)
                        && Instant::now() < deadline
                        && (queue_deadline.is_none() || allowed())
                },
            );
            if let Err(error) = result {
                if error.to_string().contains("auth_required") {
                    self.mark_auth_required(&current, allowed, queue_deadline)?;
                }
                if !self.same_login(&current) {
                    let _queue = self.renewal_queue(allowed, queue_deadline)?;
                    self.clear_renewal(&current.scope)?;
                }
                return Err(error);
            }
            File::open(&response)?.sync_all()?;
            File::open(&self.state)?.sync_all()?;
            record.completed = true;
            {
                #[cfg(any(test, feature = "test-support"))]
                test_support::run_hook("renewal_completion", &self.state);
                let _queue = self.renewal_queue(allowed, queue_deadline)?;
                ensure!(
                    self.credential(&current.scope, &current.server)? == current,
                    "cancelled"
                );
                atomic_json(&journal, &record)?;
            }
            reply = Some(self.renewal_reply(&response, &current, allowed, queue_deadline)?);
        }
        #[cfg(any(test, feature = "test-support"))]
        test_support::run_hook("renewal_response_persisted", &response);
        let applied = current.apply_reply(
            &reply.context("Invalid credential renewal response")?,
            record.issued,
            true,
        );
        if applied.is_err() {
            self.mark_auth_required(&record.base, allowed, queue_deadline)?;
            bail!("{AUTH_REQUIRED}: invalid renewal response");
        }
        ensure!(allowed(), "cancelled");
        #[cfg(any(test, feature = "test-support"))]
        test_support::run_hook("renewal_before_publication", &response);
        let _queue = self.renewal_queue(allowed, queue_deadline)?;
        // A late response is not a login and cannot restore credentials or config.
        ensure!(
            !self.revocation_pending() && self.cancellation()? == cancellation,
            "cancelled"
        );
        ensure!(
            self.credential(&current.scope, &current.server)? == record.base,
            "cancelled: device login changed"
        );
        atomic_json(
            &self.state.join(format!("{}.credential", current.scope)),
            &current,
        )?;
        self.clear_renewal(&current.scope)?;
        Ok(current)
    }
    pub(super) fn revoke_credential(
        &self,
        credential: &Credential,
        deadline: Instant,
    ) -> Result<()> {
        ensure!(Instant::now() < deadline, "Revocation deadline exceeded");
        let scratch = Scratch::new(&self.state)?;
        let body = scratch.0.join("revoke");
        atomic_json(&body, &json!({"refreshToken":credential.refresh_token}))?;
        self.raw_request(
            &credential.server,
            credential
                .refresh_token
                .is_none()
                .then_some(credential.token.as_str()),
            "POST",
            "/v1/token/revoke",
            Some(&body),
            &scratch.0.join("response"),
            JSON_LIMIT,
            deadline.saturating_duration_since(Instant::now()),
            &|| Instant::now() < deadline,
        )
    }
    pub(super) fn request(
        &self,
        server: &str,
        credential: Option<&Credential>,
        method: &str,
        route: &str,
        body: Option<&Path>,
        output: &Path,
        limit: u64,
        timeout: Duration,
        allowed: &dyn Fn() -> bool,
    ) -> Result<()> {
        self.request_with_queue_deadline(
            server, credential, method, route, body, output, limit, timeout, allowed, None,
        )
    }
    pub(super) fn request_with_queue_deadline(
        &self,
        server: &str,
        credential: Option<&Credential>,
        method: &str,
        route: &str,
        body: Option<&Path>,
        output: &Path,
        limit: u64,
        timeout: Duration,
        allowed: &dyn Fn() -> bool,
        queue_deadline: Option<Instant>,
    ) -> Result<()> {
        let Some(expected) = credential else {
            return self.raw_request(
                server, None, method, route, body, output, limit, timeout, allowed,
            );
        };
        ensure!(expected.server == server, "Credential server mismatch");
        let caller_allowed = allowed;
        let current_allowed = || caller_allowed() && !self.revocation_pending();
        let allowed: &dyn Fn() -> bool = &current_allowed;
        ensure!(allowed(), "cancelled");
        let deadline = queue_deadline.unwrap_or_else(|| Instant::now() + timeout);
        let mut current = self.current_credential(expected)?;
        let refresh_expired = current.refresh_expires_at.is_some_and(|t| t <= now());
        if refresh_expired
            || current
                .expires_at
                .is_some_and(|t| t <= now().saturating_add(30))
        {
            current = self.renew(&current, refresh_expired, allowed, deadline, queue_deadline)?;
        }
        let active = || allowed() && self.same_login(expected);
        let result = self.raw_request(
            server,
            Some(&current.token),
            method,
            route,
            body,
            output,
            limit,
            deadline.saturating_duration_since(Instant::now()),
            &active,
        );
        if result
            .as_ref()
            .err()
            .is_some_and(|e| e.to_string().contains("auth_required"))
        {
            let renewed = self.renew(&current, true, allowed, deadline, queue_deadline)?;
            ensure!(active(), "cancelled");
            fs::remove_file(output)?;
            let retry = self.raw_request(
                server,
                Some(&renewed.token),
                method,
                route,
                body,
                output,
                limit,
                deadline.saturating_duration_since(Instant::now()),
                &active,
            );
            if retry
                .as_ref()
                .err()
                .is_some_and(|e| e.to_string().contains("auth_required"))
            {
                self.mark_auth_required(&renewed, allowed, queue_deadline)?;
            }
            retry
        } else {
            result
        }
    }
}
