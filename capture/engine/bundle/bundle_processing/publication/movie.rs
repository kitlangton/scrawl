//! Bounded ISO-BMFF validation, independent of optional platform video decoders.
//! Checks the complete container and video sample/chunk extents, not codec quality.
use anyhow::{Context, Result, ensure};
use std::{fs::File, os::unix::fs::FileExt};

#[derive(Clone, Copy)]
struct Atom {
    kind: [u8; 4],
    start: u64,
    end: u64,
}
fn read(file: &File, at: u64, n: usize) -> Result<Vec<u8>> {
    ensure!(
        n <= 64 * 1024 * 1024,
        "movie metadata exceeds validation budget"
    );
    let mut bytes = vec![0; n];
    file.read_exact_at(&mut bytes, at)?;
    Ok(bytes)
}
fn u32_at(bytes: &[u8], at: usize) -> Result<u32> {
    Ok(u32::from_be_bytes(
        bytes
            .get(at..at + 4)
            .context("truncated movie table")?
            .try_into()?,
    ))
}
fn atoms(file: &File, mut at: u64, end: u64) -> Result<Vec<Atom>> {
    let mut out = Vec::new();
    while at < end {
        ensure!(end - at >= 8, "truncated movie box");
        let header = read(file, at, 8)?;
        let short = u32_at(&header, 0)?;
        let (size, head) = match short {
            0 => (end - at, 8),
            1 => (
                u64::from_be_bytes(read(file, at + 8, 8)?.try_into().unwrap()),
                16,
            ),
            size => (u64::from(size), 8),
        };
        ensure!(size >= head && size <= end - at, "invalid movie box extent");
        out.push(Atom {
            kind: header[4..8].try_into().unwrap(),
            start: at + head,
            end: at + size,
        });
        ensure!(out.len() <= 4096, "too many movie boxes");
        at += size;
    }
    Ok(out)
}
fn child(file: &File, parent: Atom, kind: &[u8; 4]) -> Result<Atom> {
    let matches: Vec<_> = atoms(file, parent.start, parent.end)?
        .into_iter()
        .filter(|a| &a.kind == kind)
        .collect();
    ensure!(
        matches.len() == 1,
        "movie requires one {} box",
        String::from_utf8_lossy(kind)
    );
    Ok(matches[0])
}
fn data(file: &File, atom: Atom) -> Result<Vec<u8>> {
    read(file, atom.start, usize::try_from(atom.end - atom.start)?)
}

