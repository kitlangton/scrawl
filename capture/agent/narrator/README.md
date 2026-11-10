# Standalone local narrator

This is an optional worker, not an app dependency. Its own manifest and lockfile
are intentional; do not add these dependencies to the root Cargo files.
Export currently owns cue scheduling, fitting, mixing and final publication.
Kokoro is text-to-speech narration; Parakeet and macOS Apple Speech transcribe
recorded audio and are not narrator provisioning dependencies.

## Provision explicitly

From the repository root, run:

```sh
node capture/agent/scripts/provision-narrator.ts --check
# First setup only, when the native worker is absent:
node capture/agent/scripts/provision-narrator.ts --build-from-source
```

Requires Node 22.18+ and Python 3.10+. A missing worker additionally requires
Cargo/Rust 1.92+ and the platform's normal native linker. No narrator executable
is published in the current release assets: a fresh setup still needs this explicit
source-build prerequisite. Do not invent a release URL or silently substitute TTS.
Existing workers are reused without Cargo; an incompatible/broken worker fails
the synthesis check. Use explicit `--rebuild-from-source` to replace that worker
from locked source (failed builds preserve it); this is never automatic. Pinned model/voice files are verified
even on reuse. Run setup without `--check` to download missing assets for an
existing worker. The Python entry point remains a compatibility launcher into
this TypeScript setup owner; Python alone owns descriptor-based cache mutation.
There is no new config writer.

`--check` does not create or change cache directories, download or build. It
reports JSON `unavailable` for missing prerequisites, `failed` for unsafe/corrupt
assets or failed synthesis, and `ready` only after pinned asset validation plus
real synthetic speech and complete mono PCM16 24kHz WAV validation. The smoke cue
and WAV live in a private disposable `/tmp/apparent-dev-narrator-smoke-*` directory
and are removed afterward; no speakers are used. Ready certifies synthesis, not
listening quality or an exported video's acceptance. Setup emits `downloading`
or `building` while those operations run, then `ready` or `failed`. These are
invocation events, not a durable background-job registry; after an interruption,
run check again rather than trusting the last event.

Interrupted partials are never read as assets. A retry uses a new exclusive
partial and verifies the completed bytes before publication. Only this attempt's
partial is removed; orphaned partials and existing user files are preserved.
No prompts, shell evaluation, secrets or OS package installation. Provisioning is
allowed to use the network: it fetches missing pinned assets and, when explicitly
requested for a missing or broken worker, runs
`cargo build --release --locked -j 3` against this manifest. Cargo may fetch
locked crates; `ort-sys` may fetch its checksum-pinned prebuilt ONNX Runtime.
The build has a 30-minute timeout and uses a disposable target outside the repo.

The default on **both macOS and Linux** is exactly
`~/.local/share/apparent/narrator` (not macOS Application Support, and not an
XDG-derived path). `--cache /absolute/directory` overrides it. Layout:

```text
apparent-narrator
model.onnx
voices/bm_fable.bin
```

Every provision hashes and size-checks both existing assets. A damaged existing
asset fails closed: remove that file explicitly before retrying. Each missing
asset downloads through HTTPS only (including redirects), with a 30-second
socket timeout, a 15-minute wall-clock deadline and its pinned byte ceiling.
Downloads use unique exclusive partial files, then publish without replacing an
existing destination. Failed downloads remove their partials. Worker installation
uses an exclusive partial plus atomic rename; failed builds leave the old worker.

Cache traversal uses directory descriptors and refuses symlink components and
source repositories. Model/voice entries cannot be symlinks. Cache and voices
directories must belong to the caller and must not be writable by other users.
No source files are overwritten. Installation can replace a worker symlink's
*directory entry*, but never writes through it. Use a cache on a local filesystem
with normal Unix hard-link and rename semantics.

## Worker protocol and offline behavior

```sh
~/.local/share/apparent/narrator/apparent-narrator \
  ~/.local/share/apparent/narrator cue.txt fresh-output.wav
```

Exactly three positional arguments: `MODEL_DIRECTORY TEXT_FILE OUTPUT_WAV`.
Input is one nonempty UTF-8 plain-text cue. Output is mono PCM16 WAV at 24,000 Hz.
Voice is fixed to `bm_fable`, speed to `1.0`, device to CPU and ONNX intra-op
parallelism to three threads. There is no duration fitting or timeline logic.
Exit zero means the complete WAV was finalized; diagnostics go to stderr.
The output path must be fresh (existing files and symlinks are refused).
The caller should use a private working directory, wait for exit zero, validate
the WAV, and only then consume it. Interrupted processes can leave an incomplete
output; the caller owns cleanup in that case.

Synthesis never provisions, invokes Cargo, calls a service or downloads anything.
Missing assets fail locally. `loqui-kokoro =0.4.0` uses `loqui-g2p`'s embedded
English lexicons and neural fallback: **no espeak executable, espeak data or
espeak shared library is needed**. Empty or nonfinite generated audio is rejected
before creating an output. Samples are filtered, saturated to [-1, 1], rounded
and scaled by 32767; there is no peak normalization or added silence.

