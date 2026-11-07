//! Capture-local typed explanations. Rust owns persistence and click-time anchors.
use anyhow::{Result, ensure};
use std::path::{Path, PathBuf};

pub use apparent_capture_engine::recording::notes::{MAX_NOTES, MAX_TEXT_BYTES, Note};

/// Unsubmitted drafts stay local to the running capture, never in exported media/text.
pub struct Store {
    path: PathBuf,
    saved: Vec<Note>,
    drafts: std::collections::BTreeMap<u32, Note>,
}
impl Store {
    pub fn new(directory: &Path) -> Self {
        Self {
            path: directory.join("notes.json"),
            saved: Vec::new(),
            drafts: Default::default(),
        }
    }
    pub fn open(&mut self, pin: u32, t: u64, x: f32, y: f32) -> Result<()> {
        self.dismiss();
        ensure!(
            self.drafts.len() < MAX_NOTES || self.drafts.contains_key(&pin),
            "Too many notes"
        );
        ensure!(
            pin > 0
                && x.is_finite()
                && y.is_finite()
                && (0.0..=1.0).contains(&x)
                && (0.0..=1.0).contains(&y),
            "Invalid note anchor"
        );
        self.drafts.entry(pin).or_insert(Note {
            pin,
            t,
            x,
            y,
            text: String::new(),
        });
        Ok(())
    }
    pub fn draft(&mut self, pin: u32, text: String) -> Result<()> {
        ensure!(
            text.len() <= MAX_TEXT_BYTES && !text.contains('\0'),
            "Note is too long"
        );
        self.drafts
            .get_mut(&pin)
            .ok_or_else(|| anyhow::anyhow!("Unknown note"))?
            .text = text;
        Ok(())
    }
    pub fn save(&mut self, pin: u32, text: String) -> Result<()> {
        self.draft(pin, text)?;
        let mut note = self.drafts[&pin].clone();
        note.text = note.text.trim().to_owned();
        // Empty Enter dismisses; it does not create a note or erase an existing one.
        if note.text.is_empty() {
            if let Some(saved) = self.saved.iter().find(|n| n.pin == pin) {
                self.drafts.get_mut(&pin).unwrap().text = saved.text.clone();
            }
            return Ok(());
        }
        let mut next = self.saved.clone();
        if let Some(old) = next.iter_mut().find(|n| n.pin == pin) {
            *old = note;
        } else {
            next.push(note);
        }
        next.sort_by_key(|n| (n.t, n.pin));
        self.commit(next)
    }
    /// Empty pins are interactions, not draft notes and must not exhaust the note limit.
    pub fn dismiss(&mut self) {
        self.drafts
            .retain(|pin, n| !n.text.trim().is_empty() || self.saved.iter().any(|s| s.pin == *pin));
    }
    pub fn delete(&mut self, pin: u32) -> Result<()> {
        ensure!(self.drafts.contains_key(&pin), "Unknown note");
        self.undo(pin)
    }
    /// Undo also accepts an empty pin whose draft was already dismissed.
    pub fn undo(&mut self, pin: u32) -> Result<()> {
        let next = self
            .saved
            .iter()
            .filter(|n| n.pin != pin)
            .cloned()
            .collect();
        self.commit(next)?;
        self.drafts.remove(&pin);
        Ok(())
    }
    fn commit(&mut self, next: Vec<Note>) -> Result<()> {
        let directory = self.path.parent().unwrap();
        let nonce = crate::shared::ids::make_bundle_id();
        apparent_capture_engine::recording::notes::commit(directory, &nonce, &next)?;
        self.saved = next;
        Ok(())
    }
    pub fn notes(&self) -> &[Note] {
        &self.saved
    }
    pub fn drafts(&self) -> Vec<Note> {
        self.drafts.values().cloned().collect()
    }
}

