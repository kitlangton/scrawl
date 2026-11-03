/** Retention contract only: never native acceptance, test reuse or release authority. */
import { createHash } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { existsSync, lstatSync, mkdirSync, readFileSync, writeFileSync, chmodSync } from 'node:fs'
import { resolve, join, dirname } from 'node:path'
import { fileURLToPath } from 'node:url'
import { platform, arch } from 'node:os'

const hash = (b: Buffer | string) => createHash('sha256').update(b).digest('hex')
type Probe = (command: string, args: string[]) => string
export const focusedProducers = {
  'focused-settings-build': { workflow: 'verification-mac.yml', buildJob: 'settings-build', nativeJob: 'settings-native' },
  'focused-share-build': { workflow: 'native-proof-mac.yml', buildJob: 'share-build', nativeJob: 'connect-worker' },
} as const
type ProducerKind = 'verified-main' | keyof typeof focusedProducers
export function producerOptions(args: string[]): { kind: ProducerKind, output: string } {
  if (args.length === 1 && args[0] && !args[0].startsWith('--')) return { kind: 'verified-main', output: args[0] }
  if (args.length === 3 && args[0] === '--producer-kind' &&
      ['verified-main', ...Object.keys(focusedProducers)].includes(args[1]) && args[2] && !args[2].startsWith('--')) {
    return { kind: args[1] as ProducerKind, output: args[2] }
  }
  throw new Error('Usage: mac-artifact.ts [--producer-kind verified-main|focused-settings-build|focused-share-build] FRESH_OUTPUT_DIRECTORY')
}
const requireMatch = (s: string, re: RegExp) => { if (!re.test(s)) throw new Error('Unexpected producer identity/configuration'); return s }
const system = (p: string) => /^\/(usr\/lib|System\/Library)\/[A-Za-z0-9_./+-]+$/.test(p) && !p.split('/').includes('..')

export function audit(loads: string, developer: string, exists = existsSync) {
  const rpaths = [...loads.matchAll(/cmd LC_RPATH\s+cmdsize \d+\s+path (\S+) \(offset \d+\)/g)].map(m => m[1])
  const allowed = ['/usr/lib/swift', ...['swift-5.5', 'swift'].map(s => `${developer}/Toolchains/XcodeDefault.xctoolchain/usr/lib/${s}/macosx`)]
  if (rpaths.some(p => !allowed.includes(p))) throw new Error('Unsafe rpath')
  const dependencies = [...loads.matchAll(/cmd LC_(?:LOAD_DYLIB|LOAD_WEAK_DYLIB|REEXPORT_DYLIB|LOAD_UPWARD_DYLIB|LAZY_LOAD_DYLIB)\s+cmdsize \d+\s+name (\S+) \(offset \d+\)/g)].map(m => m[1])
  if (!dependencies.length) throw new Error('Missing dependency audit')
  // Reject unknown dependency commands and malformed names instead of silently omitting them.
  const commands = [...loads.matchAll(/cmd LC_\w*DYLIB\b/g)]
  if (commands.length !== dependencies.length) throw new Error('Unparsed dependency command')
  const closure = dependencies.map(name => {
    if (system(name)) return { name, resolved: name, provider: 'macOS' }
    if (/^@rpath\/libswift[A-Za-z0-9_]+\.dylib$/.test(name)) {
      const candidates = rpaths.map(p => `${p}/${name.slice(7)}`).filter(exists)
      // Never rely on a producer Xcode runtime or assume a shared-cache entry exists.
      if (candidates.length && system(candidates[0])) return { name, resolved: candidates[0], provider: 'macOS' }
    }
    throw new Error('Unaccounted runtime dependency')
  })
  return { rpaths, closure }
}

