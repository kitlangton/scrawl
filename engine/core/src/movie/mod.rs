//! Bounded ISO-BMFF validation, independent of optional platform video decoders.
//! Checks the complete container and video sample/chunk extents, not codec quality.
use anyhow::{Context, Result, ensure};
use std::{fs::File, os::unix::fs::FileExt};

#[derive(Clone, Copy)]
struct Atom {
    kind: [u8; 4],
    box_start: u64,
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
fn u16_at(bytes: &[u8], at: usize) -> Result<u16> {
    Ok(u16::from_be_bytes(
        bytes
            .get(at..at + 2)
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
            box_start: at,
            start: at + head,
            end: at + size,
        });
        ensure!(out.len() <= 4096, "too many movie boxes");
        at += size;
    }
    Ok(out)
}
#[cfg(feature = "publication")]
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

const FRAGMENT_METADATA_BUDGET: u64 = 64 * 1024 * 1024;
const FRAGMENT_BOX_BUDGET: usize = 16_384;
const FRAGMENT_COUNT_BUDGET: usize = 1_024;
const FRAGMENT_SAMPLE_BUDGET: u64 = 10_000_000;
const TFHD_FRAGMENT_RELATIVE_FLAGS: u32 = 0x020038;
const TRUN_FRAGMENT_FLAGS: u32 = 0x000e01;
const NATIVE_DEFAULT_SAMPLE_FLAGS: u32 = 0x01010000;

#[derive(Default)]
struct FragmentBudget {
    metadata_bytes: u64,
    box_count: usize,
    sample_count: u64,
}

fn fragment_atoms(
    file: &File,
    at: u64,
    end: u64,
    budget: &mut FragmentBudget,
) -> Result<Vec<Atom>> {
    let children = atoms(file, at, end)?;
    budget.box_count = budget
        .box_count
        .checked_add(children.len())
        .context("fragment box count overflow")?;
    ensure!(
        budget.box_count <= FRAGMENT_BOX_BUDGET,
        "fragment box work exceeds budget"
    );
    Ok(children)
}

fn fragment_data(file: &File, atom: Atom, budget: &mut FragmentBudget) -> Result<Vec<u8>> {
    let len = atom.end - atom.start;
    budget.metadata_bytes = budget
        .metadata_bytes
        .checked_add(len)
        .context("fragment metadata length overflow")?;
    ensure!(
        budget.metadata_bytes <= FRAGMENT_METADATA_BUDGET,
        "fragment metadata exceeds validation budget"
    );
    data(file, atom)
}

fn fullbox(bytes: &[u8], version: u8, flags: u32) -> Result<()> {
    ensure!(bytes.len() >= 4, "truncated fragment full box");
    ensure!(
        bytes[0] == version && u32_at(bytes, 0)? & 0x00ff_ffff == flags,
        "unsupported fragment box version or flags"
    );
    Ok(())
}

fn exact_children(
    file: &File,
    parent: Atom,
    kinds: &[[u8; 4]],
    budget: &mut FragmentBudget,
) -> Result<Vec<Atom>> {
    let children = fragment_atoms(file, parent.start, parent.end, budget)?;
    ensure!(
        children.len() == kinds.len()
            && children
                .iter()
                .zip(kinds)
                .all(|(atom, kind)| &atom.kind == kind),
        "unsupported fragmented movie box layout"
    );
    Ok(children)
}

fn empty_table(file: &File, atom: Atom, budget: &mut FragmentBudget) -> Result<()> {
    let bytes = fragment_data(file, atom, budget)?;
    ensure!(
        bytes.len() == 8,
        "fragment initialization table must be empty"
    );
    fullbox(&bytes, 0, 0)?;
    ensure!(
        u32_at(&bytes, 4)? == 0,
        "fragment initialization table has samples"
    );
    Ok(())
}

