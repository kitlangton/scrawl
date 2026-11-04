// Host contracts; no installed app, permissions or native GUI acceptance.
const { test } = require('node:test');
const path = require('node:path');
const { stripTypeScriptTypes } = require('node:module');
const helper = path.join(__dirname, 'owned-processes.ts');

test('cleanup stops owned supervisor and descendant', async () => {
  const assert = require('node:assert/strict');
  const { spawn } = require('node:child_process');
  const { once } = require('node:events');
  const { supervisor, stopBackend } = require(helper);
    const child = spawn('/bin/bash', ['-c', supervisor, 'smoke-cleanup-test',
      '/bin/bash', '-c', 'echo $$; exec sleep 60'], { detached: true, stdio: ['pipe', 'pipe', 'inherit'] });
    try {
      const [line] = await once(child.stdout, 'data');
      const descendant = Number(line.toString().trim());
      assert.ok(descendant > 0 && descendant !== child.pid);
      process.kill(descendant, 0);
      await stopBackend(child);
      assert.throws(() => process.kill(-child.pid, 0), { code: 'ESRCH' });
      assert.throws(() => process.kill(descendant, 0), { code: 'ESRCH' });
    } finally { if (child.exitCode === null && child.signalCode === null) await stopBackend(child); }
});

test('cleanup accepts darwin eperm only after process group readback', async () => {
  const assert = require('node:assert/strict');
  const fs = require('node:fs');
  const path = require('node:path');
  const source = stripTypeScriptTypes(fs.readFileSync(helper, 'utf8'));
  const realRequire = require;

  async function runCase({ platform = 'darwin', psError, psOutput = '  999 S\n4321 Z\n', killError = 'EPERM', exited = true }) {
    const calls = [];
    const fakeProcess = {
      platform,
      kill(pid, signal) {
        calls.push(['kill', pid, signal]);
        if (signal === 0) throw Object.assign(new Error('permission denied'), { code: killError });
      },
    };
    const execFile = (file, args, options, callback) => {
      calls.push(['execFile', file, args, options]);
      callback(psError || null, { stdout: psOutput, stderr: '' });
    };
    const localRequire = name => name === 'node:child_process'
      ? { ...realRequire(name), execFile }
      : realRequire(name);
    const module = { exports: {} };
    new Function('require', 'module', '__dirname', 'process', source)(
      localRequire, module, path.dirname(helper), fakeProcess);
    const child = { pid: 4321, exitCode: exited ? 0 : null, signalCode: null };
    if (!exited) child.once = () => {};
    let error;
    try { await module.exports.stopBackend(child); } catch (caught) { error = caught; }
    return { calls, error };
  }

    const absent = await runCase({ psOutput: '  999 S\n' });
    assert.equal(absent.error, undefined, 'Darwin EPERM with no matching group is already absent');
    assert.deepEqual(absent.calls[1], ['execFile', '/bin/ps', ['-axo', 'pgid=,stat='],
      { timeout: 1000, maxBuffer: 1024 * 1024 }]);
    assert.equal((await runCase({ psOutput: '4321 Z\n' })).error, undefined,
      'a matching zombie is not a live process-group member');

    for (const rejected of [
      { psOutput: '4321 S\n' },
      { psOutput: 'not-a-process-row\n' },
      { psOutput: '' },
      { killError: 'EACCES' },
      { psError: Object.assign(new Error('ps failed'), { code: 'EIO' }) },
      { platform: 'linux', psOutput: '  999 S\n4321 Z\n' },
    ]) {
      const result = await runCase(rejected);
      assert.ok(result.error, 'live, malformed, failed-query, and non-Darwin cases must reject');
    }

    const liveAnchor = await runCase({ exited: false, psOutput: '4321 S\n' });
    assert.ok(liveAnchor.error);
    assert.equal(liveAnchor.calls.some(call => call[0] === 'execFile'), false,
      'a live supervisor anchor must reject without querying process listings');
});

test('timed out electron close terminates owned child and fails', async () => {
  const assert = require('node:assert/strict');
  const { spawn } = require('node:child_process');
  const { once } = require('node:events');
  const { stopElectron, within } = require(helper);
    const child = spawn('/bin/sleep', ['60']);
    await once(child, 'spawn');
    try {
      await assert.rejects(within(new Promise(() => {}), 10), /timed out/);
    } finally {
      await assert.rejects(stopElectron({ close: () => new Promise(() => {}), process: () => child }, 10), /timed out/);
      assert.equal(child.signalCode, 'SIGKILL');
      assert.throws(() => process.kill(child.pid, 0), { code: 'ESRCH' });
    }
});

test('Desktop compatibility exports resolve to the canonical module', () => {
  const assert = require('node:assert/strict');
  const shared = require(helper);
  const desktop = require('../../capture/desktop/scripts/mac/packaged-smoke.cjs');
  for (const name of ['within', 'stopElectron', 'supervisor', 'stopBackend']) {
    assert.equal(desktop[name], shared[name], name);
  }
});
