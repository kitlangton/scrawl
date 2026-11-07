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

The prior capture/2 runtime has private host Xvfb and isolated Linux guest
proof. Main `0570f775` delivered a fresh admitted capture archive in run
`42252644242`, with two packaged checks passing and all 54 runtime seals and
attestation verified. That archive reports publication unavailable. This
publication candidate has its own source-bound package and headless release check;
its real capture-to-Saved, guest proof and publication artifact admission remain
separate gates. X11 display and the advertised preset area are the supported sources; general picker/area selection, Wayland, macOS, audio,
and fact collectors remain unavailable. The unmerged publication candidate adds
authenticated publish/inspection and native derivative/2 registry validation.
Retained materials alone are never Saved; only validated atomic publication can
return Saved. Fixture and host Xvfb proof do not establish the full Record
capture-to-Saved path or current-candidate guest/package qualification. Existing native and installed behavior remains on its current route until
verified consumer adoption.

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
