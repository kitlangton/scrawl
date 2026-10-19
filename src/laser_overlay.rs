use std::ffi::c_void;
use std::mem::size_of;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use core_graphics_types::geometry::CGSize;
use core_video::base::{CVOptionFlags, CVTimeStamp};
use core_video::display_link::{CVDisplayLink, CVDisplayLinkRef};
use core_video::r#return::CVReturn;
use metal::{
    MTLLoadAction, MTLPixelFormat, MTLPrimitiveType, MTLResourceOptions, MTLStoreAction,
    MetalLayer, RenderPassDescriptor, RenderPipelineDescriptor, RenderPipelineState,
};
use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSPanel, NSScreen, NSStatusWindowLevel, NSView,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect};
use objc2_quartz_core::CALayer;

use crate::capture::SharedCaptureTelemetry;
use crate::sketch::{slice_laser_window, smooth_ink_points, smooth_laser_points};

const TAIL_DURATION_SEC: f32 = 0.42;
const MAX_SEGMENTS: usize = 1536;
const MAX_PINS: usize = 64;

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct LaserUniforms {
    resolution: [f32; 2],
    scale: f32,
    now_sec: f32,
    tail_duration: f32,
    tip_active: f32,
    tip_pos: [f32; 2],
    segment_count: u32,
    pin_count: u32,
    constrain_rect: f32,
    _pad1: f32,
    target_rect: [f32; 4],
    glow: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct PathSegment {
    a: [f32; 2],
    b: [f32; 2],
    w0: f32,
    w1: f32,
    kind: f32,
    width: f32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct PinMarker {
    pos: [f32; 2],
    radius: f32,
    _pad: f32,
}

pub struct MetalLaserOverlay {
    window: Retained<NSPanel>,
    display_link: CVDisplayLink,
    display_link_context: *const SharedMetalOverlay,
}

impl MetalLaserOverlay {
    /// Opens over the screen holding the centre of `on` (global points, top-left origin, like
    /// capture targets), else the main screen.
    pub fn open(
        telemetry: Arc<SharedCaptureTelemetry>,
        on: Option<[f64; 4]>,
    ) -> Result<Self, String> {
        let mtm = MainThreadMarker::new().ok_or("Metal overlay requires the main thread")?;
        let screens = NSScreen::screens(mtm);
        let primary_h = screens.firstObject().map_or(0.0, |s| s.frame().size.height);
        // AppKit frames have a bottom-left origin on the primary screen; capture frames top-left.
        let top_left = |f: NSRect| {
            [
                f.origin.x,
                primary_h - f.origin.y - f.size.height,
                f.size.width,
                f.size.height,
            ]
        };
        let screen = on
            .and_then(|[x, y, w, h]| {
                let (cx, cy) = (x + w / 2.0, y + h / 2.0);
                screens.iter().find(|s| {
                    let [sx, sy, sw, sh] = top_left(s.frame());
                    cx >= sx && cx < sx + sw && cy >= sy && cy < sy + sh
                })
            })
            .or_else(|| NSScreen::mainScreen(mtm))
            .or_else(|| screens.firstObject())
            .ok_or("No screen available for Metal overlay")?;
        let frame = screen.frame();
        let scale = (screen.backingScaleFactor() as f32).max(1.0);
        let [ox, oy, _, _] = top_left(frame);

        let shared = Arc::new(SharedMetalOverlay::new(
            telemetry,
            [ox as f32, oy as f32],
            frame.size.width as f32,
            frame.size.height as f32,
            scale,
        )?);

        let window = NSPanel::initWithContentRect_styleMask_backing_defer(
            mtm.alloc(),
            frame,
            NSWindowStyleMask::Borderless
                | NSWindowStyleMask::UtilityWindow
                | NSWindowStyleMask::NonactivatingPanel
                | NSWindowStyleMask::FullSizeContentView,
            NSBackingStoreType::Buffered,
            false,
        );
        let view =
            NSView::initWithFrame(mtm.alloc(), NSRect::new(NSPoint::new(0.0, 0.0), frame.size));
        view.setWantsLayer(true);
        let metal_layer = shared.layer_ptr();
        let cocoa_layer = unsafe { &*metal_layer.cast::<CALayer>() };
        cocoa_layer.setOpaque(false);
        view.setLayer(Some(cocoa_layer));
        window.setContentView(Some(&view));
        window.setBackgroundColor(Some(&NSColor::clearColor()));
        window.setOpaque(false);
        window.setIgnoresMouseEvents(true);
        window.setHasShadow(false);
        window.setHidesOnDeactivate(false);
        window.setCanHide(false);
        window.setLevel(NSStatusWindowLevel + 1);
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::CanJoinAllApplications
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        unsafe { window.setReleasedWhenClosed(false) };
        window.setFrame_display(frame, true);
        // Present Frame 0 synchronously so the cutout and border are already on screen
        // before TargetPickerWindow closes (zero 1-frame blink).
        shared.draw();
        window.orderFrontRegardless();

        let display_link = CVDisplayLink::from_active_cg_displays()
            .map_err(|status| format!("Could not create CVDisplayLink: {status}"))?;
        let display_link_context = Arc::into_raw(shared);
        unsafe {
            display_link.set_output_callback(
                display_link_callback,
                display_link_context.cast_mut().cast::<c_void>(),
            )
        }
        .map_err(|status| format!("Could not configure CVDisplayLink callback: {status}"))?;
        let _ = display_link.start();

        Ok(Self {
            window,
            display_link,
            display_link_context,
        })
    }

    pub fn window_number(&self) -> u32 {
        self.window.windowNumber().max(0) as u32
    }
}

impl Drop for MetalLaserOverlay {
    fn drop(&mut self) {
        let _ = self.display_link.stop();
        unsafe { drop(Arc::from_raw(self.display_link_context)) };
        self.window.orderOut(None);
        self.window.close();
    }
}

extern "C" fn display_link_callback(
    _display_link: CVDisplayLinkRef,
    _now: *const CVTimeStamp,
    _output_time: *const CVTimeStamp,
    _flags_in: CVOptionFlags,
    _flags_out: *mut CVOptionFlags,
    context: *mut c_void,
) -> CVReturn {
    if !context.is_null() {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let overlay = unsafe { &*context.cast::<SharedMetalOverlay>() };
            overlay.draw();
        }));
    }
    0
}

