//! Recent captures that arrived from elsewhere (the iPhone) and haven't been opened, copied,
//! pasted or dismissed yet. Persisted so the list survives a relaunch.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::capture::scrawl_log;
use crate::model::{ScrawlBundle, format_clock};

const CAPACITY: usize = 8;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InboxEntry {
    pub dir: PathBuf,
    /// `"iphone"` for ingested recordings, `"mac"` for local captures.
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app: Option<String>,
    pub duration_ms: u64,
    pub keyframes: usize,
    pub received_at_ms: u64,
}

impl InboxEntry {
    pub fn new(dir: &Path, bundle: &ScrawlBundle, source: &str, received_at_ms: u64) -> Self {
        Self {
            dir: dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf()),
            source: source.to_owned(),
            app: bundle.target.app.clone().filter(|app| !app.is_empty()),
            duration_ms: bundle.duration_ms,
            keyframes: bundle.keyframes.len(),
            received_at_ms,
        }
    }

    /// `UpDown · 0:25 · 5 frames · 2 min ago`
    pub fn label(&self, now_ms: u64) -> String {
        let name = self.app.as_deref().unwrap_or(match self.source.as_str() {
            "iphone" => "iPhone",
            _ => "Mac",
        });
        format!(
            "{name} · {} · {} {} · {}",
            format_clock(self.duration_ms),
            self.keyframes,
            if self.keyframes == 1 {
                "frame"
            } else {
                "frames"
            },
            relative_age(now_ms.saturating_sub(self.received_at_ms)),
        )
    }
}

pub fn relative_age(age_ms: u64) -> String {
    let minutes = age_ms / 60_000;
    match minutes {
        0 => "just now".to_owned(),
        1..=59 => format!("{minutes} min ago"),
        60..=1_439 => format!("{} hr ago", minutes / 60),
        _ => format!("{} d ago", minutes / 1_440),
    }
}

fn same_dir(a: &Path, b: &Path) -> bool {
    a == b
        || matches!(
            (a.canonicalize(), b.canonicalize()),
            (Ok(a), Ok(b)) if a == b
        )
}

pub struct Inbox {
    path: PathBuf,
    entries: Vec<InboxEntry>,
}

impl Inbox {
    pub fn default_path() -> PathBuf {
        crate::handoff::builtin_captures_dir()
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
            .join("inbox.json")
    }

    pub fn load_default() -> Self {
        Self::load(Self::default_path())
    }

    /// Reads the store, dropping entries whose bundle has since been deleted.
    pub fn load(path: PathBuf) -> Self {
        let mut entries: Vec<InboxEntry> = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        entries.retain(|entry| entry.dir.is_dir());
        entries.truncate(CAPACITY);
        Self { path, entries }
    }

