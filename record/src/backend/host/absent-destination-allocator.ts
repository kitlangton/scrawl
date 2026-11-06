import { createHash } from "node:crypto";
import { lstat, realpath } from "node:fs/promises";
import { DatabaseSync } from "node:sqlite";
import { isAbsolute, join, resolve, sep } from "node:path";
import { ensurePrivateFile, ensurePrivateRoot, hostError, validatePrivateFile } from "./private-path.js";
import { initializePrivateSqlite } from "./sqlite-initialization.js";

export interface AbsentDestinationBinding {
  bundleId: string;
  bundlePath: string;
}

const schemaVersion = 1;
const validRequestId = (value: unknown): value is string => typeof value === "string" && value.length > 0 && Buffer.byteLength(value, "utf8") <= 4096 && !value.includes("\0") && Buffer.from(value,"utf8").toString("utf8")===value;

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
    await ensureDefaultDestinationRoot(this.#destinationRoot);
    const db = await this.#open();
    try {
      const digest = createHash("sha256").update(requestId, "utf8").digest("hex");
      const bundleId = `record-${digest}.apparent`;
      const bundlePath = join(this.#destinationRoot, bundleId);
      const query=db.prepare("SELECT request_id, control_root, destination_root, bundle_id, bundle_path FROM destination_binding WHERE request_digest = ?");
      const validate=(row:Record<string,unknown>):AbsentDestinationBinding=>{
        if(row.request_id!==requestId||row.control_root!==this.#controlRoot||row.destination_root!==this.#destinationRoot||row.bundle_id!==bundleId||row.bundle_path!==bundlePath)throw hostError();
        return {bundleId,bundlePath};
      };
      const existing=query.get(digest);
      if(existing)return validate(existing);
      // No filesystem await may hold a SQLite write lock: another synchronous
      // connection on this event loop must not block the operation that owns it.
      try {await assertAbsent(bundlePath);}
      catch(error){const winner=query.get(digest);if(winner)return validate(winner);throw error;}
      db.exec("BEGIN IMMEDIATE");
      const winner=query.get(digest);
      if(winner){const binding=validate(winner);db.exec("COMMIT");return binding;}
      db.prepare("INSERT INTO destination_binding (request_digest, request_id, control_root, destination_root, bundle_id, bundle_path) VALUES (?, ?, ?, ?, ?, ?)")
        .run(digest, requestId, this.#controlRoot, this.#destinationRoot, bundleId, bundlePath);
      db.exec("COMMIT");
      return { bundleId, bundlePath };
    } catch {
      try { db.exec("ROLLBACK"); } catch { /* commit may already have completed */ }
      throw hostError();
    } finally { try { db.close(); } catch { /* preserve allocation result */ } }
  }

  /** Persist an exact caller-selected destination without touching the final path. */
  async reserveExact(requestId: string, absoluteBundlePath: string): Promise<AbsentDestinationBinding> {
    if (!validRequestId(requestId) || !validBundlePath(absoluteBundlePath) ||
        !isAbsolute(this.#controlRoot) || resolve(this.#controlRoot) !== this.#controlRoot ||
        rootsOverlap(this.#controlRoot, absoluteBundlePath)) throw hostError();
    const parent = resolve(absoluteBundlePath, "..");
    if (await realpath(parent).catch(() => "") !== parent) throw hostError();
    const parentInfo = await lstat(parent).catch(() => undefined);
    if (!parentInfo?.isDirectory() || parentInfo.isSymbolicLink()) throw hostError();
    await ensurePrivateRoot(this.#controlRoot);
    const db = await this.#open();
    try {
      const digest = createHash("sha256").update(requestId, "utf8").digest("hex");
      const bundleId = `record-${digest}.apparent`;
      const binding = { bundleId, bundlePath: absoluteBundlePath };
      const query = db.prepare("SELECT request_id, control_root, destination_root, bundle_id, bundle_path FROM destination_binding WHERE request_digest = ?");
      const validate = (row: Record<string, unknown>): AbsentDestinationBinding => {
        if (row.request_id !== requestId || row.control_root !== this.#controlRoot || row.destination_root !== "@exact" ||
            row.bundle_id !== bundleId || row.bundle_path !== absoluteBundlePath) throw hostError();
        return binding;
      };
      const existing = query.get(digest);
      if (existing) return validate(existing);
      await assertAbsent(absoluteBundlePath);
      db.exec("BEGIN IMMEDIATE");
      const winner = query.get(digest);
      if (winner) { const result = validate(winner); db.exec("COMMIT"); return result; }
      db.prepare("INSERT INTO destination_binding (request_digest, request_id, control_root, destination_root, bundle_id, bundle_path) VALUES (?, ?, ?, ?, ?, ?)")
        .run(digest, requestId, this.#controlRoot, "@exact", bundleId, absoluteBundlePath);
      db.exec("COMMIT");
      return binding;
    } catch {
      try { db.exec("ROLLBACK"); } catch { /* commit may already have completed */ }
      throw hostError();
    } finally { try { db.close(); } catch { /* preserve reservation result */ } }
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

/** Existing host-selected output folders may be user-readable, but must remain owned, non-aliased, and not writable by group/other. */
async function ensureDefaultDestinationRoot(root: string): Promise<void> {
  let info;
  try { info = await lstat(root); }
  catch (error) {
    if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw hostError();
    await ensurePrivateRoot(root);
    return;
  }
  if (!info.isDirectory() || info.isSymbolicLink() || info.uid !== process.getuid?.() || (info.mode & 0o022) !== 0 ||
      await realpath(root).catch(() => "") !== root) throw hostError();
}

function validBundlePath(path: string): boolean {
  return typeof path === "string" && path.length > 0 && Buffer.byteLength(path, "utf8") <= 4096 &&
    !path.includes("\0") && Buffer.from(path, "utf8").toString("utf8") === path &&
    isAbsolute(path) && resolve(path) === path && path !== resolve(path, "..");
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
