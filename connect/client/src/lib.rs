//! Default-off hosted handoff, shared by macOS and Linux. No network on finish/paste.
//! Secrets go only through curl's stdin config, never argv or diagnostic output.
use std::collections::BTreeSet;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail, ensure};
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
mod renewal;

/// Application-owned preferences and browser mechanics. Implementations must
/// perform fresh reads, preserve unrelated config, and serialize each write.
/// Called with queue.lock held for publication; never acquire queue.lock here.
pub trait ConfigProvider: Send + Sync {
    fn config_path(&self) -> &Path;
    fn cloud(&self) -> CloudConfig;
    fn experiment(&self, name: &str) -> bool;
    fn set_cloud(&self, value: &CloudConfig) -> Result<()>;
    fn set_experiment(&self, name: &str, enabled: bool) -> Result<()>;
    fn open_browser(&self, _uri: &Path) {}
}

pub const HOSTED_SYNC: &str = "hostedSync";
pub const HOSTED_SHARING: &str = "hostedSharing";

/// Only the foreground caller may put this result on the clipboard. Detached
/// workers never own clipboard access, including retries and account completion.
pub struct PreparedLink {
    pub text: String,
    job: Job,
    client: Client,
}

impl PreparedLink {
    pub fn authorized(&self) -> bool {
        self.client.provider.experiment(HOSTED_SHARING) && self.client.share_active(&self.job)
    }
}

const MAX_BYTES: u64 = 500_000_000;
const MAX_FILES: usize = 512;
const MAX_FILE_BYTES: u64 = 90_000_000;
const MAX_JOBS: usize = 100;
const MAX_ATTEMPTS: u32 = 8;
const RETENTION: u64 = 7 * 24 * 60 * 60;
const JSON_LIMIT: u64 = 2_000_000;

#[derive(Subcommand)]
pub enum ConnectCommand {
    /// Sign in through a device code. Upload requires explicit --consent-upload.
    Login {
        #[arg(long)]
        server: String,
        #[arg(long, default_value = "read", value_parser = ["upload", "read"])]
        scope: String,
        #[arg(long)]
        consent_upload: bool,
        /// Print the verification link without opening a browser.
        #[arg(long)]
        no_browser: bool,
    },
    /// Inspect local queue and credentials (never prints tokens); --account also reads quota.
    Status {
        #[arg(long)]
        account: bool,
    },
    /// Disable automatic uploads, cancel pending jobs and remove both local tokens.
    Logout,
    /// Disable automatic uploads and cancel pending jobs, retaining credentials.
    Disable,
    /// Retry failed queued captures. Does not discover or upload old captures.
    Retry,
    /// Explicitly upload one historical capture (requires upload consent).
    Upload { bundle: PathBuf },
    /// List or revoke public-by-link access. IDs are management IDs, not capture IDs.
    Shares {
        #[command(subcommand)]
        command: SharesCommand,
    },
}

#[derive(Subcommand)]
pub enum SharesCommand {
    /// List shared captures (read or upload login).
    List,
    /// Stop future access without deleting the private capture (upload login).
    Revoke { id: String },
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CloudConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub upload_consent: bool,
    #[serde(default)]
    pub server: String,
}

