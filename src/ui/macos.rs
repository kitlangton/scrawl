//! The macOS shell around the shared UI: the menu bar item and its menu, AppKit window levels
//! and styles for the pill and picker, the pill's CALayer shell (sharp border, shadow, GPU
//! scale and blur), Sparkle updates, and window screenshots for `scrawl preview`.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Arc;

use gpui::{AnyElement, App, Div, Window};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{AnyThread, DefinedClass, MainThreadOnly, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSCellImagePosition, NSColor, NSImage, NSMenu, NSMenuDelegate, NSMenuItem,
    NSStatusBar, NSStatusItem, NSStatusWindowLevel, NSWindow, NSWindowButton,
    NSWindowCollectionBehavior, NSWindowStyleMask,
};
use objc2_foundation::{MainThreadMarker, NSData, NSObject, NSSize, NSString};
use screencapturekit::prelude::*;
use screencapturekit::screenshot_manager::{CGImageExt, SCScreenshotManager};

use super::{ISLAND_H, LaserOverlay, MenuAction, PILL_WINDOW_H, PILL_WINDOW_W, motion};
use crate::capture::SharedCaptureTelemetry;

/// Where the file browser is called in buttons ("Show in Finder").
pub(super) const FILE_MANAGER: &str = "Finder";
/// The review header's leading inset, clear of the traffic lights.
pub(super) const HEADER_LEAD: f32 = 78.0;
pub(super) const HEADER_TRAIL: f32 = 16.0;
/// The pill's top edge, just below the 37 pt menu bar.
pub(super) const PILL_TOP: f32 = 34.0;

/// Global coordinates are points, as GPUI lays out.
pub(super) fn points_per_pixel(_window: &Window) -> f32 {
    1.0
}

/// The GPUI display that is the screen at `frame` (global points), else the primary one.
pub(super) fn display_at(
    frame: [f32; 4],
    cx: &App,
) -> Option<std::rc::Rc<dyn gpui::PlatformDisplay>> {
    let screens = crate::screens::displays();
    let id = crate::screens::index_for_rect(&screens, frame.map(f64::from)).map(|i| screens[i].id);
    id.and_then(|id| cx.displays().into_iter().find(|d| u32::from(d.id()) == id))
        .or_else(|| cx.primary_display())
}

/// A window covering the screen at `frame` lays out at its size: global coordinates are points.
pub(super) fn layout_size(
    frame: [f32; 4],
    _display: Option<&dyn gpui::PlatformDisplay>,
) -> gpui::Size<gpui::Pixels> {
    gpui::size(gpui::px(frame[2]), gpui::px(frame[3]))
}

/// Moves a window to cover the screen at `frame` (global points).
fn move_window_to_screen(ns_win: &NSWindow, frame: [f64; 4]) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(primary_h) = objc2_app_kit::NSScreen::screens(mtm)
        .firstObject()
        .map(|s| s.frame().size.height)
    else {
        return;
    };
    let [x, y, w, h] = crate::screens::flip(frame, primary_h);
    ns_win.setFrame_display(
        objc2_foundation::NSRect::new(
            objc2_foundation::NSPoint::new(x, y),
            objc2_foundation::NSSize::new(w, h),
        ),
        true,
    );
}

/// Moves the open picker to cover the screen at `frame` (global points).
pub(super) fn move_picker_window(frame: [f64; 4], _cx: &mut App) {
    if let Some(ns_win) = find_picker_ns_window() {
        move_window_to_screen(&ns_win, frame);
    }
}

/// GPUI places windows relative to their display on macOS.
pub(super) fn window_origin(
    _display: Option<&dyn gpui::PlatformDisplay>,
) -> gpui::Point<gpui::Pixels> {
    gpui::Point::default()
}

/// Where the GPUI pill window opens on `display`: centred, `PILL_TOP` below its top.
pub(super) fn hud_pill_origin(display: gpui::Bounds<gpui::Pixels>) -> gpui::Point<gpui::Pixels> {
    gpui::point(
        (display.size.width - gpui::px(PILL_WINDOW_W)) * 0.5,
        gpui::px(PILL_TOP),
    )
}

/// The GPUI pill (`SCRAWL_NATIVE_PILL=0`) stays where it opened on macOS.
pub(super) fn place_hud_pill(_point: [f64; 2]) {}

/// Dragging a region moves its window.
pub(super) fn window_drag_region(region: Div) -> Div {
    use gpui::{InteractiveElement, MouseButton};
    region.on_mouse_down(MouseButton::Left, |_, window, _| {
        window.start_window_move();
    })
}

/// A control inside a `window_drag_region` gets its clicks as it is.
pub(super) fn drag_region_control<E>(control: E) -> E {
    control
}

/// AppKit opens the menu itself (refreshed in menuNeedsUpdate:).
pub(super) fn poll_status_bar() {}

/// The traffic lights are native; nothing to draw.
pub(super) fn caption_buttons() -> Option<AnyElement> {
    None
}

/// Window levels keep the pill above the picker above the overlay.
pub(super) fn raise_window(_window: &mut Window) {}

