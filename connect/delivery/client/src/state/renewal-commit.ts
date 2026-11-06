import { lstat, unlink } from "node:fs/promises";
import path from "node:path";
import { decodeStoredCredential, type StoredCredential } from "../credentials/wire.ts";
import { decodeRenewalJournal, isCompletedRenewalFor, recoverCompletedRenewal, sameStoredCredential, RenewalJournalError } from "../credentials/renewal-journal.ts";
import type { CredentialSnapshot } from "./namespace.ts";
import { readCredential, readCancellation, pendingIntent, readStateEvidence, stateFileStamp as stamp } from "./credential-files.ts";
import { CredentialSnapshotError, checkSignal, samePathFd, withRenewalScope, withStateQueue, type NamespaceResources } from "./queue-transaction.ts";
import { replacePrivateStateFile, type StateReplacementResources } from "./replacement.ts";
import { PrivateFileError } from "./private-file.ts";

type Reason = "not_recoverable" | "stale_snapshot" | "revocation_pending" | "cancelled" | "lock_refused" | "unsafe_directory" | "unsafe_file" | "unavailable" | "publication_refused";
export type RenewalCommitResult =
  | { state: "committed"; credential: StoredCredential }
  | { state: "unchanged"; reason: Reason }
  | { state: "publication_uncertain" };
const reject = (reason: "stale_snapshot" | "revocation_pending" | "unsafe_file" | "unavailable"): never => { throw new CredentialSnapshotError(reason); };
const privateStat = (file: string) => lstat(file, { bigint: true });

/** Internal coordinated-writer preparation; no production caller or package export.
 * Caller must own the sole state writer before using this against live state.
 * Acquire scope then queue, recover only completed exact-base evidence, publish
 * the credential durably, then check evidence stamps before cleaning their paths.
 * Cleanup requires the legacy cooperative scope/queue protocol in the admitted
 * private namespace. Path check + unlink is not atomic against a same-UID process
 * bypassing those locks; that threat is outside the directory admission boundary.
 * No HTTP, expiry extension, config changes, retry or competing writer activation.
 */
export async function commitCompletedRenewal(directoryPath: string,
  context: { scope: "read" | "upload"; approvedServer: string }, expected: CredentialSnapshot,
  resources: NamespaceResources & StateReplacementResources,
  options: { signal?: AbortSignal; lockTimeoutMs?: number } = {}): Promise<RenewalCommitResult> {
  const publication: { state: "not_started" | "in_progress" | "verified" } = { state: "not_started" };
  let encoded: Buffer | undefined;
  try {
    const ctx = { scope: context.scope, approvedServer: context.approvedServer };
    const opts = { signal: options.signal, lockTimeoutMs: options.lockTimeoutMs };
    const expectedDirectory = expected.directoryIdentity, expectedCancellation = expected.cancellation;
    const snapshotBytes = Buffer.from(JSON.stringify(expected.credential));
    let original: StoredCredential;
    try { original = decodeStoredCredential(snapshotBytes, ctx.scope, ctx.approvedServer); }
    finally { snapshotBytes.fill(0); }
    return await withRenewalScope(directoryPath, ctx, resources, opts, async (scopeDirectory, scopeLease) => {
      if (scopeDirectory.identity !== expectedDirectory) reject("stale_snapshot");
      const signal = AbortSignal.any([scopeLease.signal, ...(opts.signal ? [opts.signal] : [])]);
      return withStateQueue(scopeDirectory.path, ctx, resources, { ...opts, signal }, async (directory, queueLease): Promise<RenewalCommitResult> => {
        const fence = () => { scopeLease.assertHeld(); queueLease.assertHeld(); checkSignal(signal, queueLease.signal); };
        const current = async () => {
          fence();
          if (directory.identity !== expectedDirectory || !await samePathFd(directory)) reject("stale_snapshot");
          if (await pendingIntent(directory.path)) reject("revocation_pending");
          if (await readCancellation(directory, resources, signal) !== expectedCancellation) reject("stale_snapshot");
          const credentialFile = path.join(directory.path, `${ctx.scope}.credential`);
          const before = stamp(await privateStat(credentialFile));
          const value = await readCredential(directory, ctx, resources, signal);
          if (stamp(await privateStat(credentialFile)) !== before) reject("unsafe_file");
          if (!sameStoredCredential(value, original) || value.authRequired) reject("stale_snapshot");
          if (await pendingIntent(directory.path)) reject("revocation_pending");
          fence(); return value;
        };
        const stored = await current();
        const journalName = `${ctx.scope}.renewal`, responseName = `${ctx.scope}.response`;
        const journal = await readStateEvidence(directory, journalName, 65_536, resources, signal, bytes => decodeRenewalJournal(bytes, ctx));
        if (!journal || !isCompletedRenewalFor(stored, journal.value)) return { state: "unchanged", reason: "not_recoverable" };
        const recovered = await readStateEvidence(directory, responseName, 2_000_000, resources, signal,
          bytes => recoverCompletedRenewal(stored, journal.value, bytes));
        if (!recovered) return { state: "unchanged", reason: "not_recoverable" };
        const evidence = [journal, recovered];
        await current();
        for (const { file, expectedStamp } of evidence) {
          const named = await privateStat(file);
          if (!named.isFile() || named.isSymbolicLink() || stamp(named) !== expectedStamp) reject("unsafe_file");
        }
        const credentialName = `${ctx.scope}.credential`;
        const destination = await privateStat(path.join(directory.path, credentialName));
        encoded = Buffer.from(JSON.stringify(recovered.value));
        // Every published record must remain readable by the existing bounded wire reader.
        let credential: StoredCredential;
        try { credential = decodeStoredCredential(encoded, ctx.scope, ctx.approvedServer); }
        catch { return { state: "unchanged", reason: "not_recoverable" }; }
        publication.state = "in_progress";
        const result = await replacePrivateStateFile(directory.path, credentialName, encoded,
          { device: String(destination.dev), inode: String(destination.ino) }, queueLease, resources, { signal });
        if (result.state === "publication_uncertain") return { state: "publication_uncertain" };
        if (result.state !== "committed") {
          publication.state = "not_started";
          return { state: "unchanged", reason: result.state === "destination_changed" ? "stale_snapshot" : "publication_refused" };
        }
        publication.state = "verified";
        if (await pendingIntent(directory.path)) reject("revocation_pending");
        if (await readCancellation(directory, resources, signal) !== expectedCancellation) reject("stale_snapshot");
        for (const { file, expectedStamp } of evidence) {
          fence(); if (!await samePathFd(directory)) reject("unavailable");
          const named = await privateStat(file);
          if (!named.isFile() || named.isSymbolicLink() || stamp(named) !== expectedStamp) reject("unavailable");
          // Supported mutators are excluded by our scope/queue leases. This is
          // an observed stamp check, not an atomic inode-conditional unlink.
          await unlink(file);
        }
        await directory.handle.sync(); fence();
        return { state: "committed", credential };
      });
    });
  } catch (error) {
    if (publication.state !== "not_started") return { state: "publication_uncertain" };
    if (error instanceof RenewalJournalError) return { state: "unchanged", reason: "not_recoverable" };
    if (error instanceof CredentialSnapshotError) return { state: "unchanged", reason: error.category === "invalid_credential" ? "not_recoverable" : error.category };
    if (error instanceof PrivateFileError) return { state: "unchanged", reason: error.category === "invalid_data" ? "not_recoverable" : error.category };
    return { state: "unchanged", reason: "unavailable" };
  } finally { encoded?.fill(0); }
}

