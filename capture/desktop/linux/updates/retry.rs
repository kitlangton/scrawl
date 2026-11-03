//! Cross-process failure cooldown. The caller holds the AppImage transaction lock.
use std::{fs, io::{self, Write}, path::Path, time::{SystemTime, UNIX_EPOCH}};
const COOLDOWN: u64 = 3600;

pub(super) fn now() -> io::Result<u64> {
    SystemTime::now().duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs()).map_err(io::Error::other)
}

pub(super) fn remaining(path: &Path, now: u64) -> io::Result<u64> {
    let value = match fs::read_to_string(path) {
        Ok(value) => value,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    // Retry state is disposable: corruption must not disable updates forever.
    let Ok(failed) = value.trim().parse::<u64>() else { return Ok(0) };
    // Discard a future timestamp after a backwards clock adjustment.
    Ok(if failed > now { 0 } else { COOLDOWN.saturating_sub(now - failed) })
}

pub(super) fn failed(path: &Path, now: u64) -> io::Result<()> {
    let temporary = path.with_extension(format!("retry-{}", std::process::id()));
    let mut file = fs::OpenOptions::new().write(true).create_new(true).open(&temporary)?;
    let result = (|| {
        write!(file, "{now}")?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() { let _ = fs::remove_file(&temporary); }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failures_survive_new_callers_and_expire() {
        let dir = std::env::temp_dir().join(format!("apparent-retry-{}", std::process::id()));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("retry");
        assert_eq!(remaining(&path, 100).unwrap(), 0);
        failed(&path, 100).unwrap();
        assert_eq!(remaining(&path, 101).unwrap(), 3599);
        assert_eq!(remaining(&path, 3700).unwrap(), 0);
        assert_eq!(remaining(&path, 99).unwrap(), 0);
        fs::write(&path, "broken").unwrap();
        assert_eq!(remaining(&path, 100).unwrap(), 0);
        fs::remove_dir_all(dir).unwrap();
    }
}
