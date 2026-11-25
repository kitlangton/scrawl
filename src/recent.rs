//! The newest captures for the menu bar's Recent Captures section: what each one is called, how
//! long ago it was made, and the contact sheet to show beside it. Read fresh from disk each time
//! the menu opens, from `scrawl.json` as a loose JSON value, so a bundle from another version or a
//! half-written one is skipped rather than failing the menu.

use std::path::{Path, PathBuf};

use crate::inbox::{relative_age, same_dir};

/// How many captures the menu lists.
pub const MENU_LIMIT: usize = 5;
/// Longest title before it's cut at a word.
const TITLE_CHARS: usize = 40;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentCapture {
    pub dir: PathBuf,
    pub created_at_ms: u64,
    pub duration_ms: u64,
    /// The note, else the first words said, else the app and window recorded.
    pub title: String,
    pub contact_sheet: Option<PathBuf>,
    /// The picture the row's thumbnail is cut from: the keyframe the first mark was drawn on (it
    /// carries the ink), else the first keyframe, else the contact sheet.
    pub thumbnail_source: Option<PathBuf>,
}

impl RecentCapture {
    /// `Header jumps on expand · 0:09 · 3 min ago`
    pub fn label(&self, now_ms: u64) -> String {
        format!("{} · {}", self.title, self.details(now_ms))
    }

    /// `0:09 · 3 min ago`: the part of the label a row keeps whole when the title is cut short.
    pub fn details(&self, now_ms: u64) -> String {
        format!(
            "{} · {}",
            crate::model::format_clock(self.duration_ms),
            relative_age(now_ms.saturating_sub(self.created_at_ms)),
        )
    }
}

/// The newest `limit` finished captures in `dir`, newest first, leaving out `skip` (captures the
/// menu already lists, such as unhandled iPhone captures).
pub fn recent_captures(dir: &Path, limit: usize, skip: &[PathBuf]) -> Vec<RecentCapture> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut captures: Vec<RecentCapture> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "scrawl"))
        // The inbox keeps canonical paths and the captures folder may be reached through a
        // symlink, so compare the way the inbox does.
        .filter(|path| !skip.iter().any(|skipped| same_dir(skipped, path)))
        .filter_map(|path| read_capture(&path))
        .collect();
    captures.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
    captures.truncate(limit);
    captures
}

