import assert from "node:assert/strict";
import test from "node:test";
import { createHash } from "node:crypto";
import { chmod, link, mkdir, mkdtemp, open, readFile, realpath, rename, rm, symlink, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { decodeJob } from "../../src/queue/job.ts";
import { openCommittedEncryptedSpool, EncryptedSpoolReadError } from "../../src/sender/spool-reader.ts";
import type { NamespaceResources } from "../../src/state/namespace.ts";
import { consumePrivateFile, consumePrivateSealedAsset, PrivateFileError } from "../../src/state/private-file.ts";

const id = "a".repeat(32), generation = "b".repeat(32);
const context = { origin: "https://connect.example/connect", accountId: "account", captureId: id, senderGrantId: "sender" };
const sha = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
async function fixture(size = 3) {
  const root = await mkdtemp(path.join(await realpath(os.tmpdir()), "connect-spool-reader-"));
  const spool = path.join(root, `${id}.sealed`); await mkdir(path.join(spool, "sealed"), { recursive: true, mode: 0o700 });
  const contents = new Map([["envelope.json", Buffer.from("synthetic envelope; decoder does not verify signatures")],
    ["sealed/part-000000.age", Buffer.alloc(size, 7)]]);
  const files = [...contents].map(([name, bytes]) => ({ path: name, size: bytes.length, sha256: sha(bytes) }));
  const receipt = { metadata: { context, recipientGrants: ["reader"] }, files };
  const credential = { server: context.origin, scope: "upload", token: "synthetic-secret", generation,
    grantId: "sender", refreshToken: "synthetic-refresh", expiresAt: 1000, refreshExpiresAt: 2000, encryption_identity: "identity" };
  const job = decodeJob(Buffer.from(JSON.stringify({ id, bundle: "/synthetic.apparent", server: context.origin, generation,
    created: 1, attempts: 0, nextAttempt: 0, status: "pending", encrypted: true, files, encryption: receipt.metadata })));
  const put = (name: string, value: unknown) => writeFile(path.join(root, name), JSON.stringify(value), { mode: 0o600 });
  await put(`${id}.job`, job); await put("upload.credential", credential);
  await put(`${id}.sealed/spool-state.json`, receipt);
  for (const [name, bytes] of contents) await writeFile(path.join(spool, name), bytes, { mode: 0o600 });
  let leases = 0;
  const resources: NamespaceResources = {
    async checkAcl(handle) { await handle.close(); return { state: "accepted" }; },
    async lock(handle, { signal }) {
      await handle.close(); if (signal?.aborted) return { state: "cancelled" };
      leases++; let held = true;
      return { state: "acquired", lease: { signal: signal ?? new AbortController().signal,
        assertHeld() { assert.ok(held); }, async release() { assert.ok(held); held = false; leases--; } } };
    },
  };
  return { root, spool, job, receipt, credential, contents, resources, put, leases: () => leases,
    close: () => rm(root, { recursive: true, force: true }) };
}
const category = (name: string) => (error: unknown) => error instanceof EncryptedSpoolReadError && error.category === name && error.message === name;

test("reads a committed Rust-format spool through private descriptors without product JSON writes", async () => {
  const f = await fixture(); try {
    const before = await readFile(path.join(f.spool, "spool-state.json"));
    const snapshot = await openCommittedEncryptedSpool(f.root, f.job, f.resources);
    assert.equal(f.leases(), 0); assert.ok(Object.isFrozen(snapshot.record));
    for (const [name, expected] of f.contents) {
      const bytes = await snapshot.readAsset(name); assert.deepEqual(bytes, new Uint8Array(expected)); bytes.fill(0);
      assert.deepEqual(await readFile(path.join(f.spool, name)), expected);
    }
    assert.deepEqual(await readFile(path.join(f.spool, "spool-state.json")), before);
    await assert.rejects(snapshot.readAsset("spool-state.json"), category("invalid_encrypted_spool"));
    assert.equal(f.leases(), 0);
  } finally { await f.close(); }
});

test("Rust None group values normalize consistently across committed job and spool receipt", async () => {
  const f = await fixture(); try {
    const wire = JSON.parse(JSON.stringify(f.job)); wire.encryption.context.group = null;
    await f.put(`${id}.job`, wire);
    await f.put(`${id}.sealed/spool-state.json`, { ...f.receipt,
      metadata: { ...f.receipt.metadata, context: { ...context, group: null } } });
    const job = decodeJob(Buffer.from(JSON.stringify(wire)));
    const snapshot = await openCommittedEncryptedSpool(f.root, job, f.resources);
    assert.equal(snapshot.record.metadata.context.group, undefined);
    const bytes = await snapshot.readAsset("envelope.json"); bytes.fill(0);
  } finally { await f.close(); }
});

test("sealed asset reads exceed the JSON ceiling while retaining data-hardlink compatibility", async () => {
  const f = await fixture(2_000_001); try {
    const name = "sealed/part-000000.age";
    await link(path.join(f.spool, name), path.join(f.spool, "saved-ciphertext"));
    const snapshot = await openCommittedEncryptedSpool(f.root, f.job, f.resources);
    const bytes = await snapshot.readAsset(name); assert.equal(bytes.length, 2_000_001); assert.equal(bytes[0], 7); bytes.fill(0);
    const handle = await open(path.join(f.spool, name), "r");
    await assert.rejects(consumePrivateFile(handle, { maximumBytes: 2_000_001 }, f.resources, () => null),
      e => e instanceof PrivateFileError && e.category === "unavailable");
    await assert.rejects(handle.stat(), { code: "EBADF" });
    const oversized = await open(path.join(f.spool, name), "r");
    await assert.rejects(consumePrivateSealedAsset(oversized, { maximumBytes: 32 * 1024 * 1024 + 1 }, f.resources),
      e => e instanceof PrivateFileError && e.category === "unavailable");
    await assert.rejects(oversized.stat(), { code: "EBADF" });
  } finally { await f.close(); }
});

test("changed ciphertext and non-private asset modes cannot return bytes", async () => {
  const f = await fixture(); try {
    const snapshot = await openCommittedEncryptedSpool(f.root, f.job, f.resources);
    const file = path.join(f.spool, "sealed/part-000000.age");
    await writeFile(file, Uint8Array.of(9, 9, 9));
    await assert.rejects(snapshot.readAsset("sealed/part-000000.age"), category("unsafe_file"));
    await writeFile(file, f.contents.get("sealed/part-000000.age")!); await chmod(file, 0o644);
    await assert.rejects(snapshot.readAsset("sealed/part-000000.age"), category("unsafe_file"));
    assert.equal(f.leases(), 0);
  } finally { await f.close(); }
});

test("cancellation, revocation, job mutation and a new login fence existing spool handles", async () => {
  const f = await fixture(); try {
    const snapshot = await openCommittedEncryptedSpool(f.root, f.job, f.resources);
    const abort = new AbortController(); abort.abort();
    await assert.rejects(snapshot.readAsset("envelope.json", abort.signal), category("cancelled"));
    await f.put("cancellation", "new-epoch");
    await assert.rejects(snapshot.readAsset("envelope.json"), category("stale_snapshot"));
    await rm(path.join(f.root, "cancellation")); await f.put("revocation-test.intent", {});
    await assert.rejects(snapshot.readAsset("envelope.json"), category("revocation_pending"));
    await rm(path.join(f.root, "revocation-test.intent")); await f.put(`${id}.job`, { ...f.job, status: "cancelled" });
    await assert.rejects(snapshot.readAsset("envelope.json"), category("stale_snapshot"));
    await f.put(`${id}.job`, f.job); await f.put("upload.credential", { ...f.credential, generation: "c".repeat(32) });
    await assert.rejects(snapshot.readAsset("envelope.json"), category("stale_snapshot"));
    assert.equal(f.leases(), 0);
  } finally { await f.close(); }
});

test("incomplete publication, changed receipt, substituted directories and symlink assets fail closed", async () => {
  const f = await fixture(); try {
    await assert.rejects(openCommittedEncryptedSpool(f.root, { ...f.job, files: [] }, f.resources), category("invalid_encrypted_spool"));
    const snapshot = await openCommittedEncryptedSpool(f.root, f.job, f.resources);
    await f.put(`${id}.sealed/spool-state.json`, { ...f.receipt, metadata: { ...f.receipt.metadata, recipientGrants: ["other"] } });
    await assert.rejects(snapshot.readAsset("envelope.json"), category("stale_snapshot"));
    await f.put(`${id}.sealed/spool-state.json`, f.receipt);
    const fresh = await openCommittedEncryptedSpool(f.root, f.job, f.resources);
    await rm(path.join(f.spool, "envelope.json")); await symlink("spool-state.json", path.join(f.spool, "envelope.json"));
    await assert.rejects(fresh.readAsset("envelope.json"));
    await rename(f.spool, f.spool + "-old"); await mkdir(f.spool, { mode: 0o700 });
    await assert.rejects(fresh.readAsset("envelope.json"), category("stale_snapshot"));
    assert.equal(f.leases(), 0);
  } finally { await f.close(); }
});

test("provider failures are redacted and release failures cannot return asset bytes", async () => {
  const f = await fixture(); try {
    const bad = { ...f.resources, async checkAcl(handle: import("node:fs/promises").FileHandle): Promise<{ state: "accepted" }> {
      await handle.close(); throw Error("synthetic-secret /private/spool"); } };
    await assert.rejects(openCommittedEncryptedSpool(f.root, f.job, bad), error => error instanceof EncryptedSpoolReadError && !String(error).includes("synthetic-secret"));
    const resources = { ...f.resources }; const snapshot = await openCommittedEncryptedSpool(f.root, f.job, resources);
    resources.lock = async handle => { await handle.close(); return { state: "acquired", lease: { signal: new AbortController().signal,
      assertHeld() {}, async release() { throw Error("private-release-error"); } } }; };
    await assert.rejects(snapshot.readAsset("envelope.json"), category("state_unavailable"));
  } finally { await f.close(); }
});
