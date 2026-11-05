import { createHash } from 'node:crypto'
import { execFileSync } from 'node:child_process'
import { closeSync, openSync, cpSync, existsSync, lstatSync, mkdirSync, readlinkSync, readdirSync, readFileSync, realpathSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { arch, platform } from 'node:os'
import { dirname, isAbsolute, relative, resolve, sep } from 'node:path'

export type Target = 'linux-x64' | 'macos-x64'
export type Kind = 'npm' | 'app-shell' | 'directory'
export type Stage = 'library' | 'contract' | 'development-app'
export type Owner = { schema: 1; id: string; kind: Kind; package: string; inputs: string[]; verify: string[]; build: string; targets: Target[]; stage: Stage; language: 'typescript' | 'native'; artifactPath?: string }
export const auditJob = 'Workflow runtime budget'
export const contractsJob = 'Delivery contracts'
export const workflowPath = '.github/workflows/value-stream-delivery.yml'
const roots = [
  { id: 'app-shell', source: 'composition/app', registration: 'composition/app/delivery.json' },
  { id: 'record', source: 'record', registration: 'record/delivery.json' },
  { id: 'engine', source: 'engine/sdk', registration: 'engine/delivery.json' },
  { id: 'onboarding', source: 'onboarding', registration: 'onboarding/delivery.json' },
  { id: 'agent', source: 'agent', registration: 'agent/delivery.json' },
]
const mandatoryInputs = ['composition/delivery/']
const windows = (value: string) => /^[A-Za-z]:[\\/]/.test(value) || value.includes('\\')
function safeRepoPath(value: unknown, directory = false): string {
  if (typeof value !== 'string' || !value || isAbsolute(value) || windows(value) || value.startsWith('/')) throw new Error('Unsafe repository path')
  const normalized = directory ? value.replace(/\/$/, '') : value
  if (!normalized || normalized.split('/').some(p => !p || p === '.' || p === '..')) throw new Error('Unsafe repository path')
  return directory ? `${normalized}/` : normalized
}
function readOwner(path: string): Owner {
  const raw = JSON.parse(readFileSync(path, 'utf8')) as Record<string, unknown>
  if (raw.schema !== 1 || typeof raw.id !== 'string' || !/^[a-z][a-z0-9-]*$/.test(raw.id) || !['npm', 'app-shell', 'directory'].includes(String(raw.kind)) || !['library', 'contract', 'development-app'].includes(String(raw.stage)) || typeof raw.package !== 'string' || typeof raw.build !== 'string' || !Array.isArray(raw.inputs) || !Array.isArray(raw.verify) || !Array.isArray(raw.targets)) throw new Error(`Invalid delivery registration: ${path}`)
  const inputs = raw.inputs.map(x => safeRepoPath(x, true))
  const targets = raw.targets.map(x => { if (x !== 'linux-x64' && x !== 'macos-x64') throw new Error(`Unsupported target in ${path}`); return x })
  if (!inputs.length || new Set(inputs).size !== inputs.length || !targets.length || new Set(targets).size !== targets.length || raw.verify.some(x => typeof x !== 'string' || !/^[a-z][a-z0-9:_-]*$/.test(x))) throw new Error(`Invalid delivery registration: ${path}`)
  const pkg = safeRepoPath(raw.package)
  const language = raw.language ?? 'typescript'
  if (language !== 'typescript' && language !== 'native') throw new Error(`Invalid language in ${path}`)
  const artifactPath = raw.kind === 'directory' ? safeRepoPath(raw.artifactPath) : undefined
  if (raw.kind !== 'directory' && raw.artifactPath !== undefined) throw new Error(`artifactPath is only valid for directory registrations: ${path}`)
  return { schema: 1, id: raw.id, kind: raw.kind as Kind, package: pkg, inputs, verify: raw.verify as string[], build: raw.build, targets, stage: raw.stage as Stage, language, artifactPath }
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
      owners.push(owner)
    } catch (error) { blocked.push(`${entry.id}: ${error instanceof Error ? error.message : 'invalid registration'}`) }
  }
  return { owners, pending, blocked }
}

