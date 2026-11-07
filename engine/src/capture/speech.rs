//! One bounded existing Parakeet child, consuming the sealed ORIGINAL aligned
//! WAV. No model discovery/download, detached inference authority or timing guess.
use super::*;
use std::os::fd::{FromRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::process::Child;

pub const PROFILE: &str = "parakeet-unified-en-0.6b-Q8_0";
const MODEL_BYTES: u64 = 731_357_568;
const MODEL_SHA: &str = "4b50b6dd862bf6e346929aaf4f5eaacec003bfa3f56462d6c874b41ef2f38795";
const MODEL_ENV: &str = "APPARENT_CAPTURE_SPEECH_MODEL_FILE_V1";
const PROFILE_ENV: &str = "APPARENT_CAPTURE_SPEECH_PROFILE_V1";
const MAX_WAV: u64 = 64 * 1024 * 1024;
const MAX_RESULT: usize = 4 * 1024 * 1024;
const JOB_MS: u64 = 25_000;
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    audio_bytes: u64,
    audio_sha256: String,
    duration_ms: u64,
    budget_ms: u64,
}
struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}
fn readonly(fd: RawFd) -> Result<File, &'static str> {
    File::open(format!("/proc/self/fd/{fd}")).map_err(|_| "source_mismatch")
}
/// Only an explicit host launch option selects the fixed locally provisioned
/// model. The source is opened no-follow; only its sealed expected-hash snapshot
/// may be consumed by native code. A present path is never readiness evidence.
pub fn configured() -> bool {
    std::env::var(PROFILE_ENV).as_deref() == Ok(PROFILE) && std::env::var_os(MODEL_ENV).is_some()
}
pub fn transcribe(
    shared: &Shared,
    audio: &audio::Completed,
    seal: &Value,
    duration: u64,
) -> Result<Value, &'static str> {
    let deadline = Instant::now() + Duration::from_millis(JOB_MS);
    if sync_control(shared)?.cancel_requested {
        return Err("unavailable");
    }
    if !configured() {
        return Err("unavailable");
    }
    let path = PathBuf::from(std::env::var_os(MODEL_ENV).ok_or("unavailable")?);
    if !path.is_absolute() {
        return Err("unavailable");
    }
    let model = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK)
        .open(path)
        .map_err(|_| "unavailable")?;
    if !model.metadata().map_err(|_| "unavailable")?.is_file() {
        return Err("unavailable");
    }
    let wav = readonly(audio.file.as_raw_fd())?;
    let m = wav.metadata().map_err(|_| "invalid_materials")?;
    if m.len() > MAX_WAV
        || m.len()
            != seal["identity"]["size"]
                .as_u64()
                .ok_or("invalid_materials")?
    {
        return Err("invalid_materials");
    }
    let input = Input {
        audio_bytes: m.len(),
        audio_sha256: seal["sha256"].as_str().ok_or("invalid_materials")?.into(),
        duration_ms: duration,
        budget_ms: deadline
            .saturating_duration_since(Instant::now())
            .as_millis()
            .min(JOB_MS as u128) as u64,
    };
    if sync_control(shared)?.cancel_requested || Instant::now() >= deadline || input.budget_ms == 0
    {
        return Err("unavailable");
    }
    let (child, _lifetime) = crate::execution::spawn_reexec(
        &["--capture-speech-job-v1"],
        &serde_json::to_vec(&input).map_err(|_| "internal")?,
        &[(wav.as_raw_fd(), 3), (model.as_raw_fd(), 4)],
        crate::execution::Profile::Speech,
        false,
    )?;
    let bytes = collect(OwnedChild(child), deadline, &shared.cancel, || {
        sync_control(shared).map(|j| j.cancel_requested)
    })?;
    let value: Value = serde_json::from_slice(&bytes).map_err(|_| "invalid_materials")?;
    validate_result(&value, duration)?;
    Ok(value)
}
fn collect(
    mut child: OwnedChild,
    deadline: Instant,
    cancelled: &AtomicBool,
    mut durable_cancel: impl FnMut() -> Result<bool, &'static str>,
) -> Result<Vec<u8>, &'static str> {
    let mut out = child.0.stdout.take().ok_or("internal")?;
    let fd = out.as_raw_fd();
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err("internal");
    }
    let mut bytes = Vec::new();
    let mut status = None;
    let mut eof = false;
    let mut next_control = Instant::now();
    loop {
        if cancelled.load(Ordering::Acquire) || Instant::now() >= deadline {
            return Err("unavailable");
        }
        if Instant::now() >= next_control {
            if durable_cancel()? || Instant::now() >= deadline {
                return Err("unavailable");
            }
            next_control = Instant::now() + Duration::from_millis(100);
        }
        let mut buf = [0u8; 8192];
        loop {
            match out.read(&mut buf) {
                Ok(0) => {
                    eof = true;
                    break;
                }
                Ok(n) => {
                    if bytes.len() + n > MAX_RESULT {
                        return Err("invalid_materials");
                    }
                    bytes.extend_from_slice(&buf[..n]);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return Err("unavailable"),
            }
        }
        if status.is_none() {
            status = child.0.try_wait().map_err(|_| "unavailable")?;
        }
        if eof && status.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    if durable_cancel()?
        || !status.unwrap().success()
        || Instant::now() >= deadline
        || cancelled.load(Ordering::Acquire)
    {
        return Err("unavailable");
    }
    Ok(bytes)
}

fn validate_result(value: &Value, duration: u64) -> Result<(), &'static str> {
    let object = value.as_object().ok_or("invalid_materials")?;
    if object.len() != 4
        || value["provider"] != "transcribe.cpp/parakeet"
        || value["model"] != PROFILE
    {
        return Err("invalid_materials");
    }
    let text = value["text"].as_str().ok_or("invalid_materials")?;
    let segments = value["segments"].as_array().ok_or("invalid_materials")?;
    if segments.len() > 65536
        || (!text.is_empty() && segments.is_empty())
        || (text.is_empty() && !segments.is_empty())
    {
        return Err("invalid_materials");
    }
    let mut prior = 0;
    for s in segments {
        let o = s.as_object().ok_or("invalid_materials")?;
        if o.len() != 3 || s["text"].as_str().is_none_or(|t| t.trim().is_empty()) {
            return Err("invalid_materials");
        }
        let start = s["startMs"].as_u64().ok_or("invalid_materials")?;
        let end = s["endMs"].as_u64().ok_or("invalid_materials")?;
        if start < prior || end < start || end > duration {
            return Err("invalid_materials");
        }
        prior = start;
    }
    Ok(())
}
/// Same-binary internal child. It has no Store/runtime/lease/publication access.
/// All returned text is untrusted inference until the owning parent validates
/// and seals it before the original immutable material revision.
pub fn child() -> Result<(), &'static str> {
    let mut lifetime = unsafe { File::from_raw_fd(5) };
    thread::spawn(move || {
        let mut b = [0u8; 1];
        let _ = lifetime.read(&mut b);
        unsafe { libc::_exit(1) }
    });
    let mut raw = Vec::new();
    io::stdin()
        .take(8193)
        .read_to_end(&mut raw)
        .map_err(|_| "malformed_request")?;
    if raw.len() > 8192 {
        return Err("malformed_request");
    }
    let input: Input = serde_json::from_slice(&raw).map_err(|_| "malformed_request")?;
    if input.audio_bytes > MAX_WAV || input.budget_ms == 0 || input.budget_ms > JOB_MS {
        return Err("malformed_request");
    }
    let deadline = Instant::now() + Duration::from_millis(input.budget_ms);
    let wav = unsafe { File::from_raw_fd(3) };
    let model = unsafe { File::from_raw_fd(4) };
    if wav.metadata().map_err(|_| "invalid_materials")?.len() != input.audio_bytes {
        return Err("invalid_materials");
    }
    let mut bytes = Vec::new();
    wav.take(MAX_WAV + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "invalid_materials")?;
    if bytes.len() as u64 != input.audio_bytes || sha(&bytes) != input.audio_sha256 {
        return Err("source_mismatch");
    }
    let samples =
        apparent_capture_engine::audio::decode_wav_16k(&bytes).map_err(|_| "invalid_materials")?;
    if samples.is_empty() {
        return Err("invalid_materials");
    }
    let snapshot = apparent_capture_engine::audio::model::ModelFile::verify(
        model,
        MODEL_BYTES,
        MODEL_SHA,
        deadline,
    )
    .map_err(|_| "unavailable")?;
    let mut recognizer =
        apparent_capture_engine::audio::speech::PinnedRecognizer::load(snapshot, deadline)
            .map_err(|_| "unavailable")?;
    let result = recognizer
        .run(&samples, deadline)
        .map_err(|_| "unavailable")?;
    if result.text.len() > MAX_RESULT
        || result.segments.len().saturating_add(result.words.len()) > 65536
    {
        return Err("invalid_materials");
    }
    let text = result.text.split_whitespace().collect::<Vec<_>>().join(" ");
    let segments: Vec<Value> = if text.is_empty() {
        vec![]
    } else {
        let rows: Vec<(i64, i64, &str)> =
            if result.segments.iter().any(|s| !s.text.trim().is_empty()) {
                result
                    .segments
                    .iter()
                    .filter(|s| !s.text.trim().is_empty())
                    .map(|s| (s.t0_ms, s.t1_ms, s.text.as_str()))
                    .collect()
            } else {
                result
                    .words
                    .iter()
                    .filter(|s| !s.text.trim().is_empty())
                    .map(|s| (s.t0_ms, s.t1_ms, s.text.as_str()))
                    .collect()
            };
        rows.into_iter()
            .map(|(start, end, text)| json!({"startMs":start,"endMs":end,"text":text.trim()}))
            .collect()
    };
    let value = json!({"provider":"transcribe.cpp/parakeet","model":PROFILE,"text":text,"segments":segments});
    validate_result(&value, input.duration_ms)?;
    let output = serde_json::to_vec(&value).map_err(|_| "internal")?;
    if output.len() > MAX_RESULT || Instant::now() >= deadline {
        return Err("unavailable");
    }
    io::stdout()
        .write_all(&output)
        .and_then(|_| io::stdout().flush())
        .map_err(|_| "internal")?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_is_distinct_from_unavailable_and_native_timing_is_never_clamped() {
        let empty =
            json!({"provider":"transcribe.cpp/parakeet","model":PROFILE,"text":"","segments":[]});
        assert!(validate_result(&empty, 100).is_ok());
        for (start, end) in [(-1, 5), (10, 5), (0, 101)] {
            assert!(validate_result(&json!({"provider":"transcribe.cpp/parakeet","model":PROFILE,"text":"x","segments":[{"startMs":start,"endMs":end,"text":"x"}]}),100).is_err())
        }
        assert!(validate_result(&json!({"provider":"transcribe.cpp/parakeet","model":PROFILE,"text":"x","segments":[]}),100).is_err());
    }
    #[test]
    fn actual_child_cancel_timeout_and_output_bound_are_reaped() {
        use std::process::{Command, Stdio};
        for cancelled in [false, true] {
            let process = Command::new("/bin/sleep")
                .arg("30")
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let pid = process.id();
            assert!(
                collect(
                    OwnedChild(process),
                    Instant::now() + Duration::from_millis(20),
                    &AtomicBool::new(cancelled),
                    || Ok(false)
                )
                .is_err()
            );
            let mut status = 0;
            assert_eq!(
                unsafe { libc::waitpid(pid as i32, &mut status, libc::WNOHANG) },
                -1
            );
            assert_eq!(
                io::Error::last_os_error().raw_os_error(),
                Some(libc::ECHILD)
            );
        }
        let process = Command::new("/usr/bin/head")
            .args(["-c", "5000000", "/dev/zero"])
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let pid = process.id();
        assert_eq!(
            collect(
                OwnedChild(process),
                Instant::now() + Duration::from_secs(5),
                &AtomicBool::new(false),
                || Ok(false)
            ),
            Err("invalid_materials")
        );
        let mut status = 0;
        assert_eq!(
            unsafe { libc::waitpid(pid as i32, &mut status, libc::WNOHANG) },
            -1
        );
        assert_eq!(
            io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
    }
    #[test]
    fn accepted_recovery_peer_cancel_is_observed_from_the_shared_journal_and_reaped() {
        use std::process::{Command, Stdio};
        let (base, directory, _, mut journal) = super::super::unsealed_cleanup_tests::fixture();
        journal.state = "stopping".into();
        journal.cancel_requested = false;
        let owner = Shared {
            journal: Mutex::new(journal.clone()),
            directory: directory.clone(),
            runtime: Runtime::open(&base).unwrap(),
            stop: AtomicBool::new(true),
            cancel: AtomicBool::new(false),
            epoch: Instant::now(),
        };
        persist_new(&owner).unwrap();
        let recovery = Arc::new(Shared {
            journal: Mutex::new(journal.clone()),
            directory: directory.clone(),
            runtime: Runtime::open(&base).unwrap(),
            stop: AtomicBool::new(false),
            cancel: AtomicBool::new(false),
            epoch: Instant::now(),
        });
        let process = Command::new("/bin/sleep")
            .arg("30")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let pid = process.id();
        let (sent, received) = std::sync::mpsc::sync_channel(1);
        let peer = recovery.clone();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                let mut all=HashMap::from([(journal.capture_id.clone(),peer.clone())]);
                let request=json!({"method":"cancel","captureId":journal.capture_id,"ownerToken":journal.owner_token});
                process_request(request,&peer.runtime,&directory,&mut String::new(),&mut vec![],&mut false,&mut HashMap::new(),&mut all,true).unwrap();
                assert!(peer.cancel.load(Ordering::Acquire));
                sent.send(()).unwrap();
            });
            let mut first = true;
            let outcome = collect(
                OwnedChild(process),
                Instant::now() + Duration::from_secs(5),
                &owner.cancel,
                || {
                    if first {
                        received.recv_timeout(Duration::from_secs(2)).unwrap();
                        first = false;
                        assert!(!owner.cancel.load(Ordering::Acquire));
                    }
                    sync_control(&owner).map(|j| j.cancel_requested)
                },
            );
            assert_eq!(outcome, Err("unavailable"));
        });
        assert!(owner.cancel.load(Ordering::Acquire));
        assert!(
            read_journal(&directory, &journal.capture_id, &journal.owner_token)
                .unwrap()
                .cancel_requested
        );
        let mut status = 0;
        assert_eq!(
            unsafe { libc::waitpid(pid as i32, &mut status, libc::WNOHANG) },
            -1
        );
        assert_eq!(
            io::Error::last_os_error().raw_os_error(),
            Some(libc::ECHILD)
        );
        remove_unsealed(&directory, &journal).unwrap();
        drop(owner);
        drop(recovery);
        fs::remove_dir_all(base).unwrap();
    }
    #[test]
    fn failed_or_late_durable_control_read_never_returns_a_result_and_reaps() {
        use std::process::{Command, Stdio};
        for late in [false, true] {
            let process = Command::new("/bin/sleep")
                .arg("30")
                .stdout(Stdio::piped())
                .spawn()
                .unwrap();
            let pid = process.id();
            let outcome = collect(
                OwnedChild(process),
                Instant::now() + Duration::from_millis(10),
                &AtomicBool::new(false),
                || {
                    if late {
                        std::thread::sleep(Duration::from_millis(20));
                        Ok(false)
                    } else {
                        Err("source_mismatch")
                    }
                },
            );
            assert!(outcome.is_err());
            let mut status = 0;
            assert_eq!(
                unsafe { libc::waitpid(pid as i32, &mut status, libc::WNOHANG) },
                -1
            );
            assert_eq!(
                io::Error::last_os_error().raw_os_error(),
                Some(libc::ECHILD)
            );
        }
    }
}
