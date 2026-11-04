/** Observe verification without granting reuse, publication or native acceptance. */
import { createHash, randomUUID } from 'node:crypto'
import { spawn, spawnSync } from 'node:child_process'
import { appendFileSync, lstatSync, mkdirSync, readFileSync, readlinkSync, realpathSync, renameSync, writeFileSync } from 'node:fs'
import { arch, platform, release } from 'node:os'
import { basename, dirname, isAbsolute, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

const source = fileURLToPath(import.meta.url)
const digest = (value: string | Buffer) => createHash('sha256').update(value).digest('hex')
function git(root: string, ...args: string[]): Buffer {
  // Snapshot reads must not refresh the index stat cache: the build verifier
  // treats the index as an input and protects its own reads the same way.
  const result = spawnSync('git', args, {
    cwd: root, maxBuffer: 64 * 1024 * 1024,
    env: { ...process.env, GIT_OPTIONAL_LOCKS: '0' },
  })
  if (result.status !== 0) throw new Error('Cannot establish verification source identity')
  return result.stdout
}

export function snapshot(root: string) {
  const paths = [...new Set(git(root, 'ls-files', '-z', '--cached', '--others', '--exclude-standard')
    .toString('utf8').split('\0').filter(Boolean))].sort()
  const hash = createHash('sha256')
  for (const path of paths) {
    const full = resolve(root, path)
    let value: string
    try {
      const stat = lstatSync(full)
      value = stat.isSymbolicLink() ? `link:${readlinkSync(full)}`
        : stat.isFile() ? `file:${stat.mode & 0o777}:${digest(readFileSync(full))}` : 'non-file'
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error
      value = 'missing'
    }
    hash.update(JSON.stringify([path, value]) + '\n')
  }
  return {
    sha: git(root, 'rev-parse', 'HEAD').toString().trim(),
    fingerprint: hash.digest('hex'),
    clean: git(root, 'status', '--porcelain', '--untracked-files=normal').length === 0,
    scope: 'all tracked and nonignored source; ignored/generated and external inputs are not attested',
  }
}

function toolchains() {
  return Object.fromEntries(['git', 'python3', 'node', 'npm', 'bun', 'rustc', 'cargo'].map(tool => {
    const result = spawnSync(tool, ['--version'], { encoding: 'utf8', timeout: 5000 })
    // Version probes can include machine-specific paths. Only retain a digest.
    return [tool, result.status === 0 ? digest(result.stdout.trim()) : null]
  }))
}

function atomicWrite(path: string, value: unknown) {
  const temporary = `${path}.${randomUUID()}.tmp`
  writeFileSync(temporary, JSON.stringify(value, null, 2) + '\n', { mode: 0o600, flag: 'wx' })
  renameSync(temporary, path)
}

export function externalDirectory(root: string, output: string): string {
  root = realpathSync(root)
  const external = (path: string) => {
    const distance = relative(root, path)
    if (!distance || (!distance.startsWith('../') && !isAbsolute(distance))) {
      throw new Error('Receipt directory must be outside the checkout')
    }
  }
  output = resolve(output)
  external(output)
  // Resolve the nearest existing ancestor before creating any missing children.
  // An external-looking parent symlink may lead back into the checkout.
  let ancestor = output
  const missing: string[] = []
  while (true) {
    try {
      external(resolve(realpathSync(ancestor), ...missing))
      break
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error
      missing.unshift(basename(ancestor))
      ancestor = dirname(ancestor)
    }
  }
  mkdirSync(output, { recursive: true })
  output = realpathSync(output)
  external(output)
  return output
}

export async function observe(root: string, suite: string, output: string, command: string[]) {
  if (!/^[a-z0-9][a-z0-9._-]{0,79}$/.test(suite) || !command.length) throw new Error('Invalid suite or empty command')
  const cwd = realpathSync(root)
  root = realpathSync(git(cwd, 'rev-parse', '--show-toplevel').toString().trim())
  output = externalDirectory(root, output)
  const path = resolve(output, `${suite}-${randomUUID()}.json`)
  const before = snapshot(root)
  const env = process.env
  const repo = env.GITHUB_REPOSITORY ?? ''
  const run = env.GITHUB_RUN_ID ?? ''
  const attempt = env.GITHUB_RUN_ATTEMPT ?? ''
  const evidence = /^[\w.-]+\/[\w.-]+$/.test(repo) && /^\d+$/.test(run) && /^\d+$/.test(attempt)
    ? `https://github.com/${repo}/actions/runs/${run}/attempts/${attempt}` : null
  const receipt: Record<string, unknown> = {
    schema: 1, kind: 'verification-observation', suite, source: before,
    platform: { os: platform(), arch: arch(), release: release() },
    toolchains: toolchains(), verifier: digest(readFileSync(source)),
    command: { cwd: relative(root, cwd) || '.', program: basename(command[0]), argv_sha256: digest(JSON.stringify(command)) },
    started_at: new Date().toISOString(), finished_at: null, duration_seconds: null,
    outcome: 'in_progress', exit_code: null, signal: null, evidence,
    reusable: false, reason: 'Observation only; no complete external/generated input closure or native proof',
  }
  // Write before spawning: SIGKILL or machine loss leaves an incomplete record.
  atomicWrite(path, receipt)
  const start = performance.now()
  let requested: NodeJS.Signals | null = null
  let timer: NodeJS.Timeout | undefined
  let child: ReturnType<typeof spawn> | undefined
  const interrupt = (signal: NodeJS.Signals) => {
    requested = signal
    const kill = (value: NodeJS.Signals) => {
      if (child?.pid) try { process.kill(-child.pid, value) } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== 'ESRCH') throw error
      }
    }
    kill(signal)
    timer ??= setTimeout(() => kill('SIGKILL'), 2000)
  }
  const onTerm = () => interrupt('SIGTERM')
  const onInt = () => interrupt('SIGINT')
  process.on('SIGTERM', onTerm)
  process.on('SIGINT', onInt)
  let code = 1
  try {
    const result = await new Promise<{ code: number | null; signal: NodeJS.Signals | null; spawnError?: boolean }>(done => {
      child = spawn(command[0], command.slice(1), { cwd, stdio: 'inherit', detached: true })
      child.on('error', () => done({ code: null, signal: null, spawnError: true }))
      child.on('close', (code, signal) => done({ code, signal }))
    })
    code = requested ? (requested === 'SIGINT' ? 130 : 143) : result.code ?? (result.signal ? 143 : 1)
    receipt.outcome = requested || result.signal ? 'interrupted' : code === 0 ? 'success' : 'failure'
    receipt.exit_code = result.code
    receipt.signal = requested || result.signal
    receipt.spawn_error = result.spawnError === true
  } finally {
    if (timer) clearTimeout(timer)
    process.removeListener('SIGTERM', onTerm)
    process.removeListener('SIGINT', onInt)
    receipt.finished_at = new Date().toISOString()
    receipt.duration_seconds = (performance.now() - start) / 1000
    try {
      const after = snapshot(root)
      receipt.inputs_unchanged = before.sha === after.sha && before.fingerprint === after.fingerprint
      receipt.source_after = after
    } catch {
      receipt.inputs_unchanged = false
      receipt.source_after = null
    }
    atomicWrite(path, receipt)
    // The receipt contains identities, never command output, argv or environment values.
    if (env.GITHUB_STEP_SUMMARY) appendFileSync(env.GITHUB_STEP_SUMMARY,
      `\nVerification **${suite}**: ${receipt.outcome}, ${Number(receipt.duration_seconds).toFixed(2)}s. Observation only; receipt in the verification artifact.\n`)
    console.log(`Verification receipt: ${basename(path)}; ${receipt.outcome}; ${Number(receipt.duration_seconds).toFixed(2)}s`)
  }
  return code
}

if (process.argv[1] && resolve(process.argv[1]) === source) {
  const args = process.argv.slice(2)
  if (args[0] !== '--suite' || args[2] !== '--output' || args[4] !== '--' || args.length < 6) {
    throw new Error('Usage: receipt.ts --suite NAME --output EXTERNAL_DIRECTORY -- COMMAND [ARGS]')
  }
  process.exitCode = await observe(process.cwd(), args[1], args[3], args.slice(5))
}
