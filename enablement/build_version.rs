// Build identity for Settings. Resolve once at build time, never from the user's checkout.
use std::path::Path;
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> Option<String> {
    // An unpacked source archive may sit inside an unrelated checkout.
    // Do not let Git walk up and claim that parent's identity.
    if !root.join(".git").exists() {
        return None;
    }
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        // Reading status must not rewrite the index we watch for invalidation.
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args(args)
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

pub fn label(version: &str, revision: Option<&str>, modified: bool) -> String {
    let Some(revision) =
        revision.filter(|sha| sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()))
    else {
        return format!("{version} · build unknown");
    };
    format!(
        "{version} · {}{}",
        &revision[..7],
        if modified { " +modified" } else { "" }
    )
}

// Build inputs are public labels, never arbitrary CI/runtime environment text.
pub fn release_identity(
    release: Option<&str>,
    channel: Option<&str>,
    environment: Option<&str>,
) -> (String, &'static str, &'static str) {
    let release = release
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b".+-_".contains(&b))
        })
        .unwrap_or("unknown");
    let channel = match channel {
        Some("stable") => "stable",
        Some("preview") => "preview",
        _ => "development",
    };
    let environment = match environment {
        Some("production") => "production",
        Some("synthetic") => "synthetic",
        _ => "development",
    };
    (release.to_owned(), channel, environment)
}

pub fn emit() {
    let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    for key in [
        "APPARENT_RELEASE_VERSION",
        "APPARENT_RELEASE_CHANNEL",
        "APPARENT_REPORTING_ENVIRONMENT",
    ] {
        println!("cargo:rerun-if-env-changed={key}");
    }
    // Watch the real Git paths, including linked worktrees and packed refs.
    for name in ["HEAD", "index", "packed-refs"] {
        if let Some(path) = git(&root, &["rev-parse", "--git-path", name])
            && root.join(&path).exists()
        {
            println!("cargo:rerun-if-changed={}", root.join(path).display());
        }
    }
    if let Some(reference) = git(&root, &["symbolic-ref", "-q", "HEAD"])
        && let Some(path) = git(&root, &["rev-parse", "--git-path", &reference])
        && root.join(&path).exists()
    {
        println!("cargo:rerun-if-changed={}", root.join(path).display());
    }
    // A tracked edit must invalidate the cached identity even without a commit.
    if let Some(files) = git(&root, &["ls-files", "-z"]) {
        for file in files.split('\0').filter(|file| !file.is_empty()) {
            println!("cargo:rerun-if-changed={}", root.join(file).display());
        }
    }
    let revision = git(&root, &["rev-parse", "HEAD"]).or_else(|| std::env::var("GITHUB_SHA").ok());
    let modified = git(&root, &["status", "--porcelain", "--untracked-files=no"])
        .is_some_and(|status| !status.is_empty());
    let sha = revision
        .as_deref()
        .filter(|sha| sha.len() == 40 && sha.bytes().all(|b| b.is_ascii_hexdigit()))
        .unwrap_or("unknown");
    let (release, channel, environment) = release_identity(
        std::env::var("APPARENT_RELEASE_VERSION").ok().as_deref(),
        std::env::var("APPARENT_RELEASE_CHANNEL").ok().as_deref(),
        std::env::var("APPARENT_REPORTING_ENVIRONMENT")
            .ok()
            .as_deref(),
    );
    println!("cargo:rustc-env=APPARENT_COMMIT_SHA={sha}");
    println!("cargo:rustc-env=APPARENT_BUILD_MODIFIED={modified}");
    println!("cargo:rustc-env=APPARENT_RELEASE_VERSION={release}");
    println!("cargo:rustc-env=APPARENT_RELEASE_CHANNEL={channel}");
    println!("cargo:rustc-env=APPARENT_REPORTING_ENVIRONMENT={environment}");
    println!(
        "cargo:rustc-env=APPARENT_VERSION_LABEL={}",
        label(
            &std::env::var("CARGO_PKG_VERSION").unwrap(),
            revision.as_deref(),
            modified
        )
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reporting_identity_is_explicit_and_bounded() {
        assert_eq!(
            release_identity(Some("0.12.2"), Some("preview"), Some("production")),
            ("0.12.2".into(), "preview", "production")
        );
        assert_eq!(
            release_identity(None, None, None),
            ("unknown".into(), "development", "development")
        );
        for invalid in ["", "private/path", "secret\nvalue", &"x".repeat(129)] {
            assert_eq!(
                release_identity(Some(invalid), Some("invalid"), Some("invalid")),
                ("unknown".into(), "development", "development")
            );
        }
        assert_eq!(
            release_identity(Some("v1.2.3-rc.1"), Some("stable"), Some("synthetic")),
            ("v1.2.3-rc.1".into(), "stable", "synthetic")
        );
    }

    #[test]
    fn source_archives_do_not_inherit_a_parent_checkouts_revision() {
        let directory = std::env::temp_dir().join(format!(
            "apparent-build-identity-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let archive = directory.join("archive");
        std::fs::create_dir_all(&archive).unwrap();
        let mut command = Command::new("git");
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("GIT_") {
                command.env_remove(key);
            }
        }
        let output = command
            .args(["init", "-q"])
            .arg(&directory)
            .output()
            .unwrap();
        assert!(output.status.success());
        let result = git(&archive, &["rev-parse", "--show-toplevel"]);
        std::fs::remove_dir_all(&directory).unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn about_identifies_the_compiled_revision_not_just_the_release() {
        let sha = "ad8d4b1819738ae6669826206676b1cd17bcdd88";
        assert_eq!(label("0.3.13", Some(sha), false), "0.3.13 · ad8d4b1");
        assert_eq!(
            label("0.3.13", Some(sha), true),
            "0.3.13 · ad8d4b1 +modified"
        );
        for revision in [
            None,
            Some(""),
            Some("main"),
            Some("ad8d4b1"),
            Some("💩💩💩💩💩💩💩💩💩💩"),
        ] {
            assert_eq!(label("0.3.13", revision, false), "0.3.13 · build unknown");
        }
    }
}
