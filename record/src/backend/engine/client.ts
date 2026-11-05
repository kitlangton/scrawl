/** Consumer-side Engine contract. No production transport is shipped in this package. */
export type Readiness =
  | { ready: true }
  | { ready: false; reason: string };

export type EnginePhase =
  | "starting"
  | "recording"
  | "processing"
  | "saved"
  | "cancelled"
  | "failed"
  | "rejected";

export interface CaptureReceipt {
  captureId: string;
  phase: EnginePhase;
  bundlePath?: string;
  error?: string;
}

export type EngineEvent =
  | { type: "readiness-changed"; readiness: Readiness }
  | { type: "capture-changed"; receipt: CaptureReceipt };

/**
 * Host adapter for existing Engine capabilities. start IDs must be retry-safe;
 * stop, cancel and inspect must remain scoped to the supplied capture ID.
 * All calls must be bounded. Transport ambiguity rejects rather than synthesizing
 * terminal receipts; failed/rejected guarantee no live capture for that ID.
 * Subscribe delivers authoritative, ordered events for each capture.
 */
export interface EngineClient {
  readiness(): Promise<Readiness>;
  start(requestId: string): Promise<CaptureReceipt>;
  stop(captureId: string): Promise<CaptureReceipt>;
  cancel(captureId: string): Promise<CaptureReceipt>;
  inspect(captureId: string): Promise<CaptureReceipt>;
  subscribe(listener: (event: EngineEvent) => void): () => void;
}
