import test from "node:test";
import assert from "node:assert/strict";
import { createServer, type Server } from "node:http";
import { once } from "node:events";
import { RecipientTransport, RecipientTransportError, readBoundedTransportBody } from "../../src/recipient/transport.ts";

async function fixture(handler: Parameters<typeof createServer>[0]) {
  const server: Server = createServer(handler);
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  const address = server.address();
  assert(address && typeof address === "object");
  return { server, origin: `http://127.0.0.1:${address.port}`, close: () => new Promise<void>((resolve, reject) => {
    server.close(e => e ? reject(e) : resolve()); server.closeAllConnections();
  }) };
}

function category(promise: Promise<unknown>, expected: string) {
  return assert.rejects(promise, (error: unknown) => error instanceof RecipientTransportError && error.category === expected && error.message === expected);
}

test("each GET prepares authorization and only its rejected token is retried once", async () => {
  const headers: string[] = [], phases: string[] = [];
  const f = await fixture((req, res) => {
    headers.push(req.headers.authorization ?? "");
    if (headers.length === 1) { res.writeHead(401); res.end("private diagnostic"); }
    else res.end("{}");
  });
  try {
    let prepares = 0;
    const client = new RecipientTransport(f.origin, "unused-initial", { authorize: async request => {
      phases.push(request.state);
      if (request.state === "retry") { assert.equal(request.token, "token-1"); return { state: "authorized", token: "retried" }; }
      assert.equal(request.state, "prepare"); prepares++; return { state: "authorized", token: `token-${prepares}` };
    } });
    await client.getJson("/v1/account"); await client.getJson("/v1/account");
    assert.deepEqual(headers, ["Bearer token-1", "Bearer retried", "Bearer token-2"]);
    assert.deepEqual(phases, ["prepare", "retry", "prepare"]);
  } finally { await f.close(); }
});

test("a second 401 rejects only the current token; forbidden requests never refresh", async () => {
  let status = 401, calls = 0; const phases: string[] = [];
  const f = await fixture((_req, res) => { calls++; res.writeHead(status); res.end(); });
  try {
    const client = new RecipientTransport(f.origin, "initial", { authorize: async request => {
      phases.push(request.state);
      if (request.state === "reject") { assert.equal(request.token, "new-token"); return { state: "rejected" }; }
      return { state: "authorized", token: request.state === "prepare" ? "old-token" : "new-token" };
    } });
    await category(client.getJson("/v1/account"), "auth_required");
    assert.equal(calls, 2); assert.deepEqual(phases, ["prepare", "retry", "reject"]);
    calls = 0; phases.length = 0; status = 403;
    await category(client.getJson("/v1/account"), "forbidden");
    assert.equal(calls, 1); assert.deepEqual(phases, ["prepare"]);
  } finally { await f.close(); }
});

test("unchanged tokens are never replayed and authorization waits share the request deadline", async () => {
  let calls = 0;
  const f = await fixture((_req, res) => { calls++; res.writeHead(401); res.end(); });
  try {
    const same = new RecipientTransport(f.origin, "initial", { authorize: async () => ({ state: "authorized", token: "same" }) });
    await category(same.getJson("/v1/account"), "auth_required"); assert.equal(calls, 1);
    const stalled = new RecipientTransport(f.origin, "initial", { timeoutMs: 20, authorize: async () => new Promise(() => {}) });
    await category(stalled.getJson("/v1/account"), "timeout"); assert.equal(calls, 1);
  } finally { await f.close(); }
});

test("mount-aware routes, bearer-only headers, and real response bytes", async () => {
  const seen: Array<{ url?: string; auth?: string; cookie?: string; referer?: string }> = [];
  const f = await fixture((req, res) => {
    seen.push({ url: req.url, auth: req.headers.authorization, cookie: req.headers.cookie, referer: req.headers.referer });
    res.setHeader("content-type", "application/octet-stream"); res.end(Buffer.from([0, 1, 255]));
  });
  try {
    const client = new RecipientTransport(`${f.origin}/connect`, "a-valid-token");
    assert.deepEqual([...await client.getBytes("/v1/captures/0123456789abcdef0123456789abcdef/files/recording.mp4", 90_000_000)], [0, 1, 255]);
    assert.equal(client.server, `${f.origin}/connect`);
    assert.equal(seen[0]?.url, "/connect/v1/captures/0123456789abcdef0123456789abcdef/files/recording.mp4");
    assert.equal(seen[0]?.auth, "Bearer a-valid-token");
    assert.equal(seen[0]?.cookie, undefined); assert.equal(seen[0]?.referer, undefined);
    assert(!JSON.stringify(client).includes("a-valid-token"));
    for (const path of ["sealed/part-000001.age", "evidence/envelope.json", "media/sealed/part-000015.age"]) {
      assert.equal((await client.getBytes(`/v1/captures/0123456789abcdef0123456789abcdef/files/${path}`, 3)).length, 3);
    }
  } finally { await f.close(); }
});

