import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import test from 'node:test'

const read = (path: string) => readFileSync(new URL(path, import.meta.url), 'utf8')
function block(source: string, key: string) {
  const start = source.indexOf(`  ${key}:`)
  assert(start >= 0, `Missing ${key} workflow block`)
  const next = source.slice(start + 3).search(/\n  [a-zA-Z0-9_-]+:\s*\n/)
  const end = next < 0 ? -1 : start + 3 + next
  return source.slice(start, end < 0 ? undefined : end)
}
function grants(text: string) {
  const permissions = text.match(/permissions:\n((?:\s{6,}.+\n)+)/)?.[1] ?? ''
  return Object.fromEntries([...permissions.matchAll(/^\s{6}([\w-]+):\s*(\w+)/gm)].map(m => [m[1], m[2]]))
}

test('Linux and Mac callers pass only the permissions needed by the reusable attestation job', () => {
  for (const path of ['../../../.github/workflows/verification.yml', '../../../.github/workflows/verification-mac.yml']) {
    const caller = read(path)
    const verify = block(caller.slice(caller.indexOf('\njobs:') + 1), 'verify')
    assert.deepEqual(grants(verify), { actions: 'read', attestations: 'write', contents: 'read', 'id-token': 'write' }, path)
  }
})

test('only the full Rust producer job receives attestation permissions', () => {
  const workflow = read('../../../.github/workflows/verification-platform.yml')
  assert.deepEqual(grants(block(workflow.slice(workflow.indexOf('\njobs:') + 1), 'rust')),
    { contents: 'read', 'id-token': 'write', attestations: 'write' })
  assert.deepEqual(grants(block(workflow.slice(workflow.indexOf('\njobs:') + 1), 'desktop-ui')), {})
  assert.match(workflow.slice(0, workflow.indexOf('\njobs:')), /permissions:\n  contents: read/)
})
