import test from "node:test";
import assert from "node:assert/strict";
import { chmod, lstat, mkdtemp, readFile, readdir, realpath, rename, rm, writeFile, type FileHandle } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { commitCompletedRenewal } from "../../src/state/renewal-commit.ts";
import { readCredentialSnapshot, type NamespaceResources } from "../../src/state/namespace.ts";
import type { StateReplacementResources } from "../../src/state/replacement.ts";

const ctx = { scope: "read" as const, approvedServer: "https://sync.example/connect" };
const wire = { server: ctx.approvedServer, scope: "read", token: "old-access", generation: "a".repeat(32),
  refreshToken: "old-refresh", expiresAt: 1900, refreshExpiresAt: 3000, grantId: "grant" };
const reply = { scope: "read", accessToken: "new-access", refreshToken: "new-refresh", grantId: "grant", expiresIn: 60, refreshExpiresIn: 300 };
async function fixture(run: (f: Awaited<ReturnType<typeof createFixture>>) => Promise<void>) {
  const f = await createFixture(); try { await run(f); } finally { await rm(f.root, { recursive: true, force: true }); }
}
async function createFixture() {
  const root = await mkdtemp(path.join(await realpath(os.tmpdir()), "connect-renewal-commit-")); await chmod(root, 0o700);
  const file = (name: string) => path.join(root, name);
  const put = (name: string, value: unknown) => writeFile(file(name), JSON.stringify(value), { mode: 0o600 });
  await put("read.credential", wire); await put("read.renewal", { base: wire, issued: 2000, completed: true });
  await put("read.response", reply); await put("upload.credential", { retained: "upload" });
  const events: string[] = [], held = new Set<string>();
  const state = { dispatches: 0, result: "replaced" as "replaced" | "refused" | "destination_changed" | "publication_uncertain",
    failRelease: "", afterPublish: undefined as undefined | (() => Promise<void>), beforeAcl: undefined as undefined | ((handle: FileHandle) => Promise<void>) };
  const resources: NamespaceResources & StateReplacementResources = {
    async checkAcl(handle) { try { await state.beforeAcl?.(handle); return { state: "accepted" }; } finally { await handle.close(); } },
    async lock(handle) {
      const fd = await handle.stat(); let name = "";
      try {
        for (const candidate of await readdir(root)) {
          if (candidate.endsWith(".lock") && (await lstat(file(candidate))).ino === fd.ino) name = candidate;
        }
      } finally { await handle.close(); }
      assert.ok(name); assert.equal(held.has(name), false); held.add(name); events.push(`acquire:${name}`);
      return { state: "acquired", lease: { signal: new AbortController().signal,
        assertHeld() { assert.equal(held.has(name), true); }, async release() { held.delete(name); events.push(`release:${name}`); if (state.failRelease === name) throw new Error("private diagnostic"); } } };
    },
    async replaceFile(parent, source, destination) {
      try {
        state.dispatches++; events.push(`replace:${destination}`);
        assert.ok(held.has("read.renewal.lock") && held.has("queue.lock"));
        if (state.result !== "replaced") return { state: state.result };
        await rename(file(source), file(destination)); await state.afterPublish?.(); return { state: "replaced" };
      } finally { await parent.close(); }
    },
  };
  const snapshot = await readCredentialSnapshot(root, ctx, resources); events.length = 0;
  return { root, file, put, state, events, held, resources, snapshot };
}

test("completed response commits with scope-before-queue ordering and original issue time", () => fixture(async f => {
  const result = await commitCompletedRenewal(f.root, ctx, f.snapshot, f.resources);
  assert.equal(result.state, "committed");
  const stored = JSON.parse(await readFile(f.file("read.credential"), "utf8"));
  assert.equal(stored.token, "new-access"); assert.equal(stored.expiresAt, 2060); assert.equal(stored.refreshExpiresAt, 2300);
  assert.deepEqual(JSON.parse(await readFile(f.file("upload.credential"), "utf8")), { retained: "upload" });
  assert.equal((await lstat(f.file("read.credential"))).mode & 0o777, 0o600);
  assert.deepEqual((await readdir(f.root)).sort(), ["queue.lock", "read.credential", "read.renewal.lock", "upload.credential"]);
  assert.deepEqual(f.events, ["acquire:read.renewal.lock", "acquire:queue.lock", "replace:read.credential", "release:queue.lock", "release:read.renewal.lock"]);
  assert.equal(f.held.size, 0);
}));

