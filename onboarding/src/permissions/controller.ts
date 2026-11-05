import type { CapabilitySnapshot, CommandResult, NativeRequirement, RecordClient, RecordCommand, RepairAction } from "@apparent/record/capabilities";

export type RepairPhase = "idle" | "checking" | "blocked" | "requesting" | "waiting" | "relaunch-needed" | "ready" | "closed" | "error";
export type RepairState = Readonly<{
  phase: RepairPhase;
  requestedIds: readonly string[];
  snapshot: CapabilitySnapshot | null;
  pendingRequirementId: string | null;
  error: string | null;
  closeReason?: "close" | "later" | "ready";
}>;
export type RepairTimer = { set(callback: () => void, delayMs: number): unknown; clear(handle: unknown): void };
export type PermissionRepairDependencies = { record: RecordClient; id(): string; timer?: RepairTimer };

const knownRequirements = new Set(["screen-capture", "microphone", "accessibility", "camera", "speech-recognition"]);
const inputMonitoring = "input-monitoring";
const autoCloseMs = 800;
const defaultTimer: RepairTimer = { set: (callback, delayMs) => setTimeout(callback, delayMs), clear: handle => clearTimeout(handle as ReturnType<typeof setTimeout>) };

/** A one-screen repair controller. It owns no tour progress and issues only Record permission commands. */
export class PermissionRepairController {
  #state: RepairState = { phase: "idle", requestedIds: [], snapshot: null, pendingRequirementId: null, error: null };
  #listeners = new Set<(state: RepairState) => void>();
  #unsubscribe: () => void;
  #disposed = false;
  #epoch: string | null = null;
  #revision = -1;
  #active = false;
  #inFlight: string | null = null;
  #closeTimer: unknown = null;
  #timer: RepairTimer;

