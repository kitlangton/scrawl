//! ScreenCaptureKit recording execution shared by the strict native entry and legacy adapter.
//! The legacy `start` path below retains its existing fallback, filter and timestamp behavior.

use std::path::PathBuf;
#[cfg(test)]
use std::slice;
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

use super::clock::{
    CaptureAdmissionTime, CaptureClockSample, ClockError, map_sample_to_capture_clock,
};
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
    clock_capture: Option<ClockCapture>,
    #[cfg(test)]
    test_samples: Option<Arc<Mutex<TestSampleCollection>>>,
    checked_stop: Option<(StopOperation, StopOperation)>,
}

/// Result of one native teardown operation. `NotAttempted` is never successful evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StopOperation {
    NotAttempted,
    Succeeded,
    Failed(String),
}

/// Current status of the recording-output delegate. A missing finish callback remains pending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordingFinalization {
    NotApplicable,
    Pending,
    Finished,
    Failed,
}

/// Checked result for the existing stop-then-remove native sequence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckedStopOutcome {
    pub stop_capture: StopOperation,
    pub remove_recording_output: StopOperation,
    pub recording_finalization: RecordingFinalization,
}

impl CheckedStopOutcome {
    /// Success requires both native calls and the recording output's successful finish callback.
    pub fn is_finalized(&self) -> bool {
        self.stop_capture == StopOperation::Succeeded
            && self.remove_recording_output == StopOperation::Succeeded
            && self.recording_finalization == RecordingFinalization::Finished
    }
}

fn operation_outcome(result: Result<(), screencapturekit::error::SCError>) -> StopOperation {
    match result {
        Ok(()) => StopOperation::Succeeded,
        Err(error) => StopOperation::Failed(format!("{error:?}")),
    }
}

pub(super) fn recording_finalization(
    has_recording_output: bool,
    finished: bool,
    failed: bool,
) -> RecordingFinalization {
    if !has_recording_output {
        RecordingFinalization::NotApplicable
    } else if failed {
        RecordingFinalization::Failed
    } else if finished {
        RecordingFinalization::Finished
    } else {
        RecordingFinalization::Pending
    }
}

