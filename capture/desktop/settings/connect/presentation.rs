//! Typed presentation shared by native Settings and Electron. No renderer infers
//! authorization or semantic status from English strings.
use super::*;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Tone { #[default] Neutral, Success, Progress, Warning, Failure }
impl Tone {
    pub fn color(self) -> u32 { match self {
        Self::Neutral => 0x020102, Self::Success => 0x146b43,
        Self::Progress => 0x7a58c7, Self::Warning => 0xffbe2e, Self::Failure => 0xb42318,
    } }
    pub fn symbol(self) -> &'static str { match self {
        Self::Neutral => "", Self::Success => "✓", Self::Progress => "◷",
        Self::Warning => "!", Self::Failure => "×",
    } }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Presentation {
    pub tone: Tone,
    pub headline: String,
    pub detail: String,
    pub note: String,
    pub artwork: bool,
    pub compact: bool,
    pub account_menu: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_initial: Option<String>,
    pub buttons: Vec<Button>,
    pub menu_actions: Vec<Command>,
    pub shared_links_url: Option<String>,
}

impl Controller {
    pub(super) fn presentation(&self, view: &View) -> Presentation {
        let mut p = Presentation {
            headline: "Help others see what you mean.".into(),
            detail: "Share your capture with people or agents.".into(),
            artwork: true,
            buttons: view.buttons.clone(),
            ..Default::default()
        };
        let snapshot = self.snapshot.as_ref();
        p.account_initial = snapshot.filter(|s| s.connected && !s.reauthorization_required
                && self.health != Some(Health::AuthRequired) && self.error != Some(SafeError::AuthRequired))
            .and_then(|s| s.account_initial.clone());
        if snapshot.is_some_and(|s| s.uploads_enabled) {
            p.headline = "You’re connected.".into();
            p.detail = "New captures upload privately.".into();
            p.tone = Tone::Success;
            p.compact = true;
        } else if snapshot.is_some_and(|s| s.connected) {
            p.headline = "Uploads are off.".into();
            p.detail = "You’re still signed in. New captures stay local.".into();
            p.artwork = false;
        }
        if let Some(health) = self.health {
            match health {
                Health::Offline => {
                    p.headline = "Connect is offline.".into();
                    p.detail = "Connect could not be reached. Local recording still works; your upload permission has not changed.".into();
                    p.tone = Tone::Failure;
                    p.compact = false; p.artwork = false;
                }
                Health::AuthRequired => p.failure("Sign in again.", "Sign in again to authorize this computer for Connect. Local recording still works."),
                Health::Online => {}
            }
        }
        if snapshot.is_some_and(|s| s.reauthorization_required) {
            p.failure("Sign in again.", "Sign in again to authorize this computer for Connect. Local recording still works.");
        }
        if let Some(error) = self.error {
            let title = match error {
                SafeError::Cancelled => "Connection cancelled.",
                SafeError::Offline => "Connect is offline.",
                SafeError::AuthRequired => "Sign in again.",
                SafeError::Rejected => "Sign-in wasn’t approved.",
                SafeError::Expired => "That sign-in expired.",
                SafeError::LocalState => "Connection settings unavailable.",
                SafeError::InvalidServer => "Check your Connect server.",
                SafeError::UnverifiedCommit => "Connection not confirmed.",
                SafeError::Unavailable => "Connect is unavailable.",
            };
            p.failure(title, error.message());
            if error == SafeError::Cancelled { p.tone = Tone::Neutral; }
        }
        if self.consent {
            p.headline = "Allow new capture uploads?".into();
            p.detail = "New captures upload privately, including recordings, images, audio, transcripts, context and other capture assets.".into();
            p.note = "Older captures stay local. Sharing is a separate action.".into();
            p.tone = Tone::Neutral; p.artwork = true; p.compact = false;
        }
        if let Some(request) = &self.request {
            p.tone = Tone::Progress; p.compact = false; p.artwork = false;
            match request.operation {
                Operation::Login if self.approval.is_some() => {
                    p.headline = "Finish connecting in your browser.".into();
                    p.detail = "Sign in and approve this computer.".into();
                    p.note = "Waiting for approval. Uploads haven’t started.".into();
                    p.artwork = true;
                }
                Operation::Login => {
                    p.headline = "Getting sign-in ready…".into();
                    p.detail = "Local recording still works.".into();
                }
                Operation::Probe => {
                    p.headline = "Checking connection…".into();
                    p.detail = "Local recording still works.".into();
                }
                Operation::Disable { logout } => {
                    p.headline = if logout { "Disconnecting…" } else { "Turning uploads off…" }.into();
                    p.detail = "Saving your choice and cancelling unfinished uploads.".into();
                }
            }
        }
        if let Some(error) = self.local_error {
            p.failure("Connection settings unavailable.", error.message());
        }
        // Preserve the backend's emergency/retry controls, including while state
        // readback is unavailable. Identity is separate from action availability.
        p.account_menu = snapshot.is_some_and(|s| s.connected || s.reauthorization_required)
            || view.buttons.iter().any(|b| matches!(b.command, Command::Disable | Command::Disconnect));
        p.shared_links_url = self.backend.shared_links_url();
        if p.shared_links_url.is_some() {
            p.buttons.push(button(Command::OpenSharedLinks, "Manage shared links", true));
            p.menu_actions.push(Command::OpenSharedLinks);
            p.account_menu = true;
        }
        for b in &mut p.buttons {
            if self.disabling() { continue; }
            b.title = match b.command {
                Command::Connect if self.health == Some(Health::AuthRequired) || snapshot.is_some_and(|s| s.reauthorization_required) => "Sign in again",
                Command::Connect if snapshot.is_some_and(|s| s.connected) => "Enable uploads",
                Command::Connect => "Connect",
                Command::Allow => "Allow & continue",
                Command::Cancel if self.consent => "Not now",
                Command::Cancel => "Cancel",
                Command::Disable if self.retry_disable == Some(false) => "Retry disabling uploads",
                Command::Disconnect if self.retry_disable == Some(true) => "Retry disconnecting",
                Command::Disable => "Turn off uploads",
                Command::Disconnect => "Disconnect this computer",
                _ => &b.title,
            }.into();
            if !self.disabling() && self.retry_disable.is_none() {
                b.command = match b.command {
                    Command::Disable => Command::RequestDisable,
                    Command::Disconnect => Command::RequestDisconnect,
                    other => other,
                };
            }
            if matches!(b.command, Command::Check | Command::RequestDisable | Command::RequestDisconnect | Command::Disable | Command::Disconnect) && !self.disabling() {
                p.menu_actions.push(b.command);
            }
        }
        if let Some(logout) = self.confirmation {
            p.headline = if logout { "Disconnect this computer?" } else { "Turn uploads off?" }.into();
            p.detail = if logout {
                "Unfinished uploads stop. Already uploaded captures and shared links are not deleted."
            } else {
                "Unfinished uploads stop. You stay signed in, and already uploaded captures remain."
            }.into();
            p.note.clear(); p.tone = Tone::Warning; p.artwork = false; p.compact = false;
            p.buttons = vec![
                button(if logout { Command::Disconnect } else { Command::Disable }, if logout { "Disconnect" } else { "Turn off uploads" }, true),
                button(Command::Cancel, if logout { "Keep connected" } else { "Keep uploads on" }, true),
            ];
            p.menu_actions.clear();
        }
        p.account_menu |= !p.menu_actions.is_empty();
        p
    }
}
impl Presentation {
    fn failure(&mut self, headline: &str, detail: &str) {
        self.tone = Tone::Failure; self.headline = headline.into(); self.detail = detail.into();
        self.artwork = false; self.compact = false;
    }
}
