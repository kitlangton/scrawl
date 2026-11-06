import test from "node:test";
import assert from "node:assert/strict";
import { chmod, lstat, mkdtemp, readFile, readdir, realpath, rename, rm, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { replacePrivateStateFile, type StateReplacementResources } from "../../src/state/replacement.ts";

const fence = () => ({ signal: new AbortController().signal, assertHeld() {} });
async function fixture(fn: (root: string) => Promise<void>) {
  const root = await mkdtemp(path.join(await realpath(os.tmpdir()), "connect-replacement-")); await chmod(root, 0o700);
  try { await fn(root); } finally { await rm(root, { recursive: true, force: true }); }
}
function port(operation: StateReplacementResources["replaceFile"]): StateReplacementResources {
  return { async checkAcl(handle) { await handle.close(); return { state: "accepted" }; }, replaceFile: operation };
}
const bytes = () => Buffer.from('{"synthetic":"new"}');

test("stages a detached private copy and commits only after matching readback", () => fixture(async root => {
  const input = bytes(); let borrowed: Buffer | undefined;
  const resources = port(async (parent, source, destination) => {
    try {
      input.fill(0); borrowed = await readFile(path.join(root, source));
      assert.equal(borrowed.toString(), bytes().toString());
      assert.equal((await lstat(path.join(root, source))).mode & 0o777, 0o600);
      await rename(path.join(root, source), path.join(root, destination)); return { state: "replaced" };
    } finally { await parent.close(); }
  });
  assert.deepEqual(await replacePrivateStateFile(root, "record.test", input, null, fence(), resources), { state: "committed" });
  assert.equal((await readFile(path.join(root, "record.test"))).toString(), borrowed!.toString());
  assert.deepEqual(await readdir(root), ["record.test"]);
}));

test("proven no-effect refusals remove only the original stage", () => fixture(async root => {
  await writeFile(path.join(root, "record.test"), "old", { mode: 0o600 });
  for (const state of ["refused", "destination_changed"] as const) {
    const resources = port(async parent => { await parent.close(); return { state }; });
    assert.deepEqual(await replacePrivateStateFile(root, "record.test", bytes(), null, fence(), resources), { state });
    assert.deepEqual(await readdir(root), ["record.test"]);
    assert.equal(await readFile(path.join(root, "record.test"), "utf8"), "old");
  }
}));

test("uncertain dispatch and thrown acknowledgements retain stage without retry", () => fixture(async root => {
  for (const throws of [false, true]) {
    let calls = 0;
    const resources = port(async parent => {
      calls++; await parent.close(); if (throws) throw new Error("private diagnostic");
      return { state: "publication_uncertain" };
    });
    assert.deepEqual(await replacePrivateStateFile(root, "record.test", bytes(), null, fence(), resources), { state: "publication_uncertain" });
    assert.equal(calls, 1);
  }
  assert.equal((await readdir(root)).length, 2);
}));

test("readback failure after replacement retains final bytes as uncertain", () => fixture(async root => {
  const resources = port(async (parent, source, destination) => {
    try { await rename(path.join(root, source), path.join(root, destination)); await writeFile(path.join(root, destination), "corrupt"); return { state: "replaced" }; }
    finally { await parent.close(); }
  });
  assert.deepEqual(await replacePrivateStateFile(root, "record.test", bytes(), null, fence(), resources), { state: "publication_uncertain" });
  assert.equal(await readFile(path.join(root, "record.test"), "utf8"), "corrupt");
}));

test("cleanup refuses a substituted stage inode", () => fixture(async root => {
  let stage = "";
  const resources = port(async (parent, source) => {
    try { stage = source; await rename(path.join(root, source), path.join(root, "original")); await writeFile(path.join(root, source), "replacement", { mode: 0o600 }); return { state: "refused" }; }
    finally { await parent.close(); }
  });
  assert.deepEqual(await replacePrivateStateFile(root, "record.test", bytes(), null, fence(), resources), { state: "refused" });
  assert.equal(await readFile(path.join(root, stage), "utf8"), "replacement");
  assert.equal(await readFile(path.join(root, "original"), "utf8"), bytes().toString());
}));

test("invalid names, oversized bytes, cancellation and lost fences dispatch nothing", () => fixture(async root => {
  let calls = 0;
  const resources = port(async parent => { calls++; await parent.close(); return { state: "refused" }; });
  for (const name of ["../escape", "..", "bad/name", "bad\0", "queue.lock", "read.renewal.lock"]) {
    assert.deepEqual(await replacePrivateStateFile(root, name, bytes(), null, fence(), resources), { state: "refused" });
  }
  assert.deepEqual(await replacePrivateStateFile(root, "record.test", new Uint8Array(2_000_001), null, fence(), resources), { state: "refused" });
  const cancelled = new AbortController(); cancelled.abort();
  assert.deepEqual(await replacePrivateStateFile(root, "record.test", bytes(), null, fence(), resources, { signal: cancelled.signal }), { state: "refused" });
  assert.deepEqual(await replacePrivateStateFile(root, "record.test", bytes(), null, { ...fence(), assertHeld() { throw new Error(); } }, resources), { state: "refused" });
  assert.equal(calls, 0); assert.deepEqual(await readdir(root), []);
}));

test("cancellation after an acknowledged replacement retains output and reports uncertainty", () => fixture(async root => {
  const controller = new AbortController();
  const resources = port(async (parent, source, destination) => {
    try { await rename(path.join(root, source), path.join(root, destination)); controller.abort(); return { state: "replaced" }; }
    finally { await parent.close(); }
  });
  assert.deepEqual(await replacePrivateStateFile(root, "record.test", bytes(), null, fence(), resources, { signal: controller.signal }), { state: "publication_uncertain" });
  assert.equal(await readFile(path.join(root, "record.test"), "utf8"), bytes().toString());
}));

test("changes during the ACL handoff invalidate otherwise matching readback", () => fixture(async root => {
  let published = false;
  const resources = port(async (parent, source, destination) => {
    try { await rename(path.join(root, source), path.join(root, destination)); published = true; return { state: "replaced" }; }
    finally { await parent.close(); }
  });
  resources.checkAcl = async handle => {
    try { if (published) await writeFile(path.join(root, "record.test"), "modified after read"); return { state: "accepted" }; }
    finally { await handle.close(); }
  };
  assert.deepEqual(await replacePrivateStateFile(root, "record.test", bytes(), null, fence(), resources), { state: "publication_uncertain" });
  assert.equal(await readFile(path.join(root, "record.test"), "utf8"), "modified after read");
}));
