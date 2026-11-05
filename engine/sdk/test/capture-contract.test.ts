import test from "node:test";
import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { PassThrough } from "node:stream";
import type { ChildProcessWithoutNullStreams } from "node:child_process";
import {
  CaptureClient,
  encodeCaptureRequest,
  validateCaptureRequest,
} from "../src/capture.ts";
import type {
  CaptureRequest,
  CaptureResponse,
  CaptureTransport,
} from "../src/capture.ts";
import { ChildProcessCaptureTransport } from "../src/capture.ts";
import { ReferenceCapturePeer } from "../src/capture-testing.ts";
import { validateInteractionFacts } from "../src/capture-documents/facts.ts";

// Scripted wire claims exercise SDK validation only; they are not a native peer
// or capture/readiness evidence and are never exposed by the testing export.
const captureId = "b".repeat(32);
const authority = { captureId, ownerToken: "a".repeat(64) };
const materialsKey = { captureId, materialsId: "e".repeat(32), revision: "f".repeat(64) };
const manifest = { schema: "apparent.capture-materials/1", key: materialsKey, root: "/private/materials", leaseId: "d".repeat(32), payloadFile: { relativePath: "materials.json", sha256: materialsKey.revision, identity: { device: "1", inode: "2", size: 1, modifiedNs: "3" } } };
const source = { id: "display:1", generation: "c".repeat(32), kind: "display" as const, displayId: 1, bounds: { x: 0, y: 0, width: 640, height: 360, scale: 1 } };
const publicationInput = { ...authority, publicationId: "c".repeat(32), planSha256: "d".repeat(64), expected: { materials: materialsKey, bundleId: "take", bundlePath: "/captures/take.apparent", manifestSha256: "1".repeat(64) } };
function scripted(result: (request: CaptureRequest) => unknown, options: { testDouble?: boolean; unavailable?: boolean; capabilities?: Record<string, boolean> } = {}): CaptureTransport {
  return {
    async request(request) {
      const hello = { ...new ReferenceCapturePeer().handshake, implementation: options.testDouble ? "test-double" : "native", readiness: options.unavailable ? "unavailable" : "ready", capabilities: { ...new ReferenceCapturePeer().handshake.capabilities, sourceDiscovery: true, displayCapture: true, retainedMaterials: true, publication: true, ...options.capabilities } };
      return { channel: "capture", protocolVersion: 2, rpcId: request.rpcId, result: request.method === "handshake" ? hello : result(request) };
    },
    async close() { return "already-exited"; },
  };
}

test("capture/2 is a separate strict channel and reference peer cannot claim native readiness", async () => {
  const peer = new ReferenceCapturePeer();
  const client = new CaptureClient(peer, { allowTestDouble: true });
  const handshake = await client.handshake();
  assert.equal(handshake.readiness, "unavailable");
  assert.equal(handshake.capabilities.displayCapture, false);
  await assert.rejects(
    new CaptureClient(new ReferenceCapturePeer()).handshake(),
    /explicit/,
  );
  const base = {
    channel: "capture",
    protocolVersion: 2,
    clientId: "a".repeat(32),
    rpcId: "b".repeat(32),
    method: "handshake",
  };
  assert.match(encodeCaptureRequest(base), /"channel":"capture"/);
  assert.throws(
    () => validateCaptureRequest({ ...base, unknown: true }),
    /invalid capture\/2 fields/,
  );
  assert.throws(
    () => validateCaptureRequest({ ...base, protocolVersion: 1 }),
    /unsupported/,
  );
  assert.equal((await client.discoverSources()).state, "unavailable");
  await client.close();
});

test("timeouts and late or wrong-version replies cannot resolve a new RPC", async () => {
  await assert.rejects(
    new CaptureClient(new ReferenceCapturePeer("wrong-version"), {
      allowTestDouble: true,
    }).handshake(),
    /envelope/,
  );
  await assert.rejects(
    new CaptureClient(new ReferenceCapturePeer("malformed-late-reply"), {
      allowTestDouble: true,
    }).handshake(),
    /envelope/,
  );
  const peer = new ReferenceCapturePeer("timeout");
  const client = new CaptureClient(peer, { allowTestDouble: true });
  await assert.rejects(client.handshake(10), /timed out/);
  await client.close();
});

