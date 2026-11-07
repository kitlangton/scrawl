import { identityToRecipient } from "age-encryption";
import * as ed25519 from "@noble/ed25519";
import { decodeStrictJson } from "../recipient/json.ts";
import { generateAgeIdentity } from "../recipient/crypto.ts";
import { validatePeerAuthorityKey, type PeerAuthorityKey } from "../recipient/peer-authority.ts";

export type StoredIdentityRecord = Readonly<{ version: 1; age: string; signing: string }>;
type StoredIdentityFailure = "encryption_identity_unavailable" | "cancelled";

export class StoredIdentityError extends Error {
  readonly category: StoredIdentityFailure;
  constructor(category: StoredIdentityFailure) {
    super(category);
    this.name = "StoredIdentityError";
    this.category = category;
  }
}

function fail(category: StoredIdentityFailure): never {
  throw new StoredIdentityError(category);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function validAgeShape(age: string): boolean {
  return /^AGE-SECRET-KEY-1[023456789AC-HJ-NP-Z]{58}$/.test(age);
}

/** Create one canonical private record in memory; persistence belongs to the privileged caller. */
export async function generateStoredIdentityRecord(
  options: { signal?: AbortSignal } = {},
): Promise<{ privateRecord: Uint8Array; publicKey: PeerAuthorityKey }> {
  const signal = options.signal;
  let seed: Uint8Array | undefined;
  let serialized: Uint8Array | undefined;
  let validationSeed: Uint8Array | undefined;
  try {
    if (signal?.aborted) fail("cancelled");
    const generated = await generateAgeIdentity();
    if (signal?.aborted) fail("cancelled");
    seed = ed25519.utils.randomSecretKey();
    const signingPublic = await ed25519.getPublicKeyAsync(seed);
    if (signal?.aborted) fail("cancelled");
    const signing = Array.from(seed, byte => byte.toString(16).padStart(2, "0")).join("");
    const json = JSON.stringify({ version: 1, age: generated.identity, signing });
    serialized = new TextEncoder().encode(json);
    const decoded = decodeStoredIdentityRecord(serialized);
    validationSeed = (await validateStoredIdentityRecord(decoded, { signal })).signing;
    const publicKey = validatePeerAuthorityKey({ version: 1, recipient: generated.recipient,
      signing: Array.from(signingPublic, byte => byte.toString(16).padStart(2, "0")).join("") });
    // Check the age public value against the private record as well as its canonical shape.
    const derivedRecipient = await identityToRecipient(decoded.age);
    if (signal?.aborted) fail("cancelled");
    if (derivedRecipient !== publicKey.recipient || !validAgeShape(decoded.age)
        || !validationSeed.every((byte, index) => byte === seed![index])) fail("encryption_identity_unavailable");
    const privateRecord = serialized;
    serialized = undefined;
    return Object.freeze({ privateRecord, publicKey });
  } catch (error) {
    if (error instanceof StoredIdentityError) throw error;
    if (signal?.aborted) return fail("cancelled");
    return fail("encryption_identity_unavailable");
  } finally {
    seed?.fill(0);
    validationSeed?.fill(0);
    serialized?.fill(0);
  }
}

export function decodeStoredIdentityRecord(bytes: Uint8Array): StoredIdentityRecord {
  let value: unknown;
  try { value = decodeStrictJson(bytes, 4096); } catch { return fail("encryption_identity_unavailable"); }
  if (!isRecord(value) || Object.keys(value).length !== 3
      || !Object.hasOwn(value, "version") || !Object.hasOwn(value, "age") || !Object.hasOwn(value, "signing")
      || value.version !== 1 || typeof value.age !== "string" || typeof value.signing !== "string"
      || !validAgeShape(value.age) || !/^[0-9a-f]{64}$/.test(value.signing)) {
    return fail("encryption_identity_unavailable");
  }
  return Object.freeze({ version: 1, age: value.age, signing: value.signing });
}

export async function validateStoredIdentityRecord(
  record: StoredIdentityRecord,
  options: { signal?: AbortSignal } = {},
): Promise<{ age: string; signing: Uint8Array }> {
  let seed: Uint8Array | undefined;
  const signal = options.signal;
  try {
    if (signal?.aborted) fail("cancelled");
    if (!record || typeof record !== "object") fail("encryption_identity_unavailable");
    const version = record.version;
    const age = record.age;
    const signingHex = record.signing;
    if (version !== 1 || typeof age !== "string" || !validAgeShape(age)
        || typeof signingHex !== "string" || !/^[0-9a-f]{64}$/.test(signingHex)) {
      fail("encryption_identity_unavailable");
    }
    seed = new Uint8Array(32);
    for (let i = 0; i < seed.length; i++) seed[i] = Number.parseInt(signingHex.slice(i * 2, i * 2 + 2), 16);
    await identityToRecipient(age);
    if (signal?.aborted) fail("cancelled");
    return { age, signing: seed };
  } catch (error) {
    seed?.fill(0);
    if (error instanceof StoredIdentityError) throw error;
    if (signal?.aborted) fail("cancelled");
    fail("encryption_identity_unavailable");
  }
}
