import { Encrypter, identityToRecipient } from "age-encryption";
import * as ed25519 from "@noble/ed25519";
import { openRecipientArchive, type OpenedRecipientArchive } from "./archive.ts";
import { verifyEnvelope, type Asset, type Context } from "./crypto.ts";
import { validateRecipientEnvelopeBytesWithVerifier } from "../recipient-envelope.ts";
import { decodeStrictJson } from "./json.ts";
import { exact, record, recipientAssets, validateRecipientReferences } from "./manifest.ts";
import { RecipientTransport, RecipientTransportError, type RecipientAuthorization } from "./transport.ts";

export type RecipientReadCredential = {
  server: string; token: string; grantId: string;
  identity?: { age: string; signing: Uint8Array };
};
export type RetrieveRecipientInput = {
  credential: RecipientReadCredential; captureId: string; evidenceOnly?: boolean;
  waitSeconds?: number; allowLegacyPlaintext?: boolean; signal?: AbortSignal;
  /** Borrowed identity, loaded only after the service explicitly selects encryption. */
  resolveIdentity?: () => Promise<{ age: string; signing: Uint8Array }>;
  /** Private same-login authorization, refreshed for each bounded GET. */
  authorize?: RecipientAuthorization;
};
/** Data verified for a subsequent private publication, not a published bundle. */
export type RetrievedRecipientCapture = {
  captureId: string; server: string; evidenceOnly: boolean;
  files: OpenedRecipientArchive["files"];
};
export type RecipientRetrievalFailure = "invalid_request" | "invalid_manifest" | "not_ready"
  | "encryption_downgrade_refused" | "encryption_identity_unavailable"
  | "invalid_encryption_authorization" | "encryption_sender_revoked_or_unapproved"
  | "encrypted_context_mismatch" | "transport_integrity_mismatch" | "cancelled";
export class RecipientRetrievalError extends Error {
  readonly category: RecipientRetrievalFailure;
  constructor(category: RecipientRetrievalFailure) { super(category); this.name = "RecipientRetrievalError"; this.category = category; }
}
function fail(category: RecipientRetrievalFailure): never { throw new RecipientRetrievalError(category); }
function allowed(signal?: AbortSignal): void { if (signal?.aborted) fail("cancelled"); }
const hash = async (bytes: Uint8Array<ArrayBuffer>) => Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), b => b.toString(16).padStart(2, "0")).join("");
const graphic = (value: unknown, maximum = 256): value is string => typeof value === "string" && value.length > 0 && value.length <= maximum && /^[\x21-\x7e]+$/.test(value);
type Key = { version: 1; recipient: string; signing: string };
type Peer = { grantId: string; scope: "read" | "upload"; key: Key };
type Peers = { accountId: string; devices: Peer[] };
function key(value: unknown): Key {
  if (!record(value) || !exact(value, ["version", "recipient", "signing"]) || value.version !== 1
    || typeof value.recipient !== "string" || value.recipient.length !== 62 || !/^age1[0-9a-z]+$/.test(value.recipient)
    || typeof value.signing !== "string" || !/^[a-f0-9]{64}$/.test(value.signing)) fail("invalid_encryption_authorization");
  try {
    new Encrypter().addRecipient(value.recipient);
    const point = ed25519.Point.fromBytes(unhex(value.signing), false);
    if (point.isSmallOrder()) fail("invalid_encryption_authorization");
  } catch { fail("invalid_encryption_authorization"); }
  return { version: 1, recipient: value.recipient, signing: value.signing };
}
function unhex(value: string): Uint8Array<ArrayBuffer> { return Uint8Array.from(value.match(/../g)!, byte => parseInt(byte, 16)); }
const sameKey = (a: Key, b: Key) => a.version === b.version && a.recipient === b.recipient && a.signing === b.signing;
async function peers(transport: RecipientTransport, grantId: string, ownKey: Key): Promise<Peers> {
  const value = await transport.getJson("/v1/encryption/peers");
  if (!record(value) || !exact(value, ["accountId", "grantId", "scope", "devices"])
    || !graphic(value.accountId) || value.grantId !== grantId || value.scope !== "read"
    || !Array.isArray(value.devices) || value.devices.length > 64) fail("invalid_encryption_authorization");
  const ids = new Set<string>(), devices: Peer[] = [];
  for (const raw of value.devices) {
    if (!record(raw) || !exact(raw, ["grantId", "scope", "key"]) || !graphic(raw.grantId, 512)
      || ids.has(raw.grantId) || (raw.scope !== "read" && raw.scope !== "upload")) fail("invalid_encryption_authorization");
    ids.add(raw.grantId); devices.push({ grantId: raw.grantId, scope: raw.scope, key: key(raw.key) });
  }
  const own = devices.find(p => p.grantId === grantId);
  if (!own || own.scope !== "read" || !sameKey(own.key, ownKey)) fail("invalid_encryption_authorization");
  return { accountId: value.accountId, devices };
}

