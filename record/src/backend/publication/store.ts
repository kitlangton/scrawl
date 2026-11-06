import { DatabaseSync } from "node:sqlite";
import { isDeepStrictEqual } from "node:util";
import { isAbsolute, join, resolve } from "node:path";
import { ensurePrivateFile, ensurePrivateRoot, hostError, validatePrivateFile } from "../host/private-path.js";
import { initializePrivateSqlite } from "../host/sqlite-initialization.js";
import type { ProcessingInput } from "../processing/index.js";
import type { CapturePublishInput, PublicationReceipt } from "@apparent/engine-sdk/capture";
import type { Derivative2DocumentInput } from "@apparent/engine-sdk/media2";

// Frozen SDK 0.4.1 detached-document and publication receipt-count limits.
const maxPublicationDocumentBytes = 8 * 1024 * 1024;
const maxMaterialDocumentJsonBytes = 2 * 1024 * 1024;
const maxRecoveryReceiptCount = 32;
// Record operational limits are intentionally smaller than SDK document maxima.
// Retained terminal rows keep their descriptors; capacity exhaustion never deletes history.
export const maxRecoveryJsonBytes = 2 * 1024 * 1024;
const maxRetainedRecoveryJsonBytes = 16 * 1024 * 1024;

export interface PublicationOutputMetadata { requestId: string; requestVideoProof: boolean; processing: ProcessingInput }
export interface PublicationRecoveryDescriptor {
  schema: 1;
  captureId: string;
  leaseId: string;
  publicationId: string;
  planFile: CapturePublishInput["planFile"];
  planBytesBase64: string;
  materials: CapturePublishInput["materials"];
  derivatives?: Array<{ envelope: unknown; bytesBase64: string; requestedBundleMs: readonly number[] }>;
  derivatives2?: Array<{ envelope: unknown; bytesBase64: string; request: Derivative2DocumentInput["request"]; layoutBytesBase64: string | null }>;
}
export interface ProcessingCoordinatorRecord {
  schema: 1;
  captureId: string;
  requestId: string;
  materials: { captureId: string; materialsId: string; revision: string };
  leaseId: string;
  bundleId: string;
  bundlePath: string;
  plan: unknown;
  /** Immutable final frame/layout decision for a standard visual sampling plan. */
  visualSelection?: unknown;
}

export interface PublicationBinding {
  captureId: string;
  publicationId: string;
  planSha256: string;
  materials: { captureId: string; materialsId: string; revision: string };
  leaseId: string;
  bundleId: string;
  bundlePath: string;
  manifestSha256: string;
  /** Present on new reservations; ownerToken is always supplied at recovery time. */
  recoveryInput?: PublicationRecoveryDescriptor;
  output?: PublicationOutputMetadata;
  savedReceipt?: PublicationReceipt;
  savedObservedAt?: string;
  state: "reserved" | "dispatching" | "publishing" | "saved" | "failed" | "cancelled";
}

