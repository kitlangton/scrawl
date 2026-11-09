import { test } from 'node:test';
import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import { setImmediate as tick, setTimeout as delay } from 'node:timers/promises';
import type { App } from 'electron';
import { install } from './index.ts';

function fixture(enabled = true, report = async (_kind: string) => {}) {
  const app = Object.assign(new EventEmitter(), {
    exit: (code: number) => exits.push(code),
    quit: () => {
      let prevented = false;
      app.emit('before-quit', { preventDefault: () => { prevented = true; } });
      if (!prevented) exits.push(0);
    },
  });
  const host = Object.assign(new EventEmitter(), { env: { APPARENT_DESKTOP_REPORTING: enabled ? '1' : '0' } });
  const exits: number[] = [];
  const exitWithReports = install(app as unknown as App, report, host as unknown as Pick<NodeJS.Process, 'on' | 'env'>);
  return { app, host, exits, exitWithReports };
}

test('standalone hosts install no production exception handlers', () => {
  const { app, host } = fixture(false);
  assert.deepEqual(app.eventNames(), []);
  assert.deepEqual(host.eventNames(), []);
});

for (const [event, kind] of [['uncaughtException', 'main_exception'], ['unhandledRejection', 'main_rejection']]) {
  test(`${event} reports only a fixed code and exits once`, async () => {
    const sent: string[] = [];
    const { host, exits } = fixture(true, async kind => { sent.push(kind); });
    const secret = { get message() { throw Error('must not inspect'); }, toString() { throw Error('must not format'); } };
    host.emit(event, secret);
    host.emit(event, secret);
    await tick();
    assert.deepEqual(sent, [kind]);
    assert.deepEqual(exits, [1]);
  });
}

test('a hung reporting bridge cannot keep a failed process alive', async () => {
  let release!: () => void;
  const { host, exits } = fixture(true, () => new Promise<void>(resolve => { release = resolve; }));
  host.emit('uncaughtException', Error('private'));
  await delay(650);
  assert.deepEqual(exits, [1]);
  release();
  await tick();
  assert.deepEqual(exits, [1]);
});

test('reporting failure still terminates the main process', async () => {
  const { host, exits } = fixture(true, async () => { throw Error('bridge unavailable'); });
  host.emit('uncaughtException', Error('private'));
  await tick();
  assert.deepEqual(exits, [1]);
});

test('abnormal child and renderer exits report without private details', async () => {
  const sent: string[] = [];
  const { app, exits } = fixture(true, async kind => { sent.push(kind); });
  const contents = new EventEmitter();
  app.emit('web-contents-created', {}, contents);
  for (const reason of ['clean-exit', 'killed']) {
    contents.emit('render-process-gone', {}, { reason });
    app.emit('child-process-gone', {}, { reason });
  }
  for (const reason of ['crashed', 'oom', 'launch-failed']) {
    contents.emit('render-process-gone', {}, { reason, name: 'private title', exitCode: 123 });
    app.emit('child-process-gone', {}, { reason, serviceName: 'private' });
  }
  await tick();
  assert.deepEqual(sent, ['renderer_gone', 'child_gone', 'renderer_gone', 'child_gone', 'renderer_gone', 'child_gone']);
  assert.deepEqual(exits, []);
});

test('Notes-style immediate quit waits for pending crash admission', async () => {
  let release!: () => void;
  const { app, exits } = fixture(true, () => new Promise<void>(resolve => { release = resolve; }));
  const contents = new EventEmitter();
  app.emit('web-contents-created', {}, contents);
  contents.on('render-process-gone', () => app.quit());
  contents.emit('render-process-gone', {}, { reason: 'crashed' });
  await tick();
  assert.deepEqual(exits, []);
  release();
  await tick();
  assert.deepEqual(exits, [0]);
});

test('normal quit is immediate, and a pending report cannot block quit indefinitely', async () => {
  const idle = fixture();
  idle.app.quit();
  assert.deepEqual(idle.exits, [0]);
  const { app, exits } = fixture(true, () => new Promise(() => {}));
  const contents = new EventEmitter();
  app.emit('web-contents-created', {}, contents);
  contents.emit('render-process-gone', {}, { reason: 'crashed' });
  app.quit();
  await delay(650);
  assert.deepEqual(exits, [0]);
});

test('Settings-style explicit exit waits for admission and preserves its failure code', async () => {
  let release!: () => void;
  const { app, exits, exitWithReports } = fixture(true, () => new Promise<void>(resolve => { release = resolve; }));
  const contents = new EventEmitter();
  app.emit('web-contents-created', {}, contents);
  contents.on('render-process-gone', () => exitWithReports(1));
  contents.emit('render-process-gone', {}, { reason: 'crashed' });
  await tick();
  assert.deepEqual(exits, []);
  release();
  await tick();
  assert.deepEqual(exits, [1]);
});
