import { decodeStrictJson } from "../recipient/json.ts";

export type JobStatus = "pending" | "failed" | "ready" | "awaiting_share" | "cancelled" | "expired";
export interface Asset { path: string; size: number; sha256: string }
export interface SizeFailure { path: string; size: number; limit: number }
export interface EncryptedJob { context: { origin: string; accountId: string; captureId: string; senderGrantId: string; group?: string }; recipientGrants: string[] }
/** Unknown legacy fields are retained byte-semantically as JSON values on encode. */
export type Job = {
  id: string; bundle: string; server: string; generation: string; created: number;
  attempts: number; nextAttempt: number; status: JobStatus; error: string | null;
  sizeFailure?: SizeFailure; files: Asset[]; shared: boolean; publicCopy: boolean;
  encrypted: boolean; encryption: EncryptedJob | null; grouped: boolean;
} & Record<string, unknown>;

const statuses = new Set<JobStatus>(["pending", "failed", "ready", "awaiting_share", "cancelled", "expired"]);
const object = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null && !Array.isArray(v);
const str = (v: unknown) => typeof v === "string";
const uint = (v: unknown): v is number => typeof v === "number" && Number.isSafeInteger(v) && v >= 0;
function invalid(): never { throw new Error("invalid_queue_record"); }
function validateAsset(v: unknown): Asset {
  if (!object(v) || !str(v.path) || !uint(v.size) || !str(v.sha256)) return invalid();
  return { path: v.path, size: v.size, sha256: v.sha256 };
}
function validateEncrypted(v: unknown): EncryptedJob | null {
  if (v === null) return null;
  if (!object(v) || Object.keys(v).some(k => !["context", "recipientGrants"].includes(k)) || !object(v.context) || !Array.isArray(v.recipientGrants)) return invalid();
  const c = v.context;
  if (Object.keys(c).some(k => !["origin", "accountId", "captureId", "senderGrantId", "group"].includes(k)) || ![c.origin,c.accountId,c.captureId,c.senderGrantId].every(str) || (c.group !== undefined && c.group !== null && !str(c.group)) || !v.recipientGrants.every(str)) return invalid();
  const normalized = structuredClone(v) as unknown as EncryptedJob;
  // Rust's Option group accepts null and serializes None without this field.
  if (c.group === null) delete normalized.context.group;
  return normalized;
}
export function decodeJob(bytes: Uint8Array): Job {
  const v = decodeStrictJson(bytes, 2_000_000);
  if (!object(v) || !str(v.id) || !/^[0-9a-f]{32}$/.test(v.id) || !str(v.bundle) || !str(v.server) || !str(v.generation) || !uint(v.created) || !uint(v.attempts) || v.attempts > 0xffffffff || !uint(v.nextAttempt) || !str(v.status) || !statuses.has(v.status as JobStatus) || !(v.error === undefined || v.error === null || str(v.error))) return invalid();
  if ((v.files !== undefined && (!Array.isArray(v.files) || !v.files.every(x => { try { validateAsset(x); return true; } catch { return false; } }))) || (v.shared !== undefined && typeof v.shared !== "boolean") || (v.publicCopy !== undefined && typeof v.publicCopy !== "boolean") || (v.encrypted !== undefined && typeof v.encrypted !== "boolean") || (v.grouped !== undefined && typeof v.grouped !== "boolean")) return invalid();
  let sizeFailure: SizeFailure | undefined;
  if (v.sizeFailure !== undefined && v.sizeFailure !== null) {
    if (!object(v.sizeFailure) || !str(v.sizeFailure.path) || !uint(v.sizeFailure.size) || !uint(v.sizeFailure.limit)) return invalid();
    sizeFailure = { path: v.sizeFailure.path, size: v.sizeFailure.size as number, limit: v.sizeFailure.limit as number };
  }
  const encryption = v.encryption === undefined ? null : validateEncrypted(v.encryption);
  if (v.sizeFailure === null) delete v.sizeFailure;
  // Rust serde defaults these exact fields for old jobs.
  return { ...v, id: v.id, bundle: v.bundle, server: v.server, generation: v.generation, created: v.created, attempts: v.attempts, nextAttempt: v.nextAttempt, status: v.status as JobStatus, error: v.error ?? null, ...(sizeFailure ? { sizeFailure } : {}), files: (v.files ?? []).map(validateAsset), shared: v.shared ?? false, publicCopy: v.publicCopy ?? false, encrypted: v.encrypted ?? false, encryption, grouped: v.grouped ?? false } as Job;
}
export function encodeJob(job: Job): Uint8Array {
  // Re-decode to enforce the same bounded wire contract before serialization.
  const normalized = decodeJob(new TextEncoder().encode(JSON.stringify(job)));
  return new TextEncoder().encode(JSON.stringify(normalized));
}
