import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { listShares, revokeShare, ShareRevocationError } from "../../src/account/shares.ts";

const id = "0123456789abcdef0123456789abcdef", other = "1123456789abcdef0123456789abcdef";
const generation = "b".repeat(32);
const secret = "a".repeat(64), now = "2026-01-02T03:04:05.000Z";
function row(overrides: Record<string, unknown> = {}) { return { id, captureId: other, title: "Apparent recording", status: "ready", createdAt: now, expiresAt: now, url: "http://127.0.0.1/s/" + secret, ...overrides }; }
async function withServer(handler: (request: import("node:http").IncomingMessage, response: import("node:http").ServerResponse) => void, run: (base: string, counts: { requests: number; auth: string[] }) => Promise<void>) {
  const counts = { requests: 0, auth: [] as string[] }, server = createServer((req, res) => { counts.requests++; counts.auth.push(String(req.headers.authorization || "")); handler(req, res); });
  await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve)); const address = server.address(); assert(address && typeof address !== "string");
  try { await run(`http://127.0.0.1:${address.port}`, counts); } finally { await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve())); }
}
function opts(server: string, scope: "read" | "upload" = "read", onAuth?: (scope: string) => void) { let n = 0; return { server, scope, generation, active: () => true, authorize: async (approvedServer: string, requested: "read" | "upload", _signal: AbortSignal) => { onAuth?.(requested); n++; return { approvedServer, scope: requested, token: `synthetic-${n}`, generation }; } }; }
function respond(res: import("node:http").ServerResponse, value: unknown, status = 200) { res.writeHead(status, { "content-type": "application/json" }); res.end(JSON.stringify(value)); }

test("lists every page, encodes cursor and pins active URLs to root or mounted server", async () => {
  await withServer((req, res) => {
    if (req.url === "/connect/v1/shares") return respond(res, { shares: [row({ id: other, url: `http://${req.headers.host}/connect/s/${secret}` })], nextCursor: "123:0123456789abcdef0123456789abcdef" });
    assert.equal(req.url, "/connect/v1/shares?before=123%3A0123456789abcdef0123456789abcdef");
    respond(res, { shares: [row({ status: "revoked", url: null })], nextCursor: null });
  }, async (base, counts) => {
    const listed = await listShares(opts(`${base}/connect/`));
    assert.equal(listed.shares.length, 2); assert.equal(listed.shares[1]!.status, "revoked"); assert.equal(listed.nextCursor, null); assert.equal(Object.isFrozen(listed.shares), true); assert.equal(Object.isFrozen(listed.shares[0]), true);
    assert.deepEqual(counts.auth, ["Bearer synthetic-1", "Bearer synthetic-2"]);
  });
});

test("rejects untrusted rows, duplicates, cursors, URLs, and strict JSON duplicate keys", async () => {
  const invalidBodies = [
    JSON.stringify({ shares: [row(), row()], nextCursor: null }),
    JSON.stringify({ shares: [row({ url: "http://127.0.0.1.evil/s/" + secret })], nextCursor: null }),
    JSON.stringify({ shares: [row({ url: "http://127.0.0.1/s/" + secret + "?x=y" })], nextCursor: null }),
    JSON.stringify({ shares: [row({ status: "ready", url: null })], nextCursor: null }),
    '{"shares":[],"shares":[],"nextCursor":null}',
    JSON.stringify({ shares: [], nextCursor: "123:0123456789abcdef0123456789abcdef" }),
  ];
  for (const text of invalidBodies) await withServer((req, res) => { res.writeHead(200, { "content-type": "application/json" }); res.end(text.replaceAll("http://127.0.0.1/s/", `http://${req.headers.host}/s/`)); }, async (base) => {
    await assert.rejects(listShares(opts(base)), { category: "invalid_response" });
  });
});

test("rejects wrong scope, invalid tokens and server mismatches before HTTP", async () => {
  await withServer((_req, res) => respond(res, { shares: [], nextCursor: null }), async base => {
    const wrong = { ...opts(base), authorize: async (approvedServer: string, _scope: "read" | "upload") => ({ approvedServer, scope: "upload" as const, token: "x", generation }) };
    await assert.rejects(listShares(wrong), { category: "auth_required" });
    const mismatch = { ...opts(base), authorize: async (_server: string, scope: "read" | "upload") => ({ approvedServer: base + "/connect", scope, token: "synthetic", generation }) };
    await assert.rejects(listShares(mismatch), { category: "auth_required" });
    const invalid = { ...opts(base), authorize: async (approvedServer: string, scope: "read" | "upload") => ({ approvedServer, scope, token: "bad token", generation }) };
    await assert.rejects(listShares(invalid), { category: "auth_required" });
    let malformedAuth = 0;
    const malformed = { ...opts(base), generation: { toString: () => generation } as unknown as string,
      authorize: async () => { malformedAuth++; throw Error("must not authorize"); } };
    await assert.rejects(listShares(malformed), { category: "invalid_request" }); assert.equal(malformedAuth, 0);
  });
});

