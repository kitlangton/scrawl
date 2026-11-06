import { strict as assert } from 'node:assert';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import type { Capture2Host, Capture2Receipt } from '@apparent/record/host/capture2';
import { controlCapture2, parseCapture2RecordArgs, recordWithCapture2, type Capture2Clock, type Capture2HostFactory } from '../src/recording/capture2.ts';

const source = { id: 'display-actual', generation: 'generation-1', kind: 'display' as const, displayId: 0, bounds: { x: 0, y: 0, width: 800, height: 600, scale: 1 } };
const args = (out: string, ...extra: string[]) => ['--protocol', 'capture2', '--duration', '1', '--out', out, '--source', source.id, ...extra];
function fixtureHost(out: string, outcome: 'saved' | 'materials-ready' | 'unavailable' | 'unknown-source' | 'start-fails' = 'saved', sleepHook?: () => void, closeOutcome = 'eof-exited') {
  const calls: Array<{ method: string; input?: unknown }> = [];
  const reqId = '4be6ebdb-7f40-430a-92de-7ac3c43e9e2e';
  let phase = 'recording';
  const receipt = (): Capture2Receipt => ({ requestId: reqId, phase: phase as Capture2Receipt['phase'], ...(phase === 'processing' && outcome === 'saved' ? { publication: {
    captureId: 'capture-internal', materials: { captureId: 'capture-internal', materialsId: 'materials', revision: 'a'.repeat(64) },
    publicationId: 'publication', planSha256: 'b'.repeat(64), bundleId: 'opaque-record-id', bundlePath: out,
    artifactVersion: 1, artifactProvenance: 'native', manifestSha256: 'c'.repeat(64), inventorySha256: 'd'.repeat(64),
  } } : {}) });
  const host = {
    readiness: async () => ({ ready: outcome !== 'unavailable', reason: 'native capture/2 is unavailable' }),
    discoverSources: async () => ({ state: 'available' as const, sources: outcome === 'unknown-source' ? [] : [source] }),
    start: async (input: unknown) => { calls.push({ method: 'start', input }); if (outcome === 'start-fails') throw new Error('start outcome is unknown'); return receipt(); },
    inspect: async (id: string) => { calls.push({ method: 'inspect', input: id }); return receipt(); },
    stop: async (id: string) => { calls.push({ method: 'stop', input: id }); phase = outcome === 'saved' ? 'processing' : 'materials-ready'; return receipt(); },
    cancel: async (id: string) => { calls.push({ method: 'cancel', input: id }); phase = 'cancelled'; return receipt(); },
    reconcile: async () => [], subscribe: () => () => undefined, dispose: () => calls.push({ method: 'dispose' }),
  } as unknown as Capture2Host;
  const factory: Capture2HostFactory = async paths => {
    calls.push({ method: 'compose', input: paths });
    return { host, close: async () => { calls.push({ method: 'close' }); return closeOutcome; } };
  };
  let now = 0;
  const clock: Capture2Clock = { now: () => now, sleep: async ms => { now += ms; sleepHook?.(); } };
  return { calls, factory, clock, reqId };
}

test('capture/2 requires explicit protocol, exact destination and source selector', () => {
  const parsed = parseCapture2RecordArgs(args('/tmp/fresh.apparent'));
  assert.deepEqual(parsed, { durationSeconds: 1, out: '/tmp/fresh.apparent', sourceId: source.id });
  for (const invalid of [[], args('relative.apparent'), args('/tmp/fresh.apparent', '--source', 'other'), args('/tmp/fresh.apparent', '--duration', '1')])
    assert.throws(() => parseCapture2RecordArgs(invalid));
});

