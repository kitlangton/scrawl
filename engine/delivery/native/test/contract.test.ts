import assert from 'node:assert/strict'
import { chmodSync, mkdtempSync, mkdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { resolve } from 'node:path'
import { after, test } from 'node:test'
import { createManifest, validateArtifact, validateTarget } from '../src/contract.ts'

const temp = mkdtempSync(resolve(tmpdir(), 'engine-native-producer-'))
after(() => rmSync(temp, { recursive: true, force: true }))

test('targets require the exact supported native host', () => {
  assert.equal(validateTarget('linux-x64', 'linux', 'x64').triple, 'x86_64-unknown-linux-gnu')
  assert.equal(validateTarget('macos-x64', 'darwin', 'x64').triple, 'x86_64-apple-darwin')
  assert.equal(validateTarget(undefined, 'linux', 'x64').target, 'linux-x64')
  assert.equal(validateTarget(undefined, 'darwin', 'x64').target, 'macos-x64')
  assert.throws(() => validateTarget(undefined, 'freebsd', 'x64'), /OWNER_TARGET/)
  assert.throws(() => validateTarget('macos-x64', 'linux', 'x64'), /Cross compilation/)
  assert.throws(() => validateTarget('linux-x64', 'linux', 'arm64'), /Cross compilation/)
  assert.throws(() => validateTarget('unsupported', 'freebsd', 'x64'), /OWNER_TARGET/)
})

test('native registration preserves the SDK lane and covers inherited Cargo configuration', () => {
  const registration = JSON.parse(readFileSync(new URL('../delivery.json', import.meta.url), 'utf8'))
  assert.equal(registration.id, 'engine-native')
  assert.equal(registration.language, 'native')
  assert.deepEqual(registration.inputs, ['engine/', '.cargo/'])
  assert.equal(registration.artifactPath, 'dist')
  const sdk = JSON.parse(readFileSync(new URL('../../../delivery.json', import.meta.url), 'utf8'))
  assert.equal(sdk.package, 'engine/sdk')
  assert.equal(sdk.kind, 'npm')
})

test('manifest reports the exact binary hash and unavailable capture truthfully', () => {
  const manifest = createManifest('a'.repeat(40), 'linux-x64', Buffer.from('native'), 0o755)
  assert.equal(manifest.protocolVersion, 1)
  assert.deepEqual(manifest.capture, { screenCapture: false, readiness: 'unavailable' })
  assert.equal(manifest.executable.sha256, 'bef32d2c315a289576f2a6828d27edb16bb316a4d85c271f2d794045f3ea668d')
  assert.throws(() => createManifest('not-a-sha', 'linux-x64', Buffer.from('native'), 0o755), /source SHA/)
  assert.throws(() => createManifest('a'.repeat(41), 'linux-x64', Buffer.from('native'), 0o755), /source SHA/)
  assert.throws(() => createManifest('a'.repeat(40), 'linux-x64', Buffer.from('native'), 0o644), /mode must be 0755/)
})

test('inventory rejects extra, tampered, missing and symlinked artifact files', () => {
  const artifact = resolve(temp, 'artifact'); mkdirSync(artifact)
  const binary = resolve(artifact, 'apparent-engine'); writeFileSync(binary, 'candidate', { mode: 0o755 }); chmodSync(binary, 0o755)
  const manifest = createManifest('b'.repeat(40), 'linux-x64', Buffer.from('candidate'), 0o755)
  const manifestPath = resolve(artifact, 'manifest.json')
  writeFileSync(manifestPath, `${JSON.stringify(manifest, null, 2)}\n`)
  assert.deepEqual(validateArtifact(artifact, 'linux-x64'), manifest)
  writeFileSync(resolve(artifact, 'extra'), 'no')
  assert.throws(() => validateArtifact(artifact, 'linux-x64'), /inventory/)
  rmSync(resolve(artifact, 'extra'))
  writeFileSync(binary, 'tampered')
  assert.throws(() => validateArtifact(artifact, 'linux-x64'), /manifest/)
  writeFileSync(binary, 'candidate', { mode: 0o755 }); chmodSync(binary, 0o755)
  writeFileSync(resolve(artifact, 'other'), 'no')
  rmSync(resolve(artifact, 'other'))
  rmSync(manifestPath)
  symlinkSync(binary, manifestPath)
  assert.throws(() => validateArtifact(artifact, 'linux-x64'), /Symlink/)
  const rootLink = resolve(temp, 'artifact-link')
  symlinkSync(artifact, rootLink)
  assert.throws(() => validateArtifact(rootLink, 'linux-x64'), /root must not be a symlink/)
})
