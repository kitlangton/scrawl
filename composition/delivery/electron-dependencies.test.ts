import assert from 'node:assert/strict'
import { mkdtempSync, readFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'
import { installElectronDependencies } from './runtime/electron-dependencies.ts'

test('Electron uses its locked Playwright dependency closure through the bounded signed Ubuntu installer', t => {
  const directory = mkdtempSync(join(tmpdir(), 'electron-apt-contract-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  let calls = 0
  assert.throws(() => installElectronDependencies({ directory, environment: { GITHUB_ACTIONS: 'true', RUNNER_OS: 'Linux', ImageOS: 'ubuntu24' },
    osRelease: 'ID=ubuntu\nVERSION_CODENAME=noble\n', root: false,
    run(command, args, options) {
      calls++
      assert.equal(command, 'sudo')
      assert(args.includes('240s'))
      assert.equal(options.timeout, 255_000)
      assert.deepEqual(args.slice(-4), [process.execPath, new URL('../app/node_modules/playwright/cli.js', import.meta.url).pathname, 'install-deps', 'chromium'])
      const config = readFileSync(join(directory, 'apt.conf'), 'utf8')
      assert.match(config, /Acquire::https::Timeout "20"/)
      assert.match(config, /Acquire::Retries "1"/)
      assert.match(readFileSync(join(directory, 'ubuntu.sources'), 'utf8'), /https:\/\/archive.ubuntu.com/)
      throw new Error('synthetic apt failure')
    } }), /synthetic apt failure/)
  assert.equal(calls, 1)
})
