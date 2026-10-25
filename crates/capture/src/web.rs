//! Dev pages telling Scrawl which OpenCode session they belong to.
//!
//! A project adds `<script src="http://127.0.0.1:47777/sdk.js" data-session="ses_…"></script>`
//! (or `data-directory`) to its dev build. The script registers the page with the ingest server
//! and heartbeats while the tab is open; a Mac capture of that tab is then sent to the session
//! (`routing`). Registrations last `TTL_MS` past the last heartbeat.
//!
//! Without the ingest token, only a page on a loopback origin, calling from this Mac, may
//! register, and only for its own origin. Any other origin must send the token as a bearer.
//!
//! With the `agentContext` experiment on, a loopback page also posts what it sees under clicks
//! and marks, plus errors, failed requests and video playback, to `/web/context` (`crate::page`).

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::Deserialize;

use crate::routing::{is_loopback_host, parse_url};

/// How long a registration lasts after its last heartbeat. The script beats every 30 s.
pub const TTL_MS: u64 = 120_000;
const MAX_ENTRIES: usize = 64;
const SDK_JS: &str = include_str!("sdk.js");

pub use scrawl_router::routing::Registration;

#[derive(Debug, Default)]
pub struct Registry {
    entries: Vec<Registration>,
}

impl Registry {
    pub fn register(&mut self, mut reg: Registration, now: u64) {
        reg.seen_ms = now;
        self.entries.retain(|e| {
            now.saturating_sub(e.seen_ms) <= TTL_MS && !(e.origin == reg.origin && e.tab == reg.tab)
        });
        self.entries.push(reg);
        if self.entries.len() > MAX_ENTRIES {
            self.entries.sort_by_key(|e| std::cmp::Reverse(e.seen_ms));
            self.entries.truncate(MAX_ENTRIES);
        }
    }

    pub fn unregister(&mut self, origin: &str, tab: &str) {
        self.entries
            .retain(|e| !(e.origin == origin && e.tab == tab));
    }

    pub fn live(&self, now: u64) -> Vec<Registration> {
        self.entries
            .iter()
            .filter(|e| now.saturating_sub(e.seen_ms) <= TTL_MS)
            .cloned()
            .collect()
    }

    /// The live registration for `url`'s origin: the tab on that exact URL, else the most
    /// recently heard from.
    pub fn lookup(&self, url: &str, now: u64) -> Option<Registration> {
        let origin = parse_url(url)?.origin();
        let live = self
            .live(now)
            .into_iter()
            .filter(|e| e.origin == origin)
            .collect::<Vec<_>>();
        live.iter()
            .filter(|e| e.url == url)
            .max_by_key(|e| e.seen_ms)
            .or_else(|| live.iter().max_by_key(|e| e.seen_ms))
            .cloned()
    }
}

static REGISTRY: Mutex<Registry> = Mutex::new(Registry {
    entries: Vec::new(),
});
/// Whether this process owns the ingest port, and so holds the registrations.
static SERVING: AtomicBool = AtomicBool::new(false);

fn registry() -> std::sync::MutexGuard<'static, Registry> {
    REGISTRY
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub fn set_serving() {
    SERVING.store(true, Ordering::Release);
}

/// The live registration for `url`, from this process when it runs the ingest server, else
/// from the one that does.
pub fn lookup(url: &str) -> Option<Registration> {
    let now = crate::pipeline::now_epoch_ms();
    if SERVING.load(Ordering::Acquire) {
        return registry().lookup(url, now);
    }
    // `SCRAWL_INGEST_PORT` points at a `scrawl serve --port` other than the app's.
    let port = std::env::var("SCRAWL_INGEST_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(crate::phone::DEFAULT_INGEST_PORT);
    let mut remote = Registry::default();
    for reg in fetch_registrations(port).ok()? {
        remote.entries.push(reg);
    }
    remote.lookup(url, now)
}

/// The running server's live registrations (`GET /web/registrations` with the ingest token).
pub fn fetch_registrations(port: u16) -> Result<Vec<Registration>> {
    let token = crate::handoff::ensure_ingest_token()?;
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(400))
        .with_context(|| format!("Scrawl isn't listening on port {port}"))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    write!(
        stream,
        "GET /web/registrations HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"
    )?;
    let mut response = Vec::new();
    stream.read_to_end(&mut response)?;
    let response = String::from_utf8_lossy(&response);
    let (head, body) = response
        .split_once("\r\n\r\n")
        .context("Malformed response")?;
    anyhow::ensure!(
        head.starts_with("HTTP/1.1 200"),
        "Scrawl answered {}",
        head.lines().next().unwrap_or("")
    );
    let value: serde_json::Value = serde_json::from_str(body)?;
    Ok(serde_json::from_value(
        value.get("registrations").cloned().unwrap_or_default(),
    )?)
}

