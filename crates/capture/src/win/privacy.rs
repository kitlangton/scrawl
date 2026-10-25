//! Windows privacy switches. Desktop apps need none for screen capture, input hooks or sending
//! keystrokes; the microphone (and camera) can be turned off for all desktop apps in
//! Settings › Privacy & security, which the onboarding checklist shows.

use std::collections::BTreeMap;

use windows::Win32::System::Registry::{
    HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_SZ, RegGetValueW,
};
use windows::core::PCWSTR;

use super::wide;
use crate::permissions::{Permission, Status};

const CONSENT: &str =
    r"Software\Microsoft\Windows\CurrentVersion\CapabilityAccessManager\ConsentStore";

fn read_string(root: HKEY, key: &str, value: &str) -> Option<String> {
    let key = wide(key);
    let value = wide(value);
    let mut buffer = [0_u16; 64];
    let mut size = (buffer.len() * 2) as u32;
    let status = unsafe {
        RegGetValueW(
            root,
            PCWSTR(key.as_ptr()),
            PCWSTR(value.as_ptr()),
            RRF_RT_REG_SZ,
            None,
            Some(buffer.as_mut_ptr().cast()),
            Some(&mut size),
        )
    };
    if status.is_err() {
        return None;
    }
    let chars = (size as usize / 2).saturating_sub(1).min(buffer.len());
    Some(String::from_utf16_lossy(&buffer[..chars]))
}

/// A capability's switch: denied when the device-wide, per-user or desktop-app switch is off.
fn capability(name: &str) -> Status {
    let switches = [
        (HKEY_LOCAL_MACHINE, format!(r"{CONSENT}\{name}")),
        (HKEY_CURRENT_USER, format!(r"{CONSENT}\{name}")),
        (HKEY_CURRENT_USER, format!(r"{CONSENT}\{name}\NonPackaged")),
    ];
    let denied = switches.iter().any(|(root, key)| {
        read_string(*root, key, "Value").is_some_and(|value| value.eq_ignore_ascii_case("Deny"))
    });
    if denied {
        Status::Denied
    } else {
        Status::Granted
    }
}

pub fn permissions() -> BTreeMap<Permission, Status> {
    BTreeMap::from([
        (Permission::ScreenRecording, Status::Granted),
        (Permission::Microphone, capability("microphone")),
        (Permission::InputMonitoring, Status::Granted),
        (Permission::Accessibility, Status::Granted),
        (Permission::Camera, capability("webcam")),
    ])
}

/// The Settings page that turns a permission on.
pub fn settings_uri(permission: Permission) -> &'static str {
    match permission {
        Permission::Microphone => "ms-settings:privacy-microphone",
        Permission::Camera => "ms-settings:privacy-webcam",
        _ => "ms-settings:privacy",
    }
}

pub fn open_settings(permission: Permission) {
    let _ = super::shell::open(std::path::Path::new(settings_uri(permission)));
}
