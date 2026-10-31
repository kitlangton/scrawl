//! Native AppKit `NSPanel` + `CAMetalLayer` + Metal renderer for the Scrawl recording pill.
//!
//! Replaces the GPUI `HudPillWindow` with a single fixed-size `NSPanel` (`640 × 84 pt` at `y = 34`
//! below the menu bar) that never resizes or moves during morphs. Supports two backdrop looks via
//! `pillStyle: "black" | "glass"`:
//! - `"black"`: Analytic SDF rounded-rect silhouette + shadow + specular border in Metal.
//! - `"glass"`: AppKit's `NSGlassEffectView` (inside `NSGlassEffectContainerView`) driven on the
//!   same spring silhouette inside a shared `CATransaction` with `CAMetalLayer` (`presentsWithTransaction`).

pub mod atlas;
pub mod model;
mod types;

pub use self::types::{PillAction, PillSnapshot, PillStyle};

use std::ffi::c_void;
use std::mem::size_of;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use core_graphics_types::geometry::CGSize;
use core_video::base::{CVOptionFlags, CVTimeStamp};
use core_video::display_link::{CVDisplayLink, CVDisplayLinkRef};
use core_video::r#return::CVReturn;
use metal::foreign_types::ForeignType;
use metal::{
    MTLLoadAction, MTLPixelFormat, MTLPrimitiveType, MTLStoreAction, MTLTextureUsage, MetalLayer,
    RenderPassDescriptor, RenderPipelineDescriptor, RenderPipelineState, TextureDescriptor,
};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSEvent, NSPanel, NSScreen, NSStatusWindowLevel, NSView,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use objc2_quartz_core::{CALayer, CATransaction};

use self::atlas::{GlyphAtlas, IconKind};
use self::model::{CHIP_H, CHIP_PX, ElemId, FrameState, PANEL_H, PANEL_W, PillModel, Pres};
use crate::ui::{SelectionMode, motion};

const OFFSCREEN_W_PT: f32 = 960.0;
const OFFSCREEN_H_PT: f32 = 256.0;
const TILE_PAD_PT: f32 = 20.0;

