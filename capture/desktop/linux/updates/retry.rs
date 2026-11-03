//! Cross-process failure cooldown. The caller holds the AppImage transaction lock.
use std::{fs, io::{self, Read, Write}, path::Path, time::{SystemTime, UNIX_EPOCH}};
const COOLDOWN: u64 = 3600;

pub(super) fn now() -> io::Result<u64> {
    SystemTime::now().duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs()).map_err(io::Error::other)
}

pub(super) fn remaining(path: &Path, now: u64) -> io::Result<u64> {
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut bytes = Vec::new();
    file.take(65).read_to_end(&mut bytes)?;
    let failed = (bytes.len() <= 64).then(|| std::str::from_utf8(&bytes).ok())
        .flatten().and_then(|value| value.trim().parse::<u64>().ok());
    match failed {
        Some(failed) if failed <= now && now - failed < COOLDOWN => Ok(COOLDOWN - (now - failed)),
        _ => { clear(path)?; Ok(0) }
    }
}

pub(super) fn clear(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        result => result,
    }
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
    fn persistence_child() {
        let Some(path) = std::env::var_os("APPARENT_RETRY_TEST_PATH") else { return };
        assert_eq!(remaining(Path::new(&path), 101).unwrap(), 3599);
    }
    #[test]
    fn failure_survives_process_restart() {
        let dir = std::env::temp_dir().join(format!("apparent-retry-process-{}", std::process::id()));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("retry");
        failed(&path, 100).unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["persistence_child", "--test-threads=1"])
            .env("APPARENT_RETRY_TEST_PATH", &path).status().unwrap();
        fs::remove_dir_all(dir).unwrap();
        assert!(result.success());
    }
    #[test]
    fn failures_survive_new_callers_and_expire() {
        let dir = std::env::temp_dir().join(format!("apparent-retry-{}", std::process::id()));
        fs::create_dir(&dir).unwrap();
        let path = dir.join("retry");
        assert_eq!(remaining(&path, 100).unwrap(), 0);
        failed(&path, 100).unwrap();
        assert_eq!(remaining(&path, 101).unwrap(), 3599);
        assert_eq!(remaining(&path, 3700).unwrap(), 0);
        failed(&path, 100).unwrap();
        assert_eq!(remaining(&path, 99).unwrap(), 0);
        clear(&path).unwrap(); // successful check after clock rollback
        assert_eq!(remaining(&path, 100).unwrap(), 0);
        assert!(!path.exists());
        fs::write(&path, [255, 254]).unwrap();
        assert_eq!(remaining(&path, 100).unwrap(), 0);
        assert!(!path.exists());
        fs::write(&path, "broken").unwrap();
        assert_eq!(remaining(&path, 100).unwrap(), 0);
        fs::remove_dir_all(dir).unwrap();
    }
}