function wireAssets(input: unknown): Asset[] {
  if (!Array.isArray(input) || input.length < 1 || input.length > 512) fail("invalid_manifest");
  const names = new Set<string>(), result: Asset[] = [];
  let total = 0;
  for (const raw of input) {
    if (!record(raw) || !exact(raw, ["path", "size", "sha256"]) || typeof raw.path !== "string"
      || names.has(raw.path) || !Number.isSafeInteger(raw.size) || (raw.size as number) < 0 || (raw.size as number) > 90_000_000
      || typeof raw.sha256 !== "string" || !/^[a-f0-9]{64}$/.test(raw.sha256)) fail("invalid_manifest");
    names.add(raw.path); total += raw.size as number;
    if (total > 500_000_000) fail("invalid_manifest");
    result.push({ path: raw.path, size: raw.size as number, sha256: raw.sha256 });
  }
  return result;
}
function transportAssets(files: Asset[]): { envelope: Asset; parts: Asset[] } {
  const envelope = files.find(f => f.path === "envelope.json");
  const parts = files.filter(f => f.path !== "envelope.json").sort((a, b) => a.path.localeCompare(b.path));
  if (!envelope || envelope.size < 1 || envelope.size > 128 * 1024 || parts.length < 1 || parts.length > 16) fail("invalid_manifest");
  for (const [i, file] of parts.entries()) if (file.path !== `sealed/part-${String(i).padStart(6, "0")}.age`
    || file.size < 1 || file.size > 32 * 1024 * 1024 || (i < parts.length - 1 && file.size !== 32 * 1024 * 1024)) fail("invalid_manifest");
  return { envelope, parts };
}
async function download(transport: RecipientTransport, base: string, file: Asset): Promise<Uint8Array<ArrayBuffer>> {
  const bytes = new Uint8Array(await transport.getBytes(base + file.path, Math.max(1, file.size)));
  if (bytes.length !== file.size || await hash(bytes) !== file.sha256) fail("transport_integrity_mismatch");
  return bytes;
}
async function delay(milliseconds: number, signal?: AbortSignal): Promise<void> {
  allowed(signal);
  await new Promise<void>((resolve, reject) => {
    const abort = () => { clearTimeout(timer); signal?.removeEventListener("abort", abort); reject(new RecipientRetrievalError("cancelled")); };
    const timer = setTimeout(() => { signal?.removeEventListener("abort", abort); resolve(); }, milliseconds);
    signal?.addEventListener("abort", abort, { once: true });
  });
}

/** No config/credential mutation or filesystem publication. The caller retains
 * sole-writer/renewal authority and must publish privately without overwriting.
 * Evidence-only results require the existing .evidence-only.json receipt.
 */
