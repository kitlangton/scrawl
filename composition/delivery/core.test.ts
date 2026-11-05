import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { chmodSync, existsSync, mkdtempSync, mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
import { after, test } from 'node:test'
import { assessPrior, catalog, inputDigest, reusePayload, sealPayload, type Owner, workflowPath } from './core.ts'

// Hooks export repository-local Git variables. Synthetic repositories must not
// accidentally use the hook caller's index, object database or worktree.
for (const name of execFileSync('git', ['rev-parse', '--local-env-vars'], { encoding: 'utf8' }).trim().split('\n')) delete process.env[name]
process.env.ImageOS ??= 'ubuntu24'
process.env.ImageVersion ??= '2026.01.01'
const repo = execFileSync('git', ['rev-parse', '--show-toplevel'], { encoding: 'utf8' }).trim()
const temp = mkdtempSync(resolve(tmpdir(), 'delivery-core-'))
after(() => rmSync(temp, { recursive: true, force: true }))
const owner: Owner = { schema: 1, id: 'record', kind: 'npm', package: 'record', inputs: ['record/', 'shared/ui/'], verify: ['typecheck', 'test'], build: 'build', targets: ['linux-x64'], stage: 'library', language: 'typescript' }
function git(cwd: string, ...args: string[]) { execFileSync('git', args, { cwd, stdio: 'ignore' }) }
function newRepo(path: string) {
  mkdirSync(path, { recursive: true }); git(path, 'init', '-q'); git(path, 'config', 'user.email', 'delivery-test@example.invalid'); git(path, 'config', 'user.name', 'delivery test')
  for (const dir of ['record', 'shared/ui', 'composition/delivery', '.github/workflows', 'enablement/verification']) mkdirSync(resolve(path, dir), { recursive: true })
  writeFileSync(resolve(path, 'record/index.ts'), 'one\n'); writeFileSync(resolve(path, 'record/config.json'), '{}\n'); writeFileSync(resolve(path, 'record/delivery.json'), '{}\n')
  writeFileSync(resolve(path, 'shared/ui/theme.ts'), 'theme\n')
  writeFileSync(resolve(path, 'composition/delivery/core.ts'), 'core\n'); writeFileSync(resolve(path, 'LICENSE'), 'license\n')
  writeFileSync(resolve(path, workflowPath), 'workflow\n'); writeFileSync(resolve(path, 'enablement/verification/workflow-budget.ts'), 'budget\n')
  git(path, 'add', '.'); git(path, 'commit', '-qm', 'initial'); return execFileSync('git', ['rev-parse', 'HEAD'], { cwd: path, encoding: 'utf8' }).trim()
}

test('catalog reports absent owners pending and blocks present owners without registration', () => {
  const path = resolve(temp, 'missing-registration')
  mkdirSync(resolve(path, 'composition/app'), { recursive: true })
  const missing = catalog(path)
  assert.deepEqual(missing.blocked, ['app-shell: source exists without delivery registration'])
  assert.deepEqual(missing.pending, ['record', 'engine', 'onboarding', 'agent'])
  mkdirSync(resolve(path, 'record')); mkdirSync(resolve(path, 'shared/ui'), { recursive: true })
  writeFileSync(resolve(path, 'record/delivery.json'), JSON.stringify({ ...owner, inputs: ['shared/ui/'] }))
  assert.match(catalog(path).blocked.join(';'), /Package source must be covered/)
})

test('reuse reissues the exact admitted bytes and rejects tampering and wrong receipts', () => {
  const input = resolve(temp, 'reuse-input'); mkdirSync(input); writeFileSync(resolve(input, 'package.tgz'), 'verified content')
  const env = { GITHUB_REF: 'refs/heads/main', GITHUB_REPOSITORY: 'tryapparent/apparent', GITHUB_WORKFLOW_REF: `tryapparent/apparent/${workflowPath}@refs/heads/main`, GITHUB_RUN_ID: '123', GITHUB_RUN_ATTEMPT: '1' }
  const priorDir = resolve(temp, 'reuse-prior')
  const receipt = sealPayload(repo, owner, 'linux-x64', input, priorDir, 'b'.repeat(64), env)
  const zip = resolve(temp, 'prior.zip')
  execFileSync('zip', ['-q', zip, 'component.tar.gz', 'receipt.json'], { cwd: priorDir })
  const output = resolve(temp, 'reuse-output')
  const next = reusePayload(repo, owner, 'linux-x64', zip, output, 'b'.repeat(64), receipt.sourceSha, '99', '123', '1', { ...env, GITHUB_RUN_ID: '124' })
  assert.deepEqual(readFileSync(resolve(output, 'component.tar.gz')), readFileSync(resolve(priorDir, 'component.tar.gz')))
  assert.equal(next.runId, '124'); assert.equal(next.reusedFrom.artifactId, '99')
  assert.throws(() => reusePayload(repo, owner, 'linux-x64', zip, resolve(temp, 'bad-run'), 'b'.repeat(64), receipt.sourceSha, '99', '999', '1', env), /run does not match/)
  assert.throws(() => reusePayload(repo, owner, 'linux-x64', zip, resolve(temp, 'bad-receipt'), 'c'.repeat(64), receipt.sourceSha, '99', '123', '1', env), /receipt/)
  writeFileSync(resolve(priorDir, 'component.tar.gz'), 'tampered')
  rmSync(zip)
  execFileSync('zip', ['-q', zip, 'component.tar.gz', 'receipt.json'], { cwd: priorDir })
  assert.throws(() => reusePayload(repo, owner, 'linux-x64', zip, resolve(temp, 'bad-archive'), 'b'.repeat(64), receipt.sourceSha, '99', '123', '1', env), /hash or size/)
})

test('full input closure changes for owner source, delivery core, workflow, budget, license and renames', () => {
  const path = resolve(temp, 'hash-repo'); const sha = newRepo(path)
  const initial = inputDigest(path, owner)
  const dualTargetOwner: Owner = { ...owner, targets: ['linux-x64', 'macos-x64'] }
  assert.notEqual(inputDigest(path, dualTargetOwner, 'linux-x64'), inputDigest(path, dualTargetOwner, 'macos-x64'))
  for (const [name, content] of [['composition/delivery/core.ts', 'changed core'], [workflowPath, 'changed workflow'], ['enablement/verification/workflow-budget.ts', 'changed budget'], ['LICENSE', 'changed license'], ['record/native.rs', 'native'], ['record/config.json', '{"value":2}'], ['shared/ui/theme.ts', 'changed theme']]) {
    writeFileSync(resolve(path, name), content); git(path, 'add', '-A'); git(path, 'commit', '-qm', `change ${name}`)
    assert.notEqual(inputDigest(path, owner), initial, `${name} must invalidate reuse`)
  }
  git(path, 'mv', 'record/index.ts', 'record/renamed.ts'); git(path, 'commit', '-qm', 'rename')
  assert.notEqual(inputDigest(path, owner), initial)
  assert.match(sha, /^[0-9a-f]{40,64}$/)
})

test('admission requires canonical repo, exact workflow, owner job, audit and unexpired matching digest', () => {
  const path = resolve(temp, 'admit-repo'); const sha = newRepo(path)
  git(path, 'commit', '--allow-empty', '-qm', 'later'); let current = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: path, encoding: 'utf8' }).trim()
  const digest = inputDigest(path, owner)
  const run = { id: 10, run_attempt: 1, head_sha: sha, head_branch: 'main', event: 'push', conclusion: 'failure', path: workflowPath }
  const goodJobs = [{ name: 'stream (record, linux-x64)', status: 'completed', conclusion: 'success' }, { name: 'Delivery contracts', status: 'completed', conclusion: 'success' }, { name: 'Workflow runtime budget', status: 'completed', conclusion: 'success' }]
  const artifact = [{ id: 9, name: `stream-record-linux-x64-${digest}`, expired: false }]
  const assess = (r = run, j = goodJobs, a = artifact, repository = 'tryapparent/apparent') => assessPrior(path, owner, 'linux-x64', current, repository, [r], () => j, () => a)
  assert.equal(assess().reuse, true)
  assert.equal(assess(run, goodJobs, artifact, 'someone/fork').reuse, false)
  assert.equal(assess({ ...run, path: 'other.yml' }).reuse, false)
  assert.equal(assess({ ...run, head_branch: 'feature' }).reuse, false)
  assert.equal(assess(run, [goodJobs[0], goodJobs[1]]).reuse, false)
  assert.equal(assess(run, goodJobs.slice(1)).reuse, false)
  assert.equal(assess(run, [...goodJobs, goodJobs[0]]).reuse, false)
  assert.equal(assess(run, [...goodJobs, goodJobs[1]]).reuse, false)
  assert.equal(assess(run, [goodJobs[0], goodJobs[2]]).reuse, false)
  assert.equal(assess(run, goodJobs, [{ ...artifact[0], expired: true }]).reuse, false)
  assert.equal(assess(run, goodJobs, [{ ...artifact[0], name: 'wrong-digest' }]).reuse, false)
  writeFileSync(resolve(path, 'record/native.rs'), 'new source after prior run')
  git(path, 'add', '-A'); git(path, 'commit', '-qm', 'change after candidate')
  current = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: path, encoding: 'utf8' }).trim()
  assert.equal(assess(run, goodJobs, [{ id: 11, name: `stream-record-linux-x64-${inputDigest(path, owner)}`, expired: false }]).reuse, false)
})