fn fragmented_track(file: &File, moov: Atom, budget: &mut FragmentBudget) -> Result<(u32, u32)> {
    let moov_children = exact_children(
        file,
        moov,
        [*b"mvhd", *b"trak", *b"mvex"].as_slice(),
        budget,
    )?;
    let mvhd = fragment_data(file, moov_children[0], budget)?;
    ensure!(mvhd.len() >= 20, "truncated fragmented movie header");
    fullbox(&mvhd, 0, 0)?;
    ensure!(
        u32_at(&mvhd, 12)? != 0,
        "fragmented movie timescale is zero"
    );
    let trak = moov_children[1];
    let trak_children = exact_children(file, trak, [*b"tkhd", *b"mdia"].as_slice(), budget)?;
    let tkhd = fragment_data(file, trak_children[0], budget)?;
    ensure!(tkhd.len() >= 24, "truncated fragmented track header");
    fullbox(&tkhd, 0, 1)?;
    let track_id = u32_at(&tkhd, 12)?;
    ensure!(track_id != 0, "fragmented movie has an invalid track ID");

    let mdia = trak_children[1];
    let mdia_children = exact_children(
        file,
        mdia,
        [*b"mdhd", *b"hdlr", *b"minf"].as_slice(),
        budget,
    )?;
    let mdhd = fragment_data(file, mdia_children[0], budget)?;
    ensure!(mdhd.len() >= 16, "truncated fragmented media header");
    fullbox(&mdhd, 0, 0)?;
    let timescale = u32_at(&mdhd, 12)?;
    ensure!(timescale == 600, "unsupported fragmented video timescale");
    let hdlr = fragment_data(file, mdia_children[1], budget)?;
    fullbox(&hdlr, 0, 0)?;
    ensure!(
        hdlr.get(8..12) == Some(b"vide"),
        "fragmented movie must contain only video"
    );

    let minf = mdia_children[2];
    let minf_children = exact_children(
        file,
        minf,
        [*b"vmhd", *b"dinf", *b"stbl"].as_slice(),
        budget,
    )?;
    let vmhd = fragment_data(file, minf_children[0], budget)?;
    ensure!(vmhd.len() == 12, "invalid fragmented video media header");
    fullbox(&vmhd, 0, 1)?;
    let dinf = minf_children[1];
    let dref = exact_children(file, dinf, [*b"dref"].as_slice(), budget)?[0];
    let dref_bytes = fragment_data(file, dref, budget)?;
    fullbox(&dref_bytes, 0, 0)?;
    ensure!(
        u32_at(&dref_bytes, 4)? == 1,
        "fragmented movie requires one local data reference"
    );
    let references = fragment_atoms(file, dref.start + 8, dref.end, budget)?;
    ensure!(
        references.len() == 1 && references[0].kind == *b"url ",
        "unsupported fragmented data reference"
    );
    let url = fragment_data(file, references[0], budget)?;
    fullbox(&url, 0, 1)?;
    ensure!(url.len() == 4, "invalid self-contained data reference");

    let stbl = minf_children[2];
    let tables = exact_children(
        file,
        stbl,
        [*b"stsd", *b"stts", *b"stsc", *b"stsz", *b"stco"].as_slice(),
        budget,
    )?;
    let stsd = fragment_data(file, tables[0], budget)?;
    fullbox(&stsd, 0, 0)?;
    ensure!(
        u32_at(&stsd, 4)? == 1,
        "fragmented movie requires one video sample description"
    );
    let descriptions = fragment_atoms(file, tables[0].start + 8, tables[0].end, budget)?;
    ensure!(
        descriptions.len() == 1 && descriptions[0].kind == *b"avc1",
        "unsupported fragmented video sample description"
    );
    let description = fragment_data(file, descriptions[0], budget)?;
    ensure!(
        description.len() >= 78 && u16_at(&description, 6)? == 1,
        "external or invalid fragmented sample description"
    );
    ensure!(
        u16_at(&description, 24)? != 0 && u16_at(&description, 26)? != 0,
        "empty fragmented movie dimensions"
    );
    let sample_entry_boxes = fragment_atoms(
        file,
        descriptions[0].start + 78,
        descriptions[0].end,
        budget,
    )?;
    ensure!(
        sample_entry_boxes
            .iter()
            .filter(|atom| atom.kind == *b"avcC")
            .count()
            == 1,
        "fragmented AVC description lacks one configuration"
    );
    let avcc = sample_entry_boxes
        .iter()
        .find(|atom| atom.kind == *b"avcC")
        .unwrap();
    ensure!(
        fragment_data(file, *avcc, budget)?.first() == Some(&1),
        "unsupported AVC configuration version"
    );
    for atom in &sample_entry_boxes {
        ensure!(
            matches!(&atom.kind, b"avcC" | b"colr" | b"fiel" | b"chrm"),
            "unsupported fragmented AVC sample entry box"
        );
    }
    for table in [&tables[1], &tables[2], &tables[4]] {
        empty_table(file, *table, budget)?;
    }
    let sizes = fragment_data(file, tables[3], budget)?;
    ensure!(
        sizes.len() == 12,
        "fragment initialization sizes must be empty"
    );
    fullbox(&sizes, 0, 0)?;
    ensure!(
        u32_at(&sizes, 4)? == 0 && u32_at(&sizes, 8)? == 0,
        "fragment initialization sizes contain samples"
    );

    let mvex = moov_children[2];
    let trex = exact_children(file, mvex, [*b"trex"].as_slice(), budget)?[0];
    let defaults = fragment_data(file, trex, budget)?;
    ensure!(defaults.len() == 24, "invalid fragmented track defaults");
    fullbox(&defaults, 0, 0)?;
    ensure!(
        u32_at(&defaults, 4)? == track_id
            && u32_at(&defaults, 8)? == 1
            && u32_at(&defaults, 12)? == 0
            && u32_at(&defaults, 16)? == 0
            && u32_at(&defaults, 20)? == 0,
        "unsupported fragmented track defaults"
    );
    Ok((track_id, timescale))
}

