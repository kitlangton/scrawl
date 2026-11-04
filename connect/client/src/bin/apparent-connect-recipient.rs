use apparent_connect_client::{Client, config_provider::ProductionConfig, recipient_protocol::{Request, Operation}};
use serde_json::{Value, json};
use std::{io::{Read, Write}, sync::{Arc, atomic::{AtomicBool, Ordering}}};
static CANCELLED: AtomicBool = AtomicBool::new(false);
extern "C" fn cancel(_: i32) { CANCELLED.store(true, Ordering::Relaxed); }
fn emit(value: Value) { println!("{value}"); let _ = std::io::stdout().flush(); }
fn run() -> anyhow::Result<Value> {
    let mut input = String::new();
    std::io::stdin().take(65537).read_to_string(&mut input)?;
    anyhow::ensure!(input.len() <= 65536, "Request too large");
    let request: Request = serde_json::from_str(&input)?;
    request.validate()?;
    let path = std::env::var_os("APPARENT_CONFIG").map(Into::into).unwrap_or_else(|| std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config/apparent/config.json"));
    let client = Client::new(Arc::new(ProductionConfig(path)));
    let allowed = || !CANCELLED.load(Ordering::Relaxed);
    match request.operation {
        Operation::Status { account } => client.status(account),
        Operation::Login { server, browser } => {
            let server = client.login_server(server.as_deref())?;
            client.login_presented(&server, "read", false, &allowed, &|a| {
                emit(json!({"protocol":1,"event":"approval","verificationUri":a.verification_uri,"userCode":a.user_code}));
                if browser { let _ = std::process::Command::new(if cfg!(target_os="macos") {"open"} else {"xdg-open"}).arg(a.verification_uri).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn(); }
            }, false)?;
            Ok(json!({"scope":"read"}))
        },
        Operation::Fetch { id, out, wait, evidence_only } => {
            Ok(json!({"path": client.fetch_recipient(&id, &out, wait, evidence_only, &allowed)?}))
        }
    }
}
fn main() {
    unsafe { libc::signal(libc::SIGINT, cancel as *const () as libc::sighandler_t); libc::signal(libc::SIGTERM, cancel as *const () as libc::sighandler_t); }
    match run() {
        Ok(result) => emit(json!({"protocol":1,"result":result})),
        Err(error) => { emit(json!({"protocol":1,"error":{"code":if CANCELLED.load(Ordering::Relaxed) {"cancelled"} else {"operation_failed"},"message":error.to_string()}})); std::process::exit(1); }
    }
}
