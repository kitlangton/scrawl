import assert from "node:assert/strict";
import { createServer, type Server } from "node:http";
import { once } from "node:events";
import { test } from "node:test";
import * as ed25519 from "@noble/ed25519";
import { generateAgeIdentity, encodeArchive, encryptAge, splitAgeStream, signEnvelope, type Context } from "../../src/recipient/crypto.ts";
import { retrieveRecipientCapture, RecipientRetrievalError, RecipientTransportError } from "../../src/index.ts";

const id = "0123456789abcdef0123456789abcdef", accountId = "acct-test", senderGrantId = "sender-upload";
const seed = Uint8Array.from({ length: 32 }, (_, i) => i + 3), encoder = new TextEncoder();
const digest = async (bytes: Uint8Array) => Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), b => b.toString(16).padStart(2, "0")).join("");
const fail = (category: string) => (e: unknown) => e instanceof RecipientRetrievalError && e.category === category && e.message === category;

type Payload = { status?: number; body: unknown; delayMs?: number };
async function server(run: (ctx: { origin: string; routes: Map<string, Payload>; counts: Map<string, number> }) => Promise<void>, mount = false) {
  const routes = new Map<string, Payload>(), counts = new Map<string, number>();
  const http: Server = createServer((req, res) => {
    const path = mount ? req.url!.replace(/^\/connect/, "") : req.url!; counts.set(path, (counts.get(path) ?? 0) + 1);
    assert.equal(req.method, "GET"); assert.equal(req.headers.cookie, undefined);
    assert.equal(req.headers.authorization, "Bearer synthetic-access-token");
    const item = routes.get(path);
    if (!item) { res.writeHead(404).end(); return; }
    const respond = () => {
    res.writeHead(item.status ?? 200, { "content-type": Buffer.isBuffer(item.body) ? "application/octet-stream" : "application/json" });
    const body = typeof item.body === "function" ? item.body() : item.body;
    res.end(Buffer.isBuffer(body) ? body : JSON.stringify(body));
    };
    if (item.delayMs) setTimeout(respond, item.delayMs); else respond();
  });
  http.listen(0, "127.0.0.1"); await once(http, "listening");
  const origin = `http://127.0.0.1:${(http.address() as { port: number }).port}${mount ? "/connect" : ""}`;
  try { await run({ origin, routes, counts }); } finally { http.close(); await once(http, "close"); }
}
const credential = (origin: string, identity?: { age: string; signing: Uint8Array }) => ({ server: origin, token: "synthetic-access-token", grantId: "reader", ...(identity ? { identity } : {}) });
const base = `/v1/captures/${id}`;
const plainAsset = { path: "summary.md", bytes: encoder.encode("private handoff\n") };
const bundleFiles = () => [plainAsset, { path: "apparent.json", bytes: encoder.encode("{}") }, { path: "contact-sheet.png", bytes: Uint8Array.of(1, 2, 3) }];

