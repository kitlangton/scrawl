import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { execPath } from 'node:process';
import { test } from 'node:test';
import { reconcileAndCancel, runBounded, trackChild, waitForExit } from './native-qualification.ts';

test('hung child receives TERM then KILL and exit is awaited', async () => {
  const child = trackChild(spawn(execPath, ['-e', 'process.on("SIGTERM",()=>{});setInterval(()=>{},1000)'], { stdio: 'ignore' }));
  const started = Date.now();
  await waitForExit(child, 30);
  assert.ok(Date.now() - started < 2000);
  assert.ok(child.exitCode !== null || child.signalCode !== null);
});

test('lost inspect or cancel responses fail closed', async () => {
  const mappings = { listRequestIds: async () => ['request'], getByRequestId: async () => ({ capture2Intent: {}, captureId: 'capture', ownerToken: 'owner' }) };
  await assert.rejects(reconcileAndCancel(mappings, { inspect: async () => { throw new Error('lost inspect'); } }, Date.now() + 500));
  await assert.rejects(reconcileAndCancel(mappings, { inspect: async () => ({ state: 'recording' }), cancel: async () => { throw new Error('lost cancel'); } }, Date.now() + 500));
});

test('authoritative materials-ready is terminal and does not trigger release or cancellation', async () => {
  let cancelled = false;
  const mappings = { listRequestIds: async () => ['request'], getByRequestId: async () => ({ capture2Intent: {}, captureId: 'capture', ownerToken: 'owner' }) };
  await reconcileAndCancel(mappings, { inspect: async () => ({ state: 'materials-ready' }), cancel: async () => { cancelled = true; } }, Date.now() + 500);
  assert.equal(cancelled, false);
});

test('failed terminal capture remains a failing cleanup result', async () => {
  const mappings = { listRequestIds: async () => ['request'], getByRequestId: async () => ({ capture2Intent: {}, captureId: 'capture', ownerToken: 'owner' }) };
  await assert.rejects(reconcileAndCancel(mappings, { inspect: async () => ({ state: 'failed' }) }, Date.now() + 500), /failed terminal/);
});

test('hung media probe fails within its deadline and terminates its child', async () => {
  const started = Date.now();
  await assert.rejects(runBounded('sleep', ['30'], 50), /deadline exceeded/);
  assert.ok(Date.now() - started < 2000);
});

test('qualification hard deadline terminates a TERM-resistant descendant group', async t => {
  if (process.platform !== 'linux') { t.skip('Linux Xvfb process-group qualification'); return; }
  const { mkdtemp, readFile, rm } = await import('node:fs/promises');
  const { tmpdir } = await import('node:os');
  const { join } = await import('node:path');
  const { runQualification } = await import('./native-qualification.ts');
  const root = await mkdtemp(join(tmpdir(), 'record-deadline-'));
  const pidFile = join(root, 'child.pid');
  try {
    const descendant = 'require("node:fs").writeFileSync(process.argv[1], String(process.pid));process.on("SIGTERM",()=>{});setInterval(()=>{},1000)';
    const supervisor = 'require("node:child_process").spawn(process.execPath,["-e",process.argv[1],process.argv[2]],{stdio:"ignore"});process.on("SIGTERM",()=>{});setInterval(()=>{},1000)';
    await assert.rejects(runQualification(execPath, ['-e', supervisor, descendant, pidFile], { cwd: root, env: process.env, timeout: 1000 }), /deadline exceeded/);
    const pid = Number(await readFile(pidFile, 'utf8'));
    const status = await readFile('/proc/' + pid + '/stat', 'utf8').catch(error => {
      if (error.code === 'ENOENT') return undefined;
      throw error;
    });
    assert.ok(status === undefined || status.slice(status.lastIndexOf(')') + 2).startsWith('Z'), 'owned descendant must be exited, not running');
  } finally { await rm(root, { recursive: true, force: true }); }
});