export type CredentialRejectionResult =
  | { state: "marked" }
  | { state: "unchanged"; reason: Reason | "already_changed" }
  | { state: "publication_uncertain" };

/** Internal exact-record terminal rejection. Like the donor, queue.lock protects
 * the write; a scope lock is unnecessary because an in-flight renewal must fail
 * its exact-base check after rejection. Never clears journals, config or tokens.
 * Requires the same sole-writer ownership and cooperative namespace as recovery.
 */
export async function markCredentialAuthRequired(directoryPath: string,
  context: { scope: "read" | "upload"; approvedServer: string }, expected: CredentialSnapshot,
  resources: NamespaceResources & StateReplacementResources,
  options: { signal?: AbortSignal; lockTimeoutMs?: number } = {}): Promise<CredentialRejectionResult> {
  let publication: "not_started" | "dispatched" | "verified" = "not_started", bytes: Buffer | undefined;
  try {
    const ctx = { scope: context.scope, approvedServer: context.approvedServer };
    const opts = { signal: options.signal, lockTimeoutMs: options.lockTimeoutMs };
    const expectedDirectory = expected.directoryIdentity, expectedCancellation = expected.cancellation;
    const encoded = Buffer.from(JSON.stringify(expected.credential));
    let original: StoredCredential;
    try { original = Object.freeze(decodeStoredCredential(encoded, ctx.scope, ctx.approvedServer)); }
    finally { encoded.fill(0); }
    return await withStateQueue(directoryPath, ctx, resources, opts, async (directory, lease): Promise<CredentialRejectionResult> => {
      const signal = AbortSignal.any([lease.signal, ...(opts.signal ? [opts.signal] : [])]);
      const fence = async () => {
        lease.assertHeld(); checkSignal(signal);
        if (directory.identity !== expectedDirectory || !await samePathFd(directory)) reject("stale_snapshot");
        if (await pendingIntent(directory.path)) reject("revocation_pending");
        if (await readCancellation(directory, resources, signal) !== expectedCancellation) reject("stale_snapshot");
        lease.assertHeld(); checkSignal(signal);
      };
      await fence();
      const file = path.join(directory.path, `${ctx.scope}.credential`), before = await privateStat(file);
      const current = await readCredential(directory, ctx, resources, signal);
      if (stamp(await privateStat(file)) !== stamp(before)) reject("unsafe_file");
      if (!sameStoredCredential(current, original)) return { state: "unchanged", reason: "already_changed" };
      await fence();
      if (current.authRequired) return { state: "marked" };
      bytes = Buffer.from(JSON.stringify({ ...current, authRequired: true }));
      decodeStoredCredential(bytes, ctx.scope, ctx.approvedServer);
      publication = "dispatched";
      const result = await replacePrivateStateFile(directory.path, `${ctx.scope}.credential`, bytes,
        { device: String(before.dev), inode: String(before.ino) }, lease, resources, { signal });
      if (result.state === "publication_uncertain") return { state: "publication_uncertain" };
      if (result.state !== "committed") {
        publication = "not_started";
        return { state: "unchanged", reason: result.state === "destination_changed" ? "stale_snapshot" : "publication_refused" };
      }
      publication = "verified"; await fence();
      return { state: "marked" };
    });
  } catch (error) {
    if (publication !== "not_started") return { state: "publication_uncertain" };
    if (error instanceof CredentialSnapshotError) return { state: "unchanged", reason: error.category === "invalid_credential" ? "not_recoverable" : error.category };
    if (error instanceof PrivateFileError) return { state: "unchanged", reason: error.category === "invalid_data" ? "not_recoverable" : error.category };
    return { state: "unchanged", reason: "unavailable" };
  } finally { bytes?.fill(0); }
}
