import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { execPath } from 'node:process';
import { test } from 'node:test';
import { QUALIFICATION_IPC_SCHEMA, assertQualificationShutdownProtocol, checkGeneratedNodeScript, reconcileAndCancel, runBounded, runCleanupSteps, runQualification, trackChild, waitForExit } from './native-qualification.ts';

function terminateOwnedFixtureGroup(pid: number, authorization: 'explicit-test-fixture-cleanup' | undefined): void {
  if (authorization !== 'explicit-test-fixture-cleanup') throw new Error('fixture process group cleanup requires explicit test authorization');
  try { process.kill(-pid, 'SIGKILL'); }
  catch (error) { if ((error as NodeJS.ErrnoException).code !== 'ESRCH') throw error; }
}

test('generated module syntax is checked before any supervisor or descendant code can run', async () => {
  const { mkdtemp, readFile, rm } = await import('node:fs/promises');
  const { tmpdir } = await import('node:os');
  const { join } = await import('node:path');
  const root = await mkdtemp(join(tmpdir(), 'record-script-preflight-'));
  const supervisorMarker = join(root, 'supervisor-ran');
  const descendantMarker = join(root, 'descendant-ran');
  try {
    const descendant = `require('node:fs').writeFileSync(${JSON.stringify(descendantMarker)}, 'ran')`;
    const source = `import {writeFileSync} from 'node:fs'; import {spawn} from 'node:child_process'; writeFileSync(${JSON.stringify(supervisorMarker)}, 'ran'); spawn(process.execPath, ['-e', ${JSON.stringify(descendant)}]); const = ;`;
    await assert.rejects(checkGeneratedNodeScript(source), /SyntaxError/);
    await assert.rejects(readFile(supervisorMarker), { code: 'ENOENT' });
    await assert.rejects(readFile(descendantMarker), { code: 'ENOENT' });
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('cleanup continues after ambiguous capture reconciliation and preserves the cleanup error', async () => {
  const attempted: string[] = [];
  const errors = await runCleanupSteps([
    { label: 'capture reconciliation', run: () => { attempted.push('reconcile'); throw new Error('inspection unknown'); } },
    { label: 'media client close', run: () => { attempted.push('media-close'); } },
    { label: 'Xvfb wait', run: () => { attempted.push('xvfb-wait'); } },
  ]);
  assert.deepEqual(attempted, ['reconcile','media-close','xvfb-wait']);
  assert.equal(errors.length, 1);
  assert.match(errors[0].message, /capture reconciliation: inspection unknown/);
  assert.equal((errors[0].cause as Error).message, 'inspection unknown');
});

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

test('native qualification launch is refused before spawn without a local shutdown and clearance protocol', async () => {
  const { mkdtemp, readFile, rm } = await import('node:fs/promises');
  const { tmpdir } = await import('node:os');
  const { join } = await import('node:path');
  const root = await mkdtemp(join(tmpdir(), 'record-shutdown-protocol-'));
  const marker = join(root, 'spawned');
  try {
    assert.throws(() => assertQualificationShutdownProtocol(false), /SDK 0\.4\.5 normal-close local-resource contract/);
    await assert.rejects(readFile(marker), { code: 'ENOENT' });
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('qualification timeout retains an unresolved owner until explicit fixture cleanup', async t => {
  if (process.platform !== 'linux') { t.skip('Linux Xvfb process-group qualification'); return; }
  const { mkdtemp, readFile, rm } = await import('node:fs/promises');
  const { tmpdir } = await import('node:os');
  const { join } = await import('node:path');
  const root = await mkdtemp(join(tmpdir(), 'record-deadline-'));
  const pidFile = join(root, 'child.pid');
  let retainedGroupPid: number | undefined;
  try {
    const report = { coordinatorShutdown: { scope: 'record-coordinator', admission: 'closed', localDrain: 'complete', runtimeClearance: 'unknown', work: [{ owner: 'media', state: 'unknown', reason: 'retained-job-outcome-unknown' }] },
      coordinatorDisposed: true, recoveryPeerReleases: [{ connection: 'released', runtimeClearance: 'not-established' }], captureOwner: 'retained', status: 3, authorityRoot: '/private/fixture-root' };
    const fixture = `
      const fs=require('node:fs'); fs.writeFileSync(${JSON.stringify(pidFile)},String(process.pid));
      process.on('message',message=>{ if(message?.schema===${JSON.stringify(QUALIFICATION_IPC_SCHEMA)}&&message.kind==='shutdown-request') process.send({schema:${JSON.stringify(QUALIFICATION_IPC_SCHEMA)},kind:'shutdown-report',requestId:message.requestId,report:${JSON.stringify(report)}}); });
      setInterval(()=>{},1000);
    `;
    await assert.rejects(runQualification(execPath, ['-e', fixture], { cwd: root, env: process.env, timeout: 150, shutdownReportTimeout: 300 }), error => {
      retainedGroupPid = error.ownedProcessGroupPid;
      assert.match(error.message, /deadline exceeded/);
      assert.equal(error.processGroupRetained, true);
      assert.equal(error.exitCode, null, 'Node reports no exit code while the child remains alive');
      assert.equal(error.shutdownRequestId.length > 0, true);
      assert.deepEqual(error.shutdownReport, report, 'raw work, unknown state, status and authority-root evidence survive IPC');
      assert.ok(Number.isInteger(retainedGroupPid));
      return true;
    });
    const pid = Number(await readFile(pidFile, 'utf8'));
    const status = await readFile('/proc/' + pid + '/stat', 'utf8');
    assert.ok(status && !status.slice(status.lastIndexOf(')') + 2).startsWith('Z'), 'ambiguous owner remains alive and retained after supervisor timeout');
    assert.throws(() => terminateOwnedFixtureGroup(retainedGroupPid!, undefined), /explicit test authorization/);
    const stillRetained = await readFile('/proc/' + pid + '/stat', 'utf8');
    assert.ok(!stillRetained.slice(stillRetained.lastIndexOf(')') + 2).startsWith('Z'), 'unresolved clearance cannot terminate the owner');
  } finally {
    if (retainedGroupPid) terminateOwnedFixtureGroup(retainedGroupPid, 'explicit-test-fixture-cleanup');
    await rm(root, { recursive: true, force: true });
  }
});

test('output limit sends a cooperative IPC request and preserves its shutdown report', async t => {
  if (process.platform !== 'linux') { t.skip('Linux isolated process-group IPC fixture'); return; }
  const { mkdtemp, rm } = await import('node:fs/promises');
  const { tmpdir } = await import('node:os');
  const { join } = await import('node:path');
  const root = await mkdtemp(join(tmpdir(), 'record-output-limit-'));
  let ownedPid: number | undefined;
  const report = { coordinatorShutdown: { admission: 'closed', localDrain: 'complete', runtimeClearance: 'unresolved', work: [{ owner: 'publication', state: 'active', reason: 'same-binding-publish-still-running' }] },
    coordinatorDisposed: true, recoveryPeerRelease: { connection: 'released', runtimeClearance: 'not-established' }, captureOwner: 'retained', authorityRoot: '/private/output-fixture' };
  const fixture = `
    process.on('message',message=>{ if(message?.schema===${JSON.stringify(QUALIFICATION_IPC_SCHEMA)}&&message.kind==='shutdown-request') process.send({schema:${JSON.stringify(QUALIFICATION_IPC_SCHEMA)},kind:'shutdown-report',requestId:message.requestId,report:${JSON.stringify(report)}}); });
    const timer=setInterval(()=>{},1000); process.stdout.write('x'.repeat(256));
  `;
  try {
    await assert.rejects(runQualification(execPath, ['-e', fixture], { cwd: root, env: process.env, timeout: 2000, shutdownReportTimeout: 300, outputLimitBytes: 64 }), error => {
      assert.match(error.message, /output limit/);
      assert.equal(error.processGroupRetained, true);
      assert.deepEqual(error.shutdownReport, report);
      ownedPid = error.ownedProcessGroupPid;
      return true;
    });
  } finally {
    if (ownedPid) terminateOwnedFixtureGroup(ownedPid, 'explicit-test-fixture-cleanup');
    await rm(root, { recursive: true, force: true });
  }
});

test('positive explicit fixture cleanup terminates only the owned test group', async t => {
  if (process.platform !== 'linux') { t.skip('Linux process-group clearance check'); return; }
  const { spawn } = await import('node:child_process');
  const child = trackChild(spawn(execPath, ['-e', 'setInterval(()=>{},1000)'], { detached: true, stdio: 'ignore' }));
  try {
    await new Promise(resolve => setTimeout(resolve, 30));
    assert.throws(() => terminateOwnedFixtureGroup(child.pid!, undefined), /explicit test authorization/);
    terminateOwnedFixtureGroup(child.pid!, 'explicit-test-fixture-cleanup');
    await new Promise<void>((resolve, reject) => { child.once('close', () => resolve()); child.once('error', reject); });
    assert.ok(child.exitCode !== null || child.signalCode !== null);
  } finally {
    if (child.exitCode === null && child.signalCode === null) {
      terminateOwnedFixtureGroup(child.pid!, 'explicit-test-fixture-cleanup');
    }
  }
});
