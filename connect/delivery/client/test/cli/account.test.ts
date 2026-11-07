import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { readAccountStatus } from "../../src/account/status.ts";

const generation = "b".repeat(32);
function options(server: string, scope: "read" | "upload" = "read", active: () => boolean = () => true) {
  return { server, scope, generation, active, authorize: async (approvedServer: string, requested: "read" | "upload") => ({ approvedServer, scope: requested, token: "synthetic-token", generation }) };
}
async function withServer(handler: (request: import("node:http").IncomingMessage, response: import("node:http").ServerResponse) => void,
  run: (base: string, seen: { requests: number; auth: string[]; paths: string[] }) => Promise<void>) {
  const seen = { requests: 0, auth: [] as string[], paths: [] as string[] };
  const server = createServer((req, res) => { seen.requests++; seen.auth.push(String(req.headers.authorization || "")); seen.paths.push(String(req.url)); handler(req, res); });
  await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve)); const address = server.address(); assert(address && typeof address !== "string");
  try { await run(`http://127.0.0.1:${address.port}`, seen); } finally { await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve())); }
}
function respond(res: import("node:http").ServerResponse, value: unknown) { res.writeHead(200, { "content-type": "application/json" }); res.end(JSON.stringify(value)); }

test("reads exactly one scoped account status and projects frozen known fields", async () => {
  for (const scope of ["read", "upload"] as const) await withServer((_req, res) => respond(res, { beta: "active", quotaBytes: 123, usedBytes: 999, secret: "exclude" }), async (base, seen) => {
    const result = await readAccountStatus(options(base, scope));
    assert.deepEqual(result, { beta: "active", quotaBytes: 123, usedBytes: 999 }); assert.equal(Object.isFrozen(result), true);
    assert.equal("secret" in result, false); assert.equal(seen.requests, 1); assert.deepEqual(seen.paths, ["/v1/account"]); assert.deepEqual(seen.auth, ["Bearer synthetic-token"]);
  });
});

test("pins endpoint, scope, and generation before async authorization", async () => {
  await withServer((_req, res) => respond(res, { beta: "waitlist", quotaBytes: 0, usedBytes: 0 }), async (base, seen) => {
    const mutable = options(base, "read"); let release!: (value: { approvedServer: string; scope: "read"; token: string; generation: string }) => void;
    mutable.authorize = () => new Promise(resolve => { release = resolve; });
    const pending = readAccountStatus(mutable); mutable.server = "https://evil.example"; mutable.scope = "upload"; mutable.generation = "c".repeat(32);
    await new Promise(resolve => setImmediate(resolve)); release({ approvedServer: base, scope: "read", token: "synthetic", generation });
    await pending; assert.deepEqual(seen.paths, ["/v1/account"]);
  });
  await withServer((_req, res) => respond(res, { beta: "active", quotaBytes: 1, usedBytes: 0 }), async (base, seen) => {
    const invalidAuthority = [
      { authorize: async (server: string, scope: "read" | "upload") => ({ approvedServer: server, scope, token: "x", generation: "c".repeat(32) }), category: "stale_snapshot" },
      { authorize: async (server: string, scope: "read" | "upload") => ({ approvedServer: server + "/connect", scope, token: "x", generation }), category: "auth_required" },
      { authorize: async (server: string) => ({ approvedServer: server, scope: "upload" as const, token: "x", generation }), category: "auth_required" },
      { authorize: async (server: string, scope: "read" | "upload") => ({ approvedServer: server, scope, token: "bad token", generation }), category: "auth_required" },
    ];
    for (const item of invalidAuthority) await assert.rejects(readAccountStatus({ ...options(base), authorize: item.authorize }), { category: item.category });
    await assert.rejects(readAccountStatus({ ...options(base), generation: "invalid" }), { category: "invalid_request" });
    assert.equal(seen.requests, 0);
  });
});

test("strictly rejects malformed, duplicate, unsafe, negative, and unknown status values", async () => {
  const bodies = ["null", "[]", "{}", '{"beta":"active","beta":"waitlist","quotaBytes":1,"usedBytes":0}',
    '{"beta":"active","quotaBytes":9007199254740992,"usedBytes":0}', '{"beta":"active","quotaBytes":-1,"usedBytes":0}',
    '{"beta":"other","quotaBytes":1,"usedBytes":0}', '{"beta":"active","quotaBytes":1}'];
  for (const body of bodies) await withServer((_req, res) => { res.writeHead(200).end(body); }, async base => {
    await assert.rejects(readAccountStatus(options(base)), { category: "invalid_response" });
  });
});

test("projects redirect, HTTP, cancellation, oversized body, and stale activity failures", async () => {
  await withServer((_req, res) => { res.writeHead(302, { location: "https://elsewhere.invalid" }).end("secret"); }, async base => assert.rejects(readAccountStatus(options(base)), { category: "redirect_refused" }));
  await withServer((_req, res) => { res.writeHead(500).end("server secret"); }, async base => assert.rejects(readAccountStatus(options(base)), error => error instanceof Error && (error as Error & { category?: string }).category === "request_failed" && !error.message.includes("secret")));
  await withServer((_req, res) => { res.writeHead(200).end("x".repeat(2_000_001)); }, async base => assert.rejects(readAccountStatus(options(base)), { category: "response_too_large" }));
  const abort = new AbortController();
  await withServer((_req, res) => { res.writeHead(200); res.write('{"beta":'); abort.abort(); }, async base => assert.rejects(readAccountStatus({ ...options(base), signal: abort.signal }), { category: "cancelled" }));
  let active = true;
  await withServer((_req, res) => { respond(res, { beta: "active", quotaBytes: 1, usedBytes: 1 }); active = false; }, async base => assert.rejects(readAccountStatus(options(base, "read", () => active)), { category: "stale_snapshot" }));
});
