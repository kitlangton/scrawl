import { createHash } from "node:crypto";
import { lstat } from "node:fs/promises";
import path from "node:path";
import { decodeJob, encodeJob, type Job } from "../queue/job.ts";
import { normalizeRecipientServer } from "../recipient/transport.ts";
import { verifyEnvelope } from "../recipient/crypto.ts";
import { validateRecipientEnvelopeBytesWithVerifier, RecipientEnvelopeError } from "../recipient-envelope.ts";
import { decodePeerAuthority, PeerAuthorityError, type PeerAuthorityKey } from "../recipient/peer-authority.ts";
import { replacePrivateStateFile, type StateReplacementResources } from "../state/replacement.ts";
import { readCancellation, readCredential, pendingIntent, readOptionalStateFile, readStateEvidence, stateFileStamp } from "../state/credential-files.ts";
import { CredentialSnapshotError, checkSignal, samePathFd, withStateQueue, type NamespaceResources, type NamespaceLease, type StateDirectory } from "../state/queue-transaction.ts";
import type { CredentialSnapshot } from "../state/namespace.ts";
import { admitTrustedDirectory, DirectoryAdmissionFailure } from "../state/directory.ts";
import { PrivateFileError } from "../state/private-file.ts";
import { decodeStoredEncryptedSpool, StoredSpoolDecodeError, type StoredEncryptedSpool } from "./spool-record.ts";
import type { SenderPeerAuthority } from "./authority.ts";
import { readPrivateSpoolAsset, spoolFileStamp, SpoolAssetParentChanged, type StampedSpoolAsset } from "./spool-files.ts";

type Failure = "invalid_spool" | "encryption_restage_required" | "stale_snapshot" | "revocation_pending" | "cancelled" | "unsafe_file" | "unsafe_directory" | "state_unavailable";
export class EncryptedSpoolCommitError extends Error {
  readonly category: Failure;
  constructor(category: Failure) { super(category); this.category = category; this.name = "EncryptedSpoolCommitError"; }
}
const fail = (category: Failure): never => { throw new EncryptedSpoolCommitError(category); };
const hash = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
export type EncryptedSpoolCommitResources = NamespaceResources & StateReplacementResources;
export type EncryptedSpoolCommitResult = Readonly<{ state: "committed" | "unchanged"; job: Job; record: StoredEncryptedSpool }>;
export type EncryptedSpoolCommitOptions = { signal?: AbortSignal; lockTimeoutMs?: number; active(): boolean };
function identity(job: Job): string { const raw = encodeJob(job); try { return hash(raw); } finally { raw.fill(0); } }
function detached(job: Job): Job { const raw = encodeJob(job); try { return decodeJob(raw); } finally { raw.fill(0); } }
function freeze<T>(value: T): T {
  if (value && typeof value === "object") { Object.freeze(value); for (const child of Object.values(value)) freeze(child); }
  return value;
}
function manifest(files: Job["files"] | StoredEncryptedSpool["files"]): string {
  return JSON.stringify(files.map(file => [file.path, file.size, file.sha256]).sort((a,b) => String(a[0]).localeCompare(String(b[0]))));
}

/** INTERNAL PREPARATION ONLY. Requires coordinated sole-writer admission before
 * any production caller. Rust remains the live writer; this function has no
 * package export and never creates, repairs or removes spool evidence. */