test("rejects invalid origins, token header injection, and arbitrary routes", async () => {
  for (const server of ["https://example.test/connect/extra", "https://example.test/private/../connect", "https://user@example.test", "https://example.test/connect?x=1", "https://example.test/%63onnect", "http://192.168.0.1", "http://LOCALHOST", "http://127.0.0.01"]) {
    assert.throws(() => new RecipientTransport(server, "token"), (e: unknown) => e instanceof RecipientTransportError && e.category === "invalid_server");
  }
  assert.throws(() => new RecipientTransport("https://example.test", "x\r\nX-Evil: yes"), (e: unknown) => e instanceof RecipientTransportError && e.category === "invalid_request");
  const client = new RecipientTransport("https://example.test", "token");
  assert.equal(new RecipientTransport("https://EXAMPLE.test:443/connect/", "token").server, "https://EXAMPLE.test:443/connect");
  for (const route of ["https://other.test/v1/account", "/v1/account?x=1", "/v1/%61ccount", "/v1/account/"]) {
    assert.throws(() => client.getJson(route), (e: unknown) => e instanceof RecipientTransportError && e.category === "invalid_request");
  }
  for (const route of ["transport/evidence/" + "a".repeat(64) + ".json", "sealed/part-000016.age", "evidence/apparent.json", "../secret", "%65nvelope.json"]) {
    assert.throws(() => client.getBytes(`/v1/captures/0123456789abcdef0123456789abcdef/files/${route}`, 3), { category: "invalid_request" });
  }
});

test("refuses redirects without contacting the redirect target", async () => {
  let hits = 0;
  const target = await fixture((_req, res) => { hits++; res.end("should not be contacted"); });
  const source = await fixture((_req, res) => { res.writeHead(302, { location: `${target.origin}/v1/account` }); res.end(); });
  try {
    await category(new RecipientTransport(source.origin, "token").getJson("/v1/account"), "redirect_refused");
    assert.equal(hits, 0);
  } finally { await source.close(); await target.close(); }
});

test("caps streaming responses and maps malformed strict JSON to a safe category", async () => {
  const f = await fixture((req, res) => {
    if (req.url === "/v1/account") res.end('{"x":1,"x":2}');
    else { res.write(Buffer.alloc(8)); res.end(Buffer.alloc(8)); }
  });
  try {
    const client = new RecipientTransport(f.origin, "token");
    await category(client.getJson("/v1/account"), "invalid_response");
    await category(client.getBytes("/v1/captures/0123456789abcdef0123456789abcdef/files/contact-sheet.png", 10), "response_too_large");
  } finally { await f.close(); }
});

test("caller cancellation and full-body deadline are categorized", async () => {
  const f = await fixture((req, res) => {
    res.writeHead(200, { "content-type": "application/octet-stream", "transfer-encoding": "chunked" }); res.write("x");
    if (req.url?.endsWith("contact-sheet.png")) setTimeout(() => res.end("later"), 1000);
  });
  const route = "/v1/captures/0123456789abcdef0123456789abcdef/files/contact-sheet.png";
  try {
    const controller = new AbortController(); controller.abort();
    await category(new RecipientTransport(f.origin, "token", { signal: controller.signal }).getBytes(route, 10), "cancelled");
    await category(new RecipientTransport(f.origin, "token", { timeoutMs: 30 }).getBytes(route, 10), "timeout");
  } finally { await f.close(); }
});

test("does not return a truncated body and aborts an in-flight body read", async () => {
  let requestCount = 0;
  const f = await fixture((req, res) => {
    requestCount++;
    if (requestCount === 1) { res.writeHead(200, { "content-length": "8" }); res.end("short"); return; }
    res.writeHead(200, { "content-type": "application/octet-stream", "transfer-encoding": "chunked" }); res.write("x");
  });
  const route = "/v1/captures/0123456789abcdef0123456789abcdef/files/contact-sheet.png";
  try {
    await category(new RecipientTransport(f.origin, "token").getBytes(route, 10), "request_failed");
    const controller = new AbortController();
    const pending = new RecipientTransport(f.origin, "token", { signal: controller.signal }).getBytes(route, 10);
    await new Promise(resolve => setTimeout(resolve, 20)); controller.abort();
    await category(pending, "cancelled");
  } finally { await f.close(); }
});

test("bounded response consumption preserves returned bytes and wipes intermediate chunks", async () => {
  const first = new Uint8Array([1, 2]), second = new Uint8Array([3]);
  const body = new ReadableStream<Uint8Array>({ start(controller) { controller.enqueue(first); controller.enqueue(second); controller.close(); } });
  assert.deepEqual(await readBoundedTransportBody(body, 3, new AbortController().signal), new Uint8Array([1, 2, 3]));
  assert.deepEqual(first, new Uint8Array(2)); assert.deepEqual(second, new Uint8Array(1));
});

test("bounded response refusal wipes retained and overflowing chunks", async () => {
  const first = new Uint8Array([1, 2]), second = new Uint8Array([3, 4]);
  const body = new ReadableStream<Uint8Array>({ start(controller) { controller.enqueue(first); controller.enqueue(second); controller.close(); } });
  await assert.rejects(readBoundedTransportBody(body, 3, new AbortController().signal), error => error instanceof RecipientTransportError && error.category === "response_too_large");
  assert.deepEqual(first, new Uint8Array(2)); assert.deepEqual(second, new Uint8Array(2));
});
