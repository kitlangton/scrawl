//! Linux capture/2 provider. This worker deliberately exposes X11 outputs only.
#![cfg(target_os = "linux")]

use apparent_capture_engine::{CaptureOwnerPermit, LiveCapturePermit, Runtime, new_id};
use gstreamer as gst;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};
use std::fs::{self, File};
use std::io::{self, BufRead, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::thread;
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as RandrExt;

#[path = "materials_media/mod.rs"]
#[allow(dead_code, unused_imports)]
mod materials_media;
#[path = "capture/publication.rs"]
mod publication;
#[path = "recovery.rs"]
mod recovery;
#[path = "retained.rs"]
mod retained;

const LIMIT: usize = 65_536;
const MAX_CAPTURE_RECORDS: usize = 4096;
const START_LIMIT: Duration = Duration::from_secs(10);

#[derive(Clone)]
struct Source {
    value: Value,
    x: i16,
    y: i16,
    width: u16,
    height: u16,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
struct Journal {
    capture_id: String,
    owner_token: String,
    request: Value,
    state: String,
    error: Option<String>,
    materials: Option<Value>,
    root: String,
    materials_id: Option<String>,
    lease_id: Option<String>,
    #[serde(default)]
    root_device: Option<u64>,
    #[serde(default)]
    root_inode: Option<u64>,
    #[serde(default)]
    release_phase: Option<String>,
    #[serde(default)]
    release_movie: Option<Value>,
    #[serde(default)]
    release_payload: Option<Value>,
    #[serde(default)]
    partial_movie: Option<Value>,
    #[serde(default)]
    payload_digest: Option<String>,
    #[serde(default)]
    admitted_monotonic_ns: Option<u64>,
    #[serde(default)]
    confirmed_at_ms: Option<u64>,
    #[serde(default)]
    cancel_requested: bool,
    #[serde(default)]
    publication: Option<publication::Journal>,
}

struct Shared {
    journal: Mutex<Journal>,
    directory: PathBuf,
    runtime: Runtime,
    stop: AtomicBool,
    cancel: AtomicBool,
    epoch: Instant,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn sha(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
fn monotonic_ns() -> io::Result<u64> {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    if unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) } != 0 {
        return Err(io::Error::last_os_error());
    }
    u64::try_from(ts.tv_sec)
        .ok()
        .and_then(|s| s.checked_mul(1_000_000_000))
        .and_then(|s| s.checked_add(ts.tv_nsec as u64))
        .ok_or_else(|| io::Error::other("monotonic clock overflow"))
}
fn duration_ms_ceil(value: Duration) -> u64 {
    ((value.as_nanos() + 999_999) / 1_000_000).min(9_007_199_254_740_991) as u64
}
fn hash_file(fd: i32) -> io::Result<String> {
    let mut digest = Sha256::new();
    let mut chunk = [0u8; 64 * 1024];
    let mut offset = 0i64;
    loop {
        // Read the already-created, still-open media inode without reopening a
        // caller-visible pathname or disturbing fdsink's file offset.
        let n = unsafe {
            libc::pread(
                fd,
                chunk.as_mut_ptr().cast(),
                chunk.len(),
                offset as libc::off_t,
            )
        };
        if n < 0 {
            return Err(io::Error::last_os_error());
        }
        if n == 0 {
            break;
        }
        let n = n as usize;
        digest.update(&chunk[..n]);
        offset += n as i64;
    }
    Ok(hex(&digest.finalize()))
}
fn error(code: &'static str) -> Value {
    json!({"code":code,"message":"Engine capture request failed"})
}
fn release_refusal(stage: &'static str) -> &'static str {
    if matches!(
        std::env::var("APPARENT_CAPTURE_TEST_RELEASE_DIAGNOSTICS").as_deref(),
        Ok("1")
    ) {
        eprintln!("capture release refused: {stage}");
    }
    "source_mismatch"
}
fn response(rpc: &str, result: Result<Value, &'static str>) -> Value {
    let mut value = json!({"channel":"capture","protocolVersion":2,"rpcId":rpc});
    match result {
        Ok(v) => value["result"] = v,
        Err(e) => value["error"] = error(e),
    }
    value
}
fn id(value: &Value) -> Option<&str> {
    let s = value.as_str()?;
    (s.len() == 32
        && s.bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()))
    .then_some(s)
}
fn path_id(id: &str) -> String {
    format!("{id}.json")
}
fn journal_count(directory: &Path) -> Result<usize, &'static str> {
    let mut count = 0;
    let root = pinned_dir(directory).map_err(|_| "source_mismatch")?;
    for entry in
        fs::read_dir(format!("/proc/self/fd/{}/", root.as_raw_fd())).map_err(|_| "internal")?
    {
        let entry = entry.map_err(|_| "internal")?;
        if entry
            .path()
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            count += 1;
            if count >= MAX_CAPTURE_RECORDS {
                return Ok(count);
            }
        }
    }
    Ok(count)
}
fn pinned_dir(path: &Path) -> io::Result<File> {
    apparent_capture_engine::open_directory(path)
}
fn persist(shared: &Shared) -> io::Result<()> {
    let j = shared
        .journal
        .lock()
        .map_err(|_| io::Error::other("capture journal poisoned"))?
        .clone();
    persist_journal(shared, &j)
}
fn persist_journal(shared: &Shared, journal: &Journal) -> io::Result<()> {
    let _transaction = shared.runtime.capture_transaction()?;
    let current = read_journal(&shared.directory, &journal.capture_id, &journal.owner_token)
        .map_err(|_| io::Error::other("capture journal changed"))?;
    let mut merged = journal.clone();
    let active = |state: &str| ["starting", "recording", "stopping"].contains(&state);
    let terminal = ["materials-ready", "cancelled", "failed"].contains(&current.state.as_str());
    if terminal && active(&merged.state) {
        merged = current.clone();
        if current.state == "materials-ready" && journal.cancel_requested {
            merged.state = "cancelled".into();
        }
    } else {
        merged.cancel_requested |= current.cancel_requested;
    }
    if current.state == "stopping" && merged.state == "recording" {
        merged.state = "stopping".into();
    }
    if merged.cancel_requested && merged.state == "materials-ready" {
        merged.state = "cancelled".into();
    }
    apparent_capture_engine::SecureStore::open(&shared.directory)?.replace_checked(
        &path_id(&merged.capture_id),
        &serde_json::to_vec(&merged).map_err(io::Error::other)?,
    )
}
fn persist_new(shared: &Shared) -> io::Result<()> {
    let _transaction = shared.runtime.capture_transaction()?;
    let j = shared
        .journal
        .lock()
        .map_err(|_| io::Error::other("capture journal poisoned"))?
        .clone();
    apparent_capture_engine::SecureStore::open(&shared.directory)?.write_new(
        &path_id(&j.capture_id),
        &serde_json::to_vec(&j).map_err(io::Error::other)?,
    )
}
fn read_journal(directory: &Path, capture: &str, owner: &str) -> Result<Journal, &'static str> {
    let authenticated = retained::read_authenticated_journal(directory, capture, owner)?;
    serde_json::from_slice(&authenticated.bytes).map_err(|_| "source_mismatch")
}
fn remove_unsealed(directory: &Path, journal: &Journal) -> Result<(), &'static str> {
    let expected = directory.join("materials").join(&journal.capture_id);
    let path = PathBuf::from(&journal.root);
    if path != expected
        || path.file_name().and_then(|leaf| leaf.to_str()) != Some(journal.capture_id.as_str())
    {
        return Err("source_mismatch");
    }
    let parent = Arc::new(
        apparent_capture_engine::SecureStore::open(path.parent().ok_or("internal")?)
            .map_err(|_| "source_mismatch")?,
    );
    let dir = match parent.existing_child(&journal.capture_id) {
        Ok(dir) => dir,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            // A completed removal may precede the durable terminal receipt.
            // Only the exact leaf may be absent; the pinned parent must survive.
            parent
                .directory_file()
                .map_err(|_| "source_mismatch")?
                .sync_all()
                .map_err(|_| "internal")?;
            return Ok(());
        }
        Err(_) => return Err("source_mismatch"),
    };
    let parent_file = parent.directory_file().map_err(|_| "source_mismatch")?;
    let leaf =
        std::ffi::CString::new(journal.capture_id.as_str()).map_err(|_| "source_mismatch")?;
    let meta = dir.directory_metadata().map_err(|_| "source_mismatch")?;
    if !meta.is_dir()
        || meta.file_type().is_symlink()
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o777 != 0o700
    {
        return Err("source_mismatch");
    }
    if journal.root_device.is_some_and(|x| x != meta.dev())
        || journal.root_inode.is_some_and(|x| x != meta.ino())
    {
        return Err("source_mismatch");
    }
    let mut found = Vec::new();
    for entry in
        fs::read_dir(format!("/proc/self/fd/{}/", dir.as_raw_fd())).map_err(|_| "internal")?
    {
        let entry = entry.map_err(|_| "internal")?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !["recording.mp4", "materials.json"].contains(&name.as_str()) {
            return Err("source_mismatch");
        }
        let file = dir.file(&name, false).map_err(|_| "source_mismatch")?;
        dir.validate(&file).map_err(|_| "source_mismatch")?;
        let m = file.metadata().map_err(|_| "source_mismatch")?;
        let identity = if name == "recording.mp4" {
            journal
                .partial_movie
                .as_ref()
                .or(journal.release_movie.as_ref())
        } else {
            journal.release_payload.as_ref()
        };
        let payload_matches = if name == "materials.json" && identity.is_none() {
            let bytes = dir
                .read(&name)
                .map_err(|_| "source_mismatch")?
                .ok_or("source_mismatch")?;
            journal
                .payload_digest
                .as_deref()
                .is_some_and(|expected| sha(&bytes) == expected)
        } else {
            false
        };
        if !m.is_file()
            || m.uid() != unsafe { libc::geteuid() }
            || m.mode() & 0o777 != 0o600
            || m.nlink() != 1
            || (!payload_matches
                && identity
                    .and_then(|v| v["device"].as_str())
                    .and_then(|v| v.parse::<u64>().ok())
                    != Some(m.dev()))
            || (!payload_matches
                && identity
                    .and_then(|v| v["inode"].as_str())
                    .and_then(|v| v.parse::<u64>().ok())
                    != Some(m.ino()))
        {
            return Err("source_mismatch");
        }
        found.push(name);
    }
    for name in found {
        let leaf = std::ffi::CString::new(name).map_err(|_| "source_mismatch")?;
        if unsafe { libc::unlinkat(dir.as_raw_fd(), leaf.as_ptr(), 0) } != 0 {
            return Err("internal");
        }
    }
    dir.directory_file()
        .map_err(|_| "internal")?
        .sync_all()
        .map_err(|_| "internal")?;
    // Unknown entries make unlinkat(AT_REMOVEDIR) fail; they are never traversed/deleted.
    if unsafe { libc::unlinkat(parent_file.as_raw_fd(), leaf.as_ptr(), libc::AT_REMOVEDIR) } != 0 {
        return Err("source_mismatch");
    }
    parent.check().map_err(|_| "source_mismatch")?;
    parent_file.sync_all().map_err(|_| "internal")
}
fn pause_test_cancel_after_unsealed_cleanup(journal: &Journal) {
    let Ok(target) = std::env::var("APPARENT_CAPTURE_TEST_CANCEL_CLEANUP_BUNDLE_ID") else {
        return;
    };
    if journal.request["bundleId"].as_str() == Some(target.as_str()) {
        thread::sleep(Duration::from_secs(5));
    }
}
fn operation_dir(shared: &Shared) -> Result<apparent_capture_engine::SecureStore, &'static str> {
    let j = shared.journal.lock().map_err(|_| "internal")?.clone();
    let dir = apparent_capture_engine::SecureStore::open(Path::new(&j.root))
        .map_err(|_| "source_mismatch")?;
    let meta = dir.directory_metadata().map_err(|_| "source_mismatch")?;
    if Some(meta.dev()) != j.root_device
        || Some(meta.ino()) != j.root_inode
        || meta.uid() != unsafe { libc::geteuid() }
        || meta.mode() & 0o777 != 0o700
    {
        return Err("source_mismatch");
    }
    Ok(dir)
}

