//! Application composition for the independent Connect client.
use anyhow::{Context, Result};
pub use apparent_connect_client::{
    AccountView, CloudConfig, ConnectCommand, HOSTED_SHARING, HOSTED_SYNC, PreparedLink,
    SharesCommand,
};
use apparent_connect_client::{Client, ConfigProvider};
// Package call sites own the concrete flags; this adapter preserves override semantics.
use crate::shared::handoff::experiment as app_experiment;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

struct AppConfig(PathBuf);
impl ConfigProvider for AppConfig {
    fn config_path(&self) -> &Path {
        &self.0
    }
    fn cloud(&self) -> CloudConfig {
        crate::shared::handoff::load_config_from(&self.0)
            .cloud
            .unwrap_or_default()
    }
    fn experiment(&self, name: &str) -> bool {
        app_experiment(name)
    }
    fn set_cloud(&self, value: &CloudConfig) -> Result<()> {
        self.set_cloud_allowed(value, &|| true)
    }
    fn set_cloud_allowed(&self, value: &CloudConfig, allowed: &dyn Fn() -> bool) -> Result<()> {
        crate::shared::handoff::update_config_allowed_in(&self.0, allowed, |config| {
            let cloud = config.as_object_mut().unwrap().entry("cloud")
                .or_insert_with(|| serde_json::json!({}));
            let cloud = cloud.as_object_mut().context("Invalid cloud configuration")?;
            cloud.insert("enabled".into(), value.enabled.into());
            cloud.insert("uploadConsent".into(), value.upload_consent.into());
            cloud.insert("server".into(), value.server.clone().into());
            Ok(())
        })
    }
    fn set_experiment(&self, name: &str, enabled: bool) -> Result<()> {
        crate::shared::handoff::update_config_allowed_in(&self.0, &|| true, |config| {
            let experiments = config.as_object_mut().unwrap().entry("experiments")
                .or_insert_with(|| serde_json::json!({}));
            experiments.as_object_mut().context("Invalid experiments configuration")?
                .insert(name.to_owned(), enabled.into());
            Ok(())
        })
    }
    fn open_browser(&self, uri: &Path) {
        let _ = crate::shared::handoff::open_path(uri);
    }
}
fn client_at(path: PathBuf) -> Client {
    Client::new(Arc::new(AppConfig(path)))
}

/// Settings composition only; all protocol, credential and consent policy remains
/// in the production client. The controller owns browser clicks and worker start.
pub fn settings_backend(
    config: &Path,
) -> Arc<dyn crate::capture::desktop::settings::connect::Backend> {
    Arc::new(SettingsBackend(client_at(config.into()), config.into()))
}

struct SettingsBackend(Client, PathBuf);
use crate::capture::desktop::settings::connect as settings_controller;
use apparent_connect_client::settings as client_settings;

fn settings_error(error: client_settings::SafeError) -> settings_controller::SafeError {
    use client_settings::SafeError as C;
    use settings_controller::SafeError as S;
    match error {
        C::Cancelled => S::Cancelled,
        C::Offline => S::Offline,
        C::AuthRequired => S::AuthRequired,
        C::Rejected => S::Rejected,
        C::Expired => S::Expired,
        C::LocalState => S::LocalState,
        C::InvalidServer => S::InvalidServer,
        C::Unavailable => S::Unavailable,
    }
}

impl settings_controller::Backend for SettingsBackend {
    fn shared_links_url(&self) -> Option<String> {
        crate::shared::handoff::capture_location::experiment_in(&self.1, HOSTED_SHARING)
            .ok().filter(|enabled| *enabled).and_then(|_| self.0.shared_links_url())
    }
    fn snapshot(
        &self,
    ) -> std::result::Result<settings_controller::Snapshot, settings_controller::SafeError> {
        self.0
            .settings_snapshot()
            .map(|state| settings_controller::Snapshot {
                server: state.server,
                connected: state.connected,
                uploads_enabled: state.uploads_enabled,
                reauthorization_required: state.reauthorization_required,
            })
            .map_err(settings_error)
    }
    fn login(
        &self,
        server: &str,
        allowed: Arc<dyn Fn() -> bool + Send + Sync>,
        on_approval: &dyn Fn(settings_controller::Approval),
    ) -> std::result::Result<(), settings_controller::SafeError> {
        self.0
            .login_with_approval(server, "upload", true, &*allowed, &|approval| {
                on_approval(settings_controller::Approval {
                    verification_uri: approval.verification_uri,
                    user_code: approval.user_code,
                });
            })
            .map_err(settings_error)
    }
    fn health(&self, allowed: Arc<dyn Fn() -> bool + Send + Sync>) -> settings_controller::Health {
        match self.0.settings_health(&*allowed) {
            client_settings::Health::Online => settings_controller::Health::Online,
            client_settings::Health::Offline => settings_controller::Health::Offline,
            client_settings::Health::AuthRequired => settings_controller::Health::AuthRequired,
        }
    }
    fn disable(&self, logout: bool) -> std::result::Result<(), settings_controller::SafeError> {
        self.0.settings_disable(logout).map_err(settings_error)
    }
}
fn client() -> Client {
    client_at(crate::shared::handoff::default_config_path())
}

