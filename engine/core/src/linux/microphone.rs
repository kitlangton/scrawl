//! Existing selected Pulse/PipeWire-Pulse input execution.
//! Caller owns device selection, consent, admission clock, persistence and error policy.
//! First PCM proves input activity, not hardware timestamps or durable recording.

use anyhow::{Context, Result};
use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app as gst_app;
use std::sync::Arc;

/// A chosen Pulse source feeding 16 kHz mono PCM into the same recording buffer as CPAL.
/// The pipeline and callbacks stay alive on the owning audio worker until this is dropped.
#[must_use]
pub struct Stream(gst::Pipeline);

/// The native buffer PTS mapped through its active pipeline clock, not callback arrival.
/// This is GStreamer source timing, not a hardware microphone timestamp guarantee.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PcmTiming {
    pub running_pts_ns: u64,
    pub capture_elapsed_ns: u64,
}

fn mapped_timing(elapsed: u128, clock: u64, base: u64, pts: Option<u64>) -> Option<PcmTiming> {
    let running_pts_ns = pts?;
    let capture_elapsed_ns =
        super::recorder::capture_elapsed_at_gst_pts(elapsed, clock, base, running_pts_ns)?;
    Some(PcmTiming {
        running_pts_ns,
        capture_elapsed_ns,
    })
}

/// Explicit local server selection prevents libpulse default-server discovery and
/// autospawn fallback. Never start a daemon, select a network server or edit settings.
fn local_pulse_server(runtime: &std::path::Path) -> Result<String> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    anyhow::ensure!(
        runtime.is_absolute() && runtime.canonicalize()? == runtime,
        "Invalid Pulse runtime"
    );
    for directory in [runtime.to_path_buf(), runtime.join("pulse")] {
        let metadata = std::fs::symlink_metadata(directory)?;
        anyhow::ensure!(
            metadata.is_dir()
                && metadata.uid() == unsafe { libc::geteuid() }
                && metadata.mode() & 0o077 == 0,
            "Untrusted Pulse directory"
        );
    }
    let socket = runtime.join("pulse/native");
    let metadata = std::fs::symlink_metadata(&socket)?;
    anyhow::ensure!(
        metadata.file_type().is_socket() && metadata.uid() == unsafe { libc::geteuid() },
        "No owned local Pulse server"
    );
    Ok(format!(
        "unix:{}",
        socket.to_str().context("Invalid Pulse socket path")?
    ))
}

impl Stream {
    pub fn start(name: &str, on_samples: impl Fn(&[f32]) + Send + Sync + 'static) -> Result<Self> {
        Self::start_inner(Some(name), None, None, move |samples, _| {
            on_samples(samples)
        })
    }

    /// Timestamped input for a caller retaining the SAME admission Instant.
    /// Missing/invalid native timing fails rather than inventing an audio offset.
    pub fn start_with_clock(
        name: &str,
        admission: std::time::Instant,
        on_samples: impl Fn(&[f32], PcmTiming) + Send + Sync + 'static,
    ) -> Result<Self> {
        Self::start_inner(Some(name), Some(admission), None, move |samples, timing| {
            // start_inner refuses a timed buffer without a valid mapping.
            if let Some(timing) = timing {
                on_samples(samples, timing);
            }
        })
    }

    /// The Pulse server's current default input, without changing its default.
    /// Device selection/settings remain caller policy; no fallback is attempted.
    pub fn start_default_with_clock(
        admission: std::time::Instant,
        on_samples: impl Fn(&[f32], PcmTiming) + Send + Sync + 'static,
    ) -> Result<Self> {
        let server = local_pulse_server(
            &std::env::var_os("XDG_RUNTIME_DIR")
                .map(std::path::PathBuf::from)
                .context("No owned local Pulse runtime")?,
        )?;
        Self::start_inner(
            None,
            Some(admission),
            Some(&server),
            move |samples, timing| {
                if let Some(timing) = timing {
                    on_samples(samples, timing);
                }
            },
        )
    }

