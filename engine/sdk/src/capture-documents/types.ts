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
  confirmedSource: ConfirmedSource;
  invocation: InvocationFact;
  files: SealedFile[];
  video: { file: string; width: number; height: number; durationMs: number };
  audio?: { file: string; durationMs: number; sampleRate: number; channels: number };
  facts: { context?: FactRef; interactions?: FactRef; candidates?: FactRef; transcript?: FactRef; acceptedStrokes?: FactRef };
  collection: { context: CollectionState; interactions: CollectionState; candidates: CollectionState; transcript: CollectionState; acceptedStrokes: CollectionState };
  warnings: Array<{ code: string; atMs?: number }>;
  safety: { nativeRemoteDelivery: "forbidden"; safeForProcessing: boolean };
}

export interface ConfirmedSource {
  id: string; generation: Id; confirmedAtMs: number;
  kind: "display" | "area"; displayId: number;
  rectangle: { x: number; y: number; width: number; height: number; scale: number };
  confirmation: { kind: "portal" | "native-picker" | "explicit-display"; id: Id };
}
export type InvocationFact = { invocationId: Id; observedAtMs: number } & (
  | { kind: "explicit" }
  | { kind: "picker"; confirmationId: Id }
  | { kind: "shortcut"; hostEventId: Id; consumedReservationId?: Id }
);
export type Observed<T> = { state: "collected"; value: T } | { state: "unavailable"; reason: string };
export interface ContextFacts {
  schema: "apparent.capture-context/1"; identity: { captureId: Id; materialsId: Id };
  source: ConfirmedSource; invocation: InvocationFact;
  context: Observed<{ values: Record<string, string>; changes: Array<{ timeMs: number; key: string; value: string | null }> }>;
  session: Observed<{ id: string; directory?: string; url?: string }>;
  windows: Observed<Array<{ timeMs: number; entries: Array<{ id: number; app: string; bundleId?: string; title?: string; regular?: boolean; bounds: [number, number, number, number]; layer: number; frontmost?: boolean }> }>>;
  cursor: Observed<Array<[number, number, number]>>;
}
export type InteractionEventFact =
  | { kind: "note"; t: number; pin: number; x: number; y: number; text: string }
  | { kind: "click"; t: number; x: number; y: number; button?: string; label?: string; role?: string }
  | { kind: "drag"; t: number; t1: number; x0: number; y0: number; x1: number; y1: number; label?: string; button?: string; path?: Array<[number, number]> }
  | { kind: "scroll"; t: number; x: number; y: number; dx: number; dy: number; label?: string }
  | { kind: "point"; t: number; x: number; y: number; label?: string }
  | { kind: "key"; t: number; shortcut: string }
  | { kind: "app_change"; t: number; app: string; window_title?: string }
  | { kind: "app_event"; t: number; label: string };
export interface InteractionFacts { schema: "apparent.capture-interactions/1"; identity: { captureId: Id; materialsId: Id }; events: InteractionEventFact[] }
export interface CandidateFacts { schema: "apparent.capture-candidates/1"; identity: { captureId: Id; materialsId: Id }; candidates: Array<{ candidateId: Id; timeMs: number; file: string; width: number; height: number; visualChange?: number }> }
export interface TranscriptFacts { schema: "apparent.capture-transcript/1"; identity: { captureId: Id; materialsId: Id }; provider: string; model: string; text: string; segments: Array<{ startMs: number; endMs?: number; text: string }> }
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
