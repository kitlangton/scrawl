import assert from "node:assert/strict";
import { chmod, lstat, mkdir, mkdtemp, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { DatabaseSync } from "node:sqlite";
import { test } from "node:test";
import { CaptureClient, type CaptureRequest, type CaptureResponse, type CaptureTransport, type SourceDescriptor } from "@apparent/engine-sdk/capture";
import { createCapture2Host } from "../../dist/backend/host/capture2.js";
import { AbsentDestinationAllocator, SqliteCaptureMappingStore } from "../../dist/backend/host/index.js";

const source: SourceDescriptor = { id: "a".repeat(32), generation: "b".repeat(32), kind: "display", displayId: 1, bounds: { x: 0, y: 0, width: 1280, height: 720, scale: 1 } };
const invocation = { invocationId: "c".repeat(32), observedAtMs: 1000, kind: "explicit" as const };

/** Scripted transport is synthetic contract coverage, not a native capture peer. */
class ScriptedTransport implements CaptureTransport {
  requests: CaptureRequest[] = [];
  closed = false;
  readiness: "ready" | "unavailable" = "ready";
  sources: SourceDescriptor[] = [source];
  status = new Map<string, object>();
  loseStartReply = false;
  beforeStart?: (request: Extract<CaptureRequest, { method: "start" }>) => Promise<void>;
  beforeInspect?: () => Promise<void>;
  beforeControl?: (method: "stop" | "cancel") => Promise<void>;
  setSyntheticSaved(captureId: string, bundleId: string, bundlePath: string) {
    const materials = { captureId, materialsId: "f".repeat(32), revision: "e".repeat(64) };
    const manifest = { schema: "apparent.capture-materials/1", key: materials, root: "/tmp/synthetic-materials", leaseId: "1".repeat(32), payloadFile: { relativePath: "materials.json", identity: { device: "1", inode: "2", size: 1, modifiedNs: "3" }, sha256: materials.revision } };
    this.status.set(captureId, { captureId, state: "saved", materials: manifest, publication: { captureId, materials, publicationId: "2".repeat(32), planSha256: "3".repeat(64), bundleId, bundlePath, artifactVersion: 1, artifactProvenance: "native", manifestSha256: "4".repeat(64), inventorySha256: "5".repeat(64) } });
  }
  async request(request: CaptureRequest): Promise<CaptureResponse> {
    this.requests.push(structuredClone(request));
    if (request.method === "handshake") return { channel: "capture", protocolVersion: 2, rpcId: request.rpcId, result: { channel: "capture", protocolVersion: 2, implementation: "native", readiness: this.readiness, capabilities: { sourceDiscovery: true, displayCapture: true, areaCapture: false, audioCapture: false, contextFacts: false, interactionFacts: false, candidates: false, transcript: false, strokes: false, publication: false, retainedMaterials: true } } };
    if (request.method === "discoverSources") return { channel: "capture", protocolVersion: 2, rpcId: request.rpcId, result: { state: "available", sources: this.sources } };
    if (request.method === "start") {
      this.status.set(request.captureId, { captureId: request.captureId, state: "recording" });
      await this.beforeStart?.(request);
      if (this.loseStartReply) { this.loseStartReply = false; throw new Error("synthetic lost reply"); }
      return { channel: "capture", protocolVersion: 2, rpcId: request.rpcId, result: this.status.get(request.captureId) };
    }
    if (request.method === "inspect" || request.method === "stop" || request.method === "cancel") {
      if (request.method === "stop" || request.method === "cancel") await this.beforeControl?.(request.method);
      let status = structuredClone(this.status.get(request.captureId));
      if (request.method === "inspect") await this.beforeInspect?.();
      if (request.method === "stop" && status) {
        const revision = "e".repeat(64);
        status = { captureId: request.captureId, state: "materials-ready", materials: { schema: "apparent.capture-materials/1", key: { captureId: request.captureId, materialsId: "f".repeat(32), revision }, root: "/tmp/synthetic-materials", leaseId: "1".repeat(32), payloadFile: { relativePath: "materials.json", identity: { device: "1", inode: "2", size: 1, modifiedNs: "3" }, sha256: revision } } };
        this.status.set(request.captureId, status);
      }
      if (request.method === "cancel" && status) { status = { captureId: request.captureId, state: "cancelled" }; this.status.set(request.captureId, status); }
      if (!status) return { channel: "capture", protocolVersion: 2, rpcId: request.rpcId, error: { code: "not_found", message: "not found" } };
      return { channel: "capture", protocolVersion: 2, rpcId: request.rpcId, result: status };
    }
    return { channel: "capture", protocolVersion: 2, rpcId: request.rpcId, error: { code: "unavailable", message: "unsupported fixture request" } };
  }
  async close() { this.closed = true; return "already-exited" as const; }
}

async function setup(t: { after(fn: () => void | Promise<void>): void }, ready: "ready" | "unavailable" = "ready") {
  const root = await realpath(await mkdtemp(join(tmpdir(), "record-capture2-host-")));
  t.after(() => rm(root, { recursive: true, force: true }));
  const transport = new ScriptedTransport(); transport.readiness = ready;
  const client = new CaptureClient(transport);
  const mappings = new SqliteCaptureMappingStore(join(root, "control"));
  const destinations = new AbsentDestinationAllocator(join(root, "control"), join(root, "bundles"));
  return { root, transport, client, mappings, destinations, host: createCapture2Host({ client, mappings, destinations }) };
}

test("unavailable handshake blocks admission without start dispatch", async t => {
  const ctx = await setup(t, "unavailable");
  const readiness = await ctx.host.readiness();
  assert.equal(readiness.ready, false);
  await assert.rejects(ctx.host.start({ requestId: "public/request", source, options: { audio: "none", localOnly: true }, invocation }), /no native start was dispatched/u);
  assert.equal(ctx.transport.requests.some(request => request.method === "start"), false);
  assert.equal(await ctx.mappings.getByRequestId("public/request"), undefined);
  ctx.host.dispose();
});

test("published SDK discovery, durable intent, request mapping, materials, restart, and disposal", async t => {
  const ctx = await setup(t);
  assert.equal((await ctx.host.readiness()).ready, true);
  assert.equal((await ctx.host.discoverSources()).sources[0]?.generation, source.generation);
  const stale = { ...source, generation: "d".repeat(32) };
  await assert.rejects(ctx.host.start({ requestId: "public arbitrary id", source: stale, options: { audio: "none", localOnly: true }, invocation }), /exact descriptor/u);
  assert.equal(await ctx.mappings.getByRequestId("public arbitrary id"), undefined);
  ctx.transport.beforeStart = async request => {
    const persisted = await ctx.mappings.getByRequestId("public arbitrary id");
    assert.equal(persisted?.captureId, request.captureId);
    assert.ok(persisted?.capture2Intent);
  };
  const receipt = await ctx.host.start({ requestId: "public arbitrary id", source, options: { audio: "none", localOnly: true }, invocation });
  assert.equal(receipt.requestId, "public arbitrary id");
  assert.equal(receipt.phase, "recording");
  const binding = await ctx.mappings.getByRequestId("public arbitrary id");
  assert.ok(binding?.capture2Intent);
  assert.deepEqual(await ctx.mappings.listRequestIds("capture2"), ["public arbitrary id"]);
  assert.deepEqual(await ctx.mappings.listRequestIds("v1"), []);
  assert.equal(ctx.transport.requests.find(request => request.method === "start")?.captureId, binding?.captureId);
  assert.equal(ctx.transport.requests.find(request => request.method === "start")?.ownerToken, binding?.ownerToken);
  assert.equal(JSON.stringify(receipt).includes(binding!.ownerToken), false);
  await assert.rejects(ctx.host.start({ requestId: "public arbitrary id", source, options: { audio: "microphone", localOnly: true }, invocation }), /conflicts/u);
  const stopped = await ctx.host.stop("public arbitrary id");
  assert.equal(stopped.phase, "materials-ready");
  assert.equal(stopped.materials?.leaseId, "1".repeat(32));
  ctx.host.dispose();
  assert.equal(ctx.transport.closed, false);
  const restarted = createCapture2Host({ client: ctx.client, mappings: ctx.mappings, destinations: ctx.destinations });
  const reconciled = await restarted.reconcile();
  assert.equal(reconciled[0]?.requestId, "public arbitrary id");
  assert.equal(reconciled[0]?.phase, "materials-ready");
  assert.equal(reconciled[0]?.materials?.leaseId, "1".repeat(32));
  ctx.transport.setSyntheticSaved(binding!.captureId, binding!.bundleId, binding!.bundlePath);
  const saved = await restarted.inspect("public arbitrary id");
  assert.equal(saved.phase, "processing", "native Saved is only a Record processing fact");
  assert.equal(saved.publication?.artifactProvenance, "native");
  assert.equal(ctx.transport.requests.filter(request => request.method === "start").length, 1);
  restarted.dispose();
});

test("exact destination is snapshotted into durable intent before SDK start and survives restart", async t => {
  const ctx = await setup(t); await ctx.host.discoverSources();
  const destinationPath = join(ctx.root, "requested.apparent");
  const intent = { requestId: "exact-output", source, options: { audio: "none" as const, localOnly: true as const }, invocation, destinationPath };
  ctx.transport.beforeStart = async request => {
    const mapping = await ctx.mappings.getByRequestId("exact-output");
    assert.equal(mapping?.bundlePath, destinationPath);
    assert.ok(mapping?.capture2Intent);
    assert.equal(request.bundlePath, destinationPath);
    assert.equal(await lstat(destinationPath).then(() => true, () => false), false);
  };
  ctx.transport.loseStartReply = true;
  const pending = ctx.host.start(intent);
  await new Promise(resolve => setImmediate(resolve));
  intent.destinationPath = join(ctx.root, "mutated.apparent");
  await assert.rejects(pending, /same Record request/u);
  const mapping = await ctx.mappings.getByRequestId("exact-output"); assert.ok(mapping);
  assert.equal(mapping.bundlePath, destinationPath);
  assert.equal((JSON.parse(mapping.capture2Intent!).destinationPath), destinationPath);
  assert.equal(ctx.transport.requests.find((request): request is Extract<CaptureRequest, { method: "start" }> => request.method === "start")?.bundlePath, destinationPath);
  ctx.transport.setSyntheticSaved(mapping.captureId, mapping.bundleId, destinationPath);
  ctx.host.dispose();
  const restarted = createCapture2Host({ client: new CaptureClient(ctx.transport), mappings: ctx.mappings, destinations: ctx.destinations });
  assert.equal((await restarted.start({ requestId: "exact-output", source, options: { audio: "none", localOnly: true }, invocation, destinationPath })).phase, "processing");
  await assert.rejects(restarted.start({ ...intent, destinationPath: join(ctx.root, "changed.apparent") }), /conflicts with its durable request binding/u);
  assert.equal(ctx.transport.requests.filter(request => request.method === "start").length, 1);
  restarted.dispose();
});

test("ambiguous start retains the original authority for reconciliation, without SDK close", async t => {
  const ctx = await setup(t); await ctx.host.discoverSources(); ctx.transport.loseStartReply = true;
  await assert.rejects(ctx.host.start({ requestId: "ambiguous", source, options: { audio: "none", localOnly: true }, invocation }), /same Record request/u);
  const original = await ctx.mappings.getByRequestId("ambiguous"); assert.ok(original);
  const inspected = await ctx.host.inspect("ambiguous");
  assert.equal(inspected.phase, "recording");
  assert.equal(ctx.transport.requests.filter(request => request.method === "start").length, 1);
  ctx.host.dispose(); assert.equal(ctx.transport.closed, false);
});

test("concurrent admission rejects a competing ID before it is mapped; cancel uses the durable public key", async t => {
  const ctx = await setup(t); await ctx.host.discoverSources();
  let release!: () => void;
  let entered!: () => void;
  const dispatching = new Promise<void>(resolve => { entered = resolve; });
  ctx.transport.beforeStart = () => new Promise<void>(resolve => { release = resolve; entered(); });
  const intent = { requestId: "first", source, options: { audio: "none" as const, localOnly: true as const }, invocation };
  const first = ctx.host.start(intent);
  await dispatching;
  const sameRequest = ctx.host.start(intent);
  await assert.rejects(ctx.host.start({ requestId: "competing", source, options: { audio: "none", localOnly: true }, invocation }), /request ID was not reserved/u);
  assert.equal(await ctx.mappings.getByRequestId("competing"), undefined);
  release();
  assert.deepEqual(await Promise.all([first, sameRequest]), [{ requestId: "first", phase: "recording" }, { requestId: "first", phase: "recording" }]);
  assert.equal(ctx.transport.requests.filter(request => request.method === "start").length, 1);
  assert.equal((await ctx.host.cancel("first")).phase, "cancelled");
  const cancel = [...ctx.transport.requests].reverse().find((request): request is Extract<CaptureRequest, { method: "inspect" | "stop" | "cancel" }> => request.method === "cancel");
  assert.equal(cancel?.captureId, (await ctx.mappings.getByRequestId("first"))?.captureId);
  ctx.host.dispose();
});

test("legacy v1 mapping rows migrate in place and cannot be relabeled as capture/2", async t => {
  const root = await realpath(await mkdtemp(join(tmpdir(), "record-capture2-legacy-")));
  t.after(() => rm(root, { recursive: true, force: true }));
  const control = join(root, "control");
  await mkdir(control, { mode: 0o700 }); await chmod(control, 0o700);
  const path = join(control, "capture-mappings.sqlite");
  const db = new DatabaseSync(path);
  db.exec("CREATE TABLE capture_mapping (request_id TEXT PRIMARY KEY, capture_id TEXT NOT NULL UNIQUE, owner_token TEXT NOT NULL, bundle_id TEXT NOT NULL UNIQUE, bundle_path TEXT NOT NULL UNIQUE) STRICT; PRAGMA user_version = 1;");
  const old = { requestId: "old-v1-request", captureId: "1".repeat(32), ownerToken: "2".repeat(64), bundleId: "old.apparent", bundlePath: join(root, "bundles", "old.apparent") };
  db.prepare("INSERT INTO capture_mapping VALUES (?, ?, ?, ?, ?)").run(old.requestId, old.captureId, old.ownerToken, old.bundleId, old.bundlePath);
  db.close(); await chmod(path, 0o600);
  const mappings = new SqliteCaptureMappingStore(control);
  assert.deepEqual(await mappings.getByRequestId(old.requestId), old);
  await assert.rejects(mappings.putIfAbsent({ ...old, capture2Intent: JSON.stringify({ protocol: 2 }) }), /conflicts/u);
  assert.deepEqual(await mappings.getByRequestId(old.requestId), old);
});

test("deferred controls serialize on the original request key and terminal receipts cannot regress", async t => {
  const ctx = await setup(t); await ctx.host.discoverSources();
  await ctx.host.start({ requestId: "control-order", source, options: { audio: "none", localOnly: true }, invocation });
  let release!: () => void, entered!: () => void;
  const pending = new Promise<void>(resolve => { entered = resolve; });
  ctx.transport.beforeControl = method => method === "stop" ? new Promise<void>(resolve => { release = resolve; entered(); }) : Promise.resolve();
  const stopping = ctx.host.stop("control-order");
  await pending;
  const cancelling = ctx.host.cancel("control-order");
  assert.equal(ctx.transport.requests.filter(request => request.method === "stop" || request.method === "cancel").length, 1);
  release();
  assert.equal((await stopping).phase, "materials-ready");
  assert.equal((await cancelling).phase, "cancelled");
  assert.equal((await ctx.host.inspect("control-order")).phase, "cancelled");
  ctx.host.dispose();
});

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>(done => { resolve = done; });
  return { promise, resolve };
}

