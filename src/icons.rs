//! Line icons embedded in the binary and served to GPUI's `svg()` element.

use std::borrow::Cow;

use gpui::{AssetSource, SharedString};

pub const AREA: &str = "icons/area.svg";
pub const WINDOW: &str = "icons/window.svg";
pub const SCREEN: &str = "icons/screen.svg";
pub const IPHONE: &str = "icons/iphone.svg";
pub const MIC: &str = "icons/mic.svg";
pub const CHECK: &str = "icons/check.svg";
pub const CHECK_CIRCLE: &str = "icons/check-circle.svg";
pub const REOPEN: &str = "icons/reopen.svg";
pub const CLICK: &str = "icons/click.svg";
pub const LOCK: &str = "icons/lock.svg";
pub const PERSON: &str = "icons/person.svg";
pub const SPINNER: &str = "icons/spinner.svg";
pub const GEAR: &str = "icons/gear.svg";
pub const RECORD: &str = "icons/record.svg";
pub const DRIVE: &str = "icons/drive.svg";
pub const HAND: &str = "icons/hand.svg";
pub const INFO: &str = "icons/info.svg";
pub const CHEVRONS: &str = "icons/chevrons.svg";
pub const RESET: &str = "icons/reset.svg";
pub const ARROW_UP_RIGHT: &str = "icons/arrow-up-right.svg";
pub const TERMINAL: &str = "icons/terminal.svg";
pub const X: &str = "icons/x.svg";

const ICONS: [(&str, &[u8]); 22] = [
    (SPINNER, include_bytes!("../assets/icons/spinner.svg")),
    (AREA, include_bytes!("../assets/icons/area.svg")),
    (WINDOW, include_bytes!("../assets/icons/window.svg")),
    (SCREEN, include_bytes!("../assets/icons/screen.svg")),
    (IPHONE, include_bytes!("../assets/icons/iphone.svg")),
    (MIC, include_bytes!("../assets/icons/mic.svg")),
    (CHECK, include_bytes!("../assets/icons/check.svg")),
    (
        CHECK_CIRCLE,
        include_bytes!("../assets/icons/check-circle.svg"),
    ),
    (REOPEN, include_bytes!("../assets/icons/reopen.svg")),
    (CLICK, include_bytes!("../assets/icons/click.svg")),
    (LOCK, include_bytes!("../assets/icons/lock.svg")),
    (PERSON, include_bytes!("../assets/icons/person.svg")),
    (GEAR, include_bytes!("../assets/icons/gear.svg")),
    (RECORD, include_bytes!("../assets/icons/record.svg")),
    (DRIVE, include_bytes!("../assets/icons/drive.svg")),
    (HAND, include_bytes!("../assets/icons/hand.svg")),
    (INFO, include_bytes!("../assets/icons/info.svg")),
    (CHEVRONS, include_bytes!("../assets/icons/chevrons.svg")),
    (RESET, include_bytes!("../assets/icons/reset.svg")),
    (TERMINAL, include_bytes!("../assets/icons/terminal.svg")),
    (X, include_bytes!("../assets/icons/x.svg")),
    (
        ARROW_UP_RIGHT,
        include_bytes!("../assets/icons/arrow-up-right.svg"),
    ),
];

pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> anyhow::Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(name, _)| *name == path)
            .map(|(_, bytes)| Cow::Borrowed(*bytes)))
    }

    fn list(&self, path: &str) -> anyhow::Result<Vec<SharedString>> {
        Ok(ICONS
            .iter()
            .filter(|(name, _)| name.starts_with(path))
            .map(|(name, _)| SharedString::from(*name))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_is_a_stroked_svg() {
        for (name, bytes) in ICONS {
            let svg = std::str::from_utf8(bytes).unwrap();
            assert!(svg.starts_with("<svg"), "{name}");
            assert!(svg.contains("stroke-width=\"2.5\""), "{name}");
            assert!(Assets.load(name).unwrap().is_some());
        }
        assert!(Assets.load("icons/missing.svg").unwrap().is_none());
        assert_eq!(Assets.list("icons/").unwrap().len(), ICONS.len());
    }
}
