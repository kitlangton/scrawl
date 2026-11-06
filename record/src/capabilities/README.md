# Record capability contract

Import `RecordClient`, `CapabilitySnapshot`, `RecordCommand` and `CommandResult`
from `@apparent/record/capabilities`. A consumer accepts an injected `RecordClient`;
its application logic imports neither Electron nor Engine. The capture/2 host can
be injected through the owner-local runtime adapter; native permission facts and
other host capabilities still require their authorized producer ports.

`observe` immediately supplies a complete snapshot, then complete replacements.
The snapshot contract is version 2. Version 1 snapshots must be migrated by
mapping every capture's delivery fact to `unknown` and setting
`nextShortcutAdmission` to `unavailable`; absence of those facts is never proof.
Within one `providerEpoch`, revisions strictly increase. Reconnect to the same
provider retains its epoch; provider restart changes epoch and may reset revision.
Old epochs are never restored. Reconnecting supplies accumulated capture-scoped
facts; unknown/pruned history must not be interpreted as success. The host must
serialize initial snapshot delivery and subsequent replacements without a gap.
Commands carry a unique `requestId` and observed epoch/revision; stale commands
are refused. Retain request identity after transport ambiguity and reconcile by
capture `invocationId`/capture ID in refreshed snapshots, not global busy state.
The snapshot does not expose Engine ownership tokens or private credentials.

`accepted` means dispatch/admission only. It proves neither permission grant,
registered shortcut, changed pen, first frame, save, copy nor open. `applied` is
reserved for a provider-confirmed operation; the relevant facts must be present
in its snapshot. `refused`, `cancelled` and `outcome-unknown` never imply success.
Command cancellation is not capture cancellation. Native grant status and usable
capture readiness remain distinct; a granted permission may still need relaunch.
`start-capture` with `delivery: "local-only"` is a request only. The matching
`CaptureFact.delivery` is the provider's readback of effective enforced policy:
`local-only`, `host-default` or `unknown`. A command result, including `accepted`,
does not set it. The capture/2 integration refuses a local-only Record request
because native `localOnly` does not enforce the complete Record processing and
delivery path. It accepts `host-default` when the injected worker is ready, while
the resulting delivery fact remains `unknown` until a complete policy producer
can authenticate it.

Picker success has a typed `surface: "screen"` and `target` (`display`, `area` or
`window`); picker cancellation/failure has no fabricated target. Accepted strokes
carry a typed `surface` (`screen` or `whiteboard`) and stable factual `strokeId`
on success. Free-form `detail` cannot substitute for either typed fact.

`nextShortcutAdmission` and `arm-next-shortcut` / `cancel-next-shortcut` are
request-scoped facts, not an onboarding feature. The caller supplies an opaque
`admissionId`; it is correlation only, not authorization. An arm command is only
dispatch. A provider must publish `armed` readback with delivery and expiry, then
`consumed` with the exact capture ID. That capture carries the same
`shortcutAdmissionId`, typed shortcut and first-frame outcomes, and effective
delivery fact. `cancelled` only cancels the reservation; it does not cancel a
capture. On reconnect, preserve the exact reservation identity or report it as
unavailable/unknown; never invent a consumed event. No production shortcut
reservation producer exists yet. The fixture defaults to `unavailable` and refuses
arm/cancel until its driver publishes a supported state.

## Shortcut-trial and causal note/stroke evidence

