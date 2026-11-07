use std::path::PathBuf;
use std::process::Command;

mod build_version { include!("enablement/build_version.rs"); }
mod build_cache { include!("enablement/native/build_cache.rs"); }

fn main() {
    build_version::emit();
    println!("cargo:rerun-if-env-changed=APPARENT_POSTHOG_TOKEN");
    watch_desktop_tooling();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        stage_transcribe_shared_objects();
        // cargo binaries and tests resolve the staged libraries beside themselves.
        // linuxdeploy rewrites this to the AppImage's usr/lib when packaging.
        println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
    }
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_windows_resources();
        stage_transcribe_dlls();
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    compile_helper();
    compile_laser_shader();
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

/// Keep runtime libraries AND dlopen-only CPU modules together: transcribe.cpp's
/// init_backends_default uses dladdr to search next to the loaded libtranscribe.
fn stage_transcribe_shared_objects() {
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let profile = out_dir
        .ancestors()
        .nth(3)
        .expect("Unexpected OUT_DIR layout");
    for key in [
        "DEP_TRANSCRIBE_CPP_RUNTIME_DIR",
        "DEP_TRANSCRIBE_CPP_MODULE_DIR",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
        let source =
            PathBuf::from(std::env::var_os(key).expect("dynamic transcribe runtime missing"));
        let mut count = 0;
        for entry in std::fs::read_dir(&source).expect("read transcribe runtime") {
            let path = entry.expect("read transcribe library").path();
            let name = path.file_name().unwrap();
            if !name.to_string_lossy().contains(".so") || !path.is_file() {
                continue;
            }
            count += 1;
            for dir in [profile.to_path_buf(), profile.join("deps")] {
                std::fs::create_dir_all(&dir).expect("create runtime directory");
                std::fs::copy(&path, dir.join(name)).expect("stage transcribe library");
            }
        }
        assert!(count > 0, "No shared libraries in {}", source.display());
    }
}

/// transcribe.cpp's DLLs (`dynamic-backends`: `transcribe.dll`, `ggml*.dll`) beside `apparent.exe`
/// and the test binaries, which load them at start. Its own build script copies them there too,
/// but only when it runs, and a build restored from CI's cache skips it.
#[cfg(windows)]
fn stage_transcribe_dlls() {
    println!("cargo:rerun-if-env-changed=DEP_TRANSCRIBE_CPP_RUNTIME_DIR");
    let Some(runtime) = std::env::var_os("DEP_TRANSCRIBE_CPP_RUNTIME_DIR") else {
        return;
    };
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    // OUT_DIR is <target>/<profile>/build/apparent-<hash>/out.
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
    let icon = manifest_dir.join("capture/desktop/assets/Apparent.ico");
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
      VALUE "FileDescription", "Apparent"
      VALUE "ProductName", "Apparent"
      VALUE "FileVersion", "{version}"
      VALUE "ProductVersion", "{version}"
      VALUE "OriginalFilename", "apparent.exe"
      VALUE "InternalName", "apparent"
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
    let resource = output_dir.join("apparent.rc");
    std::fs::write(&resource, script).expect("write the Windows resource script");
    embed_resource::compile(&resource, embed_resource::NONE)
        .manifest_optional()
        .expect("compile the Windows resources");
}