/// `scrawl lab`'s backdrop (`w` × `h` points): borderless, under the capsule, letting clicks through.
pub(super) fn configure_lab_backdrop(w: f32, h: f32) {
    if let Some(mtm) = MainThreadMarker::new() {
        for win in NSApplication::sharedApplication(mtm).windows() {
            let f = win.frame().size;
            if (f.width - f64::from(w)).abs() < 1.0 && (f.height - f64::from(h)).abs() < 1.0 {
                win.setStyleMask(NSWindowStyleMask::Borderless);
                configure_floating_window(&win, true, 4);
            }
        }
    }
}

/// AppKit keeps an accessory app running without windows. Info.plist's `LSUIElement` starts
/// Scrawl as one with no Dock icon, but gpui 0.2.2 sets the regular activation policy once
/// launched, so this puts it back (`stay_out_of_dock`).
pub(super) fn keep_alive(_cx: &mut App) {
    stay_out_of_dock();
}

/// Keeps Scrawl a menu bar app with no Dock icon.
fn stay_out_of_dock() {
    use objc2_app_kit::NSApplicationActivationPolicy;
    if let Some(mtm) = MainThreadMarker::new() {
        NSApplication::sharedApplication(mtm)
            .setActivationPolicy(NSApplicationActivationPolicy::Accessory);
    }
}

/// AppKit gives every window the app's icon.
pub(super) fn decorate_app_window(_window: &mut Window) {}

/// The app that was in front when Scrawl activated itself for the picker or the review window
/// (0: none).
static PREVIOUS_FRONT_APP: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Called just before Scrawl activates itself to take the keys. Scrawl in front already (one
/// key window opening over another) keeps the app remembered first.
fn remember_front_app() {
    use objc2_app_kit::NSWorkspace;
    let front = NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .map_or(0, |app| app.processIdentifier());
    if front > 0 && front != std::process::id() as i32 {
        PREVIOUS_FRONT_APP.store(front, std::sync::atomic::Ordering::Release);
    }
}

/// A menu bar (accessory) app isn't active when its menu or capsule is used, and keys only
/// reach its windows while it is: remember the app in front, then activate ignoring other apps
/// (the cooperative `NSApp.activate()` may leave the window behind the app in front).
pub(super) fn take_keys(cx: &mut App) {
    remember_front_app();
    cx.activate(true);
}

/// Run (deferred) after any window closes: once neither a picker nor a review window is left,
/// the app that was in front before gets the keys back. `NSApp.deactivate()` isn't enough: a
/// menu bar (accessory) app stays frontmost with no windows, and keys go nowhere. Deferred, a
/// picker replacing another one is open again by the time this looks.
pub(super) fn hand_back_keys(cx: &mut App) {
    use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};
    if cx.windows().iter().any(|handle| {
        handle.downcast::<super::TargetPickerWindow>().is_some()
            || handle.downcast::<super::BundleReviewWindow>().is_some()
    }) {
        return;
    }
    let previous = PREVIOUS_FRONT_APP.swap(0, std::sync::atomic::Ordering::AcqRel);
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    if !app.isActive() {
        return;
    }
    let reactivated = previous > 0
        && NSRunningApplication::runningApplicationWithProcessIdentifier(previous)
            .is_some_and(|prev| prev.activateWithOptions(NSApplicationActivationOptions::empty()));
    if !reactivated {
        app.deactivate();
    }
}

pub(super) fn start_updates() {
    crate::sparkle::start();
}

pub(super) fn check_for_updates() {
    crate::sparkle::check_for_updates();
}

pub(super) fn open_overlay(
    telemetry: Arc<SharedCaptureTelemetry>,
    on: Option<[f64; 4]>,
    _cx: &mut gpui::Context<super::ScrawlCoordinator>,
) -> Result<LaserOverlay, String> {
    LaserOverlay::open(telemetry, on)
}

/// The picker: borderless at status level + 1 over the whole screen at `frame` (global points,
/// menu bar included), taking clicks (`scrawl lab` replays let them through). Returns its window
/// number, to leave out of the recording.
pub(super) fn configure_picker_window(
    _window: &mut Window,
    lab: bool,
    frame: [f64; 4],
    _cx: &mut App,
) -> Option<u32> {
    let ns_win = find_picker_ns_window()?;
    ns_win.setStyleMask(NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel);
    configure_floating_window(&ns_win, lab, 1);
    move_window_to_screen(&ns_win, frame);
    // gpui made it key before the style change above; make it key again so it takes the keys
    // once Scrawl is active (`take_keys`).
    if !lab {
        ns_win.makeKeyAndOrderFront(None);
    }
    let num = ns_win.windowNumber().max(0) as u32;
    (num > 0).then_some(num)
}

/// The pill's native window: a non-activating panel at status level + 3 with a CALayer shell.
pub(super) struct PillChrome {
    ns_window: Option<Retained<NSWindow>>,
    shell_layer: Option<Retained<objc2_quartz_core::CALayer>>,
}

impl PillChrome {
    pub(super) fn open(_window: &mut Window, _cx: &mut App) -> Self {
        let ns_window = find_hud_pill_ns_window();
        let mut shell_layer = None;
        if let Some(ref ns_win) = ns_window {
            ns_win.setStyleMask(
                NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            );
            unsafe {
                let sel = sel!(_setPreventsActivation:);
                let responds: bool = msg_send![&**ns_win, respondsToSelector: sel];
                if responds {
                    let _: () = msg_send![&**ns_win, _setPreventsActivation: true];
                }
            }
            configure_floating_window(ns_win, false, 3);
            ns_win.orderFrontRegardless();
            apply_pill_layer_effect(
                ns_win,
                &mut shell_layer,
                205.0,
                ISLAND_H,
                0.92,
                -6.0,
                0.0,
                false,
                3.5,
                0.015,
            );
        }
        Self {
            ns_window,
            shell_layer,
        }
    }