#[derive(Clone, Deserialize, Serialize, PartialEq, Eq)]
struct Credential {
    server: String,
    #[serde(default)]
    site: Option<String>,
    scope: String,
    token: String,
    generation: String,
    #[serde(default, rename = "refreshToken")]
    refresh_token: Option<String>,
    #[serde(default, rename = "expiresAt")]
    expires_at: Option<u64>,
    #[serde(default, rename = "refreshExpiresAt")]
    refresh_expires_at: Option<u64>,
    #[serde(default, rename = "grantId")]
    grant_id: Option<String>,
    #[serde(default, rename = "authRequired")]
    auth_required: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
struct Asset {
    path: String,
    size: u64,
    sha256: String,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct Job {
    id: String,
    bundle: PathBuf,
    server: String,
    generation: String,
    created: u64,
    attempts: u32,
    next_attempt: u64,
    status: String,
    error: Option<String>,
    #[serde(default)]
    files: Vec<Asset>,
    #[serde(default)]
    shared: bool,
}

#[derive(Clone)]
pub struct Client {
    provider: std::sync::Arc<dyn ConfigProvider>,
    #[cfg(any(test, feature = "test-support"))]
    config: PathBuf,
    state: PathBuf,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn random_id() -> Result<String> {
    let mut bytes = [0u8; 16];
    File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}
fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn hex_digest(s: &str) -> bool {
    s.len() == 64
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn validate_share_url(server: &str, url: &str) -> Result<()> {
    ensure!(
        url.strip_prefix(&format!("{server}/s/"))
            .is_some_and(hex_digest),
        "Invalid share URL"
    );
    Ok(())
}

// Exact allowlist: unknown files are not uploaded, and are rejected on download.
fn asset_path(path: &str) -> bool {
    if path.len() > 128
        || !path.is_ascii()
        || path.contains('\\')
        || path.split('/').any(|p| p.is_empty() || p.starts_with('.'))
    {
        return false;
    }
    match path.split('/').collect::<Vec<_>>().as_slice() {
        [name] => matches!(
            *name,
            "summary.md"
                | "apparent.json"
                | "contact-sheet.png"
                | "contact-sheet-full.png"
                | "recording.mp4"
                | "recording.mov"
                | "audio.wav"
                | "timeline.json"
                | "finish-trace.json"
        ),
        [dir, name] if matches!(*dir, "frames" | "frames-full") => name
            .strip_prefix("frame-")
            .and_then(|s| s.strip_suffix(".png"))
            .is_some_and(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit())),
        _ => false,
    }
}
fn validate_assets(files: &[Asset]) -> Result<()> {
    ensure!(
        !files.is_empty() && files.len() <= MAX_FILES,
        "invalid_manifest: file count"
    );
    let mut names = BTreeSet::new();
    let mut size = 0u64;
    for file in files {
        ensure!(
            file.size <= MAX_FILE_BYTES,
            "file_too_large: an asset exceeds the v1 90,000,000-byte per-file limit; no files were omitted"
        );
        ensure!(
            asset_path(&file.path) && hex_digest(&file.sha256) && names.insert(file.path.clone()),
            "invalid_manifest: unsafe or duplicate file"
        );
        size = size
            .checked_add(file.size)
            .context("invalid_manifest: size overflow")?;
        ensure!(
            size <= MAX_BYTES,
            "quota_exceeded: bundle exceeds account limit"
        );
    }
    for required in ["apparent.json", "summary.md", "contact-sheet.png"] {
        ensure!(
            names.contains(required),
            "invalid_manifest: missing {required}"
        );
    }
    Ok(())
}

fn same_manifest(a: &[Asset], b: &[Asset]) -> bool {
    let mut a = a.to_vec();
    let mut b = b.to_vec();
    a.sort_by(|a, b| a.path.cmp(&b.path));
    b.sort_by(|a, b| a.path.cmp(&b.path));
    a == b
}

fn server_url(raw: &str) -> Result<String> {
    let raw = raw.trim_end_matches('/');
    ensure!(
        raw.len() <= 2048
            && raw.is_ascii()
            && !raw.bytes().any(|b| b.is_ascii_control()
                || b.is_ascii_whitespace()
                || b"\"'\\?#@".contains(&b)),
        "Invalid server URL"
    );
    let authority = raw
        .strip_prefix("https://")
        .or_else(|| raw.strip_prefix("http://"))
        .context("Server must use HTTPS")?;
    ensure!(
        !authority.is_empty() && !authority.contains('/'),
        "Server must be an origin without a path"
    );
    let host = authority.split(':').next().unwrap_or("");
    ensure!(
        !host.is_empty()
            && host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-".contains(&b)),
        "Invalid server host"
    );
    if let Some((_, port)) = authority.split_once(':') {
        ensure!(
            !port.is_empty() && port.parse::<u16>().is_ok_and(|n| n > 0),
            "Invalid server port"
        );
    }
    ensure!(
        raw.starts_with("https://") || matches!(host, "127.0.0.1" | "localhost"),
        "Plain HTTP is allowed only on loopback"
    );
    Ok(raw.to_owned())
}

// Reject symlinks at every component. State directories are private and owned by this uid.
fn no_symlinks(path: &Path) -> Result<()> {
    let mut prefix = PathBuf::new();
    for component in path.components() {
        ensure!(
            !matches!(component, std::path::Component::ParentDir),
            "Unsafe path"
        );
        prefix.push(component);
        if let Ok(meta) = fs::symlink_metadata(&prefix) {
            ensure!(!meta.file_type().is_symlink(), "Symlink path refused");
        }
    }
    Ok(())
}
// Pathname operations are safe only inside a tree other users cannot replace.
// Root and this uid are trusted; hostile same-uid processes are outside this boundary.
// A sticky shared ancestor protects an owned child, but a source asset directory
// itself must not be shared writable (sticky does not prevent inserting entries).
fn trusted_directory_tree(path: &Path, sticky_leaf: bool) -> Result<()> {
    let path = std::path::absolute(path)?;
    let mut prefix = PathBuf::new();
    let mut private_ancestor = false;
    for component in path.components() {
        ensure!(
            !matches!(component, std::path::Component::ParentDir),
            "Unsafe path"
        );
        prefix.push(component);
        let meta = fs::symlink_metadata(&prefix)?;
        let sticky = meta.mode() & libc::S_ISVTX as u32 != 0;
        #[cfg(target_os = "macos")]
        no_allow_acl(&prefix)?;
        ensure!(
            meta.is_dir()
                && !meta.file_type().is_symlink()
                && (meta.uid() == 0 || meta.uid() == unsafe { libc::geteuid() })
                && (meta.mode() & 0o022 == 0
                    || (sticky && (prefix != path || sticky_leaf))
                    || (private_ancestor && prefix != path)),
            "Unsafe shared or unowned directory tree"
        );
        // A private, trusted ancestor prevents other uids reaching descendants.
        private_ancestor |= meta.mode() & 0o077 == 0;
    }
    Ok(())
}
// Darwin ACL grants can override mode bits (including a private ancestor's
// search permissions). Fail closed on grants, without changing the user's ACL.
// Keep deny-only ACLs such as the standard home-folder delete protection.
#[cfg(target_os = "macos")]
fn no_allow_acl(path: &Path) -> Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    unsafe extern "C" {
        fn acl_get_link_np(path: *const libc::c_char, kind: libc::c_int) -> *mut libc::c_void;
        fn acl_get_entry(
            acl: *mut libc::c_void,
            entry_id: libc::c_int,
            entry: *mut *mut libc::c_void,
        ) -> libc::c_int;
        fn acl_get_tag_type(entry: *mut libc::c_void, tag: *mut libc::c_int) -> libc::c_int;
        fn acl_free(acl: *mut libc::c_void) -> libc::c_int;
    }
    let path = CString::new(path.as_os_str().as_bytes())?;
    // ACL_TYPE_EXTENDED and ACL_FIRST_ENTRY / ACL_NEXT_ENTRY from sys/acl.h.
    let acl = unsafe { acl_get_link_np(path.as_ptr(), 0x100) };
    if acl.is_null() {
        ensure!(
            std::io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT),
            "Could not verify directory ACL"
        );
        return Ok(());
    }
    let result = (|| -> Result<()> {
        let mut entry = std::ptr::null_mut();
        let mut selector = 0;
        loop {
            let found = unsafe { acl_get_entry(acl, selector, &mut entry) };
            if found != 0 {
                ensure!(
                    std::io::Error::last_os_error().raw_os_error() == Some(libc::EINVAL),
                    "Could not verify directory ACL entries"
                );
                return Ok(());
            }
            let mut tag = 0;
            ensure!(
                unsafe { acl_get_tag_type(entry, &mut tag) } == 0 && tag == 2,
                "Directory ACL grants are not supported for hosted transfers"
            );
            selector = -1;
        }
    })();
    unsafe {
        acl_free(acl);
    }
    result
}
fn private_dir(path: &Path) -> Result<()> {
    no_symlinks(path)?;
    if !path.exists() {
        if let Some(parent) = path.parent() {
            create_trusted_parents(parent)?;
        }
        match fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.into()),
        }
    }
    trusted_directory_tree(path, false)?;
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_dir()
            && !meta.file_type().is_symlink()
            && meta.uid() == unsafe { libc::geteuid() }
            && meta.mode() & 0o077 == 0,
        "Hosted state directory must be owned and mode 0700"
    );
    Ok(())
}
fn create_trusted_parents(path: &Path) -> Result<()> {
    if !path.exists() {
        create_trusted_parents(path.parent().context("Missing trusted parent")?)?;
        match fs::DirBuilder::new().mode(0o700).create(path) {
            Ok(()) => (),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.into()),
        }
    }
    trusted_directory_tree(path, true)
}
fn read_private(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    let meta = file.metadata()?;
    ensure!(
        meta.is_file()
            && meta.uid() == unsafe { libc::geteuid() }
            && meta.mode() & 0o077 == 0
            && meta.len() <= limit,
        "Hosted state file must be owned, private and bounded"
    );
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    ensure!(bytes.len() as u64 <= limit, "Hosted state too large");
    Ok(bytes)
}
fn atomic_json(path: &Path, value: &impl Serialize) -> Result<()> {
    private_dir(path.parent().context("Missing parent")?)?;
    let tmp = path.with_extension(format!("{}.tmp", random_id()?));
    let result = (|| -> Result<()> {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&tmp)?;
        f.write_all(&serde_json::to_vec(value)?)?;
        f.sync_all()?;
        fs::rename(&tmp, path)?;
        File::open(path.parent().unwrap())?.sync_all()?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}
struct Lock(File);
impl Lock {
    fn acquire(path: &Path, nonblocking: bool) -> Result<Self> {
        let f = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(path)?;
        let op = libc::LOCK_EX | if nonblocking { libc::LOCK_NB } else { 0 };
        let before = f.metadata()?;
        ensure!(
            before.is_file()
                && before.uid() == unsafe { libc::geteuid() }
                && before.mode() & 0o077 == 0
                && before.nlink() == 1,
            "Unsafe hosted lock"
        );
        ensure!(
            unsafe { libc::flock(f.as_raw_fd(), op) } == 0,
            "Hosted worker already running"
        );
        let after = fs::symlink_metadata(path)?;
        ensure!(
            after.dev() == before.dev()
                && after.ino() == before.ino()
                && !after.file_type().is_symlink(),
            "Hosted lock changed while waiting"
        );
        Ok(Self(f))
    }
}
impl Drop for Lock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}

