import assert from 'node:assert/strict'
import { execFileSync } from 'node:child_process'
import { mkdtempSync, mkdirSync, readFileSync, rmSync, symlinkSync, unlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { test } from 'node:test'
import { validateTarInventory } from './handoff.ts'
import { validateArchiveKinds } from './mac-frameworks.ts'

const owner = { id: 'app-shell', stage: 'development-app' as const }
const prefix = 'stream-payload/bundle/apparent-foundation-darwin-x64/apparent-foundation.app/Contents/Frameworks/'
function fixture(t: any) {
  const root = mkdtempSync(join(tmpdir(), 'mac-framework-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  for (const name of ['Electron Framework', 'Mantle', 'ReactiveObjC', 'Squirrel']) {
    const base = join(root, prefix + name + '.framework')
    for (const alias of ['Resources', ...(name === 'Electron Framework' ? ['Libraries', 'Helpers'] : [])]) {
      mkdirSync(join(base, 'Versions/A', alias), { recursive: true })
      symlinkSync('Versions/Current/' + alias, join(base, alias))
    }
    writeFileSync(join(base, 'Versions/A', name), 'Synthetic framework bytes; not executable')
    symlinkSync('Versions/Current/' + name, join(base, name))
    symlinkSync('A', join(base, 'Versions/Current'))
  }
  return { root, archive(extra: string[] = []) {
    const path = join(root, 'component.tar.gz')
    execFileSync('tar', ['-czf', path, '-C', root, 'stream-payload', ...extra])
    return readFileSync(path)
  } }
}
test('actual tar inventory accepts all fourteen normal Mac framework aliases without extracting or rewriting bytes', t => {
  const f = fixture(t), bytes = f.archive()
  validateTarInventory(bytes, owner, 'macos-x64')
  assert.deepEqual(f.archive(), bytes)
  for (const [id, target, stage] of [['engine', 'macos-x64', 'development-app'], ['app-shell', 'linux-x64', 'development-app'], ['app-shell', 'macos-x64', 'contract']]) {
    assert.throws(() => validateTarInventory(bytes, { id, stage: stage as any }, target as any), /links or unsupported/)
  }
})
for (const target of ['/tmp/foreign', '../A', 'B', 'Current']) test('actual tar refuses changed Current target: ' + target, t => {
  const f = fixture(t), current = join(f.root, prefix + 'Mantle.framework/Versions/Current')
  unlinkSync(current); symlinkSync(target, current)
  assert.throws(() => validateTarInventory(f.archive(), owner, 'macos-x64'), /links or unsupported/)
})
test('actual tar refuses broken targets, target-kind substitutions and unreviewed aliases', t => {
  for (const mode of ['missing', 'kind', 'extra']) {
    const f = fixture(t), base = join(f.root, prefix + 'Mantle.framework')
    if (mode === 'missing') rmSync(join(base, 'Versions/A/Mantle'))
    if (mode === 'kind') { rmSync(join(base, 'Versions/A/Mantle')); mkdirSync(join(base, 'Versions/A/Mantle')) }
    if (mode === 'extra') symlinkSync('Versions/A/Resources', join(base, 'Other'))
    assert.throws(() => validateTarInventory(f.archive(), owner, 'macos-x64'), /links or unsupported/)
  }
})
test('actual tar refuses a payload entry beneath a valid alias before extraction', t => {
  const f = fixture(t), alias = prefix + 'Mantle.framework/Resources'
  writeFileSync(join(f.root, alias, 'injected'), 'synthetic')
  assert.throws(() => validateTarInventory(f.archive([alias + '/injected']), owner, 'macos-x64'), /links or unsupported/)
})
test('kind validator refuses unsupported entries, cycles and excessive graph depth', () => {
  const name = prefix + 'Mantle.framework/Versions/Current'
  for (const kind of ['h', 'p', 'b', 'c']) assert.throws(() => validateArchiveKinds([name], [kind + 'rwxr-xr-x fixture ' + name], owner.id, 'macos-x64', owner.stage))
  const deep = Array(70).fill('dir').join('/')
  assert.throws(() => validateArchiveKinds([name, deep], ['lrwxr-xr-x fixture ' + name + ' -> A', '-rwxr-xr-x fixture ' + deep], owner.id, 'macos-x64', owner.stage))
})
test('Mac inventory refuses case-folded alias traversal, collisions and non-ASCII ambiguity', () => {
  const current = prefix + 'Mantle.framework/Versions/Current', endpoint = prefix + 'Mantle.framework/Versions/A'
  const names = [current, endpoint], entries = ['lrwxr-xr-x fixture ' + current + ' -> A', 'drwxr-xr-x fixture ' + endpoint]
  validateArchiveKinds(names, entries, owner.id, 'macos-x64', owner.stage)
  for (const path of [current.toLowerCase() + '/injected', endpoint.toLowerCase(), 'ambiguous-é']) {
    assert.throws(() => validateArchiveKinds([...names, path], [...entries, '-rwxr-xr-x fixture ' + path], owner.id, 'macos-x64', owner.stage))
  }
})