test('sealing only archives contained payloads under canonical main identity', () => {
  const input = resolve(temp, 'payload'); const output = resolve(temp, 'sealed')
  mkdirSync(input); writeFileSync(resolve(input, 'package.tgz'), 'bundle')
  const env = { GITHUB_REF: 'refs/heads/main', GITHUB_REPOSITORY: 'tryapparent/apparent', GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/value-stream-delivery.yml@refs/heads/main', GITHUB_RUN_ID: '123', GITHUB_RUN_ATTEMPT: '1' }
  const receipt = sealPayload(repo, owner, 'linux-x64', input, output, 'a'.repeat(64), env)
  assert.equal(receipt.id, owner.id)
  assert.equal(receipt.size > 0, true)
  assert.equal(existsSync(resolve(output, 'component.tar.gz')), true)
  assert.equal(receipt.sha256, createHash('sha256').update(readFileSync(resolve(output, 'component.tar.gz'))).digest('hex'))
  assert.throws(() => sealPayload(repo, owner, 'macos-x64', input, resolve(temp, 'wrong-target'), 'a'.repeat(64), env), /target/)
  const unsafe = resolve(temp, 'unsafe-payload'); mkdirSync(unsafe)
  symlinkSync('/etc/passwd', resolve(unsafe, 'escape'))
  assert.throws(() => sealPayload(repo, owner, 'linux-x64', unsafe, resolve(temp, 'unsafe-seal'), 'a'.repeat(64), env), /symlink/)
})

test('fingerprints refuse reuse when hosted runner image identity is absent', () => {
  const path = resolve(temp, 'image-repo'); newRepo(path)
  const imageOS = process.env.ImageOS; const imageVersion = process.env.ImageVersion
  try {
    delete process.env.ImageOS; delete process.env.ImageVersion
    assert.throws(() => inputDigest(path, owner), /Runner image identity/)
  } finally {
    if (imageOS) process.env.ImageOS = imageOS
    if (imageVersion) process.env.ImageVersion = imageVersion
  }
})

test('admit invokes gh api with canonical GH_REPO and no unsupported flags', () => {
  const bin = resolve(temp, 'fake-bin'); mkdirSync(bin)
  const checkout = resolve(temp, 'cli-repo'); newRepo(checkout)
  for (const file of ['core.ts', 'cli.ts']) writeFileSync(resolve(checkout, 'composition/delivery', file), readFileSync(resolve(repo, 'composition/delivery', file)))
  writeFileSync(resolve(checkout, 'record/delivery.json'), JSON.stringify(owner))
  git(checkout, 'add', '-A'); git(checkout, 'commit', '-qm', 'delivery implementation')
  const ancestor = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: checkout, encoding: 'utf8' }).trim()
  git(checkout, 'commit', '--allow-empty', '-qm', 'unchanged candidate')
  const digest = inputDigest(checkout, owner, 'linux-x64')
  const fakeGh = [
    '#!/bin/sh',
    '[ "$1" = api ] && [ "$GH_REPO" = tryapparent/apparent ] || exit 90',
    'case "$2" in',
    '  */actions/workflows/value-stream-delivery.yml) printf "%s\\n" \'{"id":4,"path":".github/workflows/value-stream-delivery.yml"}\' ;;',
    '  */actions/workflows/4/runs?*) printf "%s\\n" \'{"workflow_runs":[{"id":88,"run_attempt":1,"head_sha":"' + ancestor + '","head_branch":"main","event":"push","conclusion":"failure"}]}\' ;;',
    '  */actions/runs/88/attempts/1/jobs?*) printf "%s\\n" \'{"total_count":3,"jobs":[{"name":"stream (record, linux-x64)","status":"completed","conclusion":"success"},{"name":"Delivery contracts","status":"completed","conclusion":"success"},{"name":"Workflow runtime budget","status":"completed","conclusion":"success"}]}\' ;;',
    '  */actions/runs/88/artifacts?*) printf "%s\\n" \'{"total_count":1,"artifacts":[{"id":91,"name":"stream-record-linux-x64-' + digest + '","expired":false}]}\' ;;',
    '  *) exit 91 ;;',
    'esac',
  ].join('\n') + '\n'
  const executable = resolve(bin, 'gh'); writeFileSync(executable, fakeGh); chmodSync(executable, 0o755)
  const result = spawnSync('node', ['--experimental-strip-types', 'composition/delivery/cli.ts', 'admit', 'record', 'linux-x64'], {
    cwd: checkout, encoding: 'utf8', env: { ...process.env, PATH: `${bin}:${process.env.PATH}`, GITHUB_REPOSITORY: 'tryapparent/apparent', ImageOS: process.env.ImageOS!, ImageVersion: process.env.ImageVersion! },
  })
  assert.equal(result.status, 0, result.stderr)
  assert.match(result.stdout, /"reuse":true/)
  assert.match(result.stdout, /"prior_artifact_id":91/)
})
