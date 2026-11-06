import { constants as C, type BigIntStats } from "node:fs";
import { lstat, open, readdir } from "node:fs/promises";
import path from "node:path";
import { decodeJob, type Job } from "./job.ts";
import { consumePrivateFile, PrivateFileError } from "../state/private-file.ts";
import { CredentialSnapshotError, checkSignal, samePathFd, withStateQueue, type NamespaceLease, type StateDirectory, type NamespaceResources, type SnapshotFailure } from "../state/queue-transaction.ts";

export type QueueSnapshotError = SnapshotFailure | "invalid_queue_record";
export class QueueStoreError extends Error {
  readonly category: QueueSnapshotError;
  constructor(category: QueueSnapshotError) { super(category); this.category = category; this.name = "QueueStoreError"; }
}
const fail = (category: QueueSnapshotError): never => { throw new QueueStoreError(category); };
const identity = (s: BigIntStats) => `${s.dev}:${s.ino}:${s.mode}:${s.uid}`;
const stamp = (s: BigIntStats) => `${identity(s)}:${s.size}:${s.mtimeNs}:${s.ctimeNs}:${s.nlink}`;
const MAX_JOB_BYTES = 2_000_000;
const MAX_JOBS = 100;
function deepFreeze<T>(value: T): T {
  if (value && typeof value === "object" && !Object.isFrozen(value)) {
    Object.freeze(value);
    for (const child of Object.values(value as Record<string, unknown>)) deepFreeze(child);
  }
  return value;
}

/** Read the legacy Rust queue layout under queue.lock. Any unsafe or malformed
 * record fails the snapshot as a whole; callers must not render it as complete. */
export function readQueueSnapshot(directoryPath: string, context: { scope: "read" | "upload"; approvedServer: string },
  resources: NamespaceResources, options: { signal?: AbortSignal; lockTimeoutMs?: number } = {}): Promise<readonly Job[]> {
  const directory = directoryPath, scope = context?.scope, approvedServer = context?.approvedServer;
  const signal = options.signal, lockTimeoutMs = options.lockTimeoutMs;
  if ((scope !== "read" && scope !== "upload") || typeof approvedServer !== "string") return Promise.reject(new QueueStoreError("invalid_credential"));
  return withStateQueue(directory, { scope, approvedServer }, resources, { signal, lockTimeoutMs }, async (state, lease) => {
    return readQueueWithinLease(state, lease, resources, signal);
  }).catch(error => {
    if (error instanceof QueueStoreError) throw error;
    if (error instanceof CredentialSnapshotError) throw new QueueStoreError(error.category);
    if (signal?.aborted) throw new QueueStoreError("cancelled");
    throw new QueueStoreError("unavailable");
  });
}

/** Internal composition seam. Caller owns the admitted directory and held queue lease. */
export async function readQueueWithinLease(state: StateDirectory, lease: NamespaceLease,
  resources: NamespaceResources, signal?: AbortSignal): Promise<readonly Job[]> {
  checkSignal(signal, lease.signal);
  const names = await readdir(state.path);
  const records = names.filter(name => path.extname(name) === ".job").sort();
  if (records.length > MAX_JOBS) fail("invalid_queue_record");
  const jobs: Job[] = [];
  for (const name of records) {
    checkSignal(signal, lease.signal); lease.assertHeld();
    if (!/^[0-9a-f]{32}\.job$/.test(name)) fail("invalid_queue_record");
    if (!await samePathFd(state)) fail("unsafe_directory");
    const file = path.join(state.path, name);
    let handle;
    try { handle = await open(file, C.O_RDONLY | (C.O_NOFOLLOW ?? 0) | (C.O_NONBLOCK ?? 0)); }
    catch { fail("unsafe_file"); }
    const opened = handle!;
    let consumed = false;
    try {
      const before = await opened.stat({ bigint: true });
      const named = await lstat(file, { bigint: true });
      // Data hardlinks are permitted by the donor reader; only the lock must
      // have a single link. Stability and private metadata still apply.
      if (!named.isFile() || named.isSymbolicLink() || identity(named) !== identity(before)) fail("unsafe_file");
      const fd = opened; handle = undefined; consumed = true;
      let job!: Job;
      try {
        job = await consumePrivateFile(fd, { maximumBytes: MAX_JOB_BYTES, signal }, resources, bytes => decodeJob(bytes));
      } catch (e) {
        if (e instanceof PrivateFileError) {
          if (e.category === "invalid_data") fail("invalid_queue_record");
          fail(e.category === "unsafe_file" || e.category === "cancelled" || e.category === "unavailable" ? e.category : "unavailable");
        }
        fail("unavailable");
      }
      const after = await lstat(file, { bigint: true }).catch(() => undefined);
      if (!after || !after.isFile() || after.isSymbolicLink() || stamp(after) !== stamp(before)) fail("unsafe_file");
      if (job.id !== name.slice(0, -4)) fail("invalid_queue_record");
      checkSignal(signal, lease.signal); lease.assertHeld();
      if (!await samePathFd(state)) fail("unsafe_directory");
      jobs.push(job);
    } finally { if (!consumed) await opened.close().catch(() => {}); }
  }
  return Object.freeze(jobs.map(job => deepFreeze(structuredClone(job))));
}
