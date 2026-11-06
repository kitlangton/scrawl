import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { build } from 'esbuild';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';
import { createRecordFixture } from '@apparent/record/fixtures';
import type { CapabilitySnapshot, CommandResult, RecordClient, RecordCommand } from '@apparent/record/capabilities';
import { RECORD_CHANNELS } from '../../src/record/wire.ts';

async function fixture() {
  const handlers = new Map<string, (...args: any[]) => Promise<any>>();
  const ipc = {
    handle(channel: string, handler: (...args: any[]) => Promise<any>) {
      if (handlers.has(channel)) throw new Error('Duplicate handler');
      handlers.set(channel, handler);
    },
    removeHandler(channel: string) { handlers.delete(channel); },
  };
  (globalThis as any).__recordBridgeTestIpc = ipc;
  const output = await build({ entryPoints: [fileURLToPath(new URL('../../src/record/bridge.cts', import.meta.url))],
    bundle: true, platform: 'node', format: 'esm', write: false,
    plugins: [{ name: 'controlled-electron-record', setup(builder) {
      builder.onResolve({ filter: /^electron$/ }, () => ({ path: 'electron', namespace: 'controlled' }));
      builder.onLoad({ filter: /.*/, namespace: 'controlled' }, () => ({ contents: 'export const ipcMain = globalThis.__recordBridgeTestIpc;' }));
    } }] });
  const module = await import(`data:text/javascript;base64,${Buffer.from(output.outputFiles[0].text).toString('base64')}#${Math.random()}`);
  delete (globalThis as any).__recordBridgeTestIpc;
  const entryUrl = 'file:///trusted/record-test.html';
  const pushed: unknown[] = [];
  let contentsDestroyed = false;
  let windowDestroyed = false;
  const contents = Object.assign(new EventEmitter(), {
    mainFrame: { url: entryUrl },
    isDestroyed: () => contentsDestroyed,
    send: (channel: string, payload: unknown) => { if (channel === RECORD_CHANNELS.snapshot) pushed.push(payload); },
  });
  const window = Object.assign(new EventEmitter(), {
    webContents: contents,
    isDestroyed: () => windowDestroyed,
  });
  const event = { sender: contents, senderFrame: contents.mainFrame };
  return { handlers, ipc, register: module.registerRecordBridge, window, contents, event, entryUrl, pushed,
    destroyContents: () => { contentsDestroyed = true; }, destroyWindow: () => { windowDestroyed = true; } };
}

function refresh(snapshot: CapabilitySnapshot, requestId: string): RecordCommand {
  return { type: 'refresh', requestId, expected: { providerEpoch: snapshot.providerEpoch, revision: snapshot.revision } };
}

test('subscribes before read, returns the full snapshot, and forwards complete provider replacements', async () => {
  const f = await fixture();
  const source = createRecordFixture();
  let detached = 0;
  const client: RecordClient = { ...source.client, detach() { detached++; } };
  const dispose = f.register({ window: f.window, entryUrl: f.entryUrl, client });
  try {
    const subscribe = f.handlers.get(RECORD_CHANNELS.subscribe)!;
    const read = f.handlers.get(RECORD_CHANNELS.read)!;
    const subscribed = await subscribe(f.event, { requestId: 'sub-1', subscriptionId: 'watch-1' });
    assert.deepEqual(subscribed, { requestId: 'sub-1', subscriptionId: 'watch-1', subscribed: true });
    assert.deepEqual((f.pushed[0] as any).snapshot, source.driver.current());
    const handshake = await read(f.event, { requestId: 'read-1' });
    assert.deepEqual(handshake, { requestId: 'read-1', snapshot: source.driver.current() });

    const command = refresh(source.driver.current(), 'refresh-1');
    const result = await f.handlers.get(RECORD_CHANNELS.execute)!(f.event, { requestId: command.requestId, command });
    assert.equal(result.result.status, 'accepted');
    assert.deepEqual(source.driver.commands(), [command]);

    source.driver.disconnect();
    const last = f.pushed.at(-1) as any;
    assert.equal(last.subscriptionId, 'watch-1');
    assert.equal(last.snapshot.connection, 'disconnected');
    assert.equal(last.snapshot.revision, source.driver.current().revision);
  } finally {
    dispose();
    assert.equal(detached, 0, 'bridge disposal unsubscribes its observer but does not detach the shared RecordClient');
  }
});