    pub(super) fn window_number(&self) -> Option<u32> {
        let num = self.ns_window.as_ref()?.windowNumber().max(0) as u32;
        (num > 0).then_some(num)
    }

    pub(super) fn has_shell(&self) -> bool {
        self.shell_layer.is_some()
    }

    pub(super) fn raise(&self) {
        if let Some(ref ns_win) = self.ns_window {
            ns_win.setLevel(NSStatusWindowLevel + 3);
            ns_win.orderFrontRegardless();
        }
    }

    /// One motion-trace frame for the capsule (`SCRAWL_MOTION_TRACE`), mapping its children.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn trace(
        &self,
        width: f32,
        scale: f32,
        y_offset: f32,
        opacity: f32,
        content_alpha: f32,
        blur_radius: f32,
        content_shrink: f32,
    ) -> Option<super::motion::Xf> {
        let ns_win = self.ns_window.as_ref()?;
        Some(trace_capsule(
            ns_win,
            width,
            scale,
            y_offset,
            opacity,
            content_alpha,
            blur_radius,
            content_shrink,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn apply(
        &mut self,
        pill_width: f32,
        pill_height: f32,
        scale: f32,
        y_offset: f32,
        opacity: f32,
        is_completed: bool,
        blur_radius: f32,
        content_shrink: f32,
    ) {
        if self.ns_window.is_none() {
            self.ns_window = find_hud_pill_ns_window();
        }
        if let Some(ref ns_win) = self.ns_window {
            apply_pill_layer_effect(
                ns_win,
                &mut self.shell_layer,
                pill_width,
                pill_height,
                scale,
                y_offset,
                opacity,
                is_completed,
                blur_radius,
                content_shrink,
            );
        }
    }
}

/// The largest of Scrawl's windows, brought to the front (for `scrawl preview --capture-screenshot`).
pub(super) fn front_own_window() -> Option<isize> {
    let mtm = MainThreadMarker::new()?;
    let app = NSApplication::sharedApplication(mtm);
    app.windows()
        .iter()
        .filter(|w| {
            let size = w.frame().size;
            size.width >= 600.0 && size.height >= 400.0
        })
        .max_by_key(|w| {
            let size = w.frame().size;
            (size.width as i64) * (size.height as i64)
        })
        .map(|w| {
            w.orderFrontRegardless();
            w.windowNumber()
        })
}
/// Whether macOS asks for reduced motion (Accessibility ▸ Display ▸ Reduce motion).
pub(super) fn reduce_motion() -> bool {
    use objc2::runtime::AnyClass;
    let Some(cls) = AnyClass::get(c"NSWorkspace") else {
        return false;
    };
    unsafe {
        let workspace: *mut AnyObject = msg_send![cls, sharedWorkspace];
        !workspace.is_null() && msg_send![workspace, accessibilityDisplayShouldReduceMotion]
    }
}

struct MenuTargetIvars {
    sender: flume::Sender<MenuAction>,
}

objc2::define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = MenuTargetIvars]
    struct ScrawlMenuTarget;

    unsafe impl NSObjectProtocol for ScrawlMenuTarget {}

    impl ScrawlMenuTarget {
        #[unsafe(method(toggleCapture:))]
        fn toggle_capture(&self, _sender: &AnyObject) {
            let _ = self.ivars().sender.try_send(MenuAction::ToggleCapture);
        }

        #[unsafe(method(chooseTarget:))]
        fn choose_target(&self, _sender: &AnyObject) {
            let _ = self.ivars().sender.try_send(MenuAction::ChooseTarget);
        }

        #[unsafe(method(openLatest:))]
        fn open_latest(&self, _sender: &AnyObject) {
            let _ = self.ivars().sender.try_send(MenuAction::OpenLatest);
        }

        #[unsafe(method(exportVideo:))]
        fn export_video(&self, _sender: &AnyObject) {
            let _ = self.ivars().sender.try_send(MenuAction::ExportVideo);
        }

        #[unsafe(method(openSettings:))]
        fn open_settings(&self, _sender: &AnyObject) {
            let _ = self.ivars().sender.try_send(MenuAction::Settings);
        }

        #[unsafe(method(checkForUpdates:))]
        fn check_for_updates(&self, _sender: &AnyObject) {
            let _ = self.ivars().sender.try_send(MenuAction::CheckForUpdates);
        }

        #[unsafe(method(quitApp:))]
        fn quit_app(&self, _sender: &AnyObject) {
            let _ = self.ivars().sender.try_send(MenuAction::Quit);
        }

        #[unsafe(method(openInboxItem:))]
        fn open_inbox_item(&self, sender: &NSMenuItem) {
            let path = sender
                .representedObject()
                .and_then(|object| object.downcast::<NSString>().ok())
                .map(|path| PathBuf::from(path.to_string()));
            if let Some(path) = path {
                let _ = self.ivars().sender.try_send(MenuAction::OpenInbox(path));
            }
        }
    }

    unsafe impl NSMenuDelegate for ScrawlMenuTarget {
        #[unsafe(method(menuNeedsUpdate:))]
        fn menu_needs_update(&self, menu: &NSMenu) {
            rebuild_inbox_menu_items(menu, self);
            if let Some(item) = menu.itemWithTag(RECORD_MENU_TAG) {
                item.setTitle(&NSString::from_str(&record_menu_title()));
            }
            if let Some(item) = menu.itemWithTag(EXPORT_VIDEO_MENU_TAG) {
                item.setHidden(!crate::handoff::video_export());
            }
        }
    }
);

/// Tag on the status menu's inbox rows, so each open can replace them.
const INBOX_MENU_TAG: isize = 0x5c4a;
/// Tag on **Export Last Capture as Video…**, shown only with `"videoExport": true`.
const EXPORT_VIDEO_MENU_TAG: isize = 0x5c4e;
/// Tag on **Record Screen**, whose title carries the record shortcut.
const RECORD_MENU_TAG: isize = 0x5c4d;

/// "Record Screen" with the record shortcut as it is now (Settings can change it).
fn record_menu_title() -> String {
    format!(
        "Record Screen  {}",
        crate::settings::Shortcuts::load().record.label
    )
}

/// Lists unhandled iPhone captures at the top of the status menu, read fresh from disk so the
/// ages are current.
fn rebuild_inbox_menu_items(menu: &NSMenu, target: &ScrawlMenuTarget) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    for index in (0..menu.numberOfItems()).rev() {
        if menu
            .itemAtIndex(index)
            .is_some_and(|item| item.tag() == INBOX_MENU_TAG)
        {
            menu.removeItemAtIndex(index);
        }
    }
    let inbox = crate::inbox::Inbox::load_default();
    if inbox.len() == 0 {
        return;
    }
    let now = crate::now_epoch_ms();
    let mut rows = Vec::new();
    let has_section_header: bool = unsafe {
        msg_send![objc2::class!(NSMenuItem), respondsToSelector: sel!(sectionHeaderWithTitle:)]
    };
    let title = NSString::from_str("From iPhone");
    rows.push(if has_section_header {
        NSMenuItem::sectionHeaderWithTitle(&title, mtm)
    } else {
        let header = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &title,
                None,
                &NSString::new(),
            )
        };
        header.setEnabled(false);
        header
    });
    let icon = NSImage::imageWithSystemSymbolName_accessibilityDescription(
        &NSString::from_str("iphone"),
        None,
    );
    for entry in inbox.entries() {
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(&entry.label(now)),
                Some(sel!(openInboxItem:)),
                &NSString::new(),
            )
        };
        unsafe {
            item.setTarget(Some(target));
            item.setRepresentedObject(Some(&NSString::from_str(&entry.dir.display().to_string())));
        }
        item.setImage(icon.as_deref());
        rows.push(item);
    }
    rows.push(NSMenuItem::separatorItem(mtm));
    for (index, item) in rows.iter().enumerate() {
        item.setTag(INBOX_MENU_TAG);
        menu.insertItem_atIndex(item, index as isize);
    }
}

pub(super) fn set_status_text(text: &str) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    STATUS_BAR.with(|slot| {
        let slot = slot.borrow();
        let Some(button) = slot.as_ref().and_then(|bar| bar._item.button(mtm)) else {
            return;
        };
        button.setImagePosition(NSCellImagePosition::ImageLeft);
        button.setTitle(&NSString::from_str(text));
    });
}

/// Shows the unhandled iPhone capture count beside the menu bar icon.
pub(super) fn set_status_inbox_count(count: usize) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    STATUS_BAR.with(|slot| {
        let slot = slot.borrow();
        let Some(button) = slot.as_ref().and_then(|bar| bar._item.button(mtm)) else {
            return;
        };
        button.setImagePosition(if count == 0 {
            NSCellImagePosition::ImageOnly
        } else {
            NSCellImagePosition::ImageLeft
        });
        button.setTitle(&NSString::from_str(&if count == 0 {
            String::new()
        } else {
            count.to_string()
        }));
    });
}

impl ScrawlMenuTarget {
    fn new(sender: flume::Sender<MenuAction>, mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(MenuTargetIvars { sender });
        unsafe { msg_send![super(this), init] }
    }
}

struct StatusBarController {
    _item: Retained<NSStatusItem>,
    _menu: Retained<NSMenu>,
    _target: Retained<ScrawlMenuTarget>,
}

thread_local! {
    static STATUS_BAR: RefCell<Option<StatusBarController>> = const { RefCell::new(None) };
}

