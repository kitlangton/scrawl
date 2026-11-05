import assert from 'node:assert/strict'
import { mkdirSync, mkdtempSync, rmSync, statSync, symlinkSync, unlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { resolve } from 'node:path'
import { test } from 'node:test'
import { createHash } from 'node:crypto'
import { BASELINE_LIBRARIES, REQUIRED_PLUGINS, validateArtifact } from '../src/contract.ts'

function artifact() {
  const root = mkdtempSync(resolve(tmpdir(), 'engine-media-contract-'))
  mkdirSync(resolve(root, 'libexec')); mkdirSync(resolve(root, 'lib')); mkdirSync(resolve(root, 'plugins'))
  writeFileSync(resolve(root, 'apparent-engine-frames'), '#!/bin/sh\n', { mode: 0o755 })
  writeFileSync(resolve(root, 'libexec/apparent-engine-frames'), 'native-worker', { mode: 0o755 })
  for (const name of REQUIRED_PLUGINS) writeFileSync(resolve(root, `plugins/${name}`), 'plugin')
  const paths = ['apparent-engine-frames', ...REQUIRED_PLUGINS.map(name => `plugins/${name}`), 'libexec/apparent-engine-frames'].sort()
  const files = paths.map(path => {
    const bytes = requireBytes(resolve(root, path)); return { path, mode: statSync(resolve(root, path)).mode & 0o777, sha256: createHash('sha256').update(bytes).digest('hex') }
  })
  writeFileSync(resolve(root, 'manifest.json'), JSON.stringify({ schema: 1, sourceSha: 'a'.repeat(40), target: 'linux-x64', rustTriple: 'x86_64-unknown-linux-gnu', capabilities: ['media/1'], launch: { entry: 'apparent-engine-frames', argv: ['--media-runtime-directory', '<private-runtime-dir>', '--media-channel', 'media', '--media-protocol-version', '1'], worker: 'libexec/apparent-engine-frames', pathLookup: false, runtimeBuilds: false }, environment: { osRelease: 'fixture', kernel: 'fixture', architecture: 'x86_64', rustc: 'fixture', cargo: 'fixture', gstreamer: 'fixture', glibc: 'fixture', pluginSource: 'fixture', plugins: REQUIRED_PLUGINS, baseline: BASELINE_LIBRARIES }, runtimeFiles: files }))
  return root
}
import { readFileSync as requireBytes } from 'node:fs'

test('strict runtime inventory accepts only listed regular in-root files and detects tampering', () => {
  const root = artifact()
  try {
    validateArtifact(root)
    writeFileSync(resolve(root, 'plugins/extra.so'), 'extra')
    assert.throws(() => validateArtifact(root), /inventory/)
    rmSync(resolve(root, 'plugins/extra.so'))
    writeFileSync(resolve(root, 'plugins/libgstlibav.so'), 'changed')
    assert.throws(() => validateArtifact(root), /inventory/)
  } finally { rmSync(root, { recursive: true, force: true }) }
})

test('strict runtime inventory rejects launch identity changes', () => {
  const root = artifact()
  try {
    const path = resolve(root, 'manifest.json')
    const manifest = JSON.parse(requireBytes(path, 'utf8')) as { launch: { pathLookup: boolean } }
    manifest.launch.pathLookup = true
    writeFileSync(path, JSON.stringify(manifest))
    assert.throws(() => validateArtifact(root), /launch identity/)
  } finally { rmSync(root, { recursive: true, force: true }) }
})

test('strict runtime inventory rejects symlinks and malformed paths', () => {
  const root = artifact()
  const rootLink = `${root}-parent-link`
  try {
    symlinkSync(root, rootLink)
    assert.throws(() => validateArtifact(rootLink), /symlink/)
    unlinkSync(rootLink)
    symlinkSync('/etc/passwd', resolve(root, 'plugins/escape.so'))
    assert.throws(() => validateArtifact(root), /symlink/)
    rmSync(resolve(root, 'plugins/escape.so'))
    const path = resolve(root, 'manifest.json')
    const manifest = JSON.parse(requireBytes(path, 'utf8')) as { runtimeFiles: Array<{ path: string }> }
    manifest.runtimeFiles[0].path = '../outside'
    writeFileSync(path, JSON.stringify(manifest))
    assert.throws(() => validateArtifact(root))
  } finally { rmSync(root, { recursive: true, force: true }) }
})
