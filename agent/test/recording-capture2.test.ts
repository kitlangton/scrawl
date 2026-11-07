import { strict as assert } from 'node:assert';
import { access, mkdir, mkdtemp, readFile, rm, stat, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { CaptureClient } from '@apparent/engine-sdk/capture';
import { AbsentDestinationAllocator, SqliteCaptureMappingStore } from '@apparent/record/host';
import type { Capture2Host, Capture2Receipt } from '@apparent/record/host/capture2';
import { createCapture2Host } from '@apparent/record/host/capture2';
import type { CaptureRequest, CaptureResponse, CaptureTransport } from '@apparent/engine-sdk/capture';
import { controlCapture2, discoverCapture2Sources, parseCapture2ControlArgs, parseCapture2RecordArgs, parseCapture2SourcesArgs, recordWithCapture2, recoverRecordCaptures, retainPublicIdentity, type Capture2Clock, type Capture2HostFactory, type Capture2PublicOutput, type RecoveryCoordinatorFactory } from '../dist/recording/capture2.js';

const source = { id: 'display-actual', generation: 'a'.repeat(32), kind: 'display' as const, displayId: 0, bounds: { x: 0, y: 0, width: 800, height: 600, scale: 1 } };
const args = (out: string, ...extra: string[]) => ['--protocol', 'capture2', '--duration', '1', '--out', out, '--source', source.id, ...extra];
const defaultArgs = (out?: string) => ['--protocol', 'capture2', ...(out ? ['--out', out] : []), '--source', source.id];
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
    const allocator = new AbsentDestinationAllocator(join(paths.controlRoot, 'allocator-control'), paths.destinationRoot);
    return { host, allocateDestination: async (id, exact) => exact ? (await allocator.reserveExact(id, exact)).bundlePath : (await allocator.allocate(id)).bundlePath,
      close: async () => { calls.push({ method: 'close' }); return closeOutcome; } };
  };
  let now = 0;
  const clock: Capture2Clock = { now: () => now, sleep: async ms => { now += ms; sleepHook?.(); } };
  return { calls, factory, clock, reqId };
}

test('capture/2 requires explicit protocol and source selector; exact destination stays optional', () => {
  const parsed = parseCapture2RecordArgs(args('/tmp/fresh.apparent'));
  assert.deepEqual(parsed, { durationSeconds: 1, out: '/tmp/fresh.apparent', sourceId: source.id });
  assert.deepEqual(parseCapture2RecordArgs([...defaultArgs('/tmp/fresh.apparent'), '--duration', '3600']), { durationSeconds: 3600, out: '/tmp/fresh.apparent', sourceId: source.id });
  assert.deepEqual(parseCapture2RecordArgs(defaultArgs('/tmp/fresh.apparent')), { durationSeconds: 5, out: '/tmp/fresh.apparent', sourceId: source.id });
  assert.deepEqual(parseCapture2RecordArgs(defaultArgs()), { durationSeconds: 5, sourceId: source.id });
  for (const suffix of [['--out'], ['--out', ''], ['--out', '--source', source.id]])
    assert.throws(() => parseCapture2RecordArgs([...defaultArgs(), ...suffix]), /--out/);
  for (const invalid of [[], args('relative.apparent'), args('/tmp/fresh.apparent', '--source', 'other'), args('/tmp/fresh.apparent', '--duration', '1'),
    [...args('/tmp/fresh.apparent'), '--duration', '1'], [...defaultArgs('/tmp/fresh.apparent'), '--duration', '0'],
    [...defaultArgs('/tmp/fresh.apparent'), '--duration', '3601'], [...defaultArgs('/tmp/fresh.apparent'), '--duration', 'nope'],
    [...defaultArgs('/tmp/fresh.apparent'), '--duration'], [...defaultArgs('/tmp/fresh.apparent'), '--source', 'duplicate']])
    assert.throws(() => parseCapture2RecordArgs(invalid));
  assert.deepEqual(parseCapture2ControlArgs(['--protocol', 'capture2', 'inspect', 'id']), { method: 'inspect', requestId: 'id' });
  for (const invalid of [['inspect', 'id'], ['--protocol', 'capture-v1', 'inspect', 'id'], ['--protocol', 'capture2', '--protocol', 'capture2'], ['--protocol', 'capture2', 'inspect', 'id', '--protocol']])
    assert.throws(() => parseCapture2ControlArgs(invalid));
});