pub(super) fn install_status_bar() -> Option<flume::Receiver<MenuAction>> {
    let mtm = MainThreadMarker::new()?;
    let (sender, receiver) = flume::bounded(16);
    let target = ScrawlMenuTarget::new(sender, mtm);
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Scrawl"));

    let add = |title: &str, action: objc2::runtime::Sel| {
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                Some(action),
                &NSString::new(),
            )
        };
        unsafe { item.setTarget(Some(&target)) };
        menu.addItem(&item);
    };

    add(&record_menu_title(), sel!(toggleCapture:));
    if let Some(item) = menu.itemAtIndex(menu.numberOfItems() - 1) {
        item.setTag(RECORD_MENU_TAG);
    }
    add("Choose What to Record…", sel!(chooseTarget:));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    add("Open Latest Capture", sel!(openLatest:));
    add("Export Last Capture as Video…", sel!(exportVideo:));
    if let Some(item) = menu.itemAtIndex(menu.numberOfItems() - 1) {
        item.setTag(EXPORT_VIDEO_MENU_TAG);
        item.setHidden(!crate::handoff::video_export());
    }
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    add("Settings…", sel!(openSettings:));
    if let Some(item) = menu.itemAtIndex(menu.numberOfItems() - 1) {
        item.setKeyEquivalent(&NSString::from_str(","));
    }
    add("Check for Updates…", sel!(checkForUpdates:));
    add("Quit Scrawl", sel!(quitApp:));
    if let Some(item) = menu.itemAtIndex(menu.numberOfItems() - 1) {
        item.setKeyEquivalent(&NSString::from_str("q"));
    }

    menu.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*target)));

    let item = NSStatusBar::systemStatusBar().statusItemWithLength(-1.0);
    if let Some(button) = item.button(mtm) {
        let mb_bytes = include_bytes!("../../assets/menubar-icon@2x.png");
        let mb_data = NSData::with_bytes(mb_bytes);
        if let Some(img) = NSImage::initWithData(NSImage::alloc(), &mb_data) {
            img.setSize(NSSize::new(18.0, 18.0));
            img.setTemplate(true);
            button.setImage(Some(&img));
        } else {
            button.setTitle(&NSString::from_str("✎"));
        }
    }
    let app_bytes = include_bytes!("../../assets/app-icon.png");
    let app_data = NSData::with_bytes(app_bytes);
    if let Some(app_img) = NSImage::initWithData(NSImage::alloc(), &app_data) {
        unsafe {
            NSApplication::sharedApplication(mtm).setApplicationIconImage(Some(&app_img));
        }
    }
    item.setMenu(Some(&menu));

    STATUS_BAR.with(|slot| {
        *slot.borrow_mut() = Some(StatusBarController {
            _item: item,
            _menu: menu,
            _target: target,
        });
    });
    Some(receiver)
}

fn find_hud_pill_ns_window() -> Option<Retained<NSWindow>> {
    let mtm = MainThreadMarker::new()?;
    let app = NSApplication::sharedApplication(mtm);
    for win in app.windows() {
        let size = win.frame().size;
        if ((size.width - f64::from(PILL_WINDOW_W)).abs() < 8.0
            || (size.width - f64::from(crate::native::pill::model::PANEL_W)).abs() < 8.0)
            && (size.height - f64::from(PILL_WINDOW_H)).abs() < 8.0
        {
            return Some(win);
        }
    }
    None
}

/// Whether the pointer is over the HUD pill's window: the capsule plus a small margin, so the
/// switch chips stay while the user reaches for them.
pub(super) fn pointer_on_hud_pill() -> bool {
    use objc2::runtime::AnyClass;
    use objc2_foundation::NSPoint;
    let (Some(win), Some(cls)) = (find_hud_pill_ns_window(), AnyClass::get(c"NSEvent")) else {
        return false;
    };
    let p: NSPoint = unsafe { msg_send![cls, mouseLocation] };
    let f = win.frame();
    p.x >= f.origin.x
        && p.x <= f.origin.x + f.size.width
        && p.y >= f.origin.y
        && p.y <= f.origin.y + f.size.height
}

fn find_picker_ns_window() -> Option<Retained<NSWindow>> {
    let mtm = MainThreadMarker::new()?;
    let app = NSApplication::sharedApplication(mtm);
    for win in app.windows() {
        if win.ignoresMouseEvents() {
            continue;
        }
        let size = win.frame().size;
        if size.width >= 600.0
            && size.height >= 200.0
            && (size.width - 1120.0).abs() > 8.0
            && (size.width - f64::from(PILL_WINDOW_W)).abs() > 8.0
        {
            return Some(win);
        }
    }
    None
}

fn sync_full_size_content_view(ns_window: &NSWindow) {
    for kind in [
        NSWindowButton::CloseButton,
        NSWindowButton::MiniaturizeButton,
        NSWindowButton::ZoomButton,
    ] {
        if let Some(button) = ns_window.standardWindowButton(kind) {
            button.setHidden(true);
        }
    }
    if let Some(cv) = ns_window.contentView() {
        let bounds = cv.bounds();
        for sub in cv.subviews() {
            sub.setFrame(bounds);
        }
        if let Some(frame_view) = unsafe { cv.superview() } {
            for sub in frame_view.subviews() {
                if sub
                    .class()
                    .name()
                    .to_bytes()
                    .windows(8)
                    .any(|w| w == b"Titlebar")
                {
                    sub.setHidden(true);
                }
            }
        }
    }
}

fn configure_floating_window(ns_window: &NSWindow, ignores_mouse: bool, level_offset: isize) {
    ns_window.setHidesOnDeactivate(false);
    ns_window.setIgnoresMouseEvents(ignores_mouse);
    ns_window.setOpaque(false);
    ns_window.setBackgroundColor(Some(&NSColor::clearColor()));
    ns_window.setHasShadow(false);
    ns_window.setLevel(NSStatusWindowLevel + level_offset);
    ns_window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    sync_full_size_content_view(ns_window);
}

#[repr(C)]
struct OpaqueCGColor {
    _priv: [u8; 0],
}

