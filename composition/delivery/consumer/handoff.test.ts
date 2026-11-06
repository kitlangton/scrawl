import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { handoff, matchesInvocation } from './handoff.ts'
import { sourceDigest } from '../core.ts'

const root = execFileSync('git', ['rev-parse', '--show-toplevel'], { encoding: 'utf8' }).trim()
const sha = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim()
const owner = { schema: 1 as const, id: 'engine', kind: 'npm' as const, package: 'engine/sdk', inputs: ['engine/'], verify: [], build: 'build', targets: ['linux-x64' as const], stage: 'contract' as const, language: 'typescript' as const }
const digest = 'a'.repeat(64)
const source = sourceDigest(root, owner, sha)
const run = { id: 42, run_attempt: 2, head_sha: sha, head_branch: 'main', event: 'push', conclusion: 'success', path: '.github/workflows/stream-engine.yml' }

test('attestation invocation binds the exact repository, run and attempt', () => {
  const expected = 'https://github.com/tryapparent/apparent/actions/runs/42/attempts/2'
  assert.equal(matchesInvocation(expected, 42, 2), true)
  for (const value of [expected + '0', expected + '/extra', expected + '?other=1', expected + '#x',
    expected.replace('/42/', '/420/'), expected.replace('tryapparent/apparent', 'other/repo'),
    expected.replace('github.com', 'github.com.attacker.invalid'), undefined, 'not a URL']) {
    assert.equal(matchesInvocation(value, 42, 2), false, String(value))
  }
})

function artifact(contents: string, malformedTar = false) {
  const dir = mkdtempSync(join(tmpdir(), 'delivery-handoff-'))
  try {
    writeFileSync(join(dir, 'file.txt'), contents)
    const archive = join(dir, 'component.tar.gz')
    execFileSync('tar', ['-czf', archive, '-C', dir, 'file.txt'])
    const bytes = malformedTar ? Buffer.from('not a gzip tar archive') : readFileSync(archive)
    if (malformedTar) writeFileSync(archive, bytes)
    const receipt = { schema: 1, id: owner.id, target: 'linux-x64', stage: owner.stage, sourceSha: sha, sourceDigest: source, inputDigest: digest, sha256: createHash('sha256').update(bytes).digest('hex'), size: bytes.length, runId: String(run.id), runAttempt: String(run.run_attempt) }
    writeFileSync(join(dir, 'receipt.json'), JSON.stringify(receipt))
    const zip = join(dir, 'artifact.zip')
    execFileSync('zip', ['-q', zip, 'component.tar.gz', 'receipt.json'], { cwd: dir })
    return { bytes: readFileSync(zip), receipt, dir }
  } catch (error) { rmSync(dir, { recursive: true, force: true }); throw error }
}

function api(bytes: Buffer, onDownload: () => void = () => {}, onAttest: () => void = () => {}) {
  return {
    runs: async () => [run], jobs: async () => [{ name: 'deliver / stream (engine, linux-x64)', status: 'completed', conclusion: 'success' }, { name: 'deliver / Delivery contracts', status: 'completed', conclusion: 'success' }, { name: 'deliver / Workflow runtime budget', status: 'completed', conclusion: 'success' }],
    artifacts: async () => [{ id: 7, name: `stream-engine-linux-x64-${digest}`, expired: false }],
    download: async () => { onDownload(); return bytes }, attest: async () => { onAttest() },
  }
}
const admit = (runs: any[], jobsFor: (run: any) => any[], artifactsFor: (run: any) => any[]) => {
  const candidate = runs.find(item => item.head_sha === sha)
  if (!candidate || candidate.path !== '.github/workflows/stream-engine.yml' || candidate.conclusion !== 'success' || candidate.event !== 'push' || candidate.head_branch !== 'main') return { reuse: false, reason: 'exact main producer failed or workflow identity mismatched', input_digest: digest, prior_artifact_id: null, prior_source_sha: null, prior_run_id: null, prior_run_attempt: null }
  const jobs = jobsFor(candidate)
  if (jobs.length !== 3 || jobs.some(job => job.status !== 'completed' || job.conclusion !== 'success')) return { reuse: false, reason: 'required producer jobs failed or are missing', input_digest: digest, prior_artifact_id: null, prior_source_sha: null, prior_run_id: null, prior_run_attempt: null }
  const item = artifactsFor(candidate).find(value => value.id === 7 && !value.expired && value.name === `stream-engine-linux-x64-${digest}`)
  return item ? { reuse: true, reason: 'eligible exact artifact', input_digest: digest, prior_artifact_id: item.id, prior_source_sha: sha, prior_run_id: run.id, prior_run_attempt: run.run_attempt } : { reuse: false, reason: 'artifact missing, incompatible, or expired', input_digest: digest, prior_artifact_id: null, prior_source_sha: null, prior_run_id: null, prior_run_attempt: null }
}

