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
    NSApplication, NSAutoresizingMaskOptions, NSBezierPath, NSButton, NSCellImagePosition, NSColor,
    NSCompositingOperation, NSEvent, NSFont, NSFontWeightRegular, NSImage,
    NSImageSymbolConfiguration, NSImageView, NSLineBreakMode, NSMenu, NSMenuDelegate, NSMenuItem,
    NSStatusBar, NSStatusItem, NSStatusWindowLevel, NSTextField, NSTrackingArea,
    NSTrackingAreaOptions, NSView, NSWindow, NSWindowButton, NSWindowCollectionBehavior,
    NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSData, NSDate, NSDateFormatter, NSDateFormatterStyle, NSObject, NSPoint,
    NSRect, NSSize, NSString,
};
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

        #[unsafe(method(showAllCaptures:))]
        fn show_all_captures(&self, _sender: &AnyObject) {
            let dir = crate::handoff::default_captures_dir();
            if let Err(error) = crate::handoff::open_path(&dir) {
                crate::capture::scrawl_log(format!("Show All Captures failed: {error:#}"));
            }
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
            rebuild_recent_menu_items(menu, self);
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
/// Tag on the status menu's Recent Captures rows, so each open can replace them.
const RECENT_MENU_TAG: isize = 0x5c4f;

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

/// Lists the newest captures below **Open Latest Capture**, each with its contact sheet, read fresh
/// from disk so the titles and ages are current. Clicking one opens it in the review window, the
/// same path as an iPhone row. Captures already listed under From iPhone are left out.
fn rebuild_recent_menu_items(menu: &NSMenu, target: &ScrawlMenuTarget) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    for index in (0..menu.numberOfItems()).rev() {
        if menu
            .itemAtIndex(index)
            .is_some_and(|item| item.tag() == RECENT_MENU_TAG)
        {
            menu.removeItemAtIndex(index);
        }
    }
    // Behind the `recentCaptures` experiment until Kit turns it on for everyone (AGENTS.md).
    if !crate::handoff::experiment("recentCaptures") {
        return;
    }
    let inbox = crate::inbox::Inbox::load_default();
    let skip: Vec<PathBuf> = inbox
        .entries()
        .iter()
        .map(|entry| entry.dir.clone())
        .collect();
    let captures = crate::recent::recent_captures(
        &crate::handoff::default_captures_dir(),
        crate::recent::MENU_LIMIT,
        &skip,
    );
    if captures.is_empty() {
        return;
    }
    // Below Export Last Capture as Video… (present even while hidden), in its own section.
    let anchor = menu.indexOfItemWithTag(EXPORT_VIDEO_MENU_TAG);
    if anchor < 0 {
        return;
    }
    let now = crate::now_epoch_ms();
    let mut rows = vec![NSMenuItem::separatorItem(mtm)];
    let has_section_header: bool = unsafe {
        msg_send![objc2::class!(NSMenuItem), respondsToSelector: sel!(sectionHeaderWithTitle:)]
    };
    let title = NSString::from_str("Recent Captures");
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
    for capture in &captures {
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(&capture.label(now)),
                Some(sel!(openInboxItem:)),
                &NSString::new(),
            )
        };
        // The item keeps its title and action for VoiceOver and the keyboard (↵ opens it); the
        // row view draws it like CleanShot's: thumbnail, title, and a copy button.
        unsafe {
            item.setTarget(Some(target));
            item.setRepresentedObject(Some(&NSString::from_str(
                &capture.dir.display().to_string(),
            )));
        }
        let row = RecentCaptureRow::new(mtm, capture, target.ivars().sender.clone());
        item.setView(Some(&row));
        rows.push(item);
    }
    let show_all = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str("Show All Captures in Finder"),
            Some(sel!(showAllCaptures:)),
            &NSString::new(),
        )
    };
    unsafe { show_all.setTarget(Some(target)) };
    rows.push(show_all);
    prune_recent_thumbnails(&captures);
    for (offset, item) in rows.iter().enumerate() {
        item.setTag(RECENT_MENU_TAG);
        menu.insertItem_atIndex(item, anchor + 1 + offset as isize);
    }
}

/// A Recent Captures row is this wide, so a long title doesn't widen the menu (CleanShot's rows
/// are fixed the same way), and this tall, to fit the thumbnail.
const RECENT_ROW_W: f64 = 320.0;
const RECENT_ROW_H: f64 = 40.0;
/// Every thumbnail is this size, cut from its source to fill it, so the rows line up.
const RECENT_THUMB_W: f64 = 42.0;
const RECENT_THUMB_H: f64 = 28.0;
/// Where the row's content starts, matching a standard menu item's inset.
const RECENT_INSET: f64 = 14.0;
const RECENT_BUTTON: f64 = 20.0;

