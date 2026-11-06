import { constants } from "node:fs";
import { lstat, open, type FileHandle } from "node:fs/promises";
import path from "node:path";
import { decodeStoredIdentityRecord, validateStoredIdentityRecord, StoredIdentityError } from "../credentials/recipient-identity.ts";
import { normalizeRecipientServer, RecipientTransportError } from "../recipient/transport.ts";
import type { RecipientReadCredential } from "../recipient/retrieve.ts";
import { admitTrustedDirectory, directoryIdentity } from "./directory.ts";
import { consumePrivateFile, PrivateFileError } from "./private-file.ts";
import { readCredentialRecoverySnapshot, assertCredentialSnapshotCurrent, CredentialSnapshotError, type NamespaceResources } from "./namespace.ts";
import type { RecipientRenewalOwner } from "./renewal-owner.ts";

export type RecipientSessionFailure = "auth_required" | "renewal_required" | "encryption_identity_missing" | "encryption_identity_unavailable"
  | "state_unavailable" | "stale_snapshot" | "revocation_pending" | "invalid_server" | "cancelled" | "offline";
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
  /** Reloads writer state; an explicitly bound owner may renew the rejected token. */
  reloadAfterUnauthorized(): Promise<RecipientReadSession>;
  prepareForRequest(rejectedToken?: string, requestSignal?: AbortSignal): Promise<RecipientReadSession>;
  rejectAfterUnauthorized(token: string, requestSignal?: AbortSignal): Promise<void>;
  dispose(): void;
};

/** TypeScript reads only. An explicitly bound authoritative owner may renew. No keys are
 * created, repaired or replaced; an explicitly plaintext manifest needs no key. */
export async function openRecipientReadSession(directory: string, approvedServer: string, resources: NamespaceResources,
  options: { signal?: AbortSignal; nowSeconds?: () => number; renewRead?: RecipientRenewalOwner; requestSignal?: AbortSignal } = {}): Promise<RecipientReadSession> {
  const signal = options.signal, clock = options.nowSeconds ?? (() => Math.floor(Date.now() / 1000)), renewRead = options.renewRead;
  const initialSignal = options.requestSignal ? AbortSignal.any([options.requestSignal, ...(signal ? [signal] : [])]) : signal;
  const cancelled = () => { if (signal?.aborted) fail("cancelled"); };
  const now = () => { const value = clock(); if (!Number.isSafeInteger(value) || value < 0) fail("state_unavailable"); return value; };
  const context = { scope: "read" as const, approvedServer: "" };
  let loaded: Awaited<ReturnType<typeof readCredentialRecoverySnapshot>>;
  try { cancelled(); context.approvedServer = normalizeRecipientServer(approvedServer); loaded = await readCredentialRecoverySnapshot(directory, context, resources, { signal: initialSignal }); }
  catch (error) { return stateFailure(error); }
  const requestRenewal = async (expected: typeof loaded.snapshot, force: boolean, requestSignal?: AbortSignal) => {
    if (!renewRead) fail("renewal_required");
    const combined = requestSignal ? AbortSignal.any([requestSignal, ...(signal ? [signal] : [])]) : signal;
    try {
      await assertCredentialSnapshotCurrent(directory, expected, resources, { signal: combined });
      const result = await renewRead!(Object.freeze({ protocol: 1, operation: Object.freeze({ kind: "renew_read",
        server: expected.credential.server, generation: expected.credential.generation, token: expected.credential.token,
        cancellation: expected.cancellation, force }) }), combined);
      if (combined?.aborted) fail("cancelled");
      if (result?.state !== "current") {
        if (result && ["cancelled", "auth_required", "invalid_server", "offline", "state_unavailable"].includes(result.state)) fail(result.state as RecipientSessionFailure);
        fail("state_unavailable");
      }
      await assertCredentialSnapshotCurrent(directory, expected, resources, { signal: combined });
    } catch (error) { if (combined?.aborted) fail("cancelled"); return stateFailure(error); }
  };
  if (renewRead && !loaded.credential.authRequired && loaded.credential.refreshToken !== null
    && (loaded.recovery === "completed_response" || loaded.credential.expiresAt! <= Math.min(Number.MAX_SAFE_INTEGER, now() + 30)
      || loaded.credential.refreshExpiresAt! <= now())) {
    const original = loaded.snapshot;
    await requestRenewal(original, loaded.recovery === "completed_response", initialSignal);
    try {
      loaded = await readCredentialRecoverySnapshot(directory, context, resources, { signal: initialSignal });
      await assertCredentialSnapshotCurrent(directory, original, resources, { signal: initialSignal });
      if (loaded.recovery !== "stored") fail("state_unavailable");
    } catch (error) { return stateFailure(error); }
  }
  const snapshot = loaded.snapshot, stored = loaded.credential;
  const checkLifetime = () => {
    const time = now();
    if (stored.authRequired || stored.refreshExpiresAt !== null && stored.refreshExpiresAt <= time) fail("auth_required");
    if (stored.expiresAt !== null && stored.expiresAt <= time) fail(stored.refreshToken === null ? "auth_required" : "renewal_required");
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
  const prepareForRequest = async (rejectedToken?: string, requestSignal?: AbortSignal): Promise<RecipientReadSession> => {
    await assertIncarnation();
    const combined = requestSignal ? AbortSignal.any([requestSignal, ...(signal ? [signal] : [])]) : signal;
    let next: RecipientReadSession | undefined;
    try {
      if (renewRead && rejectedToken !== undefined) {
        const latest = await readCredentialRecoverySnapshot(directory, context, resources, { signal: combined });
        await assertIncarnation();
        if (latest.credential.token === rejectedToken) await requestRenewal(latest.snapshot, true, combined);
      }
      next = await openRecipientReadSession(directory, approvedServer, resources, { signal, nowSeconds: clock, renewRead, requestSignal: combined });
      await assertIncarnation();
      if (combined?.aborted) fail("cancelled");
      if (rejectedToken !== undefined && next.credential.token === rejectedToken) fail(stored.refreshToken === null ? "auth_required" : "renewal_required");
      return next;
    } catch (error) { next?.dispose(); if (combined?.aborted) fail("cancelled"); return stateFailure(error); }
  };
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
    reloadAfterUnauthorized: () => prepareForRequest(stored.token),
    prepareForRequest,
    async rejectAfterUnauthorized(token: string, requestSignal?: AbortSignal) {
      await assertIncarnation();
      const combined = requestSignal ? AbortSignal.any([requestSignal, ...(signal ? [signal] : [])]) : signal;
      try {
        const latest = await readCredentialRecoverySnapshot(directory, context, resources, { signal: combined });
        await assertIncarnation();
        if (!renewRead || latest.snapshot.credential.token !== token) return;
        const current = latest.snapshot;
        const result = await renewRead(Object.freeze({ protocol: 1, operation: Object.freeze({ kind: "reject_read",
          server: current.credential.server, generation: current.credential.generation, token, cancellation: current.cancellation }) }), combined);
        if (combined?.aborted) fail("cancelled");
        if (result?.state !== "rejected") fail("state_unavailable");
      } catch (error) { if (combined?.aborted) fail("cancelled"); return stateFailure(error); }
    },
    dispose() { disposed = true; identity?.signing.fill(0); },
  });
}
