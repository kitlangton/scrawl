import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, writeFileSync, rmSync } from 'node:fs'
import { fileURLToPath } from 'node:url'
import { execFileSync, spawnSync } from 'node:child_process'
import test from 'node:test'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { platformPlan } from './plan.ts'
import { workflowPathFor } from '../core.ts'

// Git exports caller-local repository paths to hooks; temporary fixtures and
// the planner subprocess must resolve their explicit cwd instead.
for (const name of execFileSync('git', ['rev-parse', '--local-env-vars'], { encoding: 'utf8' }).trim().split('\n')) delete process.env[name]

const root = fileURLToPath(new URL('../../../', import.meta.url))
const ids = ['platform-config', 'platform-resources', 'app-shell']

test('selected registrations retain component identities, toolchains and both native targets', () => {
  for (const id of ids) {
    const rows = platformPlan(root, [id]).matrix.include
    assert.deepEqual(rows.map(row => [row.id, row.target, row.runner]), [
      [id, 'linux-x64', 'ubuntu-24.04'], [id, 'macos-x64', 'macos-26-intel'],
    ])
    assert(rows.every(row => row.language === (id === 'app-shell' ? 'typescript' : 'native')))
    assert.equal(workflowPathFor(id), '.github/workflows/stream-platform.yml')
  }
  assert.equal(platformPlan(root, ids).matrix.include.length, 6)
  assert.deepEqual(platformPlan(root, []), { matrix: { include: [] }, has_ready: false })
  assert.throws(() => platformPlan(root, ['missing']), /unavailable/)
})

test('actual workflow gate rejects failures, cancellation and unexpected skips', () => {
  const source = readFileSync(new URL('../../../.github/workflows/stream-platform.yml', import.meta.url), 'utf8')
  const step = source.split('      - name: Require successful selected components\n')[1]?.split('      - uses:')[0]
  assert(step, 'selected-execution gate exists')
  const script = step.split('        run: |\n')[1]?.split('\n').map(line => line.replace(/^          /, '')).join('\n')
  assert(script?.includes('test "$CONTRACTS"'), 'executable gate exists')
  const env = { ...process.env, CONTRACTS: 'success', NATIVE_DRIVER_CONTRACTS: 'success', HAS_READY: 'true', PRODUCER: 'skipped', PR: 'success', EVENT: 'pull_request' }
  const run = (overrides: Record<string, string>) => spawnSync('bash', ['-e', '-c', script!], { env: { ...env, ...overrides } }).status
  assert.equal(run({}), 0)
  assert.equal(run({ EVENT: 'push', PRODUCER: 'success', PR: 'skipped' }), 0)
  assert.equal(run({ HAS_READY: 'false', PR: 'skipped' }), 0)
  for (const PR of ['failure', 'cancelled', 'skipped']) assert.notEqual(run({ PR }), 0)
  for (const PRODUCER of ['failure', 'cancelled', 'skipped']) assert.notEqual(run({ EVENT: 'push', PRODUCER }), 0)
  for (const CONTRACTS of ['failure', 'cancelled', 'skipped']) assert.notEqual(run({ CONTRACTS }), 0)
  for (const NATIVE_DRIVER_CONTRACTS of ['failure', 'cancelled', 'skipped', '']) {
    assert.notEqual(run({ NATIVE_DRIVER_CONTRACTS }), 0)
    assert.notEqual(run({ NATIVE_DRIVER_CONTRACTS, HAS_READY: 'false', PR: 'skipped' }), 0)
  }
  assert.notEqual(run({ HAS_READY: '' }), 0)
})

test('workflow CLI emits usable matrix outputs for empty selection and unavailable history', () => {
  const temp = mkdtempSync(join(tmpdir(), 'platform-plan-'))
  try {
    const event = join(temp, 'event.json'), output = join(temp, 'outputs')
    const sha = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim()
    const run = (payload: object) => {
      writeFileSync(event, JSON.stringify(payload)); writeFileSync(output, '')
      const result = spawnSync(process.execPath, ['--experimental-strip-types', 'composition/delivery/platform/plan.ts'], {
        cwd: root, encoding: 'utf8', env: { ...process.env, GITHUB_EVENT_NAME: 'push', GITHUB_EVENT_PATH: event, GITHUB_OUTPUT: output },
      })
      assert.equal(result.status, 0, result.stderr)
      const values = Object.fromEntries(readFileSync(output, 'utf8').trim().split('\n').map(line => {
        const index = line.indexOf('='); return [line.slice(0, index), line.slice(index + 1)]
      }))
      return { ready: values.has_ready, rows: JSON.parse(values.matrix).include }
    }
    assert.deepEqual(run({ before: sha, after: sha }), { ready: 'false', rows: [] })
    const fallback = run({ before: 'f'.repeat(40), after: sha })
    assert.equal(fallback.ready, 'true')
    assert.equal(fallback.rows.length, 6)
    assert.deepEqual([...new Set(fallback.rows.map((row: { id: string }) => row.id))].sort(), [...ids].sort())
  } finally { rmSync(temp, { recursive: true, force: true }) }
})
