import type { Asset, Context, Signed } from "./index.js";
import { decodeStrictJson } from "./recipient/json.ts";

const JSON_BYTES = 128 * 1024;
const PART_BYTES = 32 * 1024 * 1024;
const MAX_BYTES = 500_000_000;

export type RecipientEnvelope = { signed: Signed; signature: string };
export type RecipientEnvelopeFailure =
  | "invalid_encryption_envelope"
  | "unsupported_encryption_format"
  | "invalid_encryption_context"
  | "invalid_encryption_parts"
  | "quota_exceeded"
  | "encrypted_context_mismatch"
  | "invalid_encryption_signature";

export class RecipientEnvelopeError extends Error {
  readonly category: RecipientEnvelopeFailure;
  constructor(category: RecipientEnvelopeFailure) {
    super(category);
    this.category = category;
    this.name = "RecipientEnvelopeError";
  }
}

function fail(category: RecipientEnvelopeFailure): never {
  throw new RecipientEnvelopeError(category);
}

/** Raw wire entrypoint: duplicate keys and byte bounds precede DTO validation. */
export async function validateRecipientEnvelopeBytesWithVerifier(
  bytes: Uint8Array, expected: Context, key: Uint8Array,
  verify: (key: Uint8Array, signed: Signed, signature: string) => Promise<boolean>,
): Promise<RecipientEnvelope> {
  let value: unknown;
  try { value = decodeStrictJson(bytes, JSON_BYTES); }
  catch { fail("invalid_encryption_envelope"); }
  return validateRecipientEnvelopeWithVerifier(value, expected, key, verify);
}
function record(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
function exactKeys(value: Record<string, unknown>, keys: string[]): boolean {
  const actual = Object.keys(value).sort();
  return actual.length === keys.length && actual.every((key, i) => key === [...keys].sort()[i]);
}
function contextValid(value: unknown): value is Context {
  if (!record(value) || !exactKeys(value, ["origin", "accountId", "captureId", "senderGrantId", ...(Object.hasOwn(value, "group") ? ["group"] : [])])) return false;
  const { origin, accountId, captureId, senderGrantId, group } = value;
  if (typeof origin !== "string" || origin.length > 2048 || /[^\x00-\x7f]/.test(origin)) return false;
  const trimmed = origin.replace(/\/+$/, "");
  if (trimmed !== origin || !/^https?:\/\//.test(origin) || /[\x00-\x20\x7f"'\\?#@]/.test(origin)) return false;
  const endpoint = origin.replace(/^https?:\/\//, "");
  const [authority, ...path] = endpoint.split("/");
  const host = authority.split(":", 1)[0]!;
  if (!host || !/^[A-Za-z0-9.-]+$/.test(host) || path.length > 1 || path.length === 1 && path[0] !== "connect") return false;
  if (authority.includes(":")) {
    const port = authority.slice(authority.indexOf(":") + 1);
    if (!/^\+?\d+$/.test(port) || Number(port) < 1 || Number(port) > 65535) return false;
  }
  if (origin.startsWith("http://") && !["localhost", "127.0.0.1"].includes(host)) return false;
  if (typeof accountId !== "string" || !accountId.length || new TextEncoder().encode(accountId).length > 256 || /[\p{Cc}\uD800-\uDFFF]/u.test(accountId)) return false;
  if (typeof senderGrantId !== "string" || !senderGrantId.length || new TextEncoder().encode(senderGrantId).length > 256 || /[\p{Cc}\uD800-\uDFFF]/u.test(senderGrantId)) return false;
  if (typeof captureId !== "string" || !/^[0-9a-f]{32}$/.test(captureId)) return false;
  return group == null || group === "evidence" || group === "media";
}
function validateChunks(chunks: unknown): chunks is Asset[] {
  if (!Array.isArray(chunks) || chunks.length < 1 || chunks.length > 16) return false;
  let total = 0;
  for (let i = 0; i < chunks.length; i++) {
    const part = chunks[i];
    if (!record(part) || !exactKeys(part, ["path", "size", "sha256"]) || part.path !== `sealed/part-${String(i).padStart(6, "0")}.age`
      || !Number.isSafeInteger(part.size) || (part.size as number) <= 0 || (part.size as number) > PART_BYTES
      || (i < chunks.length - 1 && part.size !== PART_BYTES) || typeof part.sha256 !== "string" || !/^[0-9a-f]{64}$/.test(part.sha256)) return false;
    total += part.size as number;
    if (!Number.isSafeInteger(total)) return false;
  }
  if (total > MAX_BYTES) fail("quota_exceeded");
  return true;
}

/** Validate a parsed, bounded envelope before any transfer or decryption.
 * The expected context and key must come from the caller's current trusted state.
 * This verifies signature integrity only; it does not authorize the sender with the service.
 */
export async function validateRecipientEnvelopeWithVerifier(
  input: unknown,
  expected: Context,
  trustedSenderSigningKey: Uint8Array,
  verify: (key: Uint8Array, signed: Signed, signature: string) => Promise<boolean>,
): Promise<RecipientEnvelope> {
  if (!record(input) || !exactKeys(input, ["signed", "signature"])) fail("invalid_encryption_envelope");
  let serialized: string;
  try { serialized = JSON.stringify(input); } catch { fail("invalid_encryption_envelope"); }
  if (new TextEncoder().encode(serialized!).length > JSON_BYTES) fail("invalid_encryption_envelope");
  const signed = input.signed;
  if (!record(signed) || !exactKeys(signed, ["version", "context", "chunks"])) fail("invalid_encryption_envelope");
  if (signed.version !== 1) fail("unsupported_encryption_format");
  if (!contextValid(signed.context)) fail("invalid_encryption_context");
  if (!Array.isArray(signed.chunks) || signed.chunks.length > 16) fail("invalid_encryption_parts");
  if (!validateChunks(signed.chunks)) fail("invalid_encryption_parts");
  if (typeof input.signature !== "string" || !/^[0-9a-f]{128}$/.test(input.signature)) fail("invalid_encryption_envelope");
  // Snapshot checked primitive fields before the asynchronous verifier. Never return
  // the caller-owned object, which can change while verification is pending.
  const context: Context = {
    origin: signed.context.origin, accountId: signed.context.accountId,
    captureId: signed.context.captureId, senderGrantId: signed.context.senderGrantId,
    ...(signed.context.group == null ? {} : { group: signed.context.group }),
  };
  const checked: RecipientEnvelope = {
    signed: { version: 1, context, chunks: signed.chunks.map(({ path, size, sha256 }) => ({ path, size, sha256 })) },
    signature: input.signature,
  };
  if (context.origin !== expected.origin || context.accountId !== expected.accountId || context.captureId !== expected.captureId || context.senderGrantId !== expected.senderGrantId || context.group !== expected.group) fail("encrypted_context_mismatch");
  try {
    if (!(await verify(trustedSenderSigningKey.slice(), checked.signed, checked.signature))) fail("invalid_encryption_signature");
  } catch { fail("invalid_encryption_signature"); }
  return checked;
}