test("start snapshots intent before storage waits, and disposal prevents late dispatch", async t => {
  for (const dispose of [false, true]) {
    const ctx = await setup(t); await ctx.host.discoverSources();
    const entered = deferred(), release = deferred();
    const read = ctx.mappings.getByRequestId.bind(ctx.mappings);
    ctx.mappings.getByRequestId = async id => { entered.resolve(); await release.promise; return read(id); };
    const intent = { requestId: "immutable", source: structuredClone(source), options: { audio: "none" as "none" | "microphone", localOnly: true as const }, invocation: { ...invocation } };
    const pending = ctx.host.start(intent);
    await entered.promise;
    intent.requestId = "changed"; intent.options.audio = "microphone"; intent.invocation.observedAtMs = 2000;
    if (dispose) ctx.host.dispose();
    release.resolve();
    if (dispose) {
      await assert.rejects(pending, /disposed/u);
      assert.equal(ctx.transport.requests.some(r => r.method === "start"), false);
      assert.equal(await read("immutable"), undefined);
    } else {
      assert.equal((await pending).requestId, "immutable");
      const row = await read("immutable"); assert.ok(row);
      const sent = ctx.transport.requests.find((r): r is Extract<CaptureRequest, { method: "start" }> => r.method === "start");
      assert.equal(sent?.options.audio, "none");
      assert.equal(sent?.invocation.observedAtMs, 1000);
      assert.deepEqual(JSON.parse(row.capture2Intent!).options, sent?.options);
    }
    ctx.host.dispose(); assert.equal(ctx.transport.closed, false);
  }
});

