import test from "node:test";
import assert from "node:assert/strict";
import { constants as C } from "node:fs";
import { link, lstat, mkdir, mkdtemp, open, readdir, realpath, rename, rm, unlink, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import type { FileHandle } from "node:fs/promises";
import { readCredentialSnapshot, assertCredentialSnapshotCurrent, CredentialSnapshotError, type NamespaceResources } from "../../src/state/namespace.ts";
import { readStoredCredentialHandle, CredentialReadError } from "../../src/credentials/private-read.ts";

const context = { scope: "read" as const, approvedServer: "https://sync.example" };
const credential = (generation = "a".repeat(32), token = "access") => ({ server: context.approvedServer, site: null, scope: "read", token,
  generation, refreshToken: "refresh", expiresAt: 10, refreshExpiresAt: 20, grantId: "grant", authRequired: false,
  encryption_identity: "enc", accountInitial: null });
function resources(overrides: Partial<NamespaceResources> = {}) {
  const state = { held: false, lose: false, rejectRelease: false, beforeAcquire: undefined as undefined | (() => Promise<void>) };
  const value: NamespaceResources = {
    async checkAcl(handle: FileHandle) { await handle.close(); return { state: "accepted" }; },
    async lock(handle, { signal }) {
      try { if (state.beforeAcquire) await state.beforeAcquire(); } finally { await handle.close(); }
      if (signal?.aborted) return { state: "cancelled" };
      state.held = true;
      return { state: "acquired" as const, lease: { signal: signal ?? new AbortController().signal, assertHeld() { if (!state.held || state.lose) throw new Error("lease_lost"); },
        async release() { state.held = false; if (state.rejectRelease) throw new Error("fake release failure"); } } };
    },
    ...overrides,
  };
  return { port: value, state };
}
async function fixture() {
  const root = await mkdtemp(path.join(await realpath(os.tmpdir()), "connect-state-"));
  await mkdir(path.join(root, "state"), { mode: 0o700 });
  const dir = path.join(root, "state");
  const put = async (name: string, value: unknown) => writeFile(path.join(dir, name), typeof value === "string" ? value : JSON.stringify(value), { mode: 0o600 });
  await put("read.credential", credential());
  return { root, dir, put, close: () => rm(root, { recursive: true, force: true }) };
}
async function snapshot(f: Awaited<ReturnType<typeof fixture>>, port = resources().port) { return readCredentialSnapshot(f.dir, context, port); }
const category = async (promise: Promise<unknown>, expected: string) => {
  await assert.rejects(promise, e => e instanceof CredentialSnapshotError && e.category === expected);
};

test("reads compatible facts and creates only a stable queue.lock", async () => {
  const f = await fixture();
  try {
    const result = await snapshot(f);
    assert.equal(result.credential.generation, "a".repeat(32));
    assert.equal(result.cancellation, null);
    const first = await lstat(path.join(f.dir, "queue.lock"));
    await assertCredentialSnapshotCurrent(f.dir, result, resources().port);
    const second = await lstat(path.join(f.dir, "queue.lock"));
    assert.equal(first.ino, second.ino);
    assert.deepEqual((await readdir(f.dir)).sort(), ["queue.lock", "read.credential"]);
  } finally { await f.close(); }
});

test("accepts any JSON cancellation string and rejects malformed or oversized cancellation", async () => {
  const f = await fixture();
  try {
    await f.put("cancellation", "\"anything, including non-hex\"");
    assert.equal((await snapshot(f)).cancellation, "anything, including non-hex");
    await f.put("cancellation", "{");
    await category(snapshot(f), "invalid_credential");
    await f.put("cancellation", JSON.stringify("x".repeat(129)));
    await category(snapshot(f), "unsafe_file");
  } finally { await f.close(); }
});

test("any pending revocation intent blocks, regardless of contents", async () => {
  const f = await fixture();
  try { await f.put("revocation-garbage.intent", "not json"); await category(snapshot(f), "revocation_pending"); }
  finally { await f.close(); }
});

test("assert-current rejects login and cancellation changes but allows token rotation", async () => {
  const f = await fixture();
  try {
    const initial = await snapshot(f);
    await f.put("read.credential", credential("b".repeat(32)));
    await category(assertCredentialSnapshotCurrent(f.dir, initial, resources().port), "stale_snapshot");
    const next = await snapshot(f);
    await f.put("read.credential", credential("b".repeat(32), "rotated-access"));
    await assertCredentialSnapshotCurrent(f.dir, next, resources().port);
    await f.put("read.credential", JSON.stringify({ ...credential("b".repeat(32), "rotated-access"), authRequired: true }));
    await category(assertCredentialSnapshotCurrent(f.dir, next, resources().port), "stale_snapshot");
    await f.put("read.credential", credential("b".repeat(32), "rotated-access"));
    await f.put("cancellation", JSON.stringify("cancelled-login"));
    await category(assertCredentialSnapshotCurrent(f.dir, next, resources().port), "stale_snapshot");
  } finally { await f.close(); }
});

test("refuses a hardlinked queue lock while data hardlinks remain compatible", async () => {
  const f = await fixture();
  try {
    const outside = path.join(f.root, "data-link");
    await link(path.join(f.dir, "read.credential"), outside);
    const handle = await open(outside, C.O_RDONLY | (C.O_NOFOLLOW ?? 0));
    const data = await readStoredCredentialHandle(handle, context, resources().port);
    assert.equal(data.token, "access");
    await writeFile(path.join(f.dir, "queue.lock"), "", { mode: 0o600 });
    await link(path.join(f.dir, "queue.lock"), path.join(f.root, "lock-link"));
    await category(snapshot(f), "unsafe_file");
  } finally { await f.close(); }
});

test("lease loss, release failure, and abort fail closed", async () => {
  const f = await fixture();
  try {
    const lost = resources(); lost.state.lose = true;
    await category(snapshot(f, lost.port), "unavailable");
    const releaseFailure = resources(); releaseFailure.state.rejectRelease = true;
    await category(snapshot(f, releaseFailure.port), "unavailable");
    const abort = new AbortController(); abort.abort();
    await category(readCredentialSnapshot(f.dir, context, resources().port, { signal: abort.signal }), "cancelled");
    const duringWait = new AbortController(), waiting = resources();
    waiting.state.beforeAcquire = async () => { duringWait.abort(); };
    await category(readCredentialSnapshot(f.dir, context, waiting.port, { signal: duringWait.signal }), "cancelled");
  } finally { await f.close(); }
});

test("directory and lock replacements fail closed", async () => {
  const f = await fixture();
  try {
    const r = resources();
    r.state.beforeAcquire = async () => {
      const lock = path.join(f.dir, "queue.lock"), parked = path.join(f.dir, "queue.old");
      await rename(lock, parked); await writeFile(lock, "", { mode: 0o600 });
    };
    await category(snapshot(f, r.port), "unsafe_file");
    const before = await snapshot(f, resources().port);
    const parked = `${f.dir}.old`; await rename(f.dir, parked); await mkdir(f.dir, { mode: 0o700 });
    await f.put("read.credential", credential());
    await category(assertCredentialSnapshotCurrent(f.dir, before, resources().port), "stale_snapshot");
  } finally { await f.close(); }
});



test("cancellation records require descriptor ACL approval under the queue lease", async () => {
  const f = await fixture(); const r = resources(); let cancellationChecks = 0;
  r.port.checkAcl = async handle => {
    const metadata = await handle.stat({ bigint: true }); await handle.close();
    if (metadata.isFile() && metadata.size < 128n) { cancellationChecks++; return { state: "refused" }; }
    return { state: "accepted" };
  };
  try {
    await f.put("cancellation", JSON.stringify("epoch"));
    await category(snapshot(f, r.port), "unsafe_file");
    assert.equal(cancellationChecks, 1); assert.equal(r.state.held, false);
  } finally { await f.close(); }
});

test("a disappearing lock produces only a redacted error and releases its lease", async () => {
  const f = await fixture(); const r = resources();
  r.port.checkAcl = async handle => {
    const metadata = await handle.stat(); await handle.close();
    if (metadata.isFile()) await unlink(path.join(f.dir, "queue.lock"));
    return { state: "accepted" };
  };
  try {
    await assert.rejects(snapshot(f, r.port), error => {
      assert.equal(String(error), "CredentialSnapshotError: unavailable"); return error instanceof CredentialSnapshotError;
    });
    assert.equal(r.state.held, false);
  } finally { await f.close(); }
});

test("snapshot fields are pinned before waiting and lease loss during reads refuses a result", async () => {
  const f = await fixture();
  try {
    const initial = await snapshot(f);
    const mutable = { ...initial, credential: { ...initial.credential } }, r = resources();
    r.state.beforeAcquire = async () => { mutable.cancellation = "changed by caller"; mutable.credential.generation = "d".repeat(32); };
    await assertCredentialSnapshotCurrent(f.dir, mutable, r.port);
    const lost = resources();
    lost.port.checkAcl = async handle => {
      const metadata = await handle.stat(); await handle.close();
      if (metadata.isFile()) lost.state.lose = true;
      return { state: "accepted" };
    };
    await category(snapshot(f, lost.port), "unavailable");
    assert.equal(lost.state.held, false);
  } finally { await f.close(); }
});


test("saved snapshot credentials are immutable before a later currentness check", async () => {
  const f = await fixture();
  try {
    const saved = await snapshot(f);
    assert.equal(Object.isFrozen(saved), true);
    assert.equal(Object.isFrozen(saved.credential), true);
    for (const [key, value] of [["generation", "f".repeat(32)], ["grantId", "different-grant"], ["encryption_identity", "different-identity"]] as const) {
      assert.equal(Reflect.set(saved.credential, key, value), false);
    }
    await assertCredentialSnapshotCurrent(f.dir, saved, resources().port);
    await f.put("read.credential", credential("f".repeat(32)));
    await category(assertCredentialSnapshotCurrent(f.dir, saved, resources().port), "stale_snapshot");
  } finally { await f.close(); }
});
