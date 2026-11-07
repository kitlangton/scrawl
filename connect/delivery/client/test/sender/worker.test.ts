import assert from "node:assert/strict";
import test from "node:test";
import { lstat, mkdtemp, readFile, realpath, rename, rm, writeFile } from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import { decodeJob, encodeJob, type Job } from "../../src/queue/job.ts";
import { runUploadWorkerOnce, type UploadWorkerInput, type UploadWorkerResources, type UploadWorkerSettings } from "../../src/queue/worker.ts";
import { withStateQueue } from "../../src/state/queue-transaction.ts";

const id = "a".repeat(32), server = "https://connect.example/connect", generation = "b".repeat(32);
const deferred = () => { let resolve!: () => void; const promise = new Promise<void>(done => { resolve = done; }); return { promise, resolve }; };
async function fixture(rows: Partial<Job>[] = [{}]) {
  const root = await realpath(await mkdtemp(path.join(os.tmpdir(), "connect-upload-worker-")));
  const held = new Set<string>(); let replacements = 0;
  const jobs = rows.map((row, index) => decodeJob(Buffer.from(JSON.stringify({ id: index ? "c".repeat(32) : id,
    bundle: "/synthetic.apparent", server, generation, created: 10, status: "pending", attempts: 0, nextAttempt: 0, future: { keep: 7 }, ...row }))));
  const put = (job: Job) => writeFile(path.join(root, `${job.id}.job`), encodeJob(job), { mode: 0o600 });
  for (const job of jobs) await put(job);
  await writeFile(path.join(root, "upload.credential"), JSON.stringify({ server, scope: "upload", token: "synthetic-token", generation }), { mode: 0o600 });
  const settings: UploadWorkerSettings = { enabled: true, server, encryptionEnabled: false, sharingEnabled: true, nowSeconds: 100, retentionSeconds: 1000 };
  const workerHeld = async () => held.has(String((await lstat(path.join(root, "worker.lock"))).ino));
  const queueHeld = async () => { const stat = await lstat(path.join(root, "queue.lock")).catch(() => null); return !!stat && held.has(String(stat.ino)); };
  // Isolated fake primitives test policy and ownership; native suite proves flock/replace.
  const resources: UploadWorkerResources = {
    async checkAcl(handle) { await handle.close(); return { state: "accepted" }; },
    async lock(handle, { signal }) {
      const stat = await handle.stat(); await handle.close(); const key = String(stat.ino);
      if (signal?.aborted) return { state: "cancelled" }; if (held.has(key)) return { state: "busy" };
      held.add(key); return { state: "acquired", lease: { signal: signal ?? new AbortController().signal,
        assertHeld() { assert.ok(held.has(key)); }, async release() { held.delete(key); } } };
    },
    async replaceFile(parent, source, destination, expected) {
      assert.equal(await queueHeld(), true); replacements++;
      try { const current = await lstat(path.join(root, destination), { bigint: true });
        assert.deepEqual({ device: String(current.dev), inode: String(current.ino) }, expected.destination);
        await rename(path.join(root, source), path.join(root, destination)); return { state: "replaced" };
      } finally { await parent.close(); }
    },
  };
  const input: UploadWorkerInput = { directory: root, approvedServer: server, readSettings: async () => settings,
    upload: async () => ({ state: "ready" }), reportProgress: async () => {} };
  return { root, jobs, settings, input, resources, put, workerHeld, queueHeld,
    replacements: () => replacements,
    read: async (captureId = id) => decodeJob(await readFile(path.join(root, `${captureId}.job`))),
    async close() { assert.equal(held.size, 0); await rm(root, { recursive: true, force: true }); } };
}

test("single worker spans upload while foreground queue and cancellation remain available", async () => {
  const f = await fixture(), started = deferred(), finish = deferred();
  try {
    f.input.upload = async (job, context) => {
      assert.ok(Object.isFrozen(job.future)); await context.assertCurrent();
      assert.equal(await f.workerHeld(), true); assert.equal(await f.queueHeld(), false);
      started.resolve(); await finish.promise; return { state: "ready" };
    };
    const running = runUploadWorkerOnce(f.input, f.resources); await started.promise;
    assert.deepEqual(await runUploadWorkerOnce(f.input, f.resources), { state: "busy", events: [] });
    await withStateQueue(f.root, { scope: "upload", approvedServer: server }, f.resources, {}, async () => { await f.put({ ...f.jobs[0]!, status: "cancelled" }); });
    finish.resolve(); const result = await running;
    assert.equal(result.state, "complete"); assert.equal(result.events[0]!.state, "skipped");
    assert.equal((await f.read()).status, "cancelled"); assert.equal(f.replacements(), 0);
  } finally { finish.resolve(); await f.close(); }
});

test("ready completion retains fresh metadata and unknown fields", async () => {
  const f = await fixture(); try {
    f.input.upload = async (job, context) => {
      await context.assertCurrent();
      await withStateQueue(f.root, { scope: "upload", approvedServer: server }, f.resources, {}, async () => {
        await f.put({ ...job, files: [{ path: "summary.md", size: 1, sha256: "1".repeat(64) }], future: { keep: 9 } });
      });
      return { state: "ready" };
    };
    const result = await runUploadWorkerOnce(f.input, f.resources), saved = await f.read();
    assert.equal(result.state, "complete"); assert.equal(saved.status, "ready"); assert.equal(saved.attempts, 1);
    assert.equal(saved.files.length, 1); assert.deepEqual(saved.future, { keep: 9 }); assert.equal(f.replacements(), 1);
  } finally { await f.close(); }
});

