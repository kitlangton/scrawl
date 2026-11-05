import type { CapabilitySnapshot, CommandResult, NativeRequirement, RecordClient, RecordCommand, RepairAction } from "@apparent/record/capabilities";
import type { ProgressStore } from "../model.js";

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
export type PermissionRepairDependencies = { record: RecordClient; id(): string; timer?: RepairTimer; progress?: ProgressStore; now?(): number };

const knownRequirements = new Set(["screen-capture", "screen-recording", "microphone", "accessibility", "camera", "speech-recognition"]);
const inputMonitoring = "input-monitoring";
const autoCloseMs = 800;
const defaultTimer: RepairTimer = { set: (callback, delayMs) => setTimeout(callback, delayMs), clear: handle => clearTimeout(handle as ReturnType<typeof setTimeout>) };

/** One-screen repair policy; preserves tour milestones and dispatches only Record permission commands. */
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
  #reopenTimer: unknown = null;
  #relaunchAttempted = false;
  #generation = 0;
  #readyGeneration = 0;
  #requested = new Set<string>();

  constructor(private readonly deps: PermissionRepairDependencies) {
    this.#timer = deps.timer ?? defaultTimer;
    this.#unsubscribe = deps.record.observe(snapshot => this.observe(snapshot));
  }
  get state(): RepairState { return this.#state; }
  subscribe(listener: (state: RepairState) => void): () => void { this.#listeners.add(listener); return () => this.#listeners.delete(listener); }

  activate(requestedIds: readonly string[]): RepairState {
    if (this.#disposed) return this.#state;
    const unique = [...new Set(requestedIds)];
    if (this.#active && unique.length === this.#state.requestedIds.length && unique.every(id => this.#state.requestedIds.includes(id))) return this.#state;
    this.#active = true;
    this.#generation++;
    this.#requested.clear();
    this.#inFlight = null;
    this.#relaunchAttempted = false;
    this.clearReopenTimer();
    this.#clearTimer();
    if (unique.length === 0 || unique.some(id => id === inputMonitoring || !knownRequirements.has(id))) {
      this.#active = false;
      return this.set({ ...this.#state, phase: "error", requestedIds: unique, pendingRequirementId: null, error: "Access is unavailable right now." });
    }
    this.#inFlight = null;
    const snapshot = this.#state.snapshot;
    return this.set(this.forFacts(unique, snapshot));
  }

  async repair(requirementId: string): Promise<CommandResult | null> { return this.issueRepair(requirementId, false); }
  private async issueRepair(requirementId: string, automatic: boolean): Promise<CommandResult | null> {
    if (!this.#active || this.#disposed || this.#inFlight || this.#state.phase === "closed" || !this.#state.requestedIds.includes(requirementId)) return null;
    const snapshot = this.#state.snapshot;
    const requirement = this.requirement(snapshot, requirementId);
    const action = this.actionFor(requirement);
    if (!snapshot || snapshot.connection !== "connected" || !requirement || !action) return null;
    const requestId = this.deps.id();
    const epoch = snapshot.providerEpoch, revision = snapshot.revision;
    this.#inFlight = requestId;
    if (action === "request-safe-relaunch") { this.#relaunchAttempted = true; this.clearReopenTimer(); }
    this.set({ ...this.#state, phase: action === "request-safe-relaunch" ? "relaunch-needed" : "requesting", pendingRequirementId: requirementId, error: null });
    try {
      if (action === "request-safe-relaunch" && !await this.persistRelaunch(automatic)) { if (this.#inFlight === requestId) this.#inFlight = null; return null; }
      if (this.#disposed || !this.#active || this.#inFlight !== requestId || this.#state.snapshot?.providerEpoch !== epoch) return null;
      const latest = this.#state.snapshot;
      if (!this.#state.requestedIds.includes(requirementId) || this.actionFor(this.requirement(latest, requirementId)) !== action) { this.#inFlight = null; this.reconcile(); return null; }
      const expected = { providerEpoch: epoch, revision: latest.revision };
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
      if (action === "request-permission" && ["accepted", "applied"].includes(result.status)) this.#requested.add(requirementId);
      if (resultSnapshot.revision < revision || resultSnapshot.revision < current.revision) { this.reconcile(); return result; }
      if (resultSnapshot.revision >= (current?.revision ?? -1)) this.observe(resultSnapshot);
      if (result.status === "accepted") { this.reconcile(); if (!["ready", "relaunch-needed"].includes(this.#state.phase)) this.set({ ...this.#state, phase: "waiting", pendingRequirementId: requirementId, error: null }); }
      else if ((result.status === "refused" || result.status === "outcome-unknown") && this.#state.phase !== "ready") this.set({ ...this.#state, phase: "error", pendingRequirementId: null, error: "Access could not be updated. Check again and try once more." });
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
      if (result.snapshot.revision < revision || result.snapshot.revision < current.revision) { this.reconcile(); return result; }
      this.observe(result.snapshot);
      if ((result.status === "refused" || result.status === "outcome-unknown") && this.#state.phase !== "ready") this.set({ ...this.#state, phase: "error", error: "Access status could not be refreshed. Try again." });
      else { this.reconcile(); if (result.status === "accepted" && !["ready", "relaunch-needed"].includes(this.#state.phase)) this.set({ ...this.#state, phase: "waiting", error: null }); }
      return result;
    } catch {
      if (!this.#disposed && this.#inFlight === requestId) { this.#inFlight = null; this.set({ ...this.#state, phase: "error", error: "Access status could not be refreshed. Try again." }); }
      return null;
    }
  }
  close(): void { this.finish("close"); }
  cancel(): void { this.finish("later"); }
  dispose(): void { if (this.#disposed) return; this.#disposed = true; this.#active = false; this.#inFlight = null; this.#clearTimer(); this.clearReopenTimer(); this.#unsubscribe(); this.#listeners.clear(); }

  private observe(snapshot: CapabilitySnapshot): void {
    if (this.#disposed) return;
    if (snapshot.providerEpoch === this.#epoch && snapshot.revision <= this.#revision) return;
    const newEpoch = snapshot.providerEpoch !== this.#epoch;
    this.#epoch = snapshot.providerEpoch;
    this.#revision = snapshot.revision;
    if (!this.#active || this.#state.phase === "closed") { this.set({ ...this.#state, snapshot }); return; }
    if (newEpoch) { this.#inFlight = null; this.#clearTimer(); this.clearReopenTimer(); }
    this.set(this.forFacts(this.#state.requestedIds, snapshot));
  }

  private forFacts(requestedIds: readonly string[], snapshot: CapabilitySnapshot | null): RepairState {
    if (!requestedIds.length) { this.#clearTimer(); return { phase: "error", requestedIds, snapshot, pendingRequirementId: null, error: "No access request was supplied." }; }
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
    this.set(this.forFacts(this.#state.requestedIds, this.#state.snapshot));
  }
  private requirement(snapshot: CapabilitySnapshot | null, id: string): NativeRequirement | undefined {
    if (!snapshot || snapshot.connection !== "connected" || id === inputMonitoring || !knownRequirements.has(id)) return undefined;
    return snapshot.readiness.requirements.find(value => value.id === id);
  }
  private actionFor(row: NativeRequirement | undefined): RepairAction | null {
    if (!row) return null;
    if (row.status === "not-determined" && this.#requested.has(row.id) && row.allowedRepairActions.includes("open-system-settings")) return "open-system-settings";
    if (row.status === "not-determined" && !this.#requested.has(row.id) && row.allowedRepairActions.includes("request-permission")) return "request-permission";
    if (row.status === "denied" && row.allowedRepairActions.includes("open-system-settings")) return "open-system-settings";
    if (row.status === "granted" && row.relaunchNeeded && row.allowedRepairActions.includes("request-safe-relaunch")) return "request-safe-relaunch";
    return null;
  }
  availableAction(id: string): RepairAction | null { return this.#active && this.#state.requestedIds.includes(id) ? this.actionFor(this.requirement(this.#state.snapshot, id)) : null; }
  private scheduleClose(snapshot: CapabilitySnapshot): void {
    if (this.#closeTimer !== null) return;
    const epoch = snapshot.providerEpoch;
    const generation = this.#readyGeneration;
    this.#closeTimer = this.#timer.set(() => {
      if (generation !== this.#readyGeneration) return;
      this.#closeTimer = null;
      const current = this.#state.snapshot;
      const rows = this.#state.requestedIds.map(id => this.requirement(current, id));
      if (this.#active && this.#state.phase === "ready" && current?.providerEpoch === epoch && rows.length > 0 && rows.every(row => row?.status === "granted" && row.usableNow && !row.relaunchNeeded)) this.finish("ready");
    }, autoCloseMs);
  }
  private finish(reason: "close" | "later" | "ready"): void {
    if (this.#disposed || this.#state.phase === "closed") return;
    this.#active = false; this.#inFlight = null; this.#clearTimer();
    this.#generation++;
    this.clearReopenTimer();
    this.set({ ...this.#state, phase: "closed", pendingRequirementId: null, error: null, closeReason: reason });
  }
  #clearTimer(): void { this.#readyGeneration++; if (this.#closeTimer !== null) { this.#timer.clear(this.#closeTimer); this.#closeTimer = null; } }
  private async persistRelaunch(automatic: boolean): Promise<boolean> {
    if (!this.deps.progress) { this.set({ ...this.#state, phase: "error", pendingRequirementId: null, error: "Apparent cannot save its restart state yet." }); return false; }
    const request = this.#inFlight;
    const before = await this.deps.progress.read();
    if (this.#disposed || !this.#active || this.#inFlight !== request) return false;
    const now = this.deps.now?.() ?? Date.now();
    const previous = before.value.legacy.relaunchedAt;
    if (!Number.isFinite(now) || (automatic && previous !== null && now - previous < 120_000)) {
      this.set({ ...this.#state, phase: "relaunch-needed", pendingRequirementId: null, error: "Apparent recently reopened. Choose Reopen Apparent if access is still unavailable." });
      return false;
    }
    const result = await this.deps.progress.compareAndSwap(before.revision, { ...before.value, legacy: { ...before.value.legacy, relaunchedAt: now, reopenFix: true } });
    if (this.#disposed || !this.#active || this.#inFlight !== request) return false;
    if (result.status !== "saved") { this.set({ ...this.#state, phase: "error", pendingRequirementId: null, error: "Restart state could not be confirmed. Check again before reopening." }); return false; }
    return true;
  }
  private clearReopenTimer(): void { if (this.#reopenTimer !== null) { this.#timer.clear(this.#reopenTimer); this.#reopenTimer = null; } }
  private scheduleReopen(): void {
    if (!this.#active || this.#disposed || this.#state.phase !== "relaunch-needed") { this.clearReopenTimer(); return; }
    if (this.#inFlight || this.#relaunchAttempted || this.#reopenTimer !== null || !this.deps.progress) return;
    const row = this.#state.requestedIds.map(id => this.requirement(this.#state.snapshot, id)).find(value => value && ["screen-recording", "screen-capture"].includes(value.id) && value.status === "granted" && value.relaunchNeeded && value.allowedRepairActions.includes("request-safe-relaunch"));
    if (!row) return;
    const generation = this.#generation, epoch = this.#state.snapshot?.providerEpoch;
    this.#reopenTimer = this.#timer.set(() => {
      if (generation !== this.#generation || this.#state.snapshot?.providerEpoch !== epoch || this.#state.phase !== "relaunch-needed" || this.availableAction(row.id) !== "request-safe-relaunch") return;
      this.#reopenTimer = null; void this.issueRepair(row.id, true);
    }, 1_000);
  }
  private set(state: RepairState): RepairState { if (this.#disposed) return this.#state; this.#state = Object.freeze({ ...state }); for (const listener of this.#listeners) { try { listener(this.#state); } catch { this.#listeners.delete(listener); } } this.scheduleReopen(); return this.#state; }
}
