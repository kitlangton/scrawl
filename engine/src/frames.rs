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
mod materials_media;
mod media2;
#[path = "materials_media/worker.rs"]
mod media2_worker;
#[path = "media/protocol.rs"]
mod protocol;
#[path = "recovery.rs"]
mod recovery;
mod retained;
#[path = "media/service.rs"]
mod service;
#[path = "media/transport.rs"]
mod transport;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = if args == ["--decode-pinned-job"] {
        job::child()
    } else if args == ["--media2-native-job"] {
        media2_worker::child()
    } else if (args.len() == 6 || args.len() == 8)
        && args[0] == "--media-runtime-directory"
        && args[2] == "--media-channel"
        && args[3] == "media"
        && args[4] == "--media-protocol-version"
    {
        let role = if args.len() == 8
            && args[6] == "--recovery-peer-v1"
            && recovery::nonce(&args[7])
            && args[5] == "2"
        {
            Some(recovery::Role {
                peer_id: &args[7],
                channel: "media/2",
            })
        } else if args.len() == 6 {
            None
        } else {
            std::process::exit(2)
        };
        match args[5].as_str() {
            "1" => service::run(std::path::Path::new(&args[1])),
            "2" => media2::run(std::path::Path::new(&args[1]), role),
            _ => Err("unsupported_version"),
        }
    } else {
        Err("malformed_request")
    };
    if result.is_err() {
        eprintln!("Engine media worker stopped");
        std::process::exit(1);
    }
}
