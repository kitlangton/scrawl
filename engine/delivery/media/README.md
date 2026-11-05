# Native media producer

This owner-local producer packages the headless Linux `apparent-engine-frames`
worker and the bounded GStreamer runtime needed for media/1 decode. It does not
activate or route an application. It targets native `linux-x64` hosts only;
macOS media decoding is unimplemented.

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
