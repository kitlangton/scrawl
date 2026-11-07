import { lstat } from "node:fs/promises";
import { decodeJob, encodeJob, type Job } from "./job.ts";
import { readQueueWithinLease } from "./store.ts";
import { completeUpload, expire, type FreshQueueSnapshot } from "./transitions.ts";
import { projectUploadFailure, type UploadErrorFacts, type UploadFailureProjection } from "./upload-failure.ts";
import { normalizeRecipientServer } from "../recipient/transport.ts";
import { readCancellation, readCredential, pendingIntent, readStateEvidence, stateFileStamp } from "../state/credential-files.ts";
import { withStateQueue, withStateWorker, checkSignal, samePathFd, CredentialSnapshotError,
  type StateDirectory, type NamespaceLease, type NamespaceResources } from "../state/queue-transaction.ts";
import { replacePrivateStateFile, type StateReplacementResources } from "../state/replacement.ts";

export type UploadWorkerSettings = Readonly<{
  enabled: boolean; server: string; encryptionEnabled: boolean; sharingEnabled: boolean;
  nowSeconds: number; retentionSeconds: number;
}>;
export type UploadWorkerContext = Readonly<{ signal: AbortSignal; assertCurrent(): Promise<void> }>;
export type UploadWorkerOutcome = { state: "ready" } | { state: "failed"; facts: UploadErrorFacts } | { state: "uncertain" };
type Event = Readonly<{ id: string; state: "ready" | "retry" | "failed" | "expired" | "skipped"; diagnostic?: UploadFailureProjection["diagnostic"] }>;
export type UploadWorkerResult = Readonly<{ state: "complete" | "busy" | "uncertain"; events: readonly Event[] }>;
export type UploadWorkerResources = NamespaceResources & StateReplacementResources;
export type UploadWorkerInput = {
  directory: string; approvedServer: string; signal?: AbortSignal; queueTimeoutMs?: number;
  /** Fresh bounded local settings read. Called inside queue.lock; no network or queue reentry. */
  readSettings(signal: AbortSignal): Promise<UploadWorkerSettings>;
  /** Owns transfer/authentication and all callback work until it settles. Unknown
   * effects must return uncertain; no detached work may outlive this promise. */
  upload(job: Job, context: UploadWorkerContext): Promise<UploadWorkerOutcome>;
  /** Best-effort status-only service notification; must honor the supplied deadline. */
  reportProgress(job: Job, state: "paused" | "failed", context: UploadWorkerContext): Promise<void>;
};
export class UploadWorkerError extends Error {
  readonly category: "invalid_settings" | "cancelled" | "state_unavailable" | "stale_snapshot";
  constructor(category: UploadWorkerError["category"]) { super(category); this.category = category; this.name = "UploadWorkerError"; }
}
const fail = (category: UploadWorkerError["category"]): never => { throw new UploadWorkerError(category); };
function frozen<T>(value: T): T {
  if (value && typeof value === "object") { Object.freeze(value); for (const child of Object.values(value)) frozen(child); }
  return value;
}
function copy(job: Job): Job { const raw = encodeJob(job); try { return decodeJob(raw); } finally { raw.fill(0); } }
const epoch = (value: string | null) => JSON.stringify(value);
const sameIntent = (a: Job, b: Job) => a.id === b.id && a.bundle === b.bundle && a.server === b.server
  && a.generation === b.generation && a.created === b.created && a.encrypted === b.encrypted
  && a.publicCopy === b.publicCopy && a.grouped === b.grouped && (!a.publicCopy || b.shared);

/** INTERNAL runner preparation, not an installed worker. Requires coordinated
 * sole-writer admission and admitted local settings/transfer/progress consumers.
 * worker.lock spans uploads; queue.lock protects only short state transactions. */
