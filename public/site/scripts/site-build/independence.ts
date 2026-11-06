/** Clean source proof: Public installs/builds/tests/packages and serves dev without native toolchains. */
import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { chmodSync, cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { setTimeout as delay } from 'node:timers/promises'

const repo = resolve(import.meta.dirname, '../../../..')
const scratch = mkdtempSync(join(tmpdir(), 'public-independent-'))
const bin = join(scratch, 'deny'), root = join(scratch, 'source'), site = join(root, 'public/site')
let dev: ReturnType<typeof spawn> | undefined
try {
  mkdirSync(bin)
  const forbidden = ['cargo', 'rustc', 'rustup', 'python', 'python3']
  for (const command of forbidden) {
    const wrapper = join(bin, command)
    writeFileSync(wrapper, '#!/bin/sh\necho "Forbidden Public toolchain invocation" >&2\nexit 97\n')
    chmodSync(wrapper, 0o700)
  }
  // Copy only declared source inputs: no .git, native implementation, node_modules or build caches.
  const copy = (path: string) => {
    const source = join(repo, path), destination = join(root, path)
    mkdirSync(dirname(destination), { recursive: true })
    cpSync(source, destination, { recursive: true, filter: sourcePath => !['node_modules', 'dist', 'dist-ssr', 'dist-apparent', '__pycache__', '.git', 'target', '.build'].includes(sourcePath.split('/').at(-1)!) })
  }
  for (const path of ['public/site', 'public/delivery.json', 'composition/delivery', 'composition/architecture', '.github/actions/delivery-contracts', '.github/actions/delivery-component', '.github/workflows/stream-public.yml', '.github/actions/runtime-budget', 'enablement/verification/workflow-budget.ts', 'LICENSE', 'shared/ui', 'shared/analytics', 'shared/assets', 'README.md', 'capture/agent/skills/apparent/SKILL.md']) copy(path)
  const walk = (directory: string): string[] => readdirSync(directory, { withFileTypes: true }).flatMap(entry => entry.isDirectory() ? walk(join(directory, entry.name)) : [join(directory, entry.name)])
  assert.equal(walk(site).filter(path => /\.(rs|py|js|mjs|cjs|sh)$/.test(path)).length, 0, 'Public contains non-TypeScript owned source')
  const env = { ...process.env, PATH: `${bin}:${process.env.PATH}`, APPARENT_PUBLIC_SITE: '1' }
  const run = (command: string, args: string[]) => {
    const result = spawnSync(command, args, { cwd: site, env, stdio: 'inherit' })
    assert.ifError(result.error)
    assert.equal(result.status, 0, `${command} ${args.join(' ')} failed`)
  }
  for (const command of forbidden) assert.equal(spawnSync(command, ['--version'], { env, stdio: 'ignore' }).status, 97)
  const git = (args: string[]) => {
    const result = spawnSync('git', args, { cwd: root, env, stdio: 'inherit' })
    assert.ifError(result.error); assert.equal(result.status, 0)
  }
  // Artifact compatibility checks need a source identity, never application build output.
  git(['init', '--quiet'])
  git(['add', '.'])
  git(['-c', 'user.name=Public source fixture', '-c', 'user.email=public-fixture@example.invalid', '-c', 'core.hooksPath=/dev/null', '-c', 'commit.gpgsign=false', 'commit', '--quiet', '-m', 'isolated source fixture'])
  run('node', ['scripts/site-build/verify.ts'])
  assert.ok(existsSync(join(site, 'dist-apparent/index.html')))
  assert.match(readFileSync(join(site, 'dist-apparent/index.html'), 'utf8'), /Tell your coding agent what to change/)
  // Port 0 avoids contention with another owner. Vite prints the allocated origin.
  dev = spawn('npm', ['run', 'dev', '--', '--host', '127.0.0.1', '--port', '0'], { cwd: site, env, stdio: ['ignore', 'pipe', 'pipe'], detached: true })
  let output = ''
  dev.stdout!.on('data', data => { output += data.toString() })
  dev.stderr!.on('data', data => { output += data.toString() })
  const deadline = Date.now() + 20_000
  let origin: string | undefined
  while (Date.now() < deadline && dev.exitCode === null) {
    origin = output.match(/http:\/\/127\.0\.0\.1:\d+/)?.[0]
    if (origin) break
    await delay(50)
  }
  assert.ok(origin, `Public dev server did not start: ${output}`)
  assert.equal((await fetch(origin, { signal: AbortSignal.timeout(5_000) })).status, 200)
  const module = await fetch(`${origin}/src/main.tsx`, { signal: AbortSignal.timeout(5_000) })
  assert.equal(module.status, 200)
  assert.match(await module.text(), /createRoot/)
  console.log('Public clean-source install, build, owner tests, package and dev proof passed; Cargo/Rust/Python denied.')
} finally {
  if (dev?.pid) {
    try { process.kill(-dev.pid, 'SIGTERM') } catch (error) { if ((error as NodeJS.ErrnoException).code !== 'ESRCH') throw error }
    if (dev.exitCode === null) await new Promise<void>(done => dev!.once('exit', () => done()))
  }
  rmSync(scratch, { recursive: true, force: true })
}