test("stale login, token, auth-required, cancellation and revocation cannot publish recovery", async () => {
  for (const mutation of [
    async (f: Awaited<ReturnType<typeof createFixture>>) => f.put("read.credential", { ...wire, generation: "b".repeat(32) }),
    async (f: Awaited<ReturnType<typeof createFixture>>) => f.put("read.credential", { ...wire, token: "rotated" }),
    async (f: Awaited<ReturnType<typeof createFixture>>) => f.put("read.credential", { ...wire, authRequired: true }),
    async (f: Awaited<ReturnType<typeof createFixture>>) => f.put("cancellation", "new-epoch"),
    async (f: Awaited<ReturnType<typeof createFixture>>) => f.put("revocation-synthetic.intent", true),
  ]) await fixture(async f => {
    await mutation(f); const before = await readFile(f.file("read.credential"));
    const result = await commitCompletedRenewal(f.root, ctx, f.snapshot, f.resources);
    assert.equal(result.state, "unchanged"); assert.equal(f.state.dispatches, 0);
    assert.deepEqual(await readFile(f.file("read.credential")), before);
    assert.ok((await readdir(f.root)).includes("read.response"));
  });
});

test("incomplete, mismatched and invalid completed journals never change credentials", async () => {
  for (const mutation of [
    async (f: Awaited<ReturnType<typeof createFixture>>) => f.put("read.renewal", { base: wire, issued: 2000, completed: false }),
    async (f: Awaited<ReturnType<typeof createFixture>>) => f.put("read.renewal", { base: { ...wire, token: "other" }, issued: 2000, completed: true }),
    async (f: Awaited<ReturnType<typeof createFixture>>) => f.put("read.response", { ...reply, grantId: "other" }),
    async (f: Awaited<ReturnType<typeof createFixture>>) => writeFile(f.file("read.response"), "{invalid"),
  ]) await fixture(async f => {
    await mutation(f);
    assert.deepEqual(await commitCompletedRenewal(f.root, ctx, f.snapshot, f.resources), { state: "unchanged", reason: "not_recoverable" });
    assert.equal(f.state.dispatches, 0); assert.equal(JSON.parse(await readFile(f.file("read.credential"), "utf8")).token, "old-access");
  });
});

test("no-effect refusal retains completed evidence; unknown effects are never retried", async () => {
  for (const state of ["refused", "destination_changed", "publication_uncertain"] as const) await fixture(async f => {
    f.state.result = state;
    const result = await commitCompletedRenewal(f.root, ctx, f.snapshot, f.resources);
    assert.equal(result.state, state === "publication_uncertain" ? "publication_uncertain" : "unchanged");
    assert.equal(f.state.dispatches, 1);
    assert.ok((await readdir(f.root)).includes("read.renewal")); assert.ok((await readdir(f.root)).includes("read.response"));
  });
});

test("post-publication replacement of evidence is retained and reported uncertain", () => fixture(async f => {
  f.state.afterPublish = async () => { await rename(f.file("read.renewal"), f.file("original.renewal")); await f.put("read.renewal", { replacement: true }); };
  assert.deepEqual(await commitCompletedRenewal(f.root, ctx, f.snapshot, f.resources), { state: "publication_uncertain" });
  assert.equal(JSON.parse(await readFile(f.file("read.credential"), "utf8")).token, "new-access");
  assert.deepEqual(JSON.parse(await readFile(f.file("read.renewal"), "utf8")), { replacement: true });
  assert.ok((await readdir(f.root)).includes("read.response")); assert.equal(f.state.dispatches, 1);
}));

test("queue or scope release failure after commit reports uncertainty without retry", async () => {
  for (const lock of ["queue.lock", "read.renewal.lock"]) await fixture(async f => {
    f.state.failRelease = lock;
    assert.deepEqual(await commitCompletedRenewal(f.root, ctx, f.snapshot, f.resources), { state: "publication_uncertain" });
    assert.equal(JSON.parse(await readFile(f.file("read.credential"), "utf8")).token, "new-access");
    assert.equal(f.state.dispatches, 1); assert.equal(f.held.size, 0);
  });
});

