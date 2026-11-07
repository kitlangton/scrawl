import assert from 'node:assert/strict'
import { mkdirSync, mkdtempSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { spawnSync } from 'node:child_process'
import test from 'node:test'
import { pipewireRuntimeInputs } from '../src/pipewire-runtime.ts'

const modules = ['protocol-native', 'client-node', 'client-device', 'adapter', 'metadata', 'session-manager']
const configuration = `context.spa-libs = {
  audio.convert.* = audioconvert/libspa-audioconvert
  support.* = support/libspa-support
}
context.modules = [
${modules.map(name => `{ name = libpipewire-module-${name} }`).join('\n')}
]\n`

test('capture closure includes the standard client configuration, every module and both SPA factories', () => {
  const root = mkdtempSync(join(tmpdir(), 'capture-pipewire-inputs-'))
  try {
    const library = join(root, 'lib'); const share = join(root, 'share')
    const sources = ['share/pipewire/client.conf', ...modules.map(name => `lib/pipewire-0.3/libpipewire-module-${name}.so`),
      'lib/spa-0.2/support/libspa-support.so', 'lib/spa-0.2/audioconvert/libspa-audioconvert.so']
    for (const source of sources) {
      const path = join(root, source); mkdirSync(dirname(path), { recursive: true })
      writeFileSync(path, source.endsWith('.conf') ? configuration : source)
    }
    const inputs = pipewireRuntimeInputs(library, share)
    assert.equal(inputs.length, 9)
    assert.equal(inputs[0].path, 'pipewire-config/client.conf')
    assert.equal(readFileSync(inputs[0].source, 'utf8'), configuration)
    assert.deepEqual(inputs.map(input => input.source).sort(), sources.map(source => join(root, source)).sort())
    const config = sources[0]
    writeFileSync(join(root, config), configuration.replace('module-adapter', 'module-foreign'))
    assert.throws(() => pipewireRuntimeInputs(library, share), /module profile/)
    writeFileSync(join(root, config), configuration.replace('support/libspa-support', '../foreign/libspa-support'))
    assert.throws(() => pipewireRuntimeInputs(library, share), /SPA profile/)
    writeFileSync(join(root, config), configuration)
    const module = join(root, sources[1]); rmSync(module); symlinkSync(join(root, sources[2]), module)
    assert.throws(() => pipewireRuntimeInputs(library, share), /unsafe PipeWire runtime input/)
  } finally { rmSync(root, { recursive: true, force: true }) }
})

test('capture launch pins PipeWire closure and preserves the caller-owned desktop session', () => {
  const root = mkdtempSync(join(tmpdir(), 'capture-pipewire-launch-'))
  try {
    mkdirSync(join(root, 'libexec')); mkdirSync(join(root, 'private'), { mode: 0o700 })
    const entry = join(root, 'apparent-engine-capture-launch')
    writeFileSync(entry, readFileSync(new URL('../src/launch.sh', import.meta.url)), { mode: 0o755 })
    writeFileSync(join(root, 'libexec/apparent-engine-capture'), '#!/bin/sh\nprintf "%s\\n" "$PIPEWIRE_MODULE_DIR" "$SPA_PLUGIN_DIR" "$PIPEWIRE_CONFIG_DIR" "$PIPEWIRE_CONFIG_NAME" "$PIPEWIRE_CONFIG_PREFIX" "${PIPEWIRE_NO_CONFIG-unset}" "$WAYLAND_DISPLAY" "$DBUS_SESSION_BUS_ADDRESS"\n', { mode: 0o755 })
    const result = spawnSync(entry, ['--runtime-directory', join(root, 'private'), '--capture-channel', 'capture', '--capture-protocol-version', '2'],
      { encoding: 'utf8', env: { ...process.env, PIPEWIRE_MODULE_DIR: '/foreign', SPA_PLUGIN_DIR: '/foreign', PIPEWIRE_CONFIG_DIR: '/foreign',
        PIPEWIRE_CONFIG_NAME: 'foreign.conf', PIPEWIRE_CONFIG_PREFIX: '/foreign', PIPEWIRE_NO_CONFIG: '1',
        WAYLAND_DISPLAY: 'wayland-test-owned', DBUS_SESSION_BUS_ADDRESS: 'unix:path=/test-owned-session' } })
    assert.equal(result.status, 0, result.stderr)
    assert.deepEqual(result.stdout.trimEnd().split('\n'), [join(root, 'pipewire'), join(root, 'spa'), join(root, 'pipewire-config'), 'client.conf', '', 'unset',
      'wayland-test-owned', 'unix:path=/test-owned-session'])
  } finally { rmSync(root, { recursive: true, force: true }) }
})