test("cold reconnect handshakes owned controls even when new capture readiness is unavailable", async t => {
  const ctx = await setup(t); await ctx.host.discoverSources();
  await ctx.host.start({ requestId: "restart", source, options: { audio: "none", localOnly: true }, invocation });
  ctx.host.dispose(); ctx.transport.readiness = "unavailable";
  const cold = createCapture2Host({ client: new CaptureClient(ctx.transport), mappings: ctx.mappings, destinations: ctx.destinations });
  assert.equal((await cold.reconcile())[0]?.phase, "recording");
  assert.equal((await cold.stop("restart")).phase, "materials-ready");
  cold.dispose();
});

test("reconciliation cannot block Stop or overwrite its newer materials observation", async t => {
  const ctx = await setup(t); await ctx.host.discoverSources();
  await ctx.host.start({ requestId: "scan", source, options: { audio: "none", localOnly: true }, invocation });
  const entered = deferred(), release = deferred();
  ctx.transport.beforeInspect = async () => { entered.resolve(); await release.promise; throw new Error("late inspection failure"); };
  const scanning = ctx.host.reconcile(); await entered.promise;
  const stopped = await ctx.host.stop("scan");
  assert.equal(stopped.phase, "materials-ready");
  release.resolve();
  assert.deepEqual((await scanning)[0], stopped);
  ctx.host.dispose();
});