test("refuses redirects and bounds response body without exposing server text", async () => {
  await withServer((_req, res) => { res.writeHead(302, { location: "http://example.test" }).end("secret body"); }, async base => {
    await assert.rejects(listShares(opts(base)), { category: "redirect_refused" });
  });
  await withServer((_req, res) => { res.writeHead(200).end("x".repeat(2_000_001)); }, async base => {
    await assert.rejects(listShares(opts(base)), { category: "response_too_large" });
  });
});

test("revoke sends one DELETE then verifies all pages; only revoked, expired, or absent confirms", async () => {
  let deletes = 0, reads = 0;
  await withServer((req, res) => {
    if (req.method === "DELETE") { deletes++; return respond(res, { ok: true }); }
    reads++; if (req.url === "/v1/shares") return respond(res, { shares: [], nextCursor: "1:1123456789abcdef0123456789abcdef" });
    return respond(res, { shares: [row({ id: other, status: "revoked", url: null })], nextCursor: null });
  }, async (base, counts) => {
    await revokeShare(opts(base, "upload"), id);
    assert.equal(deletes, 1); assert.equal(reads, 2); assert.deepEqual(counts.auth, ["Bearer synthetic-1", "Bearer synthetic-2", "Bearer synthetic-3"]);
  });
});

test("active and deleted readback remain unverified; readback failure preserves ack and never retries", async () => {
  for (const state of ["ready", "deleted"]) await withServer((req, res) => req.method === "DELETE" ? respond(res, { ok: true }) : respond(res, { shares: [row({ status: state, url: state === "deleted" ? null : `http://${req.headers.host}/s/${secret}` })], nextCursor: null }), async base => {
    await assert.rejects(revokeShare(opts(base, "upload"), id), error => error instanceof ShareRevocationError && error.effect === "acknowledged_unverified" && error.category === "revocation_unverified");
  });
  let deletes = 0;
  await withServer((req, res) => { if (req.method === "DELETE") { deletes++; return respond(res, { ok: true }); } res.writeHead(503).end("secret"); }, async base => {
    await assert.rejects(revokeShare(opts(base, "upload"), id), error => error instanceof ShareRevocationError && error.effect === "acknowledged_unverified"); assert.equal(deletes, 1);
  });
});

test("invalid management IDs fail before authorization or network", async () => {
  let auth = 0;
  await withServer((_req, res) => respond(res, { ok: true }), async base => {
    const options = { ...opts(base, "upload"), authorize: async (approvedServer: string, scope: "read" | "upload") => { auth++; return { approvedServer, scope, token: "synthetic", generation }; } };
    await assert.rejects(revokeShare(options, secret), { category: "invalid_request" }); assert.equal(auth, 0);
  });
});


test("projects known fields while tolerating additive service metadata", async () => {
  await withServer((req, res) => respond(res, { shares: [row({ url: `http://${req.headers.host}/s/${secret}`, credential: "never expose" })], nextCursor: null, future: "ignored" }), async base => {
    const result = await listShares(opts(base));
    assert.equal(result.shares.length, 1); assert.equal(JSON.stringify(result).includes("never expose"), false); assert.equal("future" in result, false);
  });
});

test("a still-active ID on a later page prevents revocation confirmation", async () => {
  let deletes = 0, gets = 0;
  await withServer((req, res) => {
    if (req.method === "DELETE") { deletes++; return respond(res, { ok: true, future: "ignored" }); }
    gets++;
    if (gets === 1) return respond(res, { shares: [], nextCursor: `1:${other}` });
    return respond(res, { shares: [row({ url: `http://${req.headers.host}/s/${secret}` })], nextCursor: null });
  }, async base => {
    await assert.rejects(revokeShare(opts(base, "upload"), id), error => error instanceof ShareRevocationError && error.effect === "acknowledged_unverified" && error.category === "revocation_unverified");
    assert.equal(deletes, 1); assert.equal(gets, 2);
  });
});

test("cancellation and callback errors are projected without request or response text", async () => {
  const abort = new AbortController();
  await withServer((_req, res) => { res.writeHead(200); res.write('{"shares":'); abort.abort(); }, async base => {
    await assert.rejects(listShares({ ...opts(base), signal: abort.signal }), { category: "cancelled" });
  });
  await assert.rejects(listShares({ ...opts("https://connect.example"), authorize: async () => { throw Error("private token and path"); } }), { category: "request_failed" });
});


test("a login change cannot mix list pages or revoke readback authority", async () => {
  for (const operation of ["list", "revoke"]) {
    let grants = 0;
    await withServer((req, res) => {
      if (req.method === "DELETE") return respond(res, { ok: true });
      respond(res, { shares: [], nextCursor: grants === 1 ? `1:${other}` : null });
    }, async (base, counts) => {
      const options = { ...opts(base, operation === "list" ? "read" : "upload"),
        authorize: async (approvedServer: string, scope: "read" | "upload") => ({ approvedServer, scope, token: "synthetic", generation: ++grants === 1 ? generation : "c".repeat(32) }) };
      if (operation === "list") await assert.rejects(listShares(options), { category: "stale_snapshot" });
      else await assert.rejects(revokeShare(options, id), error => error instanceof ShareRevocationError && error.effect === "acknowledged_unverified" && error.category === "stale_snapshot");
      assert.equal(counts.requests, 1, operation);
    });
  }
});
