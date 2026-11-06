import test from "node:test";
import assert from "node:assert/strict";
import { createServer, type RequestListener } from "node:http";
import { once } from "node:events";
import { requestRenewalReply, RenewalRequestError } from "../../src/credentials/renewal-request.ts";
import { decodeStoredCredential } from "../../src/credentials/wire.ts";

const credential = (server: string) => decodeStoredCredential(Buffer.from(JSON.stringify({
  server, scope: "read", token: "synthetic-access", refreshToken: "synthetic-refresh", generation: "a".repeat(32),
  expiresAt: 100, refreshExpiresAt: 200, grantId: "synthetic-grant",
})), "read", server);
async function service(listener: RequestListener, run: (server: string) => Promise<void>) {
  const server = createServer(listener); server.listen(0, "127.0.0.1"); await once(server, "listening");
  const address = server.address(); assert(address && typeof address !== "string");
  try { await run(`http://127.0.0.1:${address.port}`); }
  finally { server.closeAllConnections(); await new Promise<void>(resolve => server.close(() => resolve())); }
}
const category = (name: string) => (error: unknown) => error instanceof RenewalRequestError && error.category === name && error.message === name;

test("single mounted refresh sends only the refresh token and returns raw owned bytes", async () => {
  let calls = 0, submitted = "";
  await service(async (req, res) => {
    calls++; assert.equal(req.method, "POST"); assert.equal(req.url, "/connect/v1/token/refresh");
    assert.equal(req.headers.authorization, undefined); assert.equal(req.headers.cookie, undefined); assert.equal(req.headers.referer, undefined);
    assert.equal(req.headers["content-type"], "application/json");
    for await (const chunk of req) submitted += chunk.toString();
    res.end("raw reply must be persisted before parsing");
  }, async origin => {
    const server = `${origin}/connect`, c = credential(server);
    const bytes = await requestRenewalReply(c, { approvedServer: server });
    assert.equal(Buffer.from(bytes).toString(), "raw reply must be persisted before parsing");
    assert.equal(submitted, JSON.stringify({ refreshToken: "synthetic-refresh" }));
    assert.equal(c.token, "synthetic-access"); bytes.fill(0);
  });
  assert.equal(calls, 1);
});

test("redirects are refused without sending credentials to the target", async () => {
  let followed = 0;
  await service((req, res) => {
    if (req.url === "/other") { followed++; res.end("unexpected"); }
    else { res.writeHead(307, { Location: "/other" }); res.end(); }
  }, async server => {
    await assert.rejects(requestRenewalReply(credential(server), { approvedServer: server }), category("redirect_refused"));
  });
  assert.equal(followed, 0);
});

test("root and mounted trailing-slash profiles preserve exact stored approval", async () => {
  const urls: string[] = [];
  await service((req, res) => { urls.push(req.url!); res.end("reply"); }, async origin => {
    for (const server of [`${origin}/`, `${origin}/connect/`]) {
      const c = credential(server);
      assert.equal(Buffer.from(await requestRenewalReply(c, { approvedServer: server })).toString(), "reply");
      assert.equal(c.server, server);
    }
  });
  assert.deepEqual(urls, ["/v1/token/refresh", "/connect/v1/token/refresh"]);
});

test("HTTP failures expose only donor-compatible allowlisted categories", async () => {
  for (const [status, reply, expected] of [
    [401, "secret diagnostic", "auth_required"], [429, "secret diagnostic", "retry_later"], [503, "secret diagnostic", "retry_later"],
    [403, '{"error":{"code":"forbidden","message":"secret diagnostic"}}', "forbidden"],
    [400, '{"error":{"code":"secret diagnostic"}}', "http_error"],
    [400, '{"error":{"code":"forbidden","code":"secret diagnostic"}}', "http_error"],
  ] as const) {
    let calls = 0;
    await service((_req, res) => { calls++; res.writeHead(status); res.end(reply); }, async server => {
      await assert.rejects(requestRenewalReply(credential(server), { approvedServer: server }), category(expected));
    });
    assert.equal(calls, 1);
  }
});

test("bounded EOF consumption refuses oversized responses without retry", async () => {
  let calls = 0;
  await service((_req, res) => { calls++; res.end(Buffer.alloc(2_000_001, 65)); }, async server => {
    await assert.rejects(requestRenewalReply(credential(server), { approvedServer: server }), category("response_too_large"));
  });
  assert.equal(calls, 1);
});

test("one deadline bounds a response whose body never finishes", async () => {
  let calls = 0;
  await service((_req, res) => { calls++; res.writeHead(200); res.write("partial"); }, async server => {
    await assert.rejects(requestRenewalReply(credential(server), { approvedServer: server, timeoutMs: 1000 }), category("timeout"));
  });
  assert.equal(calls, 1);
});

test("caller cancellation aborts a pending response and does not retry", async () => {
  let arrived!: () => void; const seen = new Promise<void>(resolve => { arrived = resolve; }); let calls = 0;
  await service((_req, res) => { calls++; res.writeHead(200); res.write("partial"); arrived(); }, async server => {
    const controller = new AbortController();
    const pending = requestRenewalReply(credential(server), { approvedServer: server, signal: controller.signal });
    const rejected = assert.rejects(pending, category("cancelled"));
    await seen; controller.abort(); await rejected;
  });
  assert.equal(calls, 1);
});

test("invalid authority, deadlines and cancelled work never reach the service", async () => {
  let calls = 0;
  await service((_req, res) => { calls++; res.end(); }, async server => {
    const c = credential(server), cancelled = new AbortController(); cancelled.abort();
    await assert.rejects(requestRenewalReply(c, { approvedServer: server, signal: cancelled.signal }), category("cancelled"));
    for (const timeoutMs of [0, -1, 30_001, NaN]) await assert.rejects(requestRenewalReply(c, { approvedServer: server, timeoutMs }), category("invalid_request"));
    await assert.rejects(requestRenewalReply({ ...c, authRequired: true }, { approvedServer: server }), category("invalid_request"));
    await assert.rejects(requestRenewalReply({ ...c, refreshToken: null }, { approvedServer: server }), category("invalid_request"));
    await assert.rejects(requestRenewalReply(c, { approvedServer: `${server}/other` }), category("invalid_server"));
    await assert.rejects(requestRenewalReply(c, { approvedServer: `${server}/connect` }), category("invalid_server"));
  });
  assert.equal(calls, 0);
});
