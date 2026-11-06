//! Independent headless media worker. Capture-v1 and live-capture admission are separate.
#[path = "media/files.rs"]
mod files;
#[path = "media/job.rs"]
mod job;
#[cfg(target_os = "linux")]
#[path = "media/linux.rs"]
mod linux;
#[cfg(target_os = "macos")]
#[path = "media/macos.rs"]
mod macos;
#[path = "media/protocol.rs"]
mod protocol;
#[path = "media/service.rs"]
mod service;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = if args == ["--decode-pinned-job"] {
        job::child()
    } else if args.len() == 6
        && args[0] == "--media-runtime-directory"
        && args[2] == "--media-channel"
        && args[3] == "media"
        && args[4] == "--media-protocol-version"
        && args[5] == "1"
    {
        service::run(std::path::Path::new(&args[1]))
    } else {
        Err("malformed_request")
    };
    if result.is_err() {
        eprintln!("Engine media worker stopped");
        std::process::exit(1);
    }
}
