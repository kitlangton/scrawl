import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { chmod, lstat, mkdir, mkdtemp, readFile, readdir, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import test from "node:test";
import { assembleCanonicalBundle, type BundleAssemblyInput } from "@apparent/record/processing/bundle";
import { prepareRecordPublication } from "../../dist/backend/publication/staging.js";
import { createCompletedCapturePublisher } from "../../dist/backend/publication/completed-capture.js";
import { createRecordPublicationOutput } from "../../dist/backend/publication/output.js";
import { CaptureClient, CapturePublicationPreflightError, preflightCapturePublication, validateFactFile, validateMaterialsDocument, type MaterialsDocument, type StrokeFacts } from "@apparent/engine-sdk/capture";
import { createRecordPublicationRecovery } from "../../dist/backend/publication/recovery.js";
import { ReferenceCapturePeer } from "@apparent/engine-sdk/capture/testing";
import { SqlitePublicationBindingStore } from "../../dist/backend/publication/store.js";
import { Media2Client, type Media2RpcRequest, encodeContactSheetLayout, prepareMedia2Request, selectDerivative2Artifact, validateDerivative2Document, type Derivative2DocumentInput } from "@apparent/engine-sdk/media2";
import { readRetainedDerivativeArtifacts } from "../../dist/backend/processing/retained-artifacts.js";
import { createRecordCaptureCoordinator } from "../../dist/backend/host/capture2/coordinator.js";
import { AbsentDestinationAllocator } from "../../dist/backend/host/absent-destination-allocator.js";

const identity = { captureId: "a".repeat(32), materialsId: "b".repeat(32) };
const bytes = (value: unknown) => new TextEncoder().encode(JSON.stringify(value));
const sha = (value: Uint8Array) => createHash("sha256").update(value).digest("hex");
const sealed = (relativePath: string, content: Uint8Array, inode: string) => ({
  relativePath, bytes: content, sha256: sha(content), byteLength: content.byteLength,
  identity: { device: "1", inode, size: content.byteLength, modifiedNs: "3" },
});

async function fixture(): Promise<BundleAssemblyInput & { manifest: MaterialsDocument["manifest"] }> {
  const root = "/private/sdk-valid-materials";
  const source = { id: "display-1", generation: "d".repeat(32), confirmedAtMs: 0, kind: "display" as const,
    displayId: 1, rectangle: { x: 0, y: 0, width: 800, height: 600, scale: 1 },
    confirmation: { kind: "explicit-display" as const, id: "e".repeat(32) } };
  const invocation = { kind: "explicit" as const, invocationId: "f".repeat(32), observedAtMs: 0 };
  const videoBytes = new TextEncoder().encode("synthetic-video-material");
  const factDocuments = {
    context: { schema: "apparent.capture-context/1", identity, source, invocation,
      context: { state: "collected", value: { values: { workspace: "demo" }, changes: [{ timeMs: 600, key: "mode", value: "edit" }] } },
      session: { state: "collected", value: { id: "session-sdk-test" } },
      windows: { state: "collected", value: [{ timeMs: 700, entries: [{ id: 2, app: "Editor", title: "Synthetic window", bounds: [0, 0, 400, 300], layer: 0, frontmost: true }] }] },
      cursor: { state: "collected", value: [[900, 0.25, 0.5]] } },
    interactions: { schema: "apparent.capture-interactions/1", identity,
      events: [{ kind: "app_change", t: 800, app: "Editor", window_title: "Synthetic window" },
        { kind: "click", t: 900, x: 0.4, y: 0.5, button: "left", label: "Save" }] },
    transcript: { schema: "apparent.capture-transcript/1", identity, provider: "fixture-provider", model: "fixture-model-v7",
      text: "hello there", segments: [{ startMs: 200, endMs: 1200, text: "hello there" }] },
  };
  const factBytes = Object.fromEntries(Object.entries(factDocuments).map(([kind, document]) => [kind, bytes(document)])) as Record<keyof typeof factDocuments, Uint8Array>;
  const factFiles = Object.fromEntries(Object.entries(factBytes).map(([kind, content]) => [kind, sealed(`facts/${kind}.json`, content, `inode-${kind}`)])) as Record<keyof typeof factDocuments, ReturnType<typeof sealed>>;
  const videoFile = sealed("capture.mp4", videoBytes, "inode-video");
  const png = new Uint8Array(await readFile(new URL("../fixtures/take.apparent/frames/frame-01.png", import.meta.url)));
  const frameFile = sealed("frames/frame-01.png", png, "inode-frame");
  const sheetBytes = new Uint8Array(await readFile(new URL("../fixtures/take.apparent/contact-sheet.png", import.meta.url)));
  const sheetFile = sealed("contact-sheet.png", sheetBytes, "inode-sheet");
  const files = [videoFile, ...Object.values(factFiles)];
  const payload = {
    schema: "apparent.capture-materials-payload/1", identity, bundleId: "sdk-bundle", root, leaseId: "c".repeat(32),
    clock: { epoch: "capture-admission-monotonic", units: "integer-milliseconds", durationMs: 4000, videoOffsetMs: 125 },
    confirmedSource: source, invocation, files: files.map(({ relativePath, sha256, identity: fileIdentity }) => ({ relativePath, sha256, identity: fileIdentity })),
    video: { file: videoFile.relativePath, width: 800, height: 600, durationMs: 4000 },
    facts: { context: { file: factFiles.context.relativePath, schema: "apparent.capture-context/1" },
      interactions: { file: factFiles.interactions.relativePath, schema: "apparent.capture-interactions/1" },
      transcript: { file: factFiles.transcript.relativePath, schema: "apparent.capture-transcript/1" } },
    collection: { context: "collected", interactions: "collected", candidates: "unavailable", transcript: "collected", acceptedStrokes: "unavailable" },
    warnings: [], safety: { nativeRemoteDelivery: "forbidden", safeForProcessing: true },
  };
  const payloadBytes = bytes(payload);
  const manifest: MaterialsDocument["manifest"] = { schema: "apparent.capture-materials/1", key: { ...identity, revision: sha(payloadBytes) }, root,
    leaseId: payload.leaseId, payloadFile: { relativePath: "materials.json", sha256: sha(payloadBytes),
      identity: { device: "1", inode: "inode-materials", size: payloadBytes.byteLength, modifiedNs: "3" } } };
  return {
    manifest, payloadBytes,
    facts: Object.fromEntries(Object.entries(factFiles).map(([kind, file]) => [kind, { file, bytes: factBytes[kind as keyof typeof factBytes] }])),
    admission: { ...identity, createdAtMs: 1234, bundleId: "sdk-bundle", bundlePath: "/captures/sdk-bundle.apparent",
      target: { mode: "display", displayId: 1, frame: { x: 0, y: 0, width: 800, height: 600, scale: 1 }, app: "Editor" },
      destination: { session: "session-sdk-test", title: "", via: "sdk" } },
    frames: [{ index: 1, timeMs: 1000, absolutePath: "/captures/sdk-bundle.apparent/frames/frame-01.png", artifactPath: frameFile.relativePath, canonicalReason: "scene_change" }],
    marks: [], artifacts: [
      { ...videoFile, materials: manifest.key, role: "video" as const }, { ...frameFile, materials: manifest.key, role: "frame" as const },
      { ...sheetFile, materials: manifest.key, role: "contact-sheet" as const },
    ],
    transcriptBounds: { policy: "estimated", basis: "SDK fixture provides explicit segment end time" },
  };
}

function addAudioMaterial(input: BundleAssemblyInput & { manifest: MaterialsDocument["manifest"] }) {
  const payload = JSON.parse(new TextDecoder().decode(input.payloadBytes));
  const audioBytes = new TextEncoder().encode("synthetic-audio-material");
  const audioFile = sealed("capture.m4a", audioBytes, "inode-audio");
  payload.files.push({ relativePath: audioFile.relativePath, sha256: audioFile.sha256, identity: audioFile.identity });
  payload.audio = { file: audioFile.relativePath, durationMs: 4000, sampleRate: 48000, channels: 2 };
  payload.clock.audioOffsetMs = 0;
  input.payloadBytes = bytes(payload);
  input.manifest.key.revision = sha(input.payloadBytes);
  input.manifest.payloadFile.sha256 = sha(input.payloadBytes);
  input.manifest.payloadFile.identity.size = input.payloadBytes.byteLength;
  for (const artifact of input.artifacts) artifact.materials = { ...input.manifest.key };
  return audioFile;
}

function stagingInput(input: BundleAssemblyInput, assembly: ReturnType<typeof assembleCanonicalBundle>, root: string,
  publicationId = "1".repeat(32), destinationPath = join(root, "destination.apparent")) {
  return { ownerToken: "c".repeat(64), publicationId, stagingRoot: join(root, "private"), destinationPath,
    materialsManifest: input.manifest, materialsPayloadBytes: input.payloadBytes, assembly,
    artifactBytes: Object.fromEntries(input.artifacts.map(artifact => [artifact.relativePath, artifact.bytes])) };
}

function assemblyForDestination(input: BundleAssemblyInput, destinationPath: string) {
  input.admission.bundlePath = destinationPath;
  for (const frame of input.frames) frame.absolutePath = join(destinationPath, frame.artifactPath);
  return assembleCanonicalBundle(input);
}

// Offline fixture only: the receipt metadata binds known synthetic PNG bytes and grants no native origin authority.
function derivative2For(input: BundleAssemblyInput, requestedBundleMs = [1000], options: {
  outputDirectory?: string;
  frameFiles?: Array<{ relativePath: string; identity: { device: string; inode: string; size: number; modifiedNs: string }; sha256: string }>;
  sheetFile?: { relativePath: string; identity: { device: string; inode: string; size: number; modifiedNs: string }; sha256: string };
  jobId?: string;
} = {}): Derivative2DocumentInput {
  const materials = validateMaterialsDocument(input.manifest, input.payloadBytes);
  const layout = { schema: "apparent.contact-sheet-layout/1" as const, width: 400, height: 300, backgroundRgb: [0, 0, 0] as [number, number, number], fit: "stretch" as const, sampling: "legacy-bilinear" as const, clipping: "reject" as const, tiles: [{ requestIndex: 1, x: 0, y: 0, width: 400, height: 300 }] };
  const request = prepareMedia2Request({ schema: "apparent.retained-frame-request/1", materials: materials.manifest.key, leaseId: materials.payload.leaseId,
    jobId: options.jobId ?? "9".repeat(32), outputDirectory: options.outputDirectory ?? "/private/sdk-output/derivative-job", requestedBundleMs,
    limits: { maxFrames: 2, maxEdge: 4096, maxTotalPixels: 64_000_000, deadlineMs: 10_000 }, contactSheet: layout }, materials).request;
  const sourceFrame = input.artifacts.find(artifact => artifact.role === "frame")!;
  const sourceSheet = input.artifacts.find(artifact => artifact.role === "contact-sheet")!;
  const frameFiles = options.frameFiles ?? requestedBundleMs.map((_, index) => ({ relativePath: `native/frame-${index + 1}.png`,
    identity: index === 0 ? sourceFrame.identity : { ...sourceFrame.identity, inode: `native-frame-${index + 1}` }, sha256: sourceFrame.sha256 }));
  const sheetFile = options.sheetFile ?? { relativePath: "native/sheet.png", identity: sourceSheet.identity, sha256: sourceSheet.sha256 };
  const payload = { schema: "apparent.media-derivative-payload/2", materials: materials.manifest.key, leaseId: materials.payload.leaseId,
    jobId: request.jobId, receiptId: "8".repeat(32), root: request.outputDirectory,
    requestSha256: prepareMedia2Request(request, materials).sha256,
    frames: requestedBundleMs.map((requested, index) => ({ requestIndex: index + 1, requestedBundleMs: requested, effectiveVideoMs: Math.max(0, requested - 125), actualVideoMs: Math.max(0, requested - 125), clampedToStart: false, clampedToEnd: false,
      width: 800, height: 600, encoding: "png", byteLength: sourceFrame.byteLength, file: frameFiles[index] })),
    contactSheet: { encoding: "png", byteLength: sourceSheet.byteLength, file: sheetFile, width: 400, height: 300,
      layoutSha256: sha(encodeContactSheetLayout(layout)), layout } };
  const bytes = new TextEncoder().encode(JSON.stringify(payload));
  const receiptFile = { relativePath: "receipt.json", identity: { device: "1", inode: "native-receipt", size: bytes.byteLength, modifiedNs: "3" }, sha256: sha(bytes) };
  return { envelope: { schema: "apparent.media-derivative/2", materials: materials.manifest.key, leaseId: materials.payload.leaseId, jobId: request.jobId,
    receiptId: payload.receiptId, root: payload.root, requestSha256: payload.requestSha256, receiptFile, payload }, bytes, request, layoutBytes: encodeContactSheetLayout(layout) };
}

async function filesystemDerivativeFixture() {
  const input = await fixture();
  const materials = validateMaterialsDocument(input.manifest, input.payloadBytes);
  const root = await realpath(await mkdtemp(join(tmpdir(), "record-derivative-files-")));
  await chmod(root, 0o700);
  await mkdir(join(root, "native"), { mode: 0o700 });
  const frameBytes = input.artifacts.find(artifact => artifact.role === "frame")!.bytes;
  const sheetBytes = input.artifacts.find(artifact => artifact.role === "contact-sheet")!.bytes;
  await writeFile(join(root, "native/frame.png"), frameBytes, { mode: 0o600 });
  await writeFile(join(root, "native/sheet.png"), sheetBytes, { mode: 0o600 });
  const fileSeal = async (relativePath: string, content: Uint8Array) => {
    const info = await lstat(join(root, relativePath), { bigint: true });
    return { relativePath, sha256: sha(content), identity: { device: String(info.dev), inode: String(info.ino), size: Number(info.size), modifiedNs: String(info.mtimeNs) } };
  };
  const frameFile = await fileSeal("native/frame.png", frameBytes);
  const sheetFile = await fileSeal("native/sheet.png", sheetBytes);
  const document = derivative2For(input, [1000], { outputDirectory: root, frameFiles: [frameFile], sheetFile });
  return { root, input, materials, document, frameBytes, sheetBytes, frameFile, sheetFile };
}

const assertAbsent = (path: string) => assert.rejects(lstat(path), { code: "ENOENT" });

test("assembles an SDK-validated materials envelope and sealed facts with default validators", async () => {
  const input = await fixture();
  const result = assembleCanonicalBundle(input, {});
  const document = JSON.parse(new TextDecoder().decode(result.apparentJson));
  assert.equal(result.materials.payload.bundleId, "sdk-bundle");
  assert.equal(result.inventory.materials.captureId, identity.captureId);
  assert.equal(result.inventory.entries.find(entry => entry.path === "capture.mp4")?.sha256, sha(input.artifacts[0]!.bytes));
  assert.equal(result.inventory.entries.find(entry => entry.path === "capture.mp4")?.provenance, "material-reference");
  assert.equal(result.inventory.entries.find(entry => entry.path === "frames/frame-01.png")?.provenance, "caller-attested");
  assert.equal(document.transcript.model, "fixture-model-v7");
  assert.deepEqual(document.context, { workspace: "demo" });
  assert.deepEqual(document.events.map((event: { kind: string }) => event.kind), ["app_change", "click"]);
  assert.deepEqual(result.validatedFacts.transcript?.identity, identity);
});

test("material references derive original media seals without caller media bytes and stage from materials", async () => {
  const input = await fixture();
  const destination = join(tmpdir(), "record-material-reference.apparent");
  const baselineInput = structuredClone(input);
  baselineInput.admission.bundlePath = destination;
  baselineInput.frames = baselineInput.frames.map(frame => ({ ...frame, absolutePath: join(destination, frame.artifactPath) }));
  const baseline = assembleCanonicalBundle(baselineInput);
  input.artifacts = input.artifacts.filter(artifact => artifact.role !== "video");
  input.materialReferences = [{ role: "video", relativePath: "capture.mp4" }];
  input.admission.bundlePath = destination;
  input.frames = input.frames.map(frame => ({ ...frame, absolutePath: join(destination, frame.artifactPath) }));
  const assembly = assembleCanonicalBundle(input);
  assert.deepEqual(assembly.apparentJson, baseline.apparentJson);
  assert.deepEqual(assembly.summaryMd, baseline.summaryMd);
  assert.deepEqual(assembly.inventory, baseline.inventory);
  const videoEntry = assembly.inventory.entries.find(entry => entry.path === "capture.mp4")!;
  const originalVideo = assembly.materials.payload.files.find(file => file.relativePath === "capture.mp4")!;
  assert.deepEqual(videoEntry, { path: "capture.mp4", role: "video", sha256: originalVideo.sha256,
    byteLength: originalVideo.identity.size, provenance: "material-reference", identity: originalVideo.identity });

  const root = await mkdtemp(join(tmpdir(), "record-reference-stage-"));
  try {
    const staged = await prepareRecordPublication({ ownerToken: "c".repeat(64), publicationId: "6".repeat(32),
      stagingRoot: join(root, "private"), destinationPath: destination, materialsManifest: input.manifest,
      materialsPayloadBytes: input.payloadBytes, assembly,
      artifactBytes: Object.fromEntries(input.artifacts.filter(artifact => artifact.role === "frame" || artifact.role === "contact-sheet")
        .map(artifact => [artifact.relativePath, artifact.bytes])) });
    assert.equal(staged.plan.entries.find(entry => entry.role === "video")?.input.kind, "material");
    assert.equal(staged.plan.entries.find(entry => entry.role === "frame")?.input.kind, "assembled");
    assert.equal(staged.plan.entries.find(entry => entry.role === "contact-sheet")?.input.kind, "assembled");
    await assertAbsent(destination);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("completed-capture composition forwards only image bytes for material-backed media", async () => {
  const input = await fixture();
  input.artifacts = input.artifacts.filter(artifact => artifact.role !== "video");
  input.materialReferences = [{ role: "video", relativePath: "capture.mp4" }];
  const root = await mkdtemp(join(tmpdir(), "record-reference-composition-"));
  const destinationPath = join(root, "final.apparent");
  assemblyForDestination(input, destinationPath);
  let planBytes: Uint8Array | undefined;
  const sdk = {
    async handshake() { return { implementation: "native", capabilities: { publication: true } }; },
    async publish(request: { planBytes: Uint8Array; captureId: string; publicationId: string; planFile: { planSha256: string } }) {
      planBytes = new Uint8Array(request.planBytes);
      return { captureId: request.captureId, state: "publishing", publicationId: request.publicationId, planSha256: request.planFile.planSha256 };
    },
  } as unknown as CaptureClient;
  const publisher = createCompletedCapturePublisher(sdk, join(root, "control"), preflightCapturePublication);
  try {
    const status = await publisher.publishCompletedCapture({ status: { captureId: identity.captureId, state: "materials-ready", materials: input.manifest } as never,
      assembly: input, ownerToken: "c".repeat(64), publicationId: "7".repeat(32), stagingRoot: join(root, "staging"), destinationPath });
    assert.equal(status.state, "publishing");
    const plan = JSON.parse(new TextDecoder().decode(planBytes!));
    assert.equal(plan.entries.find((entry: { role: string }) => entry.role === "video")?.input.kind, "material");
    assert.equal(plan.entries.find((entry: { role: string }) => entry.role === "frame")?.input.kind, "assembled");
    assert.equal(plan.entries.find((entry: { role: string }) => entry.role === "contact-sheet")?.input.kind, "assembled");
    const stage = join(root, "staging", "7".repeat(32));
    assert.equal((await readdir(stage, { recursive: true })).some(path => path === "capture.mp4"), false);
    await assertAbsent(destinationPath);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("material references reject missing, duplicate, unknown, mismatched, unsafe, and dual-bound media", async () => {
  const input = await fixture();
  const withoutVideoBytes = structuredClone(input);
  withoutVideoBytes.artifacts = withoutVideoBytes.artifacts.filter(artifact => artifact.role !== "video");
  assert.throws(() => assembleCanonicalBundle(withoutVideoBytes), /lacks its supplied sealed video artifact/u);
  const check = (references: BundleAssemblyInput["materialReferences"]) => {
    const candidate = structuredClone(withoutVideoBytes);
    candidate.materialReferences = references;
    assert.throws(() => assembleCanonicalBundle(candidate));
  };
  check([{ role: "video", relativePath: "capture.mp4" }, { role: "video", relativePath: "capture.mp4" }]);
  check([{ role: "video", relativePath: "facts/context.json" }]);
  check([{ role: "audio", relativePath: "capture.mp4" }]);
  check([{ role: "video", relativePath: "../capture.mp4" }]);
  const dualBound = structuredClone(input);
  dualBound.materialReferences = [{ role: "video", relativePath: "capture.mp4" }];
  assert.throws(() => assembleCanonicalBundle(dualBound), /conflicts with supplied artifact bytes/u);
  check([{ role: "audio", relativePath: "capture.m4a" }]);

  const withAudio = await fixture();
  const audioFile = addAudioMaterial(withAudio);
  const noAudio = structuredClone(withAudio);
  noAudio.artifacts = noAudio.artifacts.filter(artifact => artifact.role !== "video");
  noAudio.materialReferences = [{ role: "video", relativePath: "capture.mp4" }];
  assert.throws(() => assembleCanonicalBundle(noAudio), /lacks its supplied sealed audio artifact/u);

  const withReferences = structuredClone(noAudio);
  withReferences.materialReferences = [{ role: "video", relativePath: "capture.mp4" }, { role: "audio", relativePath: audioFile.relativePath }];
  const result = assembleCanonicalBundle(withReferences);
  for (const file of ["capture.mp4", audioFile.relativePath]) {
    const declared = result.materials.payload.files.find(item => item.relativePath === file)!;
    const entry = result.inventory.entries.find(item => item.path === file)!;
    assert.equal(entry.sha256, declared.sha256);
    assert.equal(entry.byteLength, declared.identity.size);
    assert.deepEqual(entry.identity, declared.identity);
    assert.equal(entry.provenance, "material-reference");
  }
  const forged = structuredClone(withoutVideoBytes);
  forged.materialReferences = [{ role: "video", relativePath: "capture.mp4", sha256: "0".repeat(64), identity: { device: "9", inode: "forged", size: 0, modifiedNs: "0" } } as never];
  const forgedAssembly = assembleCanonicalBundle(forged);
  assert.equal(forgedAssembly.inventory.entries.find(entry => entry.path === "capture.mp4")?.sha256,
    forgedAssembly.materials.payload.files.find(file => file.relativePath === "capture.mp4")?.sha256);
});

test("public assembler emits canonical videoOnly when interaction collection is unavailable", async () => {
  const input = await fixture();
  const payload = JSON.parse(new TextDecoder().decode(input.payloadBytes));
  const interactionPath = payload.facts.interactions.file;
  payload.collection.interactions = "unavailable";
  delete payload.facts.interactions;
  payload.files = payload.files.filter((file: { relativePath: string }) => file.relativePath !== interactionPath);
  delete input.facts.interactions;
  input.payloadBytes = bytes(payload);
  input.manifest.key.revision = sha(input.payloadBytes);
  input.manifest.payloadFile.sha256 = sha(input.payloadBytes);
  input.manifest.payloadFile.identity.size = input.payloadBytes.byteLength;
  for (const artifact of input.artifacts) artifact.materials = { ...input.manifest.key };

  const result = assembleCanonicalBundle(input);
  const document = JSON.parse(new TextDecoder().decode(result.apparentJson));
  assert.equal(document.interactionTelemetry, "videoOnly");
  assert.deepEqual(document.events, []);
  assert.equal(result.inventory.collection.interactions, "unavailable");
  assert.equal(result.validatedFacts.interactions, undefined);
  assert.equal(result.inventory.entries.find(entry => entry.role === "frame")?.provenance, "caller-attested");

  const collected = assembleCanonicalBundle(await fixture());
  assert.equal(Object.hasOwn(JSON.parse(new TextDecoder().decode(collected.apparentJson)), "interactionTelemetry"), false);
});

test("public SDK validator seals RGB facts and canonical assembly preserves absent, black and observed colors", async () => {
  const input = await fixture();
  const payload = JSON.parse(new TextDecoder().decode(input.payloadBytes));
  const colors = [undefined, [0, 0, 0], [15, 127, 240]] as const;
  const strokes = colors.map((rgb, index) => ({ id: String(index + 1).repeat(32), surface: "screen", surfaceId: "surface-a", transformId: "transform-a",
    startMs: 100, endMs: 200, acceptedAtMs: 250, ...(rgb === undefined ? {} : { rgb }), points: [{ timeMs: 100, x: 100, y: 100 }] }));
  const strokeBytes = bytes({ schema: "apparent.capture-strokes/1", identity, coordinates: "target-normalized-10000",
    surfaces: [{ surfaceId: "surface-a", transformId: "transform-a", width: 800, height: 600, target: "capture-rectangle" }], strokes });
  const strokeSeal = sealed("facts/strokes.json", strokeBytes, "inode-strokes");
  const strokeFile = { relativePath: strokeSeal.relativePath, identity: strokeSeal.identity, sha256: strokeSeal.sha256 };
  payload.files.push({ relativePath: strokeFile.relativePath, sha256: strokeFile.sha256, identity: strokeFile.identity });
  payload.facts.acceptedStrokes = { file: strokeFile.relativePath, schema: "apparent.capture-strokes/1" };
  payload.collection.acceptedStrokes = "collected";
  const payloadBytes = bytes(payload);
  const manifest = { ...input.manifest, key: { ...identity, revision: sha(payloadBytes) }, payloadFile: { ...input.manifest.payloadFile,
    sha256: sha(payloadBytes), identity: { ...input.manifest.payloadFile.identity, size: payloadBytes.byteLength } } };
  const materials = validateMaterialsDocument(manifest, payloadBytes);
  const validated = validateFactFile(strokeFile, strokeBytes, materials, "acceptedStrokes") as StrokeFacts;
  assert.deepEqual(validated.strokes.map(stroke => stroke.rgb), colors);
  for (const bad of [[0, 0], [0, 0, 256], [0, -1, 0], [0, 1.5, 0], [0, 0, "0"]]) {
    const invalid = bytes({ schema: "apparent.capture-strokes/1", identity, coordinates: "target-normalized-10000",
      surfaces: [{ surfaceId: "surface-a", transformId: "transform-a", width: 800, height: 600, target: "capture-rectangle" }],
      strokes: [{ ...strokes[0], rgb: bad }] });
    const invalidSeal = sealed("facts/strokes.json", invalid, `inode-bad-${String(bad.length)}-${String(bad[2])}`);
    const invalidFile = { relativePath: invalidSeal.relativePath, identity: invalidSeal.identity, sha256: invalidSeal.sha256 };
    const badPayload = structuredClone(payload);
    badPayload.files[badPayload.files.findIndex((file: { relativePath: string }) => file.relativePath === strokeFile.relativePath)] =
      { relativePath: invalidFile.relativePath, sha256: invalidFile.sha256, identity: invalidFile.identity };
    const badPayloadBytes = bytes(badPayload);
    const badManifest = { ...manifest, key: { ...identity, revision: sha(badPayloadBytes) }, payloadFile: { ...manifest.payloadFile,
      sha256: sha(badPayloadBytes), identity: { ...manifest.payloadFile.identity, size: badPayloadBytes.byteLength } } };
    const badMaterials = validateMaterialsDocument(badManifest, badPayloadBytes);
    assert.throws(() => validateFactFile(invalidFile, invalid, badMaterials, "acceptedStrokes"), /rgb|byte/u);
  }
  const result = assembleCanonicalBundle({ ...input, manifest, payloadBytes,
    artifacts: input.artifacts.map(artifact => ({ ...artifact, materials: manifest.key })),
    facts: { ...input.facts, acceptedStrokes: { file: strokeFile, bytes: strokeBytes } },
    admission: { ...input.admission, captureId: identity.captureId, materialsId: identity.materialsId },
    additionalCanonicalFacts: { strokes: strokes.map(stroke => ({ id: stroke.id, t0: 100, t1: 200, fw: 800, fh: 600, pts: [100, 100, 0] })) } });
  const document = JSON.parse(new TextDecoder().decode(result.apparentJson));
  assert.equal(Object.hasOwn(document.strokes[0], "rgb"), false);
  assert.deepEqual(document.strokes[1].rgb, [0, 0, 0]);
  assert.deepEqual(document.strokes[2].rgb, [15, 127, 240]);
  assert.deepEqual(result.validatedFacts.acceptedStrokes?.strokes.map(stroke => stroke.rgb), colors);
  const publicationRoot = await mkdtemp(join(tmpdir(), "record-rgb-preflight-"));
  try {
    const destination = join(publicationRoot, "rgb.apparent");
    const publishable = assemblyForDestination({ ...input, manifest, payloadBytes,
      artifacts: input.artifacts.map(artifact => ({ ...artifact, materials: manifest.key })),
      facts: { ...input.facts, acceptedStrokes: { file: strokeFile, bytes: strokeBytes } },
      admission: { ...input.admission, captureId: identity.captureId, materialsId: identity.materialsId },
      additionalCanonicalFacts: { strokes: strokes.map(stroke => ({ id: stroke.id, t0: 100, t1: 200, fw: 800, fh: 600, pts: [100, 100, 0] })) } }, destination);
    const prepared = await prepareRecordPublication(stagingInput({ ...input, manifest, payloadBytes }, publishable, publicationRoot,
      "d".repeat(32), destination));
    assert.equal(prepared.plan.entries.find(entry => entry.relativePath === "apparent.json")?.sha256, sha(publishable.apparentJson));
    assert.deepEqual(JSON.parse(new TextDecoder().decode(publishable.apparentJson)).strokes.map((stroke: { rgb?: number[] }) => stroke.rgb),
      [undefined, [0, 0, 0], [15, 127, 240]]);
  } finally { await rm(publicationRoot, { recursive: true, force: true }); }
  const spoof = structuredClone({ ...input, manifest, payloadBytes,
    artifacts: input.artifacts.map(artifact => ({ ...artifact, materials: manifest.key })),
    facts: { ...input.facts, acceptedStrokes: { file: strokeFile, bytes: strokeBytes } },
    admission: { ...input.admission, captureId: identity.captureId, materialsId: identity.materialsId },
    additionalCanonicalFacts: { strokes: strokes.map(stroke => ({ id: stroke.id, t0: 100, t1: 200, fw: 800, fh: 600, pts: [100, 100, 0] })) } });
  spoof.frames[0]!.absolutePath = join(spoof.admission.bundlePath, spoof.frames[0]!.artifactPath);
  Object.assign(spoof.additionalCanonicalFacts!.strokes![1]!, { rgb: [1, 2, 3] });
  assert.throws(() => assembleCanonicalBundle(spoof, {}), /cannot supply observed RGB/u);
});

test("private staging prepares SDK-validated plan without touching destination or dispatching", async () => {
  const input = await fixture();
  const root = await mkdtemp(join(tmpdir(), "record-stage-fixture-"));
  const destinationPath = join(root, "final.apparent");
  try {
    const assembly = assemblyForDestination(input, destinationPath);
    const prepared = await prepareRecordPublication({
      ownerToken: "c".repeat(64), publicationId: "b".repeat(32), stagingRoot: join(root, "private"), destinationPath,
      materialsManifest: input.manifest, materialsPayloadBytes: input.payloadBytes, assembly,
      artifactBytes: Object.fromEntries(input.artifacts.map(artifact => [artifact.relativePath, artifact.bytes])),
    });
    assert.equal(prepared.plan.publicationId, "b".repeat(32));
    assert.equal(prepared.plan.materials.revision, assembly.materials.manifest.key.revision);
    assert.equal(prepared.plan.destination.path, destinationPath);
    assert.equal(prepared.plan.entries.find(entry => entry.role === "frame")?.input.kind, "assembled");
    assert.equal(prepared.plan.entries.find(entry => entry.role === "video")?.input.kind, "material");
    assert.equal(prepared.input.planFile.planSha256, sha(prepared.input.planBytes));
    await assertAbsent(destinationPath);
    const replay = await prepareRecordPublication({
      ownerToken: "c".repeat(64), publicationId: "b".repeat(32), stagingRoot: join(root, "private"), destinationPath,
      materialsManifest: input.manifest, materialsPayloadBytes: input.payloadBytes, assembly,
      artifactBytes: Object.fromEntries(input.artifacts.map(artifact => [artifact.relativePath, artifact.bytes])),
    });
    assert.equal(replay.input.planFile.planSha256, prepared.input.planFile.planSha256, "an exact replay resumes the same staged plan");
    const planPath = join(root, "private", "b".repeat(32), "publication-plan.json");
    await writeFile(planPath, "partial plan", { mode: 0o600 });
    await assert.rejects(prepareRecordPublication({
      ownerToken: "c".repeat(64), publicationId: "b".repeat(32), stagingRoot: join(root, "private"), destinationPath,
      materialsManifest: input.manifest, materialsPayloadBytes: input.payloadBytes, assembly,
      artifactBytes: Object.fromEntries(input.artifacts.map(artifact => [artifact.relativePath, artifact.bytes])),
    }), /replay conflicts/u);
    assert.equal(await readFile(planPath, "utf8"), "partial plan", "ambiguous partial state is retained");
    const foreign = join(root, "private", "b".repeat(32), "foreign.txt");
    await writeFile(foreign, "preserve me", { mode: 0o600 });
    await assert.rejects(prepareRecordPublication({
      ownerToken: "c".repeat(64), publicationId: "b".repeat(32), stagingRoot: join(root, "private"), destinationPath,
      materialsManifest: input.manifest, materialsPayloadBytes: input.payloadBytes, assembly,
      artifactBytes: Object.fromEntries(input.artifacts.map(artifact => [artifact.relativePath, artifact.bytes])),
    }), /unrelated state/u);
    assert.equal(await readFile(foreign, "utf8"), "preserve me", "conflicting staged state is never cleaned up implicitly");
    await symlink(join(root, "private"), join(root, "root-alias"));
    await assert.rejects(prepareRecordPublication({
      ownerToken: "c".repeat(64), publicationId: "d".repeat(32), stagingRoot: join(root, "root-alias"), destinationPath,
      materialsManifest: input.manifest, materialsPayloadBytes: input.payloadBytes, assembly,
      artifactBytes: Object.fromEntries(input.artifacts.map(artifact => [artifact.relativePath, artifact.bytes])),
    }), /alias/u);
    const escaped = structuredClone(assembly);
    escaped.inventory.entries[0]!.path = "../outside.json";
    await assert.rejects(prepareRecordPublication({
      ownerToken: "c".repeat(64), publicationId: "e".repeat(32), stagingRoot: join(root, "escape-private"), destinationPath,
      materialsManifest: input.manifest, materialsPayloadBytes: input.payloadBytes, assembly: escaped,
      artifactBytes: Object.fromEntries(input.artifacts.map(artifact => [artifact.relativePath, artifact.bytes])),
    }), /invalid relative path/u);
    const concurrent = {
      ownerToken: "c".repeat(64), publicationId: "f".repeat(32), stagingRoot: join(root, "concurrent-private"), destinationPath,
      materialsManifest: input.manifest, materialsPayloadBytes: input.payloadBytes, assembly,
      artifactBytes: Object.fromEntries(input.artifacts.map(artifact => [artifact.relativePath, artifact.bytes])),
    };

    const winner = prepareRecordPublication(concurrent);
    const conflicting = prepareRecordPublication({ ...concurrent, artifactBytes: { ...concurrent.artifactBytes, "frames/frame-01.png": new Uint8Array([1, 2, 3]) } });
    const losing = assert.rejects(conflicting, /in-flight staging binding/u);
    await winner;
    await losing;
    const summary = new TextDecoder().decode(assembly.summaryMd);
    assert.match(summary, /hello there/u);
    assert.ok(summary.includes(destinationPath), "assembled summary retains the bound absolute bundle path");
    assert.match(new TextDecoder().decode(assembly.apparentJson), /contact-sheet\.png/u);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("legacy staging namespaces replay without a derivative/2 marker field", async () => {
  const input = await fixture();
  const root = await mkdtemp(join(tmpdir(), "record-stage-legacy-"));
  const destinationPath = join(root, "final.apparent");
  try {
    const assembly = assemblyForDestination(input, destinationPath);
    const request = stagingInput(input, assembly, root, "1".repeat(32), destinationPath);
    const prepared = await prepareRecordPublication(request);
    const markerPath = join(request.stagingRoot, request.publicationId, ".record-publication");
    const marker = JSON.parse(await readFile(markerPath, "utf8"));
    // This field did not exist in apparent.record-staging/1 before derivative/2.
    assert.equal(Object.hasOwn(marker, "derivatives2Sha256"), false);
    assert.equal((await prepareRecordPublication(request)).input.planFile.planSha256, prepared.input.planFile.planSha256);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("derivative/2 keeps duplicate request times in ordered one-based frame slots", async () => {
  const input = await fixture();
  const document = derivative2For(input, [1000, 1000]);
  const materials = validateMaterialsDocument(input.manifest, input.payloadBytes);
  const receipt = validateDerivative2Document(document, materials);
  assert.deepEqual(receipt.payload.frames.map(frame => [frame.requestIndex, frame.requestedBundleMs]), [[1, 1000], [2, 1000]]);
  assert.equal(selectDerivative2Artifact(receipt, { kind: "frame", requestIndex: 2 }).file.relativePath, "native/frame-2.png");
});

test("derivative/2 staging maps native frame and sheet seals to final paths and keeps native selectors", async () => {
  const input = await fixture();
  const root = await mkdtemp(join(tmpdir(), "record-stage-media2-"));
  const destinationPath = join(root, "final.apparent");
  try {
    const assembly = assemblyForDestination(input, destinationPath);
    const derivatives2 = [derivative2For(input)];
    const derivative2Artifacts = {
      "frames/frame-01.png": { receiptId: "8".repeat(32), selector: { requestIndex: 1, kind: "frame" as const } },
      "contact-sheet.png": { receiptId: "8".repeat(32), selector: { kind: "contact-sheet" as const } },
    };
    const prepared = await prepareRecordPublication({ ...stagingInput(input, assembly, root, "1".repeat(32), destinationPath), derivatives2, derivative2Artifacts });
    assert.equal(prepared.plan.entries.find(entry => entry.role === "frame")?.input.kind, "derivative2");
    assert.equal(prepared.plan.entries.find(entry => entry.role === "contact-sheet")?.input.kind, "derivative2");
    assert.equal(prepared.plan.entries.find(entry => entry.role === "frame")?.relativePath, "frames/frame-01.png");
    assert.equal(prepared.input.derivatives2?.length, 1);
    await assertAbsent(destinationPath);
    const exactReplay = await prepareRecordPublication({ ...stagingInput(input, assembly, root, "1".repeat(32), destinationPath), derivatives2, derivative2Artifacts });
    assert.equal(exactReplay.input.planFile.planSha256, prepared.input.planFile.planSha256);
    const reorderedBindings = {
      "contact-sheet.png": { selector: { kind: "contact-sheet" as const }, receiptId: "8".repeat(32) },
      "frames/frame-01.png": { selector: { kind: "frame" as const, requestIndex: 1 }, receiptId: "8".repeat(32) },
    };
    const reorderedDocument = structuredClone(derivatives2[0]!);
    reorderedDocument.envelope = Object.fromEntries(Object.entries(reorderedDocument.envelope as Record<string, unknown>).reverse());
    const reorderedReplay = await prepareRecordPublication({ ...stagingInput(input, assembly, root, "1".repeat(32), destinationPath), derivatives2: [reorderedDocument], derivative2Artifacts: reorderedBindings });
    assert.equal(reorderedReplay.input.planFile.planSha256, prepared.input.planFile.planSha256);
    const changedDocument = structuredClone(derivatives2[0]!);
    changedDocument.bytes = new Uint8Array([...changedDocument.bytes, 0x20]);
    const changedEnvelope = changedDocument.envelope as { receiptFile: { sha256: string; identity: { size: number } } };
    changedEnvelope.receiptFile.sha256 = sha(changedDocument.bytes);
    changedEnvelope.receiptFile.identity.size = changedDocument.bytes.byteLength;
    await assert.rejects(prepareRecordPublication({ ...stagingInput(input, assembly, root, "1".repeat(32), destinationPath), derivatives2: [changedDocument], derivative2Artifacts }), /replay conflicts/u);

    const missing = { ...stagingInput(input, assembly, join(root, "missing"), "2".repeat(32), destinationPath), derivatives2, derivative2Artifacts: { "frames/frame-01.png": derivative2Artifacts["frames/frame-01.png"] } };
    await assert.rejects(prepareRecordPublication(missing), /mapping is required/u);
    await assertAbsent(join(root, "missing"));
    const changedRequest = structuredClone(derivatives2[0]!);
    (changedRequest.request as { limits: { deadlineMs: number } }).limits.deadlineMs++;
    await assert.rejects(prepareRecordPublication({ ...stagingInput(input, assembly, join(root, "changed"), "3".repeat(32), destinationPath), derivatives2: [changedRequest], derivative2Artifacts }), /request binding mismatch/u);
    await assertAbsent(join(root, "changed"));

    const mismatches: Array<[string, (assemblyCopy: ReturnType<typeof assembleCanonicalBundle>, document: Derivative2DocumentInput, bindings: typeof derivative2Artifacts) => void]> = [
      ["lease", (_assembly, document) => { (document.request as { leaseId: string }).leaseId = "4".repeat(32); }],
      ["job", (_assembly, document) => { (document.request as { jobId: string }).jobId = "5".repeat(32); }],
      ["materials", (_assembly, document) => { (document.request as { materials: { captureId: string } }).materials.captureId = "6".repeat(32); }],
      ["selector", (_assembly, _document, bindings) => { bindings["frames/frame-01.png"]!.selector = { kind: "frame", requestIndex: 2 }; }],
      ["role", (assemblyCopy) => { assemblyCopy.inventory.entries.find(entry => entry.path === "frames/frame-01.png")!.role = "contact-sheet"; }],
      ["hash", (assemblyCopy) => { assemblyCopy.inventory.entries.find(entry => entry.path === "frames/frame-01.png")!.sha256 = "0".repeat(64); }],
      ["length", (assemblyCopy) => { assemblyCopy.inventory.entries.find(entry => entry.path === "frames/frame-01.png")!.byteLength++; }],
      ["identity", (assemblyCopy) => { assemblyCopy.inventory.entries.find(entry => entry.path === "frames/frame-01.png")!.identity!.inode = "other-native-file"; }],
      ["layout", (_assembly, document) => { document.layoutBytes![0] ^= 1; }],
      ["unknown path", (_assembly, _document, bindings) => { Object.assign(bindings, { "unknown.png": bindings["frames/frame-01.png"]! }); }],
    ];
    for (const [name, mutate] of mismatches) {
      const stagingRoot = join(root, `refuse-${name}`);
      const assemblyCopy = structuredClone(assembly);
      const document = structuredClone(derivatives2[0]!);
      const bindings = structuredClone(derivative2Artifacts);
      mutate(assemblyCopy, document, bindings);
      await assert.rejects(prepareRecordPublication({ ...stagingInput(input, assemblyCopy, stagingRoot, "4".repeat(32), destinationPath), derivatives2: [document], derivative2Artifacts: bindings }));
      await assertAbsent(stagingRoot);
    }
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("retained derivative reader maps exact filesystem seals through assembly into native-selector staging", async () => {
  const f = await filesystemDerivativeFixture();
  try {
    const mappings = [
      { bundlePath: "frames/renamed.png", selector: { kind: "frame", requestIndex: 1 } as const },
      { bundlePath: "sheets/renamed-contact.png", selector: { kind: "contact-sheet" } as const },
    ];
    const retained = await readRetainedDerivativeArtifacts({ document: f.document, materials: f.materials, trustedRoot: f.root, mappings,
      limits: { maxFileBytes: 1024 * 1024, maxAggregateBytes: 2 * 1024 * 1024 } });
    assert.deepEqual(retained.artifacts.map(artifact => [artifact.relativePath, artifact.role, artifact.byteLength, artifact.sha256]), [
      ["frames/renamed.png", "frame", f.frameBytes.byteLength, sha(f.frameBytes)],
      ["sheets/renamed-contact.png", "contact-sheet", f.sheetBytes.byteLength, sha(f.sheetBytes)],
    ]);
    assert.deepEqual(retained.artifacts.map(artifact => artifact.identity), [f.frameFile.identity, f.sheetFile.identity]);
    assert.deepEqual(retained.artifacts.map(artifact => artifact.bytes), [f.frameBytes, f.sheetBytes]);
    assert.deepEqual(retained.derivative2Artifacts, {
      "frames/renamed.png": { receiptId: "8".repeat(32), selector: mappings[0]!.selector },
      "sheets/renamed-contact.png": { receiptId: "8".repeat(32), selector: mappings[1]!.selector },
    });

    const destination = join(f.root, "final.apparent");
    const assemblyInput = structuredClone(f.input);
    assemblyInput.artifacts = [
      ...assemblyInput.artifacts.filter(artifact => artifact.role !== "frame" && artifact.role !== "contact-sheet"),
      ...retained.artifacts,
    ];
    assemblyInput.frames = assemblyInput.frames.map(frame => ({ ...frame, artifactPath: "frames/renamed.png", absolutePath: join(destination, "frames/renamed.png") }));
    const assembly = assemblyForDestination(assemblyInput, destination);
    const prepared = await prepareRecordPublication({ ...stagingInput(assemblyInput, assembly, f.root, "7".repeat(32), destination),
      derivatives2: [f.document], derivative2Artifacts: retained.derivative2Artifacts });
    assert.equal(prepared.plan.entries.find(entry => entry.role === "frame")?.input.kind, "derivative2");
    assert.equal(prepared.plan.entries.find(entry => entry.role === "frame")?.relativePath, "frames/renamed.png");
    assert.equal(prepared.plan.entries.find(entry => entry.role === "contact-sheet")?.input.kind, "derivative2");
    assert.equal(prepared.plan.entries.find(entry => entry.role === "contact-sheet")?.relativePath, "sheets/renamed-contact.png");
  } finally { await rm(f.root, { recursive: true, force: true }); }
});

test("retained derivative reader snapshots inputs and rejects stale bindings, duplicate selectors, and unsafe final paths", async () => {
  const f = await filesystemDerivativeFixture();
  try {
    const mappings = [{ bundlePath: "frames/renamed.png", selector: { kind: "frame", requestIndex: 1 } as const }];
    const mutableDocument = structuredClone(f.document);
    const mutableMaterials = structuredClone(f.materials);
    const pending = readRetainedDerivativeArtifacts({ document: mutableDocument, materials: mutableMaterials, trustedRoot: f.root, mappings,
      limits: { maxFileBytes: 1024 * 1024, maxAggregateBytes: 2 * 1024 * 1024 } });
    mappings[0]!.bundlePath = "../mutated.png";
    mutableDocument.bytes[0] ^= 1;
    mutableMaterials.manifest.key.captureId = "9".repeat(32);
    const snapshot = await pending;
    assert.equal(snapshot.artifacts[0]!.relativePath, "frames/renamed.png");
    assert.deepEqual(snapshot.artifacts[0]!.bytes, f.frameBytes);
    assert.deepEqual(snapshot.artifacts[0]!.materials, f.materials.manifest.key);

    const twoMappings = [
      { bundlePath: "frames/a.png", selector: { kind: "frame", requestIndex: 1 } as const },
      { bundlePath: "frames/a.png", selector: { kind: "contact-sheet" } as const },
    ];
    await assert.rejects(readRetainedDerivativeArtifacts({ document: f.document, materials: f.materials, trustedRoot: f.root, mappings: twoMappings,
      limits: { maxFileBytes: 1024 * 1024, maxAggregateBytes: 2 * 1024 * 1024 } }), /duplicate final bundle path/u);
    await assert.rejects(readRetainedDerivativeArtifacts({ document: f.document, materials: f.materials, trustedRoot: f.root,
      mappings: [{ bundlePath: "frames/a.png", selector: { kind: "frame", requestIndex: 1 } }, { bundlePath: "frames/b.png", selector: { kind: "frame", requestIndex: 1 } }],
      limits: { maxFileBytes: 1024 * 1024, maxAggregateBytes: 2 * 1024 * 1024 } }), /multiple final paths/u);
    await assert.rejects(readRetainedDerivativeArtifacts({ document: f.document, materials: f.materials, trustedRoot: f.root,
      mappings: [{ bundlePath: "frames/../escape.png", selector: { kind: "frame", requestIndex: 1 } }],
      limits: { maxFileBytes: 1024 * 1024, maxAggregateBytes: 2 * 1024 * 1024 } }), /invalid, escaping/u);
    await assert.rejects(readRetainedDerivativeArtifacts({ document: f.document, materials: f.materials, trustedRoot: `${f.root}-other`, mappings,
      limits: { maxFileBytes: 1024 * 1024, maxAggregateBytes: 2 * 1024 * 1024 } }), /trusted root differs/u);

    const changedRequest = structuredClone(f.document);
    changedRequest.request.limits.deadlineMs++;
    await assert.rejects(readRetainedDerivativeArtifacts({ document: changedRequest, materials: f.materials, trustedRoot: f.root,
      mappings: [{ bundlePath: "frames/frame.png", selector: { kind: "frame", requestIndex: 1 } }], limits: { maxFileBytes: 1024 * 1024, maxAggregateBytes: 2 * 1024 * 1024 } }), /request binding/u);
    const changedLease = structuredClone(f.document);
    changedLease.request.leaseId = "4".repeat(32);
    await assert.rejects(readRetainedDerivativeArtifacts({ document: changedLease, materials: f.materials, trustedRoot: f.root,
      mappings: [{ bundlePath: "frames/frame.png", selector: { kind: "frame", requestIndex: 1 } }], limits: { maxFileBytes: 1024 * 1024, maxAggregateBytes: 2 * 1024 * 1024 } }));
    const changedJob = structuredClone(f.document);
    changedJob.request.jobId = "5".repeat(32);
    await assert.rejects(readRetainedDerivativeArtifacts({ document: changedJob, materials: f.materials, trustedRoot: f.root,
      mappings: [{ bundlePath: "frames/frame.png", selector: { kind: "frame", requestIndex: 1 } }], limits: { maxFileBytes: 1024 * 1024, maxAggregateBytes: 2 * 1024 * 1024 } }));
    await assert.rejects(readRetainedDerivativeArtifacts({ document: f.document, materials: f.materials, trustedRoot: f.root,
      mappings: [{ bundlePath: "frames/missing.png", selector: { kind: "frame", requestIndex: 2 } }], limits: { maxFileBytes: 1024 * 1024, maxAggregateBytes: 2 * 1024 * 1024 } }));
    const changedReceiptRoot = structuredClone(f.document);
    (changedReceiptRoot.envelope as { root: string }).root = `${f.root}-other`;
    await assert.rejects(readRetainedDerivativeArtifacts({ document: changedReceiptRoot, materials: f.materials, trustedRoot: f.root,
      mappings: [{ bundlePath: "frames/frame.png", selector: { kind: "frame", requestIndex: 1 } }], limits: { maxFileBytes: 1024 * 1024, maxAggregateBytes: 2 * 1024 * 1024 } }));
    const changedMaterials = structuredClone(f.materials);
    changedMaterials.manifest.key.revision = "0".repeat(64);
    await assert.rejects(readRetainedDerivativeArtifacts({ document: f.document, materials: changedMaterials, trustedRoot: f.root,
      mappings: [{ bundlePath: "frames/frame.png", selector: { kind: "frame", requestIndex: 1 } }], limits: { maxFileBytes: 1024 * 1024, maxAggregateBytes: 2 * 1024 * 1024 } }));
    const changedLayout = structuredClone(f.document);
    changedLayout.layoutBytes![0] ^= 1;
    await assert.rejects(readRetainedDerivativeArtifacts({ document: changedLayout, materials: f.materials, trustedRoot: f.root,
      mappings: [{ bundlePath: "frames/frame.png", selector: { kind: "frame", requestIndex: 1 } }], limits: { maxFileBytes: 1024 * 1024, maxAggregateBytes: 2 * 1024 * 1024 } }), /layout/u);
    const changedReceipt = structuredClone(f.document);
    (changedReceipt.envelope as { receiptId: string }).receiptId = "9".repeat(32);
    await assert.rejects(readRetainedDerivativeArtifacts({ document: changedReceipt, materials: f.materials, trustedRoot: f.root,
      mappings: [{ bundlePath: "frames/frame.png", selector: { kind: "frame", requestIndex: 1 } }], limits: { maxFileBytes: 1024 * 1024, maxAggregateBytes: 2 * 1024 * 1024 } }));
  } finally { await rm(f.root, { recursive: true, force: true }); }
});

test("retained derivative reader refuses symlinks, changed identities and hashes, and byte-budget overflow", async () => {
  const f = await filesystemDerivativeFixture();
  try {
    const selected = [{ bundlePath: "frames/final.png", selector: { kind: "frame", requestIndex: 1 } as const }];
    const limits = { maxFileBytes: 1024 * 1024, maxAggregateBytes: 2 * 1024 * 1024 };
    await symlink(join(f.root, "native/frame.png"), join(f.root, "native/alias.png"));
    const symlinkFile = { ...f.frameFile, relativePath: "native/alias.png" };
    const symlinkDocument = derivative2For(f.input, [1000], { outputDirectory: f.root, frameFiles: [symlinkFile], sheetFile: f.sheetFile });
    await assert.rejects(readRetainedDerivativeArtifacts({ document: symlinkDocument, materials: f.materials, trustedRoot: f.root, mappings: selected, limits }), /regular private path/u);

    const wrongIdentity = { ...f.frameFile, identity: { ...f.frameFile.identity, inode: "foreign-native-inode" } };
    const identityDocument = derivative2For(f.input, [1000], { outputDirectory: f.root, frameFiles: [wrongIdentity], sheetFile: f.sheetFile });
    await assert.rejects(readRetainedDerivativeArtifacts({ document: identityDocument, materials: f.materials, trustedRoot: f.root, mappings: selected, limits }), /identity changed/u);

    const wrongHash = { ...f.frameFile, sha256: "0".repeat(64) };
    const hashDocument = derivative2For(f.input, [1000], { outputDirectory: f.root, frameFiles: [wrongHash], sheetFile: f.sheetFile });
    await assert.rejects(readRetainedDerivativeArtifacts({ document: hashDocument, materials: f.materials, trustedRoot: f.root, mappings: selected, limits }), /hash mismatch/u);

    await assert.rejects(readRetainedDerivativeArtifacts({ document: f.document, materials: f.materials, trustedRoot: f.root, mappings: selected,
      limits: { maxFileBytes: f.frameBytes.byteLength - 1, maxAggregateBytes: f.frameBytes.byteLength } }), /byte limit/u);
    const aggregateLimit = Math.max(f.frameBytes.byteLength, f.sheetBytes.byteLength, f.frameBytes.byteLength + f.sheetBytes.byteLength - 1);
    await assert.rejects(readRetainedDerivativeArtifacts({ document: f.document, materials: f.materials, trustedRoot: f.root,
      mappings: [{ bundlePath: "frames/final.png", selector: { kind: "frame", requestIndex: 1 } }, { bundlePath: "sheets/final.png", selector: { kind: "contact-sheet" } }],
      limits: { maxFileBytes: aggregateLimit, maxAggregateBytes: aggregateLimit } }), /byte limit/u);
  } finally { await rm(f.root, { recursive: true, force: true }); }
});

test("refuses same-named video bytes whose seal differs from the SDK materials reference", async () => {
  const input = await fixture();
  const video = input.artifacts.find(artifact => artifact.role === "video")!;
  const differentBytes = new TextEncoder().encode("different synthetic video bytes");
  Object.assign(video, sealed(video.relativePath, differentBytes, "inode-video-other"), { materials: video.materials });
  assert.throws(() => assembleCanonicalBundle(input, {}), /video|materials|sealed/i);
});

test("staging binds a snapshot before caller mutation", async () => {
  const source = await fixture();
  const root = await mkdtemp(join(tmpdir(), "record-stage-snapshot-"));
  try {
    const destinationPath = join(root, "destination.apparent");
    const assembly = assemblyForDestination(source, destinationPath);
    const request = stagingInput(source, assembly, root, "1".repeat(32), destinationPath);
    const expectedFrameHash = assembly.inventory.entries.find(entry => entry.path === "frames/frame-01.png")!.sha256;
    const staged = prepareRecordPublication(request);
    request.ownerToken = "d".repeat(64);
    request.artifactBytes["frames/frame-01.png"]!.fill(0);
    const result = await staged;
    assert.equal(result.input.ownerToken, "c".repeat(64));
    const frame = result.plan.entries.find(entry => entry.relativePath === "frames/frame-01.png")!;
    assert.equal(frame.sha256, expectedFrameHash);
    assert.equal(frame.input.kind, "assembled");
    assert.deepEqual(result.inventory, assembly.inventory);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("identical concurrent staging calls return independent nested results", async () => {
  const source = await fixture();
  const root = await mkdtemp(join(tmpdir(), "record-stage-results-"));
  try {
    const destinationPath = join(root, "destination.apparent");
    const assembly = assemblyForDestination(source, destinationPath);
    const request = stagingInput(source, assembly, root, "1".repeat(32), destinationPath);
    const [first, second] = await Promise.all([prepareRecordPublication(request), prepareRecordPublication(request)]);
    assert.notEqual(first, second);
    assert.notEqual(first.plan, second.plan);
    assert.notEqual(first.plan.entries, second.plan.entries);
    first.plan.entries[0]!.relativePath = "mutated";
    first.plan.destination.path = "mutated";
    first.input.planBytes.fill(0);
    first.inventory.entries[0]!.path = "mutated";
    assert.notEqual(second.plan.entries[0]!.relativePath, "mutated");
    assert.notEqual(second.plan.destination.path, "mutated");
    assert.notEqual(second.input.planBytes[0], 0);
    assert.notEqual(second.inventory.entries[0]!.path, "mutated");
    assert.notEqual(assembly.inventory.entries[0]!.path, "mutated");
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("rejects staging roots that overlap the destination in either direction", async () => {
  const source = await fixture();
  const root = await mkdtemp(join(tmpdir(), "record-stage-overlap-"));
  try {
    const ancestorDestination = join(root, "absent-final.apparent");
    const ancestorStagingRoot = join(ancestorDestination, "private");
    const ancestorAssembly = assemblyForDestination(source, ancestorDestination);
    await assert.rejects(prepareRecordPublication({ ...stagingInput(source, ancestorAssembly, root, "7".repeat(32), ancestorDestination), stagingRoot: ancestorStagingRoot }), /overlap|destination|staging/u);
    await assertAbsent(ancestorDestination);
    const descendantRoot = join(root, "other", "nested");
    const descendantDestination = join(descendantRoot, "destination.apparent");
    const descendantAssembly = assemblyForDestination(source, descendantDestination);
    await assert.rejects(prepareRecordPublication({ ...stagingInput(source, descendantAssembly, root, "3".repeat(32), descendantDestination), stagingRoot: descendantRoot }), /overlap|destination|staging/u);
    await assertAbsent(join(root, "other"));
    const physicalRoot = join(root, "physical-stage"), alias = join(root, "destination-alias");
    await mkdir(physicalRoot, { mode: 0o700 }); await symlink(physicalRoot, alias);
    const aliasedDestination = join(alias, "final.apparent");
    const aliasedAssembly = assemblyForDestination(source, aliasedDestination);
    await assert.rejects(prepareRecordPublication({ ...stagingInput(source, aliasedAssembly, root, "7".repeat(32), aliasedDestination), stagingRoot: physicalRoot }), /alias/u);
    await assertAbsent(join(physicalRoot, "7".repeat(32)));
    await assertAbsent(aliasedDestination);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("refuses a staged destination that differs from the assembled bundle path", async () => {
  const source = await fixture();
  const root = await mkdtemp(join(tmpdir(), "record-stage-destination-binding-"));
  try {
    const assembledDestination = join(root, "assembled.apparent");
    const requestedDestination = join(root, "requested.apparent");
    const assembly = assemblyForDestination(source, assembledDestination);
    const request = stagingInput(source, assembly, root, "6".repeat(32), requestedDestination);
    await assert.rejects(prepareRecordPublication(request), /destination|bundle path|binding/u);
    await assertAbsent(request.stagingRoot);
    await assertAbsent(requestedDestination);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("completed-capture composition refuses a mismatched observed status before staging", async () => {
  const input = await fixture();
  const root = await mkdtemp(join(tmpdir(), "record-composition-reject-"));
  const destinationPath = join(root, "final.apparent");
  assemblyForDestination(input, destinationPath);
  const client = { async handshake() { throw new Error("must not reach SDK"); } } as unknown as CaptureClient;
  const publisher = createCompletedCapturePublisher(client, join(root, "control"), preflightCapturePublication);
  try {
    await assert.rejects(publisher.publishCompletedCapture({
      status: { captureId: identity.captureId, state: "materials-ready", materials: { ...input.manifest, leaseId: "e".repeat(32) } } as never,
      assembly: input, ownerToken: "c".repeat(64), publicationId: "a".repeat(32),
      stagingRoot: join(root, "staging"), destinationPath,
    }), /binding conflicts/u);
    await assertAbsent(join(root, "staging"));
    await assertAbsent(join(root, "control"));
    await assert.rejects(publisher.publishCompletedCapture({
      status: { captureId: identity.captureId, state: "materials-ready", materials: input.manifest } as never,
      assembly: input, ownerToken: "c".repeat(64), publicationId: "a".repeat(32),
      stagingRoot: join(root, "staging"), destinationPath,
      derivatives: [{ envelope: {}, bytes: new Uint8Array(), requestedBundleMs: [1000] }] as never,
    }), error => error instanceof CapturePublicationPreflightError && error.dispatched === false);
    await assertAbsent(destinationPath);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("completed-capture composition refuses the packed SDK test peer without reserving or creating the destination", async () => {
  const input = await fixture();
  const root = await mkdtemp(join(tmpdir(), "record-composition-test-peer-"));
  const destinationPath = join(root, "final.apparent");
  assemblyForDestination(input, destinationPath);
  const client = new CaptureClient(new ReferenceCapturePeer(), { allowTestDouble: true });
  const publisher = createCompletedCapturePublisher(client, join(root, "control"), preflightCapturePublication);
  try {
    await assert.rejects(publisher.publishCompletedCapture({
      status: { captureId: identity.captureId, state: "materials-ready", materials: input.manifest } as never,
      assembly: input, ownerToken: "c".repeat(64), publicationId: "a".repeat(32),
      stagingRoot: join(root, "staging"), destinationPath,
    }), error => error instanceof Error && (error as Error & { dispatched?: boolean }).dispatched === false);
    assert.equal(await new SqlitePublicationBindingStore(join(root, "control")).get(identity.captureId), undefined);
    await assertAbsent(destinationPath);
  } finally { await client.close(); await rm(root, { recursive: true, force: true }); }
});

test("completed-capture composition stages the assembled bytes, isolates inputs, and returns only recovery status", async () => {
  const input = await fixture();
  const root = await mkdtemp(join(tmpdir(), "record-composition-success-"));
  const destinationPath = join(root, "final.apparent");
  assemblyForDestination(input, destinationPath);
  const derivatives2 = [derivative2For(input)];
  const derivative2Artifacts = {
    "frames/frame-01.png": { receiptId: "8".repeat(32), selector: { kind: "frame" as const, requestIndex: 1 } },
    "contact-sheet.png": { receiptId: "8".repeat(32), selector: { kind: "contact-sheet" as const } },
  };
  let dispatched: { ownerToken: string; planBytes: Uint8Array } | undefined;
  const sdk = {
    async handshake() { return { implementation: "native", capabilities: { publication: true } }; },
    async publish(request: { ownerToken: string; planBytes: Uint8Array; captureId: string; publicationId: string; planFile: { planSha256: string } }) {
      dispatched = { ownerToken: request.ownerToken, planBytes: new Uint8Array(request.planBytes) };
      return { captureId: request.captureId, state: "publishing", publicationId: request.publicationId, planSha256: request.planFile.planSha256 };
    },
  } as unknown as CaptureClient;
  const publisher = createCompletedCapturePublisher(sdk, join(root, "control"), preflightCapturePublication);
  let binaryJSONCalls = 0;
  const originalToJSON = Object.getOwnPropertyDescriptor(Uint8Array.prototype, "toJSON");
  Object.defineProperty(Uint8Array.prototype, "toJSON", { configurable: true,
    value(this: Uint8Array) { binaryJSONCalls++; return Array.from(this); } });
  const request = {
    status: { captureId: identity.captureId, state: "materials-ready", materials: input.manifest } as never,
    assembly: input, ownerToken: "c".repeat(64), publicationId: "a".repeat(32),
    stagingRoot: join(root, "staging"), destinationPath, derivatives2, derivative2Artifacts,
  };
  try {
    const pending = publisher.publishCompletedCapture(request);
    request.ownerToken = "d".repeat(64);
    request.assembly.artifacts.find(artifact => artifact.role === "frame")!.bytes.fill(0);
    const status = await pending;
    assert.equal(status.state, "publishing");
    assert.equal("ownerToken" in status, false);
    assert.equal(dispatched?.ownerToken, "c".repeat(64));
    assert.ok(dispatched?.planBytes.byteLength);
    const plan = JSON.parse(new TextDecoder().decode(dispatched!.planBytes));
    assert.equal(plan.entries.find((entry: { role: string }) => entry.role === "frame").input.kind, "derivative2");
    assert.equal(plan.entries.find((entry: { role: string }) => entry.role === "contact-sheet").input.kind, "derivative2");
    assert.equal(plan.entries.find((entry: { role: string }) => entry.role === "frame").input.selector.requestIndex, 1);
    assert.equal(binaryJSONCalls, 0, "publication must hash binary values before JSON traversal");
    await assertAbsent(destinationPath);
  } finally {
    if (originalToJSON) Object.defineProperty(Uint8Array.prototype, "toJSON", originalToJSON);
    else Reflect.deleteProperty(Uint8Array.prototype, "toJSON");
    await rm(root, { recursive: true, force: true });
  }
});

test("identical completed-capture calls coalesce and conflicting in-flight bindings are refused", async () => {
  const input = await fixture();
  const root = await mkdtemp(join(tmpdir(), "record-composition-concurrent-"));
  const destinationPath = join(root, "final.apparent");
  assemblyForDestination(input, destinationPath);
  let startDispatch!: () => void, finishDispatch!: (status: never) => void;
  const dispatched = new Promise<void>(resolve => { startDispatch = resolve; });
  const response = new Promise<never>(resolve => { finishDispatch = resolve; });
  const sdk = {
    async handshake() { return { implementation: "native", capabilities: { publication: true } }; },
    async publish(request: { captureId: string; publicationId: string; planFile: { planSha256: string } }) {
      startDispatch();
      return response.then(() => ({ captureId: request.captureId, state: "publishing", publicationId: request.publicationId, planSha256: request.planFile.planSha256 })) as never;
    },
  } as unknown as CaptureClient;
  const publisher = createCompletedCapturePublisher(sdk, join(root, "control"), preflightCapturePublication);
  const request = {
    status: { captureId: identity.captureId, state: "materials-ready", materials: input.manifest } as never,
    assembly: input, ownerToken: "c".repeat(64), publicationId: "a".repeat(32),
    stagingRoot: join(root, "staging"), destinationPath,
  };
  try {
    const first = publisher.publishCompletedCapture(request);
    const second = publisher.publishCompletedCapture(request);
    await dispatched;
    await assert.rejects(publisher.publishCompletedCapture({ ...request, publicationId: "b".repeat(32) }), /conflicts with its in-flight publication/u);
    finishDispatch(undefined as never);
    const [left, right] = await Promise.all([first, second]);
    assert.equal(left.state, "publishing");
    assert.deepEqual(left, right);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("ambiguous composed publication reconciles the same durable binding after restart", async () => {
  const input = await fixture();
  const root = await mkdtemp(join(tmpdir(), "record-composition-reconcile-"));
  const destinationPath = join(root, "final.apparent");
  const assembly = assemblyForDestination(input, destinationPath);
  const derivatives2 = [derivative2For(input)];
  const derivative2Artifacts = {
    "frames/frame-01.png": { receiptId: "8".repeat(32), selector: { kind: "frame" as const, requestIndex: 1 } },
    "contact-sheet.png": { receiptId: "8".repeat(32), selector: { kind: "contact-sheet" as const } },
  };
  let dispatches = 0, inspections = 0;
  let expectedPublicationId = "", expectedPlanSha256 = "";
  const sdk = {
    async handshake() { return { implementation: "native", capabilities: { publication: true } }; },
    async publish(request: Parameters<CaptureClient["publish"]>[0]) {
      dispatches++;
      expectedPlanSha256 = request.planFile.planSha256;
      assert.deepEqual(request.derivatives2, derivatives2);
      const plan = JSON.parse(new TextDecoder().decode(request.planBytes));
      assert.equal(plan.entries.find((entry: { role: string }) => entry.role === "frame").input.kind, "derivative2");
      throw new Error("ambiguous SDK reply");
    },
    async inspectPublication(request: Parameters<CaptureClient["inspectPublication"]>[0]) {
      inspections++;
      assert.equal(request.publicationId, expectedPublicationId);
      assert.equal(request.planSha256, expectedPlanSha256);
      assert.deepEqual(request.expected.materials, input.manifest.key);
      assert.equal(request.expected.bundlePath, destinationPath);
      assert.equal(request.expected.manifestSha256, sha(assembly.apparentJson));
      return { captureId: request.captureId, state: "publishing", publicationId: request.publicationId, planSha256: request.planSha256 };
    },
  } as unknown as CaptureClient;
  const publicationId = "a".repeat(32), ownerToken = "c".repeat(64);
  expectedPublicationId = publicationId;
  const request = {
    status: { captureId: identity.captureId, state: "materials-ready", materials: input.manifest } as never,
    assembly: input, ownerToken, publicationId, stagingRoot: join(root, "staging"), destinationPath, derivatives2, derivative2Artifacts,
  };
  try {
    const first = createCompletedCapturePublisher(sdk, join(root, "control"), preflightCapturePublication);
    await assert.rejects(first.publishCompletedCapture(request), /outcome is unknown/u);
    assert.equal(dispatches, 1);
    await rm(request.stagingRoot, { recursive: true });
    const reopened = createCompletedCapturePublisher(sdk, join(root, "control"), preflightCapturePublication);
    const conflictingRoot = join(root, "conflicting-staging");
    await assert.rejects(reopened.publishCompletedCapture({ ...request, publicationId: "b".repeat(32),
      stagingRoot: conflictingRoot }));
    await assertAbsent(conflictingRoot);
    const observed = await reopened.reconcile({
      captureId: identity.captureId, ownerToken, publicationId, materials: input.manifest.key,
      leaseId: input.manifest.leaseId, bundleId: "sdk-bundle", bundlePath: destinationPath,
    });
    assert.equal(observed.state, "publishing");
    assert.equal(inspections, 1);
    await assert.rejects(reopened.reconcile({
      captureId: identity.captureId, ownerToken, publicationId: "b".repeat(32), materials: input.manifest.key,
      leaseId: input.manifest.leaseId, bundleId: "sdk-bundle", bundlePath: destinationPath,
    }), /conflicts with the supplied original identity/u);
    assert.equal(dispatches, 1, "reconciliation does not rerun assembly, staging, or publication");
    const original = structuredClone({ publicationId, materials: input.manifest.key,
      leaseId: input.manifest.leaseId, bundleId: "sdk-bundle", bundlePath: destinationPath });
    const direct = createRecordPublicationRecovery(sdk, join(root, "control"), preflightCapturePublication);
    const pending = direct.reconcile(identity.captureId, ownerToken, original);
    original.publicationId = "b".repeat(32);
    original.materials.materialsId = "9".repeat(32);
    assert.equal((await pending).state, "publishing", "reconciliation snapshots expectations before awaiting storage");
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("reserved composed publication resumes its exact detached derivative input after restart", async () => {
  const input = await fixture();
  const root = await mkdtemp(join(tmpdir(), "record-composition-reserved-recovery-"));
  const destinationPath = join(root, "final.apparent");
  const assembly = assemblyForDestination(input, destinationPath);
  const derivatives2 = [derivative2For(input)];
  const expectedDerivatives2 = structuredClone(derivatives2);
  const derivative2Artifacts = {
    "frames/frame-01.png": { receiptId: "8".repeat(32), selector: { kind: "frame" as const, requestIndex: 1 } },
    "contact-sheet.png": { receiptId: "8".repeat(32), selector: { kind: "contact-sheet" as const } },
  };
  const publicationId = "a".repeat(32), ownerToken = "c".repeat(64), requestId = "reserved-request";
  const expectedMaterials = structuredClone(input.manifest.key);
  const expectedMaterialsDocument = structuredClone(validateMaterialsDocument(input.manifest, input.payloadBytes));
  let dispatches = 0, inspections = 0;
  let dispatchAssertion: unknown;
  const sdk = {
    async handshake() { return { implementation: "native", capabilities: { publication: true } }; },
    async publish(request: Parameters<CaptureClient["publish"]>[0]) {
      dispatches++;
      try {
        assert.equal(request.ownerToken, ownerToken);
        assert.deepEqual(request.materials, expectedMaterialsDocument);
        assert.deepEqual(request.derivatives2, expectedDerivatives2, "recovery restores exact receipt, prepared request, and detached layout bytes");
      } catch (error) { dispatchAssertion = error; throw error; }
      return { captureId: identity.captureId, state: "saved", publicationId, planSha256: request.planFile.planSha256,
        materials: expectedMaterialsDocument.manifest, publication: { captureId: identity.captureId, materials: expectedMaterials, publicationId,
          planSha256: request.planFile.planSha256, bundleId: "sdk-bundle", bundlePath: destinationPath, artifactVersion: 1 as const,
          artifactProvenance: "native" as const, manifestSha256: sha(assembly.apparentJson), inventorySha256: "9".repeat(64) } };
    },
    async inspectPublication() { inspections++; throw new Error("not_found until a publish request is actually admitted"); },
  } as unknown as CaptureClient;
  const originalReserve = SqlitePublicationBindingStore.prototype.reserve;
  let faulted = false;
  SqlitePublicationBindingStore.prototype.reserve = async function(binding) {
    const reserved = await originalReserve.call(this, binding);
    if (!faulted) { faulted = true; throw new Error("injected crash after reservation before dispatch"); }
    return reserved;
  };
  const request = { status: { captureId: identity.captureId, state: "materials-ready", materials: input.manifest } as never,
    assembly: input, ownerToken, publicationId, stagingRoot: join(root, "staging"), destinationPath, derivatives2, derivative2Artifacts,
    requestId, requestVideoProof: true };
  try {
    const first = createRecordPublicationOutput(sdk, join(root, "control"), preflightCapturePublication);
    await assert.rejects(first.publish(request), /injected crash after reservation/u);
    const store = new SqlitePublicationBindingStore(join(root, "control"));
    const reserved = await store.get(identity.captureId);
    assert.equal(reserved?.state, "reserved");
    assert.ok(reserved?.recoveryInput);
    assert.equal(JSON.stringify(reserved).includes(ownerToken), false);
    assert.equal(dispatches, 0);

    // Discard the caller-owned derivative/material objects; resume must use only the committed binding.
    request.derivatives2[0]!.bytes.fill(0);
    input.manifest.key.revision = "f".repeat(64);
    SqlitePublicationBindingStore.prototype.reserve = originalReserve;
    const reopened = createRecordPublicationOutput(sdk, join(root, "control"), preflightCapturePublication);
    let saved: Awaited<ReturnType<typeof reopened.reconcile>> | undefined;
    let resumeError: unknown;
    try { saved = await reopened.reconcile({ captureId: identity.captureId, ownerToken, publicationId,
      materials: expectedMaterials, leaseId: input.manifest.leaseId, bundleId: "sdk-bundle", bundlePath: destinationPath }); }
    catch (error) { resumeError = error; }
    assert.equal(dispatchAssertion, undefined, String(dispatchAssertion));
    if (resumeError) throw resumeError;
    assert.ok(saved);
    assert.equal(saved.state, "saved");
    assert.equal(dispatches, 1);
    assert.equal(inspections, 0, "the reserved row could not have been known to Engine before dispatch");
    assert.equal((await store.get(identity.captureId))?.state, "saved");
    assert.equal((await reopened.captureFacts.readFacts()).captures[0]?.phase, "saved");
    assert.equal((await reopened.resolveHandoff({ captureId: requestId, artifactId: "sdk-bundle" }))?.handoff.savedBundle.bundlePath, destinationPath);
  } finally {
    SqlitePublicationBindingStore.prototype.reserve = originalReserve;
    await rm(root, { recursive: true, force: true });
  }
});

test("publication output exposes only durably matched Saved facts and handoff after restart", async () => {
  const input = await fixture();
  const root = await mkdtemp(join(tmpdir(), "record-publication-output-"));
  const destinationPath = join(root, "final.apparent");
  const assembly = assemblyForDestination(input, destinationPath);
  const ownerToken = "c".repeat(64), publicationId = "a".repeat(32), requestId = "public-request-1";
  let planSha256 = "";
  const sdk = {
    async handshake() { return { implementation: "native", capabilities: { publication: true } }; },
    async publish(request: { captureId: string; publicationId: string; planBytes: Uint8Array; planFile: { planSha256: string } }) {
      planSha256 = request.planFile.planSha256;
      return { captureId: request.captureId, state: "publishing", publicationId: request.publicationId, planSha256 };
    },
    async inspectPublication(request: { captureId: string; publicationId: string; planSha256: string; expected: { materials: MaterialsDocument["manifest"]["key"]; bundleId: string; bundlePath: string; manifestSha256: string } }) {
      return { captureId: request.captureId, state: "saved", publicationId: request.publicationId, planSha256: request.planSha256,
        materials: input.manifest, publication: { captureId: request.captureId, materials: request.expected.materials, publicationId: request.publicationId,
          planSha256: request.planSha256, bundleId: request.expected.bundleId, bundlePath: request.expected.bundlePath, artifactVersion: 1 as const,
          artifactProvenance: "native" as const, manifestSha256: request.expected.manifestSha256, inventorySha256: "9".repeat(64) } };
    },
  } as unknown as CaptureClient;
  const args = { status: { captureId: identity.captureId, state: "materials-ready", materials: input.manifest } as never,
    assembly: input, ownerToken, publicationId, stagingRoot: join(root, "staging"), destinationPath, requestId, requestVideoProof: true };
  try {
    const first = createRecordPublicationOutput(sdk, join(root, "control"), preflightCapturePublication);
    assert.equal((await first.captureFacts.readFacts()).captures.length, 0);
    assert.equal(await first.resolveHandoff({ captureId: requestId, artifactId: "sdk-bundle" }), null);
    assert.equal((await first.publish(args)).state, "publishing");
    const reserved = await new SqlitePublicationBindingStore(join(root, "control")).get(identity.captureId);
    assert.equal(reserved?.state, "publishing");
    assert.equal(reserved?.output?.requestId, requestId);
    assert.equal(reserved?.output?.processing.bundlePath, destinationPath);
    assert.equal(JSON.stringify(reserved).includes(ownerToken), false);
    assert.ok(planSha256);
    assert.equal((await first.captureFacts.readFacts()).revision, 0);
    await first.reconcile({ captureId: identity.captureId, ownerToken, publicationId, materials: input.manifest.key, leaseId: input.manifest.leaseId,
      bundleId: "sdk-bundle", bundlePath: destinationPath });
    assert.equal((await first.captureFacts.readFacts()).revision, 1, "a new Saved projection advances the stable producer revision");
    await assert.rejects(first.publish({ ...args, requestId: "changed-public-request", stagingRoot: join(root, "changed-staging") }), /already bound to publication/u);
    await assertAbsent(join(root, "changed-staging"));

    const reopened = createRecordPublicationOutput(sdk, join(root, "control"), preflightCapturePublication);
    let notifications = 0;
    reopened.captureFacts.observe!(() => { throw new Error("observer isolation"); });
    const detach = reopened.captureFacts.observe!(() => { notifications++; });
    const saved = await reopened.reconcile({ captureId: identity.captureId, ownerToken, publicationId, materials: input.manifest.key,
      leaseId: input.manifest.leaseId, bundleId: "sdk-bundle", bundlePath: destinationPath });
    assert.equal(saved.state, "saved");
    assert.equal(notifications, 1, "durable output updates wake the existing provider without another capture event");
    detach();
    const facts = await reopened.captureFacts.readFacts();
    assert.deepEqual(facts.captures, [{ captureId: requestId, artifactId: "sdk-bundle", delivery: "unknown", phase: "saved",
      outcomes: [{ kind: "save", ok: true, at: (await new SqlitePublicationBindingStore(join(root, "control")).get(identity.captureId))!.savedObservedAt }] }]);
    const resolution = await reopened.resolveHandoff({ captureId: requestId, artifactId: "sdk-bundle" });
    assert.equal(resolution?.handoff.savedBundle.bundlePath, destinationPath);
    assert.equal(resolution?.handoff.requestVideoProof, true);
    assert.equal(resolution?.handoff.processing.bundlePath, destinationPath);
    resolution!.handoff.processing.frames.length = 0;
    assert.equal((await reopened.resolveHandoff({ captureId: requestId, artifactId: "sdk-bundle" }))?.handoff.processing.frames.length, assembly.processing.frames.length);
    assert.equal((await reopened.captureFacts.readFacts()).captures[0]?.delivery, "unknown", "nativeRemoteDelivery forbidden is not local-only proof");
    assert.equal(await reopened.resolveHandoff({ captureId: identity.captureId, artifactId: "sdk-bundle" }), null, "private Engine capture ID is not the public request ID");
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("publication database atomically admits immutable processing policy beside existing bindings", async () => {
  const root = await mkdtemp(join(tmpdir(), "record-processing-admission-"));
  const captureId = "a".repeat(32), requestId = "public-request-with-separate-id";
  const store = new SqlitePublicationBindingStore(join(root, "control"));
  const older = { captureId, publicationId: "1".repeat(32), planSha256: "2".repeat(64), materials: { captureId, materialsId: "b".repeat(32), revision: "3".repeat(64) },
    leaseId: "c".repeat(32), bundleId: "record-old.apparent", bundlePath: join(root, "record-old.apparent"), manifestSha256: "4".repeat(64), state: "reserved" as const };
  const record = { schema: 1 as const, captureId, requestId, materials: older.materials, leaseId: older.leaseId, bundleId: older.bundleId,
    bundlePath: older.bundlePath, plan: { createdAtMs: 1234, request: { jobId: "5".repeat(32), requestedBundleMs: [1000, 1000], layout: { width: 400, height: 300 } }, requestVideoProof: true } };
  try {
    await store.reserve(older);
    const second = new SqlitePublicationBindingStore(join(root, "control"));
    const [firstWinner, secondWinner] = await Promise.all([store.reserveProcessing(record), second.reserveProcessing(structuredClone(record))]);
    assert.deepEqual(firstWinner, record);
    assert.deepEqual(secondWinner, record, "cross-handle concurrent admission returns one immutable winner");
    assert.equal((await store.get(captureId))?.publicationId, older.publicationId, "additive processing schema preserves existing publication rows");
    assert.deepEqual(await store.listProcessing(), [record]);
    await assert.rejects(store.reserveProcessing({ ...record, plan: { ...record.plan, createdAtMs: 9999 } }), /storage is unavailable/u);
    await assert.rejects(store.reserveProcessing({ ...record, plan: { ...record.plan, ownerToken: "c".repeat(64) } }), /storage is unavailable/u);
    const snapshot = await store.getProcessing(captureId);
    (snapshot!.plan as typeof record.plan).request.requestedBundleMs[0] = 2000;
    assert.deepEqual((await store.getProcessing(captureId))?.plan, record.plan, "reads are isolated copies");
  } finally { await rm(root, { recursive: true, force: true }); }
});

// All ports and receipts below are synthetic offline consistency fixtures; this is not native authority or platform proof.
type CoordinatorCase = "complete" | "media-failed" | "media-unknown" | "media-blocked" | "publication-unknown" | "publication-reserved" | "publication-reserved-close" | "selection-failed";
async function coordinatorCase(mode: CoordinatorCase) {
  const input = await fixture();
  const root = await realpath(await mkdtemp(join(tmpdir(), "record-capture-coordinator-")));
  let releaseMediaHandshake: () => void = () => undefined;
  const controlRoot = join(root, "control"), outputRoot = join(root, "output");
  const materialRoot = join(root, "retained-materials"), derivativeRoot = join(root, "retained-derivative");
  await Promise.all([mkdir(controlRoot, { mode: 0o700 }), mkdir(outputRoot, { mode: 0o755 }), mkdir(materialRoot, { mode: 0o700 }), mkdir(derivativeRoot, { mode: 0o700 })]);
  const requestId = "coordinator-public-request";
  const captureId = identity.captureId;
  const bundleId = `record-${sha(new TextEncoder().encode(requestId))}.apparent`;
  const bundlePath = join(outputRoot, bundleId);
  const ownerToken = "c".repeat(64), materialsId = identity.materialsId, leaseId = "c".repeat(32);
  const originalPublicationReserve = SqlitePublicationBindingStore.prototype.reserve;
  let restoreReserve = () => undefined;
  if (mode === "publication-reserved" || mode === "publication-reserved-close") {
    let faulted = false;
    SqlitePublicationBindingStore.prototype.reserve = async function(binding) {
      const reserved = await originalPublicationReserve.call(this, binding);
      if (!faulted) { faulted = true; throw new Error("injected crash after coordinator reservation before dispatch"); }
      return reserved;
    };
    restoreReserve = () => { SqlitePublicationBindingStore.prototype.reserve = originalPublicationReserve; };
  }
  const seal = async (relativePath: string, content: Uint8Array) => {
    const path = join(materialRoot, relativePath);
    await mkdir(join(path, ".."), { recursive: true, mode: 0o700 });
    await writeFile(path, content, { mode: 0o600 });
    const stat = await lstat(path, { bigint: true });
    return { relativePath, sha256: sha(content), identity: { device: String(stat.dev), inode: String(stat.ino), size: Number(stat.size), modifiedNs: String(stat.mtimeNs) } };
  };
  try {
    const payload = JSON.parse(new TextDecoder().decode(input.payloadBytes));
    payload.root = materialRoot;
    payload.bundleId = bundleId;
    payload.identity = { captureId, materialsId };
    payload.leaseId = leaseId;
    const sourceFiles = input.artifacts.filter(artifact => artifact.role === "video").map(artifact => artifact.bytes);
    const factFiles = Object.values(input.facts).map(fact => fact!.bytes);
    const materialBytes = [sourceFiles[0]!, ...factFiles];
    const originalPaths = ["capture.mp4", "facts/context.json", "facts/interactions.json", "facts/transcript.json"];
    payload.files = [];
    for (let i = 0; i < originalPaths.length; i++) payload.files.push(await seal(originalPaths[i]!, materialBytes[i]!));
    const payloadBytes = bytes(payload);
    await writeFile(join(materialRoot, "materials.json"), payloadBytes, { mode: 0o600 });
    const payloadStat = await lstat(join(materialRoot, "materials.json"), { bigint: true });
    input.payloadBytes = payloadBytes;
    input.manifest.root = materialRoot;
    input.manifest.leaseId = leaseId;
    input.manifest.key = { captureId, materialsId, revision: sha(payloadBytes) };
    input.manifest.payloadFile = { relativePath: "materials.json", sha256: sha(payloadBytes), identity: { device: String(payloadStat.dev), inode: String(payloadStat.ino), size: Number(payloadStat.size), modifiedNs: String(payloadStat.mtimeNs) } };

    const png = input.artifacts.find(artifact => artifact.role === "frame")!.bytes;
    const sheet = input.artifacts.find(artifact => artifact.role === "contact-sheet")!.bytes;
    await mkdir(join(derivativeRoot, "native"), { mode: 0o700 });
    await writeFile(join(derivativeRoot, "native/frame.png"), png, { mode: 0o600 });
    await writeFile(join(derivativeRoot, "native/sheet.png"), sheet, { mode: 0o600 });
    const derivativeSeal = async (relativePath: string, content: Uint8Array) => {
      const stat = await lstat(join(derivativeRoot, relativePath), { bigint: true });
      return { relativePath, sha256: sha(content), identity: { device: String(stat.dev), inode: String(stat.ino), size: Number(stat.size), modifiedNs: String(stat.mtimeNs) } };
    };
    const frameSeal = await derivativeSeal("native/frame.png", png), sheetSeal = await derivativeSeal("native/sheet.png", sheet);

    const mapping = { requestId, captureId, ownerToken, bundleId, bundlePath, capture2Intent: "synthetic-retained-intent" };
    const mappings = {
      async listRequestIds(protocol?: "v1" | "capture2") { return protocol === "capture2" ? [requestId] : []; },
      async getByRequestId(id: string) { return id === requestId ? structuredClone(mapping) : undefined; },
      async getByCaptureId(id: string) { return id === captureId ? structuredClone(mapping) : undefined; },
      async putIfAbsent(value: typeof mapping) { return structuredClone(value); },
      async putCapture2IfUnchanged(value: typeof mapping) { return structuredClone(value); },
    };
    const materialsManifest = structuredClone(input.manifest);
    const captureClient = {
      async handshake() { return { implementation: "native", readiness: "ready", capabilities: { sourceDiscovery: true, displayCapture: true, areaCapture: false, audioCapture: false, contextFacts: false, interactionFacts: false, candidates: false, transcript: false, strokes: false, publication: true, retainedMaterials: true } }; },
      async discoverSources() { return { state: "available", sources: [{ id: "source", generation: "d".repeat(32), kind: "display", displayId: 1, bounds: { x: 0, y: 0, width: 800, height: 600, scale: 1 } }] }; },
      async inspect() { return { captureId, state: "materials-ready", materials: materialsManifest }; },
      async start() { return { captureId, state: "recording" }; }, async stop() { return { captureId, state: "materials-ready", materials: materialsManifest }; },
      async cancel() { return { captureId, state: "cancelled" }; }, async list() { return [{ captureId, state: "materials-ready", materials: materialsManifest }]; },
      async publish(request: Parameters<CaptureClient["publish"]>[0]) {
        publishes++;
        const prepared = preflightCapturePublication(request);
        if (mode === "publication-unknown") throw new Error("lost reply " + ownerToken);
        return { captureId, state: "publishing", publicationId: request.publicationId, planSha256: request.planFile.planSha256 };
      },
      async inspectPublication(request: Parameters<CaptureClient["inspectPublication"]>[0]) {
        if (mode === "publication-unknown") throw new Error("lost inspection " + ownerToken);
        if (mode === "publication-reserved" && publishes === 0) throw new Error("not_found before publish dispatch");
        return { captureId, state: "saved", publicationId: request.publicationId, planSha256: request.planSha256, materials: materialsManifest,
          publication: { captureId, materials: request.expected.materials, publicationId: request.publicationId, planSha256: request.planSha256,
            bundleId: request.expected.bundleId, bundlePath: request.expected.bundlePath, artifactVersion: 1 as const, artifactProvenance: "native" as const,
            manifestSha256: request.expected.manifestSha256, inventorySha256: "9".repeat(64) } };
      },
    } as unknown as CaptureClient;
    let derives = 0, publishes = 0, mediaNegotiated = false;
    let signalMediaHandshake!: () => void;
    const mediaHandshakeEntered = new Promise<void>(resolve => { signalMediaHandshake = resolve; });
    const mediaHandshakeGate = new Promise<void>(resolve => { releaseMediaHandshake = resolve; });
    const mediaClient = new Media2Client({
      async request(rpc: Media2RpcRequest) {
        if (rpc.method === "handshake") {
          if (mode === "media-blocked") { signalMediaHandshake(); await mediaHandshakeGate; }
          mediaNegotiated = true;
          return { channel: "media", protocolVersion: 2, rpcId: rpc.rpcId, result: {
            channel: "media", protocolVersion: 2, implementation: "native",
            capabilities: { retainedFrames: true, contactSheet: true, nativeJobAuthority: true },
          } };
        }
        assert.equal(mediaNegotiated, true, "fresh public Media2Client requires handshake before derive");
        if (rpc.method === "inspect" && mode === "media-unknown") throw new Error("lost inspect " + ownerToken);
        assert.equal(rpc.method, "derive", "synthetic single-dispatch case has no implicit cancel or repeat");
        if (rpc.method !== "derive") throw new Error("unexpected synthetic media operation");
        const admitted = await new SqlitePublicationBindingStore(controlRoot).getProcessing(captureId);
        assert.ok(admitted, "original processing policy is durable before media dispatch");
        assert.deepEqual((admitted.plan as { requestVideoProof: boolean }).requestVideoProof, true);
        assert.equal(JSON.stringify(admitted).includes(ownerToken), false);
        derives++;
        if (mode === "media-unknown") throw new Error("lost derive " + ownerToken);
        if (mode === "media-failed") return { channel: "media", protocolVersion: 2, rpcId: rpc.rpcId,
          result: { jobId: rpc.jobId, requestSha256: rpc.requestSha256, state: "failed", error: { code: "decode_failed" } } };
        const request = JSON.parse(rpc.requestJson);
        const document = derivative2For(input, [...request.requestedBundleMs], { outputDirectory: derivativeRoot, frameFiles: [frameSeal], sheetFile: sheetSeal, jobId: request.jobId });
        return { channel: "media", protocolVersion: 2, rpcId: rpc.rpcId, result: {
          jobId: request.jobId, requestSha256: rpc.requestSha256, state: "complete",
          derivative: { envelope: document.envelope, receiptJson: Buffer.from(document.bytes).toString("utf8") },
        } };
      },
      async close() { throw new Error("coordinator must not close caller-owned media client"); },
    });
    const destinations = new AbsentDestinationAllocator(controlRoot, outputRoot);
    let selections = 0;
    const options = {
      captureClient, mediaClient, mappings: mappings as never, destinations, privateControlRoot: controlRoot,
      createStartIntent: () => { throw new Error("preexisting synthetic capture must not be restarted"); },
      trustedMaterialsRoot: () => materialRoot,
      selectProcessing: ({ materials }: { mapping: Readonly<{ requestId: string }>; materials: MaterialsDocument }) => { selections++; if (mode === "selection-failed") throw new Error("invalid selection " + ownerToken); return ({
        request: { outputDirectory: derivativeRoot, requestedBundleMs: [1000], limits: { maxFrames: 2, maxEdge: 4096, maxTotalPixels: 64_000_000, deadlineMs: 10_000 },
          contactSheet: { schema: "apparent.contact-sheet-layout/1", width: 400, height: 300, backgroundRgb: [0, 0, 0], fit: "stretch", sampling: "legacy-bilinear", clipping: "reject",
            tiles: [{ requestIndex: 1, x: 0, y: 0, width: 400, height: 300 }] } },
        materialsLimits: { maxFileBytes: 4_000_000, maxAggregateBytes: 8_000_000 }, artifactLimits: { maxFileBytes: 4_000_000, maxAggregateBytes: 8_000_000 },
        frames: [{ requestIndex: 1, index: 1, timeMs: 1000, text: "synthetic selected frame", reason: "fixture selection", event: "fixture", screenName: "display", bundlePath: "frames/frame-01.png", canonicalReason: "scene_change" }],
        contactSheetPath: "contact-sheet.png", admission: { createdAtMs: 1234, destination: { session: "synthetic-session", title: "", via: "sdk" } }, marks: [], requestVideoProof: true,
      }); },
      preflight: preflightCapturePublication, pollIntervalMs: 10000,
    };
    const coordinator = await createRecordCaptureCoordinator(options as never);
    try {
      if (mode === "media-blocked") {
        await mediaHandshakeEntered;
        const stopping = coordinator.shutdown();
        await new Promise(resolve => setImmediate(resolve));
        releaseMediaHandshake();
        const shutdown = await stopping;
        assert.equal(derives, 0, "shutdown prevents a queued derive after an admitted handshake drains");
        assert.equal(publishes, 0, "shutdown does not start publication after the media handshake drains");
        assert.equal(shutdown.runtimeClearance, "unresolved");
        assert.ok(shutdown.work.some(item => item.owner === "media" && item.state === "pending"));
        return;
      }
      if (mode === "publication-reserved-close") {
        await coordinator.reconcile().catch(() => undefined);
        assert.equal(publishes, 0, "fault left an undispatched durable reservation");
        const originalGet = SqlitePublicationBindingStore.prototype.get;
        let entered!: () => void, release!: () => void;
        const gateEntered = new Promise<void>(resolve => { entered = resolve; });
        const gate = new Promise<void>(resolve => { release = resolve; });
        let gated = false;
        SqlitePublicationBindingStore.prototype.get = async function(capture) {
          const binding = await originalGet.call(this, capture);
          if (!gated && binding?.state === "reserved") { gated = true; entered(); await gate; }
          return binding;
        };
        const inspecting = coordinator.reconcile().catch(() => undefined);
        try {
          await gateEntered;
          const closing = coordinator.shutdown();
          release();
          await inspecting;
          const report = await closing;
          assert.equal(publishes, 0, "shutdown cannot resume a reserved publication after admission closes");
          assert.equal((await originalGet.call(new SqlitePublicationBindingStore(controlRoot), captureId))?.state, "reserved");
          assert.equal(report.runtimeClearance, "unresolved");
        } finally { release(); await inspecting; SqlitePublicationBindingStore.prototype.get = originalGet; }
        return;
      }
      if (mode !== "complete") {
        let notices = 0;
        const detach = coordinator.observeProcessingIssues(() => { notices++; });
        coordinator.observeProcessingIssues(() => { throw new Error("isolated observer"); });
        await coordinator.reconcile().catch(() => undefined);
        const expectedCode = { "media-failed": "media-failed", "media-unknown": "media-outcome-unknown",
          "publication-unknown": "publication-outcome-unknown", "publication-reserved": "publication-outcome-unknown", "selection-failed": "processing-failed" }[mode];
        const issues = coordinator.processingIssues();
        assert.ok(issues.some(issue => issue.requestId === requestId && issue.code === expectedCode));
        assert.ok(notices > 0, "automatic outcomes notify the host without requiring an explicit error-returning call");
        assert.equal(JSON.stringify(issues).includes(ownerToken), false, "public errors never expose private exception data");
        const facts = await coordinator.captureFacts.readFacts();
        assert.equal(facts.captures[0]?.phase, mode.endsWith("unknown") || mode === "publication-reserved" ? "unknown" : "failed");
        assert.equal(facts.captures[0]?.outcomes.some(outcome => outcome.kind === "processing" && !outcome.ok && outcome.detail === expectedCode), true);
        const provider = coordinator.createProvider({ providerEpoch: "coordinator-failure-consumer" });
        try {
          await new Promise<void>((resolve, reject) => {
            const deadline = setTimeout(() => { detach(); reject(new Error("processing issue was not observable through the public provider")); }, 5000);
            const detach = provider.observe(snapshot => {
              if (snapshot.captures.some(capture => capture.captureId === requestId && capture.phase === facts.captures[0]?.phase)) {
                clearTimeout(deadline); detach(); resolve();
              }
            });
          });
          const current = provider.current();
          const result = await provider.execute({ type: "inspect-capture", captureId: requestId, requestId: "explicit-processing-inspection",
            expected: { providerEpoch: current.providerEpoch, revision: current.revision } });
          assert.equal(result.status, "accepted", "stopped output failures permit inspection of the same original processing identity");
        } finally { provider.detach(); }
        assert.equal(await coordinator.resolveHandoff({ captureId: requestId, artifactId: bundleId }), null);
        issues[0]!.code = "startup-recovery-failed";
        assert.equal(coordinator.processingIssues().some(issue => issue.code === expectedCode), true, "issue reads are isolated");
        detach();
        const beforeShutdown = { derives, publishes };
        const shutdown = await coordinator.shutdown();
        assert.equal(shutdown.admission, "closed");
        assert.equal(shutdown.localDrain, "complete");
        assert.equal(shutdown.scope, "record-coordinator");
        assert.equal(shutdown.nextShortcutAdmission.state, "unsupported", "an unavailable shortcut producer is not evidence of idle");
        assert.equal(JSON.stringify(shutdown).includes(ownerToken), false, "shutdown facts do not expose private authority or exception text");
        assert.equal(derives, beforeShutdown.derives, "shutdown inspection never dispatches retained media");
        if (mode === "publication-reserved")
          assert.ok(publishes <= 1, "shutdown drains at most the single publication already admitted by the reserved-recovery case");
        else assert.equal(publishes, beforeShutdown.publishes, "shutdown inspection never dispatches publication");
        assert.equal(await coordinator.shutdown(), shutdown, "shutdown is idempotent and returns its immutable snapshot");
        assert.equal(shutdown.runtimeClearance, mode.endsWith("unknown") ? "unknown" : mode === "selection-failed" || mode === "publication-reserved" ? "unresolved" : "clear", JSON.stringify(shutdown));
        await assert.rejects(coordinator.captureFacts.readFacts(), /closing/u, "late reads cannot reopen ownership after shutdown");
        await coordinator.dispose();
        restoreReserve();
        if (mode === "publication-reserved") await Promise.all([rm(materialRoot, { recursive: true, force: true }), rm(derivativeRoot, { recursive: true, force: true })]);
        const reopened = await createRecordCaptureCoordinator(options as never);
        try {
          await reopened.reconcile().catch(() => undefined);
          if (mode === "publication-reserved")
            assert.equal(reopened.processingIssues().some(issue => issue.code === expectedCode), false, "successful reserved resume clears the prior publication issue");
          else assert.equal(reopened.processingIssues().some(issue => issue.code === expectedCode), true, "restart reconstructs failure from original durable job or policy");
          assert.equal(derives, mode === "selection-failed" ? 0 : 1, "unknown and failed jobs never redispatch on restart");
          assert.equal(publishes, mode === "publication-unknown" || mode === "publication-reserved" ? 1 : 0,
            "dispatching publication only inspects, while an undispatched reservation resumes its original request");
          if (mode === "publication-reserved") {
            await reopened.reconcile();
            assert.equal((await reopened.captureFacts.readFacts()).captures[0]?.phase, "saved");
            assert.equal(selections, 1, "reserved publication restart does not rerun processing selection");
            assert.equal(derives, 1, "reserved publication restart does not rerun media derivation");
            assert.equal(publishes, 1, "reserved publication restart dispatches its original tuple once");
          }
        } finally { await reopened.dispose(); }
        return;
      }
      assert.deepEqual((await coordinator.captureFacts.readFacts()).captures, [], "materials-ready and publishing are not exposed as Saved");
      assert.equal(await coordinator.resolveHandoff({ captureId: requestId, artifactId: bundleId }), null, "pending publication has no handoff");
      await coordinator.reconcile();
      if ((await coordinator.captureFacts.readFacts()).captures.length === 0) await coordinator.reconcile();
      const facts = await coordinator.captureFacts.readFacts();
      assert.deepEqual(facts.captures.map(value => [value.captureId, value.artifactId, value.phase, value.delivery]), [[requestId, bundleId, "saved", "unknown"]]);
      const resolved = await coordinator.resolveHandoff({ captureId: requestId, artifactId: bundleId });
      assert.equal(resolved?.handoff.savedBundle.bundlePath, bundlePath);
      assert.equal(resolved?.handoff.processing.frames[0]?.reason, "fixture selection");
      assert.equal(derives, 1);
      assert.equal(selections, 1, "restart reuses immutable persisted frame/layout/admission decisions");
      assert.equal((await new SqlitePublicationBindingStore(controlRoot).list()).length, 1);
      assert.equal(JSON.stringify(await new SqlitePublicationBindingStore(controlRoot).listProcessing()).includes(ownerToken), false);
      assert.equal(publishes, 1, "the original assembled plan is published once");
      const shutdown = await coordinator.shutdown();
      assert.equal(shutdown.runtimeClearance, "clear", "a terminal Saved binding and processing journal leave no unresolved coordinator ownership");
      assert.deepEqual(shutdown.work, []);
      assert.equal(shutdown.nextShortcutAdmission.state, "unsupported");
      assert.equal(await coordinator.shutdown(), shutdown);
      await rm(materialRoot, { recursive: true, force: true });
      await coordinator.dispose();
      const reopened = await createRecordCaptureCoordinator(options as never);
      try {
        assert.equal((await reopened.resolveHandoff({ captureId: requestId, artifactId: bundleId }))?.handoff.savedBundle.bundlePath, bundlePath);
        assert.equal(derives, 1, "restart resolves Saved without rereading deleted retained materials or deriving again");
        assert.equal(selections, 1, "restart does not reconstruct processing policy from caller glue");
        assert.equal(publishes, 1, "restart reconciles the same publication binding without redispatch");
      } finally { await reopened.dispose(); }
    } finally { await coordinator.dispose(); }
  } finally { restoreReserve(); releaseMediaHandshake(); await rm(root, { recursive: true, force: true }); }
}

for (const mode of ["complete", "media-failed", "media-unknown", "media-blocked", "publication-unknown", "publication-reserved", "publication-reserved-close", "selection-failed"] as const)
  test(`capture coordinator retains the original identity and exposes ${mode}`, () => coordinatorCase(mode));

for (const mode of ["pending", "failed"] as const) test(`coordinator startup ${mode} is observable and drains before cleanup`, { timeout: 5000 }, async () => {
  const root = await realpath(await mkdtemp(join(tmpdir(), "record-coordinator-startup-")));
  let signalScan!: () => void;
  const secondScan = new Promise<void>(resolve => { signalScan = resolve; });
  let scans = 0, release!: (ids: string[]) => void;
  const mappings = {
    async listRequestIds() {
      if (++scans === 1) return [];
      if (mode === "failed") throw new Error("injected private storage failure");
      if (scans === 2) {
        signalScan();
        return new Promise<string[]>(resolve => { release = resolve; });
      }
      return [];
    },
    async getByRequestId() { return undefined; }, async getByCaptureId() { return undefined; },
    async putIfAbsent() { throw new Error("unused"); }, async putCapture2IfUnchanged() { throw new Error("unused"); },
  };
  const coordinator = await createRecordCaptureCoordinator({
    captureClient: { async handshake() { return { implementation: "native", readiness: "unavailable", capabilities: {} }; } } as unknown as CaptureClient,
    mediaClient: {} as Media2Client, mappings, destinations: new AbsentDestinationAllocator(join(root, "control"), join(root, "bundles")),
    privateControlRoot: join(root, "control"), createStartIntent() { throw new Error("unused"); },
    trustedMaterialsRoot() { throw new Error("unused"); }, selectProcessing() { throw new Error("unused"); }, preflight: preflightCapturePublication,
  });
  try {
    if (mode === "failed") {
      await coordinator.reconcile().catch(() => undefined);
      assert.ok(coordinator.processingIssues().some(issue => issue.code === "startup-recovery-failed" && issue.requestId === undefined));
    } else {
      let settled = false;
      const draining = coordinator.shutdown().then(() => { settled = true; });
      await secondScan;
      assert.equal(settled, false, "dispose must await its own already-running startup work");
      assert.equal(typeof release, "function");
      release([]);
      await draining;
      assert.equal(settled, true);
      assert.equal(scans, 5, "startup, shutdown's final native scan, and durable mapping snapshot complete after the blocked scan drains");
    }
  } finally { await coordinator.dispose(); await rm(root, { recursive: true, force: true }); }
});

for (const blocked of ["discovery", "background-poll"] as const)
test(`coordinator shutdown drains an admitted ${blocked}`, { timeout: 5000 }, async () => {
  const root = await realpath(await mkdtemp(join(tmpdir(), "record-coordinator-start-drain-")));
  await Promise.all([mkdir(join(root, "control"), { mode: 0o700 }), mkdir(join(root, "bundles"), { mode: 0o755 })]);
  let entered!: () => void, release!: () => void;
  const discoveryEntered = new Promise<void>(resolve => { entered = resolve; });
  const discoveryGate = new Promise<void>(resolve => { release = resolve; });
  const source = { id: "a".repeat(32), generation: "b".repeat(32), kind: "display" as const,
    displayId: 1, bounds: { x: 0, y: 0, width: 1280, height: 720, scale: 1 } };
  const rows = new Map<string, { requestId: string; captureId: string; ownerToken: string; bundleId: string; bundlePath: string; capture2Intent: string }>();
  const mappings = {
    async listRequestIds(protocol?: "v1" | "capture2") { if (blocked === "background-poll" && pollEntered) throw new Error("mapping read failed during shutdown"); return protocol === "capture2" ? [...rows.keys()] : []; },
    async getByRequestId(requestId: string) { return rows.get(requestId); },
    async getByCaptureId(captureId: string) { return [...rows.values()].find(row => row.captureId === captureId); },
    async putIfAbsent() { throw new Error("unused"); },
    async putCapture2IfUnchanged(value: (typeof rows extends Map<string, infer T> ? T : never)) { rows.set(value.requestId, structuredClone(value)); return structuredClone(value); },
  };
  let startDispatches = 0, cancelDispatches = 0, pollEntered = false;
  const captureClient = {
    clientId: "d".repeat(32),
    async handshake() { return { implementation: "native", readiness: "ready", capabilities: { sourceDiscovery: true, displayCapture: true, retainedMaterials: true } }; },
    async discoverSources() { if (blocked === "discovery") { entered(); await discoveryGate; } return { state: "available", sources: [source] }; },
    async start(request: { captureId: string }) { startDispatches++; return { captureId: request.captureId, state: "recording" }; },
    async inspect(request: { captureId: string }) { if (blocked === "background-poll" && !pollEntered) { pollEntered = true; entered(); await discoveryGate; } return { captureId: request.captureId, state: "recording" }; },
    async cancel() { cancelDispatches++; throw new Error("shutdown must not cancel native work"); },
    async stop() { throw new Error("unused"); }, async publish() { throw new Error("unused"); }, async inspectPublication() { throw new Error("unused"); },
  } as unknown as CaptureClient;
  const coordinator = await createRecordCaptureCoordinator({
    captureClient, mediaClient: {} as Media2Client, mappings: mappings as never,
    destinations: new AbsentDestinationAllocator(join(root, "control"), join(root, "bundles")), privateControlRoot: join(root, "control"), pollIntervalMs: 100,
    createStartIntent(_requestId, discovery) { return { source: discovery.sources[0]!, options: { audio: "none", localOnly: true },
      invocation: { invocationId: "c".repeat(32), observedAtMs: 1000, kind: "explicit" }, destinationPath: join(root, "bundles", "capture.apparent") }; },
    trustedMaterialsRoot() { throw new Error("unused"); }, selectProcessing() { throw new Error("unused"); }, preflight: preflightCapturePublication,
  });
  const originalList = SqlitePublicationBindingStore.prototype.list;
  const originalPlans = SqlitePublicationBindingStore.prototype.listProcessing;
  try {
    const starting = coordinator.capture2Start.startCapture("shutdown-start");
    let readyDeadline: ReturnType<typeof setTimeout> | undefined;
    await Promise.race([discoveryEntered, new Promise<never>((_, reject) => {
      readyDeadline = setTimeout(() => reject(new Error("owned observation did not enter")), 2000);
    })]).finally(() => clearTimeout(readyDeadline));
    if (blocked === "background-poll") {
      // Isolate the owned RPC drain from unrelated filesystem scheduling.
      SqlitePublicationBindingStore.prototype.list = async () => [];
      SqlitePublicationBindingStore.prototype.listProcessing = async () => [];
    }
    let settled = false;
    const stopping = coordinator.shutdown().then(report => { settled = true; return report; });
    assert.equal((await coordinator.capture2Start.startCapture("late-start")).outcome, "refused");
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(settled, false, `shutdown drains the already admitted ${blocked} operation`);
    release();
    const startResult = await starting;
    assert.equal(startResult.outcome, "accepted");
    const report = await stopping;
    assert.equal(report.runtimeClearance, blocked === "background-poll" ? "unknown" : "unresolved");
    assert.ok(report.work.some(item => item.owner === "capture" && item.state === "active" && item.requestId === "shutdown-start"));
    assert.equal(startDispatches, 1, JSON.stringify({ startResult, mapped: rows.has("shutdown-start"), report }));
    assert.equal(cancelDispatches, 0, "shutdown does not cancel a native capture");
  } finally { release(); await coordinator.dispose(); SqlitePublicationBindingStore.prototype.list = originalList; SqlitePublicationBindingStore.prototype.listProcessing = originalPlans; await rm(root, { recursive: true, force: true }); }
});

test("coordinator disposal drains provider and direct output reads before private cleanup", { timeout: 5000 }, async () => {
  const root = await realpath(await mkdtemp(join(tmpdir(), "record-output-read-drain-")));
  const originalList = SqlitePublicationBindingStore.prototype.list;
  let release!: () => void;
  const barrier = new Promise<void>(resolve => { release = resolve; });
  let entered = 0;
  const admittedReads: Promise<unknown>[] = [];
  SqlitePublicationBindingStore.prototype.list = function () {
    entered++;
    const read = barrier.then(() => originalList.call(this));
    admittedReads.push(read);
    return read;
  };
  const reads: Promise<unknown>[] = [];
  let coordinator: Awaited<ReturnType<typeof createRecordCaptureCoordinator>> | undefined;
  let provider: ReturnType<NonNullable<typeof coordinator>["createProvider"]> | undefined;
  try {
    coordinator = await createRecordCaptureCoordinator({
      captureClient: { async handshake() { return { implementation: "native", readiness: "unavailable", capabilities: {} }; } } as unknown as CaptureClient,
      mediaClient: {} as Media2Client,
      mappings: { async listRequestIds() { return []; }, async getByRequestId() { return undefined; }, async getByCaptureId() { return undefined; },
        async putIfAbsent() { throw new Error("unused"); }, async putCapture2IfUnchanged() { throw new Error("unused"); } },
      destinations: new AbsentDestinationAllocator(join(root, "control"), join(root, "bundles")), privateControlRoot: join(root, "control"),
      createStartIntent() { throw new Error("unused"); }, trustedMaterialsRoot() { throw new Error("unused"); },
      selectProcessing() { throw new Error("unused"); }, preflight: preflightCapturePublication,
    });
    provider = coordinator.createProvider({ providerEpoch: "pending-output-reader" });
    reads.push(coordinator.captureFacts.readFacts(), coordinator.resolveHandoff({ captureId: "unavailable", artifactId: "unavailable" }));
    assert.equal(entered, 3, "provider startup and both public read projections are admitted");
    provider.detach();
    let drained = false;
    const disposal = coordinator.dispose().then(() => { drained = true; });
    await new Promise(resolve => setImmediate(resolve));
    assert.equal(drained, false, "disposal cannot finish while its output database reads are pending");
    release();
    await disposal;
    await Promise.all(reads);
    assert.equal(drained, true);
    await rm(root, { recursive: true, force: true });
    await assert.rejects(coordinator.captureFacts.readFacts(), /closing|disposed/);
    await assert.rejects(coordinator.resolveHandoff({ captureId: "unavailable", artifactId: "unavailable" }), /closing|disposed/);
    assert.equal(entered, 4, "only the three admitted projections and one shutdown snapshot read the database");
    await assert.rejects(lstat(root), { code: "ENOENT" });
  } finally {
    release();
    provider?.detach();
    await Promise.allSettled(reads);
    await Promise.allSettled(admittedReads);
    await coordinator?.dispose();
    SqlitePublicationBindingStore.prototype.list = originalList;
    await rm(root, { recursive: true, force: true });
  }
});

test("refuses an unmarked populated private namespace without changing its contents", async () => {
  const source = await fixture();
  const root = await mkdtemp(join(tmpdir(), "record-stage-unmarked-"));
  try {
    const destinationPath = join(root, "destination.apparent");
    const assembly = assemblyForDestination(source, destinationPath);
    const request = stagingInput(source, assembly, root, "1".repeat(32), destinationPath);
    const namespace = join(request.stagingRoot, request.publicationId);
    await mkdir(namespace, { recursive: true, mode: 0o700 });
    const knownFile = join(namespace, "apparent.json");
    await writeFile(knownFile, "unrelated private bytes", { mode: 0o600 });
    await assert.rejects(prepareRecordPublication(request), /unmarked|marker|namespace|state/u);
    assert.equal(await readFile(knownFile, "utf8"), "unrelated private bytes");
    await assertAbsent(join(namespace, ".record-publication"));
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("rejects assembly materials keys from a foreign capture despite matching revision and lease", async () => {
  const source = await fixture();
  const root = await mkdtemp(join(tmpdir(), "record-stage-foreign-key-"));
  try {
    const destinationPath = join(root, "destination.apparent");
    const assembly = assemblyForDestination(source, destinationPath);
    const foreignInventory = structuredClone(assembly);
    foreignInventory.inventory.materials.captureId = "9".repeat(32);
    await assert.rejects(prepareRecordPublication(stagingInput(source, foreignInventory, root, "4".repeat(32), destinationPath)), /materials|identity|capture|binding/u);
    const foreignAssemblyKey = structuredClone(assembly);
    foreignAssemblyKey.materials.manifest.key.captureId = "9".repeat(32);
    await assert.rejects(prepareRecordPublication(stagingInput(source, foreignAssemblyKey, root, "5".repeat(32), destinationPath)), /materials|identity|capture|binding/u);
    await assertAbsent(join(root, "private"));
  } finally { await rm(root, { recursive: true, force: true }); }
});
