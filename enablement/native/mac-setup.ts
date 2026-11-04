import { spawn, type ChildProcess } from 'node:child_process'
import { randomUUID } from 'node:crypto'
import { open, lstat, rename, unlink, link } from 'node:fs/promises'
import { constants } from 'node:fs'
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

type PendingStatus = { state: 'pending'; nonce: string; startedAt: number; deadlineAt: number }
type FinalStatus = { state: 'complete'; nonce: string; startedAt: number; deadlineAt: number; finishedAt: number; exit: 0 | 1 }
type SetupStatus = PendingStatus | FinalStatus

function statusError(message: string): Error { return new Error(`macOS dependency status: ${message}`) }

async function statusFile(path: string) {
  let info
  try { info = await lstat(path) } catch (error) {
    if ((error as NodeJS.ErrnoException).code === 'ENOENT') return undefined
    throw error
  }
  if (info.isSymbolicLink() || !info.isFile()) throw statusError('must be a regular, non-symlink file')
  if ((info.mode & 0o077) !== 0) throw statusError('must be private (mode 0600)')
  return info
}

async function readStatus(path: string): Promise<SetupStatus | undefined> {
  if (!await statusFile(path)) return undefined
  let value: unknown
  try {
    const handle = await open(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0))
    try { value = JSON.parse(await handle.readFile('utf8')) } finally { await handle.close() }
  } catch (error) {
    if (error instanceof SyntaxError) throw statusError('contains malformed JSON')
    if ((error as NodeJS.ErrnoException).code === 'ELOOP') throw statusError('must be a regular, non-symlink file')
    throw error
  }
  if (!value || typeof value !== 'object') throw statusError('has an invalid shape')
  const status = value as Record<string, unknown>
  const validTimes = Number.isSafeInteger(status.startedAt) && Number.isSafeInteger(status.deadlineAt)
    && Number(status.startedAt) > 0 && Number(status.startedAt) <= Date.now()
    && Number(status.deadlineAt) > Number(status.startedAt)
    && Number(status.deadlineAt) - Number(status.startedAt) <= 120_000
  if (typeof status.nonce !== 'string' || !/^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/.test(status.nonce) || !validTimes)
    throw statusError('has invalid identity or deadline metadata')
  if (status.state === 'pending' && Object.keys(status).sort().join(',') === 'deadlineAt,nonce,startedAt,state') return status as PendingStatus
  if (status.state === 'complete' && (status.exit === 0 || status.exit === 1) && Number.isFinite(status.finishedAt)
    && Object.keys(status).sort().join(',') === 'deadlineAt,exit,finishedAt,nonce,startedAt,state'
    && Number.isSafeInteger(status.finishedAt) && Number(status.finishedAt) >= Number(status.startedAt)
    && Number(status.finishedAt) <= Date.now() && (status.exit === 1 || Number(status.finishedAt) <= Number(status.deadlineAt)))
    return status as FinalStatus
  throw statusError('has an invalid terminal state')
}

async function writeFinalStatus(path: string, pending: PendingStatus, exit: 0 | 1): Promise<void> {
  const finishedAt = Date.now()
  const status: FinalStatus = { state: 'complete', nonce: pending.nonce, startedAt: pending.startedAt, deadlineAt: pending.deadlineAt, finishedAt, exit }
  const temporary = `${path}.${pending.nonce}.tmp`
  const handle = await open(temporary, 'wx', 0o600)
  try { await handle.writeFile(`${JSON.stringify(status)}\n`, 'utf8') } finally { await handle.close() }
  try {
    const current = await readStatus(path)
    if (!current || current.state !== 'pending' || current.nonce !== pending.nonce) throw statusError('pending identity changed before completion')
    await rename(temporary, path)
  } finally { await unlink(temporary).catch(() => undefined) }
}

async function prepareToStatus(path: string): Promise<void> {
  if (await statusFile(path)) throw statusError('already exists')
  const startedAt = Date.now()
  const pending: PendingStatus = { state: 'pending', nonce: randomUUID(), startedAt, deadlineAt: startedAt + 120_000 }
  console.log(`macOS dependency worker startedAt=${pending.startedAt} deadlineAt=${pending.deadlineAt}`)
  // Exclusive creation alone exposes an empty file before the async write.
  // Publish a completed sibling inode atomically, without replacing any winner.
  const temporary = `${path}.${pending.nonce}.pending`
  const handle = await open(temporary, 'wx', 0o600)
  try {
    try { await handle.writeFile(`${JSON.stringify(pending)}\n`, 'utf8') } finally { await handle.close() }
    await link(temporary, path)
  } finally { await unlink(temporary).catch(() => undefined) }
  let exit: 0 | 1 = 0
  try { await prepareMacDependencies(Math.max(1, pending.deadlineAt - Date.now())) }
  catch (error) { exit = 1; console.error(error instanceof Error ? error.message : String(error)) }
  // The supervisor awaits direct-child close and signals remaining owned groups
  // before returning; it does not separately observe every descendant exit.
  await writeFinalStatus(path, pending, exit)
  if (exit !== 0) process.exitCode = 1
}

async function awaitStatus(path: string): Promise<void> {
  const exposedAt = Date.now()
  const initial = await readStatus(path)
  if (!initial) throw statusError('does not exist')
  const waitUntil = (initial as PendingStatus).deadlineAt
  for (;;) {
    const current = await readStatus(path)
    if (!current) throw statusError('disappeared while waiting')
    if (current.nonce !== initial.nonce || current.startedAt !== initial.startedAt || current.deadlineAt !== initial.deadlineAt)
      throw statusError('identity changed while waiting')
    if (current.state === 'complete') {
      if (current.exit !== 0) {
        console.log(`macOS dependency await exposed-wait-ms=${Date.now() - exposedAt} result=failure`)
        throw statusError('preparation failed')
      }
      console.log(`macOS dependency await exposed-wait-ms=${Date.now() - exposedAt} result=success`)
      return
    }
    const remaining = waitUntil - Date.now()
    if (remaining <= 0) {
      console.log(`macOS dependency await exposed-wait-ms=${Date.now() - exposedAt} result=deadline`)
      throw statusError('preparation did not finish before its deadline')
    }
    await new Promise((resolve) => setTimeout(resolve, Math.min(remaining, 25)))
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  const [mode, statusPath, ...extra] = process.argv.slice(2)
  const run = mode === '--prepare' && statusPath && extra.length === 0 ? prepareToStatus(statusPath)
    : mode === '--await' && statusPath && extra.length === 0 ? awaitStatus(statusPath)
      : mode === undefined && statusPath === undefined ? prepareMacDependencies() : Promise.reject(statusError('usage: mac-setup.ts [--prepare STATUS | --await STATUS]'))
  run.catch((error: Error) => { console.error(error.message); process.exitCode = 1 })
}
