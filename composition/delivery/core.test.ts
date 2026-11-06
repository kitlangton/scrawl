import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { chmodSync, existsSync, mkdtempSync, mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
import { stripTypeScriptTypes } from 'node:module'
import { after, test } from 'node:test'
import { assertBuildWorkflowIdentity } from './cli.ts'
import { assessPrior, catalog, inputDigest, makePlan, optionalVerificationPeers, reusePayload, sealPayload, verifyPublicExports, type Owner, workflowPathFor } from './core.ts'

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
