import { createHash } from "node:crypto";
import { selectDerivative2Artifact, validateDerivative2Document } from "../media2/receipt.ts";
import { readProfile } from "./png.ts";
import type { FrameVisualObservationInput, FrameVisualObservation, VisualObservationSource } from "./types.ts";
export type * from "./types.ts";

/** Offline consistency validation, never authentication of native issuance.
 * Use the original materials and authenticated Media2Client completion. This reader
 * does no filesystem access, RPC, decompression, selection, cleanup or lease release. */
export function readFrameVisualObservations(input: FrameVisualObservationInput): FrameVisualObservation {
  if (!Number.isSafeInteger(input.maxFileBytes) || input.maxFileBytes < 1 || !(input.pngBytes instanceof Uint8Array) || input.pngBytes.byteLength > input.maxFileBytes) throw new Error("frame observations: file byte budget exceeded");
  const receipt = validateDerivative2Document(input.document, input.materials);
  selectDerivative2Artifact(receipt, { kind: "frame", requestIndex: input.requestIndex });
  const frame = receipt.payload.frames[input.requestIndex - 1];
  if (input.pngBytes.byteLength !== frame.byteLength) throw new Error("frame observations: sealed byte length mismatch");
  // Copy before hashing/parsing: caller mutation cannot alter the validated snapshot.
  const png = Buffer.from(input.pngBytes);
  if (createHash("sha256").update(png).digest("hex") !== frame.file.sha256) throw new Error("frame observations: sealed SHA256 mismatch");
  const source: VisualObservationSource = structuredClone({
    materials: receipt.materials, leaseId: receipt.leaseId, jobId: receipt.jobId,
    receiptId: receipt.receiptId, receiptSha256: receipt.receiptFile.sha256,
    requestSha256: receipt.requestSha256, root: receipt.root, requestIndex: frame.requestIndex,
    requestedBundleMs: frame.requestedBundleMs, effectiveVideoMs: frame.effectiveVideoMs,
    actualVideoMs: frame.actualVideoMs, clampedToStart: frame.clampedToStart, clampedToEnd: frame.clampedToEnd,
    file: frame.file, clock: input.materials.payload.clock,
    collection: input.materials.payload.collection, factRefs: input.materials.payload.facts,
  });
  const profile = readProfile(png, frame.width, frame.height);
  if (profile.state === "unavailable") return { ...profile, source };
  return {
    state: "measured", schema: "apparent.frame-visual-observations/1", source,
    pixels: { width: frame.width, height: frame.height, origin: "decoded-derivative-rgba8", alpha: "ignored" },
    algorithm: "apparent.visual-luma/1",
    thumbnail: { columns: 64, rows: 36, sampling: "cell-centres", cells: profile.thumbnail },
    regions: { columns: profile.columns, rows: profile.rows, sampling: "legacy-grid128-sampled-mean", cells: profile.regions },
  };
}