unsafe extern "C" {
    static _dispatch_main_q: c_void;
    fn dispatch_async_f(
        queue: *const c_void,
        context: *mut c_void,
        work: extern "C" fn(*mut c_void),
    );
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct PillUniforms {
    panel_size: [f32; 2],
    canvas_size: [f32; 2],
    scale: f32,
    elem_count: u32,
    is_completed: u32,
    has_ghost: u32,
    shape_rect: [f32; 4],
    shape_meta: [f32; 4],
    ghost_rect: [f32; 4],
    ghost_meta: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct TileUniforms {
    atlas_size: [f32; 2],
    canvas_size: [f32; 2],
    scale: f32,
    _pad0: f32,
    _pad1: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct TilePrim {
    rect: [f32; 4],
    uv_rect: [f32; 4],
    color: [f32; 4],
    kind: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct BlurBox {
    tile_rect: [f32; 4],
    sigma_pt: f32,
    _pad0: f32,
    _pad1: [f32; 2],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CompositeElem {
    screen_rect: [f32; 4],
    tile_rect: [f32; 4],
    base_size: [f32; 2],
    scale: f32,
    alpha: f32,
    sigma_pt: f32,
    pad_pt: f32,
    _pad0: [f32; 2],
}

struct PillViewIvars {
    action_tx: flume::Sender<PillAction>,
}

define_class!(
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = PillViewIvars]
    struct ScrawlPillView;

    unsafe impl NSObjectProtocol for ScrawlPillView {}

    impl ScrawlPillView {
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(canBecomeKeyView))]
        fn can_become_key_view(&self) -> bool {
            true
        }

        #[unsafe(method(cancelOperation:))]
        fn cancel_operation(&self, _sender: Option<&AnyObject>) {
            let _ = self.ivars().action_tx.try_send(PillAction::Cancel);
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let key_code: u16 = unsafe { msg_send![event, keyCode] };
            if key_code == 53 {
                let _ = self.ivars().action_tx.try_send(PillAction::Cancel);
            }
        }

        #[unsafe(method(isAccessibilityElement))]
        fn is_accessibility_element(&self) -> bool {
            true
        }

        #[unsafe(method_id(accessibilityRole))]
        fn accessibility_role(&self) -> Retained<NSString> {
            NSString::from_str("AXToolbar")
        }

        #[unsafe(method_id(accessibilityLabel))]
        fn accessibility_label(&self) -> Retained<NSString> {
            NSString::from_str("Scrawl Recording Controls")
        }
    }
);

impl ScrawlPillView {
    fn new(
        mtm: MainThreadMarker,
        frame: NSRect,
        action_tx: flume::Sender<PillAction>,
    ) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(PillViewIvars { action_tx });
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }
}

struct MetalPipelines {
    _device: metal::Device,
    queue: metal::CommandQueue,
    layer: MetalLayer,
    tile_pipeline: RenderPipelineState,
    blur_h_pipeline: RenderPipelineState,
    composite_pipeline: RenderPipelineState,
    offscreen_a: metal::Texture,
    offscreen_b: metal::Texture,
    atlas: GlyphAtlas,
    scale: f32,
}

impl MetalPipelines {
    fn new(scale: f32) -> Result<Self, String> {
        let device = metal::Device::system_default().ok_or("No Metal device available")?;
        let queue = device.new_command_queue();

        let metallib_bytes = include_bytes!(concat!(env!("OUT_DIR"), "/pill.metallib"));
        let library = device
            .new_library_with_data(metallib_bytes)
            .map_err(|e| format!("Failed to load pill.metallib: {e}"))?;

        let make_pipeline = |vs_name: &str,
                             fs_name: &str,
                             premul_blend: bool|
         -> Result<RenderPipelineState, String> {
            let vs = library
                .get_function(vs_name, None)
                .map_err(|e| format!("Missing vertex shader {vs_name}: {e}"))?;
            let fs = library
                .get_function(fs_name, None)
                .map_err(|e| format!("Missing fragment shader {fs_name}: {e}"))?;
            let desc = RenderPipelineDescriptor::new();
            desc.set_vertex_function(Some(&vs));
            desc.set_fragment_function(Some(&fs));
            let att = desc
                .color_attachments()
                .object_at(0)
                .ok_or("Missing color attachment 0")?;
            att.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
            if premul_blend {
                att.set_blending_enabled(true);
                att.set_source_rgb_blend_factor(metal::MTLBlendFactor::One);
                att.set_destination_rgb_blend_factor(metal::MTLBlendFactor::OneMinusSourceAlpha);
                att.set_rgb_blend_operation(metal::MTLBlendOperation::Add);
                att.set_source_alpha_blend_factor(metal::MTLBlendFactor::One);
                att.set_destination_alpha_blend_factor(metal::MTLBlendFactor::OneMinusSourceAlpha);
                att.set_alpha_blend_operation(metal::MTLBlendOperation::Add);
            } else {
                att.set_blending_enabled(false);
            }
            device
                .new_render_pipeline_state(&desc)
                .map_err(|e| format!("Pipeline {vs_name}/{fs_name} error: {e}"))
        };

        let tile_pipeline = make_pipeline("pill_tile_vertex", "pill_tile_fragment", true)?;
        let blur_h_pipeline = make_pipeline("pill_blur_h_vertex", "pill_blur_h_fragment", false)?;
        let composite_pipeline =
            make_pipeline("pill_screen_vertex", "pill_composite_fragment", false)?;

        let make_offscreen = || {
            let desc = TextureDescriptor::new();
            desc.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
            desc.set_width((OFFSCREEN_W_PT * scale).round() as u64);
            desc.set_height((OFFSCREEN_H_PT * scale).round() as u64);
            desc.set_usage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
            device.new_texture(&desc)
        };
        let offscreen_a = make_offscreen();
        let offscreen_b = make_offscreen();

        let layer = MetalLayer::new();
        layer.set_device(&device);
        layer.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
        layer.set_presents_with_transaction(true);
        layer.set_opaque(false);
        layer.set_contents_scale(f64::from(scale));
        layer.set_drawable_size(CGSize::new(
            f64::from(PANEL_W * scale),
            f64::from(PANEL_H * scale),
        ));

        let atlas = GlyphAtlas::new(&device, scale);

        Ok(Self {
            _device: device,
            queue,
            layer,
            tile_pipeline,
            blur_h_pipeline,
            composite_pipeline,
            offscreen_a,
            offscreen_b,
            atlas,
            scale,
        })
    }
}

struct PillRuntimeState {
    model: PillModel,
    pipelines: MetalPipelines,
    window: Retained<NSPanel>,
    glass_view: Option<Retained<NSView>>,
    screen_origin_top_left: [f32; 2],
    style: PillStyle,
    last_tick_now: f64,
    dirty: bool,
    ignores_mouse: bool,
    was_mouse_down: bool,
    press_down_elem: Option<ElemId>,
    paste_drag_start_screen: Option<[f32; 2]>,
    action_tx: flume::Sender<PillAction>,
}

// Accessed only on the main thread (guarded by `MainThreadMarker::new()` in `tick_main`).
unsafe impl Send for PillRuntimeState {}

struct SharedNativePill {
    state: Mutex<PillRuntimeState>,
    tick_queued: AtomicBool,
    alive: AtomicBool,
}

pub struct NativePill {
    shared: Arc<SharedNativePill>,
    display_link: Option<CVDisplayLink>,
    display_link_ctx: Option<*const SharedNativePill>,
    action_rx: flume::Receiver<PillAction>,
    window_num: u32,
}

impl NativePill {
    pub fn open() -> Result<Self, String> {
        let mtm = MainThreadMarker::new().ok_or("NativePill::open requires the main thread")?;
        let screens = crate::screens::current();
        // Drawn at the sharpest scale of any display, so moving between them needs no new
        // textures (a 1x display just downsamples).
        let scale = screens
            .iter()
            .map(|s| s.scale as f32)
            .fold(2.0_f32, f32::max);
        let [top_left_x, top_left_y] = panel_origin(&screens, crate::screens::pointer())
            .ok_or("No screen available for NativePill")?;
        let panel_rect = panel_appkit_rect(mtm, [top_left_x, top_left_y]);

        let pipelines = MetalPipelines::new(scale)?;
        let metrics = atlas::measure_metrics();
        let model = PillModel::new(metrics);
        let (action_tx, action_rx) = flume::unbounded();

        let window = NSPanel::initWithContentRect_styleMask_backing_defer(
            mtm.alloc(),
            panel_rect,
            NSWindowStyleMask::Borderless
                | NSWindowStyleMask::UtilityWindow
                | NSWindowStyleMask::NonactivatingPanel
                | NSWindowStyleMask::FullSizeContentView,
            NSBackingStoreType::Buffered,
            false,
        );
        unsafe {
            let sel = sel!(_setPreventsActivation:);
            let responds: bool = msg_send![&*window, respondsToSelector: sel];
            if responds {
                let _: () = msg_send![&*window, _setPreventsActivation: true];
            }
            window.setReleasedWhenClosed(false);
        }

        let local_bounds = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(f64::from(PANEL_W), f64::from(PANEL_H)),
        );
        let root_view = ScrawlPillView::new(mtm, local_bounds, action_tx.clone());
        root_view.setWantsLayer(true);

        // Optional AppKit Liquid Glass view (`NSGlassEffectView` on macOS 26+).
        let glass_view = create_glass_views(&root_view, local_bounds);

        let metal_view = NSView::initWithFrame(mtm.alloc(), local_bounds);
        metal_view.setWantsLayer(true);
        let cocoa_layer = unsafe { &*pipelines.layer.as_ptr().cast::<CALayer>() };
        cocoa_layer.setOpaque(false);
        metal_view.setLayer(Some(cocoa_layer));
        root_view.addSubview(&metal_view);

        window.setContentView(Some(&root_view));
        window.setBackgroundColor(Some(&NSColor::clearColor()));
        window.setOpaque(false);
        window.setHasShadow(false);
        window.setHidesOnDeactivate(false);
        window.setCanHide(false);
        window.setIgnoresMouseEvents(true);
        window.setLevel(NSStatusWindowLevel + 5);
        window.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::CanJoinAllApplications
                | NSWindowCollectionBehavior::FullScreenAuxiliary
                | NSWindowCollectionBehavior::Stationary
                | NSWindowCollectionBehavior::IgnoresCycle,
        );
        window.setFrame_display(panel_rect, true);
        window.orderFrontRegardless();
        // Force one initial AppKit display/layout pass while `glass_view` is non-hidden (alpha 0)
        // so `NSGlassEffectView` constructs its `SDFLayer` + `CABackdropLayer` + `CASDFElementLayer`
        // hierarchy on the main thread.
        window.displayIfNeeded();
        if let Some(ref gv) = glass_view {
            gv.setHidden(true);
        }
        let window_num = window.windowNumber().max(0) as u32;

        let shared = Arc::new(SharedNativePill {
            state: Mutex::new(PillRuntimeState {
                model,
                pipelines,
                window: window.clone(),
                glass_view,
                screen_origin_top_left: [top_left_x, top_left_y],
                style: PillStyle::current(),
                last_tick_now: motion::now(),
                dirty: true,
                ignores_mouse: true,
                was_mouse_down: false,
                press_down_elem: None,
                paste_drag_start_screen: None,
                action_tx,
            }),
            tick_queued: AtomicBool::new(false),
            alive: AtomicBool::new(true),
        });

        // Draw initial transparent frame synchronously.
        shared.tick_main();

        let mut display_link = None;
        let mut display_link_ctx = None;
        if let Ok(dl) = CVDisplayLink::from_active_cg_displays() {
            let ctx = Arc::into_raw(shared.clone());
            if unsafe {
                dl.set_output_callback(pill_display_link_cb, ctx.cast_mut().cast::<c_void>())
            }
            .is_ok()
            {
                let _ = dl.start();
                display_link = Some(dl);
                display_link_ctx = Some(ctx);
            } else {
                unsafe { drop(Arc::from_raw(ctx)) };
            }
        }

        Ok(Self {
            shared,
            display_link,
            display_link_ctx,
            action_rx,
            window_num,
        })
    }

    pub fn window_number(&self) -> u32 {
        self.window_num
    }

    /// Moves the capsule to the screen holding `point` (global points, top-left origin), below
    /// that screen's menu bar and notch. A no-op when it's already there.
    pub fn place_on(&self, point: [f64; 2]) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let screens = crate::screens::current();
        let Some(origin) = panel_origin(&screens, Some(point)) else {
            return;
        };
        let Ok(mut st) = self.shared.state.lock() else {
            return;
        };
        if (st.screen_origin_top_left[0] - origin[0]).abs() < 0.5
            && (st.screen_origin_top_left[1] - origin[1]).abs() < 0.5
        {
            return;
        }
        st.screen_origin_top_left = origin;
        st.window
            .setFrame_display(panel_appkit_rect(mtm, origin), true);
        // On a 1x screen AppKit resets the layer's contentsScale; keep the scale it's drawn at.
        let scale = st.pipelines.scale;
        st.pipelines.layer.set_contents_scale(f64::from(scale));
        st.pipelines.layer.set_drawable_size(CGSize::new(
            f64::from(PANEL_W * scale),
            f64::from(PANEL_H * scale),
        ));
        st.dirty = true;
    }

    /// Synchronizes coordinator state into the native pill model; the next `CVDisplayLink` tick
    /// advances springs and renders on the display refresh cadence (with main-loop fallback if
    /// `CVDisplayLink` is paused).
    pub fn sync(&self, snapshot: PillSnapshot) {
        let mut needs_fallback_tick = self.display_link.is_none();
        if let Ok(mut st) = self.shared.state.lock() {
            if st.style != snapshot.style {
                st.style = snapshot.style;
                st.dirty = true;
            }
            st.model.reduced = snapshot.reduced_motion;
            if st.model.clock != snapshot.clock {
                st.pipelines.atlas.update_clock(&snapshot.clock);
                st.model.set_clock(&snapshot.clock);
                st.dirty = true;
            }
            if let Pres::Completed {
                ref label,
                ref routes,
            } = snapshot.pres
            {
                let mut labels = st.model.labels;
                labels.completed = st.pipelines.atlas.update_completed(label).size_pt[0];
                for (i, width) in labels.routes.iter_mut().enumerate() {
                    *width = routes.get(i).map_or(0.0, |route| {
                        st.pipelines.atlas.update_route(i, route).size_pt[0]
                    });
                }
                st.model.set_label_widths(labels, &snapshot.pres);
            }
            if st.model.pres != snapshot.pres {
                let was_hidden = st.model.is_fully_hidden();
                st.model.set_pres(snapshot.pres);
                st.dirty = true;
                if was_hidden {
                    st.window.setLevel(NSStatusWindowLevel + 5);
                    st.window.orderFrontRegardless();
                }
            }
            if (motion::now() - st.last_tick_now) >= f64::from(0.045 * motion::time_scale()) {
                needs_fallback_tick = true;
            }
        }
        if needs_fallback_tick {
            self.shared.tick_main();
        }
    }

    pub fn drain_actions(&self) -> Vec<PillAction> {
        self.action_rx.try_iter().collect()
    }

    pub fn is_fully_hidden(&self) -> bool {
        self.shared
            .state
            .lock()
            .map_or(true, |st| st.model.is_fully_hidden())
    }

    pub fn pointer_on_pill(&self) -> bool {
        self.shared
            .state
            .lock()
            .map_or(false, |st| !st.ignores_mouse)
    }
}

