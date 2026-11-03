//! Secret-free curl diagnostics. Never include stderr, headers, URLs or response text.
use std::time::Duration;

pub(super) const CONNECT_TIMEOUT_SECS: u64 = 15;

pub(super) fn phase(method: &str, route: &str) -> &'static str {
    match (method, route) {
        ("POST", "/v1/device/start") => "device authorization start",
        ("POST", "/v1/device/poll") => "device approval polling",
        ("POST", "/v1/token/refresh") => "credential renewal",
        ("GET", "/v1/account") => "account check",
        ("GET", "/v1/encryption/peers") => "encryption recipient discovery",
        _ if route.starts_with("/v1/captures/") => "capture transfer",
        _ => "API request",
    }
}

pub(super) fn failure(
    stage: &str,
    exit: Option<i32>,
    http: Option<u16>,
    elapsed: Duration,
    timeout: Duration,
) -> String {
    let kind = match exit {
        Some(5) => "proxy DNS resolution failed",
        Some(6) => "host DNS resolution failed",
        Some(7) => "connection failed",
        Some(28) => "timeout",
        Some(35) => "TLS handshake failed",
        Some(51 | 58 | 60 | 77 | 83 | 90 | 91) => {
            "TLS certificate verification or configuration failed"
        }
        Some(63) => "response size limit exceeded",
        Some(97) => "proxy handshake failed",
        Some(18 | 52 | 55 | 56) => "connection interrupted",
        _ => "transport failed",
    };
    format!(
        "Hosted transport failed during {stage}: {kind} (curl_exit={}, http_status={}, elapsed_ms={}, connect_timeout_ms={}, request_timeout_ms={}). Raw network diagnostics are suppressed to protect credentials and proxy settings.",
        exit.map(|value| value.to_string())
            .unwrap_or_else(|| "signal".into()),
        http.map(|value| value.to_string())
            .unwrap_or_else(|| "unavailable".into()),
        elapsed.as_millis(),
        timeout
            .min(Duration::from_secs(CONNECT_TIMEOUT_SECS))
            .as_millis(),
        timeout.as_millis(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classifies_curl_failures_without_remote_details() {
        for (code, expected) in [
            (5, "proxy DNS"),
            (6, "host DNS"),
            (7, "connection failed"),
            (28, "timeout"),
            (35, "TLS handshake"),
            (60, "TLS certificate"),
            (63, "response size"),
            (97, "proxy handshake"),
            (56, "interrupted"),
            (99, "transport failed"),
        ] {
            let message = failure(
                phase("POST", "/v1/device/start"),
                Some(code),
                Some(0),
                Duration::from_millis(2800),
                Duration::from_secs(30),
            );
            assert!(message.contains(expected), "{message}");
            assert!(
                message.contains("device authorization start")
                    && message.contains("elapsed_ms=2800")
            );
            assert!(message.contains("connect_timeout_ms=15000"));
        }
        assert_eq!(phase("POST", "/v1/device/poll"), "device approval polling");
        assert_eq!(
            phase("GET", "/v1/captures/private-id/files/private-name"),
            "capture transfer"
        );
        assert_eq!(phase("GET", "/v1/untrusted-secret"), "API request");
        let message = failure(
            phase("GET", "/v1/untrusted-secret"),
            None,
            None,
            Duration::ZERO,
            Duration::from_millis(100),
        );
        assert!(!message.contains("untrusted-secret"));
        assert!(message.contains("connect_timeout_ms=100"));
    }
}
