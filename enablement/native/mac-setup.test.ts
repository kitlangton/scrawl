import { test } from 'node:test'
import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { prepareMacDependencies } from './mac-setup.ts'

const helper = resolve('enablement/native/mac-setup.ts')
const workflow = readFileSync('.github/workflows/verification-platform.yml', 'utf8')
const helperSource = readFileSync(helper, 'utf8')

function fixture() {
  const root = mkdtempSync(join(tmpdir(), 'apparent-mac-setup-'))
  const bin = join(root, 'bin')
  mkdirSync(bin)
  symlinkBash(bin)
  function command(name: string, body: string) {
    const path = join(bin, name)
    writeFileSync(path, `#!/bin/sh\n${body}\n`)
    chmodSync(path, 0o755)
  }
  function env() { return { ...process.env, PATH: bin } }
  return { root, bin, command, env, cleanup: () => rmSync(root, { recursive: true, force: true }) }
}
function symlinkBash(bin: string) {
  // Use a fixed system executable while keeping command lookup isolated to fixtures.
  const bash = '/bin/bash'
  writeFileSync(join(bin, 'bash'), `#!/bin/sh\nexec ${bash} "$@"\n`)
  chmodSync(join(bin, 'bash'), 0o755)
}
function prerequisites(f: ReturnType<typeof fixture>, ffmpeg = 'exit 0', metal = 'exit 0') {
  f.command('xcrun', metal)
  f.command('brew', ffmpeg); f.command('xcodebuild', 'exit 0')
}
function run(f: ReturnType<typeof fixture>, timeoutMs = 120_000) {
  const oldPath = process.env.PATH
  process.env.PATH = f.bin
  return prepareMacDependencies(timeoutMs).finally(() => { process.env.PATH = oldPath })
}
function cli(f: ReturnType<typeof fixture>, ...args: string[]) {
  return spawn(process.execPath, ['--experimental-strip-types', helper, ...args], { env: f.env(), stdio: 'ignore' })
}
function status(path: string) { return JSON.parse(readFileSync(path, 'utf8')) as Record<string, unknown> }
function waitExit(child: ReturnType<typeof cli>) { return new Promise<number | null>((resolveDone) => child.once('exit', resolveDone)) }
function workflowRun(name: string) {
  const escaped = name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')
  const match = workflow.match(new RegExp(`- name: ${escaped}\\n[\\s\\S]*?\\n        run: \\|\\n([\\s\\S]*?)(?=\\n      - )`))
  assert.ok(match, `workflow step ${name} has a run block`)
  return match[1].split('\n').map((line) => line.replace(/^          /, '')).join('\n')
}
async function waitFor(path: string) {
  const deadline = Date.now() + 5_000
  while (!existsSync(path) && Date.now() < deadline) await new Promise((resolveDone) => setTimeout(resolveDone, 10))
  assert.ok(existsSync(path), `expected ${path} to appear`)
}
async function assertStopped(pidFile: string) {
  const pid = Number(readFileSync(pidFile, 'utf8').trim())
  for (let attempt = 0; attempt < 100; attempt++) {
    const probe = spawnSync('/bin/kill', ['-0', String(pid)])
    if (probe.status !== 0) return
    const state = spawnSync('/bin/ps', ['-p', String(pid), '-o', 'stat='], { encoding: 'utf8' })
    if (state.status === 1 && !state.stdout.trim()) return
    assert.equal(state.status, 0, state.stderr)
    if (state.stdout.trim().startsWith('Z')) return
    await new Promise((resolveDone) => setTimeout(resolveDone, 10))
  }
  assert.fail(`descendant ${pid} is still running`)
}