struct Scratch(PathBuf);
impl Scratch {
    fn new(parent: &Path) -> Result<Self> {
        private_dir(parent)?;
        let path = parent.join(format!("request-{}", random_id()?));
        fs::DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self(path))
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn config_escape(value: &str) -> Result<String> {
    ensure!(
        !value.bytes().any(|b| b.is_ascii_control()),
        "Invalid transport value"
    );
    Ok(value.replace('\\', "\\\\").replace('"', "\\\""))
}

impl Client {
    pub fn shares(&self) -> Result<Value> {
        let server = server_url(&self.settings().server)?;
        let credential = self
            .credential("read", &server)
            .or_else(|_| self.credential("upload", &server))?;
        self.shares_with(&server, &credential)
    }
    fn shares_with(&self, server: &str, credential: &Credential) -> Result<Value> {
        let mut all = Vec::new();
        let mut cursors = BTreeSet::new();
        let mut route = "/v1/shares".to_owned();
        loop {
            let reply = self.json_request(
                server,
                Some(credential),
                "GET",
                &route,
                None,
                Duration::from_secs(30),
                &|| true,
            )?;
            let shares = reply["shares"]
                .as_array()
                .context("Invalid shares response")?;
            for share in shares {
                ensure!(
                    share["id"].as_str().is_some_and(valid_id)
                        && share["captureId"].as_str().is_some_and(valid_id),
                    "Invalid share identity"
                );
                if matches!(
                    share["status"].as_str(),
                    Some("revoked" | "expired" | "deleted")
                ) {
                    ensure!(share["url"].is_null(), "Inactive share exposed a URL");
                } else {
                    validate_share_url(
                        server,
                        share["url"].as_str().context("Missing share URL")?,
                    )?;
                }
                all.push(share.clone());
            }
            match reply.get("nextCursor") {
                Some(Value::Null) => break,
                Some(Value::String(cursor)) if !cursor.is_empty() && cursor.len() <= 4096 => {
                    ensure!(cursors.insert(cursor.clone()), "Repeated shares cursor");
                    let encoded: String = cursor
                        .bytes()
                        .map(|byte| {
                            if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                                (byte as char).to_string()
                            } else {
                                format!("%{byte:02X}")
                            }
                        })
                        .collect();
                    route = format!("/v1/shares?before={encoded}");
                }
                _ => bail!("Invalid shares cursor"),
            }
        }
        Ok(json!({"shares":all,"nextCursor":null}))
    }
    pub fn revoke_share(&self, id: &str) -> Result<()> {
        ensure!(
            valid_id(id),
            "Share management ID must be 32 lowercase hexadecimal characters (not the URL secret)"
        );
        let server = server_url(&self.settings().server)?;
        let credential = self.credential("upload", &server)?;
        let reply = self.json_request(
            &server,
            Some(&credential),
            "DELETE",
            &format!("/v1/shares/{id}"),
            None,
            Duration::from_secs(30),
            &|| true,
        )?;
        ensure!(reply["ok"] == true, "Share revocation failed");
        let remaining = self.shares_with(&server, &credential)?;
        ensure!(
            !remaining["shares"]
                .as_array()
                .context("Invalid shares response")?
                .iter()
                .any(|share| share["id"].as_str() == Some(id)
                    && !matches!(share["status"].as_str(), Some("revoked" | "expired"))),
            "Share is still active"
        );
        Ok(())
    }
    #[cfg(any(test, feature = "test-support"))]
    fn share(&self, bundle: &Path) -> Result<String> {
        self.share_allowed(bundle, &|| true)
    }
    pub fn prepare_link(
        &self,
        bundle: &Path,
        foreground: &dyn Fn() -> bool,
    ) -> Result<PreparedLink> {
        let (url, job) = self.share_prepared(bundle, foreground)?;
        Ok(PreparedLink {
            client: self.clone(),
            job,
            text: format!(
                "I recorded some feedback in Apparent. Open this link for the recording, transcript and annotated screenshots: {url}. Anyone with this link can view and download this capture."
            ),
        })
    }
    #[cfg(any(test, feature = "test-support"))]
    fn share_allowed(&self, bundle: &Path, foreground: &dyn Fn() -> bool) -> Result<String> {
        self.share_prepared(bundle, foreground).map(|(url, _)| url)
    }
    fn share_prepared(
        &self,
        bundle: &Path,
        foreground: &dyn Fn() -> bool,
    ) -> Result<(String, Job)> {
        ensure!(foreground(), "cancelled");
        ensure!(
            self.provider.experiment(HOSTED_SHARING),
            "Hosted sharing is disabled"
        );
        let id = self.enqueue(bundle)?;
        let mut job = self
            .jobs()?
            .into_iter()
            .find(|j| j.id == id)
            .context("Missing queued capture")?;
        ensure!(
            self.share_active(&job),
            "Upload consent changed or capture was cancelled"
        );
        let credential = self.credential("upload", &job.server)?;
        let scratch = Scratch::new(&self.state)?;
        let files = snapshot(&job.bundle, &scratch.0)?;
        {
            let _lock = Lock::acquire(&self.state.join("queue.lock"), false)?;
            ensure!(self.share_active_locked(&job), "cancelled");
            // A concurrent uploader can have advanced the same job. Merge only the
            // immutable manifest, never overwrite its progress/status/retry state.
            let mut current = self
                .jobs()?
                .into_iter()
                .find(|j| j.id == id)
                .context("Missing queued capture")?;
            ensure!(
                current.files.is_empty() || current.files == files,
                "Bundle changed after reservation"
            );
            current.files = files.clone();
            // Retain the identity until retention expires, including after ready.
            // The URL secret itself is never stored in status or application logs.
            current.shared = true;
            self.save_job(&current)?;
            job = current;
        }
        let allowed =
            || foreground() && self.provider.experiment(HOSTED_SHARING) && self.share_active(&job);
        let base = format!("/v1/captures/{id}");
        let reserved = self.json_request(
            &job.server,
            Some(&credential),
            "POST",
            &base,
            Some(json!({"files":files})),
            Duration::from_secs(30),
            &allowed,
        )?;
        ensure!(
            reserved["id"].as_str() == Some(&id),
            "Invalid reservation ID"
        );
        let reply = self.json_request(
            &job.server,
            Some(&credential),
            "POST",
            &format!("{base}/share"),
            None,
            Duration::from_secs(30),
            &allowed,
        )?;
        let url = reply["url"].as_str().context("Missing share URL")?;
        validate_share_url(&job.server, url)?;
        ensure!(
            matches!(
                reply["status"].as_str(),
                Some("uploading" | "pending" | "ready" | "paused" | "failed")
            ) && (reply["expiresAt"].is_number() || reply["expiresAt"].is_string()),
            "Invalid share response"
        );
        ensure!(allowed(), "cancelled");
        Ok((url.into(), job))
    }
    /// Providers must read current state and delegate writes to the caller's
    /// serialized config store. No preference snapshot is retained here.
    pub fn new(provider: std::sync::Arc<dyn ConfigProvider>) -> Self {
        let config = provider.config_path().to_path_buf();
        let state = config.parent().unwrap_or(Path::new(".")).join("hosted");
        Self {
            provider,
            #[cfg(any(test, feature = "test-support"))]
            config,
            state,
        }
    }
    fn settings(&self) -> CloudConfig {
        self.provider.cloud()
    }
    pub fn enabled(&self) -> bool {
        let cloud = self.provider.cloud();
        let experiment = self.provider.experiment(HOSTED_SYNC);
        experiment && cloud.enabled && cloud.upload_consent && server_url(&cloud.server).is_ok()
    }
    fn credential(&self, scope: &str, server: &str) -> Result<Credential> {
        let credential = self.stored_credential(scope)?;
        ensure!(
            credential.server == server,
            "Sign in with the requested scope and server"
        );
        Ok(credential)
    }
    fn stored_credential(&self, scope: &str) -> Result<Credential> {
        private_dir(&self.state)?;
        let credential: Credential = serde_json::from_slice(&read_private(
            &self.state.join(format!("{scope}.credential")),
            32768,
        )?)?;
        ensure!(
            credential.scope == scope
                && credential.server == server_url(&credential.server)?
                && valid_id(&credential.generation),
            "Sign in with the requested scope and server"
        );
        ensure!(
            !credential.token.is_empty()
                && credential.token.len() <= 8192
                && credential.token.bytes().all(|b| b.is_ascii_graphic()),
            "Invalid credential"
        );
        credential.validate()?;
        Ok(credential)
    }
    fn active(&self, job: &Job) -> bool {
        let Ok(_lock) = Lock::acquire(&self.state.join("queue.lock"), false) else {
            return false;
        };
        self.active_locked(job)
    }
    // Caller holds queue.lock. Never acquire it recursively from a save path.
    fn active_locked(&self, job: &Job) -> bool {
        self.current_incarnation(job)
            .is_some_and(|current| current.status == job.status && current.status != "cancelled")
    }
    fn share_active(&self, job: &Job) -> bool {
        let Ok(_lock) = Lock::acquire(&self.state.join("queue.lock"), false) else {
            return false;
        };
        self.share_active_locked(job)
    }
    // Upload progress is not a new authorization incarnation. The uploader still
    // uses active_locked's exact-status check before publishing its own results.
    fn share_active_locked(&self, job: &Job) -> bool {
        self.current_incarnation(job).is_some_and(|current| {
            matches!(current.status.as_str(), "pending" | "failed" | "ready")
                && now().saturating_sub(current.created) <= RETENTION
                && (job.files.is_empty() || current.files == job.files)
                && (!job.shared || current.shared)
        })
    }
    // Caller holds queue.lock; identity and current consent are shared by both
    // policies, but mutable upload status must not stand in for consent.
    fn current_incarnation(&self, job: &Job) -> Option<Job> {
        if !self.enabled()
            || self.settings().server != job.server
            || !self
                .credential("upload", &job.server)
                .is_ok_and(|c| c.generation == job.generation)
        {
            return None;
        }
        let current: Job = serde_json::from_slice(
            &read_private(&self.state.join(format!("{}.job", job.id)), JSON_LIMIT).ok()?,
        )
        .ok()?;
        (current.id == job.id
            && current.bundle == job.bundle
            && current.server == job.server
            && current.generation == job.generation
            && current.created == job.created)
            .then_some(current)
    }
    fn jobs(&self) -> Result<Vec<Job>> {
        private_dir(&self.state)?;
        let mut jobs = Vec::new();
        for entry in fs::read_dir(&self.state)? {
            let path = entry?.path();
            if path.extension().is_some_and(|e| e == "job") {
                ensure!(jobs.len() < MAX_JOBS, "Hosted queue exceeds bound");
                let job: Job = serde_json::from_slice(&read_private(&path, JSON_LIMIT)?)?;
                ensure!(
                    valid_id(&job.id) && path.file_stem().and_then(|s| s.to_str()) == Some(&job.id),
                    "Invalid queued ID"
                );
                jobs.push(job);
            }
        }
        Ok(jobs)
    }
    fn save_job(&self, job: &Job) -> Result<()> {
        let path = self.state.join(format!("{}.job", job.id));
        let mut job = job.clone();
        // Share admission may race an uploader snapshot. This monotonic metadata
        // must not be lost when that uploader publishes an acknowledgement.
        if let Ok(bytes) = read_private(&path, JSON_LIMIT) {
            if let Ok(current) = serde_json::from_slice::<Job>(&bytes) {
                job.shared |= current.shared;
            }
        }
        atomic_json(&path, &job)
    }
    pub fn enqueue(&self, bundle: &Path) -> Result<String> {
        self.enqueue_with_intent(bundle, false)
    }
    pub fn enqueue_explicit(&self, bundle: &Path) -> Result<String> {
        self.enqueue_with_intent(bundle, true)
    }
    fn enqueue_with_intent(&self, bundle: &Path, explicit: bool) -> Result<String> {
        ensure!(
            self.enabled(),
            "Enable hostedSync and grant upload consent with apparent connect login --scope upload --consent-upload"
        );
        private_dir(&self.state)?;
        #[cfg(any(test, feature = "test-support"))]
        test_support::run_hook("enqueue_before_lock", bundle);
        let _lock = Lock::acquire(&self.state.join("queue.lock"), false)?;
        // Disable clears the gates and cancels jobs under this same lock.
        // An admission paused before the lock must not publish after it returns.
        ensure!(self.enabled(), "Hosted uploads are disabled");
        let bundle = bundle.canonicalize()?;
        // The bundle may itself be supplied through an alias, but no bundled data may be symlinked.
        ensure!(
            bundle.is_dir()
                && bundle.extension().is_some_and(|x| x == "apparent")
                && bundle.join("apparent.json").is_file(),
            "Expected completed .apparent bundle"
        );
        let settings = self.settings();
        let credential = self.credential("upload", &settings.server)?;
        let mut jobs = self.jobs()?;
        if !explicit {
            // Deduplicate only retained queue records, even after cancellation or
            // a fresh device grant. Pruned records leave no permanent dedup history.
            if let Some(job) = jobs
                .iter()
                .find(|j| j.bundle == bundle && j.server == settings.server)
            {
                return Ok(job.id.clone());
            }
        } else {
            // Explicit consent for this bundle creates a fresh remote identity. A
            // reserved manifest and server tombstone are immutable; retry keeps an ID.
            for job in &mut jobs {
                if job.bundle == bundle && job.server == settings.server && job.status != "ready" {
                    job.status = "cancelled".into();
                    self.save_job(job)?;
                }
            }
        }
        // Reclaim only terminal/expired metadata; never drop a live job to make room.
        jobs.retain(|job| {
            if now().saturating_sub(job.created) > RETENTION
                || (!job.shared && matches!(job.status.as_str(), "ready" | "cancelled" | "expired"))
            {
                let _ = fs::remove_file(self.state.join(format!("{}.job", job.id)));
                false
            } else {
                true
            }
        });
        ensure!(
            jobs.len() < MAX_JOBS,
            "Hosted queue full; local bundle remains available"
        );
        let job = Job {
            id: random_id()?,
            bundle,
            server: settings.server,
            generation: credential.generation,
            created: now(),
            attempts: 0,
            next_attempt: 0,
            status: "pending".into(),
            error: None,
            files: Vec::new(),
            shared: false,
        };
        self.save_job(&job)?;
        Ok(job.id)
    }