test("ambiguous controls report uncertainty while retaining material lease and last observed phase", async t => {
  const ctx = await setup(t); await ctx.host.discoverSources();
  await ctx.host.start({ requestId: "retained", source, options: { audio: "none", localOnly: true }, invocation });
  const stopped = await ctx.host.stop("retained");
  ctx.transport.beforeControl = async () => { throw new Error("lost cancellation reply"); };
  const uncertain = await ctx.host.cancel("retained");
  assert.equal(uncertain.phase, "unknown");
  assert.equal(uncertain.lastKnownPhase, "materials-ready");
  assert.deepEqual(uncertain.materials, stopped.materials);
  ctx.transport.beforeControl = undefined;
  const cancelled = await ctx.host.cancel("retained");
  assert.equal(cancelled.phase, "cancelled");
  assert.deepEqual(cancelled.materials, stopped.materials, "absence is not permission to discard a retained lease");
  ctx.host.dispose();
});

test("restart refuses publication facts for a different durable destination", async t => {
  const ctx = await setup(t); await ctx.host.discoverSources();
  await ctx.host.start({ requestId: "bound", source, options: { audio: "none", localOnly: true }, invocation });
  const row = await ctx.mappings.getByRequestId("bound"); assert.ok(row);
  ctx.transport.setSyntheticSaved(row.captureId, "wrong.apparent", "/tmp/wrong.apparent");
  ctx.host.dispose();
  const cold = createCapture2Host({ client: new CaptureClient(ctx.transport), mappings: ctx.mappings, destinations: ctx.destinations });
  const receipt = await cold.inspect("bound");
  assert.equal(receipt.phase, "unknown"); assert.equal(receipt.publication, undefined);
  cold.dispose();
});


