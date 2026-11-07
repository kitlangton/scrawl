import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { lstatSync, readFileSync, readdirSync, statSync, mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, relative, resolve, sep } from 'node:path'
import { spawnSync } from 'node:child_process'
import test from 'node:test'
import { validateRecoveryAdvertisement } from '../../media/src/recovery-contract.ts'

const root = resolve(dirname(new URL(import.meta.url).pathname), '..', 'dist')
const manifestFile = resolve(root, 'manifest.json')
const artifactAvailable = (() => { try { return statSync(manifestFile).isFile() } catch { return false } })()
test('capture entry forwards only normal or exact nonce-bound recovery argv', () => {
  const root = mkdtempSync(resolve(realpathSync(tmpdir()), 'engine-capture-argv-'))
  try {
    mkdirSync(resolve(root, 'libexec')); mkdirSync(resolve(root, 'private runtime'), { mode: 0o700 })
    const entry = resolve(root, 'apparent-engine-capture-launch')
    writeFileSync(entry, readFileSync(new URL('../src/launch.sh', import.meta.url)), { mode: 0o755 })
    writeFileSync(resolve(root, 'libexec/apparent-engine-capture'), '#!/bin/sh\nprintf "%s\\n" "$@"\n', { mode: 0o755 })
    const args = ['--runtime-directory', resolve(root, 'private runtime'), '--capture-channel', 'capture', '--capture-protocol-version', '2']
    for (const suffix of [[], ['--recovery-peer-v1', 'a'.repeat(32)]]) {
      const result = spawnSync(entry, [...args, ...suffix], { encoding: 'utf8' })
      assert.equal(result.status, 0, result.stderr)
      assert.deepEqual(result.stdout.trimEnd().split('\n'), [...args, ...suffix])
    }
    // Bash on Darwin uses locale collation for ranges; exercise that behavior on Linux too.
    for (const locale of ['C', 'en_US.UTF-8']) {
      const launch = (nonce: string) => spawnSync('/bin/bash', ['--posix', '-c', 'shopt -u globasciiranges 2>/dev/null || :; . "$0" "$@"', entry, ...args, '--recovery-peer-v1', nonce], { encoding: 'utf8', env: { ...process.env, LC_ALL: locale } })
      assert.equal(launch('0123456789abcdef'.repeat(2)).status, 0)
      for (const nonce of ['A', 'F', 'G', 'Z', 'é'].map(byte => byte.repeat(32))) {
        const rejected = launch(nonce)
        assert.equal(rejected.status, 2, JSON.stringify({ locale, nonce, stderr: rejected.stderr }))
        assert.equal(rejected.stdout, '', 'invalid nonce must not reach the worker')
      }
    }
    for (const suffix of [['--recovery-peer-v1'], ['--recovery-peer-v2', 'a'.repeat(32)], ['--recovery-peer-v1', 'A'.repeat(32)], ['--recovery-peer-v1', 'a'.repeat(31)]]) assert.equal(spawnSync(entry, [...args, ...suffix]).status, 2)
  } finally { rmSync(root, { recursive: true, force: true }) }
})
test('capture artifact inventory and absolute SDK launch contract', { skip: !artifactAvailable }, () => {
  const manifest = JSON.parse(readFileSync(manifestFile, 'utf8'))
  assert.equal(manifest.schema, 1)
  assert.match(manifest.sourceSha, /^[0-9a-f]{40}$/)
  assert.equal(manifest.target, 'linux-x64')
  validateRecoveryAdvertisement(manifest.recoveryLaunch, 'capture/2')
  assert.deepEqual(manifest.capabilities, ['capture/2', 'x11-display', 'x11-area', 'hyprland-single-display', 'retained-materials', 'authenticated-publication'])
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
