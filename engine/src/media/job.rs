#[cfg(target_os = "linux")]
use super::linux as decoder;
#[cfg(target_os = "macos")]
use super::macos as decoder;
use super::{
    files::{self, Directory, Output, SourcePin},
    protocol::{Decode, Frame, Identity, Result},
};
use apparent_capture_engine::{BundleLease, LeaseKind, Runtime};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom, Write};
use std::os::fd::RawFd;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::{fs::MetadataExt, net::UnixStream, process::ExitStatusExt};
use std::path::Path;
use std::process::Child;
use std::time::Instant;

const DIAGNOSTICS_SWITCH: &str = "APPARENT_MEDIA_TEST_DIAGNOSTICS";

fn diagnostics_enabled() -> bool {
    std::env::var_os(DIAGNOSTICS_SWITCH).is_some_and(|value| value == "1")
}

/// Bounded static labels for the opt-in synthetic test. No paths, raw error text or file sink.
pub(crate) fn diagnostic_failure(stage: &'static str, code: &'static str, bus_code: Option<i32>) {
    if !diagnostics_enabled() {
        return;
    }
    let record = match bus_code {
        Some(bus_code) => format!(
            "apparent-media-diagnostic stage={stage} code={code} bus=glib-error bus-code={bus_code}\n"
        ),
        None => {
            format!("apparent-media-diagnostic stage={stage} code={code} bus=not-in-error-chain\n")
        }
    };
    write_diagnostic(&record);
}

fn write_diagnostic(record: &str) {
    if diagnostics_enabled() && record.len() <= 256 {
        let _ = std::io::stderr().lock().write_all(record.as_bytes());
    }
}

fn diagnostic_worker_exit(child: &mut Child) {
    if !diagnostics_enabled() {
        return;
    }
    let record = match child.try_wait() {
        Ok(Some(status)) => format!(
            "apparent-media-diagnostic stage=worker-exit exit-code={} signal={}\n",
            status
                .code()
                .map_or_else(|| "none".to_owned(), |code| code.to_string()),
            status
                .signal()
                .map_or_else(|| "none".to_owned(), |signal| signal.to_string()),
        ),
        _ => "apparent-media-diagnostic stage=worker-exit status=unavailable\n".to_owned(),
    };
    write_diagnostic(&record);
}

