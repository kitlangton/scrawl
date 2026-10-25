use std::path::PathBuf;
use std::process::Command;

fn main() {
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_windows_resources();
        stage_transcribe_dlls();
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    compile_laser_shader();
    compile_helper();
    compile_ui_helper();
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

/// transcribe.cpp's DLLs (`dynamic-backends`: `transcribe.dll`, `ggml*.dll`) beside `scrawl.exe`
/// and the test binaries, which load them at start. Its own build script copies them there too,
/// but only when it runs, and a build restored from CI's cache skips it.
#[cfg(windows)]
fn stage_transcribe_dlls() {
    println!("cargo:rerun-if-env-changed=DEP_TRANSCRIBE_CPP_RUNTIME_DIR");
    let Some(runtime) = std::env::var_os("DEP_TRANSCRIBE_CPP_RUNTIME_DIR") else {
        return;
    };
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    // OUT_DIR is <target>/<profile>/build/scrawl-<hash>/out.
    let Some(profile) = out_dir.ancestors().nth(3) else {
        return;
    };
    let dlls: Vec<PathBuf> = std::fs::read_dir(&runtime)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "dll"))
        .collect();
    for dir in [profile.to_path_buf(), profile.join("deps")] {
        let _ = std::fs::create_dir_all(&dir);
        for dll in &dlls {
            if let Err(error) = std::fs::copy(dll, dir.join(dll.file_name().unwrap())) {
                println!("cargo:warning=could not copy {}: {error}", dll.display());
            }
        }
    }
}

/// The app icon (Explorer, the taskbar, Alt+Tab, and each window's title bar icon) and version
/// info. GPUI's `windows-manifest` feature embeds the DPI-aware manifest, so none is added here.
#[cfg(windows)]
fn embed_windows_resources() {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let output_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let icon = manifest_dir.join("assets/Scrawl.ico");
    println!("cargo:rerun-if-changed={}", icon.display());
    let version = std::env::var("CARGO_PKG_VERSION").unwrap();
    let numeric = version
        .split(['.', '-'])
        .take(3)
        .map(|part| part.parse::<u16>().unwrap_or(0).to_string())
        .chain(std::iter::repeat("0".to_owned()))
        .take(4)
        .collect::<Vec<_>>()
        .join(",");
    let script = format!(
        r#"#include <winver.h>
1 ICON "{icon}"
1 VERSIONINFO
FILEVERSION {numeric}
PRODUCTVERSION {numeric}
FILEOS VOS_NT_WINDOWS32
FILETYPE VFT_APP
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "FileDescription", "Scrawl"
      VALUE "ProductName", "Scrawl"
      VALUE "FileVersion", "{version}"
      VALUE "ProductVersion", "{version}"
      VALUE "OriginalFilename", "scrawl.exe"
      VALUE "InternalName", "scrawl"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#,
        icon = icon.display().to_string().replace('\\', "/")
    );
    let resource = output_dir.join("scrawl.rc");
    std::fs::write(&resource, script).expect("write the Windows resource script");
    embed_resource::compile(&resource, embed_resource::NONE)
        .manifest_optional()
        .expect("compile the Windows resources");
}

fn compile_laser_shader() {
    for (src_rel, name) in [
        ("src/laser_overlay.metal", "laser_overlay"),
        ("src/native/pill/pill.metal", "pill"),
    ] {
        compile_metal_shader(src_rel, name);
    }
}

fn compile_metal_shader(src_rel: &str, name: &str) {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let output_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let source = manifest_dir.join(src_rel);
    let air = output_dir.join(format!("{name}.air"));
    let library = output_dir.join(format!("{name}.metallib"));
    println!("cargo:rerun-if-changed={}", source.display());

    let metal = Command::new("xcrun")
        .args([
            "--sdk",
            "macosx",
            "metal",
            "-c",
            "-mmacosx-version-min=15.0",
        ])
        .arg(&source)
        .arg("-o")
        .arg(&air)
        .status()
        .expect("xcrun metal is required to build Metal shaders");
    assert!(
        metal.success(),
        "Metal shader compilation failed for {src_rel}"
    );

    let metallib = Command::new("xcrun")
        .args(["--sdk", "macosx", "metallib"])
        .arg(&air)
        .arg("-o")
        .arg(&library)
        .status()
        .expect("xcrun metallib is required to build Metal shaders");
    assert!(
        metallib.success(),
        "Metal library creation failed for {src_rel}"
    );
}

/// Builds `scrawl-helper` (Swift: SpeechAnalyzer, iPhone capture, movie decoding) beside the
/// `scrawl` binary, where `helper::path` and `scripts/install-app.sh` look for it.
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
    compile_swift_binary("scrawl-helper", &sources, &plist);
}

fn compile_ui_helper() {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let ui_dir = manifest_dir.join("helper-ui");
    let helper_dir = manifest_dir.join("helper");
    println!("cargo:rerun-if-changed={}", ui_dir.display());
    let mut sources: Vec<PathBuf> = std::fs::read_dir(&ui_dir)
        .expect("helper-ui/ is missing")
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|ext| ext == "swift"))
        .collect();
    for shared in ["Permissions.swift", "Util.swift"] {
        sources.push(helper_dir.join(shared));
    }
    sources.sort();
    for source in &sources {
        println!("cargo:rerun-if-changed={}", source.display());
    }
    let plist = ui_dir.join("Info.plist");
    println!("cargo:rerun-if-changed={}", plist.display());
    compile_swift_binary("scrawl-ui-helper", &sources, &plist);
}

fn compile_swift_binary(bin_name: &str, sources: &[PathBuf], plist: &std::path::Path) {
    // OUT_DIR is target/<profile>/build/scrawl-<hash>/out.
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
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
    let built = out_dir.join(bin_name);
    let status = Command::new("xcrun")
        .args(["swiftc", "-O", "-swift-version", "5", "-target"])
        .arg(format!("{arch}-apple-macos15.0"))
        .args(sources)
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
        .arg(plist)
        .status()
        .unwrap_or_else(|_| panic!("xcrun swiftc is required to build {bin_name}"));
    assert!(
        status.success(),
        "{bin_name} failed to compile (needs the macOS 26 SDK; the result runs on macOS 15)"
    );
    let dest = profile_dir.join(bin_name);
    let _ = std::fs::remove_file(&dest);
    std::fs::copy(&built, dest).unwrap_or_else(|_| panic!("Could not place {bin_name}"));
}