test("owned capture can stop before its start reply, and a late lost reply cannot erase materials", async t => {
  const ctx = await setup(t); await ctx.host.discoverSources();
  const entered = deferred(), release = deferred();
  ctx.transport.loseStartReply = true;
  ctx.transport.beforeStart = async () => { entered.resolve(); await release.promise; };
  const starting = ctx.host.start({ requestId: "early-stop", source, options: { audio: "none", localOnly: true }, invocation });
  await entered.promise;
  const stopped = await ctx.host.stop("early-stop");
  assert.equal(stopped.phase, "materials-ready");
  release.resolve();
  assert.deepEqual(await starting, stopped);
  ctx.host.dispose();
});

test("disposal from a reserved notification keeps the binding but prevents native dispatch", async t => {
  const ctx = await setup(t); await ctx.host.discoverSources();
  ctx.host.subscribe(event => { if (event.type === "capture-changed" && event.receipt.phase === "reserved") ctx.host.dispose(); });
  await assert.rejects(ctx.host.start({ requestId: "dispose-reserved", source, options: { audio: "none", localOnly: true }, invocation }), /disposed/u);
  assert.ok(await ctx.mappings.getByRequestId("dispose-reserved"));
  assert.equal(ctx.transport.requests.some(r => r.method === "start"), false);
  assert.equal(ctx.transport.closed, false);
});


