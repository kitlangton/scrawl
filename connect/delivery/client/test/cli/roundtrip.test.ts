import assert from "node:assert/strict";
import { test } from "node:test";
import { createServer } from "node:http";
import { mkdtemp, mkdir, readFile, realpath, rename, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import { tmpdir } from "node:os";
import { runRecipientCommandFromState } from "../../src/cli/recipient-state.ts";
import type { StateRecipientOptions } from "../../src/cli/recipient-state.ts";
import { once } from "node:events";
import { createHash } from "node:crypto";
import * as ed from "@noble/ed25519";
import { generateAgeIdentity, encodeArchive, encryptAge, splitAgeStream, signEnvelope } from "../../src/recipient/crypto.ts";
import { parseRecipientCommand, runRecipientCommand } from "../../src/cli/recipient.ts";
const hash = (b: Uint8Array) => createHash("sha256").update(b).digest("hex");
const bytes = (s: string) => new TextEncoder().encode(s);

test("headless fetch and remote inspect retrieve authenticated groups before committed publication", async () => {
  const id = "a".repeat(32), routes = new Map<string, unknown>(), hits: string[] = [];
  let expectedToken = "synthetic", denyNextCapture: (() => Promise<void>) | undefined;
  let beforeResponse: ((url: string) => Promise<void>) | undefined;
  const http = createServer(async (req, res) => {
    assert.equal(req.method, "GET"); assert.equal(req.headers.authorization, `Bearer ${expectedToken}`); assert.equal(req.headers.cookie, undefined);
    hits.push(req.url!);
    if (denyNextCapture && req.url === `/v1/captures/${id}`) { const update = denyNextCapture; denyNextCapture = undefined; await update(); res.writeHead(401).end(); return; }
    await beforeResponse?.(req.url!); const body = routes.get(req.url!);
    if (body === undefined) { res.writeHead(404).end(); return; }
    res.end(body instanceof Uint8Array ? body : JSON.stringify(body));
  });
  http.listen(0, "127.0.0.1"); await once(http, "listening");
  try {
    const origin = `http://127.0.0.1:${(http.address() as { port: number }).port}`;
    const reader = await generateAgeIdentity(), sender = await generateAgeIdentity();
    const ownSeed = new Uint8Array(32).fill(12), senderSeed = new Uint8Array(32).fill(13);
    const signing = async (seed: Uint8Array) => Buffer.from(await ed.getPublicKeyAsync(seed)).toString("hex");
    routes.set("/v1/encryption/peers", { accountId: "account", grantId: "reader", scope: "read", devices: [
      { grantId: "reader", scope: "read", key: { version: 1, recipient: reader.recipient, signing: await signing(ownSeed) } },
      { grantId: "sender", scope: "upload", key: { version: 1, recipient: sender.recipient, signing: await signing(senderSeed) } },
    ] });
    const transport = [];
    for (const group of ["evidence", "media"] as const) {
      const context = { origin, accountId: "account", captureId: id, senderGrantId: "sender", group };
      const contents = group === "evidence" ? [
        { path: "apparent.json", bytes: bytes("{}") }, { path: "summary.md", bytes: bytes("synthetic evidence") }, { path: "contact-sheet.png", bytes: Uint8Array.of(1) },
      ] : [{ path: "recording.mp4", bytes: Uint8Array.of(2, 3) }];
      const files = contents.map(f => ({ path: f.path, size: f.bytes.length, sha256: hash(f.bytes) }));
      const parts = splitAgeStream(await encryptAge(encodeArchive({ version: 1, context, files }, contents.map(f => f.bytes)), [reader.recipient]));
      const chunks = parts.map((part, i) => ({ path: `sealed/part-${String(i).padStart(6, "0")}.age`, size: part.length, sha256: hash(part) }));
      const signed = { version: 1, context, chunks }, envelope = bytes(JSON.stringify({ signed, signature: await signEnvelope(senderSeed, signed) }));
      for (const f of [{ path: "envelope.json", bytes: envelope }, ...parts.map((part, i) => ({ path: chunks[i]!.path, bytes: part }))]) {
        const path = `${group}/${f.path}`; routes.set(`/v1/captures/${id}/files/${path}`, f.bytes);
        transport.push({ path, size: f.bytes.length, sha256: hash(f.bytes) });
      }
    }
    routes.set(`/v1/captures/${id}`, { id, status: "ready", format: "apparent-encrypted-v1", delivery: "evidence-first-v1", evidenceReady: true, files: transport });
    let publications = 0;
    const dependencies = {
      loadCredential: async () => ({ server: origin, token: "synthetic", grantId: "reader", identity: { age: reader.identity, signing: ownSeed } }),
      inspectLocal: async (source: string) => source,
      publish: async (capture: { evidenceOnly: boolean; files: Array<{ path: string; bytes: Uint8Array }> }, path: string) => {
        publications++; assert.equal(capture.files.length, capture.evidenceOnly ? 3 : 4);
        assert.equal(new TextDecoder().decode(capture.files.find(f => f.path === "summary.md")!.bytes), "synthetic evidence");
        return { state: "committed" as const, path, evidenceOnlyReceipt: capture.evidenceOnly };
      },
    };
    assert.deepEqual(await runRecipientCommand(parseRecipientCommand(["inspect", id]), dependencies), { ok: true, path: `./${id}.evidence.apparent` });
    assert(!hits.some(path => path.includes("/files/media/")));
    assert.deepEqual(await runRecipientCommand(parseRecipientCommand(["fetch", id, "--out", "./full", "--media"]), dependencies), { ok: true, path: "./full" });
    assert(hits.some(path => path.includes("/files/media/"))); assert.equal(publications, 2);
    for (const outcome of ["committed", "uncertain", "failed"] as const) {
      let borrowed: Uint8Array[] = [];
      const destination = `./cleanup-${outcome}`;
      const result = await runRecipientCommand(parseRecipientCommand(["fetch", id, "--out", destination]), {
        ...dependencies,
        async publish(capture) {
          borrowed = capture.files.map(file => file.bytes);
          assert.equal(new TextDecoder().decode(capture.files.find(file => file.path === "summary.md")!.bytes), "synthetic evidence");
          await Promise.resolve();
          assert(borrowed.every(buffer => buffer.some(byte => byte !== 0)), "publisher can use borrowed bytes until its promise settles");
          if (outcome === "failed") { capture.files.length = 0; throw new Error("synthetic private publisher diagnostic"); }
          return outcome === "committed" ? { state: "committed", path: destination } : { state: "publication-uncertain" };
        },
      });
      assert.equal(result.ok, outcome === "committed");
      if (!result.ok) assert.equal(result.error.category, outcome === "failed" ? "operation_failed" : "publication_uncertain");
      assert.equal(borrowed.length, 4);
      assert(borrowed.every(buffer => buffer.every(byte => byte === 0)), `owned plaintext remains after ${outcome}`);
      assert.deepEqual(ownSeed, new Uint8Array(32).fill(12), "borrowed caller identity remains unchanged");
    }
    const root = await mkdtemp(path.join(await realpath(tmpdir()), "connect-state-command-")), state = path.join(root, "state");
    try {
      await mkdir(state, { mode: 0o700 });
      const identityId = "c".repeat(32);
      await writeFile(path.join(state, "read.credential"), JSON.stringify({ server: origin, scope: "read", token: "synthetic", generation: "b".repeat(32), grantId: "reader",
        refreshToken: "refresh", expiresAt: 200, refreshExpiresAt: 400, encryption_identity: identityId }), { mode: 0o600 });
      await writeFile(path.join(state, `${identityId}.identity`), JSON.stringify({ version: 1, age: reader.identity, signing: Buffer.from(ownSeed).toString("hex") }), { mode: 0o600 });
      const resources: StateRecipientOptions["resources"] = {
        checkAcl: async h => { await h.close(); return { state: "accepted" }; },
        lock: async h => { await h.close(); let held = true; return { state: "acquired", lease: { signal: new AbortController().signal, assertHeld() { if (!held) throw Error(); }, async release() { held = false; } } }; },
        publish: async (h, source, destination) => { try { await rename(path.join(root, source), path.join(root, destination)); return { state: "published" }; } finally { await h.close(); } },
      };
      const options = { directory: state, approvedServer: origin, resources, inspectLocal: async (source: string) => source, nowSeconds: () => 100 };
      const destination = path.join(root, "state-backed.apparent");
      assert.deepEqual(await runRecipientCommandFromState(parseRecipientCommand(["fetch", id, "--out", destination]), options), { ok: true, path: destination });
      assert.equal(await readFile(path.join(destination, "summary.md"), "utf8"), "synthetic evidence");
      let peers = 0;
      beforeResponse = async url => { if (url === "/v1/encryption/peers" && ++peers === 2) await writeFile(path.join(state, "cancellation"), JSON.stringify("new epoch"), { mode: 0o600 }); };
      const refused = await runRecipientCommandFromState(parseRecipientCommand(["fetch", id, "--out", path.join(root, "cancelled.apparent")]), options);
      assert.deepEqual(refused, { ok: false, error: { category: "stale_snapshot", message: "stale_snapshot" } });
      await assert.rejects(readFile(path.join(root, "cancelled.apparent", "summary.md")));
      beforeResponse = undefined;
      const stored = JSON.parse(await readFile(path.join(state, "read.credential"), "utf8"));
      denyNextCapture = async () => {
        expectedToken = "writer-rotated";
        await writeFile(path.join(state, "read.credential"), JSON.stringify({ ...stored, token: expectedToken }), { mode: 0o600 });
      };
      let prior = hits.filter(url => url === `/v1/captures/${id}`).length;
      const rotatedOut = path.join(root, "rotated.apparent");
      assert.deepEqual(await runRecipientCommandFromState(parseRecipientCommand(["fetch", id, "--out", rotatedOut]), options), { ok: true, path: rotatedOut });
      assert.equal(hits.filter(url => url === `/v1/captures/${id}`).length - prior, 2);
      denyNextCapture = async () => {};
      prior = hits.filter(url => url === `/v1/captures/${id}`).length;
      assert.deepEqual(await runRecipientCommandFromState(parseRecipientCommand(["fetch", id, "--out", path.join(root, "unchanged.apparent")]), options),
        { ok: false, error: { category: "renewal_required", message: "renewal_required" } });
      assert.equal(hits.filter(url => url === `/v1/captures/${id}`).length - prior, 1);

    } finally { beforeResponse = undefined; await rm(root, { recursive: true, force: true }); }

  } finally { http.close(); http.closeAllConnections(); await once(http, "close"); }
});