fn read_capture(dir: &Path) -> Option<RecentCapture> {
    let text = std::fs::read_to_string(dir.join("scrawl.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let created_at_ms = json.get("createdAtMs")?.as_u64()?;
    let duration_ms = json
        .get("durationMs")
        .and_then(|value| value.as_u64())
        .unwrap_or(0);
    let text_at = |pointer: &str| {
        json.pointer(pointer)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
    };
    let title = text_at("/note")
        .map(|note| first_words(note, TITLE_CHARS))
        .or_else(|| text_at("/transcript/text").map(|said| first_words(said, TITLE_CHARS)))
        .or_else(|| {
            let place: Vec<&str> = [text_at("/target/app"), text_at("/target/windowTitle")]
                .into_iter()
                .flatten()
                .collect();
            (!place.is_empty()).then(|| first_words(&place.join(" · "), TITLE_CHARS))
        })
        .or_else(|| {
            // A screen capture names no app, but auto-crop records the apps it framed.
            let mut apps: Vec<&str> = Vec::new();
            let crops = json
                .pointer("/autocrop/crops")
                .and_then(|value| value.as_array());
            for app in crops
                .into_iter()
                .flatten()
                .filter_map(|crop| crop.get("apps")?.as_array())
                .flatten()
            {
                if let Some(app) = app.as_str().map(str::trim).filter(|app| !app.is_empty())
                    && !apps.contains(&app)
                {
                    apps.push(app);
                }
            }
            (!apps.is_empty()).then(|| first_words(&apps.join(" · "), TITLE_CHARS))
        })
        .unwrap_or_else(|| "Capture".to_owned());
    let sheet = dir.join(
        json.get("contactSheet")
            .and_then(|value| value.as_str())
            .unwrap_or("contact-sheet.png"),
    );
    let contact_sheet = sheet.is_file().then_some(sheet);
    let marked = json
        .get("marks")
        .and_then(|marks| marks.as_array())
        .into_iter()
        .flatten()
        .filter_map(|mark| mark.get("frame")?.as_str())
        .map(|frame| dir.join(frame))
        .find(|frame| frame.is_file());
    let first = Some(dir.join("frames").join("frame-01.png")).filter(|frame| frame.is_file());
    let thumbnail_source = marked.or(first).or_else(|| contact_sheet.clone());
    Some(RecentCapture {
        dir: dir.to_path_buf(),
        created_at_ms,
        duration_ms,
        title,
        contact_sheet,
        thumbnail_source,
    })
}

/// Up to `max` characters, cut at a word when it has to be cut, with an ellipsis.
fn first_words(text: &str, max: usize) -> String {
    let clean = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.chars().count() <= max {
        return clean;
    }
    let cut: String = clean.chars().take(max).collect();
    let kept = match cut.rfind(' ') {
        Some(space) if space > max / 2 => &cut[..space],
        _ => cut.as_str(),
    };
    format!(
        "{}…",
        kept.trim_end_matches(|c: char| c.is_whitespace() || ",.;:".contains(c))
    )
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
                std::env::temp_dir().join(format!("scrawl-recent-{}", crate::make_bundle_id()));
            std::fs::create_dir_all(&root).unwrap();
            Self { root }
        }

        fn bundle(&self, name: &str, json: &str, sheet: bool) -> PathBuf {
            let dir = self.root.join(format!("{name}.scrawl"));
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("scrawl.json"), json).unwrap();
            if sheet {
                std::fs::write(dir.join("contact-sheet.png"), b"png").unwrap();
            }
            dir
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    #[test]
    fn newest_first_limited_and_skipping() {
        let f = Fixture::new();
        let old = f.bundle("a", r#"{"createdAtMs": 1000, "note": "Old"}"#, true);
        let mid = f.bundle("b", r#"{"createdAtMs": 2000, "note": "Mid"}"#, true);
        let new = f.bundle("c", r#"{"createdAtMs": 3000, "note": "New"}"#, true);
        let got = recent_captures(&f.root, 2, &[]);
        assert_eq!(
            got.iter().map(|c| c.dir.clone()).collect::<Vec<_>>(),
            [new.clone(), mid.clone()]
        );
        let got = recent_captures(&f.root, 5, &[new]);
        assert_eq!(
            got.iter().map(|c| c.dir.clone()).collect::<Vec<_>>(),
            [mid, old]
        );
    }

    #[test]
    fn titles_fall_back_from_note_to_speech_to_place() {
        let f = Fixture::new();
        f.bundle(
            "n",
            r#"{"createdAtMs": 5, "note": "  Header shifts  ", "transcript": {"text": "ignored"}}"#,
            false,
        );
        f.bundle("s", r#"{"createdAtMs": 4, "note": null, "transcript": {"text": "When I click Used 12 tools this band header jumps down by four pixels"}}"#, false);
        f.bundle(
            "p",
            r#"{"createdAtMs": 3, "target": {"app": "OpenCode", "windowTitle": "session"}}"#,
            false,
        );
        f.bundle(
            "c",
            r#"{"createdAtMs": 2, "target": {"mode": "display", "app": null}, "autocrop": {"crops": [{"apps": ["Otty"]}, {"apps": ["Otty", "Safari"]}]}}"#,
            false,
        );
        f.bundle(
            "e",
            r#"{"createdAtMs": 1, "transcript": {"text": "   "}, "target": {"app": null}}"#,
            false,
        );
        let titles: Vec<String> = recent_captures(&f.root, 5, &[])
            .into_iter()
            .map(|c| c.title)
            .collect();
        assert_eq!(
            titles,
            [
                "Header shifts",
                "When I click Used 12 tools this band…",
                "OpenCode · session",
                "Otty · Safari",
                "Capture"
            ]
        );
    }

    // Symlinks are a Unix test fixture; the comparison itself is the same on every platform.
    #[cfg(unix)]
    #[test]
    fn skips_an_inbox_capture_listed_by_its_resolved_path() {
        let f = Fixture::new();
        let real = f.root.join("real");
        std::fs::create_dir_all(&real).unwrap();
        let inner = Fixture { root: real.clone() };
        let iphone = inner.bundle(
            "iphone",
            r#"{"createdAtMs": 2, "note": "From iPhone"}"#,
            false,
        );
        inner.bundle("mac", r#"{"createdAtMs": 1, "note": "Mac"}"#, false);
        std::mem::forget(inner);
        let link = f.root.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        // The inbox keeps canonical paths; the menu scans the configured (symlinked) folder.
        let skip = [iphone.canonicalize().unwrap()];
        let titles: Vec<String> = recent_captures(&link, 5, &skip)
            .into_iter()
            .map(|c| c.title)
            .collect();
        assert_eq!(titles, ["Mac"]);
    }

    #[test]
    fn thumbnail_prefers_the_marked_keyframe_then_the_first_then_the_sheet() {
        let f = Fixture::new();
        let frame = |dir: &PathBuf, name: &str| {
            std::fs::create_dir_all(dir.join("frames")).unwrap();
            std::fs::write(dir.join("frames").join(name), b"png").unwrap();
        };
        let marked = f.bundle(
            "marked",
            r#"{"createdAtMs": 3, "marks": [{"id": "m1"}, {"id": "m2", "frame": "frames/frame-02.png"}]}"#,
            true,
        );
        frame(&marked, "frame-01.png");
        frame(&marked, "frame-02.png");
        let first = f.bundle(
            "first",
            r#"{"createdAtMs": 2, "marks": [{"frame": "frames/gone.png"}]}"#,
            true,
        );
        frame(&first, "frame-01.png");
        let sheet = f.bundle("sheet", r#"{"createdAtMs": 1}"#, true);
        let sources: Vec<Option<PathBuf>> = recent_captures(&f.root, 5, &[])
            .into_iter()
            .map(|c| c.thumbnail_source)
            .collect();
        assert_eq!(
            sources,
            [
                Some(marked.join("frames/frame-02.png")),
                Some(first.join("frames/frame-01.png")),
                Some(sheet.join("contact-sheet.png")),
            ]
        );
    }

    #[test]
    fn skips_unfinished_and_foreign_folders_and_finds_the_sheet() {
        let f = Fixture::new();
        f.bundle(
            "ok",
            r#"{"createdAtMs": 9, "durationMs": 9000, "note": "Kept"}"#,
            true,
        );
        f.bundle(
            "nosheet",
            r#"{"createdAtMs": 8, "note": "No sheet"}"#,
            false,
        );
        f.bundle("broken", "{not json", true);
        f.bundle("notime", r#"{"note": "No time"}"#, true);
        std::fs::create_dir_all(f.root.join("other-folder")).unwrap();
        let got = recent_captures(&f.root, 5, &[]);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].title, "Kept");
        assert!(got[0].contact_sheet.is_some());
        assert_eq!(got[1].contact_sheet, None);
        assert_eq!(got[0].label(9 + 3 * 60_000), "Kept · 0:09 · 3 min ago");
        assert!(recent_captures(&f.root.join("missing"), 5, &[]).is_empty());
    }
}
