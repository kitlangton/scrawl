//! ScreenCaptureKit stream, filter and image acquisition used by the legacy Mac capture caller.
//! The caller supplies target/exclusion policy and neutral dimensions; this module owns the
//! ScreenCaptureKit objects, recording output, filter replacement, pixel readback and teardown.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use screencapturekit::prelude::*;
use screencapturekit::recording_output::{
    SCRecordingOutput, SCRecordingOutputCodec, SCRecordingOutputConfiguration,
    SCRecordingOutputFileType,
};
use screencapturekit::screenshot_manager::{CGImageExt, SCScreenshotManager};

#[derive(Clone, Copy, Debug)]
pub enum CaptureSource {
    /// `None` preserves the legacy refusal to fall back to a display for an absent window.
    Window(Option<u32>),
    /// A missing display ID falls back to the first shareable display, as the legacy caller did.
    Display(u32),
}

#[derive(Clone, Debug)]
pub struct FilterRequest {
    pub source: CaptureSource,
    pub excluded_window_ids: Vec<u32>,
    /// The legacy caller sets this from its existing movable-controls experiment policy.
    pub exclude_own_non_normal_windows: bool,
    pub own_process_id: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct StreamConfiguration {
    pub width: u32,
    pub height: u32,
    /// Display-local source rectangle, calculated by the legacy geometry policy.
    pub source_rect: Option<[f64; 4]>,
}

#[derive(Clone, Debug)]
pub struct CaptureRequest {
    pub filter: FilterRequest,
    pub configuration: StreamConfiguration,
}

#[derive(Debug)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub bytes: Vec<u8>,
}

/// Native candidate configuration built once by the owning sampler, as in the donor loop.
pub struct FrameConfiguration(SCStreamConfiguration);

/// Owns the stream and recording output on the legacy sampler worker.
pub struct CaptureStream {
    stream: Option<SCStream>,
    recording_output: Option<SCRecordingOutput>,
    video_path: Option<PathBuf>,
    filter: Option<SCContentFilter>,
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
        })
    }

    fn empty() -> Self {
        Self {
            stream: None,
            recording_output: None,
            video_path: None,
            filter: None,
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

/// Which requested window IDs ScreenCaptureKit currently exposes in shareable content.
/// Query failures retain the donor's empty-list behavior.
pub fn listed_window_ids(ids: &[u32]) -> Vec<u32> {
    if ids.is_empty() {
        return Vec::new();
    }
    SCShareableContent::get()
        .map(|content| {
            content
                .windows()
                .into_iter()
                .map(|window| window.window_id())
                .filter(|id| ids.contains(id))
                .collect()
        })
        .unwrap_or_default()
}

fn build_filter_and_config(
    request: &CaptureRequest,
) -> Result<Option<(SCContentFilter, SCStreamConfiguration)>, String> {
    Ok(build_filter(&request.filter)?
        .map(|filter| (filter, stream_configuration(request.configuration))))
}

fn build_filter(request: &FilterRequest) -> Result<Option<SCContentFilter>, String> {
    let content = SCShareableContent::get().map_err(|error| format!("{error:?}"))?;
    let filter = match request.source {
        CaptureSource::Window(None) => None,
        CaptureSource::Window(Some(window_id)) => content
            .windows()
            .into_iter()
            .find(|window| window.window_id() == window_id)
            .and_then(|window| SCContentFilter::create().with_window(&window).build().ok()),
        CaptureSource::Display(display_id) => content
            .displays()
            .into_iter()
            .find(|display| display.display_id() == display_id)
            .or_else(|| content.displays().into_iter().next())
            .and_then(|display| {
                let excluded: Vec<SCWindow> = content
                    .windows()
                    .into_iter()
                    .filter(|window| {
                        request.excluded_window_ids.contains(&window.window_id())
                            || (request.exclude_own_non_normal_windows
                                && window
                                    .owning_application()
                                    .as_ref()
                                    .is_some_and(|app| app.process_id() == request.own_process_id)
                                && window.window_layer() != 0)
                    })
                    .collect();
                let excluded_refs: Vec<&SCWindow> = excluded.iter().collect();
                SCContentFilter::create()
                    .with_display(&display)
                    .with_excluding_windows(&excluded_refs)
                    .build()
                    .ok()
            }),
    };
    Ok(filter)
}

fn stream_configuration(configuration: StreamConfiguration) -> SCStreamConfiguration {
    let mut config = SCStreamConfiguration::new()
        .with_width(configuration.width)
        .with_height(configuration.height)
        .with_shows_cursor(true);
    if let Some([x, y, width, height]) = configuration.source_rect {
        config.set_source_rect(CGRect::new(x, y, width, height));
    }
    config
}
