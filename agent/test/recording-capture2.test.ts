import { strict as assert } from 'node:assert';
import { mkdir, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { CaptureClient } from '@apparent/engine-sdk/capture';
import { AbsentDestinationAllocator, SqliteCaptureMappingStore } from '@apparent/record/host';
import type { Capture2Host, Capture2Receipt } from '@apparent/record/host/capture2';
import { createCapture2Host } from '@apparent/record/host/capture2';
import type { CaptureRequest, CaptureResponse, CaptureTransport } from '@apparent/engine-sdk/capture';
import { controlCapture2, discoverCapture2Sources, parseCapture2ControlArgs, parseCapture2RecordArgs, parseCapture2SourcesArgs, recordWithCapture2, retainPublicIdentity, type Capture2Clock, type Capture2HostFactory } from '../src/recording/capture2.ts';

const source = { id: 'display-actual', generation: 'a'.repeat(32), kind: 'display' as const, displayId: 0, bounds: { x: 0, y: 0, width: 800, height: 600, scale: 1 } };
const args = (out: string, ...extra: string[]) => ['--protocol', 'capture2', '--duration', '1', '--out', out, '--source', source.id, ...extra];
function fixtureHost(out: string, outcome: 'publication-processing' | 'publication-materials-ready' | 'materials-ready' | 'unavailable' | 'unknown-source' | 'start-fails' = 'publication-processing', sleepHook?: () => void, closeOutcome = 'eof-exited') {
  const calls: Array<{ method: string; input?: unknown }> = [];
  const reqId = '4be6ebdb-7f40-430a-92de-7ac3c43e9e2e';
  let phase = 'recording';
  // Deliberately synthetic: the public host may attach publication metadata to
  // a processing receipt, but this fixture does not model an Agent-visible Saved state.
  const receipt = (): Capture2Receipt => ({ requestId: reqId, phase: phase as Capture2Receipt['phase'], ...(['processing', 'materials-ready'].includes(phase) && outcome.startsWith('publication-') ? { publication: {
    captureId: 'capture-internal', materials: { captureId: 'capture-internal', materialsId: 'materials', revision: 'a'.repeat(64) },
    publicationId: 'publication', planSha256: 'b'.repeat(64), bundleId: 'opaque-record-id', bundlePath: out,
    artifactVersion: 1, artifactProvenance: 'native', manifestSha256: 'c'.repeat(64), inventorySha256: 'd'.repeat(64),
  } } : {}) });
  const host = {
    readiness: async () => ({ ready: outcome !== 'unavailable', reason: 'native capture/2 is unavailable' }),
    discoverSources: async () => ({ state: 'available' as const, sources: outcome === 'unknown-source' ? [] : [source] }),
    start: async (input: unknown) => { calls.push({ method: 'start', input }); if (outcome === 'start-fails') throw new Error('start outcome is unknown'); return receipt(); },
    inspect: async (id: string) => { calls.push({ method: 'inspect', input: id }); return receipt(); },
    stop: async (id: string) => { calls.push({ method: 'stop', input: id }); phase = outcome === 'publication-processing' ? 'processing' : 'materials-ready'; return receipt(); },
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
  assert.deepEqual(parseCapture2ControlArgs(['--protocol', 'capture2', 'inspect', 'id']), { method: 'inspect', requestId: 'id' });
  for (const invalid of [['inspect', 'id'], ['--protocol', 'capture-v1', 'inspect', 'id'], ['--protocol', 'capture2', '--protocol', 'capture2'], ['--protocol', 'capture2', 'inspect', 'id', '--protocol']])
    assert.throws(() => parseCapture2ControlArgs(invalid));
});

test('capture/2 source args reject invalid options before transport creation', () => {
  assert.deepEqual(parseCapture2SourcesArgs([]), { json: false });
  assert.deepEqual(parseCapture2SourcesArgs(['--json']), { json: true });
  for (const invalid of [['--bad'], ['--json', '--json'], ['source'], ['--out', '/tmp/a']]) assert.throws(() => parseCapture2SourcesArgs(invalid));
});

test('capture/2 sources uses the validated SDK handshake/discovery and closes without Record authority', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-capture2-sources-'));
  try {
    const runtime = join(root, 'runtime'), executable = join(root, 'engine');
    await mkdir(runtime, { mode: 0o700 }); await writeFile(executable, 'fixture', { mode: 0o700 });
    const requests: string[] = []; let closed = false;
    const transport = {
      async request(request: { method: string; rpcId: string }) {
        requests.push(request.method);
        const result = request.method === 'handshake'
          ? { channel: 'capture', protocolVersion: 2, implementation: 'native', readiness: 'ready', capabilities: { sourceDiscovery: true, displayCapture: true, areaCapture: false, audioCapture: false, contextFacts: false, interactionFacts: false, candidates: false, transcript: false, strokes: false, publication: false, retainedMaterials: true } }
          : { state: 'available', sources: [source] };
        return { channel: 'capture', protocolVersion: 2, rpcId: request.rpcId, result };
      },
      async close() { closed = true; return 'eof-exited' as const; },
    };
    const output: string[] = [], originalWrite = process.stdout.write;
    process.stdout.write = ((chunk: string | Uint8Array) => { output.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString()); return true; }) as typeof process.stdout.write;
    try { assert.equal(await discoverCapture2Sources(['--json'], { executable, runtime, transportFactory: () => transport }), 0); }
    finally { process.stdout.write = originalWrite; }
    assert.deepEqual(requests, ['handshake', 'discoverSources']); assert.equal(closed, true);
    assert.deepEqual(JSON.parse(output.join('')), { state: 'available', sources: [source], transportClose: 'eof-exited' });
  } finally { await rm(root, { recursive: true, force: true }); }
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

test('real packed SDK and Record host accept the generated invocation ID while retaining Agent UUID request identity', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-capture2-sdk-'));
  try {
    const sdkSource = { ...source, generation: 'a'.repeat(32) };
    const requests: CaptureRequest[] = [];
    const handshake = {
      channel: 'capture', protocolVersion: 2, implementation: 'native', readiness: 'ready',
      capabilities: { sourceDiscovery: true, displayCapture: true, areaCapture: false, audioCapture: false, contextFacts: false, interactionFacts: false, candidates: false, transcript: false, strokes: false, publication: false, retainedMaterials: true },
    };
    const transport: CaptureTransport = {
      request: async (request): Promise<CaptureResponse> => {
        requests.push(request);
        if (request.method === 'start') throw new Error('test transport stopped after public SDK validation');
        const result = request.method === 'handshake' ? handshake
          : request.method === 'discoverSources' ? { state: 'available', sources: [sdkSource] }
          : (() => { throw new Error(`Unexpected Engine request: ${request.method}`); })();
        return { channel: 'capture', protocolVersion: 2, rpcId: request.rpcId, result };
      },
      close: async () => 'already-exited',
    };
    const client = new CaptureClient(transport);
    const storeRoot = join(root, 'record-control');
    await mkdir(storeRoot, { mode: 0o700 });
    const host = createCapture2Host({
      client,
      mappings: new SqliteCaptureMappingStore(storeRoot),
      destinations: new AbsentDestinationAllocator(storeRoot, root),
    });
    const factory: Capture2HostFactory = async () => ({ host, close: () => client.close() });
    const out = join(root, 'exact.apparent');
    const output: string[] = [], originalWrite = process.stdout.write, stderr: string[] = [], originalError = process.stderr.write;
    process.stdout.write = ((chunk: string | Uint8Array) => { output.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString()); return true; }) as typeof process.stdout.write;
    process.stderr.write = ((chunk: string | Uint8Array) => { stderr.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString()); return true; }) as typeof process.stderr.write;
    let rejected: unknown;
    try {
      await recordWithCapture2(args(out), {
        executable: '/opt/apparent-engine', runtime: join(root, 'runtime'), controlRoot: join(root, 'control'), factory,
      });
    } catch (error) { rejected = error; }
    finally { process.stdout.write = originalWrite; process.stderr.write = originalError; }
    assert.match(String(rejected), /Capture start outcome is unknown/);
    assert.deepEqual(requests.map(request => request.method), ['handshake', 'discoverSources', 'start']);
    const start = requests.at(-1)!;
    assert.equal(start.method, 'start');
    assert.match(start.invocation.invocationId, /^[0-9a-f]{32}$/);
    const requestId = /Capture\/2 request ID: ([0-9a-f-]{36})/.exec(stderr.join(''))?.[1];
    assert.match(requestId ?? '', /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    const identity = JSON.parse(await readFile(join(root, 'control', 'capture2-requests-v1', `${requestId}.json`), 'utf8'));
    assert.equal(identity.requestId, requestId);
    assert.notEqual(start.invocation.invocationId, requestId);
    assert.equal(identity.destinationPath, out);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('capture/2 binds exact --out, uses explicit local-only options and keeps publication-shaped materials-ready pending', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-capture2-'));
  try {
    const out = join(root, 'exact.apparent'), fixture = fixtureHost(out, 'publication-materials-ready');
    const output: string[] = [], originalWrite = process.stdout.write;
    let code: number;
    process.stdout.write = ((chunk: string | Uint8Array) => { output.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString()); return true; }) as typeof process.stdout.write;
    try { code = await recordWithCapture2(args(out), {
      executable: '/opt/apparent-engine', runtime: join(root, 'runtime'), controlRoot: join(root, 'control'), factory: fixture.factory, clock: fixture.clock,
    }); } finally { process.stdout.write = originalWrite; }
    assert.equal(code, 3);
    assert.equal(JSON.parse(output.join('')).outcome, 'pending-saved-capability-unavailable');
    assert.equal(output.join('').includes(out), false);
    const start = fixture.calls.find(call => call.method === 'start')!.input as { requestId: string; source: typeof source; options: { localOnly: true; audio: string }; destinationPath: string };
    assert.equal(start.source.id, source.id); assert.deepEqual(start.options, { audio: 'none', localOnly: true }); assert.equal(start.destinationPath, out);
    assert.notEqual(start.requestId, 'opaque-record-id');
    assert(fixture.calls.some(call => call.method === 'stop'));
    const metadata = JSON.parse(await readFile(join(root, 'control', 'capture2-requests-v1', `${start.requestId}.json`), 'utf8'));
    assert.deepEqual(metadata, { schema: 'apparent.agent-capture2-request/1', requestId: start.requestId, destinationPath: out, createdAtMs: metadata.createdAtMs });
    assert.equal(fixture.calls.at(-1)?.method, 'close');
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('capture/2 processing with publication metadata remains pending, never Saved', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-capture2-'));
  try {
    const out = join(root, 'exact.apparent'), fixture = fixtureHost(out, 'publication-processing');
    const output: string[] = [], originalWrite = process.stdout.write;
    process.stdout.write = ((chunk: string | Uint8Array) => { output.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString()); return true; }) as typeof process.stdout.write;
    let code: number;
    try { code = await recordWithCapture2(args(out), {
      executable: '/opt/apparent-engine', runtime: join(root, 'runtime'), controlRoot: join(root, 'control'), factory: fixture.factory, clock: fixture.clock,
    }); } finally { process.stdout.write = originalWrite; }
    assert.equal(code, 3);
    const result = JSON.parse(output.join(''));
    assert.equal(result.outcome, 'pending-saved-capability-unavailable');
    assert.equal(result.state, 'processing');
    assert.equal(output.join('').includes(out), false);
    assert.equal(fixture.calls.some(call => call.method === 'close'), true);
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
    const out = join(root, 'exact.apparent'), fixture = fixtureHost(out, 'publication-processing', undefined, 'signal-requested-after-timeout');
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
    const out = join(root, 'exact.apparent'), fixture = fixtureHost(out, 'publication-processing');
    assert.equal(await recordWithCapture2(args(out), {
      executable: '/opt/apparent-engine', runtime: join(root, 'runtime'), controlRoot: join(root, 'control'), factory: fixture.factory, clock: fixture.clock,
    }), 3);
    const requestId = (fixture.calls.find(call => call.method === 'start')!.input as { requestId: string }).requestId;
    await assert.rejects(controlCapture2(['--protocol', 'capture2', 'inspect', '4be6ebdb-7f40-430a-92de-7ac3c43e9e2e'], {
      executable: '/opt/apparent-engine', runtime: join(root, 'runtime'), controlRoot: join(root, 'control'), factory: fixture.factory,
    }), /No retained Agent capture\/2 request has this ID/);
    const output: string[] = [], originalWrite = process.stdout.write;
    process.stdout.write = ((chunk: string | Uint8Array) => { output.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString()); return true; }) as typeof process.stdout.write;
    let code: number;
    try { code = await controlCapture2(['--protocol', 'capture2', 'inspect', requestId], {
      executable: '/opt/apparent-engine', runtime: join(root, 'runtime'), controlRoot: join(root, 'control'), factory: fixture.factory,
    }); } finally { process.stdout.write = originalWrite; }
    assert.equal(code, 3);
    assert.equal(output.join('').includes(out), false);
    assert.equal(JSON.parse(output.join('')).outcome, 'pending-saved-capability-unavailable');
    assert(fixture.calls.some(call => call.method === 'inspect' && call.input === requestId));
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('capture/2 control reports publication-shaped materials-ready as pending without a path', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-capture2-'));
  try {
    const out = join(root, 'exact.apparent'), fixture = fixtureHost(out, 'publication-materials-ready');
    assert.equal(await recordWithCapture2(args(out), {
      executable: '/opt/apparent-engine', runtime: join(root, 'runtime'), controlRoot: join(root, 'control'), factory: fixture.factory, clock: fixture.clock,
    }), 3);
    const requestId = (fixture.calls.find(call => call.method === 'start')!.input as { requestId: string }).requestId;
    const output: string[] = [], originalWrite = process.stdout.write;
    process.stdout.write = ((chunk: string | Uint8Array) => { output.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString()); return true; }) as typeof process.stdout.write;
    let code: number;
    try { code = await controlCapture2(['--protocol', 'capture2', 'inspect', requestId], {
      executable: '/opt/apparent-engine', runtime: join(root, 'runtime'), controlRoot: join(root, 'control'), factory: fixture.factory,
    }); } finally { process.stdout.write = originalWrite; }
    assert.equal(code, 3);
    assert.equal(JSON.parse(output.join('')).state, 'materials-ready');
    assert.equal(JSON.parse(output.join('')).outcome, 'pending-saved-capability-unavailable');
    assert.equal(output.join('').includes(out), false);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('capture/2 identity loading rejects oversized mutable files and symlink replacements before composing a host', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-capture2-'));
  try {
    const controlRoot = join(root, 'control'), requestId = '4be6ebdb-7f40-430a-92de-7ac3c43e9e2e';
    await retainPublicIdentity(controlRoot, requestId, join(root, 'exact.apparent'));
    const identityPath = join(controlRoot, 'capture2-requests-v1', `${requestId}.json`);
    await writeFile(identityPath, ' '.repeat(8193), { mode: 0o600 });
    let composed = false;
    const factory: Capture2HostFactory = async () => { composed = true; throw new Error('unexpected host composition'); };
    const options = { executable: '/opt/apparent-engine', runtime: join(root, 'runtime'), controlRoot, factory };
    await assert.rejects(controlCapture2(['--protocol', 'capture2', 'inspect', requestId], options), /Invalid Agent capture\/2 identity metadata/);
    await rm(identityPath);
    await symlink(join(root, 'missing-target'), identityPath);
    await assert.rejects(controlCapture2(['--protocol', 'capture2', 'inspect', requestId], options), /No retained Agent capture\/2 request has this ID/);
    assert.equal(composed, false);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('signal received while durable identity persistence is pending prevents start dispatch', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-capture2-'));
  try {
    const out = join(root, 'exact.apparent'), fixture = fixtureHost(out, 'publication-processing'); let interrupted = false;
    await assert.rejects(recordWithCapture2(args(out), {
      executable: '/opt/apparent-engine', runtime: join(root, 'runtime'), controlRoot: join(root, 'control'), factory: fixture.factory, clock: fixture.clock,
      interrupted: () => interrupted,
      persistIdentity: async (controlRoot, requestId, destinationPath) => {
        const pendingWrite = retainPublicIdentity(controlRoot, requestId, destinationPath);
        interrupted = true;
        await pendingWrite;
      },
    }), /no start was dispatched/);
    assert.equal(fixture.calls.some(call => call.method === 'start'), false);
    assert.equal(fixture.calls.at(-1)?.method, 'close');
  } finally { await rm(root, { recursive: true, force: true }); }
});
