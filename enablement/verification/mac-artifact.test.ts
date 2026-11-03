import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, copyFileSync, writeFileSync, readFileSync, readdirSync, chmodSync, rmSync, existsSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createHash } from 'node:crypto'
import { admit, verify } from './mac-artifact-consumer.ts'
import { produce, audit, producerOptions } from './mac-artifact.ts'
const developer = '/Applications/Xcode_26.0.app/Contents/Developer'
const loads = 'cmd LC_BUILD_VERSION\ncmdsize 32\nplatform 1\nminos 15.0\nsdk 26.0\ncmd LC_LOAD_DYLIB\ncmdsize 56\nname /usr/lib/libSystem.B.dylib (offset 24)'
function fixture(fn: (run: (override?: Record<string, string>, os?: string, kind?: Parameters<typeof produce>[6]) => ReturnType<typeof produce>, root: string) => void) {
  const root = mkdtempSync(join(tmpdir(), 'mac-producer-'))
  mkdirSync(join(root, '.cargo')); copyFileSync('.cargo/config.toml', join(root, '.cargo/config.toml'))
  mkdirSync(join(root, 'target/debug'), { recursive: true })
  for (const name of ['apparent', 'apparent-helper']) { writeFileSync(join(root, 'target/debug', name), name); chmodSync(join(root, 'target/debug', name), 0o755) }
  writeFileSync(join(root, 'Cargo.lock'), 'synthetic lock')
  const env = { GITHUB_REPOSITORY: 'tryapparent/apparent', GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/verification-mac.yml@refs/heads/main', GITHUB_WORKFLOW_SHA: 'b'.repeat(40), GITHUB_RUN_ID: '123', GITHUB_RUN_ATTEMPT: '1', GITHUB_JOB: 'rust' }
  const probes: Record<string, string> = { 'git status --porcelain --untracked-files=normal': '', 'git rev-parse HEAD': 'a'.repeat(40), 'git rev-parse HEAD^{tree}': 'c'.repeat(40), 'rustc -vV': 'rustc 1.90.0\nhost: x86_64-apple-darwin\nrelease: 1.90.0', 'cargo -V': 'cargo 1.90.0 (abc)', 'xcode-select -p': developer, 'xcodebuild -version': 'Xcode 26.0\nBuild version 17A123', 'xcrun --sdk macosx --show-sdk-version': '26.0', 'xcrun --sdk macosx --show-sdk-path': `${developer}/Platforms/MacOSX.platform/Developer/SDKs/MacOSX26.0.sdk`, 'xcrun swiftc --version': 'Apple Swift version 6.2\nTarget: x86_64-apple-macosx15.0' }
  try { fn((override = {}, os = 'darwin', kind = 'verified-main') => produce(root, join(root, 'payload'), { ...env, ...Object.fromEntries(Object.entries(override).filter(([k]) => !k.includes(' '))) }, (c, args) => {
    const key = [c, ...args].join(' ')
    if (key in override) return override[key]
    if (c === 'lipo') return 'x86_64'
    if (c === 'otool') return override.loads ?? loads
    assert.ok(key in probes, key); return probes[key]
  }, os as 'darwin', 'x64', kind), root) } finally { rmSync(root, { recursive: true, force: true }) }
}
test('production creator emits exact fixed payload, checkout identity, hashes and modes', () => fixture((run, root) => {
  const m = run(); assert.equal(m.source.sha, 'a'.repeat(40)); assert.equal(m.producer.workflow_sha, 'b'.repeat(40))
  assert.equal(Object.hasOwn(m.producer, 'kind'), false, 'Legacy main identity stays unchanged')
  assert.deepEqual(readdirSync(join(root, 'payload')).sort(), ['apparent', 'apparent-helper', 'manifest.json'])
  for (const f of m.files) { assert.equal(f.mode, 0o755); assert.equal(f.sha256, createHash('sha256').update(readFileSync(join(root, 'payload', f.name))).digest('hex')) }
  assert.deepEqual(JSON.parse(readFileSync(join(root, 'payload/manifest.json'), 'utf8')), m)
}))
const focused = { GITHUB_EVENT_NAME: 'workflow_dispatch', GITHUB_REF: 'refs/heads/t3/candidate',
  GITHUB_SHA: 'a'.repeat(40), GITHUB_WORKFLOW_SHA: 'a'.repeat(40), GITHUB_JOB: 'settings-build',
  GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/verification-mac.yml@refs/heads/t3/candidate' }
test('focused creator emits its explicit kind with the same bounded payload contract', () => fixture((run, root) => {
  const m = run(focused, 'darwin', 'focused-settings-build')
  assert.equal(m.producer.kind, 'focused-settings-build'); assert.equal(m.producer.job, 'settings-build')
  assert.equal(m.producer.workflow, focused.GITHUB_WORKFLOW_REF)
  assert.equal(m.source.sha, focused.GITHUB_SHA); assert.equal(m.producer.workflow_sha, m.source.sha)
  assert.equal(m.kind, 'mac-dev-executables'); assert.equal(m.schema, 1)
  assert.deepEqual(readdirSync(join(root, 'payload')).sort(), ['apparent', 'apparent-helper', 'manifest.json'])
  assert.deepEqual(JSON.parse(readFileSync(join(root, 'payload/manifest.json'), 'utf8')), m)
}))
for (const [label, override] of Object.entries<Record<string, string>>({
  event: { GITHUB_EVENT_NAME: 'workflow_run' }, main: { GITHUB_REF: 'refs/heads/main' },
  tag: { GITHUB_REF: 'refs/tags/v1' }, traversal: { GITHUB_REF: 'refs/heads/../candidate' },
  whitespace: { GITHUB_REF: 'refs/heads/t3/candidate\n' }, repository: { GITHUB_REPOSITORY: 'fork/apparent' },
  job: { GITHUB_JOB: 'settings-native' }, jobWhitespace: { GITHUB_JOB: 'settings-build\n' },
  checkout: { GITHUB_SHA: 'b'.repeat(40) }, workflowSha: { GITHUB_WORKFLOW_SHA: 'b'.repeat(40) },
  workflow: { GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/verification.yml@refs/heads/t3/candidate' },
  branchMismatch: { GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/verification-mac.yml@refs/heads/t3/other' },
  environmentSelector: { APPARENT_PREBUILT_KIND: 'focused-settings-build' },
  flags: { CARGO_PROFILE_DEV_OPT_LEVEL: '0' },
})) {
  test(`focused producer rejects ${label} before writing`, () => fixture((run, root) => {
    assert.throws(() => run({ ...focused, ...override }, 'darwin', 'focused-settings-build'))
    assert.equal(existsSync(join(root, 'payload')), false)
  }))
}
test('focused context cannot enter the legacy main creator without its explicit kind', () => fixture(run => assert.throws(() => run(focused))))
test('producer CLI kind is explicit and malformed arguments cannot fall back to main', () => {
  assert.deepEqual(producerOptions(['/tmp/output']), { kind: 'verified-main', output: '/tmp/output' })
  assert.deepEqual(producerOptions(['--producer-kind', 'focused-settings-build', '/tmp/output']), { kind: 'focused-settings-build', output: '/tmp/output' })
  for (const args of [[], ['--producer-kind'], ['--producer-kind', 'other', '/tmp/output'],
    ['--producer-kind', 'focused-settings-build'], ['/tmp/output', 'extra'], ['--unknown', '/tmp/output']]) assert.throws(() => producerOptions(args))
})
for (const [label, override] of Object.entries<Record<string, string>>({ dirty: { 'git status --porcelain --untracked-files=normal': ' M build.rs' }, identity: { GITHUB_RUN_ID: '../123' }, config: { RUSTFLAGS: '-C opt-level=3' }, sdk: { 'xcrun --sdk macosx --show-sdk-version': '27.0' }, sha: { 'git rev-parse HEAD': 'main' }, dependency: { loads: loads.replace('/usr/lib/libSystem.B.dylib', '/tmp/evil.dylib') } })) {
  test(`rejects ${label}`, () => fixture(run => assert.throws(() => run(override))))
}
test('wrong platform', () => fixture(run => assert.throws(() => run({}, 'linux'))))
test('missing helper', () => fixture((run, root) => { rmSync(join(root, 'target/debug/apparent-helper')); assert.throws(() => run()) }))
test('unsafe mode', () => fixture((run, root) => { chmodSync(join(root, 'target/debug/apparent'), 0o777); assert.throws(() => run()) }))
test('rejects unreviewed config', () => fixture((run, root) => { writeFileSync(join(root, '.cargo/config.toml'), '[build]'); assert.throws(() => run()) }))
test('runtime closure rejects traversal, loader paths, unknown commands and Xcode-only Swift', () => {
  for (const path of ['/usr/lib/../evil', '@loader_path/evil', '@rpath/libswiftCore.dylib']) assert.throws(() => audit(loads.replace('/usr/lib/libSystem.B.dylib', path), developer, () => true))
  assert.throws(() => audit(loads.replace('LC_LOAD_DYLIB', 'LC_ID_DYLIB'), developer))
  assert.throws(() => audit(loads + '\ncmd LC_RPATH\ncmdsize 32\npath /tmp (offset 12)', developer))
  const swift = loads.replace('/usr/lib/libSystem.B.dylib', '@rpath/libswiftCore.dylib') + '\ncmd LC_RPATH\ncmdsize 32\npath /usr/lib/swift (offset 12)'
  assert.equal(audit(swift, developer, () => true).closure[0].provider, 'macOS')
  assert.throws(() => audit(swift, developer, () => false))
})
test('workflow retains success-only producer after existing evidence and before final job', () => {
  const workflow = readFileSync('.github/workflows/verification-platform.yml', 'utf8')
  const segment = workflow.slice(workflow.indexOf('      - name: Retain verified Intel'))
  assert.match(segment, /if: success\(\) && runner.os == 'macOS'/)
  assert.match(segment, /if-no-files-found: error/)
  assert.ok(workflow.indexOf('name: verification-receipts-rust-') < workflow.indexOf('      - name: Retain verified Intel'))
  assert.match(segment, /needs: \[select, desktop-ui, connect-client, connect-web, rust\]/)
})

test('share build kind is bound to its own workflow and job without changing Settings', () => fixture((run) => {
  const env = { ...focused, GITHUB_JOB: 'share-build', GITHUB_WORKFLOW_REF: 'tryapparent/apparent/.github/workflows/native-proof-mac.yml@refs/heads/t3/candidate' }
  assert.throws(() => run(env, 'darwin', 'focused-settings-build'))
  assert.throws(() => run(focused, 'darwin', 'focused-share-build'))
  const manifest = run(env, 'darwin', 'focused-share-build')
  assert.equal(manifest.producer.kind, 'focused-share-build')
  assert.equal(manifest.producer.job, 'share-build')
  assert.equal(manifest.producer.workflow, env.GITHUB_WORKFLOW_REF)
  assert.equal(producerOptions(['--producer-kind', 'focused-share-build', '/fresh']).kind, 'focused-share-build')
}))

for (const kind of ['focused-settings-build', 'focused-share-build'] as const) test(`${kind} emitted manifest passes the real consumer contract`, () => fixture((run, root) => {
  const sharing = kind === 'focused-share-build'
  const jobName = sharing ? 'share-build' : 'settings-build'
  const workflow = sharing ? 'native-proof-mac.yml' : 'verification-mac.yml'
  const env = { ...focused, GITHUB_JOB: jobName, GITHUB_WORKFLOW_REF: `tryapparent/apparent/.github/workflows/${workflow}@refs/heads/t3/candidate` }
  const manifest = run(env, 'darwin', kind)
  const repository = { full_name: 'tryapparent/apparent' }
  const upstream = { id: 123, run_attempt: 1, repository, head_repository: repository, path: `.github/workflows/${workflow}`, head_branch: 't3/candidate', head_sha: manifest.source.sha, event: 'workflow_dispatch', status: 'in_progress', conclusion: null }
  const artifact = { id: 456, name: 'mac-dev-executables-123-1', workflow_run: { id: 123 }, expired: false, digest: 'sha256:' + 'f'.repeat(64) }
  const job = { name: jobName, run_id: 123, run_attempt: 1, status: 'completed', conclusion: 'success' }
  const identity = admit(upstream, artifact, [job], '123', '456', { kind, run: '123', attempt: '1', sha: manifest.source.sha, head: manifest.source.sha, tree: manifest.source.tree, workflow: manifest.producer.workflow, workflowSha: manifest.source.sha, job: sharing ? 'connect-worker' : 'settings-native', event: 'workflow_dispatch' })
  verify(join(root, 'payload'), manifest, { sha: manifest.source.sha, tree: manifest.source.tree, lock: manifest.cargo_lock_sha256, config: manifest.config_sha256 }, identity)
}))

test('selected Xcode 26.0.1 retains its exact patch/build identity', () => fixture(run => {
  const selected = '/Applications/Xcode_26.0.1.app/Contents/Developer'
  const manifest = run({
    'xcodebuild -version': 'Xcode 26.0.1\nBuild version 17A400',
    'xcode-select -p': selected,
    'xcrun --sdk macosx --show-sdk-path': `${selected}/Platforms/MacOSX.platform/Developer/SDKs/MacOSX26.0.sdk`,
  })
  assert.equal(manifest.toolchain.xcode, 'Xcode 26.0.1\nBuild version 17A400')
  assert.equal(manifest.toolchain.developer, selected)
  assert.equal(manifest.toolchain.sdk, '26.0')
}))
for (const version of ['26.0.2', '26.1', '27.0', '26.0 beta']) test(`rejects unreviewed Xcode ${version}`, () => fixture(run => {
  assert.throws(() => run({ 'xcodebuild -version': `Xcode ${version}\nBuild version 17A400` }))
}))
