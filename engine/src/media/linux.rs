//! Bounded, file-descriptor-pinned Linux GStreamer decoder.

use std::fs::File;
use std::os::fd::AsRawFd;
use std::sync::OnceLock;

use gst::prelude::*;
use gst_video::VideoFrameExt;
use gstreamer as gst;
use gstreamer_app as gst_app;
use gstreamer_video as gst_video;

const SOURCE_EDGE_LIMIT: u32 = 16_384;
const SOURCE_BYTES_LIMIT: u64 = 256 * 1024 * 1024;
const WAIT: gst::ClockTime = gst::ClockTime::from_seconds(10);

pub struct Decoded {
    pub actual_ms: u64,
    pub clamped_to_end: bool,
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

fn initialize() -> Result<(), &'static str> {
    static INIT: OnceLock<bool> = OnceLock::new();
    if *INIT.get_or_init(|| gst::init().is_ok()) {
        Ok(())
    } else {
        Err("decode_failed")
    }
}

struct Reader {
    pipeline: gst::Pipeline,
    sink: gst_app::AppSink,
    duration_ms: u64,
}

impl Reader {
    fn open(source: &File) -> Result<Self, &'static str> {
        initialize()?;
        let fd = source.as_raw_fd();
        let pipeline = gst::Pipeline::new();
        let src = gst::ElementFactory::make("filesrc")
            .build()
            .map_err(|_| "decode_failed")?;
        src.set_property("location", format!("/proc/self/fd/{fd}"));
        let decode = gst::ElementFactory::make("decodebin")
            .build()
            .map_err(|_| "decode_failed")?;
        let convert = gst::ElementFactory::make("videoconvert")
            .build()
            .map_err(|_| "decode_failed")?;
        let filter = gst::ElementFactory::make("capsfilter")
            .build()
            .map_err(|_| "decode_failed")?;
        filter.set_property(
            "caps",
            gst::Caps::builder("video/x-raw")
                .field("format", "RGBA")
                .build(),
        );
        let sink = gst::ElementFactory::make("appsink")
            .build()
            .map_err(|_| "decode_failed")?
            .downcast::<gst_app::AppSink>()
            .map_err(|_| "decode_failed")?;
        sink.set_property("sync", false);
        sink.set_property("max-buffers", 1u32);
        sink.set_property("drop", true);
        let sink_element = sink.clone().upcast::<gst::Element>();
        pipeline
            .add_many([
                src.clone(),
                decode.clone(),
                convert.clone(),
                filter.clone(),
                sink_element.clone(),
            ])
            .map_err(|_| "decode_failed")?;
        gst::Element::link_many([src.clone(), decode.clone()]).map_err(|_| "decode_failed")?;
        gst::Element::link_many([convert.clone(), filter.clone(), sink_element])
            .map_err(|_| "decode_failed")?;
        let convert_weak = convert.downgrade();
        decode.connect_pad_added(move |_, pad| {
            if let Some(convert) = convert_weak.upgrade() {
                if convert.static_pad("sink").is_some_and(|p| !p.is_linked()) {
                    let _ = pad.link(&convert.static_pad("sink").unwrap());
                }
            }
        });
        let mut reader = Self {
            pipeline,
            sink,
            duration_ms: 0,
        };
        reader
            .pipeline
            .set_state(gst::State::Paused)
            .map_err(|_| "decode_failed")?;
        let (state, _, _) = reader.pipeline.state(WAIT);
        if state.is_err() {
            return Err("source_incomplete");
        }
        reader.duration_ms = reader
            .pipeline
            .query_duration::<gst::ClockTime>()
            .map(|d| d.mseconds())
            .filter(|d| *d > 0)
            .ok_or("source_incomplete")?;
        Ok(reader)
    }

    fn frame(&self, requested: u64) -> Result<(u64, u32, u32, Vec<u8>), &'static str> {
        let target = requested.min(self.duration_ms - 1);
        self.sink.set_drop(false);
        self.pipeline
            .seek_simple(
                gst::SeekFlags::FLUSH | gst::SeekFlags::ACCURATE,
                gst::ClockTime::from_mseconds(target),
            )
            .map_err(|_| "decode_failed")?;
        let (state, _, _) = self.pipeline.state(WAIT);
        if state.is_err() {
            return Err("decode_failed");
        }
        let sample = self
            .sink
            .try_pull_preroll(WAIT)
            .ok_or("source_incomplete")?;
        let caps = sample.caps().ok_or("decode_failed")?;
        let info = gst_video::VideoInfo::from_caps(caps).map_err(|_| "decode_failed")?;
        let (width, height) = (info.width(), info.height());
        let pixels = u64::from(width)
            .checked_mul(u64::from(height))
            .ok_or("limits_exceeded")?;
        if width == 0
            || height == 0
            || width > SOURCE_EDGE_LIMIT
            || height > SOURCE_EDGE_LIMIT
            || pixels
                .checked_mul(4)
                .filter(|n| *n <= SOURCE_BYTES_LIMIT)
                .is_none()
        {
            return Err("limits_exceeded");
        }
        let buffer = sample.buffer().ok_or("decode_failed")?;
        let frame = gst_video::VideoFrameRef::from_buffer_ref_readable(buffer, &info)
            .map_err(|_| "source_incomplete")?;
        let stride = *frame.plane_stride().first().ok_or("decode_failed")?;
        let stride = usize::try_from(stride).map_err(|_| "source_incomplete")?;
        let row_bytes = (width as usize).checked_mul(4).ok_or("limits_exceeded")?;
        if stride < row_bytes {
            return Err("source_incomplete");
        }
        let plane = frame.plane_data(0).map_err(|_| "source_incomplete")?;
        let need = stride
            .checked_mul(height as usize)
            .ok_or("limits_exceeded")?;
        if need > SOURCE_BYTES_LIMIT as usize || plane.len() < need {
            return Err("source_incomplete");
        }
        let mut rgba = Vec::with_capacity(
            row_bytes
                .checked_mul(height as usize)
                .ok_or("limits_exceeded")?,
        );
        for row in 0..height as usize {
            rgba.extend_from_slice(&plane[row * stride..row * stride + row_bytes]);
        }
        let actual_ms = sample
            .buffer()
            .and_then(gst::BufferRef::pts)
            .map(|p| p.mseconds())
            .unwrap_or(target);
        Ok((actual_ms, width, height, rgba))
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