fn available_sources() -> Result<(String, Vec<Source>), &'static str> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_none() {
        return Err("unavailable");
    }
    let (connection, screen_num) = x11rb::connect(None).map_err(|_| "unavailable")?;
    let screen = connection
        .setup()
        .roots
        .get(screen_num)
        .ok_or("unavailable")?;
    let resources = connection
        .randr_get_screen_resources_current(screen.root)
        .map_err(|_| "unavailable")?
        .reply()
        .map_err(|_| "unavailable")?;
    let generation = new_id().map_err(|_| "internal")?;
    let mut sources = Vec::new();
    for output in resources.outputs {
        let info = connection
            .randr_get_output_info(output, resources.config_timestamp)
            .map_err(|_| "unavailable")?
            .reply()
            .map_err(|_| "unavailable")?;
        if info.connection != x11rb::protocol::randr::Connection::CONNECTED || info.crtc == 0 {
            continue;
        }
        let crtc = connection
            .randr_get_crtc_info(info.crtc, resources.config_timestamp)
            .map_err(|_| "unavailable")?
            .reply()
            .map_err(|_| "unavailable")?;
        if crtc.width == 0 || crtc.height == 0 {
            continue;
        }
        let display_id = u64::from(output);
        // Keep integer one on the wire: JSON.stringify in the SDK canonicalizes
        // both 1 and 1.0 to 1, and source equality binds the exact descriptor.
        let output_name = String::from_utf8_lossy(&info.name).into_owned();
        let display_bounds =
            json!({"x":crtc.x,"y":crtc.y,"width":crtc.width,"height":crtc.height,"scale":1});
        sources.push(Source { value: json!({"id":format!("display:{display_id}"),"generation":generation,"kind":"display","displayId":display_id,"bounds":display_bounds,"name":output_name}), x:crtc.x, y:crtc.y, width:crtc.width, height:crtc.height });
        if crtc.width >= 2 && crtc.height >= 2 {
            let (x, y) = (
                i32::from(crtc.x) + i32::from(crtc.width / 4),
                i32::from(crtc.y) + i32::from(crtc.height / 4),
            );
            let (Ok(x), Ok(y)) = (i16::try_from(x), i16::try_from(y)) else {
                continue;
            };
            let (width, height) = (crtc.width / 2, crtc.height / 2);
            let bounds = json!({"x":x,"y":y,"width":width,"height":height,"scale":1});
            sources.push(Source { value: json!({"id":format!("area:{display_id}:{x}:{y}:{width}:{height}"),"generation":generation,"kind":"area","displayId":display_id,"bounds":bounds,"name":output_name}), x, y, width, height });
        }
    }
    if sources.is_empty() {
        return Err("unavailable");
    }
    Ok((generation, sources))
}

fn capabilities(ready: bool) -> Value {
    json!({"sourceDiscovery":ready,"displayCapture":ready,"areaCapture":ready,"audioCapture":false,"contextFacts":false,"interactionFacts":false,"candidates":false,"transcript":false,"strokes":false,"publication":true,"retainedMaterials":ready})
}
fn handshake(ready: bool) -> Value {
    json!({"channel":"capture","protocolVersion":2,"implementation":"native","readiness":if ready {"ready"} else {"unavailable"},"capabilities":capabilities(ready)})
}
fn native_ready(sources: &[Source]) -> bool {
    if std::env::var_os("APPARENT_CAPTURE_TEST_READINESS_UNAVAILABLE").is_some() {
        return false;
    }
    if sources.is_empty() || gst::init().is_err() {
        return false;
    }
    let base = [
        "ximagesrc",
        "videoconvert",
        "h264parse",
        "mp4mux",
        "fdsink",
        "decodebin",
    ];
    let encoder = ["x264enc", "openh264enc"]
        .iter()
        .any(|name| gst::ElementFactory::find(name).is_some());
    let decoder = ["avdec_h264", "openh264dec"]
        .iter()
        .any(|name| gst::ElementFactory::find(name).is_some());
    base.iter()
        .all(|name| gst::ElementFactory::find(name).is_some())
        && encoder
        && decoder
}
fn status(j: &Journal) -> Value {
    let mut v = json!({"captureId":j.capture_id,"state":j.state});
    if let Some(m) = &j.materials {
        v["materials"] = m.clone();
    }
    if let Some(code) = &j.error {
        v["error"] = json!({"code":code});
    }
    if let Some(publication) = &j.publication {
        publication::status_fields(&mut v, publication);
    }
    v
}

fn sync_control(shared: &Shared) -> Result<Journal, &'static str> {
    let prior = shared.journal.lock().map_err(|_| "internal")?.clone();
    let durable = read_journal(&shared.directory, &prior.capture_id, &prior.owner_token)?;
    if durable.cancel_requested {
        shared.cancel.store(true, Ordering::Release);
        shared.stop.store(true, Ordering::Release);
    } else if durable.state == "stopping" {
        shared.stop.store(true, Ordering::Release);
    }
    Ok(durable)
}

