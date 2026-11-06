import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { selectComponents, selectFromEvent } from './selector.ts'
// Git exports caller-local repository paths to hooks; temporary fixtures and
// the planner subprocess must resolve their explicit cwd instead.
for (const name of execFileSync('git', ['rev-parse', '--local-env-vars'], { encoding: 'utf8' }).trim().split('\n')) delete process.env[name]


test('maps owned inputs, shared inputs, broad inputs and unknown paths', () => {
  assert.deepEqual(selectComponents(null), ['platform-config', 'platform-resources', 'app-shell'])
  assert.deepEqual(selectComponents(['composition/config/a.ts']), ['platform-config', 'app-shell'])
  assert.deepEqual(selectComponents(['shared/resources/a.ts']), ['platform-resources'])
  for (const path of ['composition/app/a.ts', 'enablement/native/owned-processes.ts', '.github/workflows/verification-platform.yml']) assert.deepEqual(selectComponents([path]), ['app-shell'])
  assert.deepEqual(selectComponents(['shared/contracts/a.ts']), ['platform-config', 'app-shell'])
  assert.deepEqual(selectComponents(['.cargo/config.toml']), ['platform-config', 'platform-resources'])
  assert.deepEqual(selectComponents(['shared/ui/button.tsx']), ['app-shell'])
  assert.deepEqual(selectComponents(['shared/config-native/a.rs']), ['platform-config'])
  assert.deepEqual(selectComponents(['composition/native-host/a.ts']), ['app-shell'])
  assert.deepEqual(selectComponents(['unrelated/file.ts']), [])
  for (const path of ['LICENSE', 'composition/delivery/cli.ts', 'composition/architecture/a.ts', '.github/actions/runtime-budget/action.yml', 'enablement/verification/workflow-budget.ts', '.github/workflows/stream-platform-config.yml', '.github/workflows/stream-platform.yml', '.github/workflows/stream-app-shell.yml', '.github/actions/delivery-contracts/action.yml', '.github/actions/delivery-component/action.yml'])
    assert.deepEqual(selectComponents([path]), ['platform-config', 'platform-resources', 'app-shell'], path)
})

function fixture(callback: (ctx: { root: string; git: (...args: string[]) => void; commit: (message: string) => string }) => void) {
  const root = mkdtempSync(join(tmpdir(), 'platform-selector-'))
  const git = (...args: string[]) => execFileSync('git', args, { cwd: root, stdio: 'ignore' })
  try {
    git('init', '-q'); git('config', 'user.email', 'selector@example.test'); git('config', 'user.name', 'Selector Test')
    const commit = (message: string) => { git('add', '-A'); git('commit', '-qm', message); return execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim() }
    callback({ root, git, commit })
  } finally { rmSync(root, { recursive: true, force: true }) }
}

test('selects PR changes from merge-base to head', () => fixture(({ root, git, commit }) => {
  writeFileSync(join(root, 'base'), 'base'); const base = commit('base')
  git('checkout', '-qb', 'feature'); mkdirSync(join(root, 'composition/app'), { recursive: true }); writeFileSync(join(root, 'composition/app/change.ts'), 'x')
  const head = commit('feature')
  git('checkout', '-qb', 'base-advanced', base)
  mkdirSync(join(root, 'shared/resources'), { recursive: true })
  writeFileSync(join(root, 'shared/resources/unrelated-main'), 'main')
  const advanced = commit('main advanced')
  assert.deepEqual(selectFromEvent(root, 'pull_request', { pull_request: { base: { sha: advanced }, head: { sha: head } } }), ['app-shell'])
  assert.deepEqual(selectFromEvent(root, 'pull_request', { pull_request: { base: { sha: base }, head: { sha: head } } }), ['app-shell'])
}))

test('push diff sees deletions and rename sides; invalid history selects all', () => fixture(({ root, commit }) => {
  mkdirSync(join(root, 'shared/resources'), { recursive: true }); writeFileSync(join(root, 'shared/resources/item'), 'v')
  const before = commit('before')
  rmSync(join(root, 'shared/resources/item'))
  mkdirSync(join(root, 'composition/app'), { recursive: true }); writeFileSync(join(root, 'composition/app/item'), 'v')
  const after = commit('rename')
  assert.deepEqual(selectFromEvent(root, 'push', { before, after }), ['platform-resources', 'app-shell'])
  assert.deepEqual(selectFromEvent(root, 'push', { before: 'f'.repeat(40), after }), ['platform-config', 'platform-resources', 'app-shell'])
}))