impl Drop for NativePill {
    fn drop(&mut self) {
        self.shared.alive.store(false, Ordering::SeqCst);
        if let Some(ref dl) = self.display_link {
            let _ = dl.stop();
        }
        if let Ok(st) = self.shared.state.lock() {
            st.window.orderOut(None);
            st.window.close();
        }
        if let Some(ctx) = self.display_link_ctx {
            unsafe { drop(Arc::from_raw(ctx)) };
        }
    }
}

/// The panel's top-left (global points) on the screen holding `point`, else the primary one.
fn panel_origin(screens: &[crate::screens::Screen], point: Option<[f64; 2]>) -> Option<[f32; 2]> {
    let index = point
        .and_then(|p| crate::screens::index_at(screens, p))
        .or(if screens.is_empty() { None } else { Some(0) })?;
    let [x, y] = crate::screens::pill_origin(
        &screens[index],
        f64::from(PANEL_W),
        f64::from(model::PANEL_Y),
    );
    Some([x as f32, y as f32])
}

/// The panel's AppKit frame for a top-left origin in global points.
fn panel_appkit_rect(mtm: MainThreadMarker, [x, y]: [f32; 2]) -> NSRect {
    let primary_h = NSScreen::screens(mtm)
        .firstObject()
        .map_or(900.0, |s| s.frame().size.height);
    let [ax, ay, w, h] = crate::screens::flip(
        [
            f64::from(x),
            f64::from(y),
            f64::from(PANEL_W),
            f64::from(PANEL_H),
        ],
        primary_h,
    );
    NSRect::new(NSPoint::new(ax, ay), NSSize::new(w, h))
}

/// Creates `NSGlassEffectView` dynamically on macOS 26+.
fn create_glass_views(root_view: &NSView, bounds: NSRect) -> Option<Retained<NSView>> {
    let glass_cls = AnyClass::get(c"NSGlassEffectView")?;
    unsafe {
        let raw_glass: *mut NSView = msg_send![glass_cls, alloc];
        if raw_glass.is_null() {
            return None;
        }
        let init_frame = NSRect::new(
            NSPoint::new((bounds.size.width - 200.0) * 0.5, 22.0),
            NSSize::new(200.0, 40.0),
        );
        let raw_glass: *mut NSView = msg_send![raw_glass, initWithFrame: init_frame];
        let glass_view = Retained::from_raw(raw_glass)?;
        glass_view.setWantsLayer(true);
        glass_view.setAlphaValue(0.0);

        if msg_send![&*glass_view, respondsToSelector: sel!(setStyle:)] {
            // 0 = NSGlassEffectViewStyleRegular (refractive Liquid Glass bottle + specular rim)
            let _: () = msg_send![&*glass_view, setStyle: 0_isize];
        }
        if msg_send![&*glass_view, respondsToSelector: sel!(setCornerRadius:)] {
            let _: () = msg_send![&*glass_view, setCornerRadius: 20.0_f64];
        }
        if msg_send![&*glass_view, respondsToSelector: sel!(setTintColor:)] {
            let tint = NSColor::colorWithSRGBRed_green_blue_alpha(0.06, 0.07, 0.10, 0.22);
            let _: () = msg_send![&*glass_view, setTintColor: Some(&*tint)];
        }

        root_view.addSubview(&glass_view);
        Some(glass_view)
    }
}

