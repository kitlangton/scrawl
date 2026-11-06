import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { CapabilitySnapshot, RecordCapabilityProviderOptions } from '@apparent/record/capabilities';
import { createRecordProviderHost } from '../../src/record/provider-host.ts';
import type { RecordNativeBindings } from '../../src/record/native-bindings.ts';

type Runtime = RecordCapabilityProviderOptions['runtime'];
function fixture(captureFacts?: RecordCapabilityProviderOptions['captureFacts']) {
  let state: Runtime['state'] = { phase: 'idle', readiness: { ready: true } };
  let report: ReturnType<Runtime['unresolvedOwnership']> = { requestIds: [], storageUnavailable: false, reconciliation: 'complete' };
  const listeners = new Set<(state: Runtime['state']) => void>();
  const counts = { dispatch: 0, dispose: 0, engine: 0, native: 0 };
  let duringDispatch: (() => Promise<void>) | undefined;
  let disposeFailure = false;
  let engineFailure = false;
  const runtime: Runtime = {
    get state() { return state; },
    startup: { state: 'ready', knownRequestIds: [] },
    subscribe(listener) { listeners.add(listener); return () => { listeners.delete(listener); }; },
    async dispatch() { counts.dispatch++; await duringDispatch?.(); return state; },
    async reconcileStartup() { return { state: 'ready', knownRequestIds: [] }; },
    unresolvedOwnership() { return structuredClone(report); },
    dispose() { counts.dispose++; if (disposeFailure) throw new Error('dispose uncertain'); listeners.clear(); return report; },
  };
  const bindings: RecordNativeBindings = {
    native: { async readFacts() { return { version: 1, providerEpoch: 'native-test', revision: 1,
      readiness: [], shortcut: { effective: null, label: null, registration: 'unknown', externallyManaged: false, draft: null } }; } },
    penConfig: { async read() { return { persisted: null, effective: null }; },
      async writeApplyAndReadBack() { return { status: 'unknown', effective: null }; } },
    clipboard: { async copyLocal() { return { status: 'refused', reason: 'not-saved' }; } },
    async close() { counts.native++; },
  };
  const host = createRecordProviderHost({ runtime, nativeBindings: bindings, captureFacts,
    async closeEngine() { counts.engine++; if (engineFailure) { engineFailure = false; throw new Error('close failed'); } } });
  const snapshot = () => {
    let current!: CapabilitySnapshot;
    const remove = host.record.observe(value => { current = value; }); remove(); return current;
  };
  return { host, counts, snapshot,
    report(value: typeof report) { report = value; },
    state(value: typeof state) { state = value; for (const listener of listeners) listener(state); },
    blockDispatch(value: () => Promise<void>) { duringDispatch = value; },
    failDispose() { disposeFailure = true; }, failEngine() { engineFailure = true; },
  };
}
function refresh(f: ReturnType<typeof fixture>, requestId: string) {
  const current = f.snapshot();
  return { type: 'refresh' as const, requestId, expected: { providerEpoch: current.providerEpoch, revision: current.revision } };
}

test('close admission atomically prevents later commands and releases only cleared owned resources', async () => {
  const f = fixture();
  assert.equal(await f.host.canClose(), true);
  const result = await f.host.record.execute(refresh(f, 'after-close-admission'));
  assert.equal(result.status, 'refused'); assert.equal(f.counts.dispatch, 0);
  await Promise.all([f.host.close(), f.host.close()]);
  assert.deepEqual(f.counts, { dispatch: 0, dispose: 1, engine: 1, native: 1 });
});