    /// Callers must observe failures after initial readiness as well.
    pub fn error(&self) -> Option<&'static str> {
        self.0
            .bus()?
            .pop_filtered(&[gst::MessageType::Error, gst::MessageType::Eos])
            .map(|_| "microphone_input_stopped")
    }

    /// Stop native input before draining the file writer. A failed transition
    /// is not permission to seal a successful microphone recording.
    pub fn stop(&self) -> Result<()> {
        self.0
            .set_state(gst::State::Null)
            .context("Microphone input did not stop")?;
        Ok(())
    }

    fn start_inner(
        name: Option<&str>,
        admission: Option<std::time::Instant>,
        server: Option<&str>,
        on_samples: impl Fn(&[f32], Option<PcmTiming>) + Send + Sync + 'static,
    ) -> Result<Self> {
        gst::init()?;
        // Only the constant part is parsed. Never interpolate a device name into a pipeline.
        let pipeline = gst::parse::launch("audioconvert name=convert ! audioresample ! audio/x-raw,format=F32LE,rate=16000,channels=1 ! appsink name=input max-buffers=4 drop=true sync=false")?
            .downcast::<gst::Pipeline>().map_err(|_| anyhow::anyhow!("Not an audio pipeline"))?;
        let mut builder = gst::ElementFactory::make("pulsesrc");
        if let Some(server) = server {
            builder = builder.property("server", server);
        }
        if let Some(name) = name {
            builder = builder.property("device", name);
        }
        let source = builder.build().context("Pulse source is unavailable")?;
        pipeline.add(&source)?;
        source.link(&pipeline.by_name("convert").context("No audio converter")?)?;
        let sink = pipeline
            .by_name("input")
            .context("No audio sink")?
            .downcast::<gst_app::AppSink>()
            .map_err(|_| anyhow::anyhow!("Not an audio sink"))?;
        let on_samples = Arc::new(on_samples);
        let (ready, first_sample) = std::sync::mpsc::sync_channel(1);
        let weak_pipeline = pipeline.downgrade();
        let timing_anchor = std::sync::Mutex::new(None::<(gst::Clock, gst::ClockTime)>);
        sink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                    if admission.is_some() && map.len() % 4 != 0 {
                        return Err(gst::FlowError::Error);
                    }
                    let pcm: Vec<f32> = map
                        .as_slice()
                        .chunks_exact(4)
                        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
                        .collect();
                    let timing = if let Some(admission) = admission {
                        let pipeline = weak_pipeline.upgrade().ok_or(gst::FlowError::Error)?;
                        let clock = pipeline.clock().ok_or(gst::FlowError::Error)?;
                        let base = pipeline.base_time().ok_or(gst::FlowError::Error)?;
                        let mut anchor = timing_anchor.lock().map_err(|_| gst::FlowError::Error)?;
                        if let Some((initial_clock, initial_base)) = anchor.as_ref() {
                            if initial_clock != &clock || *initial_base != base {
                                return Err(gst::FlowError::Error);
                            }
                        } else {
                            *anchor = Some((clock.clone(), base));
                        }
                        let pts = buffer.pts().ok_or(gst::FlowError::Error)?;
                        let segment = sample
                            .segment()
                            .and_then(|segment| segment.downcast_ref::<gst::ClockTime>())
                            .ok_or(gst::FlowError::Error)?;
                        let running = segment.to_running_time(pts).ok_or(gst::FlowError::Error)?;
                        Some(
                            mapped_timing(
                                admission.elapsed().as_nanos(),
                                clock.time().nseconds(),
                                base.nseconds(),
                                Some(running.nseconds()),
                            )
                            .ok_or(gst::FlowError::Error)?,
                        )
                    } else {
                        None
                    };
                    on_samples(&pcm, timing);
                    let _ = ready.try_send(());
                    Ok(gst::FlowSuccess::Ok)
                })
                .build(),
        );
        let stream = Self(pipeline);
        stream.0.set_state(gst::State::Playing)?;
        // GStreamer can accept Playing before Pulse reports a missing or unopenable source.
        // Do not claim the selected input works until it has delivered actual PCM.
        first_sample
            .recv_timeout(std::time::Duration::from_millis(1500))
            .context("Pulse source did not deliver audio")?;
        Ok(stream)
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let _ = self.0.set_state(gst::State::Null);
    }
}

#[cfg(test)]
mod clock_tests {
    use super::*;
    #[test]
    fn source_pts_mapping_excludes_callback_delay_and_preserves_origin() {
        let timing = mapped_timing(2_000_000, 12_000_000, 10_000_000, Some(500_000)).unwrap();
        assert_eq!(timing.capture_elapsed_ns, 500_000);
        assert_eq!(timing.running_pts_ns, 500_000);
    }
    #[test]
    fn missing_pre_admission_and_overflow_timing_are_not_offsets() {
        assert_eq!(mapped_timing(1, 0, 0, None), None);
        assert_eq!(mapped_timing(0, 10, 0, Some(1)), None);
        assert_eq!(mapped_timing(0, 0, u64::MAX, Some(1)), None);
        assert_eq!(mapped_timing(u128::MAX, 0, 0, Some(0)), None);
    }
}

#[cfg(test)]
mod local_server_tests {
    use super::*;
    use std::os::unix::{
        fs::{PermissionsExt, symlink},
        net::UnixListener,
    };
    #[test]
    fn local_server_requires_existing_owned_private_socket_without_fallback() {
        let root = std::env::temp_dir().join(format!("pulse-local-{}", crate::new_id().unwrap()));
        std::fs::create_dir(&root).unwrap();
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
        let pulse = root.join("pulse");
        std::fs::create_dir(&pulse).unwrap();
        std::fs::set_permissions(&pulse, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(local_pulse_server(&root).is_err());
        std::fs::write(pulse.join("native"), b"not a socket").unwrap();
        assert!(local_pulse_server(&root).is_err());
        std::fs::remove_file(pulse.join("native")).unwrap();
        let socket = UnixListener::bind(pulse.join("native")).unwrap();
        assert_eq!(
            local_pulse_server(&root).unwrap(),
            format!("unix:{}", pulse.join("native").display())
        );
        std::fs::set_permissions(&pulse, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(local_pulse_server(&root).is_err());
        std::fs::set_permissions(&pulse, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::rename(pulse.join("native"), pulse.join("held")).unwrap();
        symlink(pulse.join("held"), pulse.join("native")).unwrap();
        assert!(local_pulse_server(&root).is_err());
        drop(socket);
        std::fs::remove_dir_all(root).unwrap();
    }
}
