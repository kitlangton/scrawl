import { createHash } from 'node:crypto'
import { constants, lstatSync, mkdirSync, openSync, closeSync, fstatSync, readFileSync, writeFileSync } from 'node:fs'
import { join, resolve } from 'node:path'

// Public artifacts: emit only finite signatures, numbers and validated hashes.
// A matching signature is an observation, never a diagnosis or native proof.
const signatures = [
  ['allocation-failure', /cannot allocate memory|out of memory|failed to allocate|bad_alloc/i],
  ['allocator-corruption', /double free|corrupted (?:size|double-linked)|malloc\(\):|free\(\):/i],
  ['thread-creation-failure', /pthread_create.*(?:failed|error)|(?:failed|unable) to create (?:a )?thread|resource temporarily unavailable/i],
  ['assertion-failure', /assertion .*failed|assertion failed|assertion.*should not be reached/i],
  ['plugin-failure', /failed to load plugin|no (?:such )?(?:element|plugin)|missing plugin/i],
  ['loader-failure', /error while loading shared libraries|cannot open shared object file|undefined symbol/i],
] as const
const sha = /^[a-f0-9]{40}$/; const digest = /^[a-f0-9]{64}$/
const hash = (bytes: Buffer) => createHash('sha256').update(bytes).digest('hex')
const number = (x: unknown, max: number) => Number.isSafeInteger(x) && Number(x) >= 0 && Number(x) <= max
function binding(env: NodeJS.ProcessEnv) {
  if (!sha.test(env.GITHUB_SHA ?? '') || !sha.test(env.GITHUB_WORKFLOW_SHA ?? '') || !/^[1-9][0-9]{0,19}$/.test(env.GITHUB_RUN_ID ?? '') || !/^[1-9][0-9]{0,5}$/.test(env.GITHUB_RUN_ATTEMPT ?? '') || !['verify-pr', 'stream'].includes(env.GITHUB_JOB ?? '') || env.OWNER_ID !== 'engine-media-native' || env.OWNER_TARGET !== 'linux-x64') throw new Error('invalid synthetic diagnostic binding')
  return { sourceSha: env.GITHUB_SHA!, workflowSha: env.GITHUB_WORKFLOW_SHA!, runId: env.GITHUB_RUN_ID!, attempt: env.GITHUB_RUN_ATTEMPT!, job: env.GITHUB_JOB!, owner: 'engine-media-native', target: 'linux-x64' }
}
export function publicSummary(stderr: Buffer, identity: Record<string, unknown>, env: NodeJS.ProcessEnv) {
  if (env.APPARENT_MEDIA_FIXTURE || identity.generated !== true) throw new Error('public diagnostics require generated fixture')
  const bound = binding(env)
  if (!digest.test(String(identity.entrySha256)) || !digest.test(String(identity.manifestSha256)) || identity.artifactSourceSha !== bound.sourceSha) throw new Error('invalid synthetic artifact binding')
  const tail = stderr.subarray(-8192).toString('utf8')
  const categories = signatures.filter(([, pattern]) => pattern.test(tail)).map(([name]) => name)
  const exits = [...tail.matchAll(/^apparent-media-diagnostic stage=worker-exit exit-code=(none|[0-9]{1,3}) signal=(none|[0-9]{1,2})$/gm)]
  const exit = exits.at(-1)
  return { schema: 'apparent.synthetic-media-public/1', ...bound, entrySha256: identity.entrySha256, manifestSha256: identity.manifestSha256,
    stderrBytes: Math.min(stderr.length, 8192), categories: categories.length ? categories : ['unclassified'],
    workerExitCode: exit && exit[1] !== 'none' && number(Number(exit[1]), 255) ? Number(exit[1]) : null,
    workerSignal: exit && exit[2] !== 'none' && number(Number(exit[2]), 64) ? Number(exit[2]) : null,
    // These are the fixture request limits, not measured process/resource limits.
    requestLimits: { maxFrames: 48, maxEdge: 320, maxTotalPixels: 64000000, deadlineMs: 30000 },
    testCompleted: identity.testCompleted === true, clientCloseReturned: identity.clientCloseReturned === true }
}
function privateDirectory(path: string) {
  const s = lstatSync(path)
  if (!s.isDirectory() || s.isSymbolicLink() || s.uid !== process.getuid?.() || (s.mode & 0o777) !== 0o700) throw new Error('unsafe synthetic diagnostic directory')
}
function privateFile(path: string) {
  const fd = openSync(path, constants.O_RDONLY | constants.O_NOFOLLOW)
  try { const s = fstatSync(fd); if (!s.isFile() || s.nlink !== 1 || s.uid !== process.getuid?.() || (s.mode & 0o777) !== 0o600 || s.size > 8192) throw new Error('unsafe synthetic diagnostic file'); return readFileSync(fd) } finally { closeSync(fd) }
}
export function writePublicSummary(directory: string, stderr: Buffer, identity: Record<string, unknown>, env: NodeJS.ProcessEnv) {
  const summary = publicSummary(stderr, identity, env)
  mkdirSync(directory, { mode: 0o700 }); privateDirectory(directory)
  writeFileSync(join(directory, 'summary.json'), JSON.stringify(summary) + '\n', { mode: 0o600, flag: 'wx' })
}
export function stagePublicSummary(input: string, output: string, artifact: string, env: NodeJS.ProcessEnv) {
  binding(env)
  if (env.APPARENT_MEDIA_FIXTURE) throw new Error('external fixture diagnostics cannot be exported')
  // Missing evidence is honest absence; malformed or stale evidence fails closed.
  try { lstatSync(input) } catch (e) { if ((e as NodeJS.ErrnoException).code === 'ENOENT') return false; throw e }
  privateDirectory(input)
  const value = JSON.parse(privateFile(join(input, 'summary.json')).toString('utf8'))
  const expected = publicSummary(Buffer.alloc(0), { generated: true, entrySha256: value.entrySha256, manifestSha256: value.manifestSha256, artifactSourceSha: value.sourceSha, testCompleted: value.testCompleted, clientCloseReturned: value.clientCloseReturned }, env)
  const keys = Object.keys(expected).sort()
  if (JSON.stringify(Object.keys(value).sort()) !== JSON.stringify(keys)) throw new Error('unknown public diagnostic fields')
  for (const [key, field] of Object.entries(binding(env))) if (value[key] !== field) throw new Error('stale public diagnostic binding')
  if (!number(value.stderrBytes, 8192) || !Array.isArray(value.categories) || value.categories.length < 1 || value.categories.length > signatures.length || new Set(value.categories).size !== value.categories.length || value.categories.some((x: unknown) => ![...signatures.map(([name]) => name), 'unclassified'].includes(x as never)) || ![value.testCompleted, value.clientCloseReturned].every(x => typeof x === 'boolean') || !(value.workerExitCode === null || number(value.workerExitCode, 255)) || !(value.workerSignal === null || number(value.workerSignal, 64)) || JSON.stringify(value.requestLimits) !== JSON.stringify(expected.requestLimits) || value.schema !== expected.schema) throw new Error('invalid public diagnostic values')
  const manifest = readFileSync(join(artifact, 'manifest.json'))
  if (hash(manifest) !== value.manifestSha256 || JSON.parse(manifest.toString('utf8')).sourceSha !== value.sourceSha || hash(readFileSync(join(artifact, 'apparent-engine-frames'))) !== value.entrySha256) throw new Error('changed public diagnostic artifact')
  mkdirSync(output, { mode: 0o700 }); privateDirectory(output)
  // Reconstruct finite fields rather than copying any untrusted receipt bytes.
  const safe = { ...expected, stderrBytes: value.stderrBytes, categories: value.categories, workerExitCode: value.workerExitCode, workerSignal: value.workerSignal }
  writeFileSync(join(output, 'summary.json'), JSON.stringify(safe) + '\n', { mode: 0o600, flag: 'wx' })
  return true
}
// The action invokes this fixed repository-relative entry from the checkout root.
// Keep import.meta out of this module: NodeNext consumers also typecheck it as CJS.
if (process.argv[1] && resolve(process.argv[1]) === resolve('composition/delivery/runtime/media-diagnostics.ts')) {
  try {
    const [input, output, artifact] = process.argv.slice(2)
    if (!input || !output || !artifact || process.argv.length !== 5) throw new Error('invalid invocation')
    const found = stagePublicSummary(input, output, artifact, process.env)
    if (process.env.GITHUB_OUTPUT) writeFileSync(process.env.GITHUB_OUTPUT, `found=${found}\n`, { flag: 'a' })
  } catch { console.error('Synthetic public diagnostic staging refused'); process.exitCode = 1 }
}
