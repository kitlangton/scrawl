import type { BigIntStats } from "node:fs";
import { lstat, unlink } from "node:fs/promises";
import path from "node:path";
import { decodeStoredCredential, type StoredCredential } from "../credentials/wire.ts";
import { decodeRenewalJournal, sameStoredCredential, type RenewalJournal } from "../credentials/renewal-journal.ts";
import { planRenewal } from "../credentials/renewal-plan.ts";
import { requestRenewalReply, RenewalRequestError, type RenewalRequestFailure } from "../credentials/renewal-request.ts";
import { readCredentialSnapshot, type CredentialSnapshot } from "./namespace.ts";
import { commitCompletedRenewal, markCredentialAuthRequired } from "./renewal-commit.ts";
import { pendingIntent, readCancellation, readCredential, readStateEvidence, stateFileStamp } from "./credential-files.ts";
import { CredentialSnapshotError, checkSignal, samePathFd, withRenewalScope, withStateQueue, type NamespaceResources, type SnapshotFailure, type StateDirectory } from "./queue-transaction.ts";
import { replacePrivateStateFile, type StateReplacementResources } from "./replacement.ts";
import { PrivateFileError } from "./private-file.ts";

type Phase = "before_request" | "request_sent" | "response_saved";
type StopReason = SnapshotFailure | "invalid_request" | "publication_refused";
export type RenewalExchangeResult =
  | { state: "use_current"; credential: StoredCredential }
  | { state: "completed" }
  | { state: "auth_required" }
  | { state: "request_failed"; reason: RenewalRequestFailure }
  | { state: "stopped"; phase: Phase; reason: StopReason }
  | { state: "publication_uncertain"; phase: Phase };
const fail = (category: SnapshotFailure): never => { throw new CredentialSnapshotError(category); };
class WriteRefused extends Error {}
type RenewalOptions = { force?: boolean; signal?: AbortSignal; timeoutMs?: number; lockTimeoutMs?: number;
  now?: () => number; request?: typeof requestRenewalReply };

/** Internal sole-writer prerequisite, without a production caller or public export.
 * Prepare a legacy journal before one POST; persist raw response bytes before the
 * completed marker. Never parse a newly received reply, publish credentials, mark
 * authRequired, retry HTTP or activate a competing writer. The existing completed
 * recovery transaction owns merge/publication. A stopped/failed exchange may leave
 * a prepared journal or orphan response; it is not authority to repeat a POST.
 * All supported state mutators must honor the cooperative scope/queue protocol
 * within the admitted private namespace (not a hostile same-UID boundary).
 */
