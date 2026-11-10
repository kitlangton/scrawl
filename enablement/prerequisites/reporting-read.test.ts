import { test, type TestContext } from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { mkdtemp, mkdir, chmod, rm } from 'node:fs/promises';
import { createServer } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { assertReportingRead, type ReportingReadObservation } from './reporting-read.ts';

const busy = { status: 'error', code: 'app_busy' };
const off = { status: 'connected', enabled: false };
const flush = async () => { for (let n = 0; n < 12; n++) await Promise.resolve(); };

function fixture(t: TestContext, responses: unknown[] = [busy, off], timeoutMs = 5000, deadline = 95000) {
  t.mock.timers.enable({ apis: ['Date', 'setTimeout'], now: 0 });
  const calls = { reads: 0, writes: 0, clicks: 0 };
  const bridge = {
    readErrorReporting: async () => responses[Math.min(calls.reads++, responses.length - 1)],
    setErrorReporting: async () => { calls.writes++; throw Error('Mutation must not be replayed'); },
    click: async () => { calls.clicks++; throw Error('Action must not be replayed'); },
  };
  const observations: ReportingReadObservation[] = [];
  const options = { read: () => bridge.readErrorReporting(), enabled: false, label: 'Restarted reporting read',
    timeoutMs, deadline, observations };
  return { calls, bridge, observations, options };
}

test('the CommonJS native driver loads the same TypeScript assertion', () => {
  const require = createRequire(import.meta.url);
  assert.equal(require('./reporting-read.ts').assertReportingRead, assertReportingRead);
});

test('transient explicit app_busy is re-observed, retaining the failure before the exact expected value', async t => {
  const f = fixture(t);
  // Record rejection immediately so the red one-shot implementation cannot
  // become an unhandled rejection while the deterministic clock advances.
  const result = assertReportingRead(f.options).then(() => null, error => error);
  await flush();
  assert.equal(f.calls.reads, 1);
  t.mock.timers.tick(150);
  await flush();
  assert.equal(await result, null);
  assert.deepEqual(f.observations.map(item => item.response), [busy, off]);
  assert.deepEqual(f.calls, { reads: 2, writes: 0, clicks: 0 });
});

test('persistent busy exhausts the original read deadline including every backoff', async t => {
  const f = fixture(t, [busy]);
  const result = assertReportingRead(f.options).then(() => null, error => error);
  for (let elapsed = 0; elapsed < 5000; elapsed += 50) { await flush(); t.mock.timers.tick(50); }
  await flush();
  const error = await result;
  assert.match(error.message, /Restarted reporting read timed out after 5000ms/);
  assert.deepEqual(error.cause, busy);
  assert.equal(Date.now(), 5000);
  assert.equal(f.calls.reads, 34);
  assert.ok(f.observations.every(item => item.response === busy));
  assert.equal(f.observations[0].elapsedMs, 0);
  t.mock.timers.tick(5000); await flush();
  assert.deepEqual(f.calls, { reads: 34, writes: 0, clicks: 0 });
});

test('the remaining overall budget caps the logical read, including its backoff', async t => {
  const f = fixture(t, [busy, off], 5000, 100);
  const result = assertReportingRead(f.options).then(() => null, error => error);
  await flush(); t.mock.timers.tick(100); await flush();
  assert.match((await result).message, /timed out after 100ms/);
  assert.deepEqual(f.observations.map(item => item.response), [busy]);
  assert.deepEqual(f.calls, { reads: 1, writes: 0, clicks: 0 });
});

for (const response of [
  { status: 'error', code: 'config_read_failed' }, { status: 'error', code: 'admission_expired' },
  { status: 'error', code: 'timeout' }, { status: 'error', code: 'bridge_failed' },
  { status: 'error', code: 'invalid_response' }, { status: 'disconnected' }, { status: 'incompatible' },
  { status: 'connected', enabled: true }, { status: 'connected', enabled: false, unexpected: true },
  { status: 'connected', code: 'app_busy' }, null,
]) test(`non-busy response fails once without polling for a better value: ${JSON.stringify(response)}`, async t => {
  const f = fixture(t, [response, off]);
  await assert.rejects(assertReportingRead(f.options), error => {
    assert.ok(error instanceof assert.AssertionError);
    assert.equal(error.code, 'ERR_ASSERTION');
    assert.deepEqual(error.actual, response);
    assert.deepEqual(error.expected, off);
    return true;
  });
  t.mock.timers.tick(5000); await flush();
  assert.deepEqual(f.observations.map(item => item.response), [response]);
  assert.deepEqual(f.calls, { reads: 1, writes: 0, clicks: 0 });
});

