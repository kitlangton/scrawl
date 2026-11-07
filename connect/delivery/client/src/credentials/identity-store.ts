import { createHash } from "node:crypto";
import { lstat } from "node:fs/promises";
import path from "node:path";
import * as ed25519 from "@noble/ed25519";
import { identityToRecipient } from "age-encryption";
import { decodeStrictJson } from "../recipient/json.ts";
import { validatePeerAuthorityKey, type PeerAuthorityKey } from "../recipient/peer-authority.ts";
import { decodeStoredIdentityRecord, validateStoredIdentityRecord } from "./recipient-identity.ts";
import { pendingIntent, readCancellation, readStateEvidence, stateFileStamp } from "../state/credential-files.ts";
import { samePathFd, withStateQueue, type NamespaceResources } from "../state/queue-transaction.ts";
import { replacePrivateStateFile, type StateReplacementResources } from "../state/replacement.ts";

export type LoginIdentityContext = Readonly<{ scope: "read" | "upload"; approvedServer: string; loginId: string; cancellation: string | null }>;
export type LoginIdentityResult = Readonly<{ state: "published" | "existing"; identityId: string; publicKey: PeerAuthorityKey }>;
export type LoginIdentityUncertainty = Readonly<{ state: "uncertain"; identityId: string }>;
export type LoginIdentityResources = NamespaceResources & StateReplacementResources;
const MAX_RECORD = 4096;
const fail = (): never => { throw new Error("identity_unavailable"); };
const digest = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");

async function publicBinding(bytes: Uint8Array, signal?: AbortSignal): Promise<PeerAuthorityKey> {
  const record = decodeStoredIdentityRecord(bytes);
  const canonical = new TextEncoder().encode(JSON.stringify(record));
  const canonicalMatch = canonical.length === bytes.length && canonical.every((byte, index) => byte === bytes[index]);
  canonical.fill(0);
  if (!canonicalMatch) return fail();
  let validated: Awaited<ReturnType<typeof validateStoredIdentityRecord>> | undefined;
  try {
    validated = await validateStoredIdentityRecord(record, { signal });
    const signing = await ed25519.getPublicKeyAsync(validated.signing);
    if (signal?.aborted) fail();
    return validatePeerAuthorityKey({ version: 1, recipient: await identityToRecipient(validated.age),
      signing: Array.from(signing, b => b.toString(16).padStart(2, "0")).join("") });
  } finally { validated?.signing.fill(0); }
}

/** Inactive writer seam. The host must already own the admitted namespace's sole writer. */
export function publishLoginIdentity(directory: string, context: LoginIdentityContext, proposedPrivateRecord: Uint8Array,
  resources: LoginIdentityResources, options: { signal?: AbortSignal; lockTimeoutMs?: number } = {}): Promise<LoginIdentityResult | LoginIdentityUncertainty> {
  // Own the proposal before yielding; never mutate the caller's buffer.
  const proposal = proposedPrivateRecord instanceof Uint8Array ? new Uint8Array(proposedPrivateRecord) : new Uint8Array();
  const copiedContext = context && Object.freeze({ scope: context.scope, approvedServer: context.approvedServer, loginId: context.loginId, cancellation: context.cancellation });
  const copiedOptions = Object.freeze({ signal: options.signal, lockTimeoutMs: options.lockTimeoutMs });
  const identityId = typeof copiedContext?.loginId === "string" && /^[0-9a-f]{32}$/.test(copiedContext.loginId) ? copiedContext.loginId : "";
  let possiblyPublished = false;
  const run = async (): Promise<LoginIdentityResult | LoginIdentityUncertainty> => {
    const ctx = Object.freeze({ scope: copiedContext?.scope, approvedServer: copiedContext?.approvedServer }) as { scope: "read" | "upload"; approvedServer: string };
    if ((ctx.scope !== "read" && ctx.scope !== "upload") || typeof ctx.approvedServer !== "string"
        || typeof copiedContext?.loginId !== "string" || !/^[0-9a-f]{32}$/.test(copiedContext.loginId)
        || proposal.length === 0 || proposal.length > MAX_RECORD) return fail();
    await publicBinding(proposal, copiedOptions.signal);
    return withStateQueue(directory, ctx, resources, copiedOptions, async (state, lease) => {
      const name = `${identityId}.identity`;
      const fresh = async () => {
        await lease.assertCurrent();
        if (!await samePathFd(state) || await pendingIntent(state.path)
            || await readCancellation(state, resources, copiedOptions.signal) !== copiedContext!.cancellation) return false;
        const login = await readStateEvidence(state, `${ctx.scope}.login`, 128, resources, copiedOptions.signal ?? lease.signal,
          bytes => decodeStrictJson(bytes, 128));
        if (!login || typeof login.value !== "string" || login.value !== identityId) return false;
        if (await pendingIntent(state.path) || !await samePathFd(state)) return false;
        await lease.assertCurrent(); return true;
      };
      if (!await fresh()) return fail();
      const existing = await readStateEvidence(state, name, MAX_RECORD, resources, copiedOptions.signal ?? lease.signal,
        bytes => ({ bytes: new Uint8Array(bytes), key: undefined as PeerAuthorityKey | undefined }));
      if (existing) {
        let key: PeerAuthorityKey;
        try { key = await publicBinding(existing.value.bytes, copiedOptions.signal); }
        finally { existing.value.bytes.fill(0); }
        if (stateFileStamp(await lstat(existing.file, { bigint: true })) !== existing.expectedStamp || !await fresh()) return fail();
        return Object.freeze({ state: "existing", identityId, publicKey: key });
      }
      const expectedHash = digest(proposal);
      const guarded: LoginIdentityResources = { ...resources, replaceFile: async (parent, source, destination, expected, nativeOptions) => {
        if (destination !== name || expected.destination !== null || !await fresh()) {
          await parent.close().catch(() => {}); return { state: "refused" };
        }
        possiblyPublished = true;
        return resources.replaceFile(parent, source, destination, expected, nativeOptions);
      } };
      const outcome = await replacePrivateStateFile(state.path, name, proposal, null, lease, guarded, { signal: copiedOptions.signal });
      if (outcome.state === "publication_uncertain") return Object.freeze({ state: "uncertain", identityId });
      if (outcome.state !== "committed") { possiblyPublished = false; return fail(); }
      const saved = await readStateEvidence(state, name, MAX_RECORD, resources, copiedOptions.signal ?? lease.signal,
        bytes => ({ bytes: new Uint8Array(bytes), hash: digest(bytes) }));
      if (!saved) return Object.freeze({ state: "uncertain", identityId });
      let key: PeerAuthorityKey;
      try { key = await publicBinding(saved.value.bytes, copiedOptions.signal); }
      catch { saved.value.bytes.fill(0); return Object.freeze({ state: "uncertain", identityId }); }
      saved.value.bytes.fill(0);
      if (saved.value.hash !== expectedHash || stateFileStamp(await lstat(saved.file, { bigint: true })) !== saved.expectedStamp || !await fresh())
        return Object.freeze({ state: "uncertain", identityId });
      return Object.freeze({ state: "published", identityId, publicKey: key });
    });
  };
  return run().catch(error => { if (possiblyPublished) return Object.freeze({ state: "uncertain", identityId }); throw error; })
    .finally(() => proposal.fill(0));
}
