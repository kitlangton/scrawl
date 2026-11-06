import { createHash } from "node:crypto";
import * as ed25519 from "@noble/ed25519";
import { encodeArchive, encryptAge, signEnvelope, splitAgeStream, type Asset, type Context, type Signed } from "../recipient/crypto.ts";
import { recipientAssets, validateRecipientReferences } from "../recipient/manifest.ts";
import { RecipientArchiveError } from "../recipient/failure.ts";
import { decodePeerAuthority, PeerAuthorityError, type PeerAuthorityKey } from "../recipient/peer-authority.ts";
import type { SenderPeerAuthority } from "./authority.ts";
import type { EncryptedProfile, PreparedEncryptedGroup } from "./encrypted-transport.ts";
import { normalizeRecipientServer } from "../recipient/transport.ts";

export type SealInput = Readonly<{
  server: string; captureId: string; ownPublicKey: PeerAuthorityKey; signingSeed: Uint8Array;
  authority: SenderPeerAuthority; profile: EncryptedProfile;
  files: readonly Readonly<{ path: string; bytes: Uint8Array }>[];
  active(): boolean; signal?: AbortSignal;
}>;
export type SealedCapture = Readonly<
  | { profile: "root"; context: Readonly<Context>; recipientGrantIds: readonly string[]; envelope: Asset; chunks: readonly Asset[]; assets: readonly Readonly<{ path: string; bytes: Uint8Array }>[] }
  | { profile: "evidence-first-v1"; context: Readonly<Context>; recipientGrantIds: readonly string[]; groups: readonly Readonly<PreparedEncryptedGroup>[]; assets: readonly Readonly<{ path: string; bytes: Uint8Array }>[] }
