import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mkdtemp, realpath, rm } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { CaptureClient, preflightCapturePublication } from '@apparent/engine-sdk/capture';
import { createRecordPublicationOutput } from '@apparent/record/publication';
import type { RecordCapabilityProviderOptions } from '@apparent/record/capabilities';
import { createRecordProviderHost } from '../../src/record/provider-host.ts';
import type { RecordNativeBindings } from '../../src/record/native-bindings.ts';

test('admitted public publication output injects facts and resolver without inventing shutdown clearance', async t => {
  const root = await realpath(await mkdtemp(join(tmpdir(), 'record-output-host-')));
  t.after(() => rm(root, { recursive: true, force: true }));
  let nativeCalls = 0;
  const client = new CaptureClient({
    async request() { nativeCalls++; throw new Error('No native dispatch admitted by this test'); },
    async close() { return 'already-exited'; },
  });
  const output = createRecordPublicationOutput(client, root, preflightCapturePublication);
  const first = await output.captureFacts.readFacts();
  assert.deepEqual(first.captures, []);
  assert.deepEqual(first.nextShortcutAdmission, { state: 'unavailable' });
  assert.equal(await output.resolveHandoff({ captureId: 'unknown-request', artifactId: 'unknown-bundle' }), null);
  assert.equal(nativeCalls, 0, 'reading the durable output authority never dispatches publication or recording');
  const report = { requestIds: [], storageUnavailable: false, reconciliation: 'complete' as const };
  const runtime: RecordCapabilityProviderOptions['runtime'] = {
    state: { phase: 'idle', readiness: { ready: false, reason: 'Native capture not connected' } },
    startup: { state: 'ready', knownRequestIds: [] },
    subscribe() { return () => {}; },
    async dispatch() { throw new Error('No command admitted'); },
    async reconcileStartup() { return this.startup; },
    unresolvedOwnership() { return report; },
    dispose() { throw new Error('Unresolved reservation authority must not dispose runtime'); },
  };
  let engineClosed = false;
  const bindings: RecordNativeBindings = {
    native: { async readFacts() { return { version: 1, providerEpoch: 'native-unavailable', revision: 0,
      readiness: [], shortcut: { effective: null, label: null, registration: 'unknown', externallyManaged: false, draft: null } }; } },
    penConfig: { async read() { return { persisted: null, effective: null }; },
      async writeApplyAndReadBack() { return { status: 'unknown', effective: null }; } },
    clipboard: { async copyLocal(identity) {
      return await output.resolveHandoff(identity) === null
        ? { status: 'refused', reason: 'not-saved' }
        : { status: 'outcome-unknown', captureId: identity.captureId, artifactId: identity.artifactId };
    } },
    async close() { throw new Error('Bindings must stay retained'); },
  };
  const host = createRecordProviderHost({ runtime, captureFacts: output.captureFacts, nativeBindings: bindings,
    async closeEngine() { engineClosed = true; } });
  assert.equal(await host.canClose(), false, 'absence of Saved rows cannot clear unavailable shortcut reservations');
  await assert.rejects(host.close(), /ownership remains unresolved/);
  assert.equal(engineClosed, false);
  host.record.detach(); // Observer cleanup only; no runtime/native ownership clearance.
});
