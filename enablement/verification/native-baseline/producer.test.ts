import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { chmodSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { environmentSnapshot, inventory, inputPaths } from './producer.ts'

const producer = fileURLToPath(new URL('./producer.ts', import.meta.url))
test('baseline producer refuses missing normal-job provenance before writing anything', t => {
  const parent = mkdtempSync(join(tmpdir(), 'native-baseline-'))
  t.after(() => rmSync(parent, { recursive: true, force: true }))
  const output = join(parent, 'baseline')
  const env = { ...process.env }
  delete env.GITHUB_REPOSITORY; delete env.GITHUB_RUN_ID; delete env.GITHUB_RUN_ATTEMPT; delete env.GITHUB_JOB
  const result = spawnSync('node', ['--experimental-strip-types', producer, output], { env, encoding: 'utf8' })
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /GitHub run identity required/)
  assert.notEqual(spawnSync('test', ['-e', output]).status, 0)
})

test('input inventory hashes bytes and executable mode and keeps unknown paths conservative', t => {
  const base = mkdtempSync(join(tmpdir(), 'native-baseline-inputs-'))
  t.after(() => rmSync(base, { recursive: true, force: true }))
  mkdirSync(join(base, 'src'))
  writeFileSync(join(base, 'src', 'runtime-fixture.bin'), 'fixture')
  const paths = ['src/runtime-fixture.bin', 'capture/desktop/ui/frontend/src/view.tsx', 'unknown/tooling.input']
  const one = inventory(base, ['src/runtime-fixture.bin'])
  writeFileSync(join(base, 'src', 'runtime-fixture.bin'), 'changed')
  const two = inventory(base, ['src/runtime-fixture.bin'])
  assert.notEqual(one[0].sha256, two[0].sha256)
  chmodSync(join(base, 'src', 'runtime-fixture.bin'), 0o755)
  const three = inventory(base, ['src/runtime-fixture.bin'])
  assert.notEqual(two[0].mode, three[0].mode)
  assert.deepEqual(inputPaths(paths), ['src/runtime-fixture.bin', 'unknown/tooling.input'])
  assert.notDeepEqual(environmentSnapshot({ ImageOS: 'ubuntu24' }), environmentSnapshot({ ImageOS: 'ubuntu24', RUSTFLAGS: '-C opt-level=1' }))
})

test('baseline attestation follows the full platform verification job', async () => {
  const { readFileSync } = await import('node:fs')
  const { resolve } = await import('node:path')
  const workflow = readFileSync(resolve(fileURLToPath(new URL('../../../.github/workflows/verification-platform.yml', import.meta.url))), 'utf8')
  const full = workflow.indexOf('name: Execute Rust changed-tree checks')
  const seal = workflow.indexOf('name: Seal completed native verification baseline')
  const attest = workflow.indexOf('name: Attest native verification baseline')
  const finalPlatformCheck = workflow.indexOf('name: Upload Intel Mac development executable contract')
  assert(full >= 0 && seal > full && attest > seal)
  assert(finalPlatformCheck > full && seal > finalPlatformCheck)
  assert.match(workflow.slice(seal, attest), /if: success\(\).*needs\.select\.outputs\.root == 'true'/)
  assert.match(workflow.slice(attest, attest + 500), /actions\/attest-build-provenance@v2/)
})