export async function retrieveRecipientCapture(input: RetrieveRecipientInput): Promise<RetrievedRecipientCapture> {
  const { captureId, signal } = input, wait = input.waitSeconds ?? 0;
  if (!/^[a-f0-9]{32}$/.test(captureId) || !Number.isInteger(wait) || wait < 0 || wait > 900) fail("invalid_request");
  allowed(signal);
  const credential = { ...input.credential, ...(input.credential.identity ? { identity: {
    age: input.credential.identity.age, signing: new Uint8Array(input.credential.identity.signing),
  } } : {}) };
  const files: RetrievedRecipientCapture["files"] = [];
  try {
    const authorize = input.authorize;
    const transport = new RecipientTransport(credential.server, credential.token, { signal, authorize });
    const deadline = performance.now() + wait * 1000, base = `/v1/captures/${captureId}`;
    let manifest: Record<string, unknown>;
    for (;;) {
      allowed(signal);
      const remaining = deadline - performance.now();
      if (wait > 0 && remaining <= 0) fail("not_ready");
      const readinessDeadline = wait > 0 && remaining <= 30_000;
      try {
        const poll = new RecipientTransport(credential.server, credential.token, { signal, authorize, timeoutMs: wait === 0 ? 30_000 : Math.max(1, Math.min(30_000, Math.ceil(remaining))) });
        const value = await poll.getJson(base);
        if (!record(value) || value.id !== captureId || !["ready", "pending"].includes(value.status as string)) fail("invalid_manifest");
        if (value.status === "ready" || input.evidenceOnly === true && value.delivery === "evidence-first-v1" && value.evidenceReady === true) { manifest = value; break; }
      } catch (error) {
        if (error instanceof RecipientTransportError && error.category === "timeout" && readinessDeadline) fail("not_ready");
        if (!(error instanceof RecipientTransportError && error.category === "not_found" && wait > 0)) throw error;
      }
      if (wait === 0 || performance.now() >= deadline) fail("not_ready");
      await delay(Math.min(500, deadline - performance.now()), signal);
    }
    const format = manifest.format ?? "plaintext-v1", grouped = manifest.delivery === "evidence-first-v1";
    if (manifest.delivery !== undefined && (!grouped || format !== "apparent-encrypted-v1")) fail("invalid_manifest");
    const evidenceOnly = grouped && input.evidenceOnly === true;
    const names = new Set<string>();
    let total = 0;
    const append = (entries: RetrievedRecipientCapture["files"]) => {
      for (const entry of entries) {
        if (names.has(entry.path)) fail("invalid_manifest");
        names.add(entry.path); total += entry.size;
        if (total > 500_000_000) fail("invalid_manifest");
        files.push(entry);
      }
    };
    if (format === "plaintext-v1") {
      if (input.allowLegacyPlaintext !== true) fail("encryption_downgrade_refused");
      for (const asset of recipientAssets(manifest.files)) append([{ ...asset, bytes: await download(transport, base + "/files/", asset) }]);
      validateRecipientReferences(files, { origin: transport.server, accountId: "", captureId, senderGrantId: "" });
    } else if (format === "apparent-encrypted-v1") {
      if (!credential.identity && input.resolveIdentity) {
        const identity = await input.resolveIdentity();
        allowed(signal);
        credential.identity = { age: identity.age, signing: new Uint8Array(identity.signing) };
      }
      if (!credential.identity || !graphic(credential.grantId) || credential.identity.signing.length !== 32) fail("encryption_identity_unavailable");
      let ownKey: Key;
      try { ownKey = { version: 1, recipient: await identityToRecipient(credential.identity.age),
        signing: Array.from(await ed25519.getPublicKeyAsync(credential.identity.signing), b => b.toString(16).padStart(2, "0")).join("") }; }
      catch { return fail("encryption_identity_unavailable"); }
      const initial = await peers(transport, credential.grantId, ownKey);
      const wire = wireAssets(manifest.files);
      const groups = grouped ? ["evidence", "media"] as const : [undefined];
      const selected = groups.map(group => ({ group, ...transportAssets(group ? wire.filter(f => f.path.startsWith(group + "/")).map(f => ({ ...f, path: f.path.slice(group.length + 1) })) : wire) }));
      if (selected.reduce((count, group) => count + group.parts.length + 1, 0) !== wire.length) fail("invalid_manifest");
      let sender: Peer | undefined;
      for (const group of selected) {
        if (evidenceOnly && group.group === "media") continue;
        const prefix = base + "/files/" + (group.group ? group.group + "/" : "");
        const raw = await download(transport, prefix, group.envelope);
        let hint: unknown;
        try { hint = decodeStrictJson(raw); } catch { return fail("invalid_manifest"); }
        // This untrusted hint selects a key only from current authenticated peers.
        const id = record(hint) && record(hint.signed) && record(hint.signed.context) ? hint.signed.context.senderGrantId : undefined;
        const current = initial.devices.find(p => p.grantId === id && p.scope === "upload");
        if (!current) fail("encryption_sender_revoked_or_unapproved");
        if (sender && (current.grantId !== sender.grantId || !sameKey(current.key, sender.key))) fail("encrypted_context_mismatch");
        sender = current;
        const expected: Context = { origin: transport.server, accountId: initial.accountId, captureId, senderGrantId: current.grantId, ...(group.group ? { group: group.group } : {}) };
        const checked = await validateRecipientEnvelopeBytesWithVerifier(raw, expected, unhex(current.key.signing), verifyEnvelope);
        if (checked.signed.chunks.length !== group.parts.length || checked.signed.chunks.some((part, i) => {
          const asset = group.parts[i]!;
          return part.path !== asset.path || part.size !== asset.size || part.sha256 !== asset.sha256;
        })) fail("transport_integrity_mismatch");
        const parts: Uint8Array[] = [];
        for (const part of group.parts) parts.push(await download(transport, prefix, part));
        append((await openRecipientArchive({ envelope: raw, parts, expected, identity: credential.identity.age, trustedSenderSigningKey: unhex(current.key.signing), signal })).files);
      }
      if (!sender) fail("encryption_sender_revoked_or_unapproved");
      if (!evidenceOnly) { recipientAssets(files.map(({ path, size, sha256 }) => ({ path, size, sha256 }))); validateRecipientReferences(files, { origin: transport.server, accountId: initial.accountId, captureId, senderGrantId: sender.grantId }); }
      const latest = await peers(transport, credential.grantId, ownKey);
      if (latest.accountId !== initial.accountId || !latest.devices.some(p => p.scope === "upload" && p.grantId === sender!.grantId && sameKey(p.key, sender!.key))) fail("encryption_sender_revoked_or_unapproved");
    } else fail("invalid_manifest");
    allowed(signal);
    return { captureId, server: transport.server, evidenceOnly, files };
  } catch (error) {
    for (const file of files) file.bytes.fill(0);
    throw error;
  } finally { credential.identity?.signing.fill(0); }
}
