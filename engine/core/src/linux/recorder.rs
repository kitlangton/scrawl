//! GStreamer-backed Linux recording source and execution.
use std::path::Path;
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail, ensure};
use gstreamer as gst;
use gstreamer::prelude::*;
use gstreamer_app as gst_app;

const FPS: u32 = 30;

pub fn init_gstreamer() -> Result<()> {
    static READY: OnceLock<std::result::Result<(), String>> = OnceLock::new();
    match READY.get_or_init(|| gst::init().map_err(|error| error.to_string())) {
        Ok(()) => Ok(()),
        Err(error) => bail!("GStreamer is not available: {error}"),
    }
}

/// A PipeWire stream a compositor is sending, open until dropped.
pub struct Stream {
    pub node: u32,
    /// The portal's PipeWire connection; Mutter's streams are on the user's own PipeWire.
    fd: Option<OwnedFd>,
    /// Portal compositor coordinates, not XWayland pixels; Hyprland reports (0, 0) for
    /// every output, so do not use this to identify the selected output there.
    pub rect: Option<[f64; 4]>,
    /// The output name supplied by the portal (Hyprland reports this even when position is 0).
    pub mapping_id: Option<String>,
    stop: Option<flume::Sender<()>>,
}

impl Stream {
    pub fn new(
        node: u32,
        fd: Option<OwnedFd>,
        rect: Option<[f64; 4]>,
        mapping_id: Option<String>,
        stop: Option<flume::Sender<()>>,
    ) -> Self {
        Self { node, fd, rect, mapping_id, stop }
    }

