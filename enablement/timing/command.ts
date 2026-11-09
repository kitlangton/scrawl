/** Observation only: never a test receipt or an execution selector. */
import { spawn } from 'node:child_process'
import { appendFileSync } from 'node:fs'
import { performance } from 'node:perf_hooks'

const [label, ...command] = process.argv.slice(2)
if (!label || !/^[a-z0-9-]{1,64}$/.test(label) || !command.length) process.exit(2)
const started = new Date().toISOString()
const tick = performance.now()
function record(state: string, code: number | null = null, signal: string | null = null) {
  const path = process.env.APPARENT_TIMING_FILE
  if (!path) return
  try {
    appendFileSync(path, JSON.stringify({ schema: 1, label, state, started,
      elapsed_ms: Math.round(performance.now() - tick), code, signal,
      sha: process.env.GITHUB_SHA ?? null }) + '\n', { mode: 0o600 })
  } catch { /* Diagnostics must preserve the command result. */ }
}
record('started')
const child = spawn(command[0], command.slice(1), { stdio: 'inherit' })
const signals = ['SIGINT', 'SIGTERM', 'SIGHUP'] as const
const handlers = signals.map(signal => {
  const handler = () => { child.kill(signal) }
  process.on(signal, handler)
  return handler
})
let failed = false
child.once('error', () => { failed = true })
child.once('close', (code, signal) => {
  signals.forEach((value, index) => process.removeListener(value, handlers[index]))
  record(failed ? 'spawn-failed' : 'finished', code, signal)
  if (signal) process.kill(process.pid, signal)
  else process.exitCode = failed ? 1 : code ?? 1
})
