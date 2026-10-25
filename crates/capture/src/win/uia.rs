//! What clicks and marks land on, from UI Automation: Windows' counterpart of the macOS
//! Accessibility lookups in `crate::ax` (the `agentContext` experiment). Desktop apps may read
//! UI Automation without a grant. Only names, control types and automation ids are read, never
//! a control's value, so nothing typed into a field (passwords included) leaves it.

use std::time::{Duration, Instant};

use windows::Win32::Foundation::POINT;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
};
use windows::Win32::UI::Accessibility::{
    CUIAutomation, IUIAutomation, IUIAutomationElement, UIA_ButtonControlTypeId,
    UIA_CONTROLTYPE_ID, UIA_CheckBoxControlTypeId, UIA_CustomControlTypeId,
    UIA_DataItemControlTypeId, UIA_GroupControlTypeId, UIA_HyperlinkControlTypeId,
    UIA_ImageControlTypeId, UIA_ListItemControlTypeId, UIA_MenuItemControlTypeId,
    UIA_RadioButtonControlTypeId, UIA_SplitButtonControlTypeId, UIA_TabItemControlTypeId,
    UIA_TextControlTypeId, UIA_TreeItemControlTypeId,
};

use crate::ax::{Hit, clip};

thread_local! {
    /// One automation object per thread (COM, multithreaded apartment); `None` if it failed.
    static AUTOMATION: Option<IUIAutomation> = unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()
    };
}

/// Types that are usually a piece of a control (its text or icon), so the control names it.
fn is_part(kind: UIA_CONTROLTYPE_ID) -> bool {
    [
        UIA_TextControlTypeId,
        UIA_ImageControlTypeId,
        UIA_GroupControlTypeId,
        UIA_CustomControlTypeId,
    ]
    .contains(&kind)
}

fn is_control(kind: UIA_CONTROLTYPE_ID) -> bool {
    [
        UIA_ButtonControlTypeId,
        UIA_SplitButtonControlTypeId,
        UIA_HyperlinkControlTypeId,
        UIA_MenuItemControlTypeId,
        UIA_CheckBoxControlTypeId,
        UIA_RadioButtonControlTypeId,
        UIA_TabItemControlTypeId,
        UIA_ListItemControlTypeId,
        UIA_TreeItemControlTypeId,
        UIA_DataItemControlTypeId,
    ]
    .contains(&kind)
}

fn text(value: windows::core::Result<windows::core::BSTR>) -> Option<String> {
    value
        .ok()
        .map(|s| s.to_string())
        .filter(|s| !s.trim().is_empty())
}

/// The element at a global point in physical pixels (Scrawl is per-monitor DPI aware, as UI
/// Automation's coordinates are). Scrawl's own windows let clicks through or are skipped. Stops
/// reading attributes once `budget` is spent.
pub fn hit_test(x: f64, y: f64, budget: Duration) -> Option<Hit> {
    let started = Instant::now();
    let over = || started.elapsed() > budget;
    AUTOMATION.with(|automation| {
        let automation = automation.as_ref()?;
        let point = POINT {
            x: x.round() as i32,
            y: y.round() as i32,
        };
        let mut element: IUIAutomationElement =
            unsafe { automation.ElementFromPoint(point) }.ok()?;
        if unsafe { element.CurrentProcessId() }.ok()? == std::process::id() as i32 {
            return None;
        }
        let kind = unsafe { element.CurrentControlType() }.ok()?;
        // A button's text or icon names the button.
        if is_part(kind)
            && let Ok(walker) = unsafe { automation.ControlViewWalker() }
        {
            let mut current = element.clone();
            for _ in 0..2 {
                if over() {
                    break;
                }
                let Ok(parent) = (unsafe { walker.GetParentElement(&current) }) else {
                    break;
                };
                let parent_kind = unsafe { parent.CurrentControlType() }.unwrap_or_default();
                if is_control(parent_kind) {
                    element = parent;
                    break;
                }
                current = parent;
            }
        }
        let role = text(unsafe { element.CurrentLocalizedControlType() })
            .and_then(|r| clip(&r))
            .unwrap_or_else(|| "element".into());
        let mut hit = Hit {
            role,
            ..Hit::default()
        };
        // A field's name is its label, not what was typed into it.
        if !over() {
            hit.label = text(unsafe { element.CurrentName() })
                .or_else(|| text(unsafe { element.CurrentHelpText() }))
                .and_then(|s| clip(&s));
        }
        if !over() {
            hit.identifier = text(unsafe { element.CurrentAutomationId() })
                // Win32 controls number their ids; only a name helps an agent.
                .filter(|id| !id.chars().all(|c| c.is_ascii_digit()))
                .and_then(|s| clip(&s));
        }
        Some(hit)
    })
}

#[cfg(test)]
mod tests {
    /// A look at whatever is under the pointer, by hand (it reads the real desktop):
    /// `cargo test win::uia -- --ignored --nocapture`.
    #[test]
    #[ignore = "reads the desktop under the pointer"]
    fn names_the_element_under_the_pointer() {
        let (x, y) = crate::win::cursor_position().expect("a pointer");
        let hit = super::hit_test(x, y, std::time::Duration::from_millis(250));
        println!("{x},{y}: {hit:?}");
        assert!(hit.is_some_and(|h| !h.role.is_empty()));
    }
}
