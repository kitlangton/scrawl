import assert from 'node:assert/strict'
import { test } from 'node:test'
import type { SpawnSyncReturns } from 'node:child_process'
import { preparePlaywrightRuntime } from './playwright-runtime.ts'

const result = (status: number, stderr = '', error?: Error) => ({ status, stderr: Buffer.from(stderr), stdout: Buffer.alloc(0), error } as SpawnSyncReturns<Buffer>)

test('Playwright readiness probes both browsers and skips apt dependencies when launches work', () => {
  const calls: string[] = []
  preparePlaywrightRuntime((command, args) => {
    calls.push(`${command} ${args.join(' ')}`)
    return result(0)
  })
  assert.deepEqual(calls.map(call => call.split(' ')[0]), ['./node_modules/.bin/playwright-core', 'node'])
  assert.match(calls[1], /chromium, firefox/)
})

test('Playwright installs system dependencies only after a missing dependency and re-proves launch', () => {
  const calls: string[] = []
  preparePlaywrightRuntime((command, args) => {
    calls.push(`${command} ${args.join(' ')}`)
    if (command === 'node' && calls.filter(call => call.startsWith('node ')).length === 1)
      return result(1, 'Host system is missing dependencies to run browsers')
    return result(0)
  })
  assert.equal(calls.filter(call => call.includes('install-deps')).length, 1)
  assert.match(calls.at(-1)!, /^node /)
})

test('Playwright fails closed on unknown launch errors and unresolved dependencies', () => {
  let calls = 0
  assert.throws(() => preparePlaywrightRuntime((command) => {
    calls++
    return command === 'node' ? result(1, 'browser crashed unexpectedly') : result(0)
  }), /other than a missing system dependency/)
  assert.equal(calls, 2)

  const unresolved: string[] = []
  assert.throws(() => preparePlaywrightRuntime((command, args) => {
    unresolved.push(`${command} ${args.join(' ')}`)
    if (command === 'node') return result(1, 'error while loading shared libraries: libmissing.so')
    if (args[0] === 'install-deps') return result(0)
    return result(0)
  }), /Playwright browser launch after dependency install failed/)
  assert.equal(unresolved.filter(call => call.startsWith('node ')).length, 2)
})
