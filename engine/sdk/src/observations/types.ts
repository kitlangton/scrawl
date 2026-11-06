import type { MaterialsDocument, MaterialsKey, MaterialsPayload, SealedFile } from "../capture-documents/types.ts";
import type { Derivative2DocumentInput } from "../media2/types.ts";

export interface FrameVisualObservationInput {
  /** Original request/layout and detached receipt, never expectations reconstructed from output. */
  document: Derivative2DocumentInput;
  /** Previously validated original materials document. */
  materials: MaterialsDocument;
  requestIndex: number;
  pngBytes: Uint8Array;
  maxFileBytes: number;
}
export interface VisualObservationSource {
  materials: MaterialsKey;
  leaseId: string;
  jobId: string;
  receiptId: string;
  receiptSha256: string;
  requestSha256: string;
  root: string;
  requestIndex: number;
  requestedBundleMs: number;
  effectiveVideoMs: number;
  actualVideoMs: number;
  clampedToStart: boolean;
  clampedToEnd: boolean;
  file: SealedFile;
  clock: MaterialsPayload["clock"];
  collection: MaterialsPayload["collection"];
  factRefs: MaterialsPayload["facts"];
}
export type FrameVisualObservation = {
  state: "measured";
  schema: "apparent.frame-visual-observations/1";
  source: VisualObservationSource;
  pixels: { width: number; height: number; origin: "decoded-derivative-rgba8"; alpha: "ignored" };
  algorithm: "apparent.visual-luma/1";
  thumbnail: { columns: 64; rows: 36; sampling: "cell-centres"; cells: readonly number[] };
  regions: { columns: number; rows: number; sampling: "legacy-grid128-sampled-mean"; cells: readonly number[] };
} | {
  state: "unavailable";
  source: VisualObservationSource;
  reason: "observation-profile-absent" | "observation-profile-unsupported";
};
