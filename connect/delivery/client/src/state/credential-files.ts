import { constants as C, type BigIntStats } from "node:fs";
import { lstat, open, readdir, type FileHandle } from "node:fs/promises";
import path from "node:path";
import type { StoredCredential } from "../credentials/wire.ts";
import { readStoredCredentialHandle } from "../credentials/private-read.ts";
import { consumePrivateFile } from "./private-file.ts";
import { decodeStrictJson } from "../recipient/json.ts";
import { directoryIdentity as identity } from "./directory.ts";
import { CredentialSnapshotError, type SnapshotFailure, type NamespaceResources, type StateDirectory } from "./queue-transaction.ts";
const fail = (category: SnapshotFailure): never => { throw new CredentialSnapshotError(category); };

export async function pendingIntent(directoryPath: string): Promise<boolean> {
  try {
    for (const name of await readdir(directoryPath)) if (name.startsWith("revocation-") && name.endsWith(".intent")) return true;
    return false;
  } catch { return fail("unavailable"); }
}
export async function readOptionalStateFile<T>(directory: { path: string; identity: string }, name: string, maximumBytes: number,
  resources: NamespaceResources, signal: AbortSignal | undefined, decode: (bytes: Uint8Array) => T): Promise<T | null> {
  const file = path.join(directory.path, name);
  let handle: FileHandle | undefined;
  try { handle = await open(file, C.O_RDONLY | C.O_NOFOLLOW | C.O_NONBLOCK); }
  catch (error) { if ((error as NodeJS.ErrnoException).code === "ENOENT") return null; return fail("unsafe_file"); }
  try {
    const before = await handle.stat({ bigint: true }), named = await lstat(file, { bigint: true });
    if (!named.isFile() || identity(named) !== identity(before)) fail("unsafe_file");
    const transferred = handle; handle = undefined;
    const value = await consumePrivateFile(transferred, { maximumBytes, signal }, resources, decode);
    if (identity(await lstat(file, { bigint: true })) !== identity(before)) fail("unsafe_file");
    return value;
  } finally { await handle?.close().catch(() => {}); }
}
export async function readCancellation(directory: { path: string; identity: string }, resources: NamespaceResources, signal?: AbortSignal): Promise<string | null> {
  return readOptionalStateFile(directory, "cancellation", 128, resources, signal, bytes => {
    const value = decodeStrictJson(bytes, 128);
    if (typeof value !== "string") throw new Error("invalid_data");
    return value;
  });
}
export async function readCredential(directory: { path: string; identity: string }, context: { scope: "read" | "upload"; approvedServer: string }, resources: NamespaceResources, signal?: AbortSignal): Promise<StoredCredential> {
  const file = path.join(directory.path, `${context.scope}.credential`);
  let h: FileHandle;
  try { h = await open(file, C.O_RDONLY | (C.O_NOFOLLOW ?? 0) | (C.O_NONBLOCK ?? 0)); } catch { return fail("invalid_credential"); }
  try {
    const before = await h.stat({ bigint: true }), named = await lstat(file, { bigint: true });
    if (!named.isFile() || named.isSymbolicLink() || identity(named) !== identity(before)) fail("unsafe_file");
    const consumed = h; h = undefined as never;
    let credential: StoredCredential;
    try { credential = await readStoredCredentialHandle(consumed, context, resources, { signal }); }
    catch (error) {
      const namedAfterFailure = await lstat(file, { bigint: true }).catch(() => undefined);
      if (!namedAfterFailure || identity(namedAfterFailure) !== identity(before)) fail("unsafe_file");
      throw error;
    }
    const namedAfter = await lstat(file, { bigint: true });
    if (identity(namedAfter) !== identity(before)) fail("unsafe_file");
    return credential;
  } catch (e) { await h?.close().catch(() => {}); if (e instanceof CredentialSnapshotError) throw e;
    if (e && typeof e === "object" && "category" in e) {
      const category = (e as { category: string }).category;
      if (["invalid_credential", "unsafe_file", "cancelled", "unavailable"].includes(category)) fail(category as SnapshotFailure);
    }
    return fail("unavailable"); }
}

export const stateFileStamp = (s: BigIntStats) => `${identity(s)}:${s.size}:${s.mtimeNs}:${s.ctimeNs}:${s.nlink}`;
export async function readStateEvidence<T>(directory: StateDirectory, name: string, limit: number,
  resources: NamespaceResources, signal: AbortSignal, decode: (bytes: Uint8Array) => T) {
  const file = path.join(directory.path, name);
  const before = await lstat(file, { bigint: true }).catch(error => { if ((error as NodeJS.ErrnoException).code === "ENOENT") return null; throw error; });
  if (!before) return null;
  const expectedStamp = stateFileStamp(before);
  const value = await readOptionalStateFile(directory, name, limit, resources, signal, decode);
  const after = await lstat(file, { bigint: true });
  if (value === null || !after.isFile() || after.isSymbolicLink() || stateFileStamp(after) !== expectedStamp) fail("unsafe_file");
  return { value: value!, file, expectedStamp };
}
