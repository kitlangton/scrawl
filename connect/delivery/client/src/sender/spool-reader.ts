import { createHash } from "node:crypto";
import path from "node:path";
import type { Context } from "../recipient/crypto.ts";
import { decodeJob, encodeJob, type Job } from "../queue/job.ts";
import { admitTrustedDirectory, DirectoryAdmissionFailure } from "../state/directory.ts";
import { readCancellation, readCredential, pendingIntent, readOptionalStateFile, readStateEvidence } from "../state/credential-files.ts";
import { PrivateFileError } from "../state/private-file.ts";
import { checkSignal, samePathFd, withStateQueue, CredentialSnapshotError,
  type NamespaceResources, type NamespaceLease, type StateDirectory } from "../state/queue-transaction.ts";
import { decodeStoredEncryptedSpool, StoredSpoolDecodeError, type StoredEncryptedSpool, type StoredSpoolProfile } from "./spool-record.ts";
import { readPrivateSpoolAsset, SpoolAssetParentChanged } from "./spool-files.ts";

type Failure = "invalid_encrypted_spool" | "stale_snapshot" | "revocation_pending" | "unsafe_file" | "unsafe_directory" | "cancelled" | "state_unavailable";
const failures = new Set<Failure>(["invalid_encrypted_spool", "stale_snapshot", "revocation_pending", "unsafe_file", "unsafe_directory", "cancelled", "state_unavailable"]);
export class EncryptedSpoolReadError extends Error {
  readonly category: Failure;
  constructor(category: Failure) { super(category); this.category = category; this.name = "EncryptedSpoolReadError"; }
}
const fail = (category: Failure): never => { throw new EncryptedSpoolReadError(category); };
function project(error: unknown, signal?: AbortSignal): never {
  if (signal?.aborted) return fail("cancelled");
  if (error instanceof EncryptedSpoolReadError && failures.has(error.category)) return fail(error.category);
  if (error instanceof DirectoryAdmissionFailure && (error.category === "unsafe_directory" || error.category === "cancelled")) return fail(error.category);
  if (error instanceof StoredSpoolDecodeError) return fail("invalid_encrypted_spool");
  if (error instanceof SpoolAssetParentChanged) return fail("stale_snapshot");
  if (error instanceof PrivateFileError && error.category === "invalid_data") return fail("invalid_encrypted_spool");
  if (error instanceof PrivateFileError || error instanceof CredentialSnapshotError) {
    if (error.category === "cancelled" || error.category === "unsafe_file" || error.category === "unsafe_directory") return fail(error.category);
  }
  return fail("state_unavailable");
}
type Options = { signal?: AbortSignal; lockTimeoutMs?: number };
export type CommittedEncryptedSpool = Readonly<{
  profile: StoredSpoolProfile;
  record: StoredEncryptedSpool;
  /** Returns caller-owned ciphertext/envelope bytes. The caller must wipe its
   * buffer after its consumer settles. No descriptor or lease escapes. */
  readAsset(name: string, signal?: AbortSignal): Promise<Uint8Array>;
}>;
type Fence = { cancellation: string | null; identity: string | null; grant: string | null };
const hash = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
function jobIdentity(job: Job): string {
  const c = job.encryption?.context;
  return JSON.stringify([job.id, job.bundle, job.server, job.generation, job.created, job.status, job.attempts,
    job.nextAttempt, job.error, job.sizeFailure && [job.sizeFailure.path, job.sizeFailure.size, job.sizeFailure.limit],
    job.shared, job.publicCopy, job.encrypted, job.grouped,
    job.files.map(f => [f.path, f.size, f.sha256]).sort((a, b) => String(a[0]).localeCompare(String(b[0]))),
    c && [c.origin, c.accountId, c.captureId, c.senderGrantId, c.group], job.encryption && [...job.encryption.recipientGrants].sort()]);
}

/** Open only a spool already committed by the authoritative writer. This reader
 * never repairs metadata, creates a spool or starts a transfer. The caller owns
 * provider lifetime and current service authorization; this is a local fence. */
