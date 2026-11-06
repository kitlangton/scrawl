import assert from "node:assert/strict";
import test from "node:test";
import { mkdir, mkdtemp, lstat, readFile, readdir, realpath, rename, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import * as ed25519 from "@noble/ed25519";
import { generateAgeIdentity } from "../../src/recipient/crypto.ts";
import { decodeJob, encodeJob } from "../../src/queue/job.ts";
import { sealCapture } from "../../src/sender/seal.ts";
import { readCredentialSnapshot } from "../../src/state/namespace.ts";
import { openCommittedEncryptedSpool } from "../../src/sender/spool-reader.ts";
import { publishEncryptedSpool, SpoolPublicationError, type SpoolPublicationResources } from "../../src/sender/spool-publication.ts";

const id = "a".repeat(32), generation = "b".repeat(32), server = "https://connect.example/connect";
const category = (value: string) => (error: unknown) => error instanceof SpoolPublicationError && error.category === value && error.message === value;
async function fixture(grouped = false) {
  const root = await realpath(await mkdtemp(path.join(os.tmpdir(), "connect-spool-publish-")));
  let held = 0, dispatches = 0;
  const credential = { server, scope: "upload", generation, token: "synthetic-token", grantId: "sender", encryption_identity: "c".repeat(32),
    refreshToken: "synthetic-refresh", expiresAt: 2_000_000_000, refreshExpiresAt: 2_100_000_000 };
  const job = decodeJob(Buffer.from(JSON.stringify({ id, bundle: "/synthetic.apparent", server, generation,
    created: 1, attempts: 0, nextAttempt: 0, status: "pending", encrypted: true, grouped })));
  const put = (name: string, value: unknown) => writeFile(path.join(root, name), JSON.stringify(value), { mode: 0o600 });
  await put("upload.credential", credential); await writeFile(path.join(root, `${id}.job`), encodeJob(job), { mode: 0o600 });
  await put("config.json", { untouched: true, secret: "synthetic-config" });
  // Fixtures exercise orchestration; only the separate native suite proves flock/rename behavior.
  const resources: SpoolPublicationResources = {
    async checkAcl(handle) { await handle.close(); return { state: "accepted" }; },
    async lock(handle) { await handle.close(); assert.equal(held, 0); held++;
      return { state: "acquired", lease: { signal: new AbortController().signal,
        assertHeld() { assert.equal(held, 1); }, async release() { held--; } } }; },
    async publish(parent, stage, destination) {
      assert.equal(held, 1); dispatches++;
      try { await rename(path.join(root, stage), path.join(root, destination)); return { state: "published" }; }
      finally { await parent.close(); }
    },
  };
  const snapshot = await readCredentialSnapshot(root, { scope: "upload", approvedServer: server }, resources);
  const seed = new Uint8Array(32).fill(47), self = await generateAgeIdentity(), reader = await generateAgeIdentity();
  const signingKey = await ed25519.getPublicKeyAsync(seed);
  const ownPublicKey = { version: 1 as const, recipient: self.recipient, signing: Buffer.from(signingKey).toString("hex") };
  const sealed = await sealCapture({ server, captureId: id, ownPublicKey, signingSeed: seed, profile: grouped ? "evidence-first-v1" : "root",
    authority: { server, accountId: "account", grantId: "sender", scope: "upload", devices: [
      { grantId: "sender", scope: "upload", key: ownPublicKey },
      { grantId: "reader", scope: "read", key: { ...ownPublicKey, recipient: reader.recipient } }] },
    files: [{ path: "apparent.json", bytes: Buffer.from("{}") }, { path: "summary.md", bytes: Buffer.from("synthetic") },
      { path: "contact-sheet.png", bytes: Uint8Array.of(1, 2) }, ...(grouped ? [{ path: "audio.wav", bytes: Uint8Array.of(3, 4) }] : [])], active: () => true });
  const publish = (options = { active: () => true }) => publishEncryptedSpool(root, job, sealed, snapshot, signingKey, resources, options);
  return { root, job, sealed, snapshot, signingKey, resources, put, publish, held: () => held, dispatches: () => dispatches,
    async close() { assert.equal(held, 0); seed.fill(0); sealed.assets.forEach(asset => asset.bytes.fill(0)); await rm(root, { recursive: true, force: true }); } };
}

test("publishes root and grouped immutable spools before any job JSON commit", async () => {
  for (const grouped of [false, true]) {
    const f = await fixture(grouped); try {
      const before = await Promise.all([`${id}.job`, "upload.credential", "config.json"].map(name => readFile(path.join(f.root, name))));
      const outcome = await f.publish(); assert.equal(outcome.state, "committed");
      if (outcome.state !== "committed") return;
      assert.equal(f.dispatches(), 1); assert.equal(f.held(), 0);
      assert.deepEqual(JSON.parse(await readFile(path.join(outcome.path, "spool-state.json"), "utf8")), outcome.record);
      assert.equal((await lstat(outcome.path)).mode & 0o777, 0o700);
      for (const asset of f.sealed.assets) {
        assert.deepEqual(new Uint8Array(await readFile(path.join(outcome.path, asset.path))), asset.bytes);
        assert.equal((await lstat(path.join(outcome.path, asset.path))).mode & 0o777, 0o600);
      }
      for (const [i, name] of [`${id}.job`, "upload.credential", "config.json"].entries()) assert.deepEqual(await readFile(path.join(f.root, name)), before[i]);
      // Simulate the authoritative owner's later job commit solely in this synthetic fixture.
      const committedJob = { ...f.job, files: [...outcome.record.files], encryption: outcome.record.metadata };
      await f.put(`${id}.job`, committedJob);
      const reader = await openCommittedEncryptedSpool(f.root, committedJob, f.resources);
      for (const asset of f.sealed.assets) { const bytes = await reader.readAsset(asset.path); assert.deepEqual(bytes, asset.bytes); bytes.fill(0); }
    } finally { await f.close(); }
  }
});

test("staging holds no queue lease and rechecks cancellation immediately before dispatch", async () => {
  const f = await fixture(); try {
    const original = f.resources.checkAcl, rootInode = (await lstat(f.root)).ino;
    let changed = false;
    f.resources.checkAcl = async handle => {
      const stat = await handle.stat();
      if (!changed && stat.isDirectory() && stat.ino !== rootInode) {
        assert.equal(f.held(), 0); changed = true; await f.put("cancellation", "new-epoch");
      }
      return original(handle);
    };
    await assert.rejects(f.publish(), category("stale_snapshot"));
    assert.equal(changed, true); assert.equal(f.dispatches(), 0);
    assert.equal((await readdir(f.root)).some(name => name.startsWith(".apparent-stage-") || name.endsWith(".sealed")), false);
  } finally { await f.close(); }
});

test("refuses stale jobs, login changes, revocation, existing destinations and altered ciphertext", async () => {
  for (const change of ["job", "login", "revocation", "existing", "bytes"]) {
    const f = await fixture(); try {
      if (change === "job") await f.put(`${id}.job`, { ...f.job, status: "cancelled" });
      if (change === "login") await f.put("upload.credential", { server, scope: "upload", token: "replacement", generation: "d".repeat(32) });
      if (change === "revocation") await f.put("revocation-test.intent", {});
      if (change === "existing") { await mkdir(path.join(f.root, `${id}.sealed`), { mode: 0o700 }); await f.put(`${id}.sealed/sentinel`, "keep"); }
      if (change === "bytes") f.sealed.assets[0]!.bytes.fill(0);
      await assert.rejects(f.publish(), category(change === "revocation" ? "revocation_pending" : change === "existing" ? "destination_exists" : change === "bytes" ? "invalid_spool" : "stale_snapshot"));
      assert.equal(f.dispatches(), 0);
      if (change === "existing") assert.equal(JSON.parse(await readFile(path.join(f.root, `${id}.sealed/sentinel`), "utf8")), "keep");
    } finally { await f.close(); }
  }
});

test("uncertain acknowledgement and lease release failure preserve the spool without retry", async () => {
  for (const phase of ["acknowledgement", "release", "readback"]) {
    const f = await fixture(); try {
      if (phase === "acknowledgement") {
        const original = f.resources.publish;
        f.resources.publish = async (...args) => { await original(...args); throw Error("private-secret-path"); };
      } else if (phase === "release") {
        const original = f.resources.lock;
        f.resources.lock = async (...args) => {
          const result = await original(...args);
          if (result.state === "acquired") {
            const release = result.lease.release;
            result.lease.release = async () => { await release(); if (f.dispatches()) throw Error("private-release"); };
          }
          return result;
        };
      } else {
        const original = f.resources.publish;
        f.resources.publish = async (...args) => { const result = await original(...args);
          await f.put(`${id}.sealed/spool-state.json`, {}); return result; };
      }
      assert.deepEqual(await f.publish(), { state: "publication-uncertain" });
      assert.equal(f.dispatches(), 1); assert.ok((await lstat(path.join(f.root, `${id}.sealed`))).isDirectory());
      assert.equal(JSON.parse(await readFile(path.join(f.root, `${id}.job`), "utf8")).files.length, 0);
    } finally { await f.close(); }
  }
});

test("a definitive no-effect refusal cleans only its stage, without retry", async () => {
  const f = await fixture(); try {
    f.resources.publish = async parent => { await parent.close(); return { state: "refused" }; };
    await assert.rejects(f.publish(), category("publication_refused"));
    assert.equal((await readdir(f.root)).some(name => name.startsWith(".apparent-stage-") || name.endsWith(".sealed")), false);
    const abort = new AbortController(); abort.abort();
    await assert.rejects(publishEncryptedSpool(f.root, f.job, f.sealed, f.snapshot, f.signingKey, f.resources,
      { active: () => true, signal: abort.signal }), category("cancelled"));
  } finally { await f.close(); }
});
