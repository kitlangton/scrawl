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
not capture or decoder behavior. Native consumer tests separately exercise PNG
decode against retained Linux and macOS `media/1` artifacts using isolated
synthetic H.264 bundles. The macOS r2 run passed 1/1 against the admitted
`ae0789` closure on macOS 26.6.2 Apple Silicon through x64 Rosetta: two 320×180
PNGs matched SDK inspection, and the durable job reopened and cleaned up its
journal. This does not establish Intel CPU, macOS 15, native ARM, capture,
media/2, publication, clipboard, or installed-route behavior.

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

The retained Linux artifact is x86_64 only. The macOS result above used a
separately admitted x64 closure; neither run changes capture or installed
routing.

## Durable media/2 consumer

`RecordMedia2Job` is exported from
`@apparent/record/processing/media2-consumer`. The host injects the `Media2Client`
already composed for the same private capture/2 runtime, and supplies the
original owner token on each operation and reopen. The caller owns client
lifecycle. Record preflights the caller's request, contact-sheet layout and
materials, then durably stores the original request hash and materials in a private SQLite
journal before dispatch. The existing host storage primitives provide an atomic
compare-and-swap transition, so stale handles and separate processes cannot both
claim the same ready job. No SDK request or filesystem await holds a transaction. Duplicate requested times and caller-selected output policy remain
intact.

An ambiguous derive remains unresolved across restart and can only be reconciled
by inspecting the same input; Record never retries derive or cancels implicitly.
Cancellation is an explicit caller action. A complete result is accepted only
after SDK derivative validation and original request/layout binding checks; it
is a processing result, not a Saved or publication result. Synthetic offline
tests cover the client seam and consistency contract only. SDK validation does
not prove derivative origin, and do not replace the separately admitted Linux native Media/2 evidence.

Terminal results are immutable: later inspections cannot replace a failure code
or validated derivative, even when the reported state name is unchanged.
`cleanupJournal()` clears the stored request/material/derivative payload while
retaining a private SQLite tombstone. Use a new journal path for a new job;
stale handles cannot reuse the cleared namespace. Cleanup never releases native
materials or cancels work. Media/1 keeps its existing journal implementation.

## Retained derivative files

`readRetainedDerivativeArtifacts` is exported from
`@apparent/record/processing/retained-artifacts`. It accepts the original
`Derivative2DocumentInput`, its validated materials, a host-trusted immutable
derivative root, explicit `{ bundlePath, selector }` mappings, and finite
per-file/aggregate byte limits. The trusted root must equal both the original
request output directory and the SDK-validated receipt root.

The reader returns `SealedArtifact[]` compatible with canonical bundle assembly
and the exact receipt ID/selector bindings consumed by publication staging. A
final bundle path may differ from the native relative path; the original sealed
identity, hash, length, and selector remain bound. The caller still chooses
frame selection, layout, canonical reasons, final paths, and admission before
assembly. Local byte consistency does not authenticate native origin or create
Saved state, publish, copy, or release retained materials.

Standard sealed drawing consumption uses the existing original-materials ingress.
`createStandardRecordCaptureCoordinator` validates `acceptedStrokes` before planning
and final selection and persists the exact Record projection in the existing plan.
`projectOriginalAcceptedStrokes` is also exported from `@apparent/record/processing/bundle`;
it validates the original sealed fact and maps IDs, surface dimensions and normalized
bundle-clock points without shape inference. The assembler obtains optional RGB only
from that original fact, preserving absent versus black.

Canonical v1 stroke arrays omit original IDs and surface/transform references. Therefore
assembly also publishes the exact original fact path/seal as a material-backed
`companion`, with an absolute reference and count in summary/handoff. Original accepted
times, source IDs, surfaces/transforms and any producer-supported board state remain in
the unmodified document. This does not invent undo/clear events or replay missing history.
Collected-empty retains its empty document; unavailable retains no document or text.
The same publication binding persists handoff context, and only authenticated Saved
exposes it after restart. Interactions still require an explicit richer policy.

These consumer contracts and isolated fixtures do not advertise native ink. Actual
production requires Engine's authenticated accepted-stroke ingress, immutable original
fact/seals and same capture/material revision/lease, with qualified surface/transform,
clock, clipping and board undo/clear/reopen semantics plus admitted artifact/native proof.
Platform supplies controls only; it does not author canonical geometry or attest facts.