test("expiry, due time and disabled or stale incarnations precede upload", async () => {
  const f = await fixture([{ created: 1 }, { nextAttempt: 200 }]); try {
    Object.assign(f.settings, { retentionSeconds: 50 }); let uploads = 0;
    f.input.upload = async () => { uploads++; return { state: "ready" }; };
    assert.equal((await runUploadWorkerOnce(f.input, f.resources)).state, "complete");
    assert.equal((await f.read()).status, "expired"); assert.equal(uploads, 0); assert.equal(f.replacements(), 1);
    Object.assign(f.settings, { enabled: false });
    await runUploadWorkerOnce(f.input, f.resources); assert.equal(uploads, 0);
    Object.assign(f.settings, { enabled: true, nowSeconds: 201, encryptionEnabled: true });
    await runUploadWorkerOnce(f.input, f.resources); assert.equal(uploads, 0); assert.equal((await f.read("c".repeat(32))).status, "pending");
  } finally { await f.close(); }
});

test("failure policy preserves waiting attempts, bounded retries and terminal auth", async () => {
  for (const code of ["encryption_pending_recipient", "quota_exceeded", "auth_required"]) {
    const f = await fixture(); try {
      f.input.upload = async () => ({ state: "failed", facts: { message: code + " private-token /private/path", causes: [] } });
      const result = await runUploadWorkerOnce(f.input, f.resources), saved = await f.read();
      assert.equal(saved.error, code); assert.equal(saved.attempts, code === "encryption_pending_recipient" ? 0 : 1);
      assert.equal(saved.nextAttempt, code === "encryption_pending_recipient" ? 105 : 110);
      assert.equal(saved.status, code === "auth_required" ? "failed" : "pending");
      assert.equal(JSON.stringify(result).includes("private-token"), false);
      if (saved.status === "pending") { let again = 0; f.input.upload = async () => { again++; return { state: "ready" }; };
        await runUploadWorkerOnce(f.input, f.resources); assert.equal(again, 0); }
    } finally { await f.close(); }
  }
});

test("status-only progress is outside queue and fresh unsharing suppresses it", async () => {
  for (const unshare of [false, true]) {
    const f = await fixture([{ shared: true }]); try {
      let reports = 0;
      f.input.upload = async job => {
        if (unshare) await withStateQueue(f.root, { scope: "upload", approvedServer: server }, f.resources, {}, () => f.put({ ...job, shared: false }));
        return { state: "failed", facts: { message: "auth_required", causes: [] } };
      };
      f.input.reportProgress = async (job, state, context) => {
        await context.assertCurrent(); reports++; assert.equal(state, "failed"); assert.equal(job.shared, true);
        assert.equal(await f.workerHeld(), true); assert.equal(await f.queueHeld(), false); throw Error("private-progress-error");
      };
      assert.equal((await runUploadWorkerOnce(f.input, f.resources)).state, "complete"); assert.equal(reports, unshare ? 0 : 1);
    } finally { await f.close(); }
  }
});

test("unknown or uncertain upload stops without replaying further jobs", async () => {
  for (const kind of ["uncertain", "throw"]) {
    const f = await fixture([{}, {}]); try {
      let calls = 0; f.input.upload = async () => { calls++; if (kind === "throw") throw Error("private-secret"); return { state: "uncertain" }; };
      const result = await runUploadWorkerOnce(f.input, f.resources);
      assert.equal(result.state, "uncertain"); assert.equal(calls, 1); assert.equal(f.replacements(), 0); assert.equal((await f.read()).status, "pending");
      assert.equal(JSON.stringify(result).includes("private-secret"), false);
    } finally { await f.close(); }
  }
});

test("a replaced worker inode cannot publish a late ready result", async () => {
  const f = await fixture(); try {
    f.input.upload = async () => {
      await rename(path.join(f.root, "worker.lock"), path.join(f.root, "worker.lock.old"));
      await writeFile(path.join(f.root, "worker.lock"), "", { mode: 0o600 });
      return { state: "ready" };
    };
    assert.equal((await runUploadWorkerOnce(f.input, f.resources)).state, "uncertain");
    assert.equal((await f.read()).status, "pending"); assert.equal(f.replacements(), 0);
  } finally { await f.close(); }
});

test("lost replacement acknowledgement and worker-release failure never replay", async () => {
  for (const kind of ["replace", "release"]) {
    const f = await fixture(); try {
      if (kind === "replace") { const original = f.resources.replaceFile; f.resources.replaceFile = async (...args) => { await original(...args); throw Error("private-ack"); }; }
      else { const original = f.resources.lock;
        f.resources.lock = async (...args) => { const result = await original(...args);
          if (result.state === "acquired") { const release = result.lease.release; const worker = await f.workerHeld(), queue = await f.queueHeld();
            result.lease.release = async () => { await release(); if (worker && !queue) throw Error("private-release"); }; }
          return result; };
      }
      assert.equal((await runUploadWorkerOnce(f.input, f.resources)).state, "uncertain");
      assert.equal((await f.read()).status, "ready"); assert.equal(f.replacements(), 1);
    } finally { await f.close(); }
  }
});