  constructor(private readonly deps: PermissionRepairDependencies) {
    this.#timer = deps.timer ?? defaultTimer;
    this.#unsubscribe = deps.record.observe(snapshot => this.observe(snapshot));
  }
  get state(): RepairState { return this.#state; }
  subscribe(listener: (state: RepairState) => void): () => void { this.#listeners.add(listener); return () => this.#listeners.delete(listener); }

  activate(requestedIds: readonly string[]): RepairState {
    if (this.#disposed || this.#state.phase === "closed") return this.#state;
    const unique = [...new Set(requestedIds)];
    if (this.#active && unique.length === this.#state.requestedIds.length && unique.every(id => this.#state.requestedIds.includes(id))) return this.#state;
    this.#active = true;
    this.#clearTimer();
    if (unique.length === 0 || unique.some(id => id === inputMonitoring || !knownRequirements.has(id))) {
      return this.set({ ...this.#state, phase: "error", requestedIds: unique, pendingRequirementId: null, error: "Access is unavailable right now." });
    }
    this.#inFlight = null;
    const snapshot = this.#state.snapshot;
    return this.set(this.forFacts(unique, snapshot));
  }

  async repair(requirementId: string): Promise<CommandResult | null> {
    if (!this.#active || this.#disposed || this.#inFlight || this.#state.phase === "closed") return null;
    const snapshot = this.#state.snapshot;
    const requirement = this.requirement(snapshot, requirementId);
    const action = this.actionFor(requirement);
    if (!snapshot || snapshot.connection !== "connected" || !requirement || !action) return null;
    const requestId = this.deps.id();
    const epoch = snapshot.providerEpoch, revision = snapshot.revision;
    this.#inFlight = requestId;
    this.set({ ...this.#state, phase: action === "request-safe-relaunch" ? "relaunch-needed" : "requesting", pendingRequirementId: requirementId, error: null });
    try {
      const expected = { providerEpoch: epoch, revision };
      let command: RecordCommand;
      if (action === "request-safe-relaunch") command = { type: action, requestId, expected };
      else if (action === "request-permission") command = { type: action, requirementId, requestId, expected };
      else command = { type: action, requirementId, requestId, expected };
      const result = await this.deps.record.execute(command);
      if (this.#disposed || this.#inFlight !== requestId) return result;
      this.#inFlight = null;
      const current = this.#state.snapshot;
      const resultSnapshot = result.snapshot;
      if (current?.providerEpoch !== epoch) return result;
      if (result.requestId !== requestId || resultSnapshot.providerEpoch !== epoch) { this.#inFlight = null; this.reconcile(); return result; }
      if (current.revision > revision) { this.#inFlight = null; this.reconcile(); return result; }
      if (resultSnapshot.revision >= (current?.revision ?? -1)) this.observe(resultSnapshot);
      if (result.status === "accepted") this.set({ ...this.#state, phase: "waiting", pendingRequirementId: requirementId, error: null });
      else if (result.status === "refused" || result.status === "outcome-unknown") this.set({ ...this.#state, phase: "error", pendingRequirementId: null, error: "Access could not be updated. Check again and try once more." });
      else this.reconcile();
      return result;
    } catch {
      if (!this.#disposed && this.#inFlight === requestId) {
        this.#inFlight = null;
        this.set({ ...this.#state, phase: "error", pendingRequirementId: null, error: "Access could not be updated. Check again and try once more." });
      }
      return null;
    }
  }

  async refresh(): Promise<CommandResult | null> {
    const snapshot = this.#state.snapshot;
    if (!this.#active || this.#disposed || !snapshot || snapshot.connection !== "connected" || this.#inFlight) return null;
    const requestId = this.deps.id(), epoch = snapshot.providerEpoch, revision = snapshot.revision;
    this.#inFlight = requestId;
    this.set({ ...this.#state, phase: "checking", pendingRequirementId: null, error: null });
    try {
      const result = await this.deps.record.execute({ type: "refresh", requestId, expected: { providerEpoch: epoch, revision } });
      if (this.#disposed || this.#inFlight !== requestId) return result;
      this.#inFlight = null;
      const current = this.#state.snapshot;
      if (result.requestId !== requestId || result.snapshot.providerEpoch !== epoch || current?.providerEpoch !== epoch) { this.reconcile(); return result; }
      if (current.revision <= revision) this.observe(result.snapshot);
      if (result.status === "refused" || result.status === "outcome-unknown") this.set({ ...this.#state, phase: "error", error: "Access status could not be refreshed. Try again." });
      else if (result.status !== "accepted") this.reconcile();
      else this.set({ ...this.#state, phase: "waiting", error: null });
      return result;
    } catch {
      if (!this.#disposed && this.#inFlight === requestId) { this.#inFlight = null; this.set({ ...this.#state, phase: "error", error: "Access status could not be refreshed. Try again." }); }
      return null;
    }
  }
  close(): void { this.finish("close"); }
  cancel(): void { this.finish("later"); }
  dispose(): void { if (this.#disposed) return; this.#disposed = true; this.#active = false; this.#inFlight = null; this.#clearTimer(); this.#unsubscribe(); this.#listeners.clear(); }

  private observe(snapshot: CapabilitySnapshot): void {
    if (this.#disposed) return;
    if (snapshot.providerEpoch === this.#epoch && snapshot.revision <= this.#revision) return;
    const newEpoch = snapshot.providerEpoch !== this.#epoch;
    this.#epoch = snapshot.providerEpoch;
    this.#revision = snapshot.revision;
    if (!this.#active || this.#state.phase === "closed") { this.set({ ...this.#state, snapshot }); return; }
    if (newEpoch) this.#inFlight = null;
    this.set(this.forFacts(this.#state.requestedIds, snapshot));
  }

  private forFacts(requestedIds: readonly string[], snapshot: CapabilitySnapshot | null): RepairState {
    if (!snapshot || snapshot.connection !== "connected") { this.#clearTimer(); return { phase: "checking", requestedIds, snapshot, pendingRequirementId: null, error: null }; }
    const rows = requestedIds.map(id => this.requirement(snapshot, id));
    if (rows.some(row => !row)) { this.#clearTimer(); return { phase: "blocked", requestedIds, snapshot, pendingRequirementId: null, error: "Access information is unavailable right now." }; }
    if (rows.some(row => row!.relaunchNeeded)) { this.#clearTimer(); return { phase: "relaunch-needed", requestedIds, snapshot, pendingRequirementId: null, error: null }; }
    if (rows.every(row => row!.status === "granted" && row!.usableNow)) {
      this.scheduleClose(snapshot);
      return { phase: "ready", requestedIds, snapshot, pendingRequirementId: null, error: null };
    }
    this.#clearTimer();
    return { phase: "blocked", requestedIds, snapshot, pendingRequirementId: null, error: null };
  }

  private reconcile(): void {
    if (!this.#active) return;
    this.#clearTimer();
    this.set(this.forFacts(this.#state.requestedIds, this.#state.snapshot));
  }
  private requirement(snapshot: CapabilitySnapshot | null, id: string): NativeRequirement | undefined {
    if (!snapshot || snapshot.connection !== "connected" || id === inputMonitoring || !knownRequirements.has(id)) return undefined;
    return snapshot.readiness.requirements.find(value => value.id === id);
  }
  private actionFor(row: NativeRequirement | undefined): RepairAction | null {
    if (!row) return null;
    if (row.status === "not-determined" && row.allowedRepairActions.includes("request-permission")) return "request-permission";
    if (row.status === "denied" && row.allowedRepairActions.includes("open-system-settings")) return "open-system-settings";
    if (row.status === "granted" && row.relaunchNeeded && row.allowedRepairActions.includes("request-safe-relaunch")) return "request-safe-relaunch";
    return null;
  }
  private scheduleClose(snapshot: CapabilitySnapshot): void {
    this.#clearTimer();
    const epoch = snapshot.providerEpoch, revision = snapshot.revision;
    this.#closeTimer = this.#timer.set(() => {
      this.#closeTimer = null;
      const current = this.#state.snapshot;
      if (this.#active && this.#state.phase === "ready" && current?.providerEpoch === epoch && current.revision === revision && this.forFacts(this.#state.requestedIds, current).phase === "ready") this.finish("ready");
    }, autoCloseMs);
  }
  private finish(reason: "close" | "later" | "ready"): void {
    if (this.#disposed || this.#state.phase === "closed") return;
    this.#active = false; this.#inFlight = null; this.#clearTimer();
    this.set({ ...this.#state, phase: "closed", pendingRequirementId: null, error: null, closeReason: reason });
  }
  #clearTimer(): void { if (this.#closeTimer !== null) { this.#timer.clear(this.#closeTimer); this.#closeTimer = null; } }
  private set(state: RepairState): RepairState { this.#state = Object.freeze({ ...state }); for (const listener of this.#listeners) listener(this.#state); return this.#state; }
}