    /// The PipeWire fd for `pipewiresrc fd=`, or -1 for the default PipeWire.
    pub fn fd(&self) -> i32 {
        self.fd.as_ref().map_or(-1, AsRawFd::as_raw_fd)
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

/// Native source owns its portal stream through recorder finalization.
pub enum Source {
    #[cfg(test)]
    Test,
    X11 {
        rect: [f64; 4],
    },
    X11Window {
        xid: u32,
    },
    PipeWire {
        session: Stream,
        crop: Option<[u32; 4]>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CandidateFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

fn encoder() -> Option<&'static str> {
    [
        "x264enc tune=zerolatency speed-preset=veryfast key-int-max=60 bitrate=6000",
        "openh264enc complexity=low bitrate=6000000 gop-size=60",
        "vah264enc",
        "vaapih264enc",
    ]
    .into_iter()
    .find(|launch| {
        gst::ElementFactory::find(launch.split_whitespace().next().unwrap_or_default()).is_some()
    })
}

pub fn can_record_video() -> bool {
    init_gstreamer().is_ok() && encoder().is_some()
}

pub fn encode_synthetic(movie: &Path) -> Result<()> {
    init_gstreamer()?;
    let selected = encoder().ok_or_else(|| anyhow!("No H.264 encoder is available"))?;
    let launch = format!(
        "videotestsrc num-buffers=12 ! video/x-raw,width=160,height=90,framerate=10/1 ! videoconvert ! video/x-raw,format=I420 ! {selected} ! h264parse ! mp4mux ! filesink name=encoder_output"
    );
    let pipeline = gst::parse::launch(&launch)?
        .downcast::<gst::Pipeline>()
        .map_err(|_| anyhow!("Not a pipeline"))?;
    pipeline
        .by_name("encoder_output")
        .context("Encoder pipeline has no output sink")?
        .set_property("location", movie.to_string_lossy().as_ref());
    let result = (|| {
        pipeline.set_state(gst::State::Playing)?;
        let bus = pipeline.bus().context("Encoder pipeline has no bus")?;
        match bus.timed_pop_filtered(
            gst::ClockTime::from_seconds(10),
            &[gst::MessageType::Eos, gst::MessageType::Error],
        ) {
            Some(message) => match message.view() {
                gst::MessageView::Eos(..) => Ok(()),
                gst::MessageView::Error(error) => bail!("H.264 encoding failed: {error}"),
                _ => unreachable!(),
            },
            None => bail!("H.264 encoding timed out"),
        }
    })();
    let _ = pipeline.set_state(gst::State::Null);
    result?;
    ensure!(
        std::fs::metadata(movie)?.len() > 1024,
        "H.264 encoding produced an empty movie"
    );
    Ok(())
}

fn source_launch(source: &Source) -> String {
    match source {
        #[cfg(test)]
        Source::Test => "videotestsrc is-live=true pattern=smpte".to_owned(),
        Source::X11 { rect } => {
            let [x, y, w, h] = rect.map(|v| v.round() as i64);
            format!(
                "ximagesrc use-damage=false show-pointer=true startx={x} starty={y} endx={} endy={}",
                (x + w - 1).max(x),
                (y + h - 1).max(y)
            )
        }
        Source::X11Window { xid } => {
            format!("ximagesrc use-damage=false show-pointer=true xid={xid}")
        }
        Source::PipeWire { session, crop } => {
            let mut launch = format!(
                "pipewiresrc fd={} path={} do-timestamp=true keepalive-time=500 always-copy=true",
                session.fd(), session.node
            );
            if let Some([x, y, w, h]) = crop {
                // `videocrop` trims from each edge; the stream's size is only known once it
                // runs, so `crop_edges` fills them in when the caps arrive.
                let _ = (x, y, w, h);
                launch.push_str(" ! videoconvert ! videocrop name=crop");
            }
            launch
        }
    }
}

/// A running recording: GStreamer pushes frames through; Apparent samples the newest one.
pub struct Recorder {
    pub(crate) pipeline: gst::Pipeline,
    pub(crate) candidates: gst_app::AppSink,
    candidate: (u32, u32),
    video: bool,
    first_frame_ms: Arc<AtomicU64>,
    last: Option<Vec<u8>>,
}

impl Recorder {
    pub fn new(
        source: &Source,
        out: (u32, u32),
        candidate: (u32, u32),
        movie: Option<&Path>,
        started_at: Instant,
    ) -> Result<Self> {
        init_gstreamer()?;
        let encoder = movie.and(encoder());
        let (w, h) = out;
        let (cw, ch) = candidate;
        let mut launch = format!(
            "{} ! queue max-size-buffers=4 leaky=downstream ! videoconvert ! videoscale ! video/x-raw,width={w},height={h},pixel-aspect-ratio=1/1 ! tee name=t \
             t. ! queue max-size-buffers=2 leaky=downstream ! videoconvert ! videoscale ! video/x-raw,format=RGBA,width={cw},height={ch},pixel-aspect-ratio=1/1 ! appsink name=candidates max-buffers=1 drop=true sync=false emit-signals=false",
            source_launch(source)
        );
        if let (Some(encoder), Some(movie)) = (encoder, movie) {
            launch.push_str(&format!(
                " t. ! queue max-size-time=3000000000 max-size-buffers=0 max-size-bytes=0 ! videorate ! video/x-raw,framerate={FPS}/1 ! videoconvert ! video/x-raw,format=I420 ! {encoder} name=encoder ! h264parse ! mp4mux faststart=true ! filesink name=movie_sink location=\"{}\"",
                movie.display().to_string().replace('"', "\\\"")
            ));
        }
        let pipeline = gst::parse::launch(&launch)
            .with_context(|| format!("Could not build the recording pipeline: {launch}"))?
            .downcast::<gst::Pipeline>()
            .map_err(|_| anyhow!("Not a pipeline"))?;
        // The candidate sink can hold back pipeline EOS after the movie sink has finished.
        // Forward child EOS so finish() can distinguish a completed MP4 from an unfinished one.
        pipeline.set_property("message-forward", true);
        if let (
            Source::PipeWire {
                crop: Some(crop), ..
            },
            Some(element),
        ) = (source, pipeline.by_name("crop"))
        {
            crop_edges(&element, *crop);
        }
        let candidates = pipeline
            .by_name("candidates")
            .context("No candidate sink")?
            .downcast::<gst_app::AppSink>()
            .map_err(|_| anyhow!("Not an appsink"))?;
        let first_frame_ms = Arc::new(AtomicU64::new(u64::MAX));
        let video = encoder.is_some();
        if let Some(sink) = pipeline
            .by_name("encoder")
            .and_then(|e| e.static_pad("sink"))
        {
            // The movie starts at its first frame: note when that was on the capture clock, and
            // shift every frame so the movie's clock starts at zero.
            let first = Arc::clone(&first_frame_ms);
            let base: Arc<Mutex<Option<gst::ClockTime>>> = Arc::new(Mutex::new(None));
            sink.add_probe(gst::PadProbeType::BUFFER, move |_, info| {
                if let Some(gst::PadProbeData::Buffer(buffer)) = info.data.as_mut() {
                    let mut base = base
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let pts = buffer.pts();
                    if base.is_none() {
                        *base = pts;
                        first.store(started_at.elapsed().as_millis() as u64, Ordering::Release);
                    }
                    if let (Some(pts), Some(zero)) = (pts, *base) {
                        let buffer = buffer.make_mut();
                        buffer.set_pts(pts.saturating_sub(zero));
                        buffer.set_dts(gst::ClockTime::NONE);
                    }
                }
                gst::PadProbeReturn::Ok
            });
        }
        pipeline
            .set_state(gst::State::Playing)
            .context("The recording pipeline wouldn't start")?;
        Ok(Self {
            pipeline,
            candidates,
            candidate,
            video,
            first_frame_ms,
            last: None,
        })
    }

    /// An error the pipeline posted since the last look.
    pub fn error(&self) -> Option<String> {
        let bus = self.pipeline.bus()?;
        while let Some(message) = bus.pop() {
            if let gst::MessageView::Error(error) = message.view() {
                return Some(format!(
                    "{} ({})",
                    error.error(),
                    error.debug().unwrap_or_default()
                ));
            }
        }
        None
    }

    /// The newest frame, or the last one again: compositors send frames only when the screen
    /// changes.
    pub fn candidate(
        &mut self,
        mut apply_masks: impl FnMut(&mut [u8], u32, u32),
    ) -> Option<CandidateFrame> {
        let (w, h) = self.candidate;
        if let Some(sample) = self.candidates.try_pull_sample(gst::ClockTime::ZERO)
            && let Some(rgba) = rgba_of(&sample, w, h)
        {
            self.last = Some(rgba);
        }
        // Keep source pixels intact: masks may change without a new source frame.
        let mut rgba = self.last.clone()?;
        apply_masks(&mut rgba, w, h);
        Some(CandidateFrame {
            width: w,
            height: h,
            rgba,
        })
    }

    pub fn first_frame_ms(&self) -> Option<u64> {
        let value = self.first_frame_ms.load(Ordering::Acquire);
        (value != u64::MAX).then_some(value)
    }

    pub fn has_video(&self) -> bool {
        self.video
    }

    pub fn stop(&self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }

    /// Sends end-of-stream and waits (a few seconds at most) for the movie to be finished.
    pub fn finish(self, mut log_error: impl FnMut(String)) -> bool {
        if !self.pipeline.send_event(gst::event::Eos::new()) {
            let _ = self.pipeline.set_state(gst::State::Null);
            log_error("Movie failed to finish: end of stream was rejected".into());
            return false;
        }
        let deadline = Instant::now() + Duration::from_secs(8);
        let movie_sink = self.pipeline.by_name("movie_sink");
        let (message, movie_finished) = self.pipeline.bus().map_or((None, false), |bus| {
            loop {
                if Instant::now() >= deadline {
                    break (None, false);
                }
                // The candidate sink holds end-of-stream back until its last frame is taken.
                while self
                    .candidates
                    .try_pull_sample(gst::ClockTime::ZERO)
                    .is_some()
                {}
                let message = bus.timed_pop_filtered(
                    gst::ClockTime::from_mseconds(20),
                    &[
                        gst::MessageType::Eos,
                        gst::MessageType::Error,
                        gst::MessageType::Element,
                    ],
                );
                if let Some(message) = &message {
                    if message.type_() == gst::MessageType::Element
                        && let Some(forwarded) = message.structure()
                        && forwarded.name() == "GstBinForwarded"
                        && let Ok(child) = forwarded.get::<gst::Message>("message")
                        && child.type_() == gst::MessageType::Eos
                        && movie_sink
                            .as_ref()
                            .is_some_and(|sink| child.src() == Some(sink.upcast_ref()))
                    {
                        break (None, true);
                    }
                    if message.type_() == gst::MessageType::Element {
                        continue;
                    }
                    break (Some(message.clone()), false);
                }
            }
        });
        let finished = match message.as_ref().map(|m| m.view()) {
            Some(gst::MessageView::Eos(_)) => true,
            Some(gst::MessageView::Error(error)) => {
                log_error(format!("Movie failed to finish: {}", error.error()));
                false
            }
            _ if movie_finished => true,
            _ => {
                log_error("Movie failed to finish: no end of stream within 8 s".into());
                false
            }
        };
        let _ = self.pipeline.set_state(gst::State::Null);
        finished && self.video
    }
}

/// `videocrop`'s edges for keeping `[x, y, w, h]` of the stream, set once its size is known.
fn crop_edges(element: &gst::Element, crop: [u32; 4]) {
    let Some(pad) = element.static_pad("sink") else {
        return;
    };
    let element = element.downgrade();
    pad.add_probe(gst::PadProbeType::EVENT_DOWNSTREAM, move |_, info| {
        if let Some(gst::PadProbeData::Event(event)) = info.data.as_ref()
            && let gst::EventView::Caps(caps) = event.view()
            && let Some(s) = caps.caps().structure(0)
            && let (Ok(width), Ok(height)) = (s.get::<i32>("width"), s.get::<i32>("height"))
            && let Some(element) = element.upgrade()
        {
            let [x, y, w, h] = crop.map(|v| v as i32);
            element.set_property("left", x.clamp(0, width));
            element.set_property("top", y.clamp(0, height));
            element.set_property("right", (width - x - w).clamp(0, width));
            element.set_property("bottom", (height - y - h).clamp(0, height));
        }
        gst::PadProbeReturn::Ok
    });
}

/// A sample's RGBA pixels, rows packed (`width * 4` bytes each).
pub fn rgba_of(sample: &gst::Sample, width: u32, height: u32) -> Option<Vec<u8>> {
    let buffer = sample.buffer()?;
    let map = buffer.map_readable().ok()?;
    let info = sample
        .caps()
        .and_then(|caps| gstreamer_video::VideoInfo::from_caps(caps).ok());
    let stride = info
        .as_ref()
        .and_then(|i| i.stride().first().copied())
        .map_or(width as usize * 4, |s| s.max(0) as usize);
    let row = width as usize * 4;
    if stride == row {
        return map.get(..row * height as usize).map(<[u8]>::to_vec);
    }
    let mut out = Vec::with_capacity(row * height as usize);
    for y in 0..height as usize {
        out.extend_from_slice(map.get(y * stride..y * stride + row)?);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_owns_stream_fd_and_sends_close_only_when_dropped() {
        use std::io::Read;
        use std::os::unix::net::UnixStream;
        let (fd, mut peer) = UnixStream::pair().unwrap();
        peer.set_nonblocking(true).unwrap();
        let (tx, rx) = flume::bounded(1);
        let source = Source::PipeWire {
            session: Stream::new(7, Some(fd.into()), None, None, Some(tx)),
            crop: None,
        };
        // Native-handle unit coverage, not an OS portal consent/stream proof.
        assert!(rx.try_recv().is_err());
        assert_eq!(peer.read(&mut [0]).unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
        drop(source);
        assert_eq!(rx.try_recv(), Ok(()));
        assert_eq!(peer.read(&mut [0]).unwrap(), 0);
    }

    #[test]
    fn cached_frame_uses_current_masks_without_destroying_source_pixels() {
        init_gstreamer().unwrap();
        let pixels = vec![10, 20, 30, 255, 40, 50, 60, 255];
        let mut recorder = Recorder {
            pipeline: gst::Pipeline::new(),
            candidates: gst_app::AppSink::builder().build(),
            candidate: (2, 1),
            video: false,
            first_frame_ms: Arc::new(AtomicU64::new(u64::MAX)),
            last: Some(pixels.clone()),
        };
        let first = recorder.candidate(|rgba, _, _| rgba[..4].fill(0)).unwrap();
        assert_eq!(&first.rgba[..4], &[0; 4]);
        let second = recorder.candidate(|rgba, _, _| rgba[4..].fill(0)).unwrap();
        assert_eq!(&second.rgba[..4], &pixels[..4]);
        assert_eq!(&second.rgba[4..], &[0; 4]);
        assert_eq!(recorder.candidate(|_, _, _| {}).unwrap().rgba, pixels);
        assert_eq!(recorder.last.as_ref().unwrap(), &pixels);
    }

    #[test]
    fn x11_sources_record_the_rect_they_are_given() {
        let launch = source_launch(&Source::X11 {
            rect: [100.0, 50.0, 800.0, 600.0],
        });
        assert!(
            launch.contains("startx=100 starty=50 endx=899 endy=649"),
            "{launch}"
        );
    }

    fn synthetic_available() -> bool {
        if init_gstreamer().is_err()
            || encoder().is_none()
            || gst::ElementFactory::find("videotestsrc").is_none()
        {
            eprintln!("Skipping synthetic GStreamer test: test source or encoder unavailable");
            return false;
        }
        true
    }

    fn recorder(movie: &Path) -> Recorder {
        let _ = std::fs::remove_file(movie);
        Recorder::new(
            &Source::Test,
            (640, 360),
            (320, 180),
            Some(movie),
            Instant::now(),
        )
        .unwrap()
    }

    fn wait_for_candidate(recorder: &mut Recorder) {
        let deadline = Instant::now() + Duration::from_secs(4);
        while recorder.candidate(|_, _, _| {}).is_none() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            recorder.last.is_some(),
            "the synthetic pipeline never produced a frame"
        );
    }

    #[test]
    fn finished_movie_survives_a_stuck_candidate_branch() {
        if !synthetic_available() {
            return;
        }
        let movie =
            std::env::temp_dir().join(format!("apparent-eos-test-{}.mp4", std::process::id()));
        let mut recorder = recorder(&movie);
        wait_for_candidate(&mut recorder);
        let sink = recorder.candidates.static_pad("sink").unwrap();
        let _blocked = sink.add_probe(
            gst::PadProbeType::BLOCK | gst::PadProbeType::BUFFER,
            |_, _| gst::PadProbeReturn::Ok,
        );
        std::thread::sleep(Duration::from_millis(200));
        let started = Instant::now();
        assert!(
            recorder.finish(|message| panic!("unexpected finish error: {message}")),
            "movie branch finished but candidate branch held pipeline EOS"
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(std::fs::metadata(&movie).unwrap().len() > 1024);
        std::fs::remove_file(movie).unwrap();
    }

    #[test]
    fn unfinished_movie_is_not_kept_when_encoder_stalls() {
        if !synthetic_available() {
            return;
        }
        let movie = std::env::temp_dir().join(format!(
            "apparent-encoder-eos-test-{}.mp4",
            std::process::id()
        ));
        let mut recorder = recorder(&movie);
        wait_for_candidate(&mut recorder);
        let sink = recorder
            .pipeline
            .by_name("encoder")
            .unwrap()
            .static_pad("sink")
            .unwrap();
        let _blocked = sink.add_probe(
            gst::PadProbeType::BLOCK | gst::PadProbeType::BUFFER,
            |_, _| gst::PadProbeReturn::Ok,
        );
        std::thread::sleep(Duration::from_millis(200));
        assert!(
            !recorder.finish(|_| {}),
            "an encoder stall cannot finish the movie"
        );
        let _ = std::fs::remove_file(movie);
    }
}
