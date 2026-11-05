import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { resolve } from 'node:path'
import { after, test } from 'node:test'
import { catalog, makePlan, sourceDigest, type Owner } from './core.ts'

for (const name of execFileSync('git', ['rev-parse', '--local-env-vars'], { encoding: 'utf8' }).trim().split('\n')) delete process.env[name]
const temp = mkdtempSync(resolve(tmpdir(), 'connect-delivery-'))
after(() => rmSync(temp, { recursive: true, force: true }))
const owner: Owner = {
  schema: 1, id: 'connect-client', kind: 'npm', package: 'connect/delivery/client',
  inputs: ['connect/delivery/client/'], verify: ['typecheck', 'test'], build: 'build',
  targets: ['linux-x64'], stage: 'library', language: 'typescript',
}
function git(cwd: string, ...args: string[]) { execFileSync('git', args, { cwd, stdio: 'ignore' }) }
function repo(name: string) {
  const root = resolve(temp, name)
  for (const dir of ['connect/delivery/client/src', 'engine/sdk', 'composition/delivery', 'composition/architecture', '.github/workflows', 'enablement/verification']) mkdirSync(resolve(root, dir), { recursive: true })
  git(root, 'init', '-q'); git(root, 'config', 'user.email', 'delivery-test@example.invalid'); git(root, 'config', 'user.name', 'delivery test')
  writeFileSync(resolve(root, 'connect/delivery/client/package.json'), JSON.stringify({ name: '@apparent/connect-client' }))
  writeFileSync(resolve(root, 'connect/delivery/client/src/index.ts'), 'export const client = true\n')
  writeFileSync(resolve(root, 'connect/delivery.json'), JSON.stringify(owner))
  writeFileSync(resolve(root, 'engine/sdk/implementation.ts'), 'export const engine = true\n')
  writeFileSync(resolve(root, 'composition/delivery/core.ts'), 'delivery\n')
  writeFileSync(resolve(root, 'composition/architecture/guard.ts'), 'architecture\n')
  writeFileSync(resolve(root, '.github/workflows/value-stream-delivery.yml'), 'workflow\n')
  writeFileSync(resolve(root, 'enablement/verification/workflow-budget.ts'), 'budget\n')
  writeFileSync(resolve(root, 'LICENSE'), 'license\n')
  git(root, 'add', '.'); git(root, 'commit', '-qm', 'initial')
  return root
}

test('connect client is a bounded Linux npm library lane with architecture enforcement', () => {
  const root = resolve(process.cwd())
  const registration = catalog(root)
  assert.deepEqual(registration.blocked, [])
  const lane = makePlan(root).matrix.include.filter(row => row.id === 'connect-client')
  assert.deepEqual(lane.map(({ target, kind, stage, package: pkg, verify, build, architecture_owner }) => ({ target, kind, stage, package: pkg, verify, build, architecture_owner })), [
    { target: 'linux-x64', kind: 'npm', stage: 'library', package: 'connect/delivery/client', verify: ['typecheck', 'test'], build: 'build', architecture_owner: 'connect' },
  ])
})

test('connect source digest tracks its client and registration, but not Engine implementation', () => {
  const root = repo('digest')
  const initial = sourceDigest(root, owner)
  writeFileSync(resolve(root, 'engine/sdk/implementation.ts'), 'changed engine implementation\n')
  git(root, 'add', '.'); git(root, 'commit', '-qm', 'engine change')
  assert.equal(sourceDigest(root, owner), initial)
  writeFileSync(resolve(root, 'connect/delivery.json'), JSON.stringify({ ...owner, verify: ['typecheck', 'test', 'test:package'] }))
  git(root, 'add', '.'); git(root, 'commit', '-qm', 'registration change')
  const afterRegistration = sourceDigest(root, owner)
  assert.notEqual(afterRegistration, initial)
  writeFileSync(resolve(root, 'connect/delivery/client/src/index.ts'), 'export const client = false\n')
  git(root, 'add', '.'); git(root, 'commit', '-qm', 'client change')
  assert.notEqual(sourceDigest(root, owner), afterRegistration)
})
