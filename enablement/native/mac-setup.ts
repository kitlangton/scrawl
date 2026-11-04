import { spawn, type ChildProcess } from 'node:child_process'
import { performance } from 'node:perf_hooks'
import { pathToFileURL } from 'node:url'

type Command = { label: string; file: string; args: string[] }
const commands: Command[] = [
  { label: 'FFmpeg', file: 'bash', args: ['-c', 'command -v ffmpeg && command -v ffprobe || brew install ffmpeg'] },
  { label: 'Metal toolchain', file: 'bash', args: ['-c', 'xcrun --sdk macosx metal --version || xcodebuild -downloadComponent MetalToolchain'] },
]

function signalGroup(child: ChildProcess, signal: NodeJS.Signals) {
  if (!child.pid) return
  try { process.kill(-child.pid, signal) } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'ESRCH') throw error
  }
}

export async function prepareMacDependencies(timeoutMs = 120_000): Promise<void> {
  if (!Number.isFinite(timeoutMs) || timeoutMs <= 0 || timeoutMs > 120_000) throw new Error('timeoutMs must be between 1 and 120000')
  const totalStart = performance.now()
  const children: ChildProcess[] = []
  let failure: Error | undefined
  let timer: NodeJS.Timeout | undefined
  let resolveRun!: () => void
  const done = new Promise<void>((resolve) => { resolveRun = resolve })
  const signalHandlers = new Map<NodeJS.Signals, () => void>()
  const escalations = new Set<NodeJS.Timeout>()
  const escalate = () => {
    const killTimer = setTimeout(() => { stopAll('SIGKILL'); escalations.delete(killTimer) }, 1000)
    escalations.add(killTimer)
  }

  const stopAll = (signal: NodeJS.Signals) => {
    for (const child of children) signalGroup(child, signal)
  }
  const onSignal = (signal: NodeJS.Signals) => {
    if (!failure) failure = new Error(`cancelled by ${signal}`)
    stopAll('SIGTERM')
    escalate()
  }
  for (const signal of ['SIGINT', 'SIGTERM', 'SIGHUP'] as const) {
    const handler = () => onSignal(signal)
    signalHandlers.set(signal, handler)
    process.on(signal, handler)
  }

  try {
    const results = commands.map((command) => new Promise<void>((resolve, reject) => {
      const start = performance.now()
      let child: ChildProcess
      try {
        child = spawn(command.file, command.args, { stdio: 'inherit', detached: true })
      } catch (error) {
        reject(new Error(`${command.label} spawn failed: ${String(error)}`)); return
      }
      children.push(child)
      let ended = false
      const finish = (error?: Error, code?: number | null, signal?: NodeJS.Signals | null) => {
        if (ended) return
        ended = true
        const elapsed = (performance.now() - start).toFixed(0)
        console.log(`${command.label}: ${elapsed}ms exit=${code ?? signal ?? 'spawn-error'}`)
        if (error) reject(error)
        else if (code !== 0) reject(new Error(`${command.label} failed (${signal ?? code ?? 'unknown'})`))
        else resolve()
      }
      child.once('error', (error) => finish(new Error(`${command.label} spawn failed: ${error.message}`)))
      child.once('close', (code, signal) => finish(undefined, code, signal))
    }))
    timer = setTimeout(() => {
      if (!failure) failure = new Error(`macOS dependency setup timed out after ${timeoutMs}ms`)
      // The deadline is a total wall-clock bound, so timeout cleanup cannot add a grace period.
      stopAll('SIGKILL')
    }, timeoutMs)
    timer.unref()
    void Promise.all(results).then(() => { resolveRun() }, (error: Error) => {
      if (!failure) failure = error
      stopAll('SIGTERM')
      escalate()
      void Promise.allSettled(results).then(() => { resolveRun() })
    })
    await done
    if (failure) throw failure
  } finally {
    if (timer) clearTimeout(timer)
    for (const escalation of escalations) clearTimeout(escalation)
    for (const [signal, handler] of signalHandlers) process.off(signal, handler)
    // A successful direct command may still have left a background child in its owned group.
    stopAll('SIGKILL')
    const elapsed = (performance.now() - totalStart).toFixed(0)
    console.log(`macOS dependency setup total: ${elapsed}ms exit=${failure ? 'failure' : '0'}`)
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  prepareMacDependencies().catch((error: Error) => { console.error(error.message); process.exitCode = 1 })
}
