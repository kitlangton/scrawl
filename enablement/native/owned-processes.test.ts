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

test('packaged smoke imports canonical lifecycle and exports only scenario contracts', () => {
  const assert = require('node:assert/strict');
  const fs = require('node:fs');
  const { createRequire } = require('node:module');
  const entry = path.resolve(__dirname, '../../capture/desktop/scripts/mac/packaged-smoke.cjs');
  const entryRequire = createRequire(entry);
  const imported = [];
  const localRequire = name => {
    const value = entryRequire(name);
    if (name.endsWith('/owned-processes.ts')) imported.push(value);
    return value;
  };
  const loaded = { exports: {} };
  new Function('require', 'module', '__dirname', fs.readFileSync(entry, 'utf8'))(
    localRequire, loaded, path.dirname(entry));
  assert.deepEqual(imported, [require(helper)]);
  assert.deepEqual(Object.keys(loaded.exports).sort(), ['admission', 'readLocation']);
});

test('Storage entrypoint loads the canonical lifecycle without running its scenario', () => {
  const assert = require('node:assert/strict');
  const fs = require('node:fs');
  const { createRequire } = require('node:module');
  const entry = path.resolve(__dirname, '../../capture/desktop/ui/frontend/tests/mac-storage-native.cjs');
  const entryRequire = createRequire(entry);
  const imported = [];
  const registered = [];
  const localRequire = name => {
    if (name === 'node:test') return { test: (...args) => registered.push(args) };
    if (name === '@playwright/test') return {};
    const value = entryRequire(name);
    if (name.endsWith('/owned-processes.ts')) imported.push(value);
    return value;
  };
  new Function('require', '__dirname', fs.readFileSync(entry, 'utf8'))(localRequire, path.dirname(entry));
  assert.deepEqual(imported, [require(helper)]);
  assert.equal(registered.length, 1);
});

test('receipt cleanup refuses unanchored live or unknown groups', async () => {
  const assert = require('node:assert/strict');
  const fs = require('node:fs');
  const source = stripTypeScriptTypes(fs.readFileSync(helper, 'utf8'));
  const { stopOwned } = require(helper);
  assert.deepEqual(await stopOwned(undefined, 'missing'), { started: false });
  assert.deepEqual(await stopOwned({ pid: undefined }, 'spawn'), { started: false, spawnFailed: true });
  for (const status of ['present', 'ESRCH', 'EPERM']) {
    const signals = [];
    const fakeProcess = { platform: 'darwin', kill: (pid, signal) => {
      signals.push([pid, signal]);
      if (status !== 'present') throw Object.assign(new Error(status), { code: status });
    } };
    const loaded = { exports: {} };
    new Function('require', 'module', 'process', source)(require, loaded, fakeProcess);
    const operation = loaded.exports.stopOwned({ pid: 4321, exitCode: 0, signalCode: null }, 'retained', true);
    if (status === 'ESRCH') {
      assert.deepEqual(await operation, { pid: 4321, group: true, exitCode: 0, signalCode: null,
        exited: true, groupAbsent: true });
    } else await assert.rejects(operation, status === 'EPERM' ? /EPERM/ : /refusing unanchored cleanup/);
    assert.deepEqual(signals, [[-4321, 0]], 'Only probe the old group; never signal it');
  }
});

test('PID-file supervisor and receipt cleanup stop only their owned group', async () => {
  const assert = require('node:assert/strict');
  const fs = require('node:fs/promises');
  const os = require('node:os');
  const { spawn } = require('node:child_process');
  const { once } = require('node:events');
  const { pidFileSupervisor, stopOwned } = require(helper);
  const home = await fs.mkdtemp(path.join(os.tmpdir(), 'owned-group-'));
  const pidFile = path.join(home, 'rust.pid');
  await fs.writeFile(pidFile, '', { flag: 'wx', mode: 0o600 });
  const child = spawn('/bin/bash', ['-c', pidFileSupervisor, 'owned-fixture', pidFile,
    '/bin/bash', '-c', 'echo $$; exec sleep 60'], {
    env: { ...process.env, TMPDIR: home }, detached: true, stdio: ['pipe', 'pipe', 'inherit'],
  });
  try {
    const [line] = await once(child.stdout, 'data');
    const descendant = Number(line.toString().trim());
    assert.equal(Number((await fs.readFile(pidFile, 'utf8')).trim()), descendant);
    const receipt = await stopOwned(child, 'owned fixture', true);
    assert.equal(receipt.pid, child.pid);
    assert.equal(receipt.exited, true);
    assert.equal(receipt.groupAbsent, true);
    assert.equal(receipt.SIGTERM, true);
    assert.throws(() => process.kill(descendant, 0), { code: 'ESRCH' });
  } finally {
    if (child.exitCode === null && child.signalCode === null) await stopOwned(child, 'fixture finalizer', true);
    await fs.rm(home, { recursive: true });
  }
});

test('reference entrypoint imports shared direct-child cleanup without executing capture', () => {
  const assert = require('node:assert/strict');
  const fs = require('node:fs');
  const { createRequire } = require('node:module');
  const entry = path.resolve(__dirname, '../../capture/desktop/ui/frontend/tests/reference-native.cjs');
  const entryRequire = createRequire(entry);
  const imported = [];
  let registrations = 0;
  const localRequire = name => {
    if (name === 'node:test') return { test: () => { registrations++; } };
    if (name === '@playwright/test') return {};
    const value = entryRequire(name);
    if (name.endsWith('/owned-processes.ts')) imported.push(value);
    return value;
  };
  new Function('require', '__dirname', fs.readFileSync(entry, 'utf8'))(localRequire, path.dirname(entry));
  assert.deepEqual(imported, [require(helper)]);
  assert.equal(registrations, 1);
});

test('bounded waits preserve caller diagnostics, results and original failures', async () => {
  const assert = require('node:assert/strict');
  const { within, withinLabelled } = require(helper);
  assert.equal(await within(Promise.resolve('ready'), 50), 'ready');
  const original = new Error('original operation failed');
  await assert.rejects(withinLabelled(Promise.reject(original), 50, 'read'), error => error === original);
  await assert.rejects(within(new Promise(() => {}), 5), { message: 'Packaged smoke operation timed out' });
  await assert.rejects(withinLabelled(new Promise(() => {}), 5, 'Storage read'), { message: 'Storage read timed out after 5ms' });
});

test('direct-child cleanup handles absent, exited and TERM-resistant owned children', async () => {
  const assert = require('node:assert/strict');
  const { spawn } = require('node:child_process');
  const { once } = require('node:events');
  const { stopChild } = require(helper);
  await stopChild(undefined);
  await stopChild({ exitCode: 0, signalCode: null, kill: () => assert.fail('Exited child signalled') });
  const child = spawn(process.execPath, ['-e', "process.on('SIGTERM', () => {}); console.log('ready'); setInterval(() => {}, 1000)"],
    { stdio: ['ignore', 'pipe', 'inherit'] });
  try {
    await once(child.stdout, 'data');
    await stopChild(child);
    assert.equal(child.signalCode, 'SIGKILL');
    assert.throws(() => process.kill(child.pid, 0), { code: 'ESRCH' });
  } finally {
    if (child.exitCode === null && child.signalCode === null) { child.kill('SIGKILL'); await once(child, 'exit'); }
  }
});