fn recording_start(
    source: &Source,
    movie: &File,
    shared: &Shared,
    epoch: Instant,
) -> Result<apparent_capture_engine::linux::Recorder, &'static str> {
    let native = apparent_capture_engine::linux::Source::X11 {
        rect: [
            f64::from(source.x),
            f64::from(source.y),
            f64::from(source.width),
            f64::from(source.height),
        ],
    };
    let recorder = apparent_capture_engine::linux::Recorder::new_with_file(
        &native,
        video_dimensions(source),
        (64, 64),
        movie,
        epoch,
    )
    .map_err(|_| "unavailable")?;
    let deadline = Instant::now() + START_LIMIT;
    while recorder.first_frame_ms().is_none() {
        let _ = sync_control(shared)?;
        if shared.cancel.load(Ordering::Acquire) {
            recorder.stop();
            return Err("cancelled");
        }
        if Instant::now() >= deadline {
            recorder.stop();
            return Err("deadline_exceeded");
        }
        if recorder.error().is_some() {
            recorder.stop();
            return Err("unavailable");
        }
        thread::sleep(Duration::from_millis(20));
    }
    Ok(recorder)
}

fn video_dimensions(source: &Source) -> (u32, u32) {
    apparent_capture_engine::capture_dimensions(
        [f64::from(source.width), f64::from(source.height), 1.0],
        f64::from(source.width.max(source.height)),
        1.0,
    )
}

fn finalize(
    shared: &Shared,
    movie: &File,
    source: &Source,
    duration_ms: u64,
    video_origin_ns: u64,
) -> Result<Value, &'static str> {
    let (materials_id, lease_id) = (
        new_id().map_err(|_| "internal")?,
        new_id().map_err(|_| "internal")?,
    );
    let movie_meta = movie.metadata().map_err(|_| "internal")?;
    if movie_meta.len() == 0 {
        return Err("invalid_materials");
    }
    movie.sync_all().map_err(|_| "internal")?;
    let timing = apparent_capture_engine::linux::media::validate_file(movie)
        .map_err(|_| "invalid_materials")?;
    let (video_duration_ms, video_pts_start_ms) = (timing.duration_ms, timing.first_pts_ns);
    let movie_digest = hash_file(movie.as_raw_fd()).map_err(|_| "invalid_materials")?;
    let duration = duration_ms;
    let movie_file = json!({"relativePath":"recording.mp4","identity":{"device":movie_meta.dev().to_string(),"inode":movie_meta.ino().to_string(),"size":movie_meta.len(),"modifiedNs":(movie_meta.mtime() as i128 * 1_000_000_000 + movie_meta.mtime_nsec() as i128).to_string()},"sha256":movie_digest});
    let j = shared.journal.lock().map_err(|_| "internal")?.clone();
    let root = PathBuf::from(&j.root);
    let confirmed = json!({"id":source.value["id"],"generation":source.value["generation"],"confirmedAtMs":j.confirmed_at_ms.unwrap_or(0),"kind":source.value["kind"],"displayId":source.value["displayId"],"rectangle":source.value["bounds"],"confirmation":{"kind":"explicit-display","id":j.request["invocation"]["invocationId"]}});
    // Start input timestamps use the caller's clock, while detached documents
    // use the native monotonic capture epoch. Bind the explicit invocation to
    // the admission boundary rather than copying an incomparable wall clock.
    let mut invocation = j.request["invocation"].clone();
    // Caller observedAtMs is wall-clock time; preserve invocation identity, but
    // report when the native worker admitted it on the capture monotonic epoch.
    invocation["observedAtMs"] = json!(0);
    let video_offset_ms = video_origin_ns
        .checked_sub(video_pts_start_ms)
        .map(|ns| ns.saturating_add(999_999) / 1_000_000)
        .ok_or("invalid_materials")?;
    let (video_width, video_height) = video_dimensions(source);
    let payload = json!({"schema":"apparent.capture-materials-payload/1","identity":{"captureId":j.capture_id,"materialsId":materials_id},"bundleId":j.request["bundleId"],"root":root,"leaseId":lease_id,"clock":{"epoch":"capture-admission-monotonic","units":"integer-milliseconds","durationMs":duration,"videoOffsetMs":video_offset_ms},"confirmedSource":confirmed,"invocation":invocation,"files":[movie_file],"video":{"file":"recording.mp4","width":video_width,"height":video_height,"durationMs":video_duration_ms},"facts":{},"collection":{"context":"unavailable","interactions":"unavailable","candidates":"unavailable","transcript":"unavailable","acceptedStrokes":"unavailable"},"warnings":[],"safety":{"nativeRemoteDelivery":"forbidden","safeForProcessing":true}});
    let payload_bytes = serde_json::to_vec(&payload).map_err(|_| "internal")?;
    {
        let mut j = shared.journal.lock().map_err(|_| "internal")?;
        j.payload_digest = Some(sha(&payload_bytes));
    }
    persist(shared).map_err(|_| "internal")?;
    let opdir = operation_dir(shared)?;
    opdir
        .write_new("materials.json", &payload_bytes)
        .map_err(|_| "internal")?;
    let payload_file = opdir
        .file("materials.json", false)
        .map_err(|_| "internal")?;
    let p = payload_file.metadata().map_err(|_| "internal")?;
    let revision = sha(&payload_bytes);
    let manifest = json!({"schema":"apparent.capture-materials/1","key":{"captureId":j.capture_id,"materialsId":materials_id,"revision":revision},"root":root,"leaseId":lease_id,"payloadFile":{"relativePath":"materials.json","identity":{"device":p.dev().to_string(),"inode":p.ino().to_string(),"size":p.len(),"modifiedNs":(p.mtime() as i128 * 1_000_000_000 + p.mtime_nsec() as i128).to_string()},"sha256":revision}});
    let mut j = shared.journal.lock().map_err(|_| "internal")?;
    j.release_movie = Some(movie_file["identity"].clone());
    j.release_payload = Some(manifest["payloadFile"]["identity"].clone());
    drop(j);
    persist(shared).map_err(|_| "internal")?;
    Ok(manifest)
}

