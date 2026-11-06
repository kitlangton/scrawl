import assert from "node:assert/strict";
import { test } from "node:test";
import * as ed25519 from "@noble/ed25519";
import { RecipientEnvelopeError, signEnvelope, validateRecipientEnvelope, type Signed } from "../src/index.ts";

const context = { origin: "https://connect.test", accountId: "acct", captureId: "0123456789abcdef0123456789abcdef", senderGrantId: "grant" };
const chunk = { path: "sealed/part-000000.age", size: 9, sha256: "ab".repeat(32) };
const seed = Uint8Array.from({ length: 32 }, (_, i) => i + 1);

async function fixture() {
  const signed: Signed = structuredClone({ version: 1, context, chunks: [chunk] });
  const signature = await signEnvelope(seed, signed);
  return { signed, signature };
}

test("public recipient boundary accepts v1 Rust-ordered compatible envelope", async () => {
  const envelope = await fixture();
  const key = await ed25519.getPublicKeyAsync(seed);
  assert.deepEqual(await validateRecipientEnvelope(envelope, context, key), envelope);
});

test("recipient boundary accepts the canonical mounted transport base and pins it exactly", async () => {
  const ctx = { ...context, origin: "https://app.tryapparent.com/connect" };
  const signed: Signed = { version: 1, context: ctx, chunks: [chunk] };
  const envelope = { signed, signature: await signEnvelope(seed, signed) };
  const key = await ed25519.getPublicKeyAsync(seed);
  assert.deepEqual(await validateRecipientEnvelope(envelope, ctx, key), envelope);
  await assert.rejects(validateRecipientEnvelope(envelope, { ...ctx, origin: "https://app.tryapparent.com" }, key), { category: "encrypted_context_mismatch" });
});

test("recipient boundary rejects context mismatch, tampering, unknown fields and unsafe chunk sequences", async () => {
  const envelope = await fixture();
  const key = await ed25519.getPublicKeyAsync(seed);
  await assert.rejects(validateRecipientEnvelope(envelope, { ...context, accountId: "other" }, key), { category: "encrypted_context_mismatch" });
  await assert.rejects(validateRecipientEnvelope({ ...envelope, signed: { ...envelope.signed, chunks: [{ ...chunk, size: 10 }] } }, context, key), { category: "invalid_encryption_signature" });
  await assert.rejects(validateRecipientEnvelope({ ...envelope, extra: true }, context, key), { category: "invalid_encryption_envelope" });
  await assert.rejects(validateRecipientEnvelope({ ...envelope, signed: { ...envelope.signed, chunks: [{ ...chunk, path: "sealed/part-000001.age" }] } }, context, key), { category: "invalid_encryption_parts" });
  await assert.rejects(validateRecipientEnvelope({ ...envelope, signed: { ...envelope.signed, version: 2 } }, context, key), { category: "unsupported_encryption_format" });
  await assert.rejects(validateRecipientEnvelope({ ...envelope, signed: { ...envelope.signed, context: { ...context, unknown: 1 } } }, context, key), { category: "invalid_encryption_context" });
  await assert.rejects(validateRecipientEnvelope({ ...envelope, signature: "secret-ish" }, context, key), (error: unknown) => error instanceof RecipientEnvelopeError && error.category === "invalid_encryption_envelope" && error.message === "invalid_encryption_envelope");
});

