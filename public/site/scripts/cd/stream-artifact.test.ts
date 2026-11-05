import test from 'node:test'
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { gzipSync } from 'node:zlib'
import { existsSync, mkdtempSync, mkdirSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { zipSync } from 'fflate'
import { consume, consumeCurrent, decodePayloadTar, trustedRun, validateArtifact, validateJobs, workflowPath } from './stream-artifact.ts'

const sha = 'a'.repeat(40), sourceDigest = 'b'.repeat(64), environmentDigest = 'c'.repeat(64), inputDigest = 'd'.repeat(64)
const hash = (data: Uint8Array) => createHash('sha256').update(data).digest('hex')
function tar(entries: Array<{ path: string; data?: string; type?: string }>) {
  const chunks: Buffer[] = []
  for (const entry of entries) {
    const data = Buffer.from(entry.data ?? ''), header = Buffer.alloc(512)
    const write = (value: string, offset: number, length: number) => header.write(value, offset, length, 'utf8')
    const octal = (value: number, offset: number, length: number) => write(value.toString(8).padStart(length - 1, '0') + '\0', offset, length)
    let name = entry.path, prefix = ''
    if (Buffer.byteLength(name) > 100) {
      const slash = name.lastIndexOf('/')
      if (slash < 1 || Buffer.byteLength(name.slice(slash + 1)) > 100 || Buffer.byteLength(name.slice(0, slash)) > 155) throw new Error('Test tar path does not fit USTAR')
      prefix = name.slice(0, slash); name = name.slice(slash + 1)
    }
    write(name, 0, 100); if (prefix) write(prefix, 345, 155); octal(0o644, 100, 8); octal(0, 108, 8); octal(0, 116, 8)
    octal(data.length, 124, 12); octal(0, 136, 12); header.fill(32, 148, 156); write(entry.type ?? '0', 156, 1)
    write('ustar\0', 257, 6); write('00', 263, 2)
    let sum = 0; for (const byte of header) sum += byte
    write(sum.toString(8).padStart(6, '0') + '\0 ', 148, 8)
    chunks.push(header, data, Buffer.alloc((512 - data.length % 512) % 512))
  }
  chunks.push(Buffer.alloc(1024))
  return gzipSync(Buffer.concat(chunks))
}
function componentTar() {
  return tar([
    { path: 'stream-payload/', type: '5' }, { path: 'stream-payload/payload/', type: '5' },
    { path: 'stream-payload/payload/index.html', data: '<main>web bytes</main>' },
    { path: 'stream-payload/payload/assets/', type: '5' },
    { path: 'stream-payload/payload/assets/site.css', data: 'body{color:#123}' },
    { path: `stream-payload/payload/assets/${'long-'.repeat(16)}asset.css`, data: 'long USTAR path bytes' },
  ])
}
function fixture(runOverrides: Record<string, unknown> = {}) {
  const run: any = { id: 91, run_attempt: 2, status: 'completed', conclusion: 'failure', event: 'push', path: workflowPath, head_branch: 'main', head_sha: sha, repository: { full_name: 'tryapparent/apparent' }, head_repository: { full_name: 'tryapparent/apparent' }, ...runOverrides }
  const jobs = ['deliver / stream (public, web)', 'deliver / Delivery contracts', 'deliver / Workflow runtime budget'].map(name => ({ name, status: 'completed', conclusion: 'success', run_id: run.id, run_attempt: run.run_attempt }))
  const tarBytes = componentTar()
  const receipt = { schema: 1, id: 'public', target: 'web', stage: 'website', sourceSha: run.head_sha, inputDigest, sourceDigest, environmentDigest, sha256: hash(tarBytes), size: tarBytes.length, runId: String(run.id), runAttempt: String(run.run_attempt), reusedFrom: { artifactId: 77, sourceSha: '9'.repeat(40), runId: 80, runAttempt: 1 } }
  const bytes = Buffer.from(zipSync({ 'component.tar.gz': tarBytes, 'receipt.json': Buffer.from(JSON.stringify(receipt)) }))
  const artifact = { id: 123, name: `stream-public-web-${inputDigest}`, expired: false, digest: `sha256:${hash(bytes)}`, workflow_run: { id: run.id, head_sha: run.head_sha } }
  const calls: string[] = []
  const api = async (path: string) => {
    calls.push(path)
    if (path === '/actions/workflows/stream-public.yml/runs?branch=main&status=completed&per_page=100') return { workflow_runs: [run] }
    if (path.includes('/jobs?')) return { total_count: jobs.length, jobs }
    if (path.includes('/artifacts?')) return { total_count: 1, artifacts: [artifact] }
    throw new Error(`Unexpected API path: ${path}`)
  }
  const owner = { id: 'public', kind: 'directory', artifactPath: 'dist-apparent', targets: ['web'], stage: 'website' }
  const deps = { owner, ancestor: () => true, sourceDigest: () => sourceDigest, environmentDigest: () => environmentDigest }
  return { run, jobs, tarBytes, receipt, bytes, artifact, calls, api, owner, deps }
}
async function withRoot<T>(run: (root: string) => Promise<T> | T): Promise<T> {
  const root = mkdtempSync(join(tmpdir(), 'public-stream-consumer-'))
  try { return await run(root) } finally { rmSync(root, { recursive: true, force: true }) }
}

test('trust boundary accepts canonical ancestor run even if unrelated matrix work failed', () => {
  const f = fixture()
  assert.equal(trustedRun(f.run, () => true), true)
  assert.equal(trustedRun({ ...f.run, event: 'pull_request' }, () => true), false)
  validateJobs(f.jobs, f.run)
  assert.throws(() => validateJobs([...f.jobs, f.jobs[0]], f.run), /exactly once/)
  assert.throws(() => validateJobs(f.jobs.map(job => job.name === 'deliver / Delivery contracts' ? { ...job, conclusion: 'failure' } : job), f.run), /did not succeed/)
  assert.throws(() => validateJobs(f.jobs.map(job => job.name.replace('deliver / ', '')), f.run), /did not succeed/)
  assert.equal(validateArtifact(f.artifact, f.run, inputDigest), hash(f.bytes))
  assert.throws(() => validateArtifact({ ...f.artifact, expired: true }, f.run, inputDigest), /provenance/)
})

test('consumer extracts raw package bytes and stores provenance outside the served tree', () => withRoot(async root => {
  const f = fixture(), out = join(root, '.build/public-artifact')
  const result = await consume(root, out, f.api, async () => f.bytes, f.deps)
  assert.equal(result.sourceSha, sha)
  assert.equal(readFileSync(join(out, 'index.html'), 'utf8'), '<main>web bytes</main>')
  assert.equal(readFileSync(join(out, 'assets/site.css'), 'utf8'), 'body{color:#123}')
  assert.equal(readFileSync(join(out, 'assets', `${'long-'.repeat(16)}asset.css`), 'utf8'), 'long USTAR path bytes')
  assert.deepEqual(readdirSync(out).sort(), ['assets', 'index.html'])
  assert.equal(existsSync(join(out, 'receipt.json')), false)
  const savedReceipt = JSON.parse(readFileSync(`${out}-receipt.json`, 'utf8'))
  assert.equal(savedReceipt.artifactId, 123)
  assert.equal(savedReceipt.workflowRunId, f.run.id)
  assert.equal(savedReceipt.reusedFrom.artifactId, 77)
  assert.equal(f.calls.length, 3)
  await assert.rejects(consume(root, out, f.api, async () => f.bytes, f.deps), /must not already exist/)
}))

test('default consumer binds the registered Public owner and shared source/environment digests', () => withRoot(async tempRoot => {
  const root = resolve(import.meta.dirname, '../../../..')
  const core = await import('../../../../composition/delivery/core.ts')
  const owner = core.catalog(root).owners.find(candidate => candidate.id === 'public')!
  assert.ok(owner)
  const head = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim()
  const f = fixture({ head_sha: head })
  f.receipt.sourceDigest = core.sourceDigest(root, owner, head)
  f.receipt.environmentDigest = core.environmentDigest(owner, process.env)
  f.bytes = Buffer.from(zipSync({ 'component.tar.gz': f.tarBytes, 'receipt.json': Buffer.from(JSON.stringify(f.receipt)) }))
  f.artifact.digest = `sha256:${hash(f.bytes)}`
  const out = join(tempRoot, 'registered-public')
  const result = await consume(root, out, f.api, async () => f.bytes, { ancestor: () => true })
  assert.equal(result.sourceSha, head)
  assert.equal(readFileSync(join(out, 'index.html'), 'utf8'), '<main>web bytes</main>')
}))

test('same-run consumer verifies receipt identity and extracts the exact component payload', () => withRoot(async tempRoot => {
  const root = resolve(import.meta.dirname, '../../../..'), core = await import('../../../../composition/delivery/core.ts')
  const owner = core.catalog(root).owners.find(candidate => candidate.id === 'public')!
  const head = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim()
  const env = { ...process.env, GITHUB_SHA: head, GITHUB_RUN_ID: '91', GITHUB_RUN_ATTEMPT: '2', GITHUB_REPOSITORY: 'tryapparent/apparent', GITHUB_REF: 'refs/heads/main', GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/stream-public.yml@refs/heads/main', PUBLIC_COMPONENT_INPUT_DIGEST: inputDigest, ImageOS: 'ubuntu24', ImageVersion: 'consumer-image-does-not-matter' }
  const input = inputDigest
  const source = core.sourceDigest(root, owner, head), environment = core.environmentDigest(owner, env), tarBytes = componentTar()
  const receipt = { schema: 1, id: 'public', target: 'web', stage: 'website', sourceSha: head, inputDigest: input, sourceDigest: source, environmentDigest: environment, sha256: hash(tarBytes), size: tarBytes.length, runId: '91', runAttempt: '2' }
  const component = join(tempRoot, 'component'); mkdirSync(component)
  writeFileSync(join(component, 'receipt.json'), JSON.stringify(receipt)); writeFileSync(join(component, 'component.tar.gz'), tarBytes)
  const out = join(tempRoot, 'out')
  await consumeCurrent(root, component, out, {}, env)
  assert.equal(readFileSync(join(out, 'index.html'), 'utf8'), '<main>web bytes</main>')
  assert.equal(readFileSync(join(out, 'assets/site.css'), 'utf8'), 'body{color:#123}')
  assert.deepEqual(readdirSync(out).sort(), ['assets', 'index.html'])
}))

test('same-run consumer rejects receipt path, payload hash, workflow identity and environment mismatches without touching output', () => withRoot(async tempRoot => {
  const root = resolve(import.meta.dirname, '../../../..'), core = await import('../../../../composition/delivery/core.ts')
  const owner = core.catalog(root).owners.find(candidate => candidate.id === 'public')!, head = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim()
  const env = { ...process.env, GITHUB_SHA: head, GITHUB_RUN_ID: '91', GITHUB_RUN_ATTEMPT: '2', GITHUB_REPOSITORY: 'tryapparent/apparent', GITHUB_REF: 'refs/heads/main', GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/stream-public.yml@refs/heads/main', PUBLIC_COMPONENT_INPUT_DIGEST: inputDigest, ImageOS: 'ubuntu24', ImageVersion: 'consumer-image-does-not-matter' }
  const input = inputDigest, tarBytes = componentTar()
  const base = { schema: 1, id: 'public', target: 'web', stage: 'website', sourceSha: head, inputDigest: input, sourceDigest: core.sourceDigest(root, owner, head), environmentDigest: core.environmentDigest(owner, env), sha256: hash(tarBytes), size: tarBytes.length, runId: '91', runAttempt: '2' }
  const component = join(tempRoot, 'component'); mkdirSync(component); writeFileSync(join(component, 'component.tar.gz'), tarBytes)
  const out = join(tempRoot, 'out'); mkdirSync(out); writeFileSync(join(out, 'keep'), 'preserve')
  const deps = {}
  for (const receipt of [
    { ...base, sourceSha: 'e'.repeat(40) }, { ...base, runAttempt: '1' }, { ...base, inputDigest: 'e'.repeat(64) }, { ...base, environmentDigest: 'f'.repeat(64) }, { ...base, sha256: 'f'.repeat(64) },
  ]) {
    writeFileSync(join(component, 'receipt.json'), JSON.stringify(receipt))
    await assert.rejects(consumeCurrent(root, component, out, deps, env))
    assert.equal(readFileSync(join(out, 'keep'), 'utf8'), 'preserve')
  }
  await assert.rejects(consumeCurrent(root, component, join(tempRoot, 'missing-digest'), deps, { ...env, PUBLIC_COMPONENT_INPUT_DIGEST: '' }), /Invalid current Public input identity/)
  await assert.rejects(consumeCurrent(root, component, join(tempRoot, 'wrong-digest'), deps, { ...env, PUBLIC_COMPONENT_INPUT_DIGEST: 'f'.repeat(64) }), /receipt provenance or payload digest mismatch/)
  writeFileSync(join(component, 'receipt.json'), JSON.stringify(base)); writeFileSync(join(component, 'extra'), 'unexpected')
  await assert.rejects(consumeCurrent(root, component, out, deps, env), /directory contents/)
}))

test('ancestor reuse is admitted only when candidate source, receipt source and environment identities match', () => withRoot(async root => {
  const oldSha = '9'.repeat(40), f = fixture({ head_sha: oldSha }), out = join(root, 'ancestor')
  const digests = (_root: string, _owner: any, revision = 'HEAD') => revision === 'HEAD' || revision === oldSha ? sourceDigest : 'e'.repeat(64)
  const result = await consume(root, out, f.api, async () => f.bytes, { ...f.deps, sourceDigest: digests, ancestor: candidate => candidate === oldSha })
  assert.equal(result.sourceSha, oldSha)
  assert.equal(readFileSync(join(out, 'index.html'), 'utf8'), '<main>web bytes</main>')
  await assert.rejects(consume(root, join(root, 'mismatch'), f.api, async () => f.bytes, { ...f.deps, sourceDigest: (_r: string, _o: any, rev = 'HEAD') => rev === 'HEAD' ? sourceDigest : 'e'.repeat(64) }), /No successful trusted main Public artifact/)
  assert.equal(existsSync(join(root, 'mismatch')), false)
}))

test('changed source/config, missing or failed jobs, and artifact tampering do not publish output', () => withRoot(async root => {
  const out = join(root, 'published')
  const f = fixture()
  await assert.rejects(consume(root, out, f.api, async () => f.bytes, { ...f.deps, sourceDigest: () => 'e'.repeat(64) }), /No successful trusted main Public artifact/)
  await assert.rejects(consume(root, out, f.api, async () => f.bytes, { ...f.deps, environmentDigest: () => 'f'.repeat(64) }), /No successful trusted main Public artifact/)
  await assert.rejects(consume(root, out, async path => path.includes('/jobs?') ? { total_count: f.jobs.length + 1, jobs: f.jobs } : f.api(path), async () => f.bytes, f.deps), /No successful trusted main Public artifact/)
  const failedJobs = async (path: string) => path.includes('/jobs?') ? { jobs: f.jobs.filter(job => job.name !== 'deliver / Delivery contracts') } : f.api(path)
  await assert.rejects(consume(root, out, failedJobs, async () => f.bytes, f.deps), /No successful trusted main Public artifact/)
  await assert.rejects(consume(root, out, async path => path.includes('/runs?') ? { workflow_runs: [] } : f.api(path), async () => f.bytes, f.deps), /No successful trusted main Public artifact/)
  await assert.rejects(consume(root, out, f.api, async () => Buffer.from('tampered'), f.deps), /ZIP digest mismatch/)
  const badReceipt = { ...f.receipt, sourceSha: '9'.repeat(40) }
  const badZip = Buffer.from(zipSync({ 'component.tar.gz': f.tarBytes, 'receipt.json': Buffer.from(JSON.stringify(badReceipt)) }))
  const badArtifact = { ...f.artifact, digest: `sha256:${hash(badZip)}` }
  const badApi = async (path: string) => path.includes('/artifacts?') ? { total_count: 1, artifacts: [badArtifact] } : f.api(path)
  await assert.rejects(consume(root, out, badApi, async () => badZip, f.deps), /No successful trusted main Public artifact/)
  assert.equal(existsSync(out), false)
  assert.equal(existsSync(`${out}-receipt.json`), false)
}))

test('tar parser rejects unsafe paths, duplicate entries, links and special files before extraction', () => {
  for (const archive of [
    tar([{ path: 'stream-payload/payload/../escape', data: 'bad' }]),
    tar([{ path: 'stream-payload/payload/a', data: 'one' }, { path: 'stream-payload/payload/a', data: 'two' }]),
    tar([{ path: 'stream-payload/payload/link', type: '2' }]),
    tar([{ path: 'stream-payload/payload/device', type: '3' }]),
    tar([{ path: 'unexpected/payload/index.html', data: 'bad' }]),
  ]) assert.throws(() => decodePayloadTar(archive))
})

test('tar decoder accepts the producer system tar format', () => withRoot(root => {
  const payload = join(root, 'stream-payload/payload')
  mkdirSync(join(payload, 'assets'), { recursive: true })
  writeFileSync(join(payload, 'index.html'), 'producer html')
  writeFileSync(join(payload, 'assets/app.js'), 'producer js')
  const longName = `${'long-'.repeat(16)}asset.css`
  writeFileSync(join(payload, 'assets', longName), 'ustar long path')
  const archive = join(root, 'component.tar.gz')
  execFileSync('tar', ['--format=ustar', '-czf', archive, '-C', root, 'stream-payload'])
  const entries = decodePayloadTar(readFileSync(archive))
  assert.deepEqual(entries.filter(entry => entry.kind === 'file').map(entry => entry.path).sort(), [
    'stream-payload/payload/assets/app.js', `stream-payload/payload/assets/${longName}`, 'stream-payload/payload/index.html',
  ])
}))

test('ZIP outer envelope rejects link entries even when names are otherwise valid', () => withRoot(async root => {
  const f = fixture()
  const symlinkZip = Buffer.from(zipSync({
    'component.tar.gz': [f.tarBytes, { attrs: 0o120777 << 16 }],
    'receipt.json': Buffer.from(JSON.stringify(f.receipt)),
  }))
  const badArtifact = { ...f.artifact, digest: `sha256:${hash(symlinkZip)}` }
  const api = async (path: string) => path.includes('/artifacts?') ? { total_count: 1, artifacts: [badArtifact] } : f.api(path)
  await assert.rejects(consume(root, join(root, 'out'), api, async () => symlinkZip, f.deps), /regular files/)
}))

test('invalid payload archive and invalid source receipt preserve an existing destination', () => withRoot(async root => {
  const out = join(root, 'existing')
  mkdirSync(out); writeFileSync(join(out, 'keep.txt'), 'leave me')
  const f = fixture()
  await assert.rejects(consume(root, out, f.api, async () => f.bytes, f.deps), /must not already exist/)
  assert.equal(readFileSync(join(out, 'keep.txt'), 'utf8'), 'leave me')
  const malformed = fixture(), badReceiptZip = Buffer.from(zipSync({ 'component.tar.gz': malformed.tarBytes, 'receipt.json': Buffer.from('{') }))
  await assert.rejects(consume(root, join(root, 'other'), malformed.api, async () => badReceiptZip, malformed.deps), /ZIP digest mismatch/)
}))