fn worker(
    shared: Arc<Shared>,
    source: Source,
    permit: LiveCapturePermit,
    _owner: CaptureOwnerPermit,
) {
    let result = (|| -> Result<(), &'static str> {
        permit.check().map_err(|_| "internal")?;
        let capture_id = shared
            .journal
            .lock()
            .map_err(|_| "internal")?
            .capture_id
            .clone();
        let opdir = operation_dir(&shared)?;
        let movie = opdir
            .file("recording.mp4", true)
            .map_err(|_| "source_mismatch")?;
        let movie_identity = movie.metadata().map_err(|_| "internal")?;
        {
            let mut j = shared.journal.lock().map_err(|_| "internal")?;
            j.partial_movie = Some(
                json!({"device":movie_identity.dev().to_string(),"inode":movie_identity.ino().to_string()}),
            );
        }
        persist(&shared).map_err(|_| "internal")?;
        let recorder = recording_start(&source, &movie, &shared, shared.epoch)?;
        let video_origin_ns = recorder.video_origin_ns().ok_or("invalid_materials")?;
        {
            let mut j = shared.journal.lock().map_err(|_| "internal")?;
            j.state = "recording".into();
        }
        persist(&shared).map_err(|_| "internal")?;
        let mut source_check = Instant::now() + Duration::from_millis(250);
        while !shared.stop.load(Ordering::Acquire) && !shared.cancel.load(Ordering::Acquire) {
            let control = sync_control(&shared)?;
            if control.cancel_requested {
                break;
            }
            if recorder.error().is_some() {
                recorder.stop();
                return Err("unavailable");
            }
            thread::sleep(Duration::from_millis(30));
            if Instant::now() >= source_check {
                source_check = Instant::now() + Duration::from_millis(250);
                let current = available_sources().ok().and_then(|(_, values)| {
                    values.into_iter().find(|value| same_source(value, &source))
                });
                if current.is_none() {
                    recorder.stop();
                    drop(movie);
                    let j = shared.journal.lock().map_err(|_| "internal")?;
                    let _ = remove_unsealed(&shared.directory, &j);
                    return Err("source_changed");
                }
            }
        }
        if shared.cancel.load(Ordering::Acquire) {
            recorder.stop();
            drop(movie);
            let prior = shared.journal.lock().map_err(|_| "internal")?;
            remove_unsealed(&shared.directory, &prior)?;
            let cleanup_binding = prior.clone();
            drop(prior);
            pause_test_cancel_after_unsealed_cleanup(&cleanup_binding);
            let mut j = shared.journal.lock().map_err(|_| "internal")?;
            j.state = "cancelled".into();
            drop(j);
            persist(&shared).map_err(|_| "internal")?;
            return Ok(());
        }
        let capture_duration = shared
            .epoch
            .elapsed()
            .as_millis()
            .min(9_007_199_254_740_991) as u64;
        if !recorder.finish(|_| {}) {
            return Err("invalid_materials");
        }
        if let Ok(value) = std::env::var("APPARENT_CAPTURE_TEST_FINALIZATION_DELAY_MS") {
            if let Ok(ms) = value.parse::<u64>() {
                thread::sleep(Duration::from_millis(ms.min(5_000)));
            }
        }
        movie.sync_all().map_err(|_| "invalid_materials")?;
        let manifest = finalize(&shared, &movie, &source, capture_duration, video_origin_ns)?;
        let mut j = shared.journal.lock().map_err(|_| "internal")?;
        let mut ready = j.clone();
        ready.state = if ready.cancel_requested {
            "cancelled"
        } else {
            "materials-ready"
        }
        .into();
        ready.materials_id = Some(
            manifest["key"]["materialsId"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        );
        ready.lease_id = Some(manifest["leaseId"].as_str().unwrap_or_default().to_owned());
        ready.materials = Some(manifest);
        // Do not expose the terminal state to inspect until its recovery record
        // has reached disk. The journal mutex also excludes a concurrent cancel.
        persist_journal(&shared, &ready).map_err(|_| "internal")?;
        *j = read_journal(&shared.directory, &ready.capture_id, &ready.owner_token)?;
        drop(j);
        let _ = capture_id;
        Ok(())
    })();
    if let Err(code) = result {
        let durable_cancel = shared
            .journal
            .lock()
            .ok()
            .and_then(|j| read_journal(&shared.directory, &j.capture_id, &j.owner_token).ok())
            .is_some_and(|j| j.cancel_requested);
        let cleanup_failed = match shared.journal.lock() {
            Ok(j) if j.materials.is_none() => remove_unsealed(&shared.directory, &j).is_err(),
            Ok(_) => false,
            Err(_) => true,
        };
        if cleanup_failed {
            // Keep the durable active journal and cancellation intent so cold
            // inspection can retry after the original identity is restored.
            drop(permit);
            return;
        }
        if let Ok(mut j) = shared.journal.lock() {
            if code == "cancelled" || durable_cancel {
                j.state = "cancelled".into();
                j.error = None;
            } else {
                j.state = "failed".into();
                j.error = Some(code.into());
            }
        }
        let _ = persist(&shared);
    }
    drop(permit);
}

fn string<'a>(v: &'a Value, key: &str) -> Result<&'a str, &'static str> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or("malformed_request")
}
fn source_from(value: &Value, sources: &[Source]) -> Result<Source, &'static str> {
    sources
        .iter()
        .find(|s| s.value == *value)
        .cloned()
        .ok_or("source_changed")
}
fn validate_start(v: &Value) -> Result<(), &'static str> {
    let owner = string(v, "ownerToken")?;
    let capture = string(v, "captureId")?;
    if capture.len() != 32
        || !capture
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        || owner.len() != 64
        || !owner
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err("malformed_request");
    }
    let bundle = string(v, "bundleId")?;
    if bundle.is_empty()
        || bundle.len() > 255
        || bundle == "."
        || bundle == ".."
        || bundle
            .chars()
            .any(|c| c.is_control() || c == '/' || c == '\\')
    {
        return Err("malformed_request");
    }
    let bundle_path = string(v, "bundlePath")?;
    let path = Path::new(bundle_path);
    if !path.is_absolute()
        || path.to_str() != Some(bundle_path)
        || path.components().any(|c| {
            matches!(
                c,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err("malformed_request");
    }
    let options = v.get("options").ok_or("malformed_request")?;
    if options.get("audio").and_then(Value::as_str) != Some("none")
        || options.get("localOnly").and_then(Value::as_bool) != Some(true)
        || options.as_object().is_none_or(|o| o.len() != 2)
    {
        return Err("unavailable");
    }
    let invocation = v.get("invocation").ok_or("malformed_request")?;
    if invocation.get("kind").and_then(Value::as_str) != Some("explicit")
        || invocation.as_object().is_none_or(|o| o.len() != 3)
        || id(&invocation["invocationId"]).is_none()
        || invocation
            .get("observedAtMs")
            .and_then(Value::as_u64)
            .is_none()
    {
        return Err("unavailable");
    }
    Ok(())
}
fn request_binding(v: &Value) -> Value {
    json!({"captureId":v["captureId"],"ownerToken":v["ownerToken"],"bundleId":v["bundleId"],"bundlePath":v["bundlePath"],"source":v["source"],"options":v["options"],"invocation":v["invocation"]})
}
fn active_status(shared: &Shared) -> Result<Value, &'static str> {
    let prior = shared.journal.lock().map_err(|_| "internal")?.clone();
    let transaction = shared
        .runtime
        .capture_transaction()
        .map_err(|_| "internal")?;
    let durable = read_journal(&shared.directory, &prior.capture_id, &prior.owner_token)?;
    drop(transaction);
    // The worker can have admitted a native file identity in memory before
    // its next durable write. Inspection is read-only: replacing that working
    // state with this earlier disk snapshot can erase the recovery binding.
    Ok(status(&durable))
}

fn same_catalog(left: &[Source], right: &[Source]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter().all(|a| right.iter().any(|b| same_source(a, b)))
}
fn same_source(left: &Source, right: &Source) -> bool {
    let mut a = left.value.clone();
    let mut b = right.value.clone();
    a.as_object_mut().unwrap().remove("generation");
    b.as_object_mut().unwrap().remove("generation");
    a == b
}
fn refresh_catalog(generation: &mut String, sources: &mut Vec<Source>, ready: &mut bool) {
    match available_sources() {
        Ok((new_generation, new_sources)) => {
            if !same_catalog(sources, &new_sources) {
                *generation = new_generation;
                *sources = new_sources;
            }
            *ready = native_ready(sources);
        }
        Err(_) => {
            sources.clear();
            generation.clear();
            *ready = false;
        }
    }
}