test('Mac workflow orders Node, Xcode selection, dependency helper, and Rust/cache; keeps Linux selection', () => {
  const rust = workflow.slice(workflow.indexOf('\n  rust:'))
  const node = rust.indexOf('uses: actions/setup-node@v4')
  const selector = rust.indexOf('uses: maxim-lobanov/setup-xcode@v1')
  const developer = rust.indexOf('name: Pin the selected developer directory for Rust verification')
  const helperAt = rust.indexOf('node --experimental-strip-types enablement/native/mac-setup.ts \\\n            --prepare')
  const awaitAt = rust.indexOf('--await "$RUNNER_TEMP/mac-compiler-setup/status.json"')
  const cache = rust.indexOf('uses: Swatinem/rust-cache@v2')
  const rustToolchain = rust.indexOf('uses: dtolnay/rust-toolchain@stable')
  assert.ok(node >= 0 && node < helperAt && selector >= 0 && developer > selector && helperAt > developer)
  assert.ok(cache > helperAt && rustToolchain > helperAt && awaitAt > cache)
  assert.match(rust, /name: Prepare macOS compiler dependencies concurrently[\s\S]*?if: runner\.os == 'macOS' && needs\.select\.outputs\.root == 'true'/)
  assert.match(rust, /name: Require macOS compiler dependencies before Cargo[\s\S]*?--await/)
  assert.match(rust, /if: runner\.os == 'Linux' && needs\.select\.outputs\.root == 'true'[\s\S]*?dependencies\.sh/)
  assert.match(rust, /toolchain: \$\{\{ runner\.os == 'macOS' && '1\.99\.1' \|\| 'stable' \}\}/)
  assert.equal((rust.match(/actions\/setup-node@v4/g) ?? []).length, 1)
  for (const exact of ['command -v ffmpeg && command -v ffprobe || brew install ffmpeg', 'xcrun --sdk macosx metal --version || xcodebuild -downloadComponent MetalToolchain']) assert.ok(helperSource.includes(exact))
})

test('both dependency commands overlap and succeed before returning', async () => {
  const f = fixture()
  const marker = join(f.root, 'metal-started')
  try {
    const ffmpegReady = join(f.root, 'ffmpeg-ready')
    prerequisites(f,
      `: > '${ffmpegReady}'; while [ ! -e '${marker}' ]; do /bin/sleep 0.01; done; exit 0`,
      `: > '${marker}'; while [ ! -e '${ffmpegReady}' ]; do /bin/sleep 0.01; done; exit 0`)
    await run(f, 2_000)
    assert.ok(existsSync(marker), 'the FFmpeg command observed Metal start before it completed')
  } finally { f.cleanup() }
})

test('either failure stops its sibling and reaps an owned descendant', async (t) => {
  for (const failed of ['ffmpeg', 'metal']) await t.test(failed, async () => {
    const f = fixture(); const pidFile = join(f.root, 'descendant.pid')
    try {
      const failing = `while [ ! -s '${pidFile}' ]; do /bin/sleep 0.01; done; exit 9`
      const sibling = `/bin/sleep 30 & echo $! > '${pidFile}'; wait`
      prerequisites(f, failed === 'ffmpeg' ? failing : sibling, failed === 'metal' ? failing : sibling)
      if (failed === 'metal') f.command('xcodebuild', 'exit 8')
      await assert.rejects(run(f, 2_000))
      await assertStopped(pidFile)
    } finally { f.cleanup() }
  })
})

test('timeout terminates both process groups and returns within the bounded timeout', async () => {
  const f = fixture()
  const pidFile = join(f.root, 'descendant.pid')
  try {
    prerequisites(f, `/bin/sleep 30 & echo $! > '${pidFile}'; wait`, 'while :; do :; done')
    await assert.rejects(run(f, 150), /timed out/)
    await assertStopped(pidFile)
  } finally { f.cleanup() }
})

test('external cancellation terminates and reaps command descendants', async () => {
  const f = fixture(); const pidFile = join(f.root, 'descendant.pid')
  let child: ReturnType<typeof cli> | undefined
  try {
    prerequisites(f, `/bin/sleep 30 & echo $! > '${pidFile}'; wait`, 'while :; do :; done')
    child = cli(f)
    const childExit = new Promise<number | null>((resolveDone) => child!.once('exit', resolveDone))
    await new Promise<void>((resolveDone, reject) => {
      const deadline = Date.now() + 5_000
      const poll = setInterval(() => {
        if (existsSync(pidFile)) { clearInterval(poll); child.kill('SIGTERM'); resolveDone() }
        else if (Date.now() > deadline) { clearInterval(poll); child.kill('SIGKILL'); reject(new Error('fixture did not start')) }
      }, 10)
    })
    const code = await childExit
    assert.notEqual(code, 0)
    await assertStopped(pidFile)
  } finally {
    if (child && child.exitCode === null) child.kill('SIGKILL')
    f.cleanup()
  }
})