ONNX Runtime is statically linked by the default `ort =2.0.0-rc.13` build, using
API 22. No separately installed ONNX Runtime is required for that build. The
verified Linux executable links normal libc/libm/libgcc/libstdc++ libraries.
Provisioning is not promised to work offline. In particular,
`CARGO_NET_OFFLINE=true` makes this ort-sys version skip even its cached prebuilt
runtime; an offline rebuild needs explicitly configured local runtime libraries.
This does not affect an already provisioned worker's offline synthesis.

## Asset provenance

The disposable Rust trial's model and voice bytes were hashed and matched these
upstream records, rather than substituting another similarly named quantization.
Both weights/voice derive from Kokoro-82M (Apache-2.0); loqui code is MIT
and ONNX Runtime is MIT. Consult the upstream NOTICE files for embedded G2P data.

| Asset | Pinned upstream identity | Bytes | SHA-256 |
|---|---|---:|---|
| `model.onnx` | thewh1teagle/kokoro-onnx release `model-files-v1.1`, asset **520133701**, `kokoro-v1.0.int8.onnx` | 114119327 | `ae315a79b623f244700e4afb9246c46a26066782e049ba174bf3ba433970ee9c` |
| `voices/bm_fable.bin` | onnx-community/Kokoro-82M-v1.0-ONNX revision **1939ad2a8e416c0acfeecc08a694d14ef25f2231** | 522240 | `f889083196807b4adb15e9204252165f503b8d33d3982e681c52443c49d798f1` |

Model provenance: [release API](https://api.github.com/repos/thewh1teagle/kokoro-onnx/releases/tags/model-files-v1.1)
reports release ID 203169879 and the exact asset size/digest above. The release
tag resolves to source commit `b85309f90fd2660ea3309cf0f2581360e4327555`;
release binaries are not Git blobs, so the **asset digest**, not the mutable tag,
is the byte-level pin. The download URL is in the provisioner. The similarly
named HF `onnx/model_quantized.onnx` is 92,361,116 bytes and is **not** this model.
Voice provenance: [HF API at the pinned revision](https://huggingface.co/api/models/onnx-community/Kokoro-82M-v1.0-ONNX/revision/1939ad2a8e416c0acfeecc08a694d14ef25f2231?blobs=true)
provides its LFS size/digest. This is also the repo revision pinned by the
registry's `loqui =0.4.0` model inventory.

## Whine mitigation and verification

The trial's corrected decode confirmed a narrow peak at 9,600 Hz. A single RBJ
notch biquad is pinned at **9,600 Hz, Q=2.5, 24 kHz**, with f64 coefficients/state
and zero initial state per cue. It runs before PCM16 conversion. Tests pin the
impulse response, require >70 dB rejection of a steady center tone, >18 dB at
9,550/9,650 Hz, and <0.5 dB change at 100/1,000/4,000/8,000 Hz.

Measured by applying the actual Rust `prepare` function to the trial's 35-second
raw float timeline, then comparing overlapping one-second Hann-window FFT power:
9,600 Hz bin **-58.44 dB**, 9,550–9,650 Hz **-35.27 dB**, 9,500–9,700 Hz
**-29.89 dB**, aggregate 100–8,000 Hz **-0.00487 dB**. This establishes DSP
mitigation, not perceptual approval or integrated app proof.

Tests (from repository root; target stays outside source):

```sh
python3 -m unittest discover -s capture/agent/scripts/tests -p test_provision_narrator.py
cargo test --manifest-path capture/agent/narrator/Cargo.toml --locked --offline \
  --target-dir "$TMPDIR/narrator-build" -j 3
```

Genuine initial RED: the provisioner seam raised NotImplementedError for asset
verification/cache safety, and the identity DSP failed empty/nonfinite rejection,
whine attenuation and symmetric saturation assertions. Final Linux checks pass
12 provisioning tests plus four DSP and four worker-protocol tests. Real Linux
synthesis with both trial and freshly provisioned assets yielded 113,400 samples
(4.725 seconds) for the first trial cue, in 6.532/6.635 seconds wall time including
model startup. `strace -f -e trace=network` recorded no network syscalls.
Float inference can vary in its last bits; output WAV hashes need not match
across separate runs.

Fresh setup and reuse were also verified on Apple-silicon macOS 26.6.2 with
Node 24.20, Python 3.14 and Rust 1.92: 16 Python tests plus three TypeScript
contracts, pinned downloads, locked worker build, real synthesis and a second
readiness check passed. `otool -L` showed only system libraries/frameworks.
A synthetic cue produced a complete mono PCM16 24 kHz WAV with nonzero signal
and no saturated samples; it was inspected silently, without listening approval.
This does not establish older-macOS compatibility, signing/notarization for a
published worker, app CLI integration, cue fit, video export or GUI behavior.
