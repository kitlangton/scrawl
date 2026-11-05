use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::path::{Component, Path};

pub type Failure = &'static str;
pub type Result<T> = std::result::Result<T, Failure>;
const SAFE: u64 = 9_007_199_254_740_991;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Identity {
    pub device: String,
    pub inode: String,
    pub size: u64,
    pub modified_ns: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Source {
    pub bundle_id: String,
    pub bundle_path: String,
    pub video_path: String,
    pub video_offset_ms: u64,
    pub identity: Identity,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Limits {
    pub max_frames: u64,
    pub max_edge: u32,
    pub max_total_pixels: u64,
    pub deadline_ms: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Decode {
    pub job_id: String,
    pub source: Source,
    pub requested_bundle_ms: Vec<u64>,
    pub output_directory: String,
    pub limits: Limits,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Frame {
    pub request_index: usize,
    pub requested_bundle_ms: u64,
    pub effective_video_ms: u64,
    pub actual_video_ms: u64,
    pub clamped_to_start: bool,
    pub clamped_to_end: bool,
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub byte_length: u64,
    pub sha256: String,
}
pub fn hex(s: &str) -> bool {
    s.len() == 32
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
pub fn path(s: &str) -> bool {
    let p = Path::new(s);
    p.is_absolute()
        && s.len() <= 4096
        && !s.contains('\0')
        && p.components()
            .all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
        && p.to_str() == Some(s)
        && !s.contains("//")
        && !s.ends_with('/')
        && !s.split('/').any(|s| s == "." || s == "..")
}
fn digits(s: &str) -> bool {
    !s.is_empty() && s.len() <= 32 && s.bytes().all(|c| c.is_ascii_digit())
}
impl Decode {
    pub fn validate(&self) -> Result<()> {
        let s = &self.source;
        let l = &self.limits;
        if !hex(&self.job_id)
            || !path(&s.bundle_path)
            || !path(&s.video_path)
            || !path(&self.output_directory)
            || s.bundle_id.is_empty()
            || s.bundle_id.len() > 255
            || matches!(s.bundle_id.as_str(), "." | "..")
            || s.bundle_id
                .chars()
                .any(|c| c.is_control() || c == '/' || c == '\\')
            || !Path::new(&s.video_path).starts_with(&s.bundle_path)
            || s.video_path == s.bundle_path
            || Path::new(&self.output_directory).starts_with(&s.bundle_path)
            || s.video_offset_ms > SAFE
            || !digits(&s.identity.device)
            || !digits(&s.identity.inode)
            || !digits(&s.identity.modified_ns)
            || s.identity.size == 0
            || s.identity.size > SAFE
            || self.requested_bundle_ms.is_empty()
            || self.requested_bundle_ms.len() > 48
            || self.requested_bundle_ms.iter().any(|t| *t > SAFE)
        {
            return Err("malformed_request");
        }
        if !(1..=48).contains(&l.max_frames)
            || self.requested_bundle_ms.len() as u64 > l.max_frames
            || !(1..=4096).contains(&l.max_edge)
            || !(1..=64_000_000).contains(&l.max_total_pixels)
            || !(1..=60_000).contains(&l.deadline_ms)
        {
            return Err("limits_exceeded");
        }
        // Every result must fit the same 64 KiB wire ceiling as a request.
        // Reserve full per-slot metadata before any filesystem/native effect.
        let path_bytes = serde_json::to_string(&self.output_directory)
            .map_err(|_| "malformed_request")?
            .len();
        if (path_bytes + 600) * self.requested_bundle_ms.len() + 1024 > 65536 {
            return Err("limits_exceeded");
        }
        Ok(())
    }
}
pub struct Request {
    pub rpc: String,
    pub client: String,
    pub method: String,
    pub job: Option<String>,
    pub decode: Option<Decode>,
}
pub fn parse(v: Value) -> Result<Request> {
    let o = v.as_object().ok_or("malformed_request")?;
    if o.get("channel").and_then(Value::as_str) != Some("media") {
        return Err("unsupported_channel");
    }
    if o.get("protocolVersion").and_then(Value::as_u64) != Some(1) {
        return Err("unsupported_version");
    }
    let string = |k| o.get(k).and_then(Value::as_str).ok_or("malformed_request");
    let rpc = string("rpcId")?.to_owned();
    let client = string("clientId")?.to_owned();
    if !hex(&rpc) || !hex(&client) {
        return Err("malformed_request");
    }
    let method = string("method")?.to_owned();
    let mut fields = vec!["channel", "protocolVersion", "clientId", "rpcId", "method"];
    match method.as_str() {
        "handshake" => (),
        "inspect" | "cancel" => fields.push("jobId"),
        "decode" => fields.extend([
            "jobId",
            "source",
            "requestedBundleMs",
            "outputDirectory",
            "limits",
        ]),
        _ => return Err("malformed_request"),
    }
    if o.len() != fields.len() || fields.iter().any(|k| !o.contains_key(*k)) {
        return Err("malformed_request");
    }
    let job = if method != "handshake" {
        let j = string("jobId")?;
        if !hex(j) {
            return Err("malformed_request");
        }
        Some(j.to_owned())
    } else {
        None
    };
    let decode = if method == "decode" {
        let mut body = o.clone();
        for k in ["channel", "protocolVersion", "clientId", "rpcId", "method"] {
            body.remove(k);
        }
        let d: Decode =
            serde_json::from_value(Value::Object(body)).map_err(|_| "malformed_request")?;
        d.validate()?;
        Some(d)
    } else {
        None
    };
    Ok(Request {
        rpc,
        client,
        method,
        job,
        decode,
    })
}
pub fn error(code: &str) -> Value {
    json!({"code":code,"message":"Engine media request failed"})
}
pub fn failed(job: &str, code: &str) -> Value {
    json!({"jobId":job,"state":if code == "cancelled" {"cancelled"} else {"failed"},"error":error(code),"completedCount":0})
}
pub fn response(rpc: &str, result: Result<Value>) -> Value {
    let mut r = json!({"channel":"media","protocolVersion":1,"rpcId":rpc});
    match result {
        Ok(v) => r["result"] = v,
        Err(e) => r["error"] = error(e),
    }
    r
}
