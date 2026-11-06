import test from "node:test";
import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { validateMaterialsDocument, preflightCapturePublication, CapturePublicationPreflightError } from "../src/capture.ts";
import { encodeContactSheetLayout, prepareMedia2Request, selectDerivative2Artifact, validateDerivative2Document } from "../src/media2/index.ts";
import type { Derivative2DocumentInput, Derivative2Envelope, Derivative2Payload, Media2JobRequest } from "../src/media2/index.ts";
import type { CapturePublishInput, MaterialsPayload, PublicationPlan } from "../src/capture.ts";

import { fixture, id, bytes, hash, seal } from "./media2/fixture.ts";

test("media2 binds canonical complete replay request and explicit layout bytes", () => {
  const f = fixture(); const original = prepareMedia2Request(f.request, f.materials);
  const reordered = { ...f.request, materials: { revision: f.request.materials.revision, materialsId: f.request.materials.materialsId, captureId: f.request.materials.captureId } };
  assert.equal(prepareMedia2Request(reordered, f.materials).sha256, original.sha256);
  for (const change of [ { limits: { ...f.request.limits, deadlineMs: 5001 } }, { requestedBundleMs: [900, 0, 900, 5000] }, { outputDirectory: "/private/other" }, { contactSheet: { ...f.request.contactSheet!, tiles: [...f.request.contactSheet!.tiles].reverse() } } ]) {
    assert.notEqual(prepareMedia2Request({ ...f.request, ...change }, f.materials).sha256, original.sha256);
  }
  assert.equal(Buffer.from(original.bytes).toString(), JSON.stringify(original.request));
  assert.throws(() => prepareMedia2Request({ ...f.request, leaseId: id("9") }, f.materials), /lease/);
  assert.throws(() => prepareMedia2Request({ ...f.request, requestedBundleMs: [5001] }, f.materials), /capture clock/);
  assert.throws(() => prepareMedia2Request({ ...f.request, outputDirectory: "/private/materials/frames" }, f.materials), /overlaps/);
});

test("media2 preserves duplicate ordered frame slots, offsets, decoder readback and separate sheet", () => {
  const f = fixture(); const receipt = validateDerivative2Document(f.document(), f.materials);
  assert.deepEqual(receipt.payload.frames.map(x => x.requestIndex), [1, 2, 3, 4]);
  assert.equal(receipt.payload.frames[3].effectiveVideoMs, 4900);
  assert.equal(receipt.payload.frames[3].actualVideoMs, 4800);
  assert.equal(receipt.payload.frames[3].clampedToEnd, true);
  assert.notEqual(selectDerivative2Artifact(receipt, { kind: "frame", requestIndex: 2 }).file.relativePath, selectDerivative2Artifact(receipt, { kind: "frame", requestIndex: 3 }).file.relativePath);
  assert.equal(selectDerivative2Artifact(receipt, { kind: "contact-sheet" }).file.relativePath, "contact-sheet.png");
  assert.throws(() => selectDerivative2Artifact(receipt, { kind: "frame", requestIndex: 0 }), /requestIndex/);
  assert.throws(() => selectDerivative2Artifact(receipt, { kind: "frame", requestIndex: 5 }), /missing/);
});

test("media2 rejects changed original request, detached layout and receipt aliases", () => {
  const f = fixture(); const doc = f.document();
  assert.throws(() => validateDerivative2Document({ ...doc, request: { ...doc.request, limits: { ...doc.request.limits, deadlineMs: 1 } } }, f.materials), /request binding/);
  assert.throws(() => validateDerivative2Document({ ...doc, layoutBytes: Buffer.from(" "+Buffer.from(doc.layoutBytes!).toString()) }, f.materials), /layout bytes/);
  for (const mutate of [
    (p: Derivative2Payload) => { p.frames[1].requestIndex = 1; },
    (p: Derivative2Payload) => { p.frames[1].file.relativePath = p.frames[0].file.relativePath; },
    (p: Derivative2Payload) => { p.frames[0].byteLength++; },
    (p: Derivative2Payload) => { p.frames[0].effectiveVideoMs++; },
    (p: Derivative2Payload) => { p.contactSheet!.file.relativePath = "receipt.json"; },
    (p: Derivative2Payload) => { p.root = "/private/other"; },
    (p: Derivative2Payload) => { p.contactSheet!.width++; },
  ]) { const value = structuredClone(f.result); mutate(value); assert.throws(() => validateDerivative2Document(f.document(value), f.materials)); }
});

