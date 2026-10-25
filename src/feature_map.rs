//! Keeps `site/src/features.json` (the feature map) in step with the code. Every top-level key
//! of `~/.config/scrawl/config.json` the app reads and every `handoff::experiment` it asks about
//! needs a feature entry; every entry with a stable, main or experiment status must name keys
//! and flags that still exist; and every doc it links must be in the repo.
//!
//! Keys are found in the source text: the fields of `handoff::Config`, string literals passed to
//! `set_config_value_in(_, "key", …)` or `config.get("key")`, and the router's `root.get("key")`.
//! A key read any other way, nested keys, shortcuts, platforms and whether a status is still true
//! aren't checked here.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// Each `.rs` file under `src/` and the router's `crates/router/src/`, without comment lines or
/// its `#[cfg(test)] mod`.
fn sources() -> Vec<(PathBuf, String)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let mut files = Vec::new();
    walk(&repo().join("src"), &mut files);
    walk(&repo().join("crates/router/src"), &mut files);
    files.sort();
    files
        .into_iter()
        .filter(|path| !path.ends_with("feature_map.rs"))
        .map(|path| {
            let text = fs::read_to_string(&path).unwrap();
            let mut kept = String::new();
            let mut lines = text.lines().peekable();
            while let Some(line) = lines.next() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("#[cfg(test)]")
                    && lines.peek().is_some_and(|next| next.trim_start().starts_with("mod "))
                {
                    break;
                }
                if !trimmed.starts_with("//") {
                    kept.push_str(line);
                    kept.push('\n');
                }
            }
            (path, kept)
        })
        .collect()
}

/// The string literal at the start of `text` (after whitespace), if there is one.
fn literal(text: &str) -> Option<&str> {
    let rest = text.trim_start().strip_prefix('"')?;
    rest.find('"').map(|end| &rest[..end])
}

fn camel_case(snake: &str) -> String {
    let mut out = String::new();
    let mut upper = false;
    for c in snake.chars() {
        if c == '_' {
            upper = true;
        } else if upper {
            out.extend(c.to_uppercase());
            upper = false;
        } else {
            out.push(c);
        }
    }
    out
}

fn config_keys(sources: &[(PathBuf, String)]) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    for (path, text) in sources {
        if path.ends_with("handoff.rs") {
            let start = text.find("pub struct Config {").expect("handoff::Config");
            let body = &text[start..];
            let body = &body[..body.find("\n}").unwrap()];
            for line in body.lines().skip(1) {
                if let Some(field) = line.trim().strip_prefix("pub ")
                    && let Some((name, _)) = field.split_once(':')
                {
                    keys.insert(camel_case(name.trim()));
                }
            }
        }
        for (at, _) in text.match_indices("set_config_value_in(") {
            let args = &text[at + "set_config_value_in(".len()..];
            if let Some((_, rest)) = args.split_once(',')
                && let Some(key) = literal(rest)
            {
                keys.insert(key.to_owned());
            }
        }
        // The router reads its keys from the parsed file (`parse_scrawl_config`).
        if path.ends_with("crates/router/src/config.rs") {
            for pattern in ["root.get(", "text(root,"] {
                for (at, _) in text.match_indices(pattern) {
                    if let Some(key) = literal(&text[at + pattern.len()..]) {
                        keys.insert(key.to_owned());
                    }
                }
            }
        }
        for (at, _) in text.match_indices("config.get(") {
            if let Some(key) = literal(&text[at + "config.get(".len()..]) {
                keys.insert(key.to_owned());
            }
        }
    }
    keys.remove("experiments");
    keys
}

