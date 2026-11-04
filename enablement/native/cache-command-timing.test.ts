import { test } from 'node:test'
import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync, symlinkSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { spawn, spawnSync } from 'node:child_process'
import { once } from 'node:events'

const wrapper = resolve('enablement/native/cache-command-timing.ts')
test('command streams and status survive timing and failed summary writes', () => {
  const root = mkdtempSync(join(tmpdir(), 'cache-timing-'))
  try {
    for (const summary of [join(root, 'summary'), root]) {
      const result = spawnSync(process.execPath, ['--experimental-strip-types', wrapper, process.execPath, '-e',
        'process.stdout.write("{\\"ok\\":true}");process.stderr.write("diagnostic\\n");process.exitCode=7'],
      { encoding: 'utf8', timeout: 5000, env: { ...process.env, GITHUB_STEP_SUMMARY: summary } })
      assert.equal(result.status, 7); assert.equal(result.stdout, '{"ok":true}'); assert.equal(result.stderr, 'diagnostic\n')
    }
    const events = readFileSync(join(root, 'summary'), 'utf8')
    assert.match(events, /command; start; UTC .*elapsed \d+ ms; running/)
    assert.match(events, /command; end; UTC .*elapsed \d+ ms; exit 7/)
    assert.ok(events.indexOf('start') < events.indexOf('end'))
    assert.doesNotMatch(events, /diagnostic|process\.stdout/)
    const missing = spawnSync(process.execPath, ['--experimental-strip-types', wrapper, join(root, 'absent')], { timeout: 5000 })
    assert.equal(missing.status, 1)
    const signal = spawnSync(process.execPath, ['--experimental-strip-types', wrapper, process.execPath, '-e', 'process.kill(process.pid,"SIGTERM")'], { timeout: 5000 })
    assert.equal(signal.signal, 'SIGTERM')
  } finally { rmSync(root, { recursive: true, force: true }) }
})
test('external cancellation forwards SIGTERM and reaps the direct child', { timeout: 10000 }, async () => {
  const child = spawn(process.execPath, ['--experimental-strip-types', wrapper, process.execPath, '-e', 'console.log(process.pid);setInterval(()=>{},1000)'], { stdio: ['ignore', 'pipe', 'pipe'] })
  let pid: number | undefined
  const closed = once(child, 'close')
  try {
    const [data] = await once(child.stdout!, 'data')
    pid = Number(data.toString().trim()); assert.ok(pid > 0)
    child.kill('SIGTERM')
    const [code, signal] = await closed
    assert.equal(code, null); assert.equal(signal, 'SIGTERM')
    assert.throws(() => process.kill(pid!, 0), { code: 'ESRCH' })
  } finally {
    if (child.exitCode === null && child.signalCode === null) child.kill('SIGKILL')
    if (pid) { try { process.kill(pid, 'SIGKILL') } catch {} }
  }
})
test('workflow format retains action placeholder and executes from a spaced workspace', () => {
  const root = mkdtempSync(join(tmpdir(), 'cache format '))
  try {
    symlinkSync(resolve('enablement'), join(root, 'enablement'), 'dir')
    const workflow = readFileSync('.github/workflows/verification-platform.yml', 'utf8')
    const line = workflow.split('\n').find(value => value.includes('cmd-format:'))!
    assert.match(line, /runner.os == 'macOS'/); assert.ok(line.endsWith("|| '{0}' }}"))
    const template = line.match(/format\('([^']+)', github.workspace\)/)![1]
    // GitHub format escapes doubled braces before substituting the workspace.
    const format = template.replace('{{0}}', '\u0000').replace('{0}', root).replace('\u0000', '{0}')
    assert.equal((format.match(/\{0\}/g) ?? []).length, 1)
    const command = format.replace('{0}', `${process.execPath} -p 'JSON.stringify({ok:true})'`)
    const result = spawnSync('bash', ['-c', command], { encoding: 'utf8', timeout: 5000, env: { ...process.env, GITHUB_STEP_SUMMARY: join(root, 'summary') } })
    assert.equal(result.status, 0, result.stderr); assert.deepEqual(JSON.parse(result.stdout), { ok: true })
  } finally { rmSync(root, { recursive: true, force: true }) }
})