test('successful commands cannot leave owned descendants running', async () => {
  const f = fixture(); const pidFile = join(f.root, 'descendant.pid')
  try {
    prerequisites(f, `/bin/sleep 30 & echo $! > '${pidFile}'; exit 0`, 'exit 0')
    await run(f)
    await assertStopped(pidFile)
  } finally { f.cleanup() }
})

test('spawn failure is reported and fails the helper', async () => {
  const f = fixture()
  try {
    rmSync(join(f.bin, 'bash'))
    await assert.rejects(run(f, 1_000), /spawn failed/)
  } finally { f.cleanup() }
})

test('prepare and await use separate processes and gate on both overlapping commands', async () => {
  const f = fixture(); const statusPath = join(f.root, 'status.json')
  const marker = join(f.root, 'metal-started'); const ffmpegReady = join(f.root, 'ffmpeg-ready')
  let worker: ReturnType<typeof cli> | undefined; let gate: ReturnType<typeof cli> | undefined
  try {
    prerequisites(f,
      `: > '${ffmpegReady}'; while [ ! -e '${marker}' ]; do /bin/sleep 0.01; done; exit 0`,
      `: > '${marker}'; while [ ! -e '${ffmpegReady}' ]; do /bin/sleep 0.01; done; exit 0`)
    worker = cli(f, '--prepare', statusPath)
    await waitFor(statusPath)
    gate = cli(f, '--await', statusPath)
    assert.equal(await waitExit(worker), 0)
    assert.equal(await waitExit(gate), 0)
    assert.equal(status(statusPath).state, 'complete')
    assert.equal(status(statusPath).exit, 0)
    assert.ok(existsSync(marker))
  } finally {
    if (worker?.exitCode === null) worker.kill('SIGKILL')
    if (gate?.exitCode === null) gate.kill('SIGKILL')
    f.cleanup()
  }
})

test('prepare rejects preexisting status and await rejects a symlink', async () => {
  const f = fixture(); const statusPath = join(f.root, 'status.json')
  try {
    writeFileSync(statusPath, '{}', { mode: 0o600 })
    assert.notEqual(await waitExit(cli(f, '--prepare', statusPath)), 0)
    rmSync(statusPath)
    const target = join(f.root, 'target'); writeFileSync(target, '{}', { mode: 0o600 })
    // Symlink creation is deliberately local to the isolated fixture.
    const { symlinkSync } = await import('node:fs')
    symlinkSync(target, statusPath)
    assert.notEqual(await waitExit(cli(f, '--await', statusPath)), 0)
  } finally { f.cleanup() }
})

test('await rejects malformed, stale, and failed status without accepting them as success', async (t) => {
  for (const scenario of ['malformed', 'stale', 'future', 'extra-field', 'failure']) await t.test(scenario, async () => {
    const f = fixture(); const statusPath = join(f.root, 'status.json'); const now = Date.now()
    try {
      const value = scenario === 'malformed' ? '{broken' : scenario === 'stale'
        ? JSON.stringify({ state: 'pending', nonce: '00000000-0000-4000-8000-000000000000', startedAt: now - 121_000, deadlineAt: now - 1_000 })
        : scenario === 'future'
          ? JSON.stringify({ state: 'pending', nonce: '00000000-0000-4000-8000-000000000000', startedAt: now + 10_000, deadlineAt: now + 20_000 })
          : scenario === 'extra-field'
            ? JSON.stringify({ state: 'pending', nonce: '00000000-0000-4000-8000-000000000000', startedAt: now, deadlineAt: now + 120_000, arbitrary: true })
        : JSON.stringify({ state: 'complete', nonce: '00000000-0000-4000-8000-000000000000', startedAt: now - 100, deadlineAt: now + 119_900, finishedAt: now, exit: 1 })
      writeFileSync(statusPath, value, { mode: 0o600 })
      assert.notEqual(await waitExit(cli(f, '--await', statusPath)), 0)
    } finally { f.cleanup() }
  })
})

