import assert from "node:assert/strict";
import test from "node:test";
import { projectUploadFailure, type UploadErrorFacts } from "../../src/queue/upload-failure.ts";
import { decodeJob } from "../../src/queue/job.ts";
import { completeUpload, type FreshQueueSnapshot } from "../../src/queue/transitions.ts";

const facts = (message: string, causes: UploadErrorFacts["causes"] = [], extra: Partial<UploadErrorFacts> = {}): UploadErrorFacts => ({ message, causes, ...extra });
const legacyReasons: [string, string][] = [
  ["unsupported_asset: bundle contains an unknown asset; no files were omitted", "unknown_asset"],
  ["Unsafe local notes diagnostic", "unsafe_notes_log"], ["Unsafe bundle publication lock", "unsafe_publication_lock"],
  ["Symlink path refused", "symlink"], ["Symlink asset directory refused", "symlink"],
  ["Unsafe shared or unowned directory tree", "unsafe_directory"],
  ["Directory ACL grants are not supported for hosted transfers", "allow_acl"],
  ["Could not verify directory ACL", "acl_unverified"], ["Could not verify directory ACL entries", "acl_unverified"],
  ["SHA-256 tool is required", "digest"], ["SHA-256 failed", "digest"], ["Invalid SHA-256 output", "digest"],
  ["Asset changed during snapshot", "source_changed"], ["encryption_source_changed", "source_changed"],
  ["Unsafe or missing bundle asset reference", "asset_reference"], ["Invalid asset reference", "asset_reference"],
  ["Missing frame asset", "asset_reference"],
  ["encryption_identity_missing: approve this device with hostedEncryption enabled; never replace lost keys implicitly", "identity_missing"],
  ["encryption_identity_unavailable: local key is missing, invalid or not private; no plaintext fallback", "identity_file"],
  ["encryption_approval_required: this grant has no approved endpoint key", "peer_not_enrolled"],
  ["encryption_key_binding_mismatch", "key_binding"], ["invalid_encryption_authorization", "peer_authorization"],
  ["encryption_scratch_not_empty", "scratch_not_empty"],
];

test("diagnostic exact matches and stages match the Rust donor", () => {
  const stageCases: [string, string][] = [
    ["encryption_identity_missing: approve this device with hostedEncryption enabled; never replace lost keys implicitly", "identity"],
    ["encryption_identity_unavailable: local key is missing, invalid or not private; no plaintext fallback", "identity"],
    ["encryption_snapshot_failed", "snapshot"], ["encryption_pack_failed", "encrypt"],
    ["encryption_spool_receipt_failed", "spool_receipt"], ["encryption_spool_directory_unsafe", "spool_validate"],
    ["encryption_spool_invalid", "spool_validate"],
  ];
  for (const [message, stage] of stageCases) assert.equal(projectUploadFailure(facts("secret", [{ message }])).diagnostic.stage, stage);
  for (const [message, reason] of legacyReasons) assert.equal(projectUploadFailure(facts("secret", [{ message }])).diagnostic.reason, reason, message);
  assert.deepEqual(projectUploadFailure(facts("secret", [{ message: "encryption_snapshot_failed with secret" }])).diagnostic, { stage: "upload", reason: "unclassified" });
});

test("stage and reason overwrite in outer-to-inner order; IO is only an unclassified fallback", () => {
  assert.deepEqual(projectUploadFailure(facts("x", [
    { message: "encryption_snapshot_failed" }, { message: "encryption_pack_failed" }, { message: "Invalid asset reference" },
  ])).diagnostic, { stage: "encrypt", reason: "asset_reference" });
  assert.equal(projectUploadFailure(facts("x", [
    { message: "encryption_snapshot_failed", ioKind: "permission_denied" },
  ])).diagnostic.reason, "io_permission");
  assert.equal(projectUploadFailure(facts("x", [
    { message: "Invalid asset reference", ioKind: "not_found" },
  ])).diagnostic.reason, "asset_reference");
  assert.deepEqual(projectUploadFailure(facts("private-path token", [], { ioKind: "permission_denied" })).diagnostic,
    { stage: "upload", reason: "io_permission" });
});

