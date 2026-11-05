# Record semantic output

`assembleProcessingOutput` formats supplied Engine facts in the existing local
`summary.md` style: target/device and destination, initial screen/context,
warnings, selected keyframes, typed notes and app events at their capture times,
context changes, and absolute artifact paths. Window titles remain manifest-only.
When ordered frame annotations reproduce the complete supplied transcript, it
keeps those words on the selected frames. Otherwise it retains the supplied
transcript timeline, including untimed speech, rather than guess missing frame
associations; partial annotations can therefore repeat some words. Notes retain
their pin and frame reference.

The caller supplies already selected keyframes and any verified processing
warnings. Record does not infer missing audio/transcript state, produce images,
write `apparent.json`, or publish a bundle. `decodeSelectedFrames` remains an
injected consumer of the public Engine media/1 SDK. The packaged runtime and
native publication manifest are a separate Engine handoff and are not created
or selected by this formatter.

The golden tests use sanitized Rust summary fixtures and synthetic UI-like PNGs
under `test/fixtures/take.apparent`; these images verify references and layout,
not capture or decoder behavior.
