import assert from 'node:assert/strict'
import { execFileSync, spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import { chmod, copyFile, mkdir, mkdtemp, readFile, realpath, rm, stat, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { test } from 'node:test'
import { createMediaJobId, MediaClient, MediaChildProcessTransport, launchMediaWorker } from '../../../sdk/src/media.ts'

test('packaged entry decodes real H.264 with ambient plugin and loader paths closed', async t => {
  const entry = process.env.APPARENT_ENGINE_FRAMES_BIN
  if (!entry) { t.skip('requires the built package entry'); return }
  const diagnostics = process.env.APPARENT_MEDIA_TEST_DIAGNOSTICS === '1' || process.env.CI === 'true'
  const root = await mkdtemp(join(await realpath(tmpdir()), 'apparent-media-h264-'))
  await chmod(root, 0o700)
  const runtime = join(root, 'runtime'); const bundle = join(root, 'synthetic.apparent')
  await mkdir(runtime, { mode: 0o700 }); await mkdir(bundle, { mode: 0o700 })
  const video = join(bundle, 'recording.mp4')
  if (process.env.APPARENT_MEDIA_FIXTURE) await copyFile(resolve(process.env.APPARENT_MEDIA_FIXTURE), video)
  else {
    // Linux must exercise a real display-sized source under the unchanged
    // supervised address-space cap, including libav decoder thread overhead.
    const linux = process.platform === 'linux'
    const source = linux ? '1366x768:rate=30' : '640x360:rate=10'
    const encoder = linux ? ['-preset', 'veryfast', '-tune', 'zerolatency', '-g', '60', '-b:v', '6000k'] : []
    execFileSync(process.env.APPARENT_FFMPEG ?? 'ffmpeg', ['-v', 'error', '-f', 'lavfi', '-i', `testsrc2=size=${source}:duration=2`, '-an', '-c:v', 'libx264', '-threads', '1', ...encoder, '-pix_fmt', 'yuv420p', '-movflags', '+faststart', video], { timeout: 15000 })
  }
  await chmod(video, 0o600)
  await writeFile(join(bundle, 'apparent.json'), JSON.stringify({ version: 1, id: 'synthetic', durationMs: 2250, video: 'recording.mp4', videoOffsetMs: 250 }), { mode: 0o600 })
  const m = await stat(video, { bigint: true })
  const old = Object.fromEntries(['LD_LIBRARY_PATH', 'GST_PLUGIN_PATH', 'GST_PLUGIN_PATH_1_0', 'GST_PLUGIN_SYSTEM_PATH', 'GST_PLUGIN_SYSTEM_PATH_1_0'].map(k => [k, process.env[k]]))
  for (const k of Object.keys(old)) process.env[k] = '/nonexistent/host-runtime'
  let client: MediaClient | undefined
  let completed = false
  let diagnosticStderr = ''
  const observeDiagnostic = (chunk: Buffer) => { diagnosticStderr = (diagnosticStderr + chunk.toString('utf8')).slice(-8192) }
  let stopObserving = () => {}
  try {
    if (diagnostics) {
      const absoluteEntry = resolve(entry)
      const child = spawn(absoluteEntry, ['--media-runtime-directory', runtime, '--media-channel', 'media', '--media-protocol-version', '1'], {
        cwd: dirname(absoluteEntry), stdio: ['pipe', 'pipe', 'pipe'], shell: false,
        env: { ...process.env, APPARENT_MEDIA_TEST_DIAGNOSTICS: '1' },
      })
      child.stderr.on('data', observeDiagnostic)
      stopObserving = () => child.stderr.off('data', observeDiagnostic)
      client = new MediaClient(new MediaChildProcessTransport(child))
    } else client = new MediaClient(launchMediaWorker(resolve(entry), runtime))
    assert.equal((await client.handshake()).capabilities.decodeFrames, true)
    const outputDirectory = join(root, 'frames'); await mkdir(outputDirectory, { mode: 0o700 })
    const input = { jobId: createMediaJobId(), source: { bundleId: 'synthetic', bundlePath: bundle, videoPath: video, videoOffsetMs: 250, identity: { device: String(m.dev), inode: String(m.ino), size: Number(m.size), modifiedNs: String(m.mtimeNs) } }, requestedBundleMs: [1250, 0, 5000, 1250], outputDirectory, limits: { maxFrames: 48, maxEdge: 320, maxTotalPixels: 64000000, deadlineMs: 30000 } }
    const result = await client.decode(input)
    assert.deepEqual(result.frames.map(f => f.effectiveVideoMs), [1000, 0, 4750, 1000])
    assert.deepEqual(result.frames.map(f => f.clampedToStart), [false, true, false, false])
    assert.deepEqual(result.frames.map(f => f.clampedToEnd), [false, false, true, false])
    assert.ok(result.frames[2].actualVideoMs! < result.frames[2].effectiveVideoMs)
    if (process.platform === 'darwin') {
      assert.equal(result.frames[2].actualVideoMs, 1900, 'the 2-second 10-fps fixture clamps to its final sample')
      const last = await client.decode({ ...input, jobId: createMediaJobId(), outputDirectory: join(root, 'last-frame'), requestedBundleMs: [2150] })
      assert.equal(result.frames[2].sha256, last.frames[0].sha256, 'end clamp must preserve the last frame pixels')
    }
    for (const frame of result.frames) { const bytes = await readFile(frame.path); assert.equal(bytes.subarray(0, 8).toString('hex'), '89504e470d0a1a0a'); assert.equal(frame.sha256, createHash('sha256').update(bytes).digest('hex')); assert.equal(bytes.readUInt32BE(16), 320); assert.equal(bytes.readUInt32BE(20), 180) }
    assert.equal(result.frames[0].sha256, result.frames[3].sha256); assert.notEqual(result.frames[0].sha256, result.frames[1].sha256)
    const evidence = process.env.APPARENT_MEDIA_EVIDENCE
    if (evidence) {
      await mkdir(evidence, { recursive: true, mode: 0o700 })
      await copyFile(video, join(evidence, 'recording.mp4'))
      await copyFile(join(bundle, 'apparent.json'), join(evidence, 'apparent.json'))
      for (const [index, frame] of result.frames.entries()) await copyFile(frame.path, join(evidence, `frame-${index + 1}.png`))
      await writeFile(join(evidence, 'packaged-h264-result.json'), JSON.stringify({ result, sourceSha256: createHash('sha256').update(await readFile(video)).digest('hex') }, null, 2), { mode: 0o600 })
    }
    completed = true
  } finally {
    for (const [k, v] of Object.entries(old)) { if (v === undefined) delete process.env[k]; else process.env[k] = v }
    let closed = false
    try {
      await client?.close()
      closed = true
    } finally {
      stopObserving()
      if ((completed && closed) || !diagnostics) {
        await rm(root, { recursive: true, force: true })
      } else {
        const details = diagnosticStderr.split('\n').filter(line => line.startsWith('apparent-media-diagnostic ') && line.length <= 256).join('\n').slice(0, 4096)
        await writeFile(join(root, 'media-diagnostics.log'), details, { mode: 0o600 })
        if (details) process.stderr.write(details + '\n')
        console.error(`packaged-h264 synthetic runtime retained: ${root}`)
      }
    }
  }
})
