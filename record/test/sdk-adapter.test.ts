import assert from "node:assert/strict";
import { pathToFileURL } from "node:url";
import { test } from "node:test";
import { EngineSdkAdapter, type CaptureMapping, type CaptureMappingStore, type EngineSdkPort, type PreparedStart, type SdkReceipt } from "../src/backend/engine/sdk-adapter.ts";

class MemoryMappings implements CaptureMappingStore {
  readonly byRequest = new Map<string, CaptureMapping>();
  readonly byCapture = new Map<string, CaptureMapping>();
  async getByRequestId(id: string) { return this.byRequest.get(id); }
  async getByCaptureId(id: string) { return this.byCapture.get(id); }
  async putIfAbsent(mapping: CaptureMapping) {
    const existing = this.byRequest.get(mapping.requestId);
    if (existing) {
      assert.deepEqual(
        { captureId: existing.captureId, ownerToken: existing.ownerToken, bundleId: existing.bundleId, bundlePath: existing.bundlePath },
        { captureId: mapping.captureId, ownerToken: mapping.ownerToken, bundleId: mapping.bundleId, bundlePath: mapping.bundlePath },
      );
      return existing;
    }
    this.byRequest.set(mapping.requestId, mapping);
    this.byCapture.set(mapping.captureId, mapping);
    return mapping;
  }
}

const bundle = (requestId: string) => ({ bundleId: `bundle-${requestId.replace(/[^a-z0-9-]/giu, "-")}`, bundlePath: `/private/recordings/${requestId}.apparent` });
const options = (mappings = new MemoryMappings()) => ({ mappings, allocateBundle: async (id: string) => bundle(id), pollIntervalMs: 60 });

async function packedSdk() {
  const root = process.env.APPARENT_ENGINE_SDK_CONSUMER;
  assert.ok(root, "set APPARENT_ENGINE_SDK_CONSUMER to a clean consumer with packed SDK installed");
  const clientPath = `${root}/node_modules/@apparent/engine-sdk/dist/src/index.js`;
  const testingPath = `${root}/node_modules/@apparent/engine-sdk/dist/src/testing.js`;
  return { ...(await import(pathToFileURL(clientPath).href)), ...(await import(pathToFileURL(testingPath).href)) };
}

test("packed ReferenceEngine cannot establish readiness; default SDK rejects it", async t => {
  if (!process.env.APPARENT_ENGINE_SDK_CONSUMER) return t.skip("packed SDK consumer not configured");
  const { EngineClient, ReferenceEngine } = await packedSdk();
  const reference = new ReferenceEngine();
  const defaultClient = new EngineClient(reference);
  const adapter = new EngineSdkAdapter(defaultClient, options());
  assert.deepEqual(await adapter.readiness(), { ready: false, reason: "Native Engine capture is unavailable" });
  await assert.rejects(defaultClient.handshake(), /explicit test-only opt-in/);
  adapter.dispose();
  await reference.close();
});

