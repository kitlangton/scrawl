//! Rust-authoritative presentation contract for the shared recording pill.
//! No paths, capture contents, credentials or renderer-selected capture IDs cross it.
use serde::{Deserialize, Serialize};
mod coordinator;
mod session;
use crate::capture::desktop::{
    SelectionMode,
    native::pill::{PillAction, PillSnapshot, model::Pres},
};
pub use session::Session;

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub revision: u64,
    pub phase: &'static str,
    pub clock: String,
    pub label: String,
    pub warning: Option<String>,
    pub notice: Option<String>,
    pub pocket: Option<Pocket>,
    pub reduced_motion: bool,
    pub draggable: bool,
    pub input_level: Option<f32>,
    pub actions: Vec<Control>,
}

/// Display-only notification. Actions reference the same authoritative control set.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Pocket {
    pub tone: &'static str,
    pub title: String,
    pub description: String,
    pub action_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Control {
    pub id: String,
    pub label: String,
    pub enabled: bool,
    pub selected: bool,
    pub primary: bool,
}

// Match JavaScript protocol bounds, including astral Unicode characters.
fn bounded_text(value: &str, units: usize) -> String {
    String::from_utf16_lossy(&value.encode_utf16().take(units).collect::<Vec<_>>())
}

fn pocket_description(primary: &str, warning: Option<&str>) -> String {
    let text = warning.map_or_else(|| primary.to_owned(), |warning| format!("{primary}\n{warning}"));
    bounded_text(&text, 300)
}

fn upload_size(bytes: u64, round_up: bool) -> String {
    let unit = if bytes >= 1_000_000_000 { 100_000_000 } else { 1_000_000 };
    let value = bytes / unit + u64::from(round_up && bytes % unit != 0);
    if unit == 100_000_000 { format!("{}.{:01} GB", value / 10, value % 10) }
    else { format!("{value} MB") }
}

impl View {
    pub fn new(snapshot: &PillSnapshot, revision: u64, draggable: bool) -> Self {
        let mut view = Self {
            revision,
            phase: "hidden",
            clock: snapshot.clock.clone(),
            label: String::new(),
            notice: None,
            pocket: None,
            warning: snapshot
                .mic_warning
                .map(|warning| warning.announcement().to_owned()),
            reduced_motion: snapshot.reduced_motion,
            draggable: false,
            input_level: None,
            actions: vec![],
        };
        match &snapshot.pres {
            Pres::Hidden => {}
            Pres::Selecting { mode, has_iphone } => {
                view.phase = "selecting";
                view.label = "Choose what to record".into();
                view.modes(*has_iphone, Some(*mode), false);
                // Area starts through its picker; Screen and iPhone offer Record.
                if matches!(mode, SelectionMode::Screen | SelectionMode::Iphone) {
                    view.control("record", "Record", true, false, true);
                }
                view.control("cancel", "Cancel", true, false, false);
            }
            Pres::Recording { chips, has_iphone }
            | Pres::RecordingWarning { chips, has_iphone } => {
                view.phase = "recording";
                view.label = "Recording".into();
                view.draggable = draggable;
                if matches!(snapshot.pres, Pres::RecordingWarning { .. }) {
                    view.warning = Some(view.warning.map_or_else(
                        || "No words recognized yet".into(),
                        |warning| format!("{warning} · No words recognized yet"),
                    ));
                }
                if *chips {
                    view.modes(*has_iphone, None, true);
                } else {
                    view.control("whiteboard", "Whiteboard", true, false, false);
                    view.control("finish", "Finish", true, false, view.warning.is_none());
                }
                view.control("cancel", "Cancel", true, false, false);
            }
            Pres::Processing => {
                view.phase = "processing";
                view.label = "Processing recording…".into();
            }
            Pres::Completed {
                label,
                warning,
                share,
                ..
            } => {
                view.phase = "completed";
                view.label = label.clone();
                if *warning {
                    view.warning = Some(label.clone());
                    view.label = "Saved".into();
                }
                view.control("paste", "Paste", true, false, !warning);
                if let Some(enabled) = share {
                    view.control("copy-link", "Copy link", *enabled, false, false);
                }
                view.control("inspect", "Inspect", true, false, false);
                view.control("dismiss", "Dismiss", true, false, false);
            }
        }
        view.project_notifications();
        view
    }

    fn project_notifications(&mut self) {
        self.pocket = self.warning.as_ref().map(|warning| Pocket {
            tone: "warning",
            title: "Recording notice".into(),
            description: bounded_text(warning, 300),
            action_ids: vec![],
        }).or_else(|| self.notice.as_ref().map(|_| Pocket {
            tone: "info",
            title: "Control changed".into(),
            // Session retains notice after projection; render that authoritative
            // field inside this pocket rather than embedding a stale copy.
            description: String::new(),
            action_ids: vec![],
        }));
    }

