import assert from "node:assert/strict";
import test from "node:test";
import { decodeJob, encodeJob } from "../../src/queue/job.ts";
import { cancel, completeUpload, expire, recover, retry, type FreshQueueSnapshot } from "../../src/queue/transitions.ts";

const legacy = { id: "a".repeat(32), bundle: "/synthetic/capture.apparent", server: "https://api.example.test", generation: "b".repeat(32), created: 100, attempts: 2, nextAttempt: 110, status: "pending", error: null, files: [{ path: "summary.md", size: 12, sha256: "c".repeat(64) }], shared: false, publicCopy: false, encrypted: true, encryption: { context: { origin: "https://api.example.test", accountId: "synthetic-account", captureId: "a".repeat(32), senderGrantId: "synthetic-grant", group: "evidence" }, recipientGrants: ["synthetic-reader"] }, grouped: true };
const job = (extra: Record<string, unknown> = {}) => decodeJob(new TextEncoder().encode(JSON.stringify({ ...legacy, ...extra })));
const snap = (j = job(), extra: Partial<FreshQueueSnapshot> = {}): FreshQueueSnapshot => ({ job: j, enabled: true, server: j.server, generation: j.generation, encryptionEnabled: j.encrypted, sharingEnabled: true, now: 120, retentionSeconds: 1000, cancellationEpoch: "epoch-1", ...extra });

test("round trips legacy, encrypted, and public-copy queue records while retaining unknown supported fields", () => {
  const full = job({ futureLegacyField: { preserved: [1, "x"] }, sizeFailure: { path: "recording.mp4", size: 99, limit: 90 } });
  assert.deepEqual(decodeJob(encodeJob(full)), full);
  const old = decodeJob(new TextEncoder().encode(JSON.stringify({ ...legacy, files: undefined, shared: undefined, publicCopy: undefined, encrypted: undefined, encryption: undefined, grouped: undefined })));
  assert.deepEqual([old.files, old.shared, old.publicCopy, old.encrypted, old.encryption, old.grouped], [[], false, false, false, null, false]);
  const copy = job({ status: "awaiting_share", shared: true, publicCopy: true, encrypted: false, encryption: null, grouped: false });
  assert.equal(decodeJob(encodeJob(copy)).status, "awaiting_share");
  for (const status of ["ready", "cancelled", "expired"]) assert.equal(job({ status }).status, status);
});

test("bounded strict parsing fails closed without echoing private record data", () => {
  const secret = "/private/path/secret-capture";
  assert.throws(() => decodeJob(new TextEncoder().encode(`{"${secret}":1}`)), e => e instanceof Error && !e.message.includes(secret));
  const duplicated = JSON.stringify(legacy).replace('"id":', '"id":"' + "a".repeat(32) + '","id":');
  assert.throws(() => decodeJob(new TextEncoder().encode(duplicated)));
  assert.throws(() => decodeJob(new Uint8Array(128 * 1024 + 1)));
  assert.throws(() => decodeJob(new TextEncoder().encode(JSON.stringify({ ...legacy, status: "unknown" }))));
});

test("retry honors retention, enabled gates, grant generation and status semantics", () => {
  const eligible = job({ status: "failed", attempts: 7, nextAttempt: 300, error: "file_too_large", sizeFailure: { path: "recording.mp4", size: 100, limit: 90 } });
  const result = retry(snap(eligible));
  assert.equal(result.kind, "transition");
  if (result.kind === "transition") assert.deepEqual([result.job.status, result.job.attempts, result.job.nextAttempt, result.job.error, result.job.sizeFailure], ["pending", 0, 0, null, undefined]);
  assert.equal(retry(snap(eligible, { now: 2000 })).kind, "unchanged");
  assert.equal(retry(snap(eligible, { enabled: false })).kind, "unchanged");
  assert.equal(retry(snap(eligible, { generation: "d".repeat(32) })).kind, "unchanged");
  assert.equal(retry(snap(job({ status: "cancelled" }))).kind, "unchanged");
});

test("cancel, expiry, recovery and stale upload completion preserve race fences", () => {
  assert.equal(cancel(job({ status: "ready" }), false).kind, "unchanged");
  const cancelled = cancel(job(), false); assert.equal(cancelled.kind, "transition");
  if (cancelled.kind === "transition") assert.equal(cancelled.job.status, "cancelled");
  assert.equal(expire(snap(job(), { now: 1200, retentionSeconds: 100 })).kind, "transition");
  assert.equal(recover(snap(), "old-epoch").kind, "unchanged");
  assert.equal(recover(snap(job({ status: "failed" })), "epoch-1").kind, "unchanged");
  const captured = { id: "a".repeat(32), bundle: "/synthetic/capture.apparent", server: "https://api.example.test", generation: "b".repeat(32), created: 100, encrypted: true, publicCopy: false, status: "pending" as const, cancellationEpoch: "epoch-1" };
  assert.equal(completeUpload(snap(job({ status: "cancelled" })), captured, { ok: true }).kind, "unchanged");
  assert.equal(completeUpload(snap(job(), { generation: "d".repeat(32) }), captured, { ok: true }).kind, "unchanged");
  const failed = completeUpload(snap(job()), captured, { ok: false, error: "file_too_large", sizeFailure: { path: "x", size: 10, limit: 9 } });
  assert.equal(failed.kind, "transition"); if (failed.kind === "transition") assert.deepEqual([failed.job.status, failed.job.error, failed.job.sizeFailure?.size], ["failed", "file_too_large", 10]);
});


test("preserves serde option defaults and the donor's two-megabyte record bound", () => {
  const decoded = job({ error: undefined, sizeFailure: null, extension: "x".repeat(140_000) });
  assert.equal(decoded.error, null); assert.equal(decoded.sizeFailure, undefined);
  assert.deepEqual(decodeJob(encodeJob(decoded)), decoded);
  assert.throws(() => decodeJob(new Uint8Array(2_000_001)));
  assert.throws(() => job({ attempts: 0x1_0000_0000 }));
});
test("clock rollback retains jobs and pending-recipient waits do not consume attempts or increase backoff", () => {
  assert.equal(retry(snap(job(), { now: 90 })).kind, "transition");
  const j = job({ attempts: 7 }); const captured = { ...j, cancellationEpoch: "epoch-1" };
  const waiting = completeUpload(snap(j), captured, { ok: false, error: "encryption_pending_recipient" });
  assert.equal(waiting.kind, "transition");
  if (waiting.kind === "transition") assert.deepEqual([waiting.job.status, waiting.job.attempts, waiting.job.nextAttempt], ["pending", 7, 120 + 5 * 2 ** 7]);
  assert.equal(completeUpload(snap(j, { cancellationEpoch: "new-epoch" }), captured, { ok: true }).kind, "unchanged");
  assert.equal(completeUpload(snap(job({ status: "ready" })), { ...captured, status: "ready" }, { ok: true }).kind, "unchanged");
  assert.throws(() => completeUpload(snap(j), captured, { ok: false, error: "private remote details" }), { message: "invalid_queue_result" });
});
