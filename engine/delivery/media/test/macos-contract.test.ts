import assert from 'node:assert/strict'
import { mkdirSync, mkdtempSync, realpathSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { resolve } from 'node:path'
import test from 'node:test'
import { inventory } from '../src/contract.ts'
import { MAC_FILES, validateMacArtifact } from '../src/macos-contract.ts'

test('Mac manifest binds helper inventory, native architecture and system closure', () => {
  const root = mkdtempSync(resolve(realpathSync(tmpdir()), 'engine-mac-manifest-'))
  try {
    mkdirSync(resolve(root, 'libexec'))
    const macho = Buffer.alloc(32); macho.writeUInt32LE(0xfeedfacf, 0); macho.writeUInt32LE(0x01000007, 4)
    for (const file of MAC_FILES) writeFileSync(resolve(root, file), file.startsWith('libexec/') ? macho : '#!/bin/sh\n', { mode: 0o755 })
    const manifest = { schema: 1, sourceSha: 'a'.repeat(40), target: 'macos-x64', rustTriple: 'x86_64-apple-darwin', capabilities: ['media/1'], launch: { entry: MAC_FILES[0], worker: MAC_FILES[1], nativeHelper: MAC_FILES[2], argv: ['--media-runtime-directory', '<private-runtime-dir>', '--media-channel', 'media', '--media-protocol-version', '1'], pathLookup: false, runtimeBuilds: false }, environment: { architecture: 'x86_64', minimumMacOS: '15.0', osRelease: 'fixture', sdk: 'fixture', swift: 'fixture', rustc: 'fixture', cargo: 'fixture', systemLibraries: ['/usr/lib/libSystem.B.dylib'] }, runtimeFiles: inventory(root) }
    const save = () => writeFileSync(resolve(root, 'manifest.json'), JSON.stringify(manifest))
    save(); assert.equal(validateMacArtifact(root).target, 'macos-x64')
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
