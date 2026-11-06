import type { DerivativeFrame, MaterialsDocument, MaterialsKey, SealedFile } from "../capture-documents/types.ts";

/** Pixel execution only. Record supplies all layout and selection policy. */
export interface ContactSheetLayout {
  schema: "apparent.contact-sheet-layout/1";
  width: number;
  height: number;
  backgroundRgb: [number, number, number];
  fit: "stretch";
  sampling: "legacy-bilinear";
  clipping: "reject";
  tiles: Array<{ requestIndex: number; x: number; y: number; width: number; height: number }>;
}
/** No credential is serialized into the public request digest. Authentication is
 * supplied separately to the native channel and remains required on every call. */
export interface Media2JobRequest {
  schema: "apparent.retained-frame-request/1";
  materials: MaterialsKey;
  leaseId: string;
  jobId: string;
  outputDirectory: string;
  requestedBundleMs: number[];
  limits: { maxFrames: number; maxEdge: number; maxTotalPixels: number; deadlineMs: number };
  contactSheet: ContactSheetLayout | null;
}
export interface Media2PreparedRequest { request: Media2JobRequest; bytes: Uint8Array; sha256: string }
export interface Derivative2Image { encoding: "png"; byteLength: number; file: SealedFile }
export interface Derivative2Frame extends DerivativeFrame, Derivative2Image { actualVideoMs: number }
export interface Derivative2Sheet extends Derivative2Image {
  width: number;
  height: number;
  /** Hash of the exact detached layoutBytes, not an object/self hash. */
  layoutSha256: string;
  layout: ContactSheetLayout;
}
export interface Derivative2Payload {
  schema: "apparent.media-derivative-payload/2";
  materials: MaterialsKey;
  leaseId: string;
  jobId: string;
  receiptId: string;
  root: string;
  requestSha256: string;
  frames: Derivative2Frame[];
  contactSheet: Derivative2Sheet | null;
}
export interface Derivative2Envelope {
  schema: "apparent.media-derivative/2";
  materials: MaterialsKey;
  leaseId: string;
  jobId: string;
  receiptId: string;
  root: string;
  requestSha256: string;
  receiptFile: SealedFile;
  payload: Derivative2Payload;
}
export interface Derivative2DocumentInput {
  envelope: unknown;
  bytes: Uint8Array;
  /** Restore the original prepared request, never derive it from the receipt. */
  request: Media2JobRequest;
  layoutBytes: Uint8Array | null;
}
export type Derivative2Selector = { kind: "frame"; requestIndex: number } | { kind: "contact-sheet" };
export interface Derivative2ArtifactInput {
  kind: "derivative2";
  receipt: Derivative2Envelope;
  selector: Derivative2Selector;
}
export interface ValidatedDerivative2Document { envelope: Derivative2Envelope; materials: MaterialsDocument }
