import { spawn } from 'node:child_process'
import { appendFileSync } from 'node:fs'
import { basename } from 'node:path'
import { performance } from 'node:perf_hooks'

const command = process.argv.slice(2)
if (!command.length) process.exit(2)
const executable = basename(command[0])
const label = executable === 'cargo' && command[1] === 'metadata' ? 'cargo metadata'
  : ['cargo', 'rustc', 'rustup'].includes(executable) ? 'toolchain' : 'command'
const tick = performance.now()
function record(event: string, status: string) {
  const summary = process.env.GITHUB_STEP_SUMMARY
  if (!summary) return
  try {
    appendFileSync(summary, `\nCache command: ${label}; ${event}; UTC ${new Date().toISOString()}; elapsed ${Math.round(performance.now() - tick)} ms; ${status}\n`, { mode: 0o600 })
  } catch {
    // Observation must not change the underlying command result.
  }
}
record('start', 'running')
const child = spawn(command[0], command.slice(1), { stdio: 'inherit' })
const signals = ['SIGINT', 'SIGTERM', 'SIGHUP'] as const
const handlers = signals.map(signal => {
  const handler = () => { child.kill(signal) }
  process.on(signal, handler)
  return handler
})
let spawnFailed = false
child.once('error', () => { spawnFailed = true })
child.once('close', (code, signal) => {
  for (const [index, value] of signals.entries()) process.removeListener(value, handlers[index])
  record('end', spawnFailed ? 'spawn failure' : signal ? `signal ${signal}` : `exit ${code ?? 1}`)
  if (signal) process.kill(process.pid, signal)
  else process.exitCode = spawnFailed ? 1 : code ?? 1
})
