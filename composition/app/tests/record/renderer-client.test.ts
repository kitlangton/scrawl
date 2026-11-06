import assert from 'node:assert/strict';
import { build } from 'esbuild';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { createRecordFixture } from '@apparent/record/fixtures';
import type { CapabilitySnapshot, CommandResult, RecordCommand } from '@apparent/record/capabilities';
import type { RecordRendererPort } from '../../src/record/wire.js';

async function connect(port: RecordRendererPort) {
  const output = await build({ entryPoints: [fileURLToPath(new URL('../../src/record/renderer-client.ts', import.meta.url))],
    bundle: true, platform: 'browser', format: 'esm', write: false });
  const module = await import(`data:text/javascript;base64,${Buffer.from(output.outputFiles[0].text).toString('base64')}#${Math.random()}`);
  return module.connectRecordClient(port);
}

function command(snapshot: CapabilitySnapshot, requestId: string): RecordCommand {
  return { type: 'refresh', requestId, expected: { providerEpoch: snapshot.providerEpoch, revision: snapshot.revision } };
}

function portFor(source: ReturnType<typeof createRecordFixture>, execute: (command: RecordCommand) => Promise<CommandResult> = source.client.execute.bind(source.client)) {
  const listeners = new Set<(snapshot: CapabilitySnapshot) => void>();
  let unsubscriptions = 0;
  const removeProviderObserver = source.client.observe(snapshot => {
    for (const listener of [...listeners]) listener(snapshot);
  });
  const port: RecordRendererPort = {
    observe(listener) { listeners.add(listener); return () => { listeners.delete(listener); unsubscriptions++; }; },
    async read() { return source.driver.current(); },
    execute,
  };
  return { port, source, listenerCount: () => listeners.size,
    unsubscriptions: () => unsubscriptions,
    emit(snapshot: CapabilitySnapshot) { for (const listener of [...listeners]) listener(snapshot); },
    cleanup: () => removeProviderObserver() };
}

test('connect installs observation before read and public observe synchronously emits the full cached snapshot', async () => {
  const source = createRecordFixture('permission-granted-relaunch-needed');
  const order: string[] = [];
  const base = portFor(source);
  const port: RecordRendererPort = {
    ...base.port,
    observe(listener) { order.push('subscribe'); return base.port.observe(listener); },
    async read() { order.push('read'); return base.port.read(); },
  };
  const client = await connect(port);
  const seen: CapabilitySnapshot[] = [];
  client.observe((snapshot: CapabilitySnapshot) => seen.push(snapshot));
  assert.deepEqual(order, ['subscribe', 'read']);
  assert.deepEqual(seen, [source.driver.current()]);
  assert.equal(seen[0].readiness.requirements[0].relaunchNeeded, true);
  client.detach();
  base.cleanup();
});

test('full replacements advance monotonically and throwing observers do not stop peers', async () => {
  const source = createRecordFixture();
  const bridge = portFor(source);
  const client = await connect(bridge.port);
  let throwingCalls = 0;
  const removeThrowing = client.observe(() => { throwingCalls++; throw new Error('observer error'); });
  const delivered: number[] = [];
  const removeHealthy = client.observe((snapshot: CapabilitySnapshot) => delivered.push(snapshot.revision));
  assert.equal(throwingCalls, 1, 'public observers receive the cached snapshot synchronously');
  source.driver.disconnect();
  assert.equal(delivered.at(-1), source.driver.current().revision);
  const stale = { ...source.driver.current(), revision: 1 };
  bridge.emit(stale);
  assert.equal(delivered.at(-1), source.driver.current().revision);
  removeThrowing();
  removeHealthy();
  client.detach();
  bridge.cleanup();
});

test('invoke rejection returns outcome-unknown once; detach unsubscribes without cancelling capture', async () => {
  const source = createRecordFixture('unsubscribe-during-recording');
  let calls = 0;
  let providerDetach = 0;
  source.client.detach = () => { providerDetach++; };
  const bridge = portFor(source, async () => { calls++; throw new Error('lost IPC reply after dispatch'); });
  const client = await connect(bridge.port);
  const current = source.driver.current();
  const request = command(current, 'uncertain-1');
  const result = await client.execute(request);
  assert.deepEqual(result, { status: 'outcome-unknown', requestId: request.requestId, snapshot: current });
  assert.equal(calls, 1, 'uncertain commands are never retried');
  client.detach();
  assert.equal(bridge.unsubscriptions(), 1);
  assert.equal(providerDetach, 0, 'renderer detach does not call the provider/client detach method');
  assert.equal(source.driver.captureContinues(), true);
  assert.deepEqual(source.driver.commands(), []);
  bridge.cleanup();
});

test('malformed commands are rejected locally and stale result snapshots cannot roll the cache back', async () => {
  const source = createRecordFixture();
  let calls = 0;
  const current = source.driver.current();
  const bridge = portFor(source, async cmd => {
    calls++;
    return { status: 'accepted', requestId: cmd.requestId, snapshot: current };
  });
  const client = await connect(bridge.port);
  await assert.rejects(client.execute({ ...command(current, 'invalid-1'), ownerId: 'record' } as any));
  assert.equal(calls, 0);

  const sent = command(current, 'stale-result-1');
  source.driver.disconnect();
  const latest = source.driver.current();
  const result = await client.execute(sent);
  assert.equal(result.status, 'accepted');
  assert.deepEqual(result.snapshot, latest, 'transport adapter keeps the newest complete provider snapshot');
  assert.equal(calls, 1);
  client.detach();
  bridge.cleanup();
});
