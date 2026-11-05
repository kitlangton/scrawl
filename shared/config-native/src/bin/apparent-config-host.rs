use apparent_config_native::{compare_and_swap_stream_state_in, read_stream_state_in, CasResult};
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
    (id, match result { Ok(result) => result, Err(_) => unknown("operation failed") })
}

fn unknown(message: &str) -> Value { json!({"status":"unknown","message":message}) }
fn valid_owner(s: &str) -> bool { !s.is_empty() && s.len() <= 64 && s.as_bytes()[0].is_ascii_lowercase() && s.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-') }