// MARK: Who may register

/// A browser `Origin` on this machine: `http(s)://localhost`, `*.localhost`, `127.x`, `[::1]`.
pub fn is_loopback_origin(origin: &str) -> bool {
    parse_url(origin).is_some_and(|u| is_loopback_host(&u.host))
}

/// Unauthenticated registration needs a loopback page calling from this Mac (non-browser
/// clients can claim any `Origin`, so the peer address matters too); anything else needs the
/// ingest token.
pub fn may_register(origin: Option<&str>, peer_loopback: bool, token_ok: bool) -> bool {
    token_ok || (peer_loopback && origin.is_some_and(is_loopback_origin))
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Body {
    origin: Option<String>,
    url: Option<String>,
    title: Option<String>,
    session: Option<String>,
    directory: Option<String>,
    tab: Option<String>,
}

fn valid_session(id: &str) -> bool {
    id.starts_with("ses")
        && id.len() <= 64
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The registration a request describes. Without the token the page's own `Origin` header is
/// used, whatever the body says.
fn registration_from(
    body: Body,
    header_origin: Option<&str>,
    token_ok: bool,
) -> std::result::Result<Registration, &'static str> {
    let claimed = if token_ok {
        body.origin
            .as_deref()
            .or(header_origin)
            .or(body.url.as_deref())
    } else {
        header_origin
    };
    let origin = claimed
        .and_then(parse_url)
        .ok_or("missing or invalid origin")?
        .origin();
    let url = body
        .url
        .filter(|u| u.len() <= 4096 && parse_url(u).is_some_and(|p| p.origin() == origin))
        .unwrap_or_else(|| format!("{origin}/"));
    let session = body
        .session
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty());
    if session.as_deref().is_some_and(|s| !valid_session(s)) {
        return Err("invalid session ID");
    }
    let directory = body
        .directory
        .map(|d| d.trim().trim_end_matches('/').to_owned())
        .filter(|d| !d.is_empty());
    if directory
        .as_deref()
        .is_some_and(|d| !d.starts_with('/') || d.len() > 1024 || d.contains('\0'))
    {
        return Err("directory must be an absolute path");
    }
    let tab: String = body
        .tab
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .take(32)
        .collect();
    Ok(Registration {
        origin,
        url,
        title: body.title.unwrap_or_default().chars().take(200).collect(),
        session,
        directory,
        tab,
        seen_ms: 0,
    })
}

// MARK: HTTP

fn header_value<'a>(header: &'a str, name: &str) -> Option<&'a str> {
    header.lines().skip(1).find_map(|line| {
        let (k, v) = line.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim())
    })
}

