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

impl Stream {
    pub fn start(name: &str, on_samples: impl Fn(&[f32]) + Send + Sync + 'static) -> Result<Self> {
        gst::init()?;
        // Only the constant part is parsed. Never interpolate a device name into a pipeline.
        let pipeline = gst::parse::launch("audioconvert name=convert ! audioresample ! audio/x-raw,format=F32LE,rate=16000,channels=1 ! appsink name=input max-buffers=4 drop=true sync=false")?
            .downcast::<gst::Pipeline>().map_err(|_| anyhow::anyhow!("Not an audio pipeline"))?;
        let source = gst::ElementFactory::make("pulsesrc")
            .property("device", name)
            .build()
            .context("Pulse source is unavailable")?;
        pipeline.add(&source)?;
        source.link(&pipeline.by_name("convert").context("No audio converter")?)?;
        let sink = pipeline
            .by_name("input")
            .context("No audio sink")?
            .downcast::<gst_app::AppSink>()
            .map_err(|_| anyhow::anyhow!("Not an audio sink"))?;
        let on_samples = Arc::new(on_samples);
        let (ready, first_sample) = std::sync::mpsc::sync_channel(1);
        sink.set_callbacks(
            gst_app::AppSinkCallbacks::builder()
                .new_sample(move |sink| {
                    let sample = sink.pull_sample().map_err(|_| gst::FlowError::Eos)?;
                    let buffer = sample.buffer().ok_or(gst::FlowError::Error)?;
                    let map = buffer.map_readable().map_err(|_| gst::FlowError::Error)?;
                    let pcm: Vec<f32> = map
                        .as_slice()
                        .chunks_exact(4)
                        .map(|chunk| f32::from_le_bytes(chunk.try_into().unwrap()))
                        .collect();
                    on_samples(&pcm);
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
