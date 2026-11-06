import assert from "node:assert/strict";
import { test } from "node:test";
import * as ed25519 from "@noble/ed25519";
import { encodeArchive, encryptAge, generateAgeIdentity, signEnvelope, splitAgeStream, type Asset, type Context } from "../../dist/recipient/crypto.js";
import { openRecipientArchive, RecipientArchiveError } from "../../dist/recipient/archive.js";

const context: Context = { origin: "https://connect.test", accountId: "acct", captureId: "0123456789abcdef0123456789abcdef", senderGrantId: "grant", group: "evidence" };
const seed = Uint8Array.from({ length:  32 }, (_, i) => i + 1);
const encoder = new TextEncoder();
const digest = async (bytes: Uint8Array) => Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), b => b.toString(16).padStart(2, "0")).join("");
const asset = async (path: string, bytes: Uint8Array): Promise<Asset> => ({ path, size: bytes.length, sha256: await digest(bytes) });
const validFiles = () => [
  { path: "apparent.json", bytes: encoder.encode("{}") },
  { path: "summary.md", bytes: encoder.encode("A capture") },
  { path: "contact-sheet.png", bytes: Uint8Array.of(1, 2, 3) },
];

async function fixture(options: { files?: Array<{ path: string; bytes: Uint8Array }>; rawPlaintext?: Uint8Array; envelopeRaw?: string; group?: Context["group"] } = {}) {
  const ctx: Context = { ...context, ...(options.group === undefined ? {} : { group: options.group }) };
  const { identity, recipient } = await generateAgeIdentity();
  const files = options.files ?? validFiles();
  const manifest: Asset[] = await Promise.all(files.map(file => asset(file.path, file.bytes)));
  const plaintext = options.rawPlaintext ?? encodeArchive({ version: 1, context: ctx, files: manifest }, files.map(file => file.bytes));
  const ciphertext = await encryptAge(plaintext, [recipient]);
  const parts = splitAgeStream(ciphertext);
  const chunks = await Promise.all(parts.map(async (part, i) => ({ path: `sealed/part-${String(i).padStart(6, "0")}.age`, size: part.length, sha256: await digest(part) })));
  const signed = { version: 1, context: ctx, chunks };
  const envelope = { signed, signature: await signEnvelope(seed, signed) };
  const raw = options.envelopeRaw ?? JSON.stringify(envelope);
  return { input: { envelope: encoder.encode(raw), parts, identity, expected: ctx, trustedSenderSigningKey: await ed25519.getPublicKeyAsync(seed) }, envelope, parts, identity, ctx, plaintext, files };
}
const category = (name: string) => (error: unknown) => error instanceof RecipientArchiveError && error.category === name && error.message === name;

test("opens a signed age archive and returns exact verified bytes and context", async () => {
  const f = await fixture();
  const opened = await openRecipientArchive(f.input);
  assert.deepEqual(opened.context, context);
  assert.deepEqual(opened.files.map(({ path, size, sha256 }) => ({ path, size, sha256 })), await Promise.all(f.files.map(file => asset(file.path, file.bytes))));
  for (let i = 0; i < f.files.length; i++) assert.deepEqual(opened.files[i]!.bytes, f.files[i]!.bytes);
});

test("opens a media group with media-only assets", async () => {
  const files = [{ path: "recording.mp4", bytes: Uint8Array.of(4, 5, 6) }];
  const f = await fixture({ files, group: "media" });
  const opened = await openRecipientArchive(f.input);
  assert.deepEqual(opened.context, { ...context, group: "media" });
  assert.deepEqual(opened.files[0]!.bytes, files[0]!.bytes);
});

test("streams a production-sized multipart age payload across the 32 MiB boundary", async () => {
  const files = [...validFiles(), { path: "frames/frame-1.png", bytes: new Uint8Array(32 * 1024 * 1024) }];
  files[3]!.bytes[0] = 0x89;
  const f = await fixture({ files });
  assert.equal(f.parts.length, 2);
  assert.equal(f.parts[0]!.length, 32 * 1024 * 1024);
  const opened = await openRecipientArchive(f.input);
  assert.equal(opened.files[3]!.bytes.length, files[3]!.bytes.length);
  assert.equal(opened.files[3]!.bytes[0], 0x89);
});

