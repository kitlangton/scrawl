import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { lstat, mkdir, mkdtemp, readFile, rm, symlink, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import test from "node:test";
import { assembleCanonicalBundle, type BundleAssemblyInput } from "@apparent/record/processing/bundle";
import { prepareRecordPublication } from "../../dist/backend/publication/staging.js";
import { createCompletedCapturePublisher } from "../../dist/backend/publication/completed-capture.js";
import { CaptureClient, CapturePublicationPreflightError, preflightCapturePublication, validateFactFile, validateMaterialsDocument, type MaterialsDocument, type StrokeFacts } from "@apparent/engine-sdk/capture";
import { createRecordPublicationRecovery } from "../../dist/backend/publication/recovery.js";
import { ReferenceCapturePeer } from "@apparent/engine-sdk/capture/testing";
import { SqlitePublicationBindingStore } from "../../dist/backend/publication/store.js";
import { encodeContactSheetLayout, prepareMedia2Request, selectDerivative2Artifact, validateDerivative2Document, type Derivative2DocumentInput } from "@apparent/engine-sdk/media2";

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

function derivative2For(input: BundleAssemblyInput, requestedBundleMs = [1000]): Derivative2DocumentInput {
  const materials = validateMaterialsDocument(input.manifest, input.payloadBytes);
  const layout = { schema: "apparent.contact-sheet-layout/1" as const, width: 400, height: 300, backgroundRgb: [0, 0, 0] as [number, number, number], fit: "stretch" as const, sampling: "legacy-bilinear" as const, clipping: "reject" as const, tiles: [{ requestIndex: 1, x: 0, y: 0, width: 400, height: 300 }] };
  const request = prepareMedia2Request({ schema: "apparent.retained-frame-request/1", materials: materials.manifest.key, leaseId: materials.payload.leaseId,
    jobId: "9".repeat(32), outputDirectory: "/private/sdk-output/derivative-job", requestedBundleMs,
    limits: { maxFrames: 2, maxEdge: 4096, maxTotalPixels: 64_000_000, deadlineMs: 10_000 }, contactSheet: layout }, materials).request;
  const sourceFrame = input.artifacts.find(artifact => artifact.role === "frame")!;
  const sourceSheet = input.artifacts.find(artifact => artifact.role === "contact-sheet")!;
  const frameFiles = requestedBundleMs.map((_, index) => ({ relativePath: `native/frame-${index + 1}.png`,
    identity: index === 0 ? sourceFrame.identity : { ...sourceFrame.identity, inode: `native-frame-${index + 1}` }, sha256: sourceFrame.sha256 }));
  const sheetFile = { relativePath: "native/sheet.png", identity: sourceSheet.identity, sha256: sourceSheet.sha256 };
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
      "frames/frame-01.png": { receiptId: "8".repeat(32), selector: { kind: "frame" as const, requestIndex: 1 } },
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
      ["unknown path", (_assembly, _document, bindings) => { bindings["unknown.png"] = bindings["frames/frame-01.png"]!; }],
    ];
    for (const [name, mutate] of mismatches) {
      const stagingRoot = join(root, `refuse-${name}`);
      const assemblyCopy = structuredClone(assembly);
      const document = structuredClone(derivatives2[0]!);
      const bindings = structuredClone(derivative2Artifacts);
      mutate(assemblyCopy, document, bindings);
      await assert.rejects(prepareRecordPublication({ ...stagingInput(input, assemblyCopy, stagingRoot, `${name.length}`.repeat(32), destinationPath), derivatives2: [document], derivative2Artifacts: bindings }));
      await assertAbsent(stagingRoot);
    }
  } finally { await rm(root, { recursive: true, force: true }); }
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
  assemblyForDestination(input, destinationPath);
  let dispatches = 0, inspections = 0;
  let expectedPublicationId = "";
  const sdk = {
    async handshake() { return { implementation: "native", capabilities: { publication: true } }; },
    async publish() { dispatches++; throw new Error("ambiguous SDK reply"); },
    async inspectPublication(request: { captureId: string; publicationId: string; planSha256: string }) {
      inspections++;
      assert.equal(request.publicationId, expectedPublicationId);
      return { captureId: request.captureId, state: "publishing", publicationId: request.publicationId, planSha256: request.planSha256 };
    },
  } as unknown as CaptureClient;
  const publicationId = "a".repeat(32), ownerToken = "c".repeat(64);
  expectedPublicationId = publicationId;
  const request = {
    status: { captureId: identity.captureId, state: "materials-ready", materials: input.manifest } as never,
    assembly: input, ownerToken, publicationId, stagingRoot: join(root, "staging"), destinationPath,
  };
  try {
    const first = createCompletedCapturePublisher(sdk, join(root, "control"), preflightCapturePublication);
    await assert.rejects(first.publishCompletedCapture(request), /outcome is unknown/u);
    assert.equal(dispatches, 1);
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
