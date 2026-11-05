export type Id = string;
export type Sha256 = string;

export interface FileIdentity { device: string; inode: string; size: number; modifiedNs: string }
export interface SealedFile { relativePath: string; identity: FileIdentity; sha256: Sha256 }
export interface MaterialsKey { captureId: Id; materialsId: Id; revision: Sha256 }
export interface FactRef { file: string; schema: string }
export type CollectionState = "collected" | "unavailable";
export interface MaterialsPayload {
  schema: "apparent.capture-materials-payload/1";
  identity: { captureId: Id; materialsId: Id };
  bundleId: string;
  root: string;
  leaseId: Id;
  clock: { epoch: "capture-admission-monotonic"; units: "integer-milliseconds"; durationMs: number; videoOffsetMs: number; audioOffsetMs?: number };
  // Native facts remain opaque until their existing-format schemas are frozen.
  confirmedSource: unknown;
  invocation: unknown;
  files: SealedFile[];
  video: { file: string; width: number; height: number; durationMs: number };
  audio?: { file: string; durationMs: number; sampleRate: number; channels: number };
  facts: { context?: FactRef; interactions?: FactRef; candidates?: FactRef; transcript?: FactRef; acceptedStrokes?: FactRef };
  collection: { context: CollectionState; interactions: CollectionState; candidates: CollectionState; transcript: CollectionState; acceptedStrokes: CollectionState };
  warnings: Array<{ code: string; atMs?: number }>;
  safety: { nativeRemoteDelivery: "forbidden"; safeForProcessing: boolean };
}
export interface MaterialsManifest { schema: "apparent.capture-materials/1"; key: MaterialsKey; root: string; leaseId: Id; payloadFile: SealedFile }
export interface MaterialsDocument { manifest: MaterialsManifest; payload: MaterialsPayload }

export interface DerivativeFrame { requestIndex: number; requestedBundleMs: number; effectiveVideoMs: number; actualVideoMs?: number; clampedToStart: boolean; clampedToEnd: boolean; width: number; height: number; file: SealedFile }
export interface DerivativePayload { schema: "apparent.media-derivative-payload/1"; materials: MaterialsKey; jobId: Id; receiptId: Id; root: string; frames: DerivativeFrame[] }
export interface DerivativeEnvelope { schema: "apparent.media-derivative/1"; materials: MaterialsKey; jobId: Id; receiptId: Id; root: string; receiptFile: SealedFile; payload: DerivativePayload }
export interface DerivativeDocumentInput { envelope: unknown; bytes: Uint8Array; requestedBundleMs: readonly number[] }

export interface InferencePayload { schema: "apparent.transcript-result-payload/1"; materials: MaterialsKey; jobId: Id; receiptId: Id; root: string; clock: "bundle"; state: "complete" | "unavailable"; transcript?: { provider: string; model: string; text: string; segments: Array<{ startMs: number; endMs?: number; text: string }> }; warning?: { code: "transcript_unavailable"; retryable: boolean } }
export interface InferenceEnvelope { schema: "apparent.transcript-result/1"; materials: MaterialsKey; jobId: Id; receiptId: Id; root: string; receiptFile: SealedFile; payload: InferencePayload }

export interface StrokeFacts { schema: "apparent.capture-strokes/1"; identity: { captureId: Id; materialsId: Id }; coordinates: "target-normalized-10000"; surfaces: Array<{ surfaceId: string; transformId: string; width: number; height: number; target: "capture-rectangle" | "whiteboard-canvas" }>; strokes: Array<{ id: Id; surface: "screen" | "whiteboard"; surfaceId: string; transformId: string; startMs: number; endMs: number; acceptedAtMs: number; points: Array<{ timeMs: number; x: number; y: number }> }> }

export type ArtifactInput =
  | { kind: "material"; relativePath: string }
  | { kind: "assembled"; file: SealedFile }
  | { kind: "derivative"; receipt: DerivativeEnvelope; fileIndex: number };
export interface PublicationEntry { relativePath: string; sha256: Sha256; byteLength: number; role: "manifest" | "summary" | "companion" | "frame" | "contact-sheet" | "video" | "audio" | "transcript" | "context" | "interactions" | "marks"; input: ArtifactInput }
export interface PublicationPlan { schema: "apparent.bundle-publication/1"; publicationId: Id; materials: MaterialsKey; bundleId: string; stagingRoot: string; destination: { path: string; mode: "absent" }; artifactVersion: 1; entries: PublicationEntry[]; manifestPath: "apparent.json"; summaryPath: "summary.md"; profile: "apparent-bundle/1" }
export interface PublishEnvelope { stagingRoot: string; planFile: SealedFile; planSha256: Sha256 }