test('await honors the worker deadline and accepts completed success after that deadline', async (t) => {
  await t.test('pending worker that misses its original deadline fails', async () => {
    const f = fixture(); const statusPath = join(f.root, 'status.json'); const now = Date.now()
    try {
      writeFileSync(statusPath, JSON.stringify({ state: 'pending', nonce: '00000000-0000-4000-8000-000000000000', startedAt: now - 119_900, deadlineAt: now + 100 }), { mode: 0o600 })
      const started = Date.now()
      assert.notEqual(await waitExit(cli(f, '--await', statusPath)), 0)
      assert.ok(Date.now() - started < 1_000, 'await is bounded by the recorded original deadline')
    } finally { f.cleanup() }
  })
  await t.test('completed success is usable after cache restore delay', async () => {
    const f = fixture(); const statusPath = join(f.root, 'status.json'); const now = Date.now()
    try {
      const deadlineAt = now - 5_000
      writeFileSync(statusPath, JSON.stringify({ state: 'complete', nonce: '00000000-0000-4000-8000-000000000000', startedAt: deadlineAt - 120_000, deadlineAt, finishedAt: deadlineAt - 1, exit: 0 }), { mode: 0o600 })
      assert.equal(await waitExit(cli(f, '--await', statusPath)), 0)
    } finally { f.cleanup() }
  })
})

test('worker SIGTERM reaps process groups before publishing terminal failure status', async () => {
  const f = fixture(); const statusPath = join(f.root, 'status.json'); const pidFile = join(f.root, 'descendant.pid')
  let worker: ReturnType<typeof cli> | undefined
  try {
    prerequisites(f, `/bin/sleep 30 & echo $! > '${pidFile}'; wait`, 'while :; do :; done')
    worker = cli(f, '--prepare', statusPath)
    await waitFor(pidFile)
    worker.kill('SIGTERM')
    assert.notEqual(await waitExit(worker), 0)
    await assertStopped(pidFile)
    const final = status(statusPath)
    assert.equal(final.state, 'complete')
    assert.equal(final.exit, 1)
    assert.ok(Number(final.finishedAt) >= Number(final.startedAt))
  } finally {
    if (worker?.exitCode === null) worker.kill('SIGKILL')
    f.cleanup()
  }
})

test('actual workflow shell backgrounds preparation through cache and gates the Cargo marker', async (t) => {
  for (const outcome of ['success', 'failure']) await t.test(outcome, async () => {
    const f = fixture(); const cacheMarker = join(f.root, 'cache-restored'); const cargoMarker = join(f.root, 'cargo-ran')
    const env = { ...f.env(), RUNNER_TEMP: f.root }
    f.command('mkdir', 'exec /bin/mkdir "$@"')
    f.command('cat', 'exec /bin/cat "$@"')
    f.command('node', `exec '${process.execPath}' "$@"`)
    const failed = outcome === 'failure'
    prerequisites(f,
      `while [ ! -e '${cacheMarker}' ]; do /bin/sleep 0.01; done; exit ${failed ? 7 : 0}`,
      `while [ ! -e '${cacheMarker}' ]; do /bin/sleep 0.01; done; exit 0`)
    const start = workflowRun('Prepare macOS compiler dependencies concurrently')
    const gate = `${workflowRun('Require macOS compiler dependencies before Cargo')}\n/bin/touch '${cargoMarker}'`
    try {
      const started = spawnSync('/bin/bash', ['-c', start], { env, encoding: 'utf8' })
      assert.equal(started.status, 0, started.stderr)
      const statusPath = join(f.root, 'mac-compiler-setup', 'status.json')
      await waitFor(statusPath)
      assert.equal(status(statusPath).state, 'pending', 'workflow start returns while preparation is blocked on the cache marker')
      writeFileSync(cacheMarker, 'restored')
      const gated = spawnSync('/bin/bash', ['-c', gate], { env, encoding: 'utf8' })
      assert.equal(gated.status, failed ? 1 : 0, gated.stderr)
      assert.equal(existsSync(cargoMarker), !failed, 'Cargo is reached only after a successful dependency gate')
      assert.equal(status(statusPath).exit, failed ? 1 : 0)
    } finally { f.cleanup() }
  })
})