fn validate_fragment(
    file: &File,
    moof: Atom,
    mdat: Atom,
    expected_sequence: u32,
    expected_decode_time: u64,
    track_id: u32,
    budget: &mut FragmentBudget,
) -> Result<u64> {
    budget.box_count = budget
        .box_count
        .checked_add(1)
        .context("fragment box count overflow")?;
    budget.metadata_bytes = budget
        .metadata_bytes
        .checked_add((moof.end - moof.start) + (mdat.start - mdat.box_start))
        .context("fragment metadata length overflow")?;
    ensure!(
        budget.box_count <= FRAGMENT_BOX_BUDGET,
        "fragment box work exceeds budget"
    );
    ensure!(
        budget.metadata_bytes <= FRAGMENT_METADATA_BUDGET,
        "fragment metadata exceeds validation budget"
    );
    ensure!(mdat.end > mdat.start, "fragment has an empty media payload");

    let moof_children = exact_children(file, moof, [*b"mfhd", *b"traf"].as_slice(), budget)?;
    let mfhd = fragment_data(file, moof_children[0], budget)?;
    ensure!(mfhd.len() == 8, "invalid movie fragment header");
    fullbox(&mfhd, 0, 0)?;
    ensure!(
        u32_at(&mfhd, 4)? == expected_sequence,
        "unordered movie fragment sequence"
    );

    let traf = moof_children[1];
    let traf_children = exact_children(
        file,
        traf,
        [*b"tfhd", *b"tfdt", *b"trun"].as_slice(),
        budget,
    )?;
    let tfhd = fragment_data(file, traf_children[0], budget)?;
    ensure!(tfhd.len() == 20, "invalid movie track fragment header");
    fullbox(&tfhd, 0, TFHD_FRAGMENT_RELATIVE_FLAGS)?;
    ensure!(u32_at(&tfhd, 4)? == track_id, "fragment track mismatch");
    let sample_duration = u32_at(&tfhd, 8)?;
    ensure!(
        sample_duration == 20 && u32_at(&tfhd, 12)? > 0,
        "fragment has invalid sample defaults"
    );
    ensure!(
        u32_at(&tfhd, 16)? == NATIVE_DEFAULT_SAMPLE_FLAGS,
        "unsupported fragment sample flags"
    );

    let tfdt = fragment_data(file, traf_children[1], budget)?;
    ensure!(tfdt.len() == 12, "invalid track fragment decode time");
    fullbox(&tfdt, 1, 0)?;
    let decode_time = u64::from_be_bytes(tfdt[4..12].try_into()?);
    ensure!(
        decode_time == expected_decode_time,
        "fragment decode timeline is discontinuous"
    );

    let trun_atom = traf_children[2];
    let trun = fragment_data(file, trun_atom, budget)?;
    ensure!(trun.len() >= 12, "truncated track fragment run");
    fullbox(&trun, 1, TRUN_FRAGMENT_FLAGS)?;
    let samples = u32_at(&trun, 4)? as u64;
    ensure!(samples > 0, "empty track fragment run");
    budget.sample_count = budget
        .sample_count
        .checked_add(samples)
        .context("fragment sample count overflow")?;
    ensure!(
        budget.sample_count <= FRAGMENT_SAMPLE_BUDGET,
        "fragment sample work exceeds budget"
    );
    let run_bytes = samples
        .checked_mul(12)
        .and_then(|bytes| bytes.checked_add(12))
        .context("fragment run length overflow")?;
    ensure!(
        run_bytes == trun.len() as u64,
        "invalid track fragment run length"
    );
    let data_offset = i64::from(i32::from_be_bytes(trun[8..12].try_into()?));
    let mut sample_at = moof
        .box_start
        .checked_add_signed(data_offset)
        .context("fragment data offset overflow")?;
    ensure!(
        sample_at == mdat.start,
        "fragment data offset does not start at its media payload"
    );
    // This supported native profile has one fixed-cadence video run per fragment.
    // Reordered H.264 samples may permute presentation slots, but cannot duplicate,
    // omit or move a slot outside this fragment. The global sample budget also bounds
    // this temporary occupancy vector; decode times remain checked below.
    let mut presentation_slots = vec![false; usize::try_from(samples)?];
    let mut time = decode_time;
    for index in 0..usize::try_from(samples)? {
        let at = 12 + index * 12;
        let size = u64::from(u32_at(&trun, at)?);
        let sample_flags = u32_at(&trun, at + 4)?;
        ensure!(
            sample_flags & 0xfc00_0000 == 0,
            "unsupported reserved sample flags"
        );
        let composition_offset = i64::from(i32::from_be_bytes(trun[at + 8..at + 12].try_into()?));
        ensure!(size > 0, "empty fragmented video sample");
        let presentation_time = i128::from(time) + i128::from(composition_offset);
        ensure!(
            presentation_time >= 0 && presentation_time <= i128::from(u64::MAX),
            "fragment composition time overflow"
        );
        let relative_presentation = u64::try_from(presentation_time)?
            .checked_sub(decode_time)
            .context("fragment presentation precedes its decode interval")?;
        let duration = u64::from(sample_duration);
        ensure!(
            relative_presentation % duration == 0,
            "fragment presentation is off its declared sample cadence"
        );
        let slot = usize::try_from(relative_presentation / duration)?;
        ensure!(
            slot < presentation_slots.len(),
            "fragment presentation exceeds its completed decode interval"
        );
        ensure!(
            !presentation_slots[slot],
            "duplicate fragment presentation slot"
        );
        presentation_slots[slot] = true;
        let sample_end = sample_at
            .checked_add(size)
            .context("fragment sample extent overflow")?;
        ensure!(
            sample_at >= mdat.start && sample_end <= mdat.end,
            "fragment sample outside its media payload"
        );
        sample_at = sample_end;
        time = time
            .checked_add(u64::from(sample_duration))
            .context("fragment decode time overflow")?;
    }
    ensure!(sample_at == mdat.end, "unreferenced fragment media bytes");
    Ok(time)
}

