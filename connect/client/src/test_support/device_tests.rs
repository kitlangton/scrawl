use super::*;

fn begin_upload(f: &Fixture, server: &Server) -> DeviceLogin {
    f.client
        .begin_device_login(&server.url, "upload", true, Arc::new(|| true))
        .unwrap()
}

fn approve(login: &DeviceLogin) -> Value {
    let start = login.start().unwrap();
    let code = start["deviceCode"].as_str().unwrap();
    assert_eq!(login.poll(code, 30_000).unwrap()["status"], "pending");
    login.poll(code, 30_000).unwrap()
}

#[test]
fn device_login_requires_explicit_upload_consent_without_side_effects() {
    let f = Fixture::new();
    let server = Server::new("upload");
    let before = fs::read(&f.client.config).ok();
    assert!(
        f.client
            .begin_device_login(&server.url, "upload", false, Arc::new(|| true))
            .is_err()
    );
    assert_eq!(server.state.lock().unwrap().requests, 0);
    assert!(!f.client.state.join("upload.login").exists());
    assert_eq!(fs::read(&f.client.config).ok(), before);
}

#[test]
fn device_login_publishes_only_on_one_shot_commit_and_preserves_config() {
    let f = Fixture::new();
    let server = Server::new("upload");
    atomic_json(
        &f.client.config,
        &json!({"unrelated":{"kept":true},"experiments":{"hostedSync":true},"cloud":{"enabled":false,"uploadConsent":false,"server":""}}),
    )
    .unwrap();
    let login = begin_upload(&f, &server);
    let reply = approve(&login);
    assert_eq!(reply["status"], "approved");
    assert!(!f.client.state.join("upload.credential").exists());
    assert!(!f.client.settings().enabled);

    login.commit(&server.url).unwrap();
    let credential = f.client.stored_credential("upload").unwrap();
    assert_eq!(credential.server, server.url);
    assert_eq!(credential.site.as_deref(), Some(server.url.as_str()));
    assert_eq!(credential.scope, "upload");
    assert_eq!(credential.token, "synthetic-approved-secret");
    assert_eq!(
        credential.grant_id.as_deref(),
        Some("synthetic-server-grant")
    );
    assert!(f.client.settings().enabled && f.client.settings().upload_consent);
    assert_eq!(read_config(&f.client.config)["unrelated"]["kept"], true);
    assert!(login.commit(&server.url).is_err());
}

#[test]
fn device_login_rejects_wrong_poll_inputs_without_network_and_superseded_commit() {
    let f = Fixture::new();
    let server = Server::new("upload");
    let old = begin_upload(&f, &server);
    let start = old.start().unwrap();
    let code = start["deviceCode"].as_str().unwrap();
    let requests = server.state.lock().unwrap().requests;
    assert!(old.poll("wrong-device-code", 30_000).is_err());
    assert!(old.poll(code, 0).is_err());
    assert_eq!(server.state.lock().unwrap().requests, requests);

    assert_eq!(old.poll(code, 30_000).unwrap()["status"], "pending");
    old.poll(code, 30_000).unwrap();
    let replacement = begin_upload(&f, &server);
    assert!(old.commit(&server.url).is_err());
    assert!(!f.client.state.join("upload.credential").exists());
    assert!(replacement.start().is_ok());
}

#[test]
fn device_login_cancellation_and_disable_prevent_late_publication() {
    let f = Fixture::new();
    let server = Server::new("upload");
    let active = Arc::new(AtomicBool::new(true));
    let gate = active.clone();
    let login = f
        .client
        .begin_device_login(
            &server.url,
            "upload",
            true,
            Arc::new(move || gate.load(Ordering::SeqCst)),
        )
        .unwrap();
    approve(&login);
    let before = fs::read(&f.client.config).ok();
    active.store(false, Ordering::SeqCst);
    assert!(login.commit(&server.url).is_err());
    assert!(!f.client.state.join("upload.credential").exists());
    assert_eq!(fs::read(&f.client.config).ok(), before);

    let f = Fixture::new();
    let server = Server::new("upload");
    let login = begin_upload(&f, &server);
    approve(&login);
    f.client.disable(false).unwrap();
    assert!(login.commit(&server.url).is_err());
    assert!(!f.client.state.join("upload.credential").exists());
    assert!(!f.client.settings().enabled && !f.client.settings().upload_consent);
}

#[test]
fn device_login_site_and_read_server_guards_preserve_existing_state() {
    let f = Fixture::new();
    let server = Server::new("upload");
    let login = begin_upload(&f, &server);
    approve(&login);
    assert!(login.commit("https://site.example/path").is_err());
    assert!(!f.client.state.join("upload.credential").exists());
    login.commit(&server.url).unwrap();

    let f = Fixture::new();
    let server = Server::new("");
    f.authorize("https://active.example.com");
    let before = fs::read(&f.client.config).unwrap();
    let old_generation = f.client.stored_credential("read").unwrap().generation;
    let login = f
        .client
        .begin_device_login(&server.url, "read", false, Arc::new(|| true))
        .unwrap();
    approve(&login);
    assert!(login.commit(&server.url).is_err());
    assert_eq!(fs::read(&f.client.config).unwrap(), before);
    assert_eq!(
        f.client.stored_credential("read").unwrap().generation,
        old_generation
    );
}