unsafe impl objc2::RefEncode for OpaqueCGColor {
    const ENCODING_REF: objc2::Encoding =
        objc2::Encoding::Pointer(&objc2::Encoding::Struct("CGColor", &[]));
}

#[repr(C)]
#[derive(Clone, Copy)]
struct CATransform3D {
    m11: f64,
    m12: f64,
    m13: f64,
    m14: f64,
    m21: f64,
    m22: f64,
    m23: f64,
    m24: f64,
    m31: f64,
    m32: f64,
    m33: f64,
    m34: f64,
    m41: f64,
    m42: f64,
    m43: f64,
    m44: f64,
}

unsafe impl objc2::Encode for CATransform3D {
    const ENCODING: objc2::Encoding = objc2::Encoding::Struct(
        "CATransform3D",
        &[
            f64::ENCODING,
            f64::ENCODING,
            f64::ENCODING,
            f64::ENCODING,
            f64::ENCODING,
            f64::ENCODING,
            f64::ENCODING,
            f64::ENCODING,
            f64::ENCODING,
            f64::ENCODING,
            f64::ENCODING,
            f64::ENCODING,
            f64::ENCODING,
            f64::ENCODING,
            f64::ENCODING,
            f64::ENCODING,
        ],
    );
}

/// Two-layer Dynamic Island effect on `HudPillWindow`:
/// 1. `shell_layer` (inserted behind GPUI's `contentView.layer()`) renders the outer black pill
///    capsule, `1px` specular border, and drop shadow with **zero blur** so the pill's border and
///    silhouette are always 100% razor-sharp.
/// 2. `contentView.layer()` (which renders only the inner chips/text on a transparent background)
///    receives `CAFilter("gaussianBlur")` + `CATransform3D` scale so only the inner contents blur.
fn apply_pill_layer_effect(
    ns_window: &NSWindow,
    shell_slot: &mut Option<Retained<objc2_quartz_core::CALayer>>,
    pill_width: f32,
    pill_height: f32,
    scale: f32,
    y_offset: f32,
    opacity: f32,
    is_completed: bool,
    blur_radius: f32,
    // Content scale lost per px of blur (0.015 for whole-state morphs, 0 to keep it still).
    content_shrink: f32,
) {
    use objc2::runtime::AnyClass;
    use objc2_foundation::{NSArray, NSNumber, NSPoint, NSRect, NSSize};
    use objc2_quartz_core::{CALayer, CATransaction};

    let Some(cv) = ns_window.contentView() else {
        return;
    };
    cv.setWantsLayer(true);
    let Some(cv_layer) = cv.layer() else {
        return;
    };
    // GPUI attaches its `CAMetalLayer` to its child `GPUIView` (`cv.subviews().firstObject()`).
    let gpui_view = cv.subviews().firstObject();
    let content_layer = gpui_view
        .as_ref()
        .and_then(|v| {
            v.setWantsLayer(true);
            v.layer()
        })
        .unwrap_or_else(|| cv_layer.clone());

    let bounds = cv.bounds();
    let cx = bounds.size.width * 0.5;
    let cy = bounds.size.height * 0.5;
    let s = f64::from(scale.clamp(0.50, 1.15));
    let ty_unflipped = -f64::from(y_offset);
    let is_flipped = gpui_view
        .as_ref()
        .map_or_else(|| cv.isFlipped(), |v| v.isFlipped());
    let ty_cv = if is_flipped {
        f64::from(y_offset)
    } else {
        -f64::from(y_offset)
    };

    if shell_slot.is_none() {
        let shell = CALayer::new();
        cv_layer.insertSublayer_below(&shell, Some(&content_layer));
        *shell_slot = Some(shell);
    }

    let w_scaled = f64::from(pill_width) * s;
    let h_scaled = f64::from(pill_height) * s;
    let shell_rect = NSRect::new(
        NSPoint::new(cx - w_scaled * 0.5, cy - h_scaled * 0.5 + ty_unflipped),
        NSSize::new(w_scaled, h_scaled),
    );

    // Content scales subtly (`0.93 .. 1.0`) inside the crisp outer shell during morphs.
    let content_s = content_scale(scale, blur_radius, content_shrink);
    let transform = CATransform3D {
        m11: content_s,
        m12: 0.0,
        m13: 0.0,
        m14: 0.0,
        m21: 0.0,
        m22: content_s,
        m23: 0.0,
        m24: 0.0,
        m31: 0.0,
        m32: 0.0,
        m33: 1.0,
        m34: 0.0,
        m41: cx * (1.0 - content_s),
        m42: cy * (1.0 - content_s) + ty_cv,
        m43: 0.0,
        m44: 1.0,
    };

    CATransaction::begin();
    CATransaction::setDisableActions(true);
    if let Some(ref shell) = *shell_slot {
        shell.setFrame(shell_rect);
        shell.setCornerRadius(h_scaled * 0.5);
        shell.setBorderWidth(1.0);
        shell.setOpacity(opacity);
        shell.setShadowOpacity(0.40 * opacity);
        shell.setShadowRadius(14.0);
        shell.setShadowOffset(NSSize::new(0.0, -5.0));
        unsafe {
            let bg_ns = NSColor::colorWithSRGBRed_green_blue_alpha(
                20.0 / 255.0,
                20.0 / 255.0,
                22.0 / 255.0,
                0.96,
            );
            let bg_cg: *const OpaqueCGColor = msg_send![&*bg_ns, CGColor];
            let _: () = msg_send![&**shell, setBackgroundColor: bg_cg];

            let border_ns = if is_completed {
                NSColor::colorWithSRGBRed_green_blue_alpha(
                    105.0 / 255.0,
                    216.0 / 255.0,
                    159.0 / 255.0,
                    0.34,
                )
            } else {
                NSColor::colorWithSRGBRed_green_blue_alpha(1.0, 1.0, 1.0, 0.12)
            };
            let border_cg: *const OpaqueCGColor = msg_send![&*border_ns, CGColor];
            let _: () = msg_send![&**shell, setBorderColor: border_cg];
        }
    }

    unsafe {
        let _: () = msg_send![&*content_layer, setTransform: transform];
        if blur_radius > 0.18
            && let Some(ca_filter_cls) = AnyClass::get(c"CAFilter")
        {
            let filter_type = NSString::from_str("gaussianBlur");
            let filter: *mut AnyObject = msg_send![ca_filter_cls, filterWithType: &*filter_type];
            if !filter.is_null() {
                let radius_num = NSNumber::new_f64(f64::from(blur_radius));
                let radius_key = NSString::from_str("inputRadius");
                let _: () = msg_send![filter, setValue: &*radius_num, forKey: &*radius_key];
                let filter_ref = &*filter;
                let filters = NSArray::from_slice(&[filter_ref]);
                content_layer.setFilters(Some(&filters));
            }
        } else {
            content_layer.setFilters(None);
        }
    }
    CATransaction::commit();
}
/// The content layer's scale: the capsule's, less a little per px of blur.
fn content_scale(scale: f32, blur_radius: f32, content_shrink: f32) -> f64 {
    let s = f64::from(scale.clamp(0.50, 1.15));
    (s * (1.0 - f64::from(blur_radius).min(5.0) * f64::from(content_shrink))).clamp(0.85, 1.06)
}

