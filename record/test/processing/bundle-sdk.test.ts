import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { join } from "node:path";
import { tmpdir } from "node:os";
import test from "node:test";
import { assembleCanonicalBundle, type BundleAssemblyInput } from "@apparent/record/processing/bundle";
import { prepareRecordPublication } from "../../dist/backend/publication/staging.js";

const identity = { captureId: "a".repeat(32), materialsId: "b".repeat(32) };
const bytes = (value: unknown) => new TextEncoder().encode(JSON.stringify(value));
const sha = (value: Uint8Array) => createHash("sha256").update(value).digest("hex");
const sealed = (relativePath: string, content: Uint8Array, inode: string) => ({
  relativePath, bytes: content, sha256: sha(content), byteLength: content.byteLength,
  identity: { device: "1", inode, size: content.byteLength, modifiedNs: "3" },
});

async function fixture(): Promise<BundleAssemblyInput> {
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
  const png = new Uint8Array(await readFile(join(process.cwd(), "test/fixtures/take.apparent/frames/frame-01.png")));
  const frameFile = sealed("frames/frame-01.png", png, "inode-frame");
  const sheetBytes = new Uint8Array(await readFile(join(process.cwd(), "test/fixtures/take.apparent/contact-sheet.png")));
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
  const manifest = { schema: "apparent.capture-materials/1", key: { ...identity, revision: sha(payloadBytes) }, root,
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

test("private staging prepares SDK-validated plan without touching destination or dispatching", async () => {
  const input = await fixture();
  const assembly = assembleCanonicalBundle(input);
  const root = await mkdtemp(join(tmpdir(), "record-stage-fixture-"));
  const destinationPath = join(root, "final.apparent");
  try {
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
    await assert.rejects(readFile(destinationPath));
    const replay = await prepareRecordPublication({
      ownerToken: "c".repeat(64), publicationId: "b".repeat(32), stagingRoot: join(root, "private"), destinationPath,
      materialsManifest: input.manifest, materialsPayloadBytes: input.payloadBytes, assembly,
      artifactBytes: Object.fromEntries(input.artifacts.map(artifact => [artifact.relativePath, artifact.bytes])),
    });
    assert.equal(replay.input.planFile.planSha256, prepared.input.planFile.planSha256, "an exact replay resumes the same staged plan");
    const foreign = join(root, "private", "b".repeat(32), "foreign.txt");
    await writeFile(foreign, "preserve me", { mode: 0o600 });
    await assert.rejects(prepareRecordPublication({
      ownerToken: "c".repeat(64), publicationId: "b".repeat(32), stagingRoot: join(root, "private"), destinationPath,
      materialsManifest: input.manifest, materialsPayloadBytes: input.payloadBytes, assembly,
      artifactBytes: Object.fromEntries(input.artifacts.map(artifact => [artifact.relativePath, artifact.bytes])),
    }), /unrelated state/u);
    assert.equal(await readFile(foreign, "utf8"), "preserve me", "conflicting staged state is never cleaned up implicitly");
    const summary = new TextDecoder().decode(assembly.summaryMd);
    assert.match(summary, /hello there/u);
    assert.match(new TextDecoder().decode(assembly.apparentJson), /contact-sheet\.png/u);
  } finally { await rm(root, { recursive: true, force: true }); }
});

test("refuses same-named video bytes whose seal differs from the SDK materials reference", async () => {
  const input = await fixture();
  const video = input.artifacts.find(artifact => artifact.role === "video")!;
  const differentBytes = new TextEncoder().encode("different synthetic video bytes");
  Object.assign(video, sealed(video.relativePath, differentBytes, "inode-video-other"), { materials: video.materials });
  assert.throws(() => assembleCanonicalBundle(input, {}), /video|materials|sealed/i);
});
