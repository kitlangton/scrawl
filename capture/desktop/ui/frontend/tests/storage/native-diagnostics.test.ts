import { test } from 'node:test';
import assert from 'node:assert/strict';
import { retainPickerFailure } from './native-diagnostics.ts';

const identity = { pid: 4321, title: 'Apparent — UI foundation', bounds: { x: -10, y: 20, width: 900, height: 700 } };

test('picker failure evidence uses cached owned identity without querying a hung Electron main', async () => {
  const calls: unknown[] = [];
  const evidence = await retainPickerFailure(identity, 4321, '/private/evidence', async (...args) => {
    calls.push(args);
  }, cap => Math.min(cap, 1700), {});
  assert.deepEqual(evidence, { lastKnownWindowScreenshot: 'failed-owned-window.png', ownedProcessSample: 'electron-main-sample.txt' });
  assert.deepEqual(calls, [
    ['/usr/sbin/screencapture', ['-x', '-R', '-10,20,900,700', '/private/evidence/failed-owned-window.png'],
      { env: {}, timeout: 1700, killSignal: 'SIGKILL', maxBuffer: 8192 }],
    ['/usr/bin/sample', ['4321', '1', '1', '-file', '/private/evidence/electron-main-sample.txt'],
      { env: {}, timeout: 1700, killSignal: 'SIGKILL', maxBuffer: 8192 }],
  ]);
});

test('unavailable screenshot does not suppress the independent owned process sample', async () => {
  const evidence = await retainPickerFailure(identity, 4321, '/private/evidence', async file => {
    if (file.endsWith('screencapture')) throw new Error('capture unavailable');
  }, cap => cap, {});
  assert.equal(evidence.lastKnownWindowScreenshotUnavailable, 'capture unavailable');
  assert.equal(evidence.ownedProcessSample, 'electron-main-sample.txt');
});

test('unknown PID, title, rectangle or output cannot trigger native observation', async () => {
  const run = async () => { assert.fail('must not observe an unverified window/process'); };
  for (const value of [{ ...identity, pid: 9999 }, { ...identity, title: 'Foreign window' },
    { ...identity, bounds: { ...identity.bounds, width: 0 } }]) {
    await assert.rejects(retainPickerFailure(value, 4321, '/private/evidence', run, cap => cap, {}));
  }
  await assert.rejects(retainPickerFailure(identity, 4321, 'relative', run, cap => cap, {}));
});