fn process_request(
    req: Value,
    runtime: &Runtime,
    root: &Path,
    generation: &mut String,
    sources: &mut Vec<Source>,
    ready: &mut bool,
    active: &mut HashMap<String, (Arc<Shared>, thread::JoinHandle<()>)>,
    all: &mut HashMap<String, Arc<Shared>>,
    recovery_role: bool,
) -> Result<Value, &'static str> {
    let method = string(&req, "method")?;
    let capture = req.get("captureId").and_then(Value::as_str).unwrap_or("");
    let owner = req.get("ownerToken").and_then(Value::as_str).unwrap_or("");
    match method {
        "handshake" => {
            if !recovery_role {
                refresh_catalog(generation, sources, ready);
            }
            Ok(handshake(*ready && generation.len() == 32))
        }
        "publish" | "inspectPublication" => publication::handle(&req, runtime, root, all),
        "discoverSources" => {
            if recovery_role {
                return Ok(
                    json!({"state":"unavailable","sources":[],"reason":"source discovery is disabled for recovery peers"}),
                );
            }
            refresh_catalog(generation, sources, ready);
            Ok(if sources.is_empty() {
                json!({"state":"unavailable","sources":[],"reason":"X11 capture is unavailable"})
            } else {
                json!({"state":"available","sources":sources.iter().map(|s|s.value.clone()).collect::<Vec<_>>()})
            })
        }
        "start" => {
            let admitted_at = Instant::now();
            let admitted_ns = monotonic_ns().map_err(|_| "internal")?;
            validate_start(&req)?;
            if let Some(existing) = all.get(capture) {
                let mut j = existing.journal.lock().map_err(|_| "internal")?;
                if j.owner_token != owner {
                    return Err("permission_denied");
                }
                if j.request != request_binding(&req) {
                    return Err("identity_conflict");
                }
                if !active.contains_key(capture)
                    && ["starting", "recording", "stopping"].contains(&j.state.as_str())
                {
                    let Some(_owner) = runtime
                        .acquire_capture_owner(capture)
                        .map_err(|_| "internal")?
                    else {
                        drop(j);
                        return active_status(existing);
                    };
                    if j.materials.is_none() {
                        remove_unsealed(root, &j)?;
                    }
                    if j.cancel_requested {
                        j.state = "cancelled".into();
                        j.error = None;
                    } else {
                        j.state = "failed".into();
                        j.error = Some("unavailable".into());
                    }
                    drop(j);
                    persist(existing).map_err(|_| "internal")?;
                } else {
                    drop(j);
                }
                return Ok(active_status(existing)?);
            }
            let journal_store = match apparent_capture_engine::SecureStore::open(root) {
                Ok(store) => store,
                // The root path was already admitted and its ancestor chain is
                // still checked by Store. A changed private mode prevents the
                // durable journal write, preserving the existing `internal`
                // storage-failure response; path/replacement errors remain a
                // source mismatch.
                Err(error) if error.kind() == io::ErrorKind::Other => return Err("internal"),
                Err(_) => return Err("source_mismatch"),
            };
            match journal_store.file(&path_id(capture), false).map(|_| ()) {
                Ok(()) => {
                    let recovered = read_journal(root, capture, owner)?;
                    if recovered.request != request_binding(&req) {
                        return Err("identity_conflict");
                    }
                    let mut recovered = recovered;
                    if ["starting", "recording", "stopping"].contains(&recovered.state.as_str()) {
                        let Some(_owner) = runtime
                            .acquire_capture_owner(capture)
                            .map_err(|_| "internal")?
                        else {
                            let shared = Arc::new(Shared {
                                journal: Mutex::new(recovered),
                                directory: root.to_owned(),
                                runtime: runtime.clone(),
                                stop: AtomicBool::new(false),
                                cancel: AtomicBool::new(false),
                                epoch: Instant::now(),
                            });
                            all.insert(capture.into(), shared.clone());
                            return Ok(active_status(&shared)?);
                        };
                        if recovered.materials.is_none() {
                            remove_unsealed(root, &recovered)?;
                        }
                        if recovered.cancel_requested {
                            recovered.state = "cancelled".into();
                            recovered.error = None;
                        } else {
                            recovered.state = "failed".into();
                            recovered.error = Some("unavailable".into());
                        }
                        let shared = Arc::new(Shared {
                            journal: Mutex::new(recovered),
                            directory: root.to_owned(),
                            runtime: runtime.clone(),
                            stop: AtomicBool::new(false),
                            cancel: AtomicBool::new(false),
                            epoch: Instant::now(),
                        });
                        persist(&shared).map_err(|_| "internal")?;
                        all.insert(capture.into(), shared.clone());
                        return Ok(active_status(&shared)?);
                    }
                    let shared = Arc::new(Shared {
                        journal: Mutex::new(recovered),
                        directory: root.to_owned(),
                        runtime: runtime.clone(),
                        stop: AtomicBool::new(false),
                        cancel: AtomicBool::new(false),
                        epoch: Instant::now(),
                    });
                    persist(&shared).map_err(|_| "internal")?;
                    all.insert(capture.into(), shared.clone());
                    return Ok(active_status(&shared)?);
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => (),
                Err(_) => return Err("source_mismatch"),
            }
            let source = source_from(&req["source"], sources)?;
            if source.value["generation"] != *generation {
                return Err("source_changed");
            }
            if !*ready {
                return Err("unavailable");
            }
            let (_, current_sources) = available_sources().map_err(|_| "source_changed")?;
            let current = current_sources.iter().find(|candidate| {
                let mut a = candidate.value.clone();
                let mut b = source.value.clone();
                a.as_object_mut().unwrap().remove("generation");
                b.as_object_mut().unwrap().remove("generation");
                a == b
            });
            if current.is_none() {
                return Err("source_changed");
            }
            let confirmed_at_ms = duration_ms_ceil(admitted_at.elapsed());
            if journal_count(root)? >= MAX_CAPTURE_RECORDS {
                return Err("busy");
            }
            let op_root = root.join("materials").join(capture);
            let journal = Journal {
                capture_id: capture.into(),
                owner_token: owner.into(),
                request: request_binding(&req),
                state: "starting".into(),
                error: None,
                materials: None,
                root: op_root.to_string_lossy().into_owned(),
                materials_id: None,
                lease_id: None,
                root_device: None,
                root_inode: None,
                release_phase: None,
                release_movie: None,
                release_payload: None,
                partial_movie: None,
                payload_digest: None,
                admitted_monotonic_ns: Some(admitted_ns),
                confirmed_at_ms: Some(confirmed_at_ms),
                cancel_requested: false,
                publication: None,
            };
            let shared = Arc::new(Shared {
                journal: Mutex::new(journal),
                directory: root.to_owned(),
                runtime: runtime.clone(),
                stop: AtomicBool::new(false),
                cancel: AtomicBool::new(false),
                epoch: admitted_at,
            });
            // The exact start binding is durable before admission or native effects.
            let operation_owner = runtime
                .acquire_capture_owner(capture)
                .map_err(|_| "internal")?
                .ok_or("busy")?;
            match persist_new(&shared) {
                Ok(()) => (),
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    let prior = read_journal(root, capture, owner)?;
                    if prior.request != request_binding(&req) {
                        return Err("identity_conflict");
                    }
                    return Err("busy");
                }
                Err(_) => return Err("internal"),
            }
            apparent_capture_engine::SecureStore::create(&op_root)
                .and_then(|store| store.directory_file())
                .map_err(|_| {
                    if let Ok(mut j) = shared.journal.lock() {
                        j.state = "failed".into();
                        j.error = Some("source_mismatch".into());
                    }
                    let _ = persist(&shared);
                    "source_mismatch"
                })?;
            {
                let dir = pinned_dir(&op_root).map_err(|_| "source_mismatch")?;
                let m = dir.metadata().map_err(|_| "source_mismatch")?;
                let mut j = shared.journal.lock().map_err(|_| "internal")?;
                j.root_device = Some(m.dev());
                j.root_inode = Some(m.ino());
                j.state = "starting".into();
            }
            persist(&shared).map_err(|_| "internal")?;
            let permit = match runtime.acquire_live_capture().map_err(|_| "internal")? {
                Some(p) => p,
                None => {
                    let mut j = shared.journal.lock().map_err(|_| "internal")?;
                    j.state = "failed".into();
                    j.error = Some("busy".into());
                    drop(j);
                    persist(&shared).map_err(|_| "internal")?;
                    all.insert(capture.into(), shared.clone());
                    return Ok(active_status(&shared)?);
                }
            };
            all.insert(capture.into(), shared.clone());
            let worker_shared = shared.clone();
            let worker_source = source.clone();
            let join = thread::Builder::new()
                .name(format!("capture-{}", &capture[..8]))
                .spawn(move || worker(worker_shared, worker_source, permit, operation_owner))
                .map_err(|_| "internal")?;
            active.insert(capture.into(), (shared.clone(), join));
            Ok(active_status(&shared)?)
        }
        "inspect" | "stop" | "cancel" | "releaseMaterials" => {
            if capture.len() != 32 || owner.len() != 64 {
                return Err("malformed_request");
            }
            let shared = if let Some(x) = all.get(capture) {
                x.clone()
            } else {
                let mut j = read_journal(root, capture, owner).map_err(|code| {
                    if method == "releaseMaterials" && code == "source_mismatch" {
                        release_refusal("journal-read")
                    } else {
                        code
                    }
                })?;
                if ["starting", "recording", "stopping"].contains(&j.state.as_str()) {
                    if let Some(_owner) = runtime
                        .acquire_capture_owner(capture)
                        .map_err(|_| "internal")?
                    {
                        if j.materials.is_none() {
                            remove_unsealed(root, &j)?;
                        }
                        if j.cancel_requested {
                            j.state = "cancelled".into();
                            j.error = None;
                        } else {
                            j.state = "failed".into();
                            j.error = Some("unavailable".into());
                        }
                        let shared = Arc::new(Shared {
                            journal: Mutex::new(j),
                            directory: root.to_owned(),
                            runtime: runtime.clone(),
                            stop: AtomicBool::new(false),
                            cancel: AtomicBool::new(false),
                            epoch: Instant::now(),
                        });
                        persist(&shared).map_err(|_| "internal")?;
                        all.insert(capture.into(), shared.clone());
                        shared
                    } else {
                        let shared = Arc::new(Shared {
                            journal: Mutex::new(j),
                            directory: root.to_owned(),
                            runtime: runtime.clone(),
                            stop: AtomicBool::new(false),
                            cancel: AtomicBool::new(false),
                            epoch: Instant::now(),
                        });
                        all.insert(capture.into(), shared.clone());
                        shared
                    }
                } else {
                    let shared = Arc::new(Shared {
                        journal: Mutex::new(j),
                        directory: root.to_owned(),
                        runtime: runtime.clone(),
                        stop: AtomicBool::new(false),
                        cancel: AtomicBool::new(false),
                        epoch: Instant::now(),
                    });
                    all.insert(capture.into(), shared.clone());
                    shared
                }
            };
            let mut j = shared.journal.lock().map_err(|_| "internal")?;
            if j.owner_token != owner {
                return Err("permission_denied");
            }
            match method {
                "inspect" => {
                    drop(j);
                    active_status(&shared)
                }
                "stop" => {
                    if j.state == "starting" {
                        return Err("busy");
                    }
                    if j.state == "recording" {
                        j.state = "stopping".into();
                        drop(j);
                        persist(&shared).map_err(|_| "internal")?;
                        shared.stop.store(true, Ordering::Release);
                        Ok(active_status(&shared)?)
                    } else {
                        Ok(status(&j))
                    }
                }
                "cancel" => {
                    if ["starting", "recording", "stopping"].contains(&j.state.as_str()) {
                        j.cancel_requested = true;
                        let snapshot = j.clone();
                        persist_journal(&shared, &snapshot).map_err(|_| "internal")?;
                        shared.cancel.store(true, Ordering::Release);
                        shared.stop.store(true, Ordering::Release);
                        drop(j);
                        Ok(active_status(&shared)?)
                    } else if j.state == "materials-ready" {
                        drop(j);
                        publication::cancel(&shared, root)
                    } else {
                        Ok(status(&j))
                    }
                }
                "releaseMaterials" => {
                    if j.publication.as_ref().is_some_and(|publication| {
                        !["saved", "cancelled", "failed"].contains(&publication.state.as_str())
                    }) {
                        return Err("lease_busy");
                    }
                    if j.materials_id.as_deref() != req["materialsId"].as_str()
                        || j.lease_id.as_deref() != req["leaseId"].as_str()
                    {
                        return Err("lease_busy");
                    }
                    if j.release_phase.as_deref() == Some("acknowledged") {
                        return Ok(
                            json!({"released":true,"captureId":capture,"materialsId":j.materials_id}),
                        );
                    }
                    if !["materials-ready", "cancelled", "failed", "releasing"]
                        .contains(&j.state.as_str())
                    {
                        return Err("lease_busy");
                    }
                    let dir = PathBuf::from(&j.root);
                    let mid = j.materials_id.clone().ok_or("lease_busy")?;
                    if j.state == "materials-ready" {
                        j.state = "cancelled".into();
                        j.error = None;
                    }
                    j.release_phase = Some("pending".into());
                    drop(j);
                    persist(&shared).map_err(|_| "internal")?;
                    if let Ok(value) =
                        std::env::var("APPARENT_CAPTURE_TEST_RELEASE_DELAY_AFTER_INTENT_MS")
                    {
                        if let Ok(ms) = value.parse::<u64>() {
                            thread::sleep(Duration::from_millis(ms.min(5_000)));
                        }
                    }
                    let _lease = runtime
                        .material_lease(&mid, apparent_capture_engine::LeaseKind::Maintenance)
                        .map_err(|_| "internal")?
                        .ok_or("lease_busy")?;
                    let jnow = shared.journal.lock().map_err(|_| "internal")?.clone();
                    let pinned = match apparent_capture_engine::SecureStore::open(&dir) {
                        Ok(p) => p,
                        Err(e)
                            if e.kind() == io::ErrorKind::NotFound
                                && jnow.release_phase.as_deref() == Some("pending") =>
                        {
                            let mut j = shared.journal.lock().map_err(|_| "internal")?;
                            j.materials = None;
                            j.state = "cancelled".into();
                            j.error = None;
                            j.release_phase = Some("acknowledged".into());
                            drop(j);
                            persist(&shared).map_err(|_| "internal")?;
                            return Ok(
                                json!({"released":true,"captureId":capture,"materialsId":mid}),
                            );
                        }
                        Err(_) => return Err(release_refusal("root-open")),
                    };
                    let pm = pinned
                        .directory_metadata()
                        .map_err(|_| release_refusal("root-metadata"))?;
                    if Some(pm.dev()) != jnow.root_device
                        || Some(pm.ino()) != jnow.root_inode
                        || pm.uid() != unsafe { libc::geteuid() }
                        || pm.mode() & 0o777 != 0o700
                    {
                        return Err(release_refusal("root-identity"));
                    }
                    let expected = ["recording.mp4", "materials.json"];
                    let mut seen = Vec::new();
                    for entry in fs::read_dir(format!("/proc/self/fd/{}/", pinned.as_raw_fd()))
                        .map_err(|_| "internal")?
                    {
                        let e = entry.map_err(|_| "internal")?;
                        let name = e.file_name().to_string_lossy().into_owned();
                        if !expected.contains(&name.as_str()) {
                            return Err(release_refusal("unknown-entry"));
                        }
                        let f = pinned
                            .file(&name, false)
                            .map_err(|_| release_refusal("file-open"))?;
                        pinned
                            .validate(&f)
                            .map_err(|_| release_refusal("file-validation"))?;
                        let m = f.metadata().map_err(|_| release_refusal("file-metadata"))?;
                        let binding = if name == "recording.mp4" {
                            &jnow.release_movie
                        } else {
                            &jnow.release_payload
                        };
                        if !m.is_file() {
                            return Err(release_refusal("file-type"));
                        }
                        if m.uid() != unsafe { libc::geteuid() } || m.mode() & 0o777 != 0o600 {
                            return Err(release_refusal("file-owner-mode"));
                        }
                        if m.nlink() != 1 {
                            return Err(release_refusal("file-link-count"));
                        }
                        if binding.is_none() {
                            return Err(release_refusal("file-binding-missing"));
                        }
                        if binding
                            .as_ref()
                            .and_then(|v| v["device"].as_str())
                            .and_then(|x| x.parse::<u64>().ok())
                            != Some(m.dev())
                        {
                            return Err(release_refusal(if name == "recording.mp4" {
                                "movie-device"
                            } else {
                                "payload-device"
                            }));
                        }
                        if binding
                            .as_ref()
                            .and_then(|v| v["inode"].as_str())
                            .and_then(|x| x.parse::<u64>().ok())
                            != Some(m.ino())
                        {
                            return Err(release_refusal(if name == "recording.mp4" {
                                "movie-inode"
                            } else {
                                "payload-inode"
                            }));
                        }
                        if binding.as_ref().and_then(|v| v["size"].as_u64()) != Some(m.len()) {
                            return Err(release_refusal(if name == "recording.mp4" {
                                "movie-size"
                            } else {
                                "payload-size"
                            }));
                        }
                        seen.push(name);
                    }
                    // Pending release is durable authority for retrying partial exact-file cleanup.
                    if jnow.release_phase.as_deref() != Some("pending")
                        || jnow.release_movie.is_none()
                        || jnow.release_payload.is_none()
                    {
                        return Err(release_refusal("durable-binding-missing"));
                    }
                    for name in expected {
                        if seen.iter().any(|x| x == name) {
                            let c = std::ffi::CString::new(name).unwrap();
                            if unsafe { libc::unlinkat(pinned.as_raw_fd(), c.as_ptr(), 0) } != 0 {
                                return Err("internal");
                            }
                        }
                    }
                    pinned
                        .directory_file()
                        .map_err(|_| "internal")?
                        .sync_all()
                        .map_err(|_| "internal")?;
                    let parent = pinned_dir(dir.parent().ok_or("internal")?)
                        .map_err(|_| release_refusal("parent-open"))?;
                    let leaf = std::ffi::CString::new(capture)
                        .map_err(|_| release_refusal("directory-name"))?;
                    if unsafe {
                        libc::unlinkat(parent.as_raw_fd(), leaf.as_ptr(), libc::AT_REMOVEDIR)
                    } != 0
                    {
                        return Err(release_refusal("directory-unlink"));
                    }
                    parent.sync_all().map_err(|_| "internal")?;
                    let mut j = shared.journal.lock().map_err(|_| "internal")?;
                    j.materials = None;
                    j.state = "cancelled".into();
                    j.error = None;
                    j.release_phase = Some("acknowledged".into());
                    drop(j);
                    persist(&shared).map_err(|_| "internal")?;
                    Ok(json!({"released":true,"captureId":capture,"materialsId":mid}))
                }
                _ => Err("malformed_request"),
            }
        }
        _ => Err("malformed_request"),
    }
}

