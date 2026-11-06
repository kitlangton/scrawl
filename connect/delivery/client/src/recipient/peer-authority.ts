import { Encrypter } from "age-encryption";
import * as ed25519 from "@noble/ed25519";
import { exact, record } from "./manifest.ts";

export type PeerAuthorityScope = "read" | "upload";
export type PeerAuthorityKey = Readonly<{ version: 1; recipient: string; signing: string }>;
export type PeerAuthorityPeer = Readonly<{ grantId: string; scope: PeerAuthorityScope; key: PeerAuthorityKey }>;
export type PeerAuthority = Readonly<{ accountId: string; grantId: string; scope: PeerAuthorityScope; devices: readonly PeerAuthorityPeer[] }>;
export type PeerAuthorityFailure = "invalid_encryption_authorization" | "encryption_approval_required" | "encryption_key_binding_mismatch";
export class PeerAuthorityError extends Error {
  readonly category: PeerAuthorityFailure;
  constructor(category: PeerAuthorityFailure) { super(category); this.name = "PeerAuthorityError"; this.category = category; }
}
function fail(): never { throw new PeerAuthorityError("invalid_encryption_authorization"); }
function unhex(value: string): Uint8Array<ArrayBuffer> { return Uint8Array.from(value.match(/../g)!, byte => parseInt(byte, 16)); }
const graphic = (value: unknown, maximum = 256): value is string => typeof value === "string" && value.length > 0 && value.length <= maximum && /^[\x21-\x7e]+$/.test(value);

export function validatePeerAuthorityKey(value: unknown): PeerAuthorityKey {
  if (!record(value) || !exact(value, ["version", "recipient", "signing"]) || value.version !== 1
    || typeof value.recipient !== "string" || value.recipient.length !== 62 || !/^age1[0-9a-z]+$/.test(value.recipient)
    || typeof value.signing !== "string" || !/^[a-f0-9]{64}$/.test(value.signing)) fail();
  try {
    new Encrypter().addRecipient(value.recipient);
    const point = ed25519.Point.fromBytes(unhex(value.signing), false);
    if (point.isSmallOrder()) fail();
  } catch { fail(); }
  return Object.freeze({ version: 1, recipient: value.recipient, signing: value.signing });
}
export function samePeerAuthorityKey(a: PeerAuthorityKey, b: PeerAuthorityKey): boolean {
  return a.version === b.version && a.recipient === b.recipient && a.signing === b.signing;
}

/** Validate one already parsed, size-bounded /v1/encryption/peers response. */
export function decodePeerAuthority(value: unknown, expected: {
  scope: PeerAuthorityScope; grantId: string; ownPublicKey: PeerAuthorityKey;
}): PeerAuthority {
  if (expected.scope !== "read" && expected.scope !== "upload") fail();
  if (!graphic(expected.grantId, 512)) fail();
  const ownKey = validatePeerAuthorityKey(expected.ownPublicKey);
  if (!record(value) || !exact(value, ["accountId", "grantId", "scope", "devices"])
    || !graphic(value.accountId) || value.grantId !== expected.grantId || value.scope !== expected.scope
    || !Array.isArray(value.devices) || value.devices.length > 64) fail();
  const ids = new Set<string>(), devices: PeerAuthorityPeer[] = [];
  for (const raw of value.devices) {
    if (!record(raw) || !exact(raw, ["grantId", "scope", "key"]) || !graphic(raw.grantId, 512)
      || ids.has(raw.grantId) || (raw.scope !== "read" && raw.scope !== "upload")) fail();
    ids.add(raw.grantId);
    devices.push(Object.freeze({ grantId: raw.grantId, scope: raw.scope, key: validatePeerAuthorityKey(raw.key) }));
  }
  const own = devices.find(peer => peer.grantId === expected.grantId);
  if (!own) throw new PeerAuthorityError("encryption_approval_required");
  if (own.scope !== expected.scope || !samePeerAuthorityKey(own.key, ownKey))
    throw new PeerAuthorityError("encryption_key_binding_mismatch");
  return Object.freeze({ accountId: value.accountId, grantId: expected.grantId, scope: expected.scope, devices: Object.freeze(devices) });
}
