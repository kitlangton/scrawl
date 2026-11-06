import assert from 'node:assert/strict'
import { chmodSync, readFileSync, mkdirSync, mkdtempSync, realpathSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { execFileSync, spawnSync } from 'node:child_process'
import { tmpdir } from 'node:os'
import { resolve } from 'node:path'
import test from 'node:test'
import { inventory, VISUAL_OBSERVATIONS } from '../src/contract.ts'
import { MAC_FILES, validateMacArtifact, type MacManifest } from '../src/macos-contract.ts'

test('Mac manifest binds helper inventory, native architecture and system closure', () => {
  const root = mkdtempSync(resolve(realpathSync(tmpdir()), 'engine-mac-manifest-'))
  try {
    mkdirSync(resolve(root, 'libexec'))
    const macho = Buffer.alloc(32); macho.writeUInt32LE(0xfeedfacf, 0); macho.writeUInt32LE(0x01000007, 4)
    for (const file of MAC_FILES) writeFileSync(resolve(root, file), file.startsWith('libexec/') ? macho : '#!/bin/sh\n', { mode: 0o755 })
    const manifest: MacManifest = { schema: 1, sourceSha: 'a'.repeat(40), target: 'macos-x64', rustTriple: 'x86_64-apple-darwin', capabilities: ['media/1'], launch: { entry: MAC_FILES[0], worker: MAC_FILES[1], nativeHelper: MAC_FILES[2], argv: ['--media-runtime-directory', '<private-runtime-dir>', '--media-channel', 'media', '--media-protocol-version', '1'], pathLookup: false, runtimeBuilds: false }, environment: { architecture: 'x86_64', minimumMacOS: '15.0', osRelease: 'fixture', sdk: 'fixture', swift: 'fixture', rustc: 'fixture', cargo: 'fixture', systemLibraries: ['/usr/lib/libSystem.B.dylib'] }, runtimeFiles: inventory(root) }
    const save = () => writeFileSync(resolve(root, 'manifest.json'), JSON.stringify(manifest))
    save(); assert.equal(validateMacArtifact(root).target, 'macos-x64')
    manifest.visualObservations = VISUAL_OBSERVATIONS; save()
    assert.throws(() => validateMacArtifact(root), /visual observation/)
    manifest.capabilities = ['media/1', 'media/2']
    manifest.media2Launch = { argv: ['--media-runtime-directory', '<capture-runtime-dir>', '--media-channel', 'media', '--media-protocol-version', '2'], sharesCaptureRuntime: true }
    save(); assert.equal(validateMacArtifact(root).media2Launch?.sharesCaptureRuntime, true)
    manifest.visualObservations = { ...VISUAL_OBSERVATIONS, chunkType: 'wrong' } as unknown as typeof VISUAL_OBSERVATIONS; save()
    assert.throws(() => validateMacArtifact(root), /visual observation/)
    manifest.visualObservations = VISUAL_OBSERVATIONS; save()
    manifest.media2Launch.argv[5] = '1'; save()
    assert.throws(() => validateMacArtifact(root), /retained-media launch/)
    manifest.media2Launch.argv[5] = '2'; save()

    manifest.environment.systemLibraries = ['/opt/homebrew/lib/unshipped.dylib']; save()
    assert.throws(() => validateMacArtifact(root), /runtime/)
    manifest.environment.systemLibraries = ['/usr/lib/libSystem.B.dylib']; save()
    writeFileSync(resolve(root, MAC_FILES[2]), Buffer.alloc(32))
    manifest.runtimeFiles = inventory(root).filter(f => f.path !== 'manifest.json'); save()
    assert.throws(() => validateMacArtifact(root), /Mach-O/)
    rmSync(resolve(root, MAC_FILES[2])); symlinkSync('/bin/sh', resolve(root, MAC_FILES[2]))
    assert.throws(() => validateMacArtifact(root), /symlink/)
  } finally { rmSync(root, { recursive: true, force: true }) }
})

test('Mac entry forwards both published channels and rejects unsupported versions', () => {
  const root = mkdtempSync(resolve(realpathSync(tmpdir()), 'engine-mac-launch-'))
  try {
    mkdirSync(resolve(root, 'libexec')); mkdirSync(resolve(root, 'private runtime'), { mode: 0o700 })
    const entry = resolve(root, 'apparent-engine-frames')
    writeFileSync(entry, readFileSync(new URL('../src/launch-macos.sh', import.meta.url)), { mode: 0o755 })
    writeFileSync(resolve(root, MAC_FILES[1]), '#!/bin/sh\nset -eu\n[ -z "${DYLD_LIBRARY_PATH+x}" ] || exit 7\nprintf "%s\\n" "$@"\n', { mode: 0o755 })
    chmodSync(entry, 0o755)
    for (const version of ['1', '2']) {
      const args = ['--media-runtime-directory', resolve(root, 'private runtime'), '--media-channel', 'media', '--media-protocol-version', version]
      const actual = execFileSync(entry, args, { encoding: 'utf8', env: { ...process.env, DYLD_LIBRARY_PATH: '/untrusted/ambient' } })
      assert.deepEqual(actual.trimEnd().split('\n'), args)
      const recovery = [...args, '--recovery-peer-v1', 'a'.repeat(32)]
      if (version === '2') {
        assert.deepEqual(execFileSync(entry, recovery, { encoding: 'utf8' }).trimEnd().split('\n'), recovery)
        for (const locale of ['C', 'en_US.UTF-8']) {
          const launch = (nonce: string) => spawnSync('/bin/bash', ['--posix', '-c', 'shopt -u globasciiranges 2>/dev/null || :; . "$0" "$@"', entry, ...args, '--recovery-peer-v1', nonce], { encoding: 'utf8', env: { ...process.env, LC_ALL: locale } })
          assert.equal(launch('0123456789abcdef'.repeat(2)).status, 0)
          for (const nonce of ['A', 'F', 'G', 'Z', 'é'].map(byte => byte.repeat(32))) {
            const rejected = launch(nonce)
            assert.equal(rejected.status, 2, JSON.stringify({ locale, nonce, stderr: rejected.stderr }))
            assert.equal(rejected.stdout, '', 'invalid nonce must not reach the worker')
          }
        }
        for (const invalid of [['--recovery-peer-v1'], ['--recovery-peer-v2', 'a'.repeat(32)], ['--recovery-peer-v1', 'A'.repeat(32)], ['--recovery-peer-v1', 'a'.repeat(31)]]) assert.throws(() => execFileSync(entry, [...args, ...invalid], { stdio: 'pipe' }), { status: 2 })
      } else assert.throws(() => execFileSync(entry, recovery, { stdio: 'pipe' }), { status: 2 })
    }
    assert.throws(() => execFileSync(entry, ['--media-runtime-directory', resolve(root, 'private runtime'), '--media-channel', 'media', '--media-protocol-version', '3'], { stdio: 'pipe' }), { status: 2 })
  } finally { rmSync(root, { recursive: true, force: true }) }
})