test("explicit test scenarios can observe materials-ready and publishing without asserting native evidence", async () => {
  for (const [scenario, state] of [
    ["materials-ready", "materials-ready"],
    ["publishing", "publishing"],
    ["cancelled", "cancelled"],
  ] as const) {
    const client = new CaptureClient(new ReferenceCapturePeer(scenario), {
      allowTestDouble: true,
    });
    await client.handshake();
    const status = await client.inspect({
      ownerToken: "a".repeat(64),
      captureId: "b".repeat(32),
    });
    assert.equal(status.state, state);
    assert.equal(status.publication, undefined);
    if (state === "materials-ready" || state === "publishing")
      assert.equal(status.materials?.key.captureId, "b".repeat(32));
    await client.close();
  }
});

test("test-double handshake cannot return a native Saved receipt", async () => {
  const peer: CaptureTransport = {
    async request(request: CaptureRequest): Promise<CaptureResponse> {
      if (request.method === "handshake")
        return {
          channel: "capture",
          protocolVersion: 2,
          rpcId: request.rpcId,
          result: new ReferenceCapturePeer().handshake,
        };
      return {
        channel: "capture",
        protocolVersion: 2,
        rpcId: request.rpcId,
        result: {
          captureId: request.captureId,
          state: "saved",
          publication: {
            captureId: request.captureId,
            materials: {
              captureId: request.captureId,
              materialsId: "d".repeat(32),
              revision: "a".repeat(64),
            },
            publicationId: "e".repeat(32),
            planSha256: "f".repeat(64),
            bundleId: "take.apparent",
            bundlePath: "/captures/take.apparent",
            artifactVersion: 1,
            artifactProvenance: "native",
            manifestSha256: "1".repeat(64),
            inventorySha256: "2".repeat(64),
          },
        },
      };
    },
    async close() {
      return "already-exited";
    },
  };
  const client = new CaptureClient(peer, { allowTestDouble: true });
  await client.handshake();
  await assert.rejects(
    client.inspect({ ownerToken: "a".repeat(64), captureId: "b".repeat(32) }),
    /cannot provide native Saved provenance/,
  );
  await client.close();
});

test("close distinguishes observed signal exits from signal requests", async () => {
  const makeChild = (exitOnEof: boolean) => {
    const child = new EventEmitter() as EventEmitter & {
      stdin: PassThrough;
      stdout: PassThrough;
      stderr: PassThrough;
      exitCode: number | null;
      kill: (signal: "SIGTERM" | "SIGKILL") => boolean;
    };
    child.stdin = new PassThrough();
    child.stdout = new PassThrough();
    child.stderr = new PassThrough();
    child.exitCode = null;
    const finish = (signal: "SIGTERM" | "SIGKILL" | null) => {
      child.exitCode = 0;
      child.emit("exit", 0, signal);
    };
    child.stdin.end = (() => {
      if (exitOnEof) queueMicrotask(() => finish(null));
      return child.stdin;
    }) as PassThrough["end"];
    child.kill = (signal) => {
      queueMicrotask(() => finish(signal));
      return true;
    };
    return child as unknown as ChildProcessWithoutNullStreams;
  };
  assert.equal(
    await new ChildProcessCaptureTransport(makeChild(true), 10).close(),
    "eof-exited",
  );
  assert.equal(
    await new ChildProcessCaptureTransport(makeChild(false), 10).close(),
    "signal-exited",
  );
});

