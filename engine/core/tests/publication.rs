#![cfg(feature = "publication")]

use std::fs::File;

#[test]
fn extracted_movie_validator_accepts_real_mp4_and_mov_fixtures() {
    for fixture in ["tiny.mp4", "tiny.mov"] {
        let path = format!("{}/tests/fixtures/{fixture}", env!("CARGO_MANIFEST_DIR"));
        let file = File::open(path).unwrap();
        apparent_capture_engine::publication::validate_movie(&file).unwrap();
    }
}

#[test]
fn extracted_movie_validator_accepts_the_bounded_native_fragment_profile() {
    let path = format!(
        "{}/tests/fixtures/fragmented.mp4",
        env!("CARGO_MANIFEST_DIR")
    );
    let file = File::open(path).unwrap();
    apparent_capture_engine::publication::validate_movie(&file).unwrap();
}

#[test]
fn extracted_movie_validator_rejects_truncated_container() {
    let path = format!("{}/tests/fixtures/tiny.mp4", env!("CARGO_MANIFEST_DIR"));
    let bytes = std::fs::read(path).unwrap();
    let movie = std::env::temp_dir().join(format!(
        "apparent-core-publication-truncated-{}-{}.mp4",
        std::process::id(),
        std::thread::current().name().unwrap_or("test")
    ));
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&movie)
        .unwrap();
    use std::io::Write;
    file.write_all(&bytes[..bytes.len() - 1]).unwrap();
    drop(file);
    let file = File::open(&movie).unwrap();
    assert!(apparent_capture_engine::publication::validate_movie(&file).is_err());
    drop(file);
    std::fs::remove_file(movie).unwrap();
}

// Returns the fourcc position, not the box or full-box payload start. The full-box
// payload begins four bytes later; mutations below use field offsets from this position.
fn first_fourcc(bytes: &[u8], kind: &[u8; 4]) -> usize {
    bytes.windows(4).position(|window| window == kind).unwrap()
}

fn top_level_box_start(bytes: &[u8], kind: &[u8; 4], occurrence: usize) -> usize {
    let mut at = 0;
    let mut found = 0;
    while at + 8 <= bytes.len() {
        let size = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        assert!(size >= 8 && size <= bytes.len() - at);
        if &bytes[at + 4..at + 8] == kind {
            if found == occurrence {
                return at;
            }
            found += 1;
        }
        at += size;
    }
    panic!(
        "fixture lacks top-level {} box #{occurrence}",
        String::from_utf8_lossy(kind)
    );
}

fn validate_mutated_fragment(label: &str, bytes: &[u8]) -> bool {
    use std::io::Write;

    let path = std::env::temp_dir().join(format!(
        "apparent-core-fragment-{}-{label}.mp4",
        std::process::id()
    ));
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    output.write_all(bytes).unwrap();
    drop(output);
    let file = File::open(&path).unwrap();
    let rejected = apparent_capture_engine::publication::validate_movie(&file).is_err();
    drop(file);
    std::fs::remove_file(path).unwrap();
    rejected
}

