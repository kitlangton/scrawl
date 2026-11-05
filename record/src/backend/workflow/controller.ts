export type { CaptureReceipt, EngineClient, EngineEvent, EnginePhase, Readiness } from "../engine/client.js";
import type { CaptureReceipt, EngineClient, EngineEvent, EnginePhase, Readiness } from "../engine/client.js";

type Context = { captureId: string; readiness: Readiness };
export type RecordState =
  | { phase: "idle"; readiness: Readiness }
  | (Context & { phase: "checking" | "starting" | "recording" | "stopping" | "cancelling" | "processing" | "cancelled" })
  | (Context & { phase: "stop-failed" | "uncertain" | "failed"; message: string })
  | (Context & { phase: "ready"; bundlePath: string });

export type RecordCommand =
  | { type: "refresh-readiness" }
  | { type: "start"; requestId: string }
  | { type: "stop" | "retry-stop" | "cancel" | "reconcile" };

const terminal = new Set<RecordState["phase"]>(["idle", "ready", "cancelled", "failed"]);

/** Single mutation owner. Native facts remain authoritative across ambiguous replies. */
export class RecordWorkflow {
  #state: RecordState = { phase: "idle", readiness: { ready: false, reason: "Not checked" } };
  #listeners = new Set<(state: RecordState) => void>();
  #unsubscribe: () => void;
  #busy = false;
  #disposed = false;
  #revision = 0;
  #readinessRevision = 0;
  #usedIds = new Set<string>();
  private readonly engine: EngineClient;

  constructor(engine: EngineClient) {
    this.engine = engine;
    this.#unsubscribe = engine.subscribe(event => this.handle(event));
  }

