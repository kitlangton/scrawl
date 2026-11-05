import test from "node:test";
import assert from "node:assert/strict";
import {
  createMediaJobId,
  encodeMediaRequest,
  MediaClient,
  MediaChildProcessTransport,
  validateMediaRequest,
  validateMediaResponse,
  type DecodeRequest,
  type InspectRequest,
  type MediaFailure,
} from "../src/media.ts";
import { ReferenceMediaPeer } from "../src/media-testing.ts";
import { spawn } from "node:child_process";
import { EventEmitter } from "node:events";
import { fileURLToPath } from "node:url";

const source = {
  bundleId: "bundle",
  bundlePath: "/tmp/bundle.apparent",
  videoPath: "/tmp/bundle.apparent/recording.mp4",
  videoOffsetMs: 250,
  identity: { device: "1", inode: "2", size: 1000, modifiedNs: "123456789" },
};
const request = (
  times = [1250, 0, 1250],
  jobId = "3".repeat(32),
): DecodeRequest => ({
  channel: "media",
  protocolVersion: 1,
  clientId: "1".repeat(32),
  rpcId: "2".repeat(32),
  method: "decode",
  jobId,
  source,
  requestedBundleMs: times,
  outputDirectory: "/tmp/private-job",
  limits: {
    maxFrames: 48,
    maxEdge: 2560,
    maxTotalPixels: 64_000_000,
    deadlineMs: 30_000,
  },
});
const input = (times = [1250, 0, 1250], jobId = createMediaJobId()) => ({
  jobId,
  source,
  requestedBundleMs: times,
  outputDirectory: "/tmp/private-job",
  limits: request().limits,
});

test("end clamping preserves the offset-mapped seek and reports decoder position separately", async () => {
  const context = request([5250]);
  const client = new MediaClient(new ReferenceMediaPeer(), { allowTestDouble: true });
  try {
    await client.handshake();
    const result = await client.decode(context);
    const clamped = { ...result, frames: [{ ...result.frames[0], effectiveVideoMs: 5000, actualVideoMs: 1999, clampedToEnd: true }] };
    const envelope = { channel: "media", protocolVersion: 1, rpcId: context.rpcId, result: clamped };
    assert.equal(validateMediaResponse(envelope, context.rpcId, context).result, clamped);
    const inspect: InspectRequest = { channel: "media", protocolVersion: 1, clientId: context.clientId, rpcId: context.rpcId, method: "inspect", jobId: context.jobId };
    assert.equal(validateMediaResponse(envelope, context.rpcId, inspect, context).result, clamped);
    const rewrittenSeek = { ...envelope, result: { ...clamped, frames: [{ ...clamped.frames[0], effectiveVideoMs: 1999 }] } };
    assert.throws(() => validateMediaResponse(rewrittenSeek, context.rpcId, context));
    assert.throws(() => validateMediaResponse(rewrittenSeek, context.rpcId, inspect, context));
  } finally { await client.close(); }
});

test("inspection sanitizes terminal errors and validates completed frames against retained input", async () => {
  const context = request([0]);
  const inspect: InspectRequest = { channel: "media", protocolVersion: 1, clientId: context.clientId, rpcId: context.rpcId, method: "inspect", jobId: context.jobId };
  const failure = { jobId: context.jobId, state: "failed", error: { code: "decode_failed", message: "private /captures/customer/movie.mp4" }, completedCount: 0 };
  const envelope = { channel: "media", protocolVersion: 1, rpcId: inspect.rpcId, result: failure };
  const normalized = validateMediaResponse(envelope, inspect.rpcId, inspect).result as MediaFailure;
  assert.equal(normalized.error.message, "Engine media job failed (decode_failed)");
  assert.throws(() => validateMediaResponse({ ...envelope, result: { ...failure, error: { code: "cancelled", message: "wrong state" } } }, inspect.rpcId, inspect), /terminal/);

  const peer = new ReferenceMediaPeer();
  const client = new MediaClient(peer, { allowTestDouble: true });
  await client.handshake();
  const completed = await client.decode(context);
  const completeEnvelope = { ...envelope, result: completed };
  assert.throws(() => validateMediaResponse(completeEnvelope, inspect.rpcId, inspect), /decode context/);
  assert.equal(validateMediaResponse(completeEnvelope, inspect.rpcId, inspect, context).result, completed);
  assert.throws(() => validateMediaResponse(completeEnvelope, inspect.rpcId, inspect, { ...context, jobId: "9".repeat(32) }), /matching decode context/);
  assert.throws(() => validateMediaResponse({ ...completeEnvelope, result: { ...completed, frames: [{ ...completed.frames[0], width: 4096, height: 4096 }] } }, inspect.rpcId, inspect, { ...context, limits: { ...context.limits, maxTotalPixels: 1 } }), /bounds/);
  await client.close();
});

test("synthetic success honors low pixel budgets and impossible budgets fail without inventory", async () => {
  const client = new MediaClient(new ReferenceMediaPeer(), { allowTestDouble: true });
  await client.handshake();
  const onePixel = { ...input([0]), limits: { ...request().limits, maxEdge: 1, maxTotalPixels: 1 } };
  const result = await client.decode(onePixel);
  assert.deepEqual(result.frames.map(f => [f.width, f.height]), [[1, 1]]);
  const impossible = { ...input([0, 1]), limits: { ...request().limits, maxTotalPixels: 1 } };
  await assert.rejects(client.decode(impossible), /limits_exceeded/);
  assert.equal((await client.inspect(impossible)).state, "failed");
  await client.close();
});

test("media marker and strict request bounds keep capture v1 separate", () => {
  assert.throws(
    () => validateMediaRequest({ ...request(), channel: "capture" } as never),
    /channel/,
  );
  assert.throws(
    () => validateMediaRequest({ ...request(), protocolVersion: 2 }),
    /version/,
  );
  assert.throws(
    () =>
      validateMediaRequest({
        ...request(),
        source: { ...source, videoOffsetMs: -1 },
      }),
    /source/,
  );
  assert.throws(
    () => validateMediaRequest({ ...request(), outputDirectory: "relative" }),
    /decode/,
  );
  assert.throws(
    () =>
      validateMediaRequest({
        ...request(),
        limits: { ...request().limits, maxFrames: 0 },
      }),
    /limits/,
  );
  assert.throws(
    () =>
      validateMediaRequest({
        ...request(),
        source: { ...source, identity: { ...source.identity, inode: "bad" } },
      }),
    /identity/,
  );
  for (const bundleId of [
    ".",
    "..",
    "x/../y",
    "x\\y",
    "a".repeat(256),
    "é".repeat(128),
  ]) {
    assert.throws(
      () =>
        validateMediaRequest({ ...request(), source: { ...source, bundleId } }),
      /source/,
    );
  }
  assert.throws(
    () =>
      validateMediaRequest({
        ...request(),
        source: { ...source, videoPath: "/tmp/outside.mp4" },
      }),
    /source path/,
  );
  assert.throws(
    () => encodeMediaRequest({ ...request(), unexpected: true } as never),
    /fields/,
  );
});

test("test peer is opt-in and preserves requested order, duplicates, and saturating offset mapping", async () => {
  const peer = new ReferenceMediaPeer();
  await assert.rejects(
    new MediaClient(peer).handshake(),
    /explicit test-only opt-in/,
  );
  const client = new MediaClient(peer, { allowTestDouble: true });
  await client.handshake();
  const job = input([1250, 0, 250, 1250]);
  const result = await client.decode(job);
  assert.deepEqual(
    result.frames.map((f) => [
      f.requestIndex,
      f.requestedBundleMs,
      f.effectiveVideoMs,
      f.clampedToStart,
    ]),
    [
      [1, 1250, 1000, false],
      [2, 0, 0, true],
      [3, 250, 0, false],
      [4, 1250, 1000, false],
    ],
  );
  assert.equal(new Set(result.frames.map((f) => f.path)).size, 4);
  assert.equal(peer.requests.filter((r) => r.method === "decode").length, 1);
  await assert.rejects(client.decode(job), /busy/);
  const reorderedSource = {
    identity: {
      modifiedNs: source.identity.modifiedNs,
      inode: source.identity.inode,
      size: source.identity.size,
      device: source.identity.device,
    },
    videoOffsetMs: source.videoOffsetMs,
    videoPath: source.videoPath,
    bundlePath: source.bundlePath,
    bundleId: source.bundleId,
  };
  await assert.rejects(
    client.decode({ ...job, source: reorderedSource }),
    /busy/,
  );
  await assert.rejects(
    client.decode({ ...job, outputDirectory: "/tmp/other" }),
    /identity_conflict/,
  );
  await client.close();
});

test("request snapshot protects against mutation and failure is terminal without partial inventory", async () => {
  const peer = new ReferenceMediaPeer("partial-decode-failure");
  const client = new MediaClient(peer, { allowTestDouble: true });
  await client.handshake();
  const job = input([0]);
  const pending = client.decode(job);
  job.requestedBundleMs[0] = 999;
  await assert.rejects(pending, /decode_failed/);
  const decode = peer.requests.find(
    (r) => r.method === "decode",
  ) as DecodeRequest;
  assert.deepEqual(decode.requestedBundleMs, [0]);
  const status = await client.inspect(input([0], job.jobId));
  assert.equal(status.state, "failed");
  assert.equal(await client.cancel(job.jobId), "failed");
  await client.close();
});

test("result validator rejects incomplete, aliased, reordered, and oversized inventory", () => {
  const req = request([1250, 0]);
  const frames = [1250, 0].map((requestedBundleMs, index) => ({
    requestIndex: index + 1,
    requestedBundleMs,
    effectiveVideoMs: Math.max(0, requestedBundleMs - 250),
    clampedToStart: requestedBundleMs < 250,
    clampedToEnd: false,
    path: `/tmp/private-job/frame-${String(index + 1).padStart(3, "0")}.png`,
    width: 640,
    height: 480,
    byteLength: 128,
    sha256: "a".repeat(64),
  }));
  const base = {
    channel: "media",
    protocolVersion: 1,
    rpcId: req.rpcId,
    result: { jobId: req.jobId, state: "complete", frames },
  };
  assert.equal(validateMediaResponse(base, req.rpcId, req).rpcId, req.rpcId);
  assert.throws(
    () =>
      validateMediaResponse(
        { ...base, result: { ...base.result, frames: frames.slice(0, 1) } },
        req.rpcId,
        req,
      ),
    /decode result/,
  );
  assert.throws(
    () =>
      validateMediaResponse(
        { ...base, result: { ...base.result, frames: [frames[1], frames[0]] } },
        req.rpcId,
        req,
      ),
    /reference/,
  );
  assert.throws(
    () =>
      validateMediaResponse(
        {
          ...base,
          result: {
            ...base.result,
            frames: [frames[0], { ...frames[1], path: frames[0].path }],
          },
        },
        req.rpcId,
        req,
      ),
    /reference/,
  );
  assert.throws(
    () =>
      validateMediaResponse(
        {
          ...base,
          result: {
            ...base.result,
            frames: frames.map((f) => ({ ...f, width: 4096, height: 4096 })),
          },
        },
        req.rpcId,
        req,
      ),
    /bounds/,
  );
  assert.throws(
    () =>
      validateMediaResponse(
        {
          ...base,
          result: {
            ...base.result,
            frames: frames.map((f, i) =>
              i ? f : { ...f, path: "/tmp/private-job-escape/frame-001.png" },
            ),
          },
        },
        req.rpcId,
        req,
      ),
    /escapes/,
  );
});

test("timeout leaves job inspectable and later completion is recoverable", async () => {
  const client = new MediaClient(new ReferenceMediaPeer("timeout"), {
    allowTestDouble: true,
  });
  await client.handshake();
  const job = input([0]);
  await assert.rejects(client.decode(job, 20), /timed out/);
  assert.equal((await client.inspect(job)).state, "decoding");
  await new Promise((resolve) => setTimeout(resolve, 120));
  assert.equal((await client.inspect(job)).state, "complete");
  await client.close();
});

test("cancel before completion settles decode and cancellation of complete preserves terminal state", async () => {
  const client = new MediaClient(new ReferenceMediaPeer("timeout"), {
    allowTestDouble: true,
  });
  await client.handshake();
  const job = input([0]);
  const pending = client.decode(job, 500);
  await new Promise((resolve) => setTimeout(resolve, 5));
  assert.equal(await client.cancel(job.jobId), "cancelled");
  await assert.rejects(pending, /cancelled/);
  assert.equal((await client.inspect(job)).state, "cancelled");
  await client.close();

  const done = new MediaClient(new ReferenceMediaPeer(), {
    allowTestDouble: true,
  });
  await done.handshake();
  const completed = input([0]);
  await done.decode(completed);
  assert.equal(await done.cancel(completed.jobId), "complete");
  assert.equal((await done.inspect(completed)).state, "complete");
  await done.close();
});

test("child transport rejects timed out RPCs, ignores late frames, and close settles waiters", async () => {
  const fixture = fileURLToPath(
    new URL("./media-child-fixture.ts", import.meta.url),
  );
  const transport = new MediaChildProcessTransport(
    spawn(
      process.execPath,
      ["--experimental-strip-types", fixture, "timeout"],
      { stdio: ["pipe", "pipe", "pipe"] },
    ),
  );
  const client = new MediaClient(transport, { allowTestDouble: true });
  await client.handshake();
  const job = input([0]);
  await assert.rejects(client.decode(job, 50), /timed out/);
  await new Promise((resolve) => setTimeout(resolve, 120));
  assert.equal((await client.inspect(job)).state, "complete");
  const outcome = await client.close();
  assert.ok(
    [
      "already-exited",
      "eof-exited",
      "term-requested-after-timeout",
      "kill-requested-after-timeout",
    ].includes(outcome),
  );
});

