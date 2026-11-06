import { lstatSync, readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { inventory, validateObservationAdvertisement, VISUAL_OBSERVATIONS, type InventoryEntry } from './contract.ts'
import { validateRecoveryAdvertisement, type RecoveryLaunch } from './recovery-contract.ts'

export const MAC_FILES = ['apparent-engine-frames', 'libexec/apparent-engine-frames', 'libexec/apparent-engine-frames-native'] as const
export type MacManifest = {
  recoveryLaunch?: RecoveryLaunch
  schema: 1; sourceSha: string; target: 'macos-x64'; rustTriple: 'x86_64-apple-darwin'; capabilities: ['media/1'] | ['media/1', 'media/2']
  media2Launch?: { argv: string[]; sharesCaptureRuntime: true }
  launch: { entry: 'apparent-engine-frames'; worker: 'libexec/apparent-engine-frames'; nativeHelper: 'libexec/apparent-engine-frames-native'; argv: string[]; pathLookup: false; runtimeBuilds: false }
  environment: { architecture: 'x86_64'; minimumMacOS: '15.0'; osRelease: string; sdk: string; swift: string; rustc: string; cargo: string; systemLibraries: string[] }
  runtimeFiles: InventoryEntry[]
  visualObservations?: typeof VISUAL_OBSERVATIONS
}
export function validateMacArtifact(root: string): MacManifest {
  root = resolve(root)
  let ancestor = root
  for (;;) { if (lstatSync(ancestor).isSymbolicLink()) throw new Error('symlink in artifact path'); const parent = resolve(ancestor, '..'); if (ancestor === parent) break; ancestor = parent }
  const m = JSON.parse(readFileSync(resolve(root, 'manifest.json'), 'utf8')) as MacManifest
  if (m.schema !== 1 || !/^[a-f0-9]{40}$/.test(m.sourceSha) || m.target !== 'macos-x64' || m.rustTriple !== 'x86_64-apple-darwin' || !['["media/1"]', '["media/1","media/2"]'].includes(JSON.stringify(m.capabilities))) throw new Error('unsupported Mac media identity')
  const media2 = m.capabilities.length === 2
  validateRecoveryAdvertisement(m.recoveryLaunch, 'media/2', media2)
  validateObservationAdvertisement(m.visualObservations, media2)
  if (media2 ? !m.media2Launch || m.media2Launch.sharesCaptureRuntime !== true || JSON.stringify(m.media2Launch.argv) !== JSON.stringify(['--media-runtime-directory', '<capture-runtime-dir>', '--media-channel', 'media', '--media-protocol-version', '2']) : m.media2Launch !== undefined) throw new Error('unsupported Mac retained-media launch')
  if (m.launch.entry !== MAC_FILES[0] || m.launch.worker !== MAC_FILES[1] || m.launch.nativeHelper !== MAC_FILES[2] || m.launch.pathLookup !== false || m.launch.runtimeBuilds !== false || JSON.stringify(m.launch.argv) !== JSON.stringify(['--media-runtime-directory', '<private-runtime-dir>', '--media-channel', 'media', '--media-protocol-version', '1'])) throw new Error('unsupported Mac launch')
  if (m.environment.architecture !== 'x86_64' || m.environment.minimumMacOS !== '15.0' || !m.environment.osRelease || !m.environment.sdk || !m.environment.swift || !m.environment.rustc || !m.environment.cargo || !Array.isArray(m.environment.systemLibraries) || !m.environment.systemLibraries.length || m.environment.systemLibraries.some(p => !p.startsWith('/usr/lib/') && !p.startsWith('/System/Library/'))) throw new Error('unsupported Mac runtime')
  const actual = inventory(root).filter(f => f.path !== 'manifest.json')
  if (JSON.stringify(actual.map(f => f.path).sort()) !== JSON.stringify([...MAC_FILES].sort()) || actual.some(f => f.mode !== 0o755) || JSON.stringify(actual) !== JSON.stringify(m.runtimeFiles)) throw new Error('Mac inventory mismatch')
  for (const path of MAC_FILES.slice(1)) {
    const bytes = readFileSync(resolve(root, path))
    if (bytes.length < 32 || bytes.readUInt32LE(0) !== 0xfeedfacf || bytes.readUInt32LE(4) !== 0x01000007) throw new Error('expected thin x86_64 Mach-O')
  }
  return m
}
