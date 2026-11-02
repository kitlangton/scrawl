//! Keeps `public/site/src/features.json` (the feature map) in step with the code. Every top-level key
//! of `~/.config/apparent/config.json` the app reads and every `handoff::experiment` it asks about
//! needs a feature entry; every entry with a stable, main or experiment status must name keys
//! and flags that still exist; and every doc it links must be in the repo.
//!
//! Keys are found in the source text: the fields of `handoff::Config`, and string literals passed
//! to `set_config_value_in(_, "key", …)` or `config.get("key")`. A key read any other way, nested
//! keys, shortcuts, platforms and whether a status is still true aren't checked here.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// First-party Rust production sources in the root application and its Connect client crate,
/// without comment lines or test-only modules. Scan only the module roots declared by `main.rs`;
/// this avoids dependencies, generated output, and unrelated nested Cargo packages.
fn sources() -> Vec<(PathBuf, String)> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|name| name == "target" || name == "tests") {
                    continue;
                }
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    let root = repo();
    let mut files = vec![root.join("main.rs")];
    for module in ["capture", "composition", "connect", "shared"] {
        walk(&root.join(module), &mut files);
    }
    walk(&root.join("connect/client/src"), &mut files);
    files.sort();
    files.dedup();
    files
        .into_iter()
        .filter(|path| {
            path != &root.join("composition/feature_map.rs")
                && path != &root.join("connect/client/src/test_support.rs")
                && !path.starts_with(root.join("connect/client/src/test_support"))
        })
        .filter_map(|path| {
            let text = fs::read_to_string(&path).unwrap();
            // Standalone files included only by `#[cfg(test)]` are not production sources.
            if text.lines().find(|line| !line.trim().is_empty()).is_some_and(|line| line.trim() == "#[cfg(test)]") {
                return None;
            }
            Some((path, strip_test_modules(&text)))
        })
        .collect()
}

/// Remove inline `#[cfg(test)] mod ...` bodies without discarding production code after them.
fn strip_test_modules(text: &str) -> String {
    let lines: Vec<_> = text.lines().collect();
    let mut kept = String::new();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.trim_start().starts_with("#[cfg(test)]") {
            let mut module_line = i + 1;
            while module_line < lines.len() && lines[module_line].trim().is_empty() {
                module_line += 1;
            }
            let declaration = lines.get(module_line).map(|line| line.trim_start()).unwrap_or("");
            if declaration.starts_with("mod ") {
                // External test modules have no inline body to remove.
                if declaration.contains(';') {
                    i = module_line + 1;
                    continue;
                }
                let indent = lines[module_line].len() - lines[module_line].trim_start().len();
                i = module_line + 1;
                while i < lines.len() {
                    let current = lines[i];
                    let current_indent = current.len() - current.trim_start().len();
                    if current.trim() == "}" && current_indent == indent {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
                continue;
            }
        }
        if !line.trim_start().starts_with("//") {
            kept.push_str(line);
            kept.push('\n');
        }
        i += 1;
    }
    kept
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
        if path.ends_with("shared/handoff/mod.rs") {
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
    let path = repo().join("public/site/src/features.json");
    let json: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap())
        .expect("public/site/src/features.json is valid JSON");
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
fn test_module_stripping_preserves_following_production_code() {
    let source = "before();\n#[cfg(test)]\nmod tests {\n    fn only_for_tests() {}\n}\nafter();\n";
    let stripped = strip_test_modules(source);
    assert!(stripped.contains("before();"));
    assert!(stripped.contains("after();"));
    assert!(!stripped.contains("only_for_tests"));
}

#[test]
fn the_scanner_sees_known_keys_and_flags() {
    let sources = sources();
    assert!(!sources.is_empty(), "scanner found no production Rust sources");
    for suffix in [
        "main.rs",
        "composition/cli.rs",
        "capture/engine/runtime/mod.rs",
        "shared/handoff/config_store.rs",
        "connect/client/src/lib.rs",
    ] {
        assert!(
            sources.iter().any(|(path, _)| path.ends_with(suffix)),
            "scanner missed production source {suffix}"
        );
    }
    let keys = config_keys(&sources);
    let config_keys: BTreeSet<_> = [
        "cloud", "launchAtLogin", "agentHost", "agentOpencode", "updateChannel", "autocrop",
        "autoSend", "videoExport", "transcriber", "warmRecognizer", "lowerPlayback",
        "playbackLevel", "hexModel", "hexLanguage", "appleLocale", "nativePill", "pillStyle",
        "microphone", "routing", "capturesDir",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert!(
        config_keys.is_subset(&keys),
        "scanner missed fields from shared::handoff::Config: {:?}",
        config_keys.difference(&keys).collect::<Vec<_>>()
    );
    assert!(
        keys.contains("videoExport"),
        "video_export() reads Config.video_export; videoExport must remain a scanned config key"
    );
    for key in ["cloud", "microphone", "retention", "transcribeCommand", "onboarding"] {
        assert!(keys.contains(key), "scanner missed config key {key}: {keys:?}");
    }
    let package_sources: Vec<_> = sources
        .iter()
        .filter(|(path, _)| path.starts_with(repo().join("connect/client/src")))
        .cloned()
        .collect();
    let package_flags = experiments(&package_sources).unwrap();
    assert!(
        package_flags.contains("hostedSync") && package_flags.contains("hostedSharing"),
        "Connect flags must be discovered in their production package"
    );
    let flags = experiments(&sources).unwrap();
    for flag in [
        "agentContext",
        "liveTranscribe",
        "clipboardImage",
        "hostedSync",
        "hostedSharing",
    ] {
        assert!(
            flags.contains(flag),
            "scanner missed experiment {flag}: {flags:?}"
        );
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
        "public/site/src/features.json is out of date (see the feature map rule in AGENTS.md):\n- {}",
        problems.join("\n- ")
    );
}