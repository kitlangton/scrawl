import type { StoredCredential } from "../credentials/wire.ts";
import { PrivateFileError } from "./private-file.ts";
import { decodeRenewalJournal, isCompletedRenewalFor, recoverCompletedRenewal, RenewalJournalError } from "../credentials/renewal-journal.ts";
import { CredentialSnapshotError, type NamespaceResources, type SnapshotFailure, checkSignal, withStateQueue as withSnapshot } from "./queue-transaction.ts";
import { pendingIntent, readCancellation, readCredential, readOptionalStateFile } from "./credential-files.ts";
export { CredentialSnapshotError } from "./queue-transaction.ts";
export type { NamespaceResources, NamespaceLease, SnapshotFailure } from "./queue-transaction.ts";

export type CredentialSnapshot = Readonly<{ credential: StoredCredential; cancellation: string | null; directoryIdentity: string }>;
const fail = (category: SnapshotFailure): never => { throw new CredentialSnapshotError(category); };
function safeFailure(error: unknown): never {
  if (error instanceof CredentialSnapshotError) throw error;
  if (error instanceof PrivateFileError) fail(error.category === "invalid_data" ? "invalid_credential" : error.category);
  return fail("unavailable");
}
/** Reads stored credential facts under the legacy queue lock. It does not decide token expiry,
 * service authorization or renew credentials; callers must use their own approved context. */
export async function readCredentialSnapshot(directoryPath: string,
  context: { scope: "read" | "upload"; approvedServer: string }, resources: NamespaceResources,
  options: { signal?: AbortSignal; lockTimeoutMs?: number } = {}): Promise<CredentialSnapshot> {
  const ctx = { scope: context?.scope, approvedServer: context?.approvedServer } as typeof context;
  const opts = { signal: options.signal, lockTimeoutMs: options.lockTimeoutMs };
  return withSnapshot(directoryPath, ctx, resources, opts, async (directory, lease) => {
    checkSignal(opts.signal, lease.signal);
    if (await pendingIntent(directory.path)) fail("revocation_pending");
    const cancellation = await readCancellation(directory, resources, opts.signal);
    const credential = await readCredential(directory, ctx, resources, opts.signal);
    if (await pendingIntent(directory.path)) fail("revocation_pending");
    return Object.freeze({ credential, cancellation, directoryIdentity: directory.identity });
  }).catch(safeFailure);
}

/** Revalidates login incarnation and cancellation under the same queue transaction.
 * Access/refresh token rotation within the same login is allowed. Token expiry
 * and service authorization remain separate caller checks; this never renews. */
export async function assertCredentialSnapshotCurrent(directoryPath: string, snapshot: CredentialSnapshot,
  resources: NamespaceResources, options: { signal?: AbortSignal; lockTimeoutMs?: number } = {}): Promise<void> {
  if (!snapshot || !snapshot.credential) fail("stale_snapshot");
  const original = Object.freeze({ ...snapshot.credential });
  const expectedDirectory = snapshot.directoryIdentity, expectedCancellation = snapshot.cancellation;
  const opts = { signal: options.signal, lockTimeoutMs: options.lockTimeoutMs };
  await withSnapshot(directoryPath, { scope: original.scope as "read" | "upload", approvedServer: original.server }, resources, opts,
    async (directory, lease) => {
      checkSignal(opts.signal, lease.signal);
      if (directory.identity !== expectedDirectory) fail("stale_snapshot");
      if (await pendingIntent(directory.path)) fail("revocation_pending");
      const cancellation = await readCancellation(directory, resources, opts.signal);
      if (cancellation !== expectedCancellation) fail("stale_snapshot");
      const current = await readCredential(directory, { scope: original.scope as "read" | "upload", approvedServer: original.server }, resources, opts.signal);
      if (await pendingIntent(directory.path)) fail("revocation_pending");
      if (current.generation !== original.generation || current.server !== original.server || current.scope !== original.scope
        || current.grantId !== original.grantId || current.encryption_identity !== original.encryption_identity || current.authRequired) fail("stale_snapshot");
    }).catch(safeFailure);
}

export type CredentialRecoverySnapshot = Readonly<{
  snapshot: CredentialSnapshot; credential: StoredCredential; recovery: "stored" | "completed_response";
}>;
/** Reads completed writer evidence under queue.lock without another refresh request,
 * credential publication, marker repair or gate restoration. The original snapshot
 * remains the local incarnation fence; recovered expiries use journal.issued. */
export async function readCredentialRecoverySnapshot(directoryPath: string,
  context: { scope: "read" | "upload"; approvedServer: string }, resources: NamespaceResources,
  options: { signal?: AbortSignal; lockTimeoutMs?: number } = {}): Promise<CredentialRecoverySnapshot> {
  const ctx = { scope: context?.scope, approvedServer: context?.approvedServer } as typeof context;
  const opts = { signal: options.signal, lockTimeoutMs: options.lockTimeoutMs };
  return withSnapshot(directoryPath, ctx, resources, opts, async (directory, lease) => {
    checkSignal(opts.signal, lease.signal);
    if (await pendingIntent(directory.path)) fail("revocation_pending");
    const cancellation = await readCancellation(directory, resources, opts.signal);
    const stored = await readCredential(directory, ctx, resources, opts.signal);
    const snapshot = Object.freeze({ credential: stored, cancellation, directoryIdentity: directory.identity });
    const journal = await readOptionalStateFile(directory, `${ctx.scope}.renewal`, 65_536, resources, opts.signal, bytes => {
      try { return decodeRenewalJournal(bytes, ctx); }
      catch (error) { if (error instanceof RenewalJournalError) return null; throw error; }
    });
    let credential = stored, recovery: "stored" | "completed_response" = "stored";
    if (journal && isCompletedRenewalFor(stored, journal)) {
      const recovered = await readOptionalStateFile(directory, `${ctx.scope}.response`, 2_000_000, resources, opts.signal,
        bytes => recoverCompletedRenewal(stored, journal, bytes));
      if (recovered === null) return fail("invalid_credential");
      credential = recovered; recovery = "completed_response";
    }
    if (await pendingIntent(directory.path)) fail("revocation_pending");
    return Object.freeze({ snapshot, credential, recovery });
  }).catch(safeFailure);
}
