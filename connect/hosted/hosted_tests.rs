use super::*;
use apparent_connect_client::test_support::{self, ConfigWriteHook, TestHost};
use serde_json::Value;
struct AppTestHost;
impl TestHost for AppTestHost {
    fn provider(&self, path: PathBuf) -> Arc<dyn ConfigProvider> {
        Arc::new(AppConfig(path))
    }
    fn enable_experiment(&self, name: &str) {
        crate::shared::handoff::enable_experiment(name);
    }
    fn set_config_value_in(&self, path: &Path, key: &str, value: Value) -> Result<()> {
        crate::shared::handoff::set_config_value_in(path, key, value)
    }
    fn set_experiment_in(&self, path: &Path, name: &str, enabled: bool) -> Result<()> {
        crate::shared::handoff::set_experiment_in(path, name, enabled)
    }
    fn set_config_write_hook(&self, hook: Option<ConfigWriteHook>) {
        crate::shared::handoff::set_config_write_hook(hook);
    }
}
fn setup() {
    test_support::install_host(Arc::new(AppTestHost));
}
#[test]
fn hosted_settings_revocation_survives_real_config_wait() {
    setup();
    test_support::settings_revocation_survives_config_wait();
}
pub(crate) fn deferred_cancelled_settings_login(
    active: Arc<std::sync::atomic::AtomicBool>,
    cancel: impl FnOnce(),
) -> Result<()> {
    setup();
    test_support::deferred_cancelled_settings_login(active, cancel)
}
#[test]
fn hosted_connect_disabled_upload_suggests_canonical_login() {
    setup();
    test_support::hosted_connect_disabled_upload_suggests_canonical_login();
}
#[test]
fn hosted_config_setting_snapshot_cannot_restore_consent() {
    setup();
    test_support::hosted_config_setting_snapshot_cannot_restore_consent();
}
#[test]
fn hosted_config_and_queue_locks_are_not_network_held() {
    setup();
    test_support::hosted_config_and_queue_locks_are_not_network_held();
}
#[test]
fn hosted_config_subprocess_writer() {
    setup();
    test_support::hosted_config_subprocess_writer();
}
#[test]
fn hosted_config_cross_process_nested_and_top_level_updates() {
    setup();
    test_support::hosted_config_cross_process_nested_and_top_level_updates();
}
#[test]
fn hosted_config_lock_rejects_symlinks_and_hardlinks() {
    setup();
    test_support::hosted_config_lock_rejects_symlinks_and_hardlinks();
}
#[test]
fn hosted_config_pins_parent_through_publication() {
    setup();
    test_support::hosted_config_pins_parent_through_publication();
}
#[test]
fn hosted_security_fetch_parent_swap_fails_closed() {
    setup();
    test_support::hosted_security_fetch_parent_swap_fails_closed();
}
#[test]
fn hosted_security_source_parent_swap_fails_closed() {
    setup();
    test_support::hosted_security_source_parent_swap_fails_closed();
}
#[test]
fn hosted_fixture_reads_fragmented_headers_from_nonblocking_accept() {
    setup();
    test_support::hosted_fixture_reads_fragmented_headers_from_nonblocking_accept();
}
#[test]
fn hosted_fixture_reads_fragmented_body_from_nonblocking_accept() {
    setup();
    test_support::hosted_fixture_reads_fragmented_body_from_nonblocking_accept();
}
#[test]
fn hosted_sharing_management_follows_encoded_cursors_and_rejects_loops() {
    setup();
    test_support::hosted_sharing_management_follows_encoded_cursors_and_rejects_loops();
}
#[test]
fn hosted_prepared_link_survives_upload_before_foreground_delivery() {
    setup();
    test_support::hosted_prepared_link_survives_upload_before_foreground_delivery();
}
#[test]
fn hosted_prepared_link_rejects_changed_authorization_after_progress() {
    setup();
    test_support::hosted_prepared_link_rejects_changed_authorization_after_progress();
}
#[test]
fn hosted_share_request_survives_progress_during_transport() {
    setup();
    test_support::hosted_share_request_survives_progress_during_transport();
}
#[test]
fn hosted_sharing_sender_reports_interruption_and_recovers_under_same_url() {
    setup();
    test_support::hosted_sharing_sender_reports_interruption_and_recovers_under_same_url();
}
#[test]
fn hosted_sharing_management_uses_management_id_and_verifies_revoke() {
    setup();
    test_support::hosted_sharing_management_uses_management_id_and_verifies_revoke();
}
#[test]
fn hosted_sharing_reserves_before_media_reuses_url_and_resumes_queue() {
    setup();
    test_support::hosted_sharing_reserves_before_media_reuses_url_and_resumes_queue();
}
#[test]
fn hosted_roundtrip_pending_pointer_restart_complete_and_no_overwrite() {
    setup();
    test_support::hosted_roundtrip_pending_pointer_restart_complete_and_no_overwrite();
}
#[test]
fn hosted_disable_logout_and_quota_preserve_local() {
    setup();
    test_support::hosted_disable_logout_and_quota_preserve_local();
}
#[test]
fn hosted_login_snapshot_cannot_restore_consent_after_disable() {
    setup();
    test_support::hosted_login_snapshot_cannot_restore_consent_after_disable();
}
#[test]
fn hosted_upload_login_snapshot_cannot_restore_consent_after_logout() {
    setup();
    test_support::hosted_upload_login_snapshot_cannot_restore_consent_after_logout();
}
#[test]
fn hosted_read_login_preserves_upload_gates_and_active_server() {
    setup();
    test_support::hosted_read_login_preserves_upload_gates_and_active_server();
}
#[test]
fn hosted_login_pending_reject_scope_and_explicit_consent() {
    setup();
    test_support::hosted_login_pending_reject_scope_and_explicit_consent();
}
#[test]
fn hosted_integrity_and_pending_timeout_publish_nothing() {
    setup();
    test_support::hosted_integrity_and_pending_timeout_publish_nothing();
}
#[test]
fn hosted_transport_network_timeout_and_cancellation() {
    setup();
    test_support::hosted_transport_network_timeout_and_cancellation();
}
#[test]
fn hosted_paths_symlinks_oversize_and_unknown_assets() {
    setup();
    test_support::hosted_paths_symlinks_oversize_and_unknown_assets();
}
#[test]
fn hosted_malicious_manifests_symlink_output_and_bounds() {
    setup();
    test_support::hosted_malicious_manifests_symlink_output_and_bounds();
}
#[test]
fn hosted_disable_rejects_enqueue_paused_before_queue_lock() {
    setup();
    test_support::hosted_disable_rejects_enqueue_paused_before_queue_lock();
}
#[test]
fn hosted_disable_interrupts_inflight_request_and_never_resurrects() {
    setup();
    test_support::hosted_disable_interrupts_inflight_request_and_never_resurrects();
}
#[test]
fn hosted_cancellation_survives_reenable_during_staging() {
    setup();
    test_support::hosted_cancellation_survives_reenable_during_staging();
}
#[test]
fn hosted_cancellation_survives_reenable_during_real_media_put() {
    setup();
    test_support::hosted_cancellation_survives_reenable_during_real_media_put();
}
#[test]
fn hosted_explicit_upload_recovers_changed_expired_and_tombstoned_ids() {
    setup();
    test_support::hosted_explicit_upload_recovers_changed_expired_and_tombstoned_ids();
}
#[test]
fn hosted_automatic_dedup_ends_when_terminal_record_is_pruned() {
    setup();
    test_support::hosted_automatic_dedup_ends_when_terminal_record_is_pruned();
}
#[test]
fn hosted_config_pause_and_cancelled_dedup_do_not_backfill() {
    setup();
    test_support::hosted_config_pause_and_cancelled_dedup_do_not_backfill();
}
#[cfg(target_os = "macos")]
#[test]
fn hosted_security_darwin_acl_grants_fail_closed() {
    setup();
    test_support::hosted_security_darwin_acl_grants_fail_closed();
}
#[test]
fn hosted_security_sticky_owned_output_and_unshared_source_directories() {
    setup();
    test_support::hosted_security_sticky_owned_output_and_unshared_source_directories();
}
#[test]
fn hosted_no_historical_scan_private_credentials_and_explicit_read_required() {
    setup();
    test_support::hosted_no_historical_scan_private_credentials_and_explicit_read_required();
}
#[test]
fn hosted_connect_alias_command_subprocess() {
    crate::composition::cli::hosted_connect_alias_command_subprocess().unwrap();
}

#[test]
fn hosted_connect_and_cloud_execute_same_disable_without_network() {
    setup();
    test_support::connect_alias_scenario(|name, config| {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "connect::hosted::hosted_tests::hosted_connect_alias_command_subprocess",
                "--nocapture",
            ])
            .env("APPARENT_TEST_CONNECT_ALIAS", name)
            .env("APPARENT_CONFIG", config)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("1 passed"),
            "exact subprocess selected no test"
        );
        assert!(stdout.contains("Hosted uploads disabled; pending uploads cancelled."));
    });
}
#[test]
fn hosted_constructed_client_and_prepared_link_read_current_provider() {
    setup();
    test_support::hosted_constructed_client_and_prepared_link_read_current_provider();
}
#[test]
fn hosted_provider_write_failure_never_reports_disabled_or_login_success() {
    setup();
    test_support::hosted_provider_write_failure_never_reports_disabled_or_login_success();
}