export function produce(root: string, output: string, env: NodeJS.ProcessEnv, probe: Probe, os = platform(), cpu = arch(), kind: ProducerKind = 'verified-main') {
  if (!['verified-main', ...Object.keys(focusedProducers)].includes(kind)) throw new Error('Unknown producer kind')
  if (os !== 'darwin' || cpu !== 'x64') throw new Error('Expected Intel macOS')
  const q = (c: string, ...args: string[]) => probe(c, args).trim()
  const git = (...args: string[]) => q('git', ...args)
  if (git('status', '--porcelain', '--untracked-files=normal')) throw new Error('Dirty checkout')
  const sha = requireMatch(git('rev-parse', 'HEAD'), /^[a-f0-9]{40}$/)
  const tree = requireMatch(git('rev-parse', 'HEAD^{tree}'), /^[a-f0-9]{40}$/)
  const route = focusedProducers[kind as keyof typeof focusedProducers]
  const focused = Boolean(route)
  let workflow: string
  if (focused) {
    const ref = requireMatch(env.GITHUB_REF ?? '', /^refs\/heads\/[A-Za-z0-9_./-]+$/)
    if (ref === 'refs/heads/main' || /\s/.test(ref) || ref.includes('..') || ref.includes('//') ||
        env.GITHUB_REPOSITORY !== 'tryapparent/apparent' || env.GITHUB_JOB !== route.buildJob ||
        env.GITHUB_EVENT_NAME !== 'workflow_dispatch' || env.GITHUB_SHA !== sha || env.GITHUB_WORKFLOW_SHA !== sha) {
      throw new Error('Focused producer must build this dispatched candidate')
    }
    workflow = `tryapparent/apparent/.github/workflows/${route.workflow}@${ref}`
    if (env.GITHUB_WORKFLOW_REF !== workflow) throw new Error('Unexpected focused producer workflow')
  } else {
    workflow = requireMatch(env.GITHUB_WORKFLOW_REF ?? '', /^tryapparent\/apparent\/\.github\/workflows\/verification(?:-mac)?\.yml@refs\/heads\/main$/)
  }
  const identity = {
    ...(focused ? { kind } : {}),
    repository: requireMatch(env.GITHUB_REPOSITORY ?? '', /^tryapparent\/apparent$/),
    workflow,
    workflow_sha: requireMatch(env.GITHUB_WORKFLOW_SHA ?? '', /^[a-f0-9]{40}$/),
    run: requireMatch(env.GITHUB_RUN_ID ?? '', /^[1-9]\d*$/),
    attempt: requireMatch(env.GITHUB_RUN_ATTEMPT ?? '', /^[1-9]\d*$/),
    job: requireMatch(env.GITHUB_JOB ?? '', focused ? new RegExp(`^${route.buildJob}$`) : /^rust$/),
  }
  const allowed: Record<string, string> = { MACOSX_DEPLOYMENT_TARGET: '15.0' }
  for (const [key, value] of Object.entries(env)) {
    if (value && /^(CARGO_(BUILD|TARGET|PROFILE|ENCODED)|RUSTC$|RUSTDOC$|RUSTFLAGS|RUSTDOCFLAGS|RUSTC_WRAPPER|RUSTC_WORKSPACE_WRAPPER|CC($|_)|CXX($|_)|AR($|_)|CFLAGS|CXXFLAGS|CPPFLAGS|LDFLAGS|SDKROOT|DYLD_|APPARENT_|MACOSX_DEPLOYMENT_TARGET)/.test(key) && allowed[key] !== value) throw new Error('Unsupported build environment')
  }
  for (let ancestor = dirname(root); ; ancestor = dirname(ancestor)) {
    if (['config', 'config.toml'].some(n => existsSync(join(ancestor, '.cargo', n)))) throw new Error('Ancestor Cargo configuration')
    if (ancestor === dirname(ancestor)) break
  }
  for (const directory of [dirname(root), env.HOME, env.CARGO_HOME ?? (env.HOME ? join(env.HOME, '.cargo') : undefined)]) {
    if (directory && ['config', 'config.toml'].some(n => existsSync(join(directory, directory === dirname(root) ? '.cargo' : '', n)))) throw new Error('External Cargo configuration')
  }
  const config = readFileSync(join(root, '.cargo/config.toml'), 'utf8')
  if (hash(config) !== '76e18c8cd9692b54a2c10bd30825b9bc1667179708a882372735bc07efa1495b') throw new Error('Unreviewed Cargo configuration')
  const rust = requireMatch(q('rustc', '-vV'), /\nhost: x86_64-apple-darwin\n/)
  const cargo = requireMatch(q('cargo', '-V'), /^cargo \d+\.\d+\.\d+ /)
  const developer = requireMatch(q('xcode-select', '-p'), /^\/Applications\/Xcode[^/]*\.app\/Contents\/Developer$/)
  if (env.DEVELOPER_DIR && env.DEVELOPER_DIR !== developer) throw new Error('Developer directory override')
  const xcode = requireMatch(q('xcodebuild', '-version'), /^Xcode 26\.0\nBuild version [A-Za-z0-9]+$/)
  const sdk = requireMatch(q('xcrun', '--sdk', 'macosx', '--show-sdk-version'), /^26\.0$/)
  const sdkPath = q('xcrun', '--sdk', 'macosx', '--show-sdk-path')
  if (!sdkPath.startsWith(`${developer}/Platforms/MacOSX.platform/Developer/SDKs/`) || sdkPath.includes('..')) throw new Error('Unexpected SDK')
  const swift = requireMatch(q('xcrun', 'swiftc', '--version'), /Apple Swift version [\s\S]+Target: x86_64-apple-macosx/)
  const files = ['apparent', 'apparent-helper'].map(name => {
    const path = join(root, 'target/debug', name)
    const stat = lstatSync(path)
    if (!stat.isFile() || stat.isSymbolicLink() || (stat.mode & 0o777) !== 0o755) throw new Error('Unsafe executable mode/type')
    if (q('lipo', '-archs', path) !== 'x86_64') throw new Error('Unexpected Mach-O architecture')
    const loads = q('otool', '-l', path)
    if (!/cmd LC_BUILD_VERSION\s+cmdsize \d+\s+platform 1\s+minos 15\.0\s+sdk 26\.0\b/.test(loads)) throw new Error('Unexpected deployment target/SDK')
    const runtime = audit(loads, developer)
    const bytes = readFileSync(path)
    return { name, mode: stat.mode & 0o777, sha256: hash(bytes), size: bytes.length, runtime, bytes }
  })
  const manifest = { schema: 1, kind: 'mac-dev-executables', source: { sha, tree, clean: true }, producer: identity,
    target: 'x86_64-apple-darwin', profile: 'dev', deployment_target: '15.0',
    cargo_lock_sha256: hash(readFileSync(join(root, 'Cargo.lock'))), config_sha256: hash(config),
    toolchain: { rust, cargo, developer, xcode, sdk, sdkPath, swift }, environment: { MACOSX_DEPLOYMENT_TARGET: env.MACOSX_DEPLOYMENT_TARGET ?? '15.0 (Cargo config)' },
    package_exclusions: ['Sparkle.framework (dlopen, fresh installer staging)', 'Electron/frontend/resources and signing (fresh installer staging)'],
    runtime_scope: 'Mach-O load commands only; not complete application/package acceptance',
    final_job_success: 'must be independently queried by consumer',
    files: files.map(({ bytes, ...metadata }) => metadata) }
  // Fresh directory, fixed raw names: no archives, symlinks, traversal or arbitrary payload paths.
  mkdirSync(output, { mode: 0o700 })
  for (const file of files) { writeFileSync(join(output, file.name), file.bytes, { flag: 'wx', mode: file.mode }); chmodSync(join(output, file.name), file.mode) }
  writeFileSync(join(output, 'manifest.json'), JSON.stringify(manifest, null, 2) + '\n', { flag: 'wx', mode: 0o600 })
  return manifest
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const options = producerOptions(process.argv.slice(2))
  produce(process.cwd(), resolve(options.output), process.env, (command, args) => {
    const result = spawnSync(command, args, { encoding: 'utf8', timeout: 15000, maxBuffer: 16 * 1024 * 1024 })
    if (result.status !== 0) throw new Error(`Producer probe failed: ${command}`)
    return result.stdout
  }, platform(), arch(), options.kind)
}