#[test]
fn fragmented_movie_rejects_incomplete_or_unsupported_structure() {
    let path = format!(
        "{}/tests/fixtures/fragmented.mp4",
        env!("CARGO_MANIFEST_DIR")
    );
    let bytes = std::fs::read(path).unwrap();
    let mut cases = Vec::new();

    let mut missing_mdat = bytes.clone();
    let first_mdat = top_level_box_start(&missing_mdat, b"mdat", 0);
    missing_mdat[first_mdat + 4..first_mdat + 8].copy_from_slice(b"free");
    cases.push(("missing-mdat", missing_mdat));

    cases.push(("truncated-mdat", bytes[..bytes.len() - 1].to_vec()));

    let mut invalid_track = bytes.clone();
    let tfhd = first_fourcc(&invalid_track, b"tfhd");
    invalid_track[tfhd + 8..tfhd + 12].copy_from_slice(&2u32.to_be_bytes());
    cases.push(("invalid-track", invalid_track));

    let mut invalid_default = bytes.clone();
    let tfhd = first_fourcc(&invalid_default, b"tfhd");
    invalid_default[tfhd + 12..tfhd + 16].copy_from_slice(&0u32.to_be_bytes());
    cases.push(("invalid-default", invalid_default));

    let mut invalid_offset = bytes.clone();
    let trun = first_fourcc(&invalid_offset, b"trun");
    invalid_offset[trun + 12..trun + 16].copy_from_slice(&i32::MAX.to_be_bytes());
    cases.push(("invalid-offset", invalid_offset));

    let mut offset_underflow = bytes.clone();
    let trun = first_fourcc(&offset_underflow, b"trun");
    offset_underflow[trun + 12..trun + 16].copy_from_slice(&i32::MIN.to_be_bytes());
    cases.push(("offset-underflow", offset_underflow));

    let mut composition_overflow = bytes.clone();
    let trun = first_fourcc(&composition_overflow, b"trun");
    composition_overflow[trun + 24..trun + 28].copy_from_slice(&i32::MIN.to_be_bytes());
    cases.push(("composition-overflow", composition_overflow));

    for (label, offset) in [
        ("composition-outside-fragment", i32::MAX),
        ("composition-duplicate-slot", 20),
        ("composition-off-cadence", 1),
    ] {
        let mut shifted = bytes.clone();
        let trun = first_fourcc(&shifted, b"trun");
        shifted[trun + 24..trun + 28].copy_from_slice(&offset.to_be_bytes());
        cases.push((label, shifted));
    }

    let mut invalid_count = bytes.clone();
    let trun = first_fourcc(&invalid_count, b"trun");
    invalid_count[trun + 8..trun + 12].copy_from_slice(&u32::MAX.to_be_bytes());
    cases.push(("invalid-count", invalid_count));

    let mut decode_overflow = bytes.clone();
    let tfdt = first_fourcc(&decode_overflow, b"tfdt");
    decode_overflow[tfdt + 8..tfdt + 16].copy_from_slice(&u64::MAX.to_be_bytes());
    cases.push(("max-decode-time", decode_overflow));

    let moof = top_level_box_start(&bytes, b"moof", 0);
    cases.push(("init-only", bytes[..moof].to_vec()));

    for (label, mutation) in cases {
        assert!(
            validate_mutated_fragment(label, &mutation),
            "accepted {label} mutation"
        );
    }
}

#[cfg(all(target_os = "linux", feature = "linux-media"))]
#[test]
fn native_fragment_fixture_decodes_and_seeks_through_pinned_linux_reader() {
    use apparent_capture_engine::linux::media::PinnedVideoReader;
    let file = File::open(format!(
        "{}/tests/fixtures/fragmented.mp4",
        env!("CARGO_MANIFEST_DIR")
    ))
    .unwrap();
    apparent_capture_engine::publication::validate_movie(&file).unwrap();
    let reader = PinnedVideoReader::open(&file).unwrap();
    assert_eq!(reader.duration_ms(), 1000);
    for (requested, channel) in [(0, 0), (400, 1), (800, 2), (5000, 2)] {
        let frame = reader.frame(requested, 64, 64 * 64 * 4).unwrap();
        assert_eq!(
            (frame.width, frame.height, frame.rgba.len()),
            (64, 64, 64 * 64 * 4)
        );
        let center = &frame.rgba[(32 * 64 + 32) * 4..][..4];
        assert!(
            center[channel] > 140,
            "missing channel {channel} at {requested}: {center:?}"
        );
        assert!(
            center[..3]
                .iter()
                .enumerate()
                .all(|(index, &value)| index == channel || value < 100)
        );
        assert_eq!(center[3], 255);
        // The public derivative contract retains the decoder's observed PTS, including
        // edit-list/B-frame offsets; it is not normalized to the container's first PTS.
        eprintln!(
            "native fragment seek requested={requested} decoder_pts={} duration={}",
            frame.t_ms,
            reader.duration_ms()
        );
    }
}

#[cfg(all(target_os = "linux", feature = "linux-media"))]
#[test]
fn native_fragment_fixture_preserves_legacy_probe_and_frame_callers() {
    use apparent_capture_engine::linux::media::{self, FrameTimes};
    let path = std::path::PathBuf::from(format!(
        "{}/tests/fixtures/fragmented.mp4",
        env!("CARGO_MANIFEST_DIR")
    ));
    assert_eq!(media::probe(&path).unwrap().duration_ms, 1000);
    let frames = media::frames(&path, FrameTimes::At(&[0, 400, 800]), 64).unwrap();
    for (channel, frame) in frames.iter().enumerate() {
        assert_eq!((frame.width, frame.height), (64, 64));
        assert!(frame.rgba[(32 * 64 + 32) * 4 + channel] > 140);
    }
}
