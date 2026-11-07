import { lstat } from "node:fs/promises";
import { decodeJob, encodeJob } from "./job.ts";
import { readQueueWithinLease } from "./store.ts";
import { retry, type FreshQueueSnapshot } from "./transitions.ts";
import { normalizeRecipientServer } from "../recipient/transport.ts";
import { readCancellation, readCredential, pendingIntent, readStateEvidence, stateFileStamp } from "../state/credential-files.ts";
import { checkSignal, CredentialSnapshotError, samePathFd, withStateQueue, type NamespaceResources, type NamespaceLease, type StateDirectory } from "../state/queue-transaction.ts";
import { replacePrivateStateFile, type StateReplacementResources } from "../state/replacement.ts";
import type { UploadWorkerSettings } from "./worker.ts";

export type RetryResources = NamespaceResources & StateReplacementResources;
export type RetryInput = Readonly<{ directory: string; approvedServer: string; readSettings(signal: AbortSignal): Promise<UploadWorkerSettings>;
  signal?: AbortSignal; queueTimeoutMs?: number }>;
export type RetryResult = Readonly<{ state: "complete" | "uncertain"; changedJobIds: readonly string[] }>;
export class RetryError extends Error {
  readonly category: "invalid_settings" | "cancelled" | "state_unavailable" | "stale_snapshot" | "revocation_pending";
  constructor(category: RetryError["category"]) { super(category); this.name = "RetryError"; this.category = category; }
}
const fail = (category: RetryError["category"]): never => { throw new RetryError(category); };
const epoch = (v: string | null) => JSON.stringify(v);
const intent = (a: ReturnType<typeof decodeJob>, b: ReturnType<typeof decodeJob>) => a.id === b.id && a.bundle === b.bundle && a.server === b.server
  && a.generation === b.generation && a.created === b.created && a.encrypted === b.encrypted && a.publicCopy === b.publicCopy && a.grouped === b.grouped;
const frozen = <T>(x: T): T => { if (x && typeof x === "object") { Object.freeze(x); for (const v of Object.values(x)) frozen(v); } return x; };

