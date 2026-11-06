# Engine SDK 0.4.2 visual observations package

Source identity: Engine main commit `21539b722aaed810ad4a21718551cbe10d3b4227`, PR 912. The actual main push run `42308030966` succeeded. SDK artifact `13547401533` was explicitly reused from the admitted PR build source `7a92b82c8adb37b758fda333750e480db34bdf78`, run `42302764021` attempt 1; this is receipt-bound PR-artifact reuse, not a fresh-main build.

Receipt artifact: `stream-engine-linux-x64-63f7ceceab4263038801da546f4184a0cd8970c3698ecdd4efe698188fb261f8`. The exact supplied payload is `sdk/extracted/stream-payload/apparent-engine-sdk-0.4.2.tgz`, SHA-256 `62b1b2e103a585dbf10b848c9538ddcf888b3643b85d7eb89db8c1a61647066c`; its enclosing component is 55,336 bytes with SHA-256 `57b471f97045e8fad968d68551fdf7e1e2e8157a37711fec618c772fa0f8426c`. The published package manifest identifies `@apparent/engine-sdk` version `0.4.2` and exports `./observations` with public declarations for `readFrameVisualObservations`, `FrameVisualObservationInput`, `FrameVisualObservation`, and `VisualObservationSource`.

The committed Record package pins this exact tarball for development and uses its optional `^0.4.2` peer range. Packed offline import/declaration checks cover this public SDK contract. This provenance does not claim native Record acceptance, native observation authority, capture, candidate collection, Saved, or publication.

SDK 0.4.1's historical provenance remains in `engine-sdk-0.4.1.provenance.md`; its old tarball was superseded after checking current package, lock, and smoke-profile references. Historical verification records remain unchanged.
