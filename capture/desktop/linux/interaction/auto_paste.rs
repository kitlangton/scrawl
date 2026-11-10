//! Explicit user-directed paste mechanics; automatic completion policy lives in Record.
use super::hyprland::FocusedWindow;
use anyhow::{ensure, Context, Result};
use std::path::Path;
use std::time::Duration;

/// Window identity is necessary, but not sufficient: this cannot check the editor.
pub fn window_matches(original: &FocusedWindow, current: &FocusedWindow) -> bool {
    original == current
}

/// This is a user-directed trial, not an editor or attachment observer. Keep the
/// completed paper visible: dispatching paste never proves that the app accepted it.
pub fn preflight(original: &FocusedWindow) -> Result<()> {
    super::hyprland::targeted_paste_supported(original)?;
    super::hyprland::check_window(original)?;
    ensure!(
        window_matches(original, &super::hyprland::focused_window()?)
            && original.pid != std::process::id(),
        "Choose a chat composer and keep it focused until paste finishes"
    );
    Ok(())
}

pub trait UserPasteOps {
    fn check_focus(&mut self) -> Result<()>;
    fn offer_image(&mut self) -> Result<()>;
    fn offer_text(&mut self) -> Result<()>;
    fn still_ours(&mut self, value: &'static str) -> Result<bool>;
    fn paste(&mut self) -> Result<()>;
    /// Leave the text handoff on the clipboard for an ordinary manual Ctrl+V.
    /// Never overwrite a selection the user changed during the attempt.
    fn fallback(&mut self, offered: &'static str) -> Result<()>;
}

pub fn user_paste_sequence(ops: &mut impl UserPasteOps) -> Result<()> {
    let mut offered = "";
    let result = (|| {
        ops.check_focus()?;
        ops.offer_image()?;
        offered = "image";
        ops.check_focus()?;
        ensure!(
            ops.still_ours(offered)?,
            "Clipboard changed before image paste"
        );
        ops.paste()?;
        std::thread::sleep(Duration::from_millis(160));
        ops.check_focus()?;
        ensure!(
            ops.still_ours(offered)?,
            "Clipboard changed before text offer"
        );
        ops.offer_text()?;
        offered = "text";
        ops.check_focus()?;
        ensure!(
            ops.still_ours(offered)?,
            "Clipboard changed before text paste"
        );
        ops.paste()?;
        Ok(())
    })();
    let fallback = ops.fallback(offered);
    result.and(fallback)
}

pub fn paste_bundle(bundle_dir: &Path) -> Result<()> {
    let (directory, bundle) = crate::shared::handoff::load_bundle(bundle_dir)?;
    let image = std::fs::read(directory.join(&bundle.contact_sheet))
        .context("Could not read contact sheet")?;
    let summary = crate::shared::handoff::clipboard_summary(&bundle, &directory);
    let focused = super::hyprland::focused_window()?;
    preflight(&focused)?;
    user_paste_sequence(&mut LivePaste {
        original: &focused,
        image: &image,
        summary: &summary,
    })
}

struct LivePaste<'a> {
    original: &'a FocusedWindow,
    image: &'a [u8],
    summary: &'a str,
}

impl UserPasteOps for LivePaste<'_> {
    fn check_focus(&mut self) -> Result<()> {
        ensure!(
            window_matches(self.original, &super::hyprland::focused_window()?),
            "Paste target lost focus"
        );
        Ok(())
    }
    fn offer_image(&mut self) -> Result<()> {
        super::clipboard::set_png(self.image)
    }
    fn offer_text(&mut self) -> Result<()> {
        super::clipboard::set_text(self.summary)
    }
    fn still_ours(&mut self, value: &'static str) -> Result<bool> {
        match value {
            "image" => super::clipboard::still_ours("image/png", self.image),
            "text" => {
                super::clipboard::still_ours("text/plain;charset=utf-8", self.summary.as_bytes())
            }
            _ => Ok(false),
        }
    }
    fn paste(&mut self) -> Result<()> {
        std::thread::sleep(Duration::from_millis(20));
        super::hyprland::paste_into_focused(self.original, false)
    }
    fn fallback(&mut self, offered: &'static str) -> Result<()> {
        if offered == "image" {
            ensure!(
                self.still_ours("image")?,
                "Clipboard changed; not replacing newer content"
            );
            super::clipboard::set_text(self.summary)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{bail, Result};

    #[test]
    fn user_directed_paste_leaves_manual_text_fallback_without_stealing_focus() {
        #[derive(Default)]
        struct Draft {
            calls: Vec<&'static str>,
            clipboard: &'static str,
            focused: bool,
            fail_second: bool,
            fail_text_offer: bool,
            lose_focus_after_image: bool,
            user_changes_clipboard: bool,
        }
        impl UserPasteOps for Draft {
            fn check_focus(&mut self) -> Result<()> {
                if !self.focused {
                    bail!("focus lost")
                }
                Ok(())
            }
            fn offer_image(&mut self) -> Result<()> {
                self.calls.push("image");
                self.clipboard = "image";
                Ok(())
            }
            fn offer_text(&mut self) -> Result<()> {
                if self.fail_text_offer {
                    bail!("text offer refused")
                }
                self.calls.push("text");
                self.clipboard = "text";
                Ok(())
            }
            fn still_ours(&mut self, value: &'static str) -> Result<bool> {
                Ok(self.clipboard == value)
            }
            fn paste(&mut self) -> Result<()> {
                self.calls.push("paste");
                if self.fail_second
                    && self.calls.iter().filter(|&&call| call == "paste").count() == 2
                {
                    bail!("second paste refused")
                }
                if self.calls.iter().filter(|&&call| call == "paste").count() == 1 {
                    if self.lose_focus_after_image {
                        self.focused = false;
                    }
                    if self.user_changes_clipboard {
                        self.clipboard = "user content";
                    }
                }
                Ok(())
            }
            fn fallback(&mut self, offered: &'static str) -> Result<()> {
                self.calls.push("fallback");
                if self.clipboard == offered && offered == "image" {
                    self.clipboard = "text";
                }
                Ok(())
            }
        }
        let mut draft = Draft {
            clipboard: "handoff",
            focused: true,
            ..Default::default()
        };
        user_paste_sequence(&mut draft).unwrap();
        assert_eq!(draft.calls, ["image", "paste", "text", "paste", "fallback"]);
        assert_eq!(
            draft.clipboard, "text",
            "manual Ctrl+V must still paste the handoff"
        );
        let mut failed = Draft {
            clipboard: "handoff",
            focused: true,
            fail_second: true,
            ..Default::default()
        };
        assert!(user_paste_sequence(&mut failed).is_err());
        assert_eq!(
            failed.clipboard, "text",
            "failed automatic paste must leave a Ctrl+V fallback"
        );
        let mut refused = Draft {
            clipboard: "handoff",
            focused: true,
            fail_text_offer: true,
            ..Default::default()
        };
        assert!(user_paste_sequence(&mut refused).is_err());
        assert_eq!(
            refused.clipboard, "text",
            "a failed text offer must not leave the image on Ctrl+V"
        );
        let mut changed_focus = Draft {
            clipboard: "handoff",
            focused: true,
            lose_focus_after_image: true,
            ..Default::default()
        };
        assert!(user_paste_sequence(&mut changed_focus).is_err());
        assert_eq!(
            changed_focus
                .calls
                .iter()
                .filter(|&&call| call == "paste")
                .count(),
            1
        );
        assert_eq!(changed_focus.clipboard, "text");
        let mut changed_clipboard = Draft {
            clipboard: "handoff",
            focused: true,
            user_changes_clipboard: true,
            ..Default::default()
        };
        assert!(user_paste_sequence(&mut changed_clipboard).is_err());
        assert_eq!(
            changed_clipboard.clipboard, "user content",
            "do not overwrite a newer user copy"
        );
    }

    #[test]
    fn only_the_exact_current_window_can_receive_both_pastes() {
        let original = FocusedWindow {
            address: "0xabc".into(),
            pid: 42,
            class: "com.t3tools.T3Code".into(),
        };
        let mut other = original.clone();
        assert!(window_matches(&original, &original));
        other.address = "0xdef".into();
        assert!(!window_matches(&original, &other));
        other = original.clone();
        other.pid = 43;
        assert!(!window_matches(&original, &other));
    }

}
