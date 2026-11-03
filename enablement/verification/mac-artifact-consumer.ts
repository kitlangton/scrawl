/** Exact-source development inputs only; never a release or native acceptance receipt. */
import { spawnSync } from 'node:child_process'
import { createHash } from 'node:crypto'
import { chmodSync, existsSync, lstatSync, readFileSync, readdirSync } from 'node:fs'
import { resolve, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { platform, arch } from 'node:os'
import { audit } from './mac-artifact.ts'

const repository = 'tryapparent/apparent'
const digest = (data: Buffer | string) => createHash('sha256').update(data).digest('hex')
const check = (ok: unknown, message: string) => { if (!ok) throw new Error(message) }
const number = (value: string) => { check(/^[1-9]\d*$/.test(value), 'Invalid immutable identity'); return value }
export function admit(run: any, artifact: any, jobs: any[], runId: string, artifactId: string) {
  number(runId); number(artifactId)
  check(String(run.id) === runId && run.repository?.full_name === repository && run.head_repository?.full_name === repository,
    'Wrong producer repository/run')
  check(run.path === '.github/workflows/verification-mac.yml' && run.head_branch === 'main' &&
    run.event === 'workflow_run' && run.status === 'completed' && run.conclusion === 'success', 'Producer did not succeed on trusted main')
  const attempt = number(String(run.run_attempt))
  check(String(artifact.id) === artifactId && artifact.expired === false && artifact.workflow_run?.id === run.id &&
    artifact.name === `mac-dev-executables-${runId}-${attempt}`, 'Wrong or expired artifact')
  check(/^sha256:[a-f0-9]{64}$/.test(artifact.digest), 'Missing trusted archive digest')
  const rust = jobs.filter(j => j.name === 'verify / rust (macos-15-intel)')
  check(rust.length === 1 && rust[0].status === 'completed' && rust[0].conclusion === 'success' &&
    String(rust[0].run_id) === runId && String(rust[0].run_attempt) === attempt, 'Producer job did not succeed')
  return { run: runId, attempt, artifact: artifactId, digest: artifact.digest as string }
}
export function unpack(bytes: Buffer, expectedDigest: string, directory: string) {
  check(`sha256:${digest(bytes)}` === expectedDigest, 'Artifact transport digest mismatch')
  check(!existsSync(directory), 'Artifact destination must be fresh')
  // zipfile only reads the authenticated archive. Never use extractall or honor
  // archive paths/modes; write precisely the three fixed regular-file payloads.
  const script = `import io, os, stat, sys, zipfile
with zipfile.ZipFile(io.BytesIO(sys.stdin.buffer.read())) as archive:
    entries = archive.infolist()
    assert sorted(e.filename for e in entries) == ['apparent', 'apparent-helper', 'manifest.json']
    assert sum(e.file_size for e in entries) <= 768 * 1024 * 1024
    for e in entries:
        mode = e.external_attr >> 16
        assert not e.is_dir() and stat.S_IFMT(mode) in (0, stat.S_IFREG)
        assert 0 < e.file_size <= (1024 * 1024 if e.filename == 'manifest.json' else 512 * 1024 * 1024)
    os.mkdir(sys.argv[1], 0o700)
    for e in entries:
        with open(os.path.join(sys.argv[1], e.filename), 'xb') as target:
            os.chmod(target.name, 0o600)
            target.write(archive.read(e))
`
  const result = spawnSync('python3', ['-c', script, directory], { input: bytes, timeout: 60000, maxBuffer: 1024 * 1024 })
  check(result.status === 0, 'Unsafe or invalid artifact archive')
}
export function verify(directory: string, manifest: any, expected: { sha: string, tree: string, lock: string, config: string }, identity: ReturnType<typeof admit>) {
  check(!lstatSync(directory).isSymbolicLink(), 'Unsafe payload directory')
  check(JSON.stringify(readdirSync(directory).sort()) === JSON.stringify(['apparent', 'apparent-helper', 'manifest.json']), 'Unexpected payload files')
  check(manifest.schema === 1 && manifest.kind === 'mac-dev-executables' && manifest.source?.clean === true &&
    manifest.source.sha === expected.sha && manifest.source.tree === expected.tree, 'Wrong source identity')
  check(manifest.producer?.repository === repository && manifest.producer.run === identity.run && manifest.producer.attempt === identity.attempt &&
    manifest.producer.job === 'rust' && manifest.producer.workflow === `${repository}/.github/workflows/verification-mac.yml@refs/heads/main`, 'Wrong manifest producer')
  check(manifest.target === 'x86_64-apple-darwin' && manifest.profile === 'dev' && manifest.deployment_target === '15.0' &&
    manifest.cargo_lock_sha256 === expected.lock && manifest.config_sha256 === expected.config, 'Incompatible build configuration')
  check(Array.isArray(manifest.files) && manifest.files.length === 2, 'Wrong executable set')
  for (const name of ['manifest.json', 'apparent', 'apparent-helper']) {
    const stat = lstatSync(join(directory, name))
    check(stat.isFile() && !stat.isSymbolicLink() && stat.nlink === 1, 'Unsafe payload file')
    if (name === 'manifest.json') continue
    const files = manifest.files.filter((f: any) => f.name === name)
    check(files.length === 1, 'Wrong executable name')
    const f = files[0]
    check(f.mode === 0o755 && f.size === stat.size && f.sha256 === digest(readFileSync(join(directory, name))), 'Executable hash/mode mismatch')
  }
}
function command(name: string, ...args: string[]) {
  const r = spawnSync(name, args, { encoding: 'utf8', timeout: 30000, maxBuffer: 16 * 1024 * 1024 })
  check(r.status === 0, `Admission probe failed: ${name}`)
  return r.stdout.trim()
}
function github(path: string) { return JSON.parse(command('gh', 'api', `repos/${repository}/${path}`)) }
export function trustedIdentity(runId: string, artifactId: string) {
  number(runId); number(artifactId)
  const run = github(`actions/runs/${runId}`)
  const artifact = github(`actions/artifacts/${artifactId}`)
  const attempt = number(String(run.run_attempt))
  const response = github(`actions/runs/${runId}/attempts/${attempt}/jobs?per_page=100`)
  check(response.total_count <= 100, 'Unexpected job pagination')
  return admit(run, artifact, response.jobs, runId, artifactId)
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const [mode, ...args] = process.argv.slice(2)
  if (mode === 'admit' && args.length === 2) {
    trustedIdentity(args[0], args[1])
  } else if (mode === 'prepare' && args.length === 1) {
    check(platform() === 'darwin' && arch() === 'x64', 'Expected Intel macOS')
    const identity = trustedIdentity(process.env.APPARENT_PREBUILT_RUN ?? '', process.env.APPARENT_PREBUILT_ARTIFACT ?? '')
    check(!command('git', 'status', '--porcelain', '--untracked-files=normal'), 'Dirty consumer checkout')
    const directory = resolve(args[0])
    check(!existsSync(directory), 'Artifact destination must be fresh')
    const download = spawnSync('gh', ['api', `repos/${repository}/actions/artifacts/${identity.artifact}/zip`],
      { timeout: 60000, maxBuffer: 512 * 1024 * 1024 })
    check(download.status === 0, 'Immutable artifact download failed')
    unpack(download.stdout, identity.digest, directory)
    const manifestPath = join(directory, 'manifest.json')
    check(lstatSync(manifestPath).isFile() && !lstatSync(manifestPath).isSymbolicLink(), 'Unsafe manifest')
    const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'))
    verify(directory, manifest, { sha: command('git', 'rev-parse', 'HEAD'), tree: command('git', 'rev-parse', 'HEAD^{tree}'),
      lock: digest(readFileSync('Cargo.lock')), config: digest(readFileSync('.cargo/config.toml')) }, identity)
    const tools = { rust: command('rustc', '-vV'), cargo: command('cargo', '-V'), developer: command('xcode-select', '-p'),
      xcode: command('xcodebuild', '-version'), sdk: command('xcrun', '--sdk', 'macosx', '--show-sdk-version'),
      sdkPath: command('xcrun', '--sdk', 'macosx', '--show-sdk-path'), swift: command('xcrun', 'swiftc', '--version') }
    for (const [key, value] of Object.entries(tools)) check(manifest.toolchain?.[key] === value, 'Consumer toolchain differs from producer')
    for (const name of ['apparent', 'apparent-helper']) {
      const file = join(directory, name)
      check(command('lipo', '-archs', file) === 'x86_64', 'Wrong executable architecture')
      const runtime = audit(command('otool', '-l', file), tools.developer)
      check(JSON.stringify(runtime) === JSON.stringify(manifest.files.find((f: any) => f.name === name).runtime), 'Runtime closure mismatch')
      chmodSync(file, 0o755) // Actions transports raw files as 0644; only admitted fixed names regain execution.
    }
    console.log(JSON.stringify({ source: manifest.source, producer: identity, inputFiles: manifest.files }))
  } else throw new Error('Usage: mac-artifact-consumer.ts admit RUN ARTIFACT | prepare FRESH_DIRECTORY')
}