fn shrink(
    width: u32,
    height: u32,
    rgba: Vec<u8>,
    edge: u32,
) -> Result<(u32, u32, Vec<u8>), &'static str> {
    let long = width.max(height);
    if long <= edge {
        return Ok((width, height, rgba));
    }
    let scale = f64::from(edge) / f64::from(long);
    let (ow, oh) = (
        ((f64::from(width) * scale).round() as u32).max(1),
        ((f64::from(height) * scale).round() as u32).max(1),
    );
    let (sw, sh) = (width as usize, height as usize);
    let mut out = vec![
        0u8;
        (ow as usize)
            .checked_mul(oh as usize)
            .and_then(|n| n.checked_mul(4))
            .ok_or("limits_exceeded")?
    ];
    for y in 0..oh as usize {
        let (y0, y1) = (
            y * sh / oh as usize,
            ((y + 1) * sh / oh as usize).max(y * sh / oh as usize + 1),
        );
        for x in 0..ow as usize {
            let (x0, x1) = (
                x * sw / ow as usize,
                ((x + 1) * sw / ow as usize).max(x * sw / ow as usize + 1),
            );
            let mut sums = [0u64; 3];
            let mut count = 0u64;
            for sy in y0..y1 {
                for px in rgba[(sy * sw + x0) * 4..(sy * sw + x1) * 4].chunks_exact(4) {
                    for c in 0..3 {
                        sums[c] += u64::from(px[c]);
                    }
                    count += 1;
                }
            }
            let offset = (y * ow as usize + x) * 4;
            out[offset..offset + 4].copy_from_slice(&[
                (sums[0] / count) as u8,
                (sums[1] / count) as u8,
                (sums[2] / count) as u8,
                255,
            ]);
        }
    }
    Ok((ow, oh, out))
}

pub fn decode(
    source: &File,
    times: &[u64],
    max_edge: u32,
    max_pixels: u64,
    mut emit: impl FnMut(Decoded) -> Result<(), &'static str>,
) -> Result<(), &'static str> {
    if max_edge == 0 || max_edge > 4096 || max_pixels == 0 || max_pixels > 64_000_000 {
        return Err("limits_exceeded");
    }
    let metadata = source.metadata().map_err(|_| "source_incomplete")?;
    if metadata.len() == 0 {
        return Err("source_incomplete");
    }
    let reader = Reader::open(source)?;
    let mut total = 0u64;
    for &requested in times {
        let clamped_to_end = requested >= reader.duration_ms;
        let (actual_ms, w, h, raw) = reader.frame(requested)?;
        let (width, height, rgba) = shrink(w, h, raw, max_edge)?;
        total = total
            .checked_add(u64::from(width) * u64::from(height))
            .ok_or("limits_exceeded")?;
        if total > max_pixels {
            return Err("limits_exceeded");
        }
        emit(Decoded {
            actual_ms,
            clamped_to_end,
            width,
            height,
            rgba,
        })?;
    }
    Ok(())
}
