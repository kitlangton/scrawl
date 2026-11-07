//! ScreenCaptureKit recording execution shared by the strict native entry and legacy adapter.
//! The legacy `start` path below retains its existing fallback, filter and timestamp behavior.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use screencapturekit::cm::{CMSampleBufferExt, CMSampleBufferSCExt, SCFrameStatus};
use screencapturekit::prelude::*;
use screencapturekit::recording_output::{
    RecordingCallbacks, SCRecordingOutput, SCRecordingOutputCodec, SCRecordingOutputConfiguration,
    SCRecordingOutputFileType,
};
use screencapturekit::screenshot_manager::{CGImageExt, SCScreenshotManager};

use super::{
    CaptureRequest, FilterRequest, FrameConfiguration, RgbaImage, StreamConfiguration,
    build_filter, build_filter_and_config, stream_configuration,
};

/// Owns the stream and recording output on the legacy sampler worker.
pub struct CaptureStream {
    stream: Option<SCStream>,
    recording_output: Option<SCRecordingOutput>,
    video_path: Option<PathBuf>,
    filter: Option<SCContentFilter>,
    first_sample: Option<Arc<Mutex<Option<FirstSample>>>>,
    recording_state: Option<Arc<RecordingState>>,
}

/// The SCK sample's native media timestamp and monotonic callback-observation clock.
/// These remain separate; no conversion to a capture or encoded-video clock is claimed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FirstSample {
    pub presentation_value: i64,
    pub presentation_timescale: i32,
    pub presentation_flags: u32,
    pub presentation_epoch: i64,
    pub callback_observed_since_epoch_ns: u128,
    pub pixel_width: usize,
    pub pixel_height: usize,
    pub pixel_bytes_per_row: usize,
    pub pixel_data_size: usize,
}

#[derive(Default)]
struct RecordingState {
    started: AtomicBool,
    finished: AtomicBool,
    failed: AtomicBool,
}

#[derive(Clone, Copy)]
pub(super) struct PixelBufferEvidence {
    pub(super) width: usize,
    pub(super) height: usize,
    pub(super) bytes_per_row: usize,
    pub(super) data_size: usize,
    pub(super) mapped: bool,
}

fn inspect_pixel_buffer(
    buffer: &screencapturekit::cv::CVPixelBuffer,
) -> Option<PixelBufferEvidence> {
    if buffer.width() == 0 || buffer.height() == 0 || buffer.data_size() == 0 {
        return None;
    }
    let lock = buffer.lock_read_only().ok()?;
    let (width, height, bytes_per_row, mapped) = if buffer.is_planar() {
        if buffer.plane_count() == 0 {
            return None;
        }
        (
            buffer.width_of_plane(0),
            buffer.height_of_plane(0),
            buffer.bytes_per_row_of_plane(0),
            lock.base_address_of_plane(0)
                .is_some_and(|address| !address.is_null()),
        )
    } else {
        (
            buffer.width(),
            buffer.height(),
            buffer.bytes_per_row(),
            !lock.base_address().is_null(),
        )
    };
    (width > 0 && height > 0 && bytes_per_row > 0 && mapped).then_some(PixelBufferEvidence {
        width,
        height,
        bytes_per_row,
        data_size: buffer.data_size(),
        mapped,
    })
}

pub(super) fn has_usable_capture_sample(
    status: Option<SCFrameStatus>,
    data_ready: bool,
    sample_count: i64,
    pixel_buffer: Option<PixelBufferEvidence>,
) -> bool {
    status == Some(SCFrameStatus::Complete)
        && data_ready
        && sample_count > 0
        && pixel_buffer.is_some_and(|evidence| {
            evidence.width > 0
                && evidence.height > 0
                && evidence.bytes_per_row > 0
                && evidence.data_size > 0
                && evidence.mapped
        })
}

impl CaptureStream {
    /// Builds the initial filter before creating the stream. A missing/stale target yields a
    /// session without a filter; a shareable-content query failure remains distinguishable for
    /// the existing caller's permission-aware log.
    pub fn start(
        request: &CaptureRequest,
        video_output_path: Option<PathBuf>,
        started_at: Instant,
        video_offset_ms: &AtomicU64,
    ) -> Result<Self, String> {
        let Some((filter, config)) = build_filter_and_config(request)? else {
            return Ok(Self::empty());
        };

        if let Some(video_path) = video_output_path {
            if let Ok(rec_cfg_builder) = SCRecordingOutputConfiguration::new()
                && let Ok(rec_cfg) = rec_cfg_builder.with_output_url(&video_path).map(|c| {
                    c.with_video_codec(SCRecordingOutputCodec::H264)
                        .with_output_file_type(SCRecordingOutputFileType::MP4)
                })
                && let Some(rec_output) = SCRecordingOutput::new(&rec_cfg)
                && let Ok(stream) = SCStream::new(&filter, &config)
                && stream.add_recording_output(&rec_output).is_ok()
            {
                // Keep the donor's midpoint estimate around start_capture; it is not a first
                // frame or applied-filter acknowledgement.
                let before = started_at.elapsed();
                if stream.start_capture().is_ok() {
                    let after = started_at.elapsed();
                    video_offset_ms
                        .store(((before + after) / 2).as_millis() as u64, Ordering::Release);
                    return Ok(Self {
                        stream: Some(stream),
                        recording_output: Some(rec_output),
                        video_path: Some(video_path),
                        filter: Some(filter),
                        first_sample: None,
                        recording_state: None,
                    });
                }
            }
        }

        // The legacy sampler still takes candidate images when movie setup fails.
        Ok(Self {
            stream: None,
            recording_output: None,
            video_path: None,
            filter: Some(filter),
            first_sample: None,
            recording_state: None,
        })
    }