struct SharedMetalOverlay {
    telemetry: Arc<SharedCaptureTelemetry>,
    /// The screen's top-left in global points: telemetry is global, the shader draws locally.
    origin: [f32; 2],
    renderer: Mutex<MetalLaserRenderer>,
}

unsafe impl Send for SharedMetalOverlay {}
unsafe impl Sync for SharedMetalOverlay {}

struct MetalLaserRenderer {
    layer: MetalLayer,
    device: metal::Device,
    command_queue: metal::CommandQueue,
    pipeline: RenderPipelineState,
    width: f32,
    height: f32,
    scale: f32,
}

impl SharedMetalOverlay {
    fn new(
        telemetry: Arc<SharedCaptureTelemetry>,
        origin: [f32; 2],
        width: f32,
        height: f32,
        scale: f32,
    ) -> Result<Self, String> {
        let device = metal::Device::system_default().ok_or("Metal is unavailable")?;
        let layer = MetalLayer::new();
        layer.set_device(&device);
        layer.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
        layer.set_opaque(false);
        layer.set_presents_with_transaction(false);
        layer.set_display_sync_enabled(true);
        layer.set_contents_scale(f64::from(scale));
        layer.set_drawable_size(CGSize::new(
            f64::from(width * scale),
            f64::from(height * scale),
        ));

        let library = device
            .new_library_with_data(include_bytes!(concat!(
                env!("OUT_DIR"),
                "/laser_overlay.metallib"
            )))
            .map_err(|e| format!("Could not load laser shader library: {e}"))?;
        let vertex = library
            .get_function("laser_vertex", None)
            .map_err(|e| format!("Could not load laser_vertex: {e}"))?;
        let fragment = library
            .get_function("laser_fragment", None)
            .map_err(|e| format!("Could not load laser_fragment: {e}"))?;

        let descriptor = RenderPipelineDescriptor::new();
        descriptor.set_vertex_function(Some(&vertex));
        descriptor.set_fragment_function(Some(&fragment));
        let attachment = descriptor
            .color_attachments()
            .object_at(0)
            .ok_or("Missing color attachment")?;
        attachment.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
        attachment.set_blending_enabled(false);

        let pipeline = device
            .new_render_pipeline_state(&descriptor)
            .map_err(|e| format!("Could not create laser pipeline: {e}"))?;

        Ok(Self {
            telemetry,
            origin,
            renderer: Mutex::new(MetalLaserRenderer {
                command_queue: device.new_command_queue(),
                device,
                layer,
                pipeline,
                width,
                height,
                scale,
            }),
        })
    }

    fn layer_ptr(&self) -> *const metal::MetalLayerRef {
        self.renderer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .layer
            .as_ref()
    }

