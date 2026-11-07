import test from "node:test";
import assert from "node:assert/strict";
import { link, lstat, mkdir, mkdtemp, readdir, realpath, rename, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import type { FileHandle } from "node:fs/promises";
import { CredentialSnapshotError, withStateQueue, withStateWorker, type NamespaceLease, type NamespaceResources } from "../../src/state/queue-transaction.ts";

const upload = { scope: "upload" as const, approvedServer: "https://sync.example" };
function fakeResources(overrides: { lose?: boolean; releaseFails?: boolean } = {}) {
  const held = new Set<string>();
  const port: NamespaceResources = {
    async checkAcl(handle) { await handle.close(); return { state: "accepted" }; },
    async lock(handle, { signal }) {
      let key: string;
      try { const s = await handle.stat({ bigint: true }); key = `${s.dev}:${s.ino}`; }
      finally { await handle.close(); }
      if (signal?.aborted) return { state: "cancelled" };
      if (held.has(key)) return { state: "busy" };
      held.add(key);
      const lease: NamespaceLease = {
        signal: signal ?? new AbortController().signal,
        assertHeld() { if (!held.has(key) || overrides.lose) throw new Error("lease_lost"); },
        async release() { held.delete(key); if (overrides.releaseFails) throw new Error("release_failed"); },
      };
      return { state: "acquired", lease };
    },
  };
  return { port, held };
}
async function fixture() {
  const root = await mkdtemp(path.join(await realpath(os.tmpdir()), "connect-worker-"));
  const dir = path.join(root, "state"); await mkdir(dir, { mode: 0o700 });
  return { root, dir, close: () => rm(root, { recursive: true, force: true }) };
}
async function category(promise: Promise<unknown>, expected: string) {
  await assert.rejects(promise, e => e instanceof CredentialSnapshotError && e.category === expected);
}
const deferred = <T>() => { let resolve!: (value: T) => void; const promise = new Promise<T>(r => { resolve = r; }); return { promise, resolve }; };

test("worker lease spans pending async body and leaves queue independently available", async () => {
  const f = await fixture(), r = fakeResources(); const started = deferred<void>(), finish = deferred<void>();
  try {
    const lockPath = path.join(f.dir, "worker.lock");
    const running = withStateWorker(f.dir, upload, r.port, {}, async () => {
      const heldStat = await lstat(lockPath); started.resolve(); await finish.promise;
      assert.equal((await lstat(lockPath)).ino, heldStat.ino);
      await withStateQueue(f.dir, upload, r.port, {}, async () => "queue transaction");
      return "done";
    });
    await started.promise;
    await category(withStateWorker(f.dir, upload, r.port, {}, async () => assert.fail("busy worker ran")), "lock_refused");
    assert.equal(await withStateQueue(f.dir, upload, r.port, {}, async () => "queue free"), "queue free");
    finish.resolve(); assert.equal(await running, "done");
    assert.equal(r.held.size, 0);
    const released = await lstat(lockPath);
    await withStateWorker(f.dir, upload, r.port, {}, async () => {});
    assert.equal((await lstat(lockPath)).ino, released.ino);
  } finally { finish.resolve(); await f.close(); }
});

test("worker requires upload scope before admitting state", async () => {
  const f = await fixture(), r = fakeResources();
  try { assert.throws(() => withStateWorker(f.dir, { ...upload, scope: "read" }, r.port, {}, async () => {}), e => e instanceof CredentialSnapshotError && e.category === "invalid_credential"); assert.equal(r.held.size, 0); }
  finally { await f.close(); }
});

test("caller abort, lease abort, body throw and release failure all release or refuse", async () => {
  const f = await fixture();
  try {
    const caller = new AbortController(); caller.abort(); await category(withStateWorker(f.dir, upload, fakeResources().port, { signal: caller.signal }, async () => {}), "cancelled");
    const duringBody = new AbortController();
    await category(withStateWorker(f.dir, upload, fakeResources().port, { signal: duringBody.signal }, async () => { duringBody.abort(); }), "cancelled");
    const leaseAbort = new AbortController();
    const abortPort = fakeResources();
    // The lease signal is the resource port's cancellation signal, distinct from caller cancellation.
    abortPort.port.lock = async handle => {
      await handle.close();
      return { state: "acquired", lease: { signal: leaseAbort.signal, assertHeld() {}, async release() {} } };
    };
    await category(withStateWorker(f.dir, upload, abortPort.port, {}, async () => { leaseAbort.abort(); }), "cancelled");
    const thrown = fakeResources(); await assert.rejects(withStateWorker(f.dir, upload, thrown.port, {}, async () => { throw new Error("body"); }), /body/); assert.equal(thrown.held.size, 0);
    await category(withStateWorker(f.dir, upload, fakeResources({ releaseFails: true }).port, {}, async () => {}), "unavailable");
  } finally { await f.close(); }
});

test("lease loss and replaced root or worker inode fail closed", async () => {
  const f = await fixture();
  try {
    await category(withStateWorker(f.dir, upload, fakeResources({ lose: true }).port, {}, async () => {}), "unavailable");
    const rootPort = fakeResources();
    await category(withStateWorker(f.dir, upload, rootPort.port, {}, async () => {
      const moved = `${f.dir}.moved`; await rename(f.dir, moved); await mkdir(f.dir, { mode: 0o700 });
    }), "unavailable");
    await rm(f.dir, { recursive: true, force: true }); await mkdir(f.dir, { mode: 0o700 });
    const lockPort = fakeResources();
    await category(withStateWorker(f.dir, upload, lockPort.port, {}, async () => {
      const lock = path.join(f.dir, "worker.lock"); await rename(lock, `${lock}.old`); await writeFile(lock, "", { mode: 0o600 });
    }), "unsafe_file");
    assert.equal(lockPort.held.size, 0);
  } finally { await f.close(); }
});

test("hardlinked worker lock is refused and does not write product JSON", async () => {
  const f = await fixture();
  try {
    const lock = path.join(f.dir, "worker.lock"); await writeFile(lock, "", { mode: 0o600 });
    await link(lock, path.join(f.root, "worker-link"));
    await category(withStateWorker(f.dir, upload, fakeResources().port, {}, async () => assert.fail("unsafe worker lock ran")), "unsafe_file");
    assert.equal((await lstat(lock)).nlink, 2);
    assert.deepEqual((await readdir(f.dir)).sort(), ["worker.lock"]);
  } finally { await f.close(); }
});