test("diagnostic maps refuse inherited property names and include the root error", () => {
  for (const message of ["constructor", "toString", "__proto__", "hasOwnProperty"]) {
    assert.deepEqual(projectUploadFailure(facts(message, [{ message }])).diagnostic,
      { stage: "upload", reason: "unclassified" });
  }
  assert.deepEqual(projectUploadFailure(facts("encryption_snapshot_failed")).diagnostic,
    { stage: "snapshot", reason: "unclassified" });
  assert.equal(projectUploadFailure(facts("unsupported_asset encryption_snapshot_failed")).error, "unsupported_asset");
});

test("result classification preserves donor precedence and never returns source messages", () => {
  const cases: [UploadErrorFacts, string][] = [
    [facts("encryption_action_required", [], { sizeFailure: { path: "/private/path", size: 99, limit: 90 } }), "file_too_large"],
    [facts("encryption_pending_recipient encryption_action_required", [{ message: "unsupported_asset: private detail" }]), "unsupported_asset"],
    [facts("encryption_action_required", [{ message: "unsupported_asset" }]), "unsupported_asset"],
    [facts("encryption_pending_recipient encryption_action_required"), "encryption_pending_recipient"],
    [facts("encryption_action_required file_too_large"), "encryption_action_required"],
    [facts("file_too_large quota_exceeded"), "file_too_large"], [facts("quota_exceeded unsupported_asset"), "quota_exceeded"],
    [facts("unsupported_asset forbidden"), "unsupported_asset"], [facts("unauthorized"), "auth_required"],
    [facts("unrecognized secret token"), "upload_failed"],
  ];
  for (const [input, expected] of cases) assert.equal(projectUploadFailure(input).error, expected);
  const result = projectUploadFailure(facts("secret path", [], { sizeFailure: { path: "/private/path/token", size: 99, limit: 90 } }));
  assert.deepEqual(result.sizeFailure, { path: "/private/path/token", size: 99, limit: 90 });
  assert.equal(JSON.stringify(result.diagnostic).includes("secret path"), false);
  assert.equal(JSON.stringify(result.diagnostic).includes("token"), false);
});

test("malformed facts fail with a fixed message and existing transition owns attempts and terminal states", () => {
  assert.throws(() => projectUploadFailure(facts("x".repeat(4097))), { message: "invalid_upload_error_facts" });
  assert.throws(() => projectUploadFailure(facts("x", Array.from({ length: 65 }, () => ({ message: "x" })))), { message: "invalid_upload_error_facts" });
  assert.throws(() => projectUploadFailure(facts("x", [], { sizeFailure: { path: "x", size: Number.MAX_SAFE_INTEGER + 1, limit: 1 } })), { message: "invalid_upload_error_facts" });

  const base = { id: "a".repeat(32), bundle: "/synthetic/capture.apparent", server: "https://api.example.test", generation: "b".repeat(32), created: 100, attempts: 7, nextAttempt: 0, status: "pending", error: null, files: [], shared: false, publicCopy: false, encrypted: true, encryption: null, grouped: false };
  const job = decodeJob(new TextEncoder().encode(JSON.stringify(base)));
  const snapshot: FreshQueueSnapshot = { job, enabled: true, server: job.server, generation: job.generation, encryptionEnabled: true, sharingEnabled: true, now: 120, retentionSeconds: 1000, cancellationEpoch: "epoch" };
  const captured = { ...job, cancellationEpoch: "epoch" };
  const pending = projectUploadFailure(facts("encryption_pending_recipient"));
  const wait = completeUpload(snapshot, captured, { ok: false, error: pending.error });
  assert.equal(wait.kind, "transition");
  if (wait.kind === "transition") assert.deepEqual([wait.job.status, wait.job.attempts, wait.job.error, wait.job.nextAttempt], ["pending", 7, "encryption_pending_recipient", 120 + 5 * 2 ** 7]);
  const terminal = projectUploadFailure(facts("auth_required"));
  const failed = completeUpload(snapshot, captured, { ok: false, error: terminal.error });
  assert.equal(failed.kind, "transition");
  if (failed.kind === "transition") assert.deepEqual([failed.job.status, failed.job.attempts, failed.job.error], ["failed", 8, "auth_required"]);
});