const version = 2;
const id = /^[0-9a-f]{32}$/u;
const hash = /^[0-9a-f]{64}$/u;
function valid(value: PublicationBinding): boolean {
  const core = !!value && id.test(value.captureId) && id.test(value.publicationId) &&
    hash.test(value.planSha256) && value.materials?.captureId === value.captureId &&
    id.test(value.materials.materialsId) && hash.test(value.materials.revision) && id.test(value.leaseId) &&
    typeof value.bundleId === "string" && value.bundleId.length > 0 && Buffer.byteLength(value.bundleId) <= 255 &&
    !/[\\/\p{Cc}]/u.test(value.bundleId) && value.bundleId !== "." && value.bundleId !== ".." &&
    typeof value.bundlePath === "string" && isAbsolute(value.bundlePath) && resolve(value.bundlePath) === value.bundlePath && value.bundlePath.length <= 4096 && !value.bundlePath.includes("\0") &&
    hash.test(value.manifestSha256) && ["reserved", "dispatching", "publishing", "saved", "failed", "cancelled"].includes(value.state);
  if (!core) return false;
  if (value.recoveryInput !== undefined && !validRecoveryDescriptor(value.recoveryInput, value)) return false;
  if (value.output !== undefined) {
    const output = value.output;
    if (!output || typeof output.requestId !== "string" || !output.requestId.trim() || output.requestId.length > 4096 || output.requestId === value.captureId ||
      typeof output.requestVideoProof !== "boolean" || !output.processing || output.processing.bundlePath !== value.bundlePath ||
      Buffer.byteLength(JSON.stringify(output)) > 1024 * 1024 || Object.keys(output).sort().join(",") !== "processing,requestId,requestVideoProof") return false;
  }
  if (value.savedReceipt !== undefined) {
    const receipt = value.savedReceipt;
    if (!receipt || receipt.captureId !== value.captureId || receipt.publicationId !== value.publicationId || receipt.planSha256 !== value.planSha256 ||
      receipt.bundleId !== value.bundleId || receipt.bundlePath !== value.bundlePath || receipt.manifestSha256 !== value.manifestSha256 ||
      receipt.artifactVersion !== 1 || receipt.artifactProvenance !== "native" || JSON.stringify(receipt.materials) !== JSON.stringify(value.materials) || !value.savedObservedAt ||
      !hash.test(receipt.inventorySha256) || !Number.isFinite(Date.parse(value.savedObservedAt)) ||
      Object.keys(receipt).sort().join(",") !== "artifactProvenance,artifactVersion,bundleId,bundlePath,captureId,inventorySha256,manifestSha256,materials,planSha256,publicationId") return false;
  }
  if ((value.savedReceipt !== undefined || value.savedObservedAt !== undefined) && (value.state !== "saved" || !value.savedReceipt || !value.savedObservedAt)) return false;
  if (value.state === "saved" && value.output && !value.savedReceipt) return false;
  return true;
}

function validRecoveryDescriptor(value: PublicationRecoveryDescriptor, binding: PublicationBinding): boolean {
  try {
    if (!jsonWithinLimit(value, maxRecoveryJsonBytes) || !value || value.schema !== 1 || value.captureId !== binding.captureId || value.publicationId !== binding.publicationId ||
        value.leaseId !== binding.leaseId || !value.planFile || !value.materials ||
        Object.keys(value).sort().join(",") !== ["captureId", "leaseId", "materials", "planBytesBase64", "planFile", "publicationId", "schema", ...(value.derivatives === undefined ? [] : ["derivatives"]), ...(value.derivatives2 === undefined ? [] : ["derivatives2"])].sort().join(",") ||
        Object.hasOwn(value as object, "ownerToken")) return false;
    const planBytes = decodedBytes(value.planBytesBase64, maxPublicationDocumentBytes, false);
    if (!planBytes) return false;
    if (value.derivatives !== undefined && !Array.isArray(value.derivatives)) return false;
    if (value.derivatives2 !== undefined && !Array.isArray(value.derivatives2)) return false;
    const derivatives = value.derivatives ?? [];
    const derivatives2 = value.derivatives2 ?? [];
    if (derivatives.length + derivatives2.length > maxRecoveryReceiptCount) return false;
    for (const item of derivatives) {
      if (!item || Object.keys(item).sort().join(",") !== "bytesBase64,envelope,requestedBundleMs" || !item.envelope ||
          !Array.isArray(item.requestedBundleMs) || !decodedBytes(item.bytesBase64, maxPublicationDocumentBytes, false)) return false;
    }
    for (const item of derivatives2) {
      if (!item || Object.keys(item).sort().join(",") !== "bytesBase64,envelope,layoutBytesBase64,request" || !item.envelope || !item.request ||
          (item.layoutBytesBase64 !== null && !decodedBytes(item.layoutBytesBase64, maxPublicationDocumentBytes, false)) ||
          !decodedBytes(item.bytesBase64, maxPublicationDocumentBytes, false)) return false;
    }
    return !hasOwnerTokenKey(value) && jsonWithinLimit(value.materials, maxMaterialDocumentJsonBytes);
  } catch { return false; }
}