pub fn copy_link_handoff(bundle: &Path, allowed: &dyn Fn() -> bool) -> Result<PreparedLink> {
    let link = client().prepare_link(bundle, allowed)?;
    start_worker();
    Ok(link)
}
pub fn prepare_public_link_handoff(bundle: &Path, allowed: &dyn Fn() -> bool) -> Result<PreparedLink> {
    let link = client().prepare_public_link(bundle, allowed)?;
    start_worker();
    Ok(link)
}
pub fn sharing_connected() -> bool {
    client().sharing_connected()
}
pub fn sharing_server() -> Option<String> {
    client().sharing_server()
}
pub fn shared_links_url() -> Option<String> {
    client().shared_links_url()
}
pub fn account_view_in(config: &Path) -> AccountView {
    client_at(config.into()).account_view()
}
pub fn sharing_online() -> bool {
    client().sharing_online()
}
pub fn connect_sharing(allowed: &dyn Fn() -> bool) -> Result<()> {
    let server = sharing_server()
        .context("Configure the hosted API server with apparent connect login first")?;
    client().login_allowed(&server, "upload", true, true, allowed)
}
pub fn sharing_progress(bundle: &Path) -> String {
    client().sharing_progress(bundle)
}

pub fn command(command: ConnectCommand) -> Result<()> {
    let client = client();
    match command {
        ConnectCommand::Shares { command } => match command {
            SharesCommand::List => {
                println!("{}", serde_json::to_string_pretty(&client.shares()?)?);
                Ok(())
            }
            SharesCommand::Revoke { id } => {
                client.revoke_share(&id)?;
                println!("Sharing stopped. Downloads and cached previews cannot be recalled.");
                Ok(())
            }
        },
        ConnectCommand::Login {
            server,
            scope,
            consent_upload,
            no_browser,
        } => client.login(&server, &scope, consent_upload, !no_browser),
        ConnectCommand::Status { account } => {
            println!(
                "{}",
                serde_json::to_string_pretty(&client.status(account)?)?
            );
            Ok(())
        }
        ConnectCommand::Logout => {
            client.disable(true)?;
            println!(
                "Signed out locally; pending uploads cancelled. Server data expires after seven days."
            );
            Ok(())
        }
        ConnectCommand::Disable => {
            client.disable(false)?;
            println!("Hosted uploads disabled; pending uploads cancelled.");
            Ok(())
        }
        ConnectCommand::Retry => {
            client.retry()?;
            client.process_once()
        }
        ConnectCommand::Upload { bundle } => {
            // CLI errors are printed with their source chain by main. Do not
            // attach path/credential parsing errors to these public diagnostics.
            let bundle = crate::shared::handoff::resolve_bundle_dir(&bundle).map_err(|_| {
                anyhow::anyhow!("invalid_bundle: choose an existing .apparent directory")
            })?;
            client.enqueue_explicit(&bundle).map_err(|_| {
                anyhow::anyhow!(
                    "queue_unavailable: check upload consent, sign-in and private hosted state; local capture is unchanged"
                )
            })?;
            println!("Queued hosted capture; inspect its ID with apparent connect status");
            client.process_once()
        }
    }
}
pub fn fetch(id: &str, out: &Path, wait: u64) -> Result<()> {
    println!("{}", client().fetch(id, out, wait)?.display());
    Ok(())
}
/// Only a bare canonical Connect ID implies a download. Explicit local paths
/// retain local inspection semantics; fetch never overwrites existing output.
pub fn inspect_source(source: &Path) -> Result<PathBuf> {
    let Some(id) = inspect_capture_id(source) else {
        return Ok(source.to_path_buf());
    };
    client().fetch(id, &PathBuf::from(format!("./{id}.apparent")), 120)
}

fn inspect_capture_id(source: &Path) -> Option<&str> {
    source.to_str().filter(|id| {
        id.len() == 32
            && id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

#[cfg(test)]
mod inspect_tests {
    use super::*;

    #[test]
    fn only_bare_canonical_ids_trigger_connect_fetch() {
        let id = "0123456789abcdef0123456789abcdef";
        assert_eq!(inspect_capture_id(Path::new(id)), Some(id));
        for local in [
            format!("./{id}"),
            format!("/{id}"),
            format!("{id}.apparent"),
            id.to_uppercase(),
            "latest".into(),
            "apparent.json".into(),
            "0123456789abcdef0123456789abcde".into(),
            "g123456789abcdef0123456789abcdef".into(),
        ] {
            assert_eq!(inspect_capture_id(Path::new(&local)), None, "{local}");
        }
    }
}

pub fn completed(bundle: &Path) {
    if cfg!(test) {
        return;
    }
    let client = client();
    if client.enabled() {
        // Disk-only identity/queue publication happens before clipboard construction.
        // Hashing, staging and networking are exclusively background work.
        if client.enqueue(bundle).is_err() {
            crate::capture::engine::runtime::apparent_log(
                "Hosted queue unavailable; saved local capture is unchanged",
            );
        }
        start_worker();
    }
}
pub fn handoff_pointer(bundle: &Path) -> Option<String> {
    if cfg!(test) {
        return None;
    }
    client().pointer(bundle)
}
pub fn start_worker() {
    if cfg!(test) {
        return;
    }
    static STARTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if STARTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    std::thread::spawn(|| {
        let client = client();
        loop {
            if client.enabled() {
                let _ = client.process_once();
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    });
}

#[cfg(test)]
#[cfg(test)]
#[cfg(test)]
mod hosted_logging_tests;
#[cfg(test)]
mod hosted_settings_tests;
#[cfg(test)]
pub(crate) mod hosted_tests;
