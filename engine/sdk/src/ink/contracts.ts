/** Capture/2 authenticated, trusted-native-observer ingress. NOT OS attestation,
 * a renderer API, rendered-effect proof or an advertised native capability.
 * Original SDK045 StrokeFacts remain immutable accepted GESTURE HISTORY;
 * t0/t1 are gesture times, not visibility. Movie pixels are visual authority. */
export const CAPTURE_INK_VERSION = 1 as const;
export const CAPTURE_INK_LIMITS = Object.freeze({
  maxWireBytes: 65_536, maxBatchJsonBytes: 16_384,
  maxPointsPerBatch: 256, maxPointsPerGesture: 2_400, maxBatches: 4_096,
  maxSurfaces: 8, maxGestures: 4_096, maxTotalPoints: 65_536,
  maxFactBytes: 4 * 1024 * 1024, finishGraceMs: 2_000,
});
/** Canonical unsigned u64 decimal string. Never serialize ns through JS Number. */
export type InkMonotonicNs = string;
/** 32 lowercase hex, native-minted for producer/gesture; Engine-minted registration. */
export type InkId = string;
export interface InkRectangle { x: number; y: number; width: number; height: number }
export interface InkSurface {
  /** Opaque native-owner logical identity; no inferred caller-created geometry. */
  surfaceId: string; transformId: string; nativeIncarnation: string;
  target: 'capture-rectangle' | 'whiteboard-canvas';
  /** Screen samples are desktop physical coordinates. Board samples are actual
   * native paper-local observations after the donor's 64-point toolbar offset. */
  pointSpace: 'desktop-physical' | 'paper-logical';
  /** Desktop-physical content rectangle; Board means PAPER content excluding
   * the 64-point toolbar: actual viewport, independently resizable from the
   * full paper plane. Never rescale paper-local points to this rectangle.
   * Initial scale1 mapping must agree with native layout. */
  contentRectangle: InkRectangle;
  /** Screen: original physical source size. Board: exactly1600x1100. */
  plane: { width: number; height: number };
}
export interface InkSourceObservation {
  /** Identity joined by the privileged NativeHost client before forwarding. */
  host: { processEpoch: string; revision: number; pid: number; startTicks: string };
  producerId: InkId; generation: number;
  bootId: string; clock: 'linux-clock-monotonic-observation';
  /** Initially only exact single-monitor scale1 output joins are supported. */
  output: { name: string; physical: InkRectangle; compositor: InkRectangle & { scale: 1 } };
  surfaces: InkSurface[];
}
export interface RegisterInkProducerInput {
  ingressVersion: 1; ownerToken: string; captureId: InkId;
  /** Exact original Engine reference; not a claimed native identity. */
  source: { id: string; generation: string };
  observation: InkSourceObservation;
}
export interface InkRegistrationReceipt {
  ingressVersion: 1; captureId: InkId; producerId: InkId; generation: number;
  registrationId: InkId; producerToken: string; nextSequence: number;
  clock: { kind: 'linux-clock-monotonic-observation'; bootId: string; admissionMonotonicNs: InkMonotonicNs };
  limits: typeof CAPTURE_INK_LIMITS;
}
interface InkAuthority {
  ingressVersion: 1; ownerToken: string; captureId: InkId;
  registrationId: InkId; producerToken: string;
}
/** Finite coordinates must survive checked int32 normalization. Overflow or a
 * malformed sample invalidates collection; it is never silently clamped/dropped. */
export interface InkPoint { timeNs: InkMonotonicNs; x: number; y: number }
export type InkEvent =
  | { kind: 'begin'; gestureId: InkId; surfaceId: string; transformId: string;
      nativeIncarnation: string; timeNs: InkMonotonicNs; x: number; y: number; rgb?: [number, number, number] }
  | { kind: 'points'; gestureId: InkId; points: InkPoint[] }
  | { kind: 'commit'; gestureId: InkId; acceptedAtNs: InkMonotonicNs }
  | { kind: 'discard'; gestureId: InkId; observedAtNs: InkMonotonicNs;
      reason: 'undo-active' | 'clear-active' | 'source-changed' | 'input-sealed' }
  | { kind: 'undo' | 'clear'; surfaceId: string; transformId: string;
      nativeIncarnation: string; observedAtNs: InkMonotonicNs; nativeRevision: number }
  | { kind: 'surface-state'; state: 'opened' | 'closed' | 'replaced'; surface: InkSurface;
      observedAtNs: InkMonotonicNs; nativeRevision: number };
export interface SubmitInkEventsInput extends InkAuthority {
  /** Native retained BATCH sequence, starts0; no gaps or rebinding on lost reply. */
  sequence: number;
  /** Exact UTF8 JSON array of InkEvent from NativeHost, <=16KiB. Forward it
   * unchanged, not parsed/reserialized: Rust/JS float serialization can differ. */
  eventsJson: string;
  /** SHA256 of those exact UTF8 eventsJson bytes, not a JSON reserialization. */
  batchSha256: string;
}
export interface InkBatchReceipt {
  ingressVersion: 1; captureId: InkId; registrationId: InkId;
  sequence: number; batchSha256: string; nextSequence: number;
  outcome: 'accepted' | 'replayed';
}
export interface FinishInkProducerInput extends InkAuthority {
  /** null iff the genuinely observed producer completed without ANY batches. */
  finalSequence: number | null;
  /** Drain observation may follow video Stop; it is lifecycle evidence, NEVER
   * substituted for a gesture's acceptedAt or clamped into original fact time. */
  seal: { state: 'sealed-and-drained'; observedAtNs: InkMonotonicNs; nativeRevision: number };
}
export interface InkFinishReceipt {
  ingressVersion: 1; captureId: InkId; registrationId: InkId;
  finalSequence: number | null; state: 'complete' | 'unavailable';
}
/** Engine admission receipts never replace native input/effect observation or
 * the original material/Saved authority. Secrets stay in privileged host only.
 * Video Stop/Cancel remain live while the bounded pre-material drain finishes. */
export interface CaptureInkPort {
  registerInkProducer(input: RegisterInkProducerInput, timeoutMs?: number): Promise<InkRegistrationReceipt>;
  submitInkEvents(input: SubmitInkEventsInput, timeoutMs?: number): Promise<InkBatchReceipt>;
  finishInkProducer(input: FinishInkProducerInput, timeoutMs?: number): Promise<InkFinishReceipt>;
}