/** Count serialized bytes without first allocating a potentially huge JSON string. */
function jsonWithinLimit(value: unknown, maximum: number): boolean {
  let remaining = maximum;
  const ancestors = new Set<object>();
  const spend = (bytes: number) => { remaining -= bytes; return remaining >= 0; };
  const text = (value: string): boolean => value.length <= remaining && spend(Buffer.byteLength(JSON.stringify(value)));
  const visit = (value: unknown, depth: number): boolean => {
    if (depth > 64) return false;
    if (value === null) return spend(4);
    if (typeof value === "string") return text(value);
    if (typeof value === "boolean") return spend(value ? 4 : 5);
    if (typeof value === "number") return Number.isFinite(value) && spend(String(value).length);
    if (!value || typeof value !== "object" || ancestors.has(value)) return false;
    ancestors.add(value);
    try {
      if (!spend(2)) return false;
      let count = 0;
      if (Array.isArray(value)) {
        if (value.length > remaining) return false;
        for (const item of value) {
          if ((count++ > 0 && !spend(1)) || !visit(item, depth + 1)) return false;
        }
      } else {
        for (const key in value) {
          if (!Object.hasOwn(value, key)) continue;
          const item = (value as Record<string, unknown>)[key];
          if (item === undefined) continue;
          if ((count++ > 0 && !spend(1)) || !text(key) || !spend(1) || !visit(item, depth + 1)) return false;
        }
      }
      return true;
    } finally { ancestors.delete(value); }
  };
  return visit(value, 0);
}

function hasOwnerTokenKey(value: unknown): boolean {
  if (!value || typeof value !== "object") return false;
  if (Array.isArray(value)) return value.some(hasOwnerTokenKey);
  return Object.entries(value).some(([key, item]) => key === "ownerToken" || hasOwnerTokenKey(item));
}

function decodedBytes(value: unknown, maximum: number, allowEmpty: boolean): Buffer | undefined {
  if (typeof value !== "string" || value.length > 4 * Math.ceil(maximum / 3) || !/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/u.test(value)) return undefined;
  const bytes = Buffer.from(value, "base64");
  if ((!allowEmpty && bytes.byteLength === 0) || bytes.byteLength > maximum || bytes.toString("base64") !== value) return undefined;
  return bytes;
}

/** Private SQLite journal; it reserves control metadata only, never bundle paths. */
export class SqlitePublicationBindingStore {
  readonly #root: string;
  readonly #path: string;
  constructor(privateRoot: string) {
    this.#root = privateRoot;
    this.#path = join(privateRoot, "publication-bindings.sqlite");
  }

  async get(captureId: string): Promise<PublicationBinding | undefined> {
    if (!id.test(captureId)) throw hostError();
    const db = await this.#open();
    try {
      const row = db.prepare("SELECT binding_json FROM publication_binding WHERE capture_id = ?").get(captureId);
      return row ? decode(row.binding_json) : undefined;
    } catch { throw hostError(); }
    finally { try { db.close(); } catch { /* keep sanitized storage errors */ } }
  }

  async list(): Promise<PublicationBinding[]> {
    const db = await this.#open();
    try { return (db.prepare("SELECT binding_json FROM publication_binding ORDER BY capture_id").all() as Array<{binding_json: unknown}>).map(row => decode(row.binding_json)); }
    catch { throw hostError(); }
    finally { try { db.close(); } catch { /* preserve result */ } }
  }

  async getProcessing(captureId: string): Promise<ProcessingCoordinatorRecord | undefined> {
    if (!id.test(captureId)) throw hostError();
    const db = await this.#open();
    try {
      const row = db.prepare("SELECT record_json FROM publication_processing WHERE capture_id = ?").get(captureId);
      return row ? decodeProcessing(row.record_json) : undefined;
    } catch { throw hostError(); }
    finally { try { db.close(); } catch { /* preserve result */ } }
  }

  async listProcessing(): Promise<ProcessingCoordinatorRecord[]> {
    const db = await this.#open();
    try { return (db.prepare("SELECT record_json FROM publication_processing ORDER BY capture_id").all() as Array<{record_json: unknown}>).map(row => decodeProcessing(row.record_json)); }
    catch { throw hostError(); }
    finally { try { db.close(); } catch { /* preserve result */ } }
  }

