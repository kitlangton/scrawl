import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { createServer, type Server } from "node:http";
import { once } from "node:events";
import { uploadPreparedPlainCapture } from "../../src/sender/transport.ts";
import { RecipientTransportError } from "../../src/recipient/transport.ts";

const id = "0123456789abcdef0123456789abcdef";
const data = new Map<string, Uint8Array>([
  ["apparent.json", new TextEncoder().encode("{}")],
  ["summary.md", new TextEncoder().encode("synthetic upload\n")],
  ["contact-sheet.png", Uint8Array.of(1, 2, 3)],
]);
const digest = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
const manifest = [...data].map(([path, bytes]) => ({ path, size: bytes.length, sha256: digest(bytes) }));
async function fixture(handler: Parameters<typeof createServer>[0]) {
  const server: Server = createServer(handler); server.listen(0, "127.0.0.1"); await once(server, "listening");
  const address = server.address(); assert(address && typeof address === "object");
  return { server, origin: `http://127.0.0.1:${address.port}`, close: () => new Promise<void>(resolve => { server.closeAllConnections(); server.close(() => resolve()); }) };
}
function getBody(req: import("node:http").IncomingMessage): Promise<Buffer> {
  return new Promise((resolve, reject) => { const parts: Buffer[] = []; req.on("data", chunk => parts.push(Buffer.from(chunk))); req.on("end", () => resolve(Buffer.concat(parts))); req.on("error", reject); });
}
const options = (server: string, files = manifest) => ({ server, captureId: id, files,
  active: () => true,
  readAsset: async (path: string) => new Uint8Array(data.get(path)!),
  authorize: async (approvedServer: string, scope: "upload") => ({ approvedServer, scope, token: "synthetic-token" }) });
const capture = (promise: Promise<unknown>, category: string) => assert.rejects(promise, (error: unknown) =>
  error instanceof RecipientTransportError && error.category === category && error.message === category);

test("pending plaintext reservation uploads exact mounted paths and verifies unordered readback", async () => {
  const requests: Array<{ method: string; url?: string; auth?: string }> = [];
  let stored: Array<{ path: string; size: number; sha256: string }> = [];
  const f = await fixture(async (req, res) => {
    requests.push({ method: req.method!, url: req.url, auth: req.headers.authorization });
    const body = await getBody(req);
    if (req.url === `/connect/v1/captures/${id}` && req.method === "POST") { stored = JSON.parse(body.toString()).files; res.end(JSON.stringify({ id, status: "pending" })); }
    else if (req.url?.endsWith("/complete")) res.end(JSON.stringify({ id, status: "ready" }));
    else if (req.url === `/connect/v1/captures/${id}`) res.end(JSON.stringify({ id, status: "ready", files: [...stored].reverse() }));
    else { assert.equal(req.method, "PUT"); const path = req.url!.split("/files/")[1]!; assert.deepEqual(body, Buffer.from(data.get(path)!)); res.end("{}"); }
  });
  try {
    const borrowed = new Map([...data].map(([name, bytes]) => [name, new Uint8Array(bytes)]));
    await uploadPreparedPlainCapture({ ...options(`${f.origin}/connect`), readAsset: async name => borrowed.get(name)! });
    for (const [name, bytes] of borrowed) assert.deepEqual(bytes, data.get(name), "caller-owned bytes remain intact");
    assert.deepEqual(requests.map(r => r.url), [
      `/connect/v1/captures/${id}`, ...manifest.map(file => `/connect/v1/captures/${id}/files/${file.path}`),
      `/connect/v1/captures/${id}/complete`, `/connect/v1/captures/${id}`,
    ]);
    assert(requests.every(r => r.auth === "Bearer synthetic-token"));
  } finally { await f.close(); }
});

test("ready reservations skip PUT and complete but still require readback", async () => {
  let calls = 0;
  const f = await fixture(async (req, res) => { calls++; await getBody(req); res.end(JSON.stringify(req.method === "POST" ? { id, status: "ready" } : { id, status: "ready", files: manifest })); });
  try { await uploadPreparedPlainCapture(options(f.origin)); assert.equal(calls, 2); }
  finally { await f.close(); }
});

test("rejects malformed manifests, identifiers and reservation receipts before uploading", async () => {
  let calls = 0;
  const f = await fixture((_req, res) => { calls++; res.end("{}"); });
  try {
    await capture(uploadPreparedPlainCapture(options(f.origin, [...manifest, manifest[0]!])), "invalid_request");
    await capture(uploadPreparedPlainCapture({ ...options(f.origin), captureId: "../bad" }), "invalid_request");
    assert.equal(calls, 0);
    const bad = await fixture(async (_req, res) => { await getBody(_req); res.end(JSON.stringify({ id: "f".repeat(32), status: "pending" })); });
    try { await capture(uploadPreparedPlainCapture(options(bad.origin)), "invalid_response"); }
    finally { await bad.close(); }
  } finally { await f.close(); }
});

test("rejects bad asset bytes before PUT and refuses mismatched readback", async () => {
  let puts = 0;
  const f = await fixture(async (req, res) => {
    const body = await getBody(req);
    if (req.method === "PUT") puts++;
    res.end(JSON.stringify(req.method === "POST" && req.url?.endsWith("/complete") ? { id, status: "ready" }
      : req.method === "POST" ? { id, status: "pending" } : { id, status: "ready", files: manifest.map((file, i) => i ? file : { ...file, size: file.size + 1 }) }));
    void body;
  });
  try {
    const badReader = { ...options(f.origin), readAsset: async (path: string) => path === "summary.md" ? Uint8Array.of(8) : new Uint8Array(data.get(path)!) };
    await capture(uploadPreparedPlainCapture(badReader), "stale_snapshot"); assert.equal(puts, 1, "only the earlier verified asset was sent");
    await capture(uploadPreparedPlainCapture(options(f.origin)), "invalid_response");
  } finally { await f.close(); }
});

