import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { validateDerivativeEnvelope } from "../src/capture-documents/derivative.ts";
import { validateInferenceEnvelope } from "../src/capture-documents/inference.ts";
import { validateMaterialsDocument, validateStrokeFacts } from "../src/capture-documents/materials.ts";
import { validatePublicationPlan } from "../src/capture-documents/publication.ts";
import { validateContextFacts, validateFactFile } from "../src/capture-documents/facts.ts";

const id = (n: string) => n.repeat(32);
const hash = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
const bytes = (value: unknown) => Buffer.from(JSON.stringify(value), "utf8");
const sealed = (path: string, content = path) => ({ relativePath: path, identity: { device: "1", inode: "2", size: content.length, modifiedNs: "3" }, sha256: hash(Buffer.from(content)) });

function materialFixture(overrides: Record<string, unknown> = {}) {
  const payload = {
    schema: "apparent.capture-materials-payload/1",
    identity: { captureId: id("a"), materialsId: id("b") }, bundleId: "bundle-1", root: "/private/capture-materials", leaseId: id("c"),
    clock: { epoch: "capture-admission-monotonic", units: "integer-milliseconds", durationMs: 5000, videoOffsetMs: 100, audioOffsetMs: 25 },
    confirmedSource: { id: "display-1", generation: id("d"), confirmedAtMs: 0, kind: "display", displayId: 1, rectangle: { x: 0, y: 0, width: 1280, height: 720, scale: 1 }, confirmation: { kind: "explicit-display", id: id("e") } }, invocation: { kind: "explicit", invocationId: id("f"), observedAtMs: 0 }, files: [sealed("video.mp4"), sealed("context.json")],
    video: { file: "video.mp4", width: 1280, height: 720, durationMs: 4900 },
    facts: { context: { file: "context.json", schema: "apparent.capture-context/1" } },
    collection: { context: "collected", interactions: "unavailable", candidates: "unavailable", transcript: "unavailable", acceptedStrokes: "unavailable" },
    warnings: [], safety: { nativeRemoteDelivery: "forbidden", safeForProcessing: true }, ...overrides,
  };
  const payloadBytes = bytes(payload);
  const manifest = { schema: "apparent.capture-materials/1", key: { ...payload.identity, revision: hash(payloadBytes) }, root: payload.root, leaseId: payload.leaseId, payloadFile: sealed("materials.json", payloadBytes.toString()) };
  // payloadFile identifies bytes, and its identity metadata is deliberately not a filesystem claim here.
  manifest.payloadFile.identity.size = payloadBytes.byteLength;
  return { payload, payloadBytes, manifest };
}
const material = () => { const f = materialFixture(); return { ...f, document: validateMaterialsDocument(f.manifest, f.payloadBytes) }; };
const receiptFile = (path: string, bytesValue: Uint8Array) => ({ relativePath: path, identity: { device: "1", inode: "2", size: bytesValue.byteLength, modifiedNs: "3" }, sha256: hash(bytesValue) });
const assembledFile = (path: string, digest: string, size: number) => ({ relativePath: path, identity: { device: "1", inode: "2", size, modifiedNs: "3" }, sha256: digest });