test("rejects wrong identity, tampered ciphertext, and a bad signature", async () => {
  const f = await fixture();
  const wrongIdentity = { ...f.input, identity: (await generateAgeIdentity()).identity };
  await assert.rejects(openRecipientArchive(wrongIdentity), category("encryption_decrypt_failed"));
  const changedPart = f.parts.map(part => part.slice()); changedPart[0]![changedPart[0]!.length - 1] ^= 1;
  await assert.rejects(openRecipientArchive({ ...f.input, parts: changedPart }), category("invalid_encryption_parts"));
  const badSig = structuredClone(f.envelope); badSig.signature = "00".repeat(64);
  await assert.rejects(openRecipientArchive({ ...f.input, envelope: encoder.encode(JSON.stringify(badSig)) }), { category: "invalid_encryption_signature" });
});

test("rejects absent, wrong-size, extra, and truncated ciphertext parts", async () => {
  const f = await fixture();
  for (const parts of [[], [f.parts[0]!.subarray(1)], [...f.parts, f.parts[0]!.slice()], [f.parts[0]!.subarray(0, -1)]]) {
    await assert.rejects(openRecipientArchive({ ...f.input, parts }), (error: unknown) => error instanceof Error);
  }
});

test("rejects duplicate raw envelope keys before signature verification", async () => {
  const f = await fixture();
  const raw = JSON.stringify(f.envelope).replace('"version":1', '"version":1,"version":1');
  await assert.rejects(openRecipientArchive({ ...f.input, envelope: encoder.encode(raw) }), { category: "invalid_encryption_envelope" });
});

test("rejects a validly signed envelope with an unexpected context", async () => {
  const f = await fixture();
  const changed = { ...f.envelope.signed, context: { ...f.ctx, accountId: "someone-else" } };
  const envelope = { signed: changed, signature: await signEnvelope(seed, changed) };
  await assert.rejects(openRecipientArchive({ ...f.input, envelope: encoder.encode(JSON.stringify(envelope)) }), { category: "encrypted_context_mismatch" });
});

async function resignedArchive(mutator: (descriptor: Record<string, any>, plaintext: Uint8Array) => Uint8Array | undefined) {
  const f = await fixture();
  const descriptorLength = new DataView(f.plaintext.buffer).getUint32(0, false);
  const descriptor = JSON.parse(new TextDecoder().decode(f.plaintext.subarray(4, 4 + descriptorLength))) as Record<string, any>;
  const mutated = mutator(descriptor, f.plaintext);
  const plaintext = mutated ?? f.plaintext;
  // Rebuild using a fresh retained identity so the envelope is cryptographically valid.
  const pair = await generateAgeIdentity();
  const ciphertext = await encryptAge(plaintext, [pair.recipient]);
  const parts = splitAgeStream(ciphertext);
  const chunks = await Promise.all(parts.map(async (part, i) => ({ path: `sealed/part-${String(i).padStart(6, "0")}.age`, size: part.length, sha256: await digest(part) })));
  const signed = { version: 1, context: f.ctx, chunks };
  const envelope = { signed, signature: await signEnvelope(seed, signed) };
  return { ...f.input, identity: pair.identity, parts, envelope: encoder.encode(JSON.stringify(envelope)) };
}

test("rejects authenticated malformed descriptors, unknown fields, missing required files, and bad file hashes", async () => {
  const cases = [
    ["unknown descriptor field", (d: any) => { d.extra = true; }, "invalid_encryption_archive"],
    ["unknown file field", (d: any) => { d.files[0].extra = true; }, "invalid_manifest"],
    ["oversized metadata before content allocation", (d: any) => { d.files[0].size = 2_000_001; }, "invalid_manifest"],
    ["missing required file", (d: any) => { d.files = d.files.filter((f: any) => f.path !== "summary.md"); }, "invalid_manifest"],
    ["unsafe path", (d: any) => { d.files[0].path = "../secret"; }, "invalid_manifest"],
    ["bad inner context", (d: any) => { d.context.captureId = "fedcba9876543210fedcba9876543210"; }, "encrypted_context_mismatch"],
    ["hash mismatch", (d: any) => { d.files[0].sha256 = "0".repeat(64); }, "encryption_plaintext_mismatch"],
  ] as const;
  for (const [label, mutate, expected] of cases) {
    const input = await resignedArchive((descriptor, original) => {
      mutate(descriptor);
      const json = encoder.encode(JSON.stringify(descriptor));
      const out = new Uint8Array(4 + json.length + original.length - 4 - new DataView(original.buffer).getUint32(0, false));
      new DataView(out.buffer).setUint32(0, json.length, false); out.set(json, 4);
      out.set(original.subarray(4 + new DataView(original.buffer).getUint32(0, false)), 4 + json.length);
      return out;
    });
    await assert.rejects(openRecipientArchive(input), (error: unknown) => error instanceof RecipientArchiveError && error.category === expected, label);
  }
});