    pub fn pointer(&self, bundle: &Path) -> Option<String> {
        if !self.enabled() || !self.state.is_dir() {
            return None;
        }
        let job = self
            .jobs()
            .ok()?
            .into_iter()
            .find(|j| j.bundle == bundle && self.active(j) && j.status != "cancelled")?;
        Some(format!(
            "\nHosted capture: {} ({}; local original remains available).\nOn the agent machine, sign in separately with `apparent connect login --server {} --scope read`, then run:\napparent fetch {} --out ./{}.apparent --wait 120\nNever paste account tokens. Fetch waits for the complete bundle.\n",
            job.id, job.status, job.server, job.id, job.id
        ))
    }
    fn raw_request(
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
        let server = server_url(server)?;
        ensure!(
            route.starts_with("/v1/") && !route.contains(['\r', '\n', '"']),
            "Invalid API route"
        );
        ensure!(allowed(), "cancelled");
        let mut config = format!(
            "url = \"{}\"\nrequest = \"{method}\"\n",
            config_escape(&format!("{server}{route}"))?
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
                "5",
                "--max-time",
            ])
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
                bail!("Hosted request cancelled or timed out");
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
        ensure!(
            status.success(),
            "Hosted transport failed (network, timeout or response limit)"
        );
        ensure!(
            fs::metadata(output)?.len() <= limit,
            "Hosted response exceeds limit"
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
    fn json_request(
        &self,
        server: &str,
        token: Option<&Credential>,
        method: &str,
        route: &str,
        body: Option<Value>,
        timeout: Duration,
        allowed: &dyn Fn() -> bool,
    ) -> Result<Value> {
        let scratch = Scratch::new(&self.state)?;
        let body_path = scratch.0.join("body");
        if let Some(body) = &body {
            atomic_json(&body_path, body)?;
        }
        let response = scratch.0.join("response");
        self.request(
            server,
            token,
            method,
            route,
            body.as_ref().map(|_| body_path.as_path()),
            &response,
            JSON_LIMIT,
            timeout,
            allowed,
        )?;
        serde_json::from_slice(&fs::read(response)?).context("Invalid hosted JSON response")
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
        ensure!(allowed(), "cancelled");
        let server = server_url(server)?;
        ensure!(matches!(scope, "upload" | "read"), "Invalid scope");
        ensure!(
            scope != "upload" || consent,
            "Upload shares screen, voice, transcript and all bundle media with the hosted service. Repeat with --consent-upload to explicitly agree"
        );
        private_dir(&self.state)?;
        let login_id = random_id()?;
        let login_path = self.state.join(format!("{scope}.login"));
        {
            let _queue = Lock::acquire(&self.state.join("queue.lock"), false)?;
            ensure!(allowed(), "cancelled");
            atomic_json(&login_path, &login_id)?;
        }
        let caller_allowed = allowed;
        let still_current = || {
            caller_allowed()
                && read_private(&login_path, 128)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<String>(&b).ok())
                    .as_deref()
                    == Some(&login_id)
        };
        let allowed: &dyn Fn() -> bool = &still_current;
        let start = self.json_request(
            &server,
            None,
            "POST",
            "/v1/device/start",
            Some(json!({"scope":scope})),
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
            device_code.len() <= 8192
                && user_code.len() <= 128
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
            !uri.bytes()
                .any(|b| b.is_ascii_control() || b.is_ascii_whitespace() || b"\\\"'".contains(&b)),
            "Invalid verification URI"
        );
        println!("Open {uri}\nVerification code: {user_code}\nRequested scope: {scope}");
        if browser {
            self.provider.open_browser(Path::new(uri));
        }
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
                    };
                    credential.apply_reply(&reply, issued, false)?;
                    break credential;
                }
                _ => bail!("Device authorization rejected or invalid"),
            }
        };
        private_dir(&self.state)?;
        let _lock = Lock::acquire(&self.state.join("queue.lock"), false)?;
        // Read-only login must not silently enable upload or switch an active uploader's server.
        ensure!(allowed(), "cancelled");
        let mut settings = self.settings();
        #[cfg(any(test, feature = "test-support"))]
        test_support::run_hook("login_snapshot", &self.config);
        ensure!(
            scope == "upload" || !settings.enabled || settings.server == server,
            "Disable uploads before switching servers"
        );
        self.clear_renewal(scope)?;
        atomic_json(&self.state.join(format!("{scope}.credential")), &credential)?;
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
        self.provider.set_cloud(&settings)?;
        fs::remove_file(login_path)?;
        println!("Signed in for {scope}; tokens remain private on this machine.");
        Ok(())
    }
    pub fn disable(&self, logout: bool) -> Result<()> {
        private_dir(&self.state)?;
        #[cfg(any(test, feature = "test-support"))]
        test_support::run_hook("disable_before_lock", &self.config);
        let _lock = Lock::acquire(&self.state.join("queue.lock"), false)?;
        // Serialize the snapshot and gate write with login's settings/credential
        // commit. No network or staging holds this lock, only brief state work.
        let mut settings = self.settings();
        settings.enabled = false;
        settings.upload_consent = false;
        self.provider.set_cloud(&settings)?;
        atomic_json(&self.state.join("cancellation"), &random_id()?)?;
        let mut stopped = Vec::new();
        let mut revoke = Vec::new();
        for scope in ["upload", "read"] {
            match fs::remove_file(self.state.join(format!("{scope}.login"))) {
                Ok(()) => (),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                Err(e) => return Err(e.into()),
            }
        }
        for mut job in self.jobs()? {
            if job.status != "ready" {
                job.status = "cancelled".into();
                self.save_job(&job)?;
                if job.shared {
                    if let Ok(credential) = self.credential("upload", &job.server) {
                        if credential.generation == job.generation {
                            stopped.push((job, credential));
                        }
                    }
                }
            }
        }
        if logout {
            for scope in ["upload", "read"] {
                if let Ok(c) = self.stored_credential(scope) {
                    revoke.push(c);
                }
                self.clear_renewal(scope)?;
                let path = self.state.join(format!("{scope}.credential"));
                match fs::remove_file(path) {
                    Ok(()) => (),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => (),
                    Err(e) => return Err(e.into()),
                }
            }
        }
        File::open(&self.state)?.sync_all()?;
        drop(_lock);
        // Only status metadata may be sent after cancellation, never more media.
        // Bound the whole best-effort stop notification and hold neither lock.
        let deadline = Instant::now() + Duration::from_secs(2);
        for (job, credential) in stopped {
            self.notify_progress_with(&job, "paused", &credential, deadline);
        }
        for credential in revoke {
            let _ = self.revoke_credential(&credential, deadline);
        }
        Ok(())
    }
    pub fn retry(&self) -> Result<()> {
        ensure!(self.enabled(), "Hosted uploads are disabled");
        private_dir(&self.state)?;
        let _lock = Lock::acquire(&self.state.join("queue.lock"), false)?;
        for mut job in self.jobs()? {
            if self.active_locked(&job)
                && matches!(job.status.as_str(), "pending" | "failed")
                && now().saturating_sub(job.created) <= RETENTION
            {
                job.attempts = 0;
                job.next_attempt = 0;
                job.status = "pending".into();
                job.error = None;
                self.save_job(&job)?;
            }
        }
        Ok(())
    }
    pub fn process_once(&self) -> Result<()> {
        private_dir(&self.state)?;
        let _worker = match Lock::acquire(&self.state.join("worker.lock"), true) {
            Ok(lock) => lock,
            Err(_) => return Ok(()),
        };
        for mut job in self.jobs()? {
            if job.status != "pending" || job.next_attempt > now() || !self.active(&job) {
                continue;
            }
            if now().saturating_sub(job.created) > RETENTION {
                let _lock = Lock::acquire(&self.state.join("queue.lock"), false)?;
                if !self.active_locked(&job) {
                    continue;
                }
                job.status = "expired".into();
                self.save_job(&job)?;
                continue;
            }
            let result = self.upload(&mut job);

            let _lock = Lock::acquire(&self.state.join("queue.lock"), false)?;
            // Logout/disable wins over a late completion. Never restore a cancelled job.
            if !self.active_locked(&job) {
                continue;
            }
            job.attempts += 1;
            match result {
                Ok(()) => {
                    job.status = "ready".into();
                    job.error = None;
                }
                Err(error) => {
                    let text = error.to_string();
                    job.error = Some(
                        if text.contains("file_too_large") {
                            "file_too_large"
                        } else if text.contains("quota_exceeded") {
                            "quota_exceeded"
                        } else if text.contains("unsupported_asset") {
                            "unsupported_asset"
                        } else if text.contains("auth_required")
                            || text.contains("unauthorized")
                            || text.contains("forbidden")
                        {
                            "auth_required"
                        } else {
                            "upload_failed"
                        }
                        .into(),
                    );
                    if job.attempts >= MAX_ATTEMPTS
                        || matches!(
                            job.error.as_deref(),
                            Some("auth_required" | "file_too_large")
                        )
                    {
                        job.status = "failed".into();
                    }
                    job.next_attempt = now() + (5u64 * (1u64 << job.attempts.min(10))).min(3600);
                }
            }
            self.save_job(&job)?;
            drop(_lock);
            if job.error.is_some() {
                self.notify_progress(
                    &job,
                    if job.status == "failed" {
                        "failed"
                    } else {
                        "paused"
                    },
                );
            }
        }
        Ok(())
    }
    fn notify_progress(&self, job: &Job, state: &str) {
        // A sharing admission can race an uploader's older job snapshot.
        if !job.shared
            && !self
                .jobs()
                .unwrap_or_default()
                .iter()
                .any(|current| current.id == job.id && current.shared)
        {
            return;
        }
        if let Ok(credential) = self.credential("upload", &job.server) {
            if credential.generation == job.generation {
                self.notify_progress_with(
                    job,
                    state,
                    &credential,
                    Instant::now() + Duration::from_secs(2),
                );
            }
        }
    }
    fn notify_progress_with(
        &self,
        job: &Job,
        state: &str,
        credential: &Credential,
        deadline: Instant,
    ) {
        let _ = self.json_request(
            &job.server,
            Some(credential),
            "POST",
            &format!("/v1/captures/{}/share/progress", job.id),
            Some(json!({"state":state})),
            deadline.saturating_duration_since(Instant::now()),
            &|| Instant::now() < deadline,
        );
    }
    fn upload(&self, job: &mut Job) -> Result<()> {
        let credential = self.credential("upload", &job.server)?;
        let scratch = Scratch::new(&self.state)?;
        // Stage a stable snapshot through O_NOFOLLOW handles. Retry the same manifest;
        // changing local media cannot silently mutate an already reserved capture ID.
        let files = snapshot(&job.bundle, &scratch.0)?;
        ensure!(
            serde_json::to_vec(&json!({"files": files}))?.len() <= 128 * 1024,
            "invalid_manifest: exceeds v1 request limit"
        );
        if job.files.is_empty() {
            job.files = files.clone();
            let _lock = Lock::acquire(&self.state.join("queue.lock"), false)?;
            ensure!(self.active_locked(job), "cancelled");
            self.save_job(job)?;
        }
        ensure!(
            job.files == files,
            "Bundle changed after upload was queued; explicitly upload it under a new ID"
        );
        let allowed = || self.active(job);
        let base = format!("/v1/captures/{}", job.id);
        let reserved = self.json_request(
            &job.server,
            Some(&credential),
            "POST",
            &base,
            Some(json!({"files": files})),
            Duration::from_secs(30),
            &allowed,
        )?;
        ensure!(
            reserved["id"].as_str() == Some(&job.id),
            "Invalid reservation ID"
        );
        if reserved["status"] != "ready" {
            self.notify_progress(job, "uploading");
            ensure!(
                reserved["status"] == "pending",
                "Invalid reservation status"
            );
            for file in &job.files {
                let response = scratch.0.join(format!("ack-{}", random_id()?));
                self.request(
                    &job.server,
                    Some(&credential),
                    "PUT",
                    &format!("{base}/files/{}", file.path),
                    Some(&scratch.0.join(&file.path)),
                    &response,
                    JSON_LIMIT,
                    Duration::from_secs(60),
                    &allowed,
                )?;
            }
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
                completed["id"].as_str() == Some(&job.id) && completed["status"] == "ready",
                "Hosted capture did not complete"
            );
        }
        // Read back the exact manifest rather than trusting the completion write.
        let verified = self.json_request(
            &job.server,
            Some(&credential),
            "GET",
            &base,
            None,
            Duration::from_secs(30),
            &allowed,
        )?;
        let verified_files: Vec<Asset> = serde_json::from_value(verified["files"].clone())?;
        ensure!(
            verified["id"].as_str() == Some(&job.id)
                && verified["status"] == "ready"
                && same_manifest(&verified_files, &job.files),
            "Hosted completion verification failed"
        );
        Ok(())
    }
    pub fn fetch(&self, id: &str, out: &Path, wait: u64) -> Result<PathBuf> {
        ensure!(
            valid_id(id),
            "Capture ID must be 32 lowercase hexadecimal characters"
        );
        ensure!(wait <= 900, "--wait must be between 0 and 900 seconds");
        let server = server_url(&self.settings().server)?;
        let credential = self.credential("read", &server)?;
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
                &|| true,
            )?;
            ensure!(
                reply["id"].as_str() == Some(id),
                "Wrong capture ID in manifest"
            );
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
                &|| true,
            )?;
            ensure!(
                fs::metadata(&destination)?.len() == file.size
                    && digest_file(&destination)? == file.sha256,
                "Hosted file integrity mismatch"
            );
        }
        validate_bundle_references(&stage.0, &files)?;
        // Atomic no-replace publish. Linux/macOS use their kernel's exclusive rename.
        publish_noreplace(&stage.0, &out)?;
        File::open(&parent)?.sync_all()?;
        Ok(out)
    }
    pub fn status(&self, account: bool) -> Result<Value> {
        let settings = self.settings();
        let jobs = self.jobs()?;
        let mut result = json!({"server": settings.server, "enabled": self.enabled(), "uploadConsent": settings.upload_consent,
            "readSignedIn": self.credential("read", &settings.server).is_ok_and(|c| c.usable()), "uploadSignedIn": self.credential("upload", &settings.server).is_ok_and(|c| c.usable()),
            "queuePath": self.state, "queue": jobs.iter().map(|j| json!({"id":j.id,"bundle":j.bundle,"status":j.status,"attempts":j.attempts,"nextAttempt":j.next_attempt,"error":j.error})).collect::<Vec<_>>()});
        if account {
            let credential = self
                .credential("upload", &settings.server)
                .or_else(|_| self.credential("read", &settings.server))?;
            result["account"] = self.json_request(
                &settings.server,
                Some(&credential),
                "GET",
                "/v1/account",
                None,
                Duration::from_secs(30),
                &|| true,
            )?;
        }
        for scope in ["read", "upload"] {
            result[format!("{scope}Auth")] = self.auth_status(scope, &settings.server);
        }
        Ok(result)
    }
}

