// Composition consumer conformance from 4ad8f1582. Run against admitted public
// Record/SDK tarballs; this stays outside Engine because it imports Record.
// Fixture materials remain a separate conformance case. The explicit real-X11
// mode exercises the public native capture and publication path end to end.
import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import { chmodSync, copyFileSync, createReadStream, mkdirSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { setTimeout as delay } from "node:timers/promises";
import test from "node:test";
import {
  CaptureClient,
  launchCaptureEngine,
  preflightCapturePublication,
  validateMaterialsDocument,
  type MaterialsDocument,
} from "@apparent/engine-sdk/capture";
import { Media2Client, encodeContactSheetLayout, launchMedia2Worker, prepareMedia2Request, type Derivative2Envelope } from "@apparent/engine-sdk/media2";
import { assembleCanonicalBundle, type BundleAssemblyInput, type SealedArtifact } from "@apparent/record/processing/bundle";
import { readCaptureMaterials } from "@apparent/record/host/materials";
import { RecordMedia2Job } from "@apparent/record/processing/media2-consumer";
import { readRetainedDerivativeArtifacts } from "@apparent/record/processing/retained-artifacts";
import { AbsentDestinationAllocator } from "@apparent/record/host";
import { createRecordPublicationRecovery, prepareRecordPublication } from "@apparent/record/publication";

type SealedFile = { relativePath: string; identity: { device: string; inode: string; size: number; modifiedNs: string }; sha256: string };

const id = () => randomBytes(16).toString("hex");
const token = () => randomBytes(32).toString("hex");
const binary = process.env.APPARENT_ENGINE_CAPTURE_BIN;
const mediaBinary = process.env.APPARENT_ENGINE_MEDIA2_BIN;
const realCaptureMode = process.env.APPARENT_RECORD_PEER_REAL_CAPTURE === "1" || process.env.SCRAWL_GUI_OUTPUT !== undefined;
const hash = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");

function privateDir(path: string) {
  mkdirSync(path, { recursive: true, mode: 0o700 });
  chmodSync(path, 0o700);
}

function privateFile(path: string, bytes: Uint8Array) {
  writeFileSync(path, bytes, { mode: 0o600, flag: "wx" });
  chmodSync(path, 0o600);
}

async function seal(root: string, relativePath: string): Promise<SealedFile> {
  const path = join(root, relativePath);
  const digest = createHash("sha256");
  for await (const chunk of createReadStream(path)) digest.update(chunk);
  const info = statSync(path, { bigint: true });
  return {
    relativePath,
    identity: { device: info.dev.toString(), inode: info.ino.toString(), size: Number(info.size), modifiedNs: info.mtimeNs.toString() },
    sha256: digest.digest("hex"),
  };
}

async function fixture(interactionsCollected = true) {
  const base = mkdtempSync(join(tmpdir(), "apparent-record-pub-peer-"));
  chmodSync(base, 0o700);
  const runtime = join(base, "runtime");
  const captureRoot = join(runtime, "capture-v2");
  const captureId = id();
  const materialRoot = join(captureRoot, "materials", captureId);
  const materialsId = id();
  const leaseId = id();
  const ownerToken = token();
  const publicationId = id();
  const bundleRoot = join(base, "bundles");
  privateDir(bundleRoot);
  const destinationAllocator = new AbsentDestinationAllocator(join(base, "destination-control"), bundleRoot);
  const destination = await destinationAllocator.allocate(`record-publication-peer-${id()}`);
  const { bundleId, bundlePath } = destination;
  const stagingParent = join(base, "record-stage");
  const privateControlRoot = join(base, "record-control");
  privateDir(runtime);
  privateDir(captureRoot);
  privateDir(join(captureRoot, "materials"));
  privateDir(materialRoot);
  privateDir(stagingParent);

  const videoPath = join(materialRoot, "recording.mp4");
  copyFileSync(resolve("engine/core/tests/fixtures/tiny.mp4"), videoPath);
  chmodSync(videoPath, 0o600);
  const videoFile = await seal(materialRoot, "recording.mp4");
  const interactionBytes = Buffer.from(JSON.stringify({
    schema: "apparent.capture-interactions/1",
    identity: { captureId, materialsId },
    events: [{ kind: "app_event", t: 0, label: "synthetic-start" }],
  }));
  let interactionFile: SealedFile | undefined;
  if (interactionsCollected) {
    privateDir(join(materialRoot, "facts"));
    privateFile(join(materialRoot, "facts/interactions.json"), interactionBytes);
    interactionFile = await seal(materialRoot, "facts/interactions.json");
  }
  const payload = {
    schema: "apparent.capture-materials-payload/1",
    identity: { captureId, materialsId },
    bundleId,
    root: materialRoot,
    leaseId,
    clock: { epoch: "capture-admission-monotonic", units: "integer-milliseconds", durationMs: 1, videoOffsetMs: 0 },
    confirmedSource: {
      id: "display:synthetic", generation: id(), confirmedAtMs: 1,
      kind: "display", displayId: 1,
      rectangle: { x: 0, y: 0, width: 16, height: 16, scale: 1 },
      confirmation: { kind: "explicit-display", id: id() },
    },
    invocation: { kind: "explicit", invocationId: id(), observedAtMs: 0 },
    files: [videoFile, ...(interactionFile ? [interactionFile] : [])],
    video: { file: "recording.mp4", width: 16, height: 16, durationMs: 1 },
    facts: interactionFile ? { interactions: { file: interactionFile.relativePath, schema: "apparent.capture-interactions/1" } } : {},
    collection: { context: "unavailable", interactions: interactionsCollected ? "collected" : "unavailable", candidates: "unavailable", transcript: "unavailable", acceptedStrokes: "unavailable" },
    warnings: [],
    safety: { nativeRemoteDelivery: "forbidden", safeForProcessing: true },
  };
  const payloadBytes = Buffer.from(JSON.stringify(payload));
  privateFile(join(materialRoot, "materials.json"), payloadBytes);
  const payloadFile = await seal(materialRoot, "materials.json");
  const materialsManifest = {
    schema: "apparent.capture-materials/1",
    key: { captureId, materialsId, revision: payloadFile.sha256 },
    root: materialRoot,
    leaseId,
    payloadFile,
  };
  const materials = validateMaterialsDocument(materialsManifest, payloadBytes) as MaterialsDocument;

  const imageBytes = readFileSync(resolve("engine/core/tests/fixtures/frame.png"));
  privateDir(join(base, "frames"));
  const framePath = join(base, "frames/frame-01.png");
  const sheetPath = join(base, "contact-sheet.png");
  privateFile(framePath, imageBytes);
  privateFile(sheetPath, imageBytes);
  const artifact = async (path: string, role: SealedArtifact["role"]): Promise<SealedArtifact> => {
    const item = await seal(base, path);
    return { materials: materials.manifest.key, relativePath: path, bytes: readFileSync(join(base, path)), sha256: item.sha256, byteLength: item.identity.size, identity: item.identity, role };
  };
  const artifacts: SealedArtifact[] = [
    await artifact("frames/frame-01.png", "frame"),
    await artifact("contact-sheet.png", "contact-sheet"),
  ];
  const admission = {
    captureId, materialsId, createdAtMs: 1234, bundleId, bundlePath,
    target: { mode: "display" as const, displayId: 1, frame: { x: 0, y: 0, width: 16, height: 16, scale: 1 } },
  };
  const assemblyInput: BundleAssemblyInput = {
    manifest: materialsManifest,
    payloadBytes,
    facts: interactionFile ? { interactions: { file: interactionFile, bytes: interactionBytes } } : {},
    admission,
    frames: [{ index: 1, timeMs: 0, absolutePath: join(bundlePath, "frames/frame-01.png"), artifactPath: "frames/frame-01.png", canonicalReason: "initial", event: "start" }],
    marks: [],
    artifacts,
    materialReferences: [{ role: "video", relativePath: "recording.mp4" }],
  };

  const rootInfo = statSync(materialRoot, { bigint: true });
  privateFile(join(captureRoot, `${captureId}.json`), Buffer.from(JSON.stringify({
    capture_id: captureId,
    owner_token: ownerToken,
    request: { captureId, ownerToken, bundleId, bundlePath },
    state: "materials-ready",
    error: null,
    materials: materials.manifest,
    root: materialRoot,
    materials_id: materialsId,
    lease_id: leaseId,
    root_device: Number(rootInfo.dev),
    root_inode: Number(rootInfo.ino),
    release_phase: null,
    release_movie: videoFile.identity,
    release_payload: payloadFile.identity,
    partial_movie: null,
    payload_digest: payloadFile.sha256,
    admitted_monotonic_ns: null,
    confirmed_at_ms: null,
    cancel_requested: false,
    publication: null,
  })));

  return {
    base, runtime, captureRoot, captureId, materialsId, leaseId, ownerToken, publicationId, bundleId, bundlePath,
    materialRoot, stagingParent, privateControlRoot, materials, materialsManifest, payloadBytes, assemblyInput,
  };
}

async function nativeWorkspace() {
  const base = mkdtempSync(join(tmpdir(), "apparent-record-native-peer-"));
  chmodSync(base, 0o700);
  const runtime = join(base, "runtime");
  const captureRoot = join(runtime, "capture-v2");
  const bundleRoot = join(base, "bundles");
  const stagingParent = join(base, "record-stage");
  const privateControlRoot = join(base, "record-control");
  for (const path of [runtime, captureRoot, bundleRoot, stagingParent]) privateDir(path);
  const allocator = new AbsentDestinationAllocator(join(base, "destination-control"), bundleRoot);
  const destination = await allocator.allocate(`record-native-peer-${id()}`);
  return {
    base, runtime, captureRoot, bundleRoot, stagingParent, privateControlRoot,
    captureId: id(), materialsId: "", leaseId: "", ownerToken: token(), publicationId: id(),
    bundleId: destination.bundleId, bundlePath: destination.bundlePath,
  };
}

async function waitCaptureState(client: CaptureClient, authority: { captureId: string; ownerToken: string }, state: string) {
  const deadline = Date.now() + 20_000;
  let last: Awaited<ReturnType<CaptureClient["inspect"]>> | undefined;
  while (Date.now() < deadline) {
    last = await client.inspect(authority, 5000);
    if (last.state === state) return last;
    if (last.state === "failed" || last.state === "cancelled") throw new Error(`native capture ended as ${last.state}:${last.error?.code ?? "none"}`);
    await delay(40);
  }
  throw new Error(`native capture did not reach ${state}; last=${last?.state ?? "none"}`);
}

test("Record preserves collected interactions and uses the canonical videoOnly profile when unavailable", async () => {
  const collected = await fixture();
  const unavailable = await fixture(false);
  try {
    const collectedAssembly = assembleCanonicalBundle(collected.assemblyInput);
    assert.equal(collectedAssembly.inventory.collection.interactions, "collected");
    assert.equal(collectedAssembly.inventory.entries.find(entry => entry.role === "frame")?.provenance, "caller-attested");
    assert.equal(collectedAssembly.inventory.entries.find(entry => entry.role === "contact-sheet")?.provenance, "caller-attested");

    const unavailableAssembly = assembleCanonicalBundle(unavailable.assemblyInput);
    const manifest = JSON.parse(Buffer.from(unavailableAssembly.apparentJson).toString("utf8"));
    assert.equal(unavailableAssembly.inventory.collection.interactions, "unavailable");
    assert.equal(manifest.interactionTelemetry, "videoOnly");
  } finally {
    rmSync(collected.base, { recursive: true, force: true });
    rmSync(unavailable.base, { recursive: true, force: true });
  }
});

// This fixture injects the pre-receipt crash window; packaged real-capture
// runs select the release-compatible publication case below instead.
test("Record fixture-only assembly and recovery interoperate with native publication and committed restart recovery", { skip: !binary || realCaptureMode }, async () => {
  const f = await fixture(false);
  const oldReadiness = process.env.APPARENT_CAPTURE_TEST_READINESS_UNAVAILABLE;
  const oldDelay = process.env.APPARENT_CAPTURE_TEST_PUBLICATION_AFTER_COMMIT_DELAY_MS;
  let client: CaptureClient | undefined;
  try {
    process.env.APPARENT_CAPTURE_TEST_READINESS_UNAVAILABLE = "1";
    process.env.APPARENT_CAPTURE_TEST_PUBLICATION_AFTER_COMMIT_DELAY_MS = "5000";
    client = new CaptureClient(launchCaptureEngine(resolve(binary!), f.runtime, 3000));
    const hello = await client.handshake(5000);
    assert.equal(hello.implementation, "native");
    assert.equal(hello.capabilities.publication, true);
    assert.equal(hello.readiness, "unavailable", "file-only publication does not depend on screen readiness");

    const assembly = assembleCanonicalBundle(f.assemblyInput);
    const manifest = JSON.parse(Buffer.from(assembly.apparentJson).toString("utf8"));
    assert.equal(manifest.createdAtMs, 1234);
    assert.equal(manifest.durationMs, 1);
    assert.deepEqual(manifest.transcript, { source: "none", text: "", segments: [] });
    assert.deepEqual(assembly.inventory.collection, f.materials.payload.collection);
    assert.equal(assembly.inventory.observations.transcript, "unavailable");
    assert.equal(manifest.interactionTelemetry, "videoOnly");
    assert.equal(assembly.inventory.collection.interactions, "unavailable");
    assert.equal(assembly.inventory.entries.find(entry => entry.role === "frame")?.provenance, "caller-attested");
    assert.equal(assembly.inventory.entries.find(entry => entry.role === "contact-sheet")?.provenance, "caller-attested");

    const prepared = await prepareRecordPublication({
      ownerToken: f.ownerToken,
      publicationId: f.publicationId,
      stagingRoot: f.stagingParent,
      destinationPath: f.bundlePath,
      materialsManifest: f.materialsManifest,
      materialsPayloadBytes: f.payloadBytes,
      assembly,
      artifactBytes: Object.fromEntries(f.assemblyInput.artifacts.map(item => [item.relativePath, item.bytes])),
    });
    const checked = preflightCapturePublication(prepared.input);
    assert.equal(checked.plan.profile, "apparent-bundle/1");
    assert.equal(checked.plan.destination.mode, "absent");
    assert.equal(checked.plan.entries.length, assembly.inventory.entries.length);
    assert.ok(!checked.plan.entries.some(entry => entry.relativePath === ".record-publication" || entry.relativePath === "publication-plan.json"));

    const recovery = createRecordPublicationRecovery(client, f.privateControlRoot, preflightCapturePublication);
    const pendingPublish = recovery.publish(prepared.input).then(value => ({ value }), error => ({ error }));
    const commitDeadline = Date.now() + 15_000;
    let committed = false;
    let dispatchFailure: unknown;
    let observation: unknown;
    while (Date.now() < commitDeadline) {
      const completed = await Promise.race([pendingPublish, delay(20).then(() => undefined)]);
      if (completed && "error" in completed) {
        dispatchFailure = completed.error;
        try {
          observation = await client.inspectPublication({ ownerToken: f.ownerToken, captureId: f.captureId, publicationId: f.publicationId, planSha256: prepared.input.planFile.planSha256, expected: { materials: f.materials.manifest.key, bundleId: f.bundleId, bundlePath: f.bundlePath, manifestSha256: hash(assembly.apparentJson) } }, 2000);
        } catch (error) { observation = error; }
      }
      try {
        committed = statSync(f.bundlePath).isDirectory();
        if (committed) break;
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
      }
      await delay(10);
    }
    assert.equal(committed, true, `Core must commit the complete Record inventory before provider acknowledgement; dispatch=${String(dispatchFailure)} observation=${JSON.stringify(observation)}`);
    const publishing = await client.inspectPublication({
      ownerToken: f.ownerToken,
      captureId: f.captureId,
      publicationId: f.publicationId,
      planSha256: prepared.input.planFile.planSha256,
      expected: { materials: f.materials.manifest.key, bundleId: f.bundleId, bundlePath: f.bundlePath, manifestSha256: hash(assembly.apparentJson) },
    }, 3000);
    assert.equal(publishing.state, "publishing", "Record must not treat the pre-receipt commit as acknowledged Saved");

    rmSync(prepared.input.planFile.stagingRoot, { recursive: true, force: true });
    rmSync(f.materialRoot, { recursive: true, force: true });
    const child = (client as unknown as { transport: { child: {
      kill(signal: "SIGKILL"): boolean;
      once(event: "exit", listener: () => void): void;
    } } }).transport.child;
    await new Promise<void>(resolveExit => {
      child.once("exit", () => resolveExit());
      child.kill("SIGKILL");
    });
    assert.ok("error" in await pendingPublish, "the provider process interruption must lose the in-flight reply");
    await client.close();
    client = undefined;

    if (oldDelay === undefined) delete process.env.APPARENT_CAPTURE_TEST_PUBLICATION_AFTER_COMMIT_DELAY_MS;
    else process.env.APPARENT_CAPTURE_TEST_PUBLICATION_AFTER_COMMIT_DELAY_MS = oldDelay;
    client = new CaptureClient(launchCaptureEngine(resolve(binary!), f.runtime, 3000));
    await client.handshake(5000);
    const reopenedRecovery = createRecordPublicationRecovery(client, f.privateControlRoot, preflightCapturePublication);
    const saved = await reopenedRecovery.reconcile(f.captureId, f.ownerToken, {
      publicationId: f.publicationId,
      materials: f.materials.manifest.key,
      leaseId: f.leaseId,
      bundleId: f.bundleId,
      bundlePath: f.bundlePath,
    });
    assert.equal(saved.state, "saved");
    assert.equal(saved.publication?.manifestSha256, hash(assembly.apparentJson));
    assert.deepEqual(readFileSync(join(f.bundlePath, "apparent.json")), Buffer.from(assembly.apparentJson));
    assert.deepEqual(readFileSync(join(f.bundlePath, "summary.md")), Buffer.from(assembly.summaryMd));
    assert.ok(statSync(join(f.bundlePath, "recording.mp4")).isFile());
    assert.ok(statSync(join(f.bundlePath, "frames/frame-01.png")).isFile());
    assert.ok(statSync(join(f.bundlePath, "contact-sheet.png")).isFile());
    assert.throws(() => statSync(join(f.bundlePath, ".record-publication")), { code: "ENOENT" });
    assert.throws(() => statSync(join(f.bundlePath, "publication-plan.json")), { code: "ENOENT" });
  } finally {
    await client?.close();
    if (oldReadiness === undefined) delete process.env.APPARENT_CAPTURE_TEST_READINESS_UNAVAILABLE;
    else process.env.APPARENT_CAPTURE_TEST_READINESS_UNAVAILABLE = oldReadiness;
    if (oldDelay === undefined) delete process.env.APPARENT_CAPTURE_TEST_PUBLICATION_AFTER_COMMIT_DELAY_MS;
    else process.env.APPARENT_CAPTURE_TEST_PUBLICATION_AFTER_COMMIT_DELAY_MS = oldDelay;
    rmSync(f.base, { recursive: true, force: true });
  }
});

test("Record publishes explicitly selected native media2 frames and sheet from retained materials", { skip: !binary || !mediaBinary }, async () => {
  const realCapture = realCaptureMode;
  const f: any = realCapture ? await nativeWorkspace() : await fixture(false);
  let capture: CaptureClient | undefined;
  let media: Media2Client | undefined;
  let xserver: ReturnType<typeof spawn> | undefined;
  const oldDisplay = process.env.DISPLAY;
  const oldWayland = process.env.WAYLAND_DISPLAY;
  const oldGstDebug = process.env.GST_DEBUG;
  const oldGstDebugFile = process.env.GST_DEBUG_FILE;
  let read: Awaited<ReturnType<typeof readCaptureMaterials>>;
  let createdAtMs = Date.now();
  let scenarioPassed = false;
  try {
    privateDir(f.privateControlRoot);

    if (realCapture) {
      const xvfb = process.env.APPARENT_XVFB ?? "Xvfb";
      const xsetroot = process.env.APPARENT_XSETROOT ?? "xsetroot";
      let displayNumber = 101 + Math.floor(Math.random() * 80);
      let foundDisplay = false;
      for (let attempt = 0; attempt < 80; attempt++) {
        try { statSync(`/tmp/.X${displayNumber}-lock`); displayNumber = 101 + ((displayNumber - 100) % 80); }
        catch (error) { if ((error as NodeJS.ErrnoException).code === "ENOENT") { foundDisplay = true; break; } throw error; }
      }
      assert.ok(foundDisplay, "a private Xvfb display must be available for the Record publication scenario");
      const display = `:${displayNumber}`;
      xserver = spawn(xvfb, [display, "-screen", "0", "1366x768x24", "-nolisten", "tcp"], { stdio: "ignore" });
      process.env.DISPLAY = display;
      delete process.env.WAYLAND_DISPLAY;
      await delay(500);
      execFileSync(xsetroot, ["-solid", "#cc2211"]);
      capture = new CaptureClient(launchCaptureEngine(resolve(binary!), f.runtime, 3000));
      const captureHello = await capture.handshake(5000);
      assert.equal(captureHello.implementation, "native");
      assert.equal(captureHello.capabilities.publication, true);
      const sources = await capture.discoverSources(5000);
      assert.equal(sources.state, "available");
      const source = sources.sources.find(item => item.kind === "display");
      assert.ok(source, "the owned synthetic Xvfb display must be discoverable through CaptureClient");
      assert.deepEqual(source.bounds, { x: 0, y: 0, width: 1366, height: 768, scale: 1 });
      const invocationId = id();
      createdAtMs = Date.now();
      const started = await capture.start({
        captureId: f.captureId, ownerToken: f.ownerToken, bundleId: f.bundleId, bundlePath: f.bundlePath,
        source, options: { audio: "none", localOnly: true },
        invocation: { kind: "explicit", invocationId, observedAtMs: createdAtMs },
      }, 5000);
      assert.ok(["starting", "recording"].includes(started.state));
      await waitCaptureState(capture, { captureId: f.captureId, ownerToken: f.ownerToken }, "recording");
      for (const color of ["#e65b32", "#37c6a1", "#a842d4"]) {
        execFileSync(xsetroot, ["-solid", color]);
        await delay(180);
      }
      await capture.stop({ captureId: f.captureId, ownerToken: f.ownerToken }, 5000);
      const stopped = await waitCaptureState(capture, { captureId: f.captureId, ownerToken: f.ownerToken }, "materials-ready");
      assert.ok(stopped.materials, "native stop must return authenticated retained materials");
      const manifest = stopped.materials;
      f.materialsId = manifest.key.materialsId;
      f.leaseId = manifest.leaseId;
      read = await readCaptureMaterials({
        manifest,
        expected: { key: manifest.key, leaseId: manifest.leaseId, bundleId: f.bundleId },
        trustedRoot: manifest.root,
        limits: { maxAggregateBytes: 128 * 1024 * 1024 },
      });
      assert.equal(read.materials.payload.identity.captureId, f.captureId);
      assert.equal(read.materials.payload.confirmedSource.id, source.id);
      assert.deepEqual(read.materials.payload.confirmedSource.rectangle, source.bounds);
      assert.equal(read.materials.payload.video.width, 1366);
      assert.equal(read.materials.payload.video.height, 768);
      assert.ok(read.materials.payload.clock.durationMs > 0);
      assert.ok(read.materials.payload.clock.videoOffsetMs >= 0);
      assert.ok(Object.values(read.materials.payload.collection).every(value => value === "unavailable"), "the synthetic X11 case supplies no caller-created interaction facts");
      assert.equal(read.materials.payload.invocation.invocationId, invocationId);
      const moviePath = join(read.materials.manifest.root, read.materials.payload.video.file);
      const ffmpeg = process.env.APPARENT_FFMPEG ?? "ffmpeg";
      const frameHashes = execFileSync(ffmpeg, ["-v", "error", "-i", moviePath, "-f", "framemd5", "-"], { encoding: "utf8" })
        .split("\n").filter(line => line && !line.startsWith("#")).map(line => line.split(",").at(-1)?.trim()).filter(Boolean);
      assert.ok(new Set(frameHashes).size > 1, "native retained movie contains the changing synthetic X11 scene");
    } else {
      capture = new CaptureClient(launchCaptureEngine(resolve(binary!), f.runtime, 3000));
      const captureHello = await capture.handshake(5000);
      assert.equal(captureHello.implementation, "native");
      assert.equal(captureHello.capabilities.publication, true);
      read = await readCaptureMaterials({
        manifest: f.materials.manifest,
        expected: { key: f.materials.manifest.key, leaseId: f.leaseId, bundleId: f.bundleId },
        trustedRoot: f.materialRoot,
      });
      assert.deepEqual(read.materials.manifest.key, f.materials.manifest.key);
      assert.equal(read.materials.payload.leaseId, f.leaseId);
    }

    const outputDirectory = join(f.base, "native-media2-output");
    const frameWidth = realCapture ? 320 : 16;
    const frameHeight = realCapture ? Math.round(frameWidth * read.materials.payload.video.height / read.materials.payload.video.width) : 16;
    const firstTimeMs = realCapture ? Math.min(read.materials.payload.clock.durationMs, read.materials.payload.clock.videoOffsetMs) : 0;
    const secondTimeMs = read.materials.payload.clock.durationMs;
    const sheetWidth = frameWidth * 2;
    const request = {
      schema: "apparent.retained-frame-request/1" as const,
      materials: read.materials.manifest.key,
      leaseId: read.materials.payload.leaseId,
      jobId: id(),
      outputDirectory,
      requestedBundleMs: [firstTimeMs, secondTimeMs],
      limits: { maxFrames: 2, maxEdge: frameWidth, maxTotalPixels: frameWidth * frameHeight * 4, deadlineMs: 20_000 },
      contactSheet: {
        schema: "apparent.contact-sheet-layout/1" as const,
        width: sheetWidth,
        height: frameHeight,
        backgroundRgb: [11, 22, 33] as [number, number, number],
        fit: "stretch" as const,
        sampling: "legacy-bilinear" as const,
        clipping: "reject" as const,
        tiles: [
          { requestIndex: 1, x: 0, y: 0, width: frameWidth, height: frameHeight },
          { requestIndex: 2, x: frameWidth, y: 0, width: frameWidth, height: frameHeight },
        ],
      },
    };
    const layoutBytes = encodeContactSheetLayout(request.contactSheet);
    const preparedRequest = prepareMedia2Request(request, read.materials);
    assert.equal(preparedRequest.request.materials.revision, read.materials.manifest.key.revision);
    assert.equal(preparedRequest.request.leaseId, read.materials.manifest.leaseId);

    media = new Media2Client(launchMedia2Worker(resolve(mediaBinary!), f.runtime));
    const mediaHello = await media.handshake(5000);
    assert.equal(mediaHello.implementation, "native");
    assert.deepEqual(mediaHello.capabilities, { retainedFrames: true, contactSheet: true, nativeJobAuthority: true });
    const recordJob = await RecordMedia2Job.create(media, join(f.privateControlRoot, "media2-job.json"), {
      ownerToken: f.ownerToken,
      request: preparedRequest.request,
      materials: read.materials,
    });
    assert.equal(recordJob.requestSha256, preparedRequest.sha256);
    let jobStatus = await recordJob.derive(f.ownerToken, 15_000);
    const jobDeadline = Date.now() + 20_000;
    while (jobStatus.state !== "complete" && jobStatus.state !== "failed" && jobStatus.state !== "cancelled" && Date.now() < jobDeadline) {
      await delay(20);
      jobStatus = await recordJob.inspect(f.ownerToken, 5000);
    }
    assert.equal(jobStatus.state, "complete", `native retained job must complete before Record reads its outputs; got ${jobStatus.state}`);
    const derivative = recordJob.derivative;
    assert.ok(derivative, "RecordMedia2Job retains the exact completed native document");
    const envelope = derivative.envelope as Derivative2Envelope;
    assert.deepEqual(envelope.payload.materials, read.materials.manifest.key);
    assert.equal(envelope.payload.leaseId, read.materials.manifest.leaseId);
    assert.equal(envelope.payload.jobId, request.jobId);
    assert.equal(envelope.payload.requestSha256, preparedRequest.sha256);
    assert.equal(envelope.payload.root, outputDirectory);
    assert.deepEqual(envelope.payload.frames.map(frame => frame.requestedBundleMs), request.requestedBundleMs);
    assert.deepEqual(envelope.payload.frames.map(frame => frame.requestIndex), [1, 2]);
    assert.equal(envelope.payload.contactSheet?.layoutSha256, hash(layoutBytes));
    assert.deepEqual(Buffer.from(derivative.layoutBytes!), Buffer.from(layoutBytes), "Record keeps the exact canonical detached layout bytes");
    assert.equal(envelope.payload.contactSheet?.width, sheetWidth);
    assert.equal(envelope.payload.contactSheet?.height, frameHeight);
    for (const image of [...envelope.payload.frames, envelope.payload.contactSheet!]) {
      const original = readFileSync(join(outputDirectory, image.file.relativePath));
      assert.equal(original.subarray(0, 8).toString("hex"), "89504e470d0a1a0a");
      assert.equal(original.length, image.byteLength);
      assert.equal(original.readUInt32BE(16), image.width);
      assert.equal(original.readUInt32BE(20), image.height);
      assert.equal(hash(original), image.file.sha256);
      const info = statSync(join(outputDirectory, image.file.relativePath), { bigint: true });
      assert.deepEqual({ device: info.dev.toString(), inode: info.ino.toString(), size: Number(info.size), modifiedNs: info.mtimeNs.toString() }, image.file.identity);
    }

    const retained = await readRetainedDerivativeArtifacts({
      document: derivative,
      materials: read.materials,
      trustedRoot: outputDirectory,
      mappings: [
        { bundlePath: "frames/frame-01.png", selector: { kind: "frame", requestIndex: 1 } },
        { bundlePath: "frames/frame-02.png", selector: { kind: "frame", requestIndex: 2 } },
        { bundlePath: "contact-sheet.png", selector: { kind: "contact-sheet" } },
      ],
      limits: { maxFileBytes: 1024 * 1024, maxAggregateBytes: 3 * 1024 * 1024 },
    });
    assert.deepEqual(Object.keys(retained.derivative2Artifacts).sort(), ["contact-sheet.png", "frames/frame-01.png", "frames/frame-02.png"]);
    assert.deepEqual(retained.derivative2Artifacts["frames/frame-01.png"], { receiptId: envelope.payload.receiptId, selector: { kind: "frame", requestIndex: 1 } });
    assert.deepEqual(retained.derivative2Artifacts["frames/frame-02.png"], { receiptId: envelope.payload.receiptId, selector: { kind: "frame", requestIndex: 2 } });
    assert.deepEqual(retained.derivative2Artifacts["contact-sheet.png"], { receiptId: envelope.payload.receiptId, selector: { kind: "contact-sheet" } });
    for (const artifact of retained.artifacts) {
      const selected: Derivative2Envelope["payload"]["contactSheet"] | Derivative2Envelope["payload"]["frames"][number] | undefined = artifact.role === "contact-sheet"
        ? envelope.payload.contactSheet
        : envelope.payload.frames.find(frame => frame.file.sha256 === artifact.sha256 && frame.file.identity.inode === artifact.identity.inode);
      assert.ok(selected, "Record reader output is an original selected native seal");
      assert.equal(artifact.sha256, selected.file.sha256);
      assert.deepEqual(artifact.identity, selected.file.identity);
      assert.deepEqual(artifact.materials, read.materials.manifest.key);
    }

    const confirmed = read.materials.payload.confirmedSource;
    const target = confirmed.kind === "display"
      ? { mode: "display" as const, displayId: confirmed.displayId, frame: confirmed.rectangle }
      : { mode: "region" as const, displayId: confirmed.displayId, frame: confirmed.rectangle };
    const assemblyInput: BundleAssemblyInput = {
      manifest: read.materials.manifest,
      payloadBytes: read.payloadBytes,
      facts: read.assemblyFacts,
      admission: {
        captureId: read.materials.payload.identity.captureId,
        materialsId: read.materials.payload.identity.materialsId,
        createdAtMs,
        bundleId: read.materials.payload.bundleId,
        bundlePath: f.bundlePath,
        target,
      },
      frames: [
        { index: 1, timeMs: request.requestedBundleMs[0], absolutePath: join(f.bundlePath, "frames/frame-01.png"), artifactPath: "frames/frame-01.png", canonicalReason: "initial", event: "explicit Record selection from native media2 request" },
        { index: 2, timeMs: request.requestedBundleMs[1], absolutePath: join(f.bundlePath, "frames/frame-02.png"), artifactPath: "frames/frame-02.png", canonicalReason: "scene_change", event: "explicit Record selection from native media2 request" },
      ],
      marks: [],
      artifacts: retained.artifacts,
      materialReferences: [{ role: "video", relativePath: read.materials.payload.video.file }],
    };
    const assembly = assembleCanonicalBundle(assemblyInput);
    if (realCapture) {
      const canonical = JSON.parse(Buffer.from(assembly.apparentJson).toString("utf8"));
      assert.equal(canonical.interactionTelemetry, "videoOnly", "Record applies its canonical profile to the genuinely unavailable native interaction facts");
      assert.deepEqual(read.materials.payload.confirmedSource.rectangle, target.frame, "Record admission uses the original confirmed source geometry");
    }
    const prepared = await prepareRecordPublication({
      ownerToken: f.ownerToken,
      publicationId: f.publicationId,
      stagingRoot: f.stagingParent,
      destinationPath: f.bundlePath,
      materialsManifest: read.materials.manifest,
      materialsPayloadBytes: read.payloadBytes,
      assembly,
      artifactBytes: Object.fromEntries(assemblyInput.artifacts.map(item => [item.relativePath, item.bytes])),
      derivatives2: [derivative],
      derivative2Artifacts: retained.derivative2Artifacts,
    });
    const checked = preflightCapturePublication(prepared.input);
    assert.deepEqual(checked.plan, prepared.plan);
    for (const [relativePath, selector] of [
      ["frames/frame-01.png", { kind: "frame", requestIndex: 1 }],
      ["frames/frame-02.png", { kind: "frame", requestIndex: 2 }],
      ["contact-sheet.png", { kind: "contact-sheet" }],
    ] as const) {
      const entry = checked.plan.entries.find(item => item.relativePath === relativePath);
      assert.ok(entry);
      assert.equal(entry.input.kind, "derivative2", "Record plan retains native derivative selector provenance");
      if (entry.input.kind === "derivative2") {
        assert.equal(entry.input.receipt.receiptId, envelope.payload.receiptId);
        assert.deepEqual(entry.input.selector, selector);
      }
    }
    assert.equal(checked.plan.entries.find(item => item.relativePath === "recording.mp4")?.input.kind, "material");

    const recovery = createRecordPublicationRecovery(capture, f.privateControlRoot, preflightCapturePublication);
    const saved = await recovery.publish(prepared.input);
    assert.equal(saved.state, "saved", "only the authenticated native publisher receipt establishes Saved");
    assert.equal(saved.publication?.manifestSha256, hash(assembly.apparentJson));
    assert.deepEqual(readFileSync(join(f.bundlePath, "apparent.json")), Buffer.from(assembly.apparentJson));
    assert.deepEqual(readFileSync(join(f.bundlePath, "summary.md")), Buffer.from(assembly.summaryMd));
    for (const artifact of retained.artifacts) {
      assert.deepEqual(readFileSync(join(f.bundlePath, artifact.relativePath)), Buffer.from(artifact.bytes));
    }
    const publishedVideo = await seal(f.bundlePath, "recording.mp4");
    const sourceVideo = read.materials.payload.files.find(file => file.relativePath === read.materials.payload.video.file);
    assert.ok(sourceVideo);
    assert.equal(publishedVideo.sha256, sourceVideo.sha256);
    assert.equal(publishedVideo.identity.size, sourceVideo.identity.size, "Record publishes the sealed media bytes into a new owned file");
    assert.throws(() => statSync(join(f.bundlePath, ".record-publication")), { code: "ENOENT" });
    assert.throws(() => statSync(join(f.bundlePath, "publication-plan.json")), { code: "ENOENT" });
    if (realCapture) {
      const retainedMovie = await seal(read.materials.manifest.root, read.materials.payload.video.file);
      assert.deepEqual(retainedMovie.identity, sourceVideo.identity, "original native movie seal remains unchanged through publication");
      assert.equal(retainedMovie.sha256, sourceVideo.sha256);

      const evidence = process.env.APPARENT_CAPTURE_EVIDENCE_DIR;
      if (evidence) {
        const root = resolve(evidence);
        privateDir(root);
        mkdirSync(join(root, "bundle"), { recursive: true, mode: 0o700 });
        mkdirSync(join(root, "native"), { recursive: true, mode: 0o700 });
        for (const name of ["apparent.json", "summary.md", "recording.mp4"]) copyFileSync(join(f.bundlePath, name), join(root, "bundle", name));
        for (const image of retained.artifacts) {
          const destination = join(root, "bundle", image.relativePath);
          mkdirSync(join(destination, ".."), { recursive: true, mode: 0o700 });
          copyFileSync(join(f.bundlePath, image.relativePath), destination);
        }
        copyFileSync(join(read.materials.manifest.root, read.materials.manifest.payloadFile.relativePath), join(root, "native", "materials.json"));
        writeFileSync(join(root, "native", "materials-manifest.json"), `${JSON.stringify(read.materials.manifest, null, 2)}\n`, { mode: 0o600, flag: "wx" });
        writeFileSync(join(root, "native", "media2-request.json"), `${JSON.stringify(request, null, 2)}\n`, { mode: 0o600, flag: "wx" });
        writeFileSync(join(root, "native", "media2-receipt.json"), derivative.bytes, { mode: 0o600, flag: "wx" });
        writeFileSync(join(root, "native", "media2-envelope.json"), `${JSON.stringify(envelope, null, 2)}\n`, { mode: 0o600, flag: "wx" });
        writeFileSync(join(root, "native", "evidence.json"), `${JSON.stringify({
          captureId: f.captureId, materials: read.materials.manifest.key, leaseId: read.materials.manifest.leaseId,
          jobId: request.jobId, requestSha256: preparedRequest.sha256, receiptId: envelope.payload.receiptId,
          publicationId: f.publicationId, bundleId: f.bundleId, bundlePath: f.bundlePath,
          movie: { relativePath: read.materials.payload.video.file, sha256: sourceVideo.sha256, identity: sourceVideo.identity },
          frames: envelope.payload.frames.map(frame => ({ requestIndex: frame.requestIndex, requestedBundleMs: frame.requestedBundleMs, file: frame.file })),
          contactSheet: envelope.payload.contactSheet?.file,
          saved: { manifestSha256: saved.publication?.manifestSha256, inventorySha256: saved.publication?.inventorySha256 },
        }, null, 2)}\n`, { mode: 0o600, flag: "wx" });
      }

      const released = await capture.releaseMaterials({ ownerToken: f.ownerToken, captureId: f.captureId, materialsId: f.materialsId, leaseId: f.leaseId }, 5000);
      assert.equal(released.released, true, "authenticated release removes only this scenario's retained materials after Saved");
      assert.equal(released.materialsId, f.materialsId);
      rmSync(prepared.input.planFile.stagingRoot, { recursive: true, force: true });
      await capture.close();
      capture = new CaptureClient(launchCaptureEngine(resolve(binary!), f.runtime, 3000));
      await capture.handshake(5000);
      const recoveredSaved = await capture.inspectPublication({
        ownerToken: f.ownerToken, captureId: f.captureId, publicationId: f.publicationId,
        planSha256: prepared.input.planFile.planSha256,
        expected: { materials: read.materials.manifest.key, bundleId: f.bundleId, bundlePath: f.bundlePath, manifestSha256: hash(assembly.apparentJson) },
      }, 5000);
      assert.equal(recoveredSaved.state, "saved", "authenticated reconnect retains Saved after source release and staging removal");
      assert.deepEqual(recoveredSaved.publication, saved.publication);
      scenarioPassed = true;
      console.log("Real synthetic X11 capture -> retained materials -> native media2 -> Record staging -> authenticated Saved and cold inspect passed");
    }
  } finally {
    await media?.close();
    await capture?.close();
    xserver?.kill("SIGTERM");
    if (oldDisplay === undefined) delete process.env.DISPLAY; else process.env.DISPLAY = oldDisplay;
    if (oldWayland === undefined) delete process.env.WAYLAND_DISPLAY; else process.env.WAYLAND_DISPLAY = oldWayland;
    if (oldGstDebug === undefined) delete process.env.GST_DEBUG; else process.env.GST_DEBUG = oldGstDebug;
    if (oldGstDebugFile === undefined) delete process.env.GST_DEBUG_FILE; else process.env.GST_DEBUG_FILE = oldGstDebugFile;
    if (!realCapture || scenarioPassed) rmSync(f.base, { recursive: true, force: true });
    else process.stderr.write(`Record native failure workspace retained: ${f.base}\n`);
  }
});
