import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import { test } from 'node:test'
import { observe, snapshot } from './receipt.ts'

const runner = join(dirname(fileURLToPath(import.meta.url)), 'receipt.ts')
function fixture(t: { after: (fn: () => void) => void }) {
  const directory = mkdtempSync(join(tmpdir(), 'apparent-receipt-test-'))
  const root = join(directory, 'repo')
  const output = join(directory, 'receipts')
  const init = spawnSync('git', ['init', '-q', root]); assert.equal(init.status, 0)
  const git = (...args: string[]) => {
    const result = spawnSync('git', ['-c', 'core.hooksPath=/dev/null', '-c', 'commit.gpgsign=false', ...args], { cwd: root })
    assert.equal(result.status, 0, result.stderr?.toString())
  }
  git('config', 'user.name', 'Receipt fixture'); git('config', 'user.email', 'fixture@example.invalid')
  writeFileSync(join(root, 'source'), 'one')
  git('add', 'source'); git('commit', '-qm', 'fixture')
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  const receipt = () => JSON.parse(readFileSync(join(output, readdirSync(output).find(p => p.endsWith('.json'))!), 'utf8'))
  return { root, output, receipt }
}

test('source fingerprint covers working bytes, additions and mode changes, not just HEAD', t => {
  const { root } = fixture(t)
  const before = snapshot(root)
  assert.equal(before.clean, true)
  writeFileSync(join(root, 'source'), 'two')
  const changed = snapshot(root)
  assert.equal(changed.sha, before.sha)
  assert.notEqual(changed.fingerprint, before.fingerprint)
  chmodSync(join(root, 'source'), 0o755)
  assert.notEqual(snapshot(root).fingerprint, changed.fingerprint)
  writeFileSync(join(root, 'consumer'), 'new consumer')
  assert.notEqual(snapshot(root).fingerprint, changed.fingerprint)
})

test('success records timing and identity without leaking command arguments or granting reuse', async t => {
  const { root, output, receipt } = fixture(t)
  const privateValue = 'synthetic-private-argument-do-not-record'
  assert.equal(await observe(root, 'fixture', output, [process.execPath, '-e', 'process.exit(0)', privateValue]), 0)
  const value = receipt()
  assert.equal(value.outcome, 'success')
  assert.equal(value.reusable, false)
  assert.equal(value.inputs_unchanged, true)
  assert.match(value.source.sha, /^[0-9a-f]{40}$/)
  assert.match(value.verifier, /^[0-9a-f]{64}$/)
  assert.ok(value.duration_seconds >= 0)
  assert.ok(Date.parse(value.finished_at) >= Date.parse(value.started_at))
  assert.equal(JSON.stringify(value).includes(privateValue), false)
})

test('failed and unspawnable commands cannot produce successful receipts', async t => {
  for (const [program, args, status] of [[process.execPath, ['-e', 'process.exit(7)'], 7], ['/nonexistent/apparent-command', [], 1]] as const) {
    const { root, output, receipt } = fixture(t)
    assert.equal(await observe(root, 'failure', output, [program, ...args]), status)
    assert.equal(receipt().outcome, 'failure')
  }
})

test('source mutation during a passing command is explicitly recorded', async t => {
  const { root, output, receipt } = fixture(t)
  assert.equal(await observe(root, 'mutation', output, [process.execPath, '-e', "require('fs').writeFileSync('source', 'changed')"]), 0)
  assert.equal(receipt().inputs_unchanged, false)
  assert.equal(receipt().reusable, false)
})

test('receipt output inside checkout is refused before executing the command', async t => {
  const { root } = fixture(t)
  await assert.rejects(observe(root, 'bad', join(root, 'output'), [process.execPath, '-e', 'process.exit(0)']), /outside the checkout/)
  assert.equal(existsSync(join(root, 'output')), false)
  const link = join(dirname(root), 'external-link')
  symlinkSync(root, link, 'dir')
  await assert.rejects(observe(root, 'bad', join(link, 'new', 'output'), [process.execPath, '-e', 'process.exit(0)']), /outside the checkout/)
  assert.equal(existsSync(join(root, 'new')), false)
})

test('owner working directories retain full-repository source identity', async t => {
  const { root, output, receipt } = fixture(t)
  const nested = join(root, 'owner')
  mkdirSync(nested)
  const before = snapshot(root)
  assert.equal(await observe(nested, 'owner', output, [process.execPath, '-e', "if(!process.cwd().endsWith('owner'))process.exit(1)"]), 0)
  assert.equal(receipt().command.cwd, 'owner')
  assert.equal(receipt().source.fingerprint, before.fingerprint)
})

test('termination is forwarded and recorded as interrupted with a failing exit', async t => {
  const { root, output, receipt } = fixture(t)
  const child = spawn(process.execPath, [runner, '--suite', 'interrupted', '--output', output, '--',
    process.execPath, '-e', "console.log('ready');setInterval(()=>{},1000)"], { cwd: root, stdio: ['ignore', 'pipe', 'pipe'] })
  const completed = new Promise<number | null>(done => child.on('close', done))
  const timeout = setTimeout(() => child.kill('SIGTERM'), 10000)
  t.after(() => { clearTimeout(timeout); child.kill('SIGKILL') })
  await new Promise<void>((done, reject) => {
    child.stdout.on('data', data => {
      if (data.toString().includes('ready')) {
        assert.equal(receipt().outcome, 'in_progress')
        child.kill('SIGTERM'); done()
      }
    })
    child.on('error', reject)
    child.on('close', code => { if (code !== 143) reject(new Error(`Unexpected early exit ${code}`)) })
  })
  assert.equal(await completed, 143)
  assert.equal(receipt().outcome, 'interrupted')
  assert.equal(receipt().signal, 'SIGTERM')
  assert.equal(receipt().reusable, false)
})