    fn empty() -> Self {
        Self {
            stream: None,
            recording_output: None,
            video_path: None,
            filter: None,
            first_sample: None,
            recording_state: None,
        }
    }

    /// Starts a strict display recording for the standalone capture provider.
    /// Unlike the legacy sampler, a missing display or failed native output is an error.
    /// This returns after `start_capture` succeeds, before any sample is guaranteed. The
    /// caller must observe `first_sample()` before treating capture as ready.
    /// `observation_epoch` is caller-supplied and may precede source/filter setup. Elapsed
    /// observation time intentionally includes that setup; this Core adapter cannot prove
    /// the epoch is a durable capture admission or map it to encoded movie timestamps.
    /// The output URL is only a native writer argument, not an authenticated file reference.
    /// Use `SckOutputGuard` with an absent leaf in a private Store, observe and pin its file
    /// during capture, then verify it after the successful finish callback before readback.
    /// This does not establish caller authorization, capture/2 readiness or publication.
    pub fn start_explicit_display(
        display_id: u32,
        configuration: StreamConfiguration,
        video_output_path: PathBuf,
        observation_epoch: Instant,
    ) -> Result<Self, String> {
        let content = SCShareableContent::get().map_err(|error| format!("{error:?}"))?;
        let display = content
            .displays()
            .into_iter()
            .find(|display| display.display_id() == display_id)
            .ok_or_else(|| "requested display is no longer shareable".to_owned())?;
        let filter = SCContentFilter::create()
            .with_display(&display)
            .with_excluding_windows(&[])
            .build()
            .map_err(|error| format!("{error:?}"))?;
        let stream_config = stream_configuration(configuration);
        let first_sample = Arc::new(Mutex::new(None));
        let callback_sample = first_sample.clone();
        let recording_state = Arc::new(RecordingState::default());
        let recording_started = recording_state.clone();
        let recording_failed = recording_state.clone();
        let recording_finished = recording_state.clone();
        let delegate = RecordingCallbacks::new()
            .on_start(move || recording_started.started.store(true, Ordering::Release))
            .on_fail(move |_error| {
                // Only explicit, ignored native tests exercise this strict entry here.
                // Keep raw OS diagnostics out of production capture logs and receipts.
                #[cfg(test)]
                eprintln!("SCK_RECORDING_FAILURE={_error:?}");
                recording_failed.failed.store(true, Ordering::Release);
            })
            .on_finish(move || recording_finished.finished.store(true, Ordering::Release));
        let rec_config = SCRecordingOutputConfiguration::new()
            .map_err(|error| format!("{error:?}"))?
            .with_output_url(&video_output_path)
            .map_err(|error| format!("{error:?}"))?
            .with_video_codec(SCRecordingOutputCodec::H264)
            .with_output_file_type(SCRecordingOutputFileType::MP4);
        let output = SCRecordingOutput::new_with_delegate(&rec_config, delegate)
            .ok_or_else(|| "ScreenCaptureKit recording output unavailable".to_owned())?;
        let mut stream =
            SCStream::new(&filter, &stream_config).map_err(|error| format!("{error:?}"))?;
        stream
            .add_output_handler(
                move |sample: screencapturekit::cm::CMSampleBuffer, kind: SCStreamOutputType| {
                    let pixels = sample
                        .pixel_buffer()
                        .as_ref()
                        .and_then(inspect_pixel_buffer);
                    if kind == SCStreamOutputType::Screen
                        && has_usable_capture_sample(
                            sample.frame_status(),
                            sample.is_data_ready(),
                            sample.num_samples(),
                            pixels,
                        )
                    {
                        let pts = sample.presentation_timestamp();
                        if pts.is_numeric()
                            && let Ok(mut first) = callback_sample.lock()
                            && first.is_none()
                            && let Some(pixel) = pixels
                        {
                            *first = Some(FirstSample {
                                presentation_value: pts.value,
                                presentation_timescale: pts.timescale,
                                presentation_flags: pts.flags,
                                presentation_epoch: pts.epoch,
                                callback_observed_since_epoch_ns: observation_epoch
                                    .elapsed()
                                    .as_nanos(),
                                pixel_width: pixel.width,
                                pixel_height: pixel.height,
                                pixel_bytes_per_row: pixel.bytes_per_row,
                                pixel_data_size: pixel.data_size,
                            });
                        }
                    }
                },
                SCStreamOutputType::Screen,
            )
            .map_err(|error| format!("{error:?}"))?;
        stream
            .add_recording_output(&output)
            .map_err(|error| format!("{error:?}"))?;
        stream
            .start_capture()
            .map_err(|error| format!("{error:?}"))?;
        Ok(Self {
            stream: Some(stream),
            recording_output: Some(output),
            video_path: Some(video_output_path),
            filter: Some(filter),
            first_sample: Some(first_sample),
            recording_state: Some(recording_state),
        })
    }

