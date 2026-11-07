import assert from 'node:assert/strict'
import { execFileSync, spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import { chmod, copyFile, lstat, mkdir, mkdtemp, readFile, realpath, rm, stat, symlink, unlink, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { test } from 'node:test'
import { createMediaJobId, MediaClient, MediaChildProcessTransport, launchMediaWorker } from '../../../sdk/src/media.ts'

// Test-only producer. Composition independently validates this finite wire summary
// against the current run and artifact before allowing public upload.
async function writePublicSummary(directory: string, stderr: Buffer, identity: Record<string, unknown>, env: NodeJS.ProcessEnv) {
  const sha = /^[a-f0-9]{40}$/; const digest = /^[a-f0-9]{64}$/
  if (env.APPARENT_MEDIA_FIXTURE || identity.generated !== true ||
    !sha.test(env.GITHUB_SHA ?? '') || !sha.test(env.GITHUB_WORKFLOW_SHA ?? '') ||
    !/^[1-9][0-9]{0,19}$/.test(env.GITHUB_RUN_ID ?? '') || !/^[1-9][0-9]{0,5}$/.test(env.GITHUB_RUN_ATTEMPT ?? '') ||
    !['verify-pr', 'stream'].includes(env.GITHUB_JOB ?? '') || env.OWNER_ID !== 'engine-media-native' || env.OWNER_TARGET !== 'linux-x64' ||
    !digest.test(String(identity.entrySha256)) || !digest.test(String(identity.manifestSha256)) || identity.artifactSourceSha !== env.GITHUB_SHA) throw new Error('invalid generated diagnostic binding')
  const signatures = [
    ['allocation-failure', /cannot allocate memory|out of memory|failed to allocate|bad_alloc/i],
    ['allocator-corruption', /double free|corrupted (?:size|double-linked)|malloc\(\):|free\(\):/i],
    ['thread-creation-failure', /pthread_create.*(?:failed|error)|(?:failed|unable) to create (?:a )?thread|resource temporarily unavailable/i],
    ['assertion-failure', /assertion .*failed|assertion failed|assertion.*should not be reached/i],
    ['plugin-failure', /failed to load plugin|no (?:such )?(?:element|plugin)|missing plugin/i],
    ['loader-failure', /error while loading shared libraries|cannot open shared object file|undefined symbol/i],
  ] as const
  const tail = stderr.subarray(-8192).toString('utf8')
  const categories = signatures.filter(([, pattern]) => pattern.test(tail)).map(([name]) => name)
  const exit = [...tail.matchAll(/^apparent-media-diagnostic stage=worker-exit exit-code=(none|[0-9]{1,3}) signal=(none|[0-9]{1,2})$/gm)].at(-1)
  const summary = { schema: 'apparent.synthetic-media-public/1', sourceSha: env.GITHUB_SHA, workflowSha: env.GITHUB_WORKFLOW_SHA,
    runId: env.GITHUB_RUN_ID, attempt: env.GITHUB_RUN_ATTEMPT, job: env.GITHUB_JOB, owner: 'engine-media-native', target: 'linux-x64',
    entrySha256: identity.entrySha256, manifestSha256: identity.manifestSha256, stderrBytes: Math.min(stderr.length, 8192),
    categories: categories.length ? categories : ['unclassified'],
    workerExitCode: exit && exit[1] !== 'none' && Number(exit[1]) <= 255 ? Number(exit[1]) : null,
    workerSignal: exit && exit[2] !== 'none' && Number(exit[2]) <= 64 ? Number(exit[2]) : null,
    requestLimits: { maxFrames: 48, maxEdge: 320, maxTotalPixels: 64000000, deadlineMs: 30000 },
    testCompleted: identity.testCompleted === true, clientCloseReturned: identity.clientCloseReturned === true }
  await mkdir(directory, { mode: 0o700 })
  const s = await lstat(directory)
  if (!s.isDirectory() || s.isSymbolicLink() || s.uid !== process.getuid?.() || (s.mode & 0o777) !== 0o700) throw new Error('unsafe diagnostic directory')
  await writeFile(join(directory, 'summary.json'), JSON.stringify(summary) + '\n', { mode: 0o600, flag: 'wx' })
}

// This is synthetic-test evidence only; raw bytes never enter public diagnostics.
export async function retainSyntheticFailure(root: string, stderr: Buffer, identity: Record<string, unknown>) {
  const directory = await lstat(root)
  if (!directory.isDirectory() || directory.isSymbolicLink() || (directory.mode & 0o777) !== 0o700 || directory.uid !== process.getuid?.()) throw new Error('synthetic evidence root is not private and owned')
  const receipt = JSON.stringify({ schema: 'apparent.synthetic-media-failure/1', ...identity, stderrBytes: Math.min(stderr.length, 8192) }, null, 2)
  if (Buffer.byteLength(receipt) > 8192) throw new Error('synthetic identity receipt exceeds bound')
  await writeFile(join(root, 'media-stderr.raw'), stderr.subarray(-8192), { mode: 0o600, flag: 'wx' })
  await writeFile(join(root, 'media-failure-identity.json'), receipt, { mode: 0o600, flag: 'wx' })
}

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
  let diagnosticStderr: Buffer = Buffer.alloc(0)
  const diagnosticIdentity: Record<string, unknown> = { synthetic: true, generated: !process.env.APPARENT_MEDIA_FIXTURE, buildSourceSha: process.env.GITHUB_SHA ?? null, entry: resolve(entry), sourceIdentity: { device: String(m.dev), inode: String(m.ino), size: Number(m.size), modifiedNs: String(m.mtimeNs) } }
  const observeDiagnostic = (chunk: Buffer) => { diagnosticStderr = Buffer.concat([diagnosticStderr, chunk]).subarray(-8192) }
  let stopObserving = () => {}
  try {
    if (diagnostics) {
      const absoluteEntry = resolve(entry)
      diagnosticIdentity.entrySha256 = createHash('sha256').update(await readFile(absoluteEntry)).digest('hex')
      try { const bytes = await readFile(join(dirname(absoluteEntry), 'manifest.json')); diagnosticIdentity.manifestSha256 = createHash('sha256').update(bytes).digest('hex'); diagnosticIdentity.artifactSourceSha = JSON.parse(bytes.toString('utf8')).sourceSha ?? null } catch { diagnosticIdentity.manifestSha256 = null }
      const child = spawn(absoluteEntry, ['--media-runtime-directory', runtime, '--media-channel', 'media', '--media-protocol-version', '1'], {
        cwd: dirname(absoluteEntry), stdio: ['pipe', 'pipe', 'pipe'], shell: false,
        env: { ...process.env, APPARENT_MEDIA_TEST_DIAGNOSTICS: '1' },
      })
      diagnosticIdentity.pid = child.pid
      child.once('exit', (code, signal) => { diagnosticIdentity.exitCode = code; diagnosticIdentity.signal = signal })
      child.stderr.on('data', observeDiagnostic)
      stopObserving = () => child.stderr.off('data', observeDiagnostic)
      client = new MediaClient(new MediaChildProcessTransport(child))
    } else client = new MediaClient(launchMediaWorker(resolve(entry), runtime))
    assert.equal((await client.handshake()).capabilities.decodeFrames, true)
    const outputDirectory = join(root, 'frames'); await mkdir(outputDirectory, { mode: 0o700 })
    const input = { jobId: createMediaJobId(), source: { bundleId: 'synthetic', bundlePath: bundle, videoPath: video, videoOffsetMs: 250, identity: { device: String(m.dev), inode: String(m.ino), size: Number(m.size), modifiedNs: String(m.mtimeNs) } }, requestedBundleMs: [1250, 0, 5000, 1250], outputDirectory, limits: { maxFrames: 48, maxEdge: 320, maxTotalPixels: 64000000, deadlineMs: 30000 } }
    diagnosticIdentity.request = input
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
        diagnosticIdentity.testCompleted = completed; diagnosticIdentity.clientCloseReturned = closed
        if (process.env.CI === 'true' && process.env.APPARENT_SYNTHETIC_DIAGNOSTICS_DIR && !process.env.APPARENT_MEDIA_FIXTURE) {
          try { await writePublicSummary(process.env.APPARENT_SYNTHETIC_DIAGNOSTICS_DIR, diagnosticStderr, diagnosticIdentity, process.env) } catch { process.stderr.write('packaged-h264 public summary unavailable\n') }
        }
        try { await retainSyntheticFailure(root, diagnosticStderr, diagnosticIdentity) } catch { process.stderr.write('packaged-h264 private evidence retention failed\n') }
        const details = diagnosticStderr.toString('utf8').split('\n').filter(line => line.startsWith('apparent-media-diagnostic ') && line.length <= 256).join('\n').slice(0, 4096)
        await writeFile(join(root, 'media-diagnostics.log'), details, { mode: 0o600 })
        if (details) process.stderr.write(details + '\n')
        console.error(`packaged-h264 synthetic runtime retained: ${root}`)
      }
    }
  }
})

