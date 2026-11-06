import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { createServer, type Server } from "node:http";
import { once } from "node:events";
import * as ed25519 from "@noble/ed25519";
import { encryptAge, generateAgeIdentity, signEnvelope, type Asset, type Context } from "../../src/recipient/crypto.ts";
import { uploadPreparedEncryptedCapture } from "../../src/sender/encrypted-transport.ts";
import { RecipientTransportError } from "../../src/recipient/transport.ts";

const id = "0123456789abcdef0123456789abcdef", seed = Uint8Array.from({ length: 32 }, (_, i) => i + 1), enc = new TextEncoder();
const sha = (b: Uint8Array) => createHash("sha256").update(b).digest("hex");
async function fixture(origin: string, group?: "evidence" | "media") {
  const context: Context = { origin, accountId: "acct", captureId: id, senderGrantId: "sender", ...(group ? { group } : {}) };
  const { recipient } = await generateAgeIdentity(), ciphertext = await encryptAge(enc.encode("synthetic encrypted payload"), [recipient]);
  const chunkPath = `${group ? `${group}/` : ""}sealed/part-000000.age`, chunk: Asset = { path: chunkPath, size: ciphertext.length, sha256: sha(ciphertext) };
  const signed = { version: 1, context, chunks: [{ ...chunk, path: "sealed/part-000000.age" }] }, envelopeBytes = enc.encode(JSON.stringify({ signed, signature: await signEnvelope(seed, signed) }));
  const envelope: Asset = { path: `${group ? `${group}/` : ""}envelope.json`, size: envelopeBytes.length, sha256: sha(envelopeBytes) };
  return { context, assets: new Map([[envelope.path, envelopeBytes], [chunk.path, ciphertext]]), files: [envelope, chunk], key: await ed25519.getPublicKeyAsync(seed) };
}
async function server(handler: Parameters<typeof createServer>[0]) { const s: Server = createServer(handler); s.listen(0, "127.0.0.1"); await once(s, "listening"); const a = s.address(); assert(a && typeof a === "object"); const origin = `http://127.0.0.1:${a.port}`; return { s, origin, close: () => new Promise<void>(r => { s.closeAllConnections(); s.close(() => r()); }) }; }
async function body(req: import("node:http").IncomingMessage) { const chunks: Buffer[] = []; for await (const x of req) chunks.push(Buffer.from(x)); return Buffer.concat(chunks); }
function capture(p: Promise<unknown>, category: string) { return assert.rejects(p, e => e instanceof RecipientTransportError && e.category === category); }
function options(f: Awaited<ReturnType<typeof fixture>>, server: string, overrides: Record<string, unknown> = {}) {
  const recipientGrantIds = ["reader"];
  const { group: _group, ...baseContext } = f.context;
  return { server, captureId: id, profile: f.context.group ? "evidence-first-v1" as const : "root" as const, context: { ...baseContext, origin: server }, recipientGrantIds, envelope: f.files[0]!, chunks: [f.files[1]!], trustedSenderSigningKey: f.key,
    ...(f.context.group ? { groups: [{ context: f.context, envelope: f.files[0]!, chunks: [f.files[1]!] }] } : {}),
    active: () => true, readAsset: async (path: string) => new Uint8Array(f.assets.get(path)!), authorize: async (approvedServer: string, scope: "upload") => ({ approvedServer, scope, token: "synthetic", accountId: "acct", senderGrantId: "sender", recipientGrantIds }), ...overrides };
}
test("uploads signed encrypted root snapshot and validates unordered readback", async () => {
  let f!: Awaited<ReturnType<typeof fixture>>, files: Asset[] = [];
  const s = await server(async (req, res) => { const b = await body(req); if (req.method === "POST" && !req.url?.endsWith("complete")) { const v = JSON.parse(b.toString()); assert.equal(v.format, "apparent-encrypted-v1"); files = v.files; res.end(JSON.stringify({ id, status: "pending", format: v.format })); }
    else if (req.method === "PUT") { const p = req.url!.split("/files/")[1]!; assert.deepEqual(b, Buffer.from(f.assets.get(p)!)); res.end("{}"); }
    else if (req.url?.endsWith("/complete")) res.end(JSON.stringify({ id, status: "ready", format: "apparent-encrypted-v1" }));
    else res.end(JSON.stringify({ id, status: "ready", format: "apparent-encrypted-v1", files: files.reverse() })); });
  try { f = await fixture(s.origin); await uploadPreparedEncryptedCapture(options(f, s.origin)); } finally { await s.close(); }
});
test("uploads grouped evidence with pinned profile and recipient facts on every request", async () => {
  let f!: Awaited<ReturnType<typeof fixture>>, media!: Awaited<ReturnType<typeof fixture>>, count = 0, files: Asset[] = [];
  const s = await server(async (req, res) => { const b = await body(req); count++; if (req.method === "POST" && !req.url?.endsWith("complete")) { const v = JSON.parse(b.toString()); assert.equal(v.delivery, "evidence-first-v1"); files = v.files; res.end(JSON.stringify({ id, status: "pending", format: v.format, delivery: v.delivery })); } else if (req.method === "PUT") res.end("{}"); else if (req.url?.endsWith("/complete")) res.end(JSON.stringify({ id, status: "ready", format: "apparent-encrypted-v1" })); else res.end(JSON.stringify({ id, status: "ready", format: "apparent-encrypted-v1", delivery: "evidence-first-v1", files })); });
  try { f = await fixture(s.origin, "evidence"); media = await fixture(s.origin, "media"); const opts = options(f, s.origin); opts.groups = [...opts.groups!, { context: media.context, envelope: media.files[0]!, chunks: [media.files[1]!] }]; opts.readAsset = async (path: string) => new Uint8Array(f.assets.get(path) ?? media.assets.get(path)!); await uploadPreparedEncryptedCapture(opts); assert.equal(count, 7); } finally { await s.close(); }
});
test("rejects signature, context, profile and chunk-manifest mismatches before reservation", async () => {
  let f!: Awaited<ReturnType<typeof fixture>>; let calls = 0; const s = await server((_req, res) => { calls++; res.end("{}"); });
  try {
    f = await fixture(s.origin);
    await capture(uploadPreparedEncryptedCapture(options(f, s.origin, { trustedSenderSigningKey: new Uint8Array(32) })), "invalid_request");
    await capture(uploadPreparedEncryptedCapture(options(f, s.origin, { context: { ...f.context, accountId: "other", origin: s.origin } })), "invalid_request");
    await capture(uploadPreparedEncryptedCapture(options(f, s.origin, { chunks: [{ ...f.files[1]!, sha256: "0".repeat(64) }] })), "invalid_request");
    assert.equal(calls, 0);
  } finally { await s.close(); }
});
test("checks fresh authorization recipient set before each request and stops on change", async () => {
  let f!: Awaited<ReturnType<typeof fixture>>; let calls = 0, auth = 0; const s = await server((_req, res) => { calls++; res.end(JSON.stringify({ id, status: "ready", format: "apparent-encrypted-v1", files: f.files })); });
  try { f = await fixture(s.origin); await capture(uploadPreparedEncryptedCapture(options(f, s.origin, { authorize: async (approvedServer: string, scope: "upload") => ({ approvedServer, scope, token: "synthetic", accountId: "acct", senderGrantId: "sender", recipientGrantIds: ++auth === 1 ? ["reader"] : ["revoked"] }) })), "auth_required"); assert.equal(calls, 1); }
  finally { await s.close(); }
});
test("refuses profile downgrade and pre-cancellation before encrypted reservation", async () => {
  let f!: Awaited<ReturnType<typeof fixture>>, calls = 0;
  const s = await server((_req, res) => { calls++; res.end(JSON.stringify({ id, status: "pending", format: "plaintext-v1" })); });
  try {
    f = await fixture(s.origin);
    await capture(uploadPreparedEncryptedCapture(options(f, s.origin)), "invalid_response");
    const controller = new AbortController(); controller.abort();
    await capture(uploadPreparedEncryptedCapture(options(f, s.origin, { signal: controller.signal })), "cancelled");
    assert.equal(calls, 1);
  } finally { await s.close(); }
});