// Each mutation must fail at the structural boundary before cryptographic checks.
for (const [name, change, category] of [
  ["empty parts", (e: any) => { e.signed.chunks = []; }, "invalid_encryption_parts"],
  ["too many parts", (e: any) => { e.signed.chunks = Array(17).fill(chunk); }, "invalid_encryption_parts"],
  ["zero part", (e: any) => { e.signed.chunks[0].size = 0; }, "invalid_encryption_parts"],
  ["oversized part", (e: any) => { e.signed.chunks[0].size = 33554433; }, "invalid_encryption_parts"],
  ["fractional part", (e: any) => { e.signed.chunks[0].size = 1.5; }, "invalid_encryption_parts"],
  ["short nonfinal part", (e: any) => { e.signed.chunks.push({ ...chunk, path: "sealed/part-000001.age" }); }, "invalid_encryption_parts"],
  ["unknown part field", (e: any) => { e.signed.chunks[0].extra = true; }, "invalid_encryption_parts"],
  ["uppercase digest", (e: any) => { e.signed.chunks[0].sha256 = "AB".repeat(32); }, "invalid_encryption_parts"],
  ["total quota", (e: any) => { e.signed.chunks = Array.from({length: 16}, (_, i) => ({ ...chunk, path: `sealed/part-${String(i).padStart(6, "0")}.age`, size: 33554432 })); }, "quota_exceeded"],
  ["UTF8 account limit", (e: any) => { e.signed.context.accountId = "é".repeat(129); }, "invalid_encryption_context"],
  ["Unicode control", (e: any) => { e.signed.context.senderGrantId = "grant\u0085"; }, "invalid_encryption_context"],
  ["nonloopback HTTP", (e: any) => { e.signed.context.origin = "http://connect.test"; }, "invalid_encryption_context"],
  ["noncanonical origin", (e: any) => { e.signed.context.origin += "/"; }, "invalid_encryption_context"],
  ["URL credentials", (e: any) => { e.signed.context.origin = "https://user@connect.test"; }, "invalid_encryption_context"],
  ["invalid port", (e: any) => { e.signed.context.origin += ":65536"; }, "invalid_encryption_context"],
  ["arbitrary transport path", (e: any) => { e.signed.context.origin = "https://connect.test/api"; }, "invalid_encryption_context"],
  ["near canonical transport path", (e: any) => { e.signed.context.origin = "https://connect.test/connect/v1"; }, "invalid_encryption_context"],
  ["invalid capture", (e: any) => { e.signed.context.captureId = "../capture"; }, "invalid_encryption_context"],
  ["invalid group", (e: any) => { e.signed.context.group = "other"; }, "invalid_encryption_context"],
  ["JSON byte cap", (e: any) => { e.signature = "a".repeat(131072); }, "invalid_encryption_envelope"],
] as const) {
  test(`recipient boundary rejects ${name}`, async () => {
    const envelope = structuredClone(await fixture());
    change(envelope);
    await assert.rejects(validateRecipientEnvelope(envelope, context, await ed25519.getPublicKeyAsync(seed)), { category });
  });
}

test("recipient boundary pins every context field and trusted sender key", async () => {
  const envelope = await fixture();
  const key = await ed25519.getPublicKeyAsync(seed);
  for (const expected of [
    { ...context, origin: "https://other.test" },
    { ...context, captureId: "ab".repeat(16) },
    { ...context, senderGrantId: "other" },
    { ...context, group: "media" as const },
  ]) await assert.rejects(validateRecipientEnvelope(envelope, expected, key), { category: "encrypted_context_mismatch" });
  await assert.rejects(validateRecipientEnvelope(envelope, context, new Uint8Array(32)), { category: "invalid_encryption_signature" });
});

test("recipient boundary normalizes serde null group and accepts bounded Unicode IDs", async () => {
  const ctx = { ...context, accountId: "é".repeat(128) };
  const signed: Signed = { version: 1, context: ctx, chunks: [chunk] };
  const signature = await signEnvelope(seed, signed);
  const wire = { signed: { ...signed, context: { ...ctx, group: null } }, signature };
  const result = await validateRecipientEnvelope(wire, ctx, await ed25519.getPublicKeyAsync(seed));
  assert.deepEqual(result.signed.context, ctx);
});

test("recipient boundary returns the verified snapshot despite caller mutation during async verification", async () => {
  const envelope = await fixture();
  const original = structuredClone(envelope);
  const promise = validateRecipientEnvelope(envelope, context, await ed25519.getPublicKeyAsync(seed));
  envelope.signed.chunks[0]!.path = "unsafe";
  envelope.signed.context.accountId = "changed";
  envelope.signature = "00".repeat(64);
  assert.deepEqual(await promise, original);
});

for (const origin of ["http://localhost:1", "http://127.0.0.1:65535", "https://connect.test:+443"]) {
  test(`recipient boundary accepts Rust-compatible origin ${origin}`, async () => {
    const ctx = { ...context, origin };
    const signed: Signed = { version: 1, context: ctx, chunks: [chunk] };
    const envelope = { signed, signature: await signEnvelope(seed, signed) };
    assert.deepEqual(await validateRecipientEnvelope(envelope, ctx, await ed25519.getPublicKeyAsync(seed)), envelope);
  });
}

for (const group of ["evidence", "media"] as const) {
  test(`recipient boundary preserves signed ${group} grouping`, async () => {
    const ctx = { ...context, group };
    const signed: Signed = { version: 1, context: ctx, chunks: [chunk] };
    const envelope = { signed, signature: await signEnvelope(seed, signed) };
    assert.deepEqual(await validateRecipientEnvelope(envelope, ctx, await ed25519.getPublicKeyAsync(seed)), envelope);
    await assert.rejects(validateRecipientEnvelope(envelope, context, await ed25519.getPublicKeyAsync(seed)), { category: "encrypted_context_mismatch" });
  });
}

test("recipient boundary rejects lone surrogates that Rust strings cannot represent", async () => {
  const envelope = await fixture();
  envelope.signed.context.accountId = "\uD800";
  await assert.rejects(validateRecipientEnvelope(envelope, context, await ed25519.getPublicKeyAsync(seed)), { category: "invalid_encryption_context" });
});
