use std::path::PathBuf;
use std::process::Command;

mod build_version { include!("enablement/build_version.rs"); }

fn main() {
    build_version::emit();
    println!("cargo:rerun-if-env-changed=APPARENT_POSTHOG_TOKEN");
    println!("cargo:rerun-if-env-changed=APPARENT_POSTHOG_RELEASE_ID");
    let release_id = std::env::var("APPARENT_POSTHOG_RELEASE_ID").unwrap_or_default();
    assert!(release_id.is_empty() || (release_id.len() == 36
        && release_id.bytes().enumerate().all(|(i, b)| {
            if [8, 13, 18, 23].contains(&i) { b == b'-' } else { b.is_ascii_hexdigit() }
        })), "APPARENT_POSTHOG_RELEASE_ID must be a PostHog release UUID");
    println!("cargo:rustc-env=APPARENT_POSTHOG_RELEASE_ID={release_id}");
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
    compile_mac_native();
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
        "capture/desktop/scripts/support/apparent-restore-clipboard.py",
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
        "capture/desktop/scripts/linux/gui/gui-vm-reaper.py",
        "capture/desktop/scripts/linux/gui/gui-vm.py",
        "capture/desktop/scripts/linux/maintenance/migrate-scrawl.py",
        "capture/desktop/scripts/linux/maintenance/publish-main-preview.py",
        "capture/desktop/scripts/linux/appimage/smoke-cpu.sh",
        "capture/desktop/scripts/linux/gui/test-capture-smoke.py",
        "capture/desktop/scripts/linux/gui/test-gui-vm.py",
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

/// TypeScript owns native build selection/reuse; xcrun remains the compiler.
fn compile_mac_native() {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let output = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let script = root.join("enablement/native-build/build.ts");
    println!("cargo:rerun-if-changed={}", script.display());
    let status = Command::new("node")
        .arg("--experimental-strip-types")
        .arg(script)
        .arg(&root)
        .arg(output)
        .arg(std::env::var("CARGO_CFG_TARGET_ARCH").expect("Mac target architecture"))
        .status()
        .expect("Node 22+ is required for Mac native build tooling");
    assert!(status.success(), "Mac helper/shader build failed");
}