  async reserveProcessing(value: ProcessingCoordinatorRecord): Promise<ProcessingCoordinatorRecord> {
    if (!validProcessing(value)) throw hostError();
    const db = await this.#open();
    try {
      db.exec("BEGIN IMMEDIATE");
      const row = db.prepare("SELECT record_json FROM publication_processing WHERE capture_id = ? OR request_id = ?").get(value.captureId, value.requestId);
      if (row) {
        const existing = decodeProcessing(row.record_json);
        const { visualSelection: _existingSelection, ...originalExisting } = existing;
        const { visualSelection: requestedSelection, ...originalRequested } = value;
        if (!isDeepStrictEqual(originalExisting, originalRequested) || (requestedSelection !== undefined && !isDeepStrictEqual(existing.visualSelection, requestedSelection))) throw hostError();
        db.exec("COMMIT");
        return existing;
      }
      if (value.visualSelection !== undefined) throw hostError();
      db.prepare("INSERT INTO publication_processing (capture_id, request_id, record_json) VALUES (?, ?, ?)").run(value.captureId, value.requestId, JSON.stringify(value));
      db.exec("COMMIT");
      return structuredClone(value);
    } catch { try { db.exec("ROLLBACK"); } catch { /* transaction may have completed */ } throw hostError(); }
    finally { try { db.close(); } catch { /* preserve result */ } }
  }

  /** Freeze the result of one original standard sampling plan, never its requests. */
  async selectVisualProcessing(original: ProcessingCoordinatorRecord, selection: unknown): Promise<ProcessingCoordinatorRecord> {
    if (!validProcessing(original) || (original.plan as { kind?: string })?.kind !== "standard-visual" || selection === undefined)
      throw hostError();
    const db = await this.#open();
    try {
      db.exec("BEGIN IMMEDIATE");
      const row = db.prepare("SELECT record_json FROM publication_processing WHERE capture_id = ?").get(original.captureId);
      if (!row) throw hostError();
      const current = decodeProcessing(row.record_json);
      const { visualSelection: _current, ...currentIdentity } = current;
      const { visualSelection: _expected, ...expectedIdentity } = original;
      if (!isDeepStrictEqual(currentIdentity, expectedIdentity)) throw hostError();
      if (current.visualSelection !== undefined) {
        if (!isDeepStrictEqual(current.visualSelection, selection)) throw hostError();
        db.exec("COMMIT"); return current;
      }
      const next = { ...current, visualSelection: structuredClone(selection) };
      if (!validProcessing(next)) throw hostError();
      db.prepare("UPDATE publication_processing SET record_json = ? WHERE capture_id = ?").run(JSON.stringify(next), current.captureId);
      db.exec("COMMIT"); return structuredClone(next);
    } catch { try { db.exec("ROLLBACK"); } catch { /* preserve storage failure */ } throw hostError(); }
    finally { try { db.close(); } catch { /* preserve result */ } }
  }

  async reserve(binding: PublicationBinding): Promise<PublicationBinding> {
    if (!valid(binding) || binding.state !== "reserved") throw hostError();
    const db = await this.#open();
    try {
      db.exec("BEGIN IMMEDIATE");
      const found = db.prepare("SELECT binding_json FROM publication_binding WHERE capture_id = ? OR publication_id = ? OR bundle_path = ?").get(binding.captureId, binding.publicationId, binding.bundlePath);
      if (found) {
        const existing = decode(found.binding_json);
        db.exec("COMMIT");
        return existing;
      }
      if (binding.output && db.prepare("SELECT 1 AS found FROM publication_binding WHERE json_extract(binding_json, '$.output.requestId') = ? OR bundle_path = ?").get(binding.output.requestId, binding.bundlePath)) throw hostError();
      if (binding.recoveryInput) {
        // The write lock makes aggregate admission atomic across processes. Count bytes
        // in SQLite, without loading all retained descriptors into JavaScript.
        const row = db.prepare("SELECT COALESCE(SUM(length(CAST(json_extract(binding_json, '$.recoveryInput') AS BLOB))), 0) AS bytes FROM publication_binding").get();
        const retained = Number(row?.bytes);
        const added = Buffer.byteLength(JSON.stringify(binding.recoveryInput));
        if (!Number.isSafeInteger(retained) || retained < 0 || retained + added > maxRetainedRecoveryJsonBytes) throw hostError();
      }
      db.prepare("INSERT INTO publication_binding (capture_id, publication_id, bundle_path, binding_json) VALUES (?, ?, ?, ?)")
        .run(binding.captureId, binding.publicationId, binding.bundlePath, JSON.stringify(binding));
      db.exec("COMMIT");
      return structuredClone(binding);
    } catch {
      try { db.exec("ROLLBACK"); } catch { /* commit may have completed */ }
      try { db.close(); } catch { /* close may have completed */ }
      const existing = await this.get(binding.captureId).catch(() => undefined);
      if (existing) return existing;
      throw hostError();
    } finally { try { db.close(); } catch { /* already closed */ } }
  }

