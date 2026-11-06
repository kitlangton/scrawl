import type { FileHandle } from "node:fs/promises";
import { decodeStoredCredential, type StoredCredential } from "./wire.ts";
import { consumePrivateFile, PrivateFileError } from "../state/private-file.ts";

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
  let transferred = false;
  try {
    if (signal?.aborted) fail("cancelled");
    if (!context || !["read", "upload"].includes(context.scope) || typeof context.approvedServer !== "string") fail("invalid_credential");
    const scope = context.scope, server = context.approvedServer;
    transferred = true;
    return await consumePrivateFile(handle, { maximumBytes: LIMIT, signal }, resources,
      bytes => decodeStoredCredential(bytes, scope, server));
  } catch (error) {
    if (error instanceof CredentialReadError) throw error;
    if (error instanceof PrivateFileError) {
      if (error.category === "invalid_data") return fail("invalid_credential");
      if (error.category === "unsafe_file" || error.category === "cancelled" || error.category === "unavailable") return fail(error.category);
    }
    if (signal?.aborted) return fail("cancelled");
    return fail("unavailable");
  } finally { if (!transferred) await handle.close().catch(() => {}); }
}
