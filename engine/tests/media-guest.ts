// Guest-only headless H.264 check, invoked by the existing disposable VM runner.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { chmod, copyFile, mkdir, readFile, stat, writeFile } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { createMediaJobId, MediaClient, launchMediaWorker } from '../sdk/src/media.ts';

async function main() {
  const binary = process.env.SCRAWL_GUI_BINARY!;
  const root = process.env.SCRAWL_GUI_OUTPUT!;
  assert.ok(binary && root);
  await chmod(root, 0o700);
  const staged = dirname(binary);
  const runtime = join(root, 'runtime');
  const bundle = join(root, 'synthetic.apparent');
  await mkdir(runtime, { mode: 0o700 }); await mkdir(bundle, { mode: 0o700 });
  const video = join(bundle, 'recording.mp4');
  await copyFile(join(staged, 'synthetic-h264.mp4'), video); await chmod(video, 0o600);
  await writeFile(join(bundle, 'apparent.json'), JSON.stringify({ version: 1, id: 'synthetic', durationMs: 2250, video: 'recording.mp4', videoOffsetMs: 250 }), { mode: 0o600 });
  const m = await stat(video, { bigint: true });
  // Match the real Agent adapter, which provisions an empty private destination.
  const outputDirectory = join(root, 'frames');
  await mkdir(outputDirectory, { mode: 0o700 });
  // The native decoder starts no capture or display-dependent operation.
  for (const key of ['DISPLAY', 'WAYLAND_DISPLAY', 'DBUS_SESSION_BUS_ADDRESS', 'XAUTHORITY', 'XDG_RUNTIME_DIR']) delete process.env[key];
  const client = new MediaClient(launchMediaWorker(binary, runtime));
  let outcome: unknown;
  try {
    assert.equal((await client.handshake()).capabilities.decodeFrames, true);
    const input = { jobId: createMediaJobId(), source: { bundleId: 'synthetic', bundlePath: bundle, videoPath: video, videoOffsetMs: 250,
      identity: { device: String(m.dev), inode: String(m.ino), size: Number(m.size), modifiedNs: String(m.mtimeNs) } },
    requestedBundleMs: [1250, 0, 5000, 1250], outputDirectory, limits: { maxFrames: 48, maxEdge: 320, maxTotalPixels: 64000000, deadlineMs: 30000 } };
    const result = await client.decode(input);
    assert.deepEqual(result.frames.map(f => f.effectiveVideoMs), [1000, 0, 4750, 1000]);
    assert.deepEqual(result.frames.map(f => f.clampedToStart), [false, true, false, false]);
    assert.deepEqual(result.frames.map(f => f.clampedToEnd), [false, false, true, false]);
  assert.ok(result.frames[2].actualVideoMs! < result.frames[2].effectiveVideoMs);
    for (const f of result.frames) {
      const bytes = await readFile(f.path);
      assert.equal(bytes.subarray(0, 8).toString('hex'), '89504e470d0a1a0a');
      assert.equal(f.sha256, createHash('sha256').update(bytes).digest('hex'));
      assert.equal(bytes.length, f.byteLength);
      assert.equal(bytes.readUInt32BE(16), f.width); assert.equal(bytes.readUInt32BE(20), f.height);
      assert.equal(f.width, 320); assert.equal(f.height, 180);
    }
    assert.equal(result.frames[0].sha256, result.frames[3].sha256);
    assert.notEqual(result.frames[0].sha256, result.frames[1].sha256);
    assert.deepEqual(JSON.parse(await readFile(join(input.outputDirectory, 'manifest.json'), 'utf8')), result);
    assert.deepEqual(await client.inspect(input), result);
    outcome = { native: 'passed', codec: 'h264', sourceSha256: createHash('sha256').update(await readFile(video)).digest('hex'), result };
  } finally {
    const close = await client.close();
    assert.ok(['already-exited', 'eof-exited'].includes(close));
    if (outcome) await writeFile(join(root, 'media-proof.json'), JSON.stringify({ ...outcome as object, cleanup: close }, null, 2), { mode: 0o600 });
  }
}

main().catch((error: unknown) => {
  console.error(error instanceof Error ? error.message : 'Native media guest check failed');
  process.exitCode = 1;
});
