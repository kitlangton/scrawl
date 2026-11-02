//! Sparkle 2 updater integration loaded dynamically from `Contents/Frameworks/Sparkle.framework`
//! via `dlopen` + `objc2` so CLI and test binaries never require an adjacent framework.

use std::cell::RefCell;
use std::ffi::{CString, c_void};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};

use anyhow::{Context, Result, bail};
use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject, AnyProtocol, Bool, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, msg_send};
use objc2_app_kit::NSApplication;
use objc2_foundation::{NSError, NSInteger, NSObject};

use crate::capture::engine::runtime::apparent_log;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum UpdateStatus {
    Unavailable = 0,
    Idle = 1,
    Checking = 2,
    UpToDate = 3,
    UpdateAvailable = 4,
}

static UPDATE_STATUS: AtomicU8 = AtomicU8::new(UpdateStatus::Unavailable as u8);

objc2::extern_protocol!(
    #[allow(clippy::missing_safety_doc)]
    unsafe trait SPUUpdaterDelegate: NSObjectProtocol {}
);

objc2::define_class!(
    #[unsafe(super = NSObject)]
    #[name = "ApparentSparkleUpdaterDelegate"]
    #[ivars = &'static AtomicU8]
    struct SparkleDelegate;

    unsafe impl NSObjectProtocol for SparkleDelegate {}

    unsafe impl SPUUpdaterDelegate for SparkleDelegate {
        // Sparkle 2.9.4's SPUInstallerDriver checks this before starting the
        // install/restart, including when relaunch is false. NO aborts the attempt;
        // do not retain an installation block or stop a capture for an update.
        #[unsafe(method(updaterShouldRelaunchApplication:))]
        fn should_relaunch_application(&self, _updater: &AnyObject) -> Bool {
            Bool::new(!crate::capture::desktop::workflow::shutdown::busy())
        }

        #[unsafe(method(updater:didFindValidUpdate:))]
        fn did_find_update(&self, _updater: &AnyObject, _item: &AnyObject) {
            self.ivars()
                .store(UpdateStatus::UpdateAvailable as u8, Ordering::Release);
        }

        #[unsafe(method(updaterDidNotFindUpdate:error:))]
        fn did_not_find_update(&self, _updater: &AnyObject, _error: &NSError) {
            self.ivars()
                .store(UpdateStatus::UpToDate as u8, Ordering::Release);
        }

        #[unsafe(method(updater:didFinishUpdateCycleForUpdateCheck:error:))]
        fn did_finish_update_cycle(
            &self,
            _updater: &AnyObject,
            _update_check: NSInteger,
            _error: Option<&NSError>,
        ) {
            let _ = self.ivars().compare_exchange(
                UpdateStatus::Checking as u8,
                UpdateStatus::Idle as u8,
                Ordering::AcqRel,
                Ordering::Acquire,
            );
        }
    }
);

impl SparkleDelegate {
    fn new(status: &'static AtomicU8) -> Retained<Self> {
        let this = Self::alloc().set_ivars(status);
        unsafe { msg_send![super(this), init] }
    }
}

struct SparkleUpdater {
    controller: Retained<AnyObject>,
    _delegate: Retained<SparkleDelegate>,
    _handle: *mut c_void,
}

thread_local! {
    static UPDATER: RefCell<Option<SparkleUpdater>> = const { RefCell::new(None) };
}

pub fn start() {
    if let Err(error) = start_inner() {
        apparent_log(format!("Sparkle updater failed to start: {error:#}"));
    }
}

fn start_inner() -> Result<()> {
    MainThreadMarker::new().context("Sparkle must start on the main thread")?;
    if UPDATER.with(|u| u.borrow().is_some()) {
        return Ok(());
    }
    let Some(framework_path) = framework_path()? else {
        return Ok(());
    };
    let c_path = CString::new(framework_path.to_string_lossy().as_bytes())
        .context("Invalid Sparkle framework path")?;
    let handle = unsafe { libc::dlopen(c_path.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if handle.is_null() {
        bail!("Could not dlopen {}", framework_path.display());
    }
    AnyProtocol::get(c"SPUUpdaterDelegate").context("SPUUpdaterDelegate protocol missing")?;
    let class = AnyClass::get(c"SPUStandardUpdaterController")
        .context("SPUStandardUpdaterController class missing")?;
    let delegate = SparkleDelegate::new(&UPDATE_STATUS);
    let controller: Retained<AnyObject> = unsafe {
        let allocated: *mut AnyObject = msg_send![class, alloc];
        let controller: *mut AnyObject = msg_send![
            allocated,
            initWithStartingUpdater: Bool::NO,
            updaterDelegate: ProtocolObject::<dyn SPUUpdaterDelegate>::from_ref(&*delegate),
            userDriverDelegate: std::ptr::null_mut::<AnyObject>()
        ];
        Retained::from_raw(controller).context("Sparkle controller init returned null")?
    };
    unsafe {
        let updater: *mut AnyObject = msg_send![&*controller, updater];
        let updater = updater
            .as_ref()
            .context("Sparkle controller returned no updater")?;
        let mut error: Option<Retained<NSError>> = None;
        let started: Bool = msg_send![updater, startUpdater: &mut error];
        if !started.as_bool() {
            bail!("Sparkle startUpdater failed: {error:?}");
        }
        UPDATE_STATUS.store(UpdateStatus::Idle as u8, Ordering::Release);
        let auto_checks: Bool = msg_send![updater, automaticallyChecksForUpdates];
        if auto_checks.as_bool() {
            let _: () = msg_send![updater, checkForUpdatesInBackground];
        }
    }
    UPDATER.with(|u| {
        *u.borrow_mut() = Some(SparkleUpdater {
            controller,
            _delegate: delegate,
            _handle: handle,
        });
    });
    Ok(())
}

/// Whether this copy can check for updates (a development build has no Sparkle).
pub fn available() -> bool {
    UPDATE_STATUS.load(Ordering::Acquire) != UpdateStatus::Unavailable as u8
}

pub fn check_for_updates() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    UPDATER.with(|u| {
        let borrow = u.borrow();
        let Some(updater) = borrow.as_ref() else {
            apparent_log("Sparkle updater is not available");
            return;
        };
        unsafe {
            NSApplication::sharedApplication(mtm).activate();
            let _: () =
                msg_send![&*updater.controller, checkForUpdates: std::ptr::null_mut::<AnyObject>()];
            let user_driver: *mut AnyObject = msg_send![&*updater.controller, userDriver];
            if !user_driver.is_null() {
                let _: () = msg_send![user_driver, showUpdateInFocus];
            }
        }
    });
}

fn framework_path() -> Result<Option<PathBuf>> {
    let raw_exe = std::env::current_exe()?;
    let executable = std::fs::canonicalize(&raw_exe).unwrap_or(raw_exe);
    let Some(macos_dir) = executable.parent() else {
        return Ok(None);
    };
    let path = macos_dir.join("../Frameworks/Sparkle.framework/Versions/B/Sparkle");
    Ok(path.exists().then_some(path))
}