test("publication reconnect checks pending and terminal identities against the restored plan", async () => {
  const pending = { captureId, state: "publishing", materials: manifest, publicationId: publicationInput.publicationId, planSha256: publicationInput.planSha256 };
  const saved = { captureId, state: "saved", publication: { captureId, ...publicationInput.expected, publicationId: publicationInput.publicationId, planSha256: publicationInput.planSha256, artifactVersion: 1, artifactProvenance: "native", inventorySha256: "2".repeat(64) } };
  for (const result of [pending, saved, { captureId, state: "failed", error: { code: "internal" }, publicationId: publicationInput.publicationId, planSha256: publicationInput.planSha256 }, { captureId, state: "cancelled", publicationId: publicationInput.publicationId, planSha256: publicationInput.planSha256 }]) {
    const client = new CaptureClient(scripted(() => result)); await client.handshake();
    assert.equal((await client.inspectPublication(publicationInput)).state, result.state);
  }
  for (const result of [
    { ...pending, publicationId: "0".repeat(32) },
    { ...pending, planSha256: "0".repeat(64) },
    { captureId, state: "failed", error: { code: "internal" } },
    { ...saved, publication: { ...saved.publication, bundlePath: "/another/take.apparent" } },
    { ...saved, publication: { ...saved.publication, materials: { ...materialsKey, materialsId: "0".repeat(32) } } },
    { ...saved, publication: { ...saved.publication, manifestSha256: "0".repeat(64) } },
  ]) {
    const client = new CaptureClient(scripted(() => result)); await client.handshake();
    await assert.rejects(client.inspectPublication(publicationInput), /identity mismatch|immutable plan/);
  }
});

test("cached capabilities and sources cannot be changed through returned objects", async () => {
  let starts = 0;
  const client = new CaptureClient(scripted(request => {
    if (request.method === "discoverSources") return { state: "available", sources: [structuredClone(source)] };
    starts++; return { captureId, state: "starting" };
  }));
  const hello = await client.handshake(); hello.capabilities.displayCapture = false;
  const catalog = await client.discoverSources();
  catalog.sources[0].generation = "0".repeat(32);
  const start = { ...authority, bundleId: "take", bundlePath: "/captures/take.apparent", source, options: { audio: "none" as const, localOnly: true as const }, invocation: { kind: "explicit", invocationId: "a".repeat(32), observedAtMs: 0 } };
  await assert.rejects(client.start({ ...start, source: catalog.sources[0] }), /advertised/);
  assert.equal((await client.start(start)).state, "starting"); assert.equal(starts, 1);
  await assert.rejects(client.inspect({ ...authority, rpcId: "0".repeat(32) } as typeof authority), /reserved/);
});

test("test-double capabilities, unsupported methods and secret-shaped error codes are rejected", async () => {
  const falseCaps = Object.fromEntries(Object.keys(new ReferenceCapturePeer().handshake.capabilities).map(key => [key, false]));
  for (const capability of Object.keys(falseCaps)) {
    const client = new CaptureClient(scripted(() => null, { testDouble: true, unavailable: true, capabilities: { ...falseCaps, [capability]: true } }), { allowTestDouble: true });
    await assert.rejects(client.handshake(), /cannot claim/);
  }
  let calls = 0;
  const client = new CaptureClient(scripted(() => { calls++; return null; }, { unavailable: true, capabilities: falseCaps }));
  await client.handshake(); assert.equal((await client.discoverSources()).state, "unavailable");
  await assert.rejects(client.releaseMaterials({ ...authority, materialsId: materialsKey.materialsId, leaseId: manifest.leaseId }), /unavailable/);
  await assert.rejects(client.publish({} as Parameters<CaptureClient["publish"]>[0]), /unavailable/); assert.equal(calls, 0);
  const badError: CaptureTransport = { async request(request) { return request.method === "handshake" ? scripted(() => null).request(request) : { channel: "capture", protocolVersion: 2, rpcId: request.rpcId, error: { code: authority.ownerToken, message: "private diagnostic" } }; }, async close() { return "already-exited"; } };
  const errors = new CaptureClient(badError); await errors.handshake();
  await assert.rejects(errors.inspect(authority), error => !String(error).includes(authority.ownerToken) && /invalid capture error code/.test(String(error)));
  const privateFailure: CaptureTransport = { async request(request) { if (request.method === "handshake") return scripted(() => null).request(request); throw new Error(authority.ownerToken); }, async close() { return "already-exited"; } };
  const failed = new CaptureClient(privateFailure); await failed.handshake();
  await assert.rejects(failed.inspect(authority), error => !String(error).includes(authority.ownerToken) && /outcome is unknown/.test(String(error)));
});

