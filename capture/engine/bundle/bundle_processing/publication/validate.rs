use super::{
    fs::{self, Directory},
    movie,
};
use crate::capture::engine::bundle::diagnostics::{Diagnose, Step};
use crate::shared::model::ApparentBundle;
use anyhow::{Context, Result, ensure};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{File, Metadata},
    io::{BufReader, Seek, SeekFrom},
    path::Path,
};
pub(super) const TEXT_CAP: u64 = 64 * 1024 * 1024;
struct Artifact {
    file: File,
    metadata: Metadata,
}
pub(super) struct Inventory {
    files: BTreeMap<String, Artifact>,
    directories: BTreeMap<String, (Directory, Metadata)>,
}
impl Inventory {
    pub fn read(root: &Directory) -> Result<Self> {
        let mut result = Self {
            files: BTreeMap::new(),
            directories: BTreeMap::new(),
        };
        result.visit(root, "", 0)?;
        Ok(result)
    }
    fn visit(&mut self, root: &Directory, prefix: &str, depth: usize) -> Result<()> {
        ensure!(depth <= 8, "bundle nesting exceeds validation budget");
        for name in root.names()? {
            ensure!(
                !name.starts_with(".whiteboard-"),
                "unfinished whiteboard publication"
            );
            let path = format!("{prefix}{name}");
            fs::relative(&path)?;
            ensure!(
                self.files.len() + self.directories.len() < 4096,
                "bundle entry budget exceeded"
            );
            if root.is_directory(&name)? {
                let child = root.child(&name, false)?;
                let metadata = child.0.metadata()?;
                ensure!(
                    metadata.uid() == unsafe { libc::geteuid() } && metadata.mode() & 0o022 == 0,
                    "unsafe nested directory"
                );
                self.visit(&child, &format!("{path}/"), depth + 1)?;
                self.directories.insert(path, (child, metadata));
            } else {
                let file = root.leaf(&name, false)?;
                let metadata = file.metadata()?;

                ensure!(
                    metadata.len() <= 64 * 1024 * 1024 * 1024,
                    "bundle file exceeds validation budget"
                );
                self.files.insert(path, Artifact { file, metadata });
            }
        }
        Ok(())
    }
    fn file(&self, path: &str) -> Result<&File> {
        fs::relative(path)?;
        Ok(&self
            .files
            .get(path)
            .with_context(|| format!("missing required bundle artifact: {path}"))?
            .file)
    }
    pub fn validate(
        &self,
        path: &Path,
        expected: &ApparentBundle,
        summary: &str,
    ) -> Result<Vec<u8>> {
        ensure!(
            expected.version == 1 && !expected.id.is_empty(),
            "unsupported bundle identity"
        );
        ensure!(
            path.file_stem().and_then(|s| s.to_str()) == Some(expected.id.as_str()),
            "bundle ID does not match directory"
        );
        let manifest = fs::bytes(self.file("apparent.json")?, TEXT_CAP)?;
        let value: serde_json::Value = serde_json::from_slice(&manifest)?;
        let expected_value =
            serde_json::from_slice::<serde_json::Value>(&serde_json::to_vec(expected)?)?;
        ensure!(
            value == expected_value,
            "persisted manifest differs from final bundle"
        );
        let _: ApparentBundle = serde_json::from_slice(&manifest)?;
        ensure!(
            fs::bytes(self.file("summary.md")?, TEXT_CAP)? == summary.as_bytes(),
            "persisted summary differs from final summary"
        );
        ensure!(!expected.keyframes.is_empty(), "bundle has no keyframes");
        let mut indices = BTreeSet::new();
        let mut images = BTreeSet::from([expected.contact_sheet.clone()]);
        for frame in &expected.keyframes {
            ensure!(
                frame.index > 0 && indices.insert(frame.index),
                "duplicate or zero keyframe index"
            );
            images.insert(frame.file.clone());
        }
        if let Some(crop) = &expected.autocrop {
            fs::relative(&crop.full_frames)?;
            ensure!(
                self.directories.contains_key(&crop.full_frames),
                "missing full-frame directory"
            );
            ensure!(
                crop.crops.len() == expected.keyframes.len(),
                "incomplete crop metadata"
            );
            images.insert(crop.full_contact_sheet.clone());
            for frame in &expected.keyframes {
                let leaf = Path::new(&frame.file)
                    .file_name()
                    .and_then(|p| p.to_str())
                    .context("bad full-frame reference")?;
                images.insert(format!("{}/{leaf}", crop.full_frames));
            }
        }
        for image in &images {
            self.file(image)
                .and_then(validate_png)
                .with_context(|| format!("invalid PNG artifact: {image}"))
                .diagnostic(Step::PngValidate)?;
        }
        if let Some(audio) = &expected.audio {
            self.file(audio)
                .and_then(validate_wav)
                .context("invalid WAV artifact")
                .diagnostic(Step::WavValidate)?;
        }
        if let Some(video) = &expected.video {
            self.file(video)
                .and_then(movie::validate)
                .context("invalid movie artifact")
                .diagnostic(Step::MovieValidate)?;
        }
        // Validate retained originals and auxiliary media too, not only the displayed subset.
        for (name, artifact) in &self.files {
            match Path::new(name).extension().and_then(|s| s.to_str()) {
                Some("png") if !images.contains(name) => validate_png(&artifact.file)
                    .with_context(|| format!("invalid retained PNG: {name}"))
                    .diagnostic(Step::PngValidate)?,
                Some("wav") if expected.audio.as_deref() != Some(name) => {
                    validate_wav(&artifact.file)
                        .with_context(|| format!("invalid retained WAV: {name}"))
                        .diagnostic(Step::WavValidate)?
                }
                Some("mp4" | "mov") if expected.video.as_deref() != Some(name) => {
                    movie::validate(&artifact.file)
                        .with_context(|| format!("invalid retained movie: {name}"))
                        .diagnostic(Step::MovieValidate)?
                }
                _ => (),
            }
        }
        if let Some(journal) = self
            .files
            .get(crate::capture::engine::bundle::recover::UNFINISHED)
        {
            let bytes = fs::bytes(&journal.file, TEXT_CAP)?;
            let _: crate::capture::engine::bundle::recover::Unfinished =
                serde_json::from_slice(&bytes)?;
            let old: serde_json::Value = serde_json::from_slice(&bytes)?;
            ensure!(
                old["videoOffsetMs"].as_u64().unwrap_or(0) == expected.video_offset_ms,
                "recovery video offset differs from final bundle"
            );
            for field in [
                "target",
                "durationMs",
                "video",
                "strokes",
                "events",
                "note",
                "interactionTelemetry",
            ] {
                ensure!(
                    old[field] == expected_value[field],
                    "recovery journal differs from final bundle: {field}"
                );
            }
        }
        Ok(manifest)
    }
    pub fn sync_and_check(&self, root: &Directory) -> Result<()> {
        for (name, artifact) in &self.files {
            ensure!(
                fs::same(&artifact.metadata, &artifact.file.metadata()?),
                "artifact changed during validation: {name}"
            );
            ensure!(
                fs::same(&artifact.metadata, &root.file(name)?.metadata()?),
                "artifact replaced during validation: {name}"
            );
            artifact.file.sync_all()?;
        }
        // Deepest directories first, then the root. Check identities and the exact
        // complete inventory again, catching inserted/unlinked/replaced nested entries.
        for (name, (dir, before)) in self.directories.iter().rev() {
            let mut current = Directory(root.0.try_clone()?);
            for part in fs::relative(name)? {
                current = current.child(part, false)?;
            }
            ensure!(
                fs::same(before, &current.0.metadata()?) && fs::same(before, &dir.0.metadata()?),
                "nested directory changed during publication"
            );
            dir.0.sync_all()?;
        }
        let current = Self::read(root)?;
        ensure!(
            self.files.keys().eq(current.files.keys())
                && self.directories.keys().eq(current.directories.keys()),
            "bundle inventory changed during publication"
        );
        for (name, artifact) in &self.files {
            ensure!(
                fs::same(&artifact.metadata, &current.files[name].metadata),
                "artifact changed before publication: {name}"
            );
        }
        root.0.sync_all()?;
        Ok(())
    }
}
use std::os::unix::fs::MetadataExt;