export async function persistRenewalExchange(directoryPath: string,
  context: { scope: "read" | "upload"; approvedServer: string }, expected: CredentialSnapshot,
  resources: NamespaceResources & StateReplacementResources,
  options: RenewalOptions = {}): Promise<RenewalExchangeResult> {
  let phase: Phase = "before_request", uncertain = false, completedPublished = false, response: Uint8Array | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    const ctx = { scope: context.scope, approvedServer: context.approvedServer };
    const { force = false, timeoutMs = 30_000, lockTimeoutMs = 1000, now = () => Math.floor(Date.now() / 1000), request = requestRenewalReply } = options;
    if (typeof force !== "boolean" || !Number.isSafeInteger(timeoutMs) || timeoutMs < 1 || timeoutMs > 30_000)
      return { state: "stopped", phase, reason: "invalid_request" };
    const deadline = performance.now() + timeoutMs, controller = new AbortController();
    timer = setTimeout(() => controller.abort(), timeoutMs);
    const callerSignal = options.signal;
    const timedSignal = AbortSignal.any([controller.signal, ...(callerSignal ? [callerSignal] : [])]);
    const expectedDirectory = expected.directoryIdentity, expectedCancellation = expected.cancellation;
    const encoded = Buffer.from(JSON.stringify(expected.credential));
    let original: StoredCredential;
    try { original = Object.freeze(decodeStoredCredential(encoded, ctx.scope, ctx.approvedServer)); }
    finally { encoded.fill(0); }
    return await withRenewalScope(directoryPath, ctx, resources, { signal: timedSignal, lockTimeoutMs }, async (scopeDirectory, scopeLease) => {
      if (scopeDirectory.identity !== expectedDirectory) fail("stale_snapshot");
      const signal = AbortSignal.any([timedSignal, scopeLease.signal]);
      const queue = <T>(body: (directory: StateDirectory, fence: { assertHeld(): void; signal: AbortSignal }) => Promise<T>) =>
        withStateQueue(scopeDirectory.path, ctx, resources, { signal, lockTimeoutMs }, async (directory, queueLease) => {
          const fence = { signal: AbortSignal.any([signal, queueLease.signal]), assertHeld() {
            scopeLease.assertHeld(); queueLease.assertHeld(); checkSignal(signal, queueLease.signal);
          } };
          fence.assertHeld(); return body(directory, fence);
        });
      const current = async (directory: StateDirectory, signal: AbortSignal, exact: boolean) => {
        scopeLease.assertHeld(); checkSignal(signal);
        if (directory.identity !== expectedDirectory || !await samePathFd(directory)) fail("stale_snapshot");
        if (await pendingIntent(directory.path)) fail("revocation_pending");
        if (await readCancellation(directory, resources, signal) !== expectedCancellation) fail("stale_snapshot");
        const file = path.join(directory.path, `${ctx.scope}.credential`), before = stateFileStamp(await lstat(file, { bigint: true }));
        const value = await readCredential(directory, ctx, resources, signal);
        if (stateFileStamp(await lstat(file, { bigint: true })) !== before) fail("unsafe_file");
        if (value.generation !== original.generation || value.grantId !== original.grantId || value.encryption_identity !== original.encryption_identity
          || (exact && !sameStoredCredential(value, original))) fail("stale_snapshot");
        if (await pendingIntent(directory.path)) fail("revocation_pending");
        if (await readCancellation(directory, resources, signal) !== expectedCancellation) fail("stale_snapshot");
        checkSignal(signal); scopeLease.assertHeld(); return value;
      };
      const write = async (directory: StateDirectory, name: string, bytes: Uint8Array, prior: BigIntStats | null,
        fence: { assertHeld(): void; signal: AbortSignal }) => {
        fence.assertHeld();
        uncertain = true;
        const result = await replacePrivateStateFile(directory.path, name, bytes,
          prior === null ? null : { device: String(prior.dev), inode: String(prior.ino) }, fence, resources, { signal: fence.signal });
        if (result.state === "publication_uncertain") throw new Error();
        uncertain = false;
        if (result.state !== "committed") throw new WriteRefused();
      };
      const journalName = `${ctx.scope}.renewal`, responseName = `${ctx.scope}.response`;
      const prepared = await queue(async (directory, fence) => {
        const stored = await current(directory, fence.signal, false);
        const early = planRenewal({ expected: original, current: stored, now: now(), force,
          cancelled: false, revocationPending: false, journal: null, responseBytes: null });
        if (early.kind === "use_current" || (early.kind === "auth_required" && early.reason !== "refresh_expired")) {
          if (!sameStoredCredential(await current(directory, fence.signal, false), stored)) fail("stale_snapshot");
          return { result: early.kind === "use_current" ? { state: "use_current", credential: early.credential } as RenewalExchangeResult
            : { state: "auth_required" } as RenewalExchangeResult };
        }
        const journal = await readStateEvidence(directory, journalName, 65_536, resources, fence.signal, bytes => {
          try { return { journal: decodeRenewalJournal(bytes, ctx) }; } catch { return { journal: null }; }
        });
        const saved = await readStateEvidence(directory, responseName, 2_000_000, resources, fence.signal, bytes => new Uint8Array(bytes));
        let plan;
        try { plan = planRenewal({ expected: original, current: stored, now: now(), force,
          cancelled: false, revocationPending: false, journal: journal?.value.journal ?? null, responseBytes: saved?.value ?? null }); }
        finally { saved?.value.fill(0); }
        if (!sameStoredCredential(await current(directory, fence.signal, false), stored)) fail("stale_snapshot");
        if (plan.kind === "use_current") return { result: { state: "use_current", credential: plan.credential } as RenewalExchangeResult };
        if (plan.kind === "recover_completed") return { result: { state: "completed" } as RenewalExchangeResult };
        if (plan.kind === "auth_required") return { result: { state: "auth_required" } as RenewalExchangeResult };
        if (plan.kind !== "prepare_request") return fail("stale_snapshot");
        // Recheck observed evidence before clearing an orphan or replacing a journal.
        for (const evidence of [journal, saved]) if (evidence && stateFileStamp(await lstat(evidence.file, { bigint: true })) !== evidence.expectedStamp) fail("unsafe_file");
        await current(directory, fence.signal, true); fence.assertHeld();
        if (saved) { await unlink(saved.file); await directory.handle.sync(); }
        const bytes = Buffer.from(JSON.stringify(plan.journal));
        try {
          decodeRenewalJournal(bytes, ctx);
          await write(directory, journalName, bytes, journal ? await lstat(journal.file, { bigint: true }) : null, fence);
        } finally { bytes.fill(0); }
        await current(directory, fence.signal, true);
        const journalStamp = stateFileStamp(await lstat(path.join(directory.path, journalName), { bigint: true }));
        return { journal: plan.journal, journalStamp };
      });
      if ("result" in prepared) return prepared.result!;
      scopeLease.assertHeld(); checkSignal(signal);
      const remaining = Math.min(30_000, Math.floor(deadline - performance.now()));
      if (remaining < 1) fail("cancelled");
      phase = "request_sent";
      try { response = await request(original, { approvedServer: ctx.approvedServer, signal, timeoutMs: remaining }); }
      catch (error) {
        return { state: "request_failed", reason: error instanceof RenewalRequestError ? error.category : "request_failed" };
      }
      if (!(response instanceof Uint8Array) || response.length > 2_000_000)
        return { state: "request_failed", reason: "response_too_large" };
      return queue(async (directory, fence): Promise<RenewalExchangeResult> => {
        await current(directory, fence.signal, true);
        const journalFile = path.join(directory.path, journalName);
        if (stateFileStamp(await lstat(journalFile, { bigint: true })) !== prepared.journalStamp) fail("unsafe_file");
        await write(directory, responseName, response!, null, fence);
        phase = "response_saved";
        const responseFile = path.join(directory.path, responseName);
        const responseStamp = stateFileStamp(await lstat(responseFile, { bigint: true }));
        await current(directory, fence.signal, true);
        if (stateFileStamp(await lstat(journalFile, { bigint: true })) !== prepared.journalStamp) fail("unsafe_file");
        if (stateFileStamp(await lstat(responseFile, { bigint: true })) !== responseStamp) fail("unsafe_file");
        const bytes = Buffer.from(JSON.stringify({ ...prepared.journal, completed: true } satisfies RenewalJournal));
        try { await write(directory, journalName, bytes, await lstat(journalFile, { bigint: true }), fence); completedPublished = true; }
        finally { bytes.fill(0); }
        await current(directory, fence.signal, true); fence.assertHeld();
        if (stateFileStamp(await lstat(responseFile, { bigint: true })) !== responseStamp) fail("unsafe_file");
        return { state: "completed" };
      });
    });
  } catch (error) {
    if (uncertain || completedPublished) return { state: "publication_uncertain", phase };
    const reason: StopReason = error instanceof WriteRefused ? "publication_refused" : error instanceof CredentialSnapshotError ? error.category
      : error instanceof PrivateFileError ? error.category === "invalid_data" ? "invalid_credential" : error.category : "unavailable";
    return { state: "stopped", phase, reason };
  } finally { if (timer !== undefined) clearTimeout(timer); if (response instanceof Uint8Array) response.fill(0); }
}

