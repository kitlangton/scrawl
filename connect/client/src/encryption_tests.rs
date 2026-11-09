use super::*;
use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

struct Fixture {
    root: PathBuf,
    stage: PathBuf,
    spool: PathBuf,
    output: PathBuf,
    sender: Identity,
    reader: Identity,
    context: Context,
}
impl Fixture {
    fn new() -> Self {
        Self::in_temp_base(&std::env::temp_dir())
    }
    fn in_temp_base(base: &Path) -> Self {
        // Darwin's system temp directory can contain /var -> /private/var.
        // Resolve the trusted fixture base before creating private artifacts;
        // production paths still reject all symlink components unchanged.
        let root = base
            .canonicalize()
            .unwrap()
            .join(format!("crypto-{}", super::super::random_id().unwrap()));
        private_dir(&root).unwrap();
        let stage = root.join("stage");
        let spool = root.join("spool");
        let output = root.join("output");
        for p in [&stage, &spool, &output] {
            private_dir(p).unwrap();
        }
        for (path, data) in [
            (
                "apparent.json",
                b"{\"contactSheet\":\"contact-sheet.png\"}".as_slice(),
            ),
            (
                "summary.md",
                b"private synthetic transcript /home/synthetic-secret",
            ),
            ("contact-sheet.png", b"synthetic opaque image fixture"),
        ] {
            new_private_file(&stage.join(path))
                .unwrap()
                .write_all(data)
                .unwrap();
        }
        let sender = load_or_create_identity(&root.join("sender.identity")).unwrap();
        let reader = load_or_create_identity(&root.join("reader.identity")).unwrap();
        Self {
            root,
            stage,
            spool,
            output,
            sender,
            reader,
            context: Context {
                origin: "https://api.example.com".into(),
                account_id: "account-a".into(),
                capture_id: "0123456789abcdef0123456789abcdef".into(),
                sender_grant_id: "sender-grant".into(),
            },
        }
    }
    fn sender_peer(&self) -> Peer {
        Peer {
            grant_id: self.context.sender_grant_id.clone(),
            scope: "upload".into(),
            key: self.sender.public_key(),
        }
    }
    fn recipients(&self) -> Vec<Peer> {
        vec![
            self.sender_peer(),
            Peer {
                grant_id: "reader-grant".into(),
                scope: "read".into(),
                key: self.reader.public_key(),
            },
        ]
    }
    fn pack(&self) -> Vec<Asset> {
        pack(
            &self.stage,
            &self.spool,
            &self.context,
            &self.sender,
            &self.recipients(),
        )
        .unwrap()
    }
    fn unpack(&self) -> Result<Vec<Asset>> {
        unpack(
            &self.spool,
            &self.output,
            &self.context,
            &self.reader,
            &self.sender_peer(),
        )
    }
    fn resign(&self, envelope: &mut Envelope) {
        envelope.signature = hex(&self
            .sender
            .signing
            .sign(&signature_bytes(&envelope.signed).unwrap())
            .to_bytes());
        fs::write(
            self.spool.join("envelope.json"),
            serde_json::to_vec(envelope).unwrap(),
        )
        .unwrap();
    }
    fn seal_raw(&self, bytes: &[u8]) {
        private_dir(&self.spool.join("sealed")).unwrap();
        let recipient = self.reader.age.to_public();
        let writer = PartsWriter {
            spool: self.spool.clone(),
            file: None,
            size: 0,
            total: 0,
            count: 0,
        };
        let mut stream =
            age::Encryptor::with_recipients(std::iter::once(&recipient as &dyn age::Recipient))
                .unwrap()
                .wrap_output(writer)
                .unwrap();
        stream.write_all(bytes).unwrap();
        let writer = stream.finish().unwrap();
        writer.file.unwrap().sync_all().unwrap();
        let path = part_path(0);
        let chunk = Asset {
            size: fs::metadata(self.spool.join(&path)).unwrap().len(),
            sha256: digest_file(&self.spool.join(&path)).unwrap(),
            path,
        };
        let mut envelope = Envelope {
            signed: Signed {
                version: 1,
                context: self.context.clone(),
                chunks: vec![chunk],
            },
            signature: String::new(),
        };
        self.resign(&mut envelope);
    }
    fn archive(&self, mutate: impl FnOnce(&mut Inner), trailing: &[u8]) -> Vec<u8> {
        let files = stage_assets(&self.stage).unwrap();
        let mut inner = Inner {
            version: 1,
            context: self.context.clone(),
            files: files.clone(),
        };
        mutate(&mut inner);
        let descriptor = serde_json::to_vec(&inner).unwrap();
        let mut out = (descriptor.len() as u32).to_be_bytes().to_vec();
        out.extend(descriptor);
        for file in files {
            out.extend(fs::read(self.stage.join(file.path)).unwrap());
        }
        out.extend(trailing);
        out
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn fixture_resolves_temp_alias_without_relaxing_production_symlink_rejection() {
    let outer = Fixture::new();
    let alias = outer.root.join("temp-alias");
    symlink(&outer.root, &alias).unwrap();
    assert!(private_dir(&alias).is_err());
    let nested = Fixture::in_temp_base(&alias);
    assert!(!nested.root.starts_with(&alias));
    assert_eq!(nested.root, nested.root.canonicalize().unwrap());
    assert_eq!(fs::metadata(&nested.root).unwrap().mode() & 0o777, 0o700);
}

#[test]
fn roundtrip_multiple_recipients_preserves_exact_private_assets() {
    let f = Fixture::new();
    let transport = f.pack();
    validate_transport(&transport).unwrap();
    assert!(envelope_context(&f.spool).unwrap() == f.context);
    let assets = f.unpack().unwrap();
    for asset in assets {
        assert_eq!(
            fs::read(f.stage.join(&asset.path)).unwrap(),
            fs::read(f.output.join(&asset.path)).unwrap()
        );
    }
    let sender_out = f.root.join("sender-out");
    private_dir(&sender_out).unwrap();
    unpack(
        &f.spool,
        &sender_out,
        &f.context,
        &f.sender,
        &f.sender_peer(),
    )
    .unwrap();
    for asset in transport {
        let bytes = fs::read(f.spool.join(asset.path)).unwrap();
        for secret in [
            b"summary.md".as_slice(),
            b"private synthetic transcript",
            b"/home/synthetic-secret",
        ] {
            assert!(!bytes.windows(secret.len()).any(|w| w == secret));
        }
    }
}
#[test]
fn fresh_pack_randomizes_ciphertext_without_changing_source() {
    let f = Fixture::new();
    f.pack();
    let other = f.root.join("other-spool");
    private_dir(&other).unwrap();
    pack(&f.stage, &other, &f.context, &f.sender, &f.recipients()).unwrap();
    assert_ne!(
        fs::read(f.spool.join(part_path(0))).unwrap(),
        fs::read(other.join(part_path(0))).unwrap()
    );
}
#[test]
fn identity_persists_privately_and_corruption_is_not_replaced() {
    let f = Fixture::new();
    let path = f.root.join("sender.identity");
    assert_eq!(
        load_identity(&path).unwrap().public_key(),
        f.sender.public_key()
    );
    assert_eq!(
        load_or_create_identity(&path).unwrap().public_key(),
        f.sender.public_key()
    );
    assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
    assert_eq!(fs::metadata(&f.root).unwrap().mode() & 0o777, 0o700);
    fs::write(&path, b"broken identity private sentinel").unwrap();
    assert!(load_or_create_identity(&path).is_err());
    assert_eq!(
        fs::read(&path).unwrap(),
        b"broken identity private sentinel"
    );
    fs::remove_file(&path).unwrap();
    assert!(load_identity(&path).is_err());
    assert!(!path.exists());
}
#[test]
fn identity_permissions_and_symlinks_fail_closed() {
    let f = Fixture::new();
    let path = f.root.join("sender.identity");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(load_or_create_identity(&path).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    let alias = f.root.join("alias.identity");
    symlink(&path, &alias).unwrap();
    assert!(load_or_create_identity(&alias).is_err());
    assert_eq!(
        load_identity(&path).unwrap().public_key(),
        f.sender.public_key()
    );
}
#[test]
fn concurrent_identity_creation_never_replaces_winner() {
    let f = Fixture::new();
    let path = f.root.join("concurrent.identity");
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let path = path.clone();
            std::thread::spawn(move || load_or_create_identity(&path).unwrap().public_key())
        })
        .collect();
    let keys: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert!(keys.iter().all(|k| k == &keys[0]));
}
#[test]
fn registry_descriptors_are_strict_and_canonical() {
    let f = Fixture::new();
    let original = f.sender.public_key();
    validate_public_key(&original).unwrap();
    for mode in 0..7 {
        let mut key = original.clone();
        match mode {
            0 => key["version"] = 2.into(),
            1 => key["extra"] = true.into(),
            2 => key["recipient"] = key["recipient"].as_str().unwrap().to_uppercase().into(),
            3 => key["signing"] = "00".repeat(32).into(),
            4 => key["signing"] = "FF".repeat(32).into(),
            5 => key["recipient"] = "x".repeat(100_000).into(),
            _ => key["signing"] = "z".repeat(64).into(),
        }
        assert!(validate_public_key(&key).is_err(), "case {mode}");
    }
}
#[test]
fn context_replay_and_sender_substitution_release_no_plaintext() {
    let f = Fixture::new();
    f.pack();
    for mode in 0..5 {
        let mut context = f.context.clone();
        let mut sender = f.sender_peer();
        match mode {
            0 => context.origin = "https://other.example.com".into(),
            1 => context.account_id = "other-account".into(),
            2 => context.capture_id = "1123456789abcdef0123456789abcdef".into(),
            3 => context.sender_grant_id = "other-grant".into(),
            _ => sender.key = f.reader.public_key(),
        }
        assert!(unpack(&f.spool, &f.output, &context, &f.reader, &sender).is_err());
        assert!(fs::read_dir(&f.output).unwrap().next().is_none());
    }
}
#[test]
fn ciphertext_damage_missing_extra_and_bad_signature_release_no_plaintext() {
    for mode in 0..6 {
        let f = Fixture::new();
        f.pack();
        let path = f.spool.join(part_path(0));
        match mode {
            0 => {
                let mut bytes = fs::read(&path).unwrap();
                let i = bytes.len() - 1;
                bytes[i] ^= 1;
                fs::write(&path, bytes).unwrap();
            }
            1 => {
                let n = fs::metadata(&path).unwrap().len();
                OpenOptions::new()
                    .write(true)
                    .open(&path)
                    .unwrap()
                    .set_len(n - 1)
                    .unwrap();
            }
            2 => {
                fs::remove_file(&path).unwrap();
            }
            3 => {
                new_private_file(&f.spool.join("sealed/part-000001.age")).unwrap();
            }
            4 => {
                let mut e = read_envelope(&f.spool).unwrap();
                e.signature = "00".repeat(64);
                fs::write(
                    f.spool.join("envelope.json"),
                    serde_json::to_vec(&e).unwrap(),
                )
                .unwrap();
            }
            _ => {
                new_private_file(&f.spool.join("summary.md")).unwrap();
            }
        }
        assert!(f.unpack().is_err(), "case {mode}");
        assert!(fs::read_dir(&f.output).unwrap().next().is_none());
    }
}
#[test]
fn wrong_recipient_releases_no_plaintext() {
    let f = Fixture::new();
    f.pack();
    let outsider = load_or_create_identity(&f.root.join("outsider.identity")).unwrap();
    assert!(unpack(&f.spool, &f.output, &f.context, &outsider, &f.sender_peer()).is_err());
    assert!(fs::read_dir(&f.output).unwrap().next().is_none());
}
#[test]
fn authenticated_malformed_archives_are_rejected() {
    for mode in 0..9 {
        let f = Fixture::new();
        let archive = f.archive(
            |inner| match mode {
                0 => inner.version = 2,
                1 => inner.files.push(inner.files[0].clone()),
                2 => inner.files[0].path = "../escape".into(),
                3 => inner.files[0].sha256 = "00".repeat(32),
                4 => inner.files[0].size += 1,
                5 => inner.files[0].size = 90_000_001,
                6 => inner.context.account_id = "other-account".into(),
                _ => (),
            },
            if mode == 7 {
                b"extra archive record"
            } else {
                b""
            },
        );
        let archive = if mode == 8 {
            &archive[..archive.len() - 1]
        } else {
            &archive[..]
        };
        f.seal_raw(archive);
        assert!(f.unpack().is_err(), "case {mode}");
        assert!(!f.root.join("escape").exists());
    }
}
#[test]
fn signed_ciphertext_truncation_and_trailing_bytes_fail_age_finality() {
    for mode in 0..3 {
        let f = Fixture::new();
        f.pack();
        let path = f.spool.join(part_path(0));
        let mut bytes = fs::read(&path).unwrap();
        match mode {
            0 => {
                bytes.pop();
            }
            1 => bytes.extend(b"trailing ciphertext"),
            _ => {
                let n = bytes.len();
                bytes[n - 1] ^= 1;
            }
        }
        fs::write(&path, bytes).unwrap();
        let mut e = read_envelope(&f.spool).unwrap();
        e.signed.chunks[0].size = fs::metadata(&path).unwrap().len();
        e.signed.chunks[0].sha256 = digest_file(&path).unwrap();
        f.resign(&mut e);
        assert!(f.unpack().is_err(), "case {mode}");
    }
}
#[test]
fn transport_and_header_work_are_bounded() {
    let f = Fixture::new();
    let original = f.pack();
    for mode in 0..7 {
        let mut files = original.clone();
        match mode {
            0 => files[1].path = "sealed/part-000001.age".into(),
            1 => files[1].size = PART_BYTES + 1,
            2 => files.push(files[1].clone()),
            3 => files[0].size = JSON_BYTES + 1,
            4 => files[1].size = 0,
            5 => files[0].path = "summary.md".into(),
            _ => files[1].sha256 = "A".repeat(64),
        }
        assert!(validate_transport(&files).is_err(), "case {mode}");
    }
    for bytes in [
        b"age-encryption.org/v2\n".to_vec(),
        b"age-encryption.org/v1\n-> scrypt x 99\n--- x\n".to_vec(),
        format!(
            "age-encryption.org/v1\n{}--- x\n",
            "-> X25519 x\nx\n".repeat(65)
        )
        .into_bytes(),
        format!("age-encryption.org/v1\n{}", "a".repeat(HEADER_BYTES + 1)).into_bytes(),
    ] {
        assert!(bounded_header(&mut bytes.as_slice()).is_err());
    }
}
#[test]
fn source_allowlist_limits_and_nonempty_scratch_are_preserved() {
    let f = Fixture::new();
    let unknown = f.stage.join("credentials.json");
    new_private_file(&unknown).unwrap();
    assert!(pack(&f.stage, &f.spool, &f.context, &f.sender, &f.recipients()).is_err());
    assert!(fs::read_dir(&f.spool).unwrap().next().is_none());
    fs::remove_file(unknown).unwrap();
    let oversized = f.stage.join("recording.mp4");
    new_private_file(&oversized)
        .unwrap()
        .set_len(90_000_001)
        .unwrap();
    assert!(pack(&f.stage, &f.spool, &f.context, &f.sender, &f.recipients()).is_err());
    fs::remove_file(oversized).unwrap();
    let sentinel = f.spool.join("caller-owned");
    new_private_file(&sentinel)
        .unwrap()
        .write_all(b"preserve")
        .unwrap();
    assert!(pack(&f.stage, &f.spool, &f.context, &f.sender, &f.recipients()).is_err());
    assert_eq!(fs::read(sentinel).unwrap(), b"preserve");
}
#[test]
fn continuous_stream_crosses_transport_boundary_and_rejects_reordering() {
    let f = Fixture::new();
    new_private_file(&f.stage.join("recording.mp4"))
        .unwrap()
        .set_len(PART_BYTES + 1024)
        .unwrap();
    let files = f.pack();
    assert_eq!(files.len(), 3);
    f.unpack().unwrap();
    let mut e = read_envelope(&f.spool).unwrap();
    e.signed.chunks.swap(0, 1);
    f.resign(&mut e);
    let second = f.root.join("second");
    private_dir(&second).unwrap();
    assert!(unpack(&f.spool, &second, &f.context, &f.reader, &f.sender_peer()).is_err());
    assert!(fs::read_dir(second).unwrap().next().is_none());
}
#[test]
fn peer_limit_duplicate_grants_and_unknown_envelope_fields_fail_closed() {
    let f = Fixture::new();
    let many: Vec<_> = (0..65)
        .map(|i| Peer {
            grant_id: format!("g-{i}"),
            scope: "read".into(),
            key: f.reader.public_key(),
        })
        .collect();
    assert!(pack(&f.stage, &f.spool, &f.context, &f.sender, &many).is_err());
    let peers = vec![f.sender_peer(), f.sender_peer()];
    assert!(pack(&f.stage, &f.spool, &f.context, &f.sender, &peers).is_err());
    f.pack();
    let mut value: Value =
        serde_json::from_slice(&fs::read(f.spool.join("envelope.json")).unwrap()).unwrap();
    value["unknown"] = true.into();
    fs::write(
        f.spool.join("envelope.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    assert!(f.unpack().is_err());
}

#[test]
fn actual_snapshot_with_nonprivate_nested_modes_roundtrips() {
    let f = Fixture::new();
    let frames = f.stage.join("frames");
    private_dir(&frames).unwrap();
    new_private_file(&frames.join("frame-000001.png"))
        .unwrap()
        .write_all(b"frame pixels")
        .unwrap();
    fs::set_permissions(&frames, fs::Permissions::from_mode(0o755)).unwrap();
    let staged = f.root.join("snapshot");
    private_dir(&staged).unwrap();
    super::super::snapshot(&f.stage, &staged).unwrap();
    // Exercise the real producer, regardless of the test process's umask.
    fs::set_permissions(staged.join("frames"), fs::Permissions::from_mode(0o755)).unwrap();
    pack(&staged, &f.spool, &f.context, &f.sender, &f.recipients()).unwrap();
    f.unpack().unwrap();
    assert_eq!(
        fs::read(f.output.join("frames/frame-000001.png")).unwrap(),
        b"frame pixels"
    );
}

#[test]
fn inner_count_total_metadata_and_nested_unknown_fields_are_bounded() {
    for mode in 0..4 {
        let f = Fixture::new();
        let bytes = if mode == 2 {
            let mut bytes = ((JSON_BYTES + 1) as u32).to_be_bytes().to_vec();
            bytes.extend(vec![b' '; JSON_BYTES as usize + 1]);
            bytes
        } else if mode == 3 {
            let mut inner = serde_json::to_value(Inner {
                version: 1,
                context: f.context.clone(),
                files: stage_assets(&f.stage).unwrap(),
            })
            .unwrap();
            inner["files"][0]["unexpected"] = true.into();
            let descriptor = serde_json::to_vec(&inner).unwrap();
            let mut bytes = (descriptor.len() as u32).to_be_bytes().to_vec();
            bytes.extend(descriptor);
            bytes
        } else {
            f.archive(
                |inner| {
                    if mode == 0 {
                        for i in 0..513 {
                            inner.files.push(Asset {
                                path: format!("frames/frame-{i}.png"),
                                size: 0,
                                sha256: "00".repeat(32),
                            });
                        }
                    } else {
                        for i in 0..6 {
                            inner.files.push(Asset {
                                path: format!("frames/frame-{i}.png"),
                                size: 90_000_000,
                                sha256: "00".repeat(32),
                            });
                        }
                    }
                },
                b"",
            )
        };
        f.seal_raw(&bytes);
        assert!(f.unpack().is_err(), "case {mode}");
        assert!(fs::read_dir(&f.output).unwrap().next().is_none());
    }
    let f = Fixture::new();
    f.pack();
    let mut value: Value =
        serde_json::from_slice(&fs::read(f.spool.join("envelope.json")).unwrap()).unwrap();
    value["signed"]["chunks"][0]["unexpected"] = true.into();
    fs::write(
        f.spool.join("envelope.json"),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    assert!(f.unpack().is_err());
}

#[test]
fn nonempty_decrypt_scratch_and_symlink_sources_are_not_overwritten() {
    let f = Fixture::new();
    f.pack();
    let sentinel = f.output.join("summary.md");
    new_private_file(&sentinel)
        .unwrap()
        .write_all(b"caller sentinel")
        .unwrap();
    assert!(f.unpack().is_err());
    assert_eq!(fs::read(sentinel).unwrap(), b"caller sentinel");
    let f = Fixture::new();
    symlink(f.root.join("sender.identity"), f.stage.join("audio.wav")).unwrap();
    assert!(pack(&f.stage, &f.spool, &f.context, &f.sender, &f.recipients()).is_err());
    assert!(fs::read_dir(&f.spool).unwrap().next().is_none());
}

#[test]
#[ignore = "requires the production TypeScript crypto interop script and Node 22"]
fn production_typescript_crypto_interop() {
    let node = std::env::var_os("APPARENT_CONNECT_INTEROP_NODE")
        .expect("set APPARENT_CONNECT_INTEROP_NODE to the absolute Node 22 executable");
    let script = PathBuf::from(
        std::env::var_os("APPARENT_CONNECT_INTEROP_SCRIPT")
            .expect("set APPARENT_CONNECT_INTEROP_SCRIPT to the absolute interop script path"),
    );
    assert!(
        Path::new(&node).is_absolute(),
        "Node executable must be absolute"
    );
    assert!(script.is_absolute(), "interop script path must be absolute");

    let f = Fixture::new();
    // Force the TypeScript side to exercise multiple bounded transport parts.
    let summary: Vec<u8> = (0..33 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
    fs::write(f.stage.join("summary.md"), &summary).unwrap();
    let transport = f.pack();
    validate_transport(&transport).unwrap();
    let envelope = read_envelope(&f.spool).unwrap();
    let signed_bytes = signature_bytes(&envelope.signed).unwrap();
    let mut weak_key = [0u8; 32];
    weak_key[0] = 1; // Identity point.
    let mut weak_signature = [0u8; 64];
    weak_signature[0] = 1;
    let mut noncanonical_scalar = unhex::<64>(&envelope.signature).unwrap();
    noncanonical_scalar[32..].fill(0xff);
    let strict_cases = [
        (weak_key, weak_signature),
        (f.sender.signing.verifying_key().to_bytes(), weak_signature),
        (f.sender.signing.verifying_key().to_bytes(), noncanonical_scalar),
        ([0xff; 32], unhex::<64>(&envelope.signature).unwrap()),
    ].into_iter().map(|(key, signature)| {
        let accepted = VerifyingKey::from_bytes(&key)
            .is_ok_and(|key| key.verify_strict(&signed_bytes, &Signature::from_bytes(&signature)).is_ok());
        assert!(!accepted, "strict Ed25519 negative vector was accepted");
        serde_json::json!({"publicKey": hex(&key), "signature": hex(&signature), "accepted": accepted})
    }).collect::<Vec<_>>();
    let expected = stage_assets(&f.stage)
        .unwrap()
        .into_iter()
        .map(|asset| {
            let bytes = fs::read(f.stage.join(&asset.path)).unwrap();
            serde_json::json!({"path": asset.path, "hex": hex(&bytes)})
        })
        .collect::<Vec<_>>();
    let manifest = serde_json::json!({
        "context": f.context,
        "senderKey": f.sender.public_key(),
        "readerKey": f.reader.public_key(),
        "signedBytesHex": hex(&signature_bytes(&envelope.signed).unwrap()),
        "strictCases": strict_cases,
        "expected": expected,
    });
    fs::write(
        f.root.join("interop.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();

    let status = std::process::Command::new(node)
        .arg(script)
        .arg(&f.root)
        .status()
        .expect("launch production TypeScript crypto interop script");
    assert!(
        status.success(),
        "TypeScript crypto interop script failed: {status}"
    );

    let assets = f
        .unpack()
        .expect("Rust must unpack TypeScript production output");
    for asset in assets {
        assert_eq!(
            fs::read(f.stage.join(&asset.path)).unwrap(),
            fs::read(f.output.join(&asset.path)).unwrap(),
            "TypeScript roundtrip changed {}",
            asset.path
        );
    }

    // Exercise Rust's rejection paths against the TypeScript-produced archive.
    let tampered_spool = f.root.join("tampered-spool");
    private_dir(&tampered_spool).unwrap();
    for entry in fs::read_dir(&f.spool).unwrap() {
        let entry = entry.unwrap();
        let destination = tampered_spool.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            private_dir(&destination).unwrap();
            for child in fs::read_dir(entry.path()).unwrap() {
                let child = child.unwrap();
                fs::copy(child.path(), destination.join(child.file_name())).unwrap();
            }
        } else {
            fs::copy(entry.path(), destination).unwrap();
        }
    }
    let part = tampered_spool.join(part_path(0));
    let mut bytes = fs::read(&part).unwrap();
    let last = bytes.len() - 1;
    bytes[last] ^= 1;
    fs::write(part, bytes).unwrap();
    let tampered_output = f.root.join("tampered-output");
    private_dir(&tampered_output).unwrap();
    assert!(
        unpack(
            &tampered_spool,
            &tampered_output,
            &f.context,
            &f.reader,
            &f.sender_peer()
        )
        .is_err()
    );
    assert!(fs::read_dir(tampered_output).unwrap().next().is_none());

    let outsider = load_or_create_identity(&f.root.join("outsider.identity")).unwrap();
    let wrong_output = f.root.join("wrong-recipient-output");
    private_dir(&wrong_output).unwrap();
    assert!(
        unpack(
            &f.spool,
            &wrong_output,
            &f.context,
            &outsider,
            &f.sender_peer()
        )
        .is_err()
    );
    assert!(fs::read_dir(wrong_output).unwrap().next().is_none());
}
