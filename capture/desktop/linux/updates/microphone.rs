//! PulseAudio-compatible sources (including PipeWire's Pulse server). CPAL's ALSA host
//! exposes only `default`/`pipewire` on these desktops, not the individual inputs.
//! Keep source selection per stream; never change the user's system default.

use anyhow::{Context, Result};
use gst::prelude::*;
use gstreamer as gst;
use gstreamer_app as gst_app;
use std::sync::Arc;

const PREFIX: &str = "pulse:";

pub fn source(id: &str) -> Option<&str> {
    id.strip_prefix(PREFIX).filter(|name| !name.is_empty())
}

pub fn inputs() -> Option<crate::capture::engine::runtime::audio::mic::Inputs> {
    let output = std::process::Command::new("pactl")
        .args(["--format=json", "list", "sources"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let devices = parse_sources(&output.stdout)?;
    if devices.is_empty() {
        return None;
    }
    let default = std::process::Command::new("pactl")
        .arg("get-default-source")
        .output()
        .ok()
        .filter(|result| result.status.success())
        .and_then(|result| String::from_utf8(result.stdout).ok())
        .map(|name| format!("{PREFIX}{}", name.trim()));
    Some(crate::capture::engine::runtime::audio::mic::Inputs { devices, default })
}

fn parse_sources(bytes: &[u8]) -> Option<Vec<crate::capture::engine::runtime::audio::mic::Input>> {
    let sources: serde_json::Value = serde_json::from_slice(bytes).ok()?;
    let sources = sources.as_array()?;
    Some(
        sources
            .iter()
            .filter_map(|source| {
                let name = source.get("name")?.as_str()?.trim();
                if name.is_empty() {
                    return None;
                }
                Some(crate::capture::engine::runtime::audio::mic::Input {
                    id: format!("{PREFIX}{name}"),
                    name: source
                        .get("description")
                        .and_then(|d| d.as_str())
                        .filter(|d| !d.is_empty())
                        .unwrap_or(name)
                        .to_owned(),
                })
            })
            .collect(),
    )
}

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

#[cfg(test)]
mod tests {
    #[test]
    fn pulse_source_ids_keep_distinct_inputs_and_readable_names() {
        let devices = super::parse_sources(br#"[{"name":"mic_a.monitor","description":"Monitor of MicA"},{"name":"mic_b.monitor","description":"Monitor of MicB"}]"#).unwrap();
        assert_eq!(devices[0].id, "pulse:mic_a.monitor");
        assert_eq!(devices[1].name, "Monitor of MicB");
        assert_eq!(super::source(&devices[1].id), Some("mic_b.monitor"));
        assert_eq!(super::source("default"), None);
    }
}