export async function commitEncryptedSpool(directoryPath: string, input: Job, snapshot: CredentialSnapshot,
  authorityInput: SenderPeerAuthority, ownPublicKeyInput: PeerAuthorityKey, resources: EncryptedSpoolCommitResources,
  options: EncryptedSpoolCommitOptions): Promise<EncryptedSpoolCommitResult | { state: "uncertain" }> {
  const signal = options?.signal, lockTimeoutMs = options?.lockTimeoutMs, active = options?.active;
  let key: PeerAuthorityKey | undefined;
  let replacement: Uint8Array | undefined;
  const bytes: Uint8Array[] = [];
  let dispatched = false;
  const consent = () => {
    checkSignal(signal);
    try { if (typeof active !== "function" || !active()) fail("stale_snapshot"); } catch { fail("stale_snapshot"); }
    checkSignal(signal);
  };
  try {
    checkSignal(signal);
    // Detach all mutable caller facts before the first await.
    const job = detached(input), expectedJob = identity(job);
    const credential = { scope: snapshot.credential.scope, server: snapshot.credential.server, generation: snapshot.credential.generation,
      grantId: snapshot.credential.grantId, encryption_identity: snapshot.credential.encryption_identity, authRequired: snapshot.credential.authRequired };
    const copiedSnapshot = { credential, directoryIdentity: snapshot.directoryIdentity, cancellation: snapshot.cancellation };
    if (!Array.isArray(authorityInput.devices) || authorityInput.devices.length > 64) fail("invalid_spool");
    const authority = { server: authorityInput.server, accountId: authorityInput.accountId, grantId: authorityInput.grantId, scope: authorityInput.scope,
      devices: authorityInput.devices.map(peer => ({ grantId: peer.grantId, scope: peer.scope, key: { ...peer.key } })) };
    key = Object.freeze({ version: ownPublicKeyInput.version, recipient: ownPublicKeyInput.recipient, signing: ownPublicKeyInput.signing });
    const normalizedKey = key;
    const server = normalizeRecipientServer(job.server);
    if (server !== job.server || !job.encrypted || job.shared || job.publicCopy
      || job.status !== "pending" || job.files.length && !job.encryption || !/^[a-f0-9]{32}$/.test(job.generation)
      || credential.scope !== "upload" || credential.server !== server || credential.generation !== job.generation
      || credential.authRequired || !credential.grantId || !credential.encryption_identity
      || authority.scope !== "upload" || authority.server !== server || authority.grantId !== credential.grantId
      || !Array.isArray(authority.devices) || authority.devices.length > 64) fail("invalid_spool");
    const peer = decodePeerAuthority({ accountId: authority.accountId, grantId: authority.grantId, scope: "upload", devices: authority.devices },
      { scope: "upload", grantId: authority.grantId, ownPublicKey: normalizedKey });
    const grantId: string = credential.grantId!;
    const expectedContext = { origin: server, accountId: peer.accountId, captureId: job.id, senderGrantId: grantId };
    const profile = job.grouped ? "evidence-first-v1" : "root";
    if (job.encryption && (job.encryption.context.origin !== expectedContext.origin || job.encryption.context.accountId !== expectedContext.accountId
      || job.encryption.context.captureId !== expectedContext.captureId || job.encryption.context.senderGrantId !== expectedContext.senderGrantId)) fail("stale_snapshot");
    const expectedGrants = job.encryption?.recipientGrants;
    if (expectedGrants && expectedGrants.some(id => !peer.devices.some(p => p.grantId === id))) fail("encryption_restage_required");
    if (job.files.length && job.encryption) {
      // Exact committed metadata is idempotent only when the current peer registry
      // still includes every pinned recipient grant.
      const committed = decodeStoredEncryptedSpool(new TextEncoder().encode(JSON.stringify({ metadata: job.encryption, files: job.files })), { context: expectedContext, profile });
      if (committed.metadata.recipientGrants.some(id => !peer.devices.some(p => p.grantId === id))) fail("encryption_restage_required");
    } else if (job.files.length || job.encryption) fail("invalid_spool");
    const fence = { directory: copiedSnapshot.directoryIdentity, generation: credential.generation, grant: credential.grantId,
      identity: credential.encryption_identity, cancellation: copiedSnapshot.cancellation };
    if (!fence.directory) fail("invalid_spool");
    consent();
    const check = (lease: NamespaceLease) => { consent(); checkSignal(signal, lease.signal); lease.assertHeld(); };
    async function current(state: StateDirectory, lease: NamespaceLease) {
      check(lease);
      if (state.identity !== fence.directory || !await samePathFd(state)) fail("stale_snapshot");
      if (await pendingIntent(state.path)) fail("revocation_pending");
      const currentJob = await readOptionalStateFile(state, `${job.id}.job`, 2_000_000, resources, signal, decodeJob);
      if (!currentJob || identity(currentJob) !== expectedJob) fail("stale_snapshot");
      const now = await readCredential(state, { scope: "upload", approvedServer: server }, resources, signal);
      if (now.authRequired || now.generation !== fence.generation || now.grantId !== fence.grant || now.encryption_identity !== fence.identity
        || await readCancellation(state, resources, signal) !== fence.cancellation) fail("stale_snapshot");
      if (await pendingIntent(state.path)) fail("revocation_pending");
      check(lease);
    }
    const transaction = <T>(body: (state: StateDirectory, lease: NamespaceLease) => Promise<T>) => withStateQueue(directoryPath,
      { scope: "upload", approvedServer: server }, resources, { signal, lockTimeoutMs }, body);
    async function readReceipt(spool: StateDirectory) {
      const receipt = await readStateEvidence(spool, "spool-state.json", 2_000_000, resources, signal ?? new AbortController().signal, raw => ({
        record: decodeStoredEncryptedSpool(raw, { context: expectedContext, profile }), digest: hash(raw),
      }));
      if (!receipt) return fail("invalid_spool");
      const record = receipt.value.record;
      if (!record.metadata.recipientGrants.includes(grantId)) fail("invalid_spool");
      if (record.metadata.recipientGrants.some(id => !peer.devices.some(p => p.grantId === id))) fail("encryption_restage_required");
      if (expectedGrants && JSON.stringify([...record.metadata.recipientGrants].sort()) !== JSON.stringify([...expectedGrants].sort())) fail("stale_snapshot");
      if (job.files.length && manifest(job.files) !== manifest(record.files)) fail("invalid_spool");
      return { stamp: receipt.expectedStamp, digest: receipt.value.digest, record };
    }
    const staged = await transaction(async (state, lease) => {
      await current(state, lease);
      const spool = await admitTrustedDirectory(path.join(state.path, `${job.id}.sealed`), resources, "private-state", signal);
      try { return { root: state.path, stateIdentity: state.identity, spoolIdentity: spool.identity, receipt: await readReceipt(spool) }; }
      finally { await spool.handle.close().catch(() => {}); }
    });
    // All asset reads, hashing and signature work is outside the queue lease.
    const spoolPath = path.join(staged.root, `${job.id}.sealed`);
    const spoolOutside = await admitTrustedDirectory(spoolPath, resources, "private-state", signal);
    const fileFacts = new Map<string, Omit<StampedSpoolAsset, "bytes">>();
    const envelopes = new Map<string, Uint8Array>();
    try {
      if (spoolOutside.identity !== staged.spoolIdentity) fail("stale_snapshot");
      for (const file of staged.receipt.record.files) {
        consent();
        const value = await readPrivateSpoolAsset(spoolOutside, file.path, file.size, resources, signal);
        try {
          if (hash(value.bytes) !== file.sha256) fail("invalid_spool");
          fileFacts.set(file.path, { stamp: value.stamp, parentIdentity: value.parentIdentity });
          if (file.path.endsWith("envelope.json")) { bytes.push(value.bytes); envelopes.set(file.path, value.bytes); }
        } finally { if (!envelopes.has(file.path)) value.bytes.fill(0); }
      }
      for (const group of profile === "root" ? [undefined] : ["evidence", "media"] as const) {
        const prefix = group ? `${group}/` : "", envelope = envelopes.get(`${prefix}envelope.json`)!;
        const signingKey = peer.devices.find(p => p.grantId === grantId)!.key.signing;
        const verified = await validateRecipientEnvelopeBytesWithVerifier(envelope, { ...expectedContext, ...(group ? { group } : {}) },
          Buffer.from(signingKey, "hex"), verifyEnvelope);
        const chunks = staged.receipt.record.files.filter(f => f.path.startsWith(`${prefix}sealed/`)).sort((a,b) => a.path.localeCompare(b.path));
        if (verified.signed.chunks.length !== chunks.length || verified.signed.chunks.some((f,i) =>
          f.path !== chunks[i]!.path.slice(prefix.length) || f.size !== chunks[i]!.size || f.sha256 !== chunks[i]!.sha256)) fail("invalid_spool");
      }
    } finally { await spoolOutside.handle.close().catch(() => {}); }
    const resulting = detached(job);
    resulting.files = staged.receipt.record.files.map(f => ({ ...f }));
    resulting.encryption = { context: { ...staged.receipt.record.metadata.context }, recipientGrants: [...staged.receipt.record.metadata.recipientGrants] };
    replacement = encodeJob(resulting);
    const outcome: { state: "committed" | "unchanged" | "uncertain" } = { state: job.files.length ? "unchanged" : "committed" };
    await transaction(async (state, lease) => {
      await current(state, lease);
      const spool = await admitTrustedDirectory(path.join(state.path, `${job.id}.sealed`), resources, "private-state", signal);
      try {
        const receipt = await readReceipt(spool);
        if (state.identity !== staged.stateIdentity || spool.identity !== staged.spoolIdentity || receipt.stamp !== staged.receipt.stamp
          || receipt.digest !== staged.receipt.digest || !await samePathFd(spool)) fail("stale_snapshot");
        for (const file of receipt.record.files) {
          const first = fileFacts.get(file.path);
          if (!first) return fail("stale_snapshot");
          const full = path.join(spool.path, file.path), currentStamp = spoolFileStamp(await lstat(full, { bigint: true }));
          if (currentStamp !== first.stamp) fail("stale_snapshot");
          const parent = await admitTrustedDirectory(path.dirname(full), resources, "private-state", signal);
          try { if (parent.identity !== first.parentIdentity) fail("stale_snapshot"); }
          finally { await parent.handle.close().catch(() => {}); }
        }
        await current(state, lease);
        if (job.files.length) return;
        const pathJob = path.join(state.path, `${job.id}.job`);
        const fresh = await readStateEvidence(state, `${job.id}.job`, 2_000_000, resources, signal ?? new AbortController().signal, decodeJob);
        if (!fresh || identity(fresh.value) !== expectedJob) return fail("stale_snapshot");
        const file = await lstat(pathJob, { bigint: true });
        if (stateFileStamp(file) !== fresh.expectedStamp) fail("stale_snapshot");
        const result = await replacePrivateStateFile(state.path, `${job.id}.job`, replacement!,
          { device: String(file.dev), inode: String(file.ino) }, lease, resources, { signal });
        if (result.state === "committed") { dispatched = true; outcome.state = "committed"; return; }
        if (result.state === "publication_uncertain") { dispatched = true; outcome.state = "uncertain"; return; }
        if (result.state === "destination_changed" || result.state === "refused") fail("stale_snapshot");
      } finally { await spool.handle.close().catch(() => {}); }
    });
    if (outcome.state === "uncertain") return { state: "uncertain" };
    return Object.freeze({ state: outcome.state, job: freeze(detached(outcome.state === "unchanged" ? job : resulting)), record: staged.receipt.record });
  } catch (error) {
    if (dispatched) return { state: "uncertain" };
    if (error instanceof EncryptedSpoolCommitError) throw new EncryptedSpoolCommitError(error.category);
    if (error instanceof StoredSpoolDecodeError || error instanceof RecipientEnvelopeError || error instanceof PeerAuthorityError) return fail("invalid_spool");
    if (error instanceof SpoolAssetParentChanged) return fail("stale_snapshot");
    if (signal?.aborted) return fail("cancelled");
    if (error instanceof DirectoryAdmissionFailure) return fail(error.category);
    if (error instanceof PrivateFileError) return fail(error.category === "unsafe_file" ? "unsafe_file" : error.category === "cancelled" ? "cancelled" : "state_unavailable");
    if (error instanceof CredentialSnapshotError) {
      if (["cancelled", "revocation_pending", "unsafe_file", "unsafe_directory", "stale_snapshot"].includes(error.category)) return fail(error.category as Failure);
      return fail(error.category === "invalid_credential" ? "stale_snapshot" : "state_unavailable");
    }
    return fail("state_unavailable");
  } finally { replacement?.fill(0); for (const value of bytes) value.fill(0); }
}
