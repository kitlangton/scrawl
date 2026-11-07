import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { createHash, randomBytes } from 'node:crypto';
import { chmod, copyFile, mkdir, mkdtemp, readFile, readdir, realpath, rename, rm, stat, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { test } from 'node:test';
import { Media2Client, launchMedia2Worker, validateDerivative2Document, type Media2JobInput } from '../sdk/src/media2/index.ts';
import { readFrameVisualObservations } from '../sdk/src/observations/index.ts';
import { validateMaterialsDocument } from '../sdk/src/capture-documents/index.ts';
import { launchMedia2RecoveryPeer } from '../sdk/src/recovery/index.ts';
import { createMediaJobId, launchMediaWorker, MediaChildProcessTransport, MediaClient, MediaRpcError, type DecodeRequest, type MediaJobStatus } from '../sdk/src/media.ts';

const binary = process.env.APPARENT_ENGINE_FRAMES_BIN;

// Qualification-only process observation, never an SDK/consumer close workaround.
// A released recovery peer may still finalize/reap after its receipt is complete.
async function fixturePeerExited(pid: number, timeoutMs = 2000): Promise<boolean> {
  const deadline = Date.now() + timeoutMs;
  for (;;) {
    try { process.kill(pid, 0); }
    catch (error) { return (error as NodeJS.ErrnoException).code === 'ESRCH'; }
    if (Date.now() >= deadline) return false;
    await pause(10);
  }
}
const shellQuote = (value: string) => `'${value.replaceAll("'", "'\\''")}'`;

test('fixture cleanup requires observed process exit rather than a terminal receipt', async () => {
  const child = spawn(process.execPath, ['-e', 'process.stdout.write("ready"); process.stdin.resume();'], { stdio: ['pipe', 'pipe', 'pipe'] });
  const done = new Promise<void>((resolve, reject) => {
    child.once('error', reject);
    child.once('close', code => code === 0 ? resolve() : reject(new Error(`fixture process exit ${code}`)));
  });
  void done.catch(() => {});
  try {
    await new Promise<void>((resolve, reject) => { child.once('error', reject); child.stdout.once('data', () => resolve()); });
    assert.equal(await fixturePeerExited(child.pid!, 20), false, 'a live process is not cleanup permission');
    child.stdin.end();
    await done;
    assert.equal(await fixturePeerExited(child.pid!, 0), true);
  } finally {
    if (child.exitCode === null && child.signalCode === null) child.kill();
    // The main path already checks success. Await cleanup without replacing a
    // failed assertion/readiness error with the expected signal-exit rejection.
    await done.catch(() => {});
  }
});
type Input = Omit<DecodeRequest, 'channel' | 'protocolVersion' | 'clientId' | 'rpcId' | 'method'>;
const pause = (ms: number) => new Promise<void>(r => setTimeout(r, ms));

async function fixture() {
  const root = await mkdtemp(join(await realpath(tmpdir()), 'apparent-media-peer-'));
  await chmod(root, 0o700);
  const runtime = join(root, 'runtime');
  const bundle = join(root, 'synthetic.apparent');
  await mkdir(runtime, { mode: 0o700 }); await mkdir(bundle, { mode: 0o700 });
  const video = join(bundle, 'recording.mov');
  // Fixture generation only. Production decoding never invokes FFmpeg.
  if (process.env.APPARENT_MEDIA_FIXTURE) await copyFile(resolve(process.env.APPARENT_MEDIA_FIXTURE), video);
  else execFileSync('ffmpeg', ['-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=640x360:rate=10:duration=2', '-an', '-c:v', 'mjpeg', '-q:v', '3', '-threads', '1', video], { timeout: 10000 });
  await chmod(video, 0o600);
  await writeFile(join(bundle, 'apparent.json'), JSON.stringify({ version: 1, id: 'synthetic', durationMs: 2250, video: 'recording.mov', videoOffsetMs: 250 }), { mode: 0o600 });
  const metadata = await stat(video, { bigint: true });
  const input: Input = {
    jobId: createMediaJobId(),
    source: { bundleId: 'synthetic', bundlePath: bundle, videoPath: video, videoOffsetMs: 250,
      identity: { device: String(metadata.dev), inode: String(metadata.ino), size: Number(metadata.size), modifiedNs: String(metadata.mtimeNs) } },
    requestedBundleMs: [1250, 0, 5000, 1250], outputDirectory: join(root, 'output'),
    limits: { maxFrames: 48, maxEdge: 320, maxTotalPixels: 64000000, deadlineMs: 10000 },
  };
  const transport = launchMediaWorker(resolve(binary!), runtime);
  const client = new MediaClient(transport);
  const handshake = await client.handshake();
  assert.equal(handshake.implementation, 'native'); assert.equal(handshake.capabilities.decodeFrames, true);
  return { root, runtime, bundle, video, input, client, transport, async close(retain = false) { await client.close(); if (!retain) await rm(root, { recursive: true, force: true }); } };
}
async function terminal(client: MediaClient, input: Input): Promise<MediaJobStatus> {
  const until = Date.now() + 12000;
  while (Date.now() < until) {
    const status = await client.inspect(input);
    if (['complete', 'cancelled', 'failed'].includes(status.state)) return status;
    await pause(10);
  }
  throw new Error('job did not reach terminal state');
}
function code(expected: string) { return (e: unknown) => e instanceof MediaRpcError && e.code === expected; }

test('native media/1 decodes actual ordered PNGs, offsets, duplicates and end clamps durably', { skip: !binary }, async () => {
  const f = await fixture();
  try {
    const result = await f.client.decode(f.input);
    assert.deepEqual(result.frames.map(r => r.requestedBundleMs), [1250, 0, 5000, 1250]);
    assert.deepEqual(result.frames.map(r => r.effectiveVideoMs), [1000, 0, 4750, 1000]);
    assert.deepEqual(result.frames.map(r => r.clampedToStart), [false, true, false, false]);
    assert.deepEqual(result.frames.map(r => r.clampedToEnd), [false, false, true, false]);
    assert.ok(result.frames[2].actualVideoMs! < result.frames[2].effectiveVideoMs);
    if (process.platform === 'darwin') {
      assert.equal(result.frames[2].actualVideoMs, 1900, 'the 2-second 10-fps fixture clamps to its final sample');
      const last = await f.client.decode({ ...f.input, jobId: createMediaJobId(), outputDirectory: join(f.root, 'last-frame'), requestedBundleMs: [2150] });
      assert.equal(result.frames[2].sha256, last.frames[0].sha256, 'end clamp returns the same pixels as the fixture last decodable frame');
    }
    for (const frame of result.frames) {
      const bytes = await readFile(frame.path);
      assert.equal(bytes.subarray(0, 8).toString('hex'), '89504e470d0a1a0a');
      assert.equal(bytes.readUInt32BE(16), 320); assert.equal(bytes.readUInt32BE(20), 180);
      assert.equal(frame.byteLength, bytes.length);
      assert.equal(frame.sha256, createHash('sha256').update(bytes).digest('hex'));
      assert.equal(Number((await stat(frame.path)).mode) & 0o777, 0o600);
    }
    assert.equal(result.frames[0].sha256, result.frames[3].sha256);
    assert.notEqual(result.frames[0].sha256, result.frames[1].sha256);
    assert.deepEqual(JSON.parse(await readFile(join(f.input.outputDirectory, 'manifest.json'), 'utf8')), result);
    assert.deepEqual(await f.client.inspect(f.input), result);
    assert.equal(await f.client.cancel(f.input.jobId), 'complete');
    await assert.rejects(f.client.decode(f.input), code('busy'));
    await assert.rejects(f.client.decode({ ...f.input, requestedBundleMs: [0] }), code('identity_conflict'));
    const keep = process.env.APPARENT_MEDIA_EVIDENCE;
    if (keep) {
      await mkdir(keep, { recursive: true, mode: 0o700 });
      for (const [index, frame] of result.frames.entries()) await copyFile(frame.path, join(keep, `frame-${index + 1}.png`));
      await copyFile(result.frames[0].path, join(keep, 'sample.png'));
      await copyFile(f.video, join(keep, 'recording.mov'));
      await copyFile(join(f.bundle, 'apparent.json'), join(keep, 'apparent.json'));
      await writeFile(join(keep, 'native-result.json'), JSON.stringify(result, null, 2), { mode: 0o600 });
    }
  } finally { await f.close(); }
});

test('native media rejects source mismatch, unsafe markers, corrupt sources and output collisions', { skip: !binary }, async () => {
  const f = await fixture();
  const next = (out: string): Input => ({ ...f.input, jobId: createMediaJobId(), outputDirectory: join(f.root, out) });
  try {
    const mismatch = next('mismatch'); mismatch.source = { ...mismatch.source, identity: { ...mismatch.source.identity, inode: '1' } };
    await assert.rejects(f.client.decode(mismatch), code('source_mismatch'));
    await writeFile(join(f.bundle, '.unsafe-video'), '');
    await assert.rejects(f.client.decode(next('unsafe')), code('source_incomplete'));
    await rm(join(f.bundle, '.unsafe-video'));
    await mkdir(join(f.root, 'existing'), { mode: 0o700 });
    await writeFile(join(f.root, 'existing', 'keep'), 'untouched');
    await assert.rejects(f.client.decode(next('existing')), code('invalid_output'));
    await symlink(join(f.root, 'existing'), join(f.root, 'linked'));
    await assert.rejects(f.client.decode(next('linked')), code('invalid_output'));
    assert.equal(await readFile(join(f.root, 'existing', 'keep'), 'utf8'), 'untouched');
    await writeFile(f.video, 'not a movie');
    const m = await stat(f.video, { bigint: true });
    const corrupt = next('corrupt'); corrupt.source = { ...corrupt.source, identity: { device: String(m.dev), inode: String(m.ino), size: Number(m.size), modifiedNs: String(m.mtimeNs) } };
    await assert.rejects(f.client.decode(corrupt), (e: unknown) => e instanceof MediaRpcError && ['source_incomplete', 'decode_failed'].includes(e.code));
    assert.deepEqual(await readdir(corrupt.outputDirectory), []);
    assert.equal((await f.client.inspect(corrupt)).state, 'failed');
  } finally { await f.close(); }
});

test('native media enforces aggregate pixel limits and reaps deadline/cancelled jobs without inventory', { skip: !binary }, async () => {
  const f = await fixture();
  try {
    const small = { ...f.input, limits: { ...f.input.limits, maxTotalPixels: 1 } };
    await assert.rejects(f.client.decode(small), code('limits_exceeded'));
    assert.deepEqual(await readdir(small.outputDirectory), []);
    const expired = { ...f.input, jobId: createMediaJobId(), outputDirectory: join(f.root, 'deadline'), limits: { ...f.input.limits, deadlineMs: 1 } };
    await assert.rejects(f.client.decode(expired, 10000), code('deadline_exceeded'));
    assert.equal((await terminal(f.client, expired)).state, 'failed');
    assert.deepEqual(await readdir(expired.outputDirectory), []);
    const cancel = { ...f.input, jobId: createMediaJobId(), outputDirectory: join(f.root, 'cancel'), requestedBundleMs: Array.from({ length: 48 }, (_, i) => i * 20), limits: { ...f.input.limits, maxEdge: 640 } };
    const decoding = f.client.decode(cancel).then(() => 'complete', (e: unknown) => e instanceof MediaRpcError ? e.code : Promise.reject(e));
    assert.equal(await f.client.cancel(cancel.jobId), 'cancelling');
    assert.equal(await decoding, 'cancelled');
    assert.equal((await terminal(f.client, cancel)).state, 'cancelled');
    assert.deepEqual(await readdir(cancel.outputDirectory), []);
    assert.equal(await f.client.cancel(cancel.jobId), 'cancelled');
  } finally { await f.close(); }
});

test('native media exclusively claims a caller-provisioned empty private output directory', { skip: !binary }, async () => {
  const f = await fixture();
  try {
    await mkdir(f.input.outputDirectory, { mode: 0o700 });
    const result = await f.client.decode(f.input);
    assert.equal(result.state, 'complete');
    assert.deepEqual((await readdir(f.input.outputDirectory)).sort(), ['frame-001.png', 'frame-002.png', 'frame-003.png', 'frame-004.png', 'manifest.json']);
    const another = { ...f.input, jobId: createMediaJobId() };
    await assert.rejects(f.client.decode(another), code('invalid_output'));
    const stale = join(f.root, 'stale');
    await mkdir(stale, { mode: 0o700 });
    await writeFile(join(stale, '.engine-media-job'), 'foreign claim', { mode: 0o600 });
    await assert.rejects(f.client.decode({ ...f.input, jobId: createMediaJobId(), outputDirectory: stale }), code('invalid_output'));
    assert.equal(await readFile(join(stale, '.engine-media-job'), 'utf8'), 'foreign claim');
  } finally { await f.close(); }
});

test('native media survives RPC timeout and pins against a source pathname replacement', { skip: !binary }, async () => {
  const f = await fixture();
  try {
    await assert.rejects(f.client.decode(f.input, 1), /timed out/);
    const recovered = await terminal(f.client, f.input);
    assert.equal(recovered.state, 'complete');
    const swapped = { ...f.input, jobId: createMediaJobId(), outputDirectory: join(f.root, 'swapped'), requestedBundleMs: Array.from({ length: 48 }, (_, i) => i * 20), limits: { ...f.input.limits, maxEdge: 640 } };
    const decoding = f.client.decode(swapped).then(() => 'complete', (e: unknown) => e instanceof MediaRpcError ? e.code : Promise.reject(e));
    const state = await f.client.inspect(swapped);
    assert.equal(state.state, 'decoding');
    await rename(f.video, join(f.bundle, 'original.mov'));
    await writeFile(f.video, 'replacement is never decoded', { mode: 0o600 });
    assert.equal(await decoding, 'source_mismatch');
    assert.deepEqual(await readdir(swapped.outputDirectory), []);
    assert.equal((await f.client.inspect(swapped)).state, 'failed');
  } finally { await f.close(); }
});

test('independent workers cannot both claim the same empty output', { skip: !binary }, async () => {
  const f = await fixture();
  const other = new MediaClient(launchMediaWorker(resolve(binary!), f.runtime));
  try {
    await other.handshake();
    // Bootstrap the shared lease anchors before racing output admission. Core
    // deliberately fails closed if a contender sees an incomplete first anchor;
    // its separate cross-process bootstrap tests cover that boundary.
    await f.client.decode({ ...f.input, jobId: createMediaJobId(), outputDirectory: join(f.root, 'warmup'), requestedBundleMs: [0] });
    await mkdir(f.input.outputDirectory, { mode: 0o700 });
    const outcomes = await Promise.allSettled([f.client.decode(f.input), other.decode({ ...f.input, jobId: createMediaJobId() })]);
    const winners = outcomes.filter((r): r is PromiseFulfilledResult<Awaited<ReturnType<MediaClient['decode']>>> => r.status === 'fulfilled');
    assert.equal(winners.length, 1);
    const loser = outcomes.find(r => r.status === 'rejected') as PromiseRejectedResult;
    assert.ok(loser.reason instanceof MediaRpcError);
    assert.equal(loser.reason.code, 'invalid_output');
    assert.deepEqual(JSON.parse(await readFile(join(f.input.outputDirectory, 'manifest.json'), 'utf8')), winners[0].value);
  } finally { await other.close(); await f.close(); }
});

test('native media rejects foreign channels/versions, duplicate RPCs and oversized input', { skip: !binary }, async () => {
  const root = await mkdtemp(join(await realpath(tmpdir()), 'apparent-media-wire-')); await chmod(root, 0o700);
  const args = ['--media-runtime-directory', root, '--media-channel', 'media', '--media-protocol-version', '1'];
  const envelope = { channel: 'media', protocolVersion: 1, clientId: 'a'.repeat(32), rpcId: 'b'.repeat(32), method: 'handshake' };
  async function exchange(lines: string) {
    const child = spawn(resolve(binary!), args, { stdio: ['pipe', 'pipe', 'pipe'] });
    let stdout = ''; child.stdout.setEncoding('utf8').on('data', (s: string) => { stdout += s; });
    child.stdin.on('error', () => {}); child.stdin.end(lines);
    const exit = await new Promise<number | null>((res, rej) => { child.once('error', rej); child.once('close', res); });
    return { exit, responses: stdout.trim().split('\n').filter(Boolean).map(s => JSON.parse(s)) };
  }
  try {
    const foreign = await exchange(JSON.stringify({ ...envelope, channel: 'capture' }) + '\n');
    assert.equal(foreign.responses[0].error.code, 'unsupported_channel');
    const version = await exchange(JSON.stringify({ ...envelope, protocolVersion: 2 }) + '\n');
    assert.equal(version.responses[0].error.code, 'unsupported_version');
    const unknown = await exchange(JSON.stringify({ ...envelope, unknown: true }) + '\n');
    assert.equal(unknown.responses[0].error.code, 'malformed_request');
    const duplicate = await exchange(`${JSON.stringify(envelope)}\n${JSON.stringify(envelope)}\n`);
    assert.equal(duplicate.responses[1].error.code, 'malformed_request');
    assert.notEqual((await exchange('x'.repeat(65539))).exit, 0);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test('forced supervisor death stops its pinned decoder and leaves only private unmanifested reservations', { skip: !binary }, async () => {
  const f = await fixture();
  await f.client.close();
  const child = spawn(resolve(binary!), ['--media-runtime-directory', f.runtime, '--media-channel', 'media', '--media-protocol-version', '1'], { stdio: ['pipe', 'pipe', 'pipe'] });
  const client = new MediaClient(new MediaChildProcessTransport(child));
  let decoderPid: number | undefined;
  let nativePid: number | undefined;
  const macProcesses = () => execFileSync('/bin/ps', ['-axo', 'pid=,ppid=,state=,command='], { encoding: 'utf8' })
    .split('\n').flatMap(line => { const m = line.match(/^\s*(\d+)\s+(\d+)\s+(\S+)\s+(.*)$/); return m ? [{ pid: Number(m[1]), parent: Number(m[2]), state: m[3], command: m[4] }] : [] });
  let safeCleanup = false;
  try {
    await client.handshake();
    const input = { ...f.input, requestedBundleMs: Array.from({ length: 48 }, (_, i) => i * 20), limits: { ...f.input.limits, maxEdge: 640 } };
    const decoded = client.decode(input).then(() => 'complete', () => 'worker lost');
    assert.equal((await client.inspect(input)).state, 'decoding');
    const until = Date.now() + 2000;
    while ((!decoderPid || (process.platform === 'darwin' && !nativePid)) && Date.now() < until) {
      if (process.platform === 'darwin') {
        const rows = macProcesses();
        decoderPid ??= rows.find(row => row.parent === child.pid && row.command.endsWith('--decode-pinned-job'))?.pid;
        nativePid ??= rows.find(row => row.parent === decoderPid && row.command.endsWith('--decode-pinned-frames'))?.pid;
      } else {
        const children = await readFile(`/proc/${child.pid}/task/${child.pid}/children`, 'utf8');
        decoderPid = Number(children.trim().split(/\s+/)[0]) || undefined;
      }
      await pause(2);
    }
    assert.ok(decoderPid, 'owned decoder was not observed');
    if (process.platform === 'darwin') assert.ok(nativePid, 'owned AVFoundation bridge was not observed');
    const closed = new Promise<void>(r => child.once('close', () => r()));
    child.kill('SIGKILL'); await closed;
    assert.equal(await decoded, 'worker lost');
    // A zombie has exited and released every descriptor; it is not a writer.
    let stopped = false;
    const exitDeadline = Date.now() + 3000;
    while (!stopped && Date.now() < exitDeadline) {
      if (process.platform === 'darwin') {
        const rows = macProcesses();
        stopped = [decoderPid, nativePid].every(pid => !rows.some(row => row.pid === pid && !row.state.startsWith('Z')));
      } else try { stopped = (await readFile(`/proc/${decoderPid}/stat`, 'utf8')).split(') ')[1].startsWith('Z'); }
      catch (e) { if (['ENOENT', 'ESRCH'].includes((e as NodeJS.ErrnoException).code ?? '')) stopped = true; else throw e; }
      if (!stopped) await pause(10);
    }
    assert.equal(stopped, true, 'decoder remains a live writer after supervisor death');
    safeCleanup = true;
    const remaining = await readdir(input.outputDirectory);
    assert.equal(remaining.includes('manifest.json'), false);
    assert.equal((await stat(input.outputDirectory)).mode & 0o777, 0o700);
    for (const name of remaining) {
      assert.match(name, /^(?:frame-\d{3}\.png|\.engine-media-job)$/);
      assert.equal((await stat(join(input.outputDirectory, name))).mode & 0o777, 0o600);
    }
  } finally {
    child.kill('SIGKILL'); await client.close();
    if (safeCleanup) await f.close();
    else console.error('Retained synthetic fixture because decoder cleanup was not established:', f.root);
  }
});


if (process.platform === 'linux' || process.platform === 'darwin') test('packaged media/2 negotiates separately and authenticates before unknown-job lookup', { skip: !binary }, async () => {
  const root = await mkdtemp(join(await realpath(tmpdir()), 'apparent-media2-handshake-'));
  await chmod(root, 0o700);
  await mkdir(join(root, 'capture-v2'), { mode: 0o700 });
  const transport = launchMedia2Worker(resolve(binary!), root);
  const client = new Media2Client(transport);
  try {
    const hello = await client.handshake();
    const supported = process.platform === 'linux' || process.platform === 'darwin';
    assert.equal(hello.implementation, 'native');
    assert.deepEqual(hello.capabilities, { retainedFrames: supported, contactSheet: supported, nativeJobAuthority: supported });
    const reply = await transport.request({ channel: 'media', protocolVersion: 2, clientId: client.clientId, rpcId: 'b'.repeat(32), method: 'inspect', captureId: 'c'.repeat(32), ownerToken: 'd'.repeat(64), jobId: 'e'.repeat(32), requestSha256: 'f'.repeat(64) }, 5000);
    assert.deepEqual(reply.error, { code: supported ? 'not_found' : 'unavailable' });
    assert.equal(reply.result, undefined);
    const namespace = join(root, 'materials-media-2');
    assert.deepEqual(await readdir(namespace), [], 'unauthenticated lookup creates no job or output');
  } finally { await client.close(); await rm(root, { recursive: true, force: true }); }
});

// Synthetic retained authority only: this exercises native decode/registry and
// public transport, without claiming that macOS capture produced the materials.
test('packaged retained media/2 decodes H264 with native seals and cold recovery', { skip: !binary }, async () => {
  const f = await fixture();
  const id = () => randomBytes(16).toString('hex');
  const captureId = id(), materialsId = id(), leaseId = id();
  const ownerToken = randomBytes(32).toString('hex');
  const captureRoot = join(f.runtime, 'capture-v2');
  const materialRoot = join(captureRoot, 'materials', captureId);
  await mkdir(materialRoot, { recursive: true, mode: 0o700 });
  await chmod(captureRoot, 0o700); await chmod(join(captureRoot, 'materials'), 0o700);
  const seal = async (relativePath: string) => {
    const file = join(materialRoot, relativePath);
    const bytes = await readFile(file), info = await stat(file, { bigint: true });
    return { relativePath, identity: { device: String(info.dev), inode: String(info.ino), size: Number(info.size), modifiedNs: String(info.mtimeNs) }, sha256: createHash('sha256').update(bytes).digest('hex') };
  };
  await copyFile(resolve(process.env.APPARENT_RETAINED_MEDIA_FIXTURE ?? 'engine/core/tests/fixtures/tiny.mp4'), join(materialRoot, 'recording.mp4'));
  await chmod(join(materialRoot, 'recording.mp4'), 0o600);
  const video = await seal('recording.mp4');
  const payload = {
    schema: 'apparent.capture-materials-payload/1', identity: { captureId, materialsId }, bundleId: 'retained-fixture', root: materialRoot, leaseId,
    clock: { epoch: 'capture-admission-monotonic', units: 'integer-milliseconds', durationMs: 1000, videoOffsetMs: 250 },
    confirmedSource: { id: 'display:fixture', generation: id(), confirmedAtMs: 1, kind: 'display', displayId: 1, rectangle: { x: 0, y: 0, width: 16, height: 16, scale: 1 }, confirmation: { kind: 'explicit-display', id: id() } },
    invocation: { kind: 'explicit', invocationId: id(), observedAtMs: 0 }, files: [video], video: { file: 'recording.mp4', width: 16, height: 16, durationMs: 200 }, facts: {},
    collection: { context: 'unavailable', interactions: 'unavailable', candidates: 'unavailable', transcript: 'unavailable', acceptedStrokes: 'unavailable' }, warnings: [], safety: { nativeRemoteDelivery: 'forbidden', safeForProcessing: true },
  };
  const payloadBytes = Buffer.from(JSON.stringify(payload));
  await writeFile(join(materialRoot, 'materials.json'), payloadBytes, { mode: 0o600, flag: 'wx' });
  const payloadFile = await seal('materials.json');
  const manifest = { schema: 'apparent.capture-materials/1', key: { captureId, materialsId, revision: payloadFile.sha256 }, root: materialRoot, leaseId, payloadFile };
  const materials = validateMaterialsDocument(manifest, payloadBytes);
  const rootInfo = await stat(materialRoot, { bigint: true });
  assert.ok(Number.isSafeInteger(Number(rootInfo.ino)) && Number.isSafeInteger(Number(rootInfo.dev)));
  await writeFile(join(captureRoot, `${captureId}.json`), JSON.stringify({
    capture_id: captureId, owner_token: ownerToken, state: 'materials-ready', root: materialRoot, materials: manifest, materials_id: materialsId, lease_id: leaseId,
    root_device: Number(rootInfo.dev), root_inode: Number(rootInfo.ino), release_phase: null, release_movie: video.identity, release_payload: payloadFile.identity, payload_digest: payloadFile.sha256,
  }), { mode: 0o600, flag: 'wx' });
  const input: Media2JobInput = { ownerToken, materials, request: {
    schema: 'apparent.retained-frame-request/1', materials: manifest.key, leaseId, jobId: id(), outputDirectory: join(f.root, 'retained-output'), requestedBundleMs: [0, 250, 1000, 0],
    limits: { maxFrames: 4, maxEdge: 16, maxTotalPixels: 4096, deadlineMs: 20_000 },
    contactSheet: { schema: 'apparent.contact-sheet-layout/1', width: 64, height: 16, backgroundRgb: [11, 22, 33], fit: 'stretch', sampling: 'legacy-bilinear', clipping: 'reject', tiles: [1, 2, 3, 4].map((requestIndex, index) => ({ requestIndex, x: index * 16, y: 0, width: 16, height: 16 })) },
  } };
  let media = new Media2Client(launchMedia2Worker(resolve(binary!), f.runtime));
  let recovery: Awaited<ReturnType<typeof launchMedia2RecoveryPeer>> | undefined;
  let recoveryPending = false;
  let recoveryPidPath: string | undefined;
  try {
    const hello = await media.handshake();
    assert.deepEqual(hello.capabilities, { retainedFrames: true, contactSheet: true, nativeJobAuthority: true });
    let status = await media.derive(input);
    const deadline = Date.now() + 25_000;
    while (status.state === 'running' && Date.now() < deadline) { await pause(20); status = await media.inspect(input); }
    assert.equal(status.state, 'complete', `retained fixture result: ${JSON.stringify(status)}`);
    if (status.state !== 'complete') assert.fail('native derivative is absent');
    const envelope = validateDerivative2Document(status.derivative, materials);
    assert.deepEqual(envelope.payload.frames.map(frame => frame.requestedBundleMs), [0, 250, 1000, 0]);
    assert.deepEqual(envelope.payload.frames.map(frame => frame.effectiveVideoMs), [0, 0, 750, 0]);
    assert.equal(envelope.payload.frames[0].clampedToStart, true);
    assert.equal(envelope.payload.frames[2].clampedToEnd, true);
    assert.ok(envelope.payload.frames[2].actualVideoMs! < 750);
    assert.ok(envelope.payload.contactSheet);
    for (const image of [...envelope.payload.frames, envelope.payload.contactSheet!]) {
      const bytes = await readFile(join(input.request.outputDirectory, image.file.relativePath));
      assert.equal(bytes.subarray(0, 8).toString('hex'), '89504e470d0a1a0a');
      assert.equal(bytes.readUInt32BE(16), image.width); assert.equal(bytes.readUInt32BE(20), image.height);
      assert.equal(createHash('sha256').update(bytes).digest('hex'), image.file.sha256);
      const info = await stat(join(input.request.outputDirectory, image.file.relativePath), { bigint: true });
      assert.deepEqual({ device: String(info.dev), inode: String(info.ino), size: Number(info.size), modifiedNs: String(info.mtimeNs) }, image.file.identity);
    }
    const observations = [];
    for (const frame of envelope.payload.frames) {
      const pngBytes = await readFile(join(input.request.outputDirectory, frame.file.relativePath));
      const observation = readFrameVisualObservations({ document: status.derivative, materials, requestIndex: frame.requestIndex, pngBytes, maxFileBytes: 4 * 1024 * 1024 });
      assert.equal(observation.state, 'measured', 'new native frame PNGs carry the sealed observation profile');
      if (observation.state !== 'measured') assert.fail('native observations missing');
      assert.equal(observation.source.jobId, input.request.jobId);
      assert.equal(observation.source.leaseId, input.request.leaseId);
      assert.deepEqual(observation.source.materials, input.request.materials);
      assert.deepEqual(observation.source.clock, materials.payload.clock);
      assert.deepEqual(observation.pixels, { width: frame.width, height: frame.height, origin: 'decoded-derivative-rgba8', alpha: 'ignored' });
      assert.equal(observation.regions.columns, Math.min(frame.width, 128));
      assert.equal(observation.regions.rows, Math.min(frame.height, 128));
      observations.push(observation);
    }
    assert.deepEqual(observations[0].thumbnail, observations[3].thumbnail, 'duplicate decoded planes retain the same raw measurements');
    assert.deepEqual(observations[0].regions, observations[3].regions);
    await media.close();
    media = new Media2Client(launchMedia2Worker(resolve(binary!), f.runtime));
    await media.handshake();
    assert.deepEqual(await media.inspect(input), status, 'cold inspection keeps the original native receipt');
    assert.deepEqual(await media.cancel(input), status, 'completed output wins late cancellation');
    await assert.rejects(media.inspect({ ...input, ownerToken: '0'.repeat(64) }), /permission_denied/);
    // Large explicit sheet work keeps an admitted job active while controls
    // race it; this is synthetic caller policy, not a default Record layout.
    const busyInput = (jobId: string, outputDirectory: string): Media2JobInput => ({ ...input, request: {
      ...input.request, jobId, outputDirectory, requestedBundleMs: Array.from({ length: 48 }, () => 0),
      limits: { ...input.request.limits, maxFrames: 48, maxTotalPixels: 20_000_000 },
      contactSheet: { ...input.request.contactSheet!, width: 4096, height: 4096 },
    } });
    const cancelledInput = busyInput(id(), join(f.root, 'retained-cancelled'));
    assert.equal((await media.derive(cancelledInput)).state, 'running');
    assert.equal((await media.cancel(cancelledInput)).state, 'cancelled');
    assert.equal((await media.inspect(cancelledInput)).state, 'cancelled');
    const lostInput = busyInput(id(), join(f.root, 'retained-owner-lost'));
    assert.equal((await media.derive(lostInput)).state, 'running');
    const owned = (media as unknown as { transport: { child: import('node:child_process').ChildProcess } }).transport.child;
    const exited = new Promise<void>(done => owned.once('exit', () => done()));
    assert.equal(owned.kill('SIGKILL'), true, 'only this test-owned service is killed');
    await exited;
    await media.close();
    media = new Media2Client(launchMedia2Worker(resolve(binary!), f.runtime));
    await media.handshake();
    let lost = await media.inspect(lostInput);
    const lostDeadline = Date.now() + 3000;
    while (lost.state === 'running' && Date.now() < lostDeadline) { await pause(10); lost = await media.inspect(lostInput); }
    assert.equal(lost.state, 'failed');
    if (lost.state !== 'failed') assert.fail('lost job was not reconciled');
    assert.equal(lost.error.code, 'owner_lost');
    assert.equal((await stat(materialRoot)).isDirectory(), true, 'cancel/process loss retains original materials');
    // Record only our fixture-owned exec PID. This transparent wrapper forwards
    // the exact argv to the packaged binary and does not proxy or signal it.
    const recoveryEntry = join(f.root, 'observed-recovery.sh');
    recoveryPidPath = join(f.root, 'observed-recovery.pid');
    await writeFile(recoveryEntry, `#!/bin/sh\nset -e\numask 077\nprintf '%s\\n' "$$" > ${shellQuote(recoveryPidPath)}\nexec ${shellQuote(resolve(binary!))} "$@"\n`, { mode: 0o700, flag: 'wx' });
    recovery = await launchMedia2RecoveryPeer(recoveryEntry, f.runtime);
    await recovery.client.handshake();
    const releasedInput = busyInput(id(), join(f.root, 'retained-released'));
    recoveryPending = true;
    assert.equal((await recovery.client.derive(releasedInput)).state, 'running');
    const release = await recovery.release();
    assert.equal(release.connection, 'released');
    assert.equal(release.runtimeClearance, 'not-established');
    await assert.rejects(recovery.client.close(), /peer.release/);
    let retained = await media.inspect(releasedInput);
    const retainedDeadline = Date.now() + 25_000;
    while (retained.state === 'running' && Date.now() < retainedDeadline) { await pause(20); retained = await media.inspect(releasedInput); }
    assert.equal(retained.state, 'complete', `released job original tuple: ${JSON.stringify(retained)}`);
    if (retained.state !== 'complete') assert.fail('release lost native derivative authority');
    const retainedEnvelope = validateDerivative2Document(retained.derivative, materials);
    assert.equal(retainedEnvelope.payload.jobId, releasedInput.request.jobId);
    const image = retainedEnvelope.payload.frames[0];
    assert.equal(createHash('sha256').update(await readFile(join(releasedInput.request.outputDirectory, image.file.relativePath))).digest('hex'), image.file.sha256);
    recoveryPending = false;
  } finally {
    await recovery?.release();
    await media.close();
    let peerExitObserved = recoveryPidPath === undefined;
    if (recoveryPidPath) {
      const pidText = await readFile(recoveryPidPath, 'utf8').catch(() => '');
      const pid = Number(pidText.trim());
      if (/^[1-9][0-9]*\n$/.test(pidText) && Number.isSafeInteger(pid) && pid !== process.pid) peerExitObserved = await fixturePeerExited(pid);
    }
    // Neither a complete receipt nor EOF acknowledgement grants runtime cleanup.
    // Keep uncertain fixtures intact rather than race native journal replacement.
    const retain = recoveryPending || !peerExitObserved;
    await f.close(retain);
    if (retain) console.error(`Unresolved synthetic retained recovery fixture: ${f.root}; native peer exit observed=${peerExitObserved}`);
    if (!recoveryPending) assert.equal(peerExitObserved, true, 'released native peer did not exit; fixture retained');
  }
});