async function encrypted(origin: string, group?: "evidence" | "media", reader?: Awaited<ReturnType<typeof generateAgeIdentity>>) {
  const own = reader ?? await generateAgeIdentity();
  const recipient = own;
  const context: Context = { origin, accountId, captureId: id, senderGrantId, ...(group ? { group } : {}) };
  const contents = group === "media" ? [{ path: "recording.mp4", bytes: Uint8Array.of(4, 5, 6, 7) }] : bundleFiles();
  const files = await Promise.all(contents.map(async f => ({ path: f.path, size: f.bytes.length, sha256: await digest(f.bytes) })));
  const archive = encodeArchive({ version: 1, context, files }, contents.map(f => f.bytes));
  const cipher = await encryptAge(archive, [recipient.recipient]), parts = splitAgeStream(cipher);
  const chunks = await Promise.all(parts.map(async (bytes, i) => ({ path: `sealed/part-${String(i).padStart(6, "0")}.age`, size: bytes.length, sha256: await digest(bytes) })));
  const signed = { version: 1, context, chunks }, envelope = { signed, signature: await signEnvelope(seed, signed) };
  const prefix = group ? `${group}/` : "";
  const ownSeed = new Uint8Array(32).fill(19);
  return { recipient, own, ownSeed, contents, files: [
    { path: `${prefix}envelope.json`, bytes: encoder.encode(JSON.stringify(envelope)) },
    ...parts.map((bytes, i) => ({ path: `${prefix}${chunks[i]!.path}`, bytes })),
  ] };
}
function key(recipient: string, signing: Uint8Array) { return { version: 1, recipient, signing: Array.from(signing, b => b.toString(16).padStart(2, "0")).join("") }; }
async function peersPayload(f: Awaited<ReturnType<typeof encrypted>>) {
  return { accountId, grantId: "reader", scope: "read", devices: [
    { grantId: "reader", scope: "read", key: key(f.own.recipient, await ed25519.getPublicKeyAsync(f.ownSeed)) },
    { grantId: senderGrantId, scope: "upload", key: key(f.recipient.recipient, await ed25519.getPublicKeyAsync(seed)) },
  ] };
}

test("retrieves an encrypted capture from a root server using its exact signed origin", async () => server(async ({ origin, routes }) => {
  const f = await encrypted(origin); const ownSeed = f.ownSeed;
  // Fixture's recipient and read credential are the same key, with a distinct read signing seed.
  routes.set(base, { body: { id, status: "ready", format: "apparent-encrypted-v1", files: await Promise.all(f.files.map(async x => ({ path: x.path, size: x.bytes.length, sha256: await digest(x.bytes) }))) } });
  routes.set("/v1/encryption/peers", { body: await peersPayload(f) });
  for (const x of f.files) routes.set(`${base}/files/${x.path}`, { body: Buffer.from(x.bytes) });
  const signing = ownSeed.slice(); const saved = signing.slice();
  const result = await retrieveRecipientCapture({ credential: credential(origin, { age: f.own.identity, signing }), captureId: id });
  assert.deepEqual(signing, saved, "caller signing seed remains unchanged");
  assert.equal(result.server, origin); assert.equal(result.evidenceOnly, false);
  assert.deepEqual(result.files.map(x => x.path), ["summary.md", "apparent.json", "contact-sheet.png"]);
  for (let i = 0; i < result.files.length; i++) assert.deepEqual(result.files[i]!.bytes, f.contents[i]!.bytes);
}));

test("retrieves grouped evidence and media, while evidence-only skips media, on a mounted legacy server", async () => server(async ({ origin, routes, counts }) => {
  const readIdentity = await generateAgeIdentity();
  const evidence = await encrypted(origin, "evidence", readIdentity), media = await encrypted(origin, "media", readIdentity);
  routes.set(base, { body: { id, status: "ready", format: "apparent-encrypted-v1", delivery: "evidence-first-v1", evidenceReady: true,
    files: await Promise.all([...evidence.files, ...media.files].map(async x => ({ path: x.path, size: x.bytes.length, sha256: await digest(x.bytes) }))) } });
  routes.set("/v1/encryption/peers", { body: await peersPayload(evidence) });
  for (const x of [...evidence.files, ...media.files]) routes.set(`${base}/files/${x.path}`, { body: Buffer.from(x.bytes) });
  const input = { credential: credential(origin, { age: evidence.own.identity, signing: evidence.ownSeed }), captureId: id, evidenceOnly: true };
  const only = await retrieveRecipientCapture(input);
  assert.equal(only.evidenceOnly, true); assert.equal(only.files.length, 3);
  assert.equal(counts.get(`${base}/files/media/envelope.json`) ?? 0, 0);
  const full = await retrieveRecipientCapture({ ...input, evidenceOnly: false });
  assert.equal(full.evidenceOnly, false); assert.equal(full.files.length, 4);
}, true));

