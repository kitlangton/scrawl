import { spawn, type ChildProcess } from 'node:child_process';
import { randomUUID } from 'node:crypto';
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

export const QUALIFICATION_IPC_SCHEMA = 'apparent.record-native-qualification/1';

export interface QualificationShutdownEvidence {
  neverInjected?: boolean;
  externalAdmissionClosed: boolean;
  admission: string;
  localDrain: string;
  disposed: boolean;
  borrowersDrained: boolean;
}

export function canReleaseQualificationRecoveryPeers(evidence: QualificationShutdownEvidence): boolean {
  if (evidence.neverInjected === true) return true;
  return evidence.externalAdmissionClosed && evidence.admission === 'closed' && evidence.localDrain === 'complete' && evidence.disposed && evidence.borrowersDrained;
}

export type QualificationCaptureCloseOutcome = 'already-exited' | 'eof-exited' | 'exit-observed' | 'signal-exited' | 'kill-exited' | 'signal-requested-after-timeout' | 'kill-requested-after-timeout';
export type QualificationEndIntent = 'finish' | 'end' | 'interrupt' | undefined;

export interface QualificationCaptureOwnerCloseInput extends QualificationShutdownEvidence {
  endIntent: QualificationEndIntent;
  mediaDispatchedOnCapturePeer: boolean;
  publicationDispatchedOnCapturePeer: boolean;
  close: () => Promise<QualificationCaptureCloseOutcome>;
}

export type QualificationCaptureOwnerCloseResult =
  | { state: 'retained'; status: 3; reason: string }
  | { state: 'ended'; status: 0; outcome: Exclude<QualificationCaptureCloseOutcome, 'signal-requested-after-timeout' | 'kill-requested-after-timeout'>; runtimeClearance: 'not-established' }
  | { state: 'unknown'; status: 3; outcome: 'signal-requested-after-timeout' | 'kill-requested-after-timeout'; runtimeClearance: 'not-established' }
  | { state: 'unknown'; status: 3; error: string; runtimeClearance: 'not-established' };

function captureOwnerCloseRefusal(input: QualificationCaptureOwnerCloseInput): string | undefined {
  if (!input.endIntent) return 'no explicit foreground finish/end/interrupt intent';
  if (input.externalAdmissionClosed !== true || input.admission !== 'closed' || input.localDrain !== 'complete' || input.disposed !== true || input.borrowersDrained !== true)
    return 'coordinator admission, local drain, disposal, or borrower drain is unresolved';
  if (input.mediaDispatchedOnCapturePeer || input.publicationDispatchedOnCapturePeer) return 'capture-owning peer has processing/publication work';
  return undefined;
}

/** Close only the dedicated foreground capture owner after explicit end intent and a positive local drain. */
export async function closeQualificationCaptureOwner(input: QualificationCaptureOwnerCloseInput): Promise<QualificationCaptureOwnerCloseResult> {
  const refusal = captureOwnerCloseRefusal(input);
  if (refusal) return { state: 'retained', status: 3, reason: refusal };
  try {
    const outcome = await input.close();
    if (outcome === 'signal-requested-after-timeout' || outcome === 'kill-requested-after-timeout') {
      return { state: 'unknown', status: 3, outcome, runtimeClearance: 'not-established' };
    }
    switch (outcome) {
      case 'already-exited': case 'eof-exited': case 'exit-observed': case 'signal-exited': case 'kill-exited':
        return { state: 'ended', status: 0, outcome, runtimeClearance: 'not-established' };
      default:
        return { state: 'unknown', status: 3, error: 'unrecognized owning close outcome', runtimeClearance: 'not-established' };
    }
  } catch (error) {
    return { state: 'unknown', status: 3, error: error instanceof Error ? error.message : String(error), runtimeClearance: 'not-established' };
  }
}

/** Records whether the exact SDK 0.4.5 owning-close contract is present in the frozen consumer. */
export function assertQualificationShutdownProtocol(available: boolean): void {
  if (!available) throw new Error('native qualification launch refused: SDK 0.4.5 owning capture close was not validated');
}

export function assertNativeQualificationAdmission(admitted: boolean): void {
  if (!admitted) throw new Error('native qualification launch refused: independent consumer review and separate native attempt/resource authorization are not admitted');
}

