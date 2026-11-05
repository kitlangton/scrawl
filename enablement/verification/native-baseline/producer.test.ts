import assert from 'node:assert/strict'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'

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

test('workflow only emits an attested native baseline after the successful full Rust step', async () => {
  const { readFileSync } = await import('node:fs')
  const { resolve } = await import('node:path')
  const workflow = readFileSync(resolve(fileURLToPath(new URL('../../../.github/workflows/verification-platform.yml', import.meta.url))), 'utf8')
  const full = workflow.indexOf('name: Execute Rust changed-tree checks')
  const seal = workflow.indexOf('name: Seal completed native verification baseline')
  const attest = workflow.indexOf('name: Attest native verification baseline')
  assert(full >= 0 && seal > full && attest > seal)
  assert.match(workflow.slice(seal, attest), /if: success\(\).*needs\.select\.outputs\.root == 'true'/)
  assert.match(workflow.slice(attest, attest + 500), /actions\/attest-build-provenance@v2/)
})
