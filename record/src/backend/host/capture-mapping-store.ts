import { DatabaseSync } from "node:sqlite";
import { isAbsolute, join, resolve } from "node:path";
import type { CaptureMapping, CaptureMappingStore } from "../engine/sdk-adapter.js";
import { ensurePrivateFile, ensurePrivateRoot, hostError, validatePrivateFile } from "./private-path.js";

const schemaVersion = 1;
const valid = (mapping: CaptureMapping) =>
  typeof mapping.requestId === "string" && mapping.requestId.length > 0 && Buffer.byteLength(mapping.requestId) <= 4096 &&
  /^[0-9a-f]{32}$/u.test(mapping.captureId) && /^[0-9a-f]{64}$/u.test(mapping.ownerToken) &&
  typeof mapping.bundleId === "string" && Buffer.byteLength(mapping.bundleId) > 0 && Buffer.byteLength(mapping.bundleId) <= 255 &&
  !/[\\/\p{Cc}]/u.test(mapping.bundleId) && mapping.bundleId !== "." && mapping.bundleId !== ".." &&
  typeof mapping.bundlePath === "string" && isAbsolute(mapping.bundlePath) && resolve(mapping.bundlePath) === mapping.bundlePath &&
  mapping.bundlePath.length <= 4096 && !mapping.bundlePath.includes("\0");

/** SQLite-backed owner-token store. The explicit root must be a private host directory. */
export class SqliteCaptureMappingStore implements CaptureMappingStore {
  readonly #root: string;
  readonly #databasePath: string;

  constructor(privateRoot: string) {
    this.#root = privateRoot;
    this.#databasePath = join(privateRoot, "capture-mappings.sqlite");
  }

  async listRequestIds(): Promise<string[]> {
    const db = await this.#open();
    try {
      return db.prepare("SELECT request_id FROM capture_mapping ORDER BY request_id").all()
        .map(row => {
          const requestId = row.request_id;
          if (typeof requestId !== "string" || !requestId.length || Buffer.byteLength(requestId) > 4096) throw hostError();
          return requestId;
        });
    } catch {
      throw hostError();
    } finally {
      try { db.close(); } catch { /* preserve a sanitized store error */ }
    }
  }

  async getByRequestId(requestId: string): Promise<CaptureMapping | undefined> {
    return this.#read("request_id", requestId);
  }

  async getByCaptureId(captureId: string): Promise<CaptureMapping | undefined> {
    return this.#read("capture_id", captureId);
  }

  async putIfAbsent(mapping: CaptureMapping): Promise<CaptureMapping> {
    if (!valid(mapping)) throw hostError();
    let db: DatabaseSync | undefined;
    try {
      db = await this.#open();
      db.exec("BEGIN IMMEDIATE");
      const existing = db.prepare("SELECT request_id, capture_id, owner_token, bundle_id, bundle_path FROM capture_mapping WHERE request_id = ?").get(mapping.requestId);
      if (existing) {
        db.exec("COMMIT");
        return rowToMapping(existing);
      }
      db.prepare("INSERT INTO capture_mapping (request_id, capture_id, owner_token, bundle_id, bundle_path) VALUES (?, ?, ?, ?, ?)")
        .run(mapping.requestId, mapping.captureId, mapping.ownerToken, mapping.bundleId, mapping.bundlePath);
      db.exec("COMMIT");
      return { ...mapping };
    } catch {
      try { db?.exec("ROLLBACK"); } catch { /* transaction may already be committed */ }
      try { db?.close(); } catch { /* the handle may already be closed */ }
      const reconciled = await this.#read("request_id", mapping.requestId).catch(() => undefined);
      if (reconciled) return reconciled;
      throw hostError();
    } finally {
      try { db?.close(); } catch { /* already closed */ }
    }
  }

  async #read(column: "request_id" | "capture_id", key: string): Promise<CaptureMapping | undefined> {
    if (typeof key !== "string" || !key.length || Buffer.byteLength(key) > 4096) throw hostError();
    const db = await this.#open();
    try {
      const row = db.prepare(`SELECT request_id, capture_id, owner_token, bundle_id, bundle_path FROM capture_mapping WHERE ${column} = ?`).get(key);
      return row ? rowToMapping(row) : undefined;
    } catch {
      throw hostError();
    } finally {
      try { db.close(); } catch { /* preserve a sanitized store error */ }
    }
  }

  async #open(): Promise<DatabaseSync> {
    await ensurePrivateRoot(this.#root);
    await ensurePrivateFile(this.#databasePath);
    for (const suffix of ["-wal", "-shm", "-journal"]) await validatePrivateFile(this.#databasePath + suffix, true);
    let db: DatabaseSync | undefined;
    try {
      db = new DatabaseSync(this.#databasePath);
      db.exec("PRAGMA busy_timeout = 5000; PRAGMA journal_mode = WAL; PRAGMA synchronous = FULL;");
      const version = Number(db.prepare("PRAGMA user_version").get()?.user_version ?? 0);
      if (version === 0) {
        db.exec("CREATE TABLE IF NOT EXISTS capture_mapping (request_id TEXT PRIMARY KEY, capture_id TEXT NOT NULL UNIQUE, owner_token TEXT NOT NULL, bundle_id TEXT NOT NULL UNIQUE, bundle_path TEXT NOT NULL UNIQUE) STRICT; PRAGMA user_version = 1;");
      } else if (version !== schemaVersion) {
        db.close();
        throw hostError();
      }
      return db;
    } catch {
      try { db?.close(); } catch { /* preserve a sanitized store error */ }
      throw hostError();
    }
  }
}

function rowToMapping(row: Record<string, unknown>): CaptureMapping {
  const mapping = {
    requestId: row.request_id,
    captureId: row.capture_id,
    ownerToken: row.owner_token,
    bundleId: row.bundle_id,
    bundlePath: row.bundle_path,
  } as CaptureMapping;
  if (!valid(mapping)) throw hostError();
  return mapping;
}
