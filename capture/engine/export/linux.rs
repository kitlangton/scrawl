//! Legacy export caller adapter for Engine's Linux FFmpeg execution.
use std::path::Path;

use anyhow::Result;

use super::{Done, Plan};

/// Export uses FFmpeg, not the GStreamer keyframe decoder.
pub(super) fn probe(movie: &Path) -> Result<crate::capture::desktop::helper::Probe> {
    let probe = apparent_capture_engine::linux::export::probe(movie)?;
    Ok(crate::capture::desktop::helper::Probe {
        duration_ms: probe.duration_ms,
        width: probe.width,
        height: probe.height,
        audio_tracks: probe.audio_tracks,
        fps: probe.fps,
    })
}

pub(super) fn run(plan: &Plan<'_>, progress: &mut dyn FnMut(f32)) -> Result<Done> {
    let mut annotate = |frame: &mut apparent_capture_engine::linux::media::Frame,
                        ms: f64,
                        rect: [f32; 4],
                        source: (u32, u32)| {
        let mut rgba = crate::shared::model::RgbaFrame::new(
            frame.t_ms,
            frame.width,
            frame.height,
            std::mem::take(&mut frame.rgba),
        );
        crate::capture::engine::render::draw_export_annotations(
            &mut rgba, ms, rect, source, plan.ink,
        );
        frame.rgba = rgba.rgba;
    };
    let mut core_plan = apparent_capture_engine::linux::export::Plan {
        video: plan.video,
        out: plan.out,
        audio: plan.audio,
        audio_start_ms: plan.audio_start_ms,
        width: plan.width,
        height: plan.height,
        fps: plan.fps,
        codec: plan.codec,
        duration_ms: plan.duration_ms,
        crop: plan.crop,
        camera: plan.camera.clone(),
        annotate: &mut annotate,
    };
    let done = apparent_capture_engine::linux::export::run(&mut core_plan, progress)?;
    Ok(Done {
        duration_ms: done.probe.duration_ms,
        bytes: done.bytes,
    })
}
