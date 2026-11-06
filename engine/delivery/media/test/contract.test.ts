import assert from 'node:assert/strict'
import { mkdirSync, mkdtempSync, realpathSync, rmSync, statSync, symlinkSync, unlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { resolve } from 'node:path'
import { test } from 'node:test'
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { BASELINE_LIBRARIES, REQUIRED_PLUGINS, validateArtifact, VISUAL_OBSERVATIONS } from '../src/contract.ts'
import { recoveryLaunch, validateRecoveryAdvertisement } from '../src/recovery-contract.ts'

function artifact() {
  const root = mkdtempSync(resolve(realpathSync(tmpdir()), 'engine-media-contract-'))
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

test('recovery advertisement binds role/channel/argv without inferring release from old artifacts', () => {
  validateRecoveryAdvertisement(undefined, 'media/2')
  for (const channel of ['capture/2', 'media/2'] as const) {
    const expected = recoveryLaunch(channel)
    validateRecoveryAdvertisement(JSON.parse(JSON.stringify(expected)), channel)
    for (const value of [{ ...expected, startsFreshCapture: true }, { ...expected, channel: channel === 'media/2' ? 'capture/2' : 'media/2' }, { ...expected, argv: expected.argv.slice(0, -2) }, { ...expected, onInputEof: 'cancel' }, { ...expected, unrecognized: true }]) assert.throws(() => validateRecoveryAdvertisement(value, channel), /recovery lifetime/)
    assert.throws(() => validateRecoveryAdvertisement(expected, channel, false), /recovery lifetime/)
  }
})

test('Linux entry forwards recovery only for media2 and refuses malformed nonce/version', () => {
  const root = mkdtempSync(resolve(realpathSync(tmpdir()), 'engine-media-argv-'))
  try {
    mkdirSync(resolve(root, 'libexec')); mkdirSync(resolve(root, 'private runtime'), { mode: 0o700 })
    const entry = resolve(root, 'apparent-engine-frames')
    writeFileSync(entry, requireBytes(new URL('../src/launch.sh', import.meta.url)), { mode: 0o755 })
    writeFileSync(resolve(root, 'libexec/apparent-engine-frames'), '#!/bin/sh\nprintf "%s\\n" "$@"\n', { mode: 0o755 })
    const args = ['--media-runtime-directory', resolve(root, 'private runtime'), '--media-channel', 'media', '--media-protocol-version', '2']
    const suffix = ['--recovery-peer-v1', 'a'.repeat(32)]
    const result = spawnSync(entry, [...args, ...suffix], { encoding: 'utf8' })
    assert.equal(result.status, 0, result.stderr)
    assert.deepEqual(result.stdout.trimEnd().split('\n'), [...args, ...suffix])
    assert.equal(spawnSync(entry, [...args.slice(0, -1), '1', ...suffix]).status, 2)
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
    for (const invalid of [['--recovery-peer-v1'], ['--recovery-peer-v2', 'a'.repeat(32)], ['--recovery-peer-v1', 'A'.repeat(32)], ['--recovery-peer-v1', 'a'.repeat(31)]]) assert.equal(spawnSync(entry, [...args, ...invalid]).status, 2)
  } finally { rmSync(root, { recursive: true, force: true }) }
})

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

test('retained media advertises an explicit shared-capture-runtime launch without changing media1', () => {
  const root = artifact()
  try {
    const path = resolve(root, 'manifest.json')
    const manifest = JSON.parse(requireBytes(path, 'utf8'))
    manifest.visualObservations = VISUAL_OBSERVATIONS
    writeFileSync(path, JSON.stringify(manifest))
    assert.throws(() => validateArtifact(root), /visual observation/)
    manifest.capabilities = ['media/1', 'media/2']
    writeFileSync(path, JSON.stringify(manifest))
    assert.throws(() => validateArtifact(root), /retained media launch identity/)
    manifest.media2Launch = { argv: ['--media-runtime-directory', '<capture-runtime-dir>', '--media-channel', 'media', '--media-protocol-version', '2'], sharesCaptureRuntime: true }
    writeFileSync(path, JSON.stringify(manifest))
    validateArtifact(root)
    manifest.visualObservations = { ...VISUAL_OBSERVATIONS, chunkType: 'wrong' }
    writeFileSync(path, JSON.stringify(manifest))
    assert.throws(() => validateArtifact(root), /visual observation/)
    manifest.visualObservations = VISUAL_OBSERVATIONS
    manifest.media2Launch.sharesCaptureRuntime = false
    writeFileSync(path, JSON.stringify(manifest))
    assert.throws(() => validateArtifact(root), /retained media launch identity/)
  } finally { rmSync(root, { recursive: true, force: true }) }
})
