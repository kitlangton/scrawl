import * as age from "age-encryption";
import * as ed25519 from "@noble/ed25519";

export const SIGNED_ENVELOPE_DOMAIN = new TextEncoder().encode(
  "apparent-encrypted-v1\0signed-envelope\0",
);

export type Asset = { path: string; size: number; sha256: string };
export type Context = {
  origin: string;
  accountId: string;
  captureId: string;
  senderGrantId: string;
};
export type Signed = { version: number; context: Context; chunks: Asset[] };
export type Inner = { version: number; context: Context; files: Asset[] };

/** Rust serde_json emits struct fields in declaration order. Keep this explicit. */
export function signedBytes(signed: Signed): Uint8Array {
  const context: Record<string, unknown> = {
    origin: signed.context.origin,
    accountId: signed.context.accountId,
    captureId: signed.context.captureId,
    senderGrantId: signed.context.senderGrantId,
  };
  const ordered = {
    version: signed.version,
    context,
    chunks: signed.chunks.map(({ path, size, sha256 }) => ({ path, size, sha256 })),
  };
  const json = new TextEncoder().encode(JSON.stringify(ordered));
  const bytes = new Uint8Array(SIGNED_ENVELOPE_DOMAIN.length + json.length);
  bytes.set(SIGNED_ENVELOPE_DOMAIN);
  bytes.set(json, SIGNED_ENVELOPE_DOMAIN.length);
  return bytes;
}

/** Rust's plaintext age payload: u32-BE descriptor length, ordered JSON, then file bytes. */
export function encodeArchive(inner: Inner, fileBytes: Uint8Array[]): Uint8Array {
  if (inner.files.length !== fileBytes.length) throw new Error("invalid archive files");
  const context: Record<string, unknown> = {
    origin: inner.context.origin,
    accountId: inner.context.accountId,
    captureId: inner.context.captureId,
    senderGrantId: inner.context.senderGrantId,
  };
  const descriptor = new TextEncoder().encode(JSON.stringify({
    version: inner.version,
    context,
    files: inner.files.map(({ path, size, sha256 }) => ({ path, size, sha256 })),
  }));
  if (!descriptor.length || descriptor.length > 128 * 1024) throw new Error("invalid archive descriptor");
  const length = fileBytes.reduce((sum, bytes) => sum + bytes.length, 0);
  if (fileBytes.some((bytes, index) => bytes.length !== inner.files[index]!.size)) throw new Error("archive size mismatch");
  const out = new Uint8Array(4 + descriptor.length + length);
  new DataView(out.buffer).setUint32(0, descriptor.length, false);
  out.set(descriptor, 4);
  let offset = 4 + descriptor.length;
  for (const bytes of fileBytes) { out.set(bytes, offset); offset += bytes.length; }
  return out;
}

export function decodeArchive(archive: Uint8Array): { inner: Inner; fileBytes: Uint8Array[] } {
  if (archive.length < 5) throw new Error("truncated archive");
  const descriptorLength = new DataView(archive.buffer, archive.byteOffset, archive.byteLength).getUint32(0, false);
  if (!descriptorLength || descriptorLength > 128 * 1024 || descriptorLength > archive.length - 4) throw new Error("invalid archive descriptor");
  const inner = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(archive.subarray(4, 4 + descriptorLength))) as Inner;
  if (inner.version !== 1 || !Array.isArray(inner.files)) throw new Error("invalid archive descriptor");
  let offset = 4 + descriptorLength;
  const fileBytes = inner.files.map((file) => {
    if (!Number.isSafeInteger(file.size) || file.size < 0 || file.size > archive.length - offset) throw new Error("truncated archive file");
    const bytes = archive.slice(offset, offset + file.size);
    offset += file.size;
    return bytes;
  });
  if (offset !== archive.length) throw new Error("archive trailing data");
  return { inner, fileBytes };
}

export async function signEnvelope(seed: Uint8Array, signed: Signed): Promise<string> {
  if (seed.length !== 32) throw new Error("Ed25519 seed must be 32 bytes");
  return toHex(await ed25519.signAsync(signedBytes(signed), seed));
}

/** Signature primitive only: callers must validate the v1 schema, trusted context,
 * sender grant/scope/key and complete chunk manifest before using plaintext. */
export async function verifyEnvelope(
  publicKey: Uint8Array,
  signed: Signed,
  signatureHex: string,
): Promise<boolean> {
  if (publicKey.length !== 32 || !/^[0-9a-f]{128}$/.test(signatureHex)) return false;
  const signature = fromHex(signatureHex);
  try {
    if (ed25519.Point.fromBytes(publicKey, false).isSmallOrder()) return false;
    if (ed25519.Point.fromBytes(signature.subarray(0, 32), false).isSmallOrder()) return false;
  } catch {
    return false;
  }
  return ed25519.verifyAsync(signature, signedBytes(signed), publicKey, { zip215: false });
}

export async function generateAgeIdentity(): Promise<{ identity: string; recipient: string }> {
  const identity = await age.generateIdentity();
  return { identity, recipient: await age.identityToRecipient(identity) };
}

export async function encryptAge(plaintext: Uint8Array, recipients: string[]): Promise<Uint8Array> {
  if (!recipients.length || recipients.length > 64) throw new Error("invalid recipients");
  const encrypter = new age.Encrypter();
  for (const recipient of new Set(recipients)) encrypter.addRecipient(recipient);
  return encrypter.encrypt(plaintext);
}

export async function decryptAge(ciphertext: Uint8Array, identity: string): Promise<Uint8Array> {
  const decrypter = new age.Decrypter();
  decrypter.addIdentity(identity);
  return decrypter.decrypt(ciphertext, "uint8array");
}

export function splitAgeStream(ciphertext: Uint8Array, partBytes = 32 * 1024 * 1024): Uint8Array[] {
  if (!Number.isSafeInteger(partBytes) || partBytes < 1) throw new Error("invalid part size");
  const parts: Uint8Array[] = [];
  for (let offset = 0; offset < ciphertext.length; offset += partBytes) {
    parts.push(ciphertext.slice(offset, offset + partBytes));
  }
  if (!parts.length) throw new Error("empty age stream");
  return parts;
}

export function joinAgeParts(parts: Uint8Array[]): Uint8Array {
  if (!parts.length) throw new Error("empty age stream");
  const out = new Uint8Array(parts.reduce((sum, part) => sum + part.length, 0));
  let offset = 0;
  for (const part of parts) { out.set(part, offset); offset += part.length; }
  return out;
}

function toHex(bytes: Uint8Array): string {
  return Array.from(bytes, (value) => value.toString(16).padStart(2, "0")).join("");
}
function fromHex(hex: string): Uint8Array {
  if (!/^(?:[0-9a-f]{2})+$/.test(hex)) throw new Error("invalid hex");
  return Uint8Array.from(hex.match(/../g)!, (byte) => Number.parseInt(byte, 16));
}
