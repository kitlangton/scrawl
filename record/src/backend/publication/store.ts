import { DatabaseSync } from "node:sqlite";
import { isAbsolute, join, resolve } from "node:path";
import { ensurePrivateFile, ensurePrivateRoot, hostError, validatePrivateFile } from "../host/private-path.js";
import { initializePrivateSqlite } from "../host/sqlite-initialization.js";

export interface PublicationBinding {
  captureId: string;
  publicationId: string;
  planSha256: string;
  materials: { captureId: string; materialsId: string; revision: string };
  leaseId: string;
  bundleId: string;
  bundlePath: string;
  manifestSha256: string;
  state: "reserved" | "dispatching" | "publishing" | "saved" | "failed" | "cancelled";
}

const version = 1;
const id = /^[0-9a-f]{32}$/u;
const hash = /^[0-9a-f]{64}$/u;
function valid(value: PublicationBinding): boolean {
  return !!value && id.test(value.captureId) && id.test(value.publicationId) &&
    hash.test(value.planSha256) && value.materials?.captureId === value.captureId &&
    id.test(value.materials.materialsId) && hash.test(value.materials.revision) && id.test(value.leaseId) &&
    typeof value.bundleId === "string" && value.bundleId.length > 0 && Buffer.byteLength(value.bundleId) <= 255 &&
    !/[\\/\p{Cc}]/u.test(value.bundleId) && value.bundleId !== "." && value.bundleId !== ".." &&
    typeof value.bundlePath === "string" && isAbsolute(value.bundlePath) && resolve(value.bundlePath) === value.bundlePath && value.bundlePath.length <= 4096 && !value.bundlePath.includes("\0") &&
    hash.test(value.manifestSha256) && ["reserved", "dispatching", "publishing", "saved", "failed", "cancelled"].includes(value.state);
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

  async observe(binding: PublicationBinding, state: "publishing" | "saved" | "failed" | "cancelled"): Promise<boolean> {
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
      const next = { ...current, state };
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
    a.manifestSha256 === b.manifestSha256;
}