pub fn decode_available() -> bool {
    #[cfg(target_os = "linux")]
    {
        true
    }
    #[cfg(target_os = "macos")]
    {
        super::macos::available()
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Internal {
    decode: Decode,
    runtime: String,
    reserved: Vec<Identity>,
}
pub struct Active {
    pub child: Child,
    pub started: Instant,
    pub input: Decode,
    pub rpc: String,
    pub cancelled: bool,
    source: SourcePin,
    output: Output,
    _lease: BundleLease,
    _lifetime: UnixStream,
}

/// Shared media child re-exec boundary. Media/1 retains its descriptor ABI;
/// retained media/2 uses the same parent lifetime and resource limits.
pub(crate) fn spawn_reexec(
    args: &[&str],
    input: &[u8],
    inherited_fds: &[(RawFd, RawFd)],
) -> Result<(Child, UnixStream)> {
    crate::execution::spawn_reexec(
        args,
        input,
        inherited_fds,
        crate::execution::Profile::Media,
        diagnostics_enabled(),
    )
}

impl Active {
    pub fn start(
        input: Decode,
        rpc: String,
        runtime_path: &Path,
        runtime: &Runtime,
    ) -> Result<Self> {
        let lease = runtime
            .lease(&input.source.bundle_id, LeaseKind::Read)
            .map_err(|_| "source_mismatch")?
            .ok_or("busy")?;
        let source = SourcePin::open(&input)?;
        let output = Output::create(&input)?;
        let result = Self::spawn(input, rpc, runtime_path, source, output, lease);
        result
    }
    fn spawn(
        input: Decode,
        rpc: String,
        runtime_path: &Path,
        source: SourcePin,
        output: Output,
        lease: BundleLease,
    ) -> Result<Self> {
        let setup = (|| {
            let internal = Internal {
                decode: input.clone(),
                runtime: runtime_path.to_str().ok_or("internal")?.into(),
                reserved: output
                    .files
                    .iter()
                    .map(|f| f.metadata().map(|m| files::identity(&m)))
                    .collect::<std::io::Result<_>>()
                    .map_err(|_| "invalid_output")?,
            };
            let bytes = serde_json::to_vec(&internal).map_err(|_| "internal")?;
            spawn_reexec(
                &["--decode-pinned-job"],
                &bytes,
                &[
                    (source.video.as_raw_fd(), 3),
                    (output.dir.file.as_raw_fd(), 4),
                ],
            )
        })();
        match setup {
            Ok((child, lifetime)) => Ok(Self {
                child,
                started: Instant::now(),
                input,
                rpc,
                cancelled: false,
                source,
                output,
                _lease: lease,
                _lifetime: lifetime,
            }),
            Err(e) => {
                output.cleanup();
                Err(e)
            }
        }
    }
    pub fn stop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
    pub fn finish(&mut self, success: bool) -> Result<Value> {
        if !success {
            diagnostic_worker_exit(&mut self.child);
            diagnostic_failure("supervisor-finish", "decode_failed", None);
            self.output.cleanup();
            return Err("decode_failed");
        }
        let result = (|| {
            let mut bytes = Vec::new();
            let mut stdout = self.child.stdout.take().ok_or("internal")?;
            stdout
                .by_ref()
                .take(65537)
                .read_to_end(&mut bytes)
                .map_err(|_| "internal")?;
            if bytes.len() > 65536 {
                return Err("internal");
            }
            let raw: Value = serde_json::from_slice(&bytes).map_err(|_| {
                diagnostic_failure("worker-response", "decode_failed", None);
                "decode_failed"
            })?;
            if let Some(code) = raw.get("error").and_then(Value::as_str) {
                let diagnostic_code = match code {
                    "limits_exceeded" => "limits_exceeded",
                    "source_incomplete" => "source_incomplete",
                    "invalid_output" => "invalid_output",
                    _ => "decode_failed",
                };
                diagnostic_failure("worker-result", diagnostic_code, None);
                return Err(match code {
                    "limits_exceeded" => "limits_exceeded",
                    "source_incomplete" => "source_incomplete",
                    _ => "decode_failed",
                });
            }
            let frames: Vec<Frame> = serde_json::from_value(raw).map_err(|_| "decode_failed")?;
            if frames.len() != self.input.requested_bundle_ms.len() {
                return Err("decode_failed");
            }
            self.source.check()?;
            let mut pixels = 0u64;
            for (i, frame) in frames.iter().enumerate() {
                let name = format!("frame-{:03}.png", i + 1);
                let mut f = self.output.dir.read(&name).map_err(|_| "invalid_output")?;
                let actual = f.metadata().map_err(|_| "invalid_output")?;
                let original = self.output.files[i]
                    .metadata()
                    .map_err(|_| "invalid_output")?;
                if !files::same(&original, &actual)
                    || actual.len() > 256 * 1024 * 1024
                    || frame.byte_length != actual.len()
                    || frame.request_index != i + 1
                    || frame.requested_bundle_ms != self.input.requested_bundle_ms[i]
                    || frame.effective_video_ms
                        != frame
                            .requested_bundle_ms
                            .saturating_sub(self.input.source.video_offset_ms)
                    || frame.clamped_to_start
                        != (frame.requested_bundle_ms < self.input.source.video_offset_ms)
                    || frame.path
                        != Path::new(&self.input.output_directory)
                            .join(&name)
                            .to_string_lossy()
                    || frame.width == 0
                    || frame.height == 0
                    || frame.width.max(frame.height) > self.input.limits.max_edge
                {
                    return Err("invalid_output");
                }
                pixels += u64::from(frame.width) * u64::from(frame.height);
                if pixels > self.input.limits.max_total_pixels {
                    return Err("limits_exceeded");
                }
                validate_png(&mut f, frame)?;
                f.sync_all().map_err(|_| "invalid_output")?;
            }
            self.source.check()?;
            let result = json!({"jobId":self.input.job_id,"state":"complete","frames":frames});
            self.output.publish(&result)?;
            Ok(result)
        })();
        if let Err(code) = &result {
            diagnostic_failure("supervisor-finish", *code, None);
            self.output.cleanup();
        }
        result
    }
    pub fn cleanup(&self) {
        self.output.cleanup();
    }
}
impl Drop for Active {
    fn drop(&mut self) {
        self.stop();
    }
}

fn validate_png(f: &mut File, frame: &Frame) -> Result<()> {
    let mut hash = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf).map_err(|_| "invalid_output")?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    if format!("{:x}", hash.finalize()) != frame.sha256 {
        return Err("invalid_output");
    }
    f.seek(SeekFrom::Start(0)).map_err(|_| "invalid_output")?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(f));
    decoder.set_limits(png::Limits {
        bytes: 256 * 1024 * 1024,
    });
    let mut reader = decoder.read_info().map_err(|_| "invalid_output")?;
    if reader.info().width != frame.width || reader.info().height != frame.height {
        return Err("invalid_output");
    }
    let mut data = vec![0; reader.output_buffer_size().ok_or("invalid_output")?];
    reader.next_frame(&mut data).map_err(|_| "invalid_output")?;
    Ok(())
}

