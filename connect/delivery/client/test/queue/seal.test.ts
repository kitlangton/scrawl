import assert from "node:assert/strict";
import { test } from "node:test";
import * as ed25519 from "@noble/ed25519";
import { generateAgeIdentity, verifyEnvelope } from "../../src/recipient/crypto.ts";
import { openRecipientArchive } from "../../src/recipient/archive.ts";
import { validateRecipientEnvelopeBytesWithVerifier } from "../../src/recipient-envelope.ts";
import type { SenderPeerAuthority } from "../../src/sender/authority.ts";
import { decodePeerAuthority } from "../../src/recipient/peer-authority.ts";
import { sealCapture, SealError } from "../../src/sender/seal.ts";

const seed = new Uint8Array(32).fill(41), hex = (b: Uint8Array) => Buffer.from(b).toString("hex");
async function fixture() {
  const self = await generateAgeIdentity(), reader = await generateAgeIdentity(), other = await generateAgeIdentity();
  const ownPublicKey = { version: 1 as const, recipient: self.recipient, signing: hex(await ed25519.getPublicKeyAsync(seed)) };
  const value = { server: "https://connect.example", accountId: "acct", grantId: "sender", scope: "upload" as const,
    devices: [
      { grantId: "reader", scope: "read" as const, key: { version: 1 as const, recipient: reader.recipient, signing: ownPublicKey.signing } },
      { grantId: "sender", scope: "upload" as const, key: ownPublicKey },
      { grantId: "other-upload", scope: "upload" as const, key: { version: 1 as const, recipient: other.recipient, signing: ownPublicKey.signing } },
      { grantId: "reader-shared", scope: "read" as const, key: { version: 1 as const, recipient: reader.recipient, signing: ownPublicKey.signing } },
    ] };
  const { server: _server, ...wire } = value;
  const authority = Object.freeze({ ...decodePeerAuthority(wire, { scope: "upload", grantId: value.grantId, ownPublicKey }), server: value.server, scope: "upload" as const }) as SenderPeerAuthority;
  const files = [
    { path: "summary.md", bytes: new TextEncoder().encode("summary") },
    { path: "contact-sheet.png", bytes: Uint8Array.of(1, 2, 3) },
    { path: "apparent.json", bytes: new TextEncoder().encode(JSON.stringify({ contactSheet: "contact-sheet.png", audio: "audio.wav", keyframes: [] })) },
    { path: "recording.mp4", bytes: Uint8Array.of(8, 9) },
    { path: "audio.wav", bytes: Uint8Array.of(6, 7) },
  ];
  return { ownPublicKey, authority, files, self, reader, other };
}
const input = (f: Awaited<ReturnType<typeof fixture>>, profile: "root" | "evidence-first-v1" = "root", active = () => true) => ({
  server: f.authority.server, captureId: "a".repeat(32), ownPublicKey: f.ownPublicKey, signingSeed: seed,
  authority: f.authority, profile, files: f.files, active,
});

test("seals root and grouped bytes for read peers and sender self with verified signed archives", async () => {
  const f = await fixture();
  for (const profile of ["root", "evidence-first-v1"] as const) {
    const sealed = await sealCapture(input(f, profile));
    assert.deepEqual(sealed.recipientGrantIds, ["reader", "sender", "reader-shared"]);
    const groups = profile === "root" ? [{ context: sealed.context, envelope: sealed.envelope, chunks: sealed.chunks, prefix: "" }] : sealed.groups.map(g => ({ ...g, prefix: `${g.context.group}/` }));
    for (const group of groups) {
      const envelopeBytes = sealed.assets.find(a => a.path === `${group.prefix}envelope.json`)!.bytes;
      const envelope = await validateRecipientEnvelopeBytesWithVerifier(envelopeBytes, group.context, new Uint8Array(Buffer.from(f.ownPublicKey.signing, "hex")), verifyEnvelope);
      const chunks = group.chunks.map(c => sealed.assets.find(a => a.path === (profile === "root" ? c.path : c.path))!.bytes);
      assert.equal(envelope.signed.chunks.length, chunks.length);
      for (const identity of [f.reader.identity, f.self.identity]) {
        const opened = await openRecipientArchive({ envelope: envelopeBytes, parts: chunks, identity,
          expected: group.context, trustedSenderSigningKey: new Uint8Array(Buffer.from(f.ownPublicKey.signing, "hex")) });
        assert.ok(opened.files.some(file => file.path === "summary.md") === (group.context.group !== "media"));
      }
      await assert.rejects(openRecipientArchive({ envelope: envelopeBytes, parts: chunks, identity: f.other.identity,
        expected: group.context, trustedSenderSigningKey: new Uint8Array(Buffer.from(f.ownPublicKey.signing, "hex")) }));
    }
    assert.ok(!sealed.assets.some(a => a.bytes === f.files[0]!.bytes));
    for (const asset of sealed.assets) asset.bytes.fill(0);
  }
});