export async function runUploadWorkerOnce(input: UploadWorkerInput, resources: UploadWorkerResources): Promise<UploadWorkerResult> {
  const directory = input.directory, callerSignal = input.signal, queueTimeoutMs = input.queueTimeoutMs;
  const readSettings = input.readSettings, upload = input.upload, progress = input.reportProgress;
  const events: Event[] = [];
  let entered = false;
  const effects: { state: "none" | "possible" } = { state: "none" };
  const result = (state: UploadWorkerResult["state"]) => frozen({ state, events: [...events] });
  try {
    const server = normalizeRecipientServer(input.approvedServer);
    if (typeof readSettings !== "function" || typeof upload !== "function" || typeof progress !== "function") fail("invalid_settings");
    return await withStateWorker(directory, { scope: "upload", approvedServer: server }, resources, { signal: callerSignal }, async (root, worker) => {
      entered = true;
      const signal = AbortSignal.any([worker.signal, ...(callerSignal ? [callerSignal] : [])]);
      const check = (lease?: NamespaceLease) => { checkSignal(signal, lease?.signal); worker.assertHeld(); lease?.assertHeld(); };
      const transaction = <T>(body: (state: StateDirectory, lease: NamespaceLease) => Promise<T>) => withStateQueue(root.path,
        { scope: "upload", approvedServer: server }, resources, { signal, lockTimeoutMs: queueTimeoutMs }, async (state, lease) => {
          await worker.assertCurrent(); check(lease); if (state.identity !== root.identity) fail("stale_snapshot");
          return body(state, lease);
        });
      async function settings(): Promise<UploadWorkerSettings> {
        check();
        const raw = await readSettings(signal);
        const value = { enabled: raw?.enabled, server: raw?.server, encryptionEnabled: raw?.encryptionEnabled,
          sharingEnabled: raw?.sharingEnabled, nowSeconds: raw?.nowSeconds, retentionSeconds: raw?.retentionSeconds };
        let normalized = ""; try { normalized = normalizeRecipientServer(value.server); } catch { fail("invalid_settings"); }
        if (typeof value.enabled !== "boolean" || normalized !== value.server
          || typeof value.encryptionEnabled !== "boolean" || typeof value.sharingEnabled !== "boolean"
          || !Number.isSafeInteger(value.nowSeconds) || value.nowSeconds < 0 || value.nowSeconds > Number.MAX_SAFE_INTEGER - 3600
          || !Number.isSafeInteger(value.retentionSeconds) || value.retentionSeconds < 0) fail("invalid_settings");
        check(); return value;
      }
      async function current(state: StateDirectory, lease: NamespaceLease, scheduled: Job, status = "pending") {
        await worker.assertCurrent(); check(lease);
        const policy = await settings();
        if (!policy.enabled || policy.server !== server || await pendingIntent(state.path)) return null;
        const saved = await readStateEvidence(state, `${scheduled.id}.job`, 2_000_000, resources, signal, decodeJob);
        if (!saved || saved.value.status !== status || !sameIntent(scheduled, saved.value)) return null;
        let credential;
        try { credential = await readCredential(state, { scope: "upload", approvedServer: server }, resources, signal); }
        catch (error) { if (error instanceof CredentialSnapshotError && error.category === "invalid_credential") return null; throw error; }
        const job = saved.value, cancellation = await readCancellation(state, resources, signal);
        const facts: FreshQueueSnapshot = { job, enabled: policy.enabled, server: policy.server, generation: credential.generation,
          encryptionEnabled: policy.encryptionEnabled, sharingEnabled: policy.sharingEnabled, now: policy.nowSeconds,
          retentionSeconds: policy.retentionSeconds, cancellationEpoch: epoch(cancellation) };
        if (job.generation !== credential.generation || (job.publicCopy ? !policy.sharingEnabled || job.encrypted || !job.shared : job.encrypted !== policy.encryptionEnabled)) return null;
        if (await pendingIntent(state.path)) return null;
        await worker.assertCurrent(); check(lease);
        return { facts, saved, grant: credential.grantId, identity: credential.encryption_identity };
      }
      async function persist(state: StateDirectory, lease: NamespaceLease, fresh: NonNullable<Awaited<ReturnType<typeof current>>>, next: Job) {
        check(lease);
        const saved = fresh.saved;
        const named = await lstat(saved.file, { bigint: true });
        if (stateFileStamp(named) !== saved.expectedStamp || !await samePathFd(state)) fail("stale_snapshot");
        const bytes = encodeJob(next);
        try {
          const guarded: StateReplacementResources = { checkAcl: handle => resources.checkAcl(handle),
            async replaceFile(parent, ...args) {
              try {
                const latest = await current(state, lease, fresh.facts.job);
                if (!latest || latest.saved.expectedStamp !== saved.expectedStamp || latest.grant !== fresh.grant || latest.identity !== fresh.identity
                  || latest.facts.cancellationEpoch !== fresh.facts.cancellationEpoch || latest.facts.retentionSeconds !== fresh.facts.retentionSeconds) {
                  await parent.close(); return { state: "refused" };
                }
              } catch { await parent.close().catch(() => {}); return { state: "refused" }; }
              return resources.replaceFile(parent, ...args);
            } };
          const outcome = await replacePrivateStateFile(state.path, `${next.id}.job`, bytes,
            { device: String(named.dev), inode: String(named.ino) }, { signal, assertHeld: () => check(lease) }, guarded, { signal });
          if (outcome.state === "committed") { effects.state = "possible"; return true; }
          if (outcome.state === "publication_uncertain") { effects.state = "possible"; return false; }
          return fail("stale_snapshot");
        } finally { bytes.fill(0); }
      }
      const jobs = await transaction((state, lease) => readQueueWithinLease(state, lease, resources, signal));
      for (const scheduled of jobs) {
        check();
        if (scheduled.status !== "pending") continue;
        const start = await transaction(async (state, lease) => {
          const fresh = await current(state, lease, scheduled);
          if (!fresh || fresh.facts.job.nextAttempt > fresh.facts.now) return null;
          const expiration = expire(fresh.facts);
          if (expiration.kind === "transition") {
            if (!await persist(state, lease, fresh, expiration.job)) return { state: "uncertain" as const };
            events.push({ id: scheduled.id, state: "expired" }); return null;
          }
          if (fresh.facts.job.attempts >= 0xffffffff) fail("state_unavailable");
          return { state: "upload" as const, ...fresh, job: frozen(copy(fresh.facts.job)) };
        });
        if (!start) continue;
        if (start.state === "uncertain") return result("uncertain");
        const assertCurrent = async () => {
          await transaction(async (state, lease) => {
            const fresh = await current(state, lease, start.job);
            if (!fresh || fresh.facts.cancellationEpoch !== start.facts.cancellationEpoch || fresh.grant !== start.grant || fresh.identity !== start.identity) fail("stale_snapshot");
          });
        };
        try { await assertCurrent(); }
        catch (error) {
          if (error instanceof UploadWorkerError && error.category === "stale_snapshot") { events.push({ id: scheduled.id, state: "skipped" }); continue; }
          throw error;
        }
        effects.state = "possible";
        let uploaded: UploadWorkerOutcome;
        try { uploaded = await upload(start.job, Object.freeze({ signal, assertCurrent })); }
        catch { return result("uncertain"); }
        check();
        if (uploaded?.state === "uncertain") return result("uncertain");
        let failure: UploadFailureProjection | undefined;
        if (uploaded?.state === "failed") failure = projectUploadFailure(uploaded.facts);
        else if (uploaded?.state !== "ready") return result("uncertain");
        const completed = await transaction(async (state, lease) => {
          const fresh = await current(state, lease, start.job);
          if (!fresh || fresh.grant !== start.grant || fresh.identity !== start.identity) return null;
          const captured = { ...start.job, cancellationEpoch: start.facts.cancellationEpoch };
          const decision = completeUpload(fresh.facts, captured, failure ? { ok: false, error: failure.error, sizeFailure: failure.sizeFailure } : { ok: true });
          if (decision.kind === "unchanged") return null;
          if (!await persist(state, lease, fresh, decision.job)) return { state: "uncertain" as const };
          const stateName = failure ? decision.job.status === "failed" ? "failed" : "retry" : "ready";
          events.push({ id: scheduled.id, state: stateName, ...(failure ? { diagnostic: failure.diagnostic } : {}) });
          return { state: "saved" as const, job: frozen(copy(decision.job)) };
        });
        if (completed?.state === "uncertain") return result("uncertain");
        if (!completed) { events.push({ id: scheduled.id, state: "skipped" }); continue; }
        if (failure && completed.job.shared) {
          const progressSignal = AbortSignal.any([signal, AbortSignal.timeout(2000)]);
          const assertProgress = async () => {
            if (progressSignal.aborted) fail("cancelled");
            const valid = await transaction((state, lease) => current(state, lease, completed.job, completed.job.status));
            if (progressSignal.aborted) fail("cancelled");
            if (!valid || !valid.facts.job.shared || valid.facts.cancellationEpoch !== start.facts.cancellationEpoch
              || valid.grant !== start.grant || valid.identity !== start.identity) fail("stale_snapshot");
          };
          try { await assertProgress(); await progress(completed.job, completed.job.status === "failed" ? "failed" : "paused", { signal: progressSignal, assertCurrent: assertProgress }); }
          catch { /* Best-effort metadata only; no queue lease or media request is retained here. */ }
        }
      }
      return result("complete");
    });
  } catch (error) {
    if (effects.state === "possible") return result("uncertain");
    if (!entered && error instanceof CredentialSnapshotError && error.category === "lock_refused") return result("busy");
    if (callerSignal?.aborted || error instanceof CredentialSnapshotError && error.category === "cancelled") return fail("cancelled");
    if (error instanceof UploadWorkerError) throw new UploadWorkerError(error.category);
    return fail("state_unavailable");
  }
}