test("materials parse exact UTF-8 bytes and bind detached hash, identity, root and lease", () => {
  const f = materialFixture(); const doc = validateMaterialsDocument(f.manifest, f.payloadBytes);
  assert.equal(doc.payload.identity.captureId, id("a"));
  assert.throws(() => validateMaterialsDocument(f.manifest, bytes({ ...f.payload, identity: { ...f.payload.identity, materialsId: id("d") } })), /SHA256/);
  assert.throws(() => validateMaterialsDocument({ ...f.manifest, key: { ...f.manifest.key, revision: "0".repeat(64) } }, f.payloadBytes), /revision/);
  assert.throws(() => validateMaterialsDocument({ ...f.manifest, root: "/different/root" }, f.payloadBytes), /root mismatch/);
  assert.throws(() => validateMaterialsDocument({ ...f.manifest, leaseId: id("d") }, f.payloadBytes), /lease mismatch/);
  assert.throws(() => validateMaterialsDocument({ ...f.manifest, key: { ...f.manifest.key, captureId: id("d") } }, f.payloadBytes), /identity mismatch/);
  const selfHash = { ...f.payload, revision: "0".repeat(64) };
  const selfBytes = bytes(selfHash); const selfManifest = { ...f.manifest, key: { ...f.manifest.key, revision: hash(selfBytes) }, payloadFile: receiptFile("materials.json", selfBytes) };
  assert.throws(() => validateMaterialsDocument(selfManifest, selfBytes), /invalid fields/);
  void doc;
});

test("context facts bind source and invocation to the exact materials identity with explicit observations", () => {
  const f=material();
  const context={schema:"apparent.capture-context/1",identity:f.document.payload.identity,source:f.document.payload.confirmedSource,invocation:f.document.payload.invocation,context:{state:"collected",value:{values:{},changes:[]}},session:{state:"unavailable",reason:"provider does not expose session context"},windows:{state:"collected",value:[]},cursor:{state:"unavailable",reason:"cursor sampling unavailable"}};
  assert.doesNotThrow(()=>validateContextFacts(context,f.document));
  assert.throws(()=>validateContextFacts({...context,identity:{...context.identity,materialsId:id("9")}},f.document),/not bound/);
  assert.throws(()=>validateContextFacts({...context,source:{...context.source,id:"other-display"}},f.document),/do not match sealed/);
  assert.throws(()=>validateContextFacts({...context,session:{state:"collected",value:{id:"session",directory:"relative/path"}}},f.document),/absolute/);
  const raw = bytes(context); const file = receiptFile("context.json", raw);
  const fixture = materialFixture({ files: [f.payload.files[0], file] });
  const document = validateMaterialsDocument(fixture.manifest, fixture.payloadBytes);
  const reordered = { ...file, identity: { modifiedNs: file.identity.modifiedNs, size: file.identity.size, inode: file.identity.inode, device: file.identity.device } };
  assert.doesNotThrow(() => validateFactFile(reordered, raw, document, "context"));
});

test("materials reject invalid UTF-8, oversize bytes, traversal, duplicate paths and bad collection references", () => {
  const f = materialFixture();
  assert.throws(() => validateMaterialsDocument(f.manifest, Buffer.from([0xff])), /byte length/);
  const invalid = Buffer.from([0xff]); const badUtfManifest = { ...f.manifest, key: { ...f.manifest.key, revision: hash(invalid) }, payloadFile: receiptFile("materials.json", invalid) };
  assert.throws(() => validateMaterialsDocument(badUtfManifest, invalid), /UTF-8/);
  const oversized = Buffer.alloc(8 * 1024 * 1024 + 1, 32); const overManifest = { ...f.manifest, key: { ...f.manifest.key, revision: hash(oversized) }, payloadFile: receiptFile("materials.json", oversized) };
  assert.throws(() => validateMaterialsDocument(overManifest, oversized), /bounds/);
  for (const invalidPayload of [
    { ...f.payload, files: [sealed("../outside")] },
    { ...f.payload, files: [sealed("same"), sealed("same")] },
    { ...f.payload, facts: {}, collection: { ...f.payload.collection, context: "collected" } },
    { ...f.payload, facts: { context: { file: "missing.json", schema: "apparent.capture-context/1" } } },
    { ...f.payload, facts: { context: { file: "context.json", schema: "wrong" } } },
  ]) {
    const b = bytes(invalidPayload); const m = { ...f.manifest, key: { ...f.manifest.key, revision: hash(b) }, payloadFile: receiptFile("materials.json", b) };
    assert.throws(() => validateMaterialsDocument(m, b));
  }
});

