import './runtime/pr-candidate.test.ts'
import './consumer/handoff.test.ts'
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { chmodSync, existsSync, mkdtempSync, mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
import { stripTypeScriptTypes } from 'node:module'
import { after, test } from 'node:test'
import { matchPrCandidateInputs, assertBuildWorkflowIdentity } from './cli.ts'
import { prInputsMatch, sourceDigest, mediaBuildIdentity, supportsPrCandidate, packageBuildEnvironment, nativeBuildIdentity, assessPrior, catalog, inputDigest, makePlan, optionalVerificationPeers, reusePayload, sealPayload, verifyPublicExports, type Owner, workflowPathFor } from './core.ts'

// Hooks export repository-local Git variables. Synthetic repositories must not
// accidentally use the hook caller's index, object database or worktree.
for (const name of execFileSync('git', ['rev-parse', '--local-env-vars'], { encoding: 'utf8' }).trim().split('\n')) delete process.env[name]
process.env.ImageOS ??= 'ubuntu24'
process.env.ImageVersion ??= '2026.01.01'
const repo = execFileSync('git', ['rev-parse', '--show-toplevel'], { encoding: 'utf8' }).trim()
const temp = mkdtempSync(resolve(tmpdir(), 'delivery-core-'))
after(() => rmSync(temp, { recursive: true, force: true }))
const owner: Owner = { schema: 1, id: 'record', kind: 'npm', package: 'record', inputs: ['record/', 'shared/ui/'], verify: ['typecheck', 'test'], build: 'build', targets: ['linux-x64'], stage: 'library', language: 'typescript' }
test('canonical producer identity belongs to the owner regardless of the running stream', () => {
  const previous = process.env.APPARENT_DELIVERY_WORKFLOW_PATH
  try {
    process.env.APPARENT_DELIVERY_WORKFLOW_PATH = '.github/workflows/stream-agent.yml'
    assert.equal(workflowPathFor('record'), '.github/workflows/stream-record.yml')
    assert.equal(workflowPathFor('public'), '.github/workflows/stream-public.yml')
  } finally {
    if (previous === undefined) delete process.env.APPARENT_DELIVERY_WORKFLOW_PATH
    else process.env.APPARENT_DELIVERY_WORKFLOW_PATH = previous
  }
})
function git(cwd: string, ...args: string[]) { execFileSync('git', args, { cwd, stdio: 'ignore' }) }
function newRepo(path: string) {
  mkdirSync(path, { recursive: true }); git(path, 'init', '-q'); git(path, 'config', 'user.email', 'delivery-test@example.invalid'); git(path, 'config', 'user.name', 'delivery test')
  for (const dir of ['record', 'shared/ui', 'composition/delivery', 'composition/architecture', '.github/workflows', 'enablement/verification']) mkdirSync(resolve(path, dir), { recursive: true })
  writeFileSync(resolve(path, 'record/index.ts'), 'one\n'); writeFileSync(resolve(path, 'record/config.json'), '{}\n'); writeFileSync(resolve(path, 'record/delivery.json'), '{}\n')
  writeFileSync(resolve(path, 'shared/ui/theme.ts'), 'theme\n')
  writeFileSync(resolve(path, 'composition/delivery/core.ts'), 'core\n'); writeFileSync(resolve(path, 'LICENSE'), 'license\n')
  writeFileSync(resolve(path, 'composition/architecture/guard.ts'), 'guard\n')
  writeFileSync(resolve(path, workflowPathFor(owner.id)), 'workflow\n'); writeFileSync(resolve(path, 'enablement/verification/workflow-budget.ts'), 'budget\n')
  git(path, 'add', '.'); git(path, 'commit', '-qm', 'initial'); return execFileSync('git', ['rev-parse', 'HEAD'], { cwd: path, encoding: 'utf8' }).trim()
}

test('catalog reports absent owners pending and blocks present owners without registration', () => {
  const path = resolve(temp, 'missing-registration')
  mkdirSync(resolve(path, 'composition/app'), { recursive: true })
  const missing = catalog(path)
  assert.deepEqual(missing.blocked, ['app-shell: source exists without delivery registration'])
  assert.deepEqual(missing.pending, ['record', 'connect-client', 'connect-browser', 'engine', 'engine-native', 'engine-capture-native', 'engine-media-native', 'platform-config', 'platform-resources', 'onboarding', 'agent', 'public'])
  mkdirSync(resolve(path, 'record')); mkdirSync(resolve(path, 'shared/ui'), { recursive: true })
  writeFileSync(resolve(path, 'record/delivery.json'), JSON.stringify({ ...owner, inputs: ['shared/ui/'] }))
  assert.match(catalog(path).blocked.join(';'), /Package source must be covered/)
})

test('platform config native lane declares its complete source closure and composition architecture owner', () => {
  const path = resolve(temp, 'platform-config-plan')
  for (const dir of ['composition/config/native', 'shared/config-native', '.cargo']) mkdirSync(resolve(path, dir), { recursive: true })
  writeFileSync(resolve(path, 'composition/config/native/delivery.json'), JSON.stringify({ ...owner, id: 'platform-config', kind: 'directory', language: 'native', package: 'composition/config/native', artifactPath: 'dist', inputs: ['composition/config/', 'shared/config-native/', '.cargo/'], targets: ['linux-x64', 'macos-x64'], stage: 'library' }))
  const plan = makePlan(path)
  const native = plan.matrix.include.filter(row => row.id === 'platform-config')
  assert.deepEqual(plan.blocked, [])
  assert.equal(native.length, 2)
  assert(native.every(row => row.language === 'native' && row.kind === 'directory' && row.architecture_owner === 'composition'))
  assert.deepEqual(native.map(row => row.target), ['linux-x64', 'macos-x64'])
})

test('platform resources fixture has only public Linux and Mac targets and fingerprints inherited Cargo config', () => {
  const path = resolve(temp, 'platform-resources-plan')
  for (const dir of ['shared/resources/src', 'shared/resources/delivery', 'shared/resources/native', '.cargo']) mkdirSync(resolve(path, dir), { recursive: true })
  const registration = { schema: 1, id: 'platform-resources', kind: 'directory', package: 'shared/resources/delivery', inputs: ['shared/resources/', '.cargo/'], verify: ['verify'], build: 'build', targets: ['linux-x64', 'macos-x64'], stage: 'library', language: 'native', artifactPath: 'dist' }
  writeFileSync(resolve(path, 'shared/resources/delivery/delivery.json'), JSON.stringify(registration))
  writeFileSync(resolve(path, 'shared/resources/delivery/index.ts'), 'fixture\n')
  writeFileSync(resolve(path, '.cargo/config.toml'), '[build]\njobs = 3\n')
  const plan = makePlan(path)
  const native = plan.matrix.include.filter(row => row.id === 'platform-resources')
  assert.deepEqual(plan.blocked, [])
  assert.deepEqual(native.map(row => row.target), ['linux-x64', 'macos-x64'])
  assert(native.every(row => row.kind === 'directory' && row.language === 'native' && row.stage === 'library' && row.architecture_owner === 'shared'))
  assert.deepEqual(catalog(path).owners.find(item => item.id === 'platform-resources')?.inputs, ['shared/resources/', '.cargo/'])
  writeFileSync(resolve(path, 'shared/resources/delivery/delivery.json'), JSON.stringify({ ...registration, inputs: ['shared/resources/'] }))
  assert.match(catalog(path).blocked.join(';'), /must fingerprint shared\/resources\/ and \.cargo\//)
  writeFileSync(resolve(path, 'shared/resources/delivery/delivery.json'), JSON.stringify({ ...registration, targets: ['linux-x64', 'macos-x64', 'macos-arm64'] }))
  assert.match(catalog(path).blocked.join(';'), /Unsupported target/)
})

test('native Engine and SDK are independent lanes with the same architectural owner', () => {
  const path = resolve(temp, 'native-plan')
  mkdirSync(resolve(path, 'engine/sdk'), { recursive: true })
  mkdirSync(resolve(path, 'engine/delivery/native'), { recursive: true })
  writeFileSync(resolve(path, 'engine/delivery.json'), JSON.stringify({ ...owner, id: 'engine', package: 'engine/sdk', inputs: ['engine/'] }))
  writeFileSync(resolve(path, 'engine/delivery/native/delivery.json'), JSON.stringify({ ...owner, id: 'engine-native', kind: 'directory', language: 'native', package: 'engine/delivery/native', artifactPath: 'dist', inputs: ['engine/'], targets: ['linux-x64', 'macos-x64'] }))
  const plan = makePlan(path)
  assert.deepEqual(plan.blocked, [])
  const sdk = plan.matrix.include.filter(row => row.id === 'engine')
  const native = plan.matrix.include.filter(row => row.id === 'engine-native')
  assert.equal(sdk.length, 1); assert.equal(sdk[0].language, 'typescript')
  assert.equal(native.length, 2); assert(native.every(row => row.language === 'native' && row.architecture_owner === 'engine'))
})

test('native Engine media has Linux and native Intel Mac lanes with a complete owner input closure', () => {
  const path = resolve(temp, 'engine-media-plan')
  for (const dir of ['engine/delivery/media', 'engine/sdk', 'capture/engine', '.cargo']) mkdirSync(resolve(path, dir), { recursive: true })
  writeFileSync(resolve(path, 'engine/delivery.json'), JSON.stringify({ ...owner, id: 'engine', package: 'engine/sdk', inputs: ['engine/'] }))
  writeFileSync(resolve(path, 'engine/delivery/media/delivery.json'), JSON.stringify({ ...owner, id: 'engine-media-native', kind: 'directory', language: 'native', package: 'engine/delivery/media', artifactPath: 'dist', verify: ['verify'], inputs: ['engine/', 'capture/engine/', '.cargo/'], targets: ['linux-x64', 'macos-x64'] }))
  const plan = makePlan(path)
  assert.deepEqual(plan.blocked, [])
  const media = plan.matrix.include.filter(row => row.id === 'engine-media-native')
  assert.equal(media.length, 2)
  assert.equal(media[1].target, 'macos-x64')
  assert.equal(media[1].runner, 'macos-26-intel')
  assert.equal(media[0].target, 'linux-x64')
  assert.equal(media[0].runner, 'ubuntu-24.04')
  assert.equal(media[0].language, 'native')
  assert.equal(media[0].kind, 'directory')
  assert.equal(media[0].architecture_owner, 'engine')
  assert.equal(media[0].package, 'engine/delivery/media')
  assert.deepEqual(media[0].verify, ['verify'])
  assert.equal(media[0].build, 'build')
  assert.deepEqual(catalog(path).owners.find(item => item.id === 'engine-media-native')?.inputs, ['engine/', 'capture/engine/', '.cargo/'])
})


test('packed consumer imports code but verifies stylesheet exports as contained assets', () => {
  const consumer = resolve(temp, 'asset-consumer')
  const pkg = resolve(consumer, 'node_modules/@apparent/asset-fixture')
  mkdirSync(pkg, { recursive: true })
  writeFileSync(resolve(pkg, 'package.json'), JSON.stringify({ name: '@apparent/asset-fixture', type: 'module', exports: { '.': './index.js', './ui/style.css': './style.css' } }))
  writeFileSync(resolve(pkg, 'index.js'), stripTypeScriptTypes('export const ready: boolean = true;\n'))
  writeFileSync(resolve(pkg, 'style.css'), '.pill { display: flex; }\n')
  const verify = () => verifyPublicExports(consumer, '@apparent/asset-fixture', ['.', './ui/style.css'])
  verify() // Node would reject CSS if it were imported as JavaScript.
  rmSync(resolve(pkg, 'style.css')); assert.throws(verify)
  writeFileSync(resolve(pkg, 'style.css'), ''); assert.throws(verify)
  rmSync(resolve(pkg, 'style.css'))
  const external = resolve(consumer, 'external.css'); writeFileSync(external, '.outside {}')
  symlinkSync(external, resolve(pkg, 'style.css')); assert.throws(verify)
  rmSync(resolve(pkg, 'style.css')); writeFileSync(resolve(pkg, 'style.css'), '.pill {}')
  writeFileSync(resolve(pkg, 'index.js'), stripTypeScriptTypes('throw new Error("broken public JS export");\n'))
  assert.throws(verify) // Asset handling must not skip runtime code checks.
})

test('all-exports verification provisions declared optional host peers without making them required', () => {
  const manifest = { peerDependencies: { react: '^19.2.0', 'react-dom': '^19.2.0', required: '^1' }, peerDependenciesMeta: { react: { optional: true }, 'react-dom': { optional: true } } }
  assert.deepEqual(optionalVerificationPeers(manifest), ['react@^19.2.0', 'react-dom@^19.2.0'])
  assert.deepEqual(optionalVerificationPeers({ peerDependencies: { required: '^1' } }), [])
  for (const [name, version] of [['--prefix', 'outside'], ['peer', ''], ['peer', 1], ['peer', '1\0']])
    assert.throws(() => optionalVerificationPeers({ peerDependencies: { [String(name)]: version }, peerDependenciesMeta: { [String(name)]: { optional: true } } }), /Invalid optional/)

  const owner = resolve(temp, 'local-peer-owner'); mkdirSync(owner);
  const packedPeer = resolve(owner, 'peer.tgz'); writeFileSync(packedPeer, 'fixture');
  const local = { peerDependencies: { '@apparent/private': '0.1.0' }, peerDependenciesMeta: { '@apparent/private': { optional: true } }, devDependencies: { '@apparent/private': 'file:peer.tgz' } };
  assert.deepEqual(optionalVerificationPeers(local, owner), [packedPeer]);
  assert.throws(() => optionalVerificationPeers(local), /package root/);
  const outside = resolve(temp, 'outside-peer.tgz'); writeFileSync(outside, 'fixture');
  assert.throws(() => optionalVerificationPeers({ ...local, devDependencies: { '@apparent/private': 'file:../outside-peer.tgz' } }, owner), /contained/);

  const consumer = resolve(temp, 'optional-host-peer')
  const pkg = resolve(consumer, 'node_modules/optional-ui')
  mkdirSync(pkg, { recursive: true })
  writeFileSync(resolve(pkg, 'package.json'), JSON.stringify({ name: 'optional-ui', type: 'module', exports: { '.': './index.js', './ui': './ui.js' }, peerDependencies: { 'host-peer': '^1' }, peerDependenciesMeta: { 'host-peer': { optional: true } } }))
  writeFileSync(resolve(pkg, 'index.js'), stripTypeScriptTypes('export const headless: boolean = true;'))
  writeFileSync(resolve(pkg, 'ui.js'), stripTypeScriptTypes('export { supplied } from "host-peer";'))
  verifyPublicExports(consumer, 'optional-ui', ['.'])
  assert.throws(() => verifyPublicExports(consumer, 'optional-ui', ['.', './ui']))
  const peer = resolve(consumer, 'node_modules/host-peer'); mkdirSync(peer)
  writeFileSync(resolve(peer, 'package.json'), JSON.stringify({ name: 'host-peer', type: 'module', exports: './index.js' }))
  writeFileSync(resolve(peer, 'index.js'), stripTypeScriptTypes('export const supplied: boolean = true;'))
  verifyPublicExports(consumer, 'optional-ui', ['.', './ui'])
  assert.equal(JSON.parse(readFileSync(resolve(pkg, 'package.json'), 'utf8')).peerDependenciesMeta['host-peer'].optional, true)
})

test('reuse reissues the exact admitted bytes and rejects tampering and wrong receipts', () => {
  const input = resolve(temp, 'reuse-input'); mkdirSync(input); writeFileSync(resolve(input, 'package.tgz'), 'verified content')
  const env = { GITHUB_REF: 'refs/heads/main', GITHUB_REPOSITORY: 'tryapparent/apparent', GITHUB_WORKFLOW_REF: `tryapparent/apparent/${workflowPathFor(owner.id)}@refs/heads/main`, GITHUB_RUN_ID: '123', GITHUB_RUN_ATTEMPT: '1' }
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
  let previousDigest = initial
  for (const [name, content] of [['.github/actions/delivery-component/action.yml', 'changed component steps'], ['.github/actions/delivery-contracts/action.yml', 'changed delivery contracts'], ['composition/delivery/core.ts', 'changed core'], ['composition/architecture/guard.ts', 'changed architecture guard'], [workflowPathFor(owner.id), 'changed workflow'], ['enablement/verification/workflow-budget.ts', 'changed budget'], ['LICENSE', 'changed license'], ['record/native.rs', 'native'], ['record/config.json', '{"value":2}'], ['shared/ui/theme.ts', 'changed theme']]) {
    mkdirSync(resolve(path, name, '..'), { recursive: true }); writeFileSync(resolve(path, name), content); git(path, 'add', '-A'); git(path, 'commit', '-qm', `change ${name}`)
    assert.notEqual(inputDigest(path, owner), previousDigest, `${name} must invalidate reuse`)
    previousDigest = inputDigest(path, owner)
  }
  git(path, 'mv', 'record/index.ts', 'record/renamed.ts'); git(path, 'commit', '-qm', 'rename')
  assert.notEqual(inputDigest(path, owner), initial)
  assert.match(sha, /^[0-9a-f]{40,64}$/)
})

test('admission requires canonical repo, exact workflow, owner job, audit and unexpired matching digest', () => {
  const path = resolve(temp, 'admit-repo'); const sha = newRepo(path)
  git(path, 'commit', '--allow-empty', '-qm', 'later'); let current = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: path, encoding: 'utf8' }).trim()
  const digest = inputDigest(path, owner)
  const run = { id: 10, run_attempt: 1, head_sha: sha, head_branch: 'main', event: 'push', conclusion: 'failure', path: workflowPathFor(owner.id) }
  const goodJobs = [{ name: 'deliver / stream (record, linux-x64)', status: 'completed', conclusion: 'success' }, { name: 'deliver / Delivery contracts', status: 'completed', conclusion: 'success' }, { name: 'deliver / Workflow runtime budget', status: 'completed', conclusion: 'success' }]
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

test('exact-commit handoff admits only the producer receipt digest without a hosted runner identity', () => {
  const path = resolve(temp, 'exact-admit-repo'); const sha = newRepo(path)
  const exactDigest = 'b'.repeat(64)
  const run = { id: 77, run_attempt: 1, head_sha: sha, head_branch: 'main', event: 'push', conclusion: 'success', path: workflowPathFor(owner.id) }
  const jobs = [{ name: 'deliver / stream (record, linux-x64)', status: 'completed', conclusion: 'success' }, { name: 'deliver / Delivery contracts', status: 'completed', conclusion: 'success' }, { name: 'deliver / Workflow runtime budget', status: 'completed', conclusion: 'success' }]
  const artifacts = [{ id: 88, name: `stream-record-linux-x64-${exactDigest}`, expired: false }]
  const imageOS = process.env.ImageOS, imageVersion = process.env.ImageVersion
  try {
    delete process.env.ImageOS; delete process.env.ImageVersion
    const accepted = assessPrior(path, owner, 'linux-x64', sha, 'tryapparent/apparent', [run], () => jobs, () => artifacts, sha, exactDigest)
    assert.equal(accepted.reuse, true)
    assert.throws(() => assessPrior(path, owner, 'linux-x64', sha, 'tryapparent/apparent', [run], () => jobs, () => artifacts, 'f'.repeat(40), exactDigest), /same explicit source commit/)
  } finally {
    if (imageOS) process.env.ImageOS = imageOS
    if (imageVersion) process.env.ImageVersion = imageVersion
  }
})

test('sealing only archives contained payloads under canonical main identity', () => {
  const input = resolve(temp, 'payload'); const output = resolve(temp, 'sealed')
  mkdirSync(input); writeFileSync(resolve(input, 'package.tgz'), 'bundle')
  const env = { GITHUB_REF: 'refs/heads/main', GITHUB_REPOSITORY: 'tryapparent/apparent', GITHUB_WORKFLOW_REF: `tryapparent/apparent/${workflowPathFor(owner.id)}@refs/heads/main`, GITHUB_RUN_ID: '123', GITHUB_RUN_ATTEMPT: '1' }
  const receipt = sealPayload(repo, owner, 'linux-x64', input, output, 'a'.repeat(64), env)
  assert.equal(receipt.id, owner.id)
  assert.equal(receipt.size > 0, true)
  assert.equal(existsSync(resolve(output, 'component.tar.gz')), true)
  assert.equal(receipt.sha256, createHash('sha256').update(readFileSync(resolve(output, 'component.tar.gz'))).digest('hex'))
  assert.throws(() => sealPayload(repo, owner, 'linux-x64', input, resolve(temp, 'wrong-workflow'), 'a'.repeat(64), { ...env, GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/value-stream-delivery.yml@refs/heads/main' }), /canonical main workflow identity/)
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
  mkdirSync(resolve(checkout, 'composition/delivery/runtime'))
  writeFileSync(resolve(checkout, 'composition/delivery/runtime/pr-candidate.ts'), readFileSync(resolve(repo, 'composition/delivery/runtime/pr-candidate.ts')))
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
    '  */actions/workflows/stream-record.yml) printf "%s\\n" \'{"id":4,"path":"' + workflowPathFor(owner.id) + '"}\' ;;',
    '  */actions/workflows/4/runs?*) printf "%s\\n" \'{"workflow_runs":[{"id":88,"run_attempt":1,"head_sha":"' + ancestor + '","head_branch":"main","path":"' + workflowPathFor(owner.id) + '","event":"push","conclusion":"success"}]}\' ;;',
    '  */actions/runs/88/attempts/1/jobs?*) printf "%s\\n" \'{"total_count":3,"jobs":[{"name":"deliver / stream (record, linux-x64)","status":"completed","conclusion":"success"},{"name":"deliver / Delivery contracts","status":"completed","conclusion":"success"},{"name":"deliver / Workflow runtime budget","status":"completed","conclusion":"success"}]}\' ;;',
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

test('capture/2 has its own Linux native route and mandatory admission inputs', () => {
  const path = resolve(temp, 'capture-native-plan')
  newRepo(path)
  mkdirSync(resolve(path, 'engine/delivery/capture'), { recursive: true })
  mkdirSync(resolve(path, '.cargo'))
  const registration = { ...owner, id: 'engine-capture-native', kind: 'directory', language: 'native', package: 'engine/delivery/capture', artifactPath: 'dist', inputs: ['engine/', '.cargo/'], targets: ['linux-x64'] }
  writeFileSync(resolve(path, 'engine/delivery/capture/delivery.json'), JSON.stringify(registration))
  const capture = catalog(path).owners.find(item => item.id === 'engine-capture-native')!
  assert(capture)
  assert.deepEqual(capture.inputs, ['engine/', '.cargo/'])
  const row = makePlan(path).matrix.include.filter(item => item.id === capture.id)
  assert.equal(row.length, 1)
  assert.equal(row[0].target, 'linux-x64')
  assert.equal(row[0].architecture_owner, 'engine')
  assert.equal(row[0].package, 'engine/delivery/capture')
  assert.equal(row[0].language, 'native')
  assert.equal(workflowPathFor(capture.id), '.github/workflows/stream-engine.yml')
})

test('Engine components require the consolidated canonical PR workflow identity', () => {
  for (const id of ['engine', 'engine-native', 'engine-media-native', 'engine-capture-native']) {
    assert.equal(workflowPathFor(id), '.github/workflows/stream-engine.yml')
    const env = {
      GITHUB_EVENT_NAME: 'pull_request', GITHUB_REF: 'refs/pull/123/merge',
      GITHUB_REPOSITORY: 'tryapparent/apparent',
      GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/stream-engine.yml@refs/pull/123/merge',
      APPARENT_DELIVERY_WORKFLOW_PATH: '.github/workflows/stream-engine.yml',
    }
    assert.doesNotThrow(() => assertBuildWorkflowIdentity(env, id))
    assert.throws(() => assertBuildWorkflowIdentity({ ...env,
      GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/stream-agent.yml@refs/pull/123/merge',
    }, id), /canonical owner workflow/)
    assert.throws(() => assertBuildWorkflowIdentity({ ...env,
      APPARENT_DELIVERY_WORKFLOW_PATH: '.github/workflows/stream-engine-native.yml',
    }, id), /canonical owner workflow/)
  }
})


test('PR SDK candidates use separate names and canonical PR identity, then reissue exact-main bytes', () => {
  const sdk = { ...owner, id: 'engine', package: 'engine/sdk', inputs: ['engine/'] }
  const input = resolve(temp, 'sdk-candidate-input'); mkdirSync(input); writeFileSync(resolve(input, 'sdk.tgz'), 'checked SDK')
  const candidateDir = resolve(temp, 'sdk-candidate')
  const prEnv = { GITHUB_EVENT_NAME: 'pull_request', GITHUB_REF: 'refs/pull/9/merge', GITHUB_REPOSITORY: 'tryapparent/apparent', GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/stream-engine.yml@refs/pull/9/merge', GITHUB_RUN_ID: '555', GITHUB_RUN_ATTEMPT: '2' }
  const receipt = sealPayload(repo, sdk, 'linux-x64', input, candidateDir, 'b'.repeat(64), prEnv, true)
  const zip = resolve(temp, 'sdk-candidate.zip'); execFileSync('zip', ['-q', zip, 'component.tar.gz', 'receipt.json'], { cwd: candidateDir })
  const mainEnv = { ...prEnv, GITHUB_REF: 'refs/heads/main', GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/stream-engine.yml@refs/heads/main', GITHUB_RUN_ID: '556', GITHUB_RUN_ATTEMPT: '1' }
  const reissued = reusePayload(repo, sdk, 'linux-x64', zip, resolve(temp, 'sdk-promoted'), receipt.inputDigest, receipt.sourceSha, '88', '555', '2', mainEnv)
  assert.equal(reissued.runId, '556'); assert.equal(reissued.reusedFrom.runId, '555')
  assert.equal(reissued.sha256, receipt.sha256)
  assert.throws(() => reusePayload(repo, sdk, 'linux-x64', zip, resolve(temp, 'sdk-wrong-attempt'), receipt.inputDigest, receipt.sourceSha, '88', '555', '1', mainEnv), /run does not match/)
  assert.throws(() => sealPayload(repo, sdk, 'linux-x64', input, resolve(temp, 'not-pr'), receipt.inputDigest, { ...prEnv, GITHUB_EVENT_NAME: 'workflow_dispatch' }, true), /canonical/)
  assert.throws(() => sealPayload(repo, owner, 'linux-x64', input, resolve(temp, 'not-sdk'), receipt.inputDigest, prEnv, true), /canonical/)
  assert.throws(() => sealPayload(repo, sdk, 'linux-x64', input, resolve(temp, 'not-main'), receipt.inputDigest, prEnv), /canonical/)
  writeFileSync(resolve(candidateDir, 'receipt.json'), JSON.stringify({ ...receipt, sourceDigest: '0'.repeat(64) }))
  const wrongSource = resolve(temp, 'sdk-wrong-source.zip')
  execFileSync('zip', ['-q', wrongSource, 'component.tar.gz', 'receipt.json'], { cwd: candidateDir })
  assert.throws(() => reusePayload(repo, sdk, 'linux-x64', wrongSource, resolve(temp, 'sdk-reject-source'), receipt.inputDigest, receipt.sourceSha, '88', '555', '2', mainEnv), /Component source changed/)
})


test('native identity covers actual compilers, SDK and byte-affecting environment; unknown identity fails closed', () => {
  const probe = (command: string, args: string[]) => command + ' ' + args.join(' ') + ' version 1'
  const baseline = nativeBuildIdentity({}, 'linux', probe)
  assert.notDeepEqual(nativeBuildIdentity({}, 'linux', (c, a) => probe(c, a) + ' changed'), baseline)
  for (const name of ['RUSTFLAGS', 'CFLAGS', 'CXXFLAGS', 'CPPFLAGS', 'LDFLAGS', 'RUSTDOCFLAGS', 'CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER', 'CARGO_PROFILE_RELEASE_OPT_LEVEL', 'CC_x86_64_unknown_linux_gnu', 'CFLAGS_x86_64_unknown_linux_gnu']) {
    assert.notDeepEqual(nativeBuildIdentity({ [name]: 'changed' }, 'linux', probe), baseline, name)
  }
  assert.notDeepEqual(nativeBuildIdentity({}, 'darwin', probe), baseline)
  assert.throws(() => nativeBuildIdentity({}, 'linux', () => ''), /identity/)
  assert.throws(() => nativeBuildIdentity({}, 'linux', () => { throw Error('missing compiler') }), /missing compiler/)
  assert.throws(() => nativeBuildIdentity({ RUSTC_WRAPPER: 'unknown-wrapper' }, 'linux', probe), /wrappers/)
})

test('native promotion preserves archived build SHA across a different admitted main SHA', () => {
  const root = resolve(temp, 'native-provenance-repo'); newRepo(root)
  const native = { ...owner, id: 'engine-native', kind: 'directory' as const, language: 'native' as const, package: 'engine/delivery/native', inputs: ['engine/'], targets: ['linux-x64', 'macos-x64'] as const }
  const nativeOwner: Owner = { ...native, targets: [...native.targets] }
  const input = resolve(temp, 'native-provenance-payload'); mkdirSync(input)
  const buildSha = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim()
  writeFileSync(resolve(input, 'manifest.json'), JSON.stringify({ sourceSha: buildSha }))
  writeFileSync(resolve(input, 'apparent-engine'), 'synthetic native payload')
  const prEnv = { GITHUB_EVENT_NAME: 'pull_request', GITHUB_REF: 'refs/pull/9/merge', GITHUB_REPOSITORY: 'tryapparent/apparent', GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/stream-engine.yml@refs/pull/9/merge', GITHUB_RUN_ID: '777', GITHUB_RUN_ATTEMPT: '1' }
  const sealed = resolve(temp, 'native-provenance-sealed')
  const original = sealPayload(root, nativeOwner, 'macos-x64', input, sealed, 'd'.repeat(64), prEnv, true)
  assert.equal(original.buildSourceSha, buildSha)
  const zip = resolve(temp, 'native-provenance.zip'); execFileSync('zip', ['-q', zip, 'component.tar.gz', 'receipt.json'], { cwd: sealed })
  git(root, 'commit', '--allow-empty', '-qm', 'landed candidate')
  const mainSha = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim()
  const mainEnv = { ...prEnv, GITHUB_REF: 'refs/heads/main', GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/stream-engine.yml@refs/heads/main', GITHUB_RUN_ID: '778' }
  const promoted = resolve(temp, 'native-provenance-promoted')
  const receipt = reusePayload(root, nativeOwner, 'macos-x64', zip, promoted, original.inputDigest, buildSha, '98', '777', '1', mainEnv)
  assert.equal(receipt.sourceSha, mainSha); assert.equal(receipt.buildSourceSha, buildSha)
  assert.deepEqual(readFileSync(resolve(promoted, 'component.tar.gz')), readFileSync(resolve(sealed, 'component.tar.gz')))
  const manifest = execFileSync('tar', ['-xOzf', resolve(promoted, 'component.tar.gz'), 'native-provenance-payload/manifest.json'], { encoding: 'utf8' })
  assert.equal(JSON.parse(manifest).sourceSha, buildSha)
})

test('native fingerprints invalidate when the actual compiler output or flags change', () => {
  const root = resolve(temp, 'native-fingerprint'); newRepo(root)
  const bin = resolve(temp, 'native-toolchain-bin'); mkdirSync(bin)
  for (const tool of ['rustc', 'cargo', 'cc']) { const path = resolve(bin, tool); writeFileSync(path, '#!/bin/sh\nprintf "toolchain version one\\n"\n'); chmodSync(path, 0o755) }
  const nativeOwner: Owner = { ...owner, id: 'engine-native', package: 'engine/delivery/native', inputs: ['engine/'], language: 'native' }
  const saved = { PATH: process.env.PATH, RUSTFLAGS: process.env.RUSTFLAGS, CC: process.env.CC, RUSTC: process.env.RUSTC }
  try {
    process.env.PATH = `${bin}:${process.env.PATH}`; delete process.env.CC; delete process.env.RUSTC
    const initial = inputDigest(root, nativeOwner)
    writeFileSync(resolve(bin, 'rustc'), '#!/bin/sh\nprintf "toolchain version two\\n"\n')
    assert.notEqual(inputDigest(root, nativeOwner), initial)
    const changedCompiler = inputDigest(root, nativeOwner)
    process.env.RUSTFLAGS = '-C opt-level=2'
    assert.notEqual(inputDigest(root, nativeOwner), changedCompiler)
    writeFileSync(resolve(bin, 'rustc'), '#!/bin/sh\nexit 1\n')
    assert.throws(() => inputDigest(root, nativeOwner))
  } finally { for (const [key, value] of Object.entries(saved)) { if (value === undefined) delete process.env[key]; else process.env[key] = value } }
})


test('App Shell reuse fingerprints packaging overrides without unrelated workflow settings', () => {
  const baseline = packageBuildEnvironment({}, true)
  for (const name of ['ELECTRON_OVERRIDE_DIST_PATH', 'ELECTRON_MIRROR', 'ELECTRON_RUN_AS_NODE', 'npm_config_arch', 'NPM_CONFIG_PLATFORM', 'NODE_OPTIONS', 'NODE_ENV', 'NODE_PATH', 'ESBUILD_BINARY_PATH']) {
    assert.notDeepEqual(packageBuildEnvironment({ [name]: 'changed' }, true), baseline, name)
  }
  assert.deepEqual(packageBuildEnvironment({ GITHUB_RUN_ID: 'different', PUBLIC_POSTHOG_TOKEN: 'synthetic' }, true), baseline)
  assert.equal(supportsPrCandidate('app-shell', 'linux-x64'), true)
  assert.equal(supportsPrCandidate('app-shell', 'macos-x64'), true)
  assert.equal(supportsPrCandidate('app-shell', 'web'), false)
})

test('App Shell promotion retains the exact packaged archive and original build provenance', () => {
  const app: Owner = { ...owner, id: 'app-shell', kind: 'app-shell', package: 'composition/app', inputs: ['composition/app/'], targets: ['linux-x64', 'macos-x64'], stage: 'development-app' }
  const input = resolve(temp, 'app-package-candidate'); mkdirSync(resolve(input, 'bundle'), { recursive: true })
  writeFileSync(resolve(input, 'bundle/app'), 'synthetic packaged runtime bytes')
  const prEnv = { GITHUB_EVENT_NAME: 'pull_request', GITHUB_REF: 'refs/pull/9/merge', GITHUB_REPOSITORY: 'tryapparent/apparent', GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/stream-platform.yml@refs/pull/9/merge', GITHUB_RUN_ID: '887', GITHUB_RUN_ATTEMPT: '1' }
  for (const target of app.targets) {
    const sealed = resolve(temp, 'app-sealed-' + target), promoted = resolve(temp, 'app-promoted-' + target)
    const receipt = sealPayload(repo, app, target, input, sealed, 'a'.repeat(64), prEnv, true)
    const zip = resolve(temp, 'app-' + target + '.zip'); execFileSync('zip', ['-q', zip, 'component.tar.gz', 'receipt.json'], { cwd: sealed })
    const next = reusePayload(repo, app, target, zip, promoted, receipt.inputDigest, receipt.sourceSha, '99', '887', '1', { ...prEnv, GITHUB_REF: 'refs/heads/main', GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/stream-platform.yml@refs/heads/main', GITHUB_RUN_ID: '888' })
    assert.equal(next.buildSourceSha, receipt.sourceSha); assert.equal(next.reusedFrom.runId, '887')
    assert.deepEqual(readFileSync(resolve(promoted, 'component.tar.gz')), readFileSync(resolve(sealed, 'component.tar.gz')))
  }
})


test('media identity invalidates on installed runtime, fixture tool, Swift and loader changes', () => {
  const probe = (command: string, args: string[]) => command === 'dpkg-query' ? 'liba=1\nlibb=2' : command + ' ' + args.join(' ') + ' version 1'
  for (const os of ['linux', 'darwin'] as const) {
    const baseline = mediaBuildIdentity({}, os, probe)
    const commands = os === 'linux' ? ['dpkg-query', 'ffmpeg', 'pkg-config', 'ldd'] : ['ffmpeg', 'xcrun', 'sw_vers']
    for (const command of commands) assert.notDeepEqual(mediaBuildIdentity({}, os, (c, a) => probe(c, a) + (c === command ? ' changed' : '')), baseline, command)
    for (const key of ['GST_PLUGIN_PATH', 'LD_LIBRARY_PATH', 'DYLD_LIBRARY_PATH', 'PKG_CONFIG_PATH', 'SWIFTFLAGS', 'TOOLCHAINS', 'GSTREAMER_1_0_NO_PKG_CONFIG', 'SYSTEM_DEPS_GSTREAMER_1_0_LIB']) {
      assert.notDeepEqual(mediaBuildIdentity({ [key]: 'changed' }, os, probe), baseline, key)
    }
    assert.throws(() => mediaBuildIdentity({}, os, () => ''), /identity/)
    assert.throws(() => mediaBuildIdentity({}, os, () => { throw Error('missing dependency') }), /missing dependency/)
  }
  assert.deepEqual(mediaBuildIdentity({}, 'linux', probe), mediaBuildIdentity({}, 'linux', (c, a) => c === 'dpkg-query' ? 'libb=2\nliba=1' : probe(c, a)), 'package ordering is not an input change')
})


test('media fingerprint includes the actual dependency probe results and media overrides', () => {
  const root = resolve(temp, 'media-fingerprint'); newRepo(root)
  const bin = resolve(temp, 'media-toolchain-bin'); mkdirSync(bin)
  for (const tool of ['rustc', 'cargo', 'cc', 'ffmpeg', 'pkg-config', 'dpkg-query', 'ldd', 'xcrun', 'sw_vers']) { const path = resolve(bin, tool); writeFileSync(path, '#!/bin/sh\nprintf "media dependency version one\\n"\n'); chmodSync(path, 0o755) }
  const media: Owner = { ...owner, id: 'engine-media-native', package: 'engine/delivery/media', inputs: ['engine/'], language: 'native' }
  const saved = { PATH: process.env.PATH, RUSTC: process.env.RUSTC, CC: process.env.CC, GST_PLUGIN_PATH: process.env.GST_PLUGIN_PATH }
  try {
    process.env.PATH = bin + ':' + process.env.PATH; delete process.env.CC; delete process.env.RUSTC
    const initial = inputDigest(root, media)
    writeFileSync(resolve(bin, 'ffmpeg'), '#!/bin/sh\nprintf "media dependency version two\\n"\n')
    assert.notEqual(inputDigest(root, media), initial)
    const changed = inputDigest(root, media); process.env.GST_PLUGIN_PATH = '/synthetic/override'
    assert.notEqual(inputDigest(root, media), changed)
    writeFileSync(resolve(bin, 'ffmpeg'), '#!/bin/sh\nexit 1\n')
    assert.throws(() => inputDigest(root, media))
  } finally { for (const [key, value] of Object.entries(saved)) { if (value === undefined) delete process.env[key]; else process.env[key] = value } }
})

test('Config fingerprints include compiler and flag changes before candidate admission', () => {
  const root = resolve(temp, 'config-fingerprint'); newRepo(root)
  const bin = resolve(temp, 'config-toolchain-bin'); mkdirSync(bin)
  for (const tool of ['rustc', 'cargo', 'cc', 'xcrun']) { const path = resolve(bin, tool); writeFileSync(path, '#!/bin/sh\nprintf "config compiler version one\\n"\n'); chmodSync(path, 0o755) }
  const config: Owner = { ...owner, id: 'platform-config', package: 'composition/config/native', inputs: ['composition/config/', 'shared/config-native/', 'shared/contracts/', '.cargo/'], language: 'native' }
  const saved = { PATH: process.env.PATH, RUSTC: process.env.RUSTC, CC: process.env.CC, CFLAGS: process.env.CFLAGS }
  try {
    process.env.PATH = bin + ':' + process.env.PATH; delete process.env.CC; delete process.env.RUSTC
    const initial = inputDigest(root, config)
    writeFileSync(resolve(bin, 'rustc'), '#!/bin/sh\nprintf "config compiler version two\\n"\n')
    assert.notEqual(inputDigest(root, config), initial)
    const changed = inputDigest(root, config); process.env.CFLAGS = '-DSYNTHETIC_CONFIG_TEST=1'
    assert.notEqual(inputDigest(root, config), changed)
    writeFileSync(resolve(bin, 'rustc'), '#!/bin/sh\nexit 1\n')
    assert.throws(() => inputDigest(root, config))
  } finally { for (const [key, value] of Object.entries(saved)) { if (value === undefined) delete process.env[key]; else process.env[key] = value } }
})

test('Config candidate sealing retains each staged helper manifest under its own workflow', () => {
  const config: Owner = { ...owner, id: 'platform-config', kind: 'directory', package: 'composition/config/native', inputs: ['composition/config/'], language: 'native', targets: ['linux-x64', 'macos-x64'] }
  const input = resolve(temp, 'config-helper-payload'); mkdirSync(input)
  writeFileSync(resolve(input, 'manifest.json'), JSON.stringify({ sourceSha: 'b'.repeat(40), protocol: 1 }))
  writeFileSync(resolve(input, 'apparent-config-host'), 'synthetic helper')
  const prEnv = { GITHUB_EVENT_NAME: 'pull_request', GITHUB_REF: 'refs/pull/9/merge', GITHUB_REPOSITORY: 'tryapparent/apparent', GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/stream-platform.yml@refs/pull/9/merge', GITHUB_RUN_ID: '997', GITHUB_RUN_ATTEMPT: '1' }
  for (const target of config.targets) {
    const sealed = resolve(temp, 'config-helper-sealed-' + target)
    const receipt = sealPayload(repo, config, target, input, sealed, 'c'.repeat(64), prEnv, true)
    assert.equal(receipt.buildSourceSha, receipt.sourceSha)
    const manifest = execFileSync('tar', ['-xOzf', resolve(sealed, 'component.tar.gz'), 'config-helper-payload/manifest.json'], { encoding: 'utf8' })
    assert.equal(JSON.parse(manifest).sourceSha, 'b'.repeat(40))
  }
  assert.throws(() => sealPayload(repo, config, 'linux-x64', input, resolve(temp, 'config-wrong-workflow'), 'c'.repeat(64), { ...prEnv, GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/stream-engine.yml@refs/pull/9/merge' }, true), /canonical/)
})

test('Onboarding fingerprints package build overrides without probing a native toolchain', () => {
  const root = resolve(temp, 'onboarding-fingerprint'); newRepo(root)
  const bin = resolve(temp, 'onboarding-no-native'); mkdirSync(bin)
  for (const tool of ['rustc', 'cargo']) { const path = resolve(bin, tool); writeFileSync(path, '#!/bin/sh\nexit 99\n'); chmodSync(path, 0o755) }
  const onboarding: Owner = { ...owner, id: 'onboarding', package: 'onboarding', inputs: ['onboarding/'] }
  const saved = { PATH: process.env.PATH, ESBUILD_BINARY_PATH: process.env.ESBUILD_BINARY_PATH, npm_config_ignore_scripts: process.env.npm_config_ignore_scripts }
  try {
    process.env.PATH = bin + ':' + process.env.PATH
    const initial = inputDigest(root, onboarding)
    process.env.ESBUILD_BINARY_PATH = '/synthetic/pinned-esbuild'
    assert.notEqual(inputDigest(root, onboarding), initial)
    const changed = inputDigest(root, onboarding); process.env.npm_config_ignore_scripts = 'true'
    assert.notEqual(inputDigest(root, onboarding), changed)
  } finally { for (const [key, value] of Object.entries(saved)) { if (value === undefined) delete process.env[key]; else process.env[key] = value } }
  assert.deepEqual(packageBuildEnvironment({ ELECTRON_MIRROR: 'unrelated' }), packageBuildEnvironment({}))
})

test('Onboarding promotion retains its verified npm package and build provenance on both targets', () => {
  const onboarding: Owner = { ...owner, id: 'onboarding', package: 'onboarding', inputs: ['onboarding/'], targets: ['linux-x64', 'macos-x64'] }
  const input = resolve(temp, 'onboarding-candidate'); mkdirSync(input)
  writeFileSync(resolve(input, 'apparent-onboarding.tgz'), 'synthetic packaged JS and CSS')
  const prEnv = { GITHUB_EVENT_NAME: 'pull_request', GITHUB_REF: 'refs/pull/9/merge', GITHUB_REPOSITORY: 'tryapparent/apparent', GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/stream-onboarding.yml@refs/pull/9/merge', GITHUB_RUN_ID: '1001', GITHUB_RUN_ATTEMPT: '1' }
  for (const target of onboarding.targets) {
    const sealed = resolve(temp, 'onboarding-sealed-' + target), promoted = resolve(temp, 'onboarding-promoted-' + target)
    const receipt = sealPayload(repo, onboarding, target, input, sealed, 'a'.repeat(64), prEnv, true)
    const zip = resolve(temp, 'onboarding-' + target + '.zip'); execFileSync('zip', ['-q', zip, 'component.tar.gz', 'receipt.json'], { cwd: sealed })
    const next = reusePayload(repo, onboarding, target, zip, promoted, receipt.inputDigest, receipt.sourceSha, '99', '1001', '1', { ...prEnv, GITHUB_REF: 'refs/heads/main', GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/stream-onboarding.yml@refs/heads/main', GITHUB_RUN_ID: '1002' })
    assert.equal(next.buildSourceSha, receipt.sourceSha); assert.equal(next.reusedFrom.runId, '1001')
    assert.deepEqual(readFileSync(resolve(promoted, 'component.tar.gz')), readFileSync(resolve(sealed, 'component.tar.gz')))
  }
})

test('PR input equivalence survives unrelated main advances but rejects changed inputs and divergent bases', () => {
  const root = resolve(temp, 'advanced-main'); const base = newRepo(root)
  const rev = (name: string) => execFileSync('git', ['rev-parse', name], { cwd: root, encoding: 'utf8' }).trim()
  git(root, 'checkout', '-qb', 'candidate')
  writeFileSync(resolve(root, 'record/index.ts'), 'PR change'); git(root, 'add', '.'); git(root, 'commit', '-qm', 'PR')
  const head = rev('HEAD')
  const tested = execFileSync('git', ['commit-tree', rev('HEAD^{tree}'), '-p', base, '-p', head, '-m', 'tested merge'], { cwd: root, encoding: 'utf8' }).trim()
  git(root, 'checkout', '-qB', 'main', base)
  writeFileSync(resolve(root, 'unrelated.txt'), 'main advanced'); git(root, 'add', '.'); git(root, 'commit', '-qm', 'unrelated main')
  const landedBase = rev('HEAD')
  git(root, 'merge', '--no-ff', '-qm', 'land PR', head)
  git(root, 'update-ref', 'refs/pull/9/merge', tested)
  const receiver = resolve(temp, 'main-only-receiver')
  git(temp, 'clone', '--no-local', '--single-branch', '--branch', 'main', root, receiver)
  assert.throws(() => execFileSync('git', ['cat-file', '-e', tested], { cwd: receiver, stdio: 'ignore' }))
  const receiverHead = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: receiver, encoding: 'utf8' }).trim()
  assert.equal(matchPrCandidateInputs(receiver, owner, tested, base, landedBase), true)
  assert.equal(execFileSync('git', ['rev-parse', 'HEAD'], { cwd: receiver, encoding: 'utf8' }).trim(), receiverHead)
  assert.equal(execFileSync('git', ['status', '--porcelain'], { cwd: receiver, encoding: 'utf8' }), '')
  git(receiver, 'remote', 'set-url', 'origin', resolve(temp, 'missing-origin'))
  assert.equal(matchPrCandidateInputs(receiver, owner, '0'.repeat(40), base, landedBase), false)
  assert.notEqual(rev('HEAD^{tree}'), rev(tested + '^{tree}'))
  assert.equal(prInputsMatch(root, owner, tested, base, landedBase), true)
  assert.equal(prInputsMatch(root, owner, tested, head, landedBase), false)
  assert.equal(prInputsMatch(root, owner, tested, base, head), true)
  assert.equal(prInputsMatch(root, owner, tested, base, '0'.repeat(40)), false)
  assert.equal(prInputsMatch(root, owner, 'missing', base, landedBase), false)
  git(root, 'checkout', '--orphan', 'divergent'); git(root, 'commit', '-qm', 'disconnected history')
  assert.equal(prInputsMatch(root, owner, tested, base, rev('HEAD')), false)
  git(root, 'checkout', 'main')
  for (const name of ['record/index.ts', 'shared/ui/theme.ts', 'composition/delivery/core.ts', 'composition/architecture/guard.ts', workflowPathFor(owner.id), 'enablement/verification/workflow-budget.ts', 'LICENSE', 'record/delivery.json']) {
    writeFileSync(resolve(root, name), 'changed relevant input'); git(root, 'add', '.'); git(root, 'commit', '-qm', 'relevant change')
    assert.equal(prInputsMatch(root, owner, tested, base, landedBase), false, name)
    git(root, 'reset', '--hard', 'HEAD^')
  }
  for (const change of [() => git(root, 'mv', 'record/index.ts', 'record/renamed.ts'), () => git(root, 'rm', 'record/index.ts'), () => chmodSync(resolve(root, 'record/index.ts'), 0o755)]) {
    change(); git(root, 'add', '-A'); git(root, 'commit', '-qm', 'inventory change')
    assert.equal(prInputsMatch(root, owner, tested, base, landedBase), false)
    git(root, 'reset', '--hard', 'HEAD^')
  }
})

test('App Shell fingerprints its external verification inputs', () => {
  const root = resolve(temp, 'app-shell-external-inputs'); newRepo(root)
  const app: Owner = { ...JSON.parse(readFileSync(resolve(repo, 'composition/app/delivery.json'), 'utf8')), language: 'typescript' }
  for (const name of ['enablement/native/owned-processes.ts', '.github/workflows/verification-platform.yml']) {
    assert(app.files?.includes(name))
    const before = sourceDigest(root, app)
    mkdirSync(resolve(root, name, '..'), { recursive: true }); writeFileSync(resolve(root, name), 'verification input')
    git(root, 'add', '.'); git(root, 'commit', '-qm', 'external verification input')
    assert.notEqual(sourceDigest(root, app), before, name)
  }
})
