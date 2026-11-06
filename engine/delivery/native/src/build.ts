import { execFileSync, spawnSync } from 'node:child_process'
import { chmodSync, existsSync, lstatSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createManifest, validateArtifact, validateTarget } from './contract.ts'

const packageRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const repositoryRoot = execFileSync('git', ['rev-parse', '--show-toplevel'], { cwd: packageRoot, encoding: 'utf8' }).trim()
const manifestPath = resolve(repositoryRoot, 'engine/Cargo.toml')
const targetConfig = validateTarget(process.env.OWNER_TARGET)
const output = resolve(packageRoot, 'dist')

function cleanTrackedSource(): string {
  const status = execFileSync('git', ['status', '--porcelain', '--untracked-files=normal'], { cwd: repositoryRoot, encoding: 'utf8' })
  if (status) throw new Error('Native build requires clean source, including untracked inputs')
  return execFileSync('git', ['rev-parse', 'HEAD'], { cwd: repositoryRoot, encoding: 'utf8' }).trim()
}

function sanitizedEnvironment() {
  const env = { ...process.env }
  for (const name of ['DISPLAY', 'WAYLAND_DISPLAY', 'DBUS_SESSION_BUS_ADDRESS', 'XAUTHORITY', 'XDG_RUNTIME_DIR']) delete env[name]
  return env
}

const sourceSha = cleanTrackedSource()
if (existsSync(output)) {
  if (lstatSync(output).isSymbolicLink() || !lstatSync(output).isDirectory()) throw new Error('Native artifact output must be a real directory')
  if (readdirSync(output).length) {
    validateArtifact(output, targetConfig.target)
    rmSync(output, { recursive: true })
  }
}
mkdirSync(output, { recursive: true })
const cargoTargetDir = resolve(packageRoot, '.build', 'cargo-target')
const cargoCommon = ['--locked', '--target', targetConfig.triple, '--target-dir', cargoTargetDir]
if (process.env.APPARENT_CARGO_OFFLINE === '1') cargoCommon.push('--offline')
execFileSync('cargo', ['test', ...cargoCommon, '--manifest-path', resolve(repositoryRoot, 'engine/core/Cargo.toml')], { cwd: repositoryRoot, stdio: 'inherit' })
execFileSync('cargo', ['test', ...cargoCommon, '--manifest-path', manifestPath], { cwd: repositoryRoot, stdio: 'inherit' })
execFileSync('cargo', ['build', ...cargoCommon, '--release', '--manifest-path', manifestPath], { cwd: repositoryRoot, stdio: 'inherit' })
const binary = resolve(cargoTargetDir, targetConfig.triple, 'release', 'apparent-engine')
const bytes = readFileSync(binary)
writeFileSync(resolve(output, 'apparent-engine'), bytes, { flag: 'wx', mode: 0o755 })
chmodSync(resolve(output, 'apparent-engine'), 0o755)
const stagedMode = statSync(resolve(output, 'apparent-engine')).mode & 0o777
writeFileSync(resolve(output, 'manifest.json'), `${JSON.stringify(createManifest(sourceSha, targetConfig.target, bytes, stagedMode), null, 2)}\n`, { flag: 'wx', mode: 0o644 })
if (cleanTrackedSource() !== sourceSha) throw new Error('Tracked source changed during native build')
if (validateArtifact(output, targetConfig.target).sourceSha !== sourceSha) throw new Error('Artifact source SHA does not match the frozen build input')

const result = spawnSync('node', ['--experimental-strip-types', '--test', resolve(repositoryRoot, 'engine/tests/native-peer.test.ts')], {
  cwd: repositoryRoot,
  encoding: 'utf8',
  env: { ...sanitizedEnvironment(), APPARENT_ENGINE_BIN: resolve(output, 'apparent-engine') },
})
process.stdout.write(result.stdout ?? '')
process.stderr.write(result.stderr ?? '')
if (result.status !== 0) throw new Error('SDK-to-native protocol checks failed for the staged release executable')
if (cleanTrackedSource() !== sourceSha) throw new Error('Source changed during staged executable verification')
validateArtifact(output, targetConfig.target)
console.log(`Native Engine artifact built for ${targetConfig.target}: ${output}`)
