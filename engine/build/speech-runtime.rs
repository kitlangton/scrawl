use std::{fs, path::PathBuf};

/// Stage all transcribe.cpp runtime and dlopen-only CPU modules beside both
/// release binaries and Cargo test executables. This does not load a model.
pub fn stage() {
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"));
    let profile = out.ancestors().nth(3).expect("unexpected OUT_DIR layout");
    let mut staged = Vec::new();
    for key in [
        "DEP_TRANSCRIBE_CPP_RUNTIME_DIR",
        "DEP_TRANSCRIBE_CPP_MODULE_DIR",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
        let source =
            PathBuf::from(std::env::var_os(key).expect("transcribe.cpp dynamic runtime missing"));
        let mut files = fs::read_dir(&source)
            .expect("read transcribe runtime")
            .map(|entry| entry.expect("read runtime entry").path())
            .filter(|p| {
                p.is_file()
                    && p.file_name()
                        .is_some_and(|n| n.to_string_lossy().contains(".so"))
            })
            .collect::<Vec<_>>();
        files.sort();
        assert!(
            !files.is_empty(),
            "no shared objects in {}",
            source.display()
        );
        for file in files {
            let name = file.file_name().expect("runtime filename");
            for dir in [profile.to_path_buf(), profile.join("deps")] {
                fs::create_dir_all(&dir).expect("create staged runtime directory");
                fs::copy(&file, dir.join(name)).expect("stage transcribe runtime object");
            }
            staged.push(name.to_string_lossy().into_owned());
        }
    }
    staged.sort();
    staged.dedup();
    println!("cargo:rustc-link-arg=-Wl,-rpath,$ORIGIN");
    let receipt = profile.join("apparent-engine-speech-runtime.json");
    let quoted = staged
        .iter()
        .map(|s| format!("\"{s}\""))
        .collect::<Vec<_>>()
        .join(",");
    fs::write(
        receipt,
        format!("{{\"schema\":1,\"libraries\":[{quoted}]}}\n"),
    )
    .expect("write staged runtime receipt");
}