    /// Newest first.
    pub fn entries(&self) -> &[InboxEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn newest(&self) -> Option<&InboxEntry> {
        self.entries.first()
    }

    pub fn add(&mut self, entry: InboxEntry) {
        self.entries.retain(|e| !same_dir(&e.dir, &entry.dir));
        self.entries.insert(0, entry);
        self.entries.retain(|e| e.dir.is_dir());
        self.entries.truncate(CAPACITY);
        self.save();
    }

    /// Marks a bundle handled. Returns whether it was in the inbox.
    pub fn remove(&mut self, dir: &Path) -> bool {
        let before = self.entries.len();
        self.entries.retain(|e| !same_dir(&e.dir, dir));
        let removed = self.entries.len() != before;
        if removed {
            self.save();
        }
        removed
    }

    fn save(&self) {
        let result = (|| -> anyhow::Result<()> {
            if let Some(parent) = self.path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let tmp = self.path.with_extension("json.tmp");
            std::fs::write(&tmp, serde_json::to_vec_pretty(&self.entries)?)?;
            std::fs::rename(&tmp, &self.path)?;
            Ok(())
        })();
        if let Err(err) = result {
            scrawl_log(format!("inbox save failed: {err:#}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        root: PathBuf,
    }

    impl Fixture {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("scrawl-inbox-{}", crate::pipeline::make_bundle_id()));
            std::fs::create_dir_all(&root).unwrap();
            let root = root.canonicalize().unwrap();
            Self { root }
        }

        fn store(&self) -> PathBuf {
            self.root.join("inbox.json")
        }

        fn bundle(&self, name: &str) -> PathBuf {
            let dir = self.root.join(format!("{name}.scrawl"));
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }

        fn entry(&self, name: &str, received_at_ms: u64) -> InboxEntry {
            InboxEntry {
                dir: self.bundle(name),
                source: "iphone".into(),
                app: Some("UpDown".into()),
                duration_ms: 25_000,
                keyframes: 5,
                received_at_ms,
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn dirs(inbox: &Inbox) -> Vec<PathBuf> {
        inbox.entries().iter().map(|e| e.dir.clone()).collect()
    }

    #[test]
    fn adds_newest_first_and_persists() {
        let fx = Fixture::new();
        let mut inbox = Inbox::load(fx.store());
        assert_eq!(inbox.len(), 0);
        inbox.add(fx.entry("a", 1));
        inbox.add(fx.entry("b", 2));
        assert_eq!(dirs(&inbox), [fx.bundle("b"), fx.bundle("a")]);
        assert_eq!(inbox.newest().unwrap().dir, fx.bundle("b"));

        let reloaded = Inbox::load(fx.store());
        assert_eq!(reloaded.entries(), inbox.entries());
    }

    #[test]
    fn caps_at_capacity_dropping_oldest() {
        let fx = Fixture::new();
        let mut inbox = Inbox::load(fx.store());
        for i in 0..CAPACITY + 3 {
            inbox.add(fx.entry(&format!("b{i}"), i as u64));
        }
        assert_eq!(inbox.len(), CAPACITY);
        assert_eq!(
            inbox.newest().unwrap().dir,
            fx.bundle(&format!("b{}", CAPACITY + 2))
        );
        assert!(!dirs(&inbox).contains(&fx.bundle("b2")));
        assert!(dirs(&inbox).contains(&fx.bundle("b3")));
    }

    #[test]
    fn dedupes_by_dir_moving_it_to_the_front() {
        let fx = Fixture::new();
        let mut inbox = Inbox::load(fx.store());
        inbox.add(fx.entry("a", 1));
        inbox.add(fx.entry("b", 2));
        // Same bundle through a non-canonical path.
        let mut again = fx.entry("a", 3);
        again.dir = fx.root.join(".").join("a.scrawl");
        inbox.add(again);
        assert_eq!(inbox.len(), 2);
        assert_eq!(inbox.newest().unwrap().received_at_ms, 3);
        assert_eq!(inbox.entries()[1].dir, fx.bundle("b"));
    }

    #[test]
    fn remove_marks_handled() {
        let fx = Fixture::new();
        let mut inbox = Inbox::load(fx.store());
        inbox.add(fx.entry("a", 1));
        inbox.add(fx.entry("b", 2));
        assert!(inbox.remove(&fx.root.join(".").join("b.scrawl")));
        assert!(!inbox.remove(&fx.bundle("b")));
        assert!(!inbox.remove(Path::new("latest")));
        assert_eq!(dirs(&inbox), [fx.bundle("a")]);
        assert_eq!(dirs(&Inbox::load(fx.store())), [fx.bundle("a")]);
    }

    #[test]
    fn prunes_bundles_that_no_longer_exist() {
        let fx = Fixture::new();
        let mut inbox = Inbox::load(fx.store());
        inbox.add(fx.entry("a", 1));
        inbox.add(fx.entry("b", 2));
        std::fs::remove_dir_all(fx.bundle("a")).unwrap();
        assert_eq!(dirs(&Inbox::load(fx.store())), [fx.bundle("b")]);

        std::fs::remove_dir_all(fx.bundle("b")).unwrap();
        inbox.add(fx.entry("c", 3));
        assert_eq!(dirs(&inbox), [fx.bundle("c")]);
    }

    #[test]
    fn missing_or_corrupt_store_is_empty() {
        let fx = Fixture::new();
        assert_eq!(Inbox::load(fx.root.join("nope.json")).len(), 0);
        std::fs::write(fx.store(), b"{not json").unwrap();
        assert_eq!(Inbox::load(fx.store()).len(), 0);
    }

    #[test]
    fn labels_are_compact() {
        let fx = Fixture::new();
        let entry = fx.entry("a", 1_000);
        assert_eq!(
            entry.label(1_000 + 2 * 60_000 + 5_000),
            "UpDown · 0:25 · 5 frames · 2 min ago"
        );
        let single = InboxEntry {
            app: None,
            keyframes: 1,
            ..entry
        };
        assert_eq!(single.label(1_000), "iPhone · 0:25 · 1 frame · just now");
        assert_eq!(relative_age(3 * 3_600_000), "3 hr ago");
        assert_eq!(relative_age(2 * 86_400_000), "2 d ago");
    }
}
