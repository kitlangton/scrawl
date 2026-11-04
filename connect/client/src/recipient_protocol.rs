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
}

impl Request {
    /// Validate before reading configuration, credentials or making network requests.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.protocol == VERSION, "Incompatible Connect recipient protocol");
        if let Operation::Fetch { out, .. } = &self.operation {
            anyhow::ensure!(out.is_absolute(), "Recipient output must be absolute");
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
}