test('synthetic stderr retention preserves bounded raw abort bytes and source/request identity privately',async t=>{const root=await mkdtemp(join(tmpdir(),'synthetic-retention-'));await chmod(root,0o700);t.after(()=>rm(root,{recursive:true,force:true}));const raw=Buffer.concat([Buffer.alloc(16000,120),Buffer.from('allocator: synthetic failure\n\xc3\xa9','latin1')]);const identity={entrySha256:'a'.repeat(64),manifestSha256:'b'.repeat(64),artifactSourceSha:'synthetic',pid:123,exitCode:null,signal:'SIGABRT',request:{jobId:'synthetic'}};await retainSyntheticFailure(root,raw,identity);assert.deepEqual(await readFile(join(root,'media-stderr.raw')),raw.subarray(-8192));for(const f of ['media-stderr.raw','media-failure-identity.json'])assert.equal((await stat(join(root,f))).mode&0o777,0o600);const receipt=JSON.parse(await readFile(join(root,'media-failure-identity.json'),'utf8'));assert.equal(receipt.signal,'SIGABRT');assert.deepEqual(receipt.request,identity.request);assert.equal(receipt.stderrBytes,8192)})
test('synthetic stderr retention rejects nonprivate and symlink roots without writing',async t=>{const root=await mkdtemp(join(tmpdir(),'synthetic-retention-'));t.after(()=>rm(root,{recursive:true,force:true}));await chmod(root,0o755);await assert.rejects(retainSyntheticFailure(root,Buffer.from('x'),{}),/not private/);await chmod(root,0o700);const link=root+'-link';await symlink(root,link);t.after(()=>unlink(link));await assert.rejects(retainSyntheticFailure(link,Buffer.from('x'),{}),/not private/);await assert.rejects(stat(join(root,'media-stderr.raw')),{code:'ENOENT'})})
test('synthetic stderr retention never replaces prior failure evidence',async t=>{const root=await mkdtemp(join(tmpdir(),'synthetic-retention-'));await chmod(root,0o700);t.after(()=>rm(root,{recursive:true,force:true}));await retainSyntheticFailure(root,Buffer.from('first'),{});await assert.rejects(retainSyntheticFailure(root,Buffer.from('second'),{}),{code:'EEXIST'});assert.equal(await readFile(join(root,'media-stderr.raw'),'utf8'),'first')})
