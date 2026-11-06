import type { FileHandle } from "node:fs/promises";
import { decodeStoredCredential, type StoredCredential } from "./wire.ts";

export type CredentialReadFailure = "invalid_credential" | "unsafe_file" | "cancelled" | "unavailable";
export class CredentialReadError extends Error {
  readonly category: CredentialReadFailure;
  constructor(category: CredentialReadFailure) { super(category); this.name = "CredentialReadError"; this.category = category; }
}
export type CredentialReadResources = {
  /** Consumes the descriptor on every outcome; reports ACL facts, not authorization. */
  checkAcl(handle: FileHandle): Promise<{ state: "accepted" | "refused" }>;
};
const fail = (category: CredentialReadFailure): never => { throw new CredentialReadError(category); };
const cancelled = (signal?: AbortSignal) => { if (signal?.aborted) fail("cancelled"); };
const LIMIT = 32768;

/** Consume an already admitted descriptor and decode private stored facts.
 * The caller must open with NOFOLLOW inside its trusted, inode-pinned state
 * directory, approve the server, and enforce fresh authorization/revocation and
 * cancellation/generation fences. This reader neither renews nor authorizes. */
export async function readStoredCredentialHandle(
  handle: FileHandle,
  context: { scope: "read" | "upload"; approvedServer: string },
  resources: CredentialReadResources,
  options: { signal?: AbortSignal } = {},
): Promise<StoredCredential> {
  const signal = options.signal;
  let ownsHandle = true;
  let bytes: Buffer | undefined;
  try {
    cancelled(signal);
    if (!context || !["read", "upload"].includes(context.scope) || typeof context.approvedServer !== "string") fail("invalid_credential");
    const scope = context.scope, server = context.approvedServer;
    const before = await handle.stat({ bigint: true });
    if (typeof process.getuid !== "function" || !before.isFile() || before.uid !== BigInt(process.getuid())
      || (before.mode & 0o077n) !== 0n || before.size > BigInt(LIMIT)) fail("unsafe_file");
    bytes = Buffer.alloc(LIMIT + 1);
    let length = 0;
    while (length < bytes.length) {
      cancelled(signal);
      const read = await handle.read(bytes, length, bytes.length - length, length);
      if (read.bytesRead === 0) break;
      length += read.bytesRead;
    }
    cancelled(signal);
    const after = await handle.stat({ bigint: true });
    if (length > LIMIT || BigInt(length) !== before.size || !after.isFile()
      || before.dev !== after.dev || before.ino !== after.ino || before.uid !== after.uid
      || before.mode !== after.mode || before.size !== after.size
      || before.mtimeNs !== after.mtimeNs || before.ctimeNs !== after.ctimeNs) fail("unsafe_file");
    // The adapter takes ownership; no bytes leave this boundary before ACL proof.
    ownsHandle = false;
    const acl = await resources.checkAcl(handle);
    if (acl.state !== "accepted") fail("unsafe_file");
    cancelled(signal);
    try { return decodeStoredCredential(bytes.subarray(0, length), scope, server); }
    catch { return fail("invalid_credential"); }
  } catch (error) {
    if (error instanceof CredentialReadError) throw error;
    return fail("unavailable");
  } finally {
    bytes?.fill(0);
    if (ownsHandle) await handle.close().catch(() => {});
  }
}