struct ClockCapture {
    admission: CaptureAdmissionTime,
    stream_clock: screencapturekit::cm::CMClock,
    host_clock: screencapturekit::cm::CMClock,
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

#[cfg(test)]
#[derive(Clone)]
pub(super) struct TestSamplePixels {
    pub(super) pts: screencapturekit::cm::CMTime,
    pub(super) width: usize,
    pub(super) height: usize,
    pub(super) pixel_format: u32,
    pub(super) planar: bool,
    pub(super) bytes_per_row: usize,
    pub(super) data_size: usize,
    pub(super) color: TestPixelColorAttachments,
    pub(super) bytes: Vec<u8>,
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub(super) struct TestPixelColorAttachments {
    pub(super) primaries: Option<String>,
    pub(super) transfer_function: Option<String>,
    pub(super) ycbcr_matrix: Option<String>,
    pub(super) cg_color_space: Option<String>,
    pub(super) cg_color_space_icc: Option<Vec<u8>>,
}

#[cfg(test)]
#[derive(Default)]
struct TestSampleCollection {
    samples: Vec<TestSamplePixels>,
    last_pts: Option<screencapturekit::cm::CMTime>,
    refusal: Option<String>,
}

#[cfg(test)]
const TEST_SAMPLE_LIMIT: usize = 12;
#[cfg(test)]
const TEST_SAMPLE_MAX_BYTES: usize = 2 * 1024 * 1024;
#[cfg(test)]
const TEST_SAMPLE_SPACING_MS: u64 = 100;

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

#[cfg(test)]
fn copy_test_sample_pixels(
    buffer: &screencapturekit::cv::CVPixelBuffer,
    pts: screencapturekit::cm::CMTime,
) -> Result<TestSamplePixels, String> {
    const PIXEL_FORMAT_32_BGRA: u32 = u32::from_be_bytes(*b"BGRA");
    if buffer.is_planar() {
        return Err(
            "source sample is planar; correspondence observer only accepts packed BGRA".into(),
        );
    }
    let pixel_format = buffer.pixel_format();
    if pixel_format != PIXEL_FORMAT_32_BGRA {
        return Err(format!(
            "unsupported source pixel format {pixel_format:#010x}; expected 32BGRA"
        ));
    }
    let width = buffer.width();
    let height = buffer.height();
    let bytes_per_row = buffer.bytes_per_row();
    let minimum_row = width
        .checked_mul(4)
        .ok_or_else(|| "source pixel row size overflow".to_owned())?;
    if width == 0 || height == 0 || bytes_per_row < minimum_row {
        return Err("invalid source pixel dimensions or stride".into());
    }
    let byte_count = bytes_per_row
        .checked_mul(height)
        .filter(|size| *size <= TEST_SAMPLE_MAX_BYTES && *size <= buffer.data_size())
        .ok_or_else(|| "source pixel sample exceeds bounded copy budget".to_owned())?;
    let lock = buffer
        .lock_read_only()
        .map_err(|error| format!("source pixel lock failed: {error}"))?;
    let address = lock.base_address();
    if address.is_null() {
        return Err("source pixel buffer has no mapped base address".into());
    }
    // SAFETY: CVPixelBuffer is read-locked; checked dimensions/stride bound the copied extent.
    let bytes = unsafe { slice::from_raw_parts(address, byte_count) }.to_vec();
    Ok(TestSamplePixels {
        pts,
        width,
        height,
        pixel_format,
        planar: false,
        bytes_per_row,
        data_size: buffer.data_size(),
        color: test_pixel_color_attachments(buffer),
        bytes,
    })
}

/// Retain color declarations and bounded ICC bytes from the exact callback buffer used by the
/// correspondence diagnostic. The locked SCK crate does not expose the attachment wrapper, so
/// this test-only copy uses CoreVideo's retained copy API and checks CF object type/lifetime.
#[cfg(test)]
fn test_pixel_color_attachments(
    buffer: &screencapturekit::cv::CVPixelBuffer,
) -> TestPixelColorAttachments {
    use std::ffi::{CStr, c_char, c_void};

    type CfRef = *const c_void;
    const UTF8_ENCODING: u32 = 0x0800_0100;

    #[link(name = "CoreVideo", kind = "framework")]
    unsafe extern "C" {
        static kCVImageBufferColorPrimariesKey: CfRef;
        static kCVImageBufferTransferFunctionKey: CfRef;
        static kCVImageBufferYCbCrMatrixKey: CfRef;
        static kCVImageBufferCGColorSpaceKey: CfRef;
        fn CVBufferCopyAttachment(buffer: *mut c_void, key: CfRef, mode: *mut u32)
        -> *const c_void;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFGetTypeID(value: CfRef) -> usize;
        fn CFCopyDescription(value: CfRef) -> CfRef;
        fn CFStringGetCString(
            value: CfRef,
            buffer: *mut c_char,
            capacity: isize,
            encoding: u32,
        ) -> bool;
        fn CFDataGetLength(data: CfRef) -> isize;
        fn CFDataGetBytePtr(data: CfRef) -> *const u8;
        fn CFRelease(value: CfRef);
    }
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGColorSpaceGetTypeID() -> usize;
        fn CGColorSpaceCopyICCData(color_space: CfRef) -> CfRef;
    }

    unsafe fn read(buffer: *mut c_void, key: CfRef, describe: bool) -> Option<String> {
        let value = unsafe { CVBufferCopyAttachment(buffer, key, std::ptr::null_mut()) };
        if value.is_null() {
            return None;
        }
        let string = if describe {
            unsafe { CFCopyDescription(value) }
        } else {
            value
        };
        if string.is_null() {
            unsafe { CFRelease(value) };
            return None;
        }
        let mut output = [0i8; 512];
        let copied = unsafe {
            CFStringGetCString(
                string,
                output.as_mut_ptr(),
                output.len() as isize,
                UTF8_ENCODING,
            )
        };
        let result = copied.then(|| {
            // SAFETY: successful CFStringGetCString writes a NUL-terminated UTF-8 buffer.
            unsafe { CStr::from_ptr(output.as_ptr()) }
                .to_string_lossy()
                .into_owned()
        });
        if describe {
            unsafe { CFRelease(string) };
        }
        unsafe { CFRelease(value) };
        result
    }

