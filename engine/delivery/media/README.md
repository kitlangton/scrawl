# Native media producer

This owner-local producer packages the headless Linux `apparent-engine-frames`
worker and the bounded GStreamer runtime needed for media/1 decode. It does not
activate or route an application. The registered production target is
`linux-x64`. A separate native `macos-x64` build candidate reuses the moved
AVFoundation implementation; its native verification and Platform admission
are required before consumers can treat it as an available Mac artifact.

Requires Node 22.6+, Rust 1.96+, an x86_64 Linux host, GStreamer development
libraries discoverable through `pkg-config`, `ldd`, and FFmpeg only to generate
synthetic test fixtures. From this directory run `npm ci`, `npm run verify`,
then `OWNER_TARGET=linux-x64 npm run build`. Cargo inherits the repository
`.cargo` baseline and builds locked/release. `APPARENT_CARGO_OFFLINE=1` requires
the locked dependencies to be cached.

The required plugin set is `coreelements`, `typefindfunctions`, `playback`,
`app`, `videoconvertscale`, `isomp4`, and `libav`. `APPARENT_GST_PLUGIN_SOURCE` may point to an
overlay directory containing replacements for required plugins missing from
the host. Other required plugins come from the `pkg-config` plugin directory.
Overrides are explicit build inputs, checked as regular files, hashed into the
manifest, and must be supplied by the delivery environment;
the producer never installs packages. The default source is the GStreamer
plugin directory reported by `pkg-config`. Its package/OS identity and host
GStreamer version are recorded rather than presented as a universal baseline.
Overrides are restricted to local proof builds and refused when `CI` is set;
they are not eligible for composition artifact admission/reuse. The registered
producer must use the runtime installed by its declared CI environment.

`dist/` contains the absolute launch entry `apparent-engine-frames`, `lib/`
with each non-base-OS transitive library by its needed SONAME, `plugins/` with
only the required plugins, and `manifest.json`. The loader and baseline glibc
are host requirements; the manifest records the observed glibc version. The
entry accepts exactly the SDK arguments
`--media-runtime-directory DIR --media-channel media --media-protocol-version 1`,
sets closed library/plugin paths and execs the colocated worker without PATH,
Cargo, npm, Node, source, registry persistence or host plugin search.

Builds require a clean tracked/untracked checkout, freeze the full source SHA,
stage and validate a strict path/mode/hash inventory, then check the tree and
artifact again. The owner manifest is integrity metadata, not upstream
provenance. Platform discovery/registration remains a separate owner action.

On a native x86_64 Mac, the candidate build is `OWNER_TARGET=macos-x64 npm run
build`. It requires Xcode with Swift 6.2+ and the macOS 26+ SDK, the same Node
and Rust versions, and FFmpeg for synthetic test generation only. It compiles
the existing worker plus the private `FramesBridge.swift`/`Media.swift` helper;
no Desktop application or GStreamer is linked. The artifact has three executable
files: the absolute launch entry and the worker/helper under `libexec/`, plus
`manifest.json`. The manifest binds thin x86_64 Mach-O identities and only system
libraries; the deployment minimum is macOS 15.0, which is not itself runtime proof
on that OS. No cross-built artifact or arm64 proof is labelled x86_64 verification.

The Mac bridge consumes an inherited pinned movie descriptor and streams bounded
RGBA through a private binary pipe. It forbids external container references;
Rust retains the existing PNG seals, leases and publication. Its parent-lifetime
pipe terminates the helper when the job disappears. Darwin memory enforcement
checks the worker/helper combined physical footprint every 10 ms against 1536
MiB and fails closed if usage cannot be read; this is a sampled physical budget,
not Linux's instantaneous virtual-address limit. Hard frame/pixel, CPU, file-size,
descriptor and deadline bounds remain. Neither decoder starts capture, accesses
an audio device or implements raw-material/media2 decoding.