test("media2 rejects clipping, overlap, unsupported fit and combined frame/sheet over-budget", () => {
  const f = fixture();
  for (const change of [ { x: 201 }, { requestIndex: 5 }, { x: 50 } ]) {
    const layout = structuredClone(f.request.contactSheet!); Object.assign(layout.tiles[1], change);
    assert.throws(() => prepareMedia2Request({ ...f.request, contactSheet: layout }, f.materials));
  }
  assert.throws(() => prepareMedia2Request({ ...f.request, contactSheet: { ...f.request.contactSheet, fit: "cover" } }, f.materials), /unsupported/);
  const doc = f.document(); doc.request.limits.maxTotalPixels--;
  const p = structuredClone(f.result); p.requestSha256 = prepareMedia2Request(doc.request, f.materials).sha256;
  assert.throws(() => validateDerivative2Document(f.document(p), f.materials), /combined/);
});

test("media2 sheet is optional and cannot appear or disappear without rebinding request", () => {
  const f = fixture(); const request = { ...f.request, contactSheet: null };
  const payload = { ...f.result, contactSheet: null, requestSha256: prepareMedia2Request(request, f.materials).sha256 };
  const content = bytes(payload);
  const doc = { ...f.document(), request, layoutBytes: null, bytes: content, envelope: { ...(f.document().envelope as Derivative2Envelope), requestSha256: payload.requestSha256, payload, receiptFile: seal("receipt.json", content) } };
  const result = validateDerivative2Document(doc, f.materials);
  assert.equal(result.payload.contactSheet, null);
  assert.throws(() => selectDerivative2Artifact(result, { kind: "contact-sheet" }), /missing/);
  assert.throws(() => validateDerivative2Document({ ...f.document(), layoutBytes: null }, f.materials), /layout/);
  assert.throws(() => validateDerivative2Document({ ...doc, request: f.request }, f.materials), /request binding/);
});

test("publication preflight binds derivative2 frame/sheet selectors to exact detached receipts", () => {
  const f = fixture(); const derivative = f.document(); const receipt = validateDerivative2Document(derivative, f.materials);
  const manifest = bytes("synthetic canonical manifest supplied by Record"); const summary = bytes("summary");
  const plan: PublicationPlan = { schema: "apparent.bundle-publication/1", publicationId: id("3"), materials: f.materials.manifest.key, bundleId: f.materials.payload.bundleId, stagingRoot: "/private/staging", destination: { path: "/private/final.apparent", mode: "absent" }, artifactVersion: 1, manifestPath: "apparent.json", summaryPath: "summary.md", profile: "apparent-bundle/1", entries: [
    { relativePath: "apparent.json", sha256: hash(manifest), byteLength: manifest.length, role: "manifest", input: { kind: "assembled", file: seal("apparent.json", manifest) } },
    { relativePath: "summary.md", sha256: hash(summary), byteLength: summary.length, role: "summary", input: { kind: "assembled", file: seal("summary.md", summary) } },
    { relativePath: "frames/selected.png", sha256: receipt.payload.frames[1].file.sha256, byteLength: receipt.payload.frames[1].byteLength, role: "frame", input: { kind: "derivative2", receipt, selector: { kind: "frame", requestIndex: 2 } } },
    { relativePath: "contact-sheet.png", sha256: receipt.payload.contactSheet!.file.sha256, byteLength: receipt.payload.contactSheet!.byteLength, role: "contact-sheet", input: { kind: "derivative2", receipt, selector: { kind: "contact-sheet" } } },
  ] };
  const input = (): CapturePublishInput => { const content = bytes(plan); return { ownerToken: "a".repeat(64), captureId: f.materials.manifest.key.captureId, leaseId: f.materials.manifest.leaseId, publicationId: plan.publicationId, materials: f.materials, planBytes: content, planFile: { stagingRoot: plan.stagingRoot, planFile: seal("publication-plan.json", content), planSha256: hash(content) }, derivatives2: [derivative] }; };
  const prepared = preflightCapturePublication(input()); assert.equal(prepared.expected.manifestSha256, hash(manifest));
  assert.throws(() => preflightCapturePublication({ ...input(), derivatives2: [] }), CapturePublicationPreflightError);
  plan.entries[2].role = "contact-sheet";
  assert.throws(() => preflightCapturePublication(input()), CapturePublicationPreflightError);
  plan.entries[2].role = "frame"; plan.entries[2].sha256 = "f".repeat(64);
  assert.throws(() => preflightCapturePublication(input()), CapturePublicationPreflightError);
});