`shortcutTrial` is separate from `nextShortcutAdmission`: it represents a
request-scoped physical shortcut test and never starts capture. Its binding
includes the producer epoch/revision, opaque registration identity and exact
effective shortcut. `idle` is valid only at that exact source revision;
reservations retain an opaque identity through `reserved`, `pressing`,
`observed`, `expired`, `cancelled` or `unknown`. `observed` requires a physical
press/release pair. A `begin-shortcut-trial` result of `accepted` is dispatch
only and cannot produce that observation. Binding or epoch changes invalidate
the prior reservation; stale reservation identity is retained as `unknown`.
`shortcutTrialBeginBinding` is a separate optional fresh selector view. It is
available only from a coherent current idle or raw terminal producer fact and a
currently registered effective binding with no edit draft. Idle begins omit
`replaces`; a terminal rearm supplies `replaces` with the exact prior
`{ sourceEpoch, sourceRevision, bindingId, effectiveBinding, reservationId }`
identity and a different new reservation ID. The command's source selectors
must match `shortcutTrialBeginBinding`, which may have newer epoch, revision or
binding values than the retained terminal identity. Provider-projected
`unknown` from a raw `reserved`/`pressing` trial never exposes fresh rearm
selectors. The native producer must atomically compare the predecessor identity,
current GUI revision and current registration, retain predecessor correlation,
and keep accepted/ambiguous dispatch distinct from later physical readback; the
Record provider does not reset terminal state or retry a lost request.
When the native port omits this producer, the provider publishes `unavailable`
and refuses trial commands. The existing arm-next-shortcut command is not a
physical trial and must not be substituted.

A successful `note-save` outcome means a trusted producer has durably saved a
nonempty note. It exposes only opaque note and receipt IDs plus a
`screenStrokeBarrier` cursor; note content and paths are not part of the
capability. Draft, empty, failed and cancelled outcomes are non-success facts.
Screen accepted-stroke outcomes may carry the durable cursor
`{ historyId, sequence }`. A stroke is causally after a note only when it belongs to the same
capture, has the same history incarnation and a sequence strictly greater than
the note's barrier. Event timestamps and receipt arrival order do not establish
this relation. Missing cursors remain unavailable for causal comparisons.
Receipt IDs bind one immutable note identity/barrier, and stroke IDs bind one
surface and observed cursor within a capture. The provider rejects conflicting
revisions of known identities, including across producer reconnects, using its
existing fact history. Previously unseen older events can still arrive late.
The producer must retain those identities and causal order durably across a
provider restart; the Record projection does not manufacture a counter.

These are public additive contracts, not claims that the producers exist.
Production currently has no physical shortcut-trial producer, durable nonempty
note receipt/barrier producer, or screen-stroke history cursor producer. Their
owners must supply authoritative scoped facts before these capabilities become
available. Platform must publish guarded non-recording trial reservation and
physical press/release readback for the exact effective registration incarnation;
its current permission/shortcut port does not provide that operation. Engine must
publish a durable capture-scoped screen-stroke history and an authoritative
cursor barrier. Record retains note orchestration and receipt translation: the
existing authoritative note persistence remains
`capture/engine/runtime/recording/notes.rs::Store::save/commit`. Its empty-save
`Ok` and Desktop's `saved_count` are not new durable-note receipts. A producer
must bind an actual nonempty committed save to an opaque receipt and subsequent
native cursor barrier; this schema creates no competing note writer or native
protocol. Native Linux/Mac effect and published artifact evidence remain required
for those producers. Fixtures
are synthetic controlled facts only; they establish no native effect or durable
save. The fixtures cover physical pair/no event, unrelated request, stale
epoch/binding, expiry/cancellation/unknown/reconnect and note draft/empty/
failure/cancellation, pre/post-barrier strokes, unrelated history/capture and
late receipts/finalization.

`@apparent/record/fixtures` exports `createRecordFixture`, `fixtureScenarios` and
test-only driver types. Instantiate separately for each consumer test:

```ts
import type { RecordClient } from '@apparent/record/capabilities';
import { createRecordFixture } from '@apparent/record/fixtures';
const { client, driver } = createRecordFixture('reconnect-with-saved-history');
// Inject client into the consumer. Only test code receives driver.
const unsubscribe = client.observe(snapshot => renderFromFacts(snapshot));
const snapshot = driver.current();
const reply = await client.execute({
  type: 'copy-local', captureId: 'capture-saved', requestId: 'copy-1',
  expected: { providerEpoch: snapshot.providerEpoch, revision: snapshot.revision },
});
// reply.status === 'accepted' does NOT mean copied. Publish a controlled outcome
// via driver.publish({...snapshot, revision: snapshot.revision + 1, captures: ...}).
unsubscribe(); // observation only; capture continues
client.detach();
```

