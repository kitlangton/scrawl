import test from "node:test";
import assert from "node:assert/strict";
import { createServer, type RequestListener } from "node:http";
import { once } from "node:events";
import { requestDevicePoll, requestDeviceStart } from "../../src/account/device.ts";
import { RecipientTransportError } from "../../src/recipient/transport.ts";

async function service(listener: RequestListener, run: (origin: string) => Promise<void>) {
  const server = createServer(listener); server.listen(0, "127.0.0.1"); await once(server, "listening");
  const address = server.address(); assert(address && typeof address !== "string");
  try { await run(`http://127.0.0.1:${address.port}`); }
  finally { server.closeAllConnections(); await new Promise<void>(resolve => server.close(() => resolve())); }
}
const failure = (name: string) => (error: unknown) => error instanceof RecipientTransportError && error.category === name && error.message === name;
const respond = (res: import("node:http").ServerResponse, body = '{"status":"pending"}') => {
  res.writeHead(200, { "Content-Type": "application/json" }); res.end(body);
};

test("start and poll use one exact anonymous POST at root and mounted bases", async () => {
  const seen: Array<{ path: string; body: string }> = [];
  await service(async (req, res) => {
    let body = ""; for await (const chunk of req) body += chunk.toString();
    assert.equal(req.method, "POST"); assert.equal(req.headers.authorization, undefined);
    assert.equal(req.headers.cookie, undefined); assert.equal(req.headers.referer, undefined);
    assert.equal(req.headers["content-type"], "application/json");
    seen.push({ path: req.url!, body }); respond(res);
  }, async origin => {
    await requestDeviceStart({ approvedServer: `${origin}/`, scope: "read" });
    await requestDeviceStart({ approvedServer: `${origin}/connect/`, scope: "upload" });
    await requestDevicePoll({ approvedServer: origin, deviceCode: "code-123" });
    await requestDevicePoll({ approvedServer: `${origin}/connect`, deviceCode: "code-456", timeoutMs: 1000 });
  });
  assert.deepEqual(seen, [
    { path: "/v1/device/start", body: '{"scope":"read"}' },
    { path: "/connect/v1/device/start", body: '{"scope":"upload"}' },
    { path: "/v1/device/poll", body: '{"deviceCode":"code-123"}' },
    { path: "/connect/v1/device/poll", body: '{"deviceCode":"code-456"}' },
  ]);
});

test("invalid server, scope, public key, code and timeout are rejected before HTTP", async () => {
  let calls = 0;
  await service((_req, res) => { calls++; respond(res); }, async origin => {
    for (const approvedServer of [`${origin}/other`, "https://user:pass@example.com"]) {
      await assert.rejects(requestDeviceStart({ approvedServer, scope: "read" }), failure("invalid_server"));
    }
    assert.throws(() => requestDeviceStart({ approvedServer: origin, scope: "admin" as never }), failure("invalid_request"));
    assert.throws(() => requestDeviceStart({ approvedServer: origin, scope: "read", encryptionKey: { version: 1, recipient: "not-a-key", signing: "x" } }), failure("invalid_request"));
    for (const deviceCode of ["", "has space", "x".repeat(8193)])
      assert.throws(() => requestDevicePoll({ approvedServer: origin, deviceCode }), failure("invalid_request"));
    for (const timeoutMs of [0, -1, 30_001, NaN])
      await assert.rejects(requestDevicePoll({ approvedServer: origin, deviceCode: "valid", timeoutMs }), failure("invalid_request"));
    for (const signal of [null, {}, { aborted: false }] as unknown as AbortSignal[])
      await assert.rejects(requestDevicePoll({ approvedServer: origin, deviceCode: "valid", signal }), failure("invalid_request"));
    const aborted = new AbortController(); aborted.abort();
    await assert.rejects(requestDevicePoll({ approvedServer: origin, deviceCode: "valid", signal: aborted.signal }), failure("cancelled"));
  });
  assert.equal(calls, 0);
});

test("options are detached before awaiting and active is checked around network I/O", async () => {
  let arrived!: () => void; const seen = new Promise<void>(resolve => { arrived = resolve; });
  let requestPath = "", requestBody = "";
  await service(async (req, res) => {
    requestPath = req.url!; for await (const chunk of req) requestBody += chunk.toString(); arrived();
    await new Promise(resolve => setTimeout(resolve, 15)); respond(res);
  }, async origin => {
    const active = { value: true };
    const options: { approvedServer: string; scope: "read" | "upload"; timeoutMs: number; active: () => boolean } = { approvedServer: origin, scope: "read", timeoutMs: 2000, active: () => active.value };
    const pending = requestDeviceStart(options);
    await seen; options.approvedServer = `${origin}/connect`; options.scope = "upload"; active.value = false;
    await assert.rejects(pending, failure("cancelled"));
  });
  assert.equal(requestPath, "/v1/device/start"); assert.equal(requestBody, '{"scope":"read"}');
});

test("HTTP failure categories are stable and never include provider text", async () => {
  for (const [status, expected] of [[429, "slow_down"], [503, "request_failed"], [401, "unauthorized"], [403, "forbidden"]] as const) {
    let calls = 0;
    await service((_req, res) => { calls++; res.writeHead(status); res.end("provider secret diagnostic"); }, async origin => {
      await assert.rejects(requestDevicePoll({ approvedServer: origin, deviceCode: "code" }), failure(expected));
    });
    assert.equal(calls, 1);
  }
});

test("redirects are refused, malformed and duplicate-key JSON is rejected, and response size is bounded", async () => {
  await service((req, res) => {
    if (req.url === "/other") { res.end("unexpected"); return; }
    if (req.url === "/v1/device/start") { res.writeHead(307, { Location: "/other" }); res.end(); return; }
    if (req.url === "/v1/device/poll") { res.end('{"status":"pending","status":"approved"}'); return; }
  }, async origin => {
    await assert.rejects(requestDeviceStart({ approvedServer: origin, scope: "read" }), failure("redirect_refused"));
    await assert.rejects(requestDevicePoll({ approvedServer: origin, deviceCode: "code" }), failure("invalid_response"));
  });
  await service((_req, res) => { res.end(Buffer.alloc(2_000_001, 65)); }, async origin => {
    await assert.rejects(requestDevicePoll({ approvedServer: origin, deviceCode: "code" }), failure("response_too_large"));
  });
});

test("caller cancellation and deadline cover a stalled response body", async () => {
  let arrived!: () => void; const seen = new Promise<void>(resolve => { arrived = resolve; });
  await service((_req, res) => { res.writeHead(200); res.write("{"); arrived(); }, async origin => {
    const controller = new AbortController();
    const pending = requestDevicePoll({ approvedServer: origin, deviceCode: "code", timeoutMs: 2000, signal: controller.signal });
    const rejected = assert.rejects(pending, failure("cancelled")); await seen; controller.abort(); await rejected;
  });
  await service((_req, res) => { res.writeHead(200); res.write("{"); }, async origin => {
    await assert.rejects(requestDevicePoll({ approvedServer: origin, deviceCode: "code", timeoutMs: 30 }), failure("timeout"));
  });
});
