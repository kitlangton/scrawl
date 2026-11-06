import { createHash } from "node:crypto";
import { absolutePath, assertProcessingAllowed, exact, fail, id, record, safeInt, sameMaterials, validateMaterialsKey } from "../capture-documents/common.ts";
import type { MaterialsDocument } from "../capture-documents/types.ts";
import type { ContactSheetLayout, Media2JobRequest, Media2PreparedRequest } from "./types.ts";

export const MEDIA2_MAX_PIXELS = 64_000_000;
export function validateContactSheetLayout(value: unknown, count: number): asserts value is ContactSheetLayout {
  const l = record(value, "sheet layout");
  exact(l, ["schema", "width", "height", "backgroundRgb", "fit", "sampling", "clipping", "tiles"], "sheet layout");
  if (l.schema !== "apparent.contact-sheet-layout/1" || l.fit !== "stretch" || l.sampling !== "legacy-bilinear" || l.clipping !== "reject") fail("unsupported sheet pixel execution");
  safeInt(l.width, "sheet width", 1); safeInt(l.height, "sheet height", 1);
  if (l.width > 4096 || l.height > 4096) fail("sheet edge exceeds limit");
  if (!Array.isArray(l.backgroundRgb) || l.backgroundRgb.length !== 3 || l.backgroundRgb.some(v => !Number.isInteger(v) || v < 0 || v > 255)) fail("invalid sheet background RGB");
  if (!Array.isArray(l.tiles) || l.tiles.length < 1 || l.tiles.length > 48) fail("sheet tile count exceeds limit");
  const rects: Array<{ x: number; y: number; width: number; height: number }> = [];
  for (const tile of l.tiles) {
    const t = record(tile, "sheet tile"); exact(t, ["requestIndex", "x", "y", "width", "height"], "sheet tile");
    safeInt(t.requestIndex, "tile requestIndex", 1); if (t.requestIndex > count) fail("sheet references missing frame slot");
    safeInt(t.x, "tile x"); safeInt(t.y, "tile y"); safeInt(t.width, "tile width", 1); safeInt(t.height, "tile height", 1);
    if (t.x + t.width > l.width || t.y + t.height > l.height) fail("sheet tile clips outside canvas");
    const r = t as unknown as { x: number; y: number; width: number; height: number };
    if (rects.some(p => r.x < p.x + p.width && p.x < r.x + r.width && r.y < p.y + p.height && p.y < r.y + r.height)) fail("sheet tiles overlap");
    rects.push(r);
  }
}
/** Fixed field order, decimal integers, no whitespace, UTF-8 JSON.stringify.
 * No arbitrary object-key canonicalization or floating point is involved. */
export function encodeContactSheetLayout(l: ContactSheetLayout): Uint8Array {
  validateContactSheetLayout(l, 48);
  return Buffer.from(JSON.stringify({ schema: l.schema, width: l.width, height: l.height, backgroundRgb: l.backgroundRgb, fit: l.fit, sampling: l.sampling, clipping: l.clipping, tiles: l.tiles.map(t => ({ requestIndex: t.requestIndex, x: t.x, y: t.y, width: t.width, height: t.height })) }), "utf8");
}
export function prepareMedia2Request(value: unknown, materials: MaterialsDocument): Media2PreparedRequest {
  assertProcessingAllowed(materials);
  const r = record(value, "media2 request");
  exact(r, ["schema", "materials", "leaseId", "jobId", "outputDirectory", "requestedBundleMs", "limits", "contactSheet"], "media2 request");
  if (r.schema !== "apparent.retained-frame-request/1") fail("unsupported retained frame request");
  validateMaterialsKey(r.materials); sameMaterials(r.materials, materials.manifest.key, "media2 request");
  id(r.leaseId, "media2 leaseId"); if (r.leaseId !== materials.manifest.leaseId) fail("media2 lease mismatch");
  id(r.jobId, "media2 jobId"); absolutePath(r.outputDirectory, "media2 outputDirectory");
  const within = (path: string, root: string) => path === root || path.startsWith(root.endsWith("/") ? root : `${root}/`);
  if (within(r.outputDirectory, materials.manifest.root) || within(materials.manifest.root, r.outputDirectory)) fail("media2 output overlaps retained materials");
  const limits = record(r.limits, "media2 limits"); exact(limits, ["maxFrames", "maxEdge", "maxTotalPixels", "deadlineMs"], "media2 limits");
  for (const [key, max] of [["maxFrames", 48], ["maxEdge", 4096], ["maxTotalPixels", MEDIA2_MAX_PIXELS], ["deadlineMs", 60_000]] as const) { safeInt(limits[key], key, 1); if (limits[key] > max) fail(`${key} exceeds media2 limit`); }
  if (!Array.isArray(r.requestedBundleMs) || !r.requestedBundleMs.length || r.requestedBundleMs.length > (limits.maxFrames as number)) fail("media2 selected slot count exceeds limit");
  for (const time of r.requestedBundleMs) { safeInt(time, "requestedBundleMs"); if (time > materials.payload.clock.durationMs) fail("request is outside capture clock"); }
  if (r.contactSheet !== null) {
    validateContactSheetLayout(r.contactSheet, r.requestedBundleMs.length);
    if (r.contactSheet.width * r.contactSheet.height + r.requestedBundleMs.length > (limits.maxTotalPixels as number)) fail("frame and sheet pixel budget cannot fit");
  }
  const q = r as unknown as Media2JobRequest;
  const request: Media2JobRequest = { schema: q.schema, materials: { captureId: q.materials.captureId, materialsId: q.materials.materialsId, revision: q.materials.revision }, leaseId: q.leaseId, jobId: q.jobId, outputDirectory: q.outputDirectory, requestedBundleMs: [...q.requestedBundleMs], limits: { maxFrames: q.limits.maxFrames, maxEdge: q.limits.maxEdge, maxTotalPixels: q.limits.maxTotalPixels, deadlineMs: q.limits.deadlineMs }, contactSheet: q.contactSheet === null ? null : JSON.parse(Buffer.from(encodeContactSheetLayout(q.contactSheet)).toString("utf8")) as ContactSheetLayout };
  const bytes = Buffer.from(JSON.stringify(request), "utf8");
  return { request, bytes, sha256: createHash("sha256").update(bytes).digest("hex") };
}