/** The supervisor never signals a qualification process group; native owner close stays inside the SDK contract. */
export async function runQualification(command: string, args: string[], options: { cwd: string; env: NodeJS.ProcessEnv; timeout: number; shutdownReportTimeout?: number; outputLimitBytes?: number }): Promise<{ stdout: string; stderr: string; shutdownReport?: unknown }> {
  const runId = randomUUID();
  const child = spawn(command, args, { cwd: options.cwd, env: { ...options.env, APPARENT_QUALIFICATION_RUN_ID: runId }, detached: true, stdio: ['ignore', 'pipe', 'pipe', 'ipc'] });
  let stdout = '', stderr = '', capturedBytes = 0, failure: Error | undefined, childExited = false, childExitCode: number | null = null, childExitSignal: NodeJS.Signals | null = null, abortPromise: Promise<void> | undefined;
  const outputLimit = options.outputLimitBytes ?? 8 * 1024 * 1024;
  let rejectAbort!: (error: Error) => void;
  let requestedId: string | undefined, shutdownReport: unknown;
  const reports = new Map<string, unknown>();
  const reportWaiters = new Map<string, (value: unknown) => void>();
  child.on('message', message => {
    if (!message || typeof message !== 'object' || Array.isArray(message)) return;
    const value = message as Record<string, unknown>;
    if (value.schema !== QUALIFICATION_IPC_SCHEMA || value.kind !== 'shutdown-report' || typeof value.requestId !== 'string' || !('report' in value)) return;
    reports.set(value.requestId, value.report);
    reportWaiters.get(value.requestId)?.(value.report);
    if (value.requestId === runId && requestedId === undefined && value.report && typeof value.report === 'object' && (value.report as Record<string, unknown>).status === 3) {
      shutdownReport = value.report;
      failure = new Error('qualification child reported unresolved shutdown with status 3');
      abortPromise = Promise.resolve();
      Object.assign(failure, { shutdownRequestId: runId, shutdownReport, ownedProcessGroupPid: child.pid, processGroupRetained: !childExited,
        exitCode: childExitCode, signal: childExitSignal, stdout, stderr });
      rejectAbort(failure);
      return;
    }
    if (value.requestId !== requestedId) return;
    shutdownReport = value.report;
  });
  const requestShutdown = async (reason: 'deadline' | 'output-limit'): Promise<unknown> => {
    requestedId = randomUUID();
    if (!child.connected) return undefined;
    try {
      child.send({ schema: QUALIFICATION_IPC_SCHEMA, kind: 'shutdown-request', requestId: requestedId, reason }, error => {
        if (error && failure) Object.assign(failure, { shutdownRequestError: error.message });
      });
    } catch (error) {
      Object.assign(failure!, { shutdownRequestError: error instanceof Error ? error.message : String(error) });
      return undefined;
    }
    if (reports.has(requestedId)) return reports.get(requestedId);
    return new Promise(resolve => {
      const timer = setTimeout(() => resolve(undefined), options.shutdownReportTimeout ?? 3000);
      reportWaiters.set(requestedId!, value => { clearTimeout(timer); reportWaiters.delete(requestedId!); resolve(value); });
    });
  };
  const abort = (error: Error, reason: 'deadline' | 'output-limit') => {
    if (failure) return;
    failure = error;
    abortPromise = (async () => {
      const report = await requestShutdown(reason);
      Object.assign(error, {
        shutdownRequestId: requestedId,
        shutdownReport: report ?? null,
        ownedProcessGroupPid: child.pid,
        processGroupRetained: !childExited,
        exitCode: childExitCode,
        signal: childExitSignal,
        stdout,
        stderr,
      });
      rejectAbort(error);
    })();
  };
  const aborted = new Promise<never>((_, reject) => { rejectAbort = reject; });
  const timer = setTimeout(() => abort(new Error('native qualification deadline exceeded'), 'deadline'), options.timeout);
  try {
    await Promise.race([new Promise<void>((resolve, reject) => {
      const collect = (current: string, value: string, assign: (next: string) => void) => {
        if (failure) return;
        const bytes = Buffer.from(value);
        const remaining = Math.max(0, outputLimit - capturedBytes);
        capturedBytes += Math.min(remaining, bytes.length);
        assign(current + bytes.subarray(0, remaining).toString('utf8'));
        if (bytes.length > remaining) abort(new Error('qualification output limit'), 'output-limit');
      };
      child.stdout.setEncoding('utf8').on('data', value => collect(stdout, value, next => { stdout = next; }));
      child.stderr.setEncoding('utf8').on('data', value => collect(stderr, value, next => { stderr = next; }));
      child.once('error', reject);
      child.once('close', (code, signal) => {
        childExited = true;
        childExitCode = code;
        childExitSignal = signal;
        if (failure) {
          if (!abortPromise) reject(Object.assign(new Error(`${failure.message}\nstdout:\n${stdout}\nstderr:\n${stderr}`), { exitCode: code, signal, shutdownReport }));
        } else if (code === 0) resolve();
        else reject(Object.assign(new Error(`native qualification failed (${code ?? signal}):\nstdout:\n${stdout}\nstderr:\n${stderr}`), { exitCode: code, signal, shutdownReport }));
      });
    }), aborted]);
    return { stdout, stderr, ...(shutdownReport === undefined ? {} : { shutdownReport }) };
  } finally {
    clearTimeout(timer);
  }
}
