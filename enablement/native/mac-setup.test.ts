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
function cli(f: ReturnType<typeof fixture>) {
  return spawn(process.execPath, ['--experimental-strip-types', helper], { env: f.env(), stdio: 'ignore' })
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
  const developer = rust.indexOf('name: Pin the selected developer directory')
  const helperAt = rust.indexOf('node --experimental-strip-types enablement/native/mac-setup.ts')
  const cache = rust.indexOf('uses: Swatinem/rust-cache@v2')
  const rustToolchain = rust.indexOf('uses: dtolnay/rust-toolchain@stable')
  assert.ok(node >= 0 && node < helperAt && selector >= 0 && developer > selector && helperAt > developer)
  assert.ok(cache > helperAt && rustToolchain > helperAt)
  assert.match(rust, /name: Prepare macOS compiler dependencies concurrently[\s\S]*?if: runner\.os == 'macOS' && needs\.select\.outputs\.root == 'true'/)
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