/// The pill window's frame in screen points with a top-left origin (the primary display's).
fn top_left_frame(ns_window: &NSWindow) -> [f32; 4] {
    let f = ns_window.frame();
    let primary_h = MainThreadMarker::new()
        .and_then(|mtm| objc2_app_kit::NSScreen::screens(mtm).firstObject())
        .map_or(0.0, |screen| screen.frame().size.height);
    [
        f.origin.x as f32,
        (primary_h - f.origin.y - f.size.height) as f32,
        f.size.width as f32,
        f.size.height as f32,
    ]
}

/// One trace frame for the capsule (`SCRAWL_MOTION_TRACE`): its window, the shell as drawn, the
/// content's blur and opacity. Returns the mapping for the content's children.
#[allow(clippy::too_many_arguments)]
fn trace_capsule(
    ns_window: &NSWindow,
    width: f32,
    scale: f32,
    y_off: f32,
    opacity: f32,
    content_alpha: f32,
    blur_radius: f32,
    content_shrink: f32,
) -> motion::Xf {
    let win = top_left_frame(ns_window);
    let (cx, cy) = (win[2] * 0.5, win[3] * 0.5);
    let s = scale.clamp(0.50, 1.15);
    let (w, h) = (width * s, ISLAND_H * s);
    let shell = [win[0] + cx - w * 0.5, win[1] + cy - h * 0.5 + y_off, w, h];
    motion::trace_element("window", win, 0.0, 0.0, None, None);
    motion::trace_element("shell", shell, opacity, h * 0.5, None, None);
    motion::trace_channel("content.blur", "level", blur_radius);
    motion::trace_channel("content.alpha", "level", content_alpha);
    motion::trace_channel("shell.width", "position", width);
    motion::Xf {
        s: content_scale(scale, blur_radius, content_shrink) as f32,
        cx,
        cy,
        ty: y_off,
        ox: win[0],
        oy: win[1],
        alpha: opacity * content_alpha,
        clip: shell,
    }
}

pub(super) fn capture_own_window_screenshot(
    window_num: Option<isize>,
    out_path: &std::path::Path,
) -> anyhow::Result<()> {
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if let Some(num) = window_num
        && num > 0
    {
        if let Some(frame) = capture_cg_window_rgba(num as u32) {
            crate::render::save_png(&frame, out_path)?;
            return Ok(());
        }
        if std::process::Command::new("/usr/sbin/screencapture")
            .args(["-x", "-l", &num.to_string()])
            .arg(out_path)
            .status()
            .is_ok_and(|s| s.success())
            && out_path.exists()
        {
            return Ok(());
        }
    }
    let pid = std::process::id() as i32;
    let content = SCShareableContent::get()
        .map_err(|e| anyhow::anyhow!("ScreenCaptureKit unavailable: {e}"))?;
    let window = content
        .windows()
        .into_iter()
        .filter(|w| {
            w.is_on_screen()
                && w.owning_application()
                    .as_ref()
                    .is_some_and(|a| a.process_id() == pid)
        })
        .max_by_key(|w| (w.frame().size.width as u64) * (w.frame().size.height as u64))
        .ok_or_else(|| anyhow::anyhow!("Could not find Scrawl window"))?;
    let frame = window.frame();
    let filter = SCContentFilter::create()
        .with_window(&window)
        .build()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let config = SCStreamConfiguration::new()
        .with_width((frame.size.width as u32 * 2).max(400))
        .with_height((frame.size.height as u32 * 2).max(300))
        .with_shows_cursor(false);
    let image =
        SCScreenshotManager::capture_image(&filter, &config).map_err(|e| anyhow::anyhow!("{e}"))?;
    image
        .save(
            out_path
                .to_str()
                .ok_or_else(|| anyhow::anyhow!("invalid output path"))?,
            screencapturekit::screenshot_manager::ImageFormat::Png,
        )
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(())
}