impl Scratch {
    fn new_output(parent: &Path) -> Result<Self> {
        trusted_directory_tree(parent, true)?;
        let path = parent.join(format!(".apparent-fetch-{}", random_id()?));
        fs::DirBuilder::new().mode(0o700).create(&path)?;
        Ok(Self(path))
    }
}

fn digest_file(path: &Path) -> Result<String> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?;
    ensure!(file.metadata()?.is_file(), "Expected regular asset");
    #[cfg(target_os = "macos")]
    let mut command = {
        let mut c = Command::new("/usr/bin/shasum");
        c.args(["-a", "256"]);
        c
    };
    #[cfg(target_os = "linux")]
    let mut command = Command::new("sha256sum");
    let output = command
        .stdin(Stdio::from(file))
        .stderr(Stdio::null())
        .output()
        .context("SHA-256 tool is required")?;
    ensure!(output.status.success(), "SHA-256 failed");
    let output = String::from_utf8(output.stdout)?;
    let digest = output.split_whitespace().next().unwrap_or("");
    ensure!(hex_digest(digest), "Invalid SHA-256 output");
    Ok(digest.to_owned())
}

fn snapshot(bundle: &Path, stage: &Path) -> Result<Vec<Asset>> {
    no_symlinks(bundle)?;
    trusted_directory_tree(bundle, false)?;
    private_dir(stage)?;
    let mut paths = Vec::new();
    for entry in fs::read_dir(bundle)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| anyhow::anyhow!("Invalid asset name"))?;
        if matches!(name.as_str(), "frames" | "frames-full") {
            trusted_directory_tree(&entry.path(), false)?;
            ensure!(
                entry.file_type()?.is_dir() && !entry.file_type()?.is_symlink(),
                "Symlink asset directory refused"
            );
            for child in fs::read_dir(entry.path())? {
                let child = child?;
                let name = format!(
                    "{name}/{}",
                    child.file_name().to_str().context("Invalid asset name")?
                );
                ensure!(
                    asset_path(&name),
                    "unsupported_asset: bundle contains an unknown asset; no files were omitted"
                );
                paths.push(name);
            }
        } else {
            ensure!(
                asset_path(&name),
                "unsupported_asset: bundle contains an unknown asset; no files were omitted"
            );
            paths.push(name);
        }
    }
    paths.sort();

    ensure!(paths.len() <= MAX_FILES, "Too many bundle assets");
    let mut files = Vec::new();
    let mut total = 0u64;
    for path in paths {
        no_symlinks(&bundle.join(&path))?;
        #[cfg(any(test, feature = "test-support"))]
        test_support::run_hook("snapshot_open", &bundle.join(&path));
        let mut source = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
            .open(bundle.join(&path))?;
        let metadata = source.metadata()?;
        ensure!(metadata.is_file(), "Non-regular asset refused");
        ensure!(
            metadata.len() <= MAX_FILE_BYTES,
            "file_too_large: an asset exceeds the v1 90,000,000-byte per-file limit; no files were omitted"
        );
        total = total
            .checked_add(metadata.len())
            .context("Asset size overflow")?;
        ensure!(
            total <= MAX_BYTES,
            "quota_exceeded: bundle exceeds account limit"
        );
        let target = stage.join(&path);
        fs::create_dir_all(target.parent().unwrap())?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&target)?;
        let copied = std::io::copy(&mut (&mut source).take(metadata.len() + 1), &mut output)?;
        ensure!(copied == metadata.len(), "Asset changed during snapshot");
        output.sync_all()?;
        files.push(Asset {
            path,
            size: copied,
            sha256: digest_file(&target)?,
        });
    }
    validate_assets(&files)?;
    validate_bundle_references(stage, &files)?;
    Ok(files)
}

