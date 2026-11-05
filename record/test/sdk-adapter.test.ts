import assert from "node:assert/strict";
import { pathToFileURL } from "node:url";
import { test } from "node:test";
import { EngineSdkAdapter, type CaptureMapping, type CaptureMappingStore, type EngineSdkPort, type PreparedStart, type SdkReceipt } from "../src/backend/engine/sdk-adapter.ts";

class MemoryMappings implements CaptureMappingStore {
  readonly byRequest = new Map<string, CaptureMapping>();
  readonly byCapture = new Map<string, CaptureMapping>();
  async listRequestIds() { return [...this.byRequest.keys()].sort(); }
  async getByRequestId(id: string) { return this.byRequest.get(id); }
  async getByCaptureId(id: string) { return this.byCapture.get(id); }
  async putIfAbsent(mapping: CaptureMapping) {
    const existing = this.byRequest.get(mapping.requestId);
    if (existing) {
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
  reference.request = async (value: {rpcId: string}) => { rpcIds.push(value.rpcId); return request(value); };
  const mappings = new MemoryMappings();
  const firstClient = new EngineClient(reference, { allowTestDouble: true });
  const first = new EngineSdkAdapter(firstClient, options(mappings));
  const beforeDispatch = firstClient.start.bind(firstClient);
  let failOnce = true;
  firstClient.start = async (request: PreparedStart) => {
    const result = await beforeDispatch(request);
    if (failOnce) { failOnce = false; throw new Error("private path and owner token must not escape"); }
    return result;
  };
  await assert.rejects(first.start("request / one"), /outcome is unknown/);
  const savedIdentity = mappings.byRequest.get("request / one");
  assert.ok(savedIdentity);
  const receipt = await first.start("request / one");
  assert.equal(receipt.captureId, "request / one");
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
  const mapping = mappings.byRequest.get("finalize-test")!;
  const prepared = client.prepareStart(mapping.bundleId, mapping.bundlePath);
  await client.start({ ...prepared, captureId: mapping.captureId, ownerToken: mapping.ownerToken });
  reference.setScenarioState(mapping.captureId, "recording");
  await client.control("stop", mapping.captureId, mapping.ownerToken);
  reference.setScenarioState(mapping.captureId, "finalizing");
  assert.equal((await adapter.inspect(start.captureId)).phase, "processing");
  reference.setSavedFixture(mapping.captureId);
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
  await assert.rejects(second.inspect("native"), /outcome is unknown/);
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
  assert.deepEqual(sdk.calls.slice(0, 2), [`start:${"a".repeat(32)}`, `start:${"a".repeat(32)}`]);

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

for (const requestId of ['caller-owned request', 'c'.repeat(32)]) {
  test(`real controller and adapter preserve the public identity (${requestId})`, async () => {
    const {RecordWorkflow} = await import('../src/backend/workflow/controller.ts');
    const sdk=new NativeFacts();
    const adapter=new EngineSdkAdapter(sdk,options());
    const flow=new RecordWorkflow(adapter);
    try {
      await flow.dispatch({type:'start',requestId});
      assert.equal(flow.state.phase,'starting');
      assert.equal('captureId' in flow.state && flow.state.captureId,requestId);
      sdk.state='recording';
      await adapter.inspect(requestId);
      assert.equal(flow.state.phase,'recording');
      await flow.dispatch({type:'cancel'});
      assert.equal(flow.state.phase,'cancelled');
      assert.ok(sdk.calls.includes(`cancel:${'a'.repeat(32)}`));
    } finally {flow.dispose();adapter.dispose();}
  });
}

test('an observer cannot mutate returned or other observers Engine facts', async () => {
  const sdk=new NativeFacts();
  const adapter=new EngineSdkAdapter(sdk,options());
  const seen: string[]=[];
  adapter.subscribe(event=>{if(event.type==='capture-changed'){event.receipt.phase='saved';event.receipt.captureId='wrong';}});
  adapter.subscribe(event=>{if(event.type==='capture-changed')seen.push(`${event.receipt.captureId}:${event.receipt.phase}`);});
  try {
    const result=await adapter.start('owned');
    assert.deepEqual(result,{captureId:'owned',phase:'starting'});
    assert.deepEqual(seen,['owned:starting']);
  } finally {adapter.dispose();}
});

test('ambiguous start is recovered by polling with original public identity', async t => {
  const {RecordWorkflow} = await import('../src/backend/workflow/controller.ts');
  const sdk=new NativeFacts();
  const start=sdk.start.bind(sdk);
  sdk.start=async prepared=>{await start(prepared);throw new Error('lost start reply');};
  const adapter=new EngineSdkAdapter(sdk,options());
  const flow=new RecordWorkflow(adapter);
  t.after(()=>{flow.dispose();adapter.dispose();});
  await flow.dispatch({type:'start',requestId:'unknown-start'});
  assert.equal(flow.state.phase,'uncertain');
  sdk.state='recording';
  await new Promise<void>((resolve,reject)=>{
    const timeout=setTimeout(()=>reject(new Error('no authoritative recovery observation')),1500);
    const detach=flow.subscribe(state=>{if(state.phase==='recording'){clearTimeout(timeout);detach();resolve();}});
    t.after(()=>{clearTimeout(timeout);detach();});
  });
  assert.equal('captureId' in flow.state && flow.state.captureId,'unknown-start');
  await flow.dispatch({type:'cancel'});
  assert.equal(flow.state.phase,'cancelled');
});

test('explicit controller reconciliation finds the original private mapping after uncertain start', async () => {
  const {RecordWorkflow}=await import('../src/backend/workflow/controller.ts');
  const sdk=new NativeFacts();
  const original=sdk.start.bind(sdk);
  sdk.start=async prepared=>{await original(prepared);throw new Error('timeout');};
  const adapter=new EngineSdkAdapter(sdk,{...options(),pollIntervalMs:60000});
  const flow=new RecordWorkflow(adapter);
  try {
    const started=await flow.dispatch({type:'start',requestId:'recover-me'});
    assert.equal(started.phase,'uncertain');
    sdk.state='recording';
    const reconciled=await flow.dispatch({type:'reconcile'});
    assert.equal(reconciled.phase,'recording');
    const cancelled=await flow.dispatch({type:'cancel'});
    assert.equal(cancelled.phase,'cancelled');
  } finally {flow.dispose();adapter.dispose();}
});

test('late readiness cannot report usable capture after adapter disposal', async () => {
  const sdk=new NativeFacts();
  let release!: (value: Awaited<ReturnType<NativeFacts['handshake']>>) => void;
  sdk.handshake=()=>new Promise(resolve=>{release=resolve;});
  const adapter=new EngineSdkAdapter(sdk,options());
  const pending=adapter.readiness();
  adapter.dispose();
  release({implementation:'native',readiness:'ready',capabilities:{screenCapture:true}});
  assert.equal((await pending).ready,false);
});