test("strict capture paths and scroll delta types reject malformed data", async () => {
  const request = { channel: "capture", protocolVersion: 2, clientId: "a".repeat(32), rpcId: "b".repeat(32), method: "publish", ...authority, leaseId: manifest.leaseId, publicationId: publicationInput.publicationId, materials: materialsKey, stagingRoot: "/private/staging", planFile: { ...manifest.payloadFile, relativePath: "../elsewhere" }, planSha256: materialsKey.revision };
  assert.throws(() => validateCaptureRequest(request), /relative path/);
  const materials = { payload: { identity: { captureId, materialsId: materialsKey.materialsId }, clock: { durationMs: 10 } } } as Parameters<typeof validateInteractionFacts>[1];
  assert.throws(() => validateInteractionFacts({ schema: "apparent.capture-interactions/1", identity: materials.payload.identity, events: [{ kind: "scroll", t: 1, x: .5, y: .5, dx: "up", dy: {} }] }, materials), /finite numbers/);
});

test("transport rejects duplicate pending RPC IDs and malformed late envelopes", async () => {
  const child = new EventEmitter() as unknown as ChildProcessWithoutNullStreams;
  Object.assign(child, { stdin: new PassThrough(), stdout: new PassThrough(), stderr: new PassThrough() });
  const transport = new ChildProcessCaptureTransport(child, 10);
  const request = { channel: "capture", protocolVersion: 2, clientId: "a".repeat(32), rpcId: "b".repeat(32), method: "handshake" } as const;
  const first = transport.request(request, 1000);
  await assert.rejects(transport.request(request), /duplicate/);
  const rejected = assert.rejects(first, /invalid capture response envelope/);
  (child.stdout as PassThrough).write(`${JSON.stringify({ channel: "capture", protocolVersion: 2, rpcId: "c".repeat(32), result: {}, error: {} })}\n`);
  await rejected;
  assert.throws(() => new ChildProcessCaptureTransport(child, 0), /timeout/);
});

test("terminal inspection preserves retained materials authority for reconnect and release", async () => {
  const client = new CaptureClient(scripted(request => request.method === "releaseMaterials"
    ? { released: true, captureId, materialsId: materialsKey.materialsId }
    : { captureId, state: "cancelled", materials: manifest }));
  await client.handshake();
  const recovered = await client.inspect(authority);
  assert.equal(recovered.materials?.leaseId, manifest.leaseId);
  assert.equal((await client.releaseMaterials({ ...authority, materialsId: recovered.materials!.key.materialsId, leaseId: recovered.materials!.leaseId })).released, true);
});

test("a Saved start replay must name the originally requested bundle", async () => {
  const publication = { captureId, ...publicationInput.expected, publicationId: publicationInput.publicationId, planSha256: publicationInput.planSha256, artifactVersion: 1, artifactProvenance: "native", inventorySha256: "2".repeat(64) };
  const start = { ...authority, bundleId: publication.bundleId, bundlePath: publication.bundlePath, source, options: { audio: "none" as const, localOnly: true as const }, invocation: { kind: "explicit" as const, invocationId: "a".repeat(32), observedAtMs: 0 } };
  for (const changed of [{}, { bundleId: "other-bundle" }, { bundlePath: "/other/take.apparent" }]) {
    const client = new CaptureClient(scripted(request => request.method === "discoverSources" ? { state: "available", sources: [source] } : { captureId, state: "saved", publication: { ...publication, ...changed } }));
    await client.handshake(); await client.discoverSources();
    if (Object.keys(changed).length) await assert.rejects(client.start(start), /original bundle identity/);
    else assert.equal((await client.start(start)).state, "saved");
  }
});