test("two hosts cannot admit different requests from the same stale durable scan", async t => {
  const ctx = await setup(t); await ctx.host.discoverSources();
  const second = createCapture2Host({ client: new CaptureClient(ctx.transport), mappings: ctx.mappings, destinations: ctx.destinations });
  await second.discoverSources();
  const bothScanned = deferred(); let scans = 0;
  const list = ctx.mappings.listRequestIds.bind(ctx.mappings);
  ctx.mappings.listRequestIds = async protocol => {
    const ids = await list(protocol);
    if (protocol === "capture2" && scans < 2) { if (++scans === 2) bothScanned.resolve(); await bothScanned.promise; }
    return ids;
  };
  const results = await Promise.allSettled([
    ctx.host.start({ requestId: "host-a", source, options: { audio: "none", localOnly: true }, invocation }),
    second.start({ requestId: "host-b", source, options: { audio: "none", localOnly: true }, invocation }),
  ]);
  assert.equal(results.filter(result => result.status === "fulfilled").length, 1);
  assert.equal(ctx.transport.requests.filter(r => r.method === "start").length, 1);
  const ids = await list("capture2"); assert.equal(ids.length, 1);
  const refused = results.find(result => result.status === "rejected");
  assert.ok(refused?.status === "rejected" && /registry changed/u.test(String(refused.reason)));
  await ctx.host.cancel(ids[0]!);
  await second.reconcile();
  assert.equal((await second.start({ requestId: "after-terminal", source, options: { audio: "none", localOnly: true }, invocation })).phase, "recording");
  ctx.host.dispose(); second.dispose();
});
