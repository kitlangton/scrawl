import { spawn, type ChildProcess } from 'node:child_process';
import { chmod, mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import path from 'node:path';
const exited = new WeakSet<ChildProcess>();

export function trackChild(child: ChildProcess): ChildProcess {
  child.once('close', () => exited.add(child));
  return child;
}

export async function runBounded(command: string, args: string[], timeoutMs: number): Promise<{ stdout: string; stderr: string }> {
  const child = trackChild(spawn(command, args, { stdio: ['ignore', 'pipe', 'pipe'] }));
  let stdout = '', stderr = '';
  let timedOut = false;
  child.stdout?.setEncoding('utf8').on('data', chunk => stdout += chunk);
  child.stderr?.setEncoding('utf8').on('data', chunk => stderr += chunk);
  const result = new Promise<void>((resolve, reject) => {
    child.once('error', reject);
    child.once('close', (code, signal) => { if (timedOut) return; code === 0 ? resolve() : reject(new Error(`${command} failed (${signal ?? code}): ${stderr}`)); });
  });
  let timer: ReturnType<typeof setTimeout> | undefined;
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => {
      timedOut = true;
      void waitForExit(child, 500).then(() => reject(new Error(`${command} deadline exceeded`)), reject);
    }, timeoutMs);
  });
  try { await Promise.race([result, timeout]); }
  finally { clearTimeout(timer); }
  return { stdout, stderr };
}

