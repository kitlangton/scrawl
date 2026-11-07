# Native media producer

This owner-local producer packages the headless `apparent-engine-frames` worker
for registered `linux-x64` and native `macos-x64` targets. Linux carries a bounded
GStreamer runtime; Mac reuses the moved AVFoundation implementation. Registration
does not activate or route an application. Each target requires its actual native
producer verification and Platform artifact admission before consumer adoption.

New media/2 frame PNGs include the sealed optional `apOB` raw visual-observation
profile; the manifest advertises `visualObservations` separately from channel
capabilities. SDK0.4.2 `@apparent/engine-sdk/observations` validates and reads it
offline against the original derivative/material/request tuple. Old artifacts
without that advertisement remain valid, and old frame outputs return unavailable.
These are measurements of the final sized derivative pixels, not a native frame
selector or a claim of parity with unscaled input. Record owns selection policy.
The existing packaged media/2 H.264 peer checks profile seals and dimensions on
each native target. Capture candidates remain unavailable; installed routing and
macOS15 runtime qualification are separate gates.

The Linux build requires Node 22.6+, Rust 1.96+, an x86_64 host, GStreamer development
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
default media/1 launch accepts the SDK arguments
`--media-runtime-directory DIR --media-channel media --media-protocol-version 1`,
sets closed library/plugin paths and execs the colocated worker without PATH,
Cargo, npm, Node, source, registry persistence or host plugin search.

Builds require a clean tracked/untracked checkout, freeze the full source SHA,
stage and validate a strict path/mode/hash inventory, then check the tree and
artifact again. The owner manifest is integrity metadata, not upstream
provenance. Platform discovery uses the existing `engine-media-native` owner
with separate target fingerprints, receipts and admission.

On a native x86_64 Mac, run `npm ci`, `npm run verify`, then
`OWNER_TARGET=macos-x64 npm run build`. It requires Xcode with Swift 6.2+ and
the macOS 26+ SDK and the same Node and Rust versions. FFmpeg generates the
synthetic fixture movies only; it is not part of the sealed Mac runtime. It compiles
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
descriptor and deadline bounds remain. Neither decoder starts capture or accesses an audio device.
The retained-material adapter uses the same AVFoundation bridge and shared Core
PNG/receipt preparation; its owned re-exec also monitors its physical footprint
through pinning, sheet assembly and receipt publication. Both target producers
record the explicit protocol 2 argv in `media2Launch`, using the same private
runtime root as capture/2. Cold authenticated inspect/cancel remain available
when the decoder helper is unavailable; new derivation requires decoder readiness.
Native Mac media/2 package qualification and exact archive admission remain
required before consumer adoption; Mac capture/2 remains unavailable. Its supervised job binds native frames and a caller-planned contact
sheet to authenticated retained materials. Failed partial files remain private
and unpublished; no recursive caller-directory cleanup is claimed. Media/1 argv
and completed-bundle behavior remain unchanged.

The frozen Media/2 candidate `23aec4162ad939988855f205e943bd16dde53cc0` passed
host and isolated-guest package proof, including the 1366×768 30 fps H.264
fixture with ambient loader/plugin paths closed. The receipt and guest result are
at `/tmp/m2proof-23aec-UvQjX3/package-receipt.json` and
`/tmp/m2proof-23aec-UvQjX3/guest-run-23aec/result.json`; the guest artifact
manifest is `/tmp/m2proof-23aec-UvQjX3/guest-artifacts-manifest.json`. This proof
used an explicit plugin override from archive 463 and is local-only: it cannot
be reused as CI evidence or artifact admission. That historical candidate did
not establish admission; current admitted routes are recorded in the owning
[implementation inventory](../../IMPLEMENTATION.md). No installed route is implied.

Recovery-capable archives additionally declare the exact `recoveryLaunch`
schema/channel/argv. SDK0.4.3 `/recovery` negotiates that role before exposing a
client; missing metadata or an old artifact never becomes a safe-close fallback.
The peer retains finite jobs under their original deadlines when the caller
releases its connection. See [SDK ownership semantics](../../sdk/README-recovery.md).
Recovery artifact admission and affected-platform proof remain separate gates.

Media/2 records native-created output identities before writing image or receipt
bytes. After the supervised worker is reaped, cancellation, deadline, failure
and owner-loss reconciliation remove only those recorded files and the matching
empty `frames` directory. The caller's output root stays in place. Completed
output is preserved; unknown entries, replacements and reservations interrupted
before identity binding remain private and fail closed. Cleanup reuses Core's
pinned Store and no-replace leaf quarantine, not recursive pathname deletion.
