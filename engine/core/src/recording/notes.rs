//! Versioned saved-note file persistence and validation.
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const MAX_TEXT_BYTES: usize = 8_192;
pub const MAX_NOTES: usize = 100;
// JSON can escape one input byte as six bytes (e.g. a control character).
const MAX_SAVED_BYTES: usize = MAX_NOTES * MAX_TEXT_BYTES * 6 + 65_536;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Note {
    pub pin: u32,
    pub t: u64,
    pub x: f32,
    pub y: f32,
    pub text: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    version: u32,
    notes: Vec<Note>,
}

/// Commit the legacy Saved/1 document in a caller-provisioned private bundle directory.
///
/// The caller supplies the existing note-store nonce; the directory and nonce are
/// trusted inputs from the native recording owner. Product ordering, draft state,
/// and in-memory commit decisions remain with that caller.
pub fn commit(directory: &Path, nonce: &str, notes: &[Note]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;

    ensure!(
        !nonce.is_empty()
            && !nonce.contains('/')
            && nonce != "."
            && nonce != ".."
            && !nonce.contains('\0'),
        "Invalid note commit nonce"
    );
    let target = directory.join("notes.json");
    let temp = directory.join(format!(".notes-{nonce}.tmp"));
    let result = (|| {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        file.write_all(&serde_json::to_vec(&Saved {
            version: 1,
            notes: notes.to_vec(),
        })?)?;
        file.sync_all()?;
        std::fs::rename(&temp, &target)?;
        std::fs::File::open(directory)?.sync_all()?;
        Ok::<_, anyhow::Error>(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result
}

/// Read and validate the bounded legacy Saved/1 document. Missing files mean no notes.
pub fn read(directory: &Path) -> Result<Vec<Note>> {
    let path = directory.join("notes.json");
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt;
    let data = match std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&path)
    {
        Ok(file) => {
            ensure!(file.metadata()?.is_file(), "Notes must be a regular file");
            let mut data = Vec::new();
            file.take((MAX_SAVED_BYTES + 1) as u64)
                .read_to_end(&mut data)?;
            data
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    ensure!(data.len() <= MAX_SAVED_BYTES, "Notes file is too large");
    let saved: Saved = serde_json::from_slice(&data)?;
    ensure!(
        saved.version == 1 && saved.notes.len() <= MAX_NOTES,
        "Unsupported notes file"
    );
    let mut pins = std::collections::BTreeSet::new();
    for note in &saved.notes {
        ensure!(
            note.pin > 0
                && pins.insert(note.pin)
                && note.x.is_finite()
                && note.y.is_finite()
                && (0.0..=1.0).contains(&note.x)
                && (0.0..=1.0).contains(&note.y)
                && note.text.len() <= MAX_TEXT_BYTES
                && !note.text.contains('\0')
                && !note.text.trim().is_empty(),
            "Invalid saved note"
        );
    }
    Ok(saved.notes)
}