test('capture/2 duplicate and invalid durations fail before control allocation or host effects', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-capture2-invalid-'));
  try {
    const out = join(root, 'exact.apparent');
    const invalid = [
      [...args(out), '--duration', '1'], [...defaultArgs(out), '--duration', '0'],
      [...defaultArgs(out), '--duration', '3601'], [...defaultArgs(out), '--duration', 'invalid'],
      [...defaultArgs(out), '--duration'],
    ];
    for (const [index, argv] of invalid.entries()) {
      const controlRoot = join(root, `control-${index}`);
      let factoryCalled = false;
      await assert.rejects(recordWithCapture2(argv, {
        executable: '/opt/apparent-engine', runtime: join(root, 'runtime'), controlRoot,
        factory: async () => { factoryCalled = true; throw new Error('unexpected host composition'); },
      }));
      assert.equal(factoryCalled, false);
      await assert.rejects(access(controlRoot), { code: 'ENOENT' });
    }
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('recover rejects operands before creating control roots or composing Record', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-recover-invalid-'));
  let factoryCalled = false;
  try {
    await assert.rejects(recoverRecordCaptures(['--captures', root], {
      captureExecutable: '/opt/capture', mediaExecutable: '/opt/media', runtime: join(root, 'runtime'),
      controlRoot: join(root, 'control'), destinationRoot: join(root, 'bundles'),
      factory: async () => { factoryCalled = true; throw new Error('unexpected composition'); },
    }), /recover takes no operands or options/);
    assert.equal(factoryCalled, false);
    await assert.rejects(access(join(root, 'control')), { code: 'ENOENT' });
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('recover uses coordinator reconciliation and reports pending/unknown without paths', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-recover-pending-'));
  const calls: string[] = [];
  const factory: RecoveryCoordinatorFactory = async () => ({
    coordinator: {
      reconcile: async () => { calls.push('reconcile'); },
      shutdown: async () => { calls.push('shutdown'); return { scope: 'record-coordinator', admission: 'closed', localDrain: 'complete', runtimeClearance: 'unknown', nextShortcutAdmission: { state: 'unsupported', reason: 'not used' }, work: [{ owner: 'capture', state: 'unknown', reason: 'synthetic-unknown' }] }; },
      dispose: async () => { calls.push('dispose'); },
      runtime: { captures: () => [], unresolvedOwnership: () => ({ requestIds: [], storageUnavailable: false, reconciliation: 'complete' }) },
      captureFacts: { readFacts: async () => ({ captures: [{ captureId: 'public-id', phase: 'processing', outcomes: [] }] }) },
      processingIssues: () => [{ requestId: 'public-id', state: 'outcome-unknown', code: 'media-outcome-unknown', at: '2026-01-01T00:00:00.000Z' }],
      resolveHandoff: async () => { throw new Error('pending captures have no handoff'); },
    } as unknown as import('@apparent/record/host/capture2-coordinator').RecordCaptureCoordinator,
    release: async () => { calls.push('release'); return [recoveryRelease('capture', 'unconfirmed'), recoveryRelease('media')]; },
  });
  const output: string[] = [], originalWrite = process.stdout.write;
  process.stdout.write = ((chunk: string | Uint8Array) => { output.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString()); return true; }) as typeof process.stdout.write;
  try {
    const code = await recoverRecordCaptures([], { captureExecutable: '/opt/capture', mediaExecutable: '/opt/media', runtime: join(root, 'runtime'), controlRoot: join(root, 'control'), destinationRoot: join(root, 'bundles'), factory });
    assert.equal(code, 3);
    assert.deepEqual(calls, ['reconcile', 'shutdown', 'dispose', 'release']);
    const result = JSON.parse(output.join(''));
    assert.equal(result.captures[0].state, 'processing');
    assert.equal('bundlePath' in result.captures[0], false);
    assert.equal(result.processingIssues[0].code, 'media-outcome-unknown');
    assert.equal(result.peerRelease[0].connection, 'released');
    assert.equal(result.peerInputClosure, 'unconfirmed');
    assert.equal(result.shutdown.runtimeClearance, 'unknown');
  } finally { process.stdout.write = originalWrite; await rm(root, { recursive: true, force: true }); }
});

test('recover drains and disposes coordinator before releasing both recovery peers after clear shutdown', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-recover-clear-'));
  const calls: string[] = [];
  const factory: RecoveryCoordinatorFactory = async () => ({
    coordinator: {
      reconcile: async () => { calls.push('reconcile'); },
      shutdown: async () => { calls.push('shutdown'); return { scope: 'record-coordinator', admission: 'closed', localDrain: 'complete', runtimeClearance: 'clear', nextShortcutAdmission: { state: 'unsupported', reason: 'not used' }, work: [] }; },
      dispose: async () => { calls.push('dispose'); },
      runtime: { captures: () => [], unresolvedOwnership: () => ({ requestIds: [], storageUnavailable: false, reconciliation: 'complete' }) },
      captureFacts: { readFacts: async () => ({ captures: [] }) }, processingIssues: () => [], resolveHandoff: async () => null,
    } as unknown as import('@apparent/record/host/capture2-coordinator').RecordCaptureCoordinator,
    release: async () => { calls.push('release'); return [recoveryRelease('capture'), recoveryRelease('media')]; },
  });
  const originalWrite = process.stdout.write; process.stdout.write = (() => true) as typeof process.stdout.write;
  try {
    const code = await recoverRecordCaptures([], { captureExecutable: '/opt/capture', mediaExecutable: '/opt/media', runtime: join(root, 'runtime'), controlRoot: join(root, 'control'), destinationRoot: join(root, 'bundles'), factory });
    assert.equal(code, 0);
    assert.deepEqual(calls, ['reconcile', 'shutdown', 'dispose', 'release']);
  } finally { process.stdout.write = originalWrite; await rm(root, { recursive: true, force: true }); }
});

test('recover returns exit 3 when released peer input closure is unconfirmed', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-recover-close-'));
  const factory: RecoveryCoordinatorFactory = async () => ({
    coordinator: {
      reconcile: async () => undefined,
      shutdown: async () => ({ scope: 'record-coordinator', admission: 'closed', localDrain: 'complete', runtimeClearance: 'clear', nextShortcutAdmission: { state: 'unsupported', reason: 'not used' }, work: [] }),
      dispose: async () => undefined,
      runtime: { captures: () => [], unresolvedOwnership: () => ({ requestIds: [], storageUnavailable: false, reconciliation: 'complete' }) },
      captureFacts: { readFacts: async () => ({ captures: [] }) }, processingIssues: () => [], resolveHandoff: async () => null,
    } as unknown as import('@apparent/record/host/capture2-coordinator').RecordCaptureCoordinator,
    release: async () => [recoveryRelease('capture', 'unconfirmed'), recoveryRelease('media')],
  });
  const originalWrite = process.stdout.write, originalError = process.stderr.write;
  process.stdout.write = (() => true) as typeof process.stdout.write; process.stderr.write = (() => true) as typeof process.stderr.write;
  try {
    const code = await recoverRecordCaptures([], { captureExecutable: '/opt/capture', mediaExecutable: '/opt/media', runtime: join(root, 'runtime'), controlRoot: join(root, 'control'), destinationRoot: join(root, 'bundles'), factory });
    assert.equal(code, 3);
  } finally { process.stdout.write = originalWrite; process.stderr.write = originalError; await rm(root, { recursive: true, force: true }); }
});

