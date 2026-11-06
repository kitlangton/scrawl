# Capture/2 native producer candidate

This owner-local package builds `apparent-engine-capture` from the existing
Engine recorder and core storage primitives. It does not activate installed
routing or register a Platform producer. Capture-v1 and media/1 stay separate.

On native Linux x64 with Node 22.6+, Rust 1.96+, GStreamer development libraries
and the required X11/H.264 plugins, run `npm ci`, `npm run verify`, then
`OWNER_TARGET=linux-x64 npm run build`. Runtime files and their hashes are listed
in `dist/manifest.json`; the absolute entry is `dist/apparent-engine-capture-launch`.
Launch with `--runtime-directory DIR --capture-channel capture
--capture-protocol-version 2`. Provision a persistent private 0700 runtime root.
No PATH lookup, Node, npm or Cargo is needed at runtime.

Cold inspect/stop/cancel now use the existing Core metadata transaction plus a
per-capture anchored owner lock. They do not infer owner death from global live
capture admission or new source readiness. Stop/cancel are journaled for the
owner to observe; `cancel_requested` defaults false when reading earlier v2
journals, and a cancel acknowledgment follows its durable write. Cancellation is
sticky through finalization and crash recovery; a sealed manifest remains leased
until explicit release. Core keeps global live admission for actual capture.

This remains an unmerged candidate, not a usable-producer handoff. Packaged
runtime closure, real packaged SDK-to-worker execution, isolated guest proof and
independent review remain gates. X11 display and the advertised preset area are
the candidate sources; general picker/area selection, Wayland, macOS, audio,
fact collectors, media/2 and publication are unavailable. Materials are never
Saved. Existing native and installed behavior remains on its current route until
verified consumer adoption.