test("grouped sealing preserves the donor's empty media archive", async () => {
  const f = await fixture();
  const sealed = await sealCapture({ ...input(f, "evidence-first-v1"), files: f.files.filter(file => !["audio.wav", "recording.mp4"].includes(file.path)) });
  assert.equal(sealed.profile, "evidence-first-v1");
  const media = sealed.groups.find(group => group.context.group === "media")!;
  const opened = await openRecipientArchive({ envelope: sealed.assets.find(asset => asset.path === media.envelope.path)!.bytes,
    parts: media.chunks.map(chunk => sealed.assets.find(asset => asset.path === chunk.path)!.bytes), identity: f.self.identity,
    expected: media.context, trustedSenderSigningKey: new Uint8Array(Buffer.from(f.ownPublicKey.signing, "hex")) });
  assert.deepEqual(opened.files, []);
  sealed.assets.forEach(asset => asset.bytes.fill(0));
});

test("rejects over-limit file and peer inventories before copying plaintext", async () => {
  const f = await fixture();
  for (const bad of [
    { ...input(f), files: Array.from({ length: 513 }, () => f.files[0]!) },
    { ...input(f), files: f.files.map(file => file.path === "apparent.json" ? { ...file, bytes: new Uint8Array(2_000_001) } : file) },
    { ...input(f), authority: { ...f.authority, devices: Array.from({ length: 65 }, () => f.authority.devices[0]!) } },
  ]) await assert.rejects(sealCapture(bad), error => error instanceof SealError && ["invalid_manifest", "invalid_encryption_authorization"].includes(error.category));
  await assert.rejects(sealCapture({ ...input(f), authority: { ...f.authority,
    devices: f.authority.devices.map(peer => peer.grantId === "reader" ? { ...peer, grantId: "g".repeat(257) } : peer) } }),
    error => error instanceof SealError && error.category === "invalid_encryption_recipients");
});

test("requires at least one read peer and rejects mismatched keys, contexts, paths, and cancellation", async () => {
  const f = await fixture();
  const noReader = { ...f.authority, devices: f.authority.devices.filter(p => p.scope === "upload") };
  await assert.rejects(sealCapture({ ...input(f), authority: noReader }), e => e instanceof SealError && e.category === "encryption_pending_recipient");
  await assert.rejects(sealCapture({ ...input(f), signingSeed: new Uint8Array(32) }), e => e instanceof SealError && e.category === "encryption_key_binding_mismatch");
  await assert.rejects(sealCapture({ ...input(f), server: "https://elsewhere.example" }), e => e instanceof SealError);
  await assert.rejects(sealCapture({ ...input(f), files: [...f.files, { path: "../bad", bytes: Uint8Array.of(1) }] }), e => e instanceof SealError);
  await assert.rejects(sealCapture(input(f, "root", () => false)), e => e instanceof SealError && e.category === "stale_snapshot");
  const controller = new AbortController(); controller.abort();
  await assert.rejects(sealCapture({ ...input(f), signal: controller.signal }), e => e instanceof SealError && e.category === "cancelled");
});

test("detaches the file list and bytes before the first async crypto operation", async () => {
  const f = await fixture(); let changed = false;
  const sealed = await sealCapture({ ...input(f), active: () => {
    if (!changed) { changed = true; f.files[0]!.bytes.fill(0); }
    return true;
  } });
  const envelope = sealed.assets.find(a => a.path === "envelope.json")!.bytes;
  const chunks = sealed.chunks.map(c => sealed.assets.find(a => a.path === c.path)!.bytes);
  const opened = await openRecipientArchive({ envelope, parts: chunks, identity: f.self.identity, expected: sealed.context,
    trustedSenderSigningKey: new Uint8Array(Buffer.from(f.ownPublicKey.signing, "hex")) });
  assert.equal(new TextDecoder().decode(opened.files.find(file => file.path === "summary.md")!.bytes), "summary");
  assert.equal(f.files[0]!.bytes.every(b => b === 0), true);
  for (const asset of sealed.assets) asset.bytes.fill(0);
});

test("preserves safe manifest failure categories and caller-owned bytes and keys", async () => {
  const f = await fixture(), original = f.files.map(file => new Uint8Array(file.bytes)), originalSeed = new Uint8Array(seed);
  await assert.rejects(sealCapture({ ...input(f), files: f.files.filter(file => file.path !== "summary.md") }),
    e => e instanceof SealError && e.category === "invalid_manifest" && e.message === "invalid_manifest");
  const badReferences = f.files.map(file => file.path === "apparent.json"
    ? { ...file, bytes: new TextEncoder().encode('{"contactSheet":"private-secret-path"}') } : file);
  await assert.rejects(sealCapture({ ...input(f), files: badReferences }),
    e => e instanceof SealError && e.category === "unsafe_bundle_reference" && e.message === "unsafe_bundle_reference");
  f.files.forEach((file, index) => assert.deepEqual(file.bytes, original[index]));
  assert.deepEqual(seed, originalSeed);
});
