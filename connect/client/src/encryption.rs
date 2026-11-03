//! Endpoint-only v1 packing. This is not a new cryptographic format: age v1
//! (https://age-encryption.org/v1) owns key wrapping, randomness and STREAM AEAD.
//! The plaintext archive is u32-BE descriptor length, JSON descriptor, then the
//! exact ordered file bytes. No padding, alternate records or trailing bytes.
//! Parts split ONE finalized binary age stream; they are not independent files.
//! Signature input is DOMAIN || serde_json(Signed), in declaration field order.
//! Public registry authorization remains the caller's responsibility.
use super::{
    Asset, MAX_BYTES, MAX_FILES, Result, digest_file, private_dir, read_private, validate_assets,
    validate_bundle_references,
};
use age::secrecy::ExposeSecret;
use anyhow::{Context as _, ensure};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};
use zeroize::Zeroizing;

const PART_BYTES: u64 = 32 * 1024 * 1024;
const JSON_BYTES: u64 = 128 * 1024;
const HEADER_BYTES: usize = 32 * 1024;
const DOMAIN: &[u8] = b"apparent-encrypted-v1\0signed-envelope\0";

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Context {
    pub origin: String,
    pub account_id: String,
    pub capture_id: String,
    pub sender_grant_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(super) struct Peer {
    pub grant_id: String,
    pub scope: String,
    pub key: Value,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicKey {
    version: u32,
    recipient: String,
    signing: String,
}

pub(super) struct Identity {
    age: age::x25519::Identity,
    signing: SigningKey,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredIdentity {
    version: u32,
    age: String,
    signing: String,
}
impl Drop for StoredIdentity {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.age.zeroize();
        self.signing.zeroize();
    }
}
impl Identity {
    pub fn public_key(&self) -> Value {
        serde_json::json!({"version":1,"recipient":self.age.to_public().to_string(),
            "signing":hex(&self.signing.verifying_key().to_bytes())})
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex<const N: usize>(s: &str) -> Result<[u8; N]> {
    ensure!(
        s.len() == N * 2
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "invalid_encryption_key"
    );
    let mut out = [0; N];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)?;
    }
    Ok(out)
}
fn public_key(value: &Value) -> Result<(age::x25519::Recipient, VerifyingKey)> {
    // Bound strings before deserializing/copying untrusted registry entries.
    ensure!(
        value.as_object().is_some_and(|v| v.len() == 3)
            && value["recipient"].as_str().is_some_and(|s| s.len() == 62)
            && value["signing"].as_str().is_some_and(|s| s.len() == 64),
        "invalid_encryption_key"
    );
    let key: PublicKey = serde_json::from_value(value.clone())
        .map_err(|_| anyhow::anyhow!("invalid_encryption_key"))?;
    ensure!(key.version == 1, "unsupported_encryption_key");
    let recipient: age::x25519::Recipient = key
        .recipient
        .parse()
        .map_err(|_| anyhow::anyhow!("invalid_encryption_key"))?;
    ensure!(
        recipient.to_string() == key.recipient,
        "invalid_encryption_key"
    );
    let signing = VerifyingKey::from_bytes(&unhex(&key.signing)?)
        .map_err(|_| anyhow::anyhow!("invalid_encryption_key"))?;
    ensure!(!signing.is_weak(), "invalid_encryption_key");
    Ok((recipient, signing))
}
pub(super) fn validate_public_key(key: &Value) -> Result<()> {
    public_key(key).map(|_| ())
}

pub(super) fn load_identity(path: &Path) -> Result<Identity> {
    (|| -> Result<Identity> {
        private_dir(path.parent().context("identity parent")?)?;
        let bytes = Zeroizing::new(read_private(path, 4096)?);
        let stored: StoredIdentity = serde_json::from_slice(&bytes)?;
        ensure!(stored.version == 1, "identity version");
        let age: age::x25519::Identity = stored.age.parse().map_err(|_| anyhow::anyhow!("identity format"))?;
        ensure!(age.to_string().expose_secret() == &stored.age, "identity format");
        let seed = Zeroizing::new(unhex::<32>(&stored.signing)?);
        Ok(Identity { age, signing: SigningKey::from_bytes(&seed) })
    })().map_err(|_| anyhow::anyhow!("encryption_identity_unavailable: restore the original private identity; it was not replaced"))
}
pub(super) fn load_or_create_identity(path: &Path) -> Result<Identity> {
    match fs::symlink_metadata(path) {
        Ok(_) => return load_identity(path),
        Err(e) if e.kind() == io::ErrorKind::NotFound => (),
        Err(_) => anyhow::bail!("encryption_identity_unavailable"),
    }
    let parent = path.parent().context("encryption_identity_unavailable")?;
    private_dir(parent)?;
    let mut seed = Zeroizing::new([0u8; 32]);
    File::open("/dev/urandom")?.read_exact(seed.as_mut())?;
    let identity = Identity {
        age: age::x25519::Identity::generate(),
        signing: SigningKey::from_bytes(&seed),
    };
    let stored = StoredIdentity {
        version: 1,
        age: identity.age.to_string().expose_secret().to_owned(),
        signing: hex(seed.as_ref()),
    };
    let bytes = Zeroizing::new(serde_json::to_vec(&stored)?);
    let tmp = parent.join(format!(".identity-{}", super::random_id()?));
    let result = (|| -> Result<()> {
        let mut file = new_private_file(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        // Atomic create-if-absent, never rename-overwrite a concurrent identity.
        match fs::hard_link(&tmp, path) {
            Ok(()) => File::open(parent)?.sync_all()?,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => (),
            Err(e) => return Err(e.into()),
        }
        Ok(())
    })();
    let _ = fs::remove_file(tmp);
    result.map_err(|_| anyhow::anyhow!("encryption_identity_unavailable"))?;
    load_identity(path)
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Signed {
    version: u32,
    context: Context,
    #[serde(deserialize_with = "strict_assets")]
    chunks: Vec<Asset>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    signed: Signed,
    signature: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Inner {
    version: u32,
    context: Context,
    #[serde(deserialize_with = "strict_assets")]
    files: Vec<Asset>,
}
fn strict_assets<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Vec<Asset>, D::Error> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WireAsset {
        path: String,
        size: u64,
        sha256: String,
    }
    let files = Vec::<WireAsset>::deserialize(deserializer)?;
    if files.len() > MAX_FILES {
        return Err(serde::de::Error::custom("too many assets"));
    }
    Ok(files
        .into_iter()
        .map(|f| Asset {
            path: f.path,
            size: f.size,
            sha256: f.sha256,
        })
        .collect())
}
fn context_valid(context: &Context) -> Result<()> {
    ensure!(
        super::server_url(&context.origin).is_ok_and(|s| s == context.origin)
            && super::valid_id(&context.capture_id)
            && [&context.account_id, &context.sender_grant_id]
                .iter()
                .all(|s| !s.is_empty() && s.len() <= 256 && !s.chars().any(char::is_control)),
        "invalid_encryption_context"
    );
    ensure!(
        matches!(
            context.group.as_deref(),
            None | Some("evidence") | Some("media")
        ),
        "invalid_encryption_context"
    );
    Ok(())
}

pub(super) fn is_media_path(path: &str) -> bool {
    matches!(path, "audio.wav" | "recording.mp4" | "recording.mov")
}

fn validate_group_manifest(files: &[Asset], group: Option<&str>) -> Result<()> {
    ensure!(files.len() <= MAX_FILES, "invalid_manifest: file count");
    let mut names = BTreeSet::new();
    let mut total = 0u64;
    for file in files {
        ensure!(
            super::asset_path(&file.path)
                && super::hex_digest(&file.sha256)
                && names.insert(file.path.as_str()),
            "invalid_manifest: unsafe or duplicate file"
        );
        super::check_file_size(&file.path, file.size)?;
        total = total.checked_add(file.size).context("quota_exceeded")?;
        ensure!(total <= MAX_BYTES, "quota_exceeded");
    }
    match group {
        None => validate_assets(files)?,
        Some("evidence") => {
            ensure!(
                files.iter().all(|f| !is_media_path(&f.path)),
                "invalid_encryption_group"
            );
            validate_assets(files)?;
        }
        Some("media") => {
            ensure!(
                files.iter().all(|f| is_media_path(&f.path)),
                "invalid_encryption_group"
            );
        }
        _ => anyhow::bail!("invalid_encryption_group"),
    }
    Ok(())
}
fn validate_group_references(stage: &Path, files: &[Asset], group: Option<&str>) -> Result<()> {
    let names: BTreeSet<_> = files.iter().map(|f| f.path.as_str()).collect();
    match group {
        None => validate_bundle_references(stage, files)?,
        Some("evidence") => {
            let metadata: Value = serde_json::from_slice(&fs::read(stage.join("apparent.json"))?)?;
            let mut refs = files.to_vec();
            for (field, allowed) in [
                ("audio", vec!["audio.wav"]),
                ("video", vec!["recording.mp4", "recording.mov"]),
            ] {
                if let Some(value) = metadata.get(field) {
                    if value.is_null() {
                        continue;
                    }
                    let path = value.as_str().context("Invalid asset reference")?;
                    ensure!(
                        allowed.contains(&path),
                        "Unsafe or missing bundle asset reference"
                    );
                    if !names.contains(path) {
                        refs.push(Asset {
                            path: path.into(),
                            size: 0,
                            sha256: "0".repeat(64),
                        });
                    }
                }
            }
            for field in ["contactSheet", "contact_sheet"] {
                if let Some(value) = metadata.get(field).filter(|v| !v.is_null()) {
                    ensure!(
                        value.as_str().is_some_and(|p| names.contains(p)),
                        "Unsafe or missing bundle asset reference"
                    );
                }
            }
            if let Some(frames) = metadata.get("keyframes").and_then(Value::as_array) {
                for frame in frames {
                    let path = frame
                        .get("file")
                        .and_then(Value::as_str)
                        .context("Missing frame asset")?;
                    ensure!(
                        names.contains(path),
                        "Unsafe or missing bundle asset reference"
                    );
                }
            }
            if let Some(crop) = metadata.get("autocrop").filter(|v| !v.is_null()) {
                for field in ["fullContactSheet", "full_contact_sheet"] {
                    if let Some(value) = crop.get(field).filter(|v| !v.is_null()) {
                        ensure!(
                            value.as_str().is_some_and(|p| names.contains(p)),
                            "Unsafe or missing bundle asset reference"
                        );
                    }
                }
            }
            validate_bundle_references(stage, &refs)?;
        }
        Some("media") => (),
        _ => anyhow::bail!("invalid_encryption_group"),
    }
    Ok(())
}
fn signature_bytes(signed: &Signed) -> Result<Vec<u8>> {
    let mut bytes = DOMAIN.to_vec();
    bytes.extend(serde_json::to_vec(signed)?);
    Ok(bytes)
}
fn read_envelope(spool: &Path) -> Result<Envelope> {
    private_dir(spool)?;
    let bytes = read_private(&spool.join("envelope.json"), JSON_BYTES)?;
    let envelope: Envelope = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow::anyhow!("invalid_encryption_envelope"))?;
    ensure!(
        envelope.signed.version == 1,
        "unsupported_encryption_format"
    );
    context_valid(&envelope.signed.context)?;
    validate_chunks(&envelope.signed.chunks)?;
    unhex::<64>(&envelope.signature)?;
    Ok(envelope)
}
/// Unauthenticated context hint ONLY. Caller must resolve sender from its trusted
/// registry and pass the independently expected capture/origin/account to unpack.
pub(super) fn envelope_context(spool: &Path) -> Result<Context> {
    Ok(read_envelope(spool)?.signed.context)
}
fn part_path(index: usize) -> String {
    format!("sealed/part-{index:06}.age")
}
fn validate_chunks(chunks: &[Asset]) -> Result<()> {
    ensure!(
        !chunks.is_empty() && chunks.len() <= 16,
        "invalid_encryption_parts"
    );
    let mut total = 0u64;
    for (index, chunk) in chunks.iter().enumerate() {
        ensure!(
            chunk.path == part_path(index)
                && chunk.size > 0
                && chunk.size <= PART_BYTES
                && (index + 1 == chunks.len() || chunk.size == PART_BYTES)
                && super::hex_digest(&chunk.sha256),
            "invalid_encryption_parts"
        );
        total = total
            .checked_add(chunk.size)
            .context("invalid_encryption_parts")?;
    }
    ensure!(total <= MAX_BYTES, "quota_exceeded");
    Ok(())
}
pub(super) fn validate_transport(files: &[Asset]) -> Result<()> {
    ensure!(
        files.len() >= 2 && files.len() <= 17,
        "invalid_encryption_manifest"
    );
    let envelopes: Vec<_> = files.iter().filter(|f| f.path == "envelope.json").collect();
    ensure!(
        envelopes.len() == 1
            && envelopes[0].size > 0
            && envelopes[0].size <= JSON_BYTES
            && super::hex_digest(&envelopes[0].sha256),
        "invalid_encryption_manifest"
    );
    let mut chunks: Vec<_> = files
        .iter()
        .filter(|f| f.path != "envelope.json")
        .cloned()
        .collect();
    chunks.sort_by(|a, b| a.path.cmp(&b.path));
    validate_chunks(&chunks)?;
    ensure!(
        files.iter().map(|f| f.size).sum::<u64>() <= MAX_BYTES,
        "quota_exceeded"
    );
    Ok(())
}
fn new_private_file(path: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)?)
}
fn empty_private(path: &Path) -> Result<()> {
    private_dir(path)?;
    ensure!(
        fs::read_dir(path)?.next().is_none(),
        "encryption_scratch_not_empty"
    );
    Ok(())
}
fn stage_assets(stage: &Path) -> Result<Vec<Asset>> {
    private_dir(stage)?;
    let mut paths = Vec::new();
    for entry in fs::read_dir(stage)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            ensure!(
                matches!(entry.file_name().to_str(), Some("frames" | "frames-full")),
                "unsupported_asset"
            );
            // snapshot() uses ordinary nested directory modes; the private
            // stage ancestor prevents other users reaching these descendants.
            super::trusted_directory_tree(&entry.path(), false)?;
            for child in fs::read_dir(entry.path())? {
                let child = child?;
                ensure!(child.file_type()?.is_file(), "unsupported_asset");
                paths.push(format!(
                    "{}/{}",
                    entry.file_name().to_str().context("unsupported_asset")?,
                    child.file_name().to_str().context("unsupported_asset")?
                ));
                ensure!(paths.len() <= MAX_FILES, "invalid_manifest");
            }
        } else {
            ensure!(entry.file_type()?.is_file(), "unsupported_asset");
            paths.push(
                entry
                    .file_name()
                    .into_string()
                    .map_err(|_| anyhow::anyhow!("unsupported_asset"))?,
            );
        }
        ensure!(paths.len() <= MAX_FILES, "invalid_manifest");
    }
    paths.sort();
    let mut files = Vec::new();
    let mut total = 0u64;
    for path in paths {
        ensure!(super::asset_path(&path), "unsupported_asset");
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(stage.join(&path))?;
        let size = file.metadata()?.len();
        super::check_file_size(&path, size)?;
        total = total.checked_add(size).context("quota_exceeded")?;
        ensure!(total <= MAX_BYTES, "quota_exceeded");
        files.push(Asset {
            sha256: digest_file(&stage.join(&path))?,
            path,
            size,
        });
    }
    Ok(files)
}

