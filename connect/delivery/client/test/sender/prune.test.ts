import assert from "node:assert/strict";
import test from "node:test";
import { link, lstat, mkdir, mkdtemp, readFile, realpath, readdir, rename, rm, symlink, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import type { FileHandle } from "node:fs/promises";
import { decodeJob, encodeJob, type Job } from "../../src/queue/job.ts";
import { retireQueuedJob, resumeQueuedRetirement, type QueueRetirementResources } from "../../src/queue/prune.ts";
import { admitTrustedDirectory } from "../../src/state/directory.ts";
import { stateFileStamp } from "../../src/state/credential-files.ts";

const id = "a".repeat(32), server = "https://connect.example/connect", generation = "b".repeat(32);
const context = { origin: server, accountId: "account", captureId: id, senderGrantId: "sender" };
const plainJob = () => decodeJob(Buffer.from(JSON.stringify({ id, bundle: "/synthetic.apparent", server, generation, created: 1,
  attempts: 0, nextAttempt: 0, status: "cancelled" })));
const encryptedJob = (grouped = false, complete = false) => decodeJob(Buffer.from(JSON.stringify({ id, bundle: "/synthetic.apparent", server, generation, created: 1,
  attempts: 0, nextAttempt: 0, status: "expired", encrypted: true, grouped, encryption: { context, recipientGrants: ["sender"] },
  ...(complete ? { files: (grouped ? ["evidence/", "media/"] : [""]).flatMap(prefix => [
    { path: `${prefix}envelope.json`, size: 2, sha256: "c".repeat(64) },
    { path: `${prefix}sealed/part-000000.age`, size: 10, sha256: "c".repeat(64) },
  ]) } : {}) })));
const resources = (): QueueRetirementResources => ({
  async checkAcl(handle: FileHandle) { await handle.close(); return { state: "accepted" }; },
  async lock(handle: FileHandle) { await handle.close(); return { state: "refused" }; },
});
async function fixture(job: Job) {
  const temp = await realpath(await mkdtemp(path.join(os.tmpdir(), "connect-prune-"))), root = path.join(temp, "state");
  await mkdir(root, { mode: 0o700 });
  const jobPath = path.join(root, `${id}.job`);
  await writeFile(jobPath, encodeJob(job), { mode: 0o600 });
  const r = resources(), admitted = await admitTrustedDirectory(root, r, "private-state");
  let fresh = 0, freshAction: (() => Promise<void>) | undefined;
  const lease = { signal: new AbortController().signal, assertHeld() {}, async assertCurrent() {}, async release() {} };
  const stamp = stateFileStamp(await lstat(jobPath, { bigint: true }));
  const retire = (expected = job, assertFresh = async () => { fresh++; await freshAction?.(); }, activeLease = lease, expectedStamp = stamp) =>
    retireQueuedJob({ handle: admitted.handle, path: root, identity: admitted.identity }, activeLease, r,
      { job: expected, expectedStamp, assertFresh });
  return { temp, root, jobPath, resources: r, retire, lease, directory: { handle: admitted.handle, path: root, identity: admitted.identity },
    freshCount: () => fresh, setFreshAction(action: () => Promise<void>) { freshAction = action; },
    async close() { await admitted.handle.close(); await rm(temp, { recursive: true, force: true }); } };
}
async function writeSpool(root: string, grouped: boolean) {
  const spool = path.join(root, `${id}.sealed`), files: Array<{ path: string; size: number; sha256: string }> = [];
  await mkdir(spool, { mode: 0o700 });
  for (const group of grouped ? ["evidence", "media"] : [""]) {
    const prefix = group ? `${group}/` : "", base = path.join(spool, group);
    if (group) await mkdir(base, { mode: 0o700 });
    const sealed = path.join(base, "sealed"); await mkdir(sealed, { mode: 0o700 });
    for (const [name, bytes] of [["envelope.json", Buffer.from("{}")], ["sealed/part-000000.age", Buffer.from("ciphertext")]] as const) {
      const target = path.join(spool, prefix, name); await writeFile(target, bytes, { mode: 0o600 });
      files.push({ path: prefix + name, size: bytes.length, sha256: "c".repeat(64) });
    }
  }
  await writeFile(path.join(spool, "spool-state.json"), JSON.stringify({ metadata: { context, recipientGrants: ["sender"] }, files }), { mode: 0o600 });
  return spool;
}

test("retires plaintext job and exact spool names while preserving aliases and unrelated state", async () => {
  for (const grouped of [false, true]) {
    const job = encryptedJob(grouped), f = await fixture(job);
    try {
      const spool = await writeSpool(f.root, grouped);
      const alias = path.join(f.root, "kept-alias");
      await link(path.join(spool, grouped ? "evidence/envelope.json" : "envelope.json"), alias);
      await writeFile(path.join(f.root, "config.json"), "private-config", { mode: 0o600 });
      await writeFile(path.join(f.temp, "bundle.apparent"), "original-bundle");
      assert.deepEqual(await f.retire(), { state: "retired" });
      await assert.rejects(lstat(f.jobPath), { code: "ENOENT" });
      await assert.rejects(lstat(spool), { code: "ENOENT" });
      assert.equal((await readFile(alias)).toString(), "{}");
      assert.equal((await readFile(path.join(f.root, "config.json"))).toString(), "private-config");
      assert.equal((await readFile(path.join(f.temp, "bundle.apparent"))).toString(), "original-bundle");
    } finally { await f.close(); }
  }
  const f = await fixture(plainJob());
  try {
    assert.deepEqual(await f.retire(), { state: "retired" });
    await assert.rejects(lstat(f.jobPath), { code: "ENOENT" });
  } finally { await f.close(); }
});

test("stale job, lost lease, cancellation and fresh-policy denial refuse before effects", async () => {
  for (const deny of ["fresh", "lease", "cancel", "stale-job"]) {
    const f = await fixture(plainJob());
    try {
      const lease = { signal: deny === "cancel" ? AbortSignal.abort() : new AbortController().signal,
        assertHeld() { if (deny === "lease") throw new Error(); },
        async assertCurrent() { if (deny === "lease" || deny === "cancel") throw new Error(); }, async release() {} };
      if (deny === "fresh") f.setFreshAction(async () => { throw new Error("private diagnostics"); });
      const originalStamp = stateFileStamp(await lstat(f.jobPath, { bigint: true }));
      if (deny === "stale-job") await writeFile(f.jobPath, encodeJob({ ...plainJob(), status: "ready" }), { mode: 0o600 });
      const expected = plainJob(), stamp = deny === "stale-job" ? originalStamp : stateFileStamp(await lstat(f.jobPath, { bigint: true }));
      const outcome = await f.retire(expected, async () => { if (deny === "fresh") throw Error(); }, lease, stamp);
      assert.equal(outcome.state, "refused");
      assert.ok((await lstat(f.jobPath)).isFile());
    } finally { await f.close(); }
  }
});

test("rejects symlink and unknown spool entries without deleting anything", async () => {
  for (const corrupt of ["symlink", "extra", "mode"]) {
    const f = await fixture(encryptedJob());
    try {
      const spool = await writeSpool(f.root, false);
      if (corrupt === "symlink") { await rm(path.join(spool, "envelope.json")); await symlink("spool-state.json", path.join(spool, "envelope.json")); }
      if (corrupt === "extra") await writeFile(path.join(spool, "unexpected"), "keep", { mode: 0o600 });
      if (corrupt === "mode") await (await import("node:fs/promises")).chmod(spool, 0o755);
      assert.deepEqual(await f.retire(), { state: "refused" });
      assert.ok((await lstat(f.jobPath)).isFile()); assert.ok((await lstat(spool)).isDirectory());
      assert.equal((await readdir(spool)).includes("unexpected"), corrupt !== "extra" ? false : true);
    } finally { await f.close(); }
  }
});

test("policy failure after first spool unlink reports uncertainty and retains the job", async () => {
  const f = await fixture(encryptedJob());
  try {
    const spool = await writeSpool(f.root, false); let checks = 0;
    f.setFreshAction(async () => { checks++; if (checks === 3) throw Error("fence denied"); });
    assert.deepEqual(await f.retire(), { state: "uncertain" });
    assert.ok((await lstat(f.jobPath)).isFile());
    assert.ok((await readdir(spool)).length > 0);
  } finally { await f.close(); }
});

test("a file or spool root substituted by a fresh fence is retained", async () => {
  for (const target of ["file", "root"]) {
    const f = await fixture(encryptedJob());
    try {
      const spool = await writeSpool(f.root, false); let checks = 0;
      f.setFreshAction(async () => {
        checks++;
        if (checks !== 2) return;
        if (target === "file") {
          const name = path.join(spool, "sealed/part-000000.age");
          await rename(name, name + ".saved");
          await writeFile(name, "replacement", { mode: 0o600 });
        } else {
          await rename(spool, spool + ".saved");
          await mkdir(spool, { mode: 0o700 });
        }
      });
      assert.deepEqual(await f.retire(), { state: "refused" });
      assert.ok((await lstat(f.jobPath)).isFile());
      if (target === "file") assert.equal((await readFile(path.join(spool, "sealed/part-000000.age"))).toString(), "replacement");
      else assert.ok((await lstat(spool)).isDirectory());
    } finally { await f.close(); }
  }
});

test("explicit recovery finishes only known missing assets, receipt, and pinned empty directories", async () => {
  for (const cut of ["asset", "receipt", "dirs", "nested", "group"]) {
    const job = encryptedJob(true, true), f = await fixture(job);
    try {
      const spool = await writeSpool(f.root, true);
      if (cut === "asset") await (await import("node:fs/promises")).unlink(path.join(spool, "evidence/sealed/part-000000.age"));
      if (cut === "receipt") await (await import("node:fs/promises")).unlink(path.join(spool, "spool-state.json"));
      if (cut === "dirs") {
        for (const file of ["evidence/envelope.json", "evidence/sealed/part-000000.age", "media/envelope.json", "media/sealed/part-000000.age", "spool-state.json"])
          await (await import("node:fs/promises")).unlink(path.join(spool, file));
      }
      if (cut === "nested") {
        await (await import("node:fs/promises")).unlink(path.join(spool, "evidence/sealed/part-000000.age"));
        await (await import("node:fs/promises")).rmdir(path.join(spool, "evidence/sealed"));
      }
      if (cut === "group") {
        for (const file of ["evidence/envelope.json", "evidence/sealed/part-000000.age"])
          await (await import("node:fs/promises")).unlink(path.join(spool, file));
        await (await import("node:fs/promises")).rmdir(path.join(spool, "evidence/sealed"));
        await (await import("node:fs/promises")).rmdir(path.join(spool, "evidence"));
      }
      assert.deepEqual(await f.retire(), { state: "refused" });
      assert.deepEqual(await resumeQueuedRetirement(f.directory, f.lease, f.resources,
        { job, expectedStamp: stateFileStamp(await lstat(f.jobPath, { bigint: true })), assertFresh: async () => {} }), { state: "retired" }, cut);
      await assert.rejects(lstat(f.jobPath), { code: "ENOENT" });
      await assert.rejects(lstat(spool), { code: "ENOENT" });
    } finally { await f.close(); }
  }
});

test("recovery refuses an uncommitted orphan manifest and an inconsistent surviving receipt", async () => {
  for (const mismatch of ["orphan", "receipt"]) {
    const job = encryptedJob(false, mismatch !== "orphan"), f = await fixture(job);
    try {
      const spool = await writeSpool(f.root, false);
      if (mismatch === "receipt") await writeFile(path.join(spool, "spool-state.json"), JSON.stringify({
        metadata: { context, recipientGrants: ["sender", "other"] },
        files: [
          { path: "envelope.json", size: 2, sha256: "c".repeat(64) },
          { path: "sealed/part-000000.age", size: 10, sha256: "c".repeat(64) },
        ],
      }), { mode: 0o600 });
      const expectedStamp = stateFileStamp(await lstat(f.jobPath, { bigint: true }));
      assert.deepEqual(await resumeQueuedRetirement(f.directory, f.lease, f.resources,
        { job, expectedStamp, assertFresh: async () => {} }), { state: "refused" });
      assert.ok((await lstat(f.jobPath)).isFile());
      assert.ok((await lstat(spool)).isDirectory());
      assert.ok((await lstat(path.join(spool, "envelope.json"))).isFile());
    } finally { await f.close(); }
  }
});


test("explicit recovery refuses unsafe survivors and fresh authority loss", async () => {
  for (const fault of ["extra", "symlink", "size", "acl", "fence"]) {
    const job = encryptedJob(false, true), f = await fixture(job);
    try {
      const spool = await writeSpool(f.root, false);
      if (fault === "extra") await writeFile(path.join(spool, "unknown"), "retain", { mode: 0o600 });
      if (fault === "symlink") { await rm(path.join(spool, "envelope.json")); await symlink("spool-state.json", path.join(spool, "envelope.json")); }
      if (fault === "size") await writeFile(path.join(spool, "envelope.json"), "wrong-size", { mode: 0o600 });
      if (fault === "acl") f.resources.checkAcl = async handle => { await handle.close(); return { state: "refused" }; };
      const before = await readdir(spool, { recursive: true });
      const answer = await resumeQueuedRetirement(f.directory, f.lease, f.resources, { job,
        expectedStamp: stateFileStamp(await lstat(f.jobPath, { bigint: true })),
        assertFresh: async () => { if (fault === "fence") throw Error("private detail"); } });
      assert.deepEqual(answer, { state: "refused" }, fault);
      assert.deepEqual(await readdir(spool, { recursive: true }), before);
      assert.ok((await lstat(f.jobPath)).isFile());
    } finally { await f.close(); }
  }
});

test("explicit recovery stops after a partial effect and preserves remaining evidence", async () => {
  const job = encryptedJob(false, true), f = await fixture(job); let checks = 0;
  try {
    const spool = await writeSpool(f.root, false);
    await rm(path.join(spool, "sealed/part-000000.age"));
    const answer = await resumeQueuedRetirement(f.directory, f.lease, f.resources, { job,
      expectedStamp: stateFileStamp(await lstat(f.jobPath, { bigint: true })),
      assertFresh: async () => { if (++checks === 3) throw Error("cancelled or new login"); } });
    assert.deepEqual(answer, { state: "uncertain" });
    assert.ok((await lstat(f.jobPath)).isFile()); assert.ok((await lstat(path.join(spool, "spool-state.json"))).isFile());
  } finally { await f.close(); }
});