test('cold handoff downloads once, verifies bytes and warm repeat reuses the private cache', async () => {
  const fixture = artifact('verified component')
  const cache = mkdtempSync(join(tmpdir(), 'apparent-cache-'))
  const previous = process.env.XDG_CACHE_HOME
  process.env.XDG_CACHE_HOME = cache
  let downloads = 0, attestations = 0
  try {
    const first = await handoff(root, owner, 'linux-x64', sha, api(fixture.bytes, () => downloads++, () => attestations++), (runs, jobs, artifacts) => admit(runs, jobs, artifacts))
    const second = await handoff(root, owner, 'linux-x64', sha, api(fixture.bytes, () => downloads++, () => attestations++), (runs, jobs, artifacts) => admit(runs, jobs, artifacts))
    assert.equal(downloads, 1)
    assert.equal(first.cacheHit, false)
    assert.equal(second.cacheHit, true)
    assert.equal(first.cacheKey, second.cacheKey)
    assert.equal(readFileSync(second.componentPath).length, fixture.receipt.size)
    assert.equal(attestations, 2) // Cached bytes still require fresh online attestation verification.
  } finally { if (previous === undefined) delete process.env.XDG_CACHE_HOME; else process.env.XDG_CACHE_HOME = previous; rmSync(cache, { recursive: true, force: true }); rmSync(fixture.dir, { recursive: true, force: true }) }
})

test('rejects bad receipt bytes, failed producer jobs, expired artifacts and wrong targets', async () => {
  const fixture = artifact('verified component')
  const cache = mkdtempSync(join(tmpdir(), 'apparent-cache-'))
  const previous = process.env.XDG_CACHE_HOME; process.env.XDG_CACHE_HOME = cache
  let malformedDir = ''
  try {
    const malformed = artifact('invalid archive', true)
    malformedDir = malformed.dir
    await assert.rejects(() => handoff(root, owner, 'linux-x64', sha, api(malformed.bytes), (runs, jobs, artifacts) => admit(runs, jobs, artifacts)))
    await assert.rejects(() => handoff(root, owner, 'linux-x64', sha, { ...api(fixture.bytes), jobs: async () => [{ name: 'failed', status: 'completed', conclusion: 'failure' }] }, (runs, jobs, artifacts) => admit(runs, jobs, artifacts)), /failed or are missing/)
    await assert.rejects(() => handoff(root, owner, 'linux-x64', sha, { ...api(fixture.bytes), artifacts: async () => [{ id: 7, name: `stream-engine-linux-x64-${digest}`, expired: true }] }, (runs, jobs, artifacts) => admit(runs, jobs, artifacts)), /expired/)
    for (const mutation of [{ head_sha: 'f'.repeat(40) }, { path: '.github/workflows/other.yml' }, { head_branch: 'other' }]) {
      await assert.rejects(() => handoff(root, owner, 'linux-x64', sha, { ...api(fixture.bytes), runs: async () => [{ ...run, ...mutation }] }, (runs, jobs, artifacts) => admit(runs, jobs, artifacts)), /failed|mismatched|No single unexpired/)
    }
    await assert.rejects(() => handoff(root, owner, 'macos-x64', sha, api(fixture.bytes), (runs, jobs, artifacts) => admit(runs, jobs, artifacts)), /does not produce target/)
  } finally { if (previous === undefined) delete process.env.XDG_CACHE_HOME; else process.env.XDG_CACHE_HOME = previous; rmSync(cache, { recursive: true, force: true }); rmSync(fixture.dir, { recursive: true, force: true }); if (malformedDir) rmSync(malformedDir, { recursive: true, force: true }) }
})

