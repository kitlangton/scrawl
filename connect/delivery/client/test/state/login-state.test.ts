import test from "node:test";
import assert from "node:assert/strict";
import { chmod, lstat, mkdtemp, readFile, readdir, realpath, rm, writeFile, type FileHandle } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { beginLoginState, type LoginResources } from "../../src/account/login-state.ts";
import type { NamespaceResources } from "../../src/state/queue-transaction.ts";
import type { StateReplacementResources } from "../../src/state/replacement.ts";

async function fixture(run: (f: Awaited<ReturnType<typeof create>>) => Promise<void>) {
  const f = await create(); try { await run(f); } finally { await rm(f.root, { recursive: true, force: true }); }
}
async function create() {
  const root = await mkdtemp(path.join(await realpath(os.tmpdir()), "connect-login-state-")); await chmod(root, 0o700);
  const file = (name: string) => path.join(root, name), put = (name: string, value: unknown) => writeFile(file(name), JSON.stringify(value), { mode: 0o600 });
  const held = new Set<string>(), state = { dispatches: 0, result: "replaced" as "replaced" | "refused" | "destination_changed" | "publication_uncertain",
    releaseFailure: false, queueStarted: undefined as undefined | (() => void), queueContinue: undefined as undefined | (() => void) };
  const resources: LoginResources = {
    async checkAcl(h) { try { return { state: "accepted" as const }; } finally { await h.close(); } },
    async lock(h) {
      const ino = (await h.stat()).ino; await h.close(); let name = "";
      for (const n of await readdir(root)) if (n.endsWith(".lock") && (await lstat(file(n))).ino === ino) name = n;
      assert.ok(name); state.queueStarted?.(); if (state.queueContinue) await new Promise<void>(resolve => { state.queueContinue = () => { state.queueContinue = undefined; resolve(); }; });
      assert.equal(held.has(name), false); held.add(name);
      return { state: "acquired" as const, lease: { signal: new AbortController().signal,
        assertHeld() { assert.ok(held.has(name)); }, async assertCurrent() { this.assertHeld(); }, async release() { held.delete(name); if (state.releaseFailure) throw Error("release"); } } };
    },
    async replaceFile(parent, source, destination, expected) {
      try {
        state.dispatches++; assert.ok(held.has("queue.lock")); if (state.result !== "replaced") return { state: state.result };
        const old = await lstat(file(destination)).catch(() => null);
        if ((old ? { device: String(old.dev), inode: String(old.ino) } : null)?.device !== expected.destination?.device
          || (old ? String(old.ino) : undefined) !== expected.destination?.inode) {
          if (!old && expected.destination === null) { /* expected absent */ } else return { state: "destination_changed" as const };
        }
        await import("node:fs/promises").then(fs => fs.rename(file(source), file(destination)));
        return { state: "replaced" as const };
      } finally { await parent.close(); }
    },
  } as LoginResources;
  return { root, file, put, state, resources };
}
const context = (scope: "read" | "upload" = "read", uploadConsent = false) => ({ scope, approvedServer: "https://sync.example/connect", uploadConsent });

test("validates scope, explicit upload consent and already-normalized server before marker effects", () => fixture(async f => {
  assert.equal((await beginLoginState(f.root, { ...context(), scope: "invalid" as never }, f.resources)).state, "refused");
  assert.equal((await beginLoginState(f.root, context("upload"), f.resources)).state, "refused");
  assert.equal((await beginLoginState(f.root, { ...context(), approvedServer: "https://user@sync.example/connect" }, f.resources)).state, "refused");
  assert.equal(f.state.dispatches, 0); assert.deepEqual(await readdir(f.root), []);
}));

test("read and upload create only their own marker; preserve adjacent state", () => fixture(async f => {
  await f.put("upload.login", "old-upload"); await f.put("config.json", { keep: true }); await f.put("identity", { keep: true });
  await f.put("read.credential", { keep: "read" }); await f.put("upload.credential", { keep: "upload" });
  const read = await beginLoginState(f.root, context(), f.resources); assert.equal(read.state, "begun");
  const upload = await beginLoginState(f.root, context("upload", true), f.resources); assert.equal(upload.state, "begun");
  assert.equal(JSON.parse(await readFile(f.file("read.login"), "utf8")), read.state === "begun" ? read.loginId : "");
  assert.equal(JSON.parse(await readFile(f.file("upload.login"), "utf8")), upload.state === "begun" ? upload.loginId : "");
  for (const name of ["config.json", "identity", "read.credential", "upload.credential"]) assert.deepEqual(JSON.parse(await readFile(f.file(name), "utf8")), name === "config.json" || name === "identity" ? { keep: true } : { keep: name.startsWith("read") ? "read" : "upload" });
}));

test("pending intent refuses before dispatch; pending intent appearing during queue wait refuses", () => fixture(async f => {
  await f.put("revocation-a.intent", false);
  assert.equal((await beginLoginState(f.root, context(), f.resources)).state, "refused"); assert.equal(f.state.dispatches, 0);
  await rm(f.file("revocation-a.intent"));
  let started!: () => void; const waiting = new Promise<void>(r => { started = r; }); f.state.queueStarted = started;
  f.state.queueContinue = () => {}; const operation = beginLoginState(f.root, context(), f.resources);
  await waiting; await f.put("revocation-a.intent", true); f.state.queueContinue?.();
  assert.equal((await operation).state, "refused"); assert.equal(f.state.dispatches, 0);
}));

test("cancellation changed while waiting and pre-aborted signal do not dispatch", () => fixture(async f => {
  await f.put("cancellation", "one");
  let started!: () => void; const waiting = new Promise<void>(r => { started = r; }); f.state.queueStarted = started;
  const operation = beginLoginState(f.root, context(), f.resources); await waiting; await f.put("cancellation", "two");
  f.state.queueContinue?.(); assert.equal((await operation).state, "refused"); assert.equal(f.state.dispatches, 0);
  const controller = new AbortController(); controller.abort();
  assert.deepEqual(await beginLoginState(f.root, context(), f.resources, { signal: controller.signal }), { state: "refused", reason: "cancelled" });
}));

test("safe stale marker is superseded; destination conflict and publication/release uncertainty never return begun", () => fixture(async f => {
  await f.put("read.login", "stale opaque bytes");
  const begun = await beginLoginState(f.root, context(), f.resources); assert.equal(begun.state, "begun");
  f.state.result = "destination_changed";
  assert.equal((await beginLoginState(f.root, context(), f.resources)).state, "refused");
  f.state.result = "publication_uncertain";
  const uncertain = await beginLoginState(f.root, context(), f.resources); assert.equal(uncertain.state, "uncertain");
  assert.ok("proposedLoginId" in uncertain); assert.equal("loginId" in uncertain, false);
  f.state.result = "replaced"; f.state.releaseFailure = true;
  const release = await beginLoginState(f.root, context(), f.resources); assert.equal(release.state, "uncertain"); assert.equal("loginId" in release, false);
  assert.equal(f.state.dispatches, 4);
}));

test("inherited provider methods are bound to the provider instance", () => fixture(async f => {
  class Ports { count = 0; async checkAcl(h: FileHandle) { try { this.count++; return { state: "accepted" as const }; } finally { await h.close(); } }
    lock = f.resources.lock; replaceFile = f.resources.replaceFile; }
  const ports = new Ports();
  const result = await beginLoginState(f.root, context(), ports as LoginResources);
  assert.equal(result.state, "begun"); assert.ok(ports.count > 0);
}));
