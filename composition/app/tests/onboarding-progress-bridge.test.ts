import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { build } from 'esbuild';
import { test } from 'node:test';
import { fileURLToPath } from 'node:url';

async function fixture() {
  const handlers = new Map<string, (...args: any[]) => Promise<any>>();
  const ipc = {
    handle(channel: string, handler: (...args: any[]) => Promise<any>) {
      if (handlers.has(channel)) throw new Error('Duplicate handler');
      handlers.set(channel, handler);
    },
    removeHandler(channel: string) { handlers.delete(channel); },
  };
  (globalThis as any).__bridgeTestIpc = ipc;
  const output = await build({ entryPoints: [fileURLToPath(new URL('../src/onboarding-progress-bridge.cts', import.meta.url))],
    bundle: true, platform: 'node', format: 'esm', write: false,
    plugins: [{ name: 'controlled-electron-transport', setup(builder) {
      builder.onResolve({ filter: /^electron$/ }, () => ({ path: 'electron', namespace: 'controlled' }));
      builder.onLoad({ filter: /.*/, namespace: 'controlled' }, () => ({ contents: 'export const ipcMain = globalThis.__bridgeTestIpc;' }));
    } }] });
  const module = await import(`data:text/javascript;base64,${Buffer.from(output.outputFiles[0].text).toString('base64')}#${Math.random()}`);
  delete (globalThis as any).__bridgeTestIpc;
  const register = module.registerOnboardingProgressBridge ?? module.default.registerOnboardingProgressBridge;
  const entryUrl = 'file:///trusted/index.html';
  const contents = Object.assign(new EventEmitter(), { mainFrame: { url: entryUrl }, isDestroyed: () => false });
  const window = Object.assign(new EventEmitter(), { webContents: contents, isDestroyed: () => false });
  const event = { sender: contents, senderFrame: contents.mainFrame };
  return { handlers, ipc, register, window, contents, entryUrl, event };
}
const READ = 'apparent:onboarding-progress:read';
const CAS = 'apparent:onboarding-progress:compare-and-swap';

test('promoted queued CAS is revoked before provider dispatch', async () => {
  const f = await fixture();
  let writes = 0, readCalls = 0;
  let releaseFirst!: (value: any) => void;
  let dispose!: () => void;
  dispose = f.register({ window: f.window, entryUrl: f.entryUrl, store: {
    read() { return ++readCalls === 1 ? new Promise(resolve => { releaseFirst = resolve; }) : new Promise(() => {}); },
    async compareAndSwap() { writes++; return { status: 'saved', revision: 1 }; },
  } });
  try {
    const reads = Array.from({ length: 8 }, () => f.handlers.get(READ)!(f.event));
    const settledReads = Promise.allSettled(reads);
    await Promise.resolve(); // All eight providers occupy their admitted slots.
    assert.equal(readCalls, 8);
    const queued = f.handlers.get(CAS)!(f.event, { expected: 0, value: {} });
    // Reading the valid provider revision schedules revocation immediately
    // before release promotes the queued caller. No sleeps or private APIs.
    releaseFirst({ get revision() { queueMicrotask(dispose); return 0; }, value: null });
    assert.equal((await queued).status, 'unknown');
    await settledReads;
    assert.equal(writes, 0, 'queued work must not call its provider after revocation');
    assert.equal(f.handlers.size, 0);
  } finally { dispose(); }
});

test('registration rollback and repeated disposal preserve a successor', async () => {
  const f = await fixture();
  const store = { async read() { return { revision: 0, value: null }; }, async compareAndSwap() { return { status: 'saved', revision: 1 }; } };
  f.ipc.handle(CAS, async () => null);
  assert.throws(() => f.register({ window: f.window, entryUrl: f.entryUrl, store }), /Duplicate/);
  assert.equal(f.handlers.has(READ), false);
  f.ipc.removeHandler(CAS);
  const dispose = f.register({ window: f.window, entryUrl: f.entryUrl, store });
  assert.throws(() => f.register({ window: f.window, entryUrl: f.entryUrl, store }), /already registered/);
  dispose();
  const successor = f.register({ window: f.window, entryUrl: f.entryUrl, store });
  dispose();
  assert.deepEqual(await f.handlers.get(READ)!(f.event), { revision: 0, value: null });
  successor();
  assert.equal(f.handlers.size, 0);
});

test('malformed owner selectors and foreign frames never reach the provider', async () => {
  const f = await fixture();
  let writes = 0;
  const dispose = f.register({ window: f.window, entryUrl: f.entryUrl, store: {
    async read() { return { revision: 0, value: null }; },
    async compareAndSwap() { writes++; return { status: 'saved', revision: 1 }; },
  } });
  try {
    await assert.rejects(f.handlers.get(CAS)!(f.event, { expected: 0, value: {}, ownerId: 'record' }));
    await assert.rejects(f.handlers.get(CAS)!({ ...f.event, senderFrame: {} }, { expected: 0, value: {} }));
    assert.equal(writes, 0);
  } finally { dispose(); }
});

test('destroyed-window cleanup never dereferences its invalid webContents getter', async () => {
  const f = await fixture();
  let destroyed = false;
  Object.defineProperty(f.window, 'webContents', { get() { if (destroyed) throw new Error('Object has been destroyed'); return f.contents; } });
  const dispose = f.register({ window: f.window, entryUrl: f.entryUrl, store: {
    async read() { return { revision: 0, value: null }; }, async compareAndSwap() { return { status: 'saved', revision: 1 }; },
  } });
  destroyed = true;
  assert.doesNotThrow(() => f.window.emit('closed'));
  assert.equal(f.handlers.size, 0);
  assert.doesNotThrow(dispose);
});