Fixtures record admitted dispatches in `driver.commands()` and refuse disconnected,
stale or inapplicable requests. They never mutate effective preferences, delivery
readback, admission state or capture outcomes in response to a command. Publish
explicit provider-like facts with `driver.publish` (same epoch, increasing revision);
stale frames return false.
Use `disconnect`/`reconnect` for connection/epoch changes. Retired epochs return
false. Each accepted publication reaches observers; listener failures are isolated.
Script transport uncertainty/refusal/cancellation with `setNextResult`.
The named scenarios include permission relaunch, external shortcut refusal,
pen failure, unrelated capture, saved history after reconnect, ambiguous control,
unsubscribe during recording and the synthetic onboarding evidence cases above.
They make no native/local-only proof claim.

Unsubscribe and `detach()` never issue Stop/Cancel; the owning host keeps capture
alive and handles shutdown safely. Record stores general facts only. Consumers
own associations, coaching, progress and manual paste confirmation. Production
still needs native permission/shortcut/pen mappings, the existing transactional
config writer, Engine SDK adaptation, bounded RPCs and affected platform proof.
The shell/config owner must provide those adapters without a second config writer.

## Artifact-open confirmation

`open-dispatch` records only a dispatch attempt. `accepted`, an OS launcher exit
code, process startup, focus changes and file-access guesses are insufficient to
establish that the matching artifact opened. The optional `CaptureFact.artifactId`
is an opaque identity bound to the exact saved artifact, stable across reconnect
and path renames. Replacing/reprocessing the artifact requires a new identity.
An `artifact-open` outcome names that identity and is either `ok: false` with
`reason: 'refused'|'failed'`, or `ok: true` with `evidence: 'viewer-ack'|'os-observation'`.
Only an authorized provider may publish confirmation after a trusted receiving
viewer acknowledgement or a supported OS observation proves the same artifact
opened. An SDK/native adapter without that evidence must leave it unknown.
No timeout or dispatch result may manufacture confirmation. The public helper
`hasConfirmedArtifactOpen(snapshot, captureId)` requires saved state, matching
capture and current artifact identity. It proves neither playback nor watching.

Current legacy `shared::handoff::open_path` uses macOS `open` success or Linux
launcher spawning; neither implements this confirmation seam. The Engine SDK
also has no artifact-opening API. The precise remaining production prerequisite
is a trusted receiving-viewer/OS observation mapped to the saved artifact identity
by the Record host adapter. This change defines the fact and controlled fixture
coverage only; no private app monitoring, new viewer or native route is added.
Until a verified producer exists, consumers cannot complete a confirmation-based
stage from actual production facts. Their policy stays outside Record.

Explicit `start-capture` must refuse while a next-shortcut admission is `armed`.
The caller must observe consumption, cancellation or expiry before requesting an
explicit Start. An accepted cancel command alone does not disarm it. This avoids
silently applying a shortcut-only delivery reservation to an unrelated start.

## Pen color, not tool mode

`PenColor` is `coral | purple | blue | green`. `PenPreference.effective` is a
confirmed named color or `null` when unknown; `supported` lists those color
choices. `select-pen.value` is a PenColor. Ink/laser/cursor are tool modes, not
color choices and are not exposed by this preference. Fixtures begin with an
explicit synthetic stored coral selection, not a claim about product defaults.
The donor's unnamed legacy red fallback must not be relabelled coral.

The historical donor `shared/handoff/pen.rs::save` documents the original
persist-before-apply semantics; that file is not a current-main writer. The
published `@apparent/engine-sdk` 0.4.0 capture API, `@apparent/native-host/pen`,
Record's `PEN_COLOR_RGB` palette, and its `createRecordPenConfigPort` factory
(returning `RecordPenConfigPort`) are
package-level contracts. Platform #779 supplies the top-level `penColor`
config CAS/read seam through the authoritative `shared/config-native` writer,
and #786 supplies the guarded pen-effect producer. Record
#799 implements the injected read/CAS/guarded-apply/getter adapter. These source
and contract seams are implemented, but the installed Platform host binding and
activation are not established by package availability. Record publishes an
effective color only after authoritative readback; acceptance of a command or
persisted preference alone is not proof that ink changed. The palette triples
must not be interpreted as CSS named colors or replacement branding.
