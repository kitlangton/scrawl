import assert from "node:assert/strict";
import { mkdtemp, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { CaptureClient, type CaptureRequest, type CaptureResponse, type CaptureTransport, type SourceDescriptor } from "@apparent/engine-sdk/capture";
import { createRecordCapabilityProvider } from "../../dist/backend/capabilities/provider.js";
import { createCapture2Host } from "../../dist/backend/host/capture2.js";
import { createCapture2RecordRuntime } from "../../dist/backend/host/capture2/runtime.js";
import { AbsentDestinationAllocator, SqliteCaptureMappingStore } from "../../dist/backend/host/index.js";

const source: SourceDescriptor = { id: "a".repeat(32), generation: "b".repeat(32), kind: "display", displayId: 1, bounds: { x: 0, y: 0, width: 1280, height: 720, scale: 1 } };
const invocation = { invocationId: "c".repeat(32), observedAtMs: 1000, kind: "explicit" as const };

/** Synthetic published-SDK transport; no OS or native capture claim. */
class Transport implements CaptureTransport {
  requests: CaptureRequest[] = [];
  status = new Map<string, object>();
  beforeDiscovery?: () => Promise<void>;
  loseStartReply = false;
  async request(request: CaptureRequest): Promise<CaptureResponse> {
    this.requests.push(structuredClone(request));
    if (request.method === "handshake") return { channel: "capture", protocolVersion: 2, rpcId: request.rpcId, result: { channel: "capture", protocolVersion: 2, implementation: "native", readiness: "ready", capabilities: { sourceDiscovery: true, displayCapture: true, areaCapture: false, audioCapture: false, contextFacts: false, interactionFacts: false, candidates: false, transcript: false, strokes: false, publication: false, retainedMaterials: true } } };
    if (request.method === "discoverSources") { await this.beforeDiscovery?.(); return { channel: "capture", protocolVersion: 2, rpcId: request.rpcId, result: { state: "available", sources: [source] } }; }
    if (request.method === "start") { const status = { captureId: request.captureId, state: "recording" }; this.status.set(request.captureId, status); if (this.loseStartReply) { this.loseStartReply = false; throw new Error("lost start reply"); } return { channel: "capture", protocolVersion: 2, rpcId: request.rpcId, result: status }; }
    if (request.method === "stop") { const status = { captureId: request.captureId, state: "materials-ready", materials: { schema: "apparent.capture-materials/1", key: { captureId: request.captureId, materialsId: "f".repeat(32), revision: "e".repeat(64) }, root: "/synthetic/materials", leaseId: "1".repeat(32), payloadFile: { relativePath: "materials.json", identity: { device: "1", inode: "2", size: 1, modifiedNs: "3" }, sha256: "e".repeat(64) } } }; this.status.set(request.captureId, status); return { channel: "capture", protocolVersion: 2, rpcId: request.rpcId, result: status }; }
    if (request.method === "inspect" || request.method === "cancel") { const status = this.status.get(request.captureId); if (!status) return { channel: "capture", protocolVersion: 2, rpcId: request.rpcId, error: { code: "not_found", message: "unknown" } }; return { channel: "capture", protocolVersion: 2, rpcId: request.rpcId, result: status }; }
    return { channel: "capture", protocolVersion: 2, rpcId: request.rpcId, error: { code: "unavailable", message: "unsupported" } };
  }
  async close() { return "already-exited" as const; }
}

async function setup(t: { after(fn: () => void | Promise<void>): void }) {
  const root = await realpath(await mkdtemp(join(tmpdir(), "record-capture2-runtime-")));
  t.after(() => rm(root, { recursive: true, force: true }));
  const transport = new Transport();
  const sdk = new CaptureClient(transport);
  const mappings = new SqliteCaptureMappingStore(join(root, "control"));
  const destinations = new AbsentDestinationAllocator(join(root, "control"), join(root, "bundles"));
  const host = createCapture2Host({ client: sdk, mappings, destinations });
  const runtime = await createCapture2RecordRuntime({ host, createStartIntent: (requestId, discovery) => ({ source: discovery.sources[0]!, options: { audio: "none", localOnly: true }, invocation, destinationPath: join(root, `${requestId.replace(/[^a-z0-9-]/giu, "-")}.apparent`) }), pollIntervalMs: 100 });
  let outputRevision = 1;
  let outputCaptures: { captureId: string; delivery: "unknown"; artifactId: string; phase: "saved"; outcomes: { kind: "save"; at: string; ok: true }[] }[] = [];
  const client = createRecordCapabilityProvider({ runtime, capture2Start: runtime, providerEpoch: "provider-capture2", native: { readFacts: async () => ({ version: 1, providerEpoch: "native", revision: 1, readiness: [], shortcut: { effective: null, label: null, registration: "unavailable", externallyManaged: false, draft: null } }) }, captureFacts: { readFacts: async () => ({ version: 1, providerEpoch: "capture-output", revision: outputRevision, captures: outputCaptures, nextShortcutAdmission: { state: "idle" } }) } });
  await new Promise(resolve => setImmediate(resolve));
  return { root, transport, sdk, mappings, destinations, host, runtime, client, setOutput(captures: typeof outputCaptures) { outputCaptures = captures; outputRevision++; } };
}
const expected = (client: ReturnType<typeof createRecordCapabilityProvider>) => ({ providerEpoch: client.current().providerEpoch, revision: client.current().revision });

test("SDK invocation preflight refusal leaves no capture and permits a corrected start", async t => {
  const ctx = await setup(t);
  ctx.client.detach(); ctx.runtime.dispose();
  let invalid = true;
  const runtime = await createCapture2RecordRuntime({ host: ctx.host,
    createStartIntent: (requestId, discovery) => ({ source: discovery.sources[0]!,
      options: { audio: "none", localOnly: true },
      invocation: { ...invocation, invocationId: invalid ? requestId : invocation.invocationId },
      destinationPath: join(ctx.root, "corrected.apparent") }) });
  const client = createRecordCapabilityProvider({ runtime, capture2Start: runtime, providerEpoch: "preflight-provider",
    native: { readFacts: async () => ({ version: 1, providerEpoch: "native", revision: 1, readiness: [],
      shortcut: { effective: null, label: null, registration: "unavailable", externallyManaged: false, draft: null } }) } });
  t.after(() => { client.detach(); runtime.dispose(); ctx.host.dispose(); });
  await new Promise(resolve => setImmediate(resolve));
  const requestId = "12345678-1234-4123-8123-123456789abc";
  const rejected = await client.execute({ type: "start-capture", delivery: "host-default", requestId, expected: expected(client) });
  assert.equal(rejected.status, "refused");
  assert.deepEqual(await ctx.mappings.listRequestIds("capture2"), []);
  assert.equal(ctx.transport.requests.some(request => request.method === "start"), false);
  assert.equal(runtime.startup.state, "ready");
  assert.deepEqual(runtime.unresolvedOwnership().requestIds, []);
  assert.equal(runtime.state.phase, "idle");
  assert.deepEqual(rejected.snapshot.captures, [], "a validation refusal is not an unknown capture");
  invalid = false;
  const accepted = await client.execute({ type: "start-capture", delivery: "host-default", requestId: "corrected-request", expected: expected(client) });
  assert.equal(accepted.status, "accepted");
  assert.equal(ctx.transport.requests.filter(request => request.method === "start").length, 1);
  assert.deepEqual(await ctx.mappings.listRequestIds("capture2"), ["corrected-request"]);
});

test("provider composes capture2 start, stop and materials without changing public request identity", async t => {
  const ctx = await setup(t);
  const local = await ctx.client.execute({ type: "start-capture", delivery: "local-only", requestId: "local-refused", expected: expected(ctx.client) });
  assert.equal(local.status, "refused");
  assert.equal(ctx.transport.requests.some(request => request.method === "start"), false);
  const started = await ctx.client.execute({ type: "start-capture", delivery: "host-default", requestId: "public/capture id", expected: expected(ctx.client) });
  assert.equal(started.status, "accepted");
  assert.equal(started.snapshot.captures[0]?.captureId, "public/capture id");
  assert.equal(started.snapshot.captures[0]?.delivery, "unknown");
  const nativeStart = ctx.transport.requests.find((request): request is Extract<CaptureRequest, { method: "start" }> => request.method === "start");
  const mapping = await ctx.mappings.getByRequestId("public/capture id");
  assert.ok(mapping);
  assert.equal(nativeStart?.captureId, mapping.captureId);
  assert.notEqual(nativeStart?.captureId, "public/capture id");
  assert.equal(nativeStart?.bundlePath, join(ctx.root, "public-capture-id.apparent"));
  const stopped = await ctx.client.execute({ type: "stop-capture", captureId: "public/capture id", requestId: "stop", expected: expected(ctx.client) });
  assert.equal(stopped.status, "accepted");
  assert.equal(stopped.snapshot.captures[0]?.phase, "processing");
  assert.equal(stopped.snapshot.captures[0]?.outcomes.some(outcome => outcome.kind === "save" && outcome.ok), false);
  ctx.setOutput([{ captureId: "public/capture id", delivery: "unknown", artifactId: "assembled-artifact", phase: "saved", outcomes: [{ kind: "save", at: "2026-11-06T00:00:00Z", ok: true }] }]);
  const refreshed = await ctx.client.execute({ type: "refresh", requestId: "refresh-output", expected: expected(ctx.client) });
  assert.equal(refreshed.snapshot.captures.find(capture => capture.captureId === "public/capture id")?.phase, "saved", "authenticated output completes the stopped materials-ready capture");
  assert.equal((await ctx.host.inspect("public/capture id")).materials?.leaseId, "1".repeat(32));
  ctx.setOutput([{ captureId: "capture-two", delivery: "unknown", artifactId: "cached-artifact", phase: "saved", outcomes: [{ kind: "save", at: "2026-11-06T00:00:00Z", ok: true }] }]);
  await ctx.client.execute({ type: "refresh", requestId: "refresh-cached", expected: expected(ctx.client) });
  const second = await ctx.client.execute({ type: "start-capture", delivery: "host-default", requestId: "capture-two", expected: expected(ctx.client) });
  assert.equal(second.status, "accepted");
  assert.deepEqual(second.snapshot.captures.map(capture => [capture.captureId, capture.phase]).sort(([left], [right]) => String(left).localeCompare(String(right))), [["capture-two", "recording"], ["public/capture id", "saved"]]);
  await ctx.host.cancel("capture-two");
  ctx.client.detach(); ctx.runtime.dispose();
  assert.equal(await ctx.host.inspect("public/capture id").then(value => value.requestId), "public/capture id", "runtime detach leaves caller-owned host available");
  ctx.host.dispose();
});

test("fresh source selection is bound to discovery and startup reconciliation blocks only unresolved native work", async t => {
  const ctx = await setup(t);
  ctx.runtime.dispose(); ctx.client.detach(); ctx.host.dispose();
  const firstHost = createCapture2Host({ client: ctx.sdk, mappings: ctx.mappings, destinations: ctx.destinations });
  const firstRuntime = await createCapture2RecordRuntime({ host: firstHost, createStartIntent: () => ({ source: { ...source, generation: "d".repeat(32) }, options: { audio: "none", localOnly: true }, invocation, destinationPath: join(ctx.root, "stale.apparent") }) });
  assert.equal((await firstRuntime.startCapture("stale-source")).outcome, "refused");
  assert.equal(ctx.transport.requests.some(request => request.method === "start"), false);
  const validRuntime = await createCapture2RecordRuntime({ host: firstHost, createStartIntent: (_id, discovery) => ({ source: discovery.sources[0]!, options: { audio: "none", localOnly: true }, invocation, destinationPath: join(ctx.root, "active.apparent") }) });
  assert.equal((await validRuntime.startCapture("active-before-restart")).outcome, "accepted");
  validRuntime.dispose(); firstHost.dispose();
  const restartedHost = createCapture2Host({ client: ctx.sdk, mappings: ctx.mappings, destinations: ctx.destinations });
  const restarted = await createCapture2RecordRuntime({ host: restartedHost, createStartIntent: (_id, discovery) => ({ source: discovery.sources[0]!, options: { audio: "none", localOnly: true }, invocation, destinationPath: join(ctx.root, "must-wait.apparent") }) });
  assert.equal(restarted.startup.state, "blocked");
  assert.equal((await restarted.startCapture("must-wait")).outcome, "refused");
  const state = restarted.state;
  assert.equal("captureId" in state ? state.captureId : undefined, "active-before-restart");
  assert.equal((await restarted.dispatch({ type: "stop" })).phase, "processing", "owned controls remain available while startup is blocked");
  assert.equal(restarted.canStartCapture?.(), true, "materials-ready is processing, but no longer native-active");
  restarted.dispose(); restartedHost.dispose();
});

test("overlapping starts are refused while the first explicit source selection is pending", async t => {
  const ctx = await setup(t);
  let release!: () => void;
  ctx.transport.beforeDiscovery = () => new Promise<void>(resolve => { release = resolve; });
  const first = ctx.runtime.startCapture("race-one");
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(typeof release, "function");
  const second = await ctx.runtime.startCapture("race-two");
  assert.equal(second.outcome, "refused");
  release();
  assert.equal((await first).outcome, "accepted");
  assert.equal(ctx.transport.requests.filter(request => request.method === "start").length, 1);
  await ctx.host.cancel("race-one");
  ctx.client.detach(); ctx.runtime.dispose(); ctx.host.dispose();
});

test("lost start reply reserves public ownership, blocks admission and recovers through SDK inspection", async t => {
  const ctx = await setup(t);
  ctx.transport.loseStartReply = true;
  const started = await ctx.client.execute({ type: "start-capture", delivery: "host-default", requestId: "lost-reply", expected: expected(ctx.client) });
  assert.equal(started.status, "outcome-unknown");
  assert.equal(ctx.runtime.canStartCapture?.(), false);
  assert.deepEqual(ctx.runtime.unresolvedOwnership().requestIds, ["lost-reply"]);
  assert.equal(ctx.runtime.startup.state, "blocked");
  assert.equal((await ctx.runtime.startCapture("another-id")).outcome, "refused");
  await new Promise(resolve => setTimeout(resolve, 140));
  assert.equal(ctx.runtime.state.phase, "recording");
  assert.equal(ctx.runtime.state.captureId, "lost-reply");
  assert.equal(ctx.runtime.canStartCapture?.(), false);
  await ctx.host.cancel("lost-reply");
  ctx.client.detach(); ctx.runtime.dispose(); ctx.host.dispose();
});

for (const changeDuring of ['discovery', 'start'] as const) test(`readiness changes during ${changeDuring} survive definitely-undispatched refusal`, async () => {
  let emit!: (event: import('../../src/backend/host/capture2.ts').Capture2Event) => void;
  const host = {
    subscribe(listener: typeof emit) { emit = listener; return () => undefined; },
    readiness: async () => ({ ready: true }), reconcile: async () => [],
    discoverSources: async () => {
      if (changeDuring === 'discovery') emit({ type: 'readiness-changed', readiness: { ready: false, reason: 'synthetic readiness changed' } });
      return { state: 'available' as const, sources: [source] };
    },
    start: async () => {
      if (changeDuring === 'start') emit({ type: 'readiness-changed', readiness: { ready: false, reason: 'synthetic readiness changed' } });
      throw Object.assign(new Error('synthetic refusal before dispatch'), { dispatched: false });
    },
    inspect: async () => { throw new Error('no capture was dispatched'); },
    stop: async () => { throw new Error('no capture was dispatched'); },
    cancel: async () => { throw new Error('no capture was dispatched'); }, dispose() {},
  };
  const runtime = await createCapture2RecordRuntime({ host, createStartIntent: () => ({ source, options: { audio: 'none', localOnly: true }, invocation, destinationPath: '/synthetic/not-created.apparent' }) });
  try {
    assert.equal((await runtime.startCapture('not-dispatched')).outcome, 'refused');
    assert.deepEqual(runtime.unresolvedOwnership().requestIds, []);
    assert.equal(runtime.startup.state, 'ready');
    assert.equal(runtime.state.phase, 'idle');
    assert.deepEqual(runtime.state.readiness, { ready: false, reason: 'synthetic readiness changed' });
    assert.equal((await runtime.startCapture('still-unavailable')).outcome, 'refused');
  } finally { runtime.dispose(); }
});

test("a delayed stop failure cannot overwrite a newer stopped-materials event", async () => {
  let emit!: (event: import('../../src/backend/host/capture2.ts').Capture2Event) => void;
  const host = {
    subscribe(listener: typeof emit) { emit = listener; return () => undefined; },
    readiness: async () => ({ ready: true }), reconcile: async () => [],
    discoverSources: async () => ({ state: 'available' as const, sources: [source] }),
    start: async () => ({ requestId: 'ordered', phase: 'recording' as const }),
    inspect: async () => ({ requestId: 'ordered', phase: 'materials-ready' as const }),
    stop: async () => { emit({ type: 'capture-changed', receipt: { requestId: 'ordered', phase: 'materials-ready' } }); throw new Error('synthetic delayed failure'); },
    cancel: async () => ({ requestId: 'ordered', phase: 'cancelled' as const }), dispose() {},
  };
  const runtime = await createCapture2RecordRuntime({ host, createStartIntent: () => ({ source, options: { audio: 'none', localOnly: true }, invocation, destinationPath: '/synthetic/not-created.apparent' }) });
  try {
    await runtime.startCapture('ordered');
    assert.equal((await runtime.dispatch({ type: 'stop' })).phase, 'processing');
    assert.deepEqual(runtime.completionEligibleCaptureIds?.(), ['ordered']);
  } finally { runtime.dispose(); }
});

test("readiness readback and events update an owned capture without changing its phase", async () => {
  let emit!: (event: import('../../src/backend/host/capture2.ts').Capture2Event) => void;
  const host = {
    subscribe(listener: typeof emit) { emit = listener; return () => undefined; },
    readiness: async () => ({ ready: true }), reconcile: async () => [],
    discoverSources: async () => ({ state: 'available' as const, sources: [source] }),
    start: async () => ({ requestId: 'readiness-owned', phase: 'recording' as const }),
    inspect: async () => ({ requestId: 'readiness-owned', phase: 'recording' as const }),
    stop: async () => ({ requestId: 'readiness-owned', phase: 'materials-ready' as const }),
    cancel: async () => ({ requestId: 'readiness-owned', phase: 'cancelled' as const }), dispose() {},
  };
  const runtime = await createCapture2RecordRuntime({ host, createStartIntent: () => ({ source, options: { audio: 'none', localOnly: true }, invocation, destinationPath: '/synthetic/not-created.apparent' }) });
  try {
    assert.equal(runtime.state.readiness.ready, true, 'initial explicit readback is visible without a push event');
    await runtime.startCapture('readiness-owned');
    emit({ type: 'readiness-changed', readiness: { ready: false, reason: 'synthetic denied' } });
    assert.equal(runtime.state.phase, 'recording');
    assert.equal(runtime.state.readiness.ready, false);
    assert.equal((await runtime.dispatch({ type: 'refresh-readiness' })).readiness.ready, true);
    assert.equal(runtime.state.phase, 'recording');
    assert.equal((await runtime.dispatch({ type: 'stop' })).phase, 'processing');
  } finally { runtime.dispose(); }
});