  async beginDispatch(binding: PublicationBinding): Promise<boolean> {
    return this.#change(binding, "reserved", "dispatching");
  }

  async observe(binding: PublicationBinding, state: "publishing" | "saved" | "failed" | "cancelled", receipt?: PublicationReceipt): Promise<boolean> {
    if (!valid(binding) || !["publishing", "saved", "failed", "cancelled"].includes(state)) throw hostError();
    const db = await this.#open();
    try {
      db.exec("BEGIN IMMEDIATE");
      const row = db.prepare("SELECT binding_json FROM publication_binding WHERE capture_id = ?").get(binding.captureId);
      if (!row) throw hostError();
      const current = decode(row.binding_json);
      if (!same(current, binding)) throw hostError();
      // Late in-flight replies cannot regress an already observed terminal fact.
      if (["saved", "failed", "cancelled"].includes(current.state) && current.state !== state) {
        db.exec("COMMIT");
        return false;
      }
      if (state === "saved" && (!receipt || receipt.captureId !== current.captureId || receipt.publicationId !== current.publicationId || receipt.planSha256 !== current.planSha256 ||
        receipt.bundleId !== current.bundleId || receipt.bundlePath !== current.bundlePath || receipt.manifestSha256 !== current.manifestSha256 || receipt.artifactVersion !== 1 ||
        receipt.artifactProvenance !== "native" || !hash.test(receipt.inventorySha256) || JSON.stringify(receipt.materials) !== JSON.stringify(current.materials) ||
        Object.keys(receipt).sort().join(",") !== ["artifactProvenance", "artifactVersion", "bundleId", "bundlePath", "captureId", "inventorySha256", "manifestSha256", "materials", "planSha256", "publicationId"].sort().join(","))) throw hostError();
      if (state === "saved" && current.savedReceipt) {
        if (!isDeepStrictEqual(current.savedReceipt, receipt)) throw hostError();
        db.exec("COMMIT");
        return true;
      }
      const next = state === "saved" ? { ...current, state, savedReceipt: structuredClone(receipt!), savedObservedAt: new Date().toISOString() } : { ...current, state };
      db.prepare("UPDATE publication_binding SET binding_json = ? WHERE capture_id = ?").run(JSON.stringify(next), binding.captureId);
      db.exec("COMMIT");
      return true;
    } catch { try { db.exec("ROLLBACK"); } catch { /* no transaction */ } throw hostError(); }
    finally { try { db.close(); } catch { /* preserve result */ } }
  }

