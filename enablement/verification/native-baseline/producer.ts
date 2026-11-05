/** Seal evidence from a completed full native verification. This is evidence, not a verdict. */
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { lstatSync, readFileSync, readdirSync, writeFileSync } from 'node:fs'
import { arch, platform, release } from 'node:os'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { externalDirectory } from '../receipt.ts'

const self = fileURLToPath(import.meta.url)
const root = resolve(fileURLToPath(new URL('../../..', import.meta.url)))
const sha = (b: Buffer | string) => createHash('sha256').update(b).digest('hex')
function git(...args: string[]) {
  const r = spawnSync('git', args, { cwd: root, encoding: 'utf8', env: { ...process.env, GIT_OPTIONAL_LOCKS: '0' } })
  assert.equal(r.status, 0, 'Cannot establish clean native baseline source identity')
  return r.stdout.trim()
}
function command(program: string, args: string[]) {
  const r = spawnSync(program, args, { encoding: 'utf8', timeout: 10_000 })
  assert.equal(r.status, 0, `${program} probe failed`)
  return r.stdout.trim()
}
export function inventory(base: string, paths: string[]) {
  return paths.sort().map(path => {
    const stat = lstatSync(join(base, path))
    assert(stat.isFile() && !stat.isSymbolicLink(), `Unexpected native baseline input: ${path}`)
    return { path, mode: stat.mode & 0o777, sha256: sha(readFileSync(join(base, path))) }
  })
}
function walk(base: string): string[] {
  return readdirSync(base, { withFileTypes: true }).flatMap(entry => {
    const path = join(base, entry.name)
    assert(!entry.isSymbolicLink(), `Symlink in compiler evidence: ${path}`)
    return entry.isDirectory() ? walk(path) : entry.isFile() ? [path] : []
  })
}
export function inputPaths(paths: string[]) {
  // These are independently built and tested owners; their runtime/compiler
  // inputs are checked by their own jobs. Everything else remains in closure.
  return paths.filter(path => !path.startsWith('capture/desktop/ui/frontend/') &&
    !path.startsWith('composition/app/') && !path.startsWith('shared/contracts/')).sort()
}
export function environmentSnapshot(env: NodeJS.ProcessEnv) {
  return Object.fromEntries(['ImageOS', 'ImageVersion', 'RUNNER_OS', 'RUNNER_ARCH', 'MACOSX_DEPLOYMENT_TARGET',
    'RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'CARGO_BUILD_TARGET', 'CARGO_PROFILE_DEV_DEBUG', 'CARGO_PROFILE_TEST_DEBUG']
    .map(k => [k, env[k] ? sha(env[k]!) : null]))
}
export function seal(outputArg: string) {
  const env = process.env
  assert(env.GITHUB_REPOSITORY && /^\d+$/.test(env.GITHUB_RUN_ID ?? '') && /^\d+$/.test(env.GITHUB_RUN_ATTEMPT ?? ''), 'GitHub run identity required')
  assert(env.GITHUB_JOB === 'rust', 'Only the full rust verification job may produce this baseline')
  assert(git('status', '--porcelain', '--untracked-files=normal') === '', 'Baseline source must be clean')
  const head = git('rev-parse', 'HEAD')
  const workflow = git('rev-parse', 'HEAD:.github/workflows/verification-platform.yml')
  const tracked = inputPaths(git('ls-files', '-z').split('\0').filter(Boolean))
  const sourceInputs = inventory(root, tracked)
  const deps = join(root, 'target/debug/deps')
  const rustcPaths = readdirSync(deps).filter(p => p.endsWith('.d')).map(p => `target/debug/deps/${p}`)
  assert(rustcPaths.some(p => /apparent/.test(p)), 'Rust compiler dependency evidence is missing')
  const compilerInputs = inventory(root, rustcPaths)
  const cargoPaths = walk(join(root, 'target/debug/.fingerprint')).map(p => p.slice(root.length + 1))
  const buildOutputs = walk(join(root, 'target/debug/build')).filter(p => p.endsWith('/output')).map(p => p.slice(root.length + 1))
  assert(cargoPaths.some(p => p.endsWith('.json')) && buildOutputs.length > 0,
    'Cargo fingerprint and build-script input evidence is required')
  const cargoEvidence = inventory(root, [...cargoPaths, ...buildOutputs].sort())
  const image = environmentSnapshot(process.env)
  const rustc = command('rustc', ['-vV']), cargo = command('cargo', ['-V'])
  const environment = { image, rustcSha256: sha(rustc), cargoSha256: sha(cargo), rustc, cargo }
  const payload = Buffer.from(JSON.stringify({ schema: 1, sourceSha: head, sourceTree: git('rev-parse', 'HEAD^{tree}'),
    workflowBlob: workflow, trackedInputs: sourceInputs, rustcDepInfo: compilerInputs,
    cargoFingerprintsAndBuildScriptOutputs: cargoEvidence, environment }, null, 2) + '\n')
  const out = externalDirectory(root, outputArg)
  const payloadPath = join(out, 'native-baseline-payload.json')
  writeFileSync(payloadPath, payload, { flag: 'wx', mode: 0o600 })
  const receipt = { schema: 2, kind: 'successful-full-native-verification-baseline', reusable: false,
    completeness: { sourceInputs: true, rustcDepInfo: true, cargoFingerprints: true, buildScriptOutputs: true,
      externalInputs: false, runtimeFixtures: 'all non-independent tracked repository files', eligible: false },
    sourceSha: head, sourceTree: git('rev-parse', 'HEAD^{tree}'), workflowBlob: workflow,
    run: { repository: env.GITHUB_REPOSITORY, id: env.GITHUB_RUN_ID, attempt: env.GITHUB_RUN_ATTEMPT, job: env.GITHUB_JOB,
      url: `https://github.com/${env.GITHUB_REPOSITORY}/actions/runs/${env.GITHUB_RUN_ID}/attempts/${env.GITHUB_RUN_ATTEMPT}` },
    platform: { os: platform(), arch: arch(), release: release(), target: rustc.split('\n').find(x => x.startsWith('host: '))?.slice(6) },
    profile: 'debug', environment,
    checks: ['full-rust-suite', 'root-build', 'electron-rust-roundtrip-if-root-selected', 'platform-native-checks-if-root-selected'],
    payload: { name: 'native-baseline-payload.json', sha256: sha(payload), bytes: payload.length },
    note: 'Not eligible for reuse until consumer proves GitHub job success, attestation identity and complete non-external closure.' }
  writeFileSync(join(out, 'native-baseline.json'), JSON.stringify(receipt, null, 2) + '\n', { flag: 'wx', mode: 0o600 })
}
if (process.argv[1] && resolve(process.argv[1]) === self) {
  assert(process.argv.length === 3, 'Usage: producer.ts EXTERNAL_OUTPUT_DIRECTORY')
  seal(process.argv[2])
}
