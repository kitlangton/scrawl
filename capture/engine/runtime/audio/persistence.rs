//! Compatibility adapter; Core owns bounded PCM persistence and recovery primitives.
pub use apparent_capture_engine::audio::persistence::{
    Lease, RECORDING, Samples, SaveFailure, live, repair,
};
pub struct Writer(apparent_capture_engine::audio::persistence::Writer);
impl Writer {
    pub fn start(
        dir: &std::path::Path,
        samples: std::sync::Arc<std::sync::Mutex<Samples>>,
        error: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    ) -> anyhow::Result<Self> {
        apparent_capture_engine::audio::persistence::Writer::start(dir, samples, error, |message| {
            crate::capture::engine::runtime::apparent_log(message)
        })
        .map(Self)
    }
    pub fn finish(&mut self) -> anyhow::Result<(Vec<u8>, std::sync::Arc<Lease>)> {
        self.0.finish()
    }
}
#[cfg(test)]
use anyhow::Result;
#[cfg(test)]
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    ops::{Bound, RangeBounds},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
#[cfg(test)]
const RATE: usize = 16_000;
#[cfg(test)]
const HISTORY: usize = 60 * RATE;
#[cfg(test)]
const MAX_BACKLOG: usize = 2 * RATE;
#[cfg(test)]
const POLL: Duration = Duration::from_millis(100);
#[cfg(test)]
fn header(bytes: u32) -> Vec<u8> {
    let mut h =
        b"RIFF\0\0\0\0WAVEfmt \x10\0\0\0\x01\0\x01\0\x80\x3e\0\0\0\x7d\0\0\x02\0\x10\0data\0\0\0\0"
            .to_vec();
    h[4..8].copy_from_slice(&(bytes + 36).to_le_bytes());
    h[40..44].copy_from_slice(&bytes.to_le_bytes());
    h
}

#[cfg(test)]
#[path = "persistence_tests.rs"]
mod tests;