fn parse_line(bytes: &[u8]) -> Result<Value, &'static str> {
    if bytes.len() > LIMIT {
        return Err("malformed_request");
    }
    let v: Value = serde_json::from_slice(bytes).map_err(|_| "malformed_request")?;
    let o = v.as_object().ok_or("malformed_request")?;
    if o.get("channel").and_then(Value::as_str) != Some("capture") {
        return Err("unsupported_channel");
    }
    if o.get("protocolVersion").and_then(Value::as_u64) != Some(2) {
        return Err("unsupported_version");
    }
    if id(&v["clientId"]).is_none() || id(&v["rpcId"]).is_none() {
        return Err("malformed_request");
    }
    let method = string(&v, "method")?;
    let mut fields = vec!["channel", "protocolVersion", "clientId", "rpcId", "method"];
    match method {
        "handshake" | "discoverSources" => (),
        "start" => fields.extend([
            "ownerToken",
            "captureId",
            "bundleId",
            "bundlePath",
            "source",
            "options",
            "invocation",
        ]),
        "inspect" | "stop" | "cancel" => fields.extend(["ownerToken", "captureId"]),
        "releaseMaterials" => fields.extend(["ownerToken", "captureId", "materialsId", "leaseId"]),
        "publish" => fields.extend([
            "ownerToken",
            "captureId",
            "leaseId",
            "publicationId",
            "materials",
            "stagingRoot",
            "planFile",
            "planSha256",
        ]),
        "inspectPublication" => {
            fields.extend(["ownerToken", "captureId", "publicationId", "planSha256"])
        }
        _ => return Err("malformed_request"),
    }
    if o.len() != fields.len() || fields.iter().any(|f| !o.contains_key(*f)) {
        return Err("malformed_request");
    }
    Ok(v)
}

