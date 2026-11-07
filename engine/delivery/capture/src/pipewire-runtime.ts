import { lstatSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'

const modules = ['protocol-native', 'client-node', 'client-device', 'adapter', 'metadata', 'session-manager'] as const
const spa = ['support/libspa-support.so', 'audioconvert/libspa-audioconvert.so'] as const

/** The standard vendor client profile is copied unchanged, together with its
 * explicitly supported dynamically loaded modules. ldd alone cannot find them. */
export function pipewireRuntimeInputs(libraryDirectory: string, shareDirectory: string): Array<{ source: string; path: string }> {
  const configuration = resolve(shareDirectory, 'pipewire/client.conf')
  const metadata = lstatSync(configuration)
  if (!metadata.isFile() || metadata.isSymbolicLink() || metadata.size > 65_536) throw new Error('unsupported PipeWire client configuration')
  const text = readFileSync(configuration, 'utf8').replace(/#[^\n]*/g, '')
  const configuredModules = [...text.matchAll(/\bname\s*=\s*([^\s}]+)/g)].map(match => match[1]).sort()
  const expectedModules = modules.map(name => `libpipewire-module-${name}`).sort()
  if (JSON.stringify(configuredModules) !== JSON.stringify(expectedModules)) throw new Error('unsupported PipeWire client module profile')
  const blocks = [...text.matchAll(/context\.spa-libs\s*=\s*\{([^}]*)\}/g)]
  const mapping = /([a-z.]+\*)\s*=\s*([a-z]+\/libspa-[a-z]+)/g
  const declarations = blocks[0]?.[1] ?? ''
  const mappings = [...declarations.matchAll(mapping)].map(match => `${match[1]}=${match[2]}`).sort()
  if (blocks.length !== 1 || declarations.replace(mapping, '').trim() !== '' ||
      JSON.stringify(mappings) !== JSON.stringify(['audio.convert.*=audioconvert/libspa-audioconvert', 'support.*=support/libspa-support'])) {
    throw new Error('unsupported PipeWire SPA profile')
  }
  const inputs = [
    { source: configuration, path: 'pipewire-config/client.conf' },
    ...modules.map(name => ({ source: resolve(libraryDirectory, `pipewire-0.3/libpipewire-module-${name}.so`), path: `pipewire/libpipewire-module-${name}.so` })),
    ...spa.map(name => ({ source: resolve(libraryDirectory, 'spa-0.2', name), path: `spa/${name}` })),
  ]
  for (const input of inputs) {
    const entry = lstatSync(input.source)
    if (!entry.isFile() || entry.isSymbolicLink()) throw new Error(`unsafe PipeWire runtime input: ${input.path}`)
  }
  return inputs
}