test("derivative receipts bind exact material/job identity and preserve ordered frame slots and clock mapping", () => {
  const f0 = materialFixture({ clock: { ...materialFixture().payload.clock, videoOffsetMs: 0 } });
  const f = { ...f0, document: validateMaterialsDocument(f0.manifest, f0.payloadBytes) };
  const payload = { schema: "apparent.media-derivative-payload/1", materials: f.manifest.key, jobId: id("d"), receiptId: id("e"), root: "/private/derivative", frames: [
    { requestIndex: 1, requestedBundleMs: 5000, effectiveVideoMs: 5000, actualVideoMs: 1200, clampedToStart: false, clampedToEnd: true, width: 100, height: 50, file: sealed("frame-1.png") },
    { requestIndex: 2, requestedBundleMs: 5000, effectiveVideoMs: 5000, actualVideoMs: 1200, clampedToStart: false, clampedToEnd: true, width: 100, height: 50, file: sealed("frame-2.png") },
  ] };
  const receiptBytes = bytes(payload); const envelope = { schema: "apparent.media-derivative/1", materials: f.manifest.key, jobId: payload.jobId, receiptId: payload.receiptId, root: payload.root, receiptFile: receiptFile("receipt.json", receiptBytes), payload };
  assert.doesNotThrow(() => validateDerivativeEnvelope(envelope, receiptBytes, f.document, [5000, 5000]));
  assert.equal(payload.frames[0].effectiveVideoMs, 5000);
  assert.equal(payload.frames[0].actualVideoMs, 1200);
  assert.throws(() => validateDerivativeEnvelope({ ...envelope, materials: { ...envelope.materials, materialsId: id("f") } }, receiptBytes, f.document, [5000, 5000]), /identity mismatch/);
  assert.throws(() => validateDerivativeEnvelope({ ...envelope, payload: { ...payload, frames: [payload.frames[0]] } }, receiptBytes, f.document, [5000, 5000]), /detached receipt/);
  const missing = { ...payload, frames: [payload.frames[0]] }; const missingBytes = bytes(missing);
  assert.throws(() => validateDerivativeEnvelope({ ...envelope, receiptFile: receiptFile("receipt.json", missingBytes), payload: missing }, missingBytes, f.document, [5000, 5000]), /slots are missing/);
  const shifted = { ...payload, frames: payload.frames.map((x, i) => ({ ...x, requestIndex: i + 2 })) }; const shiftedBytes = bytes(shifted);
  assert.throws(() => validateDerivativeEnvelope({ ...envelope, receiptFile: receiptFile("receipt.json", shiftedBytes), payload: shifted }, shiftedBytes, f.document, [5000, 5000]), /ordered and one-based/);
  const collapsed = { ...payload, frames: payload.frames.map((x) => ({ ...x, effectiveVideoMs: x.actualVideoMs })) }; const collapsedBytes = bytes(collapsed);
  assert.throws(() => validateDerivativeEnvelope({ ...envelope, receiptFile: receiptFile("receipt.json", collapsedBytes), payload: collapsed }, collapsedBytes, f.document, [5000, 5000]), /effective seek/);
});