struct PartsWriter {
    spool: PathBuf,
    file: Option<File>,
    size: u64,
    total: u64,
    count: usize,
}
impl Write for PartsWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        if self.total >= MAX_BYTES - JSON_BYTES {
            return Err(io::Error::other("encrypted quota exceeded"));
        }
        if self.file.is_none() || self.size == PART_BYTES {
            if let Some(f) = self.file.take() {
                f.sync_all()?;
            }
            self.file = Some(
                new_private_file(&self.spool.join(part_path(self.count)))
                    .map_err(|_| io::Error::other("encrypted part creation failed"))?,
            );
            self.count += 1;
            self.size = 0;
        }
        let n = bytes
            .len()
            .min((PART_BYTES - self.size) as usize)
            .min((MAX_BYTES - JSON_BYTES - self.total) as usize);
        let n = self.file.as_mut().unwrap().write(&bytes[..n])?;
        self.size += n as u64;
        self.total += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        if let Some(f) = &mut self.file {
            f.flush()?;
        }
        Ok(())
    }
}

pub(super) fn pack(
    stage: &Path,
    spool: &Path,
    context: &Context,
    identity: &Identity,
    recipients: &[Peer],
) -> Result<Vec<Asset>> {
    context_valid(context)?;
    ensure!(
        !recipients.is_empty() && recipients.len() <= 64,
        "invalid_encryption_recipients"
    );
    let mut grants = BTreeSet::new();
    let mut keys = BTreeSet::new();
    let mut parsed = Vec::new();
    for peer in recipients {
        ensure!(
            !peer.grant_id.is_empty()
                && peer.grant_id.len() <= 256
                && matches!(peer.scope.as_str(), "read" | "upload")
                && grants.insert(&peer.grant_id),
            "invalid_encryption_recipients"
        );
        let (recipient, _) = public_key(&peer.key)?;
        // Multiple grants can intentionally share an endpoint key; wrap it once.
        if keys.insert(recipient.to_string()) {
            parsed.push(recipient);
        }
    }
    let all_files = stage_assets(stage)?;
    let files: Vec<_> = match context.group.as_deref() {
        None => all_files,
        Some("evidence") => all_files
            .into_iter()
            .filter(|f| !is_media_path(&f.path))
            .collect(),
        Some("media") => all_files
            .into_iter()
            .filter(|f| is_media_path(&f.path))
            .collect(),
        _ => anyhow::bail!("invalid_encryption_group"),
    };
    validate_group_manifest(&files, context.group.as_deref())?;
    validate_group_references(stage, &files, context.group.as_deref())?;
    let inner = Inner {
        version: 1,
        context: context.clone(),
        files: files.clone(),
    };
    let descriptor = Zeroizing::new(serde_json::to_vec(&inner)?);
    ensure!(descriptor.len() as u64 <= JSON_BYTES, "invalid_manifest");
    empty_private(spool)?;
    private_dir(&spool.join("sealed"))?;
    let writer = PartsWriter {
        spool: spool.to_owned(),
        file: None,
        size: 0,
        total: 0,
        count: 0,
    };
    let encryptor =
        age::Encryptor::with_recipients(parsed.iter().map(|r| r as &dyn age::Recipient))
            .map_err(|_| anyhow::anyhow!("encryption_failed"))?;
    let mut encrypted = encryptor.wrap_output(writer)?;
    encrypted.write_all(&(descriptor.len() as u32).to_be_bytes())?;
    encrypted.write_all(&descriptor)?;
    for asset in &files {
        let source = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(stage.join(&asset.path))?;
        ensure!(
            source.metadata()?.is_file() && source.metadata()?.len() == asset.size,
            "encryption_source_changed"
        );
        ensure!(
            io::copy(&mut source.take(asset.size + 1), &mut encrypted)? == asset.size,
            "encryption_source_changed"
        );
        ensure!(
            digest_file(&stage.join(&asset.path))? == asset.sha256,
            "encryption_source_changed"
        );
    }
    let writer = encrypted.finish()?;
    if let Some(f) = writer.file {
        f.sync_all()?;
    }
    let mut chunks = Vec::new();
    for i in 0..writer.count {
        let path = part_path(i);
        chunks.push(Asset {
            size: fs::metadata(spool.join(&path))?.len(),
            sha256: digest_file(&spool.join(&path))?,
            path,
        });
    }
    validate_chunks(&chunks)?;
    let signed = Signed {
        version: 1,
        context: context.clone(),
        chunks: chunks.clone(),
    };
    let signature = hex(&identity.signing.sign(&signature_bytes(&signed)?).to_bytes());
    let envelope = Envelope { signed, signature };
    let bytes = serde_json::to_vec(&envelope)?;
    ensure!(
        bytes.len() as u64 <= JSON_BYTES,
        "invalid_encryption_envelope"
    );
    let mut f = new_private_file(&spool.join("envelope.json"))?;
    f.write_all(&bytes)?;
    f.sync_all()?;
    chunks.insert(
        0,
        Asset {
            path: "envelope.json".into(),
            size: bytes.len() as u64,
            sha256: digest_file(&spool.join("envelope.json"))?,
        },
    );
    validate_transport(&chunks)?;
    File::open(spool.join("sealed"))?.sync_all()?;
    File::open(spool)?.sync_all()?;
    Ok(chunks)
}

