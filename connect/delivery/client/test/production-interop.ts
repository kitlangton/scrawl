// Invoked by the ignored Rust production_typescript_crypto_interop test.
// All filesystem writes are confined to that test's private synthetic spool.
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFile, writeFile, readdir, unlink } from 'node:fs/promises';
import { join } from 'node:path';
import * as ed25519 from '@noble/ed25519';
import {
  signedBytes, signEnvelope, verifyEnvelope, decryptAge, encryptAge,
  decodeArchive, encodeArchive, splitAgeStream, joinAgeParts,
} from '../src/index.ts';
import type { Context, Asset, Signed } from '../src/index.ts';

const root = process.argv[2];
assert.ok(root?.startsWith('/'), 'requires absolute private fixture root');
const readJson = async (name: string) => JSON.parse(await readFile(join(root, name), 'utf8'));
const fixture = await readJson('interop.json') as {
  context: Context; senderKey: { signing: string; recipient: string };
  readerKey: { signing: string; recipient: string };
  signedBytesHex: string; expected: { path: string; hex: string }[];
  strictCases: { publicKey: string; signature: string; accepted: boolean }[];
};
const sender = await readJson('sender.identity');
const reader = await readJson('reader.identity');
assert.equal(sender.version, 1);
assert.equal(reader.version, 1);
const senderSeed = Buffer.from(sender.signing, 'hex');
const publicKey = Buffer.from(fixture.senderKey.signing, 'hex');
assert.deepEqual(Buffer.from(await ed25519.getPublicKeyAsync(senderSeed)), publicKey);
const envelope = await readJson('spool/envelope.json');
assert.deepEqual(envelope.signed.context, fixture.context);
assert.equal(Buffer.from(signedBytes(envelope.signed)).toString('hex'), fixture.signedBytesHex);
assert.equal(await verifyEnvelope(publicKey, envelope.signed, envelope.signature), true);
assert.equal(await signEnvelope(senderSeed, envelope.signed), envelope.signature);
for (const vector of fixture.strictCases) {
  assert.equal(await verifyEnvelope(Buffer.from(vector.publicKey, 'hex'),
    envelope.signed, vector.signature), vector.accepted);
}
const digest = (bytes: Uint8Array) => createHash('sha256').update(bytes).digest('hex');
const rustParts = await Promise.all(envelope.signed.chunks.map(async (chunk: Asset) => {
  assert.match(chunk.path, /^sealed\/part-\d{6}\.age$/);
  const bytes = await readFile(join(root, 'spool', chunk.path));
  assert.equal(bytes.length, chunk.size);
  assert.equal(digest(bytes), chunk.sha256);
  return bytes;
}));
const ciphertext = joinAgeParts(rustParts);
const archive = decodeArchive(await decryptAge(ciphertext, reader.age));
assert.deepEqual(archive.inner.context, fixture.context);
assert.equal(archive.inner.files.length, fixture.expected.length);
for (const [index, file] of archive.inner.files.entries()) {
  const expected = fixture.expected.find(item => item.path === file.path);
  assert.ok(expected);
  assert.equal(Buffer.from(archive.fileBytes[index]!).toString('hex'), expected.hex);
  assert.equal(digest(archive.fileBytes[index]!), file.sha256);
}
const outsider = await import('../src/index.ts').then(module => module.generateAgeIdentity());
await assert.rejects(decryptAge(ciphertext, outsider.identity));
const corrupt = ciphertext.slice();
corrupt[corrupt.length - 1]! ^= 1;
await assert.rejects(decryptAge(corrupt, reader.age));
assert.equal(await verifyEnvelope(publicKey,
  { ...envelope.signed, context: { ...fixture.context, accountId: 'tampered' } }, envelope.signature), false);

// Same supported archive and endpoint identities; a fresh TS age stream and signature.
const tsParts = splitAgeStream(await encryptAge(encodeArchive(archive.inner, archive.fileBytes),
  [fixture.senderKey.recipient, fixture.readerKey.recipient]));
assert.ok(tsParts.length > 1);
for (const name of await readdir(join(root, 'spool/sealed'))) {
  assert.match(name, /^part-\d{6}\.age$/);
  await unlink(join(root, 'spool/sealed', name));
}
const chunks: Asset[] = [];
for (const [index, bytes] of tsParts.entries()) {
  const path = `sealed/part-${String(index).padStart(6, '0')}.age`;
  await writeFile(join(root, 'spool', path), bytes, { mode: 0o600, flag: 'wx' });
  chunks.push({ path, size: bytes.length, sha256: digest(bytes) });
}
const signed: Signed = { version: 1, context: fixture.context, chunks };
await writeFile(join(root, 'spool/envelope.json'),
  JSON.stringify({ signed, signature: await signEnvelope(senderSeed, signed) }), { mode: 0o600 });
console.log('Production Rust → TypeScript verified; TypeScript multipart output ready for Rust');