fn respond(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
    origin: Option<&str>,
) {
    let mut head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n",
        body.len()
    );
    if let Some(origin) = origin {
        head.push_str(&format!(
            "Access-Control-Allow-Origin: {origin}\r\nVary: Origin\r\nAccess-Control-Allow-Methods: POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization\r\nAccess-Control-Allow-Private-Network: true\r\nAccess-Control-Max-Age: 600\r\n"
        ));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

fn respond_json(
    stream: &mut TcpStream,
    status: &str,
    body: &serde_json::Value,
    origin: Option<&str>,
) {
    respond(
        stream,
        status,
        "application/json",
        body.to_string().as_bytes(),
        origin,
    );
}

/// Answers the SDK's routes; `false` when the request is for something else (the ScrawlKit
/// routes, which need the token).
pub fn handle(stream: &mut TcpStream, header: &str, expected_token: &str) -> Result<bool> {
    let first = header.lines().next().unwrap_or("");
    let mut parts = first.split_whitespace();
    let (method, path) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
    let path = path.split('?').next().unwrap_or("");
    let is_web = path.starts_with("/web/");
    if !(path == "/sdk.js" || is_web) {
        return Ok(false);
    }
    let origin = header_value(header, "origin").filter(|o| *o != "null");
    let token_ok = header_value(header, "authorization")
        .and_then(|v| {
            v.strip_prefix("Bearer ")
                .or_else(|| v.strip_prefix("bearer "))
        })
        .is_some_and(|t| crate::phone::constant_time_eq(t.trim(), expected_token));
    let peer_loopback = stream.peer_addr().is_ok_and(|a| a.ip().is_loopback());

    match (method, path) {
        ("GET", "/sdk.js") => {
            respond(
                stream,
                "200 OK",
                "text/javascript; charset=utf-8",
                SDK_JS.as_bytes(),
                None,
            );
        }
        ("OPTIONS", _) => respond(stream, "204 No Content", "text/plain", b"", origin),
        ("GET", "/web/registrations") => {
            if token_ok {
                let live = registry().live(crate::pipeline::now_epoch_ms());
                respond_json(
                    stream,
                    "200 OK",
                    &serde_json::json!({ "registrations": live }),
                    None,
                );
            } else {
                respond_json(
                    stream,
                    "401 Unauthorized",
                    &serde_json::json!({"ok": false}),
                    None,
                );
            }
        }
        ("POST", "/web/context") => {
            let len: usize = header_value(header, "content-length")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            anyhow::ensure!(len <= 64 * 1024, "Page context too large");
            let mut bytes = vec![0_u8; len];
            stream.read_exact(&mut bytes)?;
            // Loopback pages only, token or not.
            let Some(page_origin) = origin
                .filter(|o| peer_loopback && is_loopback_origin(o))
                .and_then(parse_url)
                .map(|u| u.origin())
            else {
                respond_json(
                    stream,
                    "403 Forbidden",
                    &serde_json::json!({"ok": false, "error": "only loopback pages send context"}),
                    None,
                );
                return Ok(true);
            };
            let wanted = crate::handoff::experiment(crate::handoff::AGENT_CONTEXT);
            if wanted && let Ok(post) = serde_json::from_slice::<crate::page::Post>(&bytes) {
                crate::page::record(&page_origin, post);
            }
            respond_json(
                stream,
                "200 OK",
                &serde_json::json!({"ok": true, "context": wanted}),
                origin,
            );
        }
        ("POST", "/web/register" | "/web/unregister") => {
            let len: usize = header_value(header, "content-length")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            anyhow::ensure!(len <= 16 * 1024, "Registration too large");
            // Read the body even when refusing, so the answer isn't lost to a reset.
            let mut bytes = vec![0_u8; len];
            stream.read_exact(&mut bytes)?;
            if !may_register(origin, peer_loopback, token_ok) {
                respond_json(
                    stream,
                    "403 Forbidden",
                    &serde_json::json!({"ok": false, "error": "only loopback pages may register without the ingest token"}),
                    None,
                );
                return Ok(true);
            }
            let body: Body = serde_json::from_slice(&bytes).unwrap_or_default();
            match registration_from(body, origin, token_ok) {
                Ok(reg) if path == "/web/unregister" => {
                    registry().unregister(&reg.origin, &reg.tab);
                    respond_json(stream, "200 OK", &serde_json::json!({"ok": true}), origin);
                }
                Ok(reg) => {
                    crate::log::scrawl_log(format!(
                        "web: {} registered (session {}, directory {})",
                        reg.url,
                        reg.session.as_deref().unwrap_or("-"),
                        reg.directory.as_deref().unwrap_or("-")
                    ));
                    // `context`: the page may send what it sees (`crate::page`); loopback only.
                    let context = origin.is_some_and(is_loopback_origin)
                        && crate::handoff::experiment(crate::handoff::AGENT_CONTEXT);
                    registry().register(reg, crate::pipeline::now_epoch_ms());
                    respond_json(
                        stream,
                        "200 OK",
                        &serde_json::json!({"ok": true, "ttlMs": TTL_MS, "context": context}),
                        origin,
                    );
                }
                Err(error) => respond_json(
                    stream,
                    "400 Bad Request",
                    &serde_json::json!({"ok": false, "error": error}),
                    origin,
                ),
            }
        }
        _ => respond_json(
            stream,
            "404 Not Found",
            &serde_json::json!({"ok": false}),
            origin,
        ),
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_origins_register_without_the_token_and_others_need_it() {
        for origin in [
            "http://localhost:5173",
            "http://127.0.0.1:3000",
            "https://app.localhost",
            "http://[::1]:8080",
        ] {
            assert!(may_register(Some(origin), true, false), "{origin}");
            // A non-browser client elsewhere can send any Origin.
            assert!(!may_register(Some(origin), false, false), "{origin}");
        }
        for origin in [
            "https://example.com",
            "http://studio.example.ts.net:5173",
            "http://localhost.evil.com",
            "http://192.168.1.4:5173",
        ] {
            assert!(!may_register(Some(origin), true, false), "{origin}");
            assert!(may_register(Some(origin), false, true), "{origin}");
        }
        assert!(!may_register(None, true, false));
    }

    #[test]
    fn unauthenticated_pages_register_only_their_own_origin() {
        let body = Body {
            origin: Some("https://victim.example".into()),
            url: Some("https://victim.example/x".into()),
            session: Some("ses_abc".into()),
            tab: Some("t1!".into()),
            ..Body::default()
        };
        let reg = registration_from(body, Some("http://localhost:5173"), false).unwrap();
        assert_eq!(reg.origin, "http://localhost:5173");
        assert_eq!(reg.url, "http://localhost:5173/");
        assert_eq!(reg.tab, "t1");
        assert_eq!(reg.session.as_deref(), Some("ses_abc"));

        let bad = Body {
            session: Some("ses_a; rm -rf".into()),
            ..Body::default()
        };
        assert!(registration_from(bad, Some("http://localhost:1"), false).is_err());
        let relative = Body {
            directory: Some("site".into()),
            ..Body::default()
        };
        assert!(registration_from(relative, Some("http://localhost:1"), false).is_err());
        assert!(registration_from(Body::default(), None, false).is_err());

        let with_token = Body {
            url: Some("https://studio.ts.net:5173/a".into()),
            directory: Some("/p/app/".into()),
            ..Body::default()
        };
        let reg = registration_from(with_token, None, true).unwrap();
        assert_eq!(reg.origin, "https://studio.ts.net:5173");
        assert_eq!(reg.directory.as_deref(), Some("/p/app"));
    }

    fn reg(origin: &str, url: &str, tab: &str, session: &str) -> Registration {
        Registration {
            origin: origin.into(),
            url: url.into(),
            tab: tab.into(),
            session: Some(session.into()),
            ..Registration::default()
        }
    }

    #[test]
    fn registrations_expire_and_prefer_the_exact_tab() {
        let mut registry = Registry::default();
        let o = "http://localhost:5173";
        registry.register(reg(o, "http://localhost:5173/a", "1", "ses_a"), 1_000);
        registry.register(reg(o, "http://localhost:5173/b", "2", "ses_b"), 2_000);
        let session = |r: &Registry, url: &str, now| r.lookup(url, now).and_then(|r| r.session);
        assert_eq!(
            session(&registry, "http://localhost:5173/a", 3_000).as_deref(),
            Some("ses_a")
        );
        // Another page on the origin goes to the newest tab.
        assert_eq!(
            session(&registry, "http://localhost:5173/c", 3_000).as_deref(),
            Some("ses_b")
        );
        assert_eq!(session(&registry, "http://localhost:3000/", 3_000), None);
        // Tab 2 stops heartbeating; tab 1 heartbeats on.
        registry.register(reg(o, "http://localhost:5173/a", "1", "ses_a"), 100_000);
        assert_eq!(
            session(&registry, "http://localhost:5173/c", 2_000 + TTL_MS + 1).as_deref(),
            Some("ses_a")
        );
        assert_eq!(
            session(&registry, "http://localhost:5173/a", 100_000 + TTL_MS + 1),
            None
        );
        // A heartbeat replaces the tab's entry instead of adding one.
        registry.register(reg(o, "http://localhost:5173/a", "1", "ses_a"), 100_500);
        assert_eq!(registry.live(100_500).len(), 2);
        registry.unregister(o, "1");
        assert_eq!(registry.live(100_500).len(), 1);
    }

    fn request(raw: &str, token: &str) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let raw = raw.to_owned();
        let client = std::thread::spawn(move || {
            let mut stream = TcpStream::connect(addr).unwrap();
            stream.write_all(raw.as_bytes()).unwrap();
            let mut out = String::new();
            stream.read_to_string(&mut out).unwrap();
            out
        });
        let (mut stream, _) = listener.accept().unwrap();
        let mut header = Vec::new();
        let mut byte = [0_u8; 1];
        while !header.ends_with(b"\r\n\r\n") {
            stream.read_exact(&mut byte).unwrap();
            header.push(byte[0]);
        }
        let header = String::from_utf8(header).unwrap();
        assert!(handle(&mut stream, &header, token).unwrap());
        drop(stream);
        client.join().unwrap()
    }

    #[test]
    fn http_routes_serve_the_script_and_check_registrations() {
        let js = request("GET /sdk.js HTTP/1.1\r\nHost: x\r\n\r\n", "tok");
        assert!(
            js.starts_with("HTTP/1.1 200") && js.contains("/web/register"),
            "{js}"
        );

        let body = r#"{"url":"http://localhost:6123/","session":"ses_webtest","tab":"zz"}"#;
        let ok = request(
            &format!(
                "POST /web/register HTTP/1.1\r\nOrigin: http://localhost:6123\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ),
            "tok",
        );
        assert!(ok.starts_with("HTTP/1.1 200"), "{ok}");
        assert!(ok.contains("Access-Control-Allow-Origin: http://localhost:6123"));
        assert_eq!(
            registry()
                .lookup("http://localhost:6123/", crate::pipeline::now_epoch_ms())
                .and_then(|r| r.session)
                .as_deref(),
            Some("ses_webtest")
        );

        let remote = request(
            &format!(
                "POST /web/register HTTP/1.1\r\nOrigin: https://example.com\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ),
            "tok",
        );
        assert!(remote.starts_with("HTTP/1.1 403"), "{remote}");
        let authed = request(
            &format!(
                "POST /web/register HTTP/1.1\r\nOrigin: https://example.com\r\nAuthorization: Bearer tok\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ),
            "tok",
        );
        assert!(authed.starts_with("HTTP/1.1 200"), "{authed}");

        let list = request("GET /web/registrations HTTP/1.1\r\n\r\n", "tok");
        assert!(list.starts_with("HTTP/1.1 401"), "{list}");
        let list = request(
            "GET /web/registrations HTTP/1.1\r\nAuthorization: Bearer tok\r\n\r\n",
            "tok",
        );
        assert!(list.contains("ses_webtest"), "{list}");

        let unregister = request(
            &format!(
                "POST /web/unregister HTTP/1.1\r\nOrigin: http://localhost:6123\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ),
            "tok",
        );
        assert!(unregister.starts_with("HTTP/1.1 200"), "{unregister}");

        // Page context is loopback-only even with the token, and ignored with the experiment off.
        let context = r#"{"tab":"zz","events":[]}"#;
        let refused = request(
            &format!(
                "POST /web/context HTTP/1.1\r\nOrigin: https://example.com\r\nAuthorization: Bearer tok\r\nContent-Length: {}\r\n\r\n{context}",
                context.len()
            ),
            "tok",
        );
        assert!(refused.starts_with("HTTP/1.1 403"), "{refused}");
        let accepted = request(
            &format!(
                "POST /web/context HTTP/1.1\r\nOrigin: http://localhost:6123\r\nContent-Length: {}\r\n\r\n{context}",
                context.len()
            ),
            "tok",
        );
        assert!(
            accepted.starts_with("HTTP/1.1 200") && accepted.contains(r#""context":false"#),
            "{accepted}"
        );
        assert!(
            registry()
                .lookup("http://localhost:6123/", crate::pipeline::now_epoch_ms())
                .is_none()
        );
    }
}
