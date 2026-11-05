import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { chmod, lstat, mkdir, mkdtemp, readFile, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { test } from "node:test";
import { SqliteCaptureMappingStore } from "../dist/backend/host/capture-mapping-store.js";
import { PrivateBundleAllocator } from "../dist/backend/host/bundle-allocator.js";

const mapping = (requestId: string, captureId = "a".repeat(32), ownerToken = "b".repeat(64)) => ({
  requestId, captureId, ownerToken, bundleId: `bundle-${captureId}`, bundlePath: `/private/${captureId}`,
});

async function temporary(t: { after(fn: () => void | Promise<void>): void }) {
  const root = await realpath(await mkdtemp(join(tmpdir(), "record-host-")));
  t.after(() => rm(root, { recursive: true, force: true }));
  return root;
}

function runChild(root: string, value: ReturnType<typeof mapping>, loseReply = false): Promise<ReturnType<typeof mapping>> {
  return new Promise((resolve, reject) => {
    const child = spawn(process.execPath, ["--experimental-strip-types", new URL("./host-child.ts", import.meta.url).pathname,
      root, value.requestId, value.captureId, value.ownerToken, ...(loseReply ? ["lose-reply"] : [])], { stdio: ["ignore", "pipe", "pipe"] });
    let stdout = "";
    let stderr = "";
    child.stdout.setEncoding("utf8").on("data", chunk => stdout += chunk);
    child.stderr.setEncoding("utf8").on("data", chunk => stderr += chunk);
    child.on("error", reject);
    child.on("close", code => code === 0 ? resolve(JSON.parse(stdout)) : reject(new Error(`host child failed: ${stderr}`)));
  });
}

test("SQLite put-if-absent serializes independent processes and persists the immutable winner", async t => {
  const root = join(await temporary(t), "mapping");
  const first = mapping("request / arbitrary", "1".repeat(32), "2".repeat(64));
  const second = mapping("request / arbitrary", "3".repeat(32), "4".repeat(64));
  const results = await Promise.all([runChild(root, first), runChild(root, second)]);
  assert.deepEqual(results[0], results[1]);
  const store = new SqliteCaptureMappingStore(root);
  assert.deepEqual(await store.getByRequestId(first.requestId), results[0]);
  assert.deepEqual(await store.getByCaptureId(results[0].captureId), results[0]);
  assert.deepEqual(await store.putIfAbsent(second), results[0]);
  const ambiguous = mapping("committed-before-lost-reply", "7".repeat(32), "8".repeat(64));
  await assert.rejects(runChild(root, ambiguous, true), /host child failed/);
  const replay = await store.putIfAbsent(mapping(ambiguous.requestId, "9".repeat(32), "a".repeat(64)));
  assert.deepEqual(replay, await store.getByRequestId(ambiguous.requestId));
  assert.equal((await lstat(join(root, "capture-mappings.sqlite"))).mode & 0o077, 0);
});

test("allocator is deterministic, private, and reserves no apparent bundle artifact", async t => {
  const root = join(await temporary(t), "bundles");
  const allocator = new PrivateBundleAllocator(root);
  const first = await allocator.allocate("request / arbitrary");
  assert.deepEqual(await allocator.allocate("request / arbitrary"), first);
  assert.equal(first.bundleId.includes("/"), false);
  assert.equal(first.bundlePath.endsWith(".apparent"), false);
  assert.equal((await lstat(first.bundlePath)).mode & 0o077, 0);
  assert.equal(await readFile(join(first.bundlePath, ".record-reservation"), "utf8"), "request / arbitrary");
  const race = await Promise.all([allocator.allocate("another request"), allocator.allocate("another request")]);
  assert.deepEqual(race[0], race[1]);
});

test("store refuses permissive roots, symlink roots, corrupt stores, and incompatible schema", async t => {
  const parent = await temporary(t);
  const permissive = join(parent, "permissive");
  await mkdir(permissive, { mode: 0o700 });
  await chmod(permissive, 0o755);
  await assert.rejects(new SqliteCaptureMappingStore(permissive).getByRequestId("x"), /storage is unavailable/);

  const target = join(parent, "target");
  await mkdir(target, { mode: 0o700 });
  const link = join(parent, "link");
  await symlink(target, link);
  await assert.rejects(new SqliteCaptureMappingStore(join(link, "child")).getByRequestId("x"), /storage is unavailable/);

  const corrupt = join(parent, "corrupt");
  await mkdir(corrupt, { mode: 0o700 });
  await writeFile(join(corrupt, "capture-mappings.sqlite"), "not sqlite", { mode: 0o600 });
  await assert.rejects(new SqliteCaptureMappingStore(corrupt).putIfAbsent(mapping("x")), /storage is unavailable/);

  const linkedDatabase = join(parent, "linked-database");
  await mkdir(linkedDatabase, { mode: 0o700 });
  const outside = join(parent, "outside.sqlite");
  await writeFile(outside, "preserve", { mode: 0o600 });
  await symlink(outside, join(linkedDatabase, "capture-mappings.sqlite"));
  const secret = "5".repeat(64);
  await assert.rejects(new SqliteCaptureMappingStore(linkedDatabase).putIfAbsent(mapping("symlink", "6".repeat(32), secret)), error => {
    assert.match(String(error), /storage is unavailable/);
    assert.equal(String(error).includes(secret), false);
    return true;
  });
  assert.equal(await readFile(outside, "utf8"), "preserve");

  const incompatible = join(parent, "incompatible");
  const store = new SqliteCaptureMappingStore(incompatible);
  await store.getByRequestId("initialize");
  const db = new DatabaseSync(join(incompatible, "capture-mappings.sqlite"));
  db.exec("PRAGMA user_version = 99");
  db.close();
  await assert.rejects(store.getByRequestId("x"), /storage is unavailable/);
});

test("bundle allocator refuses a colliding user directory and preserves it", async t => {
  const root = join(await temporary(t), "bundles");
  const allocator = new PrivateBundleAllocator(root);
  const hash = (await import("node:crypto")).createHash("sha256").update("occupied").digest("hex");
  const collision = join(root, `reservation-${hash}`);
  await mkdir(collision, { recursive: true, mode: 0o700 });
  await writeFile(join(collision, "user-data"), "keep", { mode: 0o600 });
  await assert.rejects(allocator.allocate("occupied"), /storage is unavailable/);
  assert.equal(await readFile(join(collision, "user-data"), "utf8"), "keep");
});