fn capture_cg_window_rgba(window_id: u32) -> Option<crate::model::RgbaFrame> {
    use core_graphics::geometry::{CGPoint, CGRect, CGSize};
    use std::ffi::c_void;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGWindowListCreateImage(
            screen_bounds: CGRect,
            list_option: u32,
            window_id: u32,
            image_option: u32,
        ) -> *mut c_void;
        fn CGImageGetWidth(image: *const c_void) -> usize;
        fn CGImageGetHeight(image: *const c_void) -> usize;
        fn CGColorSpaceCreateDeviceRGB() -> *mut c_void;
        fn CGBitmapContextCreate(
            data: *mut c_void,
            width: usize,
            height: usize,
            bits_per_component: usize,
            bytes_per_row: usize,
            space: *const c_void,
            bitmap_info: u32,
        ) -> *mut c_void;
        fn CGContextDrawImage(context: *mut c_void, rect: CGRect, image: *const c_void);
        fn CFRelease(cf: *const c_void);
    }

    let null_rect = CGRect::new(
        &CGPoint::new(f64::INFINITY, f64::INFINITY),
        &CGSize::new(0.0, 0.0),
    );
    // kCGWindowListOptionIncludingWindow = 1 << 3 (8), kCGWindowImageBoundsIgnoreFraming = 1
    let cg_image = unsafe { CGWindowListCreateImage(null_rect, 8, window_id, 1) };
    if cg_image.is_null() {
        return None;
    }
    let width = unsafe { CGImageGetWidth(cg_image) };
    let height = unsafe { CGImageGetHeight(cg_image) };
    if width == 0 || height == 0 {
        unsafe { CFRelease(cg_image) };
        return None;
    }
    let mut rgba = vec![0_u8; width * height * 4];
    unsafe {
        let cs = CGColorSpaceCreateDeviceRGB();
        // kCGImageAlphaPremultipliedLast = 1
        let ctx =
            CGBitmapContextCreate(rgba.as_mut_ptr().cast(), width, height, 8, width * 4, cs, 1);
        if !ctx.is_null() {
            let draw_rect = CGRect::new(
                &CGPoint::new(0.0, 0.0),
                &CGSize::new(width as f64, height as f64),
            );
            CGContextDrawImage(ctx, draw_rect, cg_image);
            CFRelease(ctx);
        }
        if !cs.is_null() {
            CFRelease(cs);
        }
        CFRelease(cg_image);
    }
    Some(crate::model::RgbaFrame::new(
        0,
        width as u32,
        height as u32,
        rgba,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reveal_appkit_queries_pass_objc2_verification() {
        let _ = reduce_motion();
        let cls = objc2::runtime::AnyClass::get(c"NSEvent").expect("NSEvent");
        let _: objc2_foundation::NSPoint = unsafe { msg_send![cls, mouseLocation] };
    }

    #[test]
    fn ca_layer_and_filter_encodings_pass_objc2_verification() {
        use objc2::runtime::AnyClass;
        use objc2_foundation::{NSArray, NSNumber};
        use objc2_quartz_core::{CALayer, CATransaction};

        let shell = CALayer::new();
        let content_layer = CALayer::new();
        let transform = CATransform3D {
            m11: 0.9,
            m12: 0.0,
            m13: 0.0,
            m14: 0.0,
            m21: 0.0,
            m22: 0.9,
            m23: 0.0,
            m24: 0.0,
            m31: 0.0,
            m32: 0.0,
            m33: 1.0,
            m34: 0.0,
            m41: 10.0,
            m42: 4.0,
            m43: 0.0,
            m44: 1.0,
        };
        CATransaction::begin();
        CATransaction::setDisableActions(true);
        unsafe {
            let bg_ns = NSColor::colorWithSRGBRed_green_blue_alpha(0.1, 0.1, 0.1, 0.95);
            let bg_cg: *const OpaqueCGColor = msg_send![&*bg_ns, CGColor];
            let _: () = msg_send![&*shell, setBackgroundColor: bg_cg];
            let _: () = msg_send![&*shell, setBorderColor: bg_cg];
            let _: () = msg_send![&*content_layer, setTransform: transform];
            if let Some(ca_filter_cls) = AnyClass::get(c"CAFilter") {
                let filter_type = NSString::from_str("gaussianBlur");
                let filter: *mut AnyObject =
                    msg_send![ca_filter_cls, filterWithType: &*filter_type];
                assert!(!filter.is_null());
                let radius_num = NSNumber::new_f64(6.0);
                let radius_key = NSString::from_str("inputRadius");
                let _: () = msg_send![filter, setValue: &*radius_num, forKey: &*radius_key];
                let filter_ref = &*filter;
                let filters = NSArray::from_slice(&[filter_ref]);
                content_layer.setFilters(Some(&filters));
            }
            content_layer.setFilters(None);
        }
        CATransaction::commit();
    }
}
