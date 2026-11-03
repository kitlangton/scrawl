//! HTTPS-authenticated publication manifest. No GitHub API is used by Main checks.
use anyhow::{ensure, Context, Result};
use serde::Deserialize;
use std::{collections::HashSet, sync::Mutex, time::{Duration, Instant}};
use super::update::{Candidate, Release, main_candidate};

pub(super) const URL: &str = "https://github.com/tryapparent/apparent/releases/download/linux-main-feed/main.json";
const LIMIT: usize = 4 * 1024 * 1024;
static CACHE: Mutex<Option<(Instant, Vec<u8>)>> = Mutex::new(None);
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Feed {
    schema: u32,
    source: String,
    release: Release,
    checksums: std::collections::BTreeMap<String, String>,
    ancestors: Vec<String>,
    bridges: Vec<Bridge>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Bridge { id: String, source: String }
fn sha(value: &str) -> bool {
    value.len() == 40 && value.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

pub(super) fn candidate(bytes: &[u8], arch: &str, current: Option<&str>, compiled: Option<&str>) -> Result<Candidate> {
    ensure!(bytes.len() <= LIMIT, "Main feed exceeds size bound");
    let feed: Feed = serde_json::from_slice(bytes).context("invalid Main feed")?;
    ensure!(feed.schema == 1 && sha(&feed.source), "unsupported Main feed identity");
    ensure!(feed.release.tag_name == format!("linux-main-v2-{}", feed.source)
        && feed.release.prerelease && !feed.release.draft && feed.release.published_at.is_some(), "invalid Main release identity");
    ensure!(!feed.ancestors.is_empty() && feed.ancestors.len() <= 50000 && feed.bridges.len() <= 100,
        "invalid Main ancestry size");
    let ancestors: HashSet<_> = feed.ancestors.iter().collect();
    ensure!(ancestors.len() == feed.ancestors.len() && feed.ancestors.iter().all(|id| sha(id))
        && ancestors.contains(&feed.source), "invalid Main ancestry identities");
    let mut bridges = HashSet::new();
    ensure!(feed.bridges.iter().all(|b| sha(&b.id) && sha(&b.source)
        && ancestors.contains(&b.source) && bridges.insert(&b.id)), "invalid bridge identities");
    if let Some(current) = current {
        ensure!(sha(current), "invalid installed Main identity");
        let ordinary = feed.ancestors.iter().any(|id| id == current);
        let bridge = feed.bridges.iter().any(|b| b.id == current && Some(b.source.as_str()) == compiled);
        ensure!(ordinary || bridge, "Main feed is stale or diverges from installed revision");
    }
    ensure!(feed.release.assets.len() == 4 && feed.checksums.len() == 2, "Main feed requires both architectures");
    let mut selected = None;
    for cpu in ["x86_64", "aarch64"] {
        let suffix = format!("-linux-{cpu}.AppImage");
        let assets: Vec<_> = feed.release.assets.iter().filter(|a| a.name.ends_with(&suffix)).collect();
        ensure!(assets.len() == 1, "ambiguous Main image");
        let image = assets[0];
        ensure!(feed.release.assets.iter().filter(|a| a.name == format!("{}.sha256", image.name)).count() == 1,
            "ambiguous Main checksum");
        let text = feed.checksums.get(&image.name).context("missing Main checksum text")?;
        let candidate = main_candidate(&feed.release, cpu, text)?;
        if cpu == arch { selected = Some(candidate) }
    }
    selected.context("unsupported Main architecture")
}

pub(super) fn load(refresh: bool, fetch: impl FnOnce(&str) -> Result<Vec<u8>>) -> Result<Vec<u8>> {
    let mut cache = CACHE.lock().map_err(|_| anyhow::anyhow!("Main feed cache poisoned"))?;
    if let Some((at, bytes)) = cache.as_ref() {
        if !refresh && at.elapsed() < Duration::from_secs(300) { return Ok(bytes.clone()) }
    }
    let bytes = fetch(URL)?;
    // Cache only structurally validated success; revalidate installed identity on every use.
    candidate(&bytes, "x86_64", None, None)?;
    *cache = Some((Instant::now(), bytes.clone()));
    Ok(bytes)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::{json, Value};
    pub(crate) fn fixture() -> Value {
        let source = "b".repeat(40);
        let tag = format!("linux-main-v2-{source}");
        let mut assets = vec![];
        let mut checksums = serde_json::Map::new();
        for arch in ["x86_64", "aarch64"] {
            let name = format!("Apparent-0.3.13-linux-{arch}.AppImage");
            checksums.insert(name.clone(), json!(format!("{}  {name}\n", "c".repeat(64))));
            for filename in [name.clone(), format!("{name}.sha256")] {
                assets.push(json!({"name":filename,"browser_download_url":format!("https://github.com/tryapparent/apparent/releases/download/{tag}/{filename}")}));
            }
        }
        json!({"schema":1,"source":source,"release":{"tag_name":tag,"prerelease":true,"draft":false,
            "published_at":"2026-11-03T00:00:00Z","assets":assets},"checksums":checksums,
            "ancestors":["a".repeat(40),source],"bridges":[{"id":"f".repeat(40),"source":"a".repeat(40)}]})
    }
    fn validate(v: &Value, current: &str, compiled: Option<&str>) -> Result<Candidate> {
        candidate(&serde_json::to_vec(v)?, "x86_64", Some(current), compiled)
    }
    #[test]
    fn normal_main_discovery_needs_no_api_and_caches_success() {
        let bytes = serde_json::to_vec(&fixture()).unwrap();
        let fetched = load(false, |url| {
            ensure!(url == URL && !url.contains("api.github.com"), "API unavailable");
            Ok(bytes.clone())
        }).unwrap();
        assert!(candidate(&fetched, "x86_64", Some(&"a".repeat(40)), None).is_ok());
        assert_eq!(load(false, |_| anyhow::bail!("network unavailable")).unwrap(), bytes);
        // Cached success does not allow rollback after another process stages a newer image.
        assert!(candidate(&fetched, "x86_64", Some(&"d".repeat(40)), None).is_err());
    }
    #[test]
    fn rejects_stale_diverged_malformed_and_tampered_metadata() {
        let base = fixture(); let current = "a".repeat(40);
        assert!(validate(&base, &current, None).is_ok());
        assert!(validate(&base, &"d".repeat(40), None).is_err());
        for field in ["schema", "source", "ancestors", "bridges", "checksums"] {
            let mut value = base.clone(); value[field] = Value::Null;
            assert!(validate(&value, &current, None).is_err(), "{field}");
        }
        let mut v = base.clone(); v["release"]["assets"][0]["browser_download_url"] = json!("https://evil.invalid/image");
        assert!(validate(&v, &current, None).is_err());
        let mut v = base.clone(); v["release"]["tag_name"] = json!(format!("linux-main-v2-{}", current));
        assert!(validate(&v, &current, None).is_err());
        let mut v = base.clone(); v["ancestors"] = json!([current, current]);
        assert!(validate(&v, &current, None).is_err());
        let mut v = base.clone(); v["checksums"]["Apparent-0.3.13-linux-x86_64.AppImage"] = json!(format!("{}  other.AppImage", "c".repeat(64)));
        assert!(validate(&v, &current, None).is_err());
        assert!(candidate(&vec![b' '; LIMIT + 1], "x86_64", None, None).is_err());
    }
    #[test]
    fn bridge_requires_exact_compiled_source_and_canonical_forward_ancestry() {
        let v = fixture(); let bridge = "f".repeat(40); let source = "a".repeat(40);
        assert!(validate(&v, &bridge, Some(&source)).is_ok());
        assert!(validate(&v, &bridge, None).is_err());
        assert!(validate(&v, &bridge, Some(&"d".repeat(40))).is_err());
        let mut v = v; v["ancestors"] = json!(["b".repeat(40)]);
        assert!(validate(&v, &bridge, Some(&source)).is_err());
    }
}
