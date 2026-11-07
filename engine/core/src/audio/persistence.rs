//! Recording-time narration: bounded sample history and an off-callback PCM writer.
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::ops::{Bound, RangeBounds};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, ensure};

const RATE: usize = 16_000;
const HISTORY: usize = 60 * RATE;
const MAX_BACKLOG: usize = 2 * RATE;
const POLL: Duration = Duration::from_millis(100);
const SYNC: Duration = Duration::from_secs(1);
pub const RECORDING: &str = ".audio-recording";

#[derive(Debug)]
pub struct SaveFailure;
impl std::fmt::Display for SaveFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Narration could not be saved; the saved prefix was retained")
    }
}
impl std::error::Error for SaveFailure {}

/// Mirrored fixed-capacity ring. Absolute indices never move when old samples expire.
/// Allocation happens before opening the input; callback pushes never allocate.
pub struct Samples {
    data: Vec<f32>,
    total: usize,
}
impl Default for Samples {
    fn default() -> Self {
        Self {
            data: vec![0.0; 2 * HISTORY],
            total: 0,
        }
    }
}
impl Samples {
    pub fn len(&self) -> usize {
        self.total
    }
    pub fn is_empty(&self) -> bool {
        self.total == 0
    }
    pub fn clear(&mut self) {
        self.total = 0;
    }
    pub fn push(&mut self, value: f32) {
        let at = self.total % HISTORY;
        self.data[at] = value;
        self.data[at + HISTORY] = value;
        self.total += 1;
    }
    pub fn extend(&mut self, values: impl IntoIterator<Item = f32>) {
        for value in values {
            self.push(value);
        }
    }
    pub fn extend_from_slice(&mut self, values: &[f32]) {
        self.extend(values.iter().copied());
    }
    pub fn get(&self, range: impl RangeBounds<usize>) -> Option<&[f32]> {
        let start = match range.start_bound() {
            Bound::Included(&v) => v,
            Bound::Excluded(&v) => v.checked_add(1)?,
            Bound::Unbounded => 0,
        };
        let end = match range.end_bound() {
            Bound::Included(&v) => v.checked_add(1)?,
            Bound::Excluded(&v) => v,
            Bound::Unbounded => self.total,
        };
        if start < self.total.saturating_sub(HISTORY) || end > self.total || end < start {
            return None;
        }
        let at = start % HISTORY;
        Some(&self.data[at..at + end - start])
    }
    pub fn recent(&self) -> &[f32] {
        self.get(self.total.saturating_sub(HISTORY)..).unwrap()
    }
}
impl From<Vec<f32>> for Samples {
    fn from(values: Vec<f32>) -> Self {
        let mut s = Self::default();
        s.extend(values);
        s
    }
}
impl std::ops::Deref for Samples {
    type Target = [f32];
    fn deref(&self) -> &[f32] {
        self.recent()
    }
}

/// Advisory kernel lock, not PID/mtime guessing. The file stays after a crash.
#[derive(Debug)]
pub struct Lease {
    file: File,
    path: PathBuf,
}
impl Lease {
    pub fn create(dir: &Path) -> Result<Self> {
        let path = dir.join(RECORDING);
        let mut file = options().create_new(true).open(&path)?;
        let initialized = (|| -> Result<()> {
            lock(&file)?;
            file.write_all(b"apparent-pcm16-16000-v1\n")?;
            file.sync_all()?;
            File::open(dir)?.sync_all()?;
            Ok(())
        })();
        if let Err(error) = initialized {
            let _ = std::fs::remove_file(&path);
            return Err(error);
        }
        Ok(Self { file, path })
    }
    pub fn acquire(dir: &Path) -> Result<Self> {
        let path = dir.join(RECORDING);
        let file = options().open(&path)?;
        lock(&file).context("Narration is still owned by a live recording")?;
        let mut identity = String::new();
        (&file).read_to_string(&mut identity)?;
        ensure!(
            identity == "apparent-pcm16-16000-v1\n",
            "Unknown narration journal"
        );
        Ok(Self { file, path })
    }
    pub fn remove(&self) -> Result<()> {
        std::fs::remove_file(&self.path)?;
        File::open(self.path.parent().context("Missing narration parent")?)?.sync_all()?;
        Ok(())
    }
}
fn options() -> OpenOptions {
    let mut opts = OpenOptions::new();
    opts.read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC);
    }
    opts
}
fn lock(file: &File) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        ensure!(
            unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0,
            "{}",
            std::io::Error::last_os_error()
        );
    }
    Ok(())
}
pub fn live(dir: &Path) -> bool {
    match options().open(dir.join(RECORDING)) {
        Ok(file) => lock(&file).is_err(),
        Err(error) => error.kind() != std::io::ErrorKind::NotFound,
    }
}

