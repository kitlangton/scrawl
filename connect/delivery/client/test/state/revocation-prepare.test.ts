import test from "node:test";
import assert from "node:assert/strict";
import { chmod, lstat, mkdtemp, readFile, readdir, realpath, rename, rm, writeFile, type FileHandle } from "node:fs/promises";
import { existsSync, renameSync, writeFileSync } from "node:fs";
import os from "node:os";
import path from "node:path";
import { prepareRevocationState } from "../../src/account/revocation-prepare.ts";
import type { NamespaceResources } from "../../src/state/queue-transaction.ts";
import type { StateReplacementResources } from "../../src/state/replacement.ts";

const context = { approvedServer: "https://sync.example/connect" };
async function fixture(run: (f: Awaited<ReturnType<typeof createFixture>>) => Promise<void>) {
  const f = await createFixture(); try { await run(f); } finally { await rm(f.root, { recursive: true, force: true }); }
}
async function createFixture() {
  const root = await mkdtemp(path.join(await realpath(os.tmpdir()), "connect-revocation-prepare-")); await chmod(root, 0o700);
  const file = (name: string) => path.join(root, name);
  const put = (name: string, value: unknown) => writeFile(file(name), JSON.stringify(value), { mode: 0o600 });
  const events: string[] = [], held = new Set<string>();
  const state = { dispatches: 0, replaceResult: "replaced" as "replaced" | "refused" | "destination_changed" | "publication_uncertain",
    failRelease: "", leaseCheck: undefined as undefined | (() => void), afterReplace: undefined as undefined | (() => Promise<void>), beforeAcl: undefined as undefined | ((handle: FileHandle) => Promise<void>) };
  const resources: NamespaceResources & StateReplacementResources = {
    async checkAcl(handle) { try { await state.beforeAcl?.(handle); return { state: "accepted" }; } finally { await handle.close(); } },
    async lock(handle) {
      const fd = await handle.stat(); let name = "";
      try { for (const candidate of await readdir(root)) if (candidate.endsWith(".lock") && (await lstat(file(candidate))).ino === fd.ino) name = candidate; }
      finally { await handle.close(); }
      assert.ok(name); assert.equal(held.has(name), false); held.add(name); events.push(`acquire:${name}`);
      return { state: "acquired", lease: { signal: new AbortController().signal,
        assertHeld() { assert.equal(held.has(name), true); state.leaseCheck?.(); }, async assertCurrent() { this.assertHeld(); }, async release() { held.delete(name); events.push(`release:${name}`); if (state.failRelease === name) throw Error(); } } };
    },
    async replaceFile(parent, source, destination) {
      try {
        state.dispatches++; events.push(`replace:${destination}`); assert.ok(held.has("queue.lock"));
        if (state.replaceResult !== "replaced") return { state: state.replaceResult };
        await rename(file(source), file(destination)); await state.afterReplace?.(); return { state: "replaced" };
      } finally { await parent.close(); }
    },
  };
  return { root, file, put, state, events, held, resources };
}

test("missing intent refuses without dispatch", () => fixture(async f => {
  assert.deepEqual(await prepareRevocationState(f.root, context, f.resources), { state: "refused", reason: "intent_unavailable" });
  assert.equal(f.state.dispatches, 0); assert.deepEqual(await readdir(f.root), ["queue.lock"]);
}));

test("prepare aggregates logout, rotates absent cancellation and removes captured logins", () => fixture(async f => {
  await f.put("revocation-a.intent", false); await f.put("revocation-b.intent", true);
  await f.put("upload.login", "upload-session"); await f.put("read.login", "read-session");
  await f.put("upload.credential", { token: "retained" }); await f.put("ready.job", { status: "ready" });
  await f.put("config.json", { enabled: true }); await f.put("identity", { retained: true });
  const receipt = await prepareRevocationState(f.root, context, f.resources);
  assert.equal(receipt.state, "prepared"); if (receipt.state !== "prepared") return;
  assert.equal(Object.isFrozen(receipt), true); assert.equal(receipt.epoch.length, 32); assert.equal(receipt.logout, true); assert.equal(receipt.intents.length, 2);
  assert.equal(JSON.parse(await readFile(f.file("cancellation"), "utf8")), receipt.epoch);
  assert.equal((await readdir(f.root)).includes("upload.login"), false); assert.equal((await readdir(f.root)).includes("read.login"), false);
  for (const name of ["revocation-a.intent", "revocation-b.intent", "upload.credential", "ready.job", "config.json", "identity"]) assert.ok((await readdir(f.root)).includes(name));
  assert.equal(f.state.dispatches, 1); assert.equal(f.held.size, 0);
}));

test("present cancellation is replaced under one queue lease", () => fixture(async f => {
  await f.put("revocation-a.intent", false); await f.put("cancellation", "old");
  const result = await prepareRevocationState(f.root, context, f.resources);
  assert.equal(result.state, "prepared"); assert.equal(f.state.dispatches, 1);
  assert.deepEqual(f.events, ["acquire:queue.lock", "replace:cancellation", "release:queue.lock"]);
}));

test("malformed login marker refuses before cancellation publication", () => fixture(async f => {
  return f.put("revocation-a.intent", false).then(() => writeFile(f.file("read.login"), "{}", { mode: 0o600 })).then(async () => {
    assert.deepEqual(await prepareRevocationState(f.root, context, f.resources), { state: "refused", reason: "unsafe_state" });
    assert.equal(f.state.dispatches, 0); assert.equal((await readdir(f.root)).includes("cancellation"), false);
    assert.ok((await readdir(f.root)).includes("revocation-a.intent"));
  });
}));

