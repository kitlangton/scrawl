// Small, OUT_DIR-local cache for expensive Apple native compiler outputs.
// Identity construction is kept at the call site so the recorded inputs mirror argv.
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(test)]
use std::time::{SystemTime, UNIX_EPOCH};
use std::{
    collections::BTreeSet,
    fs,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::OnceLock,
    time::{Duration, Instant},
};

pub fn add_field(record: &mut Vec<u8>, name: &[u8], value: &[u8]) {
    record.extend_from_slice(&(name.len() as u64).to_be_bytes());
    record.extend_from_slice(name);
    record.extend_from_slice(&(value.len() as u64).to_be_bytes());
    record.extend_from_slice(value);
}

pub fn add_file(record: &mut Vec<u8>, label: &str, path: &Path) -> std::io::Result<()> {
    add_field(record, label.as_bytes(), &fs::read(path)?);
    Ok(())
}

/// Return compiler/tool identity bytes, or None so callers can safely bypass reuse.
pub fn apple_toolchain_identity() -> Option<Vec<u8>> {
    // The selected SDK cannot change during one build-script execution. Hash it
    // once for the Swift helper and both shaders, without persisting a weaker
    // timestamp-only SDK cache across executions.
    static IDENTITY: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    IDENTITY
        .get_or_init(|| timed("Apple toolchain fingerprint", discover_apple_toolchain))
        .clone()
}