test("refuses plaintext by default before downloading files; explicit legacy opt-in retrieves exact bytes", async () => server(async ({ origin, routes, counts }) => {
  const legacy = bundleFiles();
  routes.set(base, { body: { id, status: "ready", files: await Promise.all(legacy.map(async f => ({ path: f.path, size: f.bytes.length, sha256: await digest(f.bytes) }))) } });
  for (const f of legacy) routes.set(`${base}/files/${f.path}`, { body: Buffer.from(f.bytes) });
  await assert.rejects(retrieveRecipientCapture({ credential: credential(origin), captureId: id }), fail("encryption_downgrade_refused"));
  assert.equal(counts.get(`${base}/files/${plainAsset.path}`) ?? 0, 0);
  const result = await retrieveRecipientCapture({ credential: credential(origin), captureId: id, allowLegacyPlaintext: true });
  assert.deepEqual(result.files[0]!.bytes, plainAsset.bytes);
}));

test("rejects invalid args and pre-aborted calls without network access", async () => server(async ({ origin, counts }) => {
  await assert.rejects(retrieveRecipientCapture({ credential: credential(origin), captureId: "bad" }), fail("invalid_request"));
  const c = new AbortController(); c.abort();
  await assert.rejects(retrieveRecipientCapture({ credential: credential(origin), captureId: id, signal: c.signal }), fail("cancelled"));
  assert.equal(counts.size, 0);
}));

test("does not retry authorization failures or expose service error bodies", async () => server(async ({ origin, routes, counts }) => {
  routes.set(base, { status: 401, body: { error: "secret token must not leak" } });
  await assert.rejects(retrieveRecipientCapture({ credential: credential(origin), captureId: id }), (e: unknown) => e instanceof RecipientTransportError && e.category === "unauthorized" && e.message === "unauthorized");
  assert.equal(counts.get(base), 1);
}));

async function installEncrypted(origin: string, routes: Map<string, Payload>) {
  const f = await encrypted(origin);
  const manifest = { id, status: "ready", format: "apparent-encrypted-v1", files: await Promise.all(f.files.map(async x => ({ path: x.path, size: x.bytes.length, sha256: await digest(x.bytes) }))) };
  const peers = await peersPayload(f);
  routes.set(base, { body: manifest }); routes.set("/v1/encryption/peers", { body: peers });
  for (const x of f.files) routes.set(`${base}/files/${x.path}`, { body: Buffer.from(x.bytes) });
  return { f, manifest, peers, input: { credential: credential(origin, { age: f.own.identity, signing: f.ownSeed }), captureId: id } };
}

test("requires current read identity and an approved upload sender before fetching chunks", async () => server(async ({ origin, routes, counts }) => {
  for (const mutation of ["own-key", "sender-missing", "sender-read", "duplicate-peer"] as const) {
    counts.clear(); const { peers, input } = await installEncrypted(origin, routes);
    if (mutation === "own-key") peers.devices[0]!.key.signing = peers.devices[1]!.key.signing;
    if (mutation === "sender-missing") peers.devices.pop();
    if (mutation === "sender-read") peers.devices[1]!.scope = "read";
    if (mutation === "duplicate-peer") peers.devices.push(peers.devices[0]!);
    await assert.rejects(retrieveRecipientCapture(input), fail(mutation.startsWith("sender") ? "encryption_sender_revoked_or_unapproved" : "invalid_encryption_authorization"));
    assert.equal(counts.get(`${base}/files/sealed/part-000000.age`) ?? 0, 0);
  }
}));