pub struct Writer {
    stop: Option<std::sync::mpsc::Sender<()>>,
    worker: Option<std::thread::JoinHandle<Result<()>>>,
    path: PathBuf,
    lease: Arc<Lease>,
}
impl Writer {
    pub fn start(
        dir: &Path,
        samples: Arc<Mutex<Samples>>,
        error: Arc<Mutex<Option<String>>>,
        log: impl Fn(&str) + Send + 'static,
    ) -> Result<Self> {
        Self::start_inner(dir, samples, error, log).context(SaveFailure)
    }
    fn start_inner(
        dir: &Path,
        samples: Arc<Mutex<Samples>>,
        error: Arc<Mutex<Option<String>>>,
        log: impl Fn(&str) + Send + 'static,
    ) -> Result<Self> {
        let lease = Arc::new(Lease::create(dir)?);
        let path = dir.join("audio.wav");
        let mut created_audio = false;
        let initialized = (|| -> Result<_> {
            let mut file = options().create_new(true).open(&path)?;
            created_audio = true;
            file.write_all(&header(0))?;
            file.sync_all()?;
            File::open(dir)?.sync_all()?;
            let (stop, receiver) = std::sync::mpsc::channel();
            let worker_lease = Arc::clone(&lease);
            let worker = std::thread::Builder::new()
                .name("apparent-narration".into())
                .spawn(move || {
                    let _lease = worker_lease;
                    let result = write_stream(&mut file, &samples, receiver);
                    if let Err(ref failure) = result {
                        let message =
                            format!("Narration save failed: {failure:#}; saved prefix retained");
                        log(&message);
                        if let Ok(mut error) = error.lock() {
                            *error = Some(message);
                        }
                    }
                    result
                })?;
            Ok((stop, worker))
        })();
        match initialized {
            Ok((stop, worker)) => Ok(Self {
                stop: Some(stop),
                worker: Some(worker),
                path,
                lease,
            }),
            Err(error) => {
                // Input has not opened: these bytes contain no narration. Never delete an
                // existing healthy audio file that made create_new fail.
                if created_audio {
                    let _ = std::fs::remove_file(&path);
                }
                let _ = lease.remove();
                Err(error)
            }
        }
    }
    fn join(&mut self) -> Result<()> {
        self.stop.take();
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| anyhow!("Narration writer panicked"))??;
        }
        Ok(())
    }
    pub fn finish(&mut self) -> Result<(Vec<u8>, Arc<Lease>)> {
        self.join().context(SaveFailure)?;
        Ok((
            std::fs::read(&self.path).context(SaveFailure)?,
            Arc::clone(&self.lease),
        ))
    }
}
impl Drop for Writer {
    fn drop(&mut self) {
        let _ = self.join();
    }
}

/// The same bounded disk writer for a caller-owned, exclusively created file.
/// The capture owner must journal its identity and retain the directory/owner
/// permit; this type neither discovers paths nor creates a second lease.
pub struct DescriptorWriter {
    stop: Option<std::sync::mpsc::Sender<()>>,
    worker: Option<std::thread::JoinHandle<Result<File>>>,
}
impl DescriptorWriter {
    pub fn start(
        mut file: File,
        samples: Arc<Mutex<Samples>>,
        error: Arc<Mutex<Option<String>>>,
        initial_gap_ms: u64,
    ) -> Result<Self> {
        use std::os::unix::fs::MetadataExt;
        ensure!(
            initial_gap_ms <= super::timeline::MAX_INITIAL_GAP_MS,
            "Narration initial gap exceeds limit"
        );
        let metadata = file.metadata()?;
        ensure!(
            metadata.is_file()
                && metadata.len() == 0
                && metadata.nlink() == 1
                && metadata.mode() & 0o777 == 0o600
                && metadata.uid() == unsafe { libc::geteuid() },
            "Unsafe narration descriptor"
        );
        let initial_gap_samples = usize::try_from(initial_gap_ms)
            .ok()
            .and_then(|ms| ms.checked_mul(RATE / 1000))
            .context("Narration prefix overflow")?;
        ensure!(
            initial_gap_samples <= (u32::MAX as usize - 36) / 2,
            "Narration exceeds WAV size limit"
        );
        file.seek(SeekFrom::Start(0))?;
        file.write_all(&header(0))?;
        file.sync_all()?;
        let (stop, receiver) = std::sync::mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("apparent-narration".into())
            .spawn(move || {
                let result =
                    write_stream_aligned(&mut file, &samples, receiver, initial_gap_samples);
                if let Err(ref failure) = result {
                    if let Ok(mut error) = error.lock() {
                        *error = Some(format!("Narration save failed: {failure:#}"));
                    }
                }
                result?;
                Ok(file)
            })?;
        Ok(Self {
            stop: Some(stop),
            worker: Some(worker),
        })
    }

