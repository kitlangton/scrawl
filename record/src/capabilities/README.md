# Record capabilities: fixture-ready contract, not production-ready

Import `RecordClient`, `CapabilitySnapshot`, `RecordCommand` and `CommandResult`
from `@apparent/record/capabilities`. A consumer accepts an injected `RecordClient`;
its application logic imports neither Electron nor Engine. The shell must supply
an authorized narrow preload adapter. No production adapter is shipped here.

`observe` immediately supplies a complete snapshot, then complete replacements.
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
`start-capture` with `delivery: "local-only"` requires the production provider to
enforce no upload/sharing; it must refuse if enforcement is unavailable, never
silently downgrade to host defaults. This package implements no such enforcement.

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
stale or inapplicable requests. They never mutate effective preferences or create
capture outcomes in response to a command. Publish explicit provider-like facts
with `driver.publish` (same epoch, increasing revision); stale frames return false.
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