test("rejects duplicate raw keys in an authenticated inner descriptor", async () => {
  const f = await fixture();
  const length = new DataView(f.plaintext.buffer).getUint32(0, false);
  const descriptor = new TextDecoder().decode(f.plaintext.subarray(4, 4 + length)).replace('"version":1', '"version":1,"version":1');
  const encoded = encoder.encode(descriptor);
  const plaintext = new Uint8Array(4 + encoded.length + f.plaintext.length - 4 - length);
  new DataView(plaintext.buffer).setUint32(0, encoded.length, false); plaintext.set(encoded, 4); plaintext.set(f.plaintext.subarray(4 + length), 4 + encoded.length);
  const input = await resignedArchive((_d, _p) => plaintext);
  await assert.rejects(openRecipientArchive(input), category("invalid_encryption_archive"));
});

test("rejects unsafe bundle references inside otherwise authenticated metadata", async () => {
  const files = validFiles();
  files[0] = { ...files[0]!, bytes: encoder.encode(JSON.stringify({ contactSheet: "../outside.png" })) };
  const f = await fixture({ files });
  await assert.rejects(openRecipientArchive(f.input), category("unsafe_bundle_reference"));
});

test("rejects trailing plaintext only after the age stream authenticates to final EOF", async () => {
  const f = await fixture();
  const plaintext = new Uint8Array(f.plaintext.length + 1); plaintext.set(f.plaintext); plaintext[plaintext.length - 1] = 0x7f;
  const input = await resignedArchive(() => plaintext);
  await assert.rejects(openRecipientArchive(input), (error: unknown) => error instanceof RecipientArchiveError && error.category === "invalid_encryption_archive");
});

test("rejects pre-aborted work with an allowlisted cancellation error", async () => {
  const f = await fixture(); const controller = new AbortController(); controller.abort();
  await assert.rejects(openRecipientArchive({ ...f.input, signal: controller.signal }), category("cancelled"));
});

async function replaceCiphertext(f: Awaited<ReturnType<typeof fixture>>, ciphertext: Uint8Array) {
  const parts = splitAgeStream(ciphertext);
  const chunks = await Promise.all(parts.map(async (part, i) => ({ path: `sealed/part-${String(i).padStart(6, "0")}.age`, size: part.length, sha256: await digest(part) })));
  const signed = { version: 1, context: f.ctx, chunks };
  return { ...f.input, parts, envelope: encoder.encode(JSON.stringify({ signed, signature: await signEnvelope(seed, signed) })) };
}

test("rejects truncated, corrupt-final and concatenated ciphertext even with valid hashes and signatures", async () => {
  const f = await fixture();
  const original = f.parts[0]!;
  const corrupt = original.slice(); corrupt[corrupt.length - 1] ^= 1;
  const concatenated = new Uint8Array(original.length * 2); concatenated.set(original); concatenated.set(original, original.length);
  for (const ciphertext of [original.subarray(0, -1), corrupt, concatenated]) {
    await assert.rejects(openRecipientArchive(await replaceCiphertext(f, ciphertext)), category("encryption_decrypt_failed"));
  }
});

test("refuses expensive or oversized age headers before recipient processing", async () => {
  const f = await fixture();
  for (const [header, expected] of [
    ["age-encryption.org/v1\n-> scrypt salt 30\nAA\n--- AA\n", "unsupported_encryption_header"],
    ["age-encryption.org/v1\n" + "a".repeat(1024) + "\n", "invalid_encryption_header"],
    ["age-encryption.org/v1\n" + "AA\n".repeat(11_000), "invalid_encryption_header"],
  ]) await assert.rejects(openRecipientArchive(await replaceCiphertext(f, encoder.encode(header))), category(expected!));
});

test("cancellation during asynchronous verification cannot return plaintext", async () => {
  const f = await fixture(), controller = new AbortController();
  const result = openRecipientArchive({ ...f.input, signal: controller.signal });
  controller.abort();
  await assert.rejects(result, category("cancelled"));
});
