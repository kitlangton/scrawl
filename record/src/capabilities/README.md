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
The seven named scenarios cover permission relaunch, external shortcut refusal,
pen failure, unrelated capture, saved history after reconnect, ambiguous control
and unsubscribe during recording. They make no native/local-only proof claim.

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

The inspected donor at `87c8268c162fc259457cea6babe52cbae84d3dc6` implements
`shared/handoff/pen.rs::save`: validate the palette, persist `penColor` through
`set_config_value_in`, then apply the color in memory. A failed write does not
apply the new color. That writer exists in the release-baseline donor; this
contract does not copy it to main, add another writer or change release/#658.
The eventual Record host must delegate to the authoritative writer and publish
only effective readback. Dispatch acceptance never selects a color; write
failure preserves the last confirmed effective color. Production mapping is
still unavailable, while the fixture supports all four explicit readbacks.
`PEN_COLOR_RGB` exports the donor's exact approved RGB triples for swatches;
these names must not be interpreted as CSS named colors or replacement branding.