/** Internal retry preparation. Production routing remains owned by the Rust client. */
export async function retryQueuedUploads(input: RetryInput, resources: RetryResources): Promise<RetryResult> {
  const directory = input.directory, callerSignal = input.signal, queueTimeoutMs = input.queueTimeoutMs;
  const readSettings = input.readSettings, approvedServer = input.approvedServer;
  const changed: string[] = []; let dispatched = false, committedAny = false;
  const result = (state: RetryResult["state"]): RetryResult => frozen({ state, changedJobIds: [...changed] });
  try {
    const server = normalizeRecipientServer(approvedServer);
    if (typeof readSettings !== "function") fail("invalid_settings");
    checkSignal(callerSignal);
    return await withStateQueue(directory, { scope: "upload", approvedServer: server }, resources,
      { signal: callerSignal, lockTimeoutMs: queueTimeoutMs }, async (root, lease) => {
        const signal = AbortSignal.any([lease.signal, ...(callerSignal ? [callerSignal] : [])]);
        const check = () => checkSignal(signal, lease.signal);
        const settings = async (): Promise<UploadWorkerSettings> => {
          await lease.assertCurrent(); check(); let raw: UploadWorkerSettings;
          try { raw = await readSettings(signal); } catch { return fail("state_unavailable"); }
          await lease.assertCurrent(); check();
          let normalized = ""; try { normalized = normalizeRecipientServer(raw?.server); } catch { fail("invalid_settings"); }
          if (typeof raw?.enabled !== "boolean" || normalized !== raw.server || typeof raw.encryptionEnabled !== "boolean"
            || typeof raw.sharingEnabled !== "boolean" || !Number.isSafeInteger(raw.nowSeconds) || raw.nowSeconds < 0
            || raw.nowSeconds > Number.MAX_SAFE_INTEGER - 3600 || !Number.isSafeInteger(raw.retentionSeconds) || raw.retentionSeconds < 0) fail("invalid_settings");
          return Object.freeze({ enabled: raw.enabled, server: raw.server, encryptionEnabled: raw.encryptionEnabled,
            sharingEnabled: raw.sharingEnabled, nowSeconds: raw.nowSeconds, retentionSeconds: raw.retentionSeconds });
        };
        const fresh = async (state: StateDirectory, l: NamespaceLease, scheduled: ReturnType<typeof decodeJob>) => {
          await lease.assertCurrent(); check(); l.assertHeld();
          const policy = await settings();
          if (!policy.enabled) fail("invalid_settings");
          if (policy.server !== server) return null;
          if (await pendingIntent(state.path)) fail("revocation_pending");
          const saved = await readStateEvidence(state, `${scheduled.id}.job`, 2_000_000, resources, signal, decodeJob);
          if (!saved || !intent(scheduled, saved.value)) return null;
          const job = saved.value;
          if (job.status !== "pending" && job.status !== "failed") return null;
          const credential = await readCredential(state, { scope: "upload", approvedServer: server }, resources, signal);
          const cancellation = await readCancellation(state, resources, signal);
          if (job.generation !== credential.generation || (job.publicCopy ? !policy.sharingEnabled || job.encrypted || !job.shared : job.encrypted !== policy.encryptionEnabled)) return null;
          if (await pendingIntent(state.path)) fail("revocation_pending");
          await lease.assertCurrent(); check(); l.assertHeld();
          const facts: FreshQueueSnapshot = { job, enabled: policy.enabled, server: policy.server, generation: credential.generation,
            encryptionEnabled: policy.encryptionEnabled, sharingEnabled: policy.sharingEnabled, now: policy.nowSeconds,
            retentionSeconds: policy.retentionSeconds, cancellationEpoch: epoch(cancellation) };
          return { saved, facts, grant: credential.grantId, identity: credential.encryption_identity };
        };
        const initialPolicy = await settings();
        if (!initialPolicy.enabled) fail("invalid_settings");
        if (initialPolicy.server !== server) return result("complete");
        if (await pendingIntent(root.path)) fail("revocation_pending");
        const pinnedCredential = await readCredential(root, { scope: "upload", approvedServer: server }, resources, signal);
        const pinnedCancellation = epoch(await readCancellation(root, resources, signal));
        if (await pendingIntent(root.path)) fail("revocation_pending");
        const snapshot = await readQueueWithinLease(root, lease, resources, signal);
        for (const scheduled of snapshot) {
          check();
          if (scheduled.status !== "pending" && scheduled.status !== "failed") continue;
          const current = await fresh(root, lease, scheduled); if (!current) continue;
          if (current.grant !== pinnedCredential.grantId || current.identity !== pinnedCredential.encryption_identity
            || current.facts.generation !== pinnedCredential.generation || current.facts.cancellationEpoch !== pinnedCancellation) fail("stale_snapshot");
          const decision = retry(current.facts); if (decision.kind !== "transition") continue;
          const named = await lstat(current.saved.file, { bigint: true });
          if (stateFileStamp(named) !== current.saved.expectedStamp || !await samePathFd(root)) fail("stale_snapshot");
          const bytes = encodeJob(decision.job);
          try {
            const guarded: StateReplacementResources = { checkAcl: h => resources.checkAcl(h), async replaceFile(parent, ...args) {
              try {
                await lease.assertCurrent(); check(); lease.assertHeld();
                if (!await samePathFd(root) || await pendingIntent(root.path)) { await parent.close(); return { state: "refused" }; }
                const latest = await fresh(root, lease, scheduled);
                if (!latest || latest.saved.expectedStamp !== current.saved.expectedStamp || latest.grant !== current.grant
                  || latest.identity !== current.identity || latest.facts.cancellationEpoch !== current.facts.cancellationEpoch
                  || retry(latest.facts).kind !== "transition") {
                  await parent.close(); return { state: "refused" };
                }
                await lease.assertCurrent(); check(); lease.assertHeld();
              } catch { await parent.close().catch(() => {}); return { state: "refused" }; }
              dispatched = true;
              return resources.replaceFile(parent, ...args);
            } };
            const outcome = await replacePrivateStateFile(root.path, `${scheduled.id}.job`, bytes,
              { device: String(named.dev), inode: String(named.ino) }, { signal, assertHeld: () => { check(); lease.assertHeld(); } }, guarded, { signal });
            if (outcome.state === "committed") { changed.push(scheduled.id); committedAny = true; dispatched = false; }
            else if (outcome.state === "publication_uncertain") return result("uncertain");
            else { dispatched = false; fail("stale_snapshot"); }
          } finally { bytes.fill(0); }
        }
        return result("complete");
      });
  } catch (error) {
    if (dispatched || committedAny) return result("uncertain");
    if (callerSignal?.aborted || error instanceof CredentialSnapshotError && error.category === "cancelled") fail("cancelled");
    if (error instanceof RetryError) throw new RetryError(error.category);
    if (error instanceof CredentialSnapshotError && error.category === "revocation_pending") fail("revocation_pending");
    return fail("state_unavailable");
  }
}