  get state(): RecordState { return this.#state; }
  subscribe(listener: (state: RecordState) => void): () => void {
    if (!this.#disposed) this.#listeners.add(listener);
    return () => this.#listeners.delete(listener);
  }
  dispose(): void {
    this.#disposed = true;
    this.#unsubscribe();
    this.#listeners.clear();
  }

  async dispatch(command: RecordCommand): Promise<RecordState> {
    if (this.#disposed) return this.#state;
    if (command.type === "refresh-readiness") {
      const revision = ++this.#readinessRevision;
      const readiness = await this.engine.readiness();
      if (!this.#disposed && revision === this.#readinessRevision) this.set({ ...this.#state, readiness });
      return this.#state;
    }
    if (this.#busy) return this.#state;
    if (command.type === "start") return this.start(command.requestId);
    const current = this.#state;
    if (!("captureId" in current)) return current;
    if (command.type === "stop" && current.phase !== "recording") return current;
    if (command.type === "retry-stop" && current.phase !== "stop-failed") return current;
    if (command.type === "cancel" && current.phase !== "starting" && current.phase !== "recording") return current;
    if (command.type === "reconcile" && current.phase !== "uncertain" && current.phase !== "stop-failed") return current;
    this.#busy = true;
    const phase = command.type === "cancel" ? "cancelling" : command.type === "reconcile" ? "uncertain" : "stopping";
    this.set(phase === "uncertain"
      ? { phase, captureId: current.captureId, readiness: current.readiness, message: "Checking Engine state" }
      : { phase, captureId: current.captureId, readiness: current.readiness });
    const revision = this.#revision;
    try {
      const receipt = await (command.type === "cancel" ? this.engine.cancel(current.captureId)
        : command.type === "reconcile" ? this.engine.inspect(current.captureId) : this.engine.stop(current.captureId));
      if (this.isCurrent(revision)) this.applyReply(receipt, current.captureId);
    } catch (error) {
      if (this.isCurrent(revision)) this.set({ phase: phase === "stopping" ? "stop-failed" : "uncertain", captureId: current.captureId, message: messageOf(error), readiness: this.#state.readiness });
    } finally { this.#busy = false; }
    return this.#state;
  }

  private async start(captureId: string): Promise<RecordState> {
    if (!terminal.has(this.#state.phase) || !captureId.trim() || this.#usedIds.has(captureId)) return this.#state;
    // Reserve before the first await; IDs cannot be reused within this controller.
    this.#busy = true;
    this.#usedIds.add(captureId);
    this.set({ phase: "checking", captureId, readiness: this.#state.readiness });
    let submitted = false;
    let revision = this.#revision;
    const readinessRevision = ++this.#readinessRevision;
    try {
      const readiness = await this.engine.readiness();
      if (this.#disposed) return this.#state;
      if (readinessRevision === this.#readinessRevision) this.set({ ...this.#state, readiness });
      if (!readiness.ready || !this.#state.readiness.ready) {
        this.#usedIds.delete(captureId);
        this.set({ phase: "idle", readiness: this.#state.readiness });
        return this.#state;
      }
      this.set({ phase: "starting", captureId, readiness: this.#state.readiness });
      revision = this.#revision;
      submitted = true;
      const receipt = await this.engine.start(captureId);
      if (this.isCurrent(revision)) this.applyReply(receipt, captureId);
    } catch (error) {
      if (this.isCurrent(revision)) this.set({ phase: submitted ? "uncertain" : "failed", captureId, message: messageOf(error), readiness: this.#state.readiness });
    } finally { this.#busy = false; }
    return this.#state;
  }

  private isCurrent(revision: number): boolean { return !this.#disposed && revision === this.#revision; }

  private applyReply(receipt: CaptureReceipt, captureId: string): void {
    if (receipt.captureId !== captureId) throw new Error("Engine receipt did not match the owned capture ID");
    this.accept(receipt);
  }

  private handle(event: EngineEvent): void {
    if (this.#disposed) return;
    if (event.type === "readiness-changed") {
      ++this.#readinessRevision;
      this.set({ ...this.#state, readiness: event.readiness });
      return;
    }
    const current = this.#state;
    if (!("captureId" in current) || current.captureId !== event.receipt.captureId || !canApplyEvent(current.phase, event.receipt.phase)) return;
    ++this.#revision; // An authoritative event supersedes any pending command reply.
    this.accept(event.receipt);
  }

  private accept(receipt: CaptureReceipt): void {
    const context = { captureId: receipt.captureId, readiness: this.#state.readiness };
    switch (receipt.phase) {
      case "starting": case "recording": case "processing": case "cancelled":
        this.set({ phase: receipt.phase, ...context }); break;
      case "saved":
        this.set(receipt.bundlePath ? { phase: "ready", ...context, bundlePath: receipt.bundlePath }
          : { phase: "uncertain", ...context, message: "Saved receipt did not include a bundle path" }); break;
      case "failed": case "rejected":
        this.set({ phase: "failed", ...context, message: receipt.error ?? receipt.phase }); break;
    }
  }

  private set(state: RecordState): void {
    if (this.#disposed) return;
    this.#state = Object.freeze({ ...state, readiness: Object.freeze({ ...state.readiness }) });
    for (const listener of this.#listeners) {
      try { listener(this.#state); }
      catch { this.#listeners.delete(listener); }
    }
  }
}

function messageOf(error: unknown): string { return error instanceof Error ? error.message : String(error); }
function canApplyEvent(state: RecordState["phase"], receipt: EnginePhase): boolean {
  const allowed: Partial<Record<RecordState["phase"], readonly EnginePhase[]>> = {
    starting: ["starting", "recording", "processing", "saved", "cancelled", "failed", "rejected"],
    recording: ["recording", "processing", "saved", "cancelled", "failed", "rejected"],
    stopping: ["processing", "saved", "cancelled", "failed", "rejected"],
    cancelling: ["processing", "saved", "cancelled", "failed", "rejected"],
    "stop-failed": ["recording", "processing", "saved", "cancelled", "failed", "rejected"],
    uncertain: ["starting", "recording", "processing", "saved", "cancelled", "failed", "rejected"],
    processing: ["processing", "saved", "failed", "rejected"],
  };
  return allowed[state]?.includes(receipt) ?? false;
}
