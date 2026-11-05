# Record semantic output and media consumer

`assembleProcessingOutput` formats supplied Engine facts in the existing local
`summary.md` style: target/device and destination, initial screen/context,
warnings, selected keyframes, typed notes and app events at their capture times,
context changes, and absolute artifact paths. Window titles remain manifest-only.
When ordered frame annotations reproduce the complete supplied transcript, it
keeps those words on the selected frames. Otherwise it retains the supplied
transcript timeline, including untimed speech, instead of the partial frame
annotations. This avoids guessing associations or repeating speech. Notes retain
their pin and frame reference.

The caller supplies already selected keyframes and any verified processing
warnings. Record does not infer missing audio/transcript state, produce images,
write `apparent.json`, or publish a bundle. `decodeSelectedFrames` remains an
injected consumer of the public Engine media/1 SDK. The packaged runtime and
native publication manifest are a separate Engine handoff and are not created
or selected by this formatter.

The golden tests use sanitized Rust summary fixtures and synthetic UI-like PNGs
under `test/fixtures/take.apparent`; these images verify references and layout,
not capture or decoder behavior. The native consumer test separately exercises
real PNG decode against the retained Linux `media/1` artifact from Engine source
`463f0053908a9c6030d3c5665aacd10089884aa` using an isolated synthetic H.264
bundle.

The optional `"local-clipboard"` presentation retains semantic context, notes and
warnings while omitting destination/target metadata, archive image instructions
and artifact footers. Handoff supplies access instructions before this text, so
the duration header and timed request remain at the end of a paste. The default
`"summary"` presentation preserves the archived output contract.

## Durable media consumer

`RecordMediaJob` and `processCompletedBundle` are exported from
`@apparent/record/processing/media-consumer`; frame decoding and semantic
assembly remain in `@apparent/record/processing`. A host admits an artifact through the existing
delivery contract, installs the packed `@apparent/engine-sdk`, performs the
existing artifact verification, then injects one `MediaClient` constructed with
`launchMediaWorker(absoluteEntry, privateRuntimeDirectory)`. Record accepts no
paths from `PATH`, builds no Engine source, and never closes or replaces the
injected client. Producer/admission and installed routing remain owned by
Platform/Engine; this API does not select or activate a production route.

Create a job with a stable SDK job ID, a completed bundle's canonical paths and
file identity, Record-selected timestamps, and a fresh private output directory.
`RecordMediaJob.create` durably writes that request and selection before decode
dispatch. It retains the same client, job ID and output directory after a decode
rejection or timeout. Call `inspect()` to reconcile; call `cancel()` only when
cancellation is intended, then inspect until terminal. Neither rejection,
timeout, cancel dispatch nor object disposal proves the worker stopped. Journal
cleanup is refused until a complete/failed/cancelled state is authoritative;
completed PNG outputs are retained for semantic assembly/handoff.

```ts
import { decodeSelectedFrames, assembleProcessingOutput } from "@apparent/record/processing";
import { RecordMediaJob, processCompletedBundle, createProcessingOutputDirectory }
  from "@apparent/record/processing/media-consumer";

const processing = { decodeSelectedFrames, assembleProcessingOutput };
const outputDirectory = await createProcessingOutputDirectory(privateJobs, jobId);
const job = await RecordMediaJob.create(mediaClient, processing, journalPath, selection);
const { frames, output } = await processCompletedBundle(job, processing, selection,
  { durationMs, bundlePath, transcript, videoPath });
```

The consumer test checks native handshake, durable job ownership, PNG
signatures/dimensions/hashes, and semantic text. It makes no capture-v1,
installed-route, macOS media, or current-main artifact-admission claim. The
artifact remains Linux x86_64 only.