test("pins account, exact origin and envelope chunks before fetching encrypted parts", async () => server(async ({ origin, routes, counts }) => {
  for (const mutation of ["account", "origin", "chunk"] as const) {
    counts.clear(); const { f, manifest, input, peers } = await installEncrypted(origin, routes);
    if (mutation === "account") peers.accountId = "foreign-account";
    else {
      const envelope = JSON.parse(new TextDecoder().decode(f.files[0]!.bytes));
      if (mutation === "origin") envelope.signed.context.origin = origin + "/connect";
      else envelope.signed.chunks[0].sha256 = "0".repeat(64);
      envelope.signature = await signEnvelope(seed, envelope.signed);
      const bytes = encoder.encode(JSON.stringify(envelope));
      routes.set(`${base}/files/envelope.json`, { body: Buffer.from(bytes) });
      manifest.files[0] = { path: "envelope.json", size: bytes.length, sha256: await digest(bytes) };
    }
    await assert.rejects(retrieveRecipientCapture(input));
    assert.equal(counts.get(`${base}/files/sealed/part-000000.age`) ?? 0, 0);
  }
}));

test("rejects duplicate, extra and nonsequential wire assets before fetching files", async () => server(async ({ origin, routes, counts }) => {
  for (const mutation of ["duplicate", "extra", "gap"] as const) {
    counts.clear(); const { manifest, input } = await installEncrypted(origin, routes);
    if (mutation === "duplicate") manifest.files.push(manifest.files[0]!);
    if (mutation === "extra") manifest.files.push({ ...manifest.files[0]!, path: "private.json" });
    if (mutation === "gap") manifest.files[1]!.path = "sealed/part-000001.age";
    await assert.rejects(retrieveRecipientCapture(input), fail("invalid_manifest"));
    assert.equal(counts.get(`${base}/files/envelope.json`) ?? 0, 0);
  }
}));

test("rechecks sender authorization after decrypting and preserves a caller Buffer seed", async () => server(async ({ origin, routes }) => {
  const { input, peers } = await installEncrypted(origin, routes);
  let calls = 0;
  routes.set("/v1/encryption/peers", { body: () => ++calls === 1 ? peers : { ...peers, devices: peers.devices.slice(0, 1) } });
  const signing = Buffer.from(input.credential.identity!.signing), saved = Buffer.from(signing);
  input.credential.identity!.signing = signing;
  await assert.rejects(retrieveRecipientCapture(input), fail("encryption_sender_revoked_or_unapproved"));
  assert.equal(calls, 2); assert.deepEqual(signing, saved);
}));

test("bounds pending/404 waits and cancels polling without downloading files", async (t) => server(async ({ origin, routes, counts }) => {
  const input = { credential: credential(origin), captureId: id };
  routes.set(base, { body: { id, status: "pending" } });
  await assert.rejects(retrieveRecipientCapture(input), fail("not_ready"));
  counts.clear(); routes.delete(base);
  // Timers may wake early; polling must still honor its interval and deadline.
  const nativeTimeout = globalThis.setTimeout;
  const timer = t.mock.method(globalThis, "setTimeout", (callback: (...args: unknown[]) => void, milliseconds?: number, ...args: unknown[]) =>
    nativeTimeout(callback, milliseconds !== undefined && milliseconds > 1 && milliseconds <= 500 ? Math.max(1, milliseconds - 20) : milliseconds, ...args));
  const started = performance.now();
  await assert.rejects(retrieveRecipientCapture({ ...input, waitSeconds: 1 }), fail("not_ready"));
  timer.mock.restore();
  assert.ok(performance.now() - started < 1500); assert.ok(counts.get(base)! <= 3);
  const c = new AbortController(); setTimeout(() => c.abort(), 20);
  await assert.rejects(retrieveRecipientCapture({ ...input, waitSeconds: 900, signal: c.signal }), fail("cancelled"));
}));

test("a readiness response that stalls through the polling deadline becomes not_ready", async () => server(async ({ origin, routes }) => {
  routes.set(base, { status: 404, body: {}, delayMs: 1100 });
  await assert.rejects(retrieveRecipientCapture({ credential: credential(origin), captureId: id, waitSeconds: 1 }), fail("not_ready"));
}));
