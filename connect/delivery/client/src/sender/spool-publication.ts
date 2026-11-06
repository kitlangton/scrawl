import { createHash } from "node:crypto";
import path from "node:path";
import { decodeJob, encodeJob, type Job } from "../queue/job.ts";
import { normalizeRecipientServer } from "../recipient/transport.ts";
import { verifyEnvelope } from "../recipient/crypto.ts";
import { validateRecipientEnvelopeBytesWithVerifier, RecipientEnvelopeError } from "../recipient-envelope.ts";
import { publishPrivateDirectory, PrivateDirectoryError, type PrivateDirectoryResources, type OwnedPrivateFile } from "../recipient/private-directory.ts";
import { withStateQueue, samePathFd, checkSignal, CredentialSnapshotError, type NamespaceResources,
  type StateDirectory, type NamespaceLease } from "../state/queue-transaction.ts";
import { pendingIntent, readCancellation, readCredential, readOptionalStateFile } from "../state/credential-files.ts";
import type { CredentialSnapshot } from "../state/namespace.ts";
import { admitTrustedDirectory, directoryIdentity } from "../state/directory.ts";
import { decodeStoredEncryptedSpool, StoredSpoolDecodeError, type StoredEncryptedSpool } from "./spool-record.ts";
import type { SealedCapture } from "./seal.ts";

type Failure = "invalid_spool" | "stale_snapshot" | "revocation_pending" | "cancelled" | "state_unavailable" | "destination_exists" | "publication_refused";
export class SpoolPublicationError extends Error {
  readonly category: Failure;
  constructor(category: Failure) { super(category); this.category = category; this.name = "SpoolPublicationError"; }
}
const fail = (category: Failure): never => { throw new SpoolPublicationError(category); };
const hash = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
export type SpoolPublicationResources = NamespaceResources & PrivateDirectoryResources;
type Options = { signal?: AbortSignal; lockTimeoutMs?: number; active(): boolean };
type Result = { state: "committed"; path: string; record: StoredEncryptedSpool } | { state: "publication-uncertain" };

/** INTERNAL PREPARATION ONLY. Rust remains the sole live writer. Stages outside
 * queue.lock, then rechecks caller consent and the local incarnation under that
 * lease before one native no-replace publication. Does not commit job JSON.
 * Caller supplies current local signing-key and credential-snapshot facts,
 * owns provider lifetime and must coordinate sole-writer adoption separately. */
