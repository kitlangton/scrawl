import { createHash } from "node:crypto";
import { validateMaterialsDocument } from "../../src/capture.ts";
import { encodeContactSheetLayout, prepareMedia2Request } from "../../src/media2/index.ts";
import type { Derivative2DocumentInput, Derivative2Envelope, Derivative2Payload, Media2JobRequest } from "../../src/media2/index.ts";
import type { MaterialsPayload } from "../../src/capture.ts";

export const id = (n: string) => n.repeat(32);
export const bytes = (v: unknown) => Buffer.from(JSON.stringify(v), "utf8");
export const hash = (v: Uint8Array) => createHash("sha256").update(v).digest("hex");
export const seal = (path: string, v: Uint8Array) => ({ relativePath: path, sha256: hash(v), identity: { device: "1", inode: String(path.length), size: v.byteLength, modifiedNs: "3" } });
export function fixture() {
  const payload: MaterialsPayload = {
    schema: "apparent.capture-materials-payload/1", identity: { captureId: id("a"), materialsId: id("b") }, bundleId: "record-opaque.apparent", root: "/private/materials", leaseId: id("c"),
    clock: { epoch: "capture-admission-monotonic", units: "integer-milliseconds", durationMs: 5000, videoOffsetMs: 100 },
    confirmedSource: { id: "display-1", generation: id("d"), confirmedAtMs: 0, kind: "display", displayId: 1, rectangle: { x: 0, y: 0, width: 1280, height: 720, scale: 1 }, confirmation: { kind: "explicit-display", id: id("e") } },
    invocation: { kind: "explicit", invocationId: id("f"), observedAtMs: 0 }, files: [seal("recording.mp4", bytes("video"))], video: { file: "recording.mp4", width: 1280, height: 720, durationMs: 4900 }, facts: {},
    collection: { context: "unavailable", interactions: "unavailable", candidates: "unavailable", transcript: "unavailable", acceptedStrokes: "unavailable" }, warnings: [], safety: { nativeRemoteDelivery: "forbidden", safeForProcessing: true },
  };
  const payloadBytes = bytes(payload);
  const materials = validateMaterialsDocument({ schema: "apparent.capture-materials/1", key: { ...payload.identity, revision: hash(payloadBytes) }, root: payload.root, leaseId: payload.leaseId, payloadFile: seal("materials.json", payloadBytes) }, payloadBytes);
  const request: Media2JobRequest = { schema: "apparent.retained-frame-request/1", materials: materials.manifest.key, leaseId: payload.leaseId, jobId: id("1"), outputDirectory: "/private/derivative", requestedBundleMs: [0, 900, 900, 5000], limits: { maxFrames: 4, maxEdge: 100, maxTotalPixels: 40_000, deadlineMs: 5000 }, contactSheet: { schema: "apparent.contact-sheet-layout/1", width: 200, height: 100, backgroundRgb: [1, 2, 3], fit: "stretch", sampling: "legacy-bilinear", clipping: "reject", tiles: [{ requestIndex: 2, x: 0, y: 25, width: 100, height: 50 }, { requestIndex: 3, x: 100, y: 25, width: 100, height: 50 }] } };
  const layoutBytes = encodeContactSheetLayout(request.contactSheet!);
  const png = bytes("synthetic metadata only, not PNG proof");
  const result: Derivative2Payload = { schema: "apparent.media-derivative-payload/2", materials: materials.manifest.key, leaseId: payload.leaseId, jobId: request.jobId, receiptId: id("2"), root: request.outputDirectory, requestSha256: prepareMedia2Request(request, materials).sha256,
    frames: request.requestedBundleMs.map((t, i) => ({ requestIndex: i + 1, requestedBundleMs: t, effectiveVideoMs: Math.max(0, t - 100), actualVideoMs: i === 0 ? 0 : i === 3 ? 4800 : 800, clampedToStart: i === 0, clampedToEnd: i === 3, width: 100, height: 50, encoding: "png", byteLength: png.length, file: seal(`frames/${i + 1}.png`, png) })),
    contactSheet: { encoding: "png", byteLength: png.length, file: seal("contact-sheet.png", png), width: 200, height: 100, layoutSha256: hash(layoutBytes), layout: request.contactSheet! } };
  function document(value = result): Derivative2DocumentInput {
    const content = bytes(value);
    const envelope: Derivative2Envelope = { schema: "apparent.media-derivative/2", materials: value.materials, leaseId: value.leaseId, jobId: value.jobId, receiptId: value.receiptId, root: value.root, requestSha256: value.requestSha256, receiptFile: seal("receipt.json", content), payload: value };
    return { envelope, bytes: content, request, layoutBytes };
  }
  return { materials, request, result, document };
}

