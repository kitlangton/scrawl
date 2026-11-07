import test from "node:test";
import assert from "node:assert/strict";

import { chmod, lstat, mkdir, mkdtemp, readFile, readdir, readlink, realpath, rename, rm, symlink, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { publishRevocationIntent, readRevocationIntentsWithinLease, type RevocationIntentResources, type RevocationFence } from "../../src/account/revocation-intent.ts";
import { admitTrustedDirectory } from "../../src/state/directory.ts";
import type { StateDirectory } from "../../src/state/queue-transaction.ts";

async function fixture(fn: (root: string) => Promise<void>) {
  const root = await mkdtemp(path.join(await realpath(os.tmpdir()), "connect-revocation-")); await chmod(root, 0o700);
  try { await fn(root); } finally { await rm(root, { recursive: true, force: true }); }
}
const fence = (): RevocationFence => ({ signal: new AbortController().signal, assertHeld() {} });
function resources(root: string, replace?: RevocationIntentResources["replaceFile"], checks?: { calls: number }) {
  const port: RevocationIntentResources = {
    async checkAcl(handle) { await handle.close(); return { state: "accepted" }; },
    async replaceFile(parent, source, destination, expected, options) {
      checks && checks.calls++;
      if (replace) return replace(parent, source, destination, expected, options);
      try {
        assert.equal(expected.destination, null);
        await rename(path.join(root, source), path.join(root, destination));
        return { state: "replaced" };
      } finally { await parent.close(); }
    },
    async lock(handle) { await handle.close(); return { state: "busy" }; },
  };
  return port;
}
async function admitted(root: string, port: RevocationIntentResources) {
  const d = await admitTrustedDirectory(root, port, "private-state");
  return { handle: d.handle, path: d.path, identity: d.identity } as StateDirectory;
}
const lease = () => ({ signal: new AbortController().signal, assertHeld() {}, async assertCurrent() {}, async release() {} });

test("publishes private boolean intent with random basename without attempting queue lock", () => fixture(async root => {
  const p = resources(root); const result = await publishRevocationIntent(root, true, fence(), p);
  assert.equal(result.state, "committed"); assert.match(result.name, /^revocation-[a-f0-9]{32}\.intent$/);
  assert.equal((await readFile(path.join(root, result.name), "utf8")), "true");
  assert.equal((await lstat(path.join(root, result.name))).mode & 0o777, 0o600);
  assert.deepEqual(await readdir(root), [result.name]);
}));

test("concurrent disable and logout intents coexist and aggregate logout", () => fixture(async root => {
  const p = resources(root);
  const a = await publishRevocationIntent(root, false, fence(), p);
  const b = await publishRevocationIntent(root, true, fence(), p);
  assert.notEqual(a.name, b.name); assert.equal(a.state, "committed"); assert.equal(b.state, "committed");
  const d = await admitted(root, p);
  try {
    const snapshot = await readRevocationIntentsWithinLease(d, lease(), p);
    assert.equal(snapshot.logout, true); assert.deepEqual(snapshot.intents.map(i => i.logout).sort(), [false, true]);
    assert.ok(Object.isFrozen(snapshot) && Object.isFrozen(snapshot.intents) && snapshot.intents.every(Object.isFrozen));
  } finally { await d.handle.close(); }
}));

test("expected-absent refusal never overwrites or retries", () => fixture(async root => {
  let calls = 0;
  const p = resources(root, async parent => { calls++; await parent.close(); return { state: "refused" }; });
  const result = await publishRevocationIntent(root, false, fence(), p);
  assert.equal(result.state, "refused"); assert.equal(calls, 1); assert.deepEqual(await readdir(root), []);
}));

test("uncertainty retains stage and observations; cancellation and lost fence refuse before dispatch", () => fixture(async root => {
  let calls = 0;
  const p = resources(root, async parent => { calls++; await parent.close(); return { state: "publication_uncertain" }; });
  const uncertain = await publishRevocationIntent(root, true, fence(), p);
  assert.equal(uncertain.state, "uncertain"); assert.equal(calls, 1); assert.ok((await readdir(root)).some(n => n.startsWith(".connect-stage-")));
  const cancelled = new AbortController(); cancelled.abort();
  assert.equal((await publishRevocationIntent(root, true, fence(), p, { signal: cancelled.signal })).state, "refused");
  assert.equal((await publishRevocationIntent(root, true, { ...fence(), assertHeld() { throw new Error(); } }, p)).state, "refused");
  assert.equal(calls, 1);
}));

test("cancellation or fence loss after dispatch is uncertain", () => fixture(async root => {
  for (const loss of ["cancel", "fence"] as const) {
    const controller = new AbortController();
    let held = true;
    const f: RevocationFence = { signal: controller.signal, assertHeld() { if (!held) throw new Error(); } };
    const p = resources(root, async (parent, source, destination) => {
      if (loss === "cancel") controller.abort();
      if (loss === "fence") held = false;
      await parent.close();
      // The native effect may have happened despite a lost acknowledgment.
      await rename(path.join(root, source), path.join(root, destination));
      return { state: "replaced" };
    });
    const result = await publishRevocationIntent(root, true, f, p);
    assert.equal(result.state, "uncertain", loss);
    assert.equal(await readFile(path.join(root, result.name), "utf8"), "true");
  }
}));

test("root replacement before native dispatch is refused", () => fixture(async root => {
  let calls = 0;
  let leafAclCalls = 0;
  const initialInode = (await lstat(root, { bigint: true })).ino;
  const p = resources(root, async parent => { calls++; await parent.close(); return { state: "refused" }; });
  p.checkAcl = async handle => {
    const stat = await handle.stat({ bigint: true });
    if (stat.ino === initialInode) leafAclCalls++;
    if (stat.ino === initialInode && leafAclCalls === 2) {
      await handle.close(); await rename(root, `${root}-old`); await mkdir(root, { mode: 0o700 });
      return { state: "accepted" };
    }
    await handle.close(); return { state: "accepted" };
  };
  const result = await publishRevocationIntent(root, true, fence(), p);
  assert.equal(result.state, "refused"); assert.equal(calls, 0);
  await rm(`${root}-old`, { recursive: true, force: true });
}));

test("reader accepts legacy prefix/suffix names and rejects non-boolean, oversized and symlink entries", () => fixture(async root => {
  const p = resources(root); const d = await admitted(root, p);
  try {
    await writeFile(path.join(root, "revocation-legacy-name.intent"), "false", { mode: 0o600 });
    assert.equal((await readRevocationIntentsWithinLease(d, lease(), p)).logout, false);
    await writeFile(path.join(root, "revocation-bad.intent"), "{}", { mode: 0o600 });
    await assert.rejects(readRevocationIntentsWithinLease(d, lease(), p), /revocation_intent_unavailable/);
    await rm(path.join(root, "revocation-bad.intent"));
    await writeFile(path.join(root, "revocation-large.intent"), JSON.stringify("x".repeat(130)), { mode: 0o600 });
    await assert.rejects(readRevocationIntentsWithinLease(d, lease(), p));
    await rm(path.join(root, "revocation-large.intent"));
    await symlink(path.join(root, "revocation-legacy-name.intent"), path.join(root, "revocation-link.intent"));
    await assert.rejects(readRevocationIntentsWithinLease(d, lease(), p));
  } finally { await d.handle.close(); }
}));

test("reader checks the current root and queue lease around reads and bounds inventory", () => fixture(async root => {
  const p = resources(root); const d = await admitted(root, p);
  try {
    await writeFile(path.join(root, "revocation-one.intent"), "true", { mode: 0o600 });
    const lost = lease(); lost.assertCurrent = async () => { throw new Error("lost"); };
    await assert.rejects(readRevocationIntentsWithinLease(d, lost, p));
    const many = Array.from({ length: 1025 }, (_, i) => writeFile(path.join(root, `revocation-${i}.intent`), "false", { mode: 0o600 }));
    await Promise.all(many);
    await assert.rejects(readRevocationIntentsWithinLease(d, lease(), p));
  } finally { await d.handle.close(); }
}));

test("unsafe ACL evidence fails closed", () => fixture(async root => {
  const p = resources(root); await writeFile(path.join(root, "revocation-no-acl.intent"), "false", { mode: 0o600 });
  const d = await admitted(root, p);
  try {
    const denied = { ...p, async checkAcl(handle: import("node:fs/promises").FileHandle) { await handle.close(); return { state: "refused" as const }; } };
    await assert.rejects(readRevocationIntentsWithinLease(d, lease(), denied));
  } finally { await d.handle.close(); }
}));

test("root replacement cannot turn an uncertain native result into refusal", () => fixture(async root => {
  const previous = `${root}-uncertain`;
  try {
    const p = resources(root, async parent => {
      await parent.close(); await rename(root, previous); await mkdir(root, { mode: 0o700 });
      return { state: "publication_uncertain" };
    });
    assert.equal((await publishRevocationIntent(root, true, fence(), p)).state, "uncertain");
    assert.ok((await readdir(previous)).some(name => name.startsWith(".connect-stage-")));
  } finally { await rm(previous, { recursive: true, force: true }); }
}));


test("fence failures release every transferred root descriptor", { skip: process.platform !== "linux" }, () => fixture(async root => {
  // Sweep the fence checkpoints, including the guarded handoff, without relying
  // on its current call count or a timer race. This is Linux descriptor evidence.
  const openRoots = async () => {
    const names = await readdir("/proc/self/fd");
    const targets = await Promise.all(names.map(name => readlink(`/proc/self/fd/${name}`).catch(() => "")));
    return targets.filter(target => target === root).length;
  };
  for (let failAt = 1; failAt <= 16; failAt++) {
    let calls = 0;
    const p = resources(root, async parent => { await parent.close(); return { state: "refused" }; });
    await publishRevocationIntent(root, true, { signal: new AbortController().signal,
      assertHeld() { if (++calls === failAt) throw new Error("fixture fence lost"); } }, p);
    assert.equal(await openRoots(), 0, `fence checkpoint ${failAt} must not leak a root descriptor`);
  }
}));
