import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { closeSync, openSync, cpSync, existsSync, lstatSync, mkdirSync, readlinkSync, readdirSync, readFileSync, realpathSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { arch, platform } from 'node:os'
import { dirname, isAbsolute, relative, resolve, sep } from 'node:path'

export type Target = 'linux-x64' | 'macos-x64' | 'web'
export type Kind = 'npm' | 'app-shell' | 'directory'
export type Stage = 'library' | 'contract' | 'development-app' | 'website'
export type Owner = { schema: 1; id: string; kind: Kind; package: string; inputs: string[]; verify: string[]; build: string; targets: Target[]; stage: Stage; language: 'typescript' | 'native'; artifactPath?: string; packageManager?: 'npm' | 'bun'; files?: string[]; environment?: string[] }
export const auditJob = 'Workflow runtime budget'
export const contractsJob = 'Delivery contracts'
export function workflowPathFor(ownerId: string): string {
  const stream = ['platform-config', 'platform-resources', 'app-shell'].includes(ownerId) ? 'platform' : ['engine-native', 'engine-media-native', 'engine-capture-native'].includes(ownerId) ? 'engine' : ownerId
  return `.github/workflows/stream-${stream}.yml`
}
const roots = [
  { id: 'app-shell', source: 'composition/app', registration: 'composition/app/delivery.json' },
  { id: 'record', source: 'record', registration: 'record/delivery.json' },
  { id: 'connect-client', source: 'connect/delivery/client', registration: 'connect/delivery.json' },
  { id: 'connect-browser', source: 'connect/tooling/hosting', registration: 'connect/tooling/hosting/delivery.json' },
  { id: 'engine', source: 'engine/sdk', registration: 'engine/delivery.json' },
  { id: 'engine-native', source: 'engine/delivery/native', registration: 'engine/delivery/native/delivery.json' },
  { id: 'engine-capture-native', source: 'engine/delivery/capture', registration: 'engine/delivery/capture/delivery.json' },
  { id: 'engine-media-native', source: 'engine/delivery/media', registration: 'engine/delivery/media/delivery.json' },
  { id: 'platform-config', source: 'composition/config/native', registration: 'composition/config/native/delivery.json' },
  { id: 'platform-resources', source: 'shared/resources', registration: 'shared/resources/delivery/delivery.json' },
  { id: 'onboarding', source: 'onboarding', registration: 'onboarding/delivery.json' },
  { id: 'agent', source: 'agent', registration: 'agent/delivery.json' },
  { id: 'public', source: 'public/site', registration: 'public/delivery.json' },
]
const mandatoryInputs = ['composition/delivery/', 'composition/architecture/']
const windows = (value: string) => /^[A-Za-z]:[\\/]/.test(value) || value.includes('\\')
function safeRepoPath(value: unknown, directory = false): string {
  if (typeof value !== 'string' || !value || isAbsolute(value) || windows(value) || value.startsWith('/')) throw new Error('Unsafe repository path')
  const normalized = directory ? value.replace(/\/$/, '') : value
  if (!normalized || normalized.split('/').some(p => !p || p === '.' || p === '..')) throw new Error('Unsafe repository path')
  return directory ? `${normalized}/` : normalized
}
function readOwner(path: string): Owner {
  const raw = JSON.parse(readFileSync(path, 'utf8')) as Record<string, unknown>
  if (raw.schema !== 1 || typeof raw.id !== 'string' || !/^[a-z][a-z0-9-]*$/.test(raw.id) || !['npm', 'app-shell', 'directory'].includes(String(raw.kind)) || !['library', 'contract', 'development-app', 'website'].includes(String(raw.stage)) || typeof raw.package !== 'string' || typeof raw.build !== 'string' || !Array.isArray(raw.inputs) || !Array.isArray(raw.verify) || !Array.isArray(raw.targets)) throw new Error(`Invalid delivery registration: ${path}`)
  const inputs = raw.inputs.map(x => safeRepoPath(x, true))
  const targets = raw.targets.map(x => { if (x !== 'linux-x64' && x !== 'macos-x64' && x !== 'web') throw new Error(`Unsupported target in ${path}`); return x })
  if (!inputs.length || new Set(inputs).size !== inputs.length || !targets.length || new Set(targets).size !== targets.length || raw.verify.some(x => typeof x !== 'string' || !/^[a-z][a-z0-9:_-]*$/.test(x))) throw new Error(`Invalid delivery registration: ${path}`)
  if (raw.packageManager !== undefined && !['npm', 'bun'].includes(String(raw.packageManager))) throw new Error(`Invalid package manager: ${path}`)
  if (raw.files !== undefined && (!Array.isArray(raw.files) || raw.files.some(x => typeof x !== 'string'))) throw new Error(`Invalid file inputs: ${path}`)
  const files = ((raw.files ?? []) as string[]).map(x => safeRepoPath(x))
  if (files.some(file => file.endsWith('/')) || new Set(files).size !== files.length) throw new Error(`Invalid file inputs: ${path}`)
  if (raw.environment !== undefined && (!Array.isArray(raw.environment) || raw.environment.some(x => typeof x !== 'string' || !/^[A-Z][A-Z0-9_]*$/.test(x)))) throw new Error(`Invalid environment inputs: ${path}`)
  const environment = (raw.environment ?? []) as string[]
  if (new Set(environment).size !== environment.length) throw new Error(`Duplicate environment inputs: ${path}`)
  const pkg = safeRepoPath(raw.package)
  const language = raw.language ?? 'typescript'
  if (language !== 'typescript' && language !== 'native') throw new Error(`Invalid language in ${path}`)
  if (raw.id === 'platform-resources' && (!inputs.includes('shared/resources/') || !inputs.includes('.cargo/'))) throw new Error(`platform-resources must fingerprint shared/resources/ and .cargo/: ${path}`)
  const artifactPath = raw.kind === 'directory' ? safeRepoPath(raw.artifactPath) : undefined
  if (raw.kind !== 'directory' && raw.artifactPath !== undefined) throw new Error(`artifactPath is only valid for directory registrations: ${path}`)
  if ((targets.includes('web') || raw.stage === 'website') && (raw.kind !== 'directory' || raw.stage !== 'website' || targets.length !== 1 || targets[0] !== 'web' || language !== 'typescript')) throw new Error(`Web delivery requires a TypeScript website directory: ${path}`)
  return { schema: 1, id: raw.id, kind: raw.kind as Kind, package: pkg, inputs, verify: raw.verify as string[], build: raw.build, targets, stage: raw.stage as Stage, language, artifactPath, packageManager: (raw.packageManager ?? 'npm') as 'npm' | 'bun', files, environment }
}
function git(root: string, args: string[]): string { return execFileSync('git', args, { cwd: root, encoding: 'utf8', env: { ...process.env, GIT_OPTIONAL_LOCKS: '0' }, stdio: ['ignore', 'pipe', 'pipe'] }) }
function assertNoSymlinkComponents(root: string, repoPath: string) {
  let current = root
  for (const component of repoPath.split('/')) {
    current = resolve(current, component)
    try { if (lstatSync(current).isSymbolicLink()) throw new Error(`Symlink in registered repository path: ${repoPath}`) }
    catch (error) { if ((error as NodeJS.ErrnoException).code === 'ENOENT') return; throw error }
  }
}
function pathEntryExists(path: string): boolean { try { lstatSync(path); return true } catch (error) { if ((error as NodeJS.ErrnoException).code === 'ENOENT') return false; throw error } }

export function catalog(root: string) {
  root = realpathSync(root)
  const owners: Owner[] = []
  const pending: string[] = []
  const blocked: string[] = []
  for (const entry of roots) {
    if (!pathEntryExists(resolve(root, entry.source))) { pending.push(entry.id); continue }
    if (!pathEntryExists(resolve(root, entry.registration))) { blocked.push(`${entry.id}: source exists without delivery registration`); continue }
    assertNoSymlinkComponents(root, entry.source)
    assertNoSymlinkComponents(root, entry.registration)
    try {
      const owner = readOwner(resolve(root, entry.registration))
      if (owner.id !== entry.id) throw new Error(`Registration ID mismatch: ${entry.registration}`)
      if (!owner.inputs.some(input => `${owner.package}/`.startsWith(input))) throw new Error('Package source must be covered by a declared input root')
      assertNoSymlinkComponents(root, owner.package)
      if (!statSync(resolve(root, owner.package)).isDirectory()) throw new Error(`Package path is not a directory: ${owner.package}`)
      for (const input of owner.inputs) { const path = input.replace(/\/$/, ''); assertNoSymlinkComponents(root, path); if (!existsSync(resolve(root, path))) throw new Error(`Input root is missing: ${input}`) }
      for (const file of owner.files ?? []) { assertNoSymlinkComponents(root, file); if (existsSync(resolve(root, file)) && !statSync(resolve(root, file)).isFile()) throw new Error(`File input is not a regular file: ${file}`) }
      owners.push(owner)
    } catch (error) { blocked.push(`${entry.id}: ${error instanceof Error ? error.message : 'invalid registration'}`) }
  }
  return { owners, pending, blocked }
}

function trackedInputs(root: string, owner: Owner, revision = 'HEAD'): { digest: string; entries: string[] } {
  const registration = roots.find(x => x.id === owner.id)?.registration
  if (!registration) throw new Error('Owner has no registered source identity')
  const inputRoots = [...new Set([...owner.inputs, ...mandatoryInputs, ...(owner.id === 'public' ? ['enablement/verification/web/'] : [])])].sort()
  const files = [...(owner.files ?? []), workflowPathFor(owner.id), '.github/actions/delivery-contracts/', '.github/actions/delivery-component/', '.github/actions/runtime-budget/', 'enablement/verification/workflow-budget.ts', 'LICENSE', registration]
  const paths = new Set<string>()
  for (const pathspec of [...inputRoots, ...files]) {
    const result = git(root, ['ls-tree', '-r', '-z', '--full-tree', revision, '--', pathspec])
    if (result.includes('\uFFFD')) throw new Error('Cannot fingerprint ambiguous non-UTF8 Git paths')
    for (const entry of result.split('\0').filter(Boolean)) {
      const tab = entry.indexOf('\t')
      if (tab < 0) throw new Error('Malformed git tree entry')
      const metadata = entry.slice(0, tab).split(' ')
      const path = entry.slice(tab + 1)
      safeRepoPath(path)
      if (metadata.length !== 3 || !/^(100644|100755|120000)$/.test(metadata[0]) || !/^[0-9a-f]{40,64}$/.test(metadata[2])) throw new Error('Unsupported git tree entry')
      paths.add(`${metadata[0]} ${path} ${metadata[2]}`)
    }
  }
  const entries = [...paths].sort()
  return { digest: createHash('sha256').update(entries.map(entry => `${entry}\0`).join('')).digest('hex'), entries }
}
/** Identity of the actual native compiler/linker environment, before admission. */
export function nativeBuildIdentity(env = process.env, os = platform(), probe = (command: string, args: string[]) => execFileSync(command, args, { encoding: 'utf8', timeout: 15000 }).trim()) {
  const read = (command: string, args: string[]) => { const value = probe(command, args).trim(); if (!value) throw new Error('Native toolchain identity is unavailable'); return value }
  const keys = ['RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'RUSTC', 'RUSTC_WRAPPER', 'RUSTC_WORKSPACE_WRAPPER', 'RUSTUP_TOOLCHAIN', 'CARGO_BUILD_TARGET', 'RUSTDOC', 'RUSTDOCFLAGS', 'CARGO_ENCODED_RUSTDOCFLAGS', 'RUSTC_BOOTSTRAP', 'SOURCE_DATE_EPOCH', 'CC', 'CXX', 'AR', 'CFLAGS', 'CXXFLAGS', 'CPPFLAGS', 'LDFLAGS', 'HOST_CC', 'TARGET_CC', 'HOST_CFLAGS', 'TARGET_CFLAGS', 'SDKROOT', 'DEVELOPER_DIR', 'MACOSX_DEPLOYMENT_TARGET', ...Object.keys(env).filter(key => /^(CARGO_TARGET_|CARGO_PROFILE_|CARGO_BUILD_|CC_|CXX_|AR_|CFLAGS_|CXXFLAGS_|CPPFLAGS_|LDFLAGS_)/.test(key))]
  const variables = [...new Set(keys)].sort().map(key => [key, env[key] ?? ''])
  // Unknown wrappers can change bytes without changing the compiler identity.
  if (env.RUSTC_WRAPPER || env.RUSTC_WORKSPACE_WRAPPER) throw new Error('Native reuse does not admit compiler wrappers')
  return { rustc: read(env.RUSTC || 'rustc', ['-vV']), cargo: read('cargo', ['--version']), cc: read(env.CC || 'cc', ['--version']),
    ...(os === 'darwin' ? { sdk: read('xcrun', ['--show-sdk-version']), clang: read('xcrun', ['clang', '--version']) } : {}), variables }
}
/** The signed installed package graph qualifies the Linux media closure;
 * Mac additionally qualifies its Swift/AVFoundation toolchain and fixture tools. */
export function mediaBuildIdentity(env = process.env, os = platform(), probe = (command: string, args: string[]) => execFileSync(command, args, { encoding: 'utf8', timeout: 15000 }).trim()) {
  const read = (command: string, args: string[]) => { const value = probe(command, args).trim(); if (!value) throw new Error('Media runtime identity is unavailable'); return value }
  const variables = [...new Set(['APPARENT_GST_PLUGIN_SOURCE', 'PKG_CONFIG', 'SWIFT_EXEC', 'SWIFTFLAGS', 'TOOLCHAINS', ...Object.keys(env).filter(key => /^(GST_|GSTREAMER_|GLIB_|GOBJECT_|GIO_|SYSTEM_DEPS_|SWIFT_|PKG_CONFIG_|LD_|DYLD_|FFMPEG_)/.test(key))])].sort().map(key => [key, env[key] ?? ''])
  const ffmpeg = read('ffmpeg', ['-version'])
  if (os === 'linux') return { ffmpeg, packages: read('dpkg-query', ['-W', '-f=${binary:Package}=${Version}\n']).split('\n').sort(),
    gstreamer: read('pkg-config', ['--modversion', 'gstreamer-1.0']), linkFlags: read('pkg-config', ['--cflags', '--libs', 'gstreamer-1.0']), glibc: read('ldd', ['--version']), variables }
  if (os === 'darwin') return { ffmpeg, swift: read('xcrun', ['swiftc', '--version']), osRelease: read('sw_vers', ['-productVersion']), variables }
  throw new Error('Media reuse requires Linux or macOS')
}
export function supportsPrCandidate(id: string, target: string): boolean {
  return id === 'engine' ? target === 'linux-x64' : ['engine-native', 'app-shell', 'engine-media-native', 'platform-config', 'onboarding'].includes(id) && ['linux-x64', 'macos-x64'].includes(target)
}
export function packageBuildEnvironment(env = process.env, electron = false) {
  return [...new Set(['NODE_OPTIONS', 'NODE_ENV', 'NODE_PATH', 'ESBUILD_BINARY_PATH', 'ESBUILD_WORKER_THREADS', ...Object.keys(env).filter(key => /^(npm_config_|NPM_CONFIG_)/.test(key) || (electron && key.startsWith('ELECTRON_')))])].sort().map(key => [key, env[key] ?? ''])
}
function fingerprint(root: string, owner: Owner, target: Target, revision = 'HEAD') {
  if (target !== 'linux-x64' && target !== 'macos-x64' && target !== 'web' || !owner.targets.includes(target)) throw new Error('Owner is not registered for this target')
  const source = trackedInputs(root, owner, revision).digest
  const imageOS = process.env.ImageOS ?? ''
  const imageVersion = process.env.ImageVersion ?? ''
  if (!imageOS || !imageVersion) throw new Error('Runner image identity is unavailable')
  return createHash('sha256').update(JSON.stringify({ source, environment: environmentDigest(owner), node: process.version, imageOS, imageVersion, platform: platform(), arch: arch(), target, ...(supportsPrCandidate(owner.id, target) && owner.language === 'native' ? { native: nativeBuildIdentity() } : {}), ...(['app-shell', 'onboarding'].includes(owner.id) ? { packaging: packageBuildEnvironment(process.env, owner.id === 'app-shell') } : {}), ...(owner.id === 'engine-media-native' ? { media: mediaBuildIdentity() } : {}) })).digest('hex')
}
export function sourceDigest(root: string, owner: Owner, revision = 'HEAD'): string { return trackedInputs(root, owner, revision).digest }
/** Compare committed inputs, never a checkout or an artifact-supplied digest alone. */
export function prInputsMatch(root: string, owner: Owner, testedSha: string, testedBase: string, landedBase: string): boolean {
  try {
    if (![testedSha, testedBase, landedBase].every(sha => /^[a-f0-9]{40}$/.test(sha))) return false
    if (git(root, ['rev-parse', `${testedSha}^1`]).trim() !== testedBase) return false
    git(root, ['merge-base', '--is-ancestor', testedBase, landedBase])
    return sourceDigest(root, owner, testedSha) === sourceDigest(root, owner)
  } catch { return false }
}
export function environmentDigest(owner: Owner, env = process.env): string { return createHash('sha256').update(JSON.stringify([...(owner.environment ?? [])].sort().map(key => [key, env[key] ?? '']))).digest('hex') }
export function inputDigest(root: string, owner: Owner, target: Target = owner.targets[0]): string { return fingerprint(root, owner, target) }
export function makePlan(root: string) {
  const { owners, pending, blocked } = catalog(root)
  const guardOwners: Record<string, string> = { 'app-shell': 'composition', engine: 'engine', 'engine-native': 'engine', 'engine-capture-native': 'engine', 'engine-media-native': 'engine', 'platform-config': 'composition', 'platform-resources': 'shared', record: 'record', 'connect-client': 'connect', 'connect-browser': 'connect', agent: 'agent', onboarding: 'onboarding' }
  const include = owners.flatMap(owner => owner.targets.map(target => ({ id: owner.id, target, runner: target === 'linux-x64' || target === 'web' ? 'ubuntu-24.04' : 'macos-26-intel', kind: owner.kind, stage: owner.stage, package: owner.package, verify: owner.verify, build: owner.build, language: owner.language, packageManager: owner.packageManager ?? 'npm', architecture_owner: guardOwners[owner.id] ?? '' })))
  return { matrix: { include }, has_ready: include.length > 0, pending, blocked }
}

function runNpm(root: string, cwd: string, args: string[]) {
  execFileSync('npm', args, { cwd: resolve(root, cwd), stdio: 'inherit', env: { ...process.env, CI: 'true' } })
}
function outsideCheckout(root: string, path: string) {
  const checkout = realpathSync(root)
  const absolute = resolve(path)
  let ancestor = absolute
  const missing: string[] = []
  while (!existsSync(ancestor)) { missing.unshift(ancestor.slice(dirname(ancestor).length + 1)); ancestor = dirname(ancestor) }
  const actual = resolve(realpathSync(ancestor), ...missing)
  const rel = relative(checkout, actual)
  if (!rel || (!rel.startsWith(`..${sep}`) && rel !== '..' && !isAbsolute(rel))) throw new Error('Build output must be outside the checkout')
  return absolute
}
export function buildOwner(root: string, owner: Owner, target: Target, outputPath: string) {
  if (!owner.targets.includes(target)) throw new Error('Owner is not registered for this target')
  const output = outsideCheckout(root, outputPath)
  if (existsSync(output) && readdirSync(output).length) throw new Error('Build output directory must be empty')
  mkdirSync(output, { recursive: true })
  if (owner.kind === 'app-shell') {
    runNpm(root, 'shared/contracts', ['ci'])
    runNpm(root, 'composition/app', ['ci'])
    for (const script of owner.verify) runNpm(root, owner.package, ['run', script])
    runNpm(root, owner.package, ['run', owner.build])
    const app = resolve(root, owner.package, 'dist')
    for (const child of ['app', 'shell', 'artifacts']) cpSync(resolve(app, child), resolve(output, child), { recursive: true, dereference: false, preserveTimestamps: true, verbatimSymlinks: true })
    return
  }
  if (owner.packageManager === 'bun') execFileSync('bun', ['install', '--frozen-lockfile'], { cwd: resolve(root, owner.package), stdio: 'inherit', env: { ...process.env, CI: 'true' } })
  else runNpm(root, owner.package, ['ci'])
  for (const script of owner.verify) runNpm(root, owner.package, ['run', script])
  runNpm(root, owner.package, ['run', owner.build])
  if (owner.kind === 'directory') {
    const source = resolve(root, owner.package, owner.artifactPath!)
    const packageRoot = realpathSync(resolve(root, owner.package))
    assertNoSymlinkComponents(packageRoot, owner.artifactPath!)
    const actual = realpathSync(source)
    const rel = relative(packageRoot, actual)
    if (!rel || rel === '..' || rel.startsWith(`..${sep}`) || isAbsolute(rel) || !statSync(actual).isDirectory()) throw new Error('Registered directory artifact escapes package root or is not a directory')
    cpSync(actual, resolve(output, 'payload'), { recursive: true, dereference: false, verbatimSymlinks: true })
    return
  }
  const manifest = JSON.parse(readFileSync(resolve(root, owner.package, 'package.json'), 'utf8')) as { name?: string; exports?: Record<string, unknown>; peerDependencies?: Record<string, unknown>; peerDependenciesMeta?: Record<string, { optional?: boolean }>; devDependencies?: Record<string, unknown> }
  if (!manifest.name || !/^@?[a-z0-9][a-z0-9._/-]*$/.test(manifest.name)) throw new Error('Invalid npm package name')
  const packed = execFileSync('npm', ['pack', '--ignore-scripts', '--json', '--pack-destination', output], { cwd: resolve(root, owner.package), encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'], env: { ...process.env, CI: 'true' } })
  const result = JSON.parse(packed) as Array<{ filename?: string }>
  if (result.length !== 1 || !result[0].filename) throw new Error('npm pack did not produce exactly one package')
  const tarball = resolve(output, result[0].filename)
  const consumer = resolve(output, '.consumer')
  mkdirSync(consumer)
  try {
    // This consumer exercises every export, including optional UI entrypoints.
    // Install their declared host peers here, never in the staged library artifact.
    execFileSync('npm', ['install', '--ignore-scripts', '--no-audit', '--no-fund', '--prefix', consumer, tarball, ...optionalVerificationPeers(manifest, resolve(root, owner.package))], { cwd: consumer, stdio: 'inherit', env: { ...process.env, CI: 'true' } })
    const publicPaths = Object.keys(manifest.exports ?? {}).filter(key => key === '.' || key.startsWith('./'))
    if (publicPaths.some(key => key.includes('*'))) throw new Error('Wildcard exports require an explicit consumer verification contract')
    verifyPublicExports(consumer, manifest.name, publicPaths.length ? publicPaths : ['.'])
  } finally { rmSync(consumer, { recursive: true, force: true }) }
}

/** Host peers required by the all-exports verifier, not by headless consumers. */
export function optionalVerificationPeers(manifest: { peerDependencies?: Record<string, unknown>; peerDependenciesMeta?: Record<string, { optional?: boolean }>; devDependencies?: Record<string, unknown> }, packageRoot?: string): string[] {
  return Object.entries(manifest.peerDependencies ?? {}).sort(([a], [b]) => a.localeCompare(b)).flatMap(([name, version]) => {
    if (manifest.peerDependenciesMeta?.[name]?.optional !== true) return [];
    if (!/^(?:@[a-z0-9][a-z0-9._-]*\/)?[a-z0-9][a-z0-9._-]*$/.test(name) || typeof version !== 'string' || !version.trim() || version.includes('\0'))
      throw new Error('Invalid optional verification peer');
    const supplied = manifest.devDependencies?.[name];
    if (typeof supplied === 'string' && supplied.startsWith('file:')) {
      if (!packageRoot) throw new Error('Local verification peer requires its package root');
      const path = resolve(packageRoot, supplied.slice(5));
      const rel = relative(realpathSync(packageRoot), realpathSync(path));
      if (!rel || rel === '..' || rel.startsWith(`..${sep}`) || isAbsolute(rel) || !path.endsWith('.tgz') || !lstatSync(path).isFile())
        throw new Error('Local verification peer must be a contained package tarball');
      return [path];
    }
    return [`${name}@${version}`];
  })
}

/** Exercise packed JS exports and inspect declared stylesheet assets as files. */
export function verifyPublicExports(consumer: string, packageName: string, publicPaths: string[]) {
  const imports = publicPaths.map(key => key === '.' ? packageName : packageName + key.slice(1))
  execFileSync('node', ['--input-type=module', '-e', `
    import { realpathSync, statSync, readFileSync } from 'node:fs';
    import { fileURLToPath } from 'node:url';
    import { resolve, relative, isAbsolute, sep } from 'node:path';
    const packageRoot = realpathSync(resolve('node_modules', process.argv[2]));
    for (const name of JSON.parse(process.argv[1])) {
      const resolved = import.meta.resolve(name);
      if (new URL(resolved).pathname.endsWith('.css')) {
        const file = realpathSync(fileURLToPath(resolved));
        const rel = relative(packageRoot, file);
        if (!rel || rel === '..' || rel.startsWith('..' + sep) || isAbsolute(rel) || !statSync(file).isFile() || !readFileSync(file).length)
          throw new Error('Stylesheet export is missing, empty or outside the packed package');
      } else { await import(name); }
    }
  `, JSON.stringify(imports), packageName], { cwd: consumer, stdio: 'inherit' })
}

type Run = { id: number; run_attempt: number; head_sha: string; head_branch: string; event: string; conclusion: string | null; path?: string }
type Job = { name: string; status: string; conclusion: string | null }
type Artifact = { id: number; name: string; expired: boolean }
export type AdmissionResult = { reuse: boolean; reason: string; input_digest: string; prior_artifact_id: number | null; prior_source_sha: string | null; prior_run_id: number | null; prior_run_attempt: number | null }
export function assessPrior(root: string, owner: Owner, target: Target, currentSha: string, repository: string, runs: Run[], jobsFor: (run: Run) => Job[], artifactsFor: (run: Run) => Artifact[], exactSourceSha?: string, exactInputDigest?: string): AdmissionResult {
  if (exactInputDigest && (!exactSourceSha || exactSourceSha !== currentSha)) throw new Error('Exact artifact digest requires the same explicit source commit')
  const digest = exactInputDigest ?? (exactSourceSha ? fingerprint(root, owner, target, exactSourceSha) : inputDigest(root, owner, target))
  if (!/^[0-9a-f]{64}$/.test(digest)) throw new Error('Invalid exact artifact input digest')
  const rebuild = (reason: string): AdmissionResult => ({ reuse: false, reason, input_digest: digest, prior_artifact_id: null, prior_source_sha: null, prior_run_id: null, prior_run_attempt: null })
  if (!owner.targets.includes(target)) return rebuild('owner is not registered for this target')
  if (repository !== 'tryapparent/apparent') return rebuild('repository identity is not canonical')
  for (const run of [...runs].sort((a, b) => b.id - a.id)) {
    if (!Number.isSafeInteger(run.id) || run.id <= 0 || !Number.isSafeInteger(run.run_attempt) || run.run_attempt <= 0 || !/^[0-9a-f]{40,64}$/.test(run.head_sha) || run.event !== 'push' || run.head_branch !== 'main' || run.path !== workflowPathFor(owner.id) || (exactSourceSha ? run.head_sha !== exactSourceSha : run.head_sha === currentSha)) continue
    try { execFileSync('git', ['merge-base', '--is-ancestor', run.head_sha, exactSourceSha ?? currentSha], { cwd: root, stdio: 'ignore' }) } catch { continue }
    if (!exactInputDigest && fingerprint(root, owner, target, run.head_sha) !== digest) continue
    const jobName = `deliver / stream (${owner.id}, ${target})`
    const jobs = jobsFor(run)
    if (jobs.filter(j => j.name === jobName && j.status === 'completed' && j.conclusion === 'success').length !== 1 ||
        jobs.filter(j => j.name === `deliver / ${contractsJob}` && j.status === 'completed' && j.conclusion === 'success').length !== 1 ||
        jobs.filter(j => j.name === `deliver / ${auditJob}` && j.status === 'completed' && j.conclusion === 'success').length !== 1) continue
    const wanted = `stream-${owner.id}-${target}-${digest}`
    const artifact = artifactsFor(run).find(a => Number.isSafeInteger(a.id) && a.id > 0 && a.name === wanted && !a.expired)
    if (artifact) return { reuse: true, reason: 'same workflow owner, successful audit, ancestor source and matching unexpired artifact', input_digest: digest, prior_artifact_id: artifact.id, prior_source_sha: run.head_sha, prior_run_id: run.id, prior_run_attempt: run.run_attempt }
  }
  return rebuild('no qualifying prior artifact')
}

export function sealPayload(root: string, owner: Owner, target: Target, inputDir: string, outputDir: string, inputDigestValue: string, env = process.env, candidate = false) {
  if (!/^[0-9a-f]{64}$/.test(inputDigestValue) || (target !== 'linux-x64' && target !== 'macos-x64' && target !== 'web') || !owner.targets.includes(target)) throw new Error('Invalid input digest or target')
  const source = realpathSync(inputDir)
  if (!statSync(source).isDirectory()) throw new Error('Payload input is not a directory')
  const check = (dir: string) => { for (const name of readdirSync(dir)) { const p = resolve(dir, name); const stat = lstatSync(p); if (stat.isSymbolicLink()) { const link = readlinkSync(p); if (isAbsolute(link) || windows(link)) throw new Error('Unsafe payload symlink'); const resolved = resolve(dirname(p), link); const rel = relative(source, resolved); if (rel === '..' || rel.startsWith(`..${sep}`) || isAbsolute(rel)) throw new Error('Payload symlink escapes input directory'); let actual: string; try { actual = realpathSync(p) } catch { throw new Error('Broken payload symlink') } const actualRel = relative(source, actual); if (actualRel === '..' || actualRel.startsWith(`..${sep}`) || isAbsolute(actualRel)) throw new Error('Payload symlink escapes input directory'); } else if (stat.isDirectory()) check(p); else if (!stat.isFile()) throw new Error('Unsupported payload filesystem entry') } }
  check(source)
  const prIdentity = candidate && supportsPrCandidate(owner.id, target) && env.GITHUB_EVENT_NAME === 'pull_request' && /^refs\/pull\/[1-9][0-9]*\/merge$/.test(env.GITHUB_REF ?? '') && env.GITHUB_WORKFLOW_REF === `tryapparent/apparent/${workflowPathFor(owner.id)}@${env.GITHUB_REF}`
  if ((!prIdentity && (candidate || env.GITHUB_REF !== 'refs/heads/main' || env.GITHUB_WORKFLOW_REF !== `tryapparent/apparent/${workflowPathFor(owner.id)}@refs/heads/main`)) || env.GITHUB_REPOSITORY !== 'tryapparent/apparent' || !/^[1-9][0-9]*$/.test(env.GITHUB_RUN_ID ?? '') || !/^[1-9][0-9]*$/.test(env.GITHUB_RUN_ATTEMPT ?? '')) throw new Error('Sealing requires canonical main workflow identity')
  const output = outsideCheckout(root, outputDir)
  if (output === source || output.startsWith(`${source}${sep}`) || source.startsWith(`${output}${sep}`)) throw new Error('Output directory overlaps payload input')
  const sha = git(root, ['rev-parse', 'HEAD']).trim()
  mkdirSync(output, { recursive: true })
  requireNoSymlink(output)
  if (readdirSync(output).length) throw new Error('Seal output directory must be empty')
  execFileSync('tar', [...(owner.stage === 'website' ? ['--format=ustar'] : []), '-czf', resolve(output, 'component.tar.gz'), '-C', dirname(source), source.slice(dirname(source).length + 1)], { stdio: 'inherit' })
  const archive = readFileSync(resolve(output, 'component.tar.gz'))
  const receipt = { schema: 1, id: owner.id, target, stage: owner.stage, sourceSha: sha, ...(supportsPrCandidate(owner.id, target) ? { buildSourceSha: sha } : {}), inputDigest: inputDigestValue, sourceDigest: sourceDigest(root, owner), environmentDigest: environmentDigest(owner, env), sha256: createHash('sha256').update(archive).digest('hex'), size: archive.length, runId: env.GITHUB_RUN_ID, runAttempt: env.GITHUB_RUN_ATTEMPT }
  const receiptPath = resolve(output, 'receipt.json')
  requireNoSymlink(output)
  writeFileSync(receiptPath, JSON.stringify(receipt, null, 2) + '\n', { mode: 0o600, flag: 'wx' })
  return receipt
}
function requireNoSymlink(path: string) { if (lstatSync(path).isSymbolicLink()) throw new Error('Output directory cannot be a symlink') }

/** Reissue an admitted, byte-verified archive; never extract its application payload. */
export function reusePayload(root: string, owner: Owner, target: Target, zip: string, outputDir: string, digest: string, priorSha: string, artifactId: string, priorRunId: string, priorAttempt: string, env = process.env) {
  if (!owner.targets.includes(target) || !/^[0-9a-f]{64}$/.test(digest) || !/^[0-9a-f]{40,64}$/.test(priorSha) || !/^[1-9][0-9]*$/.test(artifactId)) throw new Error('Invalid reuse identity')
  if (env.GITHUB_REF !== 'refs/heads/main' || env.GITHUB_REPOSITORY !== 'tryapparent/apparent' || env.GITHUB_WORKFLOW_REF !== `tryapparent/apparent/${workflowPathFor(owner.id)}@refs/heads/main` || !/^[1-9][0-9]*$/.test(env.GITHUB_RUN_ID ?? '') || !/^[1-9][0-9]*$/.test(env.GITHUB_RUN_ATTEMPT ?? '')) throw new Error('Reuse requires canonical main workflow identity')
  const entries = execFileSync('unzip', ['-Z1', zip], { encoding: 'utf8', maxBuffer: 4096 }).trim().split('\n').sort()
  if (JSON.stringify(entries) !== JSON.stringify(['component.tar.gz', 'receipt.json'])) throw new Error('Unexpected artifact ZIP inventory')
  const original = JSON.parse(execFileSync('unzip', ['-p', zip, 'receipt.json'], { encoding: 'utf8', maxBuffer: 16384 }))
  if (!/^[1-9][0-9]*$/.test(priorRunId) || !/^[1-9][0-9]*$/.test(priorAttempt) || original.runId !== priorRunId || original.runAttempt !== priorAttempt) throw new Error('Prior receipt run does not match admission')
  if (original.schema !== 1 || original.id !== owner.id || original.target !== target || original.stage !== owner.stage || original.inputDigest !== digest || original.sourceSha !== priorSha || !/^[0-9a-f]{64}$/.test(original.sha256) || !Number.isSafeInteger(original.size) || original.size <= 0) throw new Error('Prior receipt does not match admission')
  if (supportsPrCandidate(owner.id, target) && original.sourceDigest !== sourceDigest(root, owner)) throw new Error('Component source changed since admission')
  if (owner.stage === 'website' && (original.sourceDigest !== sourceDigest(root, owner) || original.environmentDigest !== environmentDigest(owner, env))) throw new Error('Website source or environment changed since admission')
  const output = outsideCheckout(root, outputDir)
  mkdirSync(output, { recursive: true }); requireNoSymlink(output)
  if (readdirSync(output).length) throw new Error('Reuse output directory must be empty')
  const archivePath = resolve(output, 'component.tar.gz')
  const fd = openSync(archivePath, 'wx', 0o600)
  try { execFileSync('unzip', ['-p', zip, 'component.tar.gz'], { stdio: ['ignore', fd, 'pipe'] }) }
  finally { closeSync(fd) }
  const archive = readFileSync(archivePath)
  if (archive.length !== original.size || createHash('sha256').update(archive).digest('hex') !== original.sha256) throw new Error('Prior archive hash or size mismatch')
  const receipt = { ...original, ...(supportsPrCandidate(owner.id, target) ? { buildSourceSha: original.buildSourceSha ?? original.reusedFrom?.sourceSha ?? original.sourceSha } : {}), sourceSha: git(root, ['rev-parse', 'HEAD']).trim(), runId: env.GITHUB_RUN_ID, runAttempt: env.GITHUB_RUN_ATTEMPT,
    reusedFrom: { artifactId, sourceSha: priorSha, runId: original.runId, runAttempt: original.runAttempt } }
  writeFileSync(resolve(output, 'receipt.json'), JSON.stringify(receipt, null, 2) + '\n', { flag: 'wx', mode: 0o600 })
  return receipt
}