function recoveryRelease(peerId: string, inputClosure: 'confirmed' | 'unconfirmed' = 'confirmed') {
  return { scope: 'client-connection' as const, peerId, connection: 'released' as const, inputClosure,
    runtimeClearance: 'not-established' as const, reconcile: 'original-capture-materials-job-publication-bindings' as const };
}

test('recover never releases injected peers when Record shutdown or disposal fails', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-recover-drain-failure-'));
  for (const failAt of ['shutdown', 'dispose'] as const) {
    const calls: string[] = [];
    const factory: RecoveryCoordinatorFactory = async () => ({
      coordinator: {
        reconcile: async () => undefined,
        shutdown: async () => { calls.push('shutdown'); if (failAt === 'shutdown') throw new Error('synthetic drain rejection'); return { scope: 'record-coordinator', admission: 'closed', localDrain: 'complete', runtimeClearance: 'unknown', nextShortcutAdmission: { state: 'unsupported', reason: 'synthetic' }, work: [] }; },
        dispose: async () => { calls.push('dispose'); if (failAt === 'dispose') throw new Error('synthetic dispose rejection'); },
        runtime: { captures: () => [], unresolvedOwnership: () => ({ requestIds: [], storageUnavailable: false, reconciliation: 'complete' }) },
        captureFacts: { readFacts: async () => ({ captures: [] }) }, processingIssues: () => [], resolveHandoff: async () => null,
      } as unknown as import('@apparent/record/host/capture2-coordinator').RecordCaptureCoordinator,
      release: async () => { calls.push('release'); return [recoveryRelease('capture'), recoveryRelease('media')]; },
    });
    const originalWrite = process.stdout.write, originalError = process.stderr.write;
    process.stdout.write = (() => true) as typeof process.stdout.write; process.stderr.write = (() => true) as typeof process.stderr.write;
    try {
      const code = await recoverRecordCaptures([], { captureExecutable: '/opt/capture', mediaExecutable: '/opt/media', runtime: join(root, 'runtime'), controlRoot: join(root, `control-${failAt}`), destinationRoot: join(root, 'bundles'), factory });
      assert.equal(code, 3);
      assert.equal(calls.includes('release'), false, `${failAt} failure must keep injected recovery peers owned`);
    } finally { process.stdout.write = originalWrite; process.stderr.write = originalError; }
  }
  await rm(root, { recursive: true, force: true });
});

