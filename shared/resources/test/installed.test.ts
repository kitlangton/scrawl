import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { constants } from "node:fs";
import { chmod, copyFile, lstat, mkdir, mkdtemp, open, readFile, readdir, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { test } from "node:test";
import { NativeResourceError, NativeResources, resourceManifest } from "../src/index.ts";
import { createInstalledNativeResources, type InstalledResourceTarget } from "../src/installed.ts";
import { fixture } from "./helpers.ts";

const target: InstalledResourceTarget = process.platform === "linux" ? "linux-x64" : "macos-x64";
const supportedHost = (process.platform === "linux" && process.arch === "x64")
  || (process.platform === "darwin" && process.arch === "x64");
const hash = (bytes: Uint8Array | string) => createHash("sha256").update(bytes).digest("hex");

async function admittedArtifact(scenario = "published") {
  const source = await fixture(scenario);
  const bytes = await readFile(source.executable);
  const manifest = Buffer.from(JSON.stringify(resourceManifest("a".repeat(40), target, bytes), null, 2) + "\n");
  await writeFile(join(source.directory, "manifest.json"), manifest, { mode: 0o644 });
  return {
    source,
    options: { artifactDirectory: await realpath(source.directory), trustedManifestSha256: hash(manifest), expectedSourceSha: "a".repeat(40), target },
  };
}

test("installed provider verifies admitted bytes and runs from its owned private snapshot", { skip: !supportedHost }, async () => {
  const artifact = await admittedArtifact();
  const resources = await createInstalledNativeResources(artifact.options);
  try {
    assert(resources instanceof NativeResources, "The installed instance remains NativeResources-compatible");
    for (const operation of Reflect.ownKeys(NativeResources.prototype).filter(name => name !== 'constructor')) {
      assert(Object.hasOwn(Object.getPrototypeOf(resources), operation),
        `Public SDK operation ${String(operation)} requires an explicit installed lifetime wrapper`);
    }
    assert.notEqual(resources.executable, artifact.source.executable);
    assert.equal(resources.sha256, hash(await readFile(resources.executable)));
    assert.equal(resources.manifestSha256, artifact.options.trustedManifestSha256);
    assert.equal(resources.sourceSha, artifact.options.expectedSourceSha);
    assert.equal(resources.target, target);
    assert.equal(resources.protocol, 1);
    const staged = await lstat(resources.executable);
    assert.equal(staged.mode & 0o7777, 0o755);
    assert.equal((await lstat(join(resources.executable, ".."))).mode & 0o777, 0o700);
    const handle = await artifact.source.handle();
    assert.deepEqual(await resources.checkAcl(handle), { state: "accepted" });
    assert.equal(handle.fd, -1, "ResourceProcess.launch consumes the caller descriptor");
    await resources.close();
    await assert.rejects(lstat(resources.executable), { code: "ENOENT" });
    assert.equal((await readdir(artifact.source.directory)).includes("apparent-resource-host"), true,
      "Retirement never deletes the admitted source artifact");
  } finally {
    await resources.close().catch(() => {});
    await artifact.source.close();
  }
});

test("tracks helper lock loss and does not preserve a dead lease", { skip: !supportedHost }, async () => {
  const artifact = await admittedArtifact("lost");
  const resources = await createInstalledNativeResources(artifact.options);
  try {
    const result = await resources.lock(await artifact.source.handle(), { timeoutMs: 500 });
    assert.equal(result.state, "acquired");
    if (result.state !== "acquired") return;
    let timer: ReturnType<typeof setTimeout> | undefined;
    try {
      await Promise.race([
        new Promise<void>(resolve => result.lease.signal.addEventListener("abort", () => resolve(), { once: true })),
        new Promise<never>((_, reject) => { timer = setTimeout(() => reject(new Error("Lease loss was not observed")), 1500); }),
      ]);
    } finally { clearTimeout(timer); }
    assert.equal(result.lease.state, "lost");
    await assert.rejects(result.lease.release(), (error: unknown) => error instanceof NativeResourceError && error.category === "lease_lost");
    await resources.close();
  } finally {
    await resources.close().catch(() => {});
    await artifact.source.close();
  }
});

test("preserves an explicit helper refusal and consumes its input descriptor", { skip: !supportedHost }, async () => {
  const artifact = await admittedArtifact("refused");
  const resources = await createInstalledNativeResources(artifact.options);
  try {
    const handle = await artifact.source.handle();
    assert.deepEqual(await resources.checkAcl(handle), { state: "refused" });
    assert.equal(handle.fd, -1);
  } finally {
    await resources.close();
    await artifact.source.close();
  }
});

test("retirement refuses pending operations and live leases without releasing them", { skip: !supportedHost }, async () => {
  const artifact = await admittedArtifact("delayed-published");
  const resources = await createInstalledNativeResources(artifact.options);
  const parentDirectory = await open(artifact.source.directory, constants.O_RDONLY | constants.O_DIRECTORY);
  let pending: Promise<{ state: "published" | "destination_exists" | "refused" | "publication_uncertain" }> | undefined;
  let releaseLease: (() => Promise<void>) | undefined;
  try {
    await mkdir(join(artifact.source.directory, "publish-source"));
    pending = resources.publish(parentDirectory, "publish-source", "published", { timeoutMs: 1000 });
    await assert.rejects(resources.close(), (error: unknown) => error instanceof NativeResourceError && error.category === "invalid_request");
    assert.deepEqual(await pending, { state: "published" });
    assert.equal(parentDirectory.fd, -1, "ResourceProcess.launch consumes the transferred input handle");
    assert.equal(await lstat(resources.executable).then(() => true, () => false), true,
      "A refused retirement keeps the provider available");

    const lock = await resources.lock(await artifact.source.handle(), { timeoutMs: 100 });
    assert.equal(lock.state, "acquired");
    if (lock.state !== "acquired") return;
    releaseLease = () => lock.lease.release();
    await assert.rejects(resources.close(), (error: unknown) => error instanceof NativeResourceError && error.category === "invalid_request");
    assert.equal(lock.lease.state, "held", "Close refusal must not release a caller-owned lease");
    await releaseLease();
    releaseLease = undefined;
    assert.equal(lock.lease.state, "released");
    await resources.close();
    await assert.rejects(lstat(resources.executable), { code: "ENOENT" });
  } finally {
    await pending?.catch(() => {});
    await releaseLease?.().catch(() => {});
    if (parentDirectory.fd >= 0) await parentDirectory.close();
    await resources.close().catch(() => {});
    await artifact.source.close();
  }
});

test("calls after retirement refuse before spawning and still consume their descriptors", { skip: !supportedHost }, async () => {
  const artifact = await admittedArtifact();
  const resources = await createInstalledNativeResources(artifact.options);
  await resources.close();
  const handle = await artifact.source.handle();
  try {
    await assert.rejects(resources.checkAcl(handle), (error: unknown) => error instanceof NativeResourceError && error.category === "provider_unavailable");
    assert.equal(handle.fd, -1, "Closed providers consume descriptors even on local refusal");
  } finally { await artifact.source.close(); }
});

test("rejects untrusted manifest, source, target, helper mode, symlink and extra inventory before staging", { skip: !supportedHost }, async () => {
  const artifact = await admittedArtifact();
  try {
    await assert.rejects(createInstalledNativeResources({ ...artifact.options, trustedManifestSha256: "0".repeat(64) }), /provider_mismatch/);
    await assert.rejects(createInstalledNativeResources({ ...artifact.options, expectedSourceSha: "b".repeat(40) }), /provider_mismatch/);
    await assert.rejects(createInstalledNativeResources({ ...artifact.options, target: target === "linux-x64" ? "macos-x64" : "linux-x64" }), /provider_mismatch/);
    await assert.rejects(createInstalledNativeResources({ ...artifact.options, artifactDirectory: artifact.source.directory + "/." }), /provider_mismatch/);

    await chmod(artifact.source.executable, 0o775);
    await assert.rejects(createInstalledNativeResources(artifact.options), /provider_mismatch/);
    await chmod(artifact.source.executable, 0o755);

    const aliasTarget = join(artifact.source.directory, "real-helper");
    await copyFile(artifact.source.executable, aliasTarget);
    await chmod(aliasTarget, 0o755);
    await rm(artifact.source.executable);
    await symlink(aliasTarget, artifact.source.executable);
    await assert.rejects(createInstalledNativeResources(artifact.options), /provider_mismatch/);
    await rm(artifact.source.executable);
    await copyFile(aliasTarget, artifact.source.executable);
    await chmod(artifact.source.executable, 0o755);
    await writeFile(artifact.source.executable, "#!/bin/sh\nexit 0\n");
    await chmod(artifact.source.executable, 0o755);
    await assert.rejects(createInstalledNativeResources(artifact.options), /provider_mismatch/);
    await copyFile(aliasTarget, artifact.source.executable);
    await chmod(artifact.source.executable, 0o755);
    await writeFile(join(artifact.source.directory, "unexpected"), "extra");
    await assert.rejects(createInstalledNativeResources(artifact.options), /provider_mismatch/);
    await rm(join(artifact.source.directory, "unexpected"));

    await writeFile(join(artifact.source.directory, "manifest.json"), "{}\n");
    await assert.rejects(createInstalledNativeResources(artifact.options), /provider_mismatch/);
    assert.equal((await lstat(artifact.source.executable)).isFile(), true, "Rejection never deletes caller artifacts");
  } finally { await artifact.source.close(); }
});

test('snapshots external admission identity before asynchronous filesystem work', { skip: !supportedHost }, async () => {
  const artifact = await admittedArtifact();
  let resources: Awaited<ReturnType<typeof createInstalledNativeResources>> | undefined;
  try {
    const options = { ...artifact.options };
    const pending = createInstalledNativeResources(options);
    options.expectedSourceSha = 'b'.repeat(40);
    options.trustedManifestSha256 = '0'.repeat(64);
    resources = await pending;
    assert.equal(resources.sourceSha, artifact.options.expectedSourceSha);
    assert.equal(resources.manifestSha256, artifact.options.trustedManifestSha256);
  } finally { await resources?.close(); await artifact.source.close(); }
});

test('uses a canonical private stage even when the system temp directory is aliased', { skip: !supportedHost }, async () => {
  const tempRoot = await mkdtemp(join(await realpath(tmpdir()), "installed-temp-alias-"));
  const prior = process.env.TMPDIR;
  let artifact: Awaited<ReturnType<typeof admittedArtifact>> | undefined;
  let resources: Awaited<ReturnType<typeof createInstalledNativeResources>> | undefined;
  try {
    const actual = join(tempRoot, 'temp-parent');
    const alias = join(tempRoot, 'temp-alias');
    await mkdir(actual, { mode: 0o700 }); await symlink(actual, alias);
    process.env.TMPDIR = alias;
    artifact = await admittedArtifact();
    assert.equal(artifact.options.artifactDirectory, await realpath(artifact.source.directory), 'The privileged caller supplies the canonical admitted directory');
    resources = await createInstalledNativeResources(artifact.options);
    assert(resources.executable.startsWith(actual + '/'));
    assert.throws(() => { (resources as unknown as { sha256: string }).sha256 = '0'.repeat(64); }, TypeError);
  } finally {
    if (prior === undefined) delete process.env.TMPDIR; else process.env.TMPDIR = prior;
    await resources?.close(); await artifact?.source.close(); await rm(tempRoot, { recursive: true, force: true });
  }
});

test('replacement dispatch holds the installed lifetime and retirement refuses it without dispatch', { skip: !supportedHost, timeout: 2000 }, async () => {
  const artifact = await admittedArtifact();
  let resources: Awaited<ReturnType<typeof createInstalledNativeResources>> | undefined;
  let operation: Promise<unknown> | undefined;
  let effects = 0;
  let entered!: () => void, release!: () => void;
  const dispatched = new Promise<void>(resolve => { entered = resolve; });
  const pending = new Promise<void>(resolve => { release = resolve; });
  const originalReplacement = Object.getOwnPropertyDescriptor(NativeResources.prototype, 'replaceFile')!;
  Object.defineProperty(NativeResources.prototype, 'replaceFile', { configurable: true, async value(parent: Awaited<ReturnType<typeof open>>) {
    effects++; entered(); await pending; await parent.close(); return { state: 'replaced' };
  } });
  try {
    resources = await createInstalledNativeResources(artifact.options);
    assert.deepEqual(await resources.checkAcl(await artifact.source.handle()), { state: 'accepted' });
    const live = await open(artifact.source.directory, constants.O_RDONLY | constants.O_DIRECTORY);
    operation = resources.replaceFile(live, 'source', 'destination', { source: { device: '1', inode: '1' }, destination: null });
    const started = await Promise.race([dispatched.then(() => true), operation.then(() => false, () => false)]);
    assert.equal(started, true, 'The qualified SDK dispatch must be reachable');
    await assert.rejects(resources.close(), { category: 'invalid_request' });
    assert.equal((await lstat(resources.executable)).isFile(), true, 'Pending replacement retains its staged provider');
    release(); assert.deepEqual(await operation, { state: 'replaced' }); assert.equal(live.fd, -1);
    await resources.close();
    const closed = await open(artifact.source.directory, constants.O_RDONLY | constants.O_DIRECTORY);
    await assert.rejects(resources.replaceFile(closed, 'source', 'destination', { source: { device: '1', inode: '1' }, destination: null }), { category: 'provider_unavailable' });
    assert.equal(closed.fd, -1); assert.equal(effects, 1, 'No dispatch after close or automatic retry');
  } finally {
    release(); await operation?.catch(() => {});
    Object.defineProperty(NativeResources.prototype, 'replaceFile', originalReplacement);
    await resources?.close(); await artifact.source.close();
  }
});