test("inference keeps transcript bundle-clock times and unavailable outcomes cannot contain transcript data", () => {
  const f = material();
  const base = { schema: "apparent.transcript-result-payload/1", materials: f.manifest.key, jobId: id("d"), receiptId: id("e"), root: "/private/inference", clock: "bundle", state: "unavailable", warning: { code: "transcript_unavailable", retryable: true } };
  const receiptBytes = bytes(base); const envelope = { schema: "apparent.transcript-result/1", materials: f.manifest.key, jobId: base.jobId, receiptId: base.receiptId, root: base.root, receiptFile: receiptFile("receipt.json", receiptBytes), payload: base };
  assert.doesNotThrow(() => validateInferenceEnvelope(envelope, receiptBytes, f.document));
  const bad = { ...base, transcript: { provider: "synthetic", model: "synthetic", text: "not allowed", segments: [] } }; const badBytes = bytes(bad);
  assert.throws(() => validateInferenceEnvelope({ ...envelope, receiptFile: receiptFile("receipt.json", badBytes), payload: bad }, badBytes, f.document), /forbids transcript/);
  const invalidSegments = { ...base, state: "complete", warning: undefined, transcript: { provider: "synthetic", model: "synthetic", text: "x", segments: [{ startMs: 30, endMs: 10, text: "x" }] } }; delete (invalidSegments as { warning?: unknown }).warning;
  const segmentBytes = bytes(invalidSegments);
  assert.throws(() => validateInferenceEnvelope({ ...envelope, receiptFile: receiptFile("receipt.json", segmentBytes), payload: invalidSegments }, segmentBytes, f.document), /ends before/);
  const shifted = { ...base, state: "complete", warning: undefined, transcript: { provider: "synthetic", model: "synthetic", text: "x", segments: [{ startMs: 5010, endMs: 5020, text: "x" }] } }; delete (shifted as { warning?: unknown }).warning;
  const shiftedBytes = bytes(shifted);
  assert.throws(() => validateInferenceEnvelope({ ...envelope, receiptFile: receiptFile("receipt.json", shiftedBytes), payload: shifted }, shiftedBytes, f.document), /exceeds capture duration/);
});

test("stroke facts bind material identity, declared transforms and bounded ordered times/coordinates", () => {
  const f = material(); const strokes = { schema: "apparent.capture-strokes/1", identity: f.payload.identity, coordinates: "target-normalized-10000", surfaces: [{ surfaceId: "screen-1", transformId: "transform-1", width: 1280, height: 720, target: "capture-rectangle" }], strokes: [{ id: id("f"), surface: "screen", surfaceId: "screen-1", transformId: "transform-1", startMs: 10, endMs: 20, acceptedAtMs: 25, points: [{ timeMs: 10, x: 0, y: 10000 }, { timeMs: 20, x: 10000, y: 1 }] }] };
  const check = (value: unknown) => { const factBytes = bytes(value); const factFile = { ...sealed("strokes.json"), identity: { device: "1", inode: "9", size: factBytes.byteLength, modifiedNs: "3" }, sha256: hash(factBytes) }; const fixture = materialFixture({ files: [...f.payload.files, factFile], facts: { ...f.payload.facts, acceptedStrokes: { file: "strokes.json", schema: "apparent.capture-strokes/1" } }, collection: { ...f.payload.collection, acceptedStrokes: "collected" } }); const doc = validateMaterialsDocument(fixture.manifest, fixture.payloadBytes); return validateStrokeFacts(factFile, factBytes, doc); };
  assert.doesNotThrow(() => check(strokes));
  assert.throws(() => check({ ...strokes, strokes: [{ ...strokes.strokes[0], surface: "whiteboard" }] }), /declared target/);
  assert.throws(() => check({ ...strokes, strokes: [{ ...strokes.strokes[0], acceptedAtMs: 19 }] }), /time order/);
  assert.throws(() => check({ ...strokes, strokes: [{ ...strokes.strokes[0], points: [{ timeMs: 26, x: 2, y: 2 }] }] }), /outside sorted/);
  assert.throws(() => check({ ...strokes, strokes: [{ ...strokes.strokes[0], points: [{ timeMs: 12, x: 10001, y: 2 }] }] }), /normalized range/);
});

