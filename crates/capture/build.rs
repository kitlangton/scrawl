use std::path::PathBuf;
use std::process::Command;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    if std::env::var_os("CARGO_FEATURE_HELPER").is_some() {
        compile_helper();
    }
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    if let Ok(output) = Command::new("xcode-select").arg("-p").output()
        && output.status.success()
        && let Ok(developer_dir) = String::from_utf8(output.stdout)
    {
        for runtime in ["swift-5.5/macosx", "swift/macosx"] {
            let swift_runtime = format!(
                "{}/Toolchains/XcodeDefault.xctoolchain/usr/lib/{runtime}",
                developer_dir.trim()
            );
            println!("cargo:rustc-link-arg=-Wl,-rpath,{swift_runtime}");
        }
    }
}

/// Builds `scrawl-helper` (Swift: SpeechAnalyzer, iPhone capture, movie decoding) beside the
/// binary, where `helper::path` and `scripts/install-app.sh` look for it.
fn compile_helper() {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let helper_dir = manifest_dir.join("helper");
    println!("cargo:rerun-if-changed={}", helper_dir.display());
    println!("cargo:rerun-if-env-changed=SCRAWL_HELPER_PLIST");
    println!("cargo:rerun-if-env-changed=SCRAWL_HELPER_BUNDLE_ID");
    println!("cargo:rerun-if-env-changed=SCRAWL_HELPER_NAME");
    let mut sources: Vec<PathBuf> = std::fs::read_dir(&helper_dir)
        .expect("helper/ is missing")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "swift"))
        .collect();
    sources.sort();
    for source in &sources {
        println!("cargo:rerun-if-changed={}", source.display());
    }
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let default_plist = helper_dir.join("Info.plist");
    println!("cargo:rerun-if-changed={}", default_plist.display());
    let plist = if let Some(custom_plist) = std::env::var_os("SCRAWL_HELPER_PLIST") {
        let p = PathBuf::from(custom_plist);
        println!("cargo:rerun-if-changed={}", p.display());
        p
    } else if std::env::var_os("SCRAWL_HELPER_BUNDLE_ID").is_some()
        || std::env::var_os("SCRAWL_HELPER_NAME").is_some()
    {
        let bundle_id = std::env::var("SCRAWL_HELPER_BUNDLE_ID")
            .unwrap_or_else(|_| "dev.kitlangton.scrawl.helper".to_owned());
        let name =
            std::env::var("SCRAWL_HELPER_NAME").unwrap_or_else(|_| "Scrawl Helper".to_owned());
        let template = std::fs::read_to_string(&default_plist).expect("read helper Info.plist");
        let rendered = template
            .replace("dev.kitlangton.scrawl.helper", &bundle_id)
            .replace("Scrawl Helper", &name);
        let generated = out_dir.join("scrawl-helper-Info.plist");
        std::fs::write(&generated, rendered).expect("write generated helper Info.plist");
        generated
    } else {
        default_plist
    };

    let profile_dir = out_dir
        .ancestors()
        .nth(3)
        .expect("Unexpected OUT_DIR layout");
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok(other) => other,
        Err(_) => "arm64",
    }
    .to_owned();
    let built = out_dir.join("scrawl-helper");
    let status = Command::new("xcrun")
        .args(["swiftc", "-O", "-swift-version", "5", "-target"])
        .arg(format!("{arch}-apple-macos15.0"))
        .args(&sources)
        .arg("-o")
        .arg(&built)
        .args([
            "-Xlinker",
            "-sectcreate",
            "-Xlinker",
            "__TEXT",
            "-Xlinker",
            "__info_plist",
            "-Xlinker",
        ])
        .arg(&plist)
        .status()
        .expect("xcrun swiftc is required to build scrawl-helper");
    assert!(
        status.success(),
        "scrawl-helper failed to compile (needs the macOS 26 SDK; the result runs on macOS 15)"
    );
    let dest = profile_dir.join("scrawl-helper");
    let _ = std::fs::remove_file(&dest);
    std::fs::copy(&built, dest).expect("Could not place scrawl-helper");
}
