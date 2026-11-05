import assert from "node:assert/strict";
import { test } from "node:test";
import * as ed25519 from "@noble/ed25519";
import {
  decodeArchive, decryptAge, encodeArchive, encryptAge, generateAgeIdentity, joinAgeParts, signEnvelope,
  signedBytes, splitAgeStream, verifyEnvelope,
} from "../src/index.ts";

test("signed envelope bytes preserve Rust's domain, field order and omitted group", async () => {
  const signed = {
    version: 1,
    context: { origin: "https://connect.test", accountId: "acct", captureId: "capture", senderGrantId: "grant" },
    chunks: [{ path: "sealed/part-000000.age", size: 9, sha256: "ab".repeat(32) }],
  };
  assert.equal(new TextDecoder().decode(signedBytes(signed)),
    'apparent-encrypted-v1\u0000signed-envelope\u0000{"version":1,"context":{"origin":"https://connect.test","accountId":"acct","captureId":"capture","senderGrantId":"grant"},"chunks":[{"path":"sealed/part-000000.age","size":9,"sha256":"' + "ab".repeat(32) + '"}]}');
  const seed = new Uint8Array(32);
  seed[0] = 1;
  const publicKey = await ed25519.getPublicKeyAsync(seed);
  const signature = await signEnvelope(seed, signed);
  assert.equal(await verifyEnvelope(publicKey, signed, signature), true);
  assert.equal(await verifyEnvelope(publicKey, { ...signed, version: 2 }, signature), false);
  assert.equal(await verifyEnvelope(new Uint8Array(32), signed, signature), false);
  assert.equal(await verifyEnvelope(publicKey, signed, "00".repeat(64)), false);
  assert.throws(() => splitAgeStream(new Uint8Array([1]), Number.POSITIVE_INFINITY));
  assert.throws(() => splitAgeStream(new Uint8Array([1]), 1.5));
});

test("archive framing uses Rust's big-endian descriptor length and exact file bytes", () => {
  const inner = {
    version: 1,
    context: { origin: "https://connect.test", accountId: "acct", captureId: "capture", senderGrantId: "grant" },
    files: [{ path: "notes.txt", size: 4, sha256: "00".repeat(32) }],
  };
  const bytes = Uint8Array.from([0, 1, 2, 255]);
  const archive = encodeArchive(inner, [bytes]);
  const descriptorLength = new DataView(archive.buffer).getUint32(0, false);
  assert.deepEqual(decodeArchive(archive), { inner, fileBytes: [bytes] });
  assert.equal(archive.length, 4 + descriptorLength + bytes.length);
  assert.throws(() => decodeArchive(Uint8Array.from([...archive, 1])));
});

test("age v1 exchange round-trips ordered multipart bytes and rejects tamper/wrong recipient", async () => {
  const recipient = await generateAgeIdentity();
  const outsider = await generateAgeIdentity();
  const plaintext = Uint8Array.from({ length: 80_000 }, (_, index) => index % 251);
  const ciphertext = await encryptAge(plaintext, [recipient.recipient]);
  const parts = splitAgeStream(ciphertext, 4096);
  assert.ok(parts.length > 1);
  assert.deepEqual(await decryptAge(joinAgeParts(parts), recipient.identity), plaintext);
  await assert.rejects(decryptAge(ciphertext, outsider.identity));
  const corrupted = ciphertext.slice();
  corrupted[corrupted.length - 1] ^= 1;
  await assert.rejects(decryptAge(corrupted, recipient.identity));
});
