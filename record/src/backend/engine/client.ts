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
 */
export interface EngineClient {
  readiness(): Promise<Readiness>;
  start(requestId: string): Promise<CaptureReceipt>;
  stop(captureId: string): Promise<CaptureReceipt>;
  cancel(captureId: string): Promise<CaptureReceipt>;
  inspect(captureId: string): Promise<CaptureReceipt>;
  subscribe(listener: (event: EngineEvent) => void): () => void;
}
