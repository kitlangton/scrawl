import { DatabaseSync } from "node:sqlite";
import { constants } from "node:fs";
import { open } from "node:fs/promises";
import { dirname, isAbsolute, resolve } from "node:path";
import { ensurePrivateRoot, hostError, validatePrivateFile } from "../host/private-path.js";
import { initializePrivateSqlite } from "../host/sqlite-initialization.js";

/** One private media job, using the host stores' bounded SQLite durability.
 * Every transition compares the original bytes atomically across processes.
 * A cleared row remains a tombstone so stale handles cannot reuse its path. */
export class Media2Store {
  constructor(readonly path: string) {
    if (!isAbsolute(path) || resolve(path) !== path || path.includes("\0")) throw hostError();
  }
  async create(value: unknown): Promise<void> {
    await ensurePrivateRoot(dirname(this.path));
    // Claim an absent namespace before SQLite can change an existing database's
    // journal mode or schema. Never initialize a caller's occupied path.
    try {
      const handle = await open(this.path, constants.O_CREAT | constants.O_EXCL | constants.O_WRONLY | constants.O_NOFOLLOW, 0o600);
      await handle.close();
    } catch { throw hostError(); }
    const db = await this.open(true);
    try { db.prepare("INSERT INTO media2_job (id, record_json) VALUES (1, ?)").run(JSON.stringify(value)); }
    catch { throw hostError(); }
    finally { db.close(); }
  }
  async read(): Promise<unknown> {
    const db = await this.open();
    try {
      const raw = db.prepare("SELECT record_json FROM media2_job WHERE id = 1").get()?.record_json;
      if (typeof raw !== "string") throw hostError();
      return JSON.parse(raw) as unknown;
    } finally { db.close(); }
  }
  async compareExchange(previous: unknown, next: unknown | null): Promise<boolean> {
    const db = await this.open();
    try {
      return db.prepare("UPDATE media2_job SET record_json = ? WHERE id = 1 AND record_json = ?")
        .run(next === null ? null : JSON.stringify(next), JSON.stringify(previous)).changes === 1;
    } finally { db.close(); }
  }
  private async open(create = false): Promise<DatabaseSync> {
    await ensurePrivateRoot(dirname(this.path));
    await validatePrivateFile(this.path);
    for (const suffix of ["-wal", "-shm", "-journal"]) await validatePrivateFile(this.path + suffix, true);
    let db: DatabaseSync | undefined;
    try {
      db = new DatabaseSync(this.path);
      await initializePrivateSqlite(db);
      db.exec("BEGIN IMMEDIATE");
      const version = Number(db.prepare("PRAGMA user_version").get()?.user_version ?? 0);
      if (version === 0 && create)
        db.exec("CREATE TABLE media2_job (id INTEGER PRIMARY KEY CHECK (id = 1), record_json TEXT) STRICT; PRAGMA user_version = 1");
      else if (version !== 1) throw hostError();
      db.exec("COMMIT");
      return db;
    } catch {
      try { db?.close(); } catch { /* preserve sanitized storage failure */ }
      throw hostError();
    }
  }
}
