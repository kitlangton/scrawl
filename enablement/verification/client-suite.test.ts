import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { chmodSync, mkdtempSync, mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { test } from 'node:test'
import { completed, decision, REVIEWED_PACKAGE, sourceInputs, validProof } from './client-suite.ts'
import type { Identity, Proof } from './client-suite.ts'

const current: Identity = { sha: 'a'.repeat(40), key: 'b'.repeat(64), verifier: 'c'.repeat(64), closure: REVIEWED_PACKAGE, eligible: true }
const success = (prior: Proof | null, selected: string) => completed(current, prior, selected, true, 1, 'https://example.test/evidence')
function primed() {
  let proof = success(null, 'miss')
  for (let n = 0; n < 2; n++) proof = success(proof, 'shadow')
  return proof
}

test('requires baseline plus two full matching shadow executions before reuse', () => {
  assert.equal(decision(current, null), 'miss')
  let proof = success(null, 'miss')
  for (let n = 0; n < 2; n++) {
    assert.equal(decision(current, proof), 'shadow')
    proof = success(proof, 'shadow')
  }
  assert.equal(decision(current, proof), 'reuse')
  assert.equal(decision(current, proof, true), 'shadow')
  const next = { ...current, sha: 'd'.repeat(40) }
  const reused = completed(next, proof, 'reuse', true, 0, null)
  assert.equal(reused.checked_sha, current.sha)
  assert.equal(reused.candidate_sha, next.sha)
  assert.equal(reused.evidence, proof.evidence)
})

test('audits after four reuses and permanently disables a failed audit in this verifier namespace', () => {
  let proof = primed()
  for (let n = 0; n < 4; n++) {
    assert.equal(decision(current, proof), 'reuse')
    proof = success(proof, 'reuse')
  }
  assert.equal(decision(current, proof), 'audit')
  const failure = completed(current, proof, 'audit', false, 1, null)
  assert.equal(failure.disabled, true)
  assert.equal(decision(current, failure), 'disabled')
  const recovery = success(failure, 'disabled')
  assert.equal(decision(current, recovery), 'disabled')
  assert.equal(decision({ ...current, key: 'd'.repeat(64) }, recovery), 'disabled')
})

test('incomplete, failed, mismatched, unmodelled or malformed proof never skips execution', () => {
  const proof = primed()
  for (const outcome of ['failure', 'in_progress', 'cancelled']) {
    assert.equal(decision(current, { ...proof, outcome }), 'miss')
  }
  assert.equal(decision({ ...current, key: 'd'.repeat(64) }, proof), 'miss')
  assert.equal(decision({ ...current, eligible: false }, proof), 'unmodelled-inputs')
  assert.equal(decision(current, { ...proof, eligible: false }), 'miss')
  for (const bad of [null, {}, { ...proof, shadows: 100 }, { ...proof, key: 'other' }, { ...proof, reuses: -1 }]) {
    assert.equal(validProof(bad), false)
  }
  assert.equal(validProof(proof), true)
})

test('client stream includes API, build, verifier and unknown inputs across cancelled commits', t => {
  const root = mkdtempSync(join(tmpdir(), 'apparent-client-closure-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const git = (...args: string[]) => {
    const result = spawnSync('git', ['-c', 'core.hooksPath=/dev/null', '-c', 'commit.gpgsign=false', ...args], { cwd: root })
    assert.equal(result.status, 0, result.stderr?.toString())
  }
  git('init', '-q')
  git('config', 'user.name', 'Fixture'); git('config', 'user.email', 'fixture@example.invalid')
  const paths = ['connect/client/src/lib.rs', 'connect/client/test/fixture', 'connect/client/Cargo.lock', 'shared/config-native/src/lib.rs', 'shared/config-native/Cargo.lock',
    'connect/service/src/worker.ts', 'connect/hosted/provider.rs', '.cargo/config.toml',
    'Cargo.toml', 'Cargo.lock', 'build.rs', 'rust-toolchain.toml',
    '.github/workflows/verification-platform.yml', 'enablement/verification/client-suite.ts',
    'enablement/verification/receipt.ts', 'unknown/input']
  for (const path of paths) { mkdirSync(dirname(join(root, path)), { recursive: true }); writeFileSync(join(root, path), 'baseline') }
  git('add', '.'); git('commit', '-qm', 'A')
  const baseline = sourceInputs(root)
  for (const path of paths) {
    writeFileSync(join(root, path), 'changed')
    assert.notEqual(sourceInputs(root).fingerprint, baseline.fingerprint, path)
    writeFileSync(join(root, path), 'baseline')
  }
  writeFileSync(join(root, 'connect/service/src/worker.ts'), 'change in cancelled B')
  git('add', '.'); git('commit', '-qm', 'B')
  git('commit', '--allow-empty', '-qm', 'C')
  assert.notEqual(sourceInputs(root).fingerprint, baseline.fingerprint)
  writeFileSync(join(root, 'connect/client/src/new.rs'), 'new unreviewed input')
  git('add', '.'); git('commit', '-qm', 'D')
  assert.notEqual(sourceInputs(root).reviewedPackage, baseline.reviewedPackage)
  rmSync(join(root, 'connect/client/src/lib.rs'))
  symlinkSync('/outside/unmodelled.rs', join(root, 'connect/client/src/lib.rs'))
  assert.equal(sourceInputs(root).regular, false)
})

test('independent client excludes other streams, including native-tool symlinks', t => {
  const root = mkdtempSync(join(tmpdir(), 'apparent-client-stream-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const git = (...args: string[]) => {
    const result = spawnSync('git', ['-c', 'core.hooksPath=/dev/null', '-c', 'commit.gpgsign=false', ...args], { cwd: root })
    assert.equal(result.status, 0, result.stderr?.toString())
  }
  git('init', '-q'); git('config', 'user.name', 'Fixture'); git('config', 'user.email', 'fixture@example.invalid')
  const paths = ['connect/client/src/lib.rs', 'capture/engine/runtime.rs', 'capture/desktop/consumer.rs',
    'capture/agent/inspect.rs', 'composition/main.rs', 'shared/config.rs', 'shared/ui/src/index.ts',
    'connect/web/src/App.tsx', 'public/site/index.html', 'enablement/native/linux/gui-vm.py']
  for (const path of paths) { mkdirSync(dirname(join(root, path)), { recursive: true }); writeFileSync(join(root, path), 'baseline') }
  const alias = join(root, 'enablement/native/linux/alias.py')
  symlinkSync('gui-vm.py', alias)
  git('add', '.'); git('commit', '-qm', 'baseline')
  const baseline = sourceInputs(root)
  assert.equal(baseline.regular, true)
  for (const path of paths.slice(1)) {
    writeFileSync(join(root, path), 'unrelated stream change')
    git('add', '.'); git('commit', '-qm', 'other stream')
    assert.equal(sourceInputs(root).fingerprint, baseline.fingerprint, path)
    assert.equal(sourceInputs(root).clean, true)
  }
  // Untracked files are still ineligible, even in an otherwise excluded stream.
  writeFileSync(join(root, 'capture/engine/untracked.rs'), 'unreviewed')
  assert.equal(sourceInputs(root).clean, false)
  // Unknown symlink inputs fail closed instead of following external state.
  symlinkSync('/external', join(root, 'unknown-link'))
  git('add', '.'); git('commit', '-qm', 'unknown link')
  assert.equal(sourceInputs(root).regular, false)
})

test('closure is explicitly pinned and workflow keeps root checks and saves failed audits', () => {
  const root = join(dirname(fileURLToPath(import.meta.url)), '../..')
  // Future package changes must fall back to execution, not break CI until repinned.
  assert.match(REVIEWED_PACKAGE, /^[a-f0-9]{64}$/)
  const workflow = readFileSync(join(root, '.github/workflows/verification-platform.yml'), 'utf8')
  assert.match(workflow, /client-suite\.ts --state/)
  assert.match(workflow, /python3 enablement\/verification.py --base "\$BASE" --run --online/)
  assert.match(workflow, /actions\/cache\/save@v4/)
  assert.match(workflow, /if: always\(\) && steps\.client-plan\.outputs\.namespace/)
})


test('synthetic process boundary executes shadows, skips reuse, and persists a failing audit', t => {
  const realRoot = join(dirname(fileURLToPath(import.meta.url)), '../..')
  const directory = mkdtempSync(join(tmpdir(), 'apparent-client-process-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  const root = join(directory, 'repo'), bin = join(directory, 'bin')
  mkdirSync(root); mkdirSync(bin)
  const git = (...args: string[]) => {
    const result = spawnSync('git', ['-c', 'core.hooksPath=/dev/null', '-c', 'commit.gpgsign=false', ...args], { cwd: root })
    assert.equal(result.status, 0, result.stderr?.toString())
  }
  git('init', '-q'); git('config', 'user.name', 'Fixture'); git('config', 'user.email', 'fixture@example.invalid')
  for (const path of ['connect/client/src/lib.rs', 'connect/client/Cargo.toml', 'connect/client/Cargo.lock', '.cargo/config.toml', 'capture/engine/runtime.rs']) {
    mkdirSync(dirname(join(root, path)), { recursive: true })
    writeFileSync(join(root, path), 'synthetic package closure')
  }
  git('add', '.'); git('commit', '-qm', 'synthetic orchestration fixture')
  // Substitute only the reviewed closure constant for this synthetic package.
  // Keep the production selection/execution/receipt code unchanged.
  const tools = join(directory, 'tools')
  mkdirSync(tools)
  const source = readFileSync(join(realRoot, 'enablement/verification/client-suite.ts'), 'utf8')
  const fixtureRunner = source.replace(/export const REVIEWED_PACKAGE = '[a-f0-9]+'/,
    `export const REVIEWED_PACKAGE = '${sourceInputs(root).reviewedPackage}'`)
  assert.notEqual(fixtureRunner, source)
  writeFileSync(join(tools, 'client-suite.ts'), fixtureRunner)
  writeFileSync(join(tools, 'receipt.ts'), readFileSync(join(realRoot, 'enablement/verification/receipt.ts')))
  const calls = join(directory, 'calls'), failure = join(directory, 'fail')
  for (const tool of ['cargo', 'rustc', 'curl', 'sha256sum', 'cc', 'ldd', 'xcrun']) {
    const path = join(bin, tool)
    writeFileSync(path, '#!/bin/sh\ncase "$1" in\n build|test) echo "$1" >> "$FIXTURE_CALLS"; test ! -f "$FIXTURE_FAILURE" || exit 7; echo "test result: ok. 1 passed";;\n *) echo "synthetic-tool-version";;\nesac\n')
    chmodSync(path, 0o755)
  }
  const env = Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.startsWith('APPARENT_')))
  Object.assign(env, { PATH: `${bin}:${process.env.PATH}`, GITHUB_ACTIONS: 'true', ImageOS: 'synthetic-linux',
    ImageVersion: 'fixture', CARGO_HOME: join(directory, 'cargo-home'), FIXTURE_CALLS: calls, FIXTURE_FAILURE: failure })
  const state = join(directory, 'proof', 'proof.json')
  const run = (overrides: Record<string, string> = {}) => spawnSync(process.execPath,
    [join(tools, 'client-suite.ts'), '--state', state], { cwd: root, env: { ...env, ...overrides }, encoding: 'utf8' })
  if (process.platform !== 'linux') { t.skip('Linux-only result reuse policy'); return }
  // This fixture uses fake tool processes to test orchestration, not Rust correctness.
  for (let n = 0; n < 3; n++) { const result = run(); assert.equal(result.status, 0, result.stderr) }
  assert.equal(readFileSync(calls, 'utf8').trim().split('\n').length, 6)
  const qualified = readFileSync(state, 'utf8')
  const transport = ['http_proxy', 'https_proxy', 'HTTP_PROXY', 'HTTPS_PROXY', 'ALL_PROXY', 'NO_PROXY',
    'no_proxy', 'CURL_CA_BUNDLE', 'SSL_CERT_FILE', 'SSL_CERT_DIR']
  for (const name of transport) {
    writeFileSync(state, qualified)
    const result = run({ [name]: 'synthetic-different-transport' })
    assert.equal(result.status, 0, result.stderr)
    assert.equal(JSON.parse(readFileSync(state, 'utf8')).decision, 'miss', name)
  }
  const fullCalls = 6 + transport.length * 2
  assert.equal(readFileSync(calls, 'utf8').trim().split('\n').length, fullCalls)
  writeFileSync(state, qualified)
  const checked = JSON.parse(readFileSync(state, 'utf8')).checked_sha
  writeFileSync(join(root, 'capture/engine/runtime.rs'), 'independent stream changed')
  git('add', '.'); git('commit', '-qm', 'engine candidate')
  for (let n = 0; n < 4; n++) {
    const result = run(); assert.equal(result.status, 0, result.stderr)
    const proof = JSON.parse(readFileSync(state, 'utf8'))
    assert.equal(proof.decision, 'reuse')
    assert.equal(proof.checked_sha, checked)
    assert.notEqual(proof.candidate_sha, checked)
  }
  assert.equal(readFileSync(calls, 'utf8').trim().split('\n').length, fullCalls)
  writeFileSync(failure, 'synthetic audit failure')
  assert.equal(run().status, 1)
  assert.equal(JSON.parse(readFileSync(state, 'utf8')).disabled, true)
  rmSync(failure)
  assert.equal(run().status, 0)
  assert.equal(JSON.parse(readFileSync(state, 'utf8')).decision, 'disabled')
  assert.equal(JSON.parse(readFileSync(state, 'utf8')).disabled, true)
})