test("uncertain replacement never removes login markers or retries", () => fixture(async f => {
  await f.put("revocation-a.intent", false); await f.put("read.login", "login"); f.state.replaceResult = "publication_uncertain";
  const result = await prepareRevocationState(f.root, context, f.resources);
  assert.equal(result.state, "uncertain"); assert.equal(f.state.dispatches, 1);
  assert.ok((await readdir(f.root)).includes("read.login")); assert.ok((await readdir(f.root)).includes("revocation-a.intent"));
}));

test("post-replacement intent mutation and queue release failure are uncertain", () => fixture(async f => {
  await f.put("revocation-a.intent", false);
  f.state.afterReplace = async () => { await rename(f.file("revocation-a.intent"), f.file("saved.intent")); await f.put("revocation-a.intent", true); };
  assert.equal((await prepareRevocationState(f.root, context, f.resources)).state, "uncertain");
  assert.equal(f.state.dispatches, 1); assert.ok((await readdir(f.root)).includes("revocation-a.intent"));
}));

test("queue release failure after preparation is uncertain with retained intent", () => fixture(async f => {
  await f.put("revocation-a.intent", false); f.state.failRelease = "queue.lock";
  assert.equal((await prepareRevocationState(f.root, context, f.resources)).state, "uncertain");
  assert.equal(f.state.dispatches, 1); assert.ok((await readdir(f.root)).includes("revocation-a.intent"));
}));

test("concurrently published new intent remains uncovered and retained", () => fixture(async f => {
  await f.put("revocation-a.intent", false);
  f.state.afterReplace = () => f.put("revocation-new.intent", true);
  const result = await prepareRevocationState(f.root, context, f.resources);
  assert.equal(result.state, "prepared"); if (result.state === "prepared") assert.equal(result.intents.length, 1);
  assert.ok((await readdir(f.root)).includes("revocation-new.intent"));
}));

test("captured login stamp change before dispatch causes no cancellation write", () => fixture(async f => {
  await f.put("revocation-a.intent", false); await f.put("read.login", "old");
  const login = await lstat(f.file("read.login")); let changed = false;
  f.state.beforeAcl = async handle => { if (!changed && (await handle.stat()).ino === login.ino) { changed = true; await f.put("read.login", "new"); } };
  assert.equal((await prepareRevocationState(f.root, context, f.resources)).state, "refused");
  assert.equal(f.state.dispatches, 0); assert.equal(JSON.parse(await readFile(f.file("read.login"), "utf8")), "new");
}));

test("cancellation or covered intent replacement after publication is uncertain", () => fixture(async f => {
  for (const target of ["cancellation", "revocation-a.intent"]) {
    const beforeDispatches = f.state.dispatches;
    await f.put("revocation-a.intent", false); if (target === "cancellation") await f.put("cancellation", "original");
    f.state.afterReplace = async () => {
      if (target === "cancellation") await f.put(target, "changed");
      else { await rename(f.file(target), f.file("saved.intent")); await f.put(target, true); }
    };
    const result = await prepareRevocationState(f.root, context, f.resources);
    assert.equal(result.state, "uncertain"); assert.equal(f.state.dispatches, beforeDispatches + 1);
    assert.ok((await readdir(f.root)).includes("revocation-a.intent"));
    f.state.afterReplace = undefined;
    await rm(f.file("revocation-a.intent"), { force: true }); await rm(f.file("cancellation"), { force: true });
  }
}));

test("new intent appearing during staging is left retained while existing snapshot proceeds", () => fixture(async f => {
  await f.put("revocation-a.intent", false);
  f.state.afterReplace = () => f.put("revocation-new.intent", true);
  // Native replacement callback publishes after the captured snapshot.
  const result = await prepareRevocationState(f.root, context, f.resources);
  assert.equal(result.state, "prepared"); if (result.state === "prepared") assert.equal(result.intents.length, 1);
  assert.ok((await readdir(f.root)).includes("revocation-new.intent"));
}));

test("a previously absent login appearing after epoch publication is retained and uncertain", () => fixture(async f => {
  await f.put("revocation-a.intent", false);
  f.state.afterReplace = () => f.put("read.login", "replacement-login");
  assert.equal((await prepareRevocationState(f.root, context, f.resources)).state, "uncertain");
  assert.equal(JSON.parse(await readFile(f.file("read.login"), "utf8")), "replacement-login");
  assert.ok((await readdir(f.root)).includes("revocation-a.intent"));
}));


test("partial login removal stops on substitution and preserves remaining evidence", () => fixture(async f => {
  await f.put("revocation-a.intent", true); await f.put("upload.login", "upload"); await f.put("read.login", "read");
  let published = false, substituted = false;
  f.state.afterReplace = async () => { published = true; };
  f.state.leaseCheck = () => {
    if (published && !substituted && !existsSync(f.file("upload.login"))) {
      substituted = true; renameSync(f.file("read.login"), f.file("saved-login"));
      writeFileSync(f.file("read.login"), JSON.stringify("new-login"), { mode: 0o600 });
    }
  };
  assert.equal((await prepareRevocationState(f.root, context, f.resources)).state, "uncertain");
  assert.equal(substituted, true); assert.equal(existsSync(f.file("upload.login")), false);
  assert.equal(JSON.parse(await readFile(f.file("read.login"), "utf8")), "new-login");
  assert.equal(f.state.dispatches, 1); assert.ok(existsSync(f.file("revocation-a.intent")));
}));