  async #change(binding: PublicationBinding, from: PublicationBinding["state"], to: PublicationBinding["state"]): Promise<boolean> {
    const db = await this.#open();
    try {
      db.exec("BEGIN IMMEDIATE");
      const row = db.prepare("SELECT binding_json FROM publication_binding WHERE capture_id = ?").get(binding.captureId);
      if (!row) throw hostError();
      const current = decode(row.binding_json);
      if (!same(current, binding)) throw hostError();
      if (current.state !== from) { db.exec("COMMIT"); return false; }
      const next = { ...current, state: to };
      db.prepare("UPDATE publication_binding SET binding_json = ? WHERE capture_id = ?").run(JSON.stringify(next), binding.captureId);
      db.exec("COMMIT");
      return true;
    } catch { try { db.exec("ROLLBACK"); } catch { /* no transaction */ } throw hostError(); }
    finally { try { db.close(); } catch { /* preserve result */ } }
  }

  async #open(): Promise<DatabaseSync> {
    await ensurePrivateRoot(this.#root);
    await ensurePrivateFile(this.#path);
    for (const suffix of ["-wal", "-shm", "-journal"]) await validatePrivateFile(this.#path + suffix, true);
    let db: DatabaseSync | undefined;
    try {
      db = new DatabaseSync(this.#path);
      await initializePrivateSqlite(db);
      const current = Number(db.prepare("PRAGMA user_version").get()?.user_version ?? 0);
      if (current === 0) {
        db.exec("CREATE TABLE IF NOT EXISTS publication_binding (capture_id TEXT PRIMARY KEY, publication_id TEXT NOT NULL UNIQUE, bundle_path TEXT NOT NULL UNIQUE, binding_json TEXT NOT NULL) STRICT;");
        db.exec("CREATE TABLE IF NOT EXISTS publication_processing (capture_id TEXT PRIMARY KEY, request_id TEXT NOT NULL UNIQUE, record_json TEXT NOT NULL) STRICT; PRAGMA user_version = 2;");
      } else if (current === 1) {
        db.exec("CREATE TABLE IF NOT EXISTS publication_processing (capture_id TEXT PRIMARY KEY, request_id TEXT NOT NULL UNIQUE, record_json TEXT NOT NULL) STRICT; PRAGMA user_version = 2;");
      } else if (current !== version) throw hostError();
      return db;
    } catch { try { db?.close(); } catch { /* sanitized */ } throw hostError(); }
  }
}

function validProcessing(value: ProcessingCoordinatorRecord): boolean {
  try {
    return !!value && value.schema === 1 && id.test(value.captureId) && value.requestId.length > 0 && Buffer.byteLength(value.requestId) <= 4096 && !value.requestId.includes("\0") &&
      value.requestId !== value.captureId && value.materials.captureId === value.captureId && id.test(value.materials.materialsId) && hash.test(value.materials.revision) &&
      id.test(value.leaseId) && typeof value.bundleId === "string" && value.bundleId.length > 0 && Buffer.byteLength(value.bundleId) <= 255 &&
      !/[\\/\p{Cc}]/u.test(value.bundleId) && value.bundleId !== "." && value.bundleId !== ".." && typeof value.bundlePath === "string" && value.bundlePath.length <= 4096 &&
      !value.bundlePath.includes("\0") && isAbsolute(value.bundlePath) && resolve(value.bundlePath) === value.bundlePath && value.plan !== undefined && Buffer.byteLength(JSON.stringify(value)) <= 2 * 1024 * 1024 &&
      !JSON.stringify(value).includes('"ownerToken"') && (value.visualSelection === undefined || (value.plan as { kind?: string })?.kind === "standard-visual");
  } catch { return false; }
}

function decodeProcessing(raw: unknown): ProcessingCoordinatorRecord {
  try {
    if (typeof raw !== "string") throw new Error();
    const value = JSON.parse(raw) as ProcessingCoordinatorRecord;
    if (!validProcessing(value) || JSON.stringify(value) !== raw) throw new Error();
    return structuredClone(value);
  } catch { throw hostError(); }
}

function decode(raw: unknown): PublicationBinding {
  try {
    if (typeof raw !== "string") throw new Error();
    const value = JSON.parse(raw) as PublicationBinding;
    if (!valid(value) || JSON.stringify(value) !== raw) throw new Error();
    return structuredClone(value);
  } catch { throw hostError(); }
}

function same(a: PublicationBinding, b: PublicationBinding): boolean {
  return a.captureId === b.captureId && a.publicationId === b.publicationId && a.planSha256 === b.planSha256 &&
    a.materials.captureId === b.materials.captureId && a.materials.materialsId === b.materials.materialsId &&
    a.materials.revision === b.materials.revision && a.leaseId === b.leaseId && a.bundleId === b.bundleId && a.bundlePath === b.bundlePath &&
    a.manifestSha256 === b.manifestSha256 && JSON.stringify(a.recoveryInput) === JSON.stringify(b.recoveryInput) && JSON.stringify(a.output) === JSON.stringify(b.output);
}