fn validate_bundle_references(bundle: &Path, files: &[Asset]) -> Result<()> {
    let metadata = files
        .iter()
        .find(|f| f.path == "apparent.json")
        .context("Missing bundle metadata")?;
    ensure!(metadata.size <= JSON_LIMIT, "Bundle metadata too large");
    let value: Value = serde_json::from_slice(&fs::read(bundle.join("apparent.json"))?)?;
    ensure!(value.is_object(), "Invalid bundle metadata");
    let names: BTreeSet<_> = files.iter().map(|f| f.path.as_str()).collect();
    let check = |value: &Value| -> Result<()> {
        if let Some(path) = value.as_str() {
            ensure!(
                asset_path(path) && names.contains(path),
                "Unsafe or missing bundle asset reference"
            );
        } else {
            ensure!(value.is_null(), "Invalid asset reference");
        }
        Ok(())
    };
    for field in ["contactSheet", "contact_sheet", "video", "audio"] {
        if let Some(value) = value.get(field) {
            check(value)?;
        }
    }
    if let Some(frames) = value.get("keyframes").and_then(Value::as_array) {
        for frame in frames {
            check(frame.get("file").context("Missing frame asset")?)?;
        }
    }
    if let Some(crop) = value.get("autocrop").filter(|v| !v.is_null()) {
        for field in ["fullContactSheet", "full_contact_sheet"] {
            if let Some(v) = crop.get(field) {
                check(v)?;
            }
        }
        for field in ["fullFrames", "full_frames"] {
            if let Some(v) = crop.get(field) {
                ensure!(
                    v.as_str() == Some("frames-full"),
                    "Unsafe full-frame directory"
                );
            }
        }
    }
    Ok(())
}

