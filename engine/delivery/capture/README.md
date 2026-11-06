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

This is an unmerged integration checkpoint, not a usable-producer handoff.
Live-owner reconnect/control and durable cancellation conformance, package
closure and isolated guest proof remain delivery gates. X11 display and the
advertised preset area are the candidate sources; general picker/area selection,
Wayland, macOS, audio, fact collectors, media/2 and publication are unavailable.
Materials are never Saved. Existing native and installed behavior remains on its
current route until verified consumer adoption.
