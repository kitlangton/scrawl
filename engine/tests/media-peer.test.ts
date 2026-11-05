import assert from 'node:assert/strict';
import { execFileSync, spawn } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmod, copyFile, mkdir, mkdtemp, readFile, readdir, rename, rm, stat, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { test } from 'node:test';
import { createMediaJobId, launchMediaWorker, MediaChildProcessTransport, MediaClient, MediaRpcError, type DecodeRequest, type MediaJobStatus } from '../sdk/src/media.ts';

const binary = process.env.APPARENT_ENGINE_FRAMES_BIN;
type Input = Omit<DecodeRequest, 'channel' | 'protocolVersion' | 'clientId' | 'rpcId' | 'method'>;
const pause = (ms: number) => new Promise<void>(r => setTimeout(r, ms));

async function fixture() {
  const root = await mkdtemp(join(tmpdir(), 'apparent-media-peer-'));
  await chmod(root, 0o700);
  const runtime = join(root, 'runtime');
  const bundle = join(root, 'synthetic.apparent');
  await mkdir(runtime, { mode: 0o700 }); await mkdir(bundle, { mode: 0o700 });
  const video = join(bundle, 'recording.mov');
  // Fixture generation only. Production decoding never invokes FFmpeg.
  execFileSync('ffmpeg', ['-v', 'error', '-f', 'lavfi', '-i', 'testsrc2=size=640x360:rate=10:duration=2', '-an', '-c:v', 'mjpeg', '-q:v', '3', '-threads', '1', video], { timeout: 10000 });
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
  return { root, runtime, bundle, video, input, client, transport, async close() { await client.close(); await rm(root, { recursive: true, force: true }); } };
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
    assert.ok(result.frames[2].actualVideoMs! < 2000);
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
    if (keep) { await mkdir(keep, { recursive: true, mode: 0o700 }); await copyFile(result.frames[0].path, join(keep, 'sample.png')); await writeFile(join(keep, 'native-result.json'), JSON.stringify(result, null, 2), { mode: 0o600 }); }
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

test('native media rejects foreign channels/versions, duplicate RPCs and oversized input', { skip: !binary }, async () => {
  const root = await mkdtemp(join(tmpdir(), 'apparent-media-wire-')); await chmod(root, 0o700);
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

test('forced supervisor death stops its pinned decoder and leaves only private unmanifested reservations', { skip: !binary || process.platform !== 'linux' }, async () => {
  const f = await fixture();
  await f.client.close();
  const child = spawn(resolve(binary!), ['--media-runtime-directory', f.runtime, '--media-channel', 'media', '--media-protocol-version', '1'], { stdio: ['pipe', 'pipe', 'pipe'] });
  const client = new MediaClient(new MediaChildProcessTransport(child));
  let decoderPid: number | undefined;
  let safeCleanup = false;
  try {
    await client.handshake();
    const input = { ...f.input, requestedBundleMs: Array.from({ length: 48 }, (_, i) => i * 20), limits: { ...f.input.limits, maxEdge: 640 } };
    const decoded = client.decode(input).then(() => 'complete', () => 'worker lost');
    assert.equal((await client.inspect(input)).state, 'decoding');
    const until = Date.now() + 2000;
    while (!decoderPid && Date.now() < until) {
      const children = await readFile(`/proc/${child.pid}/task/${child.pid}/children`, 'utf8');
      decoderPid = Number(children.trim().split(/\s+/)[0]) || undefined;
      if (!decoderPid) await pause(2);
    }
    assert.ok(decoderPid, 'owned decoder was not observed');
    const closed = new Promise<void>(r => child.once('close', () => r()));
    child.kill('SIGKILL'); await closed;
    assert.equal(await decoded, 'worker lost');
    // A zombie has exited and released every descriptor; it is not a writer.
    let stopped = false;
    const exitDeadline = Date.now() + 3000;
    while (!stopped && Date.now() < exitDeadline) {
      try { stopped = (await readFile(`/proc/${decoderPid}/stat`, 'utf8')).split(') ')[1].startsWith('Z'); }
      catch (e) { if ((e as NodeJS.ErrnoException).code === 'ENOENT') stopped = true; else throw e; }
      if (!stopped) await pause(10);
    }
    assert.equal(stopped, true, 'decoder remains a live writer after supervisor death');
    safeCleanup = true;
    const remaining = await readdir(input.outputDirectory);
    assert.equal(remaining.includes('manifest.json'), false);
    assert.equal((await stat(input.outputDirectory)).mode & 0o777, 0o700);
    for (const name of remaining) {
      assert.match(name, /^frame-\d{3}\.png$/);
      assert.equal((await stat(join(input.outputDirectory, name))).mode & 0o777, 0o600);
    }
  } finally {
    child.kill('SIGKILL'); await client.close();
    if (safeCleanup) await f.close();
    else console.error('Retained synthetic fixture because decoder cleanup was not established:', f.root);
  }
});
