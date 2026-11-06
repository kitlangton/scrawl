//! Versioned, closed recipient request vocabulary. No sender or local inspection operations.
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub const VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub protocol: u32,
    pub operation: Operation,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Operation {
    Login { server: Option<String>, browser: bool },
    Status { account: bool },
    Fetch { id: String, out: PathBuf, wait: u64, evidence_only: bool },
    /// Temporary same-writer renewal; returns no private credential bytes.
    RenewRead { server: String, generation: String, token: String, cancellation: Option<String>, force: bool },
    RejectRead { server: String, generation: String, token: String, cancellation: Option<String> },
}

impl Request {
    /// Validate before reading configuration, credentials or making network requests.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.protocol == VERSION, "Incompatible Connect recipient protocol");
        if let Operation::Fetch { out, .. } = &self.operation {
            anyhow::ensure!(out.is_absolute(), "Recipient output must be absolute");
        }
        if let Operation::RenewRead { server, generation, token, cancellation, .. }
            | Operation::RejectRead { server, generation, token, cancellation } = &self.operation {
            anyhow::ensure!(crate::server_url(server).ok().as_deref() == Some(server.as_str()), "Invalid renewal context");
            anyhow::ensure!(generation.len() == 32 && generation.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)), "Invalid renewal context");
            anyhow::ensure!(!token.is_empty() && token.len() <= 8192 && token.bytes().all(|b| b.is_ascii_graphic()), "Invalid renewal context");
            anyhow::ensure!(cancellation.as_ref().is_none_or(|epoch| serde_json::to_vec(epoch).is_ok_and(|b| b.len() <= 128)), "Invalid renewal context");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_sender_authority_and_unknown_fields() {
        for operation in [
            r#"{"kind":"upload","bundle":"/tmp/example"}"#,
            r#"{"kind":"login","server":null,"browser":false,"scope":"upload"}"#,
            r#"{"kind":"login","server":null,"browser":false,"consent_upload":true}"#,
            r#"{"kind":"inspect","bundle":"/tmp/example"}"#,
        ] {
            assert!(serde_json::from_str::<Request>(&format!(
                "{{\"protocol\":1,\"operation\":{operation}}}"
            )).is_err());
        }
    }

    #[test]
    fn rejects_incompatible_version_and_relative_destination() {
        let mut request = Request { protocol: 2, operation: Operation::Status { account: false } };
        assert!(request.validate().is_err());
        request.protocol = VERSION;
        assert!(request.validate().is_ok());
        request.operation = Operation::Fetch { id: "synthetic".into(), out: "relative".into(), wait: 0, evidence_only: true };
        assert!(request.validate().is_err());
    }

    #[test]
    fn read_renewal_requires_a_bounded_exact_context_without_sender_authority() {
        let value = serde_json::json!({"protocol":1,"operation":{"kind":"renew_read",
            "server":"https://app.tryapparent.com/connect","generation":"a".repeat(32),
            "token":"synthetic","cancellation":null,"force":true}});
        assert!(serde_json::from_value::<Request>(value.clone()).unwrap().validate().is_ok());
        for (key, invalid) in [("server", serde_json::json!("https://app.tryapparent.com/connect-extra")),
            ("generation", serde_json::json!("bad")), ("token", serde_json::json!("bad\n")),
            ("cancellation", serde_json::json!("x".repeat(129)))] {
            let mut bad = value.clone(); bad["operation"][key] = invalid;
            assert!(serde_json::from_value::<Request>(bad).unwrap().validate().is_err());
        }
        let mut bad = value;
        bad["operation"]["scope"] = serde_json::json!("upload");
        assert!(serde_json::from_value::<Request>(bad).is_err());
    }
}
