import assert from "node:assert/strict";
import { test } from "node:test";
import * as ed25519 from "@noble/ed25519";
import { RecipientEnvelopeError, signEnvelope, validateRecipientEnvelope, type Signed } from "../src/index.ts";

const context = { origin: "https://connect.test", accountId: "acct", captureId: "0123456789abcdef0123456789abcdef", senderGrantId: "grant" };
const chunk = { path: "sealed/part-000000.age", size: 9, sha256: "ab".repeat(32) };
const seed = Uint8Array.from({ length: 32 }, (_, i) => i + 1);

async function fixture() {
  const signed: Signed = { version: 1, context, chunks: [chunk] };
  const signature = await signEnvelope(seed, signed);
  return { signed, signature };
}

test("public recipient boundary accepts v1 Rust-ordered compatible envelope", async () => {
  const envelope = await fixture();
  const key = await ed25519.getPublicKeyAsync(seed);
  assert.deepEqual(await validateRecipientEnvelope(envelope, context, key), envelope);
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
