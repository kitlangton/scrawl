import test from "node:test";
import assert from "node:assert/strict";
import { link, lstat, mkdir, mkdtemp, readFile, realpath, readdir, rm, symlink, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import type { FileHandle } from "node:fs/promises";
import { readQueueSnapshot, QueueStoreError } from "../../src/queue/store.ts";
import type { NamespaceResources } from "../../src/state/namespace.ts";

const context = { scope: "read" as const, approvedServer: "https://sync.example" };
const id = "a".repeat(32);
const legacy = { id, bundle: "/private/capture.apparent", server: context.approvedServer, generation: "b".repeat(32), created: 10,
  attempts: 0, nextAttempt: 0, status: "pending", unknownLegacy: { keep: true } };
function resources(overrides: Partial<NamespaceResources> = {}) {
  const state = { held: false, lose: false, beforeAcquire: undefined as undefined | (() => Promise<void>), aclFiles: 0 };
  const port: NamespaceResources = {
    async checkAcl(handle: FileHandle) { const st = await handle.stat(); await handle.close(); if (st.isFile() && st.size > 0) state.aclFiles++; return { state: "accepted" }; },
    async lock(handle, { signal }) { try { await state.beforeAcquire?.(); } finally { await handle.close(); }
      if (signal?.aborted) return { state: "cancelled" };
      state.held = true; return { state: "acquired" as const, lease: { signal: signal ?? new AbortController().signal,
        assertHeld() { if (!state.held || state.lose) throw Error("lost lease private details"); }, async release() { state.held = false; } } }; },
    ...overrides,
  };
  return { port, state };
}
async function fixture() {
  const root = await mkdtemp(path.join(await realpath(os.tmpdir()), "connect-queue-"));
  const dir = path.join(root, "state"); await mkdir(dir, { mode: 0o700 });
  const put = (name: string, value: string | unknown) => writeFile(path.join(dir, name), typeof value === "string" ? value : JSON.stringify(value), { mode: 0o600 });
  return { root, dir, put, close: () => rm(root, { recursive: true, force: true }) };
}
const category = (expected: string) => (error: unknown) => error instanceof QueueStoreError && error.category === expected && String(error) === `QueueStoreError: ${expected}`;

test("reads legacy direct .job layout, applies Rust-compatible defaults, preserves unknown fields and freezes detached data", async () => {
  const f = await fixture(); try {
    await f.put(`${id}.job`, legacy); const r = resources();
    const before = await readFile(path.join(f.dir, `${id}.job`));
    const jobs = await readQueueSnapshot(f.dir, context, r.port);
    assert.equal(jobs.length, 1); const job = jobs[0]!;
    assert.equal(job.files.length, 0); assert.equal(job.shared, false); assert.equal(job.publicCopy, false); assert.equal(job.encrypted, false);
    assert.equal(job.encryption, null); assert.equal(job.grouped, false); assert.equal(job.error, null);
    assert.deepEqual(job.unknownLegacy, { keep: true }); assert.equal(Object.isFrozen(jobs), true); assert.equal(Object.isFrozen(job), true);
    assert.equal(r.state.aclFiles, 1); assert.deepEqual(await readFile(path.join(f.dir, `${id}.job`)), before);
    assert.deepEqual(await (await lstat(path.join(f.dir, "queue.lock"))).isFile(), true);
    assert.equal((await readdir(f.dir)).sort().join(","), `${id}.job,queue.lock`);
  } finally { await f.close(); }
});

test("missing records are an empty queue; malformed, bad names and id mismatch fail the whole snapshot", async () => {
  const f = await fixture(); try {
    assert.deepEqual(await readQueueSnapshot(f.dir, context, resources().port), []);
    await f.put(`${id}.job`, "{"); await assert.rejects(readQueueSnapshot(f.dir, context, resources().port), category("invalid_queue_record"));
    await rm(path.join(f.dir, `${id}.job`)); await f.put(`${id}.job`, { ...legacy, id: "c".repeat(32) });
    await assert.rejects(readQueueSnapshot(f.dir, context, resources().port), category("invalid_queue_record"));
    await rm(path.join(f.dir, `${id}.job`)); await f.put("unexpected.job", legacy);
    await assert.rejects(readQueueSnapshot(f.dir, context, resources().port), category("invalid_queue_record"));
  } finally { await f.close(); }
});

test("consumes descriptors and rejects symlinks, file replacement, directory replacement, cancellation and lease loss", async () => {
  const f = await fixture(); try {
    await f.put(`${id}.job`, legacy);
    const symlinkPath = path.join(f.dir, `${"c".repeat(32)}.job`); await symlink(`${id}.job`, symlinkPath);
    await assert.rejects(readQueueSnapshot(f.dir, context, resources().port), category("unsafe_file")); await rm(symlinkPath);
    const replaced = resources(); replaced.port.checkAcl = async handle => { const st = await handle.stat(); await handle.close();
      if (st.size > 0) { await rm(path.join(f.dir, `${id}.job`)); await f.put(`${id}.job`, legacy); } return { state: "accepted" }; };
    await assert.rejects(readQueueSnapshot(f.dir, context, replaced.port), category("unsafe_file"));
    const lost = resources(); lost.port.checkAcl = async handle => { await handle.close(); lost.state.lose = true; return { state: "accepted" }; };
    await assert.rejects(readQueueSnapshot(f.dir, context, lost.port), category("unavailable"));
    const abort = new AbortController(); abort.abort(); await assert.rejects(readQueueSnapshot(f.dir, context, resources().port, { signal: abort.signal }), category("cancelled"));
    const moved = resources(); moved.state.beforeAcquire = async () => { await rm(path.join(f.root, "state"), { recursive: true }); await mkdir(f.dir, { mode: 0o700 }); };
    await assert.rejects(readQueueSnapshot(f.dir, context, moved.port), e => e instanceof QueueStoreError && ["unsafe_directory", "unsafe_file", "unavailable"].includes(e.category));
  } finally { await f.close(); }
});

test("lock acquisition failures redact private diagnostics", async () => {
  const f = await fixture(); try {
    const r = resources(); r.port.lock = async handle => { await handle.close(); throw Error("token=super-secret /private/path"); };
    await assert.rejects(readQueueSnapshot(f.dir, context, r.port), e => { assert.equal(String(e), "QueueStoreError: unavailable"); return e instanceof QueueStoreError; });
  } finally { await f.close(); }
});

test("retains donor data-hardlink compatibility without relaxing the lock inode rule", async () => {
  const f = await fixture(); try {
    await f.put(`${id}.job`, legacy);
    await link(path.join(f.dir, `${id}.job`), path.join(f.dir, "saved-record"));
    assert.equal((await readQueueSnapshot(f.dir, context, resources().port))[0]!.id, id);
    await link(path.join(f.dir, "queue.lock"), path.join(f.dir, "saved-lock"));
    await assert.rejects(readQueueSnapshot(f.dir, context, resources().port), category("unsafe_file"));
  } finally { await f.close(); }
});

test("accepts exactly 100 records and refuses an overfull queue as a whole", async () => {
  const f = await fixture(); try {
    for (let i = 0; i < 100; i++) {
      const next = i.toString(16).padStart(32, "0");
      await f.put(`${next}.job`, { ...legacy, id: next });
    }
    assert.equal((await readQueueSnapshot(f.dir, context, resources().port)).length, 100);
    await f.put(`${id}.job`, legacy);
    await assert.rejects(readQueueSnapshot(f.dir, context, resources().port), category("invalid_queue_record"));
  } finally { await f.close(); }
});