pub fn validate(file: &File) -> Result<()> {
    let top = atoms(file, 0, file.metadata()?.len())?;
    let moovs: Vec<_> = top.iter().filter(|a| &a.kind == b"moov").collect();
    ensure!(moovs.len() == 1, "movie has no unique completed moov");
    let mdats: Vec<_> = top
        .iter()
        .filter(|a| &a.kind == b"mdat" && a.end > a.start)
        .collect();
    ensure!(!mdats.is_empty(), "movie has no media payload");
    let moov = *moovs[0];
    ensure!(
        moov.end - moov.start <= 64 * 1024 * 1024,
        "movie metadata exceeds validation budget"
    );
    let mut videos = 0;
    for trak in atoms(file, moov.start, moov.end)?
        .into_iter()
        .filter(|a| &a.kind == b"trak")
    {
        let mdia = child(file, trak, b"mdia")?;
        let hdlr = data(file, child(file, mdia, b"hdlr")?)?;
        if hdlr.get(8..12) != Some(b"vide") {
            continue;
        }
        videos += 1;
        let stbl = child(file, child(file, mdia, b"minf")?, b"stbl")?;
        let stsd = child(file, stbl, b"stsd")?;
        let descriptions = data(file, stsd)?;
        ensure!(
            u32_at(&descriptions, 4)? > 0,
            "movie has no sample description"
        );
        let entries = atoms(file, stsd.start + 8, stsd.end)?;
        ensure!(
            entries.len() as u32 == u32_at(&descriptions, 4)?,
            "invalid sample description count"
        );
        for entry in &entries {
            let description = data(file, *entry)?;
            ensure!(
                description.len() >= 28 && description[6..8] == [0, 1],
                "external or invalid video sample description"
            );
            ensure!(
                description[24..26] != [0, 0] && description[26..28] != [0, 0],
                "empty movie dimensions"
            );
        }
        let sizes = data(file, child(file, stbl, b"stsz")?)?;
        let fixed = u32_at(&sizes, 4)?;
        let count = u32_at(&sizes, 8)? as usize;
        ensure!(
            count > 0 && count <= 10_000_000,
            "empty or oversized video sample table"
        );
        ensure!(
            sizes.len() == 12 + if fixed == 0 { count * 4 } else { 0 },
            "truncated video sizes"
        );
        let tables = atoms(file, stbl.start, stbl.end)?;
        let offsets: Vec<_> = tables
            .iter()
            .filter(|a| matches!(&a.kind, b"stco" | b"co64"))
            .collect();
        ensure!(offsets.len() == 1, "movie needs a unique chunk table");
        let wide = &offsets[0].kind == b"co64";
        let chunks = data(file, *offsets[0])?;
        let chunk_count = u32_at(&chunks, 4)? as usize;
        ensure!(
            chunk_count > 0
                && chunk_count <= count
                && chunks.len() == 8 + chunk_count * if wide { 8 } else { 4 },
            "invalid movie chunk table"
        );
        let mapping = data(file, child(file, stbl, b"stsc")?)?;
        let mapping_count = u32_at(&mapping, 4)? as usize;
        ensure!(
            mapping_count > 0
                && mapping_count <= chunk_count
                && mapping.len() == 8 + 12 * mapping_count,
            "invalid sample-to-chunk table"
        );
        let mut rules = Vec::new();
        for n in 0..mapping_count {
            let first = u32_at(&mapping, 8 + 12 * n)? as usize;
            let samples = u32_at(&mapping, 12 + 12 * n)? as usize;
            let desc = u32_at(&mapping, 16 + 12 * n)? as usize;
            ensure!(
                first >= 1
                    && first <= chunk_count
                    && samples > 0
                    && desc > 0
                    && desc <= entries.len(),
                "invalid sample mapping"
            );
            ensure!(
                rules.last().is_none_or(|&(prev, _)| first > prev),
                "unordered chunk mapping"
            );
            rules.push((first, samples));
        }
        ensure!(rules[0].0 == 1, "unmapped first movie chunk");
        let mut sample = 0;
        let mut rule = 0;
        for chunk in 0..chunk_count {
            while rule + 1 < rules.len() && rules[rule + 1].0 <= chunk + 1 {
                rule += 1;
            }
            let at = 8 + chunk * if wide { 8 } else { 4 };
            let offset = if wide {
                u64::from_be_bytes(chunks[at..at + 8].try_into()?)
            } else {
                u64::from(u32_at(&chunks, at)?)
            };
            let samples = rules[rule].1;
            ensure!(
                samples <= count - sample,
                "movie chunk exceeds sample count"
            );
            let mut size = 0u64;
            for n in sample..sample + samples {
                let len = if fixed == 0 {
                    u32_at(&sizes, 12 + n * 4)?
                } else {
                    fixed
                };
                ensure!(len > 0, "empty video sample");
                size = size
                    .checked_add(u64::from(len))
                    .context("movie sample overflow")?;
            }
            ensure!(
                mdats
                    .iter()
                    .any(|a| offset >= a.start && offset <= a.end && size <= a.end - offset),
                "video samples outside media payload"
            );
            sample += samples;
        }
        ensure!(sample == count, "unreferenced video samples");
        let timing = data(file, child(file, stbl, b"stts")?)?;
        let timing_count = u32_at(&timing, 4)? as usize;
        ensure!(
            timing_count > 0 && timing.len() == 8 + 8 * timing_count,
            "invalid video timing table"
        );
        let mut timed = 0u64;
        for n in 0..timing_count {
            ensure!(
                u32_at(&timing, 12 + 8 * n)? > 0,
                "zero video sample duration"
            );
            timed += u64::from(u32_at(&timing, 8 + 8 * n)?);
        }
        ensure!(timed == count as u64, "video timing/sample mismatch");
    }
    ensure!(videos > 0, "movie has no video track");
    Ok(())
}
