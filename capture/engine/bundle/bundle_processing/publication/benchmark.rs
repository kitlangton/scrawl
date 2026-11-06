//! Copy-only benchmark inputs. Never unlink a caller's existing output or alias
//! a source artifact: publication requires one link per persisted file.
use super::{Directory, fs, preflight_sources};
use anyhow::{Context, Result, ensure};
use std::os::unix::fs::DirBuilderExt;
use std::path::{Component, Path, PathBuf};

fn resolved(path: &Path) -> Result<PathBuf> {
    let absolute = fs::absolute(path)?;
    ensure!(
        !absolute
            .components()
            .any(|c| matches!(c, Component::ParentDir)),
        "benchmark output contains parent traversal"
    );
    let mut existing = absolute.as_path();
    let mut missing = Vec::new();
    while !existing.try_exists()? {
        missing.push(
            existing
                .file_name()
                .context("invalid benchmark output")?
                .to_owned(),
        );
        existing = existing
            .parent()
            .context("benchmark output has no parent")?;
    }
    let mut result = existing.canonicalize()?;
    for name in missing.into_iter().rev() {
        result.push(name);
    }
    Ok(result)
}

pub(crate) fn stage(source: &Path, video: &Path, output: &Path) -> Result<(PathBuf, PathBuf)> {
    preflight_sources(source, Some(video))?;
    let source_path = source.canonicalize()?;
    let output_path = resolved(output)?;
    ensure!(
        !output_path.starts_with(&source_path) && !source_path.starts_with(&output_path),
        "benchmark output overlaps its source bundle"
    );
    ensure!(
        std::fs::symlink_metadata(output).is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
        "benchmark output already exists"
    );
    let leaf = video
        .file_name()
        .and_then(|s| s.to_str())
        .context("invalid benchmark movie name")?;
    let input = Directory::open(source, false)?.file(leaf)?;
    // Create exclusively, even if another worker races the preflight. Never remove
    // an old directory to make this benchmark succeed.
    let parent = output_path
        .parent()
        .context("benchmark output has no parent")?;
    Directory::open(parent, true)?;
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&output_path)?;
    let destination = Directory::open(&output_path, true)?;
    let mut copied = destination.leaf(leaf, true)?;
    std::io::copy(&mut &input, &mut copied)?;
    copied.sync_all()?;
    destination.0.sync_all()?;
    Ok((output_path.clone(), output_path.join(leaf)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::{MetadataExt, symlink};
    #[test]
    fn benchmark_copy_preserves_source_and_rejects_overlap_alias_and_existing_output() {
        let root = std::env::temp_dir().join(crate::shared::ids::make_bundle_id());
        let source = root.join("source.apparent");
        let source_dir = Directory::open(&source, true).unwrap();
        let movie = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/engine/core/tests/fixtures/tiny.mp4"));
        source_dir.write("recording.mp4", movie).unwrap();
        source_dir
            .write("apparent.json", b"source manifest sentinel")
            .unwrap();
        let video = source.join("recording.mp4");
        let before = std::fs::read(&video).unwrap();
        let manifest = std::fs::read(source.join("apparent.json")).unwrap();
        let (output, copy) = stage(&source, &video, &root.join("copy.apparent")).unwrap();
        assert_eq!(std::fs::read(copy).unwrap(), before);
        assert_eq!(std::fs::metadata(&video).unwrap().nlink(), 1);
        symlink(&source, root.join("alias")).unwrap();
        for rejected in [
            &source,
            &root,
            &source.join("child"),
            &root.join("alias"),
            &root.join("alias/child"),
            &output,
        ] {
            assert!(
                stage(&source, &video, rejected).is_err(),
                "accepted {}",
                rejected.display()
            );
            assert_eq!(std::fs::read(&video).unwrap(), before);
            assert_eq!(
                std::fs::read(source.join("apparent.json")).unwrap(),
                manifest
            );
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}