    /// Input must be stopped/drained before this call. No whole-WAV allocation
    /// or path reopening occurs; return the original descriptor for sealing.
    pub fn finish(mut self) -> Result<File> {
        self.stop.take();
        self.worker
            .take()
            .context("Narration writer already finished")?
            .join()
            .map_err(|_| anyhow!("Narration writer panicked"))?
    }
}
impl Drop for DescriptorWriter {
    fn drop(&mut self) {
        self.stop.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn write_stream(
    file: &mut File,
    samples: &Mutex<Samples>,
    receiver: std::sync::mpsc::Receiver<()>,
) -> Result<()> {
    write_stream_aligned(file, samples, receiver, 0)
}

fn write_stream_aligned(
    file: &mut File,
    samples: &Mutex<Samples>,
    receiver: std::sync::mpsc::Receiver<()>,
    initial_gap_samples: usize,
) -> Result<()> {
    let prefix_bytes = initial_gap_samples
        .checked_mul(2)
        .context("Narration prefix overflow")?;
    ensure!(
        prefix_bytes <= u32::MAX as usize - 36,
        "Narration exceeds WAV size limit"
    );
    // This runs on the disk worker, never the input callback. Do not put a long
    // admission gap into the bounded live history or count it as captured PCM.
    let zeroes = [0u8; 8192];
    let mut remaining = prefix_bytes;
    while remaining != 0 {
        let count = remaining.min(zeroes.len());
        file.write_all(&zeroes[..count])?;
        remaining -= count;
    }
    let mut written = 0;
    let mut synced = Instant::now();
    loop {
        let stopping = matches!(
            receiver.recv_timeout(POLL),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
        );
        let bytes: Vec<u8> = {
            let buffer = samples
                .lock()
                .map_err(|_| anyhow!("Narration buffer poisoned"))?;
            ensure!(
                buffer.len().saturating_sub(written) <= MAX_BACKLOG,
                "Narration disk writer fell more than two seconds behind"
            );
            buffer
                .get(written..)
                .context("Narration history expired")?
                .iter()
                .flat_map(|&s| super::pcm16(s).to_le_bytes())
                .collect()
        };
        let data_len = initial_gap_samples
            .checked_add(written)
            .context("Narration length overflow")?
            .checked_mul(2)
            .and_then(|n| n.checked_add(bytes.len()))
            .context("Narration length overflow")?;
        ensure!(
            data_len <= u32::MAX as usize - 36,
            "Narration exceeds WAV size limit"
        );
        file.write_all(&bytes)?;
        written += bytes.len() / 2;
        // File is unbuffered: every batch reaches the kernel, independent of header updates.
        if stopping || synced.elapsed() >= SYNC {
            checkpoint(file, data_len as u32)?;
            synced = Instant::now();
        }
        if stopping {
            return Ok(());
        }
    }
}
fn header(bytes: u32) -> Vec<u8> {
    let mut h =
        b"RIFF\0\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\x3e\0\0\0\x7d\0\0\x02\0\x10\0data\0\0\0\0"
            .to_vec();
    h[4..8].copy_from_slice(&(bytes + 36).to_le_bytes());
    h[40..44].copy_from_slice(&bytes.to_le_bytes());
    h
}
fn checkpoint(file: &mut File, bytes: u32) -> Result<()> {
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&header(bytes))?;
    file.flush()?;
    file.sync_all()?;
    file.seek(SeekFrom::End(0))?;
    Ok(())
}

/// Only our journalled canonical PCM is repairable. Never rewrite a healthy WAV.
/// Caller holds the lease throughout repair and recovery; raw movies remain excluded.
pub fn repair(dir: &Path, _lease: &Lease) -> Result<usize> {
    let mut file = options().open(dir.join("audio.wav"))?;
    let length = file.metadata()?.len();
    ensure!(
        length >= 44 && length <= u32::MAX as u64 + 8,
        "Invalid narration length"
    );
    let mut actual = [0; 44];
    file.read_exact(&mut actual)?;
    let expected = header(0);
    ensure!(
        actual[..4] == expected[..4] && actual[8..40] == expected[8..40],
        "Not a journalled PCM16 WAV"
    );
    let bytes = ((length - 44) / 2 * 2) as u32;
    let healthy = actual[4..8] == (bytes + 36).to_le_bytes()
        && actual[40..44] == bytes.to_le_bytes()
        && length == u64::from(bytes) + 44;
    if !healthy {
        file.set_len(u64::from(bytes) + 44)?;
        checkpoint(&mut file, bytes)?;
    }
    Ok(bytes as usize / 2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn descriptor_writer_keeps_inode_and_prefix_outside_live_history() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        // macOS temp_dir may use /var's alias. Normalize only this owned fixture;
        // production Store paths must still reject symlink ancestry.
        let path = std::env::temp_dir().canonicalize().unwrap().join(format!(
            "apparent-pinned-audio-{}",
            crate::new_id().unwrap()
        ));
        std::fs::create_dir(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let store = crate::SecureStore::open(&path).unwrap();
        let file = store.file("audio.wav", true).unwrap();
        let before = file.metadata().unwrap();
        let samples = Arc::new(Mutex::new(Samples::from(vec![0.5, -0.5, 1.0])));
        let error = Arc::new(Mutex::new(None));
        // More than the live history/backlog: prefix must be streamed to disk.
        let writer = DescriptorWriter::start(file, samples.clone(), error.clone(), 61_000).unwrap();
        let mut file = writer.finish().unwrap();
        assert_eq!(file.metadata().unwrap().ino(), before.ino());
        assert_eq!(samples.lock().unwrap().len(), 3);
        assert!(error.lock().unwrap().is_none());
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        let decoded = super::super::decode_wav_pcm16(&bytes).unwrap();
        assert_eq!(decoded.len(), 61_000 * 16 + 3);
        assert!(decoded[..61_000 * 16].iter().all(|v| *v == 0));
        assert_eq!(&decoded[61_000 * 16..], &[16384, -16384, 32767]);
        drop(file);
        drop(store);
        std::fs::remove_dir_all(path).unwrap();
    }
    struct Scratch(PathBuf);
    impl Scratch {
        fn new(name: &str) -> Self {
            let path = std::env::temp_dir()
                .join(format!("apparent-narration-{name}-{}", std::process::id()));
            std::fs::create_dir(&path).unwrap();
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
            }
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn tone(at: usize) -> f32 {
        ((at % 997) as f32 - 498.0) / 1000.0
    }
    fn wait_for(mut predicate: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !predicate() {
            assert!(Instant::now() < deadline, "synthetic writer deadline");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn long_recording_saves_expired_history_without_growing_callback_memory() {
        let dir = Scratch::new("long");
        let samples = Arc::new(Mutex::new(Samples::default()));
        let allocation = samples.lock().unwrap().data.as_ptr();
        let mut writer = Writer::start(&dir.0, samples.clone(), Arc::default(), |_| {}).unwrap();
        let mut produced = 0;
        for _ in 0..65 {
            samples
                .lock()
                .unwrap()
                .extend((produced..produced + RATE).map(tone));
            produced += RATE;
            wait_for(|| {
                std::fs::metadata(dir.0.join("audio.wav")).unwrap().len()
                    == (44 + produced * 2) as u64
            });
        }
        let (wav, lease) = writer.finish().unwrap();
        assert!(
            Lease::acquire(&dir.0).is_err(),
            "Stop must keep ownership through bundle finalization"
        );
        assert_eq!(samples.lock().unwrap().data.as_ptr(), allocation);
        assert_eq!(samples.lock().unwrap().recent().len(), HISTORY);
        let decoded = crate::audio::decode_wav_16k(&wav).unwrap();
        assert_eq!(decoded.len(), produced);
        for (at, sample) in decoded.iter().enumerate() {
            assert!((sample - tone(at)).abs() < 0.0001);
        }
        drop((writer, lease));
        assert!(Lease::acquire(&dir.0).is_ok());
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn kernel_enospc_is_a_save_failure_not_successful_silence() {
        let mut full = OpenOptions::new().write(true).open("/dev/full").unwrap();
        let samples = Mutex::new(Samples::from(vec![0.2; 160]));
        let (sender, receiver) = std::sync::mpsc::channel();
        drop(sender);
        let error = write_stream(&mut full, &samples, receiver).unwrap_err();
        assert_eq!(
            error
                .downcast_ref::<std::io::Error>()
                .unwrap()
                .raw_os_error(),
            Some(libc::ENOSPC)
        );
    }
    #[test]
    fn long_history_is_bounded_and_absolute_ranges_do_not_shift() {
        let mut samples = Samples::default();
        let allocation = samples.data.as_ptr();
        for at in 0..(3 * HISTORY + 137) {
            samples.push(at as f32);
        }
        assert_eq!(samples.data.as_ptr(), allocation);
        assert_eq!(samples.data.len(), 2 * HISTORY);
        assert_eq!(samples.len(), 3 * HISTORY + 137);
        assert!(samples.get(0..160).is_none());
        assert_eq!(
            samples.get(samples.len() - 160..).unwrap(),
            (samples.len() - 160..samples.len())
                .map(|at| at as f32)
                .collect::<Vec<_>>()
        );
        assert_eq!(samples.recent().len(), HISTORY);
    }
}
