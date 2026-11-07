import assert from "node:assert/strict";
import test from "node:test";
import { createHash } from "node:crypto";
import { link, lstat, mkdtemp, readFile, realpath, rename, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import * as ed25519 from "@noble/ed25519";
import { generateAgeIdentity } from "../../src/recipient/crypto.ts";
import { decodeJob, encodeJob, type Job } from "../../src/queue/job.ts";
import { sealCapture } from "../../src/sender/seal.ts";
import { publishEncryptedSpool, type SpoolPublicationResources } from "../../src/sender/spool-publication.ts";
import { commitEncryptedSpool, EncryptedSpoolCommitError, type EncryptedSpoolCommitResources } from "../../src/sender/spool-commit.ts";
import { readCredentialSnapshot } from "../../src/state/namespace.ts";

const id = "a".repeat(32), generation = "b".repeat(32), server = "https://connect.example/connect";
const category = (value: string) => (error: unknown) => error instanceof EncryptedSpoolCommitError && error.category === value && error.message === value;
const hash = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
async function fixture(grouped = false) {
  const root = await realpath(await mkdtemp(path.join(os.tmpdir(), "connect-spool-commit-")));
  let held = 0, replacements = 0, committing = false, commitLocks = 0, assetReads = 0;
  let beforeFinal: (() => Promise<void>) | undefined;
  const assetInodes = new Set<string>();
  const credential = { server, scope: "upload", generation, token: "synthetic-token", grantId: "sender", encryption_identity: "c".repeat(32),
    refreshToken: "synthetic-refresh", expiresAt: 2_000_000_000, refreshExpiresAt: 2_100_000_000 };
  const job = decodeJob(Buffer.from(JSON.stringify({ id, bundle: "/synthetic.apparent", server, generation, created: 1,
    attempts: 0, nextAttempt: 0, status: "pending", encrypted: true, grouped, files: [], encryption: null, futureField: { keep: 7 } })));
  const put = (name: string, value: unknown) => writeFile(path.join(root, name), JSON.stringify(value), { mode: 0o600 });
  await put("upload.credential", credential); await writeFile(path.join(root, `${id}.job`), encodeJob(job), { mode: 0o600 });
  await put("config.json", { unchanged: "synthetic-config" });
  // These ports test policy only; the separate native lane proves the primitives.
  const resources: EncryptedSpoolCommitResources & SpoolPublicationResources = {
    async checkAcl(handle) { const stat = await handle.stat({ bigint: true }); await handle.close();
      if (committing && assetInodes.has(`${stat.dev}:${stat.ino}`)) { assert.equal(held, 0, "ciphertext hashing must be outside queue.lock"); assetReads++; }
      return { state: "accepted" }; },
    async lock(handle) { await handle.close(); assert.equal(held, 0);
      if (committing && ++commitLocks === 2 && beforeFinal) await beforeFinal();
      held++; return { state: "acquired", lease: { signal: new AbortController().signal,
        assertHeld() { assert.equal(held, 1); }, async release() { held--; } } }; },
    async publish(parent, stage, destination) {
      try { await rename(path.join(root, stage), path.join(root, destination)); return { state: "published" }; }
      finally { await parent.close(); }
    },
    async replaceFile(parent, source, destination, expected) {
      assert.equal(held, 1); replacements++;
      try {
        const target = path.join(root, destination), current = await lstat(target, { bigint: true });
        assert.deepEqual({ device: String(current.dev), inode: String(current.ino) }, expected.destination);
        await rename(path.join(root, source), target); return { state: "replaced" };
      } finally { await parent.close(); }
    },
  };
  const snapshot = await readCredentialSnapshot(root, { scope: "upload", approvedServer: server }, resources);
  const seed = new Uint8Array(32).fill(47), self = await generateAgeIdentity(), reader = await generateAgeIdentity();
  const signingKey = await ed25519.getPublicKeyAsync(seed);
  const ownPublicKey = { version: 1 as const, recipient: self.recipient, signing: Buffer.from(signingKey).toString("hex") };
  const authority = { server, accountId: "account", grantId: "sender", scope: "upload" as const, devices: [
    { grantId: "sender", scope: "upload" as const, key: ownPublicKey },
    { grantId: "reader", scope: "read" as const, key: { ...ownPublicKey, recipient: reader.recipient } }] };
  const sealed = await sealCapture({ server, captureId: id, ownPublicKey, signingSeed: seed, profile: grouped ? "evidence-first-v1" : "root",
    authority, files: [{ path: "apparent.json", bytes: Buffer.from("{}") }, { path: "summary.md", bytes: Buffer.from("synthetic") },
      { path: "contact-sheet.png", bytes: Uint8Array.of(1, 2) }, ...(grouped ? [{ path: "audio.wav", bytes: Uint8Array.of(3) }] : [])], active: () => true });
  const published = await publishEncryptedSpool(root, job, sealed, snapshot, signingKey, resources, { active: () => true });
  assert.equal(published.state, "committed"); if (published.state !== "committed") throw Error("fixture publication failed");
  for (const asset of sealed.assets) { const stat = await lstat(path.join(published.path, asset.path), { bigint: true }); assetInodes.add(`${stat.dev}:${stat.ino}`); }
  const commit = async (expected: Job = job, peers = authority, options: { active(): boolean; signal?: AbortSignal } = { active: () => true }) => {
    committing = true; commitLocks = 0; assetReads = 0;
    try { return await commitEncryptedSpool(root, expected, snapshot, peers, ownPublicKey, resources, options); }
    finally { committing = false; }
  };
  return { root, job, sealed, snapshot, authority, ownPublicKey, resources, credential, commit, put, spool: published.path,
    afterHashing(callback: () => Promise<void>) { beforeFinal = callback; }, replacements: () => replacements, assetReads: () => assetReads,
    async close() { assert.equal(held, 0); seed.fill(0); sealed.assets.forEach(asset => asset.bytes.fill(0)); await rm(root, { recursive: true, force: true }); } };
}

test("recovers root and grouped orphan spools with exact job preservation and idempotence", async () => {
  for (const grouped of [false, true]) {
    const f = await fixture(grouped); try {
      const originalJob = await readFile(path.join(f.root, `${id}.job`));
      await link(path.join(f.root, `${id}.job`), path.join(f.root, "legacy-job-alias"));
      const names = ["upload.credential", "config.json", `${id}.sealed/spool-state.json`, ...f.sealed.assets.map(asset => `${id}.sealed/${asset.path}`)];
      const before = await Promise.all(names.map(name => readFile(path.join(f.root, name))));
      const result = await f.commit(); assert.equal(result.state, "committed"); if (result.state === "uncertain") return;
      assert.deepEqual(result.job, { ...f.job, files: result.record.files, encryption: result.record.metadata });
      assert.ok(Object.isFrozen(result.job.futureField)); assert.equal(f.replacements(), 1); assert.equal(f.assetReads(), f.sealed.assets.length);
      assert.deepEqual(JSON.parse(await readFile(path.join(f.root, `${id}.job`), "utf8")), result.job);
      assert.deepEqual(await readFile(path.join(f.root, "legacy-job-alias")), originalJob);
      for (const [i, name] of names.entries()) assert.deepEqual(await readFile(path.join(f.root, name)), before[i]);
      const extra = { ...f.authority, devices: [...f.authority.devices, { ...f.authority.devices[0]!, grantId: "unrelated-new-peer" }] };
      assert.equal((await f.commit(result.job, extra)).state, "unchanged"); assert.equal(f.replacements(), 1);
      const reordered = { ...result.job, files: [...result.job.files].reverse(),
        encryption: { ...result.job.encryption!, recipientGrants: [...result.job.encryption!.recipientGrants].reverse() } };
      await f.put(`${id}.job`, reordered);
      const unchanged = await f.commit(reordered);
      assert.equal(unchanged.state, "unchanged"); if (unchanged.state === "uncertain") return;
      assert.deepEqual(unchanged.job, reordered, "no-op result must preserve the stored job's exact next-call fence");
      const conflicting = { ...result.job, files: result.job.files.map((file, i) => i ? file : { ...file, sha256: "0".repeat(64) }) };
      await f.put(`${id}.job`, conflicting);
      await assert.rejects(f.commit(conflicting), category("invalid_spool")); assert.equal(f.replacements(), 1);
    } finally { await f.close(); }
  }
});

test("revoked recipients require a fresh capture instead of rewriting ciphertext", async () => {
  const f = await fixture(); try {
    const before = await readFile(path.join(f.root, `${id}.job`));
    await assert.rejects(f.commit(f.job, { ...f.authority, devices: f.authority.devices.filter(peer => peer.scope !== "read") }), category("encryption_restage_required"));
    assert.equal(f.replacements(), 0); assert.deepEqual(await readFile(path.join(f.root, `${id}.job`)), before);
  } finally { await f.close(); }
});

test("final lease refuses consumed asset, receipt, intent and login substitutions after hashing", async () => {
  for (const change of ["asset", "receipt", "job", "cancellation", "revocation", "login", "consent", "abort"]) {
    const f = await fixture(); try {
      const before = await readFile(path.join(f.root, `${id}.job`));
      let allowed = true; const abort = new AbortController();
      f.afterHashing(async () => {
        if (change === "asset") {
          const file = path.join(f.spool, "sealed/part-000000.age"), bytes = await readFile(file);
          await writeFile(file + ".replacement", bytes, { mode: 0o600 }); await rename(file + ".replacement", file);
        }
        if (change === "receipt") { const file = path.join(f.spool, "spool-state.json"); await writeFile(file, (await readFile(file, "utf8")) + "\n"); }
        if (change === "job") await f.put(`${id}.job`, { ...f.job, status: "cancelled" });
        if (change === "cancellation") await f.put("cancellation", "new-epoch");
        if (change === "revocation") await f.put("revocation-test.intent", {});
        if (change === "login") await f.put("upload.credential", { ...f.credential, generation: "d".repeat(32) });
        if (change === "consent") allowed = false;
        if (change === "abort") abort.abort();
      });
      await assert.rejects(f.commit(f.job, f.authority, { active: () => allowed, signal: abort.signal }),
        category(change === "revocation" ? "revocation_pending" : change === "abort" ? "cancelled" : "stale_snapshot"));
      assert.equal(f.replacements(), 0);
      if (change !== "job") assert.deepEqual(await readFile(path.join(f.root, `${id}.job`)), before);
    } finally { await f.close(); }
  }
});

test("lease cancellation and contention fail before any replacement", async () => {
  for (const kind of ["busy", "cancelled"] as const) {
    const f = await fixture(); try {
      const original = f.resources.lock;
      f.resources.lock = async (...args) => {
        if (kind === "busy") { await args[0].close(); return { state: "busy" }; }
        const result = await original(...args);
        if (result.state === "acquired") return { ...result, lease: { ...result.lease, signal: AbortSignal.abort() } };
        return result;
      };
      await assert.rejects(f.commit(), category(kind === "cancelled" ? "cancelled" : "state_unavailable"));
      assert.equal(f.replacements(), 0);
    } finally { await f.close(); }
  }
});

test("invalid signatures and ciphertext never reach job replacement", async () => {
  for (const change of ["signature", "ciphertext"]) {
    const f = await fixture(); try {
      if (change === "ciphertext") await writeFile(path.join(f.spool, "sealed/part-000000.age"), Uint8Array.of(0));
      else {
        const file = path.join(f.spool, "envelope.json"), envelope = JSON.parse(await readFile(file, "utf8"));
        envelope.signature = "0".repeat(128); const bytes = Buffer.from(JSON.stringify(envelope)); await writeFile(file, bytes);
        const receipt = JSON.parse(await readFile(path.join(f.spool, "spool-state.json"), "utf8"));
        const asset = receipt.files.find((asset: { path: string }) => asset.path === "envelope.json"); asset.size = bytes.length; asset.sha256 = hash(bytes);
        await f.put(`${id}.sealed/spool-state.json`, receipt);
      }
      await assert.rejects(f.commit(), error => error instanceof EncryptedSpoolCommitError && ["invalid_spool", "unsafe_file"].includes(error.category));
      assert.equal(f.replacements(), 0);
    } finally { await f.close(); }
  }
});

test("uncertain acknowledgement and release failure retain effects without retry", async () => {
  for (const phase of ["acknowledgement", "release", "refusal"]) {
    const f = await fixture(); try {
      const original = f.resources.replaceFile; let calls = 0;
      f.resources.replaceFile = async (...args) => {
        calls++;
        if (phase === "refusal") { await args[0].close(); return { state: "refused" }; }
        const result = await original(...args); if (phase === "acknowledgement") throw Error("private-secret-path"); return result;
      };
      if (phase === "release") {
        const lock = f.resources.lock;
        f.resources.lock = async (...args) => {
          const result = await lock(...args);
          if (result.state === "acquired") { const release = result.lease.release;
            result.lease.release = async () => { await release(); if (calls) throw Error("private-release"); }; }
          return result;
        };
      }
      if (phase === "refusal") await assert.rejects(f.commit(), category("stale_snapshot"));
      else assert.deepEqual(await f.commit(), { state: "uncertain" });
      assert.equal(calls, 1); assert.ok((await lstat(f.spool)).isDirectory());
      const saved = JSON.parse(await readFile(path.join(f.root, `${id}.job`), "utf8"));
      assert.equal(saved.files.length > 0, phase !== "refusal");
    } finally { await f.close(); }
  }
});

test("captures caller facts before callbacks and refuses cancellation", async () => {
  const f = await fixture(); try {
    const mutable = decodeJob(encodeJob(f.job));
    const result = await f.commit(mutable, f.authority, { active: () => { mutable.server = "https://changed.invalid"; return true; } });
    assert.equal(result.state, "committed"); if (result.state !== "uncertain") assert.equal(result.job.server, server);
    const abort = new AbortController(); abort.abort();
    await assert.rejects(commitEncryptedSpool(f.root, f.job, f.snapshot, f.authority, f.ownPublicKey, f.resources,
      { active: () => true, signal: abort.signal }), category("cancelled"));
  } finally { await f.close(); }
});