extern "C" fn pill_display_link_cb(
    _display_link: CVDisplayLinkRef,
    _now: *const CVTimeStamp,
    _output_time: *const CVTimeStamp,
    _flags_in: CVOptionFlags,
    _flags_out: *mut CVOptionFlags,
    context: *mut c_void,
) -> CVReturn {
    if !context.is_null() {
        let shared = unsafe { &*context.cast::<SharedNativePill>() };
        if shared.alive.load(Ordering::Relaxed) && !shared.tick_queued.swap(true, Ordering::AcqRel)
        {
            // Retain an Arc reference for the main-queue callback.
            unsafe {
                Arc::increment_strong_count(context.cast::<SharedNativePill>());
                dispatch_async_f(&_dispatch_main_q, context, pill_main_queue_trampoline);
            }
        }
    }
    0
}

extern "C" fn pill_main_queue_trampoline(context: *mut c_void) {
    let shared = unsafe { Arc::from_raw(context.cast::<SharedNativePill>()) };
    if shared.alive.load(Ordering::Relaxed) {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            shared.tick_main();
        }));
    }
    shared.tick_queued.store(false, Ordering::Release);
}

impl SharedNativePill {
    fn tick_main(&self) {
        if MainThreadMarker::new().is_none() {
            return;
        }
        motion::poll_tune();
        let Ok(mut st) = self.state.lock() else {
            return;
        };

        let live_style = PillStyle::current();
        if st.style != live_style {
            st.style = live_style;
            st.dirty = true;
        }

        // Poll mouse cursor and button state for hover, press squish, click, and Paste drag.
        st.poll_pointer_and_input();

        let now = motion::now();
        let raw_dt = (now - st.last_tick_now).max(0.0) as f32;
        let min_dt = 0.008 * motion::time_scale();
        if raw_dt < min_dt {
            return;
        }
        st.last_tick_now = now;
        let dt = raw_dt.min(0.025 * motion::time_scale());

        let animating = !st.model.is_at_rest() || matches!(st.model.pres, Pres::Processing);
        if !animating && !st.dirty {
            if motion::tracing() && !st.model.is_fully_hidden() {
                let frame = st.model.frame();
                st.emit_trace(&frame);
            }
            return;
        }

        if dt > 0.0 {
            st.model.advance(dt);
        }
        st.dirty = false;

        let frame = st.model.frame();
        if motion::tracing() && (!st.model.is_fully_hidden() || animating) {
            st.emit_trace(&frame);
        }
        st.render_frame(&frame);
    }
}

impl PillRuntimeState {
    fn poll_pointer_and_input(&mut self) {
        if self.model.is_fully_hidden() {
            if !self.ignores_mouse {
                self.ignores_mouse = true;
                self.window.setIgnoresMouseEvents(true);
            }
            return;
        }

        let Some(cls) = AnyClass::get(c"NSEvent") else {
            return;
        };
        let mouse_ns: NSPoint = unsafe { msg_send![cls, mouseLocation] };
        let pressed_mask: usize = unsafe { msg_send![cls, pressedMouseButtons] };
        let mouse_down = (pressed_mask & 1) != 0;

        let win_frame = self.window.frame();
        let local_x = (mouse_ns.x - win_frame.origin.x) as f32;
        let local_y = (win_frame.size.height - (mouse_ns.y - win_frame.origin.y)) as f32;
        let screen_pt = [
            self.screen_origin_top_left[0] + local_x,
            self.screen_origin_top_left[1] + local_y,
        ];

        let inside_island = self.model.hit_island(local_x, local_y);
        let should_ignore = !inside_island && self.press_down_elem.is_none();
        if self.ignores_mouse != should_ignore {
            self.ignores_mouse = should_ignore;
            self.window.setIgnoresMouseEvents(should_ignore);
        }

        let prev_hover = self.model.hovered;
        let prev_press = self.model.pressed;
        let pos_opt = inside_island.then_some([local_x, local_y]);
        self.model.set_pointer(pos_opt, mouse_down);
        if self.model.hovered != prev_hover || self.model.pressed != prev_press {
            self.dirty = true;
        }

        if mouse_down && !self.was_mouse_down {
            let hit = pos_opt.and_then(|[x, y]| self.model.hit_element(x, y));
            self.press_down_elem = hit;
            if hit == Some(ElemId::Paste) {
                self.paste_drag_start_screen = Some(screen_pt);
            } else {
                self.paste_drag_start_screen = None;
            }
        } else if !mouse_down && self.was_mouse_down {
            if let Some(down_elem) = self.press_down_elem.take() {
                let up_elem = pos_opt.and_then(|[x, y]| self.model.hit_element(x, y));
                if down_elem == ElemId::Paste {
                    if up_elem == Some(ElemId::Paste) {
                        let _ = self.action_tx.try_send(PillAction::Paste(None));
                    } else if let Some(start) = self.paste_drag_start_screen.take() {
                        let dist = (screen_pt[0] - start[0]).hypot(screen_pt[1] - start[1]);
                        if dist >= 24.0 {
                            let _ = self.action_tx.try_send(PillAction::Paste(Some(screen_pt)));
                        }
                    }
                } else if up_elem == Some(down_elem) {
                    self.dispatch_click(down_elem);
                }
            }
            self.paste_drag_start_screen = None;
        }
        self.was_mouse_down = mouse_down;
    }

    fn dispatch_click(&self, elem: ElemId) {
        let is_selecting = matches!(self.model.pres, Pres::Selecting { .. });
        let action = match elem {
            ElemId::ChipArea => {
                if is_selecting {
                    PillAction::SelectMode(SelectionMode::Area)
                } else {
                    PillAction::SwitchRecording(SelectionMode::Area)
                }
            }
            ElemId::ChipWindow => {
                if is_selecting {
                    PillAction::SelectMode(SelectionMode::Window)
                } else {
                    PillAction::SwitchRecording(SelectionMode::Window)
                }
            }
            ElemId::ChipScreen => {
                if is_selecting {
                    PillAction::SelectMode(SelectionMode::Screen)
                } else {
                    PillAction::SwitchRecording(SelectionMode::Screen)
                }
            }
            ElemId::ChipIphone => {
                if is_selecting {
                    PillAction::SelectMode(SelectionMode::Iphone)
                } else {
                    PillAction::SwitchRecording(SelectionMode::Iphone)
                }
            }
            ElemId::Record => PillAction::RecordSelected,
            ElemId::Cancel => PillAction::Cancel,
            ElemId::Finish => PillAction::Finish,
            ElemId::Close => {
                if matches!(self.model.pres, Pres::Completed { .. }) {
                    PillAction::DismissCompleted
                } else {
                    PillAction::Cancel
                }
            }
            ElemId::Paste => PillAction::Paste(None),
            ElemId::Inspect => PillAction::Inspect,
            ElemId::Route0 => PillAction::Route(0),
            ElemId::Route1 => PillAction::Route(1),
            _ => return,
        };
        let _ = self.action_tx.try_send(action);
    }

