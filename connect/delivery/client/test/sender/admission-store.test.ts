import assert from "node:assert/strict";
import test from "node:test";
import { lstat, mkdtemp, mkdir, readFile, realpath, rename, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { admitQueuedUpload, type AdmissionStoreResources } from "../../src/queue/admission-store.ts";
import { decodeJob, encodeJob } from "../../src/queue/job.ts";

const server = "https://connect.example/connect", generation = "b".repeat(32), id = "a".repeat(32);
async function fixture() {
  const base = await realpath(await mkdtemp(path.join(os.tmpdir(), "connect-admission-"))), root = path.join(base, "state"), bundle = path.join(base, "clip.apparent");
  await mkdir(root, { mode: 0o700 }); await mkdir(bundle, { mode: 0o755 }); await writeFile(path.join(bundle, "apparent.json"), "{}", { mode: 0o644 });
  await writeFile(path.join(root, "upload.credential"), JSON.stringify({ server, scope: "upload", token: "synthetic-token", generation }), { mode: 0o600 });
  const held = new Set<string>(); let writes = 0, uncertain = false;
  const resources: AdmissionStoreResources = {
    async checkAcl(h) { await h.close(); return { state: "accepted" }; },
    async lock(handle, { signal }) { const key = String((await handle.stat()).ino); await handle.close(); if (signal?.aborted) return { state: "cancelled" }; if (held.has(key)) return { state: "busy" }; held.add(key);
      return { state: "acquired", lease: { signal: signal ?? new AbortController().signal, assertHeld() { assert.ok(held.has(key)); }, async release() { held.delete(key); } } }; },
    async replaceFile(parent, source, destination, expected) { writes++; try { const current = await lstat(path.join(root, destination), { bigint: true }).catch(e => (e as NodeJS.ErrnoException).code === "ENOENT" ? null : undefined);
      if (expected.destination === null ? current !== null : !current || String(current.dev) !== expected.destination.device || String(current.ino) !== expected.destination.inode) return { state: "destination_changed" };
      await rename(path.join(root, source), path.join(root, destination)); return { state: uncertain ? "publication_uncertain" : "replaced" }; } finally { await parent.close(); } },
  };
  const settings = { enabled: true, server, encryptionEnabled: false, sharingEnabled: true, nowSeconds: 100, retentionSeconds: 60 };
  const input = { directory: root, approvedServer: server, bundle, intent: "automatic-private" as const, candidateId: id,
    readSettings: async () => settings };
  return { root, bundle, resources, input, writes: () => writes, setUncertain(v: boolean) { uncertain = v; },
    async close() { assert.equal(held.size, 0); await rm(base, { recursive: true, force: true }); } };
}
const row = (bundle: string) => decodeJob(Buffer.from(JSON.stringify({ id, bundle, server, generation, created: 70, attempts: 2, nextAttempt: 4, status: "pending", error: null, files: [], shared: false, publicCopy: false, encrypted: false, encryption: null, grouped: false, unknownField: { retained: true } })));

test("reuses an ordered matching job without effects", async () => {
  const f = await fixture();
  try {
    const existing = "c".repeat(32), reusable = { ...row(await realpath(f.bundle)), id: existing };
    await writeFile(path.join(f.root, `${existing}.job`), encodeJob(reusable), { mode: 0o600 });
    const result = await admitQueuedUpload(f.input, f.resources);
    assert.deepEqual(result, { state: "reusable", id: existing, cancelledJobIds: [], retiredJobIds: [] });
    assert.equal(f.writes(), 0);
  } finally { await f.close(); }
});

test("publishes a new job by expected-absent replacement and preserves a bundle alias", async () => {
  const f = await fixture();
  try {
    f.input.bundle = path.join(path.dirname(f.bundle), ".", path.basename(f.bundle));
    const result = await admitQueuedUpload(f.input, f.resources);
    assert.equal(result.state, "admitted"); assert.equal(f.writes(), 1);
    const saved = JSON.parse(await readFile(path.join(f.root, `${id}.job`), "utf8"));
    assert.equal(saved.bundle, await realpath(f.bundle)); assert.equal(saved.generation, generation); assert.equal(saved.status, "pending");
  } finally { await f.close(); }
});

test("explicit private admission cancels an earlier matching row before publishing", async () => {
  const f = await fixture();
  try {
    const previous = "c".repeat(32);
    await writeFile(path.join(f.root, `${previous}.job`), encodeJob({ ...row(await realpath(f.bundle)), id: previous, shared: true }), { mode: 0o600 });
    f.input.intent = "explicit-private";
    const result = await admitQueuedUpload(f.input, f.resources);
    assert.equal(result.state, "admitted");
    assert.deepEqual(result.cancelledJobIds, [previous]);
    const saved = JSON.parse(await readFile(path.join(f.root, `${previous}.job`), "utf8"));
    assert.equal(saved.status, "cancelled"); assert.deepEqual(saved.unknownField, { retained: true });
    assert.equal(f.writes(), 2);
  } finally { await f.close(); }
});

test("refuses candidate collisions and completed bundles without modifying inputs", async () => {
  const f = await fixture();
  try {
    await mkdir(path.join(f.root, `${id}.sealed`));
    assert.equal((await admitQueuedUpload(f.input, f.resources)).state, "refused"); assert.equal(f.writes(), 0);
    await rm(path.join(f.root, `${id}.sealed`), { recursive: true });
    await rm(path.join(f.bundle, "apparent.json"));
    assert.equal((await admitQueuedUpload(f.input, f.resources)).state, "refused"); assert.equal(f.writes(), 0);
  } finally { await f.close(); }
});

test("awaited settings, source and candidate changes fence publication", async () => {
  for (const change of ["disable", "source", "candidate"] as const) {
    const f = await fixture(); let reads = 0;
    const policy = { enabled: true, server, encryptionEnabled: false, sharingEnabled: true, nowSeconds: 100, retentionSeconds: 60 };
    f.input.readSettings = async () => {
      if (++reads === 3) {
        if (change === "source") await writeFile(path.join(f.bundle, "apparent.json"), "{\"changed\":true}");
        if (change === "candidate") await mkdir(path.join(f.root, `${id}.sealed`));
      }
      return change === "disable" && reads >= 3 ? { ...policy, enabled: false } : policy;
    };
    try {
      assert.equal((await admitQueuedUpload(f.input, f.resources)).state, "refused", change);
      assert.equal(f.writes(), 0, change);
    } finally { await f.close(); }
  }
});

test("an uncertain create preserves observed bytes and never reports admission", async () => {
  const f = await fixture(); f.setUncertain(true);
  try {
    const result = await admitQueuedUpload(f.input, f.resources);
    assert.equal(result.state, "uncertain"); assert.equal(f.writes(), 1);
    assert.equal(JSON.parse(await readFile(path.join(f.root, `${id}.job`), "utf8")).id, id);
  } finally { await f.close(); }
});

test("public copy keeps its awaiting-share donor shape", async () => {
  const f = await fixture();
  try {
    f.input.intent = "public-copy";
    const result = await admitQueuedUpload(f.input, f.resources);
    assert.equal(result.state, "admitted");
    const saved = JSON.parse(await readFile(path.join(f.root, `${id}.job`), "utf8"));
    assert.equal(saved.status, "awaiting_share"); assert.equal(saved.shared, true); assert.equal(saved.publicCopy, true); assert.equal(saved.encrypted, false);
  } finally { await f.close(); }
});

test("retention equality is inclusive and stale plain jobs retire through the donor helper", async () => {
  const f = await fixture();
  try {
    const inclusive = "c".repeat(32), expired = "d".repeat(32);
    await writeFile(path.join(f.root, `${inclusive}.job`), encodeJob({ ...row("/tmp/other.apparent"), id: inclusive, created: 40 }), { mode: 0o600 });
    await writeFile(path.join(f.root, `${expired}.job`), encodeJob({ ...row("/tmp/other.apparent"), id: expired, created: 39 }), { mode: 0o600 });
    const result = await admitQueuedUpload(f.input, f.resources);
    assert.equal(result.state, "admitted"); assert.deepEqual(result.retiredJobIds, [expired]);
    await assert.rejects(readFile(path.join(f.root, `${expired}.job`)), { code: "ENOENT" });
    assert.equal(JSON.parse(await readFile(path.join(f.root, `${inclusive}.job`), "utf8")).created, 40);
  } finally { await f.close(); }
});
