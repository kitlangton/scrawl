import type { Asset, Context, Signed } from "./index.js";

const JSON_BYTES = 128 * 1024;
const PART_BYTES = 32 * 1024 * 1024;
const MAX_BYTES = 500_000_000;
const MAX_FILES = 512;

export type RecipientEnvelope = { signed: Signed; signature: string };
export type RecipientEnvelopeFailure =
  | "invalid_encryption_envelope"
  | "unsupported_encryption_format"
  | "invalid_encryption_context"
  | "invalid_encryption_parts"
  | "invalid_manifest"
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
  const authority = origin.replace(/^https?:\/\//, "");
  const host = authority.split(":", 1)[0]!;
  if (!host || !/^[A-Za-z0-9.-]+$/.test(host) || authority.includes("/")) return false;
  if (authority.includes(":")) {
    const port = authority.slice(authority.indexOf(":") + 1);
    if (!/^\d+$/.test(port) || Number(port) < 1 || Number(port) > 65535) return false;
  }
  if (origin.startsWith("http://") && !["localhost", "127.0.0.1"].includes(host)) return false;
  if (typeof accountId !== "string" || !accountId.length || accountId.length > 256 || /[\x00-\x1f\x7f]/.test(accountId)) return false;
  if (typeof senderGrantId !== "string" || !senderGrantId.length || senderGrantId.length > 256 || /[\x00-\x1f\x7f]/.test(senderGrantId)) return false;
  if (typeof captureId !== "string" || !/^[0-9a-f]{32}$/.test(captureId)) return false;
  return group === undefined || group === "evidence" || group === "media";
}
function safeAssetPath(path: string): boolean {
  if (path.length > 128 || /[^\x00-\x7f]/.test(path) || path.includes("\\") || path.split("/").some((p) => !p || p.startsWith("."))) return false;
  if (path.startsWith("sealed/")) return /^sealed\/part-[0-9]{6}\.age$/.test(path);
  if (["summary.md", "apparent.json", "notes.json", "contact-sheet.png", "contact-sheet-full.png", "recording.mp4", "recording.mov", "audio.wav", "timeline.json", "finish-trace.json"].includes(path)) return true;
  const parts = path.split("/");
  return parts.length === 2 && ["frames", "frames-full"].includes(parts[0]!) && /^frame-[0-9]+\.png$/.test(parts[1]!);
}
function validAsset(asset: unknown): asset is Asset {
  return record(asset) && exactKeys(asset, ["path", "size", "sha256"]) && typeof asset.path === "string" && safeAssetPath(asset.path)
    && Number.isSafeInteger(asset.size) && (asset.size as number) >= 0 && (asset.size as number) <= 90_000_000
    && typeof asset.sha256 === "string" && /^[0-9a-f]{64}$/.test(asset.sha256);
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
  const context = signed.context;
  if (context.origin !== expected.origin || context.accountId !== expected.accountId || context.captureId !== expected.captureId || context.senderGrantId !== expected.senderGrantId || context.group !== expected.group) fail("encrypted_context_mismatch");
  try {
    if (!(await verify(trustedSenderSigningKey, signed as unknown as Signed, input.signature))) fail("invalid_encryption_signature");
  } catch { fail("invalid_encryption_signature"); }
  return input as unknown as RecipientEnvelope;
}
