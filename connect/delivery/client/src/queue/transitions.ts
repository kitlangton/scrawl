import type { Job, JobStatus } from "./job.ts";

export interface FreshQueueSnapshot {
  job: Job;
  enabled: boolean;
  server: string;
  generation: string;
  encryptionEnabled: boolean;
  sharingEnabled: boolean;
  now: number;
  retentionSeconds: number;
  cancellationEpoch: string;
}
export type Decision = { kind: "transition"; job: Job; reason: string } | { kind: "unchanged"; reason: string };
const copy = (j: Job): Job => structuredClone(j);
function unchanged(reason: string): Decision { return { kind: "unchanged", reason }; }
function transition(job: Job, reason: string): Decision { return { kind: "transition", job, reason }; }
function current(s: FreshQueueSnapshot): boolean {
  const j = s.job;
  return s.enabled && j.server === s.server && j.generation === s.generation &&
    (j.publicCopy ? s.sharingEnabled && !j.encrypted && j.shared : j.encrypted === s.encryptionEnabled);
}
function retained(s: FreshQueueSnapshot): boolean { return Math.max(0, s.now - s.job.created) <= s.retentionSeconds; }

/** Mirrors explicit `retry`: only active pending/failed records within retention reset. */
export function retry(snapshot: FreshQueueSnapshot): Decision {
  const j = snapshot.job;
  if (!current(snapshot)) return unchanged("disabled_or_stale_grant");
  if (!retained(snapshot)) return unchanged("outside_retention");
  if (j.status !== "pending" && j.status !== "failed") return unchanged("status_not_retryable");
  const next = copy(j); next.status = "pending"; next.attempts = 0; next.nextAttempt = 0; next.error = null; delete next.sizeFailure;
  return transition(next, "retry_reset");
}

/** Disable/logout cancellation wins over any later worker completion. */
export function cancel(job: Job, enabled: boolean): Decision {
  if (job.status === "ready") return unchanged("already_ready");
  if (enabled) return unchanged("still_enabled");
  if (job.status === "cancelled") return unchanged("already_cancelled");
  const next = copy(job); next.status = "cancelled";
  return transition(next, "disabled_or_revoked");
}

/** Expiry is committed only after a fresh active-incarnation check by the caller. */
export function expire(snapshot: FreshQueueSnapshot): Decision {
  if (!current(snapshot)) return unchanged("disabled_or_stale_grant");
  if (snapshot.job.status !== "pending") return unchanged("status_not_expirable");
  if (retained(snapshot)) return unchanged("within_retention");
  const next = copy(snapshot.job); next.status = "expired";
  return transition(next, "retention_elapsed");
}

/** Recovery of an in-flight pending record preserves ID, manifest and sealed intent. */
export function recover(snapshot: FreshQueueSnapshot, observedCancellationEpoch: string): Decision {
  if (observedCancellationEpoch !== snapshot.cancellationEpoch) return unchanged("cancellation_epoch_changed");
  if (!current(snapshot)) return unchanged("disabled_or_stale_grant");
  if (!retained(snapshot)) return expire(snapshot);
  if (snapshot.job.status !== "pending" || snapshot.job.nextAttempt > snapshot.now) return unchanged("not_due");
  return transition(copy(snapshot.job), "pending_recovered");
}

/** Fence late network results against fresh status, generation and cancellation. */
export function completeUpload(snapshot: FreshQueueSnapshot, captured: Pick<Job, "id" | "bundle" | "server" | "generation" | "created" | "encrypted" | "publicCopy" | "status"> & { cancellationEpoch: string }, result: { ok: true } | { ok: false; error: string; sizeFailure?: Job["sizeFailure"] }): Decision {
  const j = snapshot.job;
  if (snapshot.cancellationEpoch !== captured.cancellationEpoch || !current(snapshot) || j.id !== captured.id || j.bundle !== captured.bundle || j.server !== captured.server || j.generation !== captured.generation || j.created !== captured.created || j.encrypted !== captured.encrypted || j.publicCopy !== captured.publicCopy || j.status !== captured.status || j.status !== "pending") return unchanged("stale_completion");
  const next = copy(j); next.attempts++;
  if (result.ok) { next.status = "ready"; next.error = null; delete next.sizeFailure; return transition(next, "upload_ready"); }
  if (!["auth_required", "file_too_large", "encryption_action_required", "encryption_pending_recipient", "quota_exceeded", "unsupported_asset", "upload_failed"].includes(result.error)) throw new Error("invalid_queue_result");
  next.error = result.error; if (result.error === "file_too_large") next.sizeFailure = result.sizeFailure; else delete next.sizeFailure;
  if (result.error === "encryption_pending_recipient") next.attempts--;
  else if (["auth_required", "file_too_large", "encryption_action_required"].includes(result.error) || next.attempts >= 8) next.status = "failed";
  next.nextAttempt = snapshot.now + Math.min(5 * 2 ** Math.min(next.attempts, 10), 3600);
  return transition(next, "upload_failed");
}

export type { JobStatus };