/** Parse the exact generated module before a qualification supervisor is launched. */
export async function checkGeneratedNodeScript(source: string): Promise<void> {
  const directory = await mkdtemp(path.join(tmpdir(), 'record-native-script-check-'));
  try {
    await chmod(directory, 0o700);
    const script = path.join(directory, 'qualification.mjs');
    await writeFile(script, source, { mode: 0o600, flag: 'wx' });
    await runBounded(process.execPath, ['--check', script], 5000);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

/** Attempt every owner-local cleanup action while retaining every failure. */
export async function runCleanupSteps(steps: Array<{ label: string; run: () => unknown | Promise<unknown> }>): Promise<Error[]> {
  const errors: Error[] = [];
  for (const step of steps) {
    try { await step.run(); }
    catch (cause) { errors.push(new Error(`${step.label}: ${cause instanceof Error ? cause.message : String(cause)}`, { cause })); }
  }
  return errors;
}

export async function waitForExit(child: ChildProcess, graceMs = 1500): Promise<void> {
  if (exited.has(child)) return;
  await new Promise<void>((resolve, reject) => {
    const done = () => { exited.add(child); clearTimeout(timer); child.off('error', failed); resolve(); };
    const failed = (error: Error) => { clearTimeout(timer); reject(error); };
    const timer = setTimeout(() => {
      child.kill('SIGKILL');
      const killTimer = setTimeout(() => reject(new Error('child did not exit after SIGKILL')), graceMs);
      child.once('close', () => { clearTimeout(killTimer); done(); });
    }, graceMs);
    child.once('close', done);
    child.once('error', failed);
    child.kill('SIGTERM');
  });
}

export async function reconcileAndCancel(mappings: any, client: any, deadline: number): Promise<void> {
  for (const id of await mappings.listRequestIds('capture2')) {
    const row = await mappings.getByRequestId(id);
    if (!row?.capture2Intent) continue;
    const owned = { captureId: row.captureId, ownerToken: row.ownerToken };
    let status = await client.inspect(owned);
    if (!['starting', 'recording', 'stopping', 'cancelled', 'saved', 'failed', 'materials-ready'].includes(status.state)) {
      throw new Error('capture cleanup inspection is ambiguous: ' + status.state);
    }
    if (status.state === 'failed') throw new Error('capture entered failed terminal state during cleanup');
    if (['starting', 'recording', 'stopping'].includes(status.state)) {
      await client.cancel(owned);
      while (Date.now() < deadline) {
        status = await client.inspect(owned);
        if (['cancelled', 'saved', 'failed', 'materials-ready'].includes(status.state)) break;
        await new Promise(resolve => setTimeout(resolve, 100));
      }
      if (!['cancelled', 'saved', 'materials-ready'].includes(status.state)) throw new Error('capture cancellation did not reach authoritative terminal state');
    }
  }
}

export interface QualificationClearance {
  scope: 'record-coordinator';
  admission: 'closed';
  localDrain: 'complete';
  runtimeClearance: 'clear';
  disposed: true;
  owningClientsClosed: true;
}

/** A native run cannot start until the supervisor can request and observe local owner drain. */
export function assertQualificationShutdownProtocol(available: boolean): void {
  if (!available) throw new Error('native qualification launch refused: supervisor has no child-to-parent coordinator shutdown and clearance protocol; SDK 0.4.4 cannot release an unresolved owning peer safely');
}

function hasPositiveClearance(value: unknown): value is QualificationClearance {
  if (!value || typeof value !== 'object') return false;
  const report = value as Partial<QualificationClearance>;
  return report.scope === 'record-coordinator' && report.admission === 'closed' && report.localDrain === 'complete' &&
    report.runtimeClearance === 'clear' && report.disposed === true && report.owningClientsClosed === true;
}

/** Signal a process group only after the owner has drained and released its local SDK handles. */
export async function terminateQualificationGroupAfterClearance(pid: number, clearance: unknown, graceMs = 500): Promise<void> {
  if (!hasPositiveClearance(clearance)) throw new Error('qualification process group retained: positive coordinator clearance and client close are required');
  try { process.kill(-pid, 'SIGTERM'); }
  catch (error) { if ((error as NodeJS.ErrnoException).code === 'ESRCH') return; throw error; }
  const deadline = Date.now() + graceMs;
  while (Date.now() < deadline) {
    try { process.kill(-pid, 0); }
    catch (error) { if ((error as NodeJS.ErrnoException).code === 'ESRCH') return; throw error; }
    await new Promise(resolve => setTimeout(resolve, 20));
  }
  try { process.kill(-pid, 'SIGKILL'); }
  catch (error) { if ((error as NodeJS.ErrnoException).code !== 'ESRCH') throw error; }
}

/** The native supervisor never kills an SDK-owning group without positive local clearance. */
export async function runQualification(command: string, args: string[], options: { cwd: string; env: NodeJS.ProcessEnv; timeout: number }): Promise<{ stdout: string; stderr: string }> {
  const child = spawn(command, args, { cwd: options.cwd, env: options.env, detached: true, stdio: ['ignore', 'pipe', 'pipe'] });
  let stdout = '', stderr = '', failure: Error | undefined;
  const outputLimit = 8 * 1024 * 1024;
  let rejectAbort!: (error: Error) => void;
  const abort = (error: Error) => {
    failure ??= error;
    if (child.pid) Object.assign(error, { ownedProcessGroupPid: child.pid, processGroupRetained: true });
    rejectAbort(error);
  };
  const aborted = new Promise<never>((_, reject) => { rejectAbort = reject; });
  const timer = setTimeout(() => abort(new Error('native qualification deadline exceeded')), options.timeout);
  try {
    await Promise.race([new Promise<void>((resolve, reject) => {
      const collect = (current: string, value: string, assign: (next: string) => void) => {
        if (failure) return;
        const remaining = outputLimit - current.length;
        assign(current + value.slice(0, remaining));
        if (value.length > remaining) abort(new Error('qualification output limit'));
      };
      child.stdout.setEncoding('utf8').on('data', value => collect(stdout, value, next => { stdout = next; }));
      child.stderr.setEncoding('utf8').on('data', value => collect(stderr, value, next => { stderr = next; }));
      child.once('error', reject);
      child.once('close', code => failure ? reject(new Error(`${failure.message}\nstdout:\n${stdout}\nstderr:\n${stderr}`)) : code === 0 ? resolve() : reject(new Error(`native qualification failed (${code}):\nstdout:\n${stdout}\nstderr:\n${stderr}`)));
    }), aborted]);
    return { stdout, stderr };
  } finally {
    clearTimeout(timer);
  }
}