thread_local! {
    /// Thumbnails by source image, so reopening the menu doesn't decode the frames again.
    static RECENT_THUMBNAILS: RefCell<std::collections::HashMap<PathBuf, Retained<NSImage>>> =
        RefCell::new(std::collections::HashMap::new());
}

/// When a capture was made, in the system's own words and locale: "Today, 8:55 AM",
/// "Yesterday, 2:21 PM", else the short date and time.
fn capture_time(created_at_ms: u64) -> Retained<NSString> {
    let formatter = NSDateFormatter::new();
    formatter.setDateStyle(NSDateFormatterStyle::ShortStyle);
    formatter.setTimeStyle(NSDateFormatterStyle::ShortStyle);
    formatter.setDoesRelativeDateFormatting(true);
    formatter.stringFromDate(&NSDate::dateWithTimeIntervalSince1970(
        created_at_ms as f64 / 1000.0,
    ))
}

/// Drops thumbnails for captures no longer listed.
fn prune_recent_thumbnails(captures: &[crate::recent::RecentCapture]) {
    RECENT_THUMBNAILS.with(|cache| {
        cache.borrow_mut().retain(|source, _| {
            captures
                .iter()
                .any(|capture| capture.thumbnail_source.as_ref() == Some(source))
        })
    });
}

/// A `RECENT_THUMB_W` × `RECENT_THUMB_H` thumbnail with rounded corners, cut from the middle of
/// `source` (the top, for a tall source, where a window's content starts) so it fills the space
/// without stretching.
fn recent_thumbnail(source: &std::path::Path) -> Option<Retained<NSImage>> {
    if let Some(cached) = RECENT_THUMBNAILS.with(|cache| cache.borrow().get(source).cloned()) {
        return Some(cached);
    }
    let image = NSImage::initWithContentsOfFile(
        NSImage::alloc(),
        &NSString::from_str(&source.display().to_string()),
    )?;
    let size = image.size();
    if size.width <= 0.0 || size.height <= 0.0 {
        return None;
    }
    let target = RECENT_THUMB_W / RECENT_THUMB_H;
    // AppKit image coordinates start at the bottom left.
    let crop = if size.width / size.height > target {
        let width = size.height * target;
        NSRect::new(
            NSPoint::new((size.width - width) / 2.0, 0.0),
            NSSize::new(width, size.height),
        )
    } else {
        let height = size.width / target;
        NSRect::new(
            NSPoint::new(0.0, size.height - height),
            NSSize::new(size.width, height),
        )
    };
    let draw = block2::RcBlock::new(move |rect: NSRect| -> objc2::runtime::Bool {
        NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, 4.0, 4.0).addClip();
        image.drawInRect_fromRect_operation_fraction(
            rect,
            crop,
            NSCompositingOperation::SourceOver,
            1.0,
        );
        objc2::runtime::Bool::YES
    });
    let thumbnail = NSImage::imageWithSize_flipped_drawingHandler(
        NSSize::new(RECENT_THUMB_W, RECENT_THUMB_H),
        false,
        &draw,
    );
    RECENT_THUMBNAILS.with(|cache| {
        cache
            .borrow_mut()
            .insert(source.to_path_buf(), thumbnail.clone())
    });
    Some(thumbnail)
}

struct RecentRowIvars {
    dir: PathBuf,
    sender: flume::Sender<MenuAction>,
    label: Retained<NSTextField>,
    details: Retained<NSTextField>,
    when: Retained<NSTextField>,
    copy: Retained<RecentCopyButton>,
    /// The highlight the subviews were last colored for, so drawing doesn't recolor every pass.
    highlighted: std::cell::Cell<bool>,
}

struct CopyButtonIvars {
    /// The pointer is over the button.
    hovered: std::cell::Cell<bool>,
    /// Its row is highlighted, so the circle sits on the accent color.
    on_highlight: std::cell::Cell<bool>,
}