export async function openCommittedEncryptedSpool(directoryPath: string, input: Job,
  resources: NamespaceResources, options: Options = {}): Promise<CommittedEncryptedSpool> {
  const initialSignal = options.signal, lockTimeoutMs = options.lockTimeoutMs;
  try {
    const job = decodeJob(encodeJob(input));
    if (!job.encrypted || job.shared || job.publicCopy || !job.encryption || !job.files.length
      || !["pending", "ready"].includes(job.status) || !/^[a-f0-9]{32}$/.test(job.generation)) return fail("invalid_encrypted_spool");
    const context = { ...job.encryption.context } as Context;
    if (context.origin !== job.server || context.captureId !== job.id || context.group !== undefined) return fail("invalid_encrypted_spool");
    const profile = job.grouped ? "evidence-first-v1" : "root";
    const expected = { context, profile } as const;
    const committed = decodeStoredEncryptedSpool(new TextEncoder().encode(JSON.stringify({ metadata: job.encryption, files: job.files })), expected);
    const expectedJob = jobIdentity(job);
    const check = (signal: AbortSignal, lease: NamespaceLease) => { checkSignal(signal, lease.signal); lease.assertHeld(); };
    async function facts(state: StateDirectory, lease: NamespaceLease, signal: AbortSignal): Promise<Fence> {
      check(signal, lease);
      if (await pendingIntent(state.path)) return fail("revocation_pending");
      const current = await readOptionalStateFile(state, `${job.id}.job`, 2_000_000, resources, signal, decodeJob);
      if (!current || jobIdentity(current) !== expectedJob) return fail("stale_snapshot");
      const credential = await readCredential(state, { scope: "upload", approvedServer: job.server }, resources, signal);
      if (credential.generation !== job.generation || credential.authRequired
        || credential.grantId !== null && credential.grantId !== context.senderGrantId) return fail("stale_snapshot");
      const cancellation = await readCancellation(state, resources, signal);
      check(signal, lease);
      if (await pendingIntent(state.path)) return fail("revocation_pending");
      return { cancellation, identity: credential.encryption_identity, grant: credential.grantId };
    }
    const combine = (signal?: AbortSignal) => AbortSignal.any([initialSignal, signal].filter((s): s is AbortSignal => s !== undefined));
    async function transaction<T>(root: string, signal: AbortSignal,
      body: (state: StateDirectory, spool: StateDirectory, lease: NamespaceLease) => Promise<T>): Promise<T> {
      return withStateQueue(root, { scope: "upload", approvedServer: job.server }, resources, { signal, lockTimeoutMs }, async (state, lease) => {
        const spool = await admitTrustedDirectory(path.join(state.path, `${job.id}.sealed`), resources, "private-state", signal);
        try {
          const value = await body(state, spool, lease);
          check(signal, lease);
          if (!await samePathFd(spool)) return fail("stale_snapshot");
          return value;
        } finally { await spool.handle.close().catch(() => {}); }
      });
    }
    async function receipt(spool: StateDirectory, signal: AbortSignal) {
      const value = await readStateEvidence(spool, "spool-state.json", 2_000_000, resources, signal,
        bytes => ({ record: decodeStoredEncryptedSpool(bytes, expected), digest: hash(bytes) }));
      if (!value) return fail("invalid_encrypted_spool");
      return value;
    }
    const baseline = await transaction(directoryPath, combine(), async (state, spool, lease) => {
      const fence = await facts(state, lease, combine());
      const saved = await receipt(spool, combine());
      if (JSON.stringify([...saved.value.record.metadata.recipientGrants].sort()) !== JSON.stringify([...committed.metadata.recipientGrants].sort())
        || JSON.stringify(saved.value.record.files.map(f => [f.path, f.size, f.sha256]).sort())
          !== JSON.stringify(committed.files.map(f => [f.path, f.size, f.sha256]).sort())) return fail("stale_snapshot");
      if (JSON.stringify(await facts(state, lease, combine())) !== JSON.stringify(fence)) return fail("stale_snapshot");
      return { root: state.path, stateIdentity: state.identity, spoolIdentity: spool.identity, fence,
        receiptStamp: saved.expectedStamp, receiptDigest: saved.value.digest, record: saved.value.record };
    });
    async function assertBaseline(state: StateDirectory, spool: StateDirectory, lease: NamespaceLease, signal: AbortSignal) {
      if (state.identity !== baseline.stateIdentity || spool.identity !== baseline.spoolIdentity) return fail("stale_snapshot");
      if (JSON.stringify(await facts(state, lease, signal)) !== JSON.stringify(baseline.fence)) return fail("stale_snapshot");
      const saved = await receipt(spool, signal);
      if (saved.expectedStamp !== baseline.receiptStamp || saved.value.digest !== baseline.receiptDigest) return fail("stale_snapshot");
    }
    return Object.freeze({ profile, record: baseline.record, async readAsset(name: string, callerSignal?: AbortSignal) {
      const signal = combine(callerSignal);
      let owned: Uint8Array | undefined;
      try {
        const asset = baseline.record.files.find(file => file.path === name);
        if (!asset) return fail("invalid_encrypted_spool");
        return await transaction(baseline.root, signal, async (state, spool, lease) => {
          await assertBaseline(state, spool, lease, signal);
          const read = await readPrivateSpoolAsset(spool, asset.path, asset.size, resources, signal);
          owned = read.bytes;
          if (owned.length !== asset.size || hash(owned) !== asset.sha256) return fail("unsafe_file");
          await assertBaseline(state, spool, lease, signal);
          return owned!;
        });
      } catch (error) { owned?.fill(0); return project(error, signal); }
    } });
  } catch (error) { return project(error, initialSignal); }
}
