import test from "node:test";
import assert from "node:assert/strict";
import { EventEmitter } from "node:events";
import { createHash } from "node:crypto";
import { PassThrough } from "node:stream";
import type { ChildProcessWithoutNullStreams } from "node:child_process";
import { spawnSync } from "node:child_process";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import {
  CaptureClient,
  launchCaptureEngine,
  encodeCaptureRequest,
  validateCaptureRequest,
} from "../src/capture.ts";
import type {
  MaterialsDocument,
  CaptureRequest,
  CaptureResponse,
  CaptureTransport,
} from "../src/capture.ts";
import { ChildProcessCaptureTransport } from "../src/capture.ts";
import { ReferenceCapturePeer } from "../src/capture-testing.ts";
import { validateFactFile, validateInteractionFacts } from "../src/capture-documents/facts.ts";

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
  } as const;
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

test("synthetic CaptureClient exercises strict native ink ingress receipts and exact UTF8 digest", async()=>{
 const host={processEpoch:"epoch",revision:1,pid:2,startTicks:"9007199254740993"};
 const input={ingressVersion:1 as const,...authority,source:{id:"display:1",generation:"source-gen"},observation:{host,producerId:"c".repeat(32),generation:1,bootId:"boot",clock:"linux-clock-monotonic-observation" as const,output:{name:"DP-1",physical:{x:0,y:0,width:640,height:360},compositor:{x:0,y:0,width:640,height:360,scale:1 as const}},surfaces:[{surfaceId:"screen",transformId:"t1",nativeIncarnation:"inc",target:"capture-rectangle" as const,pointSpace:"desktop-physical" as const,contentRectangle:{x:0,y:0,width:640,height:360},plane:{width:640,height:360}}]}};
 const raw='[{"kind":"begin","gestureId":"0123456789abcdef0123456789abcdef","surfaceId":"screen","transformId":"t1","nativeIncarnation":"inc","timeNs":"9007199254740993","x":1.0,"y":2}]';const digest=createHash("sha256").update(Buffer.from(raw,"utf8")).digest("hex");
 const client=new CaptureClient(scripted(req=>req.method==="registerInkProducer"?{ingressVersion:1,captureId,producerId:input.observation.producerId,generation:1,registrationId:"d".repeat(32),producerToken:"e".repeat(64),nextSequence:0,clock:{kind:input.observation.clock,bootId:"boot",admissionMonotonicNs:"9007199254740993"},limits:{maxWireBytes:65536,maxBatchJsonBytes:16384,maxPointsPerBatch:256,maxPointsPerGesture:2400,maxBatches:4096,maxSurfaces:8,maxGestures:4096,maxTotalPoints:65536,maxFactBytes:4194304,finishGraceMs:2000}}:req.method==="submitInkEvents"?{ingressVersion:1,captureId,registrationId:"d".repeat(32),sequence:req.sequence,batchSha256:req.batchSha256,nextSequence:Number(req.sequence)+1,outcome:"accepted"}: {ingressVersion:1,captureId,registrationId:"d".repeat(32),finalSequence:null,state:"complete"})); await client.handshake();
 const registration=await client.registerInkProducer(input);assert.equal(registration.clock.admissionMonotonicNs,"9007199254740993");
 const batch=await client.submitInkEvents({ingressVersion:1,...authority,registrationId:"d".repeat(32),producerToken:"e".repeat(64),sequence:0,eventsJson:raw,batchSha256:digest});assert.equal(batch.outcome,"accepted");
 const finish=await client.finishInkProducer({ingressVersion:1,...authority,registrationId:"d".repeat(32),producerToken:"e".repeat(64),finalSequence:null,seal:{state:"sealed-and-drained",observedAtNs:"9007199254740994",nativeRevision:2}});assert.equal(finish.state,"complete");
 await assert.rejects(client.registerInkProducer({...input,source:{id:"",generation:"g"}}),/source/);
 await assert.rejects(client.submitInkEvents({ingressVersion:1,...authority,registrationId:"d".repeat(32),producerToken:"e".repeat(64),sequence:0,eventsJson:raw,batchSha256:"0".repeat(64)}),/digest/);
 const board=structuredClone(input);board.observation.surfaces=[{surfaceId:"board",transformId:"b1",nativeIncarnation:"window1",target:"whiteboard-canvas",pointSpace:"paper-logical",contentRectangle:{x:4,y:30,width:720,height:476},plane:{width:1600,height:1100}}];assert.equal((await client.registerInkProducer(board)).captureId,captureId);
 await assert.rejects(client.registerInkProducer({...input,observation:{...input.observation,surfaces:[{...input.observation.surfaces[0],pointSpace:"paper-logical"}]}}),/target\/point-space/);
 await assert.rejects(client.registerInkProducer({...input,observation:{...input.observation,surfaces:[{...input.observation.surfaces[0],plane:{width:-1,height:360}}]}}),/ink plane/);
 const malformed='[{"kind":"begin","gestureId":"0123456789abcdef0123456789abcdef","surfaceId":"screen","transformId":"t1","nativeIncarnation":"inc","timeNs":"9007199254740993","x":1,"y":2,"surprise":1}]';await assert.rejects(client.submitInkEvents({ingressVersion:1,...authority,registrationId:"d".repeat(32),producerToken:"e".repeat(64),sequence:0,eventsJson:malformed,batchSha256:createHash("sha256").update(malformed).digest("hex")}),/fields/);
 await assert.rejects(client.submitInkEvents({ingressVersion:1,...authority,registrationId:"d".repeat(32),producerToken:"e".repeat(64),sequence:0,eventsJson:raw.replace("9007199254740993","9007199254740993.0"),batchSha256:createHash("sha256").update(raw.replace("9007199254740993","9007199254740993.0")).digest("hex")}),/canonical|invalid ink/);
 const gesture="0123456789abcdef0123456789abcdef";for(const [sequence,event] of [[0,{kind:"begin",gestureId:gesture,surfaceId:"screen",transformId:"t1",nativeIncarnation:"inc",timeNs:"9007199254740993",x:1,y:2}],[1,{kind:"points",gestureId:gesture,points:[{timeNs:"9007199254740994",x:3,y:4}]}],[2,{kind:"commit",gestureId:gesture,acceptedAtNs:"9007199254740995"}]] as const){const eventsJson=JSON.stringify([event]);const batchSha256=createHash("sha256").update(Buffer.from(eventsJson,"utf8")).digest("hex");assert.equal((await client.submitInkEvents({ingressVersion:1,...authority,registrationId:"d".repeat(32),producerToken:"e".repeat(64),sequence,eventsJson,batchSha256})).sequence,sequence)}
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
      if (!("captureId" in request)) throw new Error("fixture expects an owned capture request");
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
      unref: () => void;
    };
    child.stdin = new PassThrough();
    child.stdout = new PassThrough();
    child.stderr = new PassThrough();
    child.exitCode = null;
    child.unref = () => {};
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

test("explicit owning close releases local resources even when native exit is unobserved", async () => {
  const child = new EventEmitter() as unknown as ChildProcessWithoutNullStreams;
  Object.assign(child, { stdin: new PassThrough(), stdout: new PassThrough(), stderr: new PassThrough(), exitCode: null });
  let released = 0;
  child.kill = () => false;
  child.unref = () => { released++; };
  const transport = new ChildProcessCaptureTransport(child, 10);
  const closing = transport.close();
  assert.equal(transport.close(), closing, "concurrent close shares the same outcome");
  assert.equal(await closing, "kill-requested-after-timeout", "local cleanup never invents native exit");
  assert.equal(released, 1);
  assert.ok(child.stdin.destroyed && child.stdout.destroyed && child.stderr.destroyed);
  assert.equal(child.listenerCount("exit"), 0);
  await assert.rejects(transport.request({ channel: "capture", protocolVersion: 2, clientId: "a".repeat(32), rpcId: "b".repeat(32), method: "handshake" }), /closed/);
  assert.equal(await transport.close(), "kill-requested-after-timeout");
  assert.equal(released, 1);
});

test("owning close lets a one-shot caller exit while finite inherited-pipe work continues", async () => {
  const root = await mkdtemp(join(tmpdir(), "capture-close-"));
  try {
    for (const mode of ["caller-eof", "caller-already"]) {
      const marker = join(root, mode);
      const caller = spawnSync(process.execPath, ["--experimental-strip-types",
        fileURLToPath(new URL("./capture/close-caller.ts", import.meta.url)), mode, marker],
      { encoding: "utf8", timeout: 5000 });
      await writeFile(marker + ".release", "release finite fixture", { flag: "wx", mode: 0o600 });
      // Reconcile the test-owned finite descendant even if the regression fails.
      let completion: { finished: boolean } | undefined;
      const deadline = Date.now() + 2000;
      while (Date.now() < deadline) {
        try { completion = JSON.parse(await readFile(marker, "utf8")); break; }
        catch (error) { if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error; }
        await new Promise(done => setTimeout(done, 20));
      }
      assert.equal(completion?.finished, true, "finite fixture work survives local close and finishes");
      assert.equal(caller.error, undefined, `one-shot caller must finish without exit/unref/kill: ${caller.stderr}`);
      assert.equal(caller.status, 0);
      assert.deepEqual(JSON.parse(caller.stdout), { outcome: mode === "caller-eof" ? "eof-exited" : "already-exited" });
    }
  } finally { await rm(root, { recursive: true, force: true }); }
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
  const start = { ...authority, bundleId: "take", bundlePath: "/captures/take.apparent", source, options: { audio: "none" as const, localOnly: true as const }, invocation: { kind: "explicit" as const, invocationId: "a".repeat(32), observedAtMs: 0 } };
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

// The fixture is emitted by the actual Rust owner/journal/seal test. No native
// capture is simulated and a missing executable proof is an explicit skip.
test("Rust original ink seal round-trips through the public SDK reader", { skip: !process.env.APPARENT_TEST_INK_READER_FIXTURE }, async () => {
  const fixture = JSON.parse(await readFile(process.env.APPARENT_TEST_INK_READER_FIXTURE!, "utf8"));
  const bytes = Buffer.from(fixture.bytes, "utf8");
  const fact = JSON.parse(fixture.bytes);
  const materials = {
    payload: {
      identity: fact.identity,
      clock: { durationMs: 100 },
      files: [fixture.seal],
      collection: { acceptedStrokes: "collected" },
      facts: { acceptedStrokes: { file: fixture.seal.relativePath, schema: fact.schema } },
    },
  } as unknown as MaterialsDocument;
  const result = validateFactFile(fixture.seal, bytes, materials, "acceptedStrokes");
  assert.deepEqual(result, fact);
  assert.equal(fact.strokes.length, 1);
  assert.deepEqual(fact.strokes[0].rgb, [12, 34, 56]);
  const tampered = Buffer.from(bytes); tampered[tampered.length - 2] ^= 1;
  assert.throws(() => validateFactFile(fixture.seal, tampered, materials, "acceptedStrokes"));
});

// Generated executable fixture proves only exact child launch/environment input.
// It does not open a model, runtime, microphone or native capture.
test("speech launch requires explicit local input and clears inherited model approval", async () => {
  const root = await mkdtemp(join(tmpdir(), "engine-speech-launch-"));
  const executable = join(root, "fixture-engine");
  const output = join(root, "environment.json");
  const before = { model: process.env.APPARENT_CAPTURE_SPEECH_MODEL_FILE_V1, profile: process.env.APPARENT_CAPTURE_SPEECH_PROFILE_V1, output: process.env.APPARENT_TEST_ENV_OUTPUT };
  try {
    await writeFile(executable, '#!' + process.execPath + '\nconst fs=require("node:fs");fs.writeFileSync(process.env.APPARENT_TEST_ENV_OUTPUT,JSON.stringify({model:process.env.APPARENT_CAPTURE_SPEECH_MODEL_FILE_V1,profile:process.env.APPARENT_CAPTURE_SPEECH_PROFILE_V1}),{flag:"w"});\n', { mode: 0o755 });
    process.env.APPARENT_TEST_ENV_OUTPUT = output;
    process.env.APPARENT_CAPTURE_SPEECH_MODEL_FILE_V1 = "/unapproved/ambient-model";
    process.env.APPARENT_CAPTURE_SPEECH_PROFILE_V1 = "parakeet-unified-en-0.6b-Q8_0";
    await launchCaptureEngine(executable, join(root, "runtime"), 5000).close();
    assert.deepEqual(JSON.parse(await readFile(output, "utf8")), {});
    const localSpeech = { profile: "parakeet-unified-en-0.6b-Q8_0" as const, modelFile: join(root, "explicit-model.gguf") };
    await launchCaptureEngine(executable, join(root, "runtime"), 5000, { localSpeech }).close();
    assert.deepEqual(JSON.parse(await readFile(output, "utf8")), { model: localSpeech.modelFile, profile: localSpeech.profile });
    assert.throws(() => launchCaptureEngine(executable, join(root, "runtime"), 5000, { localSpeech: { ...localSpeech, modelFile: "relative.gguf" } }));
  } finally {
    for (const [key, value] of [["APPARENT_CAPTURE_SPEECH_MODEL_FILE_V1",before.model],["APPARENT_CAPTURE_SPEECH_PROFILE_V1",before.profile],["APPARENT_TEST_ENV_OUTPUT",before.output]] as const) { if (value === undefined) delete process.env[key]; else process.env[key]=value; }
    await rm(root, { recursive: true, force: true });
  }
});
