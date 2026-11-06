import type { SizeFailure } from "./job.ts";

export type UploadErrorCode =
  | "auth_required"
  | "file_too_large"
  | "encryption_action_required"
  | "encryption_pending_recipient"
  | "quota_exceeded"
  | "unsupported_asset"
  | "upload_failed";

export type DiagnosticStage = "upload" | "identity" | "snapshot" | "encrypt" | "spool_receipt" | "spool_validate";
export type DiagnosticReason =
  | "unclassified" | "unknown_asset" | "unsafe_notes_log" | "unsafe_publication_lock"
  | "symlink" | "unsafe_directory" | "allow_acl" | "acl_unverified" | "digest"
  | "source_changed" | "asset_reference" | "identity_missing" | "identity_file"
  | "peer_not_enrolled" | "key_binding" | "peer_authorization" | "scratch_not_empty"
  | "io_permission" | "io_not_found" | "io_invalid_input" | "io_other";

export interface UploadCauseFact { message: string; ioKind?: "permission_denied" | "not_found" | "invalid_input" | "other" }
export interface SizeFailureFact { path: string; size: number; limit: number }
export interface UploadErrorFacts extends UploadCauseFact {
  /** Descendant causes, outermost to innermost. The root is `message` above. */
  causes: readonly UploadCauseFact[];
  sizeFailure?: SizeFailureFact;
}
export interface UploadFailureProjection {
  error: UploadErrorCode;
  diagnostic: { stage: DiagnosticStage; reason: DiagnosticReason };
  sizeFailure?: SizeFailure;
}

// The queue wire record caps JSON at 2 MB. Tighter per-message and cause limits
// bound projection work while leaving ample room for ordinary contextual errors.
const MAX_MESSAGE_CHARS = 4096;
const MAX_CAUSES = 64;
const stages: Record<string, DiagnosticStage> = {
  "encryption_identity_missing: approve this device with hostedEncryption enabled; never replace lost keys implicitly": "identity",
  "encryption_identity_unavailable: local key is missing, invalid or not private; no plaintext fallback": "identity",
  encryption_snapshot_failed: "snapshot", encryption_pack_failed: "encrypt",
  encryption_spool_receipt_failed: "spool_receipt",
  encryption_spool_directory_unsafe: "spool_validate", encryption_spool_invalid: "spool_validate",
};
const reasons: Record<string, DiagnosticReason> = {
  "unsupported_asset: bundle contains an unknown asset; no files were omitted": "unknown_asset",
  unsupported_asset: "unknown_asset", "Unsafe local notes diagnostic": "unsafe_notes_log",
  "Unsafe bundle publication lock": "unsafe_publication_lock", "Symlink path refused": "symlink",
  "Symlink asset directory refused": "symlink", "Unsafe shared or unowned directory tree": "unsafe_directory",
  "Directory ACL grants are not supported for hosted transfers": "allow_acl",
  "Could not verify directory ACL": "acl_unverified", "Could not verify directory ACL entries": "acl_unverified",
  "SHA-256 tool is required": "digest", "SHA-256 failed": "digest", "Invalid SHA-256 output": "digest",
  "Asset changed during snapshot": "source_changed", encryption_source_changed: "source_changed",
  "Unsafe or missing bundle asset reference": "asset_reference", "Invalid asset reference": "asset_reference",
  "Missing frame asset": "asset_reference",
  "encryption_identity_missing: approve this device with hostedEncryption enabled; never replace lost keys implicitly": "identity_missing",
  "encryption_identity_unavailable: local key is missing, invalid or not private; no plaintext fallback": "identity_file",
  "encryption_approval_required: this grant has no approved endpoint key": "peer_not_enrolled",
  encryption_key_binding_mismatch: "key_binding", invalid_encryption_authorization: "peer_authorization",
  encryption_scratch_not_empty: "scratch_not_empty",
};
const text = (v: unknown): v is string => typeof v === "string" && v.length <= MAX_MESSAGE_CHARS;
function malformed(): never { throw new Error("invalid_upload_error_facts"); }

function validate(facts: UploadErrorFacts): void {
  if (!facts || typeof facts !== "object" || !text(facts.message) || !Array.isArray(facts.causes) || facts.causes.length > MAX_CAUSES) malformed();
  for (const cause of [facts, ...facts.causes]) {
    if (!cause || typeof cause !== "object" || !text(cause.message) ||
      (cause.ioKind !== undefined && !["permission_denied", "not_found", "invalid_input", "other"].includes(cause.ioKind))) malformed();
  }
  if (facts.sizeFailure !== undefined) {
    const s = facts.sizeFailure;
    if (!s || typeof s !== "object" || typeof s.path !== "string" || s.path.length > 4096 ||
      !Number.isSafeInteger(s.size) || s.size < 0 || !Number.isSafeInteger(s.limit) || s.limit < 0) malformed();
  }
}

function resultCode(f: UploadErrorFacts): UploadErrorCode {
  if (f.sizeFailure !== undefined) return "file_too_large";
  if (f.message.startsWith("unsupported_asset") || f.causes.some(c => c.message.startsWith("unsupported_asset"))) return "unsupported_asset";
  const message = f.message;
  if (message.includes("encryption_pending_recipient")) return "encryption_pending_recipient";
  if (message.includes("encryption_") || message.includes("encrypted_")) return "encryption_action_required";
  for (const category of ["file_too_large", "quota_exceeded", "unsupported_asset"] as const) {
    if (message.includes(category)) return category;
  }
  return message.includes("auth_required") || message.includes("unauthorized") || message.includes("forbidden")
    ? "auth_required" : "upload_failed";
}

function diagnostic(f: UploadErrorFacts): { stage: DiagnosticStage; reason: DiagnosticReason } {
  let stage: DiagnosticStage = "upload";
  let reason: DiagnosticReason = "unclassified";
  for (const cause of [f, ...f.causes]) {
    if (Object.hasOwn(stages, cause.message)) stage = stages[cause.message]!;
    if (Object.hasOwn(reasons, cause.message)) reason = reasons[cause.message]!;
    if (reason === "unclassified" && cause.ioKind) {
      reason = ({ permission_denied: "io_permission", not_found: "io_not_found", invalid_input: "io_invalid_input", other: "io_other" } as const)[cause.ioKind];
    }
  }
  return { stage, reason };
}

/** Projects caller-sanitized structured facts; arbitrary errors and raw messages never escape. */
export function projectUploadFailure(facts: UploadErrorFacts): UploadFailureProjection {
  validate(facts);
  const error = resultCode(facts);
  const sizeFailure = error === "file_too_large" && facts.sizeFailure
    ? { path: facts.sizeFailure.path, size: facts.sizeFailure.size, limit: facts.sizeFailure.limit }
    : undefined;
  return { error, diagnostic: diagnostic(facts), ...(sizeFailure ? { sizeFailure } : {}) };
}
