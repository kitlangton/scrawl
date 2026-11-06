import { Decrypter } from "age-encryption";
import { verifyEnvelope, type Asset, type Context } from "./crypto.ts";
import { validateRecipientEnvelopeBytesWithVerifier, RecipientEnvelopeError } from "../recipient-envelope.ts";
import { decodeStrictJson } from "./json.ts";
import { recipientManifest, validateRecipientReferences } from "./manifest.ts";
import { fail, RecipientArchiveError } from "./failure.ts";
export { RecipientArchiveError } from "./failure.ts";

export type RecipientArchiveInput = {
  envelope: Uint8Array;
  parts: readonly Uint8Array[];
  identity: string;
  expected: Context;
  trustedSenderSigningKey: Uint8Array;
  signal?: AbortSignal;
};
export type OpenedRecipientArchive = { context: Context; files: Array<Asset & { bytes: Uint8Array }> };
function allowed(signal?: AbortSignal): void { if (signal?.aborted) fail("cancelled"); }
async function hash(bytes: Uint8Array<ArrayBuffer>): Promise<string> {
  return Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), b => b.toString(16).padStart(2, "0")).join("");
}

/** Enforce the existing X25519-only, bounded age header before invoking age. */
function header(bytes: Uint8Array): void {
  let start = 0, native = 0, grease = 0, stanzas = 0;
  for (let end = 0; end < Math.min(bytes.length, 32 * 1024); end++) {
    if (end - start + 1 > 1024) fail("invalid_encryption_header");
    if (bytes[end] !== 10) continue;
    const lineBytes = bytes.subarray(start, end);
    if (lineBytes.some(b => b < 32 || b > 126)) fail("invalid_encryption_header");
    const line = new TextDecoder().decode(lineBytes);
    if (start === 0) { if (line !== "age-encryption.org/v1") fail("invalid_encryption_header"); }
    else if (line.startsWith("--- ")) {
      if (native === 0 || stanzas > 65 || grease > 1) fail("invalid_encryption_header");
      return;
    } else if (line.startsWith("-> ")) {
      stanzas++;
      if (line.startsWith("-> X25519 ")) native++;
      else if (line.slice(3).split(" ")[0]?.endsWith("-grease")) grease++;
      else fail("unsupported_encryption_header");
      if (native > 64 || grease > 1 || stanzas > 65) fail("invalid_encryption_header");
    }
    start = end + 1;
  }
  fail("invalid_encryption_header");
}

class Bytes {
  private buffer: Uint8Array = new Uint8Array();
  private reader: ReadableStreamDefaultReader<Uint8Array>;
  private signal?: AbortSignal;
  constructor(reader: ReadableStreamDefaultReader<Uint8Array>, signal?: AbortSignal) {
    this.reader = reader; this.signal = signal;
  }
  async take(size: number): Promise<Uint8Array<ArrayBuffer>> {
    const result = new Uint8Array(size);
    let offset = 0;
    try {
      while (offset < size) {
        allowed(this.signal);
        if (this.buffer.length === 0) {
          const next = await this.reader.read();
          allowed(this.signal);
          if (next.done) fail("invalid_encryption_archive");
          this.buffer = next.value;
          if (this.buffer.length === 0) continue;
        }
        const length = Math.min(size - offset, this.buffer.length);
        result.set(this.buffer.subarray(0, length), offset);
        this.buffer = this.buffer.subarray(length); offset += length;
      }
      return result;
    } catch (error) { result.fill(0); throw error; }
  }
  async end(): Promise<void> {
    if (this.buffer.length !== 0) fail("invalid_encryption_archive");
    for (;;) {
      allowed(this.signal);
      const next = await this.reader.read();
      allowed(this.signal);
      if (next.done) return;
      if (next.value.length !== 0) fail("invalid_encryption_archive");
    }
  }
}

/** Integrity-only boundary. Caller must authorize the sender currently and must
 * stage/publish privately only after this promise succeeds and authorization is
 * rechecked. No config, credentials, filesystem or service access occurs here.
 * Ciphertext is consumed a part at a time; returned plaintext is bounded by v1's
 * 500 MB account limit and retained in memory until every check has passed.
 */
export async function openRecipientArchive(input: RecipientArchiveInput): Promise<OpenedRecipientArchive> {
  const { identity, signal } = input;
  const allocated: Uint8Array[] = [];
  let reader: ReadableStreamDefaultReader<Uint8Array> | undefined;
  try {
    allowed(signal);
    const envelope = await validateRecipientEnvelopeBytesWithVerifier(input.envelope, input.expected, input.trustedSenderSigningKey, verifyEnvelope);
    allowed(signal);
    const parts = [...input.parts];
    if (parts.length !== envelope.signed.chunks.length || parts.some((p, i) => !(p instanceof Uint8Array) || p.byteLength !== envelope.signed.chunks[i]!.size)) fail("invalid_encryption_parts");
    if (typeof identity !== "string" || identity.length !== 74 || !/^AGE-SECRET-KEY-1[0-9A-Z]+$/.test(identity)) fail("encryption_decrypt_failed");
    let index = 0;
    const encrypted = new ReadableStream<Uint8Array>({
      async pull(controller) {
        allowed(signal);
        if (index === parts.length) { controller.close(); return; }
        const bytes = new Uint8Array(parts[index]!);
        if (await hash(bytes) !== envelope.signed.chunks[index]!.sha256) fail("invalid_encryption_parts");
        allowed(signal);
        if (index === 0) header(bytes);
        index++; controller.enqueue(bytes);
      },
    }, { highWaterMark: 0 });
    const decrypter = new Decrypter(); decrypter.addIdentity(identity);
    const plaintext = await decrypter.decrypt(encrypted);
    reader = plaintext.getReader();
    const bytes = new Bytes(reader, signal);
    const prefix = await bytes.take(4);
    const size = new DataView(prefix.buffer).getUint32(0, false);
    if (size === 0 || size > 128 * 1024) fail("invalid_encryption_archive");
    const descriptor = await bytes.take(size);
    let inner: unknown;
    try { inner = decodeStrictJson(descriptor); } catch { fail("invalid_encryption_archive"); }
    const assets = recipientManifest(inner, envelope.signed.context);
    const files: OpenedRecipientArchive["files"] = [];
    for (const asset of assets) {
      const data = await bytes.take(asset.size); allocated.push(data);
      if (await hash(data) !== asset.sha256) fail("encryption_plaintext_mismatch");
      files.push({ ...asset, bytes: data });
    }
    await bytes.end(); // Read through the authenticated final age chunk and EOF.
    validateRecipientReferences(files, envelope.signed.context);
    allowed(signal);
    return { context: envelope.signed.context, files };
  } catch (error) {
    for (const bytes of allocated) bytes.fill(0);
    if (signal?.aborted) fail("cancelled");
    if (error instanceof RecipientArchiveError || error instanceof RecipientEnvelopeError) throw error;
    return fail("encryption_decrypt_failed");
  } finally {
    if (reader) { try { await reader.cancel(); } catch { /* No raw dependency diagnostics. */ } }
  }
}
