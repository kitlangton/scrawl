import assert from "node:assert/strict";
import { chmod, mkdir, mkdtemp, realpath, rm } from "node:fs/promises";
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
      await this.beforeStart?.(request);
      this.status.set(request.captureId, { captureId: request.captureId, state: "recording" });
      if (this.loseStartReply) { this.loseStartReply = false; throw new Error("synthetic lost reply"); }
      return { channel: "capture", protocolVersion: 2, rpcId: request.rpcId, result: this.status.get(request.captureId) };
    }
    if (request.method === "inspect" || request.method === "stop" || request.method === "cancel") {
      if (request.method === "stop" || request.method === "cancel") await this.beforeControl?.(request.method);
      let status = this.status.get(request.captureId);
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
