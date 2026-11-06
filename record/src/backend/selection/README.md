# Record selection policy

`associateTranscriptWithSelectedFrames` assigns timed word/segment units to an
existing ordered selection. It preserves Engine's clause boundaries (punctuation,
1.5 second measured gaps and recognizer segment starts), action anchors, the 50 ms
boundary tolerance and one-time transcript partitioning.

This is a dependency-closed policy slice, not the full selector. The caller supplies
stable selected-frame IDs, bundle-clock timestamps and absolute native candidate
paths. The function neither reads pixels nor creates frames. Image decoding and
luma measurements remain producer inputs; absent measurements Record must refuse
visual selection instead of fabricating zero differences or placeholder frames.
Interaction settling and mark/pin facts remain outside this selector. Speech timing
can associate text, but does not define visual boundaries or create keyframes.

`selectVisualKeyframes` is the bounded visual-only policy over caller-supplied,
already measured 64×36 luma samples. It ports the Engine's scene-transition
threshold and settling, score/time ranking, 750 ms spacing, novelty check and
empty-transcript long-stretch coverage. `visualKeyframeBudget` matches the
Engine duration budget. The selector validates ordered unique sample identities,
same-sized decoded planes and exact finite luma arrays; it does not decode,
resize, inspect native candidate authority or create image bytes. Its static
midpoint fallback means the caller supplied actual measurements with no visual
transition; it must never stand in for unavailable measurements. This module
does not include transcript, interaction, cursor or mark policy, so it is not a
replacement for the complete Engine selector or installed/native pipeline.