test("evidence replacement during the private read is rejected before credential publication", () => fixture(async f => {
  const response = await lstat(f.file("read.response")); let changed = false;
  f.state.beforeAcl = async handle => {
    if (changed || (await handle.stat()).ino !== response.ino) return;
    changed = true; await rename(f.file("read.renewal"), f.file("original.renewal")); await f.put("read.renewal", { replacement: true });
  };
  assert.deepEqual(await commitCompletedRenewal(f.root, ctx, f.snapshot, f.resources), { state: "unchanged", reason: "unsafe_file" });
  assert.equal(f.state.dispatches, 0);
  assert.deepEqual(JSON.parse(await readFile(f.file("read.renewal"), "utf8")), { replacement: true });
}));

test("renewed credentials must fit the existing stored-record reader bound", () => fixture(async f => {
  const base = { ...wire, site: "x".repeat(30_000) };
  await f.put("read.credential", base); await f.put("read.renewal", { base, issued: 2000, completed: true });
  await f.put("read.response", { ...reply, accessToken: "a".repeat(8192), refreshToken: "r".repeat(8192) });
  const snapshot = await readCredentialSnapshot(f.root, ctx, f.resources);
  assert.deepEqual(await commitCompletedRenewal(f.root, ctx, snapshot, f.resources), { state: "unchanged", reason: "not_recoverable" });
  assert.equal(f.state.dispatches, 0); assert.equal(JSON.parse(await readFile(f.file("read.credential"), "utf8")).token, "old-access");
}));

test("revocation arriving during publication retains recovery evidence and refuses a success claim", () => fixture(async f => {
  f.state.afterPublish = () => f.put("revocation-synthetic.intent", true);
  assert.deepEqual(await commitCompletedRenewal(f.root, ctx, f.snapshot, f.resources), { state: "publication_uncertain" });
  assert.ok((await readdir(f.root)).includes("read.renewal")); assert.ok((await readdir(f.root)).includes("read.response"));
  assert.equal(f.state.dispatches, 1);
}));

test("revocation intent arriving during response read prevents credential publication", () => fixture(async f => {
  const response = await lstat(f.file("read.response")); let changed = false;
  f.state.beforeAcl = async handle => {
    if (changed || (await handle.stat()).ino !== response.ino) return;
    changed = true; await f.put("revocation-synthetic.intent", true);
  };
  assert.deepEqual(await commitCompletedRenewal(f.root, ctx, f.snapshot, f.resources), { state: "unchanged", reason: "revocation_pending" });
  assert.equal(f.state.dispatches, 0); assert.equal(JSON.parse(await readFile(f.file("read.credential"), "utf8")).token, "old-access");
}));

test("cancellation after a credential effect retains the journal and never claims unchanged", () => fixture(async f => {
  const controller = new AbortController();
  f.state.afterPublish = async () => { controller.abort(); };
  assert.deepEqual(await commitCompletedRenewal(f.root, ctx, f.snapshot, f.resources, { signal: controller.signal }), { state: "publication_uncertain" });
  assert.equal(JSON.parse(await readFile(f.file("read.credential"), "utf8")).token, "new-access");
  assert.ok((await readdir(f.root)).includes("read.response")); assert.ok((await readdir(f.root)).includes("read.renewal"));
  assert.equal(f.state.dispatches, 1);
}));

test("credential mutation during ACL handoff invalidates the fresh full-record fence", () => fixture(async f => {
  const credential = await lstat(f.file("read.credential")); let changed = false;
  f.state.beforeAcl = async handle => {
    if (changed || (await handle.stat()).ino !== credential.ino) return;
    changed = true; await f.put("read.credential", { ...wire, token: "rotated-during-read" });
  };
  assert.deepEqual(await commitCompletedRenewal(f.root, ctx, f.snapshot, f.resources), { state: "unchanged", reason: "unsafe_file" });
  assert.equal(f.state.dispatches, 0);
  assert.equal(JSON.parse(await readFile(f.file("read.credential"), "utf8")).token, "rotated-during-read");
}));
