import { createHash } from 'node:crypto'
import { chmodSync, copyFileSync, existsSync, lstatSync, mkdirSync, readdirSync, readFileSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { dirname, relative, resolve, sep } from 'node:path'
import { execFileSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { recoveryLaunch } from '../../media/src/recovery-contract.ts'
import { pipewireRuntimeInputs } from './pipewire-runtime.ts'

const pkg = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const root = execFileSync('git', ['rev-parse', '--show-toplevel'], { cwd: pkg, encoding: 'utf8' }).trim()
const output = resolve(pkg, 'dist')
const run = (command: string, args: string[]) => execFileSync(command, args, { cwd: root, encoding: 'utf8' }).trim()
const sourceSha = run('git', ['rev-parse', 'HEAD'])
function worktreeHash(): string | undefined {
  const status = run('git', ['status', '--porcelain', '--untracked-files=all'])
  if (!status) return undefined
  const digest = createHash('sha256').update(status).update(run('git', ['diff', '--binary', 'HEAD']))
  for (const name of run('git', ['ls-files', '--modified', '--others', '--exclude-standard']).split('\n').filter(Boolean).sort()) {
    digest.update(name).update(readFileSync(resolve(root, name)))
  }
  for (const name of run('git', ['ls-files', '--deleted']).split('\n').filter(Boolean).sort()) digest.update(name).update('deleted')
  return digest.digest('hex')
}
const initialWorktreeDigest = worktreeHash()
const clean = initialWorktreeDigest === undefined
if (!clean && process.env.APPARENT_ALLOW_DIRTY_SOURCE !== '1') throw new Error('capture artifact build requires a clean checkout')
if (process.platform !== 'linux' || process.arch !== 'x64' || process.env.OWNER_TARGET !== 'linux-x64') throw new Error('capture producer supports native linux-x64 only; set OWNER_TARGET=linux-x64')
const worktreeDigest = initialWorktreeDigest
const pluginDir = run('pkg-config', ['--variable=pluginsdir', 'gstreamer-1.0'])
const encoderPlugin = ['libgstx264.so', 'libgstopenh264.so'].find(name => existsSync(resolve(pluginDir, name)))
if (!encoderPlugin) throw new Error('x264 or OpenH264 GStreamer plugin is required')
const decoderPlugin = ['libgstopenh264.so', 'libgstlibav.so'].find(name => existsSync(resolve(pluginDir, name)))
if (!decoderPlugin) throw new Error('OpenH264 decoder or GStreamer libav plugin is required')
const plugins = [...new Set(['libgstcoreelements.so', 'libgstapp.so', 'libgstvideorate.so', 'libgsttypefindfunctions.so', 'libgstplayback.so', 'libgstvideoconvertscale.so', 'libgstisomp4.so', 'libgstximagesrc.so', 'libgstpipewire.so', encoderPlugin, decoderPlugin, 'libgstvideoparsersbad.so'])]
const pipewireInputs = pipewireRuntimeInputs(run('pkg-config', ['--variable=libdir', 'gstreamer-1.0']), resolve(run('pkg-config', ['--variable=prefix', 'gstreamer-1.0']), 'share'))
const outputStat = existsSync(output) ? lstatSync(output) : undefined
if (outputStat?.isSymbolicLink() || (outputStat && !outputStat.isDirectory())) throw new Error('dist must be a real directory')
if (outputStat) rmSync(output, { recursive: true })
const pluginInputs = plugins.map(name => {
  const path = resolve(pluginDir, name)
  if (!existsSync(path) || lstatSync(path).isSymbolicLink() || !statSync(path).isFile()) throw new Error(`required runtime plugin missing: ${name}`)
  return { name, path }
})
const pluginOut = resolve(output, 'plugins')
const libOut = resolve(output, 'lib')
const workerOut = resolve(output, 'libexec')
mkdirSync(pluginOut, { recursive: true }); mkdirSync(libOut); mkdirSync(workerOut)
for (const plugin of pluginInputs) copyFileSync(plugin.path, resolve(pluginOut, plugin.name), 0)
for (const input of pipewireInputs) {
  const destination = resolve(output, input.path)
  mkdirSync(dirname(destination), { recursive: true })
  copyFileSync(input.source, destination, 0)
}
const target = resolve(pkg, '.build', 'cargo-target')
const cargo = ['build', '--manifest-path', resolve(root, 'engine/Cargo.toml'), '--features', 'capture', '--bin', 'apparent-engine-capture', '--release', '--locked', '--target-dir', target]
if (process.env.APPARENT_CARGO_OFFLINE === '1') cargo.push('--offline')
const admission = '/home/simon/.local/bin/t3-build'
if (existsSync(admission) && !process.env.T3_BUILD_JOBS) execFileSync(admission, ['--jobs', '3', '--min-jobs', '3', '--', 'cargo', ...cargo], { cwd: root, stdio: 'inherit', env: { ...process.env, CARGO_BUILD_JOBS: '3' } })
else execFileSync('cargo', cargo, { cwd: root, stdio: 'inherit', env: { ...process.env, CARGO_BUILD_JOBS: process.env.T3_BUILD_JOBS ?? '3' } })
if (worktreeHash() !== worktreeDigest) throw new Error('source tree changed during capture build')
const worker = resolve(target, 'release/apparent-engine-capture')
copyFileSync(worker, resolve(workerOut, 'apparent-engine-capture'), 0); chmodSync(resolve(workerOut, 'apparent-engine-capture'), 0o755)
copyFileSync(worker, resolve(output, 'apparent-engine-capture'), 0); chmodSync(resolve(output, 'apparent-engine-capture'), 0o755)
copyFileSync(resolve(pkg, 'src/launch.sh'), resolve(output, 'apparent-engine-capture-launch'), 0); chmodSync(resolve(output, 'apparent-engine-capture-launch'), 0o755)

const baseline = new Set(['ld-linux-x86-64.so.2', 'libc.so.6', 'libdl.so.2', 'libgcc_s.so.1', 'libm.so.6', 'libpthread.so.0', 'librt.so.1'])
const pending = [resolve(workerOut, 'apparent-engine-capture'), ...plugins.map(name => resolve(pluginOut, name)), ...pipewireInputs.filter(input => input.path.endsWith('.so')).map(input => resolve(output, input.path))]
const seen = new Set<string>()
while (pending.length) {
  const file = pending.pop()!
  if (seen.has(file)) continue
  seen.add(file)
  const output = run('ldd', [file])
  if (/=>\s+not found(?:\s|$)/m.test(output)) throw new Error(`unresolved runtime dependency: ${file}`)
  for (const line of output.split('\n')) {
    const match = line.match(/^\s*(\S+)\s+=>\s+(\/\S+)/)
    if (!match) continue
    const [, soname, resolved] = match
    if (baseline.has(soname)) continue
    const staged = resolve(libOut, soname)
    if (!existsSync(staged)) copyFileSync(resolved, staged, 0)
    pending.push(staged)
  }
}
function inventory(dir: string): Array<{ path: string; mode: number; sha256: string }> {
  const values: Array<{ path: string; mode: number; sha256: string }> = []
  const walk = (current: string) => { for (const entry of readdirSync(current).sort()) { const path = resolve(current, entry); const st = lstatSync(path); if (st.isSymbolicLink()) throw new Error('runtime artifacts cannot contain symlinks'); if (st.isDirectory()) walk(path); else if (st.isFile()) values.push({ path: relative(dir, path).split(sep).join('/'), mode: st.mode & 0o777, sha256: createHash('sha256').update(readFileSync(path)).digest('hex') }); else throw new Error('unsupported runtime artifact'); } }
  walk(dir); return values
}
const entries = inventory(output).filter(item => !item.path.endsWith('manifest.json'))
const expected = ['apparent-engine-capture', 'apparent-engine-capture-launch', 'libexec/apparent-engine-capture', ...plugins.map(name => `plugins/${name}`), ...pipewireInputs.map(input => input.path), ...entries.filter(item => item.path.startsWith('lib/')).map(item => item.path)].sort()
if (JSON.stringify(entries.map(item => item.path).sort()) !== JSON.stringify([...new Set(expected)].sort())) throw new Error('unexpected capture runtime inventory')
const manifest = { schema: 1, sourceSha, recoveryLaunch: recoveryLaunch('capture/2'), ...(worktreeDigest ? { worktreeDigest, eligibleForReuse: false } : { eligibleForReuse: true }), target: 'linux-x64', rustTriple: 'x86_64-unknown-linux-gnu', capabilities: ['capture/2', 'x11-display', 'x11-area', 'hyprland-single-display', 'retained-materials', 'authenticated-publication'], launch: { entry: 'apparent-engine-capture-launch', argv: ['--runtime-directory', '<private-runtime-dir>', '--capture-channel', 'capture', '--capture-protocol-version', '2'], worker: 'libexec/apparent-engine-capture', pathLookup: false, runtimeBuilds: false, shutdownTimeoutMs: 40_000 }, environment: { architecture: 'x86_64', osRelease: readFileSync('/etc/os-release', 'utf8').split('\n').find(x => x.startsWith('PRETTY_NAME='))?.slice(12).replace(/^"|"$/g, '') ?? 'unknown', kernel: run('uname', ['-r']), rustc: run('rustc', ['--version']), cargo: run('cargo', ['--version']), gstreamer: run('pkg-config', ['--modversion', 'gstreamer-1.0']), pluginSource: `pkg-config:${pluginDir}`, plugins, baseline: [...baseline].sort() }, runtimeFiles: entries }
writeFileSync(resolve(output, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`, { flag: 'wx', mode: 0o644 })
if (worktreeHash() !== worktreeDigest) throw new Error('source tree changed during artifact staging')
// Check publication through the closed release entry, without capture/display
// access or debug-only fault injection. Other peer scenarios run in their own
// native conformance lane; this filter executes the release-compatible case.
const peerEnvironment: NodeJS.ProcessEnv = { ...process.env, APPARENT_ENGINE_CAPTURE_BIN: resolve(output, 'apparent-engine-capture-launch') }
delete peerEnvironment.DISPLAY
delete peerEnvironment.WAYLAND_DISPLAY
delete peerEnvironment.DBUS_SESSION_BUS_ADDRESS
execFileSync(process.execPath, ['--experimental-strip-types', '--test', '--test-name-pattern=SDK packaged publication', 'engine/tests/capture-publication-peer.test.ts'], { cwd: root, stdio: 'inherit', env: peerEnvironment })
if (worktreeHash() !== worktreeDigest) throw new Error('source tree changed during publication verification')
console.log(`Capture artifact ${sourceSha}: ${output}`)