test('capture/2 omitted destination uses read-only configured capturesDir and lets Record create a private missing root', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-capture2-configured-'));
  const previousHome = process.env.HOME, previousCaptures = process.env.APPARENT_CAPTURES_DIR;
  try {
    const home = join(root, 'home'), captures = join(root, 'configured-captures');
    await mkdir(join(home, '.config', 'apparent'), { recursive: true, mode: 0o700 });
    await writeFile(join(home, '.config', 'apparent', 'config.json'), JSON.stringify({ capturesDir: captures, unrelated: 'preserved' }), { mode: 0o600 });
    process.env.HOME = home; delete process.env.APPARENT_CAPTURES_DIR;
    const fixture = fixtureHost(join(root, 'unused.apparent'));
    const output: string[] = [], originalWrite = process.stdout.write;
    process.stdout.write = ((chunk: string | Uint8Array) => { output.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString()); return true; }) as typeof process.stdout.write;
    let code: number;
    try { code = await recordWithCapture2(defaultArgs(), {
      executable: '/opt/apparent-engine', runtime: join(root, 'runtime'), controlRoot: join(root, 'control'), factory: fixture.factory, clock: fixture.clock,
    }); } finally { process.stdout.write = originalWrite; }
    assert.equal(code, 3);
    const requestId = fixture.calls.find(call => call.method === 'start')!.input && (fixture.calls.find(call => call.method === 'start')!.input as { requestId: string }).requestId;
    const identity = JSON.parse(await readFile(join(root, 'control', 'capture2-requests-v1', `${requestId}.json`), 'utf8'));
    assert.equal(identity.destinationPath.startsWith(`${captures}/`), true);
    assert.equal((await stat(captures)).mode & 0o777, 0o700);
    await assert.rejects(access(identity.destinationPath), { code: 'ENOENT' });
    const outcome = JSON.parse(output.join(''));
    assert.equal(outcome.state, 'processing'); assert.equal(outcome.outcome, 'pending-saved-capability-unavailable');
    assert.equal(JSON.stringify(outcome).includes(captures), false);
  } finally {
    if (previousHome === undefined) delete process.env.HOME; else process.env.HOME = previousHome;
    if (previousCaptures === undefined) delete process.env.APPARENT_CAPTURES_DIR; else process.env.APPARENT_CAPTURES_DIR = previousCaptures;
    await rm(root, { recursive: true, force: true });
  }
});

