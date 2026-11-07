import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import { chmodSync, copyFileSync, mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync, rmdirSync, statSync, symlinkSync, unlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import test from "node:test";
import { CaptureClient, capture2MaterialsRoot, launchCaptureEngine, type CaptureStatus } from "../sdk/src/capture.ts";
import { validateMaterialsDocument } from "../sdk/src/capture-documents/index.ts";
import { Media2Client, Media2RpcError, Media2TransportError, launchMedia2Worker, type Media2JobInput } from "../sdk/src/media2/index.ts";
import { launchCaptureRecoveryPeer } from "../sdk/src/recovery/index.ts";

const id = () => randomBytes(16).toString("hex");
const token = () => randomBytes(32).toString("hex");
const binary = process.env.APPARENT_ENGINE_CAPTURE_BIN;
const xvfbCommand = process.env.APPARENT_XVFB ?? "Xvfb";
const xsetrootCommand = process.env.APPARENT_XSETROOT ?? "xsetroot";
const ffprobeCommand = process.env.APPARENT_FFPROBE ?? "ffprobe";
const ffmpegCommand = process.env.APPARENT_FFMPEG ?? "ffmpeg";

// Ask Xvfb to reserve its own display, then probe that exact endpoint. A free
// lock-file observation and a fixed sleep do not prove our server is ready.
async function readyXvfb(command = xvfbCommand, prefix: string[] = []) {
  let diagnostics = "";
  let announcement = "";
  let launchError: Error | undefined;
  let announcementEnded = false;
  let childClosed = false;
  // A real desktop keeps clients connected. Keep this synthetic scene across
  // short probes instead of resetting the X server after each last disconnect.
  const server = spawn(command, [...prefix, "-displayfd", "3", "-screen", "0", "1366x768x24", "-nolisten", "tcp", "-noreset"], { stdio: ["ignore", "ignore", "pipe", "pipe"], detached: true });
  server.on("error", (error) => { launchError = error; });
  server.once("close", () => { childClosed = true; });
  server.stdio[3]?.once("end", () => { announcementEnded = true; });
  server.stderr?.on("data", (chunk: Buffer) => { diagnostics = (diagnostics + chunk.toString()).slice(-8192); });
  server.stdio[3]?.on("data", (chunk: Buffer) => { announcement = (announcement + chunk.toString()).slice(0, 128); });
  const deadline = Date.now() + 10_000;
  try {
    while (Date.now() < deadline) {
      if (launchError) throw launchError;
      if (server.exitCode !== null || server.signalCode !== null) throw new Error(`server exited: ${server.exitCode}/${server.signalCode}`);
      const match = /^(\d+)\n/.exec(announcement);
      if (match) {
        const display = `:${match[1]}`;
        try {
          execFileSync(xsetrootCommand, ["-solid", "#cc2211"], { env: { ...process.env, DISPLAY: display }, stdio: "pipe", timeout: 500 });
          return { server, display, diagnostics: () => diagnostics };
        } catch (error) { diagnostics = (diagnostics + `\nendpoint probe: ${String(error)}`).slice(-8192); }
      }
      await delay(50);
    }
    throw new Error("endpoint readiness deadline exceeded");
  } catch (error) {
    if (server.exitCode === null && server.signalCode === null && server.pid) {
      // The delayed fixture can still be a shell with an owned sleep child.
      try { process.kill(-server.pid, "SIGTERM"); } catch (signalError) { if ((signalError as NodeJS.ErrnoException).code !== "ESRCH") throw signalError; }
      await Promise.race([new Promise<void>((done) => server.once("close", () => done())), delay(1_000)]);
    }
    throw new Error(`private Xvfb startup failed: ${String(error)}; pid=${server.pid ?? "none"} exit=${server.exitCode} signal=${server.signalCode} closed=${childClosed} displayfdEnded=${announcementEnded} announcement=${JSON.stringify(announcement)}; ${diagnostics.trim() || "no Xvfb stderr"}`);
  }
}

test("private Xvfb readiness waits beyond the former fixed delay and diagnoses launch failure", { skip: !binary }, async () => {
  const fixture = mkdtempSync(join(tmpdir(), "capture-xvfb-readiness-"));
  const wrapper = join(fixture, "delayed-xvfb.sh");
  writeFileSync(wrapper, '#!/bin/sh\nprintf "delayed fixture: waiting\\n" >&2\nsleep 1\nprintf "delayed fixture: executing Xvfb\\n" >&2\nexec "$@"\n', { mode: 0o700 });
  let ready: Awaited<ReturnType<typeof readyXvfb>> | undefined;
  try {
    const began = Date.now();
    ready = await readyXvfb(wrapper, [xvfbCommand]);
    assert.ok(Date.now() - began >= 750, "the fixture actually starts later than the former 500ms sleep");
    execFileSync(xsetrootCommand, ["-solid", "#228844"], { env: { ...process.env, DISPLAY: ready.display }, timeout: 500 });
    await assert.rejects(readyXvfb(join(fixture, "missing-xvfb")), /private Xvfb startup failed:.*ENOENT/);
  } finally {
    if (ready) {
      ready.server.kill("SIGTERM");
      await Promise.race([new Promise<void>((done) => ready!.server.once("close", () => done())), delay(1_000)]);
    }
    rmSync(fixture, { recursive: true, force: true });
  }
});

// Optional extension of this existing real capture scenario. Producer/guest
// qualification supplies the separate packaged media entry explicitly.
async function deriveRetainedFrames(runtime: string, ownerToken: string, materials: ReturnType<typeof validateMaterialsDocument>) {
  const executable = process.env.APPARENT_ENGINE_MEDIA2_BIN;
  if (!executable) return;
  const outputParent = mkdtempSync(join(tmpdir(), "capture-media2-"));
  chmodSync(outputParent, 0o700);
  const input: Media2JobInput = {
    ownerToken, materials,
    request: {
      schema: "apparent.retained-frame-request/1", materials: materials.manifest.key,
      leaseId: materials.manifest.leaseId, jobId: id(), outputDirectory: join(outputParent, "output"),
      requestedBundleMs: [materials.payload.clock.durationMs, 0, materials.payload.clock.durationMs],
      limits: { maxFrames: 3, maxEdge: 160, maxTotalPixels: 100000, deadlineMs: 10000 },
      contactSheet: { schema: "apparent.contact-sheet-layout/1", width: 320, height: 120,
        backgroundRgb: [11, 22, 33], fit: "stretch", sampling: "legacy-bilinear", clipping: "reject",
        tiles: [{ requestIndex: 1, x: 0, y: 0, width: 160, height: 120 }, { requestIndex: 2, x: 160, y: 0, width: 160, height: 120 }] },
    },
  };
  let media = new Media2Client(launchMedia2Worker(resolve(executable), runtime));
  observeEngineStderr(media);
  const competing = new Media2Client(launchMedia2Worker(resolve(executable), runtime));
  try {
    const hello = await media.handshake();
    assert.deepEqual(hello.capabilities, { retainedFrames: true, contactSheet: true, nativeJobAuthority: true });
    await competing.handshake();
    const racing = await Promise.allSettled([media.derive(input), competing.derive(input)]);
    const admitted = racing.find(result => result.status === "fulfilled");
    assert.ok(admitted && admitted.status === "fulfilled", "one duplicate peer must admit the original request");
    for (const result of racing) if (result.status === "rejected") {
      assert.ok(result.reason instanceof Media2RpcError && ["invalid_output", "busy"].includes(result.reason.code), "a losing peer reports a bounded admission conflict");
    }
    // Both process loops remain usable while the single admitted job proceeds.
    await Promise.all([media.handshake(1000), competing.handshake(1000)]);
    const concurrent = await competing.inspect(input, 1000);
    assert.ok(["running", "complete"].includes(concurrent.state));
    let status = admitted.value;
    const until = Date.now() + 15000;
    while (status.state === "running" && Date.now() < until) { await delay(10); status = await media.inspect(input); }
    assert.equal(status.state, "complete");
    if (status.state !== "complete") throw new Error("retained media did not complete");
    const derivative = status.derivative;
    // The public SDK validates detached bytes, original request/layout, native
    // material/lease/job/root binding and every ordered frame/sheet seal.
    const envelope = derivative.envelope as import("../sdk/src/media2/index.ts").Derivative2Envelope;
    assert.deepEqual(envelope.payload.frames.map(frame => frame.requestedBundleMs), input.request.requestedBundleMs);
    assert.deepEqual(envelope.payload.frames.map(frame => frame.requestIndex), [1, 2, 3]);
    assert.equal(envelope.payload.frames[0].file.sha256, envelope.payload.frames[2].file.sha256);
    assert.ok(envelope.payload.contactSheet);
    const images = [...envelope.payload.frames, envelope.payload.contactSheet];
    for (const image of images) {
      const bytes = readFileSync(join(envelope.root, image.file.relativePath));
      assert.equal(bytes.subarray(0, 8).toString("hex"), "89504e470d0a1a0a");
      assert.equal(createHash("sha256").update(bytes).digest("hex"), image.file.sha256);
      assert.equal(bytes.length, image.byteLength);
      assert.equal(bytes.readUInt32BE(16), image.width);
      assert.equal(bytes.readUInt32BE(20), image.height);
    }
    await assert.rejects(media.inspect({ ...input, ownerToken: token() }), /permission_denied/);
    await assert.rejects(media.inspect({ ...input, request: { ...input.request, requestedBundleMs: [0, 0, materials.payload.clock.durationMs] } }), /identity_conflict/);
    assert.deepEqual(await media.derive(input), status, "same request retains native receipt identity");
    await media.close();
    media = new Media2Client(launchMedia2Worker(resolve(executable), runtime));
    await media.handshake();
    assert.deepEqual(await media.inspect(input), status, "cold authenticated replay keeps the original derivative");
    assert.deepEqual(await media.cancel(input), status, "completed native work wins late cancellation");

    // Exercise the real native path with enough retained frames to keep the
    // worker busy. Timeout is an unknown RPC result; only the explicit public
    // cancel below may stop the accepted original job.
    const longRequest = (jobId: string, outputDirectory: string): Media2JobInput => ({
      ...input,
      request: {
        ...input.request,
        jobId,
        outputDirectory,
        requestedBundleMs: Array.from({ length: 48 }, (_, index) => Math.floor(materials.payload.clock.durationMs * index / 47)),
        limits: { maxFrames: 48, maxEdge: 1366, maxTotalPixels: 64_000_000, deadlineMs: 60_000 },
        contactSheet: null,
      },
    });
    const cancelInput = longRequest(id(), join(outputParent, "cancel-output"));
    let timedOut: unknown;
    try { await media.derive(cancelInput, 1); } catch (error) { timedOut = error; }
    assert.ok(timedOut instanceof Media2TransportError, "short RPC deadline reports an unknown transport outcome");
    assert.equal(timedOut.dispatched, "unknown", "timeout does not claim that dispatch was cancelled");
    const reconciled = await media.derive(cancelInput);
    assert.equal(reconciled.jobId, cancelInput.request.jobId, "reconciliation uses the original job identity");
    assert.equal(reconciled.requestSha256, (await media.inspect(cancelInput)).requestSha256, "reconciliation preserves the original request digest");
    assert.equal(reconciled.state, "running", "the timed-out native job remains accepted and running");
    const cancelled = await media.cancel(cancelInput);
    assert.equal(cancelled.jobId, cancelInput.request.jobId);
    assert.equal(cancelled.state, "cancelled", "terminal cancellation is returned only after native child stop and reap");
    assert.ok(!("derivative" in cancelled), "cancelled work has no completed derivative");
    assert.equal((await media.inspect(cancelInput)).state, "cancelled", "the original tuple observes the durable cancellation");
    assert.equal(statSync(materials.manifest.root).isDirectory(), true, "cancellation retains the original material root and lease");

    // Process loss is injected only through this helper's own client transport
    // handle. The native child lifetime is tied to the service and reconciles
    // through the same durable registry after cold restart.
    const lossInput = longRequest(id(), join(outputParent, "owner-loss-output"));
    const lossAccepted = await media.derive(lossInput);
    assert.equal(lossAccepted.jobId, lossInput.request.jobId);
    assert.equal(lossAccepted.state, "running", "process-loss case starts with active native work");
    const ownedChild = (media as unknown as { transport: { child: import("node:child_process").ChildProcess } }).transport.child;
    const serviceExit = new Promise<void>((resolveExit) => ownedChild.once("exit", () => resolveExit()));
    assert.equal(ownedChild.kill("SIGKILL"), true, "kill targets only this helper's owned media service");
    await serviceExit;
    await media.close();
    media = new Media2Client(launchMedia2Worker(resolve(executable), runtime));
    await media.handshake();
    const afterLoss = await media.inspect(lossInput);
    assert.ok(afterLoss.state === "complete" || (afterLoss.state === "failed" && afterLoss.error.code === "owner_lost"),
      `cold inspect resolves active work as owner_lost or verified complete, got ${afterLoss.state}${afterLoss.state === "failed" ? `:${afterLoss.error.code}` : ""}`);
    assert.equal(afterLoss.jobId, lossAccepted.jobId, "cold recovery keeps the original job identity");
    assert.equal(afterLoss.requestSha256, lossAccepted.requestSha256, "cold recovery keeps the original request digest, output path, materials and lease binding");
    if (afterLoss.state === "complete") {
      const envelope = afterLoss.derivative.envelope as import("../sdk/src/media2/index.ts").Derivative2Envelope;
      assert.equal(envelope.payload.jobId, lossInput.request.jobId);
      assert.equal(envelope.payload.requestSha256, afterLoss.requestSha256);
      assert.deepEqual(envelope.payload.materials, materials.manifest.key);
    }
    if (process.env.APPARENT_CAPTURE_EVIDENCE_DIR) {
      const evidence = join(resolve(process.env.APPARENT_CAPTURE_EVIDENCE_DIR), "media2");
      mkdirSync(evidence, { recursive: true, mode: 0o700 });
      writeFileSync(join(evidence, "request.json"), JSON.stringify(input.request), { mode: 0o600 });
      writeFileSync(join(evidence, "receipt.json"), derivative.bytes, { mode: 0o600 });
      writeFileSync(join(evidence, "envelope.json"), JSON.stringify(envelope), { mode: 0o600 });
      writeFileSync(join(evidence, "conformance.json"), JSON.stringify({
        timedOut: { jobId: cancelInput.request.jobId, requestSha256: reconciled.requestSha256, dispatched: timedOut.dispatched },
        cancelled: { jobId: cancelled.jobId, state: cancelled.state, materialsRoot: materials.manifest.root },
        processLoss: { jobId: lossInput.request.jobId, requestSha256: afterLoss.requestSha256, state: afterLoss.state,
          error: afterLoss.state === "failed" ? afterLoss.error.code : undefined },
      }, null, 2), { mode: 0o600 });
      for (const [index, image] of images.entries()) copyFileSync(join(envelope.root, image.file.relativePath), join(evidence, `image-${index + 1}.png`));
    }
    console.log("Native capture -> authenticated retained media2 PNGs/contact sheet, timeout/cancel/reap, and cold process-loss reconciliation passed");
  } finally { await media.close(); await competing.close(); rmSync(outputParent, { recursive: true, force: true }); }
}

async function until(client: CaptureClient, authority: { ownerToken: string; captureId: string }, states: string[]) {
  const deadline = Date.now() + 12_000;
  let last: CaptureStatus | undefined;
  while (Date.now() < deadline) {
    last = await client.inspect(authority);
    if (states.includes(last.state)) return last;
    if (last.state === "failed") throw new Error(`capture failed: ${last.error?.code ?? "unknown"}`);
    await delay(40);
  }
  throw new Error(`capture did not reach ${states.join("/")}; last=${last?.state}:${last?.error?.code ?? "none"}`);
}
function startInput(source: Awaited<ReturnType<CaptureClient["discoverSources"]>>["sources"][number], captureId: string, ownerToken: string, suffix: string) {
  return {
    captureId, ownerToken, bundleId: `capture-${suffix}.apparent`,
    bundlePath: resolve(tmpdir(), `capture-${suffix}.apparent`), source,
    options: { audio: "none" as const, localOnly: true as const },
    invocation: { kind: "explicit" as const, invocationId: id(), observedAtMs: Date.now() },
  };
}
function observeEngineStderr(client: CaptureClient | Media2Client) {
  const verbose = process.env.APPARENT_CAPTURE_DEBUG_STDERR === "1";
  if (!verbose && process.env.APPARENT_CAPTURE_TEST_RELEASE_DIAGNOSTICS !== "1") return;
  const child = (client as unknown as { transport: { child: {
    stderr: NodeJS.ReadableStream;
    on(event: "exit", listener: (code: number | null, signal: NodeJS.Signals | null) => void): void;
  } } }).transport.child;
  const stages = new Set([
    "root-open", "root-metadata", "root-identity", "unknown-entry", "file-open",
    "file-validation", "file-metadata", "file-binding-missing", "file-type",
    "file-owner-mode", "file-link-count", "file-identity", "durable-binding-missing",
    "parent-open", "directory-name", "directory-unlink", "journal-read",
    "movie-device", "movie-inode", "movie-size", "payload-device", "payload-inode", "payload-size",
  ]);
  let pending = "";
  child.stderr.on("data", (chunk: Buffer) => {
    if (verbose) { process.stderr.write(chunk); return; }
    const lines = (pending + chunk.toString("utf8")).split("\n");
    pending = (lines.pop() ?? "").slice(-160);
    for (const raw of lines) {
      const line = raw.endsWith("\r") ? raw.slice(0, -1) : raw;
      const prefix = "capture release refused: ";
      if (line.length <= 160 && line.startsWith(prefix) && stages.has(line.slice(prefix.length))) {
        process.stderr.write(`${line}\n`);
      }
    }
  });
  child.on("exit", (code, signal) => {
    process.stderr.write(`capture worker exited: code=${code} signal=${signal}\n`);
  });
}
function capturePeerClient(runtime: string) {
  const executable = binary;
  if (!executable) throw new Error("capture peer binary is unavailable");
  const client = new CaptureClient(launchCaptureEngine(resolve(executable), runtime, 10_000));
  observeEngineStderr(client);
  return client;
}

test("SDK capture/2 records a selected X11 output, seals materials, and releases only by owner", { skip: !binary }, async () => {
  const { server: xserver, display, diagnostics: xserverDiagnostics } = await readyXvfb();
  const oldDisplay = process.env.DISPLAY;
  const oldWayland = process.env.WAYLAND_DISPLAY;
  const oldGstDebug = process.env.GST_DEBUG;
  const oldGstDebugFile = process.env.GST_DEBUG_FILE;
  const oldFinalizeDelay = process.env.APPARENT_CAPTURE_TEST_FINALIZATION_DELAY_MS;
  const oldReleaseDelay = process.env.APPARENT_CAPTURE_TEST_RELEASE_DELAY_AFTER_INTENT_MS;
  const oldReadinessUnavailable = process.env.APPARENT_CAPTURE_TEST_READINESS_UNAVAILABLE;
  const oldCancelCleanupBundle = process.env.APPARENT_CAPTURE_TEST_CANCEL_CLEANUP_BUNDLE_ID;
  const oldReleaseDiagnostics = process.env.APPARENT_CAPTURE_TEST_RELEASE_DIAGNOSTICS;
  process.env.DISPLAY = display;
  delete process.env.WAYLAND_DISPLAY;
  process.env.APPARENT_CAPTURE_TEST_RELEASE_DIAGNOSTICS = "1";
  const runtime = mkdtempSync(join(tmpdir(), "apparent-capture-runtime-"));
  chmodSync(runtime, 0o700);
  process.env.GST_DEBUG = "2";
  process.env.GST_DEBUG_FILE = join(runtime, "gstreamer.log");
  process.env.APPARENT_CAPTURE_TEST_FINALIZATION_DELAY_MS = "750";
  process.env.APPARENT_CAPTURE_TEST_RELEASE_DELAY_AFTER_INTENT_MS = "900";
  process.env.APPARENT_CAPTURE_TEST_CANCEL_CLEANUP_BUNDLE_ID = "capture-cancel-ack-crash.apparent";
  let client: CaptureClient | undefined;
  let scenarioPassed = false;
  let preserveCancelRuntime = false;
  try {
    process.stderr.write("capture peer: Xvfb endpoint verified before starting Engine worker\n");
    client = capturePeerClient(runtime);
    let hello;
    try {
      hello = await client.handshake(15_000);
    } catch (error) {
      const evidence = process.env.APPARENT_CAPTURE_EVIDENCE_DIR;
      if (evidence) {
        mkdirSync(evidence, { recursive: true, mode: 0o700 });
        const diagnostic = join(evidence, "startup-diagnostics");
        mkdirSync(diagnostic, { mode: 0o700 });
        for (const name of ["gstreamer.log", "capture-v2/journal.json"]) {
          try { copyFileSync(join(runtime, name), join(diagnostic, name.replaceAll("/", "-"))); } catch {}
        }
        writeFileSync(join(diagnostic, "xserver.stderr"), xserverDiagnostics(), { mode: 0o600 });
        writeFileSync(join(diagnostic, "worker.stderr-note"), `${String(error)}\n`, { mode: 0o600 });
      }
      throw new Error(`capture handshake failed; Xvfb=${xserver.exitCode ?? "running"}; ${xserverDiagnostics().trim() || "no Xvfb stderr"}; ${String(error)}`);
    }
    assert.equal(hello.implementation, "native");
    assert.equal(hello.readiness, "ready");
    assert.equal(hello.capabilities.publication, true, "native publication is available independently of screen readiness");
    assert.equal(hello.capabilities.audioCapture, false);
    const discovery = await client.discoverSources();
    assert.equal(discovery.state, "available");
    const source = discovery.sources.find((item) => item.kind === "display");
    assert.ok(source, "X11 display source must be advertised");
    assert.equal(source.bounds.width, 1366);
    assert.equal(source.bounds.height, 768);

    const failedJournal = startInput(source, id(), token(), "journal-write-failure");
    chmodSync(join(runtime, "capture-v2"), 0o500);
    await assert.rejects(client.start(failedJournal), /internal/);
    assert.throws(() => statSync(join(runtime, "capture-v2", "materials", failedJournal.captureId)), { code: "ENOENT" }, "journal persistence failure precedes managed directory creation");
    chmodSync(join(runtime, "capture-v2"), 0o700);

    const captureId = id();
    const ownerToken = token();
    const input = startInput(source, captureId, ownerToken, "first");
    const started = await client.start(input);
    assert.ok(["starting", "recording"].includes(started.state));
    const recording = await until(client, { captureId, ownerToken }, ["recording"]);
    assert.equal(recording.state, "recording");
    const recoveryObserver = await launchCaptureRecoveryPeer(resolve(binary!), runtime);
    try {
      assert.equal((await recoveryObserver.client.handshake()).readiness, "unavailable");
      assert.equal((await recoveryObserver.client.inspect({ captureId, ownerToken })).state, "recording");
    } finally {
      const released = await recoveryObserver.release();
      assert.equal(released.connection, "released");
      assert.equal(released.runtimeClearance, "not-established");
    }
    assert.equal((await client.inspect({ captureId, ownerToken })).state, "recording", "recovery EOF must leave another peer's actual native capture running");
    for (const color of ["#11bb44", "#2244dd", "#e6d22a", "#cc2211"]) {
      execFileSync(xsetrootCommand, ["-solid", color]);
      await delay(180);
    }
    process.env.APPARENT_CAPTURE_TEST_READINESS_UNAVAILABLE = "1";
    const coldStopClient = capturePeerClient(runtime);
    try {
      const coldHello = await coldStopClient.handshake(15_000);
      assert.equal(coldHello.readiness, "unavailable", "cold control client has no new source readiness");
      const coldObserved = await coldStopClient.inspect({ captureId, ownerToken });
      assert.equal(coldObserved.state, "recording", "cold inspect observes the authenticated live owner");
      const coldStopped = await coldStopClient.stop({ captureId, ownerToken });
      assert.equal(coldStopped.state, "stopping", "cold stop reaches the live owner without claiming global capture admission");
    } finally {
      await coldStopClient.close();
      if (oldReadinessUnavailable === undefined) delete process.env.APPARENT_CAPTURE_TEST_READINESS_UNAVAILABLE;
      else process.env.APPARENT_CAPTURE_TEST_READINESS_UNAVAILABLE = oldReadinessUnavailable;
    }
    const ready = await until(client, { captureId, ownerToken }, ["materials-ready"]);
    assert.equal(ready.state, "materials-ready");
    const manifest = ready.materials!;
    const trustedMaterialsRoot = capture2MaterialsRoot(runtime, captureId);
    assert.equal(manifest.root, trustedMaterialsRoot, "native materials root matches the independently configured runtime and admitted capture");
    const payloadPath = join(trustedMaterialsRoot, manifest.payloadFile.relativePath);
    const payloadBytes = readFileSync(payloadPath);
    const payloadStat = statSync(payloadPath, { bigint: true });
    assert.equal(createHash("sha256").update(payloadBytes).digest("hex"), manifest.payloadFile.sha256);
    assert.equal(payloadStat.ino.toString(), manifest.payloadFile.identity.inode);
    assert.equal(payloadStat.dev.toString(), manifest.payloadFile.identity.device);
    assert.equal(payloadStat.size, BigInt(manifest.payloadFile.identity.size));
    assert.equal(payloadStat.mtimeNs.toString(), manifest.payloadFile.identity.modifiedNs);
    const payload = JSON.parse(payloadBytes.toString("utf8"));
    const detached = validateMaterialsDocument(manifest, payloadBytes);
    assert.equal(detached.payload.identity.captureId, captureId);
    assert.equal(payload.schema, "apparent.capture-materials-payload/1");
    assert.equal(payload.safety.nativeRemoteDelivery, "forbidden");
    assert.equal(payload.safety.safeForProcessing, true);
    assert.equal(payload.invocation.invocationId, input.invocation.invocationId, "native clock normalization preserves caller invocation identity");
    assert.equal(payload.invocation.observedAtMs, 0, "native receipt time is represented in the capture epoch, not copied from caller wall time");
    assert.ok(payload.confirmedSource.confirmedAtMs > 0, "source confirmation time is measured after native source revalidation");
    assert.equal(payload.clock.epoch, "capture-admission-monotonic");
    assert.equal(payload.clock.units, "integer-milliseconds");
    const mappedVideoEndMs = payload.clock.videoOffsetMs + payload.video.durationMs;
    const finalFrameIntervalMs = Math.ceil(1000 / 30);
    const integerRoundingMs = 2; // ceil offset + ceil video duration versus floored capture stop.
    const endpointDifferenceMs = Math.abs(mappedVideoEndMs - payload.clock.durationMs);
    assert.ok(
      endpointDifferenceMs <= finalFrameIntervalMs + integerRoundingMs,
      `capture clock and mapped MP4 endpoint differ by at most one scheduled final-frame interval (clock=${payload.clock.durationMs}ms, videoOffset=${payload.clock.videoOffsetMs}ms, video=${payload.video.durationMs}ms, endpoint=${mappedVideoEndMs}ms, difference=${endpointDifferenceMs}ms, finalFrame=${finalFrameIntervalMs}ms, rounding=${integerRoundingMs}ms)`,
    );
    assert.ok(payload.clock.videoOffsetMs > 0, "video offset maps the first scheduled video PTS into the capture admission clock");
    assert.equal(payload.video.width, 1366);
    assert.equal(payload.video.height, 768);
    assert.ok(Object.values(payload.collection).every((state) => state === "unavailable"));
    const movie = join(manifest.root, "recording.mp4");
    const videoFile = payload.files.find((file: { relativePath: string }) => file.relativePath === "recording.mp4");
    assert.ok(videoFile);
    const movieBytes = readFileSync(movie);
    const movieStat = statSync(movie, { bigint: true });
    assert.equal(createHash("sha256").update(movieBytes).digest("hex"), videoFile.sha256);
    assert.equal(movieStat.ino.toString(), videoFile.identity.inode);
    assert.equal(movieStat.dev.toString(), videoFile.identity.device);
    assert.equal(movieStat.size, BigInt(videoFile.identity.size));
    assert.equal(movieStat.mtimeNs.toString(), videoFile.identity.modifiedNs);
    const probe = execFileSync(ffprobeCommand, ["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=codec_name,width,height", "-show_entries", "format=duration", "-of", "json", movie], { encoding: "utf8" });
    const probeResult = JSON.parse(probe);
    const stream = probeResult.streams[0];
    assert.equal(stream.codec_name, "h264");
    assert.equal(stream.width, 1366);
    assert.equal(stream.height, 768);
    assert.ok(Math.abs(Number(probeResult.format.duration) * 1000 - payload.video.durationMs) < 250, "manifest video duration matches the finalized MP4 timeline");
    const png = join(runtime, "first-frame.png");
    execFileSync(ffmpegCommand, ["-v", "error", "-i", movie, "-frames:v", "1", "-y", png]);
    assert.equal(readFileSync(png).subarray(0, 8).toString("hex"), "89504e470d0a1a0a");
    const hashes = execFileSync(ffmpegCommand, ["-v", "error", "-i", movie, "-f", "framemd5", "-"], { encoding: "utf8" }).split("\n").filter((line) => line && !line.startsWith("#"));
    assert.ok(new Set(hashes.map((line) => line.split(",").at(-1)?.trim())).size > 1, "selected source must contain changing frames");
    if (process.env.APPARENT_CAPTURE_EVIDENCE_DIR) {
      const evidence = resolve(process.env.APPARENT_CAPTURE_EVIDENCE_DIR);
      mkdirSync(evidence, { recursive: true, mode: 0o700 });
      copyFileSync(movie, join(evidence, "recording.mp4"));
      copyFileSync(payloadPath, join(evidence, "materials.json"));
      copyFileSync(png, join(evidence, "first-frame.png"));
      writeFileSync(join(evidence, "manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`, { mode: 0o600, flag: "wx" });
    }

    await deriveRetainedFrames(runtime, ownerToken, detached);
    const replay = await client.start(input);
    assert.equal(replay.state, "materials-ready");
    const conflict = { ...input, bundlePath: `${input.bundlePath}.other` };
    await assert.rejects(client.start(conflict), /identity_conflict/);
    await assert.rejects(client.inspect({ captureId, ownerToken: token() }), /permission_denied/);
    await assert.rejects(client.releaseMaterials({ captureId, ownerToken: token(), materialsId: manifest.key.materialsId, leaseId: manifest.leaseId }), /permission_denied/);

    const area = discovery.sources.find((item) => item.kind === "area");
    assert.ok(area);
    const second = startInput(area, id(), token(), "second");
    assert.equal(second.source.bounds.width, 683);
    assert.equal(second.source.bounds.height, 384);
    const secondStart = await client.start(second);
    assert.ok(["starting", "recording"].includes(secondStart.state), "new capture can start while old materials stay retained");
    await until(client, { captureId: second.captureId, ownerToken: second.ownerToken }, ["recording"]);
    for (const color of ["#e65b32", "#37c6a1", "#a842d4"]) {
      execFileSync(xsetrootCommand, ["-solid", color]);
      await delay(160);
    }
    await client.stop({ captureId: second.captureId, ownerToken: second.ownerToken });
    const areaReady = await until(client, { captureId: second.captureId, ownerToken: second.ownerToken }, ["materials-ready"]);
    const areaRoot = capture2MaterialsRoot(runtime, second.captureId);
    assert.equal(areaReady.materials!.root, areaRoot);
    const areaDocument = validateMaterialsDocument(areaReady.materials!, readFileSync(join(areaRoot, areaReady.materials!.payloadFile.relativePath)));
    assert.equal(areaDocument.payload.confirmedSource.rectangle.width, 683, "confirmed source keeps the actual physical area");
    assert.equal(areaDocument.payload.video.width, 682, "manifest reports the actual even encoded width");
    assert.equal(areaDocument.payload.video.height, 384);
    const areaMovie = join(areaReady.materials!.root, "recording.mp4");
    const areaInfo = JSON.parse(execFileSync(ffprobeCommand, ["-v", "error", "-select_streams", "v:0", "-show_entries", "stream=width,height,codec_name", "-of", "json", areaMovie], { encoding: "utf8" }));
    assert.deepEqual(areaInfo.streams[0], { codec_name: "h264", width: 682, height: 384 }, "the odd-width physical area is resampled to valid even encoded dimensions");
    if (process.env.APPARENT_CAPTURE_EVIDENCE_DIR) {
      const evidence = join(resolve(process.env.APPARENT_CAPTURE_EVIDENCE_DIR), "area");
      mkdirSync(evidence, { recursive: true, mode: 0o700 });
      copyFileSync(areaMovie, join(evidence, "recording.mp4"));
      copyFileSync(join(areaReady.materials!.root, areaReady.materials!.payloadFile.relativePath), join(evidence, "materials.json"));
      writeFileSync(join(evidence, "manifest.json"), `${JSON.stringify(areaReady.materials, null, 2)}\n`, { mode: 0o600, flag: "wx" });
      writeFileSync(join(evidence, "ffprobe.json"), `${JSON.stringify(areaInfo, null, 2)}\n`, { mode: 0o600, flag: "wx" });
      execFileSync(ffmpegCommand, ["-v", "error", "-i", areaMovie, "-frames:v", "1", join(evidence, "first-frame.png")]);
    }

    const third = startInput(source, id(), token(), "eof");
    await client.start(third);
    await until(client, { captureId: third.captureId, ownerToken: third.ownerToken }, ["recording"]);
    // Recording begins at the first source buffer; let it traverse the encoder
    // and muxer before exercising stdin-EOF finalization.
    await delay(160);
    assert.equal(await client.close(), "eof-exited", "stdin EOF drains and exits without a signal request");
    client = capturePeerClient(runtime);
    await delay(100);
    await client.handshake(15_000);
    const recovered = await client.inspect({ captureId: third.captureId, ownerToken: third.ownerToken });
    assert.equal(recovered.state, "materials-ready", `authenticated reconnect recovers the sealed EOF result (${recovered.error?.code ?? "no error"})`);
    assert.ok(recovered.materials?.payloadFile.sha256);
    await client.releaseMaterials({ captureId: third.captureId, ownerToken: third.ownerToken, materialsId: recovered.materials!.key.materialsId, leaseId: recovered.materials!.leaseId });

    const reconnectSources = await client.discoverSources();
    const reconnectSource = reconnectSources.sources.find((item) => item.kind === "display");
    assert.ok(reconnectSource);
    const fourth = startInput(reconnectSource, id(), token(), "cancel");
    await client.start(fourth);
    await until(client, { captureId: fourth.captureId, ownerToken: fourth.ownerToken }, ["recording"]);
    process.env.APPARENT_CAPTURE_TEST_READINESS_UNAVAILABLE = "1";
    const coldCancelClient = capturePeerClient(runtime);
    try {
      const coldHello = await coldCancelClient.handshake(15_000);
      assert.equal(coldHello.readiness, "unavailable");
      const coldCancelled = await coldCancelClient.cancel({ captureId: fourth.captureId, ownerToken: fourth.ownerToken });
      assert.ok(["starting", "recording", "stopping"].includes(coldCancelled.state));
    } finally {
      await coldCancelClient.close();
      if (oldReadinessUnavailable === undefined) delete process.env.APPARENT_CAPTURE_TEST_READINESS_UNAVAILABLE;
      else process.env.APPARENT_CAPTURE_TEST_READINESS_UNAVAILABLE = oldReadinessUnavailable;
    }
    const cancelled = await until(client, { captureId: fourth.captureId, ownerToken: fourth.ownerToken }, ["cancelled"]);
    assert.equal(cancelled.error, undefined, "cancelled is a terminal state, not a failed result");
    const fourthRoot = join(runtime, "capture-v2", "materials", fourth.captureId);
    try { statSync(fourthRoot); preserveCancelRuntime = true; } catch {}
    assert.throws(() => statSync(fourthRoot), { code: "ENOENT" }, "cancel never leaves a sealed material root");

    const fifth = startInput(reconnectSource, id(), token(), "crash-after-seal");
    await client.start(fifth);
    await until(client, { captureId: fifth.captureId, ownerToken: fifth.ownerToken }, ["recording"]);
    await client.stop({ captureId: fifth.captureId, ownerToken: fifth.ownerToken });
    const fifthReady = await until(client, { captureId: fifth.captureId, ownerToken: fifth.ownerToken }, ["materials-ready"]);
    const nativeChild = (client as unknown as { transport: { child: { kill: (signal: string) => boolean; once: (event: string, callback: () => void) => void } } }).transport.child;
    const childExit = new Promise<void>((resolveExit) => nativeChild.once("exit", resolveExit));
    assert.equal(nativeChild.kill("SIGKILL"), true);
    await childExit;
    client = capturePeerClient(runtime);
    await delay(100);
    await client.handshake(15_000);
    const afterCrash = await client.inspect({ captureId: fifth.captureId, ownerToken: fifth.ownerToken });
    assert.equal(afterCrash.state, "materials-ready", `crash recovery preserves the sealed state (${afterCrash.error?.code ?? "no error"})`);
    assert.equal(afterCrash.materials?.key.revision, fifthReady.materials?.key.revision, "crash recovery preserves the exact sealed manifest");
    await client.cancel({ captureId: fifth.captureId, ownerToken: fifth.ownerToken });
    const pendingRelease = client.releaseMaterials({ captureId: fifth.captureId, ownerToken: fifth.ownerToken, materialsId: afterCrash.materials!.key.materialsId, leaseId: afterCrash.materials!.leaseId }).then(value => ({ value }), error => ({ error }));
    const releaseJournal = join(runtime, "capture-v2", `${fifth.captureId}.json`);
    const releaseDeadline = Date.now() + 4_000;
    while (Date.now() < releaseDeadline && JSON.parse(readFileSync(releaseJournal, "utf8")).release_phase !== "pending") await delay(10);
    assert.equal(JSON.parse(readFileSync(releaseJournal, "utf8")).release_phase, "pending", "release intent is durable before deletion begins");
    const releasingChild = (client as unknown as { transport: { child: { kill: (signal: string) => boolean; once: (event: string, callback: () => void) => void } } }).transport.child;
    const releaseExit = new Promise<void>((resolveExit) => releasingChild.once("exit", resolveExit));
    assert.equal(releasingChild.kill("SIGKILL"), true);
    await releaseExit;
    const lostReleaseReply = await pendingRelease;
    assert.ok("error" in lostReleaseReply, "crash after durable release intent leaves the reply unknown to the caller");
    delete process.env.APPARENT_CAPTURE_TEST_RELEASE_DELAY_AFTER_INTENT_MS;
    client = capturePeerClient(runtime);
    await delay(100);
    await client.handshake(15_000);
    const pendingReplay = await client.inspect({ captureId: fifth.captureId, ownerToken: fifth.ownerToken });
    assert.equal(pendingReplay.state, "cancelled");
    assert.equal(pendingReplay.materials?.key.materialsId, afterCrash.materials!.key.materialsId, "crash during release retains original manifest and lease authority");
    await client.releaseMaterials({ captureId: fifth.captureId, ownerToken: fifth.ownerToken, materialsId: afterCrash.materials!.key.materialsId, leaseId: afterCrash.materials!.leaseId });
    const releasedReplay = await client.inspect({ captureId: fifth.captureId, ownerToken: fifth.ownerToken });
    assert.equal(releasedReplay.state, "cancelled");
    assert.equal(releasedReplay.materials, undefined);

    const postCrashSources = await client.discoverSources();
    const postCrashSource = postCrashSources.sources.find((item) => item.kind === "display");
    assert.ok(postCrashSource);
    const cancelCrash = startInput(postCrashSource, id(), token(), "cancel-ack-crash");
    await client.start(cancelCrash);
    await until(client, { captureId: cancelCrash.captureId, ownerToken: cancelCrash.ownerToken }, ["recording"]);
    const coldCancellation = capturePeerClient(runtime);
    try {
      await coldCancellation.handshake(15_000);
      await coldCancellation.cancel({ captureId: cancelCrash.captureId, ownerToken: cancelCrash.ownerToken });
    } finally {
      await coldCancellation.close();
    }
    const cancelledRoot = join(runtime, "capture-v2", "materials", cancelCrash.captureId);
    const cleanupDeadline = Date.now() + 4_000;
    let cleanupObserved = false;
    while (Date.now() < cleanupDeadline) {
      try { statSync(cancelledRoot); }
      catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
        cleanupObserved = true;
        break;
      }
      await delay(10);
    }
    assert.equal(cleanupObserved, true, "the owner reaches the deterministic post-cleanup pause");
    const cancellationJournal = JSON.parse(readFileSync(join(runtime, "capture-v2", `${cancelCrash.captureId}.json`), "utf8"));
    assert.equal(cancellationJournal.state, "recording", "terminal cancellation is not durable before cleanup acknowledgment");
    assert.equal(cancellationJournal.cancel_requested, true, "cancel acknowledgment is already durable");
    const cancelOwner = (client as unknown as { transport: { child: { kill: (signal: string) => boolean; once: (event: string, callback: () => void) => void } } }).transport.child;
    const cancelOwnerExit = new Promise<void>((resolveExit) => cancelOwner.once("exit", resolveExit));
    assert.equal(cancelOwner.kill("SIGKILL"), true, "crash after the durable cancellation acknowledgment");
    await cancelOwnerExit;
    client = capturePeerClient(runtime);
    await delay(100);
    await client.handshake(15_000);
    let cancelledCrash: CaptureStatus;
    try {
      cancelledCrash = await client.inspect({ captureId: cancelCrash.captureId, ownerToken: cancelCrash.ownerToken });
    } catch (error) {
      preserveCancelRuntime = true;
      throw error;
    }
    assert.equal(cancelledCrash.state, "cancelled", "accepted cancellation survives owner crash and recovery");
    assert.throws(() => statSync(join(runtime, "capture-v2", "materials", cancelCrash.captureId)), { code: "ENOENT" });

    const afterCancelSources = await client.discoverSources();
    const afterCancelSource = afterCancelSources.sources.find((item) => item.kind === "display");
    assert.ok(afterCancelSource);
    const sixth = startInput(afterCancelSource, id(), token(), "crash-during-capture");
    await client.start(sixth);
    await until(client, { captureId: sixth.captureId, ownerToken: sixth.ownerToken }, ["recording"]);
    const nextNativeChild = (client as unknown as { transport: { child: { kill: (signal: string) => boolean; once: (event: string, callback: () => void) => void } } }).transport.child;
    const nextExit = new Promise<void>((resolveExit) => nextNativeChild.once("exit", resolveExit));
    assert.equal(nextNativeChild.kill("SIGKILL"), true);
    await nextExit;
    client = capturePeerClient(runtime);
    await delay(100);
    await client.handshake(15_000);
    const failedCrash = await client.inspect({ captureId: sixth.captureId, ownerToken: sixth.ownerToken });
    assert.equal(failedCrash.state, "failed", "a crash before sealing is never recovered as Saved or materials-ready");
    assert.equal(failedCrash.error?.code, "unavailable");
    assert.throws(() => statSync(join(runtime, "capture-v2", "materials", sixth.captureId)), { code: "ENOENT" }, "crash recovery removes only unsealed partial files");

    await client.releaseMaterials({ captureId: second.captureId, ownerToken: second.ownerToken, materialsId: areaReady.materials!.key.materialsId, leaseId: areaReady.materials!.leaseId });
    const runtimeRoot = join(runtime, "capture-v2");
    const heldRoot = `${runtimeRoot}.held-${id()}`;
    renameSync(runtimeRoot, heldRoot);
    symlinkSync(heldRoot, runtimeRoot);
    await assert.rejects(client.releaseMaterials({ captureId, ownerToken, materialsId: manifest.key.materialsId, leaseId: manifest.leaseId }), /source_mismatch|internal/);
    unlinkSync(runtimeRoot);
    renameSync(heldRoot, runtimeRoot);
    const heldOperation = `${manifest.root}.held-${id()}`;
    renameSync(manifest.root, heldOperation);
    mkdirSync(manifest.root, { mode: 0o700 });
    await assert.rejects(client.releaseMaterials({ captureId, ownerToken, materialsId: manifest.key.materialsId, leaseId: manifest.leaseId }), /source_mismatch/);
    rmdirSync(manifest.root);
    renameSync(heldOperation, manifest.root);
    const unexpected = join(manifest.root, "caller-owned.txt");
    writeFileSync(unexpected, "preserve this caller file", { mode: 0o600, flag: "wx" });
    await assert.rejects(client.releaseMaterials({ captureId, ownerToken, materialsId: manifest.key.materialsId, leaseId: manifest.leaseId }), /source_mismatch/);
    assert.equal(readFileSync(unexpected, "utf8"), "preserve this caller file", "unknown contents are preserved on refused cleanup");
    unlinkSync(unexpected);
    const abandoned = await client.cancel({ captureId, ownerToken });
    assert.equal(abandoned.state, "cancelled", "cancel on ready materials durably marks abandonment");
    assert.equal(abandoned.materials?.key.materialsId, manifest.key.materialsId, "cancel retains material and lease authority");
    const released = await client.releaseMaterials({ captureId, ownerToken, materialsId: manifest.key.materialsId, leaseId: manifest.leaseId });
    assert.equal(released.released, true);
    assert.throws(() => statSync(movie), { code: "ENOENT" });
    assert.throws(() => statSync(join(manifest.root, "apparent.json")), { code: "ENOENT" });
    const afterRelease = await client.inspect({ captureId, ownerToken });
    assert.equal(afterRelease.state, "cancelled");
    assert.equal(afterRelease.materials, undefined, "released terminal observation never exposes a missing manifest");
    await client.close();
    client = capturePeerClient(runtime);
    await delay(100);
    await client.handshake(15_000);
    const replayRelease = await client.releaseMaterials({ captureId, ownerToken, materialsId: manifest.key.materialsId, leaseId: manifest.leaseId });
    assert.equal(replayRelease.released, true, "release acknowledgment replays after worker restart");
    scenarioPassed = true;
  } finally {
    await client?.close();
    if (xserver.exitCode === null && xserver.signalCode === null) {
      xserver.kill("SIGTERM");
      await Promise.race([new Promise<void>((resolveExit) => xserver.once("close", () => resolveExit())), delay(1_000)]);
    }
    if (oldDisplay === undefined) delete process.env.DISPLAY; else process.env.DISPLAY = oldDisplay;
    if (oldWayland === undefined) delete process.env.WAYLAND_DISPLAY; else process.env.WAYLAND_DISPLAY = oldWayland;
    if (oldGstDebug === undefined) delete process.env.GST_DEBUG; else process.env.GST_DEBUG = oldGstDebug;
    if (oldGstDebugFile === undefined) delete process.env.GST_DEBUG_FILE; else process.env.GST_DEBUG_FILE = oldGstDebugFile;
    if (oldFinalizeDelay === undefined) delete process.env.APPARENT_CAPTURE_TEST_FINALIZATION_DELAY_MS; else process.env.APPARENT_CAPTURE_TEST_FINALIZATION_DELAY_MS = oldFinalizeDelay;
    if (oldReleaseDelay === undefined) delete process.env.APPARENT_CAPTURE_TEST_RELEASE_DELAY_AFTER_INTENT_MS; else process.env.APPARENT_CAPTURE_TEST_RELEASE_DELAY_AFTER_INTENT_MS = oldReleaseDelay;
    if (oldReadinessUnavailable === undefined) delete process.env.APPARENT_CAPTURE_TEST_READINESS_UNAVAILABLE; else process.env.APPARENT_CAPTURE_TEST_READINESS_UNAVAILABLE = oldReadinessUnavailable;
    if (oldCancelCleanupBundle === undefined) delete process.env.APPARENT_CAPTURE_TEST_CANCEL_CLEANUP_BUNDLE_ID; else process.env.APPARENT_CAPTURE_TEST_CANCEL_CLEANUP_BUNDLE_ID = oldCancelCleanupBundle;
    if (oldReleaseDiagnostics === undefined) delete process.env.APPARENT_CAPTURE_TEST_RELEASE_DIAGNOSTICS; else process.env.APPARENT_CAPTURE_TEST_RELEASE_DIAGNOSTICS = oldReleaseDiagnostics;
    if (process.env.APPARENT_CAPTURE_EVIDENCE_DIR) {
      try {
        const evidence = resolve(process.env.APPARENT_CAPTURE_EVIDENCE_DIR);
        mkdirSync(evidence, { recursive: true, mode: 0o700 });
        copyFileSync(join(runtime, "gstreamer.log"), join(evidence, "gstreamer.log"));
        const diagnostic = join(evidence, "startup-diagnostics");
        mkdirSync(diagnostic, { recursive: true, mode: 0o700 });
        writeFileSync(join(diagnostic, "xserver.stderr"), xserverDiagnostics(), { mode: 0o600 });
        writeFileSync(join(diagnostic, "xserver-exit.json"), `${JSON.stringify({ exitCode: xserver.exitCode, signalCode: xserver.signalCode })}\n`, { mode: 0o600 });
      } catch { /* no diagnostic file was produced */ }
    }
    const preserveRoot = process.env.APPARENT_CAPTURE_TEST_PRESERVE_RUNTIME_DIR;
    if (scenarioPassed) {
      rmSync(runtime, { recursive: true, force: true });
    } else if (preserveCancelRuntime && preserveRoot) {
      const destination = join(resolve(preserveRoot), "runtime");
      mkdirSync(dirname(destination), { recursive: true, mode: 0o700 });
      chmodSync(dirname(destination), 0o700);
      renameSync(runtime, destination);
    } else {
      // Retain failed synthetic journals privately; they contain owner tokens.
      process.stderr.write(`capture peer failed; private synthetic runtime retained at ${runtime}\n`);

    }
  }
});