test('rejects extra selectors, foreign windows and subframes before RecordClient.execute', async () => {
  const f = await fixture();
  const source = createRecordFixture();
  const dispose = f.register({ window: f.window, entryUrl: f.entryUrl, client: source.client });
  try {
    const handler = f.handlers.get(RECORD_CHANNELS.execute)!;
    const command = refresh(source.driver.current(), 'blocked-1');
    await assert.rejects(handler(f.event, { requestId: command.requestId, command, ownerId: 'onboarding' }));
    await assert.rejects(handler({ ...f.event, sender: new EventEmitter() }, { requestId: command.requestId, command }));
    await assert.rejects(handler({ ...f.event, senderFrame: { url: f.entryUrl } }, { requestId: command.requestId, command }));
    await assert.rejects(handler({ ...f.event, senderFrame: { url: 'file:///foreign.html' } }, { requestId: command.requestId, command }));
    assert.deepEqual(source.driver.commands(), []);
  } finally { dispose(); }
});

test('revocation denies a promoted queued command and late provider completion stays unknown', { timeout: 5_000 }, async () => {
  const f = await fixture();
  const source = createRecordFixture('unsubscribe-during-recording');
  let providerCalls = 0;
  let detachCalls = 0;
  const pending: Array<(result: CommandResult) => void> = [];
  let resolveAdmitted!: () => void;
  const admitted = new Promise<void>(resolve => { resolveAdmitted = resolve; });
  const client: RecordClient = {
    observe: source.client.observe.bind(source.client),
    execute(command) {
      providerCalls++;
      if (providerCalls === 8) resolveAdmitted();
      return new Promise(resolve => pending.push(result => resolve(result)));
    },
    detach() { detachCalls++; },
  };
  const dispose = f.register({ window: f.window, entryUrl: f.entryUrl, client });
  const handler = f.handlers.get(RECORD_CHANNELS.execute)!;
  const calls = Array.from({ length: 8 }, (_, index) => {
    const command = refresh(source.driver.current(), `pending-${index}`);
    return handler(f.event, { requestId: command.requestId, command });
  });
  await admitted;
  assert.equal(providerCalls, 8);
  const queuedCommand = refresh(source.driver.current(), 'queued-1');
  const queued = handler(f.event, { requestId: queuedCommand.requestId, command: queuedCommand });
  dispose();
  await assert.rejects(queued);
  const results = await Promise.all(calls);
  assert(results.every((reply: any) => reply.result.status === 'outcome-unknown'));
  assert.equal(providerCalls, 8, 'revocation must stop queued work before provider dispatch');
  assert.equal(source.driver.captureContinues(), true, 'bridge cleanup cannot cancel an owner capture');
  assert.equal(detachCalls, 0, 'bridge cleanup cannot detach the caller-owned RecordClient');
  for (const resolve of pending) resolve({ status: 'accepted', requestId: 'late', snapshot: source.driver.current() });
  await Promise.resolve();
});

test('generation cleanup removes only its own handlers and permits a successor registration', async () => {
  const f = await fixture();
  const first = createRecordFixture();
  const oldDispose = f.register({ window: f.window, entryUrl: f.entryUrl, client: first.client });
  oldDispose();
  const second = createRecordFixture();
  const dispose = f.register({ window: f.window, entryUrl: f.entryUrl, client: second.client });
  oldDispose();
  assert.equal(f.handlers.size, 4);
  const reply = await f.handlers.get(RECORD_CHANNELS.read)!(f.event, { requestId: 'successor-read' });
  assert.deepEqual(reply.snapshot, second.driver.current());
  dispose();
  assert.equal(f.handlers.size, 0);
});
