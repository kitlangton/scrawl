import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { chmodSync, mkdtempSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { resolve } from 'node:path'
import { after, test } from 'node:test'
import { manifestFor, targetInfo, validateArtifact } from '../src/contract.ts'

const temp = mkdtempSync(resolve(tmpdir(), 'config-native-contract-'))
after(() => rmSync(temp, { recursive: true, force: true }))

test('producer registration covers both native and typed client contract inputs', () => {
  const registration = JSON.parse(readFileSync(new URL('../delivery.json', import.meta.url), 'utf8'))
  assert.deepEqual(registration.inputs, ['composition/config/', 'shared/config-native/', 'shared/contracts/', '.cargo/'])
  assert.deepEqual(registration.verify, ['verify'])
  const workflow = readFileSync(new URL('../../../../.github/workflows/stream-platform.yml', import.meta.url), 'utf8')
  for (const path of ['composition/config/**', 'shared/config-native/**', 'shared/contracts/**', '.cargo/**']) assert(workflow.includes(`- '${path}'`), `workflow must trigger on ${path}`)
})

test('native artifact contract binds inventory, full SHA, target triple, bytes, protocol and executable mode', () => {
  const dir = resolve(temp, 'valid'); mkdirSync(dir)
  const bytes = Buffer.from('synthetic executable')
  const executable = resolve(dir, 'apparent-config-host')
  writeFileSync(executable, bytes, { mode: 0o755 })
  chmodSync(executable, 0o755)
  const sourceSha = 'a'.repeat(40)
  const manifest = manifestFor(sourceSha, 'linux-x64', bytes, statSync(executable).mode & 0o777)
  writeFileSync(resolve(dir, 'manifest.json'), `${JSON.stringify(manifest)}\n`)
  assert.deepEqual(readdirSync(dir).sort(), ['apparent-config-host', 'manifest.json'])
  assert.equal(validateArtifact(dir, 'linux-x64').sha256, createHash('sha256').update(bytes).digest('hex'))
  assert.equal(manifest.triple, 'x86_64-unknown-linux-gnu')
  assert.deepEqual(manifest.args, ['--config', '<caller-config-path>', '--owner', '<compiled-owner-id>'])
  assert.throws(() => validateArtifact(dir, 'macos-x64'), /manifest/)
  writeFileSync(resolve(dir, 'extra-resource'), 'not allowed')
  assert.throws(() => validateArtifact(dir, 'linux-x64'), /exactly/)
})

test('target mapping and manifest construction reject unsupported targets and modes', () => {
  assert.deepEqual(targetInfo('macos-x64', 'darwin', 'x64'), { target: 'macos-x64', triple: 'x86_64-apple-darwin' })
  assert.throws(() => targetInfo('macos-x64', 'linux', 'x64'), /Unsupported/)
  assert.throws(() => targetInfo('linux-x64', 'linux', 'arm64'), /Unsupported/)
  assert.throws(() => targetInfo('linux-arm64'), /Unsupported/)
  assert.throws(() => manifestFor('a'.repeat(40), 'linux-x64', Buffer.from('x'), 0o644), /0755/)
  assert.throws(() => manifestFor('short', 'linux-x64', Buffer.from('x'), 0o755), /full source SHA/)
  assert.throws(() => manifestFor('a'.repeat(41), 'linux-x64', Buffer.from('x'), 0o755), /full source SHA/)
})

test('artifact validation refuses changed bytes, wrong modes and symlinked payloads', () => {
  const dir = resolve(temp, 'tamper'); mkdirSync(dir)
  const executable = resolve(dir, 'apparent-config-host')
  const bytes = Buffer.from('expected')
  writeFileSync(executable, bytes, { mode: 0o755 }); chmodSync(executable, 0o755)
  writeFileSync(resolve(dir, 'manifest.json'), JSON.stringify(manifestFor('b'.repeat(40), 'linux-x64', bytes, 0o755)))
  writeFileSync(executable, 'changed'); assert.throws(() => validateArtifact(dir, 'linux-x64'), /manifest/)
  writeFileSync(executable, bytes); chmodSync(executable, 0o775); assert.throws(() => validateArtifact(dir, 'linux-x64'), /0755/)
  chmodSync(executable, 0o755)
  const alias = resolve(temp, 'alias'); symlinkSync(dir, alias); assert.throws(() => validateArtifact(alias, 'linux-x64'), /directory/)
  const outside = resolve(temp, 'outside'); writeFileSync(outside, bytes, { mode: 0o755 })
  rmSync(executable); symlinkSync(outside, executable); assert.throws(() => validateArtifact(dir, 'linux-x64'), /regular/)
})
