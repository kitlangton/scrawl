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
function inventory(paths: string[]) {
  return paths.sort().map(path => {
    const stat = lstatSync(join(root, path))
    assert(stat.isFile() && !stat.isSymbolicLink(), `Unexpected native baseline input: ${path}`)
    return { path, mode: stat.mode & 0o777, sha256: sha(readFileSync(join(root, path))) }
  })
}
export function seal(outputArg: string) {
  const env = process.env
  assert(env.GITHUB_REPOSITORY && /^\d+$/.test(env.GITHUB_RUN_ID ?? '') && /^\d+$/.test(env.GITHUB_RUN_ATTEMPT ?? ''), 'GitHub run identity required')
  assert(env.GITHUB_JOB === 'rust', 'Only the full rust verification job may produce this baseline')
  assert(git('status', '--porcelain', '--untracked-files=normal') === '', 'Baseline source must be clean')
  const head = git('rev-parse', 'HEAD')
  const workflow = git('rev-parse', 'HEAD:.github/workflows/verification-platform.yml')
  const tracked = git('ls-files', '-z').split('\0').filter(p => p &&
    (p.endsWith('.rs') || p.endsWith('.toml') || p.endsWith('.lock') || p.startsWith('capture/desktop/ui/frontend/') ||
      p.startsWith('capture/desktop/scripts/') || p.startsWith('capture/engine/') || p.startsWith('shared/')))
  const paths = readdirSync(join(root, 'target/debug/deps')).filter(p => /^(apparent|apparent_inspect).*\.d$/.test(p))
  assert(paths.length > 0, 'Rust compiler dependency evidence is missing')
  const depInfo = paths.map(path => ({ path, sha256: sha(readFileSync(join(root, 'target/debug/deps', path))) }))
  const payload = Buffer.from(JSON.stringify({ schema: 1, sourceSha: head, sourceTree: git('rev-parse', 'HEAD^{tree}'),
    workflowBlob: workflow, trackedInputs: inventory(tracked), rustcDepInfo: depInfo }, null, 2) + '\n')
  const out = externalDirectory(root, outputArg)
  const payloadPath = join(out, 'native-baseline-payload.json')
  writeFileSync(payloadPath, payload, { flag: 'wx', mode: 0o600 })
  const rustc = command('rustc', ['-vV']), cargo = command('cargo', ['-V'])
  const receipt = { schema: 1, kind: 'successful-full-native-verification-baseline', reusable: false,
    completeness: 'unproven', sourceSha: head, sourceTree: git('rev-parse', 'HEAD^{tree}'), workflowBlob: workflow,
    run: { repository: env.GITHUB_REPOSITORY, id: env.GITHUB_RUN_ID, attempt: env.GITHUB_RUN_ATTEMPT, job: env.GITHUB_JOB,
      url: `https://github.com/${env.GITHUB_REPOSITORY}/actions/runs/${env.GITHUB_RUN_ID}/attempts/${env.GITHUB_RUN_ATTEMPT}` },
    platform: { os: platform(), arch: arch(), release: release(), target: command('rustc', ['-vV']).split('\n').find(x => x.startsWith('host: '))?.slice(6) },
    profile: 'debug', rustcSha256: sha(rustc), cargoSha256: sha(cargo), rustc, cargo,
    environment: Object.fromEntries(['RUSTFLAGS','CARGO_ENCODED_RUSTFLAGS','CARGO_BUILD_TARGET','CARGO_PROFILE_DEV_DEBUG','CARGO_PROFILE_TEST_DEBUG','MACOSX_DEPLOYMENT_TARGET'].map(k => [k, env[k] ? sha(env[k]!) : null])),
    checks: ['full-rust-suite', 'root-build', 'electron-rust-roundtrip-if-root-selected', 'platform-native-checks-if-root-selected'],
    payload: { name: 'native-baseline-payload.json', sha256: sha(payload), bytes: payload.length },
    note: 'Producer step runs only after successful full suite. GitHub job result and workflow attestation must be independently verified by a consumer.' }
  writeFileSync(join(out, 'native-baseline.json'), JSON.stringify(receipt, null, 2) + '\n', { flag: 'wx', mode: 0o600 })
}
if (process.argv[1] && resolve(process.argv[1]) === self) {
  assert(process.argv.length === 3, 'Usage: producer.ts EXTERNAL_OUTPUT_DIRECTORY')
  seal(process.argv[2])
}
