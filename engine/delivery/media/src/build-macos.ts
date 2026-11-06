import { execFileSync } from 'node:child_process'
import { chmodSync, copyFileSync, existsSync, lstatSync, mkdirSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { inventory } from './contract.ts'
import { validateMacArtifact, type MacManifest } from './macos-contract.ts'

if (process.platform !== 'darwin' || process.arch !== 'x64' || process.env.OWNER_TARGET !== 'macos-x64') throw new Error('Mac production requires native macos-x64')
const pkg = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const run = (cmd: string, args: string[], cwd = pkg) => execFileSync(cmd, args, { cwd, encoding: 'utf8' }).trim()
const root = run('git', ['rev-parse', '--show-toplevel'])
const frozen = () => { if (run('git', ['status', '--porcelain', '--untracked-files=normal'], root)) throw new Error('clean source required'); return run('git', ['rev-parse', 'HEAD'], root) }
const sourceSha = frozen()
const output = resolve(pkg, 'dist')
if (existsSync(output)) { if (lstatSync(output).isSymbolicLink() || !lstatSync(output).isDirectory()) throw new Error('unsafe dist'); if (readdirSync(output).length) { validateMacArtifact(output); rmSync(output, { recursive: true }) } }
mkdirSync(resolve(output, 'libexec'), { recursive: true })
const cargoTarget = resolve(pkg, '.build/cargo-target')
execFileSync('cargo', ['build', '--manifest-path', resolve(root, 'engine/Cargo.toml'), '--features', 'media', '--bin', 'apparent-engine-frames', '--release', '--locked', ...(process.env.APPARENT_CARGO_OFFLINE === '1' ? ['--offline'] : []), '--target', 'x86_64-apple-darwin', '--target-dir', cargoTarget], { cwd: root, stdio: 'inherit', env: { ...process.env, CARGO_BUILD_JOBS: process.env.T3_BUILD_JOBS ?? '3', MACOSX_DEPLOYMENT_TARGET: '15.0' } })
const worker = resolve(output, 'libexec/apparent-engine-frames')
const helper = resolve(output, 'libexec/apparent-engine-frames-native')
copyFileSync(resolve(cargoTarget, 'x86_64-apple-darwin/release/apparent-engine-frames'), worker)
execFileSync('xcrun', ['swiftc', '-O', '-swift-version', '5', '-target', 'x86_64-apple-macos15.0', '-parse-as-library', resolve(root, 'engine/native/Media.swift'), resolve(root, 'engine/native/FramesBridge.swift'), '-o', helper], { stdio: 'inherit' })
copyFileSync(resolve(pkg, 'src/launch-macos.sh'), resolve(output, 'apparent-engine-frames'))
const libraries = new Set<string>()
for (const file of [worker, helper]) {
  if (run('lipo', ['-archs', file]) !== 'x86_64') throw new Error('unexpected Mach-O architecture')
  for (const line of run('otool', ['-L', file]).split('\n').slice(1)) {
    const dependency = line.trim().split(' (')[0]
    if (!dependency.startsWith('/usr/lib/') && !dependency.startsWith('/System/Library/')) throw new Error(`non-system runtime dependency: ${dependency}`)
    libraries.add(dependency)
  }
}
for (const file of ['apparent-engine-frames', 'libexec/apparent-engine-frames', 'libexec/apparent-engine-frames-native']) chmodSync(resolve(output, file), 0o755)
const manifest: MacManifest = { schema: 1, sourceSha, target: 'macos-x64', rustTriple: 'x86_64-apple-darwin', capabilities: ['media/1'], launch: { entry: 'apparent-engine-frames', worker: 'libexec/apparent-engine-frames', nativeHelper: 'libexec/apparent-engine-frames-native', argv: ['--media-runtime-directory', '<private-runtime-dir>', '--media-channel', 'media', '--media-protocol-version', '1'], pathLookup: false, runtimeBuilds: false }, environment: { architecture: 'x86_64', minimumMacOS: '15.0', osRelease: run('sw_vers', ['-productVersion']), sdk: run('xcrun', ['--show-sdk-version']), swift: run('xcrun', ['swiftc', '--version']), rustc: run('rustc', ['--version']), cargo: run('cargo', ['--version']), systemLibraries: [...libraries].sort() }, runtimeFiles: inventory(output) }
writeFileSync(resolve(output, 'manifest.json'), `${JSON.stringify(manifest, null, 2)}\n`, { flag: 'wx', mode: 0o644 })
validateMacArtifact(output)
execFileSync('node', ['--experimental-strip-types', '--test', resolve(pkg, 'test/packaged-h264.test.ts'), resolve(root, 'engine/tests/media-peer.test.ts')], { cwd: pkg, stdio: 'inherit', env: { ...process.env, APPARENT_ENGINE_FRAMES_BIN: resolve(output, 'apparent-engine-frames') } })
if (frozen() !== sourceSha) throw new Error('source changed during Mac build')
validateMacArtifact(output)