fn validate_fragmented_movie(file: &File, top: &[Atom]) -> Result<u64> {
    ensure!(
        top.len() >= 4 && top[0].kind == *b"ftyp" && top[1].kind == *b"moov",
        "invalid fragmented movie initialization segment"
    );
    ensure!(
        (top.len() - 2) % 2 == 0,
        "truncated fragmented movie segment"
    );
    let mut budget = FragmentBudget {
        box_count: top.len(),
        ..FragmentBudget::default()
    };
    ensure!(
        budget.box_count <= FRAGMENT_BOX_BUDGET,
        "fragment box work exceeds budget"
    );
    let ftyp = fragment_data(file, top[0], &mut budget)?;
    ensure!(
        ftyp.as_slice() == b"iso5\x00\x00\x00\x01isomiso5hlsf",
        "unsupported fragmented movie file type"
    );
    let moov = top[1];
    ensure!(
        moov.end - moov.start <= FRAGMENT_METADATA_BUDGET,
        "fragment initialization metadata exceeds budget"
    );
    let (track_id, timescale) = fragmented_track(file, moov, &mut budget)?;
    let mut expected_decode_time = 0;
    let fragment_count = (top.len() - 2) / 2;
    ensure!(
        fragment_count <= FRAGMENT_COUNT_BUDGET,
        "fragment count exceeds validation budget"
    );
    for (index, pair) in top[2..].chunks_exact(2).enumerate() {
        ensure!(
            pair[0].kind == *b"moof" && pair[1].kind == *b"mdat",
            "unsupported fragmented movie segment layout"
        );
        expected_decode_time = validate_fragment(
            file,
            pair[0],
            pair[1],
            u32::try_from(index + 1)?,
            expected_decode_time,
            track_id,
            &mut budget,
        )?;
    }
    expected_decode_time
        .checked_mul(1000)
        .and_then(|value| value.checked_add(u64::from(timescale) - 1))
        .map(|value| value / u64::from(timescale))
        .context("fragmented movie duration overflow")
}

/// Complete duration for the supported native fragmented profile, not a provenance claim.
/// GStreamer can initially report only its first fragment. Other formats retain their
/// existing decoder path; matching bytes still undergo the full bounded fragment validation.
#[cfg(all(target_os = "linux", feature = "linux-media"))]
pub(crate) fn native_fragment_duration_ms(file: &File) -> Result<Option<u64>> {
    if file.metadata()?.len() < 28 {
        return Ok(None);
    }
    let header = read(file, 0, 28)?;
    if &header[4..8] != b"ftyp" || &header[8..28] != b"iso5\x00\x00\x00\x01isomiso5hlsf" {
        return Ok(None);
    }
    let top = atoms(file, 0, file.metadata()?.len())?;
    validate_fragmented_movie(file, &top).map(Some)
}

#[cfg(feature = "publication")]
pub fn validate_movie(file: &File) -> Result<()> {
    let top = atoms(file, 0, file.metadata()?.len())?;
    if top.iter().any(|atom| atom.kind == *b"moof") {
        return validate_fragmented_movie(file, &top).map(|_| ());
    }
    validate_completed_movie(file, &top)
}

#[cfg(feature = "publication")]
fn validate_completed_movie(file: &File, top: &[Atom]) -> Result<()> {
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
