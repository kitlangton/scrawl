//! Account-authorized encrypted handoff. No secret or plaintext is sent to discovery.
use super::encryption::{Context as EncryptionContext, Peer};
use super::*;

const FORMAT: &str = "apparent-encrypted-v1";

fn group_files(files: &[Asset], group: &str) -> Vec<Asset> {
    files
        .iter()
        .filter_map(|file| {
            file.path
                .strip_prefix(&format!("{group}/"))
                .map(|path| Asset {
                    path: path.into(),
                    size: file.size,
                    sha256: file.sha256.clone(),
                })
        })
        .collect()
}

fn validate_grouped_transport(files: &[Asset]) -> Result<()> {
    ensure!(files.len() <= MAX_FILES, "invalid_manifest");
    let mut count = 0;
    let mut total = 0u64;
    for group in ["evidence", "media"] {
        let selected = group_files(files, group);
        encryption::validate_transport(&selected)?;
        count += selected.len();
        for file in selected {
            total = total.checked_add(file.size).context("quota_exceeded")?;
        }
    }
    ensure!(
        count == files.len() && total <= MAX_BYTES,
        "invalid_manifest"
    );
    Ok(())
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct EncryptedJob {
    context: EncryptionContext,
    recipient_grants: Vec<String>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PeerSet {
    account_id: String,
    grant_id: String,
    scope: String,
    devices: Vec<Peer>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredSpool {
    metadata: EncryptedJob,
    files: Vec<Asset>,
}

impl Client {
    /// Each group is authenticated independently; ordinary publication waits for
    /// all groups, while evidence publication carries an explicit local receipt.
    pub(super) fn fetch_grouped(
        &self,
        id: &str,
        server: &str,
        credential: &Credential,
        manifest: &Value,
        stage: &Path,
        out: &Path,
        parent: &Path,
        evidence_only: bool,
    ) -> Result<PathBuf> {
        let files: Vec<Asset> = serde_json::from_value(manifest["files"].clone())?;
        validate_grouped_transport(&files)?;
        let identity = self.endpoint_identity(credential)?;
        let peers = self.encryption_peers(credential, &|| true)?;
        let plain = Scratch::new_output(parent)?;
        let mut combined = Vec::new();
        let mut sender_binding = None;
        for group in ["evidence", "media"] {
            if evidence_only && group == "media" {
                continue;
            }
            let transport = stage.join(group);
            private_dir(&transport)?;
            for file in group_files(&files, group) {
                let destination = transport.join(&file.path);
                private_dir(destination.parent().context("invalid_path")?)?;
                self.request(
                    server,
                    Some(credential),
                    "GET",
                    &format!("/v1/captures/{id}/files/{group}/{}", file.path),
                    None,
                    &destination,
                    file.size.max(1),
                    Duration::from_secs(60),
                    &|| true,
                )?;
                ensure!(
                    fs::metadata(&destination)?.len() == file.size
                        && digest_file(&destination)? == file.sha256,
                    "encrypted_transport_integrity_mismatch"
                );
            }
            let context = encryption::envelope_context(&transport)?;
            ensure!(
                context.origin == server
                    && context.capture_id == id
                    && context.account_id == peers.account_id
                    && context.group.as_deref() == Some(group),
                "encrypted_context_mismatch"
            );
            let sender = peers
                .devices
                .iter()
                .find(|peer| peer.scope == "upload" && peer.grant_id == context.sender_grant_id)
                .context("encryption_sender_revoked_or_unapproved")?;
            let binding = (sender.grant_id.clone(), serde_json::to_value(&sender.key)?);
            if let Some(expected) = &sender_binding {
                ensure!(expected == &binding, "encrypted_context_mismatch");
            }
            sender_binding = Some(binding);
            let unpacked = Scratch::new_output(parent)?;
            let inner = encryption::unpack(&transport, &unpacked.0, &context, &identity, sender)?;
            for file in &inner {
                let destination = plain.0.join(&file.path);
                private_dir(destination.parent().context("invalid_path")?)?;
                ensure!(
                    !destination.exists(),
                    "invalid_manifest: overlapping groups"
                );
                fs::rename(unpacked.0.join(&file.path), destination)?;
            }
            combined.extend(inner);
        }
        if evidence_only {
            atomic_json(
                &plain.0.join(".evidence-only.json"),
                &json!({
                    "version":1,"captureId":id,"server":server,"media":"not downloaded",
                    "guidance":"Use apparent fetch <id> --media --out <new-directory> for the complete recording"
                }),
            )?;
        } else {
            validate_assets(&combined)?;
            validate_bundle_references(&plain.0, &combined)?;
        }
        let latest = self.encryption_peers(credential, &|| true)?;
        let (grant, key) = sender_binding.context("encryption_sender_revoked_or_unapproved")?;
        ensure!(
            latest.account_id == peers.account_id
                && latest.devices.iter().any(|peer| peer.scope == "upload"
                    && peer.grant_id == grant
                    && serde_json::to_value(&peer.key).ok().as_ref() == Some(&key)),
            "encryption_sender_revoked_or_unapproved"
        );
        publish_noreplace(&plain.0, out)?;
        File::open(parent)?.sync_all()?;
        Ok(out.to_path_buf())
    }
    fn endpoint_identity(&self, credential: &Credential) -> Result<encryption::Identity> {
        let id = credential.encryption_identity.as_deref()
            .filter(|id| valid_id(id))
            .context("encryption_identity_missing: approve this device with hostedEncryption enabled; never replace lost keys implicitly")?;
        encryption::load_identity(&self.state.join(format!("{id}.identity")))
            .map_err(|_| anyhow::anyhow!("encryption_identity_unavailable: local key is missing, invalid or not private; no plaintext fallback"))
    }

    fn encryption_peers(
        &self,
        credential: &Credential,
        allowed: &dyn Fn() -> bool,
    ) -> Result<PeerSet> {
        let reply = self.json_request(
            &credential.server,
            Some(credential),
            "GET",
            "/v1/encryption/peers",
            None,
            Duration::from_secs(30),
            allowed,
        )?;
        let peers: PeerSet = serde_json::from_value(reply)
            .map_err(|_| anyhow::anyhow!("invalid_encryption_authorization"))?;
        ensure!(
            !peers.account_id.is_empty()
                && peers.account_id.len() <= 256
                && peers.account_id.bytes().all(|b| b.is_ascii_graphic())
                && peers.devices.len() <= 64
                && credential.grant_id.as_deref() == Some(&peers.grant_id)
                && peers.scope == credential.scope,
            "invalid_encryption_authorization"
        );
        let mut ids = BTreeSet::new();
        for peer in &peers.devices {
            ensure!(
                !peer.grant_id.is_empty()
                    && peer.grant_id.len() <= 512
                    && peer.grant_id.bytes().all(|b| b.is_ascii_graphic())
                    && matches!(peer.scope.as_str(), "read" | "upload")
                    && ids.insert(&peer.grant_id),
                "invalid_encryption_authorization"
            );
            encryption::validate_public_key(&peer.key)?;
        }
        let own = peers
            .devices
            .iter()
            .find(|p| p.grant_id == peers.grant_id)
            .context("encryption_approval_required: this grant has no approved endpoint key")?;
        let identity = self.endpoint_identity(credential)?;
        ensure!(
            own.scope == credential.scope
                && serde_json::to_value(&own.key)? == identity.public_key(),
            "encryption_key_binding_mismatch"
        );
        Ok(peers)
    }

    fn encryption_recipients_current(&self, job: &Job, credential: &Credential) -> Result<()> {
        let peers = self.encryption_peers(credential, &|| self.active(job))?;
        let pinned = job
            .encryption
            .as_ref()
            .context("encryption_spool_invalid")?;
        ensure!(
            pinned.context.origin == job.server
                && pinned.context.capture_id == job.id
                && pinned.context.account_id == peers.account_id
                && pinned.context.sender_grant_id == peers.grant_id,
            "encryption_spool_binding_mismatch"
        );
        ensure!(
            pinned
                .recipient_grants
                .iter()
                .all(|id| peers.devices.iter().any(|p| &p.grant_id == id)),
            "encryption_restage_required: a recipient was revoked; explicitly upload under a fresh capture ID"
        );
        Ok(())
    }

    pub(super) fn upload_encrypted(&self, job: &mut Job) -> Result<()> {
        ensure!(!job.shared, "encrypted_sharing_unsupported");
        let credential = self.credential("upload", &job.server)?;
        let identity = self.endpoint_identity(&credential)?;
        let spool = self.state.join(format!("{}.sealed", job.id));
        let peers = self.encryption_peers(&credential, &|| self.active(job))?;
        if !spool.exists() {
            // Never regenerate ciphertext once its immutable manifest was committed.
            ensure!(
                job.files.is_empty() && job.encryption.is_none(),
                "encryption_spool_lost: explicitly upload the local bundle under a fresh capture ID"
            );
            let recipients: Vec<Peer> = peers
                .devices
                .iter()
                .filter(|p| p.scope == "read" || p.grant_id == peers.grant_id)
                .cloned()
                .collect();
            ensure!(
                recipients.iter().any(|p| p.scope == "read"),
                "encryption_pending_recipient: approved; waiting for an authorized agent device"
            );
            let context = EncryptionContext {
                origin: job.server.clone(),
                account_id: peers.account_id.clone(),
                capture_id: job.id.clone(),
                sender_grant_id: peers.grant_id.clone(),
                group: None,
            };
            let ordinary = Scratch::new(&self.state)?;
            let ordinary_files =
                snapshot(&job.bundle, &ordinary.0).context("encryption_snapshot_failed")?;
            let sealed = Scratch::new(&self.state)?;
            let files = if job.grouped {
                let mut files = Vec::new();
                for group in ["evidence", "media"] {
                    let source = Scratch::new(&self.state)?;
                    for asset in &ordinary_files {
                        if encryption::is_media_path(&asset.path) != (group == "media") {
                            continue;
                        }
                        let target = source.0.join(&asset.path);
                        private_dir(target.parent().context("invalid_asset")?)?;
                        fs::copy(ordinary.0.join(&asset.path), target)?;
                    }
                    let mut bound = context.clone();
                    bound.group = Some(group.into());
                    let destination = sealed.0.join(group);
                    private_dir(&destination)?;
                    for mut file in
                        encryption::pack(&source.0, &destination, &bound, &identity, &recipients)?
                    {
                        file.path = format!("{group}/{}", file.path);
                        files.push(file);
                    }
                }
                validate_grouped_transport(&files)?;
                files
            } else {
                encryption::pack(&ordinary.0, &sealed.0, &context, &identity, &recipients)
                    .context("encryption_pack_failed")?
            };
            let metadata = EncryptedJob {
                context,
                recipient_grants: recipients.iter().map(|p| p.grant_id.clone()).collect(),
            };
            atomic_json(
                &sealed.0.join("spool-state.json"),
                &StoredSpool { metadata, files },
            )
            .context("encryption_spool_receipt_failed")?;
            // Immutable spool comes before remote reservation. If publication of the
            // job is interrupted, the private spool receipt recovers these same bytes.
            let _lock = Lock::acquire(&self.state.join("queue.lock"), false)?;
            ensure!(self.active_locked(job), "cancelled");
            publish_noreplace(&sealed.0, &spool)?;
            File::open(&self.state)?.sync_all()?;
        }
        private_dir(&spool).context("encryption_spool_directory_unsafe")?;
        let saved: StoredSpool =
            serde_json::from_slice(&read_private(&spool.join("spool-state.json"), JSON_LIMIT)?)
                .map_err(|_| anyhow::anyhow!("encryption_spool_invalid"))?;
        if job.grouped {
            validate_grouped_transport(&saved.files)?;
        } else {
            encryption::validate_transport(&saved.files)?;
        }
        ensure!(
            job.files.is_empty() || same_manifest(&job.files, &saved.files),
            "encryption_spool_invalid"
        );
        job.files = saved.files;
        job.encryption = Some(saved.metadata);
        self.encryption_recipients_current(job, &credential)?;
        for file in &job.files {
            let source = spool.join(&file.path);
            no_symlinks(&source)?;
            ensure!(
                fs::metadata(&source)?.is_file()
                    && fs::metadata(&source)?.len() == file.size
                    && digest_file(&source)? == file.sha256,
                "encryption_spool_corrupt"
            );
        }
        {
            let _lock = Lock::acquire(&self.state.join("queue.lock"), false)?;
            ensure!(self.active_locked(job), "cancelled");
            self.save_job(job)?;
        }
        let allowed = || self.active(job);
        let base = format!("/v1/captures/{}", job.id);
        let reservation = self.json_request(
            &job.server,
            Some(&credential),
            "POST",
            &base,
            Some(if job.grouped {
                json!({"format":FORMAT,"delivery":"evidence-first-v1","files":job.files})
            } else {
                json!({"format":FORMAT,"files":job.files})
            }),
            Duration::from_secs(30),
            &allowed,
        )?;
        ensure!(
            reservation["id"].as_str() == Some(&job.id)
                && reservation["format"].as_str() == Some(FORMAT),
            "encryption_downgrade_refused"
        );
        ensure!(
            if job.grouped {
                reservation["delivery"] == "evidence-first-v1"
            } else {
                reservation["delivery"].is_null()
            },
            "unsupported_encryption_delivery"
        );
        if reservation["status"] != "ready" {
            ensure!(
                reservation["status"] == "pending",
                "invalid_encrypted_capture_status"
            );
            let ack = Scratch::new(&self.state)?;
            for file in &job.files {
                // Revocation observed at any transfer boundary stops the remaining
                // sends. Bytes already transmitted cannot be recalled.
                self.encryption_recipients_current(job, &credential)?;
                self.request(
                    &job.server,
                    Some(&credential),
                    "PUT",
                    &format!("{base}/files/{}", file.path),
                    Some(&spool.join(&file.path)),
                    &ack.0.join(random_id()?),
                    JSON_LIMIT,
                    Duration::from_secs(60),
                    &allowed,
                )?;
            }
            self.encryption_recipients_current(job, &credential)?;
            let completed = self.json_request(
                &job.server,
                Some(&credential),
                "POST",
                &format!("{base}/complete"),
                None,
                Duration::from_secs(30),
                &allowed,
            )?;
            ensure!(
                completed["id"].as_str() == Some(&job.id)
                    && completed["status"] == "ready"
                    && completed["format"].as_str() == Some(FORMAT),
                "encrypted_completion_failed"
            );
        }
        let verified = self.json_request(
            &job.server,
            Some(&credential),
            "GET",
            &base,
            None,
            Duration::from_secs(30),
            &allowed,
        )?;
        let files: Vec<Asset> = serde_json::from_value(verified["files"].clone())?;
        ensure!(
            verified["id"].as_str() == Some(&job.id)
                && verified["status"] == "ready"
                && verified["format"].as_str() == Some(FORMAT)
                && same_manifest(&files, &job.files),
            "encrypted_completion_failed"
        );
        Ok(())
    }

    pub(super) fn fetch_encrypted(
        &self,
        id: &str,
        server: &str,
        credential: &Credential,
        manifest: &Value,
        stage: &Path,
        out: &Path,
        parent: &Path,
    ) -> Result<PathBuf> {
        let files: Vec<Asset> = serde_json::from_value(manifest["files"].clone())?;
        encryption::validate_transport(&files)?;
        let identity = self.endpoint_identity(credential)?;
        let peers = self.encryption_peers(credential, &|| true)?;
        for file in &files {
            let destination = stage.join(&file.path);
            if let Some(parent) = destination.parent() {
                private_dir(parent)?;
            }
            self.request(
                server,
                Some(credential),
                "GET",
                &format!("/v1/captures/{id}/files/{}", file.path),
                None,
                &destination,
                file.size.max(1),
                Duration::from_secs(60),
                &|| true,
            )?;
            ensure!(
                fs::metadata(&destination)?.len() == file.size
                    && digest_file(&destination)? == file.sha256,
                "encrypted_transport_integrity_mismatch"
            );
        }
        let context = encryption::envelope_context(stage)?;
        ensure!(context.group.is_none(), "encrypted_context_mismatch");
        ensure!(
            context.origin == server
                && context.capture_id == id
                && context.account_id == peers.account_id,
            "encrypted_context_mismatch"
        );
        let sender = peers
            .devices
            .iter()
            .find(|p| p.scope == "upload" && p.grant_id == context.sender_grant_id)
            .context("encryption_sender_revoked_or_unapproved")?;
        let plain = Scratch::new_output(parent)?;
        let inner = encryption::unpack(stage, &plain.0, &context, &identity, sender)?;
        validate_assets(&inner)?;
        validate_bundle_references(&plain.0, &inner)?;
        // Recheck authorization after download/decrypt, before publishing plaintext.
        let latest = self.encryption_peers(credential, &|| true)?;
        ensure!(
            latest.account_id == peers.account_id
                && latest.devices.iter().any(|p| p.grant_id == sender.grant_id
                    && serde_json::to_value(&p.key).ok() == serde_json::to_value(&sender.key).ok()),
            "encryption_sender_revoked_or_unapproved"
        );
        publish_noreplace(&plain.0, out)?;
        File::open(parent)?.sync_all()?;
        Ok(out.to_path_buf())
    }
}