/// Track Desktop-owned build/runtime tools and packaging inputs after their physical move.
fn watch_desktop_tooling() {
    for path in [
        "capture/desktop/scripts/packaging/Apparent.entitlements",
        "capture/desktop/scripts/support/bench-start.ts",
        "capture/desktop/scripts/packaging/build-mac-preview.sh",
        "capture/desktop/scripts/support/check-ca-roots.sh",
        "capture/desktop/scripts/support/check-permission-panel-dock.sh",
        "capture/desktop/scripts/packaging/install-app-remote.sh",
        "capture/desktop/scripts/packaging/install-app.sh",
        "capture/desktop/scripts/linux/appimage/appimage.sh",
        "capture/desktop/scripts/linux/maintenance/build-playback.sh",
        "capture/desktop/scripts/linux/gui/capture-smoke.py",
        "capture/desktop/scripts/linux/appimage/check-appimage-decoder.sh",
        "capture/desktop/scripts/linux/appimage/check-appimage-encoder.sh",
        "capture/desktop/scripts/linux/appimage/check-cpu-tiers.py",
        "capture/desktop/scripts/linux/appimage/dependencies.sh",
        "enablement/native/linux/gui-vm-reaper.py",
        "enablement/native/linux/gui-vm.py",
        "capture/desktop/scripts/linux/maintenance/migrate-scrawl.py",
        "capture/desktop/scripts/linux/maintenance/publish-main-preview.py",
        "capture/desktop/scripts/linux/appimage/smoke-cpu.sh",
        "capture/desktop/scripts/linux/gui/test-capture-smoke.py",
        "enablement/native/linux/test-gui-vm.py",
        "capture/desktop/scripts/linux/maintenance/test-native-launch.sh",
        "capture/desktop/scripts/linux/updater/test-updater-gui.py",
        "capture/desktop/scripts/linux/maintenance/test_migrate_scrawl.py",
        "capture/desktop/scripts/linux/updater/updater-fixture.py",
        "capture/desktop/scripts/linux/updater/updater-gui-scenario.sh",
        "capture/desktop/scripts/linux/updater/updater-gui.py",
        "capture/desktop/scripts/release/prepare-mac-signing.sh",
        "capture/desktop/scripts/release/preview_appcast.py",
        "capture/desktop/scripts/release/release-app.sh",
        "capture/desktop/scripts/release/setup-sparkle.sh",
        "capture/desktop/scripts/tests/local_preview_update.py",
        "capture/desktop/scripts/tests/test_export_audio.py",
        "capture/desktop/scripts/tests/test_launch_registration.py",
        "capture/desktop/scripts/tests/test_linux_preview_publication.py",
        "capture/desktop/scripts/tests/test_preview_appcast.py",
        "capture/desktop/scripts/tests/test_preview_apple_signing.py",
        "capture/desktop/scripts/tests/test_preview_release.py",
        "capture/desktop/scripts/tests/test_preview_sparkle_setup.py",
        "capture/desktop/scripts/release/verify-preview-sparkle-key.swift",
        "capture/desktop/scripts/support/virtual-display.swift",
        "capture/desktop/packaging/linux/dev.kitlangton.apparent.desktop",
        "capture/desktop/packaging/linux/dev.kitlangton.apparent.png",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
}

fn compile_laser_shader() {
    for (src_rel, name) in [
        ("capture/desktop/native/laser_overlay.metal", "laser_overlay"),
        ("capture/desktop/native/pill/pill.metal", "pill"),
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

    let mut metal = Command::new("xcrun");
    metal.args(["--sdk", "macosx", "metal", "-c", "-mmacosx-version-min=15.0"])
        .arg(&source).arg("-o").arg(&air);
    let mut metallib = Command::new("xcrun");
    metallib.args(["--sdk", "macosx", "metallib"])
        .arg(&air).arg("-o").arg(&library);
    let key = native_key(&[("source", &source)]).and_then(|mut key| {
        add_command(&mut key, &metal);
        add_command(&mut key, &metallib);
        push_field(&mut key, b"metal", &build_cache::tool_identity("metal", &["--version"])?);
        push_field(&mut key, b"metallib", &build_cache::tool_identity("metallib", &[])?);
        Some(key)
    });
    let cache = output_dir.join(".native-cache").join(name);
    let outputs = [("shader.air", air.clone(), false), ("shader.metallib", library.clone(), false)];
    let mut compile = || -> std::io::Result<()> {
        if !metal.status()?.success() {
            return Err(std::io::Error::other(format!("Metal shader compilation failed for {src_rel}")));
        }
        if !metallib.status()?.success() {
            return Err(std::io::Error::other(format!("Metal library creation failed for {src_rel}")));
        }
        Ok(())
    };
    let cached = build_cache::timed(name, || match key {
        Some(key) => build_cache::reuse_or_build(&cache, &key, &outputs, compile).expect("Metal shader build/cache failed"),
        None => { let _ = std::fs::remove_file(cache.join("receipt")); compile().expect("Metal shader compilation failed; xcrun metal is required"); false }
    });
    println!("cargo:warning=native build {name} cache: {}", if cached { "reused" } else { "compiled" });
}

fn push_field(record: &mut Vec<u8>, name: &[u8], value: &[u8]) { build_cache::add_field(record, name, value); }

fn native_key(files: &[(&str, &std::path::Path)]) -> Option<Vec<u8>> {
    let mut key = build_cache::recipe_identity();
    build_cache::add_compiler_env(&mut key);
    for (label, path) in files { if build_cache::add_file(&mut key, label, path).is_err() { return None; } }
    if let Some(identity) = build_cache::apple_toolchain_identity() { push_field(&mut key, b"apple-toolchain", &identity); } else { return None; }
    Some(key)
}

fn add_command(key: &mut Vec<u8>, command: &Command) {
    build_cache::add_field(key, b"command", &command_args_record(command));
}

fn command_args_record(command: &Command) -> Vec<u8> {
    let mut argv = Vec::new();
    for arg in std::iter::once(command.get_program()).chain(command.get_args()) {
        build_cache::add_field(&mut argv, b"arg", arg.as_encoded_bytes());
    }
    argv
}

/// Builds `apparent-helper` (Swift: SpeechAnalyzer, iPhone capture, movie decoding) beside the
/// `apparent` binary, where `helper::path` and `scripts/install-app.sh` look for it.
fn compile_helper() {
    let manifest_dir = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    // Keep owner-native input lists explicit: new files must not silently enter the helper build.
    // Sort the merged list by basename to preserve the former helper/*.swift compiler order.
    let owner_sources: &[&[&str]] = &[
        &[
            "engine/native/Export.swift",
            "engine/native/LiveTranscribe.swift",
            "engine/native/Media.swift",
            "capture/engine/native/Mirror.swift",
            "engine/native/PlaybackVolume.swift",
            "engine/native/ScreenProbe.swift",
        ],
        &[
            "capture/desktop/native/Onboarding.swift",
            "capture/desktop/native/OnboardingDemo.swift",
            "capture/desktop/native/PermissionPanel.swift",
            "capture/desktop/native/Settings.swift",
            "capture/desktop/native/ShortcutKeys.swift",
        ],
        &["shared/native/main.swift"],
    ];
    let mut sources: Vec<PathBuf> = owner_sources
        .iter()
        .flat_map(|group| group.iter())
        .map(|source| manifest_dir.join(source))
        .collect();
    sources.sort_by(|left, right| left.file_name().cmp(&right.file_name()));
    for source in &sources {
        println!("cargo:rerun-if-changed={}", source.display());
    }
    let original_plist = manifest_dir.join("shared/native/Info.plist");
    println!("cargo:rerun-if-changed={}", original_plist.display());
    println!("cargo:rerun-if-env-changed=APPARENT_HELPER_BUNDLE_ID");
    println!("cargo:rerun-if-env-changed=APPARENT_PREVIEW_BUILD");
    // OUT_DIR is target/<profile>/build/apparent-<hash>/out.
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let plist = if let Ok(bundle_id) = std::env::var("APPARENT_HELPER_BUNDLE_ID") {
        assert_eq!(bundle_id, "dev.simonvanlaak.apparent.preview.helper");
        let source = std::fs::read_to_string(&original_plist).expect("read helper plist");
        let original = "<string>dev.kitlangton.apparent.helper</string>";
        assert!(source.contains(original), "helper bundle ID changed");
        let preview_plist = out_dir.join("preview-helper-Info.plist");
        std::fs::write(
            &preview_plist,
            source.replace(original, &format!("<string>{bundle_id}</string>")),
        )
        .expect("write preview helper plist");
        preview_plist
    } else {
        original_plist.clone()
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
    let built = out_dir.join("apparent-helper");
    let mut command = Command::new("xcrun");
    command
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
        .arg(&plist);
    let command_args = command_args_record(&command);
    let key_inputs: Vec<_> = sources.iter().map(|p| ("swift-source", p.as_path()))
        .chain([("original-plist", original_plist.as_path()), ("effective-plist", plist.as_path())])
        .collect();
    let identity = move || native_key(&key_inputs).and_then(|mut key| {
            // Identical field bytes/order as add_command, prepared before the
            // compile closure takes the mutable command borrow.
            build_cache::add_field(&mut key, b"command", &command_args);
            push_field(&mut key, b"swiftc", &build_cache::tool_identity("swiftc", &["--version"])?);
            Some(key)
        });
    let compile = || -> std::io::Result<()> {
        if !build_cache::timed("swift compiler execution", || command.status())?.success() {
            return Err(std::io::Error::other("apparent-helper failed to compile (needs the macOS 26 SDK; the result runs on macOS 15)"));
        }
        Ok(())
    };
    let cache = out_dir.join(".native-cache").join("swift-helper");
    let outputs = [("apparent-helper", built.clone(), true)];
    let cached = build_cache::timed("swift-helper", || {
        build_cache::cold_reuse_or_build(&cache, &outputs, identity, compile)
            .expect("Swift helper build/cache failed")
    });
    println!("cargo:warning=native build swift-helper cache: {}", if cached { "reused" } else { "compiled" });
    let dest = profile_dir.join("apparent-helper");
    let _ = std::fs::remove_file(&dest);
    std::fs::copy(&built, dest).expect("Could not place apparent-helper");
}