test("publication binds exact plan bytes and enforces absent-only destination and bundle profile inventory", () => {
  const f = material();
  const derivativePayload = { schema: "apparent.media-derivative-payload/1", materials: f.manifest.key, jobId: id("a"), receiptId: id("b"), root: "/private/derivative", frames: [{ requestIndex: 1, requestedBundleMs: 10, effectiveVideoMs: 0, actualVideoMs: 200, clampedToStart: true, clampedToEnd: false, width: 10, height: 5, file: { ...sealed("frame.png"), sha256: "3".repeat(64), identity: { device: "1", inode: "2", size: 50, modifiedNs: "3" } } }] };
  const derivativeBytes = bytes(derivativePayload);
  const derivativeReceipt = { schema: "apparent.media-derivative/1", materials: f.manifest.key, jobId: derivativePayload.jobId, receiptId: derivativePayload.receiptId, root: derivativePayload.root, receiptFile: receiptFile("derivative-receipt.json", derivativeBytes), payload: derivativePayload };
  const proofs = [{ envelope: derivativeReceipt, bytes: derivativeBytes, requestedBundleMs: [10] }];
  const plan = { schema: "apparent.bundle-publication/1", publicationId: id("d"), materials: f.manifest.key, bundleId: f.payload.bundleId, stagingRoot: "/private/staging/op-1", destination: { path: "/private/final/bundle.apparent", mode: "absent" }, artifactVersion: 1, entries: [
    { relativePath: "apparent.json", sha256: "1".repeat(64), byteLength: 100, role: "manifest", input: { kind: "assembled", file: assembledFile("apparent.json", "1".repeat(64), 100) } },
    { relativePath: "summary.md", sha256: "2".repeat(64), byteLength: 10, role: "summary", input: { kind: "assembled", file: assembledFile("summary.md", "2".repeat(64), 10) } },
    { relativePath: "frames/1.png", sha256: "3".repeat(64), byteLength: 50, role: "frame", input: { kind: "derivative", receipt: derivativeReceipt, fileIndex: 1 } },
    { relativePath: "contact-sheet.png", sha256: "4".repeat(64), byteLength: 50, role: "contact-sheet", input: { kind: "assembled", file: assembledFile("contact-sheet.png", "4".repeat(64), 50) } },
  ], manifestPath: "apparent.json", summaryPath: "summary.md", profile: "apparent-bundle/1" };
  const planBytes = bytes(plan); const envelope = { stagingRoot: plan.stagingRoot, planFile: receiptFile("plan.json", planBytes), planSha256: hash(planBytes) };
  assert.equal(validatePublicationPlan(envelope, planBytes, f.document, proofs).profile, "apparent-bundle/1");
  assert.throws(() => validatePublicationPlan(envelope, planBytes, f.document), /detached receipt bytes/);
  assert.throws(() => validatePublicationPlan(envelope, planBytes, f.document, [{ ...proofs[0], bytes: Buffer.from("tampered") }]), /byte length|SHA256/);
  const otherBundle = { ...plan, bundleId: "another-bundle" }; const otherBundleBytes = bytes(otherBundle);
  assert.throws(() => validatePublicationPlan({ ...envelope, planFile: receiptFile("plan.json", otherBundleBytes), planSha256: hash(otherBundleBytes) }, otherBundleBytes, f.document, proofs), /bundle identity/);
  const unsafeFixture = materialFixture({ safety: { nativeRemoteDelivery: "forbidden", safeForProcessing: false } });
  const unsafeDocument = validateMaterialsDocument(unsafeFixture.manifest, unsafeFixture.payloadBytes);
  assert.throws(() => validatePublicationPlan(envelope, planBytes, unsafeDocument), /not safe for processing/);
  assert.throws(() => validatePublicationPlan({ ...envelope, stagingRoot: "/elsewhere" }, planBytes, f.document, proofs), /stagingRoot mismatch/);
  assert.throws(() => validatePublicationPlan({ ...envelope, planSha256: "f".repeat(64) }, planBytes, f.document), /planSha256/);
  const occupiedMode = { ...plan, destination: { ...plan.destination, mode: "replace" } }; const occupiedBytes = bytes(occupiedMode);
  assert.throws(() => validatePublicationPlan({ ...envelope, planFile: receiptFile("plan.json", occupiedBytes), planSha256: hash(occupiedBytes) }, occupiedBytes, f.document), /absent-only/);
  const duplicate = { ...plan, entries: [...plan.entries, { ...plan.entries[2] }] }; const duplicateBytes = bytes(duplicate);
  assert.throws(() => validatePublicationPlan({ ...envelope, planFile: receiptFile("plan.json", duplicateBytes), planSha256: hash(duplicateBytes) }, duplicateBytes, f.document, proofs), /duplicate/);
});

