//! Secret-free curl diagnostics. Never include stderr, headers, URLs or response text.
use super::*;
use std::time::Duration;

pub(super) const CONNECT_TIMEOUT_SECS: u64 = 15;

pub(super) fn phase(method: &str, route: &str) -> &'static str {
    match (method, route) {
        ("POST", "/v1/device/start") => "device authorization start",
        ("POST", "/v1/device/poll") => "device approval polling",
        ("POST", "/v1/token/refresh") => "credential renewal",
        ("GET", "/v1/account") => "account check",
        ("GET", "/v1/encryption/peers") => "encryption recipient discovery",
        _ if route.starts_with("/v1/captures/") => "capture transfer",
        _ => "API request",
    }
}

pub(super) fn failure(
    stage: &str,
    exit: Option<i32>,
    http: Option<u16>,
    elapsed: Duration,
    timeout: Duration,
) -> String {
    let kind = match exit {
        Some(5) => "proxy DNS resolution failed",
        Some(6) => "host DNS resolution failed",
        Some(7) => "connection failed",
        Some(28) => "timeout",
        Some(35) => "TLS handshake failed",
        Some(51 | 58 | 60 | 77 | 83 | 90 | 91) => {
            "TLS certificate verification or configuration failed"
        }
        Some(63) => "response size limit exceeded",
        Some(97) => "proxy handshake failed",
        Some(18 | 52 | 55 | 56) => "connection interrupted",
        _ => "transport failed",
    };
    format!(
        "Hosted transport failed during {stage}: {kind} (curl_exit={}, http_status={}, elapsed_ms={}, connect_timeout_ms={}, request_timeout_ms={}). Raw network diagnostics are suppressed to protect credentials and proxy settings.",
        exit.map(|value| value.to_string())
            .unwrap_or_else(|| "signal".into()),
        http.map(|value| value.to_string())
            .unwrap_or_else(|| "unavailable".into()),
        elapsed.as_millis(),
        timeout
            .min(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .as_millis(),
        timeout.as_millis(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classifies_curl_failures_without_remote_details() {
        for (code, expected) in [
            (5, "proxy DNS"),
            (6, "host DNS"),
            (7, "connection failed"),
            (28, "timeout"),
            (35, "TLS handshake"),
            (60, "TLS certificate"),
            (63, "response size"),
            (97, "proxy handshake"),
            (56, "interrupted"),
            (99, "transport failed"),
        ] {
            let message = failure(
                phase("POST", "/v1/device/start"),
                Some(code),
                Some(0),
                Duration::from_millis(2800),
                Duration::from_secs(30),
            );
            assert!(message.contains(expected), "{message}");
            assert!(
                message.contains("device authorization start")
                    && message.contains("elapsed_ms=2800")
            );
            assert!(message.contains("connect_timeout_ms=15000"));
        }
        assert_eq!(phase("POST", "/v1/device/poll"), "device approval polling");
        assert_eq!(
            phase("GET", "/v1/captures/private-id/files/private-name"),
            "capture transfer"
        );
        assert_eq!(phase("GET", "/v1/untrusted-secret"), "API request");
        let message = failure(
            phase("GET", "/v1/untrusted-secret"),
            None,
            None,
            Duration::ZERO,
            Duration::from_millis(100),
        );
        assert!(!message.contains("untrusted-secret"));
        assert!(message.contains("connect_timeout_ms=100"));
    }
}

impl Client {
    pub(super) fn raw_request(
        &self,
        server: &str,
        token: Option<&str>,
        method: &str,
        route: &str,
        body: Option<&Path>,
        output: &Path,
        limit: u64,
        timeout: Duration,
        allowed: &dyn Fn() -> bool,
    ) -> Result<()> {
        let url = api_url(server, route)?;
        ensure!(allowed(), "cancelled");
        let mut config = format!(
            "url = \"{}\"\nrequest = \"{method}\"\n",
            config_escape(&url)?
        );
        if let Some(token) = token {
            config.push_str(&format!(
                "header = \"Authorization: Bearer {}\"\n",
                config_escape(token)?
            ));
        }
        if let Some(body) = body {
            config.push_str(&format!(
                "data-binary = \"@{}\"\n",
                config_escape(&body.to_string_lossy())?
            ));
            config.push_str(if method == "PUT" {
                "header = \"Content-Type: application/octet-stream\"\n"
            } else {
                "header = \"Content-Type: application/json\"\n"
            });
        }
        let response = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(output)?;
        drop(response);
        let timeout = timeout
            .max(Duration::from_millis(1))
            .min(Duration::from_secs(60));
        let started = Instant::now();
        let stage = transport::phase(method, route);
        let mut child = Command::new("curl")
            .args([
                "--disable",
                "--silent",
                "--show-error",
                "--no-netrc",
                "--proto",
                "=http,https",
                "--max-redirs",
                "0",
                "--connect-timeout",
            ])
            .arg(transport::CONNECT_TIMEOUT_SECS.to_string())
            .arg("--max-time")
            .arg(format!("{:.3}", timeout.as_secs_f64()))
            .args(["--max-filesize", &limit.to_string(), "--output"])
            .arg(output)
            .args(["--write-out", "%{http_code}", "--config", "-"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("curl is required for hosted transport")?;
        let input_result = child
            .stdin
            .take()
            .context("Missing curl stdin")?
            .write_all(config.as_bytes());
        // Do not retain a second token-bearing config during polling.
        drop(config);
        if input_result.is_err() {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Hosted transport input failed");
        }
        let deadline = Instant::now() + timeout + Duration::from_millis(250);
        let status = loop {
            #[cfg(any(test, feature = "test-support"))]
            test_support::run_hook("transport_poll", body.unwrap_or(output));
            if !allowed() || Instant::now() > deadline {
                let _ = child.kill();
                let _ = child.wait();
                ensure!(allowed(), "cancelled");
                bail!("Hosted request timed out during {stage} (elapsed_ms={}, request_timeout_ms={}, curl_exit=unavailable; child terminated)", started.elapsed().as_millis(), timeout.as_millis());
            }
            if let Some(status) = child.try_wait()? {
                break status;
            }
            std::thread::sleep(Duration::from_millis(20));
        };
        let mut code = String::new();
        child
            .stdout
            .take()
            .context("Missing curl stdout")?
            .take(16)
            .read_to_string(&mut code)?;
        #[cfg(test)]
        if !status.success() {
            // Numeric evidence only; config, paths, stderr and response text
            // can carry credentials or user data and must not be printed.
            eprintln!(
                "hosted curl failure: exit={:?}; http={:?}; body_bytes={:?}; response_bytes={:?}",
                status.code(),
                code.trim().parse::<u16>().ok(),
                body.and_then(|path| fs::metadata(path).ok())
                    .map(|m| m.len()),
                fs::metadata(output).ok().map(|m| m.len()),
            );
        }
        ensure!(status.success(), "{}", transport::failure(
            stage, status.code(), code.trim().parse::<u16>().ok().filter(|code| *code <= 599),
            started.elapsed(), timeout,
        ));
        ensure!(
            fs::metadata(output)?.len() <= limit,
            "Hosted transport response size limit exceeded during {stage} (curl_exit=0, elapsed_ms={}, response_limit_bytes={limit}; client size guard)", started.elapsed().as_millis()
        );
        if !code.starts_with('2') {
            if code.trim() == "401" {
                bail!("auth_required: approve this device again with apparent connect login");
            }
            if code.trim() == "429" || code.starts_with('5') {
                bail!("Hosted API temporarily unavailable; retry later");
            }
            // Only expose a known non-secret error code, not remote text or curl stderr.
            let error = fs::read(output)
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
            let code = error
                .as_ref()
                .and_then(|v| v.pointer("/error/code"))
                .and_then(Value::as_str)
                .unwrap_or("http_error");
            let safe = match code {
                "file_too_large"
                | "manifest_conflict"
                | "beta_required"
                | "slow_down"
                | "quota_exceeded"
                | "unauthorized"
                | "forbidden"
                | "expired"
                | "rejected"
                | "not_found"
                | "conflict"
                | "authorization_pending"
                | "device_expired"
                | "device_rejected"
                | "reauthorization_required" => code,
                _ => "http_error",
            };
            bail!("Hosted API: {safe}");
        }
        ensure!(allowed(), "cancelled");
        Ok(())
    }
    pub(super) fn json_request(
        &self,
        server: &str,
        token: Option<&Credential>,
        method: &str,
        route: &str,
        body: Option<Value>,
        timeout: Duration,
        allowed: &dyn Fn() -> bool,
    ) -> Result<Value> {
        self.json_request_until(server, token, method, route, body, timeout, allowed, None)
    }
    pub(super) fn json_request_until(
        &self,
        server: &str,
        token: Option<&Credential>,
        method: &str,
        route: &str,
        body: Option<Value>,
        timeout: Duration,
        allowed: &dyn Fn() -> bool,
        queue_deadline: Option<Instant>,
    ) -> Result<Value> {
        let scratch = Scratch::new(&self.state)?;
        let body_path = scratch.0.join("body");
        if let Some(body) = &body {
            atomic_json(&body_path, body)?;
        }
        let response = scratch.0.join("response");
        self.request_with_queue_deadline(
            server,
            token,
            method,
            route,
            body.as_ref().map(|_| body_path.as_path()),
            &response,
            JSON_LIMIT,
            timeout,
            allowed,
            queue_deadline,
        )?;
        serde_json::from_slice(&fs::read(response)?).context("Invalid hosted JSON response")
    }
}

impl Client {
    pub fn fetch(&self, id: &str, out: &Path, wait: u64) -> Result<PathBuf> {
        self.fetch_inner(id, out, wait, false).map_err(|error| self.fetch_error(error))
    }
    pub(super) fn fetch_error(&self, error: anyhow::Error) -> anyhow::Error {
        match error.to_string().as_str() {
            "Hosted API: not_found" => anyhow::anyhow!("Capture unavailable (not_found). The sender may still be uploading, the capture may have expired or been deleted, or this account/API origin may not have access. Check apparent connect status and confirm the API origin and approved read account with the sender; ask the sender to check upload status or provide a fresh handoff. Retry with --wait 120 if upload is still in progress."),
            message if message.starts_with("auth_required:") || matches!(message, "Hosted API: unauthorized" | "Hosted API: forbidden" | "Hosted API: reauthorization_required") => anyhow::anyhow!("Capture read authorization failed. Run `{}` and ask the user to approve its verification link/code, then retry inspection.", self.read_login_command().unwrap_or_else(|_| "apparent connect login".into())),
            _ => error,
        }
    }
    /// Fetch an immutable evidence snapshot; legacy captures retain full retrieval.
    pub fn fetch_evidence(&self, id: &str, out: &Path, wait: u64) -> Result<PathBuf> {
        self.fetch_inner(id, out, wait, true).map_err(|error| self.fetch_error(error))
    }
    pub(super) fn fetch_inner(&self, id: &str, out: &Path, wait: u64, evidence: bool) -> Result<PathBuf> {
        self.fetch_recipient(id, out, wait, evidence, &|| true)
    }
    pub fn fetch_recipient(&self, id: &str, out: &Path, wait: u64, evidence: bool, allowed: &dyn Fn() -> bool) -> Result<PathBuf> {
        ensure!(
            valid_id(id),
            "Capture ID must be 32 lowercase hexadecimal characters"
        );
        ensure!(wait <= 900, "--wait must be between 0 and 900 seconds");
        let server = self.login_server(None)?;
        let login = self.read_login_command()?;
        let credential = self.credential("read", &server).with_context(|| format!(
            "Read approval required. Run `{login}`, give the verification link and code to the user for approval, then retry this command. Upload consent is not required."
        ))?;
        let out = std::path::absolute(out)?;
        no_symlinks(&out)?;
        ensure!(
            fs::symlink_metadata(&out).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
            "Output already exists; choose a new directory (nothing is overwritten)"
        );
        let parent = out.parent().context("Missing output parent")?;
        ensure!(parent.is_dir(), "Output parent must already exist");
        trusted_directory_tree(parent, true)?;
        let parent = parent.canonicalize()?;
        let out = parent.join(out.file_name().context("Invalid output name")?);
        let stage = Scratch::new_output(&parent)?;
        #[cfg(any(test, feature = "test-support"))]
        test_support::run_hook("fetch_stage", &stage.0);
        let deadline = Instant::now() + Duration::from_secs(wait);
        let base = format!("/v1/captures/{id}");
        let manifest = loop {
            let timeout = if wait == 0 {
                Duration::from_secs(30)
            } else {
                deadline
                    .saturating_duration_since(Instant::now())
                    .min(Duration::from_secs(30))
            };
            ensure!(
                !timeout.is_zero(),
                "Capture is still pending; retry fetch later"
            );
            let reply = self.json_request(
                &server,
                Some(&credential),
                "GET",
                &base,
                None,
                timeout,
                allowed,
            );
            // A copied pointer can precede server reservation. A 404 also hides
            // inaccessible captures: wait only within the caller's budget and
            // never infer existence or retry authorization failures.
            let reply = match reply {
                Ok(reply) => reply,
                Err(error) if error.to_string() == "Hosted API: not_found"
                    && wait > 0 && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(500)
                        .min(deadline.saturating_duration_since(Instant::now())));
                    if Instant::now() >= deadline { return Err(error); }
                    continue;
                }
                Err(error) => return Err(error),
            };
            ensure!(
                reply["id"].as_str() == Some(id),
                "Wrong capture ID in manifest"
            );
            if evidence && reply["delivery"] == "evidence-first-v1"
                && reply["evidenceReady"] == true
            {
                ensure!(matches!(reply["status"].as_str(), Some("pending" | "ready")), "Invalid capture status");
                break reply;
            }
            match reply["status"].as_str() {
                Some("ready") => break reply,
                Some("pending") => {
                    ensure!(
                        wait > 0 && Instant::now() < deadline,
                        "Capture is still pending; retry fetch later"
                    );
                    std::thread::sleep(
                        Duration::from_millis(500)
                            .min(deadline.saturating_duration_since(Instant::now())),
                    );
                }
                _ => bail!("Invalid capture status"),
            }
        };
        let files: Vec<Asset> =
            serde_json::from_value(manifest["files"].clone()).context("Invalid hosted manifest")?;
        match manifest.get("format").and_then(Value::as_str).unwrap_or("plaintext-v1") {
            "apparent-encrypted-v1" => {
                ensure!(credential.encryption_identity.is_some(), "encryption_approval_required: run apparent connect login to approve this agent's decryption key, then ask the sender to retry the upload");
                if manifest["delivery"] == "evidence-first-v1" {
                    return self.fetch_grouped(id, &server, &credential, &manifest, &stage.0, &out, &parent, evidence);
                }
                ensure!(manifest["delivery"].is_null(), "unsupported_encryption_delivery");
                return self.fetch_encrypted(id, &server, &credential, &manifest, &stage.0, &out, &parent);
            }
            "plaintext-v1" => ensure!(!self.provider.experiment(HOSTED_ENCRYPTION), "encryption_downgrade_refused: capture is explicitly plaintext; disable the experiment only to deliberately fetch legacy content"),
            _ => bail!("unsupported_encryption_format"),
        }
        validate_assets(&files)?;
        for file in &files {
            let destination = stage.0.join(&file.path);
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent)?;
            }
            self.request(
                &server,
                Some(&credential),
                "GET",
                &format!("{base}/files/{}", file.path),
                None,
                &destination,
                file.size.max(1),
                Duration::from_secs(60),
                allowed,
            )?;
            ensure!(
                fs::metadata(&destination)?.len() == file.size
                    && digest_file(&destination)? == file.sha256,
                "Hosted file integrity mismatch"
            );
        }
        validate_bundle_references(&stage.0, &files)?;
        ensure!(allowed(), "cancelled");
        // Atomic no-replace publish. Linux/macOS use their kernel's exclusive rename.
        publish_noreplace(&stage.0, &out)?;
        File::open(&parent)?.sync_all()?;
        Ok(out)
    }
}