test('capture/2 readiness and fresh source discovery fail before start', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-capture2-'));
  try {
    for (const mode of ['unavailable', 'unknown-source'] as const) {
      const fixture = fixtureHost(join(root, `${mode}.apparent`), mode);
      await assert.rejects(recordWithCapture2(args(join(root, `${mode}.apparent`)), {
        executable: '/opt/apparent-engine', runtime: join(root, 'runtime'), controlRoot: join(root, 'control', mode), factory: fixture.factory, clock: fixture.clock,
      }), mode === 'unavailable' ? /provider unavailable/ : /not present in current/);
      assert.equal(fixture.calls.some(call => call.method === 'start'), false);
      assert.equal(fixture.calls.at(-1)?.method, 'close');
    }
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('capture/2 binds exact --out, uses explicit local-only options and reports materials-ready as pending', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-capture2-'));
  try {
    const out = join(root, 'exact.apparent'), fixture = fixtureHost(out, 'materials-ready');
    const code = await recordWithCapture2(args(out), {
      executable: '/opt/apparent-engine', runtime: join(root, 'runtime'), controlRoot: join(root, 'control'), factory: fixture.factory, clock: fixture.clock,
    });
    assert.equal(code, 3);
    const start = fixture.calls.find(call => call.method === 'start')!.input as { requestId: string; source: typeof source; options: { localOnly: true; audio: string }; destinationPath: string };
    assert.equal(start.source.id, source.id); assert.deepEqual(start.options, { audio: 'none', localOnly: true }); assert.equal(start.destinationPath, out);
    assert.notEqual(start.requestId, 'opaque-record-id');
    assert(fixture.calls.some(call => call.method === 'stop'));
    const metadata = JSON.parse(await readFile(join(root, 'control', 'capture2-requests-v1', `${start.requestId}.json`), 'utf8'));
    assert.deepEqual(metadata, { schema: 'apparent.agent-capture2-request/1', requestId: start.requestId, destinationPath: out, createdAtMs: metadata.createdAtMs });
    assert.equal(fixture.calls.at(-1)?.method, 'close');
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('capture/2 returns success only for a native publication at the exact requested path', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-capture2-'));
  try {
    const out = join(root, 'exact.apparent'), fixture = fixtureHost(out, 'saved');
    const code = await recordWithCapture2(args(out), {
      executable: '/opt/apparent-engine', runtime: join(root, 'runtime'), controlRoot: join(root, 'control'), factory: fixture.factory, clock: fixture.clock,
    });
    assert.equal(code, 0);
    assert(fixture.calls.some(call => call.method === 'stop'));
    assert.equal(fixture.calls.at(-1)?.method, 'close');
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('capture/2 retains the original public identity after an ambiguous start and never retries start', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-capture2-'));
  try {
    const out = join(root, 'exact.apparent'), fixture = fixtureHost(out, 'start-fails');
    await assert.rejects(recordWithCapture2(args(out), {
      executable: '/opt/apparent-engine', runtime: join(root, 'runtime'), controlRoot: join(root, 'control'), factory: fixture.factory, clock: fixture.clock,
    }), /start outcome is unknown/);
    const starts = fixture.calls.filter(call => call.method === 'start');
    assert.equal(starts.length, 1);
    const requestId = (starts[0].input as { requestId: string }).requestId;
    const identityPath = join(root, 'control', 'capture2-requests-v1', `${requestId}.json`);
    const metadata = JSON.parse(await readFile(identityPath, 'utf8'));
    assert.equal(metadata.requestId, requestId); assert.equal(metadata.schema, 'apparent.agent-capture2-request/1');
    assert.equal(fixture.calls.at(-1)?.method, 'close');
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('capture/2 signal stops an observed recording and does not discard it with cancel', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-capture2-'));
  try {
    const out = join(root, 'exact.apparent'); let interrupted = false;
    const fixture = fixtureHost(out, 'materials-ready', () => { interrupted = true; });
    const code = await recordWithCapture2(args(out), {
      executable: '/opt/apparent-engine', runtime: join(root, 'runtime'), controlRoot: join(root, 'control'), factory: fixture.factory, clock: fixture.clock, interrupted: () => interrupted,
    });
    assert.equal(code, 3); assert(fixture.calls.some(call => call.method === 'stop'));
    assert.equal(fixture.calls.some(call => call.method === 'cancel'), false);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('capture/2 never reports path success when transport close is unverified', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-capture2-'));
  try {
    const out = join(root, 'exact.apparent'), fixture = fixtureHost(out, 'saved', undefined, 'signal-requested-after-timeout');
    const code = await recordWithCapture2(args(out), {
      executable: '/opt/apparent-engine', runtime: join(root, 'runtime'), controlRoot: join(root, 'control'), factory: fixture.factory, clock: fixture.clock,
    });
    assert.equal(code, 3);
    assert.equal(fixture.calls.at(-1)?.method, 'close');
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('capture/2 controls require retained Agent identity and reuse the same public request ID', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-capture2-'));
  try {
    const out = join(root, 'exact.apparent'), fixture = fixtureHost(out, 'materials-ready');
    assert.equal(await recordWithCapture2(args(out), {
      executable: '/opt/apparent-engine', runtime: join(root, 'runtime'), controlRoot: join(root, 'control'), factory: fixture.factory, clock: fixture.clock,
    }), 3);
    const requestId = (fixture.calls.find(call => call.method === 'start')!.input as { requestId: string }).requestId;
    await assert.rejects(controlCapture2(['inspect', '4be6ebdb-7f40-430a-92de-7ac3c43e9e2e'], {
      executable: '/opt/apparent-engine', runtime: join(root, 'runtime'), controlRoot: join(root, 'control'), factory: fixture.factory,
    }), /No retained Agent capture\/2 request has this ID/);
    const code = await controlCapture2(['inspect', requestId], {
      executable: '/opt/apparent-engine', runtime: join(root, 'runtime'), controlRoot: join(root, 'control'), factory: fixture.factory,
    });
    assert.equal(code, 3);
    assert(fixture.calls.some(call => call.method === 'inspect' && call.input === requestId));
  } finally { await rm(root, { recursive: true, force: true }); }
});