objc2::define_class!(
    /// The clipboard button at the end of a Recent Captures row: an icon in a circle that lights
    /// up while the pointer is over it, like the link button on CleanShot X's recent captures.
    #[unsafe(super = NSButton)]
    #[thread_kind = MainThreadOnly]
    #[ivars = CopyButtonIvars]
    struct RecentCopyButton;

    unsafe impl NSObjectProtocol for RecentCopyButton {}

    impl RecentCopyButton {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, dirty: NSRect) {
            let ivars = self.ivars();
            let fill = match (ivars.on_highlight.get(), ivars.hovered.get()) {
                (true, true) => NSColor::whiteColor().colorWithAlphaComponent(0.35),
                (true, false) => NSColor::whiteColor().colorWithAlphaComponent(0.18),
                (false, true) => NSColor::tertiaryLabelColor(),
                (false, false) => NSColor::quaternaryLabelColor(),
            };
            fill.setFill();
            NSBezierPath::bezierPathWithOvalInRect(self.bounds()).fill();
            let _: () = unsafe { msg_send![super(self), drawRect: dirty] };
        }

        #[unsafe(method(updateTrackingAreas))]
        fn update_tracking_areas(&self) {
            for area in self.trackingAreas().iter() {
                self.removeTrackingArea(&area);
            }
            // ActiveAlways: a menu tracks the mouse itself, so the app is never "active" here.
            // EnabledDuringMouseDrag: a press on the menu bar icon can drag straight onto it.
            let options = NSTrackingAreaOptions::MouseEnteredAndExited
                | NSTrackingAreaOptions::ActiveAlways
                | NSTrackingAreaOptions::EnabledDuringMouseDrag
                | NSTrackingAreaOptions::InVisibleRect;
            let area = unsafe {
                NSTrackingArea::initWithRect_options_owner_userInfo(
                    NSTrackingArea::alloc(),
                    self.bounds(),
                    options,
                    Some(self),
                    None,
                )
            };
            self.addTrackingArea(&area);
            let _: () = unsafe { msg_send![super(self), updateTrackingAreas] };
        }

        #[unsafe(method(mouseEntered:))]
        fn mouse_entered(&self, _event: &NSEvent) {
            self.ivars().hovered.set(true);
            NSView::setNeedsDisplay(self, true);
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            self.ivars().hovered.set(false);
            NSView::setNeedsDisplay(self, true);
        }
    }
);

impl RecentCopyButton {
    fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = mtm.alloc::<Self>().set_ivars(CopyButtonIvars {
            hovered: std::cell::Cell::new(false),
            on_highlight: std::cell::Cell::new(false),
        });
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        let symbol = NSImage::imageWithSystemSymbolName_accessibilityDescription(
            &NSString::from_str("doc.on.clipboard"),
            Some(&NSString::from_str("Copy for Agent")),
        );
        let config = NSImageSymbolConfiguration::configurationWithPointSize_weight(10.0, unsafe {
            NSFontWeightRegular
        });
        let symbol = symbol.and_then(|symbol| symbol.imageWithSymbolConfiguration(&config));
        this.setImage(symbol.as_deref());
        this.setImagePosition(NSCellImagePosition::ImageOnly);
        this.setBordered(false);
        this.setContentTintColor(Some(&NSColor::secondaryLabelColor()));
        this.setToolTip(Some(&NSString::from_str("Copy for Agent")));
        unsafe { this.setAction(Some(sel!(copyForAgent:))) };
        this
    }

    /// Follows its row's highlight, so the circle stays visible on the accent color.
    fn set_on_highlight(&self, on: bool) {
        self.ivars().on_highlight.set(on);
        NSView::setNeedsDisplay(self, true);
    }
}

objc2::define_class!(
    /// One Recent Captures row: thumbnail, `title · length · age`, and a button that copies the
    /// capture for an agent. A click elsewhere on the row opens the capture in the review window.
    #[unsafe(super = NSView)]
    #[thread_kind = MainThreadOnly]
    #[ivars = RecentRowIvars]
    struct RecentCaptureRow;

    unsafe impl NSObjectProtocol for RecentCaptureRow {}

    impl RecentCaptureRow {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            // A menu draws no highlight behind a view, so the row draws the standard one.
            let highlighted = self
                .enclosingMenuItem()
                .is_some_and(|item| item.isHighlighted());
            if highlighted {
                let bounds = self.bounds();
                let rect = NSRect::new(
                    NSPoint::new(5.0, 0.0),
                    NSSize::new(bounds.size.width - 10.0, bounds.size.height),
                );
                NSColor::selectedContentBackgroundColor().setFill();
                NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, 5.0, 5.0).fill();
            }
            let ivars = self.ivars();
            if ivars.highlighted.replace(highlighted) != highlighted {
                let (text, secondary) = if highlighted {
                    (
                        NSColor::selectedMenuItemTextColor(),
                        NSColor::selectedMenuItemTextColor(),
                    )
                } else {
                    (NSColor::labelColor(), NSColor::secondaryLabelColor())
                };
                ivars.label.setTextColor(Some(&text));
                ivars.details.setTextColor(Some(&secondary));
                ivars.when.setTextColor(Some(&secondary));
                ivars.copy.setContentTintColor(Some(&secondary));
                ivars.copy.set_on_highlight(highlighted);
            }
        }

        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, _event: &NSEvent) {
            let ivars = self.ivars();
            let _ = ivars.sender.try_send(MenuAction::OpenInbox(ivars.dir.clone()));
            self.close_menu();
        }

        #[unsafe(method(copyForAgent:))]
        fn copy_for_agent(&self, _sender: &AnyObject) {
            if let Err(error) = crate::handoff::copy_bundle_to_clipboard(&self.ivars().dir) {
                crate::capture::scrawl_log(format!("Recent Captures copy failed: {error:#}"));
            }
            self.close_menu();
        }
    }
);