function trackedInputs(root: string, owner: Owner, target: Target, revision = 'HEAD'): { digest: string; entries: string[] } {
  const registration = roots.find(x => x.id === owner.id)?.registration
  if (!registration) throw new Error('Owner has no registered source identity')
  const inputRoots = [...new Set([...owner.inputs, ...mandatoryInputs])].sort()
  const files = [workflowPath, '.github/actions/runtime-budget/', 'enablement/verification/workflow-budget.ts', 'LICENSE', registration]
  const paths = new Set<string>()
  for (const pathspec of [...inputRoots, ...files]) {
    const result = git(root, ['ls-tree', '-r', '-z', '--full-tree', revision, '--', pathspec])
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
  return { digest: createHash('sha256').update(entries.map(entry => `${entry}\n`).join('')).digest('hex'), entries }
}
function fingerprint(root: string, owner: Owner, target: Target, revision = 'HEAD') {
  if (target !== 'linux-x64' && target !== 'macos-x64' || !owner.targets.includes(target)) throw new Error('Owner is not registered for this target')
  const source = trackedInputs(root, owner, target, revision).digest
  const imageOS = process.env.ImageOS ?? ''
  const imageVersion = process.env.ImageVersion ?? ''
  if (!imageOS || !imageVersion) throw new Error('Runner image identity is unavailable')
  return createHash('sha256').update(JSON.stringify({ source, node: process.version, imageOS, imageVersion, platform: platform(), arch: arch(), target })).digest('hex')
}
export function inputDigest(root: string, owner: Owner, target: Target = owner.targets[0]): string { return fingerprint(root, owner, target) }
export function makePlan(root: string) {
  const { owners, pending, blocked } = catalog(root)
  const include = owners.flatMap(owner => owner.targets.map(target => ({ id: owner.id, target, runner: target === 'linux-x64' ? 'ubuntu-24.04' : 'macos-26-intel', kind: owner.kind, stage: owner.stage, package: owner.package, verify: owner.verify, build: owner.build, language: owner.language })))
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
  runNpm(root, owner.package, ['ci'])
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
  const manifest = JSON.parse(readFileSync(resolve(root, owner.package, 'package.json'), 'utf8')) as { name?: string; exports?: Record<string, unknown> }
  if (!manifest.name || !/^@?[a-z0-9][a-z0-9._/-]*$/.test(manifest.name)) throw new Error('Invalid npm package name')
  const packed = execFileSync('npm', ['pack', '--ignore-scripts', '--json', '--pack-destination', output], { cwd: resolve(root, owner.package), encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'], env: { ...process.env, CI: 'true' } })
  const result = JSON.parse(packed) as Array<{ filename?: string }>
  if (result.length !== 1 || !result[0].filename) throw new Error('npm pack did not produce exactly one package')
  const tarball = resolve(output, result[0].filename)
  const consumer = resolve(output, '.consumer')
  mkdirSync(consumer)
  try {
    execFileSync('npm', ['install', '--ignore-scripts', '--no-audit', '--no-fund', '--prefix', consumer, tarball], { cwd: consumer, stdio: 'inherit', env: { ...process.env, CI: 'true' } })
    const publicPaths = Object.keys(manifest.exports ?? {}).filter(key => key === '.' || key.startsWith('./'))
    if (publicPaths.some(key => key.includes('*'))) throw new Error('Wildcard exports require an explicit consumer verification contract')
    const imports = (publicPaths.length ? publicPaths : ['.']).map(key => key === '.' ? manifest.name! : manifest.name! + key.slice(1))
    execFileSync('node', ['--input-type=module', '-e', imports.map(name => `await import(${JSON.stringify(name)})`).join(';')], { cwd: consumer, stdio: 'inherit' })
  } finally { rmSync(consumer, { recursive: true, force: true }) }
}

type Run = { id: number; run_attempt: number; head_sha: string; head_branch: string; event: string; conclusion: string | null; path?: string }
type Job = { name: string; status: string; conclusion: string | null }
type Artifact = { id: number; name: string; expired: boolean }
export type AdmissionResult = { reuse: boolean; reason: string; input_digest: string; prior_artifact_id: number | null; prior_source_sha: string | null; prior_run_id: number | null; prior_run_attempt: number | null }
export function assessPrior(root: string, owner: Owner, target: Target, currentSha: string, repository: string, runs: Run[], jobsFor: (run: Run) => Job[], artifactsFor: (run: Run) => Artifact[]): AdmissionResult {
  const digest = inputDigest(root, owner, target)
  const rebuild = (reason: string): AdmissionResult => ({ reuse: false, reason, input_digest: digest, prior_artifact_id: null, prior_source_sha: null, prior_run_id: null, prior_run_attempt: null })
  if (!owner.targets.includes(target)) return rebuild('owner is not registered for this target')
  if (repository !== 'tryapparent/apparent') return rebuild('repository identity is not canonical')
  for (const run of [...runs].sort((a, b) => b.id - a.id)) {
    if (!Number.isSafeInteger(run.id) || run.id <= 0 || !Number.isSafeInteger(run.run_attempt) || run.run_attempt <= 0 || !/^[0-9a-f]{40,64}$/.test(run.head_sha) || run.event !== 'push' || run.head_branch !== 'main' || run.path !== workflowPath || run.head_sha === currentSha) continue
    try { execFileSync('git', ['merge-base', '--is-ancestor', run.head_sha, currentSha], { cwd: root, stdio: 'ignore' }) } catch { continue }
    if (fingerprint(root, owner, target, run.head_sha) !== digest) continue
    const jobName = `stream (${owner.id}, ${target})`
    const jobs = jobsFor(run)
    if (jobs.filter(j => j.name === jobName && j.status === 'completed' && j.conclusion === 'success').length !== 1 ||
        jobs.filter(j => j.name === contractsJob && j.status === 'completed' && j.conclusion === 'success').length !== 1 ||
        jobs.filter(j => j.name === auditJob && j.status === 'completed' && j.conclusion === 'success').length !== 1) continue
    const wanted = `stream-${owner.id}-${target}-${digest}`
    const artifact = artifactsFor(run).find(a => Number.isSafeInteger(a.id) && a.id > 0 && a.name === wanted && !a.expired)
    if (artifact) return { reuse: true, reason: 'same workflow owner, successful audit, ancestor source and matching unexpired artifact', input_digest: digest, prior_artifact_id: artifact.id, prior_source_sha: run.head_sha, prior_run_id: run.id, prior_run_attempt: run.run_attempt }
  }
  return rebuild('no qualifying prior artifact')
}

export function sealPayload(root: string, owner: Owner, target: Target, inputDir: string, outputDir: string, inputDigestValue: string, env = process.env) {
  if (!/^[0-9a-f]{64}$/.test(inputDigestValue) || (target !== 'linux-x64' && target !== 'macos-x64') || !owner.targets.includes(target)) throw new Error('Invalid input digest or target')
  const source = realpathSync(inputDir)
  if (!statSync(source).isDirectory()) throw new Error('Payload input is not a directory')
  const check = (dir: string) => { for (const name of readdirSync(dir)) { const p = resolve(dir, name); const stat = lstatSync(p); if (stat.isSymbolicLink()) { const link = readlinkSync(p); if (isAbsolute(link) || windows(link)) throw new Error('Unsafe payload symlink'); const resolved = resolve(dirname(p), link); const rel = relative(source, resolved); if (rel === '..' || rel.startsWith(`..${sep}`) || isAbsolute(rel)) throw new Error('Payload symlink escapes input directory'); let actual: string; try { actual = realpathSync(p) } catch { throw new Error('Broken payload symlink') } const actualRel = relative(source, actual); if (actualRel === '..' || actualRel.startsWith(`..${sep}`) || isAbsolute(actualRel)) throw new Error('Payload symlink escapes input directory'); } else if (stat.isDirectory()) check(p); else if (!stat.isFile()) throw new Error('Unsupported payload filesystem entry') } }
  check(source)
  if (env.GITHUB_REF !== 'refs/heads/main' || env.GITHUB_REPOSITORY !== 'tryapparent/apparent' || env.GITHUB_WORKFLOW_REF !== `tryapparent/apparent/${workflowPath}@refs/heads/main` || !/^[1-9][0-9]*$/.test(env.GITHUB_RUN_ID ?? '') || !/^[1-9][0-9]*$/.test(env.GITHUB_RUN_ATTEMPT ?? '')) throw new Error('Sealing requires canonical main workflow identity')
  const output = outsideCheckout(root, outputDir)
  if (output === source || output.startsWith(`${source}${sep}`) || source.startsWith(`${output}${sep}`)) throw new Error('Output directory overlaps payload input')
  const sha = git(root, ['rev-parse', 'HEAD']).trim()
  mkdirSync(output, { recursive: true })
  requireNoSymlink(output)
  if (readdirSync(output).length) throw new Error('Seal output directory must be empty')
  execFileSync('tar', ['-czf', resolve(output, 'component.tar.gz'), '-C', dirname(source), source.slice(dirname(source).length + 1)], { stdio: 'inherit' })
  const archive = readFileSync(resolve(output, 'component.tar.gz'))
  const receipt = { schema: 1, id: owner.id, target, stage: owner.stage, sourceSha: sha, inputDigest: inputDigestValue, sha256: createHash('sha256').update(archive).digest('hex'), size: archive.length, runId: env.GITHUB_RUN_ID, runAttempt: env.GITHUB_RUN_ATTEMPT }
  const receiptPath = resolve(output, 'receipt.json')
  requireNoSymlink(output)
  writeFileSync(receiptPath, JSON.stringify(receipt, null, 2) + '\n', { mode: 0o600, flag: 'wx' })
  return receipt
}
function requireNoSymlink(path: string) { if (lstatSync(path).isSymbolicLink()) throw new Error('Output directory cannot be a symlink') }

/** Reissue an admitted, byte-verified archive; never extract its application payload. */
export function reusePayload(root: string, owner: Owner, target: Target, zip: string, outputDir: string, digest: string, priorSha: string, artifactId: string, priorRunId: string, priorAttempt: string, env = process.env) {
  if (!owner.targets.includes(target) || !/^[0-9a-f]{64}$/.test(digest) || !/^[0-9a-f]{40,64}$/.test(priorSha) || !/^[1-9][0-9]*$/.test(artifactId)) throw new Error('Invalid reuse identity')
  if (env.GITHUB_REF !== 'refs/heads/main' || env.GITHUB_REPOSITORY !== 'tryapparent/apparent' || env.GITHUB_WORKFLOW_REF !== `tryapparent/apparent/${workflowPath}@refs/heads/main` || !/^[1-9][0-9]*$/.test(env.GITHUB_RUN_ID ?? '') || !/^[1-9][0-9]*$/.test(env.GITHUB_RUN_ATTEMPT ?? '')) throw new Error('Reuse requires canonical main workflow identity')
  const entries = execFileSync('unzip', ['-Z1', zip], { encoding: 'utf8', maxBuffer: 4096 }).trim().split('\n').sort()
  if (JSON.stringify(entries) !== JSON.stringify(['component.tar.gz', 'receipt.json'])) throw new Error('Unexpected artifact ZIP inventory')
  const original = JSON.parse(execFileSync('unzip', ['-p', zip, 'receipt.json'], { encoding: 'utf8', maxBuffer: 16384 }))
  if (!/^[1-9][0-9]*$/.test(priorRunId) || !/^[1-9][0-9]*$/.test(priorAttempt) || original.runId !== priorRunId || original.runAttempt !== priorAttempt) throw new Error('Prior receipt run does not match admission')
  if (original.schema !== 1 || original.id !== owner.id || original.target !== target || original.stage !== owner.stage || original.inputDigest !== digest || original.sourceSha !== priorSha || !/^[0-9a-f]{64}$/.test(original.sha256) || !Number.isSafeInteger(original.size) || original.size <= 0) throw new Error('Prior receipt does not match admission')
  const output = outsideCheckout(root, outputDir)
  mkdirSync(output, { recursive: true }); requireNoSymlink(output)
  if (readdirSync(output).length) throw new Error('Reuse output directory must be empty')
  const archivePath = resolve(output, 'component.tar.gz')
  const fd = openSync(archivePath, 'wx', 0o600)
  try { execFileSync('unzip', ['-p', zip, 'component.tar.gz'], { stdio: ['ignore', fd, 'pipe'] }) }
  finally { closeSync(fd) }
  const archive = readFileSync(archivePath)
  if (archive.length !== original.size || createHash('sha256').update(archive).digest('hex') !== original.sha256) throw new Error('Prior archive hash or size mismatch')
  const receipt = { ...original, sourceSha: git(root, ['rev-parse', 'HEAD']).trim(), runId: env.GITHUB_RUN_ID, runAttempt: env.GITHUB_RUN_ATTEMPT,
    reusedFrom: { artifactId, sourceSha: priorSha, runId: original.runId, runAttempt: original.runAttempt } }
  writeFileSync(resolve(output, 'receipt.json'), JSON.stringify(receipt, null, 2) + '\n', { flag: 'wx', mode: 0o600 })
  return receipt
}
