/** Bounded Connect client result reuse. Root/application/native checks still run. */
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { existsSync, lstatSync, mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs'
import { arch, homedir, platform, release } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { externalDirectory } from './receipt.ts'

const source = fileURLToPath(import.meta.url)
const hash = (input: string | Buffer) => createHash('sha256').update(input).digest('hex')
// Reviewed package sources, fixtures, lockfiles and Cargo configuration. Any
// change disables reuse until its closure is reviewed and this signature updated.
export const REVIEWED_PACKAGE = '724ac70896b6e23684d9eca2a255f727d0e8dc4027956f3956afd65342758c36'
export type Identity = { sha: string; key: string; eligible: boolean; verifier: string; closure: string }
export type Proof = {
  schema: 1; eligible: boolean; key: string; verifier: string; outcome: string; checked_sha: string;
  candidate_sha: string; evidence: string | null; shadows: number; reuses: number;
  disabled: boolean; decision: string; duration_seconds: number;
}
function git(root: string, ...args: string[]): string {
  const result = spawnSync('git', args, { cwd: root, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 })
  if (result.status !== 0) throw new Error('Cannot establish client suite source identity')
  return result.stdout
}
export function sourceInputs(root: string) {
  const paths = git(root, 'ls-files', '-z').split('\0').filter(Boolean).sort()
  const all = createHash('sha256'), packageHash = createHash('sha256')
  let regular = true
  for (const path of paths) {
    // The one native config resource is an explicit path dependency. Other
    // owners cannot change this standalone package's source inputs.
    // Root checks still exercise their real-provider integration; service/API,
    // build/configuration, verification and unknown paths remain conservative.
    if (!path.startsWith('shared/config-native/') && ['capture/', 'composition/', 'shared/', 'public/', 'connect/web/',
      'enablement/native/'].some(prefix => path.startsWith(prefix))) continue
    let bytes = 'missing'
    try {
      const stat = lstatSync(join(root, path))
      if (!stat.isFile()) regular = false
      else bytes = `${stat.mode & 0o777}:${hash(readFileSync(join(root, path)))}`
    } catch { regular = false }
    const row = JSON.stringify([path, bytes]) + '\n'
    all.update(row)
    if (path.startsWith('connect/client/') || path.startsWith('shared/config-native/') || path.startsWith('.cargo/') ||
        ['Cargo.toml', 'Cargo.lock', 'rust-toolchain', 'rust-toolchain.toml'].includes(path)) packageHash.update(row)
  }
  return { fingerprint: all.digest('hex'), reviewedPackage: packageHash.digest('hex'), regular,
    clean: git(root, 'status', '--porcelain', '--untracked-files=normal').trim() === '' }
}
function probe(command: string, args: string[]): string | null {
  const result = spawnSync(command, args, { encoding: 'utf8', timeout: 10000 })
  return result.status === 0 ? hash(result.stdout + result.stderr) : null
}
export function identity(root: string): Identity {
  const inputs = sourceInputs(root)
  const env = process.env
  const tools = { rustc: probe('rustc', ['-vV']), cargo: probe('cargo', ['-vV']),
    curl: probe('curl', ['--version']), checksum: platform() === 'linux' ? probe('sha256sum', ['--version']) : 'not-linux',
    cc: probe('cc', ['--version']), libc: platform() === 'linux' ? probe('ldd', ['--version']) : 'not-linux', node: process.version,
    sdk: platform() === 'darwin' ? probe('xcrun', ['--show-sdk-version']) : 'not-macos' }
  const environment = Object.fromEntries(Object.entries(env).filter(([name]) =>
    /^(CARGO|RUST|CC$|CXX$|AR$|CFLAGS|CPPFLAGS|LDFLAGS|MACOSX_|SDKROOT|DEVELOPER_DIR|LANG$|LC_|APPARENT_|TMPDIR$|CURL_|SSL_CERT_)/.test(name)
    || /^(http|https|all|no)_proxy$/i.test(name)).sort())
  const cargoHome = env.CARGO_HOME || join(homedir(), '.cargo')
  const globalConfig = ['config', 'config.toml'].some(name => existsSync(join(cargoHome, name)))
  const verifier = hash(readFileSync(source))
  const key = hash(JSON.stringify({ source: inputs.fingerprint, tools, verifier,
    environment: hash(JSON.stringify(environment)), platform: [platform(), arch(), release(), env.ImageOS, env.ImageVersion] }))
  return { sha: git(root, 'rev-parse', 'HEAD').trim(), key, verifier, closure: inputs.reviewedPackage,
    eligible: inputs.clean && inputs.regular && inputs.reviewedPackage === REVIEWED_PACKAGE && !globalConfig &&
      Boolean(platform() === 'linux' && env.GITHUB_ACTIONS === 'true' && env.ImageOS && env.ImageVersion) &&
      Object.values(tools).every(Boolean) && !Object.keys(env).some(name => name.startsWith('APPARENT_')) }
}
export function validProof(value: unknown): value is Proof {
  const p = value as Proof | null
  return Boolean(p && p.schema === 1 && /^[a-f0-9]{64}$/.test(p.key) && /^[a-f0-9]{64}$/.test(p.verifier) &&
    /^[a-f0-9]{40}$/.test(p.checked_sha) && typeof p.disabled === 'boolean' && typeof p.eligible === 'boolean' &&
    Number.isInteger(p.shadows) && p.shadows >= 0 && p.shadows <= 2 &&
    Number.isInteger(p.reuses) && p.reuses >= 0 && p.reuses <= 4)
}
export function decision(current: Identity, prior: Proof | null, shadow = false): string {
  if (prior && prior.verifier !== current.verifier) prior = null
  if (prior?.disabled) return 'disabled'
  if (!current.eligible) return 'unmodelled-inputs'
  const matches = prior?.eligible === true && prior.outcome === 'success' && prior.key === current.key && prior.verifier === current.verifier
  if (!matches) return 'miss'
  if (shadow || prior.shadows < 2) return 'shadow'
  return prior.reuses >= 4 ? 'audit' : 'reuse'
}
export function completed(current: Identity, prior: Proof | null, selected: string, success: boolean, seconds: number, evidence: string | null): Proof {
  const matching = prior?.eligible === true && prior.outcome === 'success' && prior.key === current.key && prior.verifier === current.verifier
  const reused = selected === 'reuse'
  return { schema: 1, eligible: current.eligible, key: current.key, verifier: current.verifier,
    outcome: success ? 'success' : 'failure', candidate_sha: current.sha,
    checked_sha: reused ? prior!.checked_sha : current.sha, evidence: reused ? prior!.evidence : evidence,
    shadows: success && matching ? Math.min(2, prior!.shadows + (reused ? 0 : 1)) : 0,
    reuses: reused ? prior!.reuses + 1 : 0,
    disabled: Boolean(prior?.disabled || (!success && (selected === 'audit' || selected === 'shadow'))),
    decision: selected, duration_seconds: seconds }
}
function write(path: string, value: unknown) {
  mkdirSync(dirname(path), { recursive: true })
  const temp = `${path}.${process.pid}.tmp`
  writeFileSync(temp, JSON.stringify(value, null, 2) + '\n', { mode: 0o600 })
  renameSync(temp, path)
}
function execute(root: string, offline: boolean): boolean {
  const env = { ...process.env, CARGO_TARGET_DIR: join(root, 'target', 'ci-connect-client') }
  for (const operation of ['build', 'test']) {
    const args = [operation, '--manifest-path', 'connect/client/Cargo.toml', '-j', process.env.T3_BUILD_JOBS || '3', '--locked']
    if (offline) args.push('--offline')
    if (operation === 'test') args.push('--', '--test-threads=2')
    console.log(`Connect client: cargo ${operation}`)
    const result = spawnSync('cargo', args, { cwd: root, env, encoding: 'utf8', maxBuffer: 32 * 1024 * 1024 })
    process.stdout.write(result.stdout || '')
    process.stderr.write(result.stderr || '')
    if (result.status !== 0) return false
    if (operation === 'test' && ![...result.stdout.matchAll(/test result: ok\. (\d+) passed/g)].some(m => Number(m[1]) > 0)) return false
  }
  return true
}
export function run(root: string, statePath: string, offline = false, shadow = false): number {
  statePath = join(externalDirectory(root, dirname(statePath)), statePath.split('/').at(-1)!)
  const current = identity(root)
  let prior: Proof | null = null
  let malformed = false
  try {
    const data = readFileSync(statePath, 'utf8')
    const parsed = JSON.parse(data)
    if (validProof(parsed)) prior = parsed.verifier === current.verifier ? parsed : null
    else malformed = true
  } catch (error) { malformed = (error as NodeJS.ErrnoException).code !== 'ENOENT' }
  const selected = malformed ? 'invalid-proof' : decision(current, prior, shadow)
  const started = performance.now()
  // A cancelled run cannot leave its in-progress state looking like proof.
  write(statePath, { ...completed(current, prior, selected, false, 0, null), outcome: 'in_progress', disabled: Boolean(prior?.disabled || malformed) })
  const success = selected === 'reuse' || execute(root, offline)
  const after = identity(root)
  const unchanged = current.key === after.key && current.sha === after.sha && (!current.eligible || after.eligible)
  const repo = process.env.GITHUB_REPOSITORY || '', id = process.env.GITHUB_RUN_ID || ''
  const attempt = process.env.GITHUB_RUN_ATTEMPT || ''
  const evidence = /^[\w.-]+\/[\w.-]+$/.test(repo) && /^\d+$/.test(id) && /^\d+$/.test(attempt)
    ? `https://github.com/${repo}/actions/runs/${id}/attempts/${attempt}` : null
  const result = completed(current, prior, selected, success && unchanged, (performance.now() - started) / 1000, evidence)
  result.disabled ||= malformed
  write(statePath, result)
  console.log(JSON.stringify({ suite: 'connect-client', decision: selected, outcome: result.outcome,
    checked_sha: result.checked_sha, candidate_sha: result.candidate_sha, shadows: result.shadows,
    reuse_disabled: result.disabled, duration_seconds: result.duration_seconds }))
  return success && unchanged ? 0 : 1
}
if (process.argv[1] && resolve(process.argv[1]) === source) {
  const root = git(process.cwd(), 'rev-parse', '--show-toplevel').trim()
  const args = process.argv.slice(2)
  if (args.length === 1 && args[0] === '--namespace') {
    console.log(`namespace=connect-client-v1-${platform()}-${arch()}-${hash(readFileSync(source))}`)
  } else if (args[0] === '--state' && args[1] && args.slice(2).every(v => ['--offline', '--shadow'].includes(v))) {
    process.exitCode = run(root, resolve(args[1]), args.includes('--offline'), args.includes('--shadow'))
  } else throw new Error('Usage: client-suite.ts --namespace | --state FILE [--offline] [--shadow]')
}
