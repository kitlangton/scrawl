import { createHash } from 'node:crypto'
import { execFileSync, spawnSync } from 'node:child_process'
import { chmodSync, existsSync, lstatSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { manifestFor, targetInfo, validateArtifact } from './contract.ts'

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const repositoryRoot = execFileSync('git', ['rev-parse', '--show-toplevel'], { cwd: packageRoot, encoding: 'utf8' }).trim()
const manifestPath = resolve(repositoryRoot, 'shared/config-native/Cargo.toml')
const { target, triple } = targetInfo(process.env.OWNER_TARGET)
const output = resolve(packageRoot, 'dist')

function frozenSourceSha(): string {
  if (execFileSync('git', ['status', '--porcelain', '--untracked-files=normal'], { cwd: repositoryRoot, encoding: 'utf8' })) throw new Error('Native config build requires a clean checkout, including untracked inputs')
  return execFileSync('git', ['rev-parse', 'HEAD'], { cwd: repositoryRoot, encoding: 'utf8' }).trim()
}

const sourceSha = frozenSourceSha()
if (existsSync(output)) {
  if (lstatSync(output).isSymbolicLink() || !lstatSync(output).isDirectory()) throw new Error('Config artifact output must be a real directory')
  if (readdirSync(output).length) rmSync(output, { recursive: true })
}
mkdirSync(output, { recursive: true })
const targetDir = resolve(packageRoot, '.build', 'cargo-target')
const jobs = process.env.T3_BUILD_JOBS ?? '3'
if (!/^[1-9][0-9]*$/.test(jobs)) throw new Error('Invalid admitted Cargo job count')
const cargo = ['--locked', '--target', triple, '--target-dir', targetDir, '-j', jobs]
if (process.env.APPARENT_CARGO_OFFLINE === '1') cargo.push('--offline')
execFileSync('cargo', ['test', ...cargo, '--manifest-path', manifestPath], { cwd: repositoryRoot, stdio: 'inherit' })
execFileSync('cargo', ['build', ...cargo, '--release', '--manifest-path', manifestPath], { cwd: repositoryRoot, stdio: 'inherit' })
const built = resolve(targetDir, triple, 'release', 'apparent-config-host')
const bytes = readFileSync(built)
const staged = resolve(output, 'apparent-config-host')
writeFileSync(staged, bytes, { flag: 'wx', mode: 0o755 })
chmodSync(staged, 0o755)
const mode = statSync(staged).mode & 0o777
const manifest = manifestFor(sourceSha, target, bytes, mode)
writeFileSync(resolve(output, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`, { flag: 'wx', mode: 0o644 })
if (frozenSourceSha() !== sourceSha) throw new Error('Frozen source changed during native config build')
if (createHash('sha256').update(readFileSync(staged)).digest('hex') !== manifest.sha256) throw new Error('Staged config helper hash mismatch')
validateArtifact(output, target)

const suite = spawnSync('node', ['--experimental-strip-types', '--test', 'composition/config/test/native.test.ts'], {
  cwd: repositoryRoot,
  encoding: 'utf8',
  env: { ...process.env, APPARENT_CONFIG_HOST_BIN: staged },
})
process.stdout.write(suite.stdout ?? '')
process.stderr.write(suite.stderr ?? '')
if (suite.status !== 0) throw new Error('Public config client integration suite failed against staged native helper')
if (frozenSourceSha() !== sourceSha) throw new Error('Source changed during staged config helper verification')
validateArtifact(output, target)
console.log(`Native config host artifact built for ${target}: ${output}`)
