# Record selection policy

`associateTranscriptWithSelectedFrames` assigns timed word/segment units to an
existing ordered selection. It preserves Engine's clause boundaries (punctuation,
1.5 second measured gaps and recognizer segment starts), action anchors, the 50 ms
boundary tolerance and one-time transcript partitioning.

This is a dependency-closed policy slice, not the full selector. The caller supplies
stable selected-frame IDs, bundle-clock timestamps and absolute native candidate
paths. The function neither reads pixels nor creates frames. Image decoding, luma and
region metrics, candidate observations, interaction settling and mark/pin facts remain
Engine producer inputs; absent those facts Record must refuse visual selection instead
of fabricating zero differences or placeholder frames. Speech timing can associate
text, but does not define visual boundaries or create keyframes.
