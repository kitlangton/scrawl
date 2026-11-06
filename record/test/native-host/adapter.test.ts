import assert from 'node:assert/strict';
import { test } from 'node:test';
import type { NativeHostPortInput } from '../../src/backend/host/native/index.ts';
import { createRecordNativePort } from '../../src/backend/host/native/index.ts';
import { createRecordCapabilityProvider } from '../../src/backend/capabilities/provider.ts';
import type { RecordHostRuntime } from '../../src/backend/host/runtime.ts';

const snap = (revision = 3, epoch = 'desktop-a') => ({ api: 1 as const, processEpoch: epoch, revision,
  scope: 'running-desktop-process' as const, freshness: 'cached-gui-observation' as const,
  permissions: [{ id: 'microphone', status: 'granted' as const, usableNow: false, relaunchNeeded: true, actions: ['open-system-settings' as const] }],
  shortcuts: { record: { configured: { keyCode: 83, modifiers: 2560, label: '⌥⇧S' }, effective: null,
    registration: 'unknown' as const, canValidate: true, canSave: false }, paste: { configured: null, effective: null, registration: 'unavailable' as const, canValidate: false, canSave: false } } });
function runtime(): RecordHostRuntime {
  const state = { phase: 'idle' as const, readiness: { ready: false, reason: 'fixture' } };
  return { state, startup: { state: 'ready', knownRequestIds: [] }, subscribe: () => () => undefined, dispatch: async () => state,
    reconcileStartup: async () => ({ state: 'ready', knownRequestIds: [] }), unresolvedOwnership: () => ({ requestIds: [], storageUnavailable: false, reconciliation: 'complete' }),
    dispose: () => ({ requestIds: [], storageUnavailable: false, reconciliation: 'complete' }) } as unknown as RecordHostRuntime;
}
function command(client: ReturnType<typeof createRecordCapabilityProvider>, type: string, fields: Record<string, unknown> = {}) {
  const state = client.current(); return client.execute({ type, ...fields, requestId: `r-${Math.random()}`, expected: { providerEpoch: state.providerEpoch, revision: state.revision } } as never);
}
const until = async (predicate: () => boolean) => { for (let i = 0; i < 30 && !predicate(); i++) await new Promise(resolve => setTimeout(resolve, 0)); assert.ok(predicate()); };

test('maps cached permission and configured/effective registration facts without conflation', async () => {
  const port = { readSnapshot: async () => snap() } as NativeHostPortInput;
  const client = createRecordCapabilityProvider({ runtime: runtime(), native: createRecordNativePort(port, { requiredPermissionIds: ['microphone', 'camera'] }), providerEpoch: 'record' });
  await until(() => client.current().readiness.requirements.some(x => x.id === 'microphone'));
  const current = client.current();
  const mic = current.readiness.requirements.find(x => x.id === 'microphone')!;
  assert.equal(mic.status, 'granted'); assert.equal(mic.usableNow, false); assert.equal(mic.relaunchNeeded, true);
  assert.equal(client.current().readiness.requirements.find(x => x.id === 'camera')?.status, 'unavailable');
  assert.equal(current.shortcut.configured, '⌥⇧S'); assert.equal(current.shortcut.effective, null); assert.equal(current.shortcut.registration, 'unknown');
  client.detach();
});

