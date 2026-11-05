import { createHash } from 'node:crypto'
import { lstatSync, readdirSync, readFileSync, realpathSync, statSync } from 'node:fs'
import { isAbsolute, relative, resolve, sep } from 'node:path'

export type InventoryEntry = { path: string; mode: number; sha256: string }
export type MediaManifest = {
  schema: 1; sourceSha: string; target: 'linux-x64'; rustTriple: 'x86_64-unknown-linux-gnu'
  capabilities: ['media/1']; launch: { entry: 'apparent-engine-frames'; argv: ['--media-runtime-directory', '<private-runtime-dir>', '--media-channel', 'media', '--media-protocol-version', '1']; worker: 'libexec/apparent-engine-frames'; pathLookup: false; runtimeBuilds: false }
  environment: { osRelease: string; kernel: string; architecture: 'x86_64'; rustc: string; cargo: string; gstreamer: string; glibc: string; pluginSource: string; plugins: string[]; baseline: string[] }
  runtimeFiles: InventoryEntry[]
}

export const REQUIRED_PLUGINS = ['libgstcoreelements.so', 'libgsttypefindfunctions.so', 'libgstplayback.so', 'libgstapp.so', 'libgstvideoconvertscale.so', 'libgstisomp4.so', 'libgstlibav.so'] as const
export const BASELINE_LIBRARIES = ['ld-linux-x86-64.so.2', 'libc.so.6', 'libdl.so.2', 'libgcc_s.so.1', 'libm.so.6', 'libpthread.so.0', 'librt.so.1'] as const

export function sha256(data: Buffer) { return createHash('sha256').update(data).digest('hex') }
export function inventory(root: string): InventoryEntry[] {
  const entries: InventoryEntry[] = []
  const walk = (dir: string) => { for (const name of readdirSync(dir).sort()) { const path = resolve(dir, name); const rel = relative(root, path).split(sep).join('/'); const st = lstatSync(path); if (st.isSymbolicLink()) throw new Error(`symlink is forbidden: ${rel}`); if (st.isDirectory()) walk(path); else if (st.isFile()) entries.push({ path: rel, mode: st.mode & 0o777, sha256: sha256(readFileSync(path)) }); else throw new Error(`unsupported artifact entry: ${rel}`) } }
  walk(root); return entries
}
export function validateArtifact(directory: string): MediaManifest {
  const absolute = resolve(directory)
  let component: string = sep
  for (const part of absolute.split(sep).filter(Boolean)) { component = resolve(component, part); if (lstatSync(component).isSymbolicLink()) throw new Error('symlink in artifact path') }
  if (!statSync(absolute).isDirectory()) throw new Error('artifact is not a directory')
  const root = realpathSync(absolute)
  const manifestPath = resolve(root, 'manifest.json'); const manifest = JSON.parse(readFileSync(manifestPath, 'utf8')) as MediaManifest
  if (manifest.schema !== 1 || !/^[0-9a-f]{40}$/.test(manifest.sourceSha) || manifest.target !== 'linux-x64' || manifest.rustTriple !== 'x86_64-unknown-linux-gnu' || JSON.stringify(manifest.capabilities) !== '["media/1"]') throw new Error('unsupported media artifact identity')
  if (manifest.launch.entry !== 'apparent-engine-frames' || JSON.stringify(manifest.launch.argv) !== JSON.stringify(['--media-runtime-directory', '<private-runtime-dir>', '--media-channel', 'media', '--media-protocol-version', '1']) || manifest.launch.worker !== 'libexec/apparent-engine-frames' || manifest.launch.pathLookup !== false || manifest.launch.runtimeBuilds !== false) throw new Error('unsupported media launch identity')
  if (manifest.environment.architecture !== 'x86_64' || !manifest.environment.osRelease || !manifest.environment.kernel || !manifest.environment.rustc || !manifest.environment.cargo || !manifest.environment.gstreamer || !manifest.environment.glibc || !manifest.environment.pluginSource || JSON.stringify(manifest.environment.plugins) !== JSON.stringify(REQUIRED_PLUGINS) || JSON.stringify(manifest.environment.baseline) !== JSON.stringify(BASELINE_LIBRARIES)) throw new Error('invalid media runtime identity')
  const actual = inventory(root).filter(x => x.path !== 'manifest.json')
  if (!Array.isArray(manifest.runtimeFiles) || new Set(manifest.runtimeFiles.map(x => x.path)).size !== manifest.runtimeFiles.length || JSON.stringify(actual) !== JSON.stringify(manifest.runtimeFiles)) throw new Error('artifact inventory does not match manifest')
  const wanted = ['apparent-engine-frames', 'libexec/apparent-engine-frames', ...REQUIRED_PLUGINS.map(name => `plugins/${name}`), ...actual.filter(x => x.path.startsWith('lib/')).map(x => x.path)].sort()
  if (JSON.stringify(actual.map(x => x.path).sort()) !== JSON.stringify([...new Set(wanted)].sort())) throw new Error('artifact has undeclared runtime files')
  for (const executable of ['apparent-engine-frames', 'libexec/apparent-engine-frames']) { const item = actual.find(x => x.path === executable); if (!item || item.mode !== 0o755) throw new Error('invalid launch executable') }
  for (const item of actual) { if (item.path.startsWith('/') || item.path.split('/').includes('..') || item.path.split('/').includes('') || isAbsolute(item.path)) throw new Error('unsafe inventory path') }
  return manifest
}
