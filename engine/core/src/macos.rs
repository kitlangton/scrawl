//! ScreenCaptureKit stream, filter and image acquisition used by the legacy Mac capture caller.
//! The caller supplies target/exclusion policy and neutral dimensions; this module owns the
//! ScreenCaptureKit objects, recording output, filter replacement, pixel readback and teardown.

use screencapturekit::prelude::*;

mod output;
mod recording;
#[cfg(test)]
mod tests;

pub use output::SckOutputGuard;
pub use recording::{CaptureStream, FirstSample};

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
