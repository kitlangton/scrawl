import assert from "node:assert/strict";
import { test } from "node:test";
import { createServer } from "node:http";
import { once } from "node:events";
import { createHash } from "node:crypto";
import * as ed from "@noble/ed25519";
import { generateAgeIdentity, encodeArchive, encryptAge, splitAgeStream, signEnvelope } from "../../src/recipient/crypto.ts";
import { parseRecipientCommand, runRecipientCommand } from "../../src/cli/recipient.ts";
const hash = (b: Uint8Array) => createHash("sha256").update(b).digest("hex");
const bytes = (s: string) => new TextEncoder().encode(s);

test("headless fetch and remote inspect retrieve authenticated groups before committed publication", async () => {
  const id = "a".repeat(32), routes = new Map<string, unknown>(), hits: string[] = [];
  const http = createServer((req, res) => {
    assert.equal(req.headers.authorization, "Bearer synthetic"); assert.equal(req.headers.cookie, undefined);
    hits.push(req.url!); const body = routes.get(req.url!);
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
  } finally { http.close(); http.closeAllConnections(); await once(http, "close"); }
});