    fn emit_trace(&self, frame: &FrameState) {
        motion::trace_frame();
        let [ox, oy] = self.screen_origin_top_left;
        let win = [ox, oy, PANEL_W, PANEL_H];
        let sh = &frame.shape;
        let shell = [ox + sh.l, oy + sh.t, sh.width(), sh.height()];
        motion::trace_element("window", win, 0.0, 0.0, None, None);
        motion::trace_element("shell", shell, sh.alpha, sh.radius, None, None);
        motion::trace_channel("content.blur", "level", frame.content_blur);
        motion::trace_channel("content.alpha", "level", frame.content_alpha);
        motion::trace_channel(
            "shell.width",
            "position",
            (self.model.r.x - self.model.l.x).max(0.0),
        );

        for elem in &frame.elems {
            let [ex, ey, ew, eh] = elem.rect;
            let screen_rect = if matches!(elem.id, ElemId::DividerLead | ElemId::DividerClose) {
                let dw = 1.0 * elem.scale;
                let dh = 16.0 * elem.scale;
                [ox + ex + (ew - dw) * 0.5, oy + ey + (eh - dh) * 0.5, dw, dh]
            } else {
                [ox + ex, oy + ey, ew, eh]
            };
            motion::trace_element(
                elem.id.trace_key(),
                screen_rect,
                elem.alpha,
                0.0,
                Some("row"),
                Some(shell),
            );
        }
    }

    fn render_frame(&mut self, frame: &FrameState) {
        let use_glass = self.style == PillStyle::Glass && self.glass_view.is_some();
        let Some(drawable) = self.pipelines.layer.next_drawable() else {
            return;
        };
        let cmd = self.pipelines.encode(frame, use_glass, drawable.texture());

        // Synchronize `NSGlassEffectView` geometry and Metal drawable presentation in one CATransaction.
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        if let Some(ref gv) = self.glass_view {
            if use_glass && frame.shape.alpha > 0.005 {
                gv.setHidden(false);
                let gx = f64::from(frame.shape.l);
                let gy = f64::from(PANEL_H - frame.shape.b);
                let gw = f64::from(frame.shape.width());
                let gh = f64::from(frame.shape.height());
                gv.setFrame(NSRect::new(NSPoint::new(gx, gy), NSSize::new(gw, gh)));
                gv.setAlphaValue(f64::from(frame.shape.alpha));
                unsafe {
                    if msg_send![&**gv, respondsToSelector: sel!(setCornerRadius:)] {
                        let _: () =
                            msg_send![&**gv, setCornerRadius: f64::from(frame.shape.radius)];
                    }
                    if msg_send![&**gv, respondsToSelector: sel!(setTintColor:)] {
                        let tint = if frame.is_completed {
                            NSColor::colorWithSRGBRed_green_blue_alpha(0.04, 0.14, 0.09, 0.28)
                        } else {
                            NSColor::colorWithSRGBRed_green_blue_alpha(0.06, 0.07, 0.10, 0.22)
                        };
                        let _: () = msg_send![&**gv, setTintColor: Some(&*tint)];
                    }
                }
                gv.setNeedsLayout(true);
                gv.layoutSubtreeIfNeeded();
            } else {
                gv.setHidden(true);
            }
        }
        cmd.commit();
        cmd.wait_until_scheduled();
        drawable.present();
        CATransaction::commit();
    }
}

