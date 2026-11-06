import assert from "node:assert/strict";
import test from "node:test";
import { createServer } from "node:http";
import { once } from "node:events";
import { createHash } from "node:crypto";
import { mkdtemp, mkdir, readFile, realpath, rename, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { parseRecipientCommand, RecipientCommandError, runRecipientCommand } from "../../src/cli/recipient.ts";
import { runRecipientCommandFromState, type StateRecipientOptions } from "../../src/cli/recipient-state.ts";

test("state-backed local inspection never admits state or invokes a resource", async () => {
  const fail = async () => { throw new Error("local inspection touched private state"); };
  const result = await runRecipientCommandFromState(parseRecipientCommand(["inspect", "./local.apparent"]), {
    directory: "/not-read", approvedServer: "not-a-url", resources: { checkAcl: fail, lock: fail, publish: fail }, inspectLocal: async source => source,
  });
  assert.deepEqual(result, { ok: true, path: "./local.apparent" });
});

test("command adapters cannot project arbitrary diagnostic strings as a failure category", async () => {
  const failure = new RecipientCommandError("private diagnostic" as never);
  Reflect.set(failure, "category", "another private diagnostic");
  const result = await runRecipientCommand(parseRecipientCommand(["fetch", "a".repeat(32), "--out", "./out"]), {
    loadCredential: async () => { throw failure; }, publish: async () => { throw Error(); }, inspectLocal: async source => source,
  });
  assert.deepEqual(result, { ok: false, error: { category: "operation_failed", message: "operation_failed" } });
});

test("explicit legacy plaintext reads skip identity loading while the default refuses downgrade", async () => {
  const id = "a".repeat(32), files = new Map([
    ["summary.md", Buffer.from("synthetic legacy recording")], ["apparent.json", Buffer.from("{}")], ["contact-sheet.png", Buffer.from([1, 2, 3])],
  ]);
  const manifest = { id, status: "ready", format: "plaintext-v1", files: [...files].map(([path, bytes]) => ({ path, size: bytes.length, sha256: createHash("sha256").update(bytes).digest("hex") })) };
  const server = createServer((request, response) => {
    assert.equal(request.headers.authorization, "Bearer legacy");
    if (request.url === `/v1/captures/${id}`) response.end(JSON.stringify(manifest));
    else { const bytes = files.get(request.url!.slice(`/v1/captures/${id}/files/`.length)); if (bytes) response.end(bytes); else response.writeHead(404).end(); }
  });
  server.listen(0, "127.0.0.1"); await once(server, "listening");
  const root = await mkdtemp(path.join(await realpath(tmpdir()), "connect-legacy-command-")), state = path.join(root, "state");
  try {
    const origin = `http://127.0.0.1:${(server.address() as { port: number }).port}`;
    await mkdir(state, { mode: 0o700 });
    await writeFile(path.join(state, "read.credential"), JSON.stringify({ server: origin, scope: "read", token: "legacy", generation: "c".repeat(32) }), { mode: 0o600 });
    const resources: StateRecipientOptions["resources"] = {
      checkAcl: async h => { await h.close(); return { state: "accepted" }; },
      lock: async h => { await h.close(); let held = true; return { state: "acquired", lease: { signal: new AbortController().signal, assertHeld() { if (!held) throw Error(); }, async release() { held = false; } } }; },
      publish: async (h, source, destination) => { try { await rename(path.join(root, source), path.join(root, destination)); return { state: "published" }; } finally { await h.close(); } },
    };
    const options = { directory: state, approvedServer: origin, resources, inspectLocal: async (source: string) => source };
    const command = parseRecipientCommand(["fetch", id, "--out", path.join(root, "legacy.apparent")]);
    assert.deepEqual(await runRecipientCommandFromState(command, options), { ok: false, error: { category: "encryption_downgrade_refused", message: "encryption_downgrade_refused" } });
    assert.deepEqual(await runRecipientCommandFromState(command, { ...options, allowLegacyPlaintext: true }), { ok: true, path: path.join(root, "legacy.apparent") });
    assert.equal(await readFile(path.join(root, "legacy.apparent", "summary.md"), "utf8"), "synthetic legacy recording");
  } finally { server.closeAllConnections(); server.close(); await once(server, "close"); await rm(root, { recursive: true, force: true }); }
});