struct PartsReader {
    files: std::vec::IntoIter<File>,
    current: Option<File>,
}
impl Read for PartsReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        loop {
            if self.current.is_none() {
                self.current = self.files.next();
            }
            let Some(file) = &mut self.current else {
                return Ok(0);
            };
            let n = file.read(bytes)?;
            if n > 0 {
                return Ok(n);
            }
            self.current = None;
        }
    }
}
// Read the entire textual header under a hard byte/stanza cap BEFORE giving age
// a reader. age still owns grammar/base64/MAC validation. Only native X25519 and
// the one mandatory library-generated GREASE stanza are admitted; no scrypt,
// plugins, SSH, armor or unbounded recipient work.
fn bounded_header(reader: &mut impl Read) -> Result<Vec<u8>> {
    let mut header = Vec::new();
    let mut line = Vec::new();
    let mut stanzas = 0;
    let mut native = 0;
    let mut grease = 0;
    loop {
        let mut byte = [0];
        reader.read_exact(&mut byte)?;
        header.push(byte[0]);
        line.push(byte[0]);
        ensure!(
            header.len() <= HEADER_BYTES && line.len() <= 1024,
            "invalid_encryption_header"
        );
        if byte[0] != b'\n' {
            continue;
        }
        if header.len() == line.len() {
            ensure!(
                line == b"age-encryption.org/v1\n",
                "invalid_encryption_header"
            );
        } else if line.starts_with(b"--- ") {
            ensure!(
                native > 0 && stanzas <= 65 && grease <= 1,
                "invalid_encryption_header"
            );
            return Ok(header);
        } else if line.starts_with(b"-> ") {
            stanzas += 1;
            if line.starts_with(b"-> X25519 ") {
                native += 1;
            } else {
                let tag = line[3..]
                    .split(|b| *b == b' ' || *b == b'\n')
                    .next()
                    .unwrap_or_default();
                ensure!(tag.ends_with(b"-grease"), "unsupported_encryption_header");
                grease += 1;
            }
            ensure!(
                native <= 64 && grease <= 1 && stanzas <= 65,
                "invalid_encryption_header"
            );
        }
        line.clear();
    }
}
fn spool_files(spool: &Path, chunks: &[Asset]) -> Result<PartsReader> {
    let mut expected = BTreeSet::from(["envelope.json".to_owned(), "sealed".to_owned()]);
    for entry in fs::read_dir(spool)? {
        let entry = entry?;
        ensure!(
            expected.remove(entry.file_name().to_str().unwrap_or("")),
            "invalid_encryption_parts"
        );
    }
    ensure!(expected.is_empty(), "invalid_encryption_parts");
    private_dir(&spool.join("sealed"))?;
    let mut expected: BTreeSet<_> = chunks
        .iter()
        .map(|c| c.path.strip_prefix("sealed/").unwrap().to_owned())
        .collect();
    for entry in fs::read_dir(spool.join("sealed"))? {
        let entry = entry?;
        ensure!(
            entry.file_type()?.is_file()
                && expected.remove(entry.file_name().to_str().unwrap_or("")),
            "invalid_encryption_parts"
        );
    }
    ensure!(expected.is_empty(), "invalid_encryption_parts");
    let mut files = Vec::new();
    for chunk in chunks {
        // Verify all ciphertext before writing ANY plaintext. Read/hash uses
        // caller-owned immutable secure scratch (same-uid mutation is not covered).
        let path = spool.join(&chunk.path);
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)?;
        ensure!(
            file.metadata()?.is_file()
                && file.metadata()?.len() == chunk.size
                && digest_file(&path)? == chunk.sha256,
            "encryption_ciphertext_mismatch"
        );
        files.push(file);
    }
    Ok(PartsReader {
        files: files.into_iter(),
        current: None,
    })
}