test("packed SDK mapping survives timeout and client reconnect with fresh arbitrary Record IDs", async t => {
  if (!process.env.APPARENT_ENGINE_SDK_CONSUMER) return t.skip("packed SDK consumer not configured");
  const { EngineClient, ReferenceEngine } = await packedSdk();
  const reference = new ReferenceEngine();
  const rpcIds: string[] = [];
  const request = reference.request.bind(reference);
  reference.request = async value => { rpcIds.push(value.rpcId); return request(value); };
  const mappings = new MemoryMappings();
  const firstClient = new EngineClient(reference, { allowTestDouble: true });
  const first = new EngineSdkAdapter(firstClient, options(mappings));
  const beforeDispatch = firstClient.start.bind(firstClient);
  let failOnce = true;
  firstClient.start = async request => {
    const result = await beforeDispatch(request);
    if (failOnce) { failOnce = false; throw new Error("private path and owner token must not escape"); }
    return result;
  };
  await assert.rejects(first.start("request / one"), /outcome is unknown/);
  const savedIdentity = mappings.byRequest.get("request / one");
  assert.ok(savedIdentity);
  const receipt = await first.start("request / one");
  assert.equal(receipt.captureId, savedIdentity.captureId);
  assert.equal("ownerToken" in receipt, false);
  assert.equal(JSON.stringify(receipt).includes(savedIdentity.ownerToken), false);
  assert.equal(JSON.stringify(receipt).includes(savedIdentity.bundlePath), false);

  const reconnect = new EngineSdkAdapter(new EngineClient(reference, { allowTestDouble: true }), options(mappings));
  const replay = await reconnect.start("request / one");
  assert.equal(replay.captureId, receipt.captureId);
  assert.equal(mappings.byRequest.size, 1);
  assert.equal(await reconnect.readiness().then(value => value.ready), false);
  assert.equal((await reconnect.cancel(replay.captureId)).phase, "cancelled");
  assert.deepEqual(new Set(rpcIds).size, rpcIds.length);
  first.dispose(); reconnect.dispose();
  await reference.close();
});

test("packed SDK observed finalizing maps to processing and synthetic saved is never ready", async t => {
  if (!process.env.APPARENT_ENGINE_SDK_CONSUMER) return t.skip("packed SDK consumer not configured");
  const { EngineClient, ReferenceEngine } = await packedSdk();
  const reference = new ReferenceEngine();
  const client = new EngineClient(reference, { allowTestDouble: true });
  const mappings = new MemoryMappings();
  const adapter = new EngineSdkAdapter(client, options(mappings));
  const events: string[] = [];
  adapter.subscribe(event => { if (event.type === "capture-changed") events.push(event.receipt.phase); });
  const start = await adapter.start("finalize-test");
  const mapping = mappings.byCapture.get(start.captureId)!;
  const prepared = client.prepareStart(mapping.bundleId, mapping.bundlePath);
  await client.start({ ...prepared, captureId: mapping.captureId, ownerToken: mapping.ownerToken });
  reference.setScenarioState(start.captureId, "recording");
  await client.control("stop", start.captureId, mapping.ownerToken);
  reference.setScenarioState(start.captureId, "finalizing");
  assert.equal((await adapter.inspect(start.captureId)).phase, "processing");
  reference.setSavedFixture(start.captureId);
  await assert.rejects(adapter.inspect(start.captureId), /outcome is unknown/);
  assert.ok(events.includes("starting"));
  assert.ok(events.includes("processing"));
  assert.equal(events.includes("saved"), false);
  adapter.dispose();
  await reference.close();
});

class NativeFacts implements EngineSdkPort {
  readonly clientId = "1234567890abcdef1234567890abcdef";
  readonly calls: string[] = [];
  state: SdkReceipt["state"] = "starting";
  async handshake() { return { implementation: "native" as const, readiness: "ready" as const, capabilities: { screenCapture: true } }; }
  prepareStart(bundleId: string, bundlePath: string): PreparedStart {
    return { protocolVersion: 1, clientId: this.clientId, rpcId: "0".repeat(32), method: "start", captureId: "a".repeat(32), ownerToken: "b".repeat(64), bundleId, bundlePath, options: { screen: true } };
  }
  async start(request: PreparedStart) { this.calls.push(`start:${request.captureId}`); return { receipt: { captureId: request.captureId, state: this.state }, captureId: request.captureId, ownerToken: "b".repeat(64) }; }
  async control(method: "inspect" | "stop" | "cancel", captureId: string): Promise<SdkReceipt> {
    this.calls.push(`${method}:${captureId}`);
    if (method === "cancel") this.state = "cancelled";
    return { captureId, state: this.state, ...(this.state === "saved" ? { bundlePath: bundle("native").bundlePath, artifactProvenance: "native" as const } : {}) };
  }
  async close() { this.calls.push("close"); return "already-exited" as const; }
}