fn experiments(sources: &[(PathBuf, String)]) -> Result<BTreeSet<String>, String> {
    let mut consts = BTreeMap::new();
    for (_, text) in sources {
        for line in text.lines() {
            let line = line.trim();
            let line = line.strip_prefix("pub ").unwrap_or(line);
            if let Some(rest) = line.strip_prefix("const ")
                && let Some((name, value)) = rest.split_once(": &str =")
                && let Some(value) = literal(value)
            {
                consts.insert(name.trim().to_owned(), value.to_owned());
            }
        }
    }
    let mut flags = BTreeSet::new();
    for (path, text) in sources {
        for (at, _) in text.match_indices("experiment(") {
            // `enable_experiment(` and `fn experiment(` aren't questions.
            let prefix = &text[..at];
            if prefix.ends_with(|c: char| c.is_alphanumeric() || c == '_') || prefix.ends_with("fn ") {
                continue;
            }
            let arg = &text[at + "experiment(".len()..];
            if let Some(name) = literal(arg) {
                flags.insert(name.to_owned());
                continue;
            }
            let path_arg = arg[..arg.find(')').unwrap_or(arg.len())].trim();
            let name = path_arg.rsplit("::").next().unwrap_or(path_arg);
            match consts.get(name) {
                Some(value) => {
                    flags.insert(value.clone());
                }
                None => return Err(format!("{}: can't resolve experiment({path_arg})", path.display())),
            }
        }
    }
    Ok(flags)
}

struct Map {
    keys: BTreeSet<String>,
    flags: BTreeSet<String>,
    docs: Vec<(String, String)>,
}

fn feature_map() -> Map {
    let path = repo().join("site/src/features.json");
    let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap())
        .expect("site/src/features.json is valid JSON");
    let mut map = Map { keys: BTreeSet::new(), flags: BTreeSet::new(), docs: Vec::new() };
    for feature in json["features"].as_array().expect("features") {
        let id = feature["id"].as_str().unwrap_or("?").to_owned();
        let status = feature["status"].as_str().unwrap_or("");
        let on_main = status == "main" || status.starts_with("stable@") || status.starts_with("experiment:");
        if !on_main {
            continue;
        }
        if let Some(flag) = status.strip_prefix("experiment:") {
            map.flags.insert(flag.to_owned());
        }
        for config in feature["config"].as_array().into_iter().flatten() {
            let key = config["key"].as_str().unwrap_or("");
            map.keys.insert(key.split('.').next().unwrap_or(key).to_owned());
        }
        for doc in feature["docs"].as_array().into_iter().flatten() {
            let doc = doc.as_str().unwrap_or("");
            map.docs.push((id.clone(), doc.split('#').next().unwrap_or(doc).to_owned()));
        }
    }
    map
}

#[test]
fn the_scanner_sees_known_keys_and_flags() {
    let sources = sources();
    let keys = config_keys(&sources);
    for key in ["autoSend", "shortcuts", "retention", "transcribeCommand", "onboarding", "usbApps"] {
        assert!(keys.contains(key), "scanner missed config key {key}: {keys:?}");
    }
    let flags = experiments(&sources).unwrap();
    for flag in ["agentContext", "liveTranscribe", "smartRouting", "clipboardImage"] {
        assert!(flags.contains(flag), "scanner missed experiment {flag}: {flags:?}");
    }
}

#[test]
fn the_feature_map_covers_every_config_key_and_experiment() {
    let sources = sources();
    let keys = config_keys(&sources);
    let flags = experiments(&sources).unwrap_or_else(|e| panic!("{e}"));
    let map = feature_map();
    let mut problems = Vec::new();
    for key in keys.difference(&map.keys) {
        problems.push(format!("config key \"{key}\" is read by the app but no feature lists it"));
    }
    for key in map.keys.difference(&keys) {
        if !flags.contains(key) {
            problems.push(format!("a feature lists config key \"{key}\", which the app no longer reads"));
        }
    }
    for flag in flags.difference(&map.flags) {
        problems.push(format!("experiment \"{flag}\" has no feature with status experiment:{flag}"));
    }
    for flag in &map.flags {
        if !flags.contains(flag) && !keys.contains(flag) {
            problems.push(format!("status experiment:{flag} names a flag the app no longer reads"));
        }
    }
    for (id, doc) in &map.docs {
        if !repo().join(doc).exists() {
            problems.push(format!("\"{id}\" links {doc}, which isn't in the repo"));
        }
    }
    assert!(
        problems.is_empty(),
        "site/src/features.json is out of date (see the feature map rule in AGENTS.md):\n- {}",
        problems.join("\n- ")
    );
}