pub(super) fn unpack(
    spool: &Path,
    output: &Path,
    context: &Context,
    identity: &Identity,
    sender: &Peer,
) -> Result<Vec<Asset>> {
    context_valid(context)?;
    let envelope = read_envelope(spool)?;
    ensure!(
        &envelope.signed.context == context
            && sender.scope == "upload"
            && sender.grant_id == context.sender_grant_id,
        "encryption_context_mismatch"
    );
    let (_, key) = public_key(&sender.key)?;
    let signature = Signature::from_bytes(&unhex::<64>(&envelope.signature)?);
    key.verify_strict(&signature_bytes(&envelope.signed)?, &signature)
        .map_err(|_| anyhow::anyhow!("encryption_signature_invalid"))?;
    let mut parts = spool_files(spool, &envelope.signed.chunks)?;
    let header = bounded_header(&mut parts)?;
    let reader = io::Cursor::new(header).chain(parts);
    let decryptor =
        age::Decryptor::new(reader).map_err(|_| anyhow::anyhow!("encryption_decrypt_failed"))?;
    let mut plaintext = decryptor
        .decrypt(std::iter::once(&identity.age as &dyn age::Identity))
        .map_err(|_| anyhow::anyhow!("encryption_decrypt_failed"))?;
    let mut length = [0; 4];
    plaintext.read_exact(&mut length)?;
    let length = u32::from_be_bytes(length) as usize;
    ensure!(
        length > 0 && length as u64 <= JSON_BYTES,
        "invalid_encryption_archive"
    );
    let mut descriptor = Zeroizing::new(vec![0; length]);
    plaintext.read_exact(&mut descriptor)?;
    let inner: Inner = serde_json::from_slice(&descriptor)
        .map_err(|_| anyhow::anyhow!("invalid_encryption_archive"))?;
    ensure!(
        inner.version == 1 && &inner.context == context,
        "encryption_context_mismatch"
    );
    validate_group_manifest(&inner.files, context.group.as_deref())?;
    empty_private(output)?;
    for asset in &inner.files {
        let path = output.join(&asset.path);
        private_dir(path.parent().context("invalid_encryption_archive")?)?;
        let mut file = new_private_file(&path)?;
        ensure!(
            io::copy(&mut (&mut plaintext).take(asset.size), &mut file)? == asset.size,
            "encryption_archive_truncated"
        );
        file.sync_all()?;
        ensure!(
            digest_file(&path)? == asset.sha256,
            "encryption_plaintext_mismatch"
        );
    }
    // Read through authenticated FINAL chunk and EOF. Detect extra plaintext,
    // trailing ciphertext, incomplete final AEAD and concatenated age streams.
    let mut tail = [0];
    ensure!(
        plaintext.read(&mut tail)? == 0,
        "encryption_archive_trailing_data"
    );
    validate_group_references(output, &inner.files, context.group.as_deref())?;
    File::open(output)?.sync_all()?;
    Ok(inner.files)
}

#[cfg(test)]
#[path = "encryption_tests.rs"]
mod tests;