pub fn read(directory: &Path) -> Result<Vec<Note>> {
    apparent_capture_engine::recording::notes::read(directory)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn dir() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "apparent-notes-{}",
            crate::shared::ids::make_bundle_id()
        ));
        std::fs::create_dir(&p).unwrap();
        p
    }
    #[test]
    fn failed_undo_keeps_durable_note_and_draft_for_recovery() {
        let directory = dir();
        let mut store = Store::new(&directory);
        store.open(1, 10, 0.5, 0.5).unwrap();
        store.save(1, "keep".into()).unwrap();
        let original = store.path.clone();
        store.path = directory.join("missing").join("notes.json");
        assert!(store.undo(1).is_err());
        assert_eq!(store.notes()[0].text, "keep");
        assert_eq!(store.drafts()[0].pin, 1);
        assert_eq!(read(&directory).unwrap()[0].pin, 1);
        store.path = original;
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn undo_removes_saved_note_and_draft_and_accepts_dismissed_pin() {
        let directory = dir();
        let mut store = Store::new(&directory);
        store.open(1, 10, 0.5, 0.5).unwrap();
        store.save(1, "attached".into()).unwrap();
        store.undo(1).unwrap();
        store.undo(2).unwrap();
        assert!(store.drafts().is_empty());
        assert!(read(&directory).unwrap().is_empty());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn accepted_escaped_notes_round_trip_at_full_capacity() {
        let directory = dir();
        let mut store = Store::new(&directory);
        for pin in 1..=MAX_NOTES as u32 {
            store.open(pin, u64::from(pin), 0.5, 0.5).unwrap();
            store.save(pin, "\u{1}".repeat(MAX_TEXT_BYTES)).unwrap();
        }
        assert_eq!(read(&directory).unwrap(), store.notes());
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn note_reader_rejects_fifo_without_waiting_for_a_writer() {
        use std::os::unix::ffi::OsStrExt;
        let directory = dir();
        let path =
            std::ffi::CString::new(directory.join("notes.json").as_os_str().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
        assert!(
            read(&directory)
                .unwrap_err()
                .to_string()
                .contains("regular file")
        );
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn empty_and_deleted_pins_do_not_exhaust_note_capacity() {
        let directory = dir();
        let mut store = Store::new(&directory);
        for pin in 1..=250 {
            store.open(pin, 1, 0.5, 0.5).unwrap();
            store.save(pin, String::new()).unwrap();
        }
        store.save(250, "keep".into()).unwrap();
        store.delete(250).unwrap();
        assert!(store.drafts().is_empty());
        assert!(read(&directory).unwrap().is_empty());
        std::fs::remove_dir_all(directory).unwrap();
    }
    #[test]
    fn notes_save_edit_delete_preserve_click_time_and_exclude_drafts() {
        let p = dir();
        let mut s = Store::new(&p);
        s.open(1, 125, 0.2, 0.3).unwrap();
        s.draft(1, "unsent".into()).unwrap();
        assert!(read(&p).unwrap().is_empty());
        s.save(1, " first \n second ".into()).unwrap();
        s.open(1, 900, 0.9, 0.9).unwrap();
        s.save(1, "edited".into()).unwrap();
        assert_eq!(
            read(&p).unwrap(),
            vec![Note {
                pin: 1,
                t: 125,
                x: 0.2,
                y: 0.3,
                text: "edited".into()
            }]
        );
        s.open(2, 200, 0.3, 0.4).unwrap();
        s.save(2, "  ".into()).unwrap();
        assert_eq!(s.notes().len(), 1);
        s.save(1, "".into()).unwrap();
        assert_eq!(s.drafts()[0].text, "edited");
        s.delete(1).unwrap();
        assert!(read(&p).unwrap().is_empty());
        std::fs::remove_dir_all(p).unwrap();
    }
    #[test]
    fn failed_note_save_keeps_draft_and_saved_count() {
        let p = dir();
        let mut s = Store::new(&p);
        s.open(1, 1, 0.2, 0.3).unwrap();
        std::fs::create_dir(p.join("notes.json")).unwrap();
        assert!(s.save(1, "keep this".into()).is_err());
        assert!(s.notes().is_empty());
        assert_eq!(s.drafts()[0].text, "keep this");
        std::fs::remove_dir_all(p).unwrap();
    }
}
