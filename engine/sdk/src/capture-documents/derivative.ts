import { absolutePath, assertProcessingAllowed, contained, exact, fail, id, optionalExact, parseDetached, record, safeInt, sameMaterials, validateMaterialsKey, validateSealedFile } from "./common.ts";
import type { DerivativeEnvelope, DerivativePayload, MaterialsDocument, SealedFile } from "./types.ts";
import { isDeepStrictEqual } from "node:util";

export function validateDerivativeEnvelope(value: unknown, receiptBytes: Uint8Array, materials: MaterialsDocument, requestedBundleTimes: readonly number[]): asserts value is DerivativeEnvelope {
  assertProcessingAllowed(materials);
  const e = record(value, "derivative envelope"); exact(e, ["schema", "materials", "jobId", "receiptId", "root", "receiptFile", "payload"], "derivative envelope");
  if (e.schema !== "apparent.media-derivative/1") fail("unsupported derivative envelope");
  validateMaterialsKey(e.materials); sameMaterials(e.materials as DerivativeEnvelope["materials"], materials.manifest.key, "derivative"); id(e.jobId, "derivative jobId"); id(e.receiptId, "derivative receiptId"); absolutePath(e.root, "derivative root"); validateSealedFile(e.receiptFile, "derivative receiptFile");
  if ((e.receiptFile as SealedFile).identity.size !== receiptBytes.byteLength) fail("derivative receipt byte length differs from sealed identity");
  const parsed = parseDetached<unknown>(receiptBytes, (e.receiptFile as SealedFile).sha256, "derivative receipt payload");
  validatePayload(parsed, materials, e as unknown as DerivativeEnvelope, requestedBundleTimes);
  if (!isDeepStrictEqual(e.payload, parsed)) fail("derivative envelope payload differs from detached receipt bytes");
}

function validatePayload(value: unknown, materials: MaterialsDocument, envelope: DerivativeEnvelope, requests: readonly number[]): asserts value is DerivativePayload {
  const p = record(value, "derivative payload"); exact(p, ["schema", "materials", "jobId", "receiptId", "root", "frames"], "derivative payload");
  if (p.schema !== "apparent.media-derivative-payload/1") fail("unsupported derivative payload schema");
  validateMaterialsKey(p.materials); sameMaterials(p.materials as DerivativePayload["materials"], materials.manifest.key, "derivative payload");
  if (p.jobId !== envelope.jobId || p.receiptId !== envelope.receiptId || p.root !== envelope.root) fail("derivative envelope/receipt identity mismatch");
  validateDerivativeFrameSlots(p.frames, materials, envelope.root, envelope.receiptFile.relativePath, requests);
}

/** Shared clock/slot rules; this validates metadata, not native provenance. */
export function validateDerivativeFrameSlots(frames: unknown, materials: MaterialsDocument, root: string, receiptPath: string, requests: readonly number[]): void {
  if (!Array.isArray(frames) || frames.length !== requests.length || frames.length < 1 || frames.length > 48) fail("derivative frame slots are missing, extra or exceed limits");
  const seen = new Set<number>();
  const paths = new Set<string>();
  let pixels = 0;
  frames.forEach((frame, i) => {
    const f = record(frame, `frames[${i}]`); optionalExact(f, ["requestIndex", "requestedBundleMs", "effectiveVideoMs", "actualVideoMs", "clampedToStart", "clampedToEnd", "width", "height", "file"], ["requestIndex", "requestedBundleMs", "effectiveVideoMs", "clampedToStart", "clampedToEnd", "width", "height", "file"], `frames[${i}]`);
    safeInt(f.requestIndex, "requestIndex", 1); if (f.requestIndex !== i + 1 || seen.has(f.requestIndex as number)) fail("frame slots must be ordered and one-based"); seen.add(f.requestIndex as number);
    safeInt(f.requestedBundleMs, "requested frame time"); if (f.requestedBundleMs !== requests[i]) fail("derivative frame request does not match requested slot");
    safeInt(f.effectiveVideoMs, "effectiveVideoMs"); const expected = Math.max(0, (f.requestedBundleMs as number) - materials.payload.clock.videoOffsetMs); if (f.effectiveVideoMs !== expected) fail("effective seek does not match bundle clock mapping");
    if (typeof f.clampedToStart !== "boolean" || typeof f.clampedToEnd !== "boolean" || (f.clampedToStart && f.clampedToEnd)) fail("invalid frame clamp mapping");
    const startsBeforeVideo = (f.requestedBundleMs as number) < materials.payload.clock.videoOffsetMs;
    if (f.clampedToStart !== startsBeforeVideo) fail("start clamp does not match bundle/video offset mapping");
    if (f.actualVideoMs !== undefined) safeInt(f.actualVideoMs, "actualVideoMs");
    // Decoder PTS may include an edit-list/B-frame offset. It is independent of
    // the requested seek even at either clamp; never normalize it to zero or
    // infer a numeric ordering between these distinct clocks.
    safeInt(f.width, "frame width", 1); safeInt(f.height, "frame height", 1); validateSealedFile(f.file, `frames[${i}].file`); contained(root, (f.file as SealedFile).relativePath, "derivative frame");
    const path = (f.file as SealedFile).relativePath;
    if (paths.has(path) || path === receiptPath) fail("derivative frame path aliases another slot or receipt");
    paths.add(path);
    pixels += (f.width as number) * (f.height as number);
    if ((f.width as number) > 4096 || (f.height as number) > 4096 || pixels > 64_000_000) fail("derivative image inventory exceeds limits");
  });
}
