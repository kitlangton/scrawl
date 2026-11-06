import { DatabaseSync } from "node:sqlite";
import { isDeepStrictEqual } from "node:util";
import { isAbsolute, join, resolve } from "node:path";
import { ensurePrivateFile, ensurePrivateRoot, hostError, validatePrivateFile } from "../host/private-path.js";
import { initializePrivateSqlite } from "../host/sqlite-initialization.js";
import type { ProcessingInput } from "../processing/index.js";
import type { PublicationReceipt } from "@apparent/engine-sdk/capture";

export interface PublicationOutputMetadata { requestId: string; requestVideoProof: boolean; processing: ProcessingInput }

export interface PublicationBinding {
  captureId: string;
  publicationId: string;
  planSha256: string;
  materials: { captureId: string; materialsId: string; revision: string };
  leaseId: string;
  bundleId: string;
  bundlePath: string;
  manifestSha256: string;
  output?: PublicationOutputMetadata;
  savedReceipt?: PublicationReceipt;
  savedObservedAt?: string;
  state: "reserved" | "dispatching" | "publishing" | "saved" | "failed" | "cancelled";
}

const version = 1;
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
      if (current === 0) db.exec("CREATE TABLE IF NOT EXISTS publication_binding (capture_id TEXT PRIMARY KEY, publication_id TEXT NOT NULL UNIQUE, bundle_path TEXT NOT NULL UNIQUE, binding_json TEXT NOT NULL) STRICT; PRAGMA user_version = 1;");
      else if (current !== version) throw hostError();
      return db;
    } catch { try { db?.close(); } catch { /* sanitized */ } throw hostError(); }
  }
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
    a.manifestSha256 === b.manifestSha256 && JSON.stringify(a.output) === JSON.stringify(b.output);
}