test("child transport enforces UTF-8, channel, version, fields, size, and CRLF policy", async () => {
  const fixture = fileURLToPath(
    new URL("./media-child-fixture.ts", import.meta.url),
  );
  const crlf = new MediaClient(
    new MediaChildProcessTransport(
      spawn(process.execPath, ["--experimental-strip-types", fixture, "crlf"], {
        stdio: ["pipe", "pipe", "pipe"],
      }),
    ),
    { allowTestDouble: true },
  );
  assert.equal((await crlf.handshake()).implementation, "test-double");
  await crlf.close();

  for (const [mode, expected] of [
    ["bad-utf8", /malformed media response/],
    ["oversized", /exceeds protocol limit/],
    ["wrong-channel", /incompatible media peer/],
    ["wrong-version", /incompatible media peer/],
    ["unknown-fields", /protocol fields/],
  ] as const) {
    const client = new MediaClient(
      new MediaChildProcessTransport(
        spawn(process.execPath, ["--experimental-strip-types", fixture, mode], {
          stdio: ["pipe", "pipe", "pipe"],
        }),
      ),
      { allowTestDouble: true },
    );
    await assert.rejects(client.handshake(), expected);
    await client.close();
  }
});

test("child close reports a kill request without claiming reaping and is idempotent", async () => {
  const fixture = fileURLToPath(
    new URL("./media-child-fixture.ts", import.meta.url),
  );
  const child = spawn(
    process.execPath,
    ["--experimental-strip-types", fixture, "hold"],
    { stdio: ["pipe", "pipe", "pipe"] },
  );
  await new Promise<void>((resolve) =>
    child.stderr.once("data", () => resolve()),
  );
  const transport = new MediaChildProcessTransport(child, 30);
  const first = transport.close();
  const second = transport.close();
  assert.equal(first, second);
  assert.equal(await first, "kill-requested-after-timeout");
});

test("closing settles an outstanding child RPC without waiting for its timeout", async () => {
  const fixture = fileURLToPath(
    new URL("./media-child-fixture.ts", import.meta.url),
  );
  const client = new MediaClient(
    new MediaChildProcessTransport(
      spawn(
        process.execPath,
        ["--experimental-strip-types", fixture, "timeout"],
        {
          stdio: ["pipe", "pipe", "pipe"],
        },
      ),
    ),
    { allowTestDouble: true },
  );
  await client.handshake();
  const pending = client.decode(input([0]), 30_000);
  const rejection = pending.catch((error: unknown) => error);
  assert.ok(["already-exited", "eof-exited"].includes(await client.close()));
  const error = await rejection;
  assert.ok(error instanceof Error && /transport closing/.test(error.message));
});

test("timed-out RPCs still count toward the bounded child write queue", async () => {
  const child = new EventEmitter() as EventEmitter & {
    stdin: EventEmitter & {
      writableLength: number;
      write: (line: string, callback: (error?: Error | null) => void) => void;
      end: () => void;
    };
    stdout: EventEmitter;
    stderr: EventEmitter;
    exitCode: number | null;
    signalCode: NodeJS.Signals | null;
    kill: (signal?: NodeJS.Signals) => boolean;
  };
  child.stdin = Object.assign(new EventEmitter(), {
    writableLength: 0,
    write: (_line: string, _callback: (error?: Error | null) => void) => true,
    end: () => {
      child.exitCode = 0;
      child.emit("exit", 0, null);
    },
  });
  child.stdout = new EventEmitter();
  child.stderr = new EventEmitter();
  child.exitCode = null;
  child.signalCode = null;
  child.kill = () => true;
  const transport = new MediaChildProcessTransport(child as never, 30);
  child.stderr.emit("error", new Error("synthetic stderr stream failure"));
  const pending: Promise<unknown>[] = [];
  const largeSource = {
    ...source,
    identity: { ...source.identity, device: "1".repeat(36_000) },
  };
  for (let index = 0; index < 32; index++) {
    const rpcId = index.toString(16).padStart(32, "0");
    pending.push(
      transport
        .request({ ...request([0]), rpcId, source: largeSource }, 1)
        .catch((error: unknown) => error),
    );
  }
  await new Promise((resolve) => setTimeout(resolve, 5));
  const results = await Promise.all(pending);
  assert.ok(
    results.some(
      (result) =>
        result instanceof Error && /write buffer limit/.test(result.message),
    ),
  );
  await transport.close();
});