test('capture/2 omitted duration waits five seconds after observed recording readiness', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-capture2-default-'));
  try {
    const out = join(root, 'exact.apparent');
    let now = 0, phase = 'starting', stopAt = -1;
    const calls: string[] = [];
    const host = {
      readiness: async () => ({ ready: true }),
      discoverSources: async () => ({ state: 'available' as const, sources: [source] }),
      start: async () => { calls.push('start'); return { requestId: 'unused', phase: 'starting' as const }; },
      inspect: async () => {
        calls.push('inspect');
        if (phase === 'starting' && now >= 2_000) phase = 'recording';
        return { requestId: 'unused', phase };
      },
      stop: async () => { stopAt = now; phase = 'materials-ready'; return { requestId: 'unused', phase }; },
      cancel: async () => { phase = 'cancelled'; return { requestId: 'unused', phase }; },
      reconcile: async () => [], subscribe: () => () => undefined, dispose: () => undefined,
    } as unknown as Capture2Host;
    const allocator = new AbsentDestinationAllocator(join(root, 'control', 'allocator'), root);
    const factory: Capture2HostFactory = async paths => ({ host, allocateDestination: async (id, exact) => exact ? (await allocator.reserveExact(id, exact)).bundlePath : (await allocator.allocate(id)).bundlePath, close: async () => 'eof-exited' });
    const clock: Capture2Clock = { now: () => now, sleep: async ms => { now += ms; } };
    assert.equal(await recordWithCapture2(defaultArgs(out), {
      executable: '/opt/apparent-engine', runtime: join(root, 'runtime'), controlRoot: join(root, 'control'), factory, clock,
    }), 3);
    assert.equal(stopAt, 7_000);
    assert.equal(stopAt - 2_000, 5_000);
    assert(calls.includes('start'));
  } finally { await rm(root, { recursive: true, force: true }); }
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
  const priorCaptures = process.env.APPARENT_CAPTURES_DIR;
  try {
    const captures = join(root, 'captures'); await mkdir(captures, { mode: 0o755 });
    process.env.APPARENT_CAPTURES_DIR = captures;
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
    const hostDestination = new AbsentDestinationAllocator(storeRoot, captures);
    const host = createCapture2Host({
      client,
      mappings: new SqliteCaptureMappingStore(storeRoot),
      destinations: hostDestination,
    });
    const factory: Capture2HostFactory = async () => ({ host, allocateDestination: async (id, exact) => exact ? (await hostDestination.reserveExact(id, exact)).bundlePath : (await hostDestination.allocate(id)).bundlePath, close: () => client.close() });
    const output: string[] = [], originalWrite = process.stdout.write, stderr: string[] = [], originalError = process.stderr.write;
    process.stdout.write = ((chunk: string | Uint8Array) => { output.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString()); return true; }) as typeof process.stdout.write;
    process.stderr.write = ((chunk: string | Uint8Array) => { stderr.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString()); return true; }) as typeof process.stderr.write;
    let rejected: unknown;
    try {
      await recordWithCapture2(defaultArgs(), {
        executable: '/opt/apparent-engine', runtime: join(root, 'runtime'), controlRoot: join(root, 'control'), factory,
      });
    } catch (error) { rejected = error; }
    finally { process.stdout.write = originalWrite; process.stderr.write = originalError; }
    assert.match(String(rejected), /Capture start outcome is unknown/);
    assert.deepEqual(requests.map(request => request.method), ['handshake', 'discoverSources', 'start']);
    const start = requests.at(-1)!;
    assert.equal(start.method, 'start');
    assert.equal(start.destinationPath, undefined);
    assert.match(start.invocation.invocationId, /^[0-9a-f]{32}$/);
    const requestId = /Capture\/2 request ID: ([0-9a-f-]{36})/.exec(stderr.join(''))?.[1];
    assert.match(requestId ?? '', /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/);
    const identity = JSON.parse(await readFile(join(root, 'control', 'capture2-requests-v1', `${requestId}.json`), 'utf8'));
    assert.equal(identity.requestId, requestId);
    assert.notEqual(start.invocation.invocationId, requestId);
    assert.equal(identity.destinationPath.startsWith(`${captures}/`), true);
    assert.equal(identity.destinationPath.endsWith('.apparent'), true);
    assert.equal((await stat(captures)).mode & 0o777, 0o755);
    await assert.rejects(access(identity.destinationPath), { code: 'ENOENT' });
    assert.deepEqual(await hostDestination.allocate(requestId!), { bundleId: identity.destinationPath.split('/').at(-1), bundlePath: identity.destinationPath });
  } finally {
    if (priorCaptures === undefined) delete process.env.APPARENT_CAPTURES_DIR; else process.env.APPARENT_CAPTURES_DIR = priorCaptures;
    await rm(root, { recursive: true, force: true });
  }
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

