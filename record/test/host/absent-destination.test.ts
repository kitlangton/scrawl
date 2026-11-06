import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { chmod, link, lstat, mkdir, mkdtemp, readFile, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { test } from "node:test";
import { AbsentDestinationAllocator, type AbsentDestinationBinding } from "../../dist/backend/host/index.js";

async function temporary(t: { after(fn: () => void | Promise<void>): void }): Promise<string> {
  const root = await realpath(await mkdtemp(join(tmpdir(), "record-capture2-allocator-")));
  t.after(() => rm(root, { recursive: true, force: true }));
  return root;
}

function child(control: string, output: string, requestId: string, loseReply = false, exactPath?: string): Promise<{ stdout: string; code: number | null }> {
  return new Promise((resolve, reject) => {
    const args = ["--experimental-strip-types", new URL("./absent-destination-child.ts", import.meta.url).pathname, control, output, requestId];
    if (exactPath) args.push(exactPath);
    const proc = spawn(process.execPath, args, {
      stdio: ["ignore", "pipe", "pipe"], env: { ...process.env, ...(loseReply ? { RECORD_ALLOCATOR_LOSE_REPLY: "1" } : {}) },
    });
    let stdout = "", stderr = "";
    proc.stdout.setEncoding("utf8").on("data", chunk => stdout += chunk);
    proc.stderr.setEncoding("utf8").on("data", chunk => stderr += chunk);
    proc.on("error", reject);
    proc.on("close", code => code === 0 ? resolve({ stdout, code }) : reject(new Error(`allocator child failed: ${stderr}`)));
  });
}

function roots(parent: string) { return [join(parent, "control"), join(parent, "bundles")] as const; }

async function assertAbsent(path: string) {
  await assert.rejects(lstat(path), error => (error as NodeJS.ErrnoException).code === "ENOENT");
}

test("concurrent processes choose one durable binding; restart and lost reply replay it", async t => {
  const parent = await temporary(t), [control, output] = roots(parent), request = " arbitrary/request id with private text ";
  const [a, b] = await Promise.all([child(control, output, request), child(control, output, request)]);
  const binding: AbsentDestinationBinding = JSON.parse(a.stdout);
  assert.deepEqual(binding, JSON.parse(b.stdout));
  assert.match(binding.bundleId, /^record-[0-9a-f]{64}\.apparent$/u);
  assert.equal(binding.bundlePath, join(output, binding.bundleId));
  assert.equal(binding.bundlePath.includes(request), false);
  await assertAbsent(binding.bundlePath);
  const lost = await child(control, output, "lost reply request", true);
  assert.equal(lost.stdout, "");
  const replay = await new AbsentDestinationAllocator(control, output).allocate("lost reply request");
  assert.equal(replay.bundlePath, join(output, replay.bundleId));
  await assertAbsent(replay.bundlePath);
  const dbInfo = await lstat(join(control, "capture2-destination-bindings.sqlite"));
  assert.equal(dbInfo.mode & 0o077, 0);
});

test("default allocation accepts an owned 0755 destination root without weakening private storage", async t => {
  const parent = await temporary(t), control = join(parent, "control"), output = join(parent, "ordinary-output");
  await mkdir(output, { mode: 0o755 }); await chmod(output, 0o755);
  const before = await lstat(output), allocator = new AbsentDestinationAllocator(control, output);
  const request = "ordinary default output", binding = await allocator.allocate(request);
  assert.equal(binding.bundlePath, join(output, binding.bundleId));
  assert.equal((await lstat(output)).mode & 0o777, before.mode & 0o777, "allocation never changes the configured directory mode");
  await assertAbsent(binding.bundlePath);
  assert.equal((await lstat(control)).mode & 0o777, 0o700);
  assert.equal((await lstat(join(control, "capture2-destination-bindings.sqlite"))).mode & 0o777, 0o600);
  for (const suffix of ["-wal", "-shm", "-journal"]) {
    const path = join(control, `capture2-destination-bindings.sqlite${suffix}`);
    try { assert.equal((await lstat(path)).mode & 0o777, 0o600); }
    catch (error) { if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error; }
  }

  await mkdir(binding.bundlePath, { mode: 0o755 });
  await writeFile(join(binding.bundlePath, "published.bytes"), "synthetic publication", { mode: 0o600 });
  assert.deepEqual(await new AbsentDestinationAllocator(control, output).allocate(request), binding, "reopen replays the exact binding after publication populates the path");
  assert.equal(await readFile(join(binding.bundlePath, "published.bytes"), "utf8"), "synthetic publication");

  const occupiedRequest = "occupied default output";
  const digest = (await import("node:crypto")).createHash("sha256").update(occupiedRequest).digest("hex");
  const occupied = join(output, `record-${digest}.apparent`);
  await mkdir(occupied, { mode: 0o700 }); await writeFile(join(occupied, "keep"), "user data", { mode: 0o600 });
  await assert.rejects(allocator.allocate(occupiedRequest), /storage is unavailable/u);
  assert.equal(await readFile(join(occupied, "keep"), "utf8"), "user data");

  const missingOutput = join(parent, "new-private-output");
  const created = await new AbsentDestinationAllocator(join(parent, "new-control"), missingOutput).allocate("new root");
  assert.equal((await lstat(missingOutput)).mode & 0o777, 0o700, "a newly created output root keeps its private default mode");
  await assertAbsent(created.bundlePath);
});

test("default allocation rejects unsafe, aliased, non-directory, and foreign destination roots", async t => {
  const parent = await temporary(t);
  for (const [name, mode] of [["other-writable", 0o757], ["group-and-other-writable", 0o777]] as const) {
    const root = join(parent, name); await mkdir(root, { mode: 0o700 }); await chmod(root, mode);
    await assert.rejects(new AbsentDestinationAllocator(join(parent, `${name}-control`), root).allocate(name), /storage is unavailable/u);
  }
  const target = join(parent, "target"); await mkdir(target, { mode: 0o700 });
  const alias = join(parent, "destination-alias"); await symlink(target, alias);
  await assert.rejects(new AbsentDestinationAllocator(join(parent, "alias-control"), alias).allocate("alias"), /storage is unavailable/u);
  const nonDirectory = join(parent, "not-a-directory"); await writeFile(nonDirectory, "preserve");
  await assert.rejects(new AbsentDestinationAllocator(join(parent, "file-control"), nonDirectory).allocate("non-directory"), /storage is unavailable/u);
  assert.equal(await readFile(nonDirectory, "utf8"), "preserve");

  const foreign = await lstat("/root").catch(() => undefined);
  if (foreign && foreign.uid !== process.getuid?.()) {
    await assert.rejects(new AbsentDestinationAllocator(join(parent, "foreign-control"), "/root").allocate("foreign"), /storage is unavailable/u);
  }
});

test("first reservation refuses an existing final path and preserves it; known replay tolerates later population", async t => {
  const parent = await temporary(t), [control, output] = roots(parent), request = "occupied";
  const digest = (await import("node:crypto")).createHash("sha256").update(request).digest("hex");
  const collision = join(output, `record-${digest}.apparent`);
  await mkdir(collision, { recursive: true, mode: 0o700 });
  await writeFile(join(collision, "keep"), "user", { mode: 0o600 });
  const allocator = new AbsentDestinationAllocator(control, output);
  await assert.rejects(allocator.allocate(request), /storage is unavailable/);
  assert.equal(await readFile(join(collision, "keep"), "utf8"), "user");
  const db = new DatabaseSync(join(control, "capture2-destination-bindings.sqlite"));
  try { assert.equal(Number(db.prepare("SELECT count(*) AS count FROM destination_binding").get()?.count), 0); }
  finally { db.close(); }

  const first = await allocator.allocate("known");
  await mkdir(first.bundlePath, { mode: 0o700 });
  await writeFile(join(first.bundlePath, "native-published"), "synthetic", { mode: 0o600 });
  assert.deepEqual(await allocator.allocate("known"), first);
  assert.equal(await readFile(join(first.bundlePath, "native-published"), "utf8"), "synthetic");
});

test("same request cannot change roots; output/control roots must be safely separate", async t => {
  const parent = await temporary(t), [control, output] = roots(parent);
  const first = await new AbsentDestinationAllocator(control, output).allocate("fixed");
  await assert.rejects(new AbsentDestinationAllocator(control, join(parent, "other-output")).allocate("fixed"), /storage is unavailable/);
  await assert.rejects(new AbsentDestinationAllocator(join(parent, "nested"), join(parent, "nested", "output")).allocate("x"), /storage is unavailable/);
  await assertAbsent(first.bundlePath);
});

test("symlink, hard-linked and permissive store paths are rejected without touching aliases", async t => {
  const parent = await temporary(t), [control, output] = roots(parent);
  const good = new AbsentDestinationAllocator(control, output);
  const binding = await good.allocate("base");
  await assertAbsent(binding.bundlePath);

  const target = join(parent, "target"); await mkdir(target, { mode: 0o700 });
  const linkRoot = join(parent, "link-root"); await symlink(target, linkRoot);
  await assert.rejects(new AbsentDestinationAllocator(linkRoot, join(parent, "other")).allocate("symlink"), /storage is unavailable/);
  await assert.rejects(new AbsentDestinationAllocator(join(parent, "control-symlink"), linkRoot).allocate("output-symlink"), /storage is unavailable/);

  const permissive = join(parent, "permissive"); await mkdir(permissive, { mode: 0o700 }); await chmod(permissive, 0o755);
  await assert.rejects(new AbsentDestinationAllocator(permissive, join(parent, "out-permissive")).allocate("permissive"), /storage is unavailable/);

  const dbPath = join(control, "capture2-destination-bindings.sqlite"), alias = join(parent, "database-alias");
  await link(dbPath, alias);
  await assert.rejects(good.allocate("hardlink"), /storage is unavailable/);
  assert.equal(await lstat(alias).then(info => info.nlink), 2);
  await assertAbsent(join(output, `record-${(await import("node:crypto")).createHash("sha256").update("hardlink").digest("hex")}.apparent`));
  assert.equal(dirname(binding.bundlePath), output);
});

test("same-process concurrent allocations resolve to durable bindings and leave every final path absent", async t => {
  const parent = await temporary(t), [control, output] = roots(parent);
  const allocator = new AbsentDestinationAllocator(control, output);
  const requests = ["same request", "other request", "same request", "third request", "other request", "unicode λ 🐸"];
  const bindings = await Promise.all(requests.map(request => allocator.allocate(request)));
  const byRequest = new Map<string, AbsentDestinationBinding>();
  for (let index = 0; index < requests.length; index++) {
    const request = requests[index]!, binding = bindings[index]!;
    const prior = byRequest.get(request);
    if (prior) assert.deepEqual(binding, prior);
    else byRequest.set(request, binding);
    assert.equal(binding.bundlePath, join(output, binding.bundleId));
    await assertAbsent(binding.bundlePath);
  }
  assert.equal(new Set([...byRequest.values()].map(binding => binding.bundleId)).size, byRequest.size);
  for (const [request, binding] of byRequest) assert.deepEqual(await allocator.allocate(request), binding);
});

test("tampered durable bundle identity is rejected without following or modifying its path", async t => {
  const parent = await temporary(t), [control, output] = roots(parent);
  const allocator = new AbsentDestinationAllocator(control, output);
  const dbPath = join(control, "capture2-destination-bindings.sqlite");
  const foreign = join(parent, "foreign-destination");
  await mkdir(foreign, { mode: 0o700 });
  await writeFile(join(foreign, "keep"), "synthetic", { mode: 0o600 });

  for (const [request, column, value] of [
    ["tampered id", "bundle_id", "foreign.apparent"],
    ["tampered path", "bundle_path", foreign],
  ] as const) {
    const binding = await allocator.allocate(request);
    const digest = (await import("node:crypto")).createHash("sha256").update(request).digest("hex");
    const db = new DatabaseSync(dbPath);
    try {
      db.prepare(`UPDATE destination_binding SET ${column} = ? WHERE request_digest = ?`).run(value, digest);
    } finally { db.close(); }

    await assert.rejects(allocator.allocate(request), error => {
      assert.equal((error as Error).message, "Record host storage is unavailable or invalid");
      assert.equal((error as Error).message.includes(foreign), false);
      return true;
    });
    await assertAbsent(binding.bundlePath);
    assert.equal(await readFile(join(foreign, "keep"), "utf8"), "synthetic");
  }
});

test("ill-formed UTF-16 request IDs are rejected before creating allocator metadata", async t => {
  const parent = await temporary(t), [control, output] = roots(parent);
  const allocator = new AbsentDestinationAllocator(control, output);
  await assert.rejects(allocator.allocate("invalid-\ud800-id"), /storage is unavailable/u);
  await assertAbsent(join(control, "capture2-destination-bindings.sqlite"));
  await assertAbsent(output);
});

test("exact reservation preserves parent permissions, binds exact path, and replays after publication", async t => {
  const parent = await temporary(t), control = join(parent, "control"), userParent = join(parent, "ordinary-output");
  await mkdir(userParent, { mode: 0o755 }); await chmod(userParent, 0o755);
  const before = await lstat(userParent), path = join(userParent, "requested.apparent");
  const allocator = new AbsentDestinationAllocator(control, join(parent, "bundles"));
  const binding = await allocator.reserveExact("exact request", path);
  assert.equal(binding.bundlePath, path);
  assert.match(binding.bundleId, /^record-[0-9a-f]{64}\.apparent$/u);
  assert.notEqual(binding.bundleId, "requested.apparent");
  assert.equal((await lstat(userParent)).mode, before.mode);
  await assertAbsent(path);
  await assert.rejects(allocator.reserveExact("exact request", join(userParent, "changed.apparent")), /storage is unavailable/u);
  await mkdir(path, { mode: 0o755 }); await writeFile(join(path, "native.bytes"), Buffer.from([0, 1, 2, 255]));
  assert.deepEqual(await new AbsentDestinationAllocator(control, join(parent, "other-root")).reserveExact("exact request", path), binding);
  assert.deepEqual(await readFile(join(path, "native.bytes")), Buffer.from([0, 1, 2, 255]));
});

test("exact reservations reject occupied, malformed, aliased, and control-namespace paths", async t => {
  const parent = await temporary(t), control = join(parent, "control"), output = join(parent, "ordinary");
  await mkdir(output, { mode: 0o755 });
  const allocator = new AbsentDestinationAllocator(control, join(parent, "bundles"));
  const occupied = join(output, "occupied.apparent"); await writeFile(occupied, "preserve");
  await assert.rejects(allocator.reserveExact("occupied", occupied), /storage is unavailable/u);
  assert.equal(await readFile(occupied, "utf8"), "preserve");
  const occupiedDirectory = join(output, "occupied-directory.apparent"); await mkdir(occupiedDirectory);
  await assert.rejects(allocator.reserveExact("occupied-directory", occupiedDirectory), /storage is unavailable/u);
  const occupiedLink = join(output, "occupied-link.apparent"); await symlink(occupied, occupiedLink);
  await assert.rejects(allocator.reserveExact("occupied-link", occupiedLink), /storage is unavailable/u);
  assert.equal(await readFile(occupied, "utf8"), "preserve");
  await assert.rejects(allocator.reserveExact("bad", `${output}/../normalized.apparent`), /storage is unavailable/u);
  await assert.rejects(allocator.reserveExact("bad", join(control, "inside.apparent")), /storage is unavailable/u);
  await assert.rejects(allocator.reserveExact("bad", join(parent, "missing", "file.apparent")), /storage is unavailable/u);
  const alias = join(parent, "alias"); await symlink(output, alias);
  await assert.rejects(allocator.reserveExact("bad", join(alias, "file.apparent")), /storage is unavailable/u);
  await assert.rejects(allocator.reserveExact("bad", join(output, "nul\0path")), /storage is unavailable/u);
  await assert.rejects(allocator.reserveExact("bad", join(output, "λ".repeat(2100))), /storage is unavailable/u);
});

test("subprocess reservations serialize same-request and same-path races durably", async t => {
  const parent = await temporary(t), control = join(parent, "control"), output = join(parent, "ordinary");
  await mkdir(output, { mode: 0o755 });
  const sharedPath = join(output, "shared.apparent");
  const sameRequest = await Promise.all([child(control, join(parent, "bundles"), "same", false, sharedPath), child(control, join(parent, "bundles"), "same", false, sharedPath)]);
  assert.deepEqual(JSON.parse(sameRequest[0]!.stdout), JSON.parse(sameRequest[1]!.stdout));
  const changedPath = await Promise.allSettled([
    child(control, join(parent, "bundles"), "changed-request", false, join(output, "winner-a.apparent")),
    child(control, join(parent, "bundles"), "changed-request", false, join(output, "winner-b.apparent")),
  ]);
  assert.equal(changedPath.filter(result => result.status === "fulfilled").length, 1);
  const competing = await Promise.allSettled([
    child(control, join(parent, "bundles"), "left", false, join(output, "left.apparent")),
    child(control, join(parent, "bundles"), "right", false, join(output, "right.apparent")),
  ]);
  assert.equal(
    competing.filter(result => result.status === "fulfilled").length,
    2,
    `independent absent destinations must both reserve; subprocess failures: ${competing
      .map((result, index) => result.status === "rejected" ? `child ${index}: ${String(result.reason)}` : undefined)
      .filter(Boolean)
      .join(" | ")}`,
  );
  const samePath = await Promise.allSettled([
    child(control, join(parent, "bundles"), "path-a", false, join(output, "contended.apparent")),
    child(control, join(parent, "bundles"), "path-b", false, join(output, "contended.apparent")),
  ]);
  assert.equal(samePath.filter(result => result.status === "fulfilled").length, 1);
});


test("exact destination cannot contain private control storage, even when both paths are absent", async t => {
  const parent = await temporary(t);
  const destination = join(parent, "requested.apparent");
  const control = join(destination, "private-control");
  const allocator = new AbsentDestinationAllocator(control, join(parent, "default-output"));
  await assert.rejects(allocator.reserveExact("ancestor-overlap", destination), /storage is unavailable/u);
  await assertAbsent(destination);
  await assertAbsent(control);
});
