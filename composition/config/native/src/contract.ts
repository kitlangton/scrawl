import { createHash } from 'node:crypto'
import { lstatSync, readdirSync, readFileSync, statSync } from 'node:fs'
import { arch, platform } from 'node:os'
import { resolve } from 'node:path'

export type ConfigTarget = 'linux-x64' | 'macos-x64'
export type ConfigTriple = 'x86_64-unknown-linux-gnu' | 'x86_64-apple-darwin'
export type ConfigManifest = {
  schema: 1
  sourceSha: string
  target: ConfigTarget
  triple: ConfigTriple
  protocol: 1
  sha256: string
  mode: 0o755
  args: ['--config', '<caller-config-path>', '--owner', '<compiled-owner-id>']
  runtimeFiles: []
  appleSigning: 'unsigned-unnotarized'
  runtime: { pathLookup: false; cargo: false; npm: false }
}

export function targetInfo(value: string | undefined, hostOS = platform(), hostArch = arch()): { target: ConfigTarget; triple: ConfigTriple } {
  if (hostArch === 'x64' && value === 'linux-x64' && hostOS === 'linux') return { target: value, triple: 'x86_64-unknown-linux-gnu' }
  if (hostArch === 'x64' && value === 'macos-x64' && hostOS === 'darwin') return { target: value, triple: 'x86_64-apple-darwin' }
  throw new Error(`Unsupported config helper target: ${value ?? '(missing)'}`)
}

export function manifestFor(sourceSha: string, target: ConfigTarget, bytes: Uint8Array, mode: number): ConfigManifest {
  if (!/^(?:[0-9a-f]{40}|[0-9a-f]{64})$/.test(sourceSha)) throw new Error('Invalid full source SHA')
  const triple = target === 'linux-x64' ? 'x86_64-unknown-linux-gnu' : target === 'macos-x64' ? 'x86_64-apple-darwin' : undefined
  if (!triple || !bytes.length) throw new Error('Invalid target or empty helper')
  if (mode !== 0o755) throw new Error('Config helper must have mode 0755')
  return { schema: 1, sourceSha, target, triple, protocol: 1, sha256: createHash('sha256').update(bytes).digest('hex'), mode: 0o755, args: ['--config', '<caller-config-path>', '--owner', '<compiled-owner-id>'], runtimeFiles: [], appleSigning: 'unsigned-unnotarized', runtime: { pathLookup: false, cargo: false, npm: false } }
}

export function validateArtifact(directory: string, expectedTarget: ConfigTarget): ConfigManifest {
  if (!lstatSync(directory).isDirectory() || lstatSync(directory).isSymbolicLink()) throw new Error('Config artifact must be a real directory')
  const names = readdirSync(directory).sort()
  if (names.length !== 2 || names[0] !== 'apparent-config-host' || names[1] !== 'manifest.json') throw new Error('Config artifact must contain exactly the helper and manifest')
  const executable = resolve(directory, 'apparent-config-host')
  for (const name of names) if (!lstatSync(resolve(directory, name)).isFile() || lstatSync(resolve(directory, name)).isSymbolicLink()) throw new Error('Config artifact entries must be regular files')
  const mode = statSync(executable).mode & 0o777
  const manifest = JSON.parse(readFileSync(resolve(directory, 'manifest.json'), 'utf8')) as ConfigManifest
  const expected = manifestFor(manifest.sourceSha, expectedTarget, readFileSync(executable), mode)
  if (JSON.stringify(manifest) !== JSON.stringify(expected)) throw new Error('Config artifact manifest does not match target, bytes, mode or protocol contract')
  return manifest
}
