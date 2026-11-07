# Synthetic publication fixtures

`tiny.mp4` is a two-frame blue 16×16 H.264 movie (0.2 seconds, no audio).
It contains no capture or user data. Generated offline with:

```
ffmpeg -nostdin -v error -f lavfi -i color=c=blue:s=16x16:r=10:d=0.2 \
  -an -c:v libx264 -threads 1 -pix_fmt yuv420p -movflags +faststart tiny.mp4
```

FFprobe reported H.264, 16×16, two frames, duration 0.200000, size 1592.
SHA-256: `1b384568b3ba22e9aea619a2a086e2b20af22f3d791ea17b8dc228fc0e1988fe`.
Tests generate real PNG/WAV bytes through the existing production encoders.
No fixture generation tool is needed to run the tests.

## QuickTime MOV (not a renamed MP4)

`tiny.mov` is independently muxed as QuickTime MOV, with the same entirely
synthetic two blue 16×16 frames and no audio or capture/user data. Generated
offline using FFmpeg 6.1.1 (`6.1.1-3ubuntu5+esm15`):

```
ffmpeg -nostdin -v error -f lavfi -i color=c=blue:s=16x16:r=10:d=0.2 \
  -an -c:v libx264 -threads 1 -pix_fmt yuv420p -map_metadata -1 \
  -fflags +bitexact -flags:v +bitexact -movflags +faststart -f mov tiny.mov
ffprobe -v error -count_frames -show_format -show_streams -of json tiny.mov
ffmpeg -nostdin -v error -xerror -i tiny.mov -map 0:v:0 -f framemd5 -
```

SHA-256: `5f31c3a6867d6f7ec774102571baf8887cebf897f758ceb41851665f6a8172fe`.
Size: 1506 bytes. FFprobe selected its shared `mov,mp4,m4a,3gp,3g2,mj2`
demuxer (`QuickTime / MOV`, probe score 100); that demuxer name alone does
not distinguish MOV from MP4. The actual `ftyp` major and compatible brands
are both `qt  `, minor version 512. There is one H.264 High `avc1` video
stream, yuv420p, 16×16, 10 fps, duration 0.200000 seconds, two declared and
two fully decoded frames. There is no audio stream. Only synthetic handler,
vendor and encoder metadata remain; no title, location or creation date.

Direct atom inspection of these exact bytes found:

| Box/table | Observed value |
|---|---|
| Top-level layout | `ftyp` @0 (20 bytes), `moov` @20 (743), `wide` @763 (8), `mdat` @771 (735) |
| `stsd` | one `avc1` description, data reference 1, dimensions 16×16 |
| `stts` | one entry: 2 samples × 1024 ticks; timescale 10240 |
| `stss` | sync sample 1 |
| `stsc` | one entry: first chunk 1, 2 samples, description 1 |
| `stsz` | variable sizes: 715 and 12 bytes |
| `stco` | one chunk at byte 779, with 727 sample bytes ending at EOF 1506 |

Both decoded yuv420p frames are 384 bytes and have framemd5
`449f1aa73405fb30c333533ce279a095`; the decoded content was also inspected
as uniform blue pixels. The tests pass both original MP4 and actual MOV
through the unchanged publication validator and reject a structurally
boxed but empty/truncated media payload for each.

The Mac screen recorder requests H.264 (`src/capture/macos.rs`), matching
this codec family. This FFmpeg fixture covers completed, nonfragmented
MOV with `stsd`/`stts`/`stsc`/`stsz`/`stco`, not every sample-table variant
Apple might emit, HEVC shrink output, native capture or native playback.
Production publication still checks structure/extents, not codec decoding.

## Publication PNG

`frame.png` preserves the 982-byte synthetic PNG fixture previously read from
`record/test/fixtures/take.apparent/frames/frame-01.png`, byte for byte. Engine
publication tests own this copy so native producer checks do not import Record
implementation or depend on its test-input closure. Its SHA-256 is
`d71f1e8f554e73d8774980d2dc39e1dc589810ad5c14a387ed65db76e9d81467`. Record retains its independently required fixture.

## Native fragmented H.264

`fragmented.mp4` is the 2173-byte output of the device-free segmented writer
fixture at source `71a911ce130b7142dfc5c819984d6d769f48d1f0`, executed on
ARM64 macOS26.6.2 with Swift6.4/SDK27.0. It contains 30 synthetic 64×64 frames
(red, green, then blue), no audio or captured/user content. AVAssetReader
verified every frame, exact `i/30` presentation times and the color transitions;
all frames were also decoded and inspected on Linux. SHA-256:
`e16f0883774629ac35f932e1068c44946ce0c1813985cd82714177bca85a7a38`.

The file uses an H.264 `avc1` initialization segment with empty conventional
sample tables, then three `moof`/`mdat` pairs. The deliberately narrow validator
supports this single-video, timescale-600/fixed-20-tick sample profile, including
bounded H.264 presentation reordering within each fragment. It does not claim
general fragmented-MP4/CMAF support, variable sample durations or capture/2
availability. Existing completed MP4/MOV validation remains separate.