test('capture/2 inspect reports Saved only from an exact public Record fact and handoff binding', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-capture2-public-saved-'));
  const requestId = '4be6ebdb-7f40-430a-92de-7ac3c43e9e2e';
  const expectedPath = join(root, 'expected.apparent');
  try {
    await mkdir(expectedPath);
    const controlRoot = join(root, 'control');
    await retainPublicIdentity(controlRoot, requestId, expectedPath);
    const publicCalls: string[] = [];
    const exactOutput: Capture2PublicOutput = {
      captureFacts: { readFacts: async () => { publicCalls.push('facts'); return { captures: [{ captureId: requestId, artifactId: 'public-artifact', phase: 'saved' }] }; } },
      resolveHandoff: async identity => {
        publicCalls.push(`resolve:${identity.captureId}:${identity.artifactId}`);
        return { captureId: requestId, artifactId: 'public-artifact', handoff: { savedBundle: { status: 'saved', bundlePath: expectedPath } } };
      },
    };
    const run = async (phase: string, publicationOutput?: Capture2PublicOutput, closeOutcome = 'eof-exited') => {
      const hostCalls: string[] = [];
      const host = {
        inspect: async (id: string) => { hostCalls.push(`inspect:${id}`); return { requestId: id, phase, publication: { phase: 'saved', bundlePath: expectedPath } }; },
        dispose: () => hostCalls.push('dispose'),
      } as unknown as Capture2Host;
      const factory: Capture2HostFactory = async () => ({ host, allocateDestination: async () => expectedPath, close: async () => closeOutcome, ...(publicationOutput ? { publicationOutput } : {}) });
      const output: string[] = [], originalWrite = process.stdout.write;
      process.stdout.write = ((chunk: string | Uint8Array) => { output.push(typeof chunk === 'string' ? chunk : Buffer.from(chunk).toString()); return true; }) as typeof process.stdout.write;
      let code: number;
      try { code = await controlCapture2(['--protocol', 'capture2', 'inspect', requestId], { executable: '/opt/apparent-engine', runtime: join(root, 'runtime'), controlRoot, factory }); }
      finally { process.stdout.write = originalWrite; }
      return { code, result: JSON.parse(output.join('')), hostCalls };
    };

    const saved = await run('processing', exactOutput);
    assert.equal(saved.code, 0);
    assert.deepEqual(saved.result, { requestId, state: 'saved', outcome: 'saved', bundlePath: expectedPath });
    assert.deepEqual(publicCalls, ['facts', `resolve:${requestId}:public-artifact`]);
    assert.deepEqual(saved.hostCalls, [`inspect:${requestId}`, 'dispose']);

    const unverifiedClose = await run('processing', exactOutput, 'signal-requested-after-timeout');
    assert.equal(unverifiedClose.code, 3);
    assert.equal(unverifiedClose.result.outcome, 'close-unverified');
    assert.equal('bundlePath' in unverifiedClose.result, false);
    assert.deepEqual(publicCalls, ['facts', `resolve:${requestId}:public-artifact`]);

    const mismatches: Array<{ name: string; output: Capture2PublicOutput }> = [
      { name: 'wrong request', output: { ...exactOutput, captureFacts: { readFacts: async () => ({ captures: [{ captureId: 'aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa', artifactId: 'public-artifact', phase: 'saved' }] }) } } },
      { name: 'wrong artifact', output: { ...exactOutput, resolveHandoff: async () => ({ captureId: requestId, artifactId: 'other-artifact', handoff: { savedBundle: { status: 'saved', bundlePath: expectedPath } } }) } },
      { name: 'wrong path', output: { ...exactOutput, resolveHandoff: async () => ({ captureId: requestId, artifactId: 'public-artifact', handoff: { savedBundle: { status: 'saved', bundlePath: join(root, 'other.apparent') } } }) } },
      { name: 'processing fact', output: { ...exactOutput, captureFacts: { readFacts: async () => ({ captures: [{ captureId: requestId, artifactId: 'public-artifact', phase: 'processing' }] }) } } },
    ];
    for (const mismatch of mismatches) {
      const result = await run('processing', mismatch.output);
      assert.equal(result.code, 3, mismatch.name);
      assert.equal(result.result.state, 'processing', mismatch.name);
      assert.equal(result.result.outcome, 'pending-saved-capability-unavailable', mismatch.name);
      assert.equal('bundlePath' in result.result, false, mismatch.name);
      assert.deepEqual(result.hostCalls, [`inspect:${requestId}`, 'dispose'], mismatch.name);
    }

    const noContext = await run('processing');
    assert.equal(noContext.code, 3);
    assert.equal(noContext.result.outcome, 'pending-saved-capability-unavailable');
    assert.equal('bundlePath' in noContext.result, false);

    let factReads = 0, resolverCalls = 0;
    const unknownOutput: Capture2PublicOutput = {
      captureFacts: { readFacts: async () => { factReads++; throw new Error('public facts unavailable'); } },
      resolveHandoff: async () => { resolverCalls++; return null; },
    };
    const unknown = await run('unknown', unknownOutput);
    assert.equal(unknown.code, 3);
    assert.equal(unknown.result.outcome, 'uncertain');
    assert.equal(factReads, 1);
    assert.equal(resolverCalls, 0);
    assert.deepEqual(unknown.hostCalls, [`inspect:${requestId}`, 'dispose']);
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

test('default capture collection cannot overlap Agent control storage before allocation or start', async () => {
  const root = await mkdtemp(join(tmpdir(), 'agent-capture2-overlap-'));
  const previous = process.env.APPARENT_CAPTURES_DIR;
  try {
    const controlRoot = join(root, 'control');
    for (const captures of [controlRoot, join(controlRoot, 'captures'), root]) {
      process.env.APPARENT_CAPTURES_DIR = captures;
      let calls = 0;
      await assert.rejects(recordWithCapture2(defaultArgs(), {
        controlRoot, executable: '/opt/apparent-engine', runtime: join(root, 'runtime'),
        factory: async () => { calls++; throw new Error('must not compose'); },
      }), /separate from the Agent control root/);
      assert.equal(calls, 0);
      await assert.rejects(access(controlRoot), { code: 'ENOENT' });
    }
  } finally {
    if (previous === undefined) delete process.env.APPARENT_CAPTURES_DIR; else process.env.APPARENT_CAPTURES_DIR = previous;
    await rm(root, { recursive: true, force: true });
  }
});
