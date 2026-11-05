import assert from "node:assert/strict";
import { createHash as cryptoHash } from "node:crypto";
import { lstat, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import type { CaptureClient, CaptureStatus } from "@apparent/engine-sdk/capture";
import { createRecordPublicationRecovery } from "../../dist/backend/publication/recovery.js";
import { SqlitePublicationBindingStore } from "../../dist/backend/publication/store.js";

const captureId = "a".repeat(32);
const publicationId = "b".repeat(32);
const ownerToken = "c".repeat(64);
const materials = {
  manifest: {
    schema: "apparent.capture-materials/1" as const,
    key: { captureId, materialsId: "d".repeat(32), revision: "e".repeat(64) },
    root: "/private/materials",
    leaseId: "f".repeat(32),
    payloadFile: { relativePath: "materials.json", identity: { device: "1", inode: "2", size: 1, modifiedNs: "3" }, sha256: "e".repeat(64) },
  },
  payload: { schema: "apparent.capture-materials-payload/1", bundleId: "take.apparent" },
};

function makeInput(path = "/tmp/apparent-publication-test/take.apparent") {
  const planBytes = Buffer.from(JSON.stringify({
    schema: "apparent.bundle-publication/1",
    publicationId,
    materials: materials.manifest.key,
    bundleId: materials.payload.bundleId,
    destination: { path, mode: "absent" },
    entries: [{ relativePath: "apparent.json", sha256: "1".repeat(64) }],
  }));
  const planSha256 = createHash(planBytes);
  return {
    ownerToken, captureId, leaseId: materials.manifest.leaseId, publicationId,
    planBytes: new Uint8Array(planBytes), materials: structuredClone(materials),
    planFile: {
      stagingRoot: "/private/staging",
      planSha256,
      planFile: { relativePath: "plan.json", identity: { device: "1", inode: "4", size: planBytes.length, modifiedNs: "5" }, sha256: planSha256 },
    },
  } as unknown as Parameters<CaptureClient["publish"]>[0];
}

function createHash(bytes: Uint8Array): string {
  // The SDK validates this same exact-byte digest before transport dispatch.
  return cryptoHash("sha256").update(bytes).digest("hex");
}

function pending(capture: string, pub: string, sha: string): CaptureStatus {
  return { captureId: capture, state: "publishing", publicationId: pub, planSha256: sha, materials: materials.manifest };
}

async function temp() { return mkdtemp(join(tmpdir(), "record-pub-recovery-")); }

test("lost publish reply survives reopen and reconciliation uses the durable original binding", async () => {
  const root = await temp();
  const input = makeInput(join(root, "final.apparent"));
  let dispatches = 0;
  const store = new SqlitePublicationBindingStore(join(root, "private"));
  const sdk = {
    async publish() { dispatches++; throw new Error("lost response including private data"); },
    async inspectPublication(request: { captureId: string; publicationId: string; planSha256: string; expected: { bundlePath: string; bundleId: string; manifestSha256: string; materials: typeof materials.manifest.key } }) {
      assert.equal(request.captureId, captureId);
      assert.equal(request.publicationId, publicationId);
      assert.equal(request.planSha256, input.planFile.planSha256);
      assert.deepEqual(request.expected, { materials: materials.manifest.key, bundleId: "take.apparent", bundlePath: join(root, "final.apparent"), manifestSha256: "1".repeat(64) });
      return pending(captureId, publicationId, input.planFile.planSha256);
    },
  } as unknown as CaptureClient;
  try {
    const first = createRecordPublicationRecovery(sdk, join(root, "private"));
    await assert.rejects(first.publish(input), /outcome is unknown/);
    await assert.rejects(lstat(join(root, "final.apparent")), { code: "ENOENT" });
    assert.equal(dispatches, 1);
    const durable = await store.get(captureId);
    assert.ok(durable);
    assert.equal(durable.state, "dispatching");
    const reopened = createRecordPublicationRecovery(sdk, join(root, "private"));
    assert.equal((await reopened.reconcile(captureId, ownerToken)).state, "publishing");
    assert.equal((await store.get(captureId))?.publicationId, publicationId);
    assert.equal(dispatches, 1);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("existing final destination is refused without reserving or replacing it", async () => {
  const root = await temp();
  const finalPath = join(root, "already-there.apparent");
  await writeFile(finalPath, "preserve");
  let dispatches = 0;
  const sdk = { async publish() { dispatches++; throw new Error("should not dispatch"); } } as unknown as CaptureClient;
  try {
    const recovery = createRecordPublicationRecovery(sdk, join(root, "private"));
    await assert.rejects(recovery.publish(makeInput(finalPath)), /destination already exists/);
    assert.equal(await readFile(finalPath, "utf8"), "preserve");
    assert.equal(await new SqlitePublicationBindingStore(join(root, "private")).get(captureId), undefined);
    assert.equal(dispatches, 0);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("mutated or foreign publication binding is refused without redispatch", async () => {
  const root = await temp();
  let dispatches = 0;
  const sdk = { async publish() { dispatches++; throw new Error("ambiguous"); } } as unknown as CaptureClient;
  try {
    const recovery = createRecordPublicationRecovery(sdk, join(root, "private"));
    const input = makeInput(join(root, "one.apparent"));
    await assert.rejects(recovery.publish(input));
    const changed = makeInput(join(root, "two.apparent"));
    await assert.rejects(recovery.publish(changed), /conflicts with its durable binding/);
    const changedLease = makeInput(join(root, "one.apparent"));
    changedLease.leaseId = "0".repeat(32);
    changedLease.materials.manifest.leaseId = "0".repeat(32);
    await assert.rejects(recovery.publish(changedLease), /conflicts with its durable binding/);
    assert.equal(dispatches, 1);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("concurrent workflow instances admit one publication dispatch", async () => {
  const root = await temp();
  let dispatches = 0;
  let release!: (status: CaptureStatus) => void;
  const sdk = {
    publish() { dispatches++; return new Promise<CaptureStatus>(resolve => { release = resolve; }); },
    async inspectPublication(request: { captureId: string; publicationId: string; planSha256: string }) { return pending(request.captureId, request.publicationId, request.planSha256); },
  } as unknown as CaptureClient;
  try {
    const one = createRecordPublicationRecovery(sdk, join(root, "private"));
    const two = createRecordPublicationRecovery(sdk, join(root, "private"));
    const input = makeInput(join(root, "final.apparent"));
    const a = one.publish(input);
    while (!release) await new Promise(resolve => setImmediate(resolve));
    assert.throws(() => one.publish(makeInput(join(root, "foreign.apparent"))), /conflicts with its in-flight binding/);
    const b = await two.publish(input);
    assert.equal(b.state, "publishing");
    assert.equal(dispatches, 1);
    release(pending(captureId, publicationId, input.planFile.planSha256));
    assert.equal((await a).state, "publishing");
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("coalesced publish callers receive independent observation objects", async () => {
  const root = await temp();
  let release!: (status: CaptureStatus) => void;
  const input = makeInput(join(root, "final.apparent"));
  const sdk = { publish() { return new Promise<CaptureStatus>(resolve => { release = resolve; }); } } as unknown as CaptureClient;
  try {
    const recovery = createRecordPublicationRecovery(sdk, join(root, "private"));
    const first = recovery.publish(input);
    const firstMutated = first.then(status => { status.captureId = "changed"; return status; });
    const second = recovery.publish(input);
    while (!release) await new Promise(resolve => setImmediate(resolve));
    release(pending(captureId, publicationId, input.planFile.planSha256));
    assert.equal((await firstMutated).captureId, "changed");
    assert.equal((await second).captureId, captureId);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("private storage failure retains material ownership and never dispatches", async () => {
  const root = await temp();
  let dispatches = 0;
  const sdk = { async publish() { dispatches++; throw new Error("should not dispatch"); } } as unknown as CaptureClient;
  const notDirectory = join(root, "not-a-directory");
  await writeFile(notDirectory, "occupied");
  try {
    const recovery = createRecordPublicationRecovery(sdk, join(notDirectory, "private"));
    await assert.rejects(recovery.publish(makeInput(join(root, "final.apparent"))), /storage is unavailable/);
    assert.equal(dispatches, 0);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("foreign observed bindings and stale pending replies cannot replace a terminal observation", async () => {
  const root = await temp();
  try {
    const store = new SqlitePublicationBindingStore(join(root, "private"));
    const input = makeInput(join(root, "final.apparent"));
    const record = {
      captureId, publicationId, planSha256: input.planFile.planSha256, materials: materials.manifest.key, leaseId: materials.manifest.leaseId,
      bundleId: "take.apparent", bundlePath: join(root, "final.apparent"), manifestSha256: "1".repeat(64), state: "reserved" as const,
    };
    await store.reserve(record);
    await store.observe(record, "cancelled");
    assert.equal(await store.observe(record, "publishing"), false);
    assert.equal(await store.observe(record, "failed"), false);
    assert.equal((await store.get(captureId))?.state, "cancelled");
    await assert.rejects(store.observe({ ...record, manifestSha256: "2".repeat(64) }, "failed"), /storage is unavailable/);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("a delayed reconcile cannot return publishing after a newer terminal observation", async () => {
  const root = await temp();
  let first!: (status: CaptureStatus) => void;
  let calls = 0;
  const input = makeInput(join(root, "final.apparent"));
  const sdk = {
    async publish() { throw new Error("ambiguous"); },
    inspectPublication() {
      if (++calls === 1) return new Promise<CaptureStatus>(resolve => { first = resolve; });
      return Promise.resolve({ ...pending(captureId, publicationId, input.planFile.planSha256), state: "cancelled" });
    },
  } as unknown as CaptureClient;
  try {
    const recovery = createRecordPublicationRecovery(sdk, join(root, "private"));
    await assert.rejects(recovery.publish(input));
    const older = recovery.reconcile(captureId, ownerToken);
    while (!first) await new Promise(resolve => setImmediate(resolve));
    assert.equal((await recovery.reconcile(captureId, ownerToken)).state, "cancelled");
    first(pending(captureId, publicationId, input.planFile.planSha256));
    await assert.rejects(older, /unresolved/);
    assert.equal((await new SqlitePublicationBindingStore(join(root, "private")).get(captureId))?.state, "cancelled");
  } finally { await rm(root, { recursive: true, force: true }); }
});
