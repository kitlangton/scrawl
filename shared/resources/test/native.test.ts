import assert from "node:assert/strict";
import { constants } from "node:fs";
import { mkdtemp, open, mkdir, chmod, writeFile, readFile, stat, lstat, symlink, link, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { once } from "node:events";
import { createInterface } from "node:readline";
import { NativeResources } from "../src/index.ts";
import { provider } from "./helpers.ts";

const executable = process.env.APPARENT_RESOURCE_HOST_BIN;
assert(executable, "test:native requires an explicitly supplied prebuilt resource helper");
const admitted = await provider(executable);
const resources = new NativeResources(admitted);
async function fixture() {
  const directory = await mkdtemp(join(tmpdir(), "apparent-resources-native-")); await chmod(directory, 0o700);
  const lock = join(directory, "queue.lock"); await writeFile(lock, "", { mode: 0o600 });
  return { directory, lock, handle: () => open(lock, constants.O_RDWR | constants.O_NOFOLLOW),
    parent: () => open(directory, constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW),
    close: () => rm(directory, { recursive: true, force: true }) };
}
async function rawLock(path: string, timeout = 1000) {
  const handle = await open(path, constants.O_RDWR | constants.O_NOFOLLOW);
  const child = spawn(executable!, ["lock", "--timeout-ms", String(timeout)], { stdio: ["pipe", "pipe", "pipe", handle.fd] }) as ChildProcessWithoutNullStreams;
  const exited = new Promise(resolve => child.once("close", resolve));
  await handle.close();
  const iterator = createInterface({ input: child.stdout })[Symbol.asyncIterator]();
  const next = async () => { const item = await iterator.next(); assert.equal(item.done, false); return JSON.parse(item.value!).state as string; };
  return { child, exited, next };
}

test("real flock contention, deadline, cancellation, release and persistent inode", async () => {
  const f = await fixture(); let release: (() => Promise<void>) | undefined;
  try {
    const original = await stat(f.lock), h = await f.handle();
    const first = await resources.lock(h, { timeoutMs: 0 }); assert.equal(h.fd, -1); assert.equal(first.state, "acquired"); if (first.state !== "acquired") return;
    release = () => first.lease.release();
    assert.deepEqual(await resources.lock(await f.handle(), { timeoutMs: 30 }), { state: "busy" });
    const c = new AbortController(), waiting = resources.lock(await f.handle(), { timeoutMs: 60_000, signal: c.signal });
    setTimeout(() => c.abort(), 30); assert.deepEqual(await waiting, { state: "cancelled" });
    await release(); release = undefined;
    const next = await resources.lock(await f.handle(), { timeoutMs: 100 }); assert.equal(next.state, "acquired"); if (next.state === "acquired") await next.lease.release();
    const current = await stat(f.lock); assert.equal(current.dev, original.dev); assert.equal(current.ino, original.ino);
  } finally { await release?.(); await f.close(); }
});

test("EOF and abrupt helper death release the ownership-transferred flock", async () => {
  const f = await fixture(); let child: ChildProcessWithoutNullStreams | undefined;
  try {
    const held = await rawLock(f.lock); child = held.child; assert.equal(await held.next(), "acquired");
    assert.deepEqual(await resources.lock(await f.handle(), { timeoutMs: 0 }), { state: "busy" });
    const waiting = await rawLock(f.lock); waiting.child.stdin.end(); assert.equal(await waiting.next(), "cancelled"); await waiting.exited;
    held.child.kill("SIGKILL"); await held.exited; child = undefined;
    const next = await rawLock(f.lock); child = next.child; assert.equal(await next.next(), "acquired");
    next.child.stdin.end(); assert.equal(await next.next(), "released"); await next.exited; child = undefined;
    const final = await resources.lock(await f.handle(), { timeoutMs: 0 }); assert.equal(final.state, "acquired"); if (final.state === "acquired") await final.lease.release();
  } finally { child?.kill("SIGKILL"); await f.close(); }
});

test("real descriptor validation refuses unsafe files without altering them", async () => {
  const f = await fixture(); try {
    await chmod(f.lock, 0o640); assert.deepEqual(await resources.lock(await f.handle(), { timeoutMs: 0 }), { state: "refused" });
    await chmod(f.lock, 0o600); await link(f.lock, join(f.directory, "hardlink")); assert.deepEqual(await resources.lock(await f.handle(), { timeoutMs: 0 }), { state: "refused" });
    await rm(join(f.directory, "hardlink")); assert.deepEqual(await resources.lock(await open(f.lock, "r"), { timeoutMs: 0 }), { state: "refused" });
    assert.deepEqual(await resources.lock(await f.parent(), { timeoutMs: 0 }), { state: "refused" });
    assert.equal((await stat(f.lock)).size, 0);
  } finally { await f.close(); }
});

async function stage(parent: string, name: string, contents: string) {
  const path = join(parent, name); await mkdir(path, { mode: 0o700 }); await chmod(path, 0o700);
  const file = await open(join(path, "bundle.txt"), "wx", 0o600); await file.writeFile(contents); await file.sync(); await file.close();
  const directory = await open(path, constants.O_RDONLY | constants.O_DIRECTORY); await directory.sync(); await directory.close();
  return stat(path);
}

test("real publication keeps exact staged identity and never replaces a destination", async () => {
  const f = await fixture(); try {
    const original = await stage(f.directory, "stage", "synthetic bundle");
    const parent = await f.parent(); assert.deepEqual(await resources.publish(parent, "stage", "final"), { state: "published" }); assert.equal(parent.fd, -1);
    const final = await stat(join(f.directory, "final")); assert.equal(final.ino, original.ino); assert.equal(final.dev, original.dev); assert.equal(final.mode & 0o777, 0o700);
    assert.equal(await readFile(join(f.directory, "final/bundle.txt"), "utf8"), "synthetic bundle");
    await stage(f.directory, "second", "replacement refused");
    assert.deepEqual(await resources.publish(await f.parent(), "second", "final"), { state: "destination_exists" });
    assert.equal(await readFile(join(f.directory, "final/bundle.txt"), "utf8"), "synthetic bundle"); assert.equal(await readFile(join(f.directory, "second/bundle.txt"), "utf8"), "replacement refused");
    await symlink("final", join(f.directory, "alias"));
    assert.deepEqual(await resources.publish(await f.parent(), "alias", "escaped"), { state: "refused" });
    assert.equal((await lstat(join(f.directory, "alias"))).isSymbolicLink(), true);
  } finally { await f.close(); }
});

test("two concurrent no-replace publishers yield one winner and retain the losing stage", async () => {
  const f = await fixture(); try {
    await stage(f.directory, "one", "one"); await stage(f.directory, "two", "two");
    const results = await Promise.all([resources.publish(await f.parent(), "one", "final"), resources.publish(await f.parent(), "two", "final")]);
    assert.deepEqual(results.map(x => x.state).sort(), ["destination_exists", "published"]);
    const winner = results[0]!.state === "published" ? "one" : "two", loser = winner === "one" ? "two" : "one";
    assert.equal(await readFile(join(f.directory, "final/bundle.txt"), "utf8"), winner);
    assert.equal(await readFile(join(f.directory, loser + "/bundle.txt"), "utf8"), loser);
  } finally { await f.close(); }
});

test("publication preserves legitimate Unicode/space output names", async () => {
  const f = await fixture(); try {
    const source = "résumé stage", destination = "Sam's review 👋.apparent";
    const before = await stage(f.directory, source, "unicode output");
    assert.deepEqual(await resources.publish(await f.parent(), source, destination), { state: "published" });
    const after = await stat(join(f.directory, destination)); assert.equal(after.ino, before.ino);
    assert.equal(await readFile(join(f.directory, destination, "bundle.txt"), "utf8"), "unicode output");
  } finally { await f.close(); }
});
test("native ACL facts cover descriptors and Darwin allow/deny entries", async () => {
  const f = await fixture();
  const chmodAcl = async (args: string[]) => { const { execFileSync } = await import("node:child_process"); execFileSync("/bin/chmod", args); };
  try {
    assert.deepEqual(await resources.checkAcl(await f.handle()), { state: "accepted" });
    assert.deepEqual(await resources.checkAcl(await f.parent()), { state: "accepted" });
    if (process.platform === "darwin") {
      await chmodAcl(["+a", "everyone allow read", f.lock]);
      assert.deepEqual(await resources.checkAcl(await f.handle()), { state: "refused" });
      await chmodAcl(["-N", f.lock]);
      await chmodAcl(["+a", "everyone deny delete", f.lock]);
      assert.deepEqual(await resources.checkAcl(await f.handle()), { state: "accepted" });
      await chmodAcl(["-N", f.lock]);
      await chmodAcl(["+a", "everyone allow search", f.directory]);
      assert.deepEqual(await resources.checkAcl(await f.parent()), { state: "refused" });
    }
  } finally {
    if (process.platform === "darwin") { await chmodAcl(["-N", f.lock]); await chmodAcl(["-N", f.directory]); }
    await f.close();
  }
});

test("sticky shared output parents preserve the legacy no-replace contract", async () => {
  const f = await fixture(); try {
    await stage(f.directory, "stage", "sticky parent output"); await chmod(f.directory, 0o1777);
    assert.deepEqual(await resources.publish(await f.parent(), "stage", "final"), { state: "published" });
    await stage(f.directory, "second", "must remain staged"); await chmod(f.directory, 0o777);
    assert.deepEqual(await resources.publish(await f.parent(), "second", "other"), { state: "refused" });
    assert.equal(await readFile(join(f.directory, "second/bundle.txt"), "utf8"), "must remain staged");
  } finally { await chmod(f.directory, 0o700); await f.close(); }
});
