import { constants } from "node:fs";
import { lstat, open, type FileHandle } from "node:fs/promises";
import path from "node:path";
import { decodeStoredIdentityRecord, validateStoredIdentityRecord, StoredIdentityError } from "../credentials/recipient-identity.ts";
import { normalizeRecipientServer, RecipientTransportError } from "../recipient/transport.ts";
import type { RecipientReadCredential } from "../recipient/retrieve.ts";
import { admitTrustedDirectory, directoryIdentity } from "./directory.ts";
import { consumePrivateFile, PrivateFileError } from "./private-file.ts";
import { readCredentialRecoverySnapshot, assertCredentialSnapshotCurrent, CredentialSnapshotError, type NamespaceResources } from "./namespace.ts";

export type RecipientSessionFailure = "auth_required" | "renewal_required" | "encryption_identity_missing" | "encryption_identity_unavailable"
  | "state_unavailable" | "stale_snapshot" | "revocation_pending" | "invalid_server" | "cancelled";
export class RecipientSessionError extends Error {
  readonly category: RecipientSessionFailure;
  constructor(category: RecipientSessionFailure) { super(category); this.name = "RecipientSessionError"; this.category = category; }
}
const fail = (category: RecipientSessionFailure): never => { throw new RecipientSessionError(category); };
function stateFailure(error: unknown): never {
  if (error instanceof RecipientSessionError) throw error;
  if (error instanceof CredentialSnapshotError) {
    if (error.category === "stale_snapshot" || error.category === "revocation_pending" || error.category === "cancelled") fail(error.category);
    if (error.category === "invalid_credential") fail("auth_required");
  }
  if (error instanceof RecipientTransportError && error.category === "invalid_server") fail("invalid_server");
  return fail("state_unavailable");
}
export type RecipientReadSession = {
  readonly credential: RecipientReadCredential;
  resolveIdentity(): Promise<{ age: string; signing: Uint8Array }>;
  assertCurrent(): Promise<void>;
  /** Reloads existing writer state only; never sends a refresh request. */
  reloadAfterUnauthorized(): Promise<RecipientReadSession>;
  dispose(): void;
};

/** Read-only session. Renewal remains with the authoritative writer. No keys are
 * created, repaired or replaced; an explicitly plaintext manifest needs no key. */
export async function openRecipientReadSession(directory: string, approvedServer: string, resources: NamespaceResources,
  options: { signal?: AbortSignal; nowSeconds?: () => number } = {}): Promise<RecipientReadSession> {
  const signal = options.signal, clock = options.nowSeconds ?? (() => Math.floor(Date.now() / 1000));
  const cancelled = () => { if (signal?.aborted) fail("cancelled"); };
  let loaded: Awaited<ReturnType<typeof readCredentialRecoverySnapshot>>;
  try { cancelled(); loaded = await readCredentialRecoverySnapshot(directory, { scope: "read", approvedServer: normalizeRecipientServer(approvedServer) }, resources, { signal }); }
  catch (error) { return stateFailure(error); }
  const snapshot = loaded.snapshot, stored = loaded.credential;
  const checkLifetime = () => {
    const now = clock();
    if (!Number.isSafeInteger(now) || now < 0) fail("state_unavailable");
    if (stored.authRequired || stored.refreshExpiresAt !== null && stored.refreshExpiresAt <= now) fail("auth_required");
    if (stored.expiresAt !== null && stored.expiresAt <= now) fail(stored.refreshToken === null ? "auth_required" : "renewal_required");
  };
  checkLifetime();
  let disposed = false;
  let identity: { age: string; signing: Uint8Array } | undefined;
  let resolving: Promise<{ age: string; signing: Uint8Array }> | undefined;
  const assertIncarnation = async () => {
    if (disposed) fail("cancelled"); cancelled();
    try { await assertCredentialSnapshotCurrent(directory, snapshot, resources, { signal }); }
    catch (error) { return stateFailure(error); }
    if (disposed) fail("cancelled"); cancelled();
  };
  const assertCurrent = async () => { checkLifetime(); await assertIncarnation(); checkLifetime(); };
  const loadIdentity = async () => {
    await assertCurrent();
    const id = stored.encryption_identity;
    if (!id || !/^[a-f0-9]{32}$/.test(id)) fail("encryption_identity_missing");
    if (!stored.grantId) fail("auth_required");
    let admitted: Awaited<ReturnType<typeof admitTrustedDirectory>> | undefined;
    let handle: FileHandle | undefined;
    let loaded: { age: string; signing: Uint8Array } | undefined;
    try {
      admitted = await admitTrustedDirectory(directory, resources, "private-state", signal);
      if (admitted.identity !== snapshot.directoryIdentity) fail("stale_snapshot");
      const file = path.join(admitted.path, `${id}.identity`);
      handle = await open(file, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
      const before = await handle.stat({ bigint: true }), named = await lstat(file, { bigint: true });
      if (!named.isFile() || directoryIdentity(named) !== directoryIdentity(before)) fail("encryption_identity_unavailable");
      const transferred = handle; handle = undefined;
      const record = await consumePrivateFile(transferred, { maximumBytes: 4096, signal }, resources, decodeStoredIdentityRecord);
      if (directoryIdentity(await lstat(file, { bigint: true })) !== directoryIdentity(before)
        || directoryIdentity(await admitted.handle.stat({ bigint: true })) !== admitted.identity
        || directoryIdentity(await lstat(admitted.path, { bigint: true })) !== admitted.identity) fail("encryption_identity_unavailable");
      loaded = await validateStoredIdentityRecord(record, { signal });
      await assertCurrent();
      identity = loaded; return loaded;
    } catch (error) {
      loaded?.signing.fill(0);
      if (error instanceof RecipientSessionError) throw error;
      if (signal?.aborted || error instanceof StoredIdentityError && error.category === "cancelled"
        || error instanceof PrivateFileError && error.category === "cancelled") fail("cancelled");
      return fail("encryption_identity_unavailable");
    } finally { await handle?.close().catch(() => {}); await admitted?.handle.close().catch(() => {}); }
  };
  return Object.freeze({
    credential: Object.freeze({ server: stored.server, token: stored.token, grantId: stored.grantId ?? "" }),
    async resolveIdentity() { if (disposed) fail("cancelled"); cancelled(); return resolving ??= loadIdentity(); },
    assertCurrent,
    async reloadAfterUnauthorized() {
      await assertIncarnation();
      let next: RecipientReadSession | undefined;
      try {
        next = await openRecipientReadSession(directory, approvedServer, resources, { signal, nowSeconds: clock });
        await assertIncarnation();
        if (next.credential.token === stored.token) fail(stored.refreshToken === null ? "auth_required" : "renewal_required");
        return next;
      } catch (error) { next?.dispose(); return stateFailure(error); }
    },
    dispose() { disposed = true; identity?.signing.fill(0); },
  });
}