    fn draw(&self) {
        let is_active = self.telemetry.active.load(Ordering::Acquire);
        let is_selecting = self.telemetry.selecting.load(Ordering::Acquire);
        let is_device_pending = self.telemetry.capture_mode.load(Ordering::Acquire) == 3;
        if !is_active && !is_selecting && !is_device_pending {
            return;
        }
        let now_ms = self.telemetry.elapsed_ms();
        let now_sec = if is_selecting {
            0.0
        } else {
            now_ms as f32 / 1_000.0
        };
        let picker_fade = if is_selecting {
            let elapsed = self
                .telemetry
                .picker_started_at
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .elapsed()
                .as_secs_f32();
            let t = (elapsed / 0.15).clamp(0.0, 1.0);
            1.0 - (1.0 - t).powi(3)
        } else {
            1.0
        };
        let tail_ms = (TAIL_DURATION_SEC * 1_000.0) as u64;
        let min_laser_ms = now_ms.saturating_sub(tail_ms);

        let picker_sel = if is_selecting {
            *self
                .telemetry
                .picker_rect
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
        } else {
            None
        };
        let target_frame = *self
            .telemetry
            .target_frame
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let (tx, ty, tw, th, is_constrained, mode_code) = if is_selecting {
            if let Some(([rx, ry, rw, rh], kind)) = picker_sel {
                (rx, ry, rw.max(1.0), rh.max(1.0), true, f32::from(kind))
            } else {
                (0.0, 0.0, 1.0, 1.0, false, -picker_fade.max(0.001))
            }
        } else {
            let tx = target_frame.x as f32;
            let ty = target_frame.y as f32;
            let tw = (target_frame.width as f32).max(1.0);
            let th = (target_frame.height as f32).max(1.0);
            let mode = self.telemetry.capture_mode.load(Ordering::Acquire) as f32;
            let constrained = mode > 0.5 && tx > -10_000.0;
            (tx, ty, tw, th, constrained, mode)
        };
        let [ox, oy] = self.origin;
        let (tx, ty) = (tx - ox, ty - oy);
        let to_screen = |nx: f32, ny: f32| -> [f32; 2] { [tx + nx * tw, ty + ny * th] };

        let committed_strokes = self
            .telemetry
            .strokes
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let active_laser = self
            .telemetry
            .active_laser
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();
        let active_ink = self
            .telemetry
            .live_ink
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone();

        let mut strokes: Vec<(crate::model::Stroke, bool)> =
            committed_strokes.into_iter().map(|s| (s, false)).collect();
        if let Some(laser) = active_laser {
            strokes.push((laser, true));
        }
        if let Some(ink) = active_ink {
            strokes.push((ink, true));
        }

        let mut segments = Vec::with_capacity(256);
        let mut pins = Vec::new();

        for (stroke, is_live) in &strokes {
            if stroke.pin.is_some() {
                if let Some([nx, ny]) = stroke.normalized_points().first().copied()
                    && pins.len() < MAX_PINS
                {
                    pins.push(PinMarker {
                        pos: to_screen(nx, ny),
                        radius: 11.5,
                        _pad: 0.0,
                    });
                }
                continue;
            }
            if stroke.laser {
                if stroke.t1 < min_laser_ms {
                    continue;
                }
                let raw = slice_laser_window(&stroke.timed_points(), now_ms, tail_ms, to_screen);
                let smoothed = smooth_laser_points(&raw);
                for pair in smoothed.windows(2) {
                    if segments.len() >= MAX_SEGMENTS {
                        break;
                    }
                    let (a, w0) = pair[0];
                    let (b, w1) = pair[1];
                    segments.push(PathSegment {
                        a,
                        b,
                        w0,
                        w1,
                        kind: 0.0,
                        width: 3.0,
                    });
                }
            } else {
                // Keep live Shift strokes at 100% while held, then hold for 650ms and fade out over 650ms.
                let fade_alpha = if *is_live {
                    1.0
                } else {
                    let age_ms = now_ms.saturating_sub(stroke.t1);
                    if age_ms >= 1_300 {
                        continue;
                    } else if age_ms <= 650 {
                        1.0
                    } else {
                        let t = ((age_ms - 650) as f32 / 650.0).clamp(0.0, 1.0);
                        (1.0 - t) * (1.0 - t)
                    }
                };
                let pts: Vec<[f32; 2]> = stroke
                    .normalized_points()
                    .into_iter()
                    .map(|[nx, ny]| to_screen(nx, ny))
                    .collect();
                let smoothed = smooth_ink_points(&pts);
                for pair in smoothed.windows(2) {
                    if segments.len() >= MAX_SEGMENTS {
                        break;
                    }
                    segments.push(PathSegment {
                        a: pair[0].0,
                        b: pair[1].0,
                        w0: pair[0].1,
                        w1: pair[1].1,
                        kind: 1.0,
                        width: fade_alpha,
                    });
                }
            }
        }

        let shift_held = self.telemetry.shift_held.load(Ordering::Acquire);
        let pointer = {
            let [px, py] = *self
                .telemetry
                .pointer_screen
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            [px - ox, py - oy]
        };

        if segments.is_empty() {
            segments.push(PathSegment::default());
        }
        if pins.is_empty() {
            pins.push(PinMarker::default());
        }

        let renderer = self
            .renderer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        objc2::rc::autoreleasepool(|_| {
            let Some(drawable) = renderer.layer.next_drawable() else {
                return;
            };
            let pass = RenderPassDescriptor::new();
            let attachment = pass.color_attachments().object_at(0).unwrap();
            attachment.set_texture(Some(drawable.texture()));
            attachment.set_load_action(MTLLoadAction::Clear);
            attachment.set_clear_color(metal::MTLClearColor::new(0.0, 0.0, 0.0, 0.0));
            attachment.set_store_action(MTLStoreAction::Store);

            let uniforms = LaserUniforms {
                resolution: [
                    renderer.width * renderer.scale,
                    renderer.height * renderer.scale,
                ],
                scale: renderer.scale,
                now_sec,
                tail_duration: picker_fade,
                tip_active: if is_active && shift_held { 1.0 } else { 0.0 },
                tip_pos: pointer,
                segment_count: if !is_active || (segments.len() == 1 && segments[0].width == 0.0) {
                    0
                } else {
                    segments.len().min(MAX_SEGMENTS) as u32
                },
                pin_count: if !is_active || (pins.len() == 1 && pins[0].radius == 0.0) {
                    0
                } else {
                    pins.len().min(MAX_PINS) as u32
                },
                constrain_rect: if is_constrained { 1.0 } else { 0.0 },
                _pad1: mode_code,
                target_rect: [tx, ty, tw, th],
                glow: [
                    f32::from(u8::from(
                        is_active && self.telemetry.start_glow.load(Ordering::Acquire),
                    )),
                    0.0,
                    0.0,
                    0.0,
                ],
            };

            let cmd = renderer.command_queue.new_command_buffer();
            let enc = cmd.new_render_command_encoder(pass);
            enc.set_render_pipeline_state(&renderer.pipeline);
            enc.set_fragment_bytes(
                0,
                size_of::<LaserUniforms>() as u64,
                (&raw const uniforms).cast(),
            );
            // Up to MAX_SEGMENTS * 32 B = 48 KB, past the 4 KB `set_fragment_bytes` limit (128
            // segments), so segments go in a buffer. Metal retains it until the command buffer completes.
            let segment_buffer = renderer.device.new_buffer_with_data(
                segments.as_ptr().cast(),
                (segments.len() * size_of::<PathSegment>()) as u64,
                MTLResourceOptions::StorageModeShared,
            );
            enc.set_fragment_buffer(1, Some(&segment_buffer), 0);
            enc.set_fragment_bytes(
                2,
                (pins.len() * size_of::<PinMarker>()) as u64,
                pins.as_ptr().cast(),
            );
            enc.draw_primitives(MTLPrimitiveType::Triangle, 0, 3);
            enc.end_encoding();
            cmd.present_drawable(drawable);
            cmd.commit();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `setFragmentBytes` aborts in the driver past 4 KB.
    const SET_BYTES_LIMIT: usize = 4096;

    /// Uniforms and pins still go through `set_fragment_bytes`; segments don't fit and use a buffer.
    #[test]
    fn inline_fragment_bytes_fit_the_metal_limit() {
        assert!(size_of::<LaserUniforms>() <= SET_BYTES_LIMIT);
        assert!(MAX_PINS * size_of::<PinMarker>() <= SET_BYTES_LIMIT);
    }

    /// The shader must load on the oldest macOS Scrawl supports (`LSMinimumSystemVersion` 26.0).
    /// Without `-mmacosx-version-min`, `xcrun metal` targets the build machine's macOS and the
    /// overlay (ink, laser, edge glow) silently fails to open on older systems.
    #[test]
    fn shader_targets_the_minimum_supported_macos() {
        let library = include_bytes!(concat!(env!("OUT_DIR"), "/laser_overlay.metallib"));
        let text = String::from_utf8_lossy(library);
        let marker = "-apple-macosx";
        let at = text.find(marker).expect("metallib names its target");
        let major: u32 = text[at + marker.len()..]
            .split('.')
            .next()
            .and_then(|v| v.parse().ok())
            .expect("target has a major version");
        assert!(major <= 26, "laser shader targets macOS {major}, newer than 26.0");
    }
}
