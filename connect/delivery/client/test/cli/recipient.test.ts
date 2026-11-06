import { test } from "node:test";
import assert from "node:assert/strict";
import { createServer } from "node:http";
import { parseRecipientCommand, publishVerified, runRecipientCommand } from "../../src/cli/recipient.ts";
import type { RecipientCliDependencies } from "../../src/cli/recipient.ts";

const id = "0123456789abcdef0123456789abcdef";
const deps = (patch: Partial<RecipientCliDependencies> = {}): RecipientCliDependencies => ({
  loadCredential: async () => ({ server: "http://127.0.0.1:1", token: "safe-test-token", grantId: "grant" }),
  publish: async (_capture, path) => ({ state: "committed", path }),
  inspectLocal: async source => source,
  ...patch,
});

test("parses fetch contracts and rejects malformed input", () => {
  assert.deepEqual(parseRecipientCommand(["fetch", id, "--media", "--out", "./full", "--wait", "3"]),
    { kind: "fetch", id, out: "./full", waitSeconds: 3, media: true });
  for (const args of [["fetch", "UPPER", "--out", "x"], ["fetch", id, "--out", "x", "--wait", "901"], ["fetch", id, "--out"], ["fetch", id]]) {
    assert.throws(() => parseRecipientCommand(args), { category: "invalid_arguments" });
  }
});

test("only a bare lowercase ID selects remote inspect with evidence defaults", () => {
  assert.deepEqual(parseRecipientCommand(["inspect", id]), { kind: "inspect-remote", id, out: `./${id}.evidence.apparent`, waitSeconds: 120 });
  for (const source of [`./${id}`, `/${id}`, `${id}.apparent`, id.toUpperCase(), "latest"]) {
    assert.deepEqual(parseRecipientCommand(["inspect", source]), { kind: "inspect-local", source });
  }
});

test("local inspect performs no credential, publication, or network work", async () => {
  let touched = false;
  const result = await runRecipientCommand(parseRecipientCommand(["inspect", "./bundle.apparent"]), deps({
    loadCredential: async () => { touched = true; throw Error(); },
    publish: async () => { touched = true; throw Error(); },
    inspectLocal: async source => source,
  }));
  assert.deepEqual(result, { ok: true, path: "./bundle.apparent" });
  assert.equal(touched, false);
});

test("retrieval uses injected credential and category-only failures never project success", async () => {
  const server = createServer((_request, response) => { response.writeHead(404, { "content-type": "application/json" }); response.end("{}"); });
  await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
  const address = server.address(); assert(address && typeof address !== "string");
  let published = false;
  try {
    const result = await runRecipientCommand({ kind: "fetch", id, out: "./out", waitSeconds: 0, media: true }, deps({
      loadCredential: async () => ({ server: `http://127.0.0.1:${address.port}`, token: "safe-test-token", grantId: "read-grant" }),
      publish: async () => { published = true; return { state: "committed", path: "./out" }; },
    }));
    assert.deepEqual(result, { ok: false, error: { category: "not_found", message: "not_found" } });
    assert.equal(published, false);
  } finally { await new Promise<void>((resolve, reject) => server.close(error => error ? reject(error) : resolve())); }
});

test("publication uncertainty and missing evidence receipt never project success", async () => {
  const capture = { captureId: id, server: "https://connect.example", evidenceOnly: true, files: [] };
  let calls = 0;
  const uncertain = await publishVerified(capture, "./out", async () => { calls++; return { state: "publication-uncertain" }; });
  assert.deepEqual(uncertain, { ok: false, error: { category: "publication_uncertain", message: "publication outcome is uncertain; inspect the destination before taking further action" } });
  const missingReceipt = await publishVerified(capture, "./out", async () => ({ state: "committed", path: "./out" }));
  assert.deepEqual(missingReceipt, { ok: false, error: { category: "publication_unverified", message: "evidence-only publication lacks its required receipt" },
    publication: { state: "committed", path: "./out" } });
  assert.equal(calls, 1);
});

test("a publisher cannot change the required evidence-only receipt by mutating its input", async () => {
  const capture = { captureId: id, server: "https://connect.example", evidenceOnly: true, files: [] };
  const result = await publishVerified(capture, "./out", async value => {
    value.evidenceOnly = false;
    return { state: "committed", path: "./out" };
  });
  assert.equal(result.ok, false);
  if (!result.ok) assert.equal(result.error.category, "publication_unverified");
});

test("duplicate wait flags are rejected even when the first value is zero", () => {
  assert.throws(() => parseRecipientCommand(["fetch", id, "--out", "x", "--wait", "0", "--wait", "1"]), { category: "invalid_arguments" });
});
test("cancellation before and during credential loading prevents subsequent effects", async () => {
  const c = new AbortController(); c.abort(); let calls = 0;
  const command = parseRecipientCommand(["fetch", id, "--out", "x"]);
  const before = await runRecipientCommand(command, deps({ cancelled: c.signal, loadCredential: async () => { calls++; throw Error(); } }));
  assert.equal(before.ok, false); assert.equal(calls, 0);
  const during = new AbortController();
  const after = await runRecipientCommand(command, deps({ cancelled: during.signal, loadCredential: async () => { during.abort(); return { server: "http://127.0.0.1:1", token: "synthetic", grantId: "reader" }; }, publish: async () => { calls++; throw Error(); } }));
  assert.deepEqual(after, { ok: false, error: { category: "cancelled", message: "cancelled" } }); assert.equal(calls, 0);
});


test("recipient command intent is pinned before waiting for credentials", async () => {
  let requested = "";
  const server = createServer((request, response) => { requested = request.url!; response.writeHead(404).end(); });
  await new Promise<void>(resolve => server.listen(0, "127.0.0.1", resolve));
  const address = server.address(); assert(address && typeof address !== "string");
  const command = parseRecipientCommand(["fetch", id, "--out", "./original"]);
  assert.equal(command.kind, "fetch"); if (command.kind !== "fetch") throw Error();
  try {
    await runRecipientCommand(command, deps({ loadCredential: async () => {
      command.id = "b".repeat(32); command.out = "./changed";
      return { server: `http://127.0.0.1:${address.port}`, token: "synthetic", grantId: "reader" };
    } }));
    assert.equal(requested, `/v1/captures/${id}`);
  } finally { server.closeAllConnections(); await new Promise<void>(resolve => server.close(() => resolve())); }
});
