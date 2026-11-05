import { createHash } from "node:crypto";
import { lstat } from "node:fs/promises";
import { DatabaseSync } from "node:sqlite";
import { isAbsolute, join, resolve, sep } from "node:path";
import { ensurePrivateFile, ensurePrivateRoot, hostError, validatePrivateFile } from "./private-path.js";
import { initializePrivateSqlite } from "./sqlite-initialization.js";

export interface AbsentDestinationBinding {
  bundleId: string;
  bundlePath: string;
}

const schemaVersion = 1;
const validRequestId = (value: unknown): value is string => typeof value === "string" && value.length > 0 && Buffer.byteLength(value, "utf8") <= 4096 && !value.includes("\0");

/** Durable identity reservation for capture/2. This stores metadata only and never creates the final bundle path. */
export class AbsentDestinationAllocator {
  readonly #controlRoot: string;
  readonly #destinationRoot: string;
  readonly #databasePath: string;

  constructor(controlRoot: string, destinationRoot: string) {
    this.#controlRoot = controlRoot;
    this.#destinationRoot = destinationRoot;
    this.#databasePath = join(controlRoot, "capture2-destination-bindings.sqlite");
  }

  async allocate(requestId: string): Promise<AbsentDestinationBinding> {
    if (!validRequestId(requestId)) throw hostError();
    if (!isAbsolute(this.#controlRoot) || resolve(this.#controlRoot) !== this.#controlRoot ||
        !isAbsolute(this.#destinationRoot) || resolve(this.#destinationRoot) !== this.#destinationRoot ||
        rootsOverlap(this.#controlRoot, this.#destinationRoot)) throw hostError();

    await ensurePrivateRoot(this.#controlRoot);
    await ensurePrivateRoot(this.#destinationRoot);
    const db = await this.#open();
    try {
      db.exec("BEGIN IMMEDIATE");
      const digest = createHash("sha256").update(requestId, "utf8").digest("hex");
      const row = db.prepare("SELECT request_id, control_root, destination_root, bundle_id, bundle_path FROM destination_binding WHERE request_digest = ?").get(digest);
      if (row) {
        if (row.request_id !== requestId || row.control_root !== this.#controlRoot || row.destination_root !== this.#destinationRoot) throw hostError();
        const binding = { bundleId: row.bundle_id as string, bundlePath: row.bundle_path as string };
        db.exec("COMMIT");
        return binding;
      }

      const bundleId = `record-${digest}.apparent`;
      const bundlePath = join(this.#destinationRoot, bundleId);
      await assertAbsent(bundlePath);
      db.prepare("INSERT INTO destination_binding (request_digest, request_id, control_root, destination_root, bundle_id, bundle_path) VALUES (?, ?, ?, ?, ?, ?)")
        .run(digest, requestId, this.#controlRoot, this.#destinationRoot, bundleId, bundlePath);
      db.exec("COMMIT");
      return { bundleId, bundlePath };
    } catch {
      try { db.exec("ROLLBACK"); } catch { /* commit may already have completed */ }
      throw hostError();
    } finally { try { db.close(); } catch { /* preserve allocation result */ } }
  }

  async #open(): Promise<DatabaseSync> {
    await ensurePrivateFile(this.#databasePath);
    for (const suffix of ["-wal", "-shm", "-journal"]) await validatePrivateFile(this.#databasePath + suffix, true);
    let db: DatabaseSync | undefined;
    try {
      db = new DatabaseSync(this.#databasePath);
      await initializePrivateSqlite(db);
      const version = Number(db.prepare("PRAGMA user_version").get()?.user_version ?? 0);
      if (version === 0) db.exec("CREATE TABLE IF NOT EXISTS destination_binding (request_digest TEXT PRIMARY KEY, request_id TEXT NOT NULL, control_root TEXT NOT NULL, destination_root TEXT NOT NULL, bundle_id TEXT NOT NULL UNIQUE, bundle_path TEXT NOT NULL UNIQUE) STRICT; PRAGMA user_version = 1;");
      else if (version !== schemaVersion) throw hostError();
      return db;
    } catch { try { db?.close(); } catch { /* sanitized */ } throw hostError(); }
  }
}

async function assertAbsent(path: string): Promise<void> {
  try { await lstat(path); } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return;
    throw hostError();
  }
  throw hostError();
}

function rootsOverlap(left: string, right: string): boolean {
  const beneath = (parent: string, candidate: string) => candidate === parent || candidate.startsWith(parent.endsWith(sep) ? parent : parent + sep);
  return beneath(left, right) || beneath(right, left);
}