impl RecentCaptureRow {
    fn new(
        mtm: MainThreadMarker,
        capture: &crate::recent::RecentCapture,
        sender: flume::Sender<MenuAction>,
    ) -> Retained<Self> {
        // The title is cut short when it has to be; the length and age beside it never are.
        let label = NSTextField::labelWithString(&NSString::from_str(&capture.title), mtm);
        label.setFont(Some(&NSFont::menuFontOfSize(0.0)));
        label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        label.setTextColor(Some(&NSColor::labelColor()));
        // Line one: the title, then the length at the right. Line two: when it was made, written
        // the way the system writes dates ("Today, 8:55 AM"), like Dropover's recent shelves.
        let details = NSTextField::labelWithString(
            &NSString::from_str(&crate::model::format_clock(capture.duration_ms)),
            mtm,
        );
        details.setFont(Some(&NSFont::menuFontOfSize(0.0)));
        details.setTextColor(Some(&NSColor::secondaryLabelColor()));
        let when = NSTextField::labelWithString(&capture_time(capture.created_at_ms), mtm);
        when.setFont(Some(&NSFont::menuFontOfSize(11.0)));
        when.setTextColor(Some(&NSColor::secondaryLabelColor()));
        when.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        let button_x = RECENT_ROW_W - RECENT_INSET - RECENT_BUTTON;
        let copy = RecentCopyButton::new(
            mtm,
            NSRect::new(
                NSPoint::new(button_x, (RECENT_ROW_H - RECENT_BUTTON) / 2.0),
                NSSize::new(RECENT_BUTTON, RECENT_BUTTON),
            ),
        );

        let this = mtm.alloc::<Self>().set_ivars(RecentRowIvars {
            dir: capture.dir.clone(),
            sender,
            label: label.clone(),
            details: details.clone(),
            when: when.clone(),
            copy: copy.clone(),
            highlighted: std::cell::Cell::new(false),
        });
        let frame = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(RECENT_ROW_W, RECENT_ROW_H),
        );
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        unsafe { copy.setTarget(Some(&this)) };
        // The menu stretches the row to its own width: the title takes the extra space, and the
        // length, age and button stay at the right edge.
        this.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);

        let mut x = RECENT_INSET;
        if let Some(thumbnail) = capture
            .thumbnail_source
            .as_deref()
            .and_then(recent_thumbnail)
        {
            let view = NSImageView::imageViewWithImage(&thumbnail, mtm);
            view.setFrame(NSRect::new(
                NSPoint::new(x, (RECENT_ROW_H - RECENT_THUMB_H) / 2.0),
                NSSize::new(RECENT_THUMB_W, RECENT_THUMB_H),
            ));
            this.addSubview(&view);
        }
        // The text lines up whether or not there's a thumbnail.
        x += RECENT_THUMB_W + 10.0;
        let label_h = 17.0;
        let when_h = 14.0;
        // AppKit's y runs up: the title line sits above the middle, the time line below it.
        let title_y = RECENT_ROW_H / 2.0;
        let when_y = title_y - when_h - 1.0;
        let details_w = details.fittingSize().width.ceil();
        let details_x = button_x - 8.0 - details_w;
        details.setFrame(NSRect::new(
            NSPoint::new(details_x, title_y),
            NSSize::new(details_w, label_h),
        ));
        when.setFrame(NSRect::new(
            NSPoint::new(x, when_y),
            NSSize::new((details_x - 8.0 - x).max(40.0), when_h),
        ));
        label.setFrame(NSRect::new(
            NSPoint::new(x, title_y),
            NSSize::new((details_x - 8.0 - x).max(40.0), label_h),
        ));
        label.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        details.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
        when.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        copy.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
        this.addSubview(&label);
        this.addSubview(&details);
        this.addSubview(&when);
        this.addSubview(&copy);
        this
    }

    /// Closes the status menu, as clicking an ordinary menu item does.
    fn close_menu(&self) {
        if let Some(menu) = self
            .enclosingMenuItem()
            .and_then(|item| unsafe { item.menu() })
        {
            menu.cancelTracking();
        }
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
