import assert from "node:assert/strict";
import test from "node:test";
import { lstat, mkdtemp, readFile, realpath, rename, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { decodeJob, encodeJob, type Job } from "../../src/queue/job.ts";
import { retryQueuedUploads, type RetryInput, type RetryResources } from "../../src/queue/retry.ts";

const server = "https://connect.example/connect", generation = "b".repeat(32), id = "a".repeat(32);
const base = (i: string, extra: Partial<Job> = {}) => decodeJob(Buffer.from(JSON.stringify({ id: i, bundle: "/private/synthetic.apparent", server, generation, created: 10,
  attempts: 4, nextAttempt: 500, status: "failed", error: "upload_failed", sizeFailure: { path: "x", size: 8, limit: 4 }, files: [{ path: "x", size: 3, sha256: "d".repeat(64) },],
  shared: true, publicCopy: false, encrypted: false, encryption: null, grouped: true, mystery: { retained: true }, ...extra })));
async function fixture(rows: Job[] = [base(id)]) {
  const root = await realpath(await mkdtemp(path.join(os.tmpdir(), "connect-retry-"))), held = new Set<string>(); let writes = 0, releaseFailure = false;
  for (const job of rows) await put(job);
  async function put(job: Job) { await writeFile(path.join(root, `${job.id}.job`), encodeJob(job), { mode: 0o600 }); }
  await writeFile(path.join(root, "upload.credential"), JSON.stringify({ server, scope: "upload", token: "synthetic-token", generation }), { mode: 0o600 });
  const settings = { enabled: true, server, encryptionEnabled: false, sharingEnabled: true, nowSeconds: 100, retentionSeconds: 90 };
  const resources: RetryResources = {
    async checkAcl(h) { await h.close(); return { state: "accepted" }; },
    async lock(handle, { signal }) { const key = String((await handle.stat()).ino); await handle.close(); if (signal?.aborted) return { state: "cancelled" }; if (held.has(key)) return { state: "busy" }; held.add(key);
      return { state: "acquired", lease: { signal: signal ?? new AbortController().signal, assertHeld() { assert.ok(held.has(key)); }, async release() { held.delete(key); if (releaseFailure) throw Error("secret-release"); } } }; },
    async replaceFile(parent, source, destination, expected) { writes++; try { const lock = await lstat(path.join(root, "queue.lock")); assert.ok(held.has(String(lock.ino)), "replacement remains inside queue lease"); const current = await lstat(path.join(root, destination), { bigint: true });
      assert.deepEqual({ device: String(current.dev), inode: String(current.ino) }, expected.destination); await rename(path.join(root, source), path.join(root, destination)); return { state: "replaced" }; }
      finally { await parent.close(); } },
  };
  const input: RetryInput = { directory: root, approvedServer: server, readSettings: async () => settings };
  return { root, rows, settings, resources, input, put, writes: () => writes, setReleaseFailure(v: boolean) { releaseFailure = v; },
    async read(jobId = id) { return decodeJob(await readFile(path.join(root, `${jobId}.job`))); },
    async close() { assert.equal(held.size, 0); await rm(root, { recursive: true, force: true }); } };
}

test("resets eligible pending and failed jobs while preserving fields, unknown data and shared intent", async () => {
  const f = await fixture([base(id), base("c".repeat(32), { status: "pending", attempts: 2 })]);
  try {
    const before = await f.read(); const result = await retryQueuedUploads(f.input, f.resources);
    assert.deepEqual(result, { state: "complete", changedJobIds: [id, "c".repeat(32)] });
    assert.ok(Object.isFrozen(result) && Object.isFrozen(result.changedJobIds));
    const after = await f.read(); assert.equal(after.status, "pending"); assert.equal(after.attempts, 0); assert.equal(after.nextAttempt, 0);
    assert.equal(after.error, null); assert.equal("sizeFailure" in after, false); assert.deepEqual(after.files, before.files);
    assert.deepEqual(after.mystery, before.mystery); assert.equal(after.shared, true); assert.equal(after.bundle, before.bundle);
  } finally { await f.close(); }
});

test("skips expired, stale-generation, wrong-server, incompatible mode, public-sharing and terminal rows", async () => {
  const rows = [base(id), base("c".repeat(32), { created: 9 }), base("d".repeat(32), { generation: "c".repeat(32) }),
    base("e".repeat(32), { server: "https://other.example" }), base("f".repeat(32), { encrypted: true }),
    base("1".repeat(32), { publicCopy: true, shared: true }), base("2".repeat(32), { status: "ready" })];
  const f = await fixture(rows);
  f.settings.sharingEnabled = false;
  try { const result = await retryQueuedUploads(f.input, f.resources); assert.deepEqual(result.changedJobIds, [id]); assert.equal(f.writes(), 1); }
  finally { await f.close(); }
});

test("disabled settings reject before writes and malformed queue state is not treated as empty", async () => {
  const f = await fixture(); try { f.settings.enabled = false;
    await assert.rejects(retryQueuedUploads(f.input, f.resources), e => e.category === "invalid_settings"); assert.equal(f.writes(), 0);
    f.settings.enabled = true; await writeFile(path.join(f.root, `${"9".repeat(32)}.job`), "{");
    await assert.rejects(retryQueuedUploads(f.input, f.resources), e => e.category === "state_unavailable");
  } finally { await f.close(); }
});

test("replacement acknowledgement uncertainty stops later jobs and preserves observed committed IDs", async () => {
  const f = await fixture([base(id), base("c".repeat(32))]);
  const replace = f.resources.replaceFile;
  f.resources.replaceFile = async (parent, source, destination, expected, options) => { const result = await replace(parent, source, destination, expected, options); return { state: "publication_uncertain" }; };
  try { const result = await retryQueuedUploads(f.input, f.resources); assert.equal(result.state, "uncertain"); assert.deepEqual(result.changedJobIds, []); assert.equal(f.writes(), 1); }
  finally { await f.close(); }
});

test("reports post-commit lease release failure as uncertain with observed IDs", async () => {
  const f = await fixture(); f.setReleaseFailure(true);
  try { const result = await retryQueuedUploads(f.input, f.resources); assert.equal(result.state, "uncertain"); assert.deepEqual(result.changedJobIds, [id]); }
  finally { f.setReleaseFailure(false); await f.close(); }
});

test("awaited settings boundary fences cancellation, revocation, login, job and destination changes", async () => {
  for (const change of ["cancel", "revocation", "login", "job", "destination"] as const) {
    const f = await fixture(); const abort = new AbortController(); let calls = 0;
    f.input.readSettings = async signal => {
      calls++;
      if (calls === 3) {
        if (change === "cancel") abort.abort();
        if (change === "revocation") await writeFile(path.join(f.root, "revocation-test.intent"), "{}", { mode: 0o600 });
        if (change === "login") await writeFile(path.join(f.root, "upload.credential"), JSON.stringify({ server, scope: "upload", token: "new-token", generation: "c".repeat(32) }), { mode: 0o600 });
        if (change === "job") await f.put(base(id, { status: "cancelled" }));
        if (change === "destination") { const jobPath = path.join(f.root, `${id}.job`); await writeFile(jobPath + ".replacement", encodeJob(base(id)), { mode: 0o600 }); await rename(jobPath + ".replacement", jobPath); }
      }
      return f.settings;
    };
    f.input.signal = abort.signal;
    try {
      const result = await retryQueuedUploads(f.input, f.resources);
      assert.equal(result.state, "complete"); assert.deepEqual(result.changedJobIds, []); assert.equal(f.writes(), 0);
    } catch (error) {
      assert.ok(["cancelled", "revocation_pending", "stale_snapshot", "state_unavailable"].includes((error as { category?: string }).category ?? ""), String(error));
      assert.equal(f.writes(), 0);
    } finally { await f.close(); }
  }
});

test("dispatch refuses expired decisions and a replaced queue lock inode", async () => {
  for (const change of ["expired", "lock"] as const) {
    const f = await fixture(); let reads = 0, effects = 0;
    f.input.readSettings = async () => {
      if (++reads === 3) {
        if (change === "expired") f.settings.nowSeconds = 101;
        else {
          await rename(path.join(f.root, "queue.lock"), path.join(f.root, "old-queue.lock"));
          await writeFile(path.join(f.root, "queue.lock"), "", { mode: 0o600 });
        }
      }
      return f.settings;
    };
    // Native replacement knows parent/file identity, not queue policy. Do not
    // make this primitive fake conceal a missing caller-side lease fence.
    f.resources.replaceFile = async (parent, source, destination) => {
      try { effects++; await rename(path.join(f.root, source), path.join(f.root, destination)); return { state: "replaced" }; }
      finally { await parent.close(); }
    };
    try {
      await retryQueuedUploads(f.input, f.resources).catch(error => assert.ok(["stale_snapshot", "state_unavailable"].includes(error.category)));
      assert.equal(effects, 0, change);
      assert.equal((await f.read()).attempts, 4);
    } finally { await f.close(); }
  }
});
