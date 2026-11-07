# Capture/2 native producer candidate

This owner-local package builds `apparent-engine-capture` from the existing
Engine recorder and core storage primitives. It does not activate installed
routing; its existing registration remains owner-local in `delivery.json`. Capture-v1 and media/1 stay separate.

On native Linux x64 with Node 22.6+, Rust 1.96+, GStreamer development libraries
and the required X11/H.264 plugins, run `npm ci`, `npm run verify`, then
`OWNER_TARGET=linux-x64 npm run build`. Runtime files and their hashes are listed
in `dist/manifest.json`; the absolute entry is `dist/apparent-engine-capture-launch`.
Launch with `--runtime-directory DIR --capture-channel capture
--capture-protocol-version 2`. Provision a persistent private 0700 runtime root.
No PATH lookup, Node, npm or Cargo is needed at runtime.

The producer also stages the PipeWire GStreamer plugin, the unchanged standard
vendor client configuration, its six client modules and two SPA factories.
The launcher pins their configuration/module/plugin directories inside the sealed
payload, while retaining the caller's Wayland and session-bus authority. This
runtime closure alone does not enable or qualify a Wayland capture source.

The Linux capture feature also stages the exact `transcribe.cpp` runtime and
dlopen-only CPU backend modules into Cargo's profile and `deps` directories with
an `$ORIGIN` runpath. `apparent-engine-speech-runtime.json` records the sorted
library names; the closed package seeds ELF closure inspection from every listed
runtime/module as well as the worker and media plugins. It contains no model and
does not load a recognizer during build or capture readiness.

Capture/2's supported retained-material path is
`<canonical-private-runtime>/capture-v2/materials/<original-captureId>`.
SDK0.4.4's public `capture2MaterialsRoot(runtimeDirectory, captureId)` derives this
expected root from independently owned launch/admission inputs; consumers must
still authenticate native observations and validate exact material/lease/sealed
file bindings before reading. The helper creates nothing and does not discover
authority from a manifest. Keep the owning runtime and durable capture mapping;
never adopt another runtime or infer Saved/cleanup permission from this path.

The build executes a headless public SDK publication/reconnect check through the
closed release entry, with display and session-bus variables removed. It uses
Engine-owned synthetic media and caller-attested images. This checks the actual
packaged publication route, not real capture or native derivative provenance;
the separate Record/native-media and guest checks retain those gates.

Cold inspect/stop/cancel now use the existing Core metadata transaction plus a
per-capture anchored owner lock. They do not infer owner death from global live
capture admission or new source readiness. Stop/cancel are journaled for the
owner to observe; `cancel_requested` defaults false when reading earlier v2
journals, and a cancel acknowledgment follows its durable write. Cancellation is
sticky through finalization and crash recovery; a sealed manifest remains leased
until explicit release. Core keeps global live admission for actual capture.

The published capture/2 provider supports X11 displays and the advertised preset
area, authenticated retained materials, and native derivative/2 publication.
The X11 capture-to-media/Record/Saved path and same-recording Copy continuation
have separate retained evidence; they do not establish installed Omarchy use.

This source adds one-monitor Hyprland/Omarchy portal capture when Wayland and
XWayland are available. It keeps the existing protocol and recorder, binds the
portal output name and dimensions to the advertised display, and retains the
PipeWire session through finalization. Other Wayland backends, multiple monitors,
arbitrary areas, windows, macOS, audio and fact collectors remain unsupported.
A source-bound closed package passed the public SDK 0.4.5 peer in a disposable
Omarchy guest: native monitor consent, first-frame recording, Stop, sealed H.264
readback and cold authenticated materials inspection. That proof used one
1280×800 output at scale 1. Exact-main CI/archive admission, other scale
configurations and the desktop Saved/Copy adoption remain separate gates.
The first movie frames can include the native chooser fading out; no initial
UI exclusion or clean-frame guarantee is claimed.

Pending portal consent observes authenticated durable Stop/Cancel intent. The
original backend is retained privately so cold controls do not depend on source
discovery. Finish without a frame fails with no materials; only actual Cancel
intent is classified as cancellation. Existing journals remain readable and
retain their original starting-stop behavior. Retained materials alone are never
Saved; validated atomic publication is required. Existing installed routing is
unchanged until its separate adoption gate clears.

Use `launchCaptureEngine(executable, runtimeDirectory, 40_000)` for this producer;
`manifest.launch.shutdownTimeoutMs` declares this grace. SDK 0.3.x's default
three-second close grace is shorter than native finalization: EOS may take eight
seconds, reader preroll ten, and complete decode validation another ten. Closing
requests EOF; its transport outcome never proves materials-ready or Saved. Retain
the original identity and reconcile through authenticated inspection after an
unknown result. Capture validation is bounded and can fail; long/high-resolution
capture and recover/reprocess remain full Engine acceptance gates.

RPC duplicate detection covers the latest 4,096 accepted IDs with bounded memory.
Use fresh RPC IDs; durable capture/request replay is a separate journal binding.

New recovery-capable archives declare `recoveryLaunch` with the exact
`apparent.engine-recovery-peer/1` identity and eight-argument launch. Use SDK
0.4.3's separately negotiated recovery peer, not legacy close, to relinquish an
unresolved recovery connection after local operations drain. Absent metadata
does not imply support. See [SDK ownership semantics](../../sdk/README-recovery.md).
Archive admission and native per-platform qualification remain separate gates.