pub fn child() -> Result<()> {
    // A private inherited pipe provides a portable parent-death fallback; Linux
    // additionally binds SIGKILL in pre_exec before the decoder can start.
    let mut life = unsafe { File::from_raw_fd(5) };
    std::thread::spawn(move || {
        let mut b = [0];
        let _ = life.read(&mut b);
        std::process::exit(1);
    });
    let internal: Internal =
        serde_json::from_reader(std::io::stdin().take(65537)).map_err(|_| "malformed_request")?;
    internal.decode.validate()?;
    let runtime = Runtime::open(Path::new(&internal.runtime)).map_err(|_| "source_mismatch")?;
    let _lease = runtime
        .lease(&internal.decode.source.bundle_id, LeaseKind::Read)
        .map_err(|_| "source_mismatch")?
        .ok_or("busy")?;
    let source = unsafe { File::from_raw_fd(3) };
    if files::identity(&source.metadata().map_err(|_| "source_mismatch")?)
        != internal.decode.source.identity
    {
        return Err("source_mismatch");
    }
    let dir = Directory {
        file: unsafe { File::from_raw_fd(4) },
        path: Path::new(&internal.decode.output_directory).to_owned(),
    };
    let decoded = decode_child(&internal, &source, &dir);
    let value = match decoded {
        Ok(frames) => json!(frames),
        Err(e) => json!({"error":e}),
    };
    serde_json::to_writer(std::io::stdout(), &value).map_err(|_| "internal")?;
    Ok(())
}
fn decode_child(internal: &Internal, source: &File, dir: &Directory) -> Result<Vec<Frame>> {
    let d = &internal.decode;
    let times: Vec<u64> = d
        .requested_bundle_ms
        .iter()
        .map(|t| t.saturating_sub(d.source.video_offset_ms))
        .collect();
    let mut frames = Vec::new();
    decoder::decode(
        source,
        &times,
        d.limits.max_edge,
        d.limits.max_total_pixels,
        |frame| {
            let i = frames.len();
            let name = format!("frame-{:03}.png", i + 1);
            let c = std::ffi::CString::new(name.as_str()).map_err(|_| "invalid_output")?;
            let fd = unsafe {
                libc::openat(
                    dir.file.as_raw_fd(),
                    c.as_ptr(),
                    libc::O_WRONLY | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK,
                )
            };
            if fd < 0 {
                return Err("invalid_output");
            }
            let mut file = unsafe { File::from_raw_fd(fd) };
            let m = file.metadata().map_err(|_| "invalid_output")?;
            if !m.is_file()
                || m.uid() != unsafe { libc::geteuid() }
                || m.mode() & 0o777 != 0o600
                || m.nlink() != 1
                || internal.reserved.get(i) != Some(&files::identity(&m))
            {
                return Err("invalid_output");
            }
            {
                let mut encoder = png::Encoder::new(&mut file, frame.width, frame.height);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                let mut writer = encoder.write_header().map_err(|_| {
                    diagnostic_failure("png-header", "decode_failed", None);
                    "decode_failed"
                })?;
                writer.write_image_data(&frame.rgba).map_err(|_| {
                    diagnostic_failure("png-image-write", "decode_failed", None);
                    "decode_failed"
                })?;
                writer.finish().map_err(|_| {
                    diagnostic_failure("png-finish", "decode_failed", None);
                    "decode_failed"
                })?;
            }
            file.sync_all().map_err(|_| "invalid_output")?;
            let mut read = dir.read(&name).map_err(|_| "invalid_output")?;
            let mut hasher = Sha256::new();
            let mut buf = [0u8; 65536];
            loop {
                let n = read.read(&mut buf).map_err(|_| "invalid_output")?;
                if n == 0 {
                    break;
                }
                hasher.update(&buf[..n]);
            }
            frames.push(Frame {
                request_index: i + 1,
                requested_bundle_ms: d.requested_bundle_ms[i],
                effective_video_ms: times[i],
                actual_video_ms: frame.actual_ms,
                clamped_to_start: d.requested_bundle_ms[i] < d.source.video_offset_ms,
                clamped_to_end: frame.clamped_to_end,
                path: Path::new(&d.output_directory)
                    .join(name)
                    .to_string_lossy()
                    .into_owned(),
                width: frame.width,
                height: frame.height,
                byte_length: file.metadata().map_err(|_| "invalid_output")?.len(),
                sha256: format!("{:x}", hasher.finalize()),
            });
            Ok(())
        },
    )?;
    Ok(frames)
}
