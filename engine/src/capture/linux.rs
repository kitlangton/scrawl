//! Linux X11 source discovery, GStreamer readiness and native movie execution.
//! Capture admission, authenticated journals, control and publication remain in `capture.rs`.

use super::{
    CaptureOwnerPermit, Journal, LiveCapturePermit, Shared, VideoReadback, finalize, operation_dir,
    persist, persist_journal, read_journal, remove_unsealed, same_source, sync_control,
};
use apparent_capture_engine::new_id;
use gstreamer as gst;
use serde_json::{Value, json};
use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::sync::{Arc, atomic::Ordering};
use std::thread;
use std::time::{Duration, Instant};
use x11rb::connection::Connection;
use x11rb::protocol::randr::ConnectionExt as RandrExt;

const START_LIMIT: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub(super) struct Source {
    pub(super) value: Value,
    pub(super) x: i16,
    pub(super) y: i16,
    pub(super) width: u16,
    pub(super) height: u16,
}

pub(super) fn available_sources() -> Result<(String, Vec<Source>), &'static str> {
    if std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_none() {
        return Err("unavailable");
    }
    let (connection, screen_num) = x11rb::connect(None).map_err(|_| "unavailable")?;
    let screen = connection
        .setup()
        .roots
        .get(screen_num)
        .ok_or("unavailable")?;
    let resources = connection
        .randr_get_screen_resources_current(screen.root)
        .map_err(|_| "unavailable")?
        .reply()
        .map_err(|_| "unavailable")?;
    let generation = new_id().map_err(|_| "internal")?;
    let mut sources = Vec::new();
    for output in resources.outputs {
        let info = connection
            .randr_get_output_info(output, resources.config_timestamp)
            .map_err(|_| "unavailable")?
            .reply()
            .map_err(|_| "unavailable")?;
        if info.connection != x11rb::protocol::randr::Connection::CONNECTED || info.crtc == 0 {
            continue;
        }
        let crtc = connection
            .randr_get_crtc_info(info.crtc, resources.config_timestamp)
            .map_err(|_| "unavailable")?
            .reply()
            .map_err(|_| "unavailable")?;
        if crtc.width == 0 || crtc.height == 0 {
            continue;
        }
        let display_id = u64::from(output);
        // Keep integer one on the wire: JSON.stringify in the SDK canonicalizes
        // both 1 and 1.0 to 1, and source equality binds the exact descriptor.
        let output_name = String::from_utf8_lossy(&info.name).into_owned();
        let display_bounds =
            json!({"x":crtc.x,"y":crtc.y,"width":crtc.width,"height":crtc.height,"scale":1});
        sources.push(Source { value: json!({"id":format!("display:{display_id}"),"generation":generation,"kind":"display","displayId":display_id,"bounds":display_bounds,"name":output_name}), x:crtc.x, y:crtc.y, width:crtc.width, height:crtc.height });
        if crtc.width >= 2 && crtc.height >= 2 {
            let (x, y) = (
                i32::from(crtc.x) + i32::from(crtc.width / 4),
                i32::from(crtc.y) + i32::from(crtc.height / 4),
            );
            let (Ok(x), Ok(y)) = (i16::try_from(x), i16::try_from(y)) else {
                continue;
            };
            let (width, height) = (crtc.width / 2, crtc.height / 2);
            let bounds = json!({"x":x,"y":y,"width":width,"height":height,"scale":1});
            sources.push(Source { value: json!({"id":format!("area:{display_id}:{x}:{y}:{width}:{height}"),"generation":generation,"kind":"area","displayId":display_id,"bounds":bounds,"name":output_name}), x, y, width, height });
        }
    }
    if sources.is_empty() {
        return Err("unavailable");
    }
    Ok((generation, sources))
}
pub(super) fn native_ready(sources: &[Source]) -> bool {
    if std::env::var_os("APPARENT_CAPTURE_TEST_READINESS_UNAVAILABLE").is_some() {
        return false;
    }
    if sources.is_empty() || gst::init().is_err() {
        return false;
    }
    let base = [
        "ximagesrc",
        "videoconvert",
        "h264parse",
        "mp4mux",
        "fdsink",
        "decodebin",
    ];
    let encoder = ["x264enc", "openh264enc"]
        .iter()
        .any(|name| gst::ElementFactory::find(name).is_some());
    let decoder = ["avdec_h264", "openh264dec"]
        .iter()
        .any(|name| gst::ElementFactory::find(name).is_some());
    base.iter()
        .all(|name| gst::ElementFactory::find(name).is_some())
        && encoder
        && decoder
}
fn recording_start(
    source: &Source,
    movie: &File,
    shared: &Shared,
    epoch: Instant,
) -> Result<apparent_capture_engine::linux::Recorder, &'static str> {
    let native = apparent_capture_engine::linux::Source::X11 {
        rect: [
            f64::from(source.x),
            f64::from(source.y),
            f64::from(source.width),
            f64::from(source.height),
        ],
    };
    let recorder = apparent_capture_engine::linux::Recorder::new_with_file(
        &native,
        video_dimensions(source),
        (64, 64),
        movie,
        epoch,
    )
    .map_err(|_| "unavailable")?;
    let deadline = Instant::now() + START_LIMIT;
    while recorder.first_frame_ms().is_none() {
        let _ = sync_control(shared)?;
        if shared.cancel.load(Ordering::Acquire) {
            recorder.stop();
            return Err("cancelled");
        }
        if Instant::now() >= deadline {
            recorder.stop();
            return Err("deadline_exceeded");
        }
        if recorder.error().is_some() {
            recorder.stop();
            return Err("unavailable");
        }
        thread::sleep(Duration::from_millis(20));
    }
    Ok(recorder)
}

