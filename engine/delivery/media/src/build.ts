import { execFileSync, spawnSync } from 'node:child_process'
import { chmodSync, copyFileSync, existsSync, lstatSync, mkdirSync, readFileSync, readdirSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { BASELINE_LIBRARIES, inventory, REQUIRED_PLUGINS, sha256, validateArtifact, type MediaManifest } from './contract.ts'

const pkg = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const root = execFileSync('git', ['rev-parse', '--show-toplevel'], { cwd: pkg, encoding: 'utf8' }).trim()
const output = resolve(pkg, 'dist')
function frozen(): string { const dirty = execFileSync('git', ['status', '--porcelain', '--untracked-files=normal'], { cwd: root, encoding: 'utf8' }); if (dirty) throw new Error('media build requires a clean checkout'); return execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim() }
function run(command: string, args: string[], cwd = root): string { return execFileSync(command, args, { cwd, encoding: 'utf8' }).trim() }
function copyLibrary(path: string, libDir: string) { const real = run('realpath', ['-e', path]); const name = path.split('/').at(-1)!; const destination = resolve(libDir, name); if (!existsSync(destination)) copyFileSync(real, destination, 0); return destination }
const sourceSha = frozen()
if (process.env.CI && process.env.APPARENT_GST_PLUGIN_SOURCE) throw new Error('plugin overrides are local proof inputs only; admitted CI must use its declared system runtime')
if (process.platform !== 'linux' || process.arch !== 'x64') throw new Error('media producer only builds on native linux-x64')
if (process.env.OWNER_TARGET !== 'linux-x64') throw new Error('OWNER_TARGET=linux-x64 is required')
const plugins = [...REQUIRED_PLUGINS]
const gstDir = run('pkg-config', ['--variable=pluginsdir', 'gstreamer-1.0'])
const stagePlugins = resolve(output, 'plugins'); const libDir = resolve(output, 'lib'); const workerDir = resolve(output, 'libexec')
if (existsSync(output)) { if (lstatSync(output).isSymbolicLink() || !lstatSync(output).isDirectory()) throw new Error('dist must be a real directory'); if (readdirSync(output).length) { validateArtifact(output); rmSync(output, { recursive: true }) } }
const pluginInputs = plugins.map(name => {
  const override = process.env.APPARENT_GST_PLUGIN_SOURCE ? resolve(process.env.APPARENT_GST_PLUGIN_SOURCE, name) : undefined
  const from = override && existsSync(override) ? override : resolve(gstDir, name)
  if (lstatSync(from).isSymbolicLink() || !statSync(from).isFile()) throw new Error(`required regular plugin is missing: ${name}`)
  return { name, from }
})
mkdirSync(stagePlugins, { recursive: true }); mkdirSync(libDir); mkdirSync(workerDir)
for (const { name, from } of pluginInputs) copyFileSync(from, resolve(stagePlugins, name), 0)
const cargoTarget = resolve(pkg, '.build', 'cargo-target')
const common = ['--manifest-path', resolve(root, 'engine/Cargo.toml'), '--features', 'media', '--bin', 'apparent-engine-frames', '--release', '--locked', '--target-dir', cargoTarget]
if (process.env.APPARENT_CARGO_OFFLINE === '1') common.push('--offline')
const admission = '/home/simon/.local/bin/t3-build'
const needsAdmission = existsSync(admission) && !process.env.T3_BUILD_JOBS
execFileSync(needsAdmission ? admission : 'cargo', needsAdmission ? ['--jobs', '3', '--min-jobs', '3', '--', 'cargo', 'build', ...common] : ['build', ...common], { cwd: root, stdio: 'inherit', env: { ...process.env, CARGO_BUILD_JOBS: process.env.T3_BUILD_JOBS ?? '2' } })
const worker = resolve(cargoTarget, 'release/apparent-engine-frames')
copyFileSync(worker, resolve(workerDir, 'apparent-engine-frames'), 0); chmodSync(resolve(workerDir, 'apparent-engine-frames'), 0o755)
const launch = resolve(pkg, 'src/launch.sh'); copyFileSync(launch, resolve(output, 'apparent-engine-frames'), 0); chmodSync(resolve(output, 'apparent-engine-frames'), 0o755)

// Resolve the executable/plugin ELF closure, bundling every dependency outside
// the declared Ubuntu base-OS set under its required SONAME.
const baseline = new Set<string>(BASELINE_LIBRARIES)
const pending = [resolve(workerDir, 'apparent-engine-frames'), ...plugins.map(name => resolve(stagePlugins, name))]
const seen = new Set<string>()
while (pending.length) {
  const file = pending.pop()!; if (seen.has(file)) continue; seen.add(file)
  const outputText = run('ldd', [file])
  if (/=>\s+not found(?:\s|$)/m.test(outputText)) throw new Error(`unresolved runtime dependency: ${file}`)
  for (const line of outputText.split('\n')) {
    const match = line.match(/^\s*(\S+)\s+=>\s+(\/\S+)/); if (!match) continue
    const [, soname, resolved] = match
    if (baseline.has(soname)) continue
    const staged = resolve(libDir, soname); if (!existsSync(staged)) copyFileSync(resolved, staged, 0)
    pending.push(staged)
  }
}

const osRelease = readFileSync('/etc/os-release', 'utf8').split('\n').find(x => x.startsWith('PRETTY_NAME='))?.slice(12).replace(/^"|"$/g, '') ?? 'unknown'
const glibc = run('ldd', ['--version']).split('\n')[0]
const rustc = run('rustc', ['--version']); const cargo = run('cargo', ['--version']); const gstreamer = run('pkg-config', ['--modversion', 'gstreamer-1.0'])
const pluginSource = process.env.APPARENT_GST_PLUGIN_SOURCE ? `pkg-config:${gstDir};explicit-override:${process.env.APPARENT_GST_PLUGIN_SOURCE}` : `pkg-config:${gstDir}`
const manifest: MediaManifest = { schema: 1, sourceSha, target: 'linux-x64', rustTriple: 'x86_64-unknown-linux-gnu', capabilities: ['media/1'], launch: { entry: 'apparent-engine-frames', argv: ['--media-runtime-directory', '<private-runtime-dir>', '--media-channel', 'media', '--media-protocol-version', '1'], worker: 'libexec/apparent-engine-frames', pathLookup: false, runtimeBuilds: false }, environment: { osRelease, kernel: run('uname', ['-r']), architecture: 'x86_64', rustc, cargo, gstreamer, glibc, pluginSource, plugins, baseline: [...baseline].sort() }, runtimeFiles: inventory(output) }
writeFileSync(resolve(output, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`, { flag: 'wx', mode: 0o644 })
if (frozen() !== sourceSha) throw new Error('source changed during media build')
validateArtifact(output)
const test = spawnSync('node', ['--experimental-strip-types', '--test', 'test/contract.test.ts', 'test/packaged-h264.test.ts', resolve(root, 'engine/tests/media-peer.test.ts')], { cwd: pkg, encoding: 'utf8', env: { ...process.env, APPARENT_ENGINE_FRAMES_BIN: resolve(output, 'apparent-engine-frames') } })
process.stdout.write(test.stdout ?? ''); process.stderr.write(test.stderr ?? '')
if (test.status !== 0) throw new Error('SDK to packaged native worker tests failed')
if (frozen() !== sourceSha) throw new Error('source changed during packaged media verification')
validateArtifact(output)
console.log(`Media artifact ${sourceSha}: ${output}`)