fn validate_png(file: &File) -> Result<()> {
    ensure!(
        file.metadata()?.len() <= 512 * 1024 * 1024,
        "PNG exceeds validation budget"
    );
    let mut file = file.try_clone()?;
    file.seek(SeekFrom::Start(0))?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_limits(png::Limits {
        bytes: 512 * 1024 * 1024,
    });
    let mut reader = decoder.read_info()?;
    let info = reader.info();
    ensure!(
        info.width > 0
            && info.height > 0
            && u64::from(info.width) * u64::from(info.height) <= 128 * 1024 * 1024,
        "invalid or oversized PNG dimensions"
    );
    let size = reader
        .output_buffer_size()
        .context("PNG output exceeds validation budget")?;
    ensure!(
        size <= 512 * 1024 * 1024,
        "PNG output exceeds validation budget"
    );
    let mut pixels = vec![0; size];
    reader.next_frame(&mut pixels)?;
    reader.finish()?;
    Ok(())
}
fn validate_wav(file: &File) -> Result<()> {
    use std::os::unix::fs::FileExt;
    let mut header = [0; 12];
    file.read_exact_at(&mut header, 0)?;
    ensure!(
        &header[..4] == b"RIFF" && &header[8..] == b"WAVE",
        "not a WAV file"
    );
    ensure!(
        u64::from(u32::from_le_bytes(header[4..8].try_into()?)) + 8 == file.metadata()?.len(),
        "truncated WAV container"
    );
    let mut file = file.try_clone()?;
    file.seek(SeekFrom::Start(0))?;
    let mut reader = hound::WavReader::new(BufReader::new(file))?;
    let spec = reader.spec();
    ensure!(
        spec.channels > 0 && spec.sample_rate > 0,
        "invalid WAV format"
    );
    let count = reader.len();
    let mut decoded = 0u32;
    match spec.sample_format {
        hound::SampleFormat::Int => {
            for sample in reader.samples::<i32>() {
                sample?;
                decoded += 1;
            }
        }
        hound::SampleFormat::Float => {
            for sample in reader.samples::<f32>() {
                ensure!(sample?.is_finite(), "nonfinite WAV sample");
                decoded += 1;
            }
        }
    }
    ensure!(decoded == count, "truncated WAV samples");
    Ok(())
}