test('a thrown transport failure is preserved and never retried', async t => {
  const f = fixture(t);
  const failure = new Error('synthetic IPC disconnected');
  f.options.read = async () => { f.calls.reads++; throw failure; };
  await assert.rejects(assertReportingRead(f.options), error => error === failure);
  assert.equal(f.observations[0].error, failure.message);
  assert.deepEqual(f.calls, { reads: 1, writes: 0, clicks: 0 });
});

test('a pending read times out; its later success cannot retry or alter the failure receipt', async t => {
  const f = fixture(t);
  let resolve!: (value: unknown) => void;
  f.options.read = () => { f.calls.reads++; return new Promise(done => { resolve = done; }); };
  const result = assertReportingRead(f.options).then(() => null, error => error);
  await flush(); t.mock.timers.tick(5000); await flush();
  assert.match((await result).message, /timed out after 5000ms/);
  const receipt = structuredClone(f.observations);
  resolve(off); await flush(); t.mock.timers.tick(5000); await flush();
  assert.deepEqual(f.observations, receipt);
  assert.deepEqual(f.calls, { reads: 1, writes: 0, clicks: 0 });
});

test('late connected responses fail even before the timeout callback runs', async t => {
  const f = fixture(t, [off]);
  f.options.read = async () => { f.calls.reads++; t.mock.timers.setTime(5000); return off; };
  await assert.rejects(assertReportingRead(f.options), /timed out after 5000ms/);
  assert.equal(f.observations[0].late, true);
  assert.deepEqual(f.observations[0].response, off);
  assert.deepEqual(f.calls, { reads: 1, writes: 0, clicks: 0 });
});

test('retry request time consumes the same deadline instead of receiving a fresh timeout', async t => {
  const f = fixture(t);
  let resolve!: (value: unknown) => void;
  f.options.read = () => {
    f.calls.reads++;
    return new Promise(done => { resolve = done; });
  };
  const result = assertReportingRead(f.options).then(() => null, error => error);
  t.mock.timers.tick(4800); resolve(busy); await flush();
  t.mock.timers.tick(150); await flush();
  assert.equal(f.calls.reads, 2);
  t.mock.timers.tick(50); await flush();
  assert.match((await result).message, /timed out after 5000ms/);
  assert.equal(f.observations[0].durationMs, 4800);
  assert.equal(f.observations[1].durationMs, 50);
  resolve(off); await flush();
  assert.deepEqual(f.calls, { reads: 2, writes: 0, clicks: 0 });
});

test('an expired overall deadline admits no read or action', async t => {
  const f = fixture(t, [off], 5000, 0);
  await assert.rejects(assertReportingRead(f.options), /timed out/);
  assert.deepEqual(f.calls, { reads: 0, writes: 0, clicks: 0 });
});

test('the shipped socket transport re-observes busy using only reporting read requests', async t => {
  const home = await mkdtemp(join(tmpdir(), 'reporting-read-'));
  t.after(() => rm(home, { recursive: true, force: true }));
  const directory = join(home, '.config/apparent/agent-control');
  await mkdir(directory, { recursive: true, mode: 0o700 });
  const socketPath = join(directory, 'capture.sock');
  const requests: Record<string, unknown>[] = [];
  const server = createServer(socket => {
    socket.once('data', data => {
      requests.push(JSON.parse(data.toString()));
      socket.end(JSON.stringify(requests.length === 1 ? { error: 'app_busy' } : { errorReporting: { enabled: false } }) + '\n');
    });
  });
  await new Promise<void>(resolve => server.listen(socketPath, resolve));
  t.after(() => new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve())));
  await chmod(socketPath, 0o600);
  const require = createRequire(import.meta.url);
  const { errorReporting } = require('../../capture/desktop/ui/frontend/electron/runtime.cjs');
  const observations: ReportingReadObservation[] = [];
  await assertReportingRead({ read: () => errorReporting(home), enabled: false, label: 'Restarted reporting read',
    timeoutMs: 5000, deadline: Date.now() + 95000, observations });
  assert.deepEqual(observations.map(item => item.response), [busy, off]);
  assert.equal(requests.length, 2);
  for (const request of requests) {
    assert.deepEqual(Object.keys(request).sort(), ['deadline', 'errorReporting']);
    assert.equal(request.errorReporting, 'read', 'No set, click, or other action is replayed');
    assert.ok(Number.isSafeInteger(request.deadline));
  }
});