test('rejects path traversal and symlink entries in component tar archives', async () => {
  const dir = mkdtempSync(join(tmpdir(), 'delivery-tar-'))
  const cache = mkdtempSync(join(tmpdir(), 'apparent-cache-'))
  const previous = process.env.XDG_CACHE_HOME; process.env.XDG_CACHE_HOME = cache
  try {
    writeFileSync(join(dir, 'payload'), 'bad')
    execFileSync('tar', ['-czf', join(dir, 'component.tar.gz'), '-C', dir, '--transform=s|payload|../escape|', 'payload'], { stdio: ['ignore', 'pipe', 'pipe'] })
    const traversal = readFileSync(join(dir, 'component.tar.gz'))
    const makeZip = (bytes: Buffer, name: string) => {
      writeFileSync(join(dir, 'component.tar.gz'), bytes)
      const receipt = { schema: 1, id: owner.id, target: 'linux-x64', stage: owner.stage, sourceSha: sha, sourceDigest: source, inputDigest: digest, sha256: createHash('sha256').update(bytes).digest('hex'), size: bytes.length, runId: String(run.id), runAttempt: String(run.run_attempt) }
      writeFileSync(join(dir, 'receipt.json'), JSON.stringify(receipt))
      execFileSync('zip', ['-q', `${name}.zip`, 'component.tar.gz', 'receipt.json'], { cwd: dir })
      return readFileSync(join(dir, `${name}.zip`))
    }
    const traversalZip = makeZip(traversal, 'traversal')
    await assert.rejects(() => handoff(root, owner, 'linux-x64', sha, api(traversalZip), (runs, jobs, artifacts) => admit(runs, jobs, artifacts)), /unsafe path/)
    symlinkSync('missing-target', join(dir, 'link'))
    execFileSync('tar', ['-czf', join(dir, 'component.tar.gz'), '-C', dir, 'link'], { stdio: ['ignore', 'pipe', 'pipe'] })
    const symlinkArchive = readFileSync(join(dir, 'component.tar.gz'))
    const symlinkZip = makeZip(symlinkArchive, 'symlink')
    await assert.rejects(() => handoff(root, owner, 'linux-x64', sha, api(symlinkZip), (runs, jobs, artifacts) => admit(runs, jobs, artifacts)), /links or unsupported/)
  } finally { if (previous === undefined) delete process.env.XDG_CACHE_HOME; else process.env.XDG_CACHE_HOME = previous; rmSync(cache, { recursive: true, force: true }); rmSync(dir, { recursive: true, force: true }) }
})

test('concurrent callers publish one complete immutable cache entry', async () => {
  const fixture = artifact('verified component')
  const cache = mkdtempSync(join(tmpdir(), 'apparent-cache-'))
  const previous = process.env.XDG_CACHE_HOME; process.env.XDG_CACHE_HOME = cache
  try {
    let downloads = 0
    const create = () => handoff(root, owner, 'linux-x64', sha, api(fixture.bytes, () => downloads++), (runs, jobs, artifacts) => admit(runs, jobs, artifacts))
    const [a, b] = await Promise.all([create(), create()])
    assert.equal(a.cacheKey, b.cacheKey)
    assert.equal(downloads, 1)
    assert.equal(readFileSync(a.componentPath).length, fixture.receipt.size)
  } finally { if (previous === undefined) delete process.env.XDG_CACHE_HOME; else process.env.XDG_CACHE_HOME = previous; rmSync(cache, { recursive: true, force: true }); rmSync(fixture.dir, { recursive: true, force: true }) }
})

test('corrupt cached bytes trigger one fresh download and failed fresh attestation never succeeds from stale cache', async () => {
  const fixture = artifact('verified component')
  const cache = mkdtempSync(join(tmpdir(), 'apparent-cache-'))
  const previous = process.env.XDG_CACHE_HOME; process.env.XDG_CACHE_HOME = cache
  let downloads = 0
  try {
    const make = (attest = async () => {}) => handoff(root, owner, 'linux-x64', sha, { ...api(fixture.bytes, () => downloads++), attest }, (runs, jobs, artifacts) => admit(runs, jobs, artifacts))
    const first = await make()
    writeFileSync(first.componentPath, 'corrupt')
    const repaired = await make()
    assert.equal(repaired.cacheHit, false)
    assert.equal(downloads, 2)
    writeFileSync(repaired.componentPath, 'corrupt-again')
    await assert.rejects(() => make(async () => { throw new Error('attestation service unavailable') }), /attestation service unavailable/)
    assert.equal(downloads, 3)
  } finally { if (previous === undefined) delete process.env.XDG_CACHE_HOME; else process.env.XDG_CACHE_HOME = previous; rmSync(cache, { recursive: true, force: true }); rmSync(fixture.dir, { recursive: true, force: true }) }
})
