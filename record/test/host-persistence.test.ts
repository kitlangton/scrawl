import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { chmod, link, lstat, mkdir, mkdtemp, readFile, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { test } from "node:test";
import { initializePrivateSqlite } from "../dist/backend/host/sqlite-initialization.js";
import { SqliteCaptureMappingStore } from "../dist/backend/host/capture-mapping-store.js";
import { PrivateBundleAllocator } from "../dist/backend/host/bundle-allocator.js";
import { EngineSdkAdapter, type EngineSdkPort, type PreparedStart } from "../dist/backend/engine/sdk-adapter.js";

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
    const child = spawn(process.execPath, ["--experimental-strip-types", new URL("./helpers/host-child.ts", import.meta.url).pathname,
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
  assert.deepEqual(await store.listRequestIds(), [first.requestId]);
  assert.equal(JSON.stringify(await store.listRequestIds()).includes(first.ownerToken), false);
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

test("database hard links are refused without changing the outside SQLite file", async t => {
  const parent = await temporary(t);
  const outside = join(parent, "outside.sqlite");
  const db = new DatabaseSync(outside);
  db.exec("CREATE TABLE private_data (value TEXT); INSERT INTO private_data VALUES ('preserve')");
  db.close(); await chmod(outside, 0o600);
  const original = await readFile(outside);
  const root = join(parent, "mapping"); await mkdir(root, { mode: 0o700 });
  await link(outside, join(root, "capture-mappings.sqlite"));
  const store = new SqliteCaptureMappingStore(root);
  await assert.rejects(store.putIfAbsent(mapping("hardlink")), /storage is unavailable/);
  await assert.rejects(store.getByRequestId("hardlink"), /storage is unavailable/);
  assert.deepEqual(await readFile(outside), original);
});

test("allocator recovers an empty interrupted reservation and preserves mismatching markers", async t => {
  const root = join(await temporary(t), "bundles");
  const requestId = "interrupted creator";
  const hash = (await import("node:crypto")).createHash("sha256").update(requestId).digest("hex");
  const abandoned = join(root, `reservation-${hash}`);
  // Filesystem state after a process exits between mkdir and marker publication.
  await mkdir(abandoned, { recursive: true, mode: 0o700 });
  const allocator = new PrivateBundleAllocator(root);
  const winners = await Promise.all([allocator.allocate(requestId), allocator.allocate(requestId)]);
  assert.deepEqual(winners[0], winners[1]);
  assert.equal(winners[0]!.bundlePath, abandoned);
  assert.equal(await readFile(join(abandoned, ".record-reservation"), "utf8"), requestId);
  await writeFile(join(abandoned, ".record-reservation"), "another owner", { mode: 0o600 });
  await assert.rejects(allocator.allocate(requestId), /storage is unavailable/);
  assert.equal(await readFile(join(abandoned, ".record-reservation"), "utf8"), "another owner");
});

test("SQLite sidecars remain private and pre-existing sidecar aliases are refused", async t => {
  const parent = await temporary(t);
  const root = join(parent, "live");
  const store = new SqliteCaptureMappingStore(root);
  await store.putIfAbsent(mapping("private sidecars"));
  const db = new DatabaseSync(join(root, "capture-mappings.sqlite"));
  try {
    db.exec("PRAGMA journal_mode=WAL; BEGIN IMMEDIATE; UPDATE capture_mapping SET request_id=request_id; COMMIT");
    await store.getByRequestId("private sidecars");
    for (const suffix of ["", "-wal", "-shm"]) {
      const info = await lstat(join(root, `capture-mappings.sqlite${suffix}`));
      assert.equal(info.mode & 0o077, 0);
      assert.equal(info.uid, process.getuid?.());
      assert.equal(info.nlink, 1);
    }
  } finally { db.close(); }
  for (const suffix of ["-wal", "-shm", "-journal"]) {
    const unsafe = join(parent, `unsafe${suffix}`); await mkdir(unsafe, { mode: 0o700 });
    const outside = join(parent, `outside${suffix}`); await writeFile(outside, "preserve", { mode: 0o600 });
    await symlink(outside, join(unsafe, `capture-mappings.sqlite${suffix}`));
    await assert.rejects(new SqliteCaptureMappingStore(unsafe).getByRequestId("x"), /storage is unavailable/);
    assert.equal(await readFile(outside, "utf8"), "preserve");
  }
});

test("first-open WAL initialization retries SQLite lock contention and stops after a bounded budget", async t => {
  const parent = await temporary(t);
  const root = join(parent, "mapping");
  await mkdir(root, { mode: 0o700 });
  const path = join(root, "capture-mappings.sqlite");
  const lock = new DatabaseSync(path);
  await chmod(path, 0o600);
  lock.exec("BEGIN EXCLUSIVE");
  const release = setTimeout(() => lock.exec("ROLLBACK"), 80);
  try {
    assert.deepEqual(await new SqliteCaptureMappingStore(root).listRequestIds(), []);
  } finally { clearTimeout(release); try { lock.exec("ROLLBACK"); } catch { /* timer may have released it */ } lock.close(); }

  const blocked = join(parent, "blocked");
  await mkdir(blocked, { mode: 0o700 });
  const blockedDb = new DatabaseSync(join(blocked, "capture-mappings.sqlite"));
  await chmod(join(blocked, "capture-mappings.sqlite"), 0o600);
  blockedDb.exec("BEGIN EXCLUSIVE");
  const started = Date.now();
  try {
    await assert.rejects(new SqliteCaptureMappingStore(blocked).listRequestIds(), /storage is unavailable/);
    assert.ok(Date.now() - started < 2000, "WAL retry must stop within its bounded retry budget");
  } finally { blockedDb.exec("ROLLBACK"); blockedDb.close(); }
});

test("real adapter recovers persisted identity after an ambiguous start and host recreation", async t => {
  const parent = await temporary(t);
  const allocator = new PrivateBundleAllocator(join(parent, "bundles"));
  const mappings = () => new SqliteCaptureMappingStore(join(parent, "mapping"));
  const started: PreparedStart[] = [];
  let preparations = 0;
  const sdk: EngineSdkPort = {
    clientId: "c".repeat(32),
    handshake: async () => ({ implementation: "native", capabilities: { screenCapture: false }, readiness: "unavailable" }),
    prepareStart: (bundleId, bundlePath) => {
      preparations++;
      return { protocolVersion: 1, clientId: "c".repeat(32), rpcId: "d".repeat(32), method: "start", captureId: "a".repeat(32), ownerToken: "b".repeat(64), bundleId, bundlePath, options: { screen: true } };
    },
    start: async prepared => { started.push(prepared); if (started.length === 1) throw new Error("lost reply"); return { captureId: prepared.captureId, ownerToken: prepared.ownerToken, receipt: { captureId: prepared.captureId, state: "starting" } }; },
    control: async (_method, captureId, token) => { assert.equal(token, "b".repeat(64)); return { captureId, state: "recording" }; },
    close: async () => { throw new Error("adapter must not close the shared SDK"); },
  };
  const first = new EngineSdkAdapter(sdk, { mappings: mappings(), allocateBundle: id => allocator.allocate(id) });
  const id = "arbitrary public request / restart";
  await assert.rejects(first.start(id), /outcome is unknown/);
  first.dispose();
  // Simulate native contents arriving before the host reconnects. Existing identity
  // must win without asking the allocator to claim this populated destination.
  await writeFile(join(started[0]!.bundlePath, "native-pending"), "controlled fixture");
  const second = new EngineSdkAdapter(sdk, { mappings: mappings(), allocateBundle: async () => { throw new Error("must reuse stored allocation"); } });
  try {
    assert.equal((await second.inspect(id)).captureId, id);
    const result = await second.start(id);
    assert.equal(result.captureId, id);
    assert.equal(result.phase, "recording", "observed phase cannot regress on a replay receipt");
    assert.equal(preparations, 1);
    assert.equal(started[1]!.bundlePath, started[0]!.bundlePath);
    assert.equal(started[1]!.captureId, started[0]!.captureId);
    assert.equal(JSON.stringify(result).includes("b".repeat(64)), false);
  } finally { second.dispose(); }
});


test("WAL initialization restores the normal transaction busy timeout", async t => {
  const root=await temporary(t);
  const db=new DatabaseSync(join(root,"timeout.sqlite"));
  try {
    await initializePrivateSqlite(db);
    assert.equal(db.prepare("PRAGMA busy_timeout").get()?.timeout,5000);
    assert.equal(db.prepare("PRAGMA synchronous").get()?.synchronous,2);
    assert.equal(db.prepare("PRAGMA journal_mode").get()?.journal_mode,"wal");
  } finally { db.close(); }
});