test("pins authorization, rejects redirects, and never exposes bearer tokens in errors", async () => {
  let calls = 0;
  const f = await fixture((_req, res) => { calls++; res.writeHead(302, { location: "https://elsewhere.invalid" }); res.end("private"); });
  try {
    const pinned = { ...options(f.origin), authorize: async () => ({ approvedServer: "https://other.invalid", scope: "upload" as const, token: "secret-token" }) };
    await capture(uploadPreparedPlainCapture(pinned), "auth_required"); assert.equal(calls, 0);
    const redirected = { ...options(f.origin), authorize: async (approvedServer: string, scope: "upload") => ({ approvedServer, scope, token: "secret-token" }) };
    let error: unknown;
    try { await uploadPreparedPlainCapture(redirected); assert.fail("redirect should fail"); } catch (cause) { error = cause; }
    assert.equal((error as Error).message, "redirect_refused"); assert(!String(error).includes("secret-token")); assert.equal(calls, 1);
  } finally { await f.close(); }
});

test("requires upload scope on every request and rejects incorrect readback ID or status", async () => {
  let calls = 0;
  const f = await fixture(async (_req, res) => { calls++; await getBody(_req); res.end(JSON.stringify(calls === 1
    ? { id, status: "ready" } : { id: "f".repeat(32), status: "pending", files: manifest })); });
  try {
    const wrongScope = { ...options(f.origin), authorize: async (approvedServer: string) => ({ approvedServer, scope: "read" as "upload", token: "synthetic-token" }) };
    await capture(uploadPreparedPlainCapture(wrongScope), "auth_required"); assert.equal(calls, 0);
    await capture(uploadPreparedPlainCapture(options(f.origin)), "invalid_response");
  } finally { await f.close(); }
});

test("pre-cancellation and invalid server configuration dispatch no request", async () => {
  let calls = 0;
  const f = await fixture((_req, res) => { calls++; res.end("{}"); });
  try {
    const controller = new AbortController(); controller.abort();
    await capture(uploadPreparedPlainCapture({ ...options(f.origin), signal: controller.signal }), "cancelled");
    await capture(uploadPreparedPlainCapture(options(`${f.origin}?private=1`)), "invalid_server");
    assert.equal(calls, 0);
  } finally { await f.close(); }
});

test("checks caller freshness at every request and before final readback", async () => {
  let calls = 0, activeChecks = 0;
  const f = await fixture(async (req, res) => { calls++; const body = await getBody(req); res.end(JSON.stringify(req.method === "POST" && req.url?.endsWith("/complete") ? { id, status: "ready" }
    : req.method === "POST" ? { id, status: "pending" } : { id, status: "ready", files: manifest })); void body; });
  try {
    await capture(uploadPreparedPlainCapture({ ...options(f.origin), active: () => ++activeChecks < 5 }), "stale_snapshot");
    assert(calls < 2 + manifest.length + 2, "stale callback stops before a later mutation/readback request");
  } finally { await f.close(); }
});

test("snapshot-reader failures are redacted and cancellation settles even if the reader ignores its signal", async () => {
  const f = await fixture(async (req, res) => { await getBody(req); res.end(JSON.stringify({ id, status: "pending" })); });
  try {
    await capture(uploadPreparedPlainCapture({ ...options(f.origin), readAsset: async () => { throw Error("private-token /private/capture"); } }), "request_failed");
    const modified = new RecipientTransportError("invalid_response"); modified.message = "private-token /private/capture";
    await capture(uploadPreparedPlainCapture({ ...options(f.origin), readAsset: async () => { throw modified; } }), "invalid_response");
    const controller = new AbortController();
    let started!: () => void;
    const reading = new Promise<void>(resolve => { started = resolve; });
    const upload = uploadPreparedPlainCapture({ ...options(f.origin), signal: controller.signal,
      readAsset: async () => { started(); return new Promise<Uint8Array>(() => {}); } });
    await reading; controller.abort();
    await capture(upload, "cancelled");
  } finally { await f.close(); }
});

test("captures callbacks, identifiers and cancellation signal before the first await", async () => {
  const f = await fixture(async (req, res) => { await getBody(req); res.end(JSON.stringify(req.method === "POST"
    ? { id, status: "ready" } : { id, status: "ready", files: manifest })); });
  try {
    const input = { ...options(f.origin), signal: new AbortController().signal };
    const original = input.authorize;
    input.authorize = async (...args) => {
      input.captureId = "f".repeat(32); input.authorize = async () => { throw Error("mutated-secret"); };
      return original(...args);
    };
    await uploadPreparedPlainCapture(input);
  } finally { await f.close(); }
});

test("cancellation inside the freshness guard does not start an unbounded authorization callback", { timeout: 1000 }, async () => {
  const controller = new AbortController(); let calls = 0;
  await capture(uploadPreparedPlainCapture({ ...options("http://127.0.0.1:1"), signal: controller.signal,
    active: () => { controller.abort(); return true; },
    authorize: async () => { calls++; return new Promise<never>(() => {}); } }), "cancelled");
  assert.equal(calls, 0);
});