export async function publishEncryptedSpool(directoryPath: string, input: Job, sealed: SealedCapture,
  snapshot: CredentialSnapshot, trustedSenderSigningKey: Uint8Array, resources: SpoolPublicationResources,
  options: Options): Promise<Result> {
  const copies: OwnedPrivateFile[] = [];
  let key: Uint8Array | undefined;
  const signal = options?.signal, active = options?.active, lockTimeoutMs = options?.lockTimeoutMs;
  const check = () => {
    checkSignal(signal);
    try { if (typeof active !== "function" || !active()) fail("stale_snapshot"); } catch { fail("stale_snapshot"); }
    checkSignal(signal);
  };
  let effect: "none" | "possible" | "confirmed" = "none";
  try {
    check();
    const job = decodeJob(encodeJob(input)), expectedJob = hash(encodeJob(job));
    const source = snapshot?.credential;
    const fence = { directory: snapshot?.directoryIdentity, cancellation: snapshot?.cancellation,
      generation: source?.generation, grant: source?.grantId, identity: source?.encryption_identity };
    if (!job.encrypted || job.shared || job.publicCopy || job.status !== "pending" || job.files.length || job.encryption
      || normalizeRecipientServer(job.server) !== job.server || source?.scope !== "upload" || source.server !== job.server
      || source.authRequired || source.generation !== job.generation || !/^[a-f0-9]{32}$/.test(job.generation)
      || !source.encryption_identity || !source.grantId || !fence.directory) fail("invalid_spool");
    const context = { origin: sealed.context.origin, accountId: sealed.context.accountId,
      captureId: sealed.context.captureId, senderGrantId: sealed.context.senderGrantId }, profile = sealed.profile;
    if (context.origin !== job.server || context.captureId !== job.id || sealed.context.group !== undefined
      || context.senderGrantId !== source.grantId || typeof context.accountId !== "string" || !/^[\x21-\x7e]{1,256}$/.test(context.accountId)
      || (profile === "evidence-first-v1") !== job.grouped || !["root", "evidence-first-v1"].includes(profile)
      || !(trustedSenderSigningKey instanceof Uint8Array) || trustedSenderSigningKey.length !== 32
      || !Array.isArray(sealed.recipientGrantIds) || sealed.recipientGrantIds.length > 64
      || sealed.recipientGrantIds.some(id => typeof id !== "string" || !/^[\x21-\x7e]{1,256}$/.test(id))) fail("invalid_spool");
    key = new Uint8Array(trustedSenderSigningKey);
    if (profile === "root" ? !Array.isArray(sealed.chunks) || sealed.chunks.length < 1 || sealed.chunks.length > 16
      : !Array.isArray(sealed.groups) || sealed.groups.length !== 2 || sealed.groups.some(group =>
        !Array.isArray(group.chunks) || group.chunks.length < 1 || group.chunks.length > 16)) fail("invalid_spool");
    const descriptors = profile === "root" ? [sealed.envelope, ...sealed.chunks] : sealed.groups.flatMap(group => [group.envelope, ...group.chunks]);
    if (descriptors.some(file => !file || typeof file.path !== "string" || file.path.length > 128
      || typeof file.sha256 !== "string" || !/^[a-f0-9]{64}$/.test(file.sha256) || !Number.isSafeInteger(file.size))) fail("invalid_spool");
    const files = descriptors.map(({ path, size, sha256 }) => ({ path, size, sha256 }));
    const raw = new TextEncoder().encode(JSON.stringify({ metadata: { context, recipientGrants: [...sealed.recipientGrantIds] }, files }));
    copies.push({ path: "spool-state.json", bytes: raw });
    const receiptHash = hash(raw);
    const record = decodeStoredEncryptedSpool(raw, { context, profile });
    if (!record.metadata.recipientGrants.includes(context.senderGrantId)
      || !Array.isArray(sealed.assets) || sealed.assets.length !== record.files.length) fail("invalid_spool");
    const names = new Set<string>();
    for (const asset of sealed.assets) {
      const expected = record.files.find(file => file.path === asset.path);
      if (!expected) return fail("invalid_spool");
      if (names.has(asset.path) || !(asset.bytes instanceof Uint8Array) || asset.bytes.length !== expected.size) fail("invalid_spool");
      names.add(asset.path);
      const bytes = new Uint8Array(asset.bytes); copies.push({ path: expected.path, bytes });
      if (hash(bytes) !== expected.sha256) fail("invalid_spool");
    }
    // Check the actual envelope/chunk metadata, rather than trusting a mutable
    // sealer result merely because it originated from the sealer once.
    for (const group of profile === "root" ? [undefined] : ["evidence", "media"] as const) {
      const prefix = group ? `${group}/` : "";
      const envelope = copies.find(file => file.path === `${prefix}envelope.json`)!;
      const verified = await validateRecipientEnvelopeBytesWithVerifier(envelope.bytes, { ...context, ...(group ? { group } : {}) }, key, verifyEnvelope);
      const chunks = record.files.filter(file => file.path.startsWith(`${prefix}sealed/`)).sort((a,b) => a.path.localeCompare(b.path));
      if (verified.signed.chunks.length !== chunks.length || verified.signed.chunks.some((file, i) =>
        file.path !== chunks[i]!.path.slice(prefix.length) || file.size !== chunks[i]!.size || file.sha256 !== chunks[i]!.sha256)) fail("invalid_spool");
    }
    async function current(state: StateDirectory, lease: NamespaceLease) {
      check(); checkSignal(signal, lease.signal); lease.assertHeld();
      if (state.identity !== fence.directory || !await samePathFd(state)) fail("stale_snapshot");
      if (await pendingIntent(state.path)) fail("revocation_pending");
      const currentJob = await readOptionalStateFile(state, `${job.id}.job`, 2_000_000, resources, signal, decodeJob);
      if (!currentJob || hash(encodeJob(currentJob)) !== expectedJob) fail("stale_snapshot");
      const credential = await readCredential(state, { scope: "upload", approvedServer: job.server }, resources, signal);
      if (credential.authRequired || credential.generation !== fence.generation || credential.grantId !== fence.grant
        || credential.encryption_identity !== fence.identity || await readCancellation(state, resources, signal) !== fence.cancellation) fail("stale_snapshot");
      if (await pendingIntent(state.path)) fail("revocation_pending");
      check(); checkSignal(signal, lease.signal); lease.assertHeld();
    }
    const transaction = <T>(body: (state: StateDirectory, lease: NamespaceLease) => Promise<T>) => withStateQueue(directoryPath,
      { scope: "upload", approvedServer: job.server }, resources, { signal, lockTimeoutMs }, body);
    const root = await transaction(async (state, lease) => { await current(state, lease); return state.path; });
    let refused: unknown;
    const guarded: PrivateDirectoryResources = {
      checkAcl: handle => resources.checkAcl(handle),
      async publish(parent, stage, destination) {
        let dispatched = false;
        try {
          return await transaction(async (state, lease) => {
            await current(state, lease);
            if (directoryIdentity(await parent.stat({ bigint: true })) !== state.identity) fail("stale_snapshot");
            checkSignal(signal, lease.signal);
            dispatched = true;
            effect = "possible";
            const result = await resources.publish(parent, stage, destination,
              { signal: AbortSignal.any([lease.signal, ...(signal ? [signal] : [])]) });
            effect = result.state === "published" ? "confirmed" : result.state === "refused" || result.state === "destination_exists" ? "none" : "possible";
            return result;
          });
        } catch (error) {
          if (dispatched) { effect = "possible"; return { state: "publication_uncertain" }; }
          await parent.close().catch(() => {}); refused = error; return { state: "refused" };
        }
      },
    };
    let result: Awaited<ReturnType<typeof publishPrivateDirectory>>;
    try { result = await publishPrivateDirectory(root, path.join(root, `${job.id}.sealed`), copies, guarded, { signal }); }
    catch (error) { throw refused ?? error; }
    if (result.state === "publication-uncertain") return result;
    // Receipt readback and fresh fences are short queue transactions. Neither
    // this capability nor the core publishes job/credential/config JSON.
    await transaction(async (state, lease) => {
      await current(state, lease);
      const spool = await admitTrustedDirectory(result.path, resources, "private-state", signal);
      try {
        const saved = await readOptionalStateFile(spool, "spool-state.json", 2_000_000, resources, signal, bytes => hash(bytes));
        if (saved !== receiptHash || !await samePathFd(spool)) fail("invalid_spool");
      } finally { await spool.handle.close().catch(() => {}); }
      await current(state, lease);
    });
    return { state: "committed", path: result.path, record };
  } catch (error) {
    if (effect !== "none") return { state: "publication-uncertain" };
    if (error instanceof SpoolPublicationError) throw new SpoolPublicationError(error.category);
    if (error instanceof StoredSpoolDecodeError || error instanceof RecipientEnvelopeError) return fail("invalid_spool");
    if (signal?.aborted || error instanceof CredentialSnapshotError && error.category === "cancelled") return fail("cancelled");
    if (error instanceof PrivateDirectoryError && (error.category === "destination_exists" || error.category === "publication_refused")) return fail(error.category);
    if (error instanceof CredentialSnapshotError && error.category === "revocation_pending") return fail("revocation_pending");
    return fail("state_unavailable");
  } finally { key?.fill(0); for (const file of copies) file.bytes.fill(0); }
}