>;
export type SealFailure = "invalid_request" | "invalid_server" | "invalid_encryption_recipients" | "encryption_pending_recipient" | "invalid_encryption_authorization" | "encryption_approval_required" | "encryption_key_binding_mismatch" | "cancelled" | "stale_snapshot" | "invalid_manifest" | "unsafe_bundle_reference" | "quota_exceeded" | "encryption_failed";
export class SealError extends Error { readonly category: SealFailure; constructor(category: SealFailure) { super(category); this.name = "SealError"; this.category = category; } }
const fail = (category: SealFailure): never => { throw new SealError(category); };
const idPattern = /^[a-f0-9]{32}$/;
const hash = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
const safePath = (path: string) => !path.includes("\\") && !path.startsWith("/") && !path.includes("\0") && path.split("/").every(p => p && p !== "." && p !== "..");
function check(active: () => boolean, signal?: AbortSignal): void {
  if (signal?.aborted) fail("cancelled");
  try { if (!active()) fail("stale_snapshot"); } catch { fail("stale_snapshot"); }
  if (signal?.aborted) fail("cancelled");
}
/** In-memory preparation of fresh bytes only. Returned transport buffers belong to the caller. */
export async function sealCapture(input: SealInput): Promise<SealedCapture> {
  let seed: Uint8Array | undefined;
  let signal: AbortSignal | undefined;
  const owned: Uint8Array[] = [], outputs: Uint8Array[] = [];
  let succeeded = false;
  try {
    if (!input || typeof input !== "object") fail("invalid_request");
    const server = input.server, captureId = input.captureId, profile = input.profile;
    const active = input.active; signal = input.signal;
    if (typeof active !== "function" || !(signal === undefined || signal instanceof AbortSignal) || typeof captureId !== "string" || !idPattern.test(captureId)
      || (profile !== "root" && profile !== "evidence-first-v1") || !Array.isArray(input.files)) fail("invalid_request");
    if (signal?.aborted) fail("cancelled");
    let origin = "";
    try { origin = normalizeRecipientServer(server); } catch { fail("invalid_server"); }
    const authorityInput = input.authority;
    if (!authorityInput || origin !== authorityInput.server || authorityInput.scope !== "upload" || typeof authorityInput.grantId !== "string" || authorityInput.grantId.length === 0
      || !Array.isArray(authorityInput.devices) || authorityInput.devices.length > 64) fail("invalid_encryption_authorization");
    const ownPublicKey = Object.freeze({ version: input.ownPublicKey.version, recipient: input.ownPublicKey.recipient, signing: input.ownPublicKey.signing });
    if (!(input.signingSeed instanceof Uint8Array) || input.signingSeed.byteLength !== 32) fail("invalid_request");
    seed = new Uint8Array(input.signingSeed);
    const authoritySnapshot = { accountId: authorityInput.accountId, grantId: authorityInput.grantId, devices: authorityInput.devices.map(p => ({ grantId: p.grantId, scope: p.scope, key: { version: p.key.version, recipient: p.key.recipient, signing: p.key.signing } })) };
    const authority = decodePeerAuthority({ ...authoritySnapshot, scope: "upload" },
      { scope: "upload", grantId: authoritySnapshot.grantId, ownPublicKey });
    const selected = authority.devices.filter(p => p.scope === "read" || p.grantId === authority.grantId);
    if (!selected.some(p => p.scope === "read")) fail("encryption_pending_recipient");
    if (selected.some(peer => peer.grantId.length > 256)) fail("invalid_encryption_recipients");
    const recipientGrantIds = Object.freeze(selected.map(p => p.grantId));
    const recipients = [...new Set(selected.map(p => p.key.recipient))];
    if (!recipients.length || recipients.length > 64) fail("invalid_encryption_authorization");
    // Bound and inspect primitive metadata before copying any plaintext.
    if (input.files.length < 3 || input.files.length > 512) fail("invalid_manifest");
    const names = new Set<string>(); let total = 0;
    const rows = input.files.map(f => {
      if (!f || typeof f.path !== "string" || !safePath(f.path) || f.path.length > 128 || !(f.bytes instanceof Uint8Array)
        || f.bytes.byteLength > 90_000_000 || names.has(f.path)) fail("invalid_manifest");
      names.add(f.path); total += f.bytes.byteLength; if (total > 500_000_000) fail("quota_exceeded");
      return { path: f.path, source: f.bytes, size: f.bytes.byteLength };
    });
    // Reuse manifest admission before copying bytes; the real hashes follow below.
    recipientAssets(rows.map(row => ({ path: row.path, size: row.size, sha256: "0".repeat(64) })));
    // Copy only after every shape and size has passed.
    for (const row of rows) { row.source = new Uint8Array(row.source); owned.push(row.source); }
    const files: Array<Asset & { bytes: Uint8Array }> = rows.map(row => ({ path: row.path, size: row.size, sha256: hash(row.source), bytes: row.source }));
    check(active, signal);
    if (Buffer.from(await ed25519.getPublicKeyAsync(seed)).toString("hex") !== ownPublicKey.signing) fail("encryption_key_binding_mismatch");
    check(active, signal);
    const base: Context = Object.freeze({ origin, accountId: authority.accountId, captureId, senderGrantId: authority.grantId });
    const groups: PreparedEncryptedGroup[] = [], transportAssets: Array<{ path: string; bytes: Uint8Array }> = []; let transportTotal = 0;
    const selectedGroups = profile === "root" ? [undefined] : ["evidence", "media"] as const;
    for (const group of selectedGroups) {
      check(active, signal);
      const ctx: Context = group ? Object.freeze({ ...base, group }) : base;
      const groupFiles = profile === "root" ? files : files.filter(f => ["audio.wav", "recording.mp4", "recording.mov"].includes(f.path) === (group === "media"));
      const assets = recipientAssets(groupFiles.map(({ path, size, sha256 }) => ({ path, size, sha256 })), group);
      validateRecipientReferences(groupFiles, ctx);
      const archive = encodeArchive({ version: 1, context: ctx, files: assets }, groupFiles.map(file => file.bytes)); owned.push(archive);
      check(active, signal);
      const ciphertext = await encryptAge(archive, recipients); owned.push(ciphertext);
      check(active, signal);
      const parts = splitAgeStream(ciphertext); outputs.push(...parts);
      if (parts.length > 16) fail("quota_exceeded");
      const prefix = group ? `${group}/` : "";
      const chunkAssets = parts.map((bytes, i) => { transportTotal += bytes.length; if (transportTotal > 500_000_000) fail("quota_exceeded"); transportAssets.push({ path: `${prefix}sealed/part-${String(i).padStart(6, "0")}.age`, bytes }); return { path: `sealed/part-${String(i).padStart(6, "0")}.age`, size: bytes.length, sha256: hash(bytes) }; });
      const signed: Signed = { version: 1, context: ctx, chunks: chunkAssets };
      const signature = await signEnvelope(seed, signed); check(active, signal);
      const envelopeBytes = new TextEncoder().encode(JSON.stringify({ signed, signature }));
      if (envelopeBytes.byteLength > 128 * 1024) { envelopeBytes.fill(0); fail("invalid_manifest"); }
      transportTotal += envelopeBytes.length; if (transportTotal > 500_000_000) { envelopeBytes.fill(0); fail("quota_exceeded"); }
      outputs.push(envelopeBytes);
      transportAssets.push({ path: `${prefix}envelope.json`, bytes: envelopeBytes });
      const envelope = { path: "envelope.json", size: envelopeBytes.length, sha256: hash(envelopeBytes) };
      groups.push({ context: ctx, envelope, chunks: chunkAssets });
      // Avoid retaining per-group ciphertext after parts have been copied.
      ciphertext.fill(0);
    }
    check(active, signal);
    const detachedGroups = groups.map(g => Object.freeze({ context: Object.freeze({ ...g.context }), envelope: Object.freeze({ ...g.envelope, ...(profile === "evidence-first-v1" ? { path: `${g.context.group}/${g.envelope.path}` } : {}) }), chunks: Object.freeze(g.chunks.map(c => Object.freeze({ ...c, ...(profile === "evidence-first-v1" ? { path: `${g.context.group}/${c.path}` } : {}) }))) }));
    const assets = Object.freeze(transportAssets.map(a => Object.freeze(a)));
    succeeded = true;
    if (profile === "root") return Object.freeze({ profile, context: base, recipientGrantIds, envelope: detachedGroups[0]!.envelope, chunks: detachedGroups[0]!.chunks, assets }) as SealedCapture;
    return Object.freeze({ profile, context: base, recipientGrantIds, groups: Object.freeze(detachedGroups), assets }) as SealedCapture;
  } catch (e) {
    if (e instanceof SealError) throw new SealError(e.category);
    if (e instanceof PeerAuthorityError) fail(e.category);
    if (e instanceof RecipientArchiveError && (e.category === "invalid_manifest" || e.category === "unsafe_bundle_reference" || e.category === "quota_exceeded")) fail(e.category);
    if (signal?.aborted) fail("cancelled");
    return fail("encryption_failed");
  } finally {
    seed?.fill(0); for (const bytes of owned) bytes.fill(0);
    if (!succeeded) for (const bytes of outputs) bytes.fill(0);
  }
}
