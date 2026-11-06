//! Version 2 metadata framing. Native authority is checked by the service.
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub type Result<T> = std::result::Result<T, &'static str>;
pub const MAX_LINE: usize = 1024 * 1024;

pub struct Request {
    pub rpc: String,
    pub client: String,
    pub method: String,
    pub authority: Option<Authority>,
    pub request_json: Option<String>,
}
pub struct Authority {
    pub capture_id: String,
    pub owner_token: String,
    pub job_id: String,
    pub request_sha256: String,
}
fn hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn parse(value: Value) -> Result<Request> {
    let object = value.as_object().ok_or("malformed_request")?;
    if object.get("channel").and_then(Value::as_str) != Some("media") {
        return Err("unsupported_channel");
    }
    if object.get("protocolVersion").and_then(Value::as_u64) != Some(2) {
        return Err("unsupported_version");
    }
    let text = |key| {
        object
            .get(key)
            .and_then(Value::as_str)
            .ok_or("malformed_request")
    };
    let rpc = text("rpcId")?.to_owned();
    let client = text("clientId")?.to_owned();
    if !hex(&rpc, 32) || !hex(&client, 32) {
        return Err("malformed_request");
    }
    let method = text("method")?.to_owned();
    let mut fields = vec!["channel", "protocolVersion", "clientId", "rpcId", "method"];
    match method.as_str() {
        "handshake" => (),
        "derive" | "inspect" | "cancel" => {
            fields.extend(["captureId", "ownerToken", "jobId", "requestSha256"])
        }
        _ => return Err("malformed_request"),
    }
    if method == "derive" {
        fields.push("requestJson");
    }
    if object.len() != fields.len() || fields.iter().any(|key| !object.contains_key(*key)) {
        return Err("malformed_request");
    }
    let authority = if method == "handshake" {
        None
    } else {
        let capture_id = text("captureId")?.to_owned();
        let owner_token = text("ownerToken")?.to_owned();
        let job_id = text("jobId")?.to_owned();
        let request_sha256 = text("requestSha256")?.to_owned();
        if !hex(&capture_id, 32)
            || !hex(&owner_token, 64)
            || !hex(&job_id, 32)
            || !hex(&request_sha256, 64)
        {
            return Err("malformed_request");
        }
        Some(Authority {
            capture_id,
            owner_token,
            job_id,
            request_sha256,
        })
    };
    let request_json = if method == "derive" {
        let bytes = text("requestJson")?;
        if bytes.len() > 64 * 1024
            || format!("{:x}", Sha256::digest(bytes.as_bytes()))
                != authority
                    .as_ref()
                    .ok_or("malformed_request")?
                    .request_sha256
        {
            return Err("malformed_request");
        }
        Some(bytes.to_owned())
    } else {
        None
    };
    Ok(Request {
        rpc,
        client,
        method,
        authority,
        request_json,
    })
}
pub fn response(rpc: &str, result: Result<Value>) -> Value {
    let mut value = json!({"channel":"media","protocolVersion":2,"rpcId":rpc});
    match result {
        Ok(result) => value["result"] = result,
        Err(code) => value["error"] = json!({"code":code}),
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    fn base() -> Value {
        json!({"channel":"media","protocolVersion":2,"clientId":"a".repeat(32),"rpcId":"b".repeat(32),"method":"handshake"})
    }
    #[test]
    fn explicit_channel_and_strict_authority_fields() {
        assert!(parse(base()).is_ok());
        let mut value = base();
        value["protocolVersion"] = json!(1);
        assert_eq!(parse(value).err(), Some("unsupported_version"));
        let mut value = base();
        value["ownerToken"] = json!("c".repeat(64));
        assert_eq!(parse(value).err(), Some("malformed_request"));
        let mut value = base();
        value["method"] = json!("inspect");
        assert_eq!(parse(value.clone()).err(), Some("malformed_request"));
        value["captureId"] = json!("c".repeat(32));
        value["ownerToken"] = json!("d".repeat(64));
        value["jobId"] = json!("e".repeat(32));
        value["requestSha256"] = json!("f".repeat(64));
        assert!(parse(value.clone()).is_ok());
        value["requestJson"] = json!("{}");
        assert_eq!(parse(value.clone()).err(), Some("malformed_request"));
        value["method"] = json!("derive");
        assert_eq!(parse(value.clone()).err(), Some("malformed_request"));
        value["requestSha256"] = json!(format!("{:x}", Sha256::digest(b"{}")));
        assert!(parse(value).is_ok());
    }
}