impl MetalPipelines {
    /// Encodes `frame`'s three passes into `target` (`BGRA8`, the panel's size at `scale`); the
    /// caller commits the returned buffer.
    fn encode(
        &self,
        frame: &FrameState,
        use_glass: bool,
        target: &metal::TextureRef,
    ) -> metal::CommandBuffer {
        let mut prims: Vec<TilePrim> = Vec::with_capacity(frame.elems.len() * 3);
        let mut blur_boxes: Vec<BlurBox> = Vec::with_capacity(frame.elems.len());
        let mut comp_elems: Vec<CompositeElem> = Vec::with_capacity(frame.elems.len());

        // Tiles are packed in rows as wide as they need, so a long lead or session chip isn't cut.
        let (mut slot_x, mut slot_y, mut row_h) = (4.0_f32, 4.0_f32, 0.0_f32);
        // The composite shader reads at most 16.
        for elem in frame.elems.iter().take(16) {
            let tile_w = (elem.base_size[0] + 2.0 * TILE_PAD_PT).min(OFFSCREEN_W_PT - 8.0);
            let tile_h = (elem.base_size[1] + 2.0 * TILE_PAD_PT).min(OFFSCREEN_H_PT - 8.0);
            if slot_x + tile_w > OFFSCREEN_W_PT - 4.0 {
                slot_x = 4.0;
                slot_y += row_h + 4.0;
                row_h = 0.0;
            }
            if slot_y + tile_h > OFFSCREEN_H_PT - 4.0 {
                break;
            }
            let tile_rect = [slot_x, slot_y, tile_w, tile_h];

            emit_elem_prims(
                &self.atlas,
                use_glass,
                elem,
                [slot_x + TILE_PAD_PT, slot_y + TILE_PAD_PT],
                frame.t,
                &mut prims,
            );

            blur_boxes.push(BlurBox {
                tile_rect,
                sigma_pt: elem.blur,
                _pad0: 0.0,
                _pad1: [0.0; 2],
            });

            comp_elems.push(CompositeElem {
                screen_rect: elem.rect,
                tile_rect,
                base_size: elem.base_size,
                scale: elem.scale,
                alpha: elem.alpha,
                sigma_pt: elem.blur,
                pad_pt: TILE_PAD_PT,
                _pad0: [0.0; 2],
            });
            slot_x += tile_w + 4.0;
            row_h = row_h.max(tile_h);
        }

        let (has_ghost, ghost_rect, ghost_meta) = if let Some(ref g) = frame.ghost {
            (1_u32, [g.l, g.t, g.r, g.b], [g.radius, g.alpha, 0.0, 0.0])
        } else {
            (0_u32, [0.0; 4], [0.0; 4])
        };

        let tile_uniforms = TileUniforms {
            atlas_size: [1024.0, 512.0],
            canvas_size: [OFFSCREEN_W_PT, OFFSCREEN_H_PT],
            scale: self.scale,
            _pad0: 0.0,
            _pad1: [0.0; 2],
        };

        let uniforms = PillUniforms {
            panel_size: [PANEL_W, PANEL_H],
            canvas_size: [OFFSCREEN_W_PT, OFFSCREEN_H_PT],
            scale: self.scale,
            elem_count: comp_elems.len() as u32,
            is_completed: u32::from(frame.is_completed),
            has_ghost,
            shape_rect: [frame.shape.l, frame.shape.t, frame.shape.r, frame.shape.b],
            shape_meta: [
                frame.shape.radius,
                frame.shape.alpha,
                frame.shape.blur,
                if use_glass { 1.0 } else { 0.0 },
            ],
            ghost_rect,
            ghost_meta,
        };

        let prim_count = prims.len() as u64;
        let box_count = blur_boxes.len() as u64;
        if prims.is_empty() {
            prims.push(TilePrim::default());
        }
        if blur_boxes.is_empty() {
            blur_boxes.push(BlurBox::default());
        }
        if comp_elems.is_empty() {
            comp_elems.push(CompositeElem::default());
        }

        let cmd = self.queue.new_command_buffer().to_owned();

        // Pass 1: Render crisp unblurred element primitives into `offscreen_a`.
        {
            let pass = RenderPassDescriptor::new();
            let att = pass.color_attachments().object_at(0).unwrap();
            att.set_texture(Some(&self.offscreen_a));
            att.set_load_action(MTLLoadAction::Clear);
            att.set_clear_color(metal::MTLClearColor::new(0.0, 0.0, 0.0, 0.0));
            att.set_store_action(MTLStoreAction::Store);
            let enc = cmd.new_render_command_encoder(&pass);
            if prim_count > 0 {
                enc.set_render_pipeline_state(&self.tile_pipeline);
                enc.set_vertex_bytes(
                    0,
                    size_of::<TileUniforms>() as u64,
                    (&raw const tile_uniforms).cast::<c_void>(),
                );
                enc.set_vertex_bytes(
                    1,
                    (prims.len() * size_of::<TilePrim>()) as u64,
                    prims.as_ptr().cast::<c_void>(),
                );
                enc.set_fragment_bytes(
                    0,
                    size_of::<TileUniforms>() as u64,
                    (&raw const tile_uniforms).cast::<c_void>(),
                );
                enc.set_fragment_texture(0, Some(&self.atlas.texture));
                enc.draw_primitives_instanced(MTLPrimitiveType::Triangle, 0, 6, prim_count);
            }
            enc.end_encoding();
        }

        // Pass 2: 1D Horizontal Gaussian blur from `offscreen_a` into `offscreen_b`.
        {
            let pass = RenderPassDescriptor::new();
            let att = pass.color_attachments().object_at(0).unwrap();
            att.set_texture(Some(&self.offscreen_b));
            att.set_load_action(MTLLoadAction::Clear);
            att.set_clear_color(metal::MTLClearColor::new(0.0, 0.0, 0.0, 0.0));
            att.set_store_action(MTLStoreAction::Store);
            let enc = cmd.new_render_command_encoder(&pass);
            if box_count > 0 {
                enc.set_render_pipeline_state(&self.blur_h_pipeline);
                enc.set_vertex_bytes(
                    0,
                    size_of::<TileUniforms>() as u64,
                    (&raw const tile_uniforms).cast::<c_void>(),
                );
                enc.set_vertex_bytes(
                    1,
                    (blur_boxes.len() * size_of::<BlurBox>()) as u64,
                    blur_boxes.as_ptr().cast::<c_void>(),
                );
                enc.set_fragment_bytes(
                    0,
                    size_of::<TileUniforms>() as u64,
                    (&raw const tile_uniforms).cast::<c_void>(),
                );
                enc.set_fragment_texture(0, Some(&self.offscreen_a));
                enc.draw_primitives_instanced(MTLPrimitiveType::Triangle, 0, 6, box_count);
            }
            enc.end_encoding();
        }

        // Pass 3: Capsule SDF silhouette + 1D Vertical Gaussian blur + hairline rim into drawable.
        {
            let pass = RenderPassDescriptor::new();
            let att = pass.color_attachments().object_at(0).unwrap();
            att.set_texture(Some(target));
            att.set_load_action(MTLLoadAction::Clear);
            att.set_clear_color(metal::MTLClearColor::new(0.0, 0.0, 0.0, 0.0));
            att.set_store_action(MTLStoreAction::Store);
            let enc = cmd.new_render_command_encoder(&pass);
            enc.set_render_pipeline_state(&self.composite_pipeline);
            enc.set_fragment_bytes(
                0,
                size_of::<PillUniforms>() as u64,
                (&raw const uniforms).cast::<c_void>(),
            );
            enc.set_fragment_bytes(
                1,
                (comp_elems.len() * size_of::<CompositeElem>()) as u64,
                comp_elems.as_ptr().cast::<c_void>(),
            );
            enc.set_fragment_texture(0, Some(&self.offscreen_b));
            enc.draw_primitives(MTLPrimitiveType::Triangle, 0, 3);
            enc.end_encoding();
        }

        cmd
    }
}