fn publish_noreplace(from: &Path, to: &Path) -> Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let from = CString::new(from.as_os_str().as_bytes())?;
    let to = CString::new(to.as_os_str().as_bytes())?;
    #[cfg(target_os = "linux")]
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            from.as_ptr(),
            libc::AT_FDCWD,
            to.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    #[cfg(target_os = "macos")]
    let result = unsafe { libc::renamex_np(from.as_ptr(), to.as_ptr(), libc::RENAME_EXCL) };
    ensure!(
        result == 0,
        "Could not publish capture without replacing existing output: {}",
        std::io::Error::last_os_error()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hosted_asset_boundary() {
        for path in [
            "../summary.md",
            "/summary.md",
            "frames/../../secret",
            "frames/.credential",
            "frames/a\\b.png",
            "credentials.json",
        ] {
            assert!(!asset_path(path), "accepted {path}");
        }
        for path in [
            "summary.md",
            "apparent.json",
            "contact-sheet.png",
            "frames/frame-01.png",
            "frames-full/frame-01.png",
            "recording.mov",
            "audio.wav",
        ] {
            assert!(asset_path(path), "rejected {path}");
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct AccountView {
    pub server: Option<String>,
    pub connected: bool,
}

impl Client {
    pub fn sharing_connected(&self) -> bool {
        let client = self;
        client.enabled()
            && client
                .credential("upload", &client.settings().server)
                .is_ok_and(|c| c.usable())
    }

    pub fn sharing_server(&self) -> Option<String> {
        server_url(&self.settings().server).ok()
    }

    pub fn shared_links_url(&self) -> Option<String> {
        let client = self;
        let server = self.sharing_server()?;
        let credential = client
            .credential("upload", &server)
            .or_else(|_| client.credential("read", &server))
            .ok()?;
        let site = server_url(credential.site.as_deref()?).ok()?;
        Some(format!("{site}/hosted/shares"))
    }

    /// A bounded, read-only probe. An HTTP rejection still proves server connectivity;
    /// a transport error does not. Never admits an upload or writes a preference.
    pub fn sharing_online(&self) -> bool {
        let client = self;
        let Some(server) = self.sharing_server() else {
            return false;
        };
        match client.json_request(
            &server,
            None,
            "GET",
            "/v1/account",
            None,
            Duration::from_secs(3),
            &|| true,
        ) {
            Ok(_) => true,
            Err(error) => {
                error.to_string().starts_with("Hosted API:")
                    || error.to_string().starts_with("auth_required:")
            }
        }
    }

    pub fn sharing_progress(&self, bundle: &Path) -> String {
        let client = self;
        let canonical = fs::canonicalize(bundle).ok();
        let job = client
            .jobs()
            .unwrap_or_default()
            .into_iter()
            .find(|job| Some(&job.bundle) == canonical.as_ref());
        match job {
            Some(job) if job.status == "ready" => "Link copied · ready".into(),
            Some(job) if job.status == "failed" => "Link copied · upload failed".into(),
            Some(job) if job.status == "cancelled" || !client.enabled() => {
                "Link copied · upload paused".into()
            }
            Some(job) if job.next_attempt > now() => "Link copied · waiting for connection".into(),
            _ => "Link copied · uploading · Estimating…".into(),
        }
    }
    pub fn account_view(&self) -> AccountView {
        let server = server_url(&self.settings().server).ok();
        let connected = self.enabled()
            && server
                .as_ref()
                .is_some_and(|server| self.credential("upload", server).is_ok_and(|c| c.usable()));
        AccountView { server, connected }
    }
}

#[cfg(any(test, feature = "test-support"))]
pub mod test_support;
