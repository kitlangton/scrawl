import { createHash } from 'node:crypto'
import { lstatSync, readdirSync, readFileSync, realpathSync, statSync } from 'node:fs'
import { arch, platform } from 'node:os'
import { isAbsolute, relative, resolve, sep } from 'node:path'

export type OwnerTarget = 'linux-x64' | 'macos-x64'

export type NativeManifest = {
  schema: 1
  sourceSha: string
  target: OwnerTarget
  rustTriple: string
  os: 'linux' | 'darwin'
  arch: 'x64'
  protocolVersion: 1
  executable: {
    path: 'apparent-engine'
    sha256: string
    mode: number
    launchIdentity: {
      argv: ['--runtime-directory', '<caller-provisioned-private-persistent-dir>']
      pathLookup: false
      cargoAtRuntime: false
      npmAtRuntime: false
    }
  }
  capture: { screenCapture: false; readiness: 'unavailable' }
  runtimeFiles: []
  appleSigning: 'unsigned-unnotarized'
}

const targets: Record<OwnerTarget, { os: 'linux' | 'darwin'; triple: string }> = {
  'linux-x64': { os: 'linux', triple: 'x86_64-unknown-linux-gnu' },
  'macos-x64': { os: 'darwin', triple: 'x86_64-apple-darwin' },
}

export function validateTarget(value: string | undefined, hostOS = platform(), hostArch = arch()): { target: OwnerTarget; os: 'linux' | 'darwin'; triple: string } {
  const localTarget = hostOS === 'linux' && hostArch === 'x64' ? 'linux-x64' : hostOS === 'darwin' && hostArch === 'x64' ? 'macos-x64' : undefined
  const target = value ?? localTarget
  if (target !== 'linux-x64' && target !== 'macos-x64') throw new Error('OWNER_TARGET must select linux-x64 or macos-x64 on a supported x64 host')
  const selected = targets[target]
  if (selected.os !== hostOS || hostArch !== 'x64') throw new Error(`Cross compilation is not supported: ${value} requires its native x64 host`)
  return { target, ...selected }
}

export function sha256(bytes: Buffer): string { return createHash('sha256').update(bytes).digest('hex') }

export function createManifest(sourceSha: string, target: OwnerTarget, binary: Buffer, mode: number): NativeManifest {
  if (!/^(?:[0-9a-f]{40}|[0-9a-f]{64})$/.test(sourceSha)) throw new Error('Invalid full source SHA')
  if (mode !== 0o755) throw new Error('Native executable mode must be 0755')
  const selected = targets[target]
  return {
    schema: 1,
    sourceSha,
    target,
    rustTriple: selected.triple,
    os: selected.os,
    arch: 'x64',
    protocolVersion: 1,
    executable: {
      path: 'apparent-engine',
      sha256: sha256(binary),
      mode: mode & 0o777,
      launchIdentity: {
        argv: ['--runtime-directory', '<caller-provisioned-private-persistent-dir>'],
        pathLookup: false,
        cargoAtRuntime: false,
        npmAtRuntime: false,
      },
    },
    capture: { screenCapture: false, readiness: 'unavailable' },
    runtimeFiles: [],
    appleSigning: 'unsigned-unnotarized',
  }
}

function rejectSymlinkTree(root: string, current = root) {
  for (const name of readdirSync(current)) {
    const path = resolve(current, name)
    const entry = lstatSync(path)
    if (entry.isSymbolicLink()) throw new Error(`Symlink in native artifact: ${relative(root, path)}`)
    if (entry.isDirectory()) rejectSymlinkTree(root, path)
    else if (!entry.isFile()) throw new Error(`Unsupported native artifact entry: ${relative(root, path)}`)
  }
}

export function validateArtifact(directory: string, expectedTarget: OwnerTarget): NativeManifest {
  if (lstatSync(directory).isSymbolicLink()) throw new Error('Artifact root must not be a symlink')
  const root = realpathSync(directory)
  if (!statSync(root).isDirectory()) throw new Error('Artifact is not a directory')
  rejectSymlinkTree(root)
  const names = readdirSync(root).sort()
  if (names.length !== 2 || names[0] !== 'apparent-engine' || names[1] !== 'manifest.json') throw new Error('Artifact inventory must contain only apparent-engine and manifest.json')
  const binaryPath = resolve(root, 'apparent-engine')
  const binary = readFileSync(binaryPath)
  const mode = statSync(binaryPath).mode & 0o777
  if (mode !== 0o755) throw new Error('Native artifact mode must be 0755')
  const manifest = JSON.parse(readFileSync(resolve(root, 'manifest.json'), 'utf8')) as NativeManifest
  if (manifest.schema !== 1 || manifest.target !== expectedTarget || manifest.protocolVersion !== 1 || manifest.executable.path !== 'apparent-engine' || manifest.executable.sha256 !== sha256(binary) || manifest.executable.mode !== mode || !/^(?:[0-9a-f]{40}|[0-9a-f]{64})$/.test(manifest.sourceSha)) throw new Error('Native artifact manifest does not match staged bytes')
  const expected = createManifest(manifest.sourceSha, manifest.target, binary, mode)
  if (JSON.stringify(manifest) !== JSON.stringify(expected)) throw new Error('Native artifact manifest contains unsupported or inaccurate fields')
  const executableRelative = relative(root, realpathSync(binaryPath))
  if (!executableRelative || executableRelative.startsWith(`..${sep}`) || isAbsolute(executableRelative)) throw new Error('Executable escapes artifact directory')
  return manifest
}
