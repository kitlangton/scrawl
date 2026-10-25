//! What the SwiftUI windows' `Permissions.request` / `openSettings` / `closeGuide` do, for the
//! Rune windows that run in Scrawl's own process (`"ui": "rune"`). The drag panel for
//! Screen Recording and Accessibility is the SwiftUI `scrawl-helper permission-panel` unless
//! the `runePermissionPanel` experiment draws it with Rune (`rune_permission_panel`).

use std::collections::HashMap;
use std::ffi::{CStr, c_char, c_int, c_void};
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, OnceLock};

use objc2::msg_send;
use objc2::runtime::{AnyClass, AnyObject, Bool};
use objc2_foundation::NSString;

unsafe extern "C" {
    fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrustedWithOptions(options: *const c_void) -> bool;
}

const RTLD_LAZY: c_int = 1;

fn load(framework: &CStr) {
    unsafe {
        dlopen(framework.as_ptr(), RTLD_LAZY);
    }
}

/// The system prompt for one permission, as the row's Allow button asks for it.
pub(crate) fn request(permission: &str) {
    match permission {
        "microphone" => request_capture_device("soun"),
        "camera" => request_capture_device("vide"),
        "speech-recognition" => request_speech(),
        "accessibility" => prompt_accessibility(),
        "screen-recording" => crate::capture::request_screen_capture(),
        _ => {}
    }
}

/// `AVCaptureDevice.requestAccess(for:)`; the media types are AVMediaTypeAudio / Video's values.
fn request_capture_device(media_type: &str) {
    load(c"/System/Library/Frameworks/AVFoundation.framework/AVFoundation");
    let Some(class) = AnyClass::get(c"AVCaptureDevice") else {
        return;
    };
    let media_type = NSString::from_str(media_type);
    let block = block2::RcBlock::new(|_granted: Bool| {});
    unsafe {
        let _: () =
            msg_send![class, requestAccessForMediaType: &*media_type, completionHandler: &*block];
    }
}

fn request_speech() {
    load(c"/System/Library/Frameworks/Speech.framework/Speech");
    let Some(class) = AnyClass::get(c"SFSpeechRecognizer") else {
        return;
    };
    let block = block2::RcBlock::new(|_status: isize| {});
    unsafe {
        let _: () = msg_send![class, requestAuthorization: &*block];
    }
}

fn prompt_accessibility() {
    use core_foundation::base::TCFType;
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFDictionary;
    use core_foundation::string::CFString;
    let options = CFDictionary::from_CFType_pairs(&[(
        CFString::new("AXTrustedCheckOptionPrompt"),
        CFBoolean::true_value(),
    )]);
    unsafe {
        AXIsProcessTrustedWithOptions(options.as_concrete_TypeRef().cast());
    }
}

/// The drag panels docked beside System Settings, told `granted` when their row turns on.
fn panels() -> &'static Mutex<HashMap<String, Child>> {
    static PANELS: OnceLock<Mutex<HashMap<String, Child>>> = OnceLock::new();
    PANELS.get_or_init(Default::default)
}

fn tell(child: &mut Child, command: &str) {
    if matches!(child.try_wait(), Ok(None))
        && let Some(stdin) = child.stdin.as_mut()
    {
        let _ = writeln!(stdin, "{command}");
    }
}

/// The System Settings pane; for the lists an app is dragged into (Screen Recording,
/// Accessibility) the drag panel, flying from `from` (the button, global top-left points).
pub(crate) fn open_settings(permission: &str, from: Option<[f32; 4]>) {
    if matches!(permission, "screen-recording" | "accessibility") {
        if super::rune_permission_panel::open(permission, from) {
            return;
        }
        let Ok(helper) = crate::helper::ui_path() else {
            return;
        };
        let mut command = Command::new(helper);
        command.args(["permission-panel", "--permission", permission]);
        if let Some([x, y, w, h]) = from {
            command.args([
                "--from",
                &format!("{},{},{},{}", x as i32, y as i32, w as i32, h as i32),
            ]);
        }
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut panels = panels().lock().unwrap_or_else(|e| e.into_inner());
        reap(&mut panels);
        if let Some(old) = panels.remove(permission) {
            retire(old, "cancel");
        }
        match command.spawn() {
            Ok(child) => {
                panels.insert(permission.to_owned(), child);
            }
            Err(error) => crate::log::scrawl_log(format!("permission-panel: {error}")),
        }
        return;
    }
    let anchor = match permission {
        "camera" => "Privacy_Camera",
        "speech-recognition" => "Privacy_SpeechRecognition",
        _ => "Privacy_Microphone",
    };
    open_url(&format!(
        "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?{anchor}"
    ));
}

/// Tells this permission's drag panel that the row turned on, so it says so and goes.
pub(crate) fn close_guide(permission: &str) {
    super::rune_permission_panel::granted(permission);
    let mut panels = panels().lock().unwrap_or_else(|e| e.into_inner());
    reap(&mut panels);
    if let Some(child) = panels.remove(permission) {
        retire(child, "granted");
    }
}

/// Forgets the panels that have exited by themselves (closed by the user, or done).
fn reap(panels: &mut HashMap<String, Child>) {
    panels.retain(|_, child| matches!(child.try_wait(), Ok(None)));
}

/// Tells a panel its last command and closes its stdin; a thread waits for it to finish its
/// animation and exit, so it doesn't linger as a zombie.
fn retire(mut child: Child, command: &str) {
    tell(&mut child, command);
    drop(child.stdin.take());
    let _ = std::thread::Builder::new()
        .name("permission-panel-wait".into())
        .spawn(move || {
            let _ = child.wait();
        });
}

pub(crate) fn open_url(url: &str) {
    let (Some(workspace), Some(url_class)) =
        (AnyClass::get(c"NSWorkspace"), AnyClass::get(c"NSURL"))
    else {
        return;
    };
    let url = NSString::from_str(url);
    unsafe {
        let url: *mut AnyObject = msg_send![url_class, URLWithString: &*url];
        if url.is_null() {
            return;
        }
        let shared: *mut AnyObject = msg_send![workspace, sharedWorkspace];
        let _: Bool = msg_send![shared, openURL: url];
    }
}
