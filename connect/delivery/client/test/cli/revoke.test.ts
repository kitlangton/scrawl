import test from "node:test";
import assert from "node:assert/strict";
import { createServer, type RequestListener } from "node:http";
import { once } from "node:events";
import { revokeDeviceGrant } from "../../src/account/revoke.ts";
import type { StoredCredential } from "../../src/credentials/wire.ts";
import { RecipientTransportError } from "../../src/recipient/transport.ts";

async function service(listener: RequestListener, run: (origin: string) => Promise<void>) {
  const server = createServer(listener); server.listen(0, "127.0.0.1"); await once(server, "listening");
  const address = server.address(); assert(address && typeof address !== "string");
  try { await run(`http://127.0.0.1:${address.port}`); }
  finally { server.closeAllConnections(); await new Promise<void>(resolve => server.close(() => resolve())); }
}
const failure = (category: string) => (error: unknown) => error instanceof RecipientTransportError && error.category === category && error.message === category;
const credential = (server: string, changes: Partial<StoredCredential> = {}): StoredCredential => ({
  server, site: null, scope: "read", token: "access-secret", generation: "0123456789abcdef0123456789abcdef",
  refreshToken: "refresh-secret", expiresAt: 1, refreshExpiresAt: 2, grantId: "grant-1", authRequired: true,
  encryption_identity: null, accountInitial: null, ...changes,
});

test("renewable revoke prefers refresh credential, pins scope and server, and returns frozen acknowledgement", async () => {
  const seen: Array<{ path: string; body: string; authorization?: string; cookie?: string; referer?: string }> = [];
  await service(async (req, res) => {
    let body = ""; for await (const chunk of req) body += chunk.toString();
    seen.push({ path: req.url!, body, authorization: req.headers.authorization, cookie: req.headers.cookie, referer: req.headers.referer });
    res.setHeader("Content-Type", "application/json"); res.end('{"ok":true}');
  }, async origin => {
    const result = await revokeDeviceGrant(credential(origin, { scope: "upload" }), { approvedServer: origin });
    assert.deepEqual(result, { ok: true }); assert.equal(Object.isFrozen(result), true);
  });
  assert.deepEqual(seen, [{ path: "/v1/token/revoke", body: '{"refreshToken":"refresh-secret"}', authorization: undefined, cookie: undefined, referer: undefined }]);
});

test("legacy revoke uses bearer and omits null refreshToken; expired and auth-required facts do not block cleanup", async () => {
  await service(async (req, res) => {
    let body = ""; for await (const chunk of req) body += chunk.toString();
    assert.equal(req.url, "/v1/token/revoke"); assert.equal(req.headers.authorization, "Bearer access-secret");
    assert.equal(body, "{}"); res.end('{"ok":true}');
  }, async origin => {
    await revokeDeviceGrant(credential(origin, { refreshToken: null, refreshExpiresAt: null, grantId: null, expiresAt: null, authRequired: true }), { approvedServer: origin });
  });
});

test("stored tuple and approved server are validated before one request", async () => {
  let calls = 0;
  await service((_req, res) => { calls++; res.end('{"ok":true}'); }, async origin => {
    await assert.rejects(revokeDeviceGrant(credential(`${origin}/connect`), { approvedServer: origin }), failure("invalid_request"));
    await assert.rejects(revokeDeviceGrant(credential(origin, { scope: "admin" as never }), { approvedServer: origin }), failure("invalid_request"));
    await assert.rejects(revokeDeviceGrant(credential(origin), { approvedServer: `${origin}/other` }), failure("invalid_server"));
    await assert.rejects(revokeDeviceGrant(credential(origin, { refreshToken: null }), { approvedServer: origin }), failure("invalid_request"));
    await assert.rejects(revokeDeviceGrant(credential(origin, { token: "private invalid token" }), { approvedServer: origin }), failure("invalid_request"));
  });
  assert.equal(calls, 0);
});

test("acknowledgement must be strict known success and errors expose no response text", async () => {
  for (const body of ["", "not json", '{"ok":true,"extra":"x"}', '{"ok":false}', '{"ok":true,"ok":true}']) {
    await service((_req, res) => { res.end(body); }, async origin => {
      await assert.rejects(revokeDeviceGrant(credential(origin), { approvedServer: origin }), failure("invalid_response"));
    });
  }
  await service((_req, res) => { res.writeHead(401); res.end("access-secret provider-private-text"); }, async origin => {
    await assert.rejects(revokeDeviceGrant(credential(origin), { approvedServer: origin }), error => {
      assert(failure("unauthorized")(error)); assert(!String(error).includes("access-secret")); assert(!String(error).includes("provider-private-text")); return true;
    });
  });
});

test("duplicate-key and oversized acknowledgement bodies are rejected", async () => {
  await service((_req, res) => { res.end('{"ok":true,"ok":true}'); }, async origin => {
    await assert.rejects(revokeDeviceGrant(credential(origin), { approvedServer: origin }), failure("invalid_response"));
  });
  await service((_req, res) => { res.end(Buffer.alloc(2_000_001, 65)); }, async origin => {
    await assert.rejects(revokeDeviceGrant(credential(origin), { approvedServer: origin }), failure("response_too_large"));
  });
});

test("credential and options mutations after dispatch cannot change the pinned request", async () => {
  let arrived!: () => void; const seen = new Promise<void>(resolve => { arrived = resolve; });
  let body = "";
  await service(async (req, res) => {
    for await (const chunk of req) body += chunk.toString(); arrived();
    await new Promise(resolve => setTimeout(resolve, 10)); res.end('{"ok":true}');
  }, async origin => {
    const stored = credential(origin) as StoredCredential & { token: string; refreshToken: string };
    const options = { approvedServer: origin, active: () => true };
    const pending = revokeDeviceGrant(stored, options);
    await seen; stored.refreshToken = "sibling-secret"; stored.token = "mutated-secret"; options.approvedServer = `${origin}/connect`;
    await pending;
  });
  assert.equal(body, '{"refreshToken":"refresh-secret"}');
});

test("each failed response performs one request and cancellation or inactive fence is honored", async () => {
  for (const [status, category] of [[429, "slow_down"], [307, "redirect_refused"]] as const) {
    let calls = 0;
    await service((_req, res) => { calls++; res.writeHead(status, { Location: "/elsewhere" }); res.end("private"); }, async origin => {
      await assert.rejects(revokeDeviceGrant(credential(origin), { approvedServer: origin }), failure(category));
    });
    assert.equal(calls, 1);
  }
  let calls = 0;
  await service((_req, res) => { calls++; res.end('{"ok":true}'); }, async origin => {
    const controller = new AbortController(); controller.abort();
    await assert.rejects(revokeDeviceGrant(credential(origin), { approvedServer: origin, signal: controller.signal }), failure("cancelled"));
    await assert.rejects(revokeDeviceGrant(credential(origin), { approvedServer: origin, active: () => false }), failure("cancelled"));
  });
  assert.equal(calls, 0);
});
