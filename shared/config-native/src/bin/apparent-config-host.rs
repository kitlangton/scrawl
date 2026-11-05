use apparent_config_native::{compare_and_swap_stream_state_in, read_stream_state_in, CasResult, ConfigPublicationUncertain};
use serde_json::{json, Value};
use std::{io::{self, BufRead, Read, Write}, path::PathBuf};
const LIMIT: usize = 64 * 1024;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 4 || args[0] != "--config" || args[2] != "--owner" { std::process::exit(2); }
    let path = PathBuf::from(&args[1]);
    let owner = &args[3];
    if !path.is_absolute() || !valid_owner(owner) { std::process::exit(2); }
    let stdin = io::stdin();
    let mut stdout = io::BufWriter::new(io::stdout().lock());
    let mut reader = stdin.lock();
    loop {
        let mut line = Vec::with_capacity(LIMIT);
        let read = match reader.by_ref().take((LIMIT + 1) as u64).read_until(b'\n', &mut line) { Ok(n) => n, Err(_) => break };
        if read == 0 { break; }
        if line.len() > LIMIT || line.last() != Some(&b'\n') { break; }
        line.pop();
        let (id, mut result) = match serde_json::from_slice::<Value>(&line) {
            Ok(request) => dispatch(&request, &path, owner),
            Err(_) => (String::new(), unknown("invalid request")),
        };
        let response = json!({"protocol":1,"id":id,"result":result});
        if serde_json::to_vec(&response).map_or(true, |bytes| bytes.len() + 1 > LIMIT) {
            result = unknown("response exceeds size limit");
        }
        let response = json!({"protocol":1,"id":id,"result":result});
        if serde_json::to_writer(&mut stdout, &response).is_err() || stdout.write_all(b"\n").is_err() || stdout.flush().is_err() { break; }
    }
}

fn dispatch(req: &Value, path: &PathBuf, owner: &str) -> (String, Value) {
    let Some(object) = req.as_object() else { return (String::new(), unknown("invalid request")); };
    let id = object.get("id").and_then(Value::as_str).filter(|id| id.len() <= 128).unwrap_or("").to_owned();
    let valid_keys = object.keys().all(|k| ["protocol", "id", "operation", "expected", "value"].contains(&k.as_str()));
    if !valid_keys || object.get("protocol").and_then(Value::as_u64) != Some(1) || id.is_empty() { return (id, unknown("invalid request")); }
    let result = match object.get("operation").and_then(Value::as_str) {
        Some("read") if object.len() == 3 => read_stream_state_in(path, owner).map(|(revision, value)| json!({"revision":revision,"value":value})),
        Some("compareAndSwap") if object.len() == 5 => {
            let expected = object.get("expected").and_then(Value::as_u64);
            let value = object.get("value");
            match (expected, value) {
                (Some(expected), Some(value)) => compare_and_swap_stream_state_in(path, owner, expected, value.clone()).map(|v| match v {
                    CasResult::Saved { revision } => json!({"status":"saved","revision":revision}),
                    CasResult::Conflict { revision, current } => json!({"status":"conflict","revision":revision,"current":current}),
                }),
                _ => return (id, unknown("invalid request")),
            }
        }
        _ => return (id, unknown("invalid request")),
    };
    (id, match result { Ok(result) => result, Err(error) => operation_failure(&error) })
}

// Only fixed categories and numeric errno values cross this boundary. Never
// expose error Display text: it may contain paths, settings or credentials.
fn operation_failure(error: &anyhow::Error) -> Value {
    if error.downcast_ref::<ConfigPublicationUncertain>().is_some() {
        return unknown("operation failed: publication-unconfirmed");
    }
    if error.downcast_ref::<serde_json::Error>().is_some() {
        return unknown("operation failed: stored-json-invalid");
    }
    const INVARIANTS: &[(&str, &str)] = &[
        ("Config ancestry must be trusted or sticky", "unsafe-ancestry"),
        ("Config directory must be owned and not writable by other users", "unsafe-directory"),
        ("Config lock must be an owned private regular file", "unsafe-lock"),
        ("Config lock changed while waiting", "lock-replaced"),
        ("Config must be a regular file", "config-not-regular"),
        ("streamState must be an object", "invalid-owner-map"),
        ("Invalid streamState entry", "invalid-owner-entry"),
        ("Invalid streamState revision", "invalid-owner-revision"),
        ("revision limit reached", "revision-limit"),
    ];
    for cause in error.chain() {
        let text = cause.to_string();
        if let Some((_, category)) = INVARIANTS.iter().find(|(message, _)| text == *message) {
            return unknown(&format!("operation failed: {category}"));
        }
    }
    const PHASES: &[&str] = &[
        "config-parent-create", "config-parent-resolve", "config-root-open",
        "config-ancestor-open", "config-ancestor-stat", "config-directory-stat",
        "config-lock-open", "config-lock-stat", "config-lock-acquire",
        "config-lock-reopen", "config-lock-restat", "config-snapshot-open",
        "config-snapshot-stat", "config-snapshot-read", "config-random-open",
        "config-random-read", "config-temp-open", "config-temp-write",
        "config-temp-sync", "config-rename", "config-directory-sync",
    ];
    let phase = error.chain().find_map(|cause| {
        PHASES.iter().copied().find(|phase| cause.to_string() == *phase)
    });
    if let Some(error) = error.downcast_ref::<io::Error>() {
        if let Some(phase) = phase {
            return unknown(&format!("operation failed: {phase}:io-{}", error.raw_os_error().unwrap_or(0)));
        }
        return unknown(&format!("operation failed: io-{}", error.raw_os_error().unwrap_or(0)));
    }
    unknown("operation failed: unclassified")
}

fn unknown(message: &str) -> Value { json!({"status":"unknown","message":message}) }
fn valid_owner(s: &str) -> bool { !s.is_empty() && s.len() <= 64 && s.as_bytes()[0].is_ascii_lowercase() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn storage_diagnostics_never_expose_error_context() {
        let private = "synthetic-secret /private/synthetic-config.json";
        let cases = [
            (anyhow::anyhow!(private), "unclassified"),
            (anyhow::Error::new(serde_json::from_str::<Value>("synthetic-secret").unwrap_err()).context(private), "stored-json-invalid"),
            (anyhow::Error::new(io::Error::other(private)), "io-0"),
            (anyhow::anyhow!("Config lock changed while waiting").context(private), "lock-replaced"),
            (anyhow::Error::new(io::Error::from_raw_os_error(libc::EACCES)).context(private), "io-13"),
            (anyhow::Error::new(io::Error::from_raw_os_error(libc::EIO)).context(private).context("config-directory-sync"), "config-directory-sync:io-5"),
            (anyhow::Error::new(ConfigPublicationUncertain).context(private), "publication-unconfirmed"),
        ];
        for (error, category) in cases {
            let response = operation_failure(&error);
            assert_eq!(response, unknown(&format!("operation failed: {category}")));
            assert!(!response.to_string().contains(private));
        }
    }
}