fn send(stdout: &mut std::io::StdoutLock<'_>, value: Value) -> io::Result<()> {
    let mut bytes = serde_json::to_vec(&value).map_err(io::Error::other)?;
    if bytes.len() > LIMIT {
        return Err(io::Error::other("capture response exceeds protocol limit"));
    }
    bytes.push(b'\n');
    stdout.write_all(&bytes)?;
    stdout.flush()
}

fn main() {
    let startup = std::time::Instant::now();
    let diagnose_startup = std::env::var_os("GST_DEBUG").is_some();
    if diagnose_startup {
        eprintln!("capture startup: argument validation started");
    }
    let args: Vec<String> = std::env::args().collect();
    let recovery_role =
        if args.len() == 9 && args[7] == "--recovery-peer-v1" && recovery::nonce(&args[8]) {
            Some(recovery::Role {
                peer_id: &args[8],
                channel: "capture/2",
            })
        } else {
            None
        };
    if !(args.len() == 7 || recovery_role.is_some())
        || args[1] != "--runtime-directory"
        || args[3] != "--capture-channel"
        || args[4] != "capture"
        || args[5] != "--capture-protocol-version"
        || args[6] != "2"
    {
        std::process::exit(2);
    }
    let runtime_path = PathBuf::from(&args[2]);
    if !runtime_path.is_absolute() {
        std::process::exit(2);
    }
    if diagnose_startup {
        eprintln!(
            "capture startup: opening runtime after {}ms",
            startup.elapsed().as_millis()
        );
    }
    let runtime = match Runtime::open(&runtime_path) {
        Ok(r) => r,
        Err(_) => std::process::exit(2),
    };
    if diagnose_startup {
        eprintln!(
            "capture startup: runtime opened after {}ms",
            startup.elapsed().as_millis()
        );
    }
    let root = runtime_path.join("capture-v2");
    if apparent_capture_engine::SecureStore::ensure(&root).is_err()
        || apparent_capture_engine::SecureStore::ensure(&root.join("materials")).is_err()
    {
        std::process::exit(2);
    }
    if let Some(role) = recovery_role {
        recovery::lifecycle(role, "ready");
    }
    if diagnose_startup && recovery_role.is_none() {
        eprintln!(
            "capture startup: secure store ready after {}ms",
            startup.elapsed().as_millis()
        );
        eprintln!("capture startup: probing X11 sources");
    }
    let (mut generation, mut sources) = if recovery_role.is_some() {
        (String::new(), Vec::new())
    } else {
        available_sources().unwrap_or_else(|_| (String::new(), Vec::new()))
    };
    if diagnose_startup && recovery_role.is_none() {
        eprintln!(
            "capture startup: X11 probe returned {} sources after {}ms",
            sources.len(),
            startup.elapsed().as_millis()
        );
        eprintln!("capture startup: checking native GStreamer readiness");
    }
    let mut ready = recovery_role.is_none() && native_ready(&sources);
    if diagnose_startup && recovery_role.is_none() {
        eprintln!(
            "capture startup: native readiness={} after {}ms",
            ready,
            startup.elapsed().as_millis()
        );
    }
    let inputs = std::sync::mpsc::sync_channel::<Option<Vec<u8>>>(8);
    let input_tx = inputs.0;
    let input_rx = inputs.1;
    thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut reader = stdin.lock();
        loop {
            let mut buf = Vec::new();
            match (&mut reader)
                .take((LIMIT + 2) as u64)
                .read_until(b'\n', &mut buf)
            {
                Ok(0) => {
                    let _ = input_tx.send(None);
                    break;
                }
                Ok(n) if buf.ends_with(b"\n") && n <= LIMIT + 1 => {
                    buf.pop();
                    if buf.last() == Some(&b'\r') {
                        buf.pop();
                    }
                    if buf.contains(&b'\r') || input_tx.send(Some(buf)).is_err() {
                        break;
                    }
                }
                _ => {
                    let _ = input_tx.send(Some(Vec::new()));
                    break;
                }
            }
        }
    });
    let mut stdout = std::io::stdout().lock();
    let mut active: HashMap<String, (Arc<Shared>, thread::JoinHandle<()>)> = HashMap::new();
    let mut all: HashMap<String, Arc<Shared>> = HashMap::new();
    let (publication_tx, publication_rx) =
        std::sync::mpsc::channel::<(String, Result<Value, &'static str>)>();
    let mut seen = VecDeque::new();
    let mut ids = std::collections::HashSet::new();
    loop {
        match input_rx.recv_timeout(Duration::from_millis(20)) {
            Ok(Some(line)) => {
                let fallback = serde_json::from_slice::<Value>(&line)
                    .ok()
                    .and_then(|v| v["rpcId"].as_str().map(str::to_owned))
                    .filter(|s| s.len() == 32)
                    .unwrap_or_default();
                let result = parse_line(&line);
                let mut deferred = false;
                let (rpc, output) = match result {
                    Err(code) => (fallback.clone(), Err(code)),
                    Ok(req) => {
                        let rpc = req["rpcId"].as_str().unwrap_or("").to_owned();
                        if !ids.insert(rpc.clone()) {
                            (rpc, Err("malformed_request"))
                        } else {
                            seen.push_back(rpc.clone());
                            if seen.len() > 4096 {
                                if let Some(old) = seen.pop_front() {
                                    ids.remove(&old);
                                }
                            }
                            let processed = if recovery_role.is_some() && req["method"] == "start" {
                                Err("unavailable")
                            } else if matches!(
                                req["method"].as_str(),
                                Some("publish" | "inspectPublication")
                            ) {
                                let capture = req["captureId"].as_str().unwrap_or_default();
                                let owner = req["ownerToken"].as_str().unwrap_or_default();
                                let publication_key = format!("publication:{capture}");
                                if req["method"] == "inspectPublication"
                                    && active.contains_key(&publication_key)
                                {
                                    publication::observe(&req, &root)
                                } else {
                                    let shared = if let Some(shared) = all.get(capture) {
                                        Ok(shared.clone())
                                    } else {
                                        read_journal(&root, capture, owner).map(|journal| {
                                            let shared = Arc::new(Shared {
                                                journal: Mutex::new(journal),
                                                directory: root.clone(),
                                                runtime: runtime.clone(),
                                                stop: AtomicBool::new(false),
                                                cancel: AtomicBool::new(false),
                                                epoch: Instant::now(),
                                            });
                                            all.insert(capture.to_owned(), shared.clone());
                                            shared
                                        })
                                    };
                                    match shared {
                                        Err(code) => Err(code),
                                        Ok(shared) => {
                                            if active.contains_key(&publication_key) {
                                                Err("busy")
                                            } else {
                                                let mut worker_all = HashMap::new();
                                                worker_all
                                                    .insert(capture.to_owned(), shared.clone());
                                                let request = req.clone();
                                                let runtime = runtime.clone();
                                                let root = root.clone();
                                                let sender = publication_tx.clone();
                                                let response_id = rpc.clone();
                                                match thread::Builder::new()
                                                    .name(format!(
                                                        "capture-publish-{}",
                                                        &capture[..8.min(capture.len())]
                                                    ))
                                                    .spawn(move || {
                                                        let result = publication::handle(
                                                            &request,
                                                            &runtime,
                                                            &root,
                                                            &mut worker_all,
                                                        );
                                                        let _ = sender.send((response_id, result));
                                                    }) {
                                                    Ok(worker) => {
                                                        active.insert(
                                                            publication_key,
                                                            (shared, worker),
                                                        );
                                                        deferred = true;
                                                        Ok(Value::Null)
                                                    }
                                                    Err(_) => Err("internal"),
                                                }
                                            }
                                        }
                                    }
                                }
                            } else {
                                process_request(
                                    req,
                                    &runtime,
                                    &root,
                                    &mut generation,
                                    &mut sources,
                                    &mut ready,
                                    &mut active,
                                    &mut all,
                                    recovery_role.is_some(),
                                )
                            };
                            (rpc, processed)
                        }
                    }
                };
                if !deferred
                    && !rpc.is_empty()
                    && send(&mut stdout, response(&rpc, output)).is_err()
                    && recovery_role.is_none()
                {
                    break;
                }
            }
            Ok(None) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                if let Some(role) = recovery_role {
                    recovery::lifecycle(role, "input-closed");
                    let _ = recovery::detach_outputs();
                } else {
                    for (shared, _) in active.values() {
                        if let Ok(j) = shared.journal.lock() {
                            if j.state == "recording" || j.state == "stopping" {
                                shared.stop.store(true, Ordering::Release);
                            } else if j.state == "starting" {
                                shared.cancel.store(true, Ordering::Release);
                            }
                        }
                    }
                }
                break;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => (),
        }
        loop {
            match publication_rx.try_recv() {
                Ok((rpc, result)) => {
                    if send(&mut stdout, response(&rpc, result)).is_err() && recovery_role.is_none()
                    {
                        break;
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
            }
        }
        let completed: Vec<String> = active
            .iter()
            .filter_map(|(id, (_, h))| h.is_finished().then_some(id.clone()))
            .collect();
        for id in completed {
            if let Some((shared, handle)) = active.remove(&id) {
                if handle.join().is_err() {
                    if let Ok(mut j) = shared.journal.lock() {
                        if id.starts_with("publication:") {
                            publication::worker_panicked(&mut j);
                        } else {
                            j.state = "failed".into();
                            j.error = Some("internal".into());
                        }
                    }
                    let _ = persist(&shared);
                }
            }
        }
    }
    for (_, (_, handle)) in active {
        let _ = handle.join();
    }
}

#[cfg(test)]
mod unsealed_cleanup_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn fixture() -> (PathBuf, PathBuf, PathBuf, Journal) {
        let capture_id = new_id().unwrap();
        let base = std::env::temp_dir().join(format!("apparent-cleanup-{capture_id}"));
        let capture_root = base.join("capture-v2");
        let materials_parent = capture_root.join("materials");
        let root = materials_parent.join(&capture_id);
        fs::create_dir(&base).unwrap();
        fs::set_permissions(&base, fs::Permissions::from_mode(0o700)).unwrap();
        fs::create_dir(&capture_root).unwrap();
        fs::set_permissions(&capture_root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::create_dir(&materials_parent).unwrap();
        fs::set_permissions(&materials_parent, fs::Permissions::from_mode(0o700)).unwrap();
        let store = apparent_capture_engine::SecureStore::create(&root).unwrap();
        let mut movie = store.file("recording.mp4", true).unwrap();
        movie.write_all(b"unsealed partial media").unwrap();
        movie.sync_all().unwrap();
        let file = movie.metadata().unwrap();
        let directory = store.directory_metadata().unwrap();
        let journal = Journal {
            capture_id: capture_id.clone(),
            owner_token: "a".repeat(64),
            request: json!({"bundleId":"cleanup-test.apparent"}),
            state: "recording".into(),
            error: None,
            materials: None,
            publication: None,
            root: root.to_string_lossy().into_owned(),
            materials_id: None,
            lease_id: None,
            root_device: Some(directory.dev()),
            root_inode: Some(directory.ino()),
            release_phase: None,
            release_movie: None,
            release_payload: None,
            partial_movie: Some(
                json!({"device":file.dev().to_string(),"inode":file.ino().to_string()}),
            ),
            payload_digest: None,
            admitted_monotonic_ns: None,
            confirmed_at_ms: None,
            cancel_requested: true,
        };
        (base, capture_root, root, journal)
    }

    #[test]
    fn status_read_cannot_erase_an_unpersisted_movie_binding_before_crash_recovery() {
        let (base, directory, root, mut journal) = fixture();
        journal.state = "starting".into();
        journal.cancel_requested = false;
        let mut durable = journal.clone();
        durable.partial_movie = None;
        let store = apparent_capture_engine::SecureStore::open(&directory).unwrap();
        store
            .write_new(
                &path_id(&journal.capture_id),
                &serde_json::to_vec(&durable).unwrap(),
            )
            .unwrap();
        let shared = Shared {
            journal: Mutex::new(journal),
            directory: directory.clone(),
            runtime: Runtime::open(&base).unwrap(),
            stop: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
            epoch: Instant::now(),
        };
        // Exact worker window: its file identity is in memory, but its next
        // persist has not happened. RPC inspection must not replace that state.
        assert_eq!(active_status(&shared).unwrap()["state"], "starting");
        persist(&shared).unwrap();
        shared.journal.lock().unwrap().state = "recording".into();
        persist(&shared).unwrap();
        let saved = read_journal(&directory, &durable.capture_id, &durable.owner_token).unwrap();
        let cleanup = remove_unsealed(&directory, &saved);
        if cleanup.is_err() {
            eprintln!("retained status-race fixture: {}", base.display());
        }
        assert_eq!(
            cleanup,
            Ok(()),
            "cold recovery must retain the admitted movie identity"
        );
        assert!(!root.exists());
        drop(shared);
        drop(store);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn missing_unsealed_leaf_is_idempotent_but_missing_parent_is_not() {
        let (base, capture_root, root, journal) = fixture();
        remove_unsealed(&capture_root, &journal).unwrap();
        assert!(!root.exists());
        remove_unsealed(&capture_root, &journal).unwrap();
        assert!(fs::remove_dir(root.parent().unwrap()).is_ok());
        assert_eq!(
            remove_unsealed(&capture_root, &journal),
            Err("source_mismatch")
        );
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn replaced_unsealed_root_or_file_is_never_removed() {
        let (base, capture_root, root, journal) = fixture();
        let original = root.with_file_name("original-root");
        fs::rename(&root, &original).unwrap();
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(
            remove_unsealed(&capture_root, &journal),
            Err("source_mismatch")
        );
        assert!(root.is_dir());
        assert!(original.join("recording.mp4").is_file());
        fs::remove_dir(&root).unwrap();
        std::os::unix::fs::symlink(&original, &root).unwrap();
        assert_eq!(
            remove_unsealed(&capture_root, &journal),
            Err("source_mismatch")
        );
        assert!(
            fs::symlink_metadata(&root)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        fs::remove_file(&root).unwrap();
        fs::rename(&original, &root).unwrap();

        let moved_file = root.with_file_name("recording-original.mp4");
        fs::rename(root.join("recording.mp4"), &moved_file).unwrap();
        apparent_capture_engine::SecureStore::open(&root)
            .unwrap()
            .write_new("recording.mp4", b"replacement partial media")
            .unwrap();
        assert_eq!(
            remove_unsealed(&capture_root, &journal),
            Err("source_mismatch")
        );
        assert_eq!(
            fs::read(root.join("recording.mp4")).unwrap(),
            b"replacement partial media"
        );
        assert!(moved_file.is_file());
        fs::remove_dir_all(base).unwrap();
    }
}

#[cfg(test)]
mod publication_capability_tests {
    #[test]
    fn publication_reconciliation_is_advertised_when_screen_capture_is_unavailable() {
        assert_eq!(super::capabilities(false)["publication"], true);
        assert_eq!(super::capabilities(false)["displayCapture"], false);
        assert_eq!(super::capabilities(false)["areaCapture"], false);
    }
}