fn discover_apple_toolchain() -> Option<Vec<u8>> {
    let dev = command_output("xcode-select", &["-p"])?;
    let watches = developer_watches(
        Path::new(dev.trim()),
        std::env::var_os("DEVELOPER_DIR").as_deref(),
        Path::new("/var/db/xcode_select_link"),
    )?;
    let xcode = command_output("xcodebuild", &["-version"])?;
    let sdk_path = command_output("xcrun", &["--sdk", "macosx", "--show-sdk-path"])?;
    let sdk_version = command_output("xcrun", &["--sdk", "macosx", "--show-sdk-version"])?;
    let sdk_build = command_output("xcrun", &["--sdk", "macosx", "--show-sdk-build-version"])?;
    let mut record = Vec::new();
    for program in ["xcrun", "xcode-select", "xcodebuild"] {
        let path = std::env::split_paths(&std::env::var_os("PATH")?)
            .map(|dir| dir.join(program))
            .find(|path| path.is_file())?;
        timed(&format!("Apple identity tool {program}"), || {
            fingerprint_tool(&mut record, &path)
        })?;
    }
    for path in watches {
        if path.exists() {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }
    let mut sdks = vec![PathBuf::from(sdk_path.trim())];
    if let Some(sdk) = std::env::var_os("SDKROOT") {
        let sdk = PathBuf::from(sdk);
        // A non-absolute custom SDK cannot be identified reliably here.
        if !sdk.is_absolute() || !sdk.is_dir() {
            return None;
        }
        sdks.push(sdk);
    }
    for sdk in sdks {
        println!("cargo:rerun-if-changed={}", sdk.display());
        add_field(&mut record, b"sdk-path", sdk.as_os_str().as_encoded_bytes());
        add_field(
            &mut record,
            b"sdk-tree-sha256",
            timed("SDK content fingerprint", || tree_digest(&sdk))
                .ok()?
                .as_bytes(),
        );
    }
    for (name, value) in [
        ("developer-dir", dev.trim()),
        ("xcode", xcode.trim()),
        ("sdk-path", sdk_path.trim()),
        ("sdk-version", sdk_version.trim()),
        ("sdk-build", sdk_build.trim()),
    ] {
        add_field(&mut record, name.as_bytes(), value.as_bytes());
    }
    Some(record)
}

pub fn developer_watches(
    selected: &Path,
    explicit: Option<&std::ffi::OsStr>,
    selector: &Path,
) -> Option<Vec<PathBuf>> {
    let mut watches = vec![
        selected.parent()?.join("Info.plist"),
        selected.parent()?.join("version.plist"),
    ];
    if let Some(explicit) = explicit {
        let explicit = Path::new(explicit);
        if !explicit.is_absolute()
            || !explicit.is_dir()
            || fs::canonicalize(explicit).ok()? != fs::canonicalize(selected).ok()?
        {
            return None;
        }
        // Cargo follows directory symlinks recursively. An explicit validated
        // selection makes the global selector (and its entire Xcode tree) irrelevant.
    } else {
        watches.push(selector.to_path_buf());
    }
    Some(watches)
}

pub fn tool_identity(executable: &str, version_args: &[&str]) -> Option<Vec<u8>> {
    let (path, sdk_args): (String, Vec<&str>) = if executable == "metal" || executable == "metallib"
    {
        (
            command_output("xcrun", &["--sdk", "macosx", "--find", executable])?,
            vec!["--sdk", "macosx", executable],
        )
    } else {
        (
            command_output("xcrun", &["--find", executable])?,
            vec![executable],
        )
    };
    let path = PathBuf::from(path.trim());
    let mut record = Vec::new();
    timed(&format!("native compiler fingerprint {executable}"), || {
        fingerprint_tool(&mut record, &path)
    })?;
    if !version_args.is_empty() {
        let mut args = sdk_args;
        args.extend_from_slice(version_args);
        let version = command_output("xcrun", &args)?;
        add_field(&mut record, b"tool-version", version.trim().as_bytes());
    }
    Some(record)
}

fn fingerprint_tool(record: &mut Vec<u8>, path: &Path) -> Option<()> {
    println!("cargo:rerun-if-changed={}", path.display());
    let resolved = fs::canonicalize(path).ok()?;
    println!("cargo:rerun-if-changed={}", resolved.display());
    add_field(record, b"tool-path", path.as_os_str().as_encoded_bytes());
    add_field(
        record,
        b"resolved-tool-path",
        resolved.as_os_str().as_encoded_bytes(),
    );
    add_field(
        record,
        b"tool-sha256",
        digest(fs::read(resolved).ok()?).ok()?.as_bytes(),
    );
    Some(())
}

pub fn add_relevant_env(record: &mut Vec<u8>, names: &[&str]) {
    for name in names {
        let value = std::env::var_os(name);
        add_field(record, name.as_bytes(), &[u8::from(value.is_some())]);
        if let Some(value) = value {
            add_field(record, name.as_bytes(), value.as_encoded_bytes());
        }
        println!("cargo:rerun-if-env-changed={name}");
    }
}

pub fn add_compiler_env(record: &mut Vec<u8>) {
    add_relevant_env(
        record,
        &[
            "PATH",
            "DEVELOPER_DIR",
            "TOOLCHAINS",
            "SDKROOT",
            "MACOSX_DEPLOYMENT_TARGET",
            "SWIFT_EXEC",
            "SWIFTFLAGS",
            "CLANG_MODULE_CACHE_PATH",
            "SWIFT_MODULECACHE_PATH",
            "DYLD_LIBRARY_PATH",
            "DYLD_FRAMEWORK_PATH",
            "CARGO_CFG_TARGET_ARCH",
            "CARGO_CFG_TARGET_OS",
            "PROFILE",
            "CARGO_CFG_TARGET_ENV",
            "APPARENT_HELPER_BUNDLE_ID",
            "APPARENT_PREVIEW_BUILD",
        ],
    );
    let mut extra: Vec<_> = std::env::vars_os()
        .filter(|(key, _)| {
            let key = key.to_string_lossy();
            [
                "SWIFT_", "SWIFT", "CLANG", "XCODE", "SDK", "MACOSX_", "DYLD_",
            ]
            .iter()
            .any(|prefix| key.starts_with(prefix))
        })
        .collect();
    extra.sort_by(|a, b| a.0.cmp(&b.0));
    for (key, value) in extra {
        let name = key.to_string_lossy().into_owned();
        add_field(record, key.as_encoded_bytes(), value.as_encoded_bytes());
        println!("cargo:rerun-if-env-changed={name}");
    }
}

/// Uses the fixed macOS system tool instead of introducing a build dependency.
pub fn digest(bytes: Vec<u8>) -> std::io::Result<String> {
    digest_stream(|input| input.write_all(&bytes))
}

// Stream SDK bytes rather than holding an SDK in memory or starting one hash
// process per header/module. Escaping symlinks and special files disable reuse.
#[derive(Default)]
struct TreeMetrics {
    canonicalize: Duration,
    metadata: Duration,
    enumerate: Duration,
    link_read: Duration,
    file_stream: Duration,
    paths: u64,
    directories: u64,
    links: u64,
    files: u64,
    bytes: u64,
}

fn measure<T>(elapsed: &mut Duration, operation: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let result = operation();
    *elapsed += start.elapsed();
    result
}

fn tree_digest(path: &Path) -> std::io::Result<String> {
    let mut metrics = TreeMetrics::default();
    let result = (|| {
        let root = measure(&mut metrics.canonicalize, || fs::canonicalize(path))?;
        digest_stream(|input| {
            write_tree(&root, &root, &mut BTreeSet::new(), input, &mut metrics)
        })
    })();
    // Aggregate existing operations only: no paths, file contents, extra reads or
    // traversals. File streaming includes open/read and writes to the hash pipe;
    // it is not an isolated disk-I/O or hashing measurement.
    println!(
        "cargo:warning=native build SDK walk: canonicalize={:.3}s metadata={:.3}s enumerate-sort={:.3}s link-read={:.3}s file-stream={:.3}s paths={} directories={} links={} files={} bytes={} success={}",
        metrics.canonicalize.as_secs_f64(), metrics.metadata.as_secs_f64(),
        metrics.enumerate.as_secs_f64(), metrics.link_read.as_secs_f64(),
        metrics.file_stream.as_secs_f64(), metrics.paths, metrics.directories,
        metrics.links, metrics.files, metrics.bytes, result.is_ok(),
    );
    result
}

fn write_tree(
    path: &Path,
    root: &Path,
    seen: &mut BTreeSet<PathBuf>,
    input: &mut dyn Write,
    metrics: &mut TreeMetrics,
) -> std::io::Result<()> {
    metrics.paths += 1;
    fn field(input: &mut dyn Write, value: &[u8]) -> std::io::Result<()> {
        input.write_all(&(value.len() as u64).to_be_bytes())?;
        input.write_all(value)
    }
    field(
        input,
        path.strip_prefix(root)
            .map_err(std::io::Error::other)?
            .as_os_str()
            .as_encoded_bytes(),
    )?;
    let resolved = measure(&mut metrics.canonicalize, || fs::canonicalize(path))?;
    if !resolved.starts_with(root) {
        return Err(std::io::Error::other("SDK symlink escapes SDK root"));
    }
    let metadata = measure(&mut metrics.metadata, || fs::symlink_metadata(path))?;
    if metadata.file_type().is_symlink() {
        metrics.links += 1;
        input.write_all(b"L")?;
        let link = measure(&mut metrics.link_read, || fs::read_link(path))?;
        field(input, link.as_os_str().as_encoded_bytes())?;
        return write_tree(&resolved, root, seen, input, metrics);
    }
    if !seen.insert(resolved) {
        return input.write_all(b"R");
    }
    if metadata.is_dir() {
        metrics.directories += 1;
        input.write_all(b"D")?;
        let children = measure(&mut metrics.enumerate, || {
            let mut children = fs::read_dir(path)?
                .map(|entry| entry.map(|entry| entry.path()))
                .collect::<std::io::Result<Vec<_>>>()?;
            children.sort();
            Ok::<_, std::io::Error>(children)
        })?;
        input.write_all(&(children.len() as u64).to_be_bytes())?;
        for child in children {
            write_tree(&child, root, seen, input, metrics)?;
        }
    } else if metadata.is_file() {
        metrics.files += 1;
        input.write_all(b"F")?;
        input.write_all(&metadata.len().to_be_bytes())?;
        let bytes = measure(&mut metrics.file_stream, || {
            std::io::copy(&mut fs::File::open(path)?, input)
        })?;
        metrics.bytes += bytes;
        if bytes != metadata.len() {
            return Err(std::io::Error::other("SDK file changed while hashing"));
        }
    } else {
        return Err(std::io::Error::other("unsupported SDK file type"));
    }
    Ok(())
}

fn digest_stream(
    write: impl FnOnce(&mut dyn Write) -> std::io::Result<()>,
) -> std::io::Result<String> {
    let mut child = Command::new("/usr/bin/openssl")
        .args(["dgst", "-sha256", "-r"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let written = write_buffered(child.stdin.take().unwrap(), write);
    let output = child.wait_with_output()?;
    written?;
    if !output.status.success() {
        return Err(std::io::Error::other("SHA-256 command failed"));
    }
    let hash = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_owned();
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(std::io::Error::other("invalid SHA-256 output"));
    }
    Ok(hash)
}

// SDK records contain many tiny length/type/path writes. Batch their transport
// without changing the byte stream or retaining the SDK in memory. Flush errors
// must propagate, and the pipe must close before waiting for the hash command's EOF.
fn write_buffered(
    sink: impl Write,
    write: impl FnOnce(&mut dyn Write) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let mut input = BufWriter::with_capacity(64 * 1024, sink);
    let result = write(&mut input).and_then(|()| input.flush());
    // Avoid an implicit retry on drop after a failed write/flush.
    let (_sink, _pending) = input.into_parts();
    result
}

/// Reuse only if receipt and every cached output validate; returns true on a cache hit.
/// On a miss the receipt is removed before invoking build and is published atomically only
/// after all outputs exist and have been hashed.
pub fn reuse_or_build<F>(
    cache: &Path,
    key: &[u8],
    outputs: &[(&str, PathBuf, bool)],
    build: F,
) -> std::io::Result<bool>
where
    F: FnOnce() -> std::io::Result<()>,
{
    fs::create_dir_all(cache)?;
    let key = match digest(key.to_vec()) {
        Ok(k) => k,
        Err(_) => {
            let _ = fs::remove_file(cache.join("receipt"));
            return build().map(|_| false);
        }
    };
    let receipt = cache.join("receipt");
    if let Ok(text) = fs::read_to_string(&receipt) {
        let mut lines = text.lines();
        if lines.next() == Some(key.as_str()) {
            let entries: Vec<_> = lines.collect();
            if entries.len() == outputs.len() {
                let mut valid = true;
                for ((name, path, executable), line) in outputs.iter().zip(entries) {
                    let mut fields = line.splitn(2, '\t');
                    let got_name = fields.next().unwrap_or("");
                    let expected = fields.next().unwrap_or("");
                    let actual = fs::read(path).ok().and_then(|b| digest(b).ok());
                    #[cfg(unix)]
                    let mode_ok = !executable
                        || fs::metadata(path).is_ok_and(|m| {
                            std::os::unix::fs::PermissionsExt::mode(&m.permissions()) & 0o111 != 0
                        });
                    #[cfg(not(unix))]
                    let mode_ok = !executable;
                    if got_name != *name || actual.as_deref() != Some(expected) || !mode_ok {
                        valid = false;
                        break;
                    }
                }
                if valid {
                    return Ok(true);
                }
            }
        }
    }
    let _ = fs::remove_file(&receipt);
    build()?;
    let mut lines = format!("{key}\n");
    for (name, source, executable) in outputs {
        let bytes = fs::read(source)?;
        #[cfg(unix)]
        if *executable && fs::metadata(source)?.permissions().mode() & 0o111 == 0 {
            return Err(std::io::Error::other(
                "native compiler produced a non-executable helper",
            ));
        }
        #[cfg(not(unix))]
        if *executable {
            let _ = executable;
        }
        let hash = match digest(bytes) {
            Ok(h) => h,
            Err(e) => {
                let _ = fs::remove_file(&receipt);
                return Err(e);
            }
        };
        lines.push_str(&format!("{name}\t{hash}\n"));
    }
    let tmp = cache.join(format!("receipt.{}.tmp", std::process::id()));
    fs::write(&tmp, lines)?;
    fs::rename(tmp, receipt)?;
    Ok(false)
}

pub fn recipe_identity() -> Vec<u8> {
    let mut record = Vec::new();
    add_field(
        &mut record,
        b"build-script-source",
        include_bytes!("../../build.rs"),
    );
    add_field(
        &mut record,
        b"cache-helper-source",
        include_bytes!("build_cache.rs"),
    );
    record
}

pub fn timed<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let value = f();
    println!(
        "cargo:warning=native build {label}: {:.3}s",
        start.elapsed().as_secs_f64()
    );
    value
}

fn command_output(program: &str, args: &[&str]) -> Option<String> {
    // Only fixed tool names/arguments from this module are logged, never values
    // supplied through environment variables or paths returned by the probes.
    let output = timed(
        &format!("tool probe {program} {}", args.join(" ")),
        || Command::new(program).args(args).output(),
    )
    .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).ok())
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sdk_metrics_preserve_serialization_and_operation_errors() {
        let root = temp();
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("a"), b"abc").unwrap();
        let root = fs::canonicalize(root).unwrap();
        let mut bytes = Vec::new();
        let mut metrics = TreeMetrics::default();
        write_tree(&root, &root, &mut BTreeSet::new(), &mut bytes, &mut metrics).unwrap();
        // Existing format: empty root path, directory/one child, path "a",
        // regular file/three bytes, then its unchanged payload.
        assert_eq!(bytes, b"\0\0\0\0\0\0\0\0D\0\0\0\0\0\0\0\x01\0\0\0\0\0\0\0\x01aF\0\0\0\0\0\0\0\x03abc");
        assert_eq!((metrics.paths, metrics.directories, metrics.files, metrics.bytes), (2, 1, 1, 3));
        let error = measure(&mut metrics.metadata, || {
            Err::<(), _>(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "fixture"))
        }).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::PermissionDenied);
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn system_sha256_matches_known_vectors() {
        for (bytes, expected) in [
            (Vec::new(), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"),
            (b"abc".to_vec(), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"),
            (vec![b'a'; 1_000_000], "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"),
        ] {
            assert_eq!(digest(bytes).unwrap(), expected);
        }
    }
    #[test]
    fn sdk_transport_preserves_bytes_batches_writes_and_propagates_errors() {
        #[derive(Default)]
        struct Sink { bytes: Vec<u8>, writes: usize }
        impl Write for Sink {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.writes += 1;
                self.bytes.extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
        }
        let root = temp();
        fs::create_dir_all(&root).unwrap();
        for index in 0..4000 {
            fs::write(root.join(format!("header-{index}.h")), b"small SDK header\n").unwrap();
        }
        let root = fs::canonicalize(root).unwrap();
        let mut direct = Sink::default();
        let mut metrics = TreeMetrics::default();
        write_tree(&root, &root, &mut BTreeSet::new(), &mut direct, &mut metrics).unwrap();
        assert_eq!((metrics.paths, metrics.directories, metrics.links, metrics.files, metrics.bytes),
            (4001, 1, 0, 4000, 68000));
        let mut buffered = Sink::default();
        write_buffered(&mut buffered, |input| {
            write_tree(&root, &root, &mut BTreeSet::new(), input, &mut TreeMetrics::default())
        }).unwrap();
        assert_eq!(direct.bytes, buffered.bytes);
        eprintln!("SDK fixture: {} bytes, {} direct writes, {} buffered writes",
            direct.bytes.len(), direct.writes, buffered.writes);
        assert!(buffered.writes * 100 < direct.writes);
        assert_eq!(tree_digest(&root).unwrap(), digest(direct.bytes).unwrap());
        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "closed"))
            }
            fn flush(&mut self) -> std::io::Result<()> { Ok(()) }
        }
        assert_eq!(write_buffered(Broken, |input| input.write_all(b"tail"))
            .unwrap_err().kind(), std::io::ErrorKind::BrokenPipe);
        struct FlushFailure;
        impl Write for FlushFailure {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> { Ok(bytes.len()) }
            fn flush(&mut self) -> std::io::Result<()> {
                Err(std::io::Error::other("flush failed"))
            }
        }
        assert!(write_buffered(FlushFailure, |input| input.write_all(b"tail")).is_err());
        assert!(write_buffered(Vec::new(), |_| Err(std::io::Error::other("source failed"))).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    fn temp() -> PathBuf {
        std::env::temp_dir().join(format!(
            "apparent-cache-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ))
    }
    #[test]
    fn fields_are_length_delimited() {
        let mut a = Vec::new();
        add_field(&mut a, b"ab", b"c");
        let mut b = Vec::new();
        add_field(&mut b, b"a", b"bc");
        assert_ne!(a, b);
    }
    #[test]
    fn sdk_content_and_internal_links_are_fingerprinted_and_escaping_links_fail() {
        let root = temp();
        let sdk = root.join("sdk");
        fs::create_dir_all(&sdk).unwrap();
        fs::write(sdk.join("SDKSettings.json"), b"unchanged").unwrap();
        fs::write(sdk.join("header.h"), b"before").unwrap();
        let first = tree_digest(&sdk).unwrap();
        assert_eq!(first, tree_digest(&sdk).unwrap());
        fs::write(sdk.join("header.h"), b"edited").unwrap();
        assert_ne!(first, tree_digest(&sdk).unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            symlink("header.h", sdk.join("alias.h")).unwrap();
            let linked = tree_digest(&sdk).unwrap();
            fs::write(sdk.join("header.h"), b"latest").unwrap();
            assert_ne!(linked, tree_digest(&sdk).unwrap());
            fs::write(root.join("outside"), b"outside").unwrap();
            symlink("../outside", sdk.join("escape.h")).unwrap();
            assert!(tree_digest(&sdk).is_err());
        }
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn hit_corruption_missing_and_failed_build_behavior() {
        let root = temp();
        fs::create_dir_all(&root).unwrap();
        let cache = root.join("cache");
        let out = root.join("output");
        fs::write(&out, b"first").unwrap();
        let outputs = [("artifact", out.clone(), false)];
        let key = b"identity";
        assert!(!reuse_or_build(&cache, key, &outputs, || Ok(())).unwrap());
        assert!(reuse_or_build(&cache, key, &outputs, || panic!("unexpected build")).unwrap());
        fs::write(&out, b"corrupt").unwrap();
        assert!(
            !reuse_or_build(&cache, key, &outputs, || { fs::write(&out, b"rebuilt") }).unwrap()
        );
        fs::remove_file(&out).unwrap();
        assert!(!reuse_or_build(&cache, key, &outputs, || { fs::write(&out, b"again") }).unwrap());
        let receipt = cache.join("receipt");
        assert!(
            reuse_or_build(&cache, b"changed", &outputs, || Err(std::io::Error::other(
                "failed"
            )))
            .is_err()
        );
        assert!(!receipt.exists());
        let _ = fs::remove_dir_all(root);
    }
}
