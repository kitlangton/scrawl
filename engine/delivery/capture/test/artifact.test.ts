import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { lstatSync, readFileSync, readdirSync, statSync } from 'node:fs'
import { dirname, relative, resolve, sep } from 'node:path'
import { spawnSync } from 'node:child_process'
import test from 'node:test'

const root = resolve(dirname(new URL(import.meta.url).pathname), '..', 'dist')
const manifestFile = resolve(root, 'manifest.json')
const artifactAvailable = (() => { try { return statSync(manifestFile).isFile() } catch { return false } })()
test('capture artifact inventory and absolute SDK launch contract', { skip: !artifactAvailable }, () => {
  const manifest = JSON.parse(readFileSync(manifestFile, 'utf8'))
  assert.equal(manifest.schema, 1)
  assert.match(manifest.sourceSha, /^[0-9a-f]{40}$/)
  assert.equal(manifest.target, 'linux-x64')
  assert.deepEqual(manifest.capabilities, ['capture/2', 'x11-display', 'x11-area', 'retained-materials', 'authenticated-publication'])
  assert.deepEqual(manifest.launch.argv, ['--runtime-directory', '<private-runtime-dir>', '--capture-channel', 'capture', '--capture-protocol-version', '2'])
  assert.equal(manifest.launch.pathLookup, false)
  assert.equal(manifest.launch.shutdownTimeoutMs, 40_000)
  const actual: Array<{ path: string; mode: number; sha256: string }> = []
  const walk = (dir: string) => {
    for (const name of readdirSync(dir).sort()) {
      const path = resolve(dir, name)
      const meta = lstatSync(path)
      assert.equal(meta.isSymbolicLink(), false)
      if (meta.isDirectory()) walk(path)
      else {
        assert.equal(meta.isFile(), true)
        actual.push({ path: relative(root, path).split(sep).join('/'), mode: meta.mode & 0o777, sha256: createHash('sha256').update(readFileSync(path)).digest('hex') })
      }
    }
  }
  walk(root)
  const recorded = actual.filter(item => item.path !== 'manifest.json')
  assert.deepEqual(recorded, manifest.runtimeFiles)
  const launched = spawnSync(resolve(root, manifest.launch.entry), [], { encoding: 'utf8' })
  assert.equal(launched.status, 2)
})