export type CredentialRenewalResult =
  | { state: "credential"; credential: StoredCredential; source: "stored" | "renewed" | "other_writer" }
  | { state: "auth_required" }
  | { state: "request_failed"; reason: RenewalRequestFailure }
  | { state: "stopped"; reason: StopReason | "not_recoverable" }
  | { state: "publication_uncertain" };

/** Internal coordinated-writer workflow, not live writer authority or a public
 * package export. One deadline includes exchange, recovery and terminal marking.
 * Scope leases are released between the exchange and recovery transactions: a
 * cooperating winner is reloaded, never overwritten or followed by a second POST.
 * Ambiguous exchange/commit results stop immediately, retaining recovery evidence.
 * A credential result reports stored facts, not unexpired access or service
 * authorization; consumers retain request-time lifetime/authorization checks.
 */
export async function renewStateCredential(directoryPath: string,
  context: { scope: "read" | "upload"; approvedServer: string }, expected: CredentialSnapshot,
  resources: NamespaceResources & StateReplacementResources,
  options: RenewalOptions = {}): Promise<CredentialRenewalResult> {
  let timer: ReturnType<typeof setTimeout> | undefined;
  try {
    const ctx = { scope: context.scope, approvedServer: context.approvedServer }, opts = { ...options };
    const timeout = opts.timeoutMs ?? 30_000;
    if (!Number.isSafeInteger(timeout) || timeout < 1 || timeout > 30_000) return { state: "stopped", reason: "invalid_request" };
    const controller = new AbortController(), deadline = performance.now() + timeout;
    timer = setTimeout(() => controller.abort(), timeout);
    const signal = AbortSignal.any([controller.signal, ...(opts.signal ? [opts.signal] : [])]);
    const encoded = Buffer.from(JSON.stringify(expected.credential));
    let credential: StoredCredential;
    try { credential = Object.freeze(decodeStoredCredential(encoded, ctx.scope, ctx.approvedServer)); }
    finally { encoded.fill(0); }
    const snapshot = Object.freeze({ credential, directoryIdentity: expected.directoryIdentity, cancellation: expected.cancellation });
    const stateOptions = { signal, lockTimeoutMs: opts.lockTimeoutMs };
    const observedChange = async (): Promise<CredentialRenewalResult | null> => {
      const current = await readCredentialSnapshot(directoryPath, ctx, resources, stateOptions);
      if (current.directoryIdentity !== snapshot.directoryIdentity || current.cancellation !== snapshot.cancellation
        || current.credential.generation !== credential.generation || current.credential.grantId !== credential.grantId
        || current.credential.encryption_identity !== credential.encryption_identity) fail("stale_snapshot");
      checkSignal(signal);
      if (current.credential.authRequired) return { state: "auth_required" };
      return sameStoredCredential(current.credential, credential) ? null
        : { state: "credential", source: "other_writer", credential: Object.freeze({ ...current.credential }) };
    };
    const rejectCurrent = async (): Promise<CredentialRenewalResult> => {
      const result = await markCredentialAuthRequired(directoryPath, ctx, snapshot, resources, stateOptions);
      if (result.state === "marked") return { state: "auth_required" };
      if (result.state === "publication_uncertain") return result;
      if (result.reason === "already_changed") return await observedChange() ?? { state: "stopped", reason: "stale_snapshot" };
      return { state: "stopped", reason: result.reason };
    };
    const remaining = Math.floor(deadline - performance.now());
    if (remaining < 1) fail("cancelled");
    const exchange = await persistRenewalExchange(directoryPath, ctx, snapshot, resources, { ...opts, signal, timeoutMs: remaining });
    if (exchange.state === "publication_uncertain") return { state: "publication_uncertain" };
    if (exchange.state === "stopped") return { state: "stopped", reason: exchange.reason };
    if (exchange.state === "request_failed") return exchange.reason === "auth_required" ? await rejectCurrent() : exchange;
    if (exchange.state === "auth_required") return await rejectCurrent();
    checkSignal(signal);
    if (exchange.state === "use_current") return { state: "credential", credential: exchange.credential,
      source: sameStoredCredential(exchange.credential, credential) ? "stored" : "other_writer" };
    const committed = await commitCompletedRenewal(directoryPath, ctx, snapshot, resources, stateOptions);
    if (committed.state === "committed") return { state: "credential", credential: Object.freeze({ ...committed.credential }), source: "renewed" };
    if (committed.state === "publication_uncertain") return committed;
    if (committed.reason === "stale_snapshot" || committed.reason === "not_recoverable") {
      const changed = await observedChange();
      if (changed) return changed;
      if (committed.reason === "not_recoverable") return await rejectCurrent();
    }
    return { state: "stopped", reason: committed.reason };
  } catch (error) {
    return { state: "stopped", reason: error instanceof CredentialSnapshotError ? error.category : "unavailable" };
  } finally { if (timer !== undefined) clearTimeout(timer); }
}