fn emit_elem_prims(
    atlas: &GlyphAtlas,
    is_glass: bool,
    elem: &model::FrameElem,
    [ox, oy]: [f32; 2],
    t: f32,
    out: &mut Vec<TilePrim>,
) {
    let [bw, bh] = elem.base_size;
    let push_shape = |out: &mut Vec<TilePrim>, rect: [f32; 4], radius: f32, color: [f32; 4]| {
        out.push(TilePrim {
            rect,
            uv_rect: [radius, 0.0, 0.0, 0.0],
            color,
            kind: 0,
            _pad0: 0,
            _pad1: 0,
            _pad2: 0,
        });
    };
    let push_sprite = |out: &mut Vec<TilePrim>, rect: [f32; 4], uv: [f32; 4], color: [f32; 4]| {
        out.push(TilePrim {
            rect,
            uv_rect: uv,
            color,
            kind: 1,
            _pad0: 0,
            _pad1: 0,
            _pad2: 0,
        });
    };
    let center_text = |sp: atlas::Sprite| -> [f32; 4] {
        [
            ox + (bw - sp.size_pt[0]) * 0.5,
            oy + (bh - sp.size_pt[1]) * 0.5,
            sp.size_pt[0],
            sp.size_pt[1],
        ]
    };

    match elem.id {
        ElemId::ChipArea | ElemId::ChipWindow | ElemId::ChipScreen | ElemId::ChipIphone => {
            let mode = match elem.id {
                ElemId::ChipArea => SelectionMode::Area,
                ElemId::ChipWindow => SelectionMode::Window,
                ElemId::ChipScreen => SelectionMode::Screen,
                ElemId::ChipIphone => SelectionMode::Iphone,
                _ => unreachable!(),
            };
            if is_glass {
                let smoke_a = 0.20 + 0.18 * elem.active + 0.10 * elem.hover;
                push_shape(out, [ox, oy, bw, bh], bh * 0.5, [0.06, 0.07, 0.10, smoke_a]);
            } else {
                let bg_a = 0.16 * elem.active
                    + 0.08 * elem.hover * (1.0 - elem.active)
                    + 0.06 * elem.press;
                if bg_a > 0.002 {
                    push_shape(out, [ox, oy, bw, bh], bh * 0.5, [1.0, 1.0, 1.0, bg_a]);
                }
            }
            let ic = atlas.icon(IconKind::from_mode(mode));
            let tx = atlas.label(mode.label());
            let fg = if is_glass {
                0.90 + 0.10 * elem.active.max(elem.hover)
            } else {
                0.608 + 0.353 * elem.active.max(elem.hover * 0.65)
            };
            push_sprite(
                out,
                [ox + CHIP_PX, oy + (CHIP_H - 14.0) * 0.5, 14.0, 14.0],
                ic.uv,
                [fg, fg, fg, 1.0],
            );
            push_sprite(
                out,
                [
                    ox + CHIP_PX + 14.0 + 6.0 - 2.0,
                    oy + (CHIP_H - tx.size_pt[1]) * 0.5,
                    tx.size_pt[0],
                    tx.size_pt[1],
                ],
                tx.uv,
                [fg, fg, fg, 1.0],
            );
        }
        ElemId::Record | ElemId::Finish | ElemId::Paste => {
            let label = match elem.id {
                ElemId::Record => "Record",
                ElemId::Finish => "Finish",
                ElemId::Paste => "Paste",
                _ => unreachable!(),
            };
            let tx = atlas.label(label);
            // `ACCENT_COLOR = 0x3b5cf6`, `ACCENT_HOVER = 0x4b6bff`
            let r = (0.231 + 0.063 * elem.hover - 0.03 * elem.press).clamp(0.0, 1.0);
            let g = (0.361 + 0.059 * elem.hover - 0.04 * elem.press).clamp(0.0, 1.0);
            let b = (0.965 + 0.035 * elem.hover - 0.04 * elem.press).clamp(0.0, 1.0);
            push_shape(out, [ox, oy, bw, bh], bh * 0.5, [r, g, b, 1.0]);
            push_sprite(out, center_text(tx), tx.uv, [0.95, 0.95, 0.96, 1.0]);
        }
        ElemId::Lead => {
            if is_glass {
                push_shape(out, [ox, oy, bw, bh], bh * 0.5, [0.06, 0.07, 0.10, 0.24]);
            }
            let tx = atlas.clock_sprite;
            let cycle = 1.6_f32;
            let ripple = (t / cycle).fract();
            let breath = 0.5 + 0.5 * (std::f32::consts::TAU * t / cycle).cos();
            let halo_r = 4.0 + 6.0 * ripple;
            let halo_a = 0.34 * (1.0 - ripple).powi(2);
            push_shape(
                out,
                [
                    ox + 16.0 - halo_r,
                    oy + 16.0 - halo_r,
                    halo_r * 2.0,
                    halo_r * 2.0,
                ],
                halo_r,
                [0.941, 0.267, 0.220, halo_a],
            );
            push_shape(
                out,
                [ox + 12.0, oy + 12.0, 8.0, 8.0],
                4.0,
                [0.941, 0.267, 0.220, 0.78 + 0.22 * breath],
            );
            push_sprite(
                out,
                [
                    ox + CHIP_H - 2.0,
                    oy + (CHIP_H - tx.size_pt[1]) * 0.5,
                    tx.size_pt[0],
                    tx.size_pt[1],
                ],
                tx.uv,
                [0.961, 0.961, 0.969, 1.0],
            );
        }
        ElemId::Route0 | ElemId::Route1 => {
            let index = usize::from(elem.id == ElemId::Route1);
            // "Send to DESKTOP" is a suggestion: a faint fill sets it apart from the runner-up
            // without competing with Paste.
            let suggestion = atlas.route_text(index).starts_with("Send to ");
            let rest = if suggestion { 0.07 } else { 0.0 };
            if is_glass {
                let smoke_a = 0.20 + rest + 0.12 * elem.hover + 0.06 * elem.press;
                push_shape(out, [ox, oy, bw, bh], bh * 0.5, [0.06, 0.07, 0.10, smoke_a]);
            } else {
                let bg_a = rest + 0.08 * elem.hover + 0.06 * elem.press;
                if bg_a > 0.002 {
                    push_shape(out, [ox, oy, bw, bh], bh * 0.5, [1.0, 1.0, 1.0, bg_a]);
                }
            }
            let tx = atlas.route_sprites[index];
            let fg = if is_glass {
                0.92 + 0.08 * elem.hover
            } else if suggestion {
                0.88 + 0.08 * elem.hover
            } else {
                0.74 + 0.22 * elem.hover
            };
            push_sprite(out, center_text(tx), tx.uv, [fg, fg, fg, 1.0]);
        }
        ElemId::Cancel | ElemId::Inspect => {
            let label = if elem.id == ElemId::Cancel {
                "Cancel"
            } else {
                "Inspect"
            };
            if is_glass {
                let smoke_a = 0.20 + 0.12 * elem.hover + 0.06 * elem.press;
                push_shape(out, [ox, oy, bw, bh], bh * 0.5, [0.06, 0.07, 0.10, smoke_a]);
            } else if elem.hover > 0.01 || elem.press > 0.01 {
                let bg_a = 0.08 * elem.hover + 0.06 * elem.press;
                push_shape(out, [ox, oy, bw, bh], bh * 0.5, [1.0, 1.0, 1.0, bg_a]);
            }
            let tx = atlas.label(label);
            let fg = if is_glass {
                0.92 + 0.08 * elem.hover
            } else {
                0.74 + 0.22 * elem.hover
            };
            push_sprite(out, center_text(tx), tx.uv, [fg, fg, fg, 1.0]);
        }
        ElemId::Close => {
            if is_glass {
                let smoke_a = 0.20 + 0.12 * elem.hover + 0.06 * elem.press;
                push_shape(
                    out,
                    [ox + 2.0, oy + 2.0, bh - 4.0, bh - 4.0],
                    (bh - 4.0) * 0.5,
                    [0.06, 0.07, 0.10, smoke_a],
                );
            } else if elem.hover > 0.01 || elem.press > 0.01 {
                let bg_a = 0.10 * elem.hover + 0.06 * elem.press;
                push_shape(
                    out,
                    [ox + 2.0, oy + 2.0, bh - 4.0, bh - 4.0],
                    (bh - 4.0) * 0.5,
                    [1.0, 1.0, 1.0, bg_a],
                );
            }
            let ic = atlas.icon(IconKind::Close);
            let fg = if is_glass {
                0.90 + 0.10 * elem.hover
            } else {
                0.61 + 0.35 * elem.hover
            };
            push_sprite(
                out,
                [
                    ox + (CHIP_H - 12.0) * 0.5,
                    oy + (CHIP_H - 12.0) * 0.5,
                    12.0,
                    12.0,
                ],
                ic.uv,
                [fg, fg, fg, 1.0],
            );
        }
        ElemId::DividerLead | ElemId::DividerClose => {
            push_shape(
                out,
                [ox + (bw - 1.0) * 0.5, oy + (bh - 16.0) * 0.5, 1.0, 16.0],
                0.5,
                [1.0, 1.0, 1.0, 0.11],
            );
        }
        ElemId::Processing => {
            let head = (t * 8.0) % 8.0;
            for k in 0..8 {
                let ang = (k as f32) * std::f32::consts::FRAC_PI_4 - std::f32::consts::FRAC_PI_2;
                let dcx = ox + 16.0 + 6.2 * ang.cos();
                let dcy = oy + 16.0 + 6.2 * ang.sin();
                let phase_d = (head - k as f32).rem_euclid(8.0);
                let da = (1.0 - phase_d / 7.5).clamp(0.18, 1.0);
                push_shape(
                    out,
                    [dcx - 1.35, dcy - 1.35, 2.7, 2.7],
                    1.35,
                    [0.961, 0.961, 0.969, da],
                );
            }
            let tx = atlas.label("Processing…");
            push_sprite(
                out,
                [
                    ox + CHIP_H + 6.0 - 2.0,
                    oy + (CHIP_H - tx.size_pt[1]) * 0.5,
                    tx.size_pt[0],
                    tx.size_pt[1],
                ],
                tx.uv,
                [0.961, 0.961, 0.969, 1.0],
            );
        }
        ElemId::DoneLead => {
            push_shape(
                out,
                [ox + 5.0, oy + 5.0, 22.0, 22.0],
                11.0,
                [0.412, 0.847, 0.624, 0.22],
            );
            let ic = atlas.icon(IconKind::Check);
            push_sprite(
                out,
                [
                    ox + (CHIP_H - 13.0) * 0.5,
                    oy + (CHIP_H - 13.0) * 0.5,
                    13.0,
                    13.0,
                ],
                ic.uv,
                [0.412, 0.847, 0.624, 1.0],
            );
            let tx = atlas.completed_sprite;
            push_sprite(
                out,
                [
                    ox + CHIP_H + 8.0 - 2.0,
                    oy + (CHIP_H - tx.size_pt[1]) * 0.5,
                    tx.size_pt[0],
                    tx.size_pt[1],
                ],
                tx.uv,
                [0.961, 0.961, 0.969, 1.0],
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metal_pipelines_compile_and_initialize() {
        let pipelines = MetalPipelines::new(2.0).expect("MetalPipelines::new failed");
        assert_eq!(pipelines.scale, 2.0);
    }

    /// Draws the capsule for `pres` offscreen (never on screen) after `seconds` of motion from
    /// `from`, over a grey backdrop, as RGBA at 2x.
    fn still(
        pipelines: &mut MetalPipelines,
        model: &mut PillModel,
        pres: &Pres,
        seconds: f32,
    ) -> crate::model::RgbaFrame {
        if let Pres::Completed { label, routes } = pres {
            let mut labels = model.labels;
            labels.completed = pipelines.atlas.update_completed(label).size_pt[0];
            for (i, width) in labels.routes.iter_mut().enumerate() {
                *width = routes.get(i).map_or(0.0, |route| {
                    pipelines.atlas.update_route(i, route).size_pt[0]
                });
            }
            model.set_label_widths(labels, pres);
        }
        model.set_pres(pres.clone());
        let steps = (seconds * 240.0).round() as usize;
        for _ in 0..steps {
            model.advance(1.0 / 240.0);
        }
        let frame = model.frame();
        let (w, h) = (
            (PANEL_W * pipelines.scale) as u64,
            (PANEL_H * pipelines.scale) as u64,
        );
        let desc = TextureDescriptor::new();
        desc.set_pixel_format(MTLPixelFormat::BGRA8Unorm);
        desc.set_width(w);
        desc.set_height(h);
        desc.set_storage_mode(metal::MTLStorageMode::Shared);
        desc.set_usage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
        let target = pipelines._device.new_texture(&desc);
        let cmd = pipelines.encode(&frame, false, &target);
        cmd.commit();
        cmd.wait_until_completed();
        let mut bgra = vec![0_u8; (w * h * 4) as usize];
        target.get_bytes(
            bgra.as_mut_ptr().cast::<c_void>(),
            w * 4,
            metal::MTLRegion {
                origin: metal::MTLOrigin { x: 0, y: 0, z: 0 },
                size: metal::MTLSize {
                    width: w,
                    height: h,
                    depth: 1,
                },
            },
            0,
        );
        // Premultiplied BGRA over a grey backdrop.
        let back = [0x5a_u8, 0x5e, 0x66];
        let mut rgba = Vec::with_capacity(bgra.len());
        for px in bgra.chunks_exact(4) {
            let a = f32::from(px[3]) / 255.0;
            for (src, bg) in [px[2], px[1], px[0]].into_iter().zip(back) {
                rgba.push((f32::from(src) + f32::from(bg) * (1.0 - a)).min(255.0) as u8);
            }
            rgba.push(255);
        }
        crate::model::RgbaFrame {
            t_ms: 0,
            width: w as u32,
            height: h as u32,
            rgba,
            badge_top_right: false,
        }
    }

    /// The Completed island's routing states, drawn offscreen. With `SCRAWL_PILL_STILLS=DIR` the
    /// stills, and frames of the morph from "Copied" to the question, are written there.
    #[test]
    fn route_chips_render_offscreen() {
        let mut pipelines = MetalPipelines::new(2.0).expect("MetalPipelines::new failed");
        let mut model = PillModel::new(atlas::measure_metrics());
        let completed = |label: &str, routes: &[&str]| Pres::Completed {
            label: label.into(),
            routes: routes.iter().map(|r| (*r).to_owned()).collect(),
        };
        let states = [
            ("1-copied", completed("Copied", &[])),
            (
                "2-ask",
                completed("Copied", &["Send to DESKTOP", "OPENCODE IOS"]),
            ),
            (
                "3-undo-window",
                completed("Sending to DESKTOP…", &["Send to OPENCODE IOS"]),
            ),
            ("4-sent", completed("Sent to OPENCODE IOS", &[])),
        ];
        let out = std::env::var_os("SCRAWL_PILL_STILLS").map(std::path::PathBuf::from);
        if let Some(dir) = &out {
            std::fs::create_dir_all(dir).unwrap();
        }
        let mut widths = Vec::new();
        for (name, pres) in &states {
            let frame = still(&mut pipelines, &mut model, pres, 1.5);
            assert!(model.is_at_rest(), "{name} didn't settle");
            let (_, items) = model::layout_of(pres, "0:00", model.labels, &model.metrics);
            widths.push(items.last().map_or(0.0, |i| i.rect[0] + i.rect[2]) - items[0].rect[0]);
            // Something was drawn: some pixels differ from the backdrop.
            assert!(
                frame.rgba.chunks_exact(4).any(|px| px[0] < 0x30),
                "{name} drew nothing"
            );
            if let Some(dir) = &out {
                crate::render::save_png(&frame, &dir.join(format!("{name}.png"))).unwrap();
            }
        }
        // The question is wider than plain Copied by its two chips and still fits the panel.
        assert!(widths[1] > widths[0] + 150.0, "{widths:?}");
        assert!(widths.iter().all(|w| *w < PANEL_W - 40.0), "{widths:?}");
        if let Some(dir) = &out {
            still(&mut pipelines, &mut model, &states[0].1, 1.5);
            for (i, t) in [0.0_f32, 0.05, 0.1, 0.15, 0.2, 0.3, 0.45]
                .into_iter()
                .enumerate()
            {
                let dt = t - if i == 0 {
                    0.0
                } else {
                    [0.0, 0.05, 0.1, 0.15, 0.2, 0.3][i - 1]
                };
                let frame = still(&mut pipelines, &mut model, &states[1].1, dt);
                crate::render::save_png(
                    &frame,
                    &dir.join(format!("morph-{:03}ms.png", (t * 1000.0) as u32)),
                )
                .unwrap();
            }
        }
    }
}
