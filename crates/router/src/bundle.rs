//! Finding and reading `.scrawl` bundles.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, ensure};

use crate::model::ScrawlBundle;

/// A bundle's folder from a path, a `scrawl.json` in it, a bare bundle ID (looked up in the
/// host's captures folder) or `latest`.
pub fn resolve_bundle_dir(path_or_id: &Path) -> Result<PathBuf> {
    resolve_bundle_dir_in(crate::env::get().dirs.captures.as_deref(), path_or_id)
}

/// [`resolve_bundle_dir`] with the captures folder given.
pub fn resolve_bundle_dir_in(captures: Option<&Path>, path_or_id: &Path) -> Result<PathBuf> {
    if path_or_id == Path::new("latest") {
        return newest_bundle(captures.context("No captures folder")?);
    }
    let mut path = path_or_id.to_owned();
    if path.file_name().is_some_and(|name| name == "scrawl.json") {
        path = path
            .parent()
            .context("Missing bundle directory")?
            .to_owned();
        if path.as_os_str().is_empty() {
            path = PathBuf::from(".");
        }
    }
    if !path.exists()
        && path.components().count() == 1
        && let Some(captures) = captures
    {
        path = captures.join(&path);
        if path.extension().is_none() {
            path.set_extension("scrawl");
        }
    }
    ensure!(
        path.is_dir(),
        "Bundle directory not found: {}",
        path.display()
    );
    let path = plain_path(
        path.canonicalize()
            .with_context(|| format!("Could not resolve {}", path.display()))?,
    );
    ensure!(
        path.extension().is_some_and(|ext| ext == "scrawl"),
        "Expected a .scrawl directory: {}",
        path.display()
    );
    Ok(path)
}

pub fn newest_bundle(captures: &Path) -> Result<PathBuf> {
    let mut candidates = Vec::new();
    for entry in
        fs::read_dir(captures).with_context(|| format!("Could not read {}", captures.display()))?
    {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "scrawl") && path.is_dir() {
            // Bundle creation time is stable even after later zoom-frame extraction.
            let created = fs::read(path.join("scrawl.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice::<ScrawlBundle>(&bytes).ok())
                .map(|bundle| bundle.created_at_ms)
                .unwrap_or_else(|| {
                    entry
                        .metadata()
                        .ok()
                        .and_then(|m| m.created().or_else(|_| m.modified()).ok())
                        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                        .map_or(0, |time| time.as_millis() as u64)
                });
            candidates.push((created, path));
        }
    }
    candidates.sort_unstable();
    candidates
        .pop()
        .context("No .scrawl captures found")?
        .1
        .canonicalize()
        .map(plain_path)
        .map_err(Into::into)
}

/// `canonicalize` on Windows returns `\\?\C:\…`; agents and terminals want `C:\…`.
pub fn plain_path(path: PathBuf) -> PathBuf {
    match path.to_str().and_then(|p| p.strip_prefix(r"\\?\")) {
        Some(rest) if rest.as_bytes().get(1) == Some(&b':') => PathBuf::from(rest),
        _ => path,
    }
}

pub fn load_bundle(bundle_dir: &Path) -> Result<(PathBuf, ScrawlBundle)> {
    load_bundle_in(crate::env::get().dirs.captures.as_deref(), bundle_dir)
}

/// [`load_bundle`] with the captures folder given.
pub fn load_bundle_in(
    captures: Option<&Path>,
    bundle_dir: &Path,
) -> Result<(PathBuf, ScrawlBundle)> {
    let directory = resolve_bundle_dir_in(captures, bundle_dir)?;
    let path = directory.join("scrawl.json");
    let bundle = serde_json::from_slice(
        &fs::read(&path).with_context(|| format!("Could not read {}", path.display()))?,
    )
    .with_context(|| format!("Invalid bundle manifest: {}", path.display()))?;
    Ok((directory, bundle))
}
