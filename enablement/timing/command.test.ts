import { test } from 'node:test'
import { strict as assert } from 'node:assert'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { spawnSync } from 'node:child_process'

for (const scenario of ['success', 'failure', 'signal', 'spawn-failure', 'unwritable']) {
  test(scenario, () => {
    const dir = mkdtempSync(join(tmpdir(), 'apparent-dev-timing-test-'))
    try {
      const path = join(dir, 'timing.jsonl')
      const command = scenario === 'spawn-failure' ? ['/nonexistent-synthetic-command']
        : [process.execPath, '-e', scenario === 'signal' ? 'process.kill(process.pid,"SIGTERM")'
          : `process.exit(${scenario === 'failure' ? 7 : 0})`]
      const result = spawnSync(process.execPath, ['--experimental-strip-types',
        'enablement/timing/command.ts', 'synthetic', ...command], {
        env: { ...process.env, APPARENT_TIMING_FILE: scenario === 'unwritable' ? dir : path },
      })
      if (scenario === 'signal') assert.equal(result.signal, 'SIGTERM')
      else assert.equal(result.status, scenario === 'failure' ? 7 : scenario === 'spawn-failure' ? 1 : 0)
      if (scenario !== 'unwritable') {
        const rows = readFileSync(path, 'utf8').trim().split('\n').map(line => JSON.parse(line))
        assert.equal(rows.length, 2)
        assert.equal(rows[0].state, 'started')
        assert.equal(rows[1].state, scenario === 'spawn-failure' ? 'spawn-failed' : 'finished')
        assert.ok(rows[1].elapsed_ms >= 0)
        assert.ok(!readFileSync(path, 'utf8').includes(command.at(-1)!))
      }
    } finally { rmSync(dir, { recursive: true, force: true }) }
  })
}

test('forwards TERM received by wrapper and retains final timing', async () => {
  const { spawn } = await import('node:child_process')
  const dir = mkdtempSync(join(tmpdir(), 'apparent-dev-timing-signal-'))
  try {
    const path = join(dir, 'timing.jsonl')
    const child = spawn(process.execPath, ['--experimental-strip-types',
      'enablement/timing/command.ts', 'synthetic-signal', process.execPath, '-e',
      'console.log("ready"); setInterval(()=>{},1000)'], {
      env: { ...process.env, APPARENT_TIMING_FILE: path }, stdio: ['ignore', 'pipe', 'ignore'],
    })
    const outcome = new Promise(resolve => child.once('close', (code, signal) => resolve({ code, signal })))
    await new Promise<void>((resolve, reject) => {
      child.stdout.once('data', () => resolve())
      child.once('error', reject)
    })
    child.kill('SIGTERM')
    assert.deepEqual(await outcome, { code: null, signal: 'SIGTERM' })
    const rows = readFileSync(path, 'utf8').trim().split('\n').map(line => JSON.parse(line))
    assert.equal(rows.at(-1).signal, 'SIGTERM')
  } finally { rmSync(dir, { recursive: true, force: true }) }
})