pub(super) fn video_dimensions(source: &Source) -> (u32, u32) {
    apparent_capture_engine::capture_dimensions(
        [f64::from(source.width), f64::from(source.height), 1.0],
        f64::from(source.width.max(source.height)),
        1.0,
    )
}
pub(super) fn worker(
    shared: Arc<Shared>,
    source: Source,
    permit: LiveCapturePermit,
    _owner: CaptureOwnerPermit,
) {
    let result = (|| -> Result<(), &'static str> {
        permit.check().map_err(|_| "internal")?;
        let capture_id = shared
            .journal
            .lock()
            .map_err(|_| "internal")?
            .capture_id
            .clone();
        let opdir = operation_dir(&shared)?;
        let movie = opdir
            .file("recording.mp4", true)
            .map_err(|_| "source_mismatch")?;
        let movie_identity = movie.metadata().map_err(|_| "internal")?;
        {
            let mut j = shared.journal.lock().map_err(|_| "internal")?;
            j.partial_movie = Some(
                json!({"device":movie_identity.dev().to_string(),"inode":movie_identity.ino().to_string()}),
            );
        }
        persist(&shared).map_err(|_| "internal")?;
        let recorder = recording_start(&source, &movie, &shared, shared.epoch)?;
        let video_origin_ns = recorder.video_origin_ns().ok_or("invalid_materials")?;
        {
            let mut j = shared.journal.lock().map_err(|_| "internal")?;
            j.state = "recording".into();
        }
        persist(&shared).map_err(|_| "internal")?;
        let mut source_check = Instant::now() + Duration::from_millis(250);
        while !shared.stop.load(Ordering::Acquire) && !shared.cancel.load(Ordering::Acquire) {
            let control = sync_control(&shared)?;
            if control.cancel_requested {
                break;
            }
            if recorder.error().is_some() {
                recorder.stop();
                return Err("unavailable");
            }
            thread::sleep(Duration::from_millis(30));
            if Instant::now() >= source_check {
                source_check = Instant::now() + Duration::from_millis(250);
                let current = available_sources().ok().and_then(|(_, values)| {
                    values.into_iter().find(|value| same_source(value, &source))
                });
                if current.is_none() {
                    recorder.stop();
                    drop(movie);
                    let j = shared.journal.lock().map_err(|_| "internal")?;
                    let _ = remove_unsealed(&shared.directory, &j);
                    return Err("source_changed");
                }
            }
        }
        if shared.cancel.load(Ordering::Acquire) {
            recorder.stop();
            drop(movie);
            let prior = shared.journal.lock().map_err(|_| "internal")?;
            remove_unsealed(&shared.directory, &prior)?;
            let cleanup_binding = prior.clone();
            drop(prior);
            pause_test_cancel_after_unsealed_cleanup(&cleanup_binding);
            let mut j = shared.journal.lock().map_err(|_| "internal")?;
            j.state = "cancelled".into();
            drop(j);
            persist(&shared).map_err(|_| "internal")?;
            return Ok(());
        }
        let capture_duration = shared
            .epoch
            .elapsed()
            .as_millis()
            .min(9_007_199_254_740_991) as u64;
        if !recorder.finish(|_| {}) {
            return Err("invalid_materials");
        }
        if let Ok(value) = std::env::var("APPARENT_CAPTURE_TEST_FINALIZATION_DELAY_MS") {
            if let Ok(ms) = value.parse::<u64>() {
                thread::sleep(Duration::from_millis(ms.min(5_000)));
            }
        }
        movie.sync_all().map_err(|_| "invalid_materials")?;
        let manifest = finalize(&shared, &movie, &source, capture_duration, video_origin_ns)?;
        let mut j = shared.journal.lock().map_err(|_| "internal")?;
        let mut ready = j.clone();
        ready.state = if ready.cancel_requested {
            "cancelled"
        } else {
            "materials-ready"
        }
        .into();
        ready.materials_id = Some(
            manifest["key"]["materialsId"]
                .as_str()
                .unwrap_or_default()
                .to_owned(),
        );
        ready.lease_id = Some(manifest["leaseId"].as_str().unwrap_or_default().to_owned());
        ready.materials = Some(manifest);
        // Do not expose the terminal state to inspect until its recovery record
        // has reached disk. The journal mutex also excludes a concurrent cancel.
        persist_journal(&shared, &ready).map_err(|_| "internal")?;
        *j = read_journal(&shared.directory, &ready.capture_id, &ready.owner_token)?;
        drop(j);
        let _ = capture_id;
        Ok(())
    })();
    if let Err(code) = result {
        let durable_cancel = shared
            .journal
            .lock()
            .ok()
            .and_then(|j| read_journal(&shared.directory, &j.capture_id, &j.owner_token).ok())
            .is_some_and(|j| j.cancel_requested);
        let cleanup_failed = match shared.journal.lock() {
            Ok(j) if j.materials.is_none() => remove_unsealed(&shared.directory, &j).is_err(),
            Ok(_) => false,
            Err(_) => true,
        };
        if cleanup_failed {
            // Keep the durable active journal and cancellation intent so cold
            // inspection can retry after the original identity is restored.
            drop(permit);
            return;
        }
        if let Ok(mut j) = shared.journal.lock() {
            if code == "cancelled" || durable_cancel {
                j.state = "cancelled".into();
                j.error = None;
            } else {
                j.state = "failed".into();
                j.error = Some(code.into());
            }
        }
        let _ = persist(&shared);
    }
    drop(permit);
}

fn pause_test_cancel_after_unsealed_cleanup(journal: &Journal) {
    let Ok(target) = std::env::var("APPARENT_CAPTURE_TEST_CANCEL_CLEANUP_BUNDLE_ID") else {
        return;
    };
    if journal.request["bundleId"].as_str() == Some(target.as_str()) {
        thread::sleep(Duration::from_secs(5));
    }
}

pub(super) fn readback_movie(movie: &File) -> Result<VideoReadback, &'static str> {
    let timing = apparent_capture_engine::linux::media::validate_file(movie)
        .map_err(|_| "invalid_materials")?;
    Ok(VideoReadback {
        duration_ms: timing.duration_ms,
        first_pts_ns: timing.first_pts_ns,
    })
}
