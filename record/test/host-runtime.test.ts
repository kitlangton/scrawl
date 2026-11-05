import assert from "node:assert/strict";
import { chmod, mkdtemp, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { SqliteCaptureMappingStore } from "../dist/backend/host/capture-mapping-store.js";
import { createRecordHostRuntime } from "../dist/backend/host/runtime.js";
import type { EngineSdkPort, PreparedStart, SdkReceipt } from "../dist/backend/engine/sdk-adapter.js";

const token = "b".repeat(64);

function controlledSdk() {
  const facts = new Map<string, SdkReceipt>();
  const inspectFailures = new Set<string>();
  let nativeReady = false;
  let starts = 0;
  let loseNextStartReply = false;
  const sdk: EngineSdkPort = {
    clientId: "c".repeat(32),
    handshake: async () => ({ implementation: "native", capabilities: { screenCapture: nativeReady }, readiness: nativeReady ? "ready" : "unavailable" }),
    prepareStart: (bundleId, bundlePath): PreparedStart => {
      const captureId = (++starts).toString(16).padStart(32, "0");
      return { protocolVersion: 1, clientId: "c".repeat(32), rpcId: "d".repeat(32), method: "start", captureId, ownerToken: token, bundleId, bundlePath, options: { screen: true } };
    },
    start: async request => {
      facts.set(request.captureId, { captureId: request.captureId, state: "recording" });
      if (loseNextStartReply) { loseNextStartReply = false; throw new Error("lost reply fixture"); }
      return { captureId: request.captureId, ownerToken: token, receipt: facts.get(request.captureId)! };
    },
    control: async (method, captureId, ownerToken) => {
      assert.equal(ownerToken, token);
      if (inspectFailures.has(captureId)) throw new Error("private inspection failure");
      const fact = facts.get(captureId);
      if (!fact) return { captureId, state: "rejected", error: { code: "unknown", message: "not found" } };
      if (method === "stop" && fact.state === "recording") {
        const mapping = await mappings.getByCaptureId(captureId);
        assert.ok(mapping);
        const saved = { captureId, state: "saved" as const, artifactProvenance: "native" as const, bundlePath: mapping.bundlePath };
        facts.set(captureId, saved);
        return saved;
      }
      return fact;
    },
    close: async () => { throw new Error("runtime must not close caller-owned SDK"); },
  };
  let mappings = new SqliteCaptureMappingStore("/tmp/not-used-yet");
  return {
    sdk,
    facts,
    inspectFailures,
    setReady(value: boolean) { nativeReady = value; },
    loseStartReply() { loseNextStartReply = true; },
    setMappings(value: SqliteCaptureMappingStore) { mappings = value; },
  };
}

async function fixture(t: { after(fn: () => void | Promise<void>): void }) {
  const root = await realpath(await mkdtemp(join(tmpdir(), "record-runtime-")));
  t.after(() => rm(root, { recursive: true, force: true }));
  const roots = { mappingRoot: join(root, "mapping"), bundleRoot: join(root, "bundles") };
  const provider = controlledSdk();
  provider.setMappings(new SqliteCaptureMappingStore(roots.mappingRoot));
  const create = () => createRecordHostRuntime(provider.sdk, roots);
  return { provider, roots, create };
}

test("clean boot admits no capture while native readiness is unavailable", async t => {
  const { provider, create } = await fixture(t);
  const runtime = await create();
  assert.deepEqual(runtime.startup, { state: "ready", knownRequestIds: [] });
  assert.deepEqual(await runtime.dispatch({ type: "start", requestId: "blocked by native readiness" }), {
    phase: "idle", readiness: { ready: false, reason: "Native Engine capture is unavailable" },
  });
  assert.equal(runtime.startup.knownRequestIds.length, 0);
  assert.equal(provider.facts.size, 0);
  assert.deepEqual(runtime.dispose(), { requestIds: [], storageUnavailable: false, reconciliation: "complete" });
});

test("restart keeps active and lost-reply ownership blocked until terminal inspection", async t => {
  const { provider, roots, create } = await fixture(t);
  provider.setReady(true);
  provider.loseStartReply();
  const first = await create();
  const id = "restart request / public identity";
  const afterLostReply = await first.dispatch({ type: "start", requestId: id });
  assert.equal(afterLostReply.phase, "uncertain");
  assert.deepEqual(first.dispose(), { requestIds: [id], storageUnavailable: false, reconciliation: "complete" });

  const restarted = await create();
  assert.deepEqual(restarted.startup, { state: "blocked", knownRequestIds: [id], unresolved: [{ requestId: id, state: "active", phase: "recording" }] });
  assert.equal((await restarted.dispatch({ type: "start", requestId: "must wait" })).phase, "idle");
  assert.equal((await new SqliteCaptureMappingStore(roots.mappingRoot).getByRequestId(id))?.ownerToken, token);
  const mapping = await new SqliteCaptureMappingStore(roots.mappingRoot).getByRequestId(id);
  assert.ok(mapping);
  provider.facts.set(mapping.captureId, { captureId: mapping.captureId, state: "saved", bundlePath: mapping.bundlePath, artifactProvenance: "native" });
  assert.deepEqual(await restarted.reconcileStartup(), { state: "ready", knownRequestIds: [id] });
  assert.equal((await restarted.dispatch({ type: "start", requestId: id })).phase, "idle", "terminal history cannot reuse its request identity");
  assert.equal(JSON.stringify(restarted.startup).includes(token), false);
  assert.deepEqual(restarted.dispose(), { requestIds: [], storageUnavailable: false, reconciliation: "complete" });
});

test("ambiguous inspection and multiple active mappings block every fresh start", async t => {
  const { provider, roots, create } = await fixture(t);
  const store = new SqliteCaptureMappingStore(roots.mappingRoot);
  const first = { requestId: "one", captureId: "1".repeat(32), ownerToken: token, bundleId: "bundle-one", bundlePath: join(roots.bundleRoot, "one") };
  const second = { requestId: "two", captureId: "2".repeat(32), ownerToken: token, bundleId: "bundle-two", bundlePath: join(roots.bundleRoot, "two") };
  await store.putIfAbsent(first);
  await store.putIfAbsent(second);
  provider.facts.set(first.captureId, { captureId: first.captureId, state: "recording" });
  provider.facts.set(second.captureId, { captureId: second.captureId, state: "recording" });
  provider.inspectFailures.add(first.captureId);
  const restarted = await create();
  assert.equal(restarted.startup.state, "blocked");
  if (restarted.startup.state === "blocked") {
    assert.deepEqual(restarted.startup.unresolved.map(item => item.state), ["unknown", "active"]);
    assert.equal(restarted.startup.unresolved.some(item => "ownerToken" in item), false);
  }
  assert.equal((await restarted.dispatch({ type: "start", requestId: "three" })).phase, "idle");
  provider.inspectFailures.clear();
  assert.deepEqual((await restarted.reconcileStartup()).state, "blocked", "two active captures remain operator visible");
  assert.equal(restarted.unresolvedOwnership().requestIds.length, 2);
  assert.equal(JSON.stringify(restarted.unresolvedOwnership()).includes(token), false);
  restarted.dispose();
});

test("concurrent reconciliation shares one scan; disposal detaches without stopping or closing", async t => {
  const { provider, roots, create } = await fixture(t);
  provider.setReady(true);
  const store = new SqliteCaptureMappingStore(roots.mappingRoot);
  const mapping = { requestId: "live", captureId: "3".repeat(32), ownerToken: token, bundleId: "bundle-live", bundlePath: join(roots.bundleRoot, "live") };
  await store.putIfAbsent(mapping);
  provider.facts.set(mapping.captureId, { captureId: mapping.captureId, state: "recording" });
  const restarted = await create();
  const pending = provider.sdk.control;
  let release!: () => void;
  let entered!: () => void;
  const enteredPromise = new Promise<void>(resolve => { entered = resolve; });
  const gate = new Promise<void>(resolve => { release = resolve; });
  provider.sdk.control = async (...args) => { entered(); await gate; return pending(...args); };
  // The factory's mandatory initial inspection is complete. A second scan shares
  // its in-flight read, and start cannot pass while that scan is pending.
  const scan = restarted.reconcileStartup();
  const secondScan = restarted.reconcileStartup();
  await enteredPromise;
  assert.equal((await restarted.dispatch({ type: "start", requestId: "race" })).phase, "idle");
  assert.deepEqual(restarted.dispose(), { requestIds: ["live"], storageUnavailable: false, reconciliation: "interrupted" });
  release();
  assert.equal((await scan).state, "blocked");
  assert.equal((await secondScan).state, "blocked");
  assert.equal(provider.facts.size, 1, "disposal does not issue stop/cancel");
});

test("startup reconciliation cannot strand a capture started through this runtime", async t => {
  const { provider, create } = await fixture(t);
  provider.setReady(true);
  const runtime = await create();
  await runtime.dispatch({ type: "start", requestId: "owned during scan" });
  assert.equal((await runtime.reconcileStartup()).state, "blocked");
  assert.equal((await runtime.dispatch({ type: "stop" })).phase, "ready");
  assert.equal((await runtime.reconcileStartup()).state, "ready");
  assert.equal((await runtime.dispatch({ type: "start", requestId: "next capture" })).phase, "recording");
  runtime.dispose();
});

test("live stop remains admitted while a startup inspection is pending", async t => {
  const { provider, create } = await fixture(t);
  provider.setReady(true);
  const runtime = await create();
  await runtime.dispatch({ type: "start", requestId: "pending scan owner" });
  const original = provider.sdk.control;
  let enter!: () => void, release!: () => void;
  const entered = new Promise<void>(resolve => { enter = resolve; });
  const gate = new Promise<void>(resolve => { release = resolve; });
  provider.sdk.control = async (...args) => { if (args[0] === "inspect") { enter(); await gate; } return original(...args); };
  const scan = runtime.reconcileStartup();
  await entered;
  const stop = runtime.dispatch({ type: "stop" });
  assert.equal(runtime.state.phase, "stopping");
  release(); await scan;
  assert.equal((await stop).phase, "ready");
  runtime.dispose();
});

test("a start refused during another command does not consume its request ID", async t => {
  const { provider, create } = await fixture(t);
  const runtime = await create();
  let release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  const original = provider.sdk.handshake;
  provider.sdk.handshake = async () => { await gate; return original(); };
  provider.setReady(true);
  const first = runtime.dispatch({ type: "start", requestId: "first command" });
  await runtime.dispatch({ type: "start", requestId: "retry after busy" });
  assert.equal(runtime.startup.knownRequestIds.includes("retry after busy"), false);
  release(); await first;
  await runtime.dispatch({ type: "stop" });
  assert.equal((await runtime.dispatch({ type: "start", requestId: "retry after busy" })).phase, "recording");
  assert.equal(runtime.startup.knownRequestIds.includes("retry after busy"), true);
  runtime.dispose();
});

test("a scan cannot erase a start reservation before its mapping is persisted", async t => {
  const { provider, create } = await fixture(t);
  const runtime = await create();
  let release!: () => void;
  const gate = new Promise<void>(resolve => { release = resolve; });
  const original = provider.sdk.handshake;
  provider.sdk.handshake = async () => { await gate; return original(); };
  provider.setReady(true);
  const pending = runtime.dispatch({ type: "start", requestId: "reserved before mapping" });
  await runtime.reconcileStartup();
  assert.equal(runtime.startup.knownRequestIds.includes("reserved before mapping"), true);
  release(); await pending;
  await runtime.dispatch({ type: "stop" });
  await runtime.dispatch({ type: "start", requestId: "reserved before mapping" });
  assert.equal(provider.facts.size, 1);
  assert.equal(runtime.state.phase, "ready");
  runtime.dispose();
});

test("disposing a scan preserves newly discovered identities and reports incomplete reconciliation", async t => {
  const { provider, roots, create } = await fixture(t);
  const runtime = await create();
  const mapping = { requestId: "discovered after boot", captureId: "4".repeat(32), ownerToken: token, bundleId: "newly-discovered", bundlePath: join(roots.bundleRoot, "newly-discovered") };
  await new SqliteCaptureMappingStore(roots.mappingRoot).putIfAbsent(mapping);
  provider.facts.set(mapping.captureId, { captureId: mapping.captureId, state: "recording" });
  let enter!: () => void, release!: () => void;
  const entered = new Promise<void>(resolve => { enter = resolve; });
  const gate = new Promise<void>(resolve => { release = resolve; });
  const control = provider.sdk.control;
  provider.sdk.control = async (...args) => { enter(); await gate; return control(...args); };
  const scan = runtime.reconcileStartup();
  await entered;
  assert.deepEqual(runtime.unresolvedOwnership(), { requestIds: [mapping.requestId], storageUnavailable: false, reconciliation: "pending" });
  assert.deepEqual(runtime.dispose(), { requestIds: [mapping.requestId], storageUnavailable: false, reconciliation: "interrupted" });
  release();
  assert.equal((await scan).state, "blocked");
  assert.equal(runtime.unresolvedOwnership().reconciliation, "interrupted");
});

test("failed storage enumeration retains known identities until a successful rescan", async t => {
  const { provider, roots, create } = await fixture(t);
  provider.setReady(true);
  const runtime = await create();
  await runtime.dispatch({ type: "start", requestId: "known before storage failure" });
  await runtime.dispatch({ type: "stop" });
  await chmod(roots.mappingRoot, 0o755);
  try {
    assert.equal((await runtime.reconcileStartup()).state, "blocked");
    assert.deepEqual(runtime.unresolvedOwnership(), { requestIds: ["known before storage failure"], storageUnavailable: true, reconciliation: "failed" });
    assert.equal((await runtime.dispatch({ type: "start", requestId: "must not start" })).phase, "ready");
  } finally { await chmod(roots.mappingRoot, 0o700); }
  assert.equal((await runtime.reconcileStartup()).state, "ready");
  assert.deepEqual(runtime.dispose(), { requestIds: [], storageUnavailable: false, reconciliation: "complete" });
});
