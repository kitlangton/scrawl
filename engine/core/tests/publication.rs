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
