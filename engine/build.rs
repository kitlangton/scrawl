#[path = "build/speech-runtime.rs"]
mod speech_runtime;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux")
        && std::env::var_os("CARGO_FEATURE_CAPTURE").is_some()
    {
        speech_runtime::stage();
    }
}
