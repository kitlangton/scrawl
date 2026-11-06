import { constants as C } from "node:fs";
import { lstat, open, type FileHandle } from "node:fs/promises";
import path from "node:path";
import type { CredentialReadResources } from "../credentials/private-read.ts";
import { admitTrustedDirectory, DirectoryAdmissionFailure, directoryIdentity } from "./directory.ts";

export type NamespaceLease = { assertHeld(): void; readonly signal: AbortSignal; release(): Promise<void> };
export type NamespaceResources = CredentialReadResources & {
  /** Consumes the lock descriptor. Busy waits are cancellation-aware and timeout-bounded. */
  lock(handle: FileHandle, options: { timeoutMs: number; signal?: AbortSignal }): Promise<
    | { state: "acquired"; lease: NamespaceLease }
    | { state: "busy" | "cancelled" | "failed" | "refused" }
  >;
};
export type SnapshotFailure = "unsafe_directory" | "unsafe_file" | "invalid_credential" | "cancelled" | "unavailable" | "lock_refused" | "revocation_pending" | "stale_snapshot";
export class CredentialSnapshotError extends Error {
  readonly category: SnapshotFailure;
  constructor(category: SnapshotFailure) { super(category); this.category = category; this.name = "CredentialSnapshotError"; }
}
const fail = (category: SnapshotFailure): never => { throw new CredentialSnapshotError(category); };
const identity = directoryIdentity;
function assertLease(lease: NamespaceLease): void { try { lease.assertHeld(); } catch { fail("unavailable"); } }
export const checkSignal = (signal?: AbortSignal, leaseSignal?: AbortSignal) => { if (signal?.aborted || leaseSignal?.aborted) fail("cancelled"); };
export const samePathFd = async (directory: { handle: FileHandle; path: string; identity: string }) => {
  try {
    const [fd, named] = await Promise.all([directory.handle.stat({ bigint: true }), lstat(directory.path, { bigint: true })]);
    return named.isDirectory() && identity(fd) === directory.identity && identity(named) === directory.identity;
  } catch { return false; }
};
async function lockFile(directory: { handle: FileHandle; path: string; identity: string }, resources: NamespaceResources,
  timeoutMs: number, signal?: AbortSignal) {
  checkSignal(signal);
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs < 0 || timeoutMs > 60_000 || !await samePathFd(directory)) fail("unsafe_directory");
  const lockPath = path.join(directory.path, "queue.lock");
  let handle: FileHandle | undefined;
  try { handle = await open(lockPath, C.O_RDWR | C.O_CREAT | (C.O_NOFOLLOW ?? 0) | (C.O_NONBLOCK ?? 0), 0o600); }
  catch { fail("unsafe_file"); }
  const lockHandle = handle!;
  let verifyHandle: FileHandle | undefined;
  let lease: NamespaceLease | undefined;
  try {
    verifyHandle = await open(lockPath, C.O_RDONLY | (C.O_NOFOLLOW ?? 0) | (C.O_NONBLOCK ?? 0));
    const before = await lockHandle.stat({ bigint: true }), named = await lstat(lockPath, { bigint: true });
    if (!before.isFile() || before.uid !== BigInt(process.getuid!()) || (before.mode & 0o077n) !== 0n || before.nlink !== 1n
      || !named.isFile() || named.isSymbolicLink() || identity(before) !== identity(named)) {
      await lockHandle.close().catch(() => {}); handle = undefined; fail("unsafe_file");
    }
    handle = undefined;
    const acquired = await resources.lock(lockHandle, { timeoutMs, signal });
    if (acquired.state !== "acquired") fail(acquired.state === "cancelled" ? "cancelled" : acquired.state === "busy" ? "lock_refused" : "unavailable");
    lease = (acquired as { state: "acquired"; lease: NamespaceLease }).lease;
    assertLease(lease);
    const after = await lstat(lockPath, { bigint: true }), fd = await verifyHandle.stat({ bigint: true });
    if (!after.isFile() || after.isSymbolicLink() || after.nlink !== 1n || identity(after) !== identity(before) || identity(fd) !== identity(before) || fd.nlink !== 1n) fail("unsafe_file");
    if (!await samePathFd(directory)) fail("unsafe_directory");
    await verifyHandle.close(); verifyHandle = undefined;
    return { lease, lockPath, lockIdentity: identity(before) };
  } catch (e) {
    await handle?.close().catch(() => {}); await verifyHandle?.close().catch(() => {});
    if (lease) { try { await lease.release(); } catch {} }
    if (e instanceof CredentialSnapshotError) throw e;
    return fail("unavailable");
  }
}
async function release(lease: NamespaceLease) {
  try { await lease.release(); } catch { fail("unavailable"); }
}
export async function withStateQueue<T>(directoryPath: string, context: { scope: "read" | "upload"; approvedServer: string }, resources: NamespaceResources,
  options: { signal?: AbortSignal; lockTimeoutMs?: number }, body: (directory: { handle: FileHandle; path: string; identity: string }, lease: NamespaceLease) => Promise<T>): Promise<T> {
  const signal = options.signal, lockTimeoutMs = options.lockTimeoutMs, scope = context?.scope, approvedServer = context?.approvedServer;
  if ((scope !== "read" && scope !== "upload") || typeof approvedServer !== "string") fail("invalid_credential");
  checkSignal(signal);
  let directory: Awaited<ReturnType<typeof admitTrustedDirectory>>;
  try { directory = await admitTrustedDirectory(directoryPath, resources, "private-state", signal); }
  catch (e) { if (e instanceof DirectoryAdmissionFailure) return fail(e.category); return fail("unsafe_directory"); }
  try {
    const lock = await lockFile(directory!, resources, lockTimeoutMs ?? 1000, signal);
    try {
      const lease = lock.lease;
      checkSignal(signal, lease.signal);
      assertLease(lease);
      const result = await body(directory!, lease);
      checkSignal(signal, lease.signal);
      assertLease(lease);
      if (!await samePathFd(directory!)) fail("unavailable");
      const named = await lstat(lock.lockPath, { bigint: true });
      if (!named.isFile() || named.nlink !== 1n || identity(named) !== lock.lockIdentity) fail("unsafe_file");
      return result;
    } finally { await release(lock.lease); }
  } finally { await directory!.handle.close().catch(() => {}); }
}

