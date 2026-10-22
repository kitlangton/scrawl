//! What Linux lets Scrawl use. Nothing asks for the microphone outside a sandbox. On X11 the
//! screen, the pointer and the keys are open to every app. On Wayland the screen goes through
//! the ScreenCast portal (which asks once, when recording starts), and clicks and ⇧ need the
//! input devices (`/dev/input/event*`), which only members of the `input` group may read: the
//! Accessibility row stands for that (`crate::linux::input`).

use std::collections::BTreeMap;

use crate::onboarding::{Permission, Status};

pub const INPUT_COMMAND: &str = "sudo usermod -aG input \"$USER\"";

pub fn copy_input_command() {
    if let Err(error) = super::clipboard::set_text(INPUT_COMMAND) {
        crate::capture::scrawl_log(format!("Copy input command: {error:#}"));
    }
}

pub fn permissions() -> BTreeMap<Permission, Status> {
    let input = if super::wayland_session() && !super::input::devices_readable() {
        Status::Denied
    } else {
        Status::Granted
    };
    BTreeMap::from([
        (Permission::ScreenRecording, Status::Granted),
        (Permission::Microphone, Status::Granted),
        (Permission::InputMonitoring, Status::Granted),
        (Permission::Accessibility, input),
        (Permission::Camera, Status::Granted),
    ])
}

/// Explains (in the browser) how to let Scrawl read the input devices: the README's section.
pub fn open_settings(permission: Permission) {
    if permission == Permission::Accessibility {
        let _ = super::shell::open(std::path::Path::new(
            "https://github.com/kitlangton/scrawl/blob/main/docs/linux.md#clicks-and-ink-on-wayland",
        ));
    }
}