    unsafe fn read_icc(buffer: *mut c_void, key: CfRef) -> Option<Vec<u8>> {
        const MAX_ICC_PROFILE_BYTES: usize = 32 * 1024;
        let color_space = unsafe { CVBufferCopyAttachment(buffer, key, std::ptr::null_mut()) };
        if color_space.is_null() {
            return None;
        }
        // Accept only a genuine Core Graphics color-space object, not a textual description.
        let is_color_space = unsafe { CFGetTypeID(color_space) == CGColorSpaceGetTypeID() };
        if !is_color_space {
            unsafe { CFRelease(color_space) };
            return None;
        }
        let data = unsafe { CGColorSpaceCopyICCData(color_space) };
        unsafe { CFRelease(color_space) };
        if data.is_null() {
            return None;
        }
        let length = unsafe { CFDataGetLength(data) };
        if length <= 0 || length as usize > MAX_ICC_PROFILE_BYTES {
            unsafe { CFRelease(data) };
            return None;
        }
        let bytes = unsafe { CFDataGetBytePtr(data) };
        if bytes.is_null() {
            unsafe { CFRelease(data) };
            return None;
        }
        // SAFETY: CFData owns a live allocation of the checked positive length until release.
        let copied = unsafe { slice::from_raw_parts(bytes, length as usize) }.to_vec();
        unsafe { CFRelease(data) };
        Some(copied)
    }

