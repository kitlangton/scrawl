import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { chmodSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
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

test('source closure invalidates client fixtures, consumers, shared code, config and cancellation-spanning changes', t => {
  const root = mkdtempSync(join(tmpdir(), 'apparent-client-closure-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const git = (...args: string[]) => {
    const result = spawnSync('git', ['-c', 'core.hooksPath=/dev/null', '-c', 'commit.gpgsign=false', ...args], { cwd: root })
    assert.equal(result.status, 0, result.stderr?.toString())
  }
  git('init', '-q')
  git('config', 'user.name', 'Fixture'); git('config', 'user.email', 'fixture@example.invalid')
  const paths = ['connect/client/src/lib.rs', 'connect/client/test/fixture', 'connect/client/Cargo.lock',
    'connect/hosted/provider.rs', 'shared/config.rs', '.cargo/config.toml',
    'capture/desktop/consumer.rs', 'enablement/verification/client-suite.ts', 'public/site/index.html']
  for (const path of paths) { mkdirSync(dirname(join(root, path)), { recursive: true }); writeFileSync(join(root, path), 'baseline') }
  git('add', '.'); git('commit', '-qm', 'A')
  const baseline = sourceInputs(root)
  for (const path of paths.slice(0, -1)) {
    writeFileSync(join(root, path), 'changed')
    assert.notEqual(sourceInputs(root).fingerprint, baseline.fingerprint, path)
    writeFileSync(join(root, path), 'baseline')
  }
  writeFileSync(join(root, 'public/site/index.html'), 'marketing')
  assert.equal(sourceInputs(root).fingerprint, baseline.fingerprint)
  writeFileSync(join(root, 'shared/config.rs'), 'change in cancelled B')
  git('add', '.'); git('commit', '-qm', 'B')
  git('commit', '--allow-empty', '-qm', 'C')
  assert.notEqual(sourceInputs(root).fingerprint, baseline.fingerprint)
  writeFileSync(join(root, 'connect/client/src/new.rs'), 'new unreviewed input')
  git('add', '.'); git('commit', '-qm', 'D')
  assert.notEqual(sourceInputs(root).reviewedPackage, baseline.reviewedPackage)
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
  for (const path of ['connect/client/src/lib.rs', 'connect/client/Cargo.toml', 'connect/client/Cargo.lock', '.cargo/config.toml']) {
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
  for (const tool of ['cargo', 'rustc', 'curl', 'cc', 'ldd', 'xcrun']) {
    const path = join(bin, tool)
    writeFileSync(path, '#!/bin/sh\ncase "$1" in\n build|test) echo "$1" >> "$FIXTURE_CALLS"; test ! -f "$FIXTURE_FAILURE" || exit 7; echo "test result: ok. 1 passed";;\n *) echo "synthetic-tool-version";;\nesac\n')
    chmodSync(path, 0o755)
  }
  const env = Object.fromEntries(Object.entries(process.env).filter(([name]) => !name.startsWith('APPARENT_')))
  Object.assign(env, { PATH: `${bin}:${process.env.PATH}`, GITHUB_ACTIONS: 'true', ImageOS: 'synthetic-linux',
    ImageVersion: 'fixture', CARGO_HOME: join(directory, 'cargo-home'), FIXTURE_CALLS: calls, FIXTURE_FAILURE: failure })
  const state = join(directory, 'proof', 'proof.json')
  const run = () => spawnSync(process.execPath, [join(tools, 'client-suite.ts'), '--state', state], { cwd: root, env, encoding: 'utf8' })
  if (process.platform !== 'linux') { t.skip('Linux-only result reuse policy'); return }
  // This fixture uses fake tool processes to test orchestration, not Rust correctness.
  for (let n = 0; n < 3; n++) { const result = run(); assert.equal(result.status, 0, result.stderr) }
  assert.equal(readFileSync(calls, 'utf8').trim().split('\n').length, 6)
  for (let n = 0; n < 4; n++) {
    const result = run(); assert.equal(result.status, 0, result.stderr)
    assert.equal(JSON.parse(readFileSync(state, 'utf8')).decision, 'reuse')
  }
  assert.equal(readFileSync(calls, 'utf8').trim().split('\n').length, 6)
  writeFileSync(failure, 'synthetic audit failure')
  assert.equal(run().status, 1)
  assert.equal(JSON.parse(readFileSync(state, 'utf8')).disabled, true)
  rmSync(failure)
  assert.equal(run().status, 0)
  assert.equal(JSON.parse(readFileSync(state, 'utf8')).decision, 'disabled')
  assert.equal(JSON.parse(readFileSync(state, 'utf8')).disabled, true)
})
