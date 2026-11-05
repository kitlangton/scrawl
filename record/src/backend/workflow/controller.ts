export type { CaptureReceipt, EngineClient, EngineEvent, EnginePhase, Readiness } from "../engine/client.js";
import type { CaptureReceipt, EngineClient, EngineEvent, EnginePhase, Readiness } from "../engine/client.js";

export type RecordState =
  | { phase: "idle"; readiness: Readiness }
  | { phase: "starting"; captureId: string; readiness: Readiness }
  | { phase: "recording"; captureId: string; readiness: Readiness }
  | { phase: "stopping"; captureId: string; readiness: Readiness }
  | { phase: "stop-failed"; captureId: string; message: string; readiness: Readiness }
  | { phase: "processing"; captureId: string; readiness: Readiness }
  | { phase: "ready"; captureId: string; bundlePath: string; readiness: Readiness }
  | { phase: "cancelled"; captureId: string; readiness: Readiness }
  | { phase: "failed"; captureId: string; message: string; readiness: Readiness };

export type RecordCommand =
  | { type: "refresh-readiness" }
  | { type: "start"; requestId: string }
  | { type: "stop" }
  | { type: "retry-stop" }
  | { type: "cancel" };

const terminal = new Set<RecordState["phase"]>(["idle", "ready", "cancelled", "failed"]);

/** Explicit local workflow. An EngineClient must be supplied by the eventual host adapter. */
export class RecordWorkflow {
  private readonly engine: EngineClient;
  #state: RecordState = { phase: "idle", readiness: { ready: false, reason: "Not checked" } };
  #listeners = new Set<(state: RecordState) => void>();
  #unsubscribe: () => void;

  constructor(engine: EngineClient) {
    this.engine = engine;
    this.#unsubscribe = engine.subscribe((event) => this.handle(event));
  }

  get state(): RecordState {
    return this.#state;
  }

  subscribe(listener: (state: RecordState) => void): () => void {
    this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }

  dispose(): void {
    this.#unsubscribe();
    this.#listeners.clear();
  }

  async dispatch(command: RecordCommand): Promise<RecordState> {
    switch (command.type) {
      case "refresh-readiness": {
        const readiness = await this.engine.readiness();
        this.set({ ...this.#state, readiness });
        return this.#state;
      }
      case "start":
        return this.start(command.requestId);
      case "stop":
        return this.stop(false);
      case "retry-stop":
        return this.stop(true);
      case "cancel":
        return this.cancel();
    }
  }

  private async start(requestId: string): Promise<RecordState> {
    if (!terminal.has(this.#state.phase)) return this.#state;
    const readiness = await this.engine.readiness();
    this.set({ ...this.#state, readiness });
    if (!readiness.ready) return this.#state;
    const receipt = await this.engine.start(requestId);
    if (receipt.captureId !== requestId) {
      this.set({ phase: "failed", captureId: requestId, message: "Engine start receipt did not match the owned request ID", readiness });
      return this.#state;
    }
    this.accept(receipt);
    return this.#state;
  }

  private async stop(retry: boolean): Promise<RecordState> {
    const current = this.#state;
    if (retry) {
      if (current.phase !== "stop-failed") return current;
    } else if (current.phase !== "recording") return current;
    if (!("captureId" in current)) return current;
    const captureId = current.captureId;
    this.set({ phase: "stopping", captureId, readiness: current.readiness });
    try {
      // A failed response is retried with the same ID. Engine stop is ID-scoped and
      // the production CLI reports already-ended/retained terminal receipts safely.
      const receipt = await this.engine.stop(captureId);
      if (receipt.captureId !== captureId) throw new Error("Engine stop receipt did not match the owned capture ID");
      this.accept(receipt);
    } catch (error) {
      this.set({
        phase: "stop-failed",
        captureId,
        message: messageOf(error),
        readiness: current.readiness,
      });
    }
    return this.#state;
  }

  private async cancel(): Promise<RecordState> {
    const current = this.#state;
    if (current.phase !== "starting" && current.phase !== "recording") return current;
    try {
      const receipt = await this.engine.cancel(current.captureId);
      if (receipt.captureId !== current.captureId) throw new Error("Engine cancel receipt did not match the owned capture ID");
      this.accept(receipt);
    } catch (error) {
      this.set({ phase: "failed", captureId: current.captureId, message: messageOf(error), readiness: current.readiness });
    }
    return this.#state;
  }

  private handle(event: EngineEvent): void {
    if (event.type === "readiness-changed") {
      this.set({ ...this.#state, readiness: event.readiness });
      return;
    }
    const current = this.#state;
    if ("captureId" in current && current.captureId !== event.receipt.captureId) return;
    if (current.phase === "idle" || current.phase === "ready" || current.phase === "cancelled" || current.phase === "failed") return;
    if (!canApplyEvent(current.phase, event.receipt.phase)) return;
    this.accept(event.receipt);
  }

  private accept(receipt: CaptureReceipt): void {
    const readiness = this.#state.readiness;
    switch (receipt.phase) {
      case "starting": this.set({ phase: "starting", captureId: receipt.captureId, readiness }); break;
      case "recording": this.set({ phase: "recording", captureId: receipt.captureId, readiness }); break;
      case "processing": this.set({ phase: "processing", captureId: receipt.captureId, readiness }); break;
      case "saved":
        if (receipt.bundlePath) this.set({ phase: "ready", captureId: receipt.captureId, bundlePath: receipt.bundlePath, readiness });
        else this.set({ phase: "failed", captureId: receipt.captureId, message: "Saved receipt did not include a bundle path", readiness });
        break;
      case "cancelled": this.set({ phase: "cancelled", captureId: receipt.captureId, readiness }); break;
      case "failed":
      case "rejected": this.set({ phase: "failed", captureId: receipt.captureId, message: receipt.error ?? receipt.phase, readiness }); break;
    }
  }

  private set(state: RecordState): void {
    this.#state = state;
    for (const listener of this.#listeners) listener(state);
  }
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function canApplyEvent(state: RecordState["phase"], receipt: EnginePhase): boolean {
  const allowed: Partial<Record<RecordState["phase"], readonly EnginePhase[]>> = {
    starting: ["starting", "recording", "processing", "saved", "cancelled", "failed", "rejected"],
    recording: ["recording", "processing", "saved", "cancelled", "failed", "rejected"],
    stopping: ["processing", "saved", "cancelled", "failed", "rejected"],
    "stop-failed": ["recording", "processing", "saved", "cancelled", "failed", "rejected"],
    processing: ["processing", "saved", "failed", "rejected"],
  };
  return allowed[state]?.includes(receipt) ?? false;
}