test('edits validate returned candidate, cancel does not save, and stale host revision invalidates edit', async () => {
  let current = snap(); const calls: string[] = [];
  const port: NativeHostPortInput = { readSnapshot: async () => current,
    requestPermission: async () => ({ status: 'accepted' }), openSystemSettings: async () => ({ status: 'accepted' }),
    validateRecordShortcut: async (key, modifiers) => { calls.push(`validate:${key}:${modifiers}`); return { status: 'accepted', shortcut: { keyCode: 82, modifiers, label: '⌥⇧R' } }; },
    saveRecordShortcut: async shortcut => { calls.push(`save:${shortcut.keyCode}:${shortcut.label}`); return { status: 'accepted' }; } };
  const native = createRecordNativePort(port, { requiredPermissionIds: [] });
  const client = createRecordCapabilityProvider({ runtime: runtime(), native, providerEpoch: 'record' });
  await until(() => client.current().shortcut.configured === '⌥⇧S');
  const resBegin = await command(client, 'shortcut-begin-edit'); assert.equal(resBegin.status, 'accepted'); await until(() => !!client.current().shortcut.draft);
  const session = client.current().shortcut.draft!.sessionId;
  const invalid = await command(client, 'shortcut-submit', { sessionId: session, value: 'Ctrl++R' }); assert.equal(invalid.status, 'refused'); assert.deepEqual(calls, []);
  await command(client, 'shortcut-submit', { sessionId: session, value: 'Alt+Shift+R' });
  await until(() => calls.length === 1);
  // Validation result keycode and label are used verbatim; cancel never writes.
  await command(client, 'shortcut-cancel', { sessionId: session }); assert.equal(calls.length, 1);
  await command(client, 'shortcut-begin-edit'); await until(() => !!client.current().shortcut.draft);
  const next = client.current().shortcut.draft!.sessionId;
  await command(client, 'shortcut-submit', { sessionId: next, value: 'Alt+Shift+R' }); await until(() => calls.length === 2);
  current = snap(4);
  // This is an external producer revision. A read invalidates the old candidate before save.
  await command(client, 'refresh'); await until(() => !client.current().shortcut.draft);
  const result = await command(client, 'shortcut-save', { sessionId: next }); assert.equal(result.status, 'refused'); assert.equal(calls.length, 2);
  client.detach();
});

test('passes the accepted epoch/revision exactly and never retries unknown mutation', async () => {
  let count = 0; const seen: unknown[] = [];
  const observed = snap(); observed.permissions[0].actions = ['request-permission'];
  const port = { readSnapshot: async () => observed, requestPermission: async (_id: string, state: unknown) => { seen.push(state); count++; return { status: 'outcome-unknown' as const }; },
    openSystemSettings: async () => ({ status: 'accepted' as const }), validateRecordShortcut: async () => ({ status: 'outcome-unknown' as const }), saveRecordShortcut: async () => ({ status: 'accepted' as const }) } satisfies NativeHostPortInput;
  const client = createRecordCapabilityProvider({ runtime: runtime(), native: createRecordNativePort(port, { requiredPermissionIds: ['microphone'] }), providerEpoch: 'record' });
  await until(() => client.current().readiness.requirements.some(x => x.id === 'microphone'));
  const result = await command(client, 'request-permission', { requirementId: 'microphone' });
  assert.equal(result.status, 'outcome-unknown'); assert.equal(count, 1); assert.deepEqual(seen, [{ processEpoch: 'desktop-a', revision: 3 }]);
  client.detach();
});

test('saves only the native validation result and leaves readback to a later snapshot', async () => {
  const initial = snap(); initial.shortcuts.record.canSave = true; initial.shortcuts.record.registration = 'registered';
  const saved: unknown[] = [];
  const port: NativeHostPortInput = { readSnapshot: async () => initial,
    requestPermission: async () => ({ status: 'accepted' }), openSystemSettings: async () => ({ status: 'accepted' }),
    validateRecordShortcut: async (_key, modifiers) => ({ status: 'accepted', shortcut: { keyCode: 82, modifiers, label: '⌥⇧R' } }),
    saveRecordShortcut: async shortcut => { saved.push(shortcut); return { status: 'refused', reason: 'registration-refused' }; } };
  const client = createRecordCapabilityProvider({ runtime: runtime(), native: createRecordNativePort(port, { requiredPermissionIds: [] }), providerEpoch: 'record-save' });
  await until(() => client.current().shortcut.configured === '⌥⇧S');
  await command(client, 'shortcut-begin-edit'); await until(() => !!client.current().shortcut.draft);
  const sessionId = client.current().shortcut.draft!.sessionId;
  await command(client, 'shortcut-submit', { sessionId, value: 'Alt+Shift+R' });
  await until(() => !!client.current().shortcut.draft && client.current().shortcut.draft!.value === 'Alt+Shift+R');
  const result = await command(client, 'shortcut-save', { sessionId });
  assert.equal(result.status, 'refused'); assert.deepEqual(saved, [{ keyCode: 82, modifiers: 2560, label: '⌥⇧R' }]);
  assert.equal(client.current().shortcut.configured, '⌥⇧S'); assert.equal(client.current().shortcut.effective, null);
  client.detach();
});