test("native facts preserve processing and saved provenance; stale phases and disposal are safe", async () => {
  const sdk = new NativeFacts();
  const mappings = new MemoryMappings();
  const adapter = new EngineSdkAdapter(sdk, options(mappings));
  const events: string[] = [];
  const detach = adapter.subscribe(event => { if (event.type === "capture-changed") events.push(event.receipt.phase); });
  const started = await adapter.start("native");
  assert.equal(started.phase, "starting");
  sdk.state = "finalizing";
  const processing = await adapter.inspect(started.captureId);
  assert.equal(processing.phase, "processing");
  sdk.state = "recording"; // stale observation cannot regress accepted state.
  assert.equal((await adapter.inspect(started.captureId)).phase, "processing");
  sdk.state = "saved";
  const ready = await adapter.inspect(started.captureId);
  assert.equal(ready.phase, "saved");
  assert.equal(ready.bundlePath, bundle("native").bundlePath);
  assert.deepEqual(events, ["starting", "processing", "saved"]);
  detach(); adapter.dispose();
  assert.equal(sdk.calls.includes("close"), false);
});

test("synthetic saved receipts are rejected by the adapter", async () => {
  const synthetic = new NativeFacts();
  const control = synthetic.control.bind(synthetic);
  synthetic.control = async (method, captureId) => method === "inspect"
    ? { captureId, state: "saved", bundlePath: bundle("native").bundlePath, artifactProvenance: "test-synthetic" }
    : control(method, captureId);
  const second = new EngineSdkAdapter(synthetic, options());
  await second.start("native");
  await assert.rejects(second.inspect("a".repeat(32)), /outcome is unknown/);
  second.dispose();
});

test("double starts reuse one durable identity and same-capture mutations serialize", async () => {
  const sdk = new NativeFacts();
  const mappings = new MemoryMappings();
  let allocations = 0;
  const adapter = new EngineSdkAdapter(sdk, { ...options(mappings), allocateBundle: async id => { allocations++; return bundle(id); } });
  const [first, replay] = await Promise.all([adapter.start("arbitrary request id"), adapter.start("arbitrary request id")]);
  assert.equal(first.captureId, replay.captureId);
  assert.equal(allocations, 1);
  assert.deepEqual(sdk.calls.slice(0, 2), [`start:${first.captureId}`, `start:${first.captureId}`]);

  let inFlight = 0;
  let maxInFlight = 0;
  const control = sdk.control.bind(sdk);
  sdk.control = async (method, captureId) => {
    inFlight++;
    maxInFlight = Math.max(maxInFlight, inFlight);
    await new Promise(resolve => setTimeout(resolve, 10));
    try { return await control(method, captureId); }
    finally { inFlight--; }
  };
  await Promise.all([adapter.inspect(first.captureId), adapter.cancel(first.captureId)]);
  assert.equal(maxInFlight, 1);
  adapter.dispose();
});

test("disposal during a pending poll detaches replies and never closes the SDK", async () => {
  const sdk = new NativeFacts();
  const adapter = new EngineSdkAdapter(sdk, options());
  const started = await adapter.start("dispose-poll");
  let entered!: () => void;
  let release!: () => void;
  const pollEntered = new Promise<void>(resolve => { entered = resolve; });
  const blocked = new Promise<void>(resolve => { release = resolve; });
  const control = sdk.control.bind(sdk);
  let holdPoll = true;
  sdk.control = async (method, captureId) => {
    if (method === "inspect" && holdPoll) {
      holdPoll = false;
      entered();
      await blocked;
    }
    return control(method, captureId);
  };
  const events: string[] = [];
  adapter.subscribe(event => { if (event.type === "capture-changed") events.push(event.receipt.phase); });
  await pollEntered;
  adapter.dispose();
  release();
  await new Promise(resolve => setTimeout(resolve, 80));
  assert.deepEqual(events, []);
  assert.equal(sdk.calls.includes("close"), false);
  assert.equal(sdk.calls.some(call => call === `cancel:${started.captureId}`), false);
});
