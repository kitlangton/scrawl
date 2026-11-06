import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { createRecordFixture } from '@apparent/record/fixtures';
import type { CapabilitySnapshot } from '@apparent/record/capabilities';
import { RECORD_CHANNELS } from '../../src/record/wire.ts';

async function fixture() {
  const source = createRecordFixture('unsubscribe-during-recording');
  const listeners = new Map<string, Set<(...args: unknown[]) => void>>();
  const calls: string[] = [];
  const payloads: Array<{ channel: string; payload: any }> = [];
  let replyMode: 'valid' | 'bad-correlation' | 'reject-command' = 'valid';
  let resolveUnsubscribe!: () => void;
  const unsubscribed = new Promise<void>(resolve => { resolveUnsubscribe = resolve; });
  const ipcRenderer = {
    on(channel: string, listener: (...args: unknown[]) => void) {
      let values = listeners.get(channel);
      if (!values) listeners.set(channel, values = new Set());
      values.add(listener);
    },
    removeListener(channel: string, listener: (...args: unknown[]) => void) { listeners.get(channel)?.delete(listener); },
    async invoke(channel: string, payload: any) {
      calls.push(channel);
      payloads.push({ channel, payload });
      if (channel === RECORD_CHANNELS.subscribe) {
        emit(payload.subscriptionId, source.driver.current());
        return { requestId: payload.requestId, subscriptionId: payload.subscriptionId, subscribed: true };
      }
      if (channel === RECORD_CHANNELS.unsubscribe) {
        resolveUnsubscribe();
        return { requestId: payload.requestId, subscriptionId: payload.subscriptionId, subscribed: false };
      }
      if (channel === RECORD_CHANNELS.read) return { requestId: payload.requestId, snapshot: source.driver.current() };
      if (channel === RECORD_CHANNELS.execute) {
        if (replyMode === 'reject-command') throw new Error('lost invoke reply');
        const result = await source.client.execute(payload.command);
        return { requestId: replyMode === 'bad-correlation' ? 'other-request' : payload.requestId, result };
      }
      throw new Error(`Unexpected channel ${channel}`);
    },
  };
  function emit(subscriptionId: string, snapshot: CapabilitySnapshot) {
    for (const listener of [...(listeners.get(RECORD_CHANNELS.snapshot) ?? [])]) listener({}, { subscriptionId, snapshot });
  }
  (globalThis as any).__recordPreloadTestIpc = ipcRenderer;
  const output = await build({ entryPoints: [fileURLToPath(new URL('../../src/record/preload.cts', import.meta.url))],
    bundle: true, platform: 'node', format: 'esm', write: false,
    plugins: [{ name: 'controlled-electron-preload-record', setup(builder) {
      builder.onResolve({ filter: /^electron$/ }, () => ({ path: 'electron', namespace: 'controlled' }));
      builder.onLoad({ filter: /.*/, namespace: 'controlled' }, () => ({ contents: 'export const ipcRenderer = globalThis.__recordPreloadTestIpc;' }));
    } }] });
  const module = await import(`data:text/javascript;base64,${Buffer.from(output.outputFiles[0].text).toString('base64')}#${Math.random()}`);
  delete (globalThis as any).__recordPreloadTestIpc;
  return { port: module.createRecordPreloadPort(), source, listeners, calls, payloads, emit, unsubscribed,
    setMode(value: typeof replyMode) { replyMode = value; } };
}

test('preload port subscribes before read and sends full trusted provider snapshots', async () => {
  const f = await fixture();
  const seen: CapabilitySnapshot[] = [];
  const remove = f.port.observe((snapshot: CapabilitySnapshot) => seen.push(snapshot));
  const initial = await f.port.read();
  assert.deepEqual(f.calls.slice(0, 2), [RECORD_CHANNELS.subscribe, RECORD_CHANNELS.read]);
  assert.deepEqual(initial, f.source.driver.current());
  assert.deepEqual(seen[0], initial, 'subscribe push may arrive before the read handshake');
  f.source.driver.disconnect();
  const disconnected = f.source.driver.current();
  f.emit('record-bridge-invalid-subscription', disconnected);
  assert.equal(seen.at(-1)?.connection, 'connected', 'foreign subscription pushes are ignored');
  const subscription = f.payloads.find(value => value.channel === RECORD_CHANNELS.subscribe)!;
  f.emit(subscription.payload.subscriptionId, disconnected);
  assert.equal(seen.at(-1)?.connection, 'disconnected');
  remove();
});

test('preload rejects malformed commands before IPC and validates correlated provider results', async () => {
  const f = await fixture();
  const snapshot = await f.port.read();
  const command = { type: 'refresh', requestId: 'request-good', expected: { providerEpoch: snapshot.providerEpoch, revision: snapshot.revision } } as const;
  const before = f.calls.filter(value => value === RECORD_CHANNELS.execute).length;
  await assert.rejects(f.port.execute({ ...command, ownerId: 'onboarding' } as any));
  assert.equal(f.calls.filter(value => value === RECORD_CHANNELS.execute).length, before);
  assert.equal((await f.port.execute(command as any)).status, 'accepted');
  f.setMode('bad-correlation');
  await assert.rejects(f.port.execute({ ...command, requestId: 'request-bad-correlation' } as any), /correlation/);
});

test('observer exceptions are isolated and last unsubscribe never cancels capture', async () => {
  const f = await fixture();
  const delivered: number[] = [];
  const removeThrowing = f.port.observe(() => { throw new Error('observer failed'); });
  const removeHealthy = f.port.observe((snapshot: CapabilitySnapshot) => delivered.push(snapshot.revision));
  await f.port.read();
  assert.equal(f.source.driver.captureContinues(), true);
  removeThrowing();
  removeHealthy();
  await f.unsubscribed;
  assert(delivered.length >= 1);
  assert.equal(f.listeners.get(RECORD_CHANNELS.snapshot)?.size ?? 0, 0);
  assert.equal(f.source.driver.captureContinues(), true);
  assert.deepEqual(f.source.driver.commands(), []);
});