test('queued/in-flight commands, incomplete reconciliation and processing facts block close', { timeout: 2000 }, async () => {
  const f = fixture();
  let release!: () => void;
  let entered!: () => void;
  const dispatched = new Promise<void>(resolve => { entered = resolve; });
  const pending = new Promise<void>(resolve => { release = resolve; });
  f.blockDispatch(async () => { entered(); await pending; });
  const operation = f.host.record.execute(refresh(f, 'pending-refresh'));
  assert.equal(await f.host.canClose(), false);
  await dispatched;
  release(); await operation;
  for (const reconciliation of ['pending', 'failed', 'interrupted'] as const) {
    f.report({ requestIds: [], storageUnavailable: false, reconciliation });
    assert.equal(await f.host.canClose(), false);
  }
  f.report({ requestIds: [], storageUnavailable: true, reconciliation: 'complete' });
  assert.equal(await f.host.canClose(), false);
  f.report({ requestIds: [], storageUnavailable: false, reconciliation: 'complete' });
  f.state({ phase: 'processing', captureId: 'owned-capture', readiness: { ready: true } });
  assert.equal(await f.host.canClose(), false, 'native IDs alone cannot clear Record processing');
  await assert.rejects(f.host.close(), /ownership remains unresolved/);
  assert.equal(f.counts.engine, 0); assert.equal(f.counts.native, 0);
});

test('client detach removes only its own observations and does not dispose the runtime', async () => {
  const f = fixture(); let observations = 0;
  f.host.record.observe(() => { observations++; });
  assert.equal(observations, 1);
  f.host.record.detach();
  f.state({ phase: 'idle', readiness: { ready: false, reason: 'synthetic unavailable' } });
  assert.equal(observations, 1); assert.equal(f.counts.dispose, 0);
  await f.host.close(); assert.equal(f.counts.dispose, 1);
});

test('cleanup retries retain their stage; uncertain provider disposal never becomes clearance', async () => {
  const f = fixture(); f.failEngine();
  await assert.rejects(f.host.close(), /close failed/);
  assert.equal(f.counts.native, 0);
  await f.host.close();
  assert.deepEqual(f.counts, { dispatch: 0, dispose: 1, engine: 2, native: 1 });
  const unknown = fixture(); unknown.failDispose();
  await assert.rejects(unknown.host.close(), /dispose uncertain/);
  assert.equal(await unknown.host.canClose(), false);
  await assert.rejects(unknown.host.close(), /cleanup ownership remains unknown/);
  assert.equal(unknown.counts.engine, 0); assert.equal(unknown.counts.native, 0);
});

test('pending initial output facts cannot clear an armed reservation or active capture', { timeout: 2000 }, async () => {
  let release!: (facts: Awaited<ReturnType<NonNullable<RecordCapabilityProviderOptions['captureFacts']>['readFacts']>>) => void;
  const pending = new Promise<Awaited<ReturnType<NonNullable<RecordCapabilityProviderOptions['captureFacts']>['readFacts']>>>(resolve => { release = resolve; });
  const f = fixture({ readFacts: () => pending });
  let settled = false;
  const clearance = f.host.canClose().then(value => { settled = true; return value; });
  await Promise.resolve();
  assert.equal(settled, false);
  assert.equal(f.counts.dispose, 0); assert.equal(f.counts.engine, 0);
  release({ version: 1, providerEpoch: 'outputs', revision: 1, captures: [],
    nextShortcutAdmission: { state: 'armed', admissionId: 'reserved', delivery: 'local-only', expiresAt: '2099-01-01T00:00:00Z' } });
  assert.equal(await clearance, false);
  await assert.rejects(f.host.close(), /ownership remains unresolved/);
  assert.equal(f.counts.dispose, 0); assert.equal(f.counts.engine, 0);
  for (const phase of ['recording', 'processing', 'unknown'] as const) {
    const active = fixture({ async readFacts() { return { version: 1, providerEpoch: 'outputs', revision: 1,
      captures: [{ captureId: 'active', delivery: 'local-only', phase, outcomes: [] }], nextShortcutAdmission: { state: 'idle' } }; } });
    assert.equal(await active.host.canClose(), false);
    assert.equal(active.counts.dispose, 0);
  }
  const unavailable = fixture({ async readFacts() { throw new Error('facts unavailable'); } });
  assert.equal(await unavailable.host.canClose(), false);
});
