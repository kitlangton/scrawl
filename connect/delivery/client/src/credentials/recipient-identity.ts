import { identityToRecipient } from "age-encryption";
import { decodeStrictJson } from "../recipient/json.ts";

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