    /// First complete screen sample with a mapped, nonempty pixel buffer. Its PTS is on the
    /// native media clock; callback observation is measured separately by `Instant`. No clock
    /// conversion is inferred. `None` means no usable timestamped image sample arrived.
    pub fn first_sample(&self) -> Option<FirstSample> {
        self.first_sample
            .as_ref()
            .and_then(|sample| sample.lock().ok().and_then(|sample| *sample))
    }

    /// A `true` result means the recording output delegate finished successfully; `false`
    /// includes an output failure or a stop that did not produce a finish callback.
    pub fn recording_finished(&self) -> bool {
        self.recording_state.as_ref().is_some_and(|state| {
            state.finished.load(Ordering::Acquire) && !state.failed.load(Ordering::Acquire)
        })
    }

    pub fn recording_started(&self) -> bool {
        self.recording_state
            .as_ref()
            .is_some_and(|state| state.started.load(Ordering::Acquire))
    }

    pub fn recording_failed(&self) -> bool {
        self.recording_state
            .as_ref()
            .is_some_and(|state| state.failed.load(Ordering::Acquire))
    }

    /// An empty session used by the legacy caller to preserve its previous start-failure loop.
    pub fn unavailable() -> Self {
        Self::empty()
    }

    pub fn has_filter(&self) -> bool {
        self.filter.is_some()
    }

    pub fn video_path(&self) -> Option<PathBuf> {
        self.video_path.clone()
    }

    pub fn frame_configuration(&self, configuration: StreamConfiguration) -> FrameConfiguration {
        FrameConfiguration(stream_configuration(configuration))
    }

    /// Rebuilds the filter and updates the recording stream when one exists. A stream update
    /// error is returned separately from the new filter: the donor still used that new filter
    /// for candidate screenshots after a movie update error.
    pub fn update_filter(
        &mut self,
        request: &FilterRequest,
    ) -> Result<Option<Result<(), String>>, String> {
        let Some(new_filter) = build_filter(request)? else {
            return Ok(None);
        };
        let update = self.stream.as_ref().map(|stream| {
            stream
                .update_content_filter(&new_filter)
                .map_err(|error| format!("{error:?}"))
        });
        self.filter = Some(new_filter);
        Ok(Some(update.unwrap_or(Ok(()))))
    }

    /// Acquires an RGBA candidate from the current native filter. Timestamp policy stays with
    /// the caller; this method only returns validated dimensions and pixel bytes.
    pub fn capture_frame(&self, configuration: &FrameConfiguration) -> Result<RgbaImage, String> {
        #[cfg(debug_assertions)]
        if std::env::var("APPARENT_TEST_DROP_SCK_FRAMES").as_deref() == Ok("1") {
            return Err("test-injected screenshot failure".into());
        }
        let filter = self.filter.as_ref().ok_or("no ScreenCaptureKit filter")?;
        let image = SCScreenshotManager::capture_image(filter, &configuration.0)
            .map_err(|error| format!("{error:?}"))?;
        let width = u32::try_from(image.width()).map_err(|error| error.to_string())?;
        let height = u32::try_from(image.height()).map_err(|error| error.to_string())?;
        if width == 0 || height == 0 {
            return Err("screenshot has zero dimensions".into());
        }
        let bytes = image.rgba_data().map_err(|error| format!("{error:?}"))?;
        if bytes.len() != (width as usize) * (height as usize) * 4 {
            return Err("screenshot has invalid pixel length".into());
        }
        Ok(RgbaImage {
            width,
            height,
            bytes,
        })
    }

    /// Preserves the existing explicit stream stop/output removal order and ignored teardown
    /// errors. This runs on the owning sampler thread before it returns the movie path.
    pub fn stop(&mut self) {
        if let Some(stream) = self.stream.take() {
            let _ = stream.stop_capture();
            if let Some(ref output) = self.recording_output {
                let _ = stream.remove_recording_output(output);
            }
        }
        self.recording_output = None;
    }
}

impl Drop for CaptureStream {
    fn drop(&mut self) {
        self.stop();
    }
}