test("derivatives preserve real decoder PTS independently at both clamps and reject path aliases", () => {
  const fixture = materialFixture({ clock: { epoch: "capture-admission-monotonic", units: "integer-milliseconds", durationMs: 2250, videoOffsetMs: 250 }, video: { file: "video.mp4", width: 640, height: 360, durationMs: 2000 } });
  const document = validateMaterialsDocument(fixture.manifest, fixture.payloadBytes);
  // Synthetic receipt carrying the clock values observed in the native H.264
  // conformance case. This validates protocol data, not media or native provenance.
  const requests = [0, 5000, 2251, 5000];
  const payload = { schema: "apparent.media-derivative-payload/1", materials: fixture.manifest.key, jobId: id("d"), receiptId: id("e"), root: "/private/derivative", frames: requests.map((time, index) => ({ requestIndex: index + 1, requestedBundleMs: time, effectiveVideoMs: Math.max(0, time - 250), actualVideoMs: index === 0 ? 200 : 2199, clampedToStart: index === 0, clampedToEnd: index !== 0, width: 320, height: 180, file: sealed(`frame-${index + 1}.png`) })) };
  const check = (value: typeof payload) => { const raw = bytes(value); const envelope = { schema: "apparent.media-derivative/1", materials: fixture.manifest.key, jobId: value.jobId, receiptId: value.receiptId, root: value.root, receiptFile: receiptFile("receipt.json", raw), payload: value }; validateDerivativeEnvelope(envelope, raw, document, requests); };
  assert.doesNotThrow(() => check(payload));
  assert.throws(() => check({ ...payload, frames: payload.frames.map((f) => ({ ...f, file: sealed("same.png") })) }), /aliases/);
  assert.throws(() => check({ ...payload, frames: payload.frames.map((f, i) => i === 0 ? { ...f, file: sealed("receipt.json") } : f) }), /aliases/);
  assert.throws(() => check({ ...payload, frames: payload.frames.map((f) => ({ ...f, width: 4097 })) }), /limits/);
});

test("stroke fact envelopes bind the entire sealed identity and exact byte length", () => {
  const base = materialFixture();
  const value = { schema: "apparent.capture-strokes/1", identity: base.payload.identity, coordinates: "target-normalized-10000", surfaces: [], strokes: [] };
  const raw = bytes(value); const file = receiptFile("strokes.json", raw);
  const fixture = materialFixture({ files: [...base.payload.files, file], facts: { ...base.payload.facts, acceptedStrokes: { file: "strokes.json", schema: "apparent.capture-strokes/1" } }, collection: { ...base.payload.collection, acceptedStrokes: "collected" } });
  const document = validateMaterialsDocument(fixture.manifest, fixture.payloadBytes);
  assert.doesNotThrow(() => validateStrokeFacts(file, raw, document));
  assert.throws(() => validateStrokeFacts({ ...file, identity: { ...file.identity, inode: "999" } }, raw, document), /sealed accepted-strokes reference/);
  const wrongSize = { ...file, identity: { ...file.identity, size: raw.byteLength + 1 } };
  const badFixture = materialFixture({ ...fixture.payload, files: [...base.payload.files, wrongSize] });
  const badDocument = validateMaterialsDocument(badFixture.manifest, badFixture.payloadBytes);
  assert.throws(() => validateStrokeFacts(wrongSize, raw, badDocument), /sealed size/);
});
