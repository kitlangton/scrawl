import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { chmod, lstat, mkdtemp, readFile, realpath, rm, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";
import {
  Media2Client,
  Media2RpcError,
  Media2TransportError,
  preflightMedia2Dispatch,
  encodeContactSheetLayout,
} from "@apparent/engine-sdk/media2";
import type {
  Media2JobInput,
  Media2RpcRequest,
  Media2Transport,
  Derivative2DocumentInput,
} from "@apparent/engine-sdk/media2";
import type { MaterialsDocument } from "@apparent/engine-sdk/capture";
import { RecordMedia2Job } from "../../dist/backend/processing/media2-consumer.js";

const key = { captureId: "a".repeat(32), materialsId: "b".repeat(32), revision: "c".repeat(64) };
const leaseId = "d".repeat(32);
const token = "e".repeat(64);
const materials = (): MaterialsDocument => ({
  manifest: {
    schema: "apparent.capture-materials/1", key, root: "/private/media2-fixture-materials", leaseId,
    payloadFile: { relativePath: "materials.json", identity: { device: "1", inode: "2", size: 1, modifiedNs: "3" }, sha256: "f".repeat(64) },
  },
  payload: {
    schema: "apparent.capture-materials-payload/1", identity: { captureId: key.captureId, materialsId: key.materialsId },
    bundleId: "synthetic-media2-materials", root: "/private/media2-fixture-materials", leaseId,
    clock: { epoch: "capture-admission-monotonic", units: "integer-milliseconds", durationMs: 4000, videoOffsetMs: 125 },
    confirmedSource: { id: "synthetic-display", generation: "1".repeat(32), confirmedAtMs: 0, kind: "display", displayId: 1,
      rectangle: { x: 0, y: 0, width: 640, height: 360, scale: 1 }, confirmation: { kind: "explicit-display", id: "2".repeat(32) } },
    invocation: { kind: "explicit", invocationId: "3".repeat(32), observedAtMs: 0 }, files: [],
    video: { file: "capture.mp4", width: 640, height: 360, durationMs: 4000 }, facts: {},
    collection: { context: "unavailable", interactions: "unavailable", candidates: "unavailable", transcript: "unavailable", acceptedStrokes: "unavailable" },
    warnings: [], safety: { nativeRemoteDelivery: "forbidden", safeForProcessing: true },
  },
});

function input(root: string): Media2JobInput {
  return {
    ownerToken: token,
    materials: materials(),
    request: {
      schema: "apparent.retained-frame-request/1", materials: key, leaseId, jobId: "4".repeat(32),
      outputDirectory: join(root, "derived-output"), requestedBundleMs: [1000, 1000, 1800],
      limits: { maxFrames: 3, maxEdge: 320, maxTotalPixels: 64_000_000, deadlineMs: 10_000 },
      contactSheet: { schema: "apparent.contact-sheet-layout/1", width: 960, height: 180, backgroundRgb: [0, 0, 0],
        fit: "stretch", sampling: "legacy-bilinear", clipping: "reject",
        tiles: [
          { requestIndex: 1, x: 0, y: 0, width: 320, height: 180 },
          { requestIndex: 2, x: 320, y: 0, width: 320, height: 180 },
          { requestIndex: 3, x: 640, y: 0, width: 320, height: 180 },
        ] },
    },
  };
}

async function privateRoot(): Promise<string> {
  const path = await realpath(await mkdtemp(join(tmpdir(), "record-media2-job-")));
  await chmod(path, 0o700);
  return path;
}

test("media2 job journals immutable duplicate slots before dispatch and reconciles ambiguous work only by inspect", async () => {
  const root = await privateRoot();
  try {
    const original = input(root);
    const originalRequest = structuredClone(original.request);
    const originalMaterials = structuredClone(original.materials);
    const journal = join(root, "media2-job.json");
    const calls: Array<{ method: string; input: Media2JobInput }> = [];
    let inspections = 0;
    const seam = {
      async derive(value: Media2JobInput) {
        calls.push({ method: "derive", input: structuredClone(value) });
        throw new Media2TransportError();
      },
      async inspect(value: Media2JobInput) {
        calls.push({ method: "inspect", input: structuredClone(value) });
        if (++inspections === 1) throw new Media2TransportError();
        const prepared = preflightMedia2Dispatch(value);
        return { jobId: prepared.input.request.jobId, requestSha256: prepared.prepared.sha256, state: "running" as const };
      },
      async cancel(value: Media2JobInput) {
        calls.push({ method: "cancel", input: structuredClone(value) });
        const prepared = preflightMedia2Dispatch(value);
        return { jobId: prepared.input.request.jobId, requestSha256: prepared.prepared.sha256, state: "cancelled" as const };
      },
    } as unknown as Media2Client; // Offline seam only; no handshake or native receipt is asserted.

    const job = await RecordMedia2Job.create(seam, journal, original);
    const before = JSON.parse(await readFile(journal, "utf8"));
    assert.equal((await stat(journal)).mode & 0o777, 0o600);
    assert.equal(before.state, "ready");
    assert.equal(before.dispatchAttempted, false);
    assert.deepEqual(before.request.requestedBundleMs, [1000, 1000, 1800]);
    assert.deepEqual(before.request.contactSheet, originalRequest.contactSheet);
    assert.deepEqual(before.materials, originalMaterials);
    assert.equal((await readFile(journal, "utf8")).includes(token), false);

    original.request.requestedBundleMs[0] = 2000;
    original.materials.payload.video.width = 1;
    await assert.rejects(RecordMedia2Job.reopen(seam, journal, original), /does not match its original request and materials/);
    await assert.rejects(job.derive("invalid-token"), { name: "Media2PreflightError" });
    assert.equal(job.state, "ready", "preflight rejection has no dispatch or persisted state transition");
    assert.equal(JSON.parse(await readFile(journal, "utf8")).dispatchAttempted, false);

    await assert.rejects(job.derive(token), Media2TransportError);
    assert.equal(job.state, "unknown");
    assert.equal(calls.filter(call => call.method === "derive").length, 1);
    assert.deepEqual(calls[0]!.input.request, originalRequest);
    assert.equal(calls[0]!.input.ownerToken, token);
    const submitted = JSON.parse(await readFile(journal, "utf8"));
    assert.equal(submitted.dispatchAttempted, true);
    assert.equal(submitted.state, "unknown");
    assert.equal(submitted.requestSha256, job.requestSha256);
    assert.equal(JSON.stringify(submitted).includes(token), false);
    await assert.rejects(job.derive(token), /requires inspection/);
    assert.equal(calls.filter(call => call.method === "derive").length, 1, "derive is never retried automatically");

    const changedRequest = structuredClone(originalRequest);
    changedRequest.contactSheet!.backgroundRgb = [1, 0, 0];
    await assert.rejects(RecordMedia2Job.reopen(seam, journal, { ownerToken: token, request: changedRequest, materials: originalMaterials }), /does not match/);
    const changedMaterials = structuredClone(originalMaterials);
    changedMaterials.payload.video.width++;
    await assert.rejects(RecordMedia2Job.reopen(seam, journal, { ownerToken: token, request: originalRequest, materials: changedMaterials }), /does not match/);

    const reopened = await RecordMedia2Job.reopen(seam, journal, { ownerToken: token, request: originalRequest, materials: originalMaterials });
    assert.equal(reopened.state, "unknown");
    await assert.rejects(reopened.inspect(token), /outcome is unknown/);
    assert.equal(reopened.state, "unknown");
    assert.equal(calls.some(call => call.method === "cancel"), false, "ambiguous recovery never cancels implicitly");
    const status = await reopened.inspect(token);
    assert.equal(status.state, "running");
    assert.deepEqual(calls.filter(call => call.method === "inspect").map(call => call.input.request), [originalRequest, originalRequest]);
    assert.equal(calls.at(-1)!.input.ownerToken, token);
    assert.equal(await reopened.cancel(token).then(value => value.state), "cancelled");
    assert.equal(reopened.state, "cancelled");
    assert.equal(calls.filter(call => call.method === "cancel").length, 1, "cancel occurs only after explicit call");
    await reopened.cleanupJournal();
    await assert.rejects(lstat(journal), { code: "ENOENT" });
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("packed Media2Client unavailable capability remains undispatched and Record never closes its injected client", async () => {
  const root = await privateRoot();
  try {
    const requests: Media2RpcRequest[] = [];
    let closeCalls = 0;
    const transport: Media2Transport = {
      async request(request) {
        requests.push(structuredClone(request));
        return { channel: "media", protocolVersion: 2, rpcId: request.rpcId, result: {
          channel: "media", protocolVersion: 2, implementation: "test-double",
          capabilities: { retainedFrames: false, contactSheet: false, nativeJobAuthority: false },
        } };
      },
      async close() { closeCalls++; return { closed: true } as never; },
    };
    const client = new Media2Client(transport, { allowTestDouble: true });
    await client.handshake();
    const journal = join(root, "unavailable-job.json");
    const job = await RecordMedia2Job.create(client, journal, input(root));
    await assert.rejects(job.derive(token), (error: unknown) => error instanceof Media2RpcError && error.code === "unavailable" && error.dispatched === false);
    assert.equal(job.state, "unavailable");
    assert.equal(JSON.parse(await readFile(journal, "utf8")).dispatchAttempted, false);
    assert.deepEqual(requests.map(request => request.method), ["handshake"], "unavailable derive sends no job RPC");
    assert.equal(closeCalls, 0, "the Record job does not close its injected SDK client");
    await job.cleanupJournal();
    assert.equal(closeCalls, 0);
    await client.close(); // The caller retains client lifecycle ownership.
    assert.equal(closeCalls, 1);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("invalid media2 request is rejected before a private journal or client operation exists", async () => {
  const root = await privateRoot();
  try {
    const bad = input(root);
    bad.request.requestedBundleMs[2] = 5000;
    const journal = join(root, "invalid-job.json");
    const client = {} as Media2Client;
    await assert.rejects(RecordMedia2Job.create(client, journal, bad), { name: "Media2PreflightError" });
    await assert.rejects(lstat(journal), { code: "ENOENT" });
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("a failed terminal inspection preserves the durable terminal result across reopen", async () => {
  const root = await privateRoot();
  try {
    const original = input(root);
    const prepared = preflightMedia2Dispatch(original);
    const client = {
      async derive() { return { jobId: original.request.jobId, requestSha256: prepared.prepared.sha256, state: "failed", error: { code: "unavailable" } }; },
      async inspect() { throw new Media2TransportError(); },
    } as unknown as Media2Client;
    const journal = join(root, "terminal.json");
    const job = await RecordMedia2Job.create(client, journal, original);
    await job.derive(token);
    await assert.rejects(job.inspect(token), /outcome is unknown/);
    assert.equal(job.state, "failed");
    const reopened = await RecordMedia2Job.reopen(client, journal, original);
    assert.equal(reopened.state, "failed");
    await reopened.cleanupJournal();
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("invalid operation timeouts fail preflight before dispatch state or client effects", async () => {
  const root = await privateRoot();
  try {
    let calls = 0;
    const client = { async derive() { calls++; throw new RangeError("invalid timeout"); } } as unknown as Media2Client;
    const journal = join(root, "invalid-timeout.json");
    const job = await RecordMedia2Job.create(client, journal, input(root));
    await assert.rejects(job.derive(token, 0), { name: "Media2PreflightError" });
    assert.equal(job.state, "ready");
    assert.equal(calls, 0);
    assert.equal(JSON.parse(await readFile(journal, "utf8")).dispatchAttempted, false);
    await job.cleanupJournal();
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("offline validated derivative completion survives reopen, caller mutation and failed or conflicting inspection", async () => {
  const root = await privateRoot();
  try {
    const original = input(root);
    const prepared = preflightMedia2Dispatch(original);
    const hash = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
    const seal = (path: string) => ({ relativePath: path, identity: { device: "1", inode: path, size: 11, modifiedNs: "3" }, sha256: "f".repeat(64) });
    // Synthetic metadata for offline validation only: no image or native job authority is produced.
    const layoutBytes = encodeContactSheetLayout(original.request.contactSheet!);
    const payload = {
      schema: "apparent.media-derivative-payload/2", materials: key, leaseId,
      jobId: original.request.jobId, receiptId: "8".repeat(32), root: original.request.outputDirectory,
      requestSha256: prepared.prepared.sha256,
      frames: original.request.requestedBundleMs.map((time, index) => ({ requestIndex: index + 1,
        requestedBundleMs: time, effectiveVideoMs: time - 125, actualVideoMs: time - 125,
        clampedToStart: false, clampedToEnd: false, width: 320, height: 180,
        encoding: "png", byteLength: 11, file: seal(`frame-${index + 1}.png`) })),
      contactSheet: { encoding: "png", byteLength: 11, file: seal("sheet.png"), width: 960, height: 180,
        layoutSha256: hash(layoutBytes), layout: original.request.contactSheet },
    };
    const bytes = new TextEncoder().encode(JSON.stringify(payload));
    const derivative: Derivative2DocumentInput = { request: original.request, bytes, layoutBytes,
      envelope: { schema: "apparent.media-derivative/2", materials: key, leaseId,
        jobId: original.request.jobId, receiptId: payload.receiptId, root: payload.root,
        requestSha256: payload.requestSha256, payload,
        receiptFile: { relativePath: "receipt.json", identity: { device: "1", inode: "receipt", size: bytes.length, modifiedNs: "3" }, sha256: hash(bytes) } } };
    const complete = { state: "complete" as const, jobId: original.request.jobId, requestSha256: prepared.prepared.sha256, derivative };
    let inspection = 0;
    const client = {
      async derive() { return structuredClone(complete); },
      async inspect() {
        if (++inspection === 1) throw new Media2TransportError();
        if (inspection === 2) return { ...complete, jobId: "0".repeat(32) };
        return { state: "running", jobId: complete.jobId, requestSha256: complete.requestSha256 };
      },
    } as unknown as Media2Client;
    const journal = join(root, "complete.json");
    const job = await RecordMedia2Job.create(client, journal, original);
    const status = await job.derive(token);
    assert.equal(status.state, "complete");
    assert.equal(job.state, "complete");
    const returned = job.derivative!;
    returned.bytes[0] = 0;
    assert.notEqual(job.derivative!.bytes[0], 0, "callers cannot mutate the persisted completion");
    await assert.rejects(job.inspect(token), /outcome is unknown/);
    await assert.rejects(job.inspect(token), /mismatched original job binding/);
    await assert.rejects(job.inspect(token), /conflicts with the durable terminal result/);
    assert.equal(job.state, "complete");
    const reopened = await RecordMedia2Job.reopen(client, journal, original);
    assert.equal(reopened.state, "complete");
    assert.deepEqual(reopened.derivative!.bytes, bytes);
    assert.deepEqual(Buffer.from(reopened.derivative!.layoutBytes!), Buffer.from(layoutBytes));
    await reopened.cleanupJournal();
  } finally { await rm(root, { recursive: true, force: true }); }
});
