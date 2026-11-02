//! Account-authorized encrypted handoff. No secret or plaintext is sent to discovery.
use super::encryption::{Context as EncryptionContext, Peer};
use super::*;

const FORMAT: &str = "apparent-encrypted-v1";

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
            };
            let ordinary = Scratch::new(&self.state)?;
            snapshot(&job.bundle, &ordinary.0).context("encryption_snapshot_failed")?;
            let sealed = Scratch::new(&self.state)?;
            let files = encryption::pack(&ordinary.0, &sealed.0, &context, &identity, &recipients)
                .context("encryption_pack_failed")?;
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
        encryption::validate_transport(&saved.files)?;
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
            Some(json!({"format":FORMAT,"files":job.files})),
            Duration::from_secs(30),
            &allowed,
        )?;
        ensure!(
            reservation["id"].as_str() == Some(&job.id)
                && reservation["format"].as_str() == Some(FORMAT),
            "encryption_downgrade_refused"
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