    // SAFETY: CoreVideo exports these static CFString keys for process lifetime.
    unsafe {
        let buffer = buffer.as_ptr();
        TestPixelColorAttachments {
            primaries: read(buffer, kCVImageBufferColorPrimariesKey, false),
            transfer_function: read(buffer, kCVImageBufferTransferFunctionKey, false),
            ycbcr_matrix: read(buffer, kCVImageBufferYCbCrMatrixKey, false),
            cg_color_space: read(buffer, kCVImageBufferCGColorSpaceKey, true),
            cg_color_space_icc: read_icc(buffer, kCVImageBufferCGColorSpaceKey),
        }
    }
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
                        clock_capture: None,
                        #[cfg(test)]
                        test_samples: None,
                        checked_stop: None,
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
            clock_capture: None,
            #[cfg(test)]
            test_samples: None,
            checked_stop: None,
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
            clock_capture: None,
            #[cfg(test)]
            test_samples: None,
            checked_stop: None,
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
        #[cfg(test)]
        let test_samples = Arc::new(Mutex::new(TestSampleCollection::default()));
        #[cfg(test)]
        let callback_test_samples = test_samples.clone();
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
                    let pixel_buffer = sample.pixel_buffer();
                    let pixels = pixel_buffer.as_ref().and_then(inspect_pixel_buffer);
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
                        #[cfg(test)]
                        if let Some(buffer) = pixel_buffer.as_ref()
                            && let Ok(mut observations) = callback_test_samples.lock()
                            && observations.refusal.is_none()
                            && observations.samples.len() < TEST_SAMPLE_LIMIT
                        {
                            let due = match observations.last_pts {
                                None => true,
                                Some(previous) => {
                                    match super::clock::checked_relative_milliseconds(previous, pts)
                                    {
                                        Ok(elapsed) => elapsed >= TEST_SAMPLE_SPACING_MS,
                                        Err(error) => {
                                            observations.refusal = Some(format!(
                                                "source sample PTS sequence is invalid: {error}"
                                            ));
                                            false
                                        }
                                    }
                                }
                            };
                            if due {
                                match copy_test_sample_pixels(buffer, pts) {
                                    Ok(observation) => {
                                        observations.last_pts = Some(pts);
                                        observations.samples.push(observation);
                                    }
                                    Err(error) => observations.refusal = Some(error),
                                }
                            }
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
            clock_capture: None,
            #[cfg(test)]
            test_samples: Some(test_samples),
            checked_stop: None,
        })
    }

    /// Strict entry carrying a host-clock observation sampled by the caller exactly at its
    /// capture-admission point. `Instant` remains only the independent callback-arrival
    /// observation used by `FirstSample`; it does not supply the capture epoch.
    pub fn start_explicit_display_with_clock(
        display_id: u32,
        configuration: StreamConfiguration,
        video_output_path: PathBuf,
        observation_epoch: Instant,
        admission: CaptureAdmissionTime,
    ) -> Result<Self, String> {
        let mut capture = Self::start_explicit_display(
            display_id,
            configuration,
            video_output_path,
            observation_epoch,
        )?;
        let stream_clock = capture
            .stream
            .as_ref()
            .and_then(SCStream::synchronization_clock)
            .ok_or(ClockError::ClockUnavailable);
        let stream_clock = match stream_clock {
            Ok(clock) => clock,
            Err(error) => {
                capture.stop();
                return Err(error.to_string());
            }
        };
        let host_clock = screencapturekit::cm::CMClock::host_time_clock();
        capture.clock_capture = Some(ClockCapture {
            admission,
            stream_clock,
            host_clock,
        });
        Ok(capture)
    }

    /// First complete screen sample with a mapped, nonempty pixel buffer. Its PTS is on the
    /// native media clock; callback observation is measured separately by `Instant`. No clock
    /// conversion is inferred. `None` means no usable timestamped image sample arrived.
    pub fn first_sample(&self) -> Option<FirstSample> {
        self.first_sample
            .as_ref()
            .and_then(|sample| sample.lock().ok().and_then(|sample| *sample))
    }

    /// Convert the retained first-content sample from the SCK output clock into host time and
    /// capture-relative milliseconds. This describes the source sample only; it is not a
    /// recording-output/MP4 origin or `videoOffsetMs` measurement.
    pub fn first_sample_capture_clock(&self) -> Result<Option<CaptureClockSample>, ClockError> {
        let Some(clock_capture) = &self.clock_capture else {
            return Err(ClockError::ClockUnavailable);
        };
        let Some(sample) = self.first_sample() else {
            return Ok(None);
        };
        let source_pts = screencapturekit::cm::CMTime {
            value: sample.presentation_value,
            timescale: sample.presentation_timescale,
            flags: sample.presentation_flags,
            epoch: sample.presentation_epoch,
        };
        map_sample_to_capture_clock(
            source_pts,
            &clock_capture.stream_clock,
            &clock_capture.host_clock,
            clock_capture.admission,
        )
        .map(Some)
    }

    #[cfg(test)]
    pub(super) fn test_sample_pixels(&self) -> Result<Vec<TestSamplePixels>, String> {
        let state = self
            .test_samples
            .as_ref()
            .ok_or_else(|| "strict SCK sample observer unavailable".to_owned())?;
        let state = state
            .lock()
            .map_err(|_| "strict SCK sample observer poisoned".to_owned())?;
        if let Some(refusal) = &state.refusal {
            return Err(refusal.clone());
        }
        Ok(state.samples.clone())
    }

    #[cfg(test)]
    pub(super) fn test_sample_count(&self) -> Result<usize, String> {
        let state = self
            .test_samples
            .as_ref()
            .ok_or_else(|| "strict SCK sample observer unavailable".to_owned())?;
        let state = state
            .lock()
            .map_err(|_| "strict SCK sample observer poisoned".to_owned())?;
        if let Some(refusal) = &state.refusal {
            return Err(refusal.clone());
        }
        Ok(state.samples.len())
    }

    #[cfg(test)]
    pub(super) fn test_sample_clock(
        &self,
        pts: screencapturekit::cm::CMTime,
    ) -> Result<CaptureClockSample, ClockError> {
        let Some(clock_capture) = &self.clock_capture else {
            return Err(ClockError::ClockUnavailable);
        };
        map_sample_to_capture_clock(
            pts,
            &clock_capture.stream_clock,
            &clock_capture.host_clock,
            clock_capture.admission,
        )
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

    /// Reports the latest callback state without inferring completion from stop return or
    /// callback order. The output object remains owned until this CaptureStream is dropped.
    pub fn recording_finalization(&self) -> RecordingFinalization {
        recording_finalization(
            self.recording_output.is_some(),
            self.recording_state
                .as_ref()
                .is_some_and(|state| state.finished.load(Ordering::Acquire)),
            self.recording_failed(),
        )
    }

    /// Performs the existing blocking `stop_capture` then `remove_recording_output` sequence,
    /// preserving each result. Taking the stream before either call ensures `Drop` cannot
    /// retry native teardown after an error. The output/delegate remains owned by `self` so the
    /// caller can retain its pinned file guard while finalization is pending or failed.
    pub fn stop_checked(&mut self) -> CheckedStopOutcome {
        if self.checked_stop.is_none()
            && let Some(stream) = self.stream.take()
        {
            let stopped = operation_outcome(stream.stop_capture());
            let removed = self
                .recording_output
                .as_ref()
                .map(|output| operation_outcome(stream.remove_recording_output(output)))
                .unwrap_or(StopOperation::NotAttempted);
            self.checked_stop = Some((stopped, removed));
        }
        self.checked_stop_outcome()
    }

    /// Re-reads callbacks after a caller's own bounded wait; this performs no native operation.
    pub fn checked_stop_outcome(&self) -> CheckedStopOutcome {
        let (stop_capture, remove_recording_output) = self
            .checked_stop
            .clone()
            .unwrap_or((StopOperation::NotAttempted, StopOperation::NotAttempted));
        CheckedStopOutcome {
            stop_capture,
            remove_recording_output,
            recording_finalization: self.recording_finalization(),
        }
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
