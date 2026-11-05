import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync, symlinkSync, chmodSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import test from 'node:test'
import { assessPrior, buildOwner, catalog, environmentDigest, inputDigest, makePlan, sourceDigest, workflowPathFor } from './core.ts'
import { assertBuildWorkflowIdentity } from './cli.ts'

// Hooks must not redirect synthetic Git repositories into the caller's index.
for (const name of execFileSync('git', ['rev-parse', '--local-env-vars'], { encoding: 'utf8' }).trim().split('\n')) delete process.env[name]

const registration = { schema: 1, id: 'public', kind: 'directory', package: 'public/site', packageManager: 'bun', inputs: ['public/', 'shared/ui/'], files: ['README.md', 'capture/agent/skills/apparent/SKILL.md', '.npmrc'], environment: ['PUBLIC_POSTHOG_ENABLED', 'PUBLIC_POSTHOG_TOKEN'], verify: [], build: 'verify:installed', artifactPath: 'dist-apparent', targets: ['web'], stage: 'website' }
function fixture() {
  const root = mkdtempSync(join(tmpdir(), 'public-delivery-core-'))
  const put = (path: string, value: string) => { mkdirSync(dirname(join(root, path)), { recursive: true }); writeFileSync(join(root, path), value) }
  for (const [path, value] of Object.entries({ 'public/delivery.json': JSON.stringify(registration), 'public/site/source.ts': 'source', 'shared/ui/token.css': 'tokens', 'README.md': 'readme', 'composition/delivery/core.ts': 'delivery', 'composition/architecture/check.ts': 'guard', 'enablement/verification/web/baseline.ts': 'baseline verifier', [workflowPathFor('public')]: 'workflow', '.github/actions/delivery-component/action.yml': 'component steps', '.github/actions/delivery-contracts/action.yml': 'contract steps' })) put(path, value)
  const git = (...args: string[]) => execFileSync('git', args, { cwd: root, encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim()
  git('init'); git('config', 'user.name', 'Fixture'); git('config', 'user.email', 'fixture@example.invalid')
  const commit = () => { git('add', '-A'); git('commit', '-m', 'fixture') }
  commit()
  const owner = () => { const result = catalog(root); assert.deepEqual(result.blocked, []); return result.owners.find(o => o.id === 'public')! }
  return { root, put, git, commit, owner, clean: () => rmSync(root, { recursive: true, force: true }) }
}
test('Public is registered once as a Bun web directory without changing native target routing', () => {
  const f = fixture()
  try {
    const plan = makePlan(f.root).matrix.include
    assert.equal(plan.length, 1)
    assert.deepEqual([plan[0].id, plan[0].target, plan[0].runner, plan[0].kind, plan[0].stage, plan[0].packageManager], ['public', 'web', 'ubuntu-24.04', 'directory', 'website', 'bun'])
    f.put('public/delivery.json', JSON.stringify({ ...registration, targets: ['macos-x64'] }))
    assert.match(catalog(f.root).blocked.join(' '), /Web delivery/)
    f.put('public/delivery.json', JSON.stringify({ ...registration, inputs: ['shared/ui/'], files: ['public/site/source.ts'] }))
    assert.match(catalog(f.root).blocked.join(' '), /Package source must be covered/)
    f.put('public/delivery.json', JSON.stringify({ ...registration, files: ['../outside'] }))
    assert.match(catalog(f.root).blocked.join(' '), /Unsafe/)
  } finally { f.clean() }
})
test('explicit content/config files and their deletion invalidate reuse; unrelated native changes do not', () => {
  const f = fixture()
  try {
    let previous = sourceDigest(f.root, f.owner())
    for (const [path, contents] of [['README.md', 'new readme'], ['capture/agent/skills/apparent/SKILL.md', 'skill'], ['.npmrc', 'ignore-scripts=true']]) {
      f.put(path, contents); f.commit()
      const next = sourceDigest(f.root, f.owner()); assert.notEqual(next, previous, path); previous = next
    }
    rmSync(join(f.root, '.npmrc')); f.commit()
    const next = sourceDigest(f.root, f.owner()); assert.notEqual(next, previous)
    f.put('capture/engine/native.rs', 'native'); f.commit(); assert.equal(sourceDigest(f.root, f.owner()), next)
    symlinkSync(join(f.root, 'README.md'), join(f.root, '.npmrc'))
    assert.match(catalog(f.root).blocked.join(' '), /Symlink/)
  } finally { f.clean() }
})

test('Public verifier closure participates in its fingerprint', () => {
  const f = fixture()
  try {
    const before = sourceDigest(f.root, f.owner())
    f.put('enablement/verification/web/baseline.ts', 'changed baseline verifier')
    f.commit()
    assert.notEqual(sourceDigest(f.root, f.owner()), before)
  } finally { f.clean() }
})
test('production build configuration changes the common reuse identity without exposing values', () => {
  const f = fixture(), saved = { ImageOS: process.env.ImageOS, ImageVersion: process.env.ImageVersion, PUBLIC_POSTHOG_TOKEN: process.env.PUBLIC_POSTHOG_TOKEN }
  try {
    process.env.ImageOS = 'ubuntu24'; process.env.ImageVersion = 'fixture'; process.env.PUBLIC_POSTHOG_TOKEN = 'first-public-key'
    const owner = f.owner(), first = inputDigest(f.root, owner, 'web')
    process.env.PUBLIC_POSTHOG_TOKEN = 'second-public-key'
    assert.notEqual(first, inputDigest(f.root, owner, 'web'))
    const digest = environmentDigest(owner)
    assert.match(digest, /^[a-f0-9]{64}$/); assert(!digest.includes('second-public-key'))
  } finally {
    for (const [key, value] of Object.entries(saved)) { if (value === undefined) delete process.env[key]; else process.env[key] = value }
    f.clean()
  }
})
test('directory builder uses locked Bun installation and copies only the declared verified artifact', () => {
  const f = fixture(), outputRoot = mkdtempSync(join(tmpdir(), 'public-built-')), saved = process.env.PATH
  try {
    const bin = join(outputRoot, 'bin'); mkdirSync(bin)
    const log = join(outputRoot, 'calls')
    for (const command of ['bun', 'npm']) {
      const script = join(bin, command)
      writeFileSync(script, `#!/bin/sh\nprintf '%s\\n' '${command}' \"$*\" >> '${log}'\n${command === 'npm' ? 'mkdir -p dist-apparent\nprintf verified > dist-apparent/index.html\nprintf private > private.txt' : ''}\n`)
      chmodSync(script, 0o700)
    }
    process.env.PATH = `${bin}:${saved}`
    buildOwner(f.root, f.owner(), 'web', join(outputRoot, 'output'))
    assert.deepEqual(readFileSync(log, 'utf8').trim().split('\n'), ['bun', 'install --frozen-lockfile', 'npm', 'run verify:installed'])
    assert.equal(readFileSync(join(outputRoot, 'output/payload/index.html'), 'utf8'), 'verified')
    assert.throws(() => readFileSync(join(outputRoot, 'output/private.txt')))
  } finally { process.env.PATH = saved; f.clean(); rmSync(outputRoot, { recursive: true, force: true }) }
})

test('web reuse admits unchanged Public bytes after native-only changes and rejects changed build config', () => {
  const f = fixture(), saved = { ImageOS: process.env.ImageOS, ImageVersion: process.env.ImageVersion, PUBLIC_POSTHOG_TOKEN: process.env.PUBLIC_POSTHOG_TOKEN }
  try {
    process.env.ImageOS = 'ubuntu24'; process.env.ImageVersion = 'fixture'; process.env.PUBLIC_POSTHOG_TOKEN = 'first-public-key'
    const previous = f.git('rev-parse', 'HEAD')
    f.put('capture/engine/native.rs', 'unrelated native source'); f.commit()
    const current = f.git('rev-parse', 'HEAD'), owner = f.owner(), digest = inputDigest(f.root, owner, 'web')
    const run = { id: 12, run_attempt: 1, head_sha: previous, head_branch: 'main', event: 'push', conclusion: 'success', path: workflowPathFor('public') }
    const jobs = ['deliver / stream (public, web)', 'deliver / Delivery contracts', 'deliver / Workflow runtime budget'].map(name => ({ name, status: 'completed', conclusion: 'success' }))
    const artifacts = [{ id: 13, name: `stream-public-web-${digest}`, expired: false }]
    const admit = () => assessPrior(f.root, owner, 'web', current, 'tryapparent/apparent', [run], () => jobs, () => artifacts)
    assert.equal(admit().reuse, true)
    process.env.PUBLIC_POSTHOG_TOKEN = 'changed-public-key'
    assert.equal(admit().reuse, false)
  } finally {
    for (const [key, value] of Object.entries(saved)) { if (value === undefined) delete process.env[key]; else process.env[key] = value }
    f.clean()
  }
})

test('source identities distinguish filenames containing serialized manifest separators', () => {
  const f = fixture()
  try {
    f.put('public/site/a.ts', 'first'); f.put('public/site/b.ts', 'second'); f.commit()
    const before = sourceDigest(f.root, f.owner()), firstBlob = f.git('rev-parse', 'HEAD:public/site/a.ts')
    rmSync(join(f.root, 'public/site/a.ts')); rmSync(join(f.root, 'public/site/b.ts'))
    f.put(`public/site/a.ts ${firstBlob}\n100644 public/site/b.ts`, 'second'); f.commit()
    assert.notEqual(sourceDigest(f.root, f.owner()), before)
  } finally { f.clean() }
})

test('canonical PR workflow identity is accepted for builds only', () => {
  const valid = { GITHUB_EVENT_NAME: 'pull_request', GITHUB_REPOSITORY: 'tryapparent/apparent', GITHUB_REF: 'refs/pull/732/merge', GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/stream-public.yml@refs/pull/732/merge', APPARENT_DELIVERY_WORKFLOW_PATH: '.github/workflows/stream-public.yml' }
  assert.doesNotThrow(() => assertBuildWorkflowIdentity(valid, 'public'))
  assert.throws(() => assertBuildWorkflowIdentity({ ...valid, GITHUB_REPOSITORY: 'fork/apparent' }, 'public'), /canonical owner workflow/)
  assert.throws(() => assertBuildWorkflowIdentity({ ...valid, GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/value-stream-delivery.yml@refs/pull/732/merge' }, 'public'), /canonical owner workflow/)
  assert.throws(() => assertBuildWorkflowIdentity({ ...valid, GITHUB_REF: 'refs/heads/main' }, 'public'), /canonical owner workflow/)
  assert.doesNotThrow(() => assertBuildWorkflowIdentity({ GITHUB_EVENT_NAME: 'push', GITHUB_REF: 'refs/heads/main' }, 'public'))
})