    /// Keep completion feedback in the pocket instead of growing the control row.
    pub fn with_completion_notice(mut self, note: Option<&str>) -> Self {
        if self.phase == "completed" {
            if let Some(note) = note.filter(|note| !note.is_empty()) {
                self.label = "Saved".into();
                if self.warning.is_none() {
                    self.pocket = Some(Pocket {
                        tone: "info",
                        title: "Sharing".into(),
                        description: bounded_text(note, 300),
                        action_ids: vec![],
                    });
                }
            }
        }
        self
    }

    /// Numeric upload facts only: private bundle paths never cross the bridge.
    /// The caller must supply facts for the currently selected completed capture.
    pub fn with_size_failure(mut self, size: u64, limit: u64) -> Self {
        if self.phase == "completed" {
            self.pocket = Some(Pocket {
                tone: "failure",
                title: "Too large to upload".into(),
                description: pocket_description(&format!("{} · limit {}. Your recording is saved. Paste it locally.", upload_size(size, true), upload_size(limit, false)), self.warning.as_deref()),
                action_ids: self.actions.iter().filter(|a| a.id == "paste" && a.enabled).map(|a| a.id.clone()).collect(),
            });
        }
        self
    }

    /// The new bridge never falls back to an implicit latest capture.
    pub fn with_handoff(mut self, completed: bool, review: bool) -> Self {
        for action in &mut self.actions {
            let available = match action.id.as_str() {
                "paste" | "copy-link" => completed,
                "inspect" => review,
                _ => true,
            };
            action.enabled &= available;
        }
        self
    }

    fn control(&mut self, id: &str, label: &str, enabled: bool, selected: bool, primary: bool) {
        self.actions.push(Control {
            id: id.into(),
            label: label.into(),
            enabled,
            selected,
            primary,
        });
    }

    fn modes(&mut self, iphone: bool, selected: Option<SelectionMode>, switching: bool) {
        for mode in SelectionMode::available(iphone) {
            let label = match mode {
                SelectionMode::Area => "Area",
                SelectionMode::Screen => "Screen",
                SelectionMode::Iphone => "iPhone",
                SelectionMode::Window => continue,
            };
            self.control(
                &format!(
                    "{}{}",
                    if switching { "switch-" } else { "select-" },
                    mode.key()
                ),
                label,
                true,
                selected == Some(*mode),
                false,
            );
        }
    }

    /// Known rejection advances the matching revision so the renderer can recover.
    /// A stale command never consumes a newer capture's revision.
    pub fn consume(&mut self, command: Command, timely: bool) -> Option<PillAction> {
        if command.revision != self.revision {
            return None;
        }
        let action = timely.then(|| self.admit(command)).flatten();
        self.revision += 1;
        self.notice = action
            .is_none()
            .then(|| "Control request expired or changed. Try again.".into());
        self.project_notifications();
        action
    }

    /// Admission is checked again against current coordinator state before dispatch.
    #[cfg(target_os = "linux")]
    pub fn drag_point(&self, command: &Command, timely: bool) -> Option<[f32; 2]> {
        if !timely || command.action != "drag" || command.revision != self.revision
            || self.phase != "recording" || !self.draggable { return None; }
        command.target.filter(|point| point.iter().all(|v| v.is_finite()))
    }

    /// Admission is checked again against current coordinator state before dispatch.
    pub fn admit(&self, command: Command) -> Option<PillAction> {
        if command.revision != self.revision
            || !self
                .actions
                .iter()
                .any(|a| a.id == command.action && a.enabled)
        {
            return None;
        }
        if command.target.is_some() && command.action != "paste" {
            return None;
        }
        if command
            .target
            .is_some_and(|point| !point.iter().all(|v| v.is_finite()))
        {
            return None;
        }
        Some(match command.action.as_str() {
            "record" => PillAction::RecordSelected,
            "cancel" => PillAction::Cancel,
            "finish" => PillAction::Finish,
            "whiteboard" => PillAction::Whiteboard,
            "paste" => PillAction::Paste(command.target),
            "copy-link" => PillAction::CopyLink,
            "inspect" => PillAction::Inspect,
            "dismiss" => PillAction::DismissCompleted,
            action if action.starts_with("select-") => {
                PillAction::SelectMode(SelectionMode::from_key(&action[7..])?)
            }
            action if action.starts_with("switch-") => {
                PillAction::SwitchRecording(SelectionMode::from_key(&action[7..])?)
            }
            _ => return None,
        })
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub revision: u64,
    pub action: String,
    #[serde(default)]
    pub target: Option<[f32; 2]>,
}

#[cfg(test)]
mod tests;
