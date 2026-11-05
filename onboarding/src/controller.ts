import { hasConfirmedArtifactOpen, type CapabilitySnapshot, type RecordClient, type RecordCommand, type CommandResult } from "@apparent/record/capabilities";
import { initialProgress, normalizeProgress, nextStage, type OnboardingProgress, type ProgressStore, type Revision, type SaveResult, type Stage } from "./model.js";

export type ControllerState = { progress: OnboardingProgress; revision: Revision; snapshot: CapabilitySnapshot | null; error: string | null; saving: boolean; publication: "confirmed" | "unknown"; command: "idle" | "accepted" | "applied" | "unknown" | "refused" };
export type Command = "continue" | "back" | "defer" | "reopen" | "refresh" | "manual-paste-confirm" | "retry";
export type OnboardingDependencies = { progress: ProgressStore; record: RecordClient; id(): string };
const stages: Stage[] = ["welcome", "pen", "permissions", "shortcut", "practice-start", "screen", "whiteboard", "board-ink", "screen-ink", "stop", "preparing", "copied", "paste", "locate", "complete"];
type Payload = RecordCommand extends infer C ? C extends RecordCommand ? Omit<C, "requestId" | "expected"> : never : never;
const successful = (snapshot: CapabilitySnapshot | null, captureId: string | null, kind: string, detail?: string) => {
  const capture = captureId ? snapshot?.captures.find(c => c.captureId === captureId) : undefined;
  return capture?.outcomes.some(o => o.kind === kind && o.ok && (detail === undefined || ("detail" in o && o.detail === detail))) ?? false;
};
const eligiblePracticeCapture = (snapshot: CapabilitySnapshot | null, baseline: string[]) => snapshot?.captures.find(c => !baseline.includes(c.captureId) && c.phase === "recording" && successful(snapshot, c.captureId, "shortcut") && successful(snapshot, c.captureId, "first-frame-start"));

/** Onboarding owns lesson/progress policy; Record facts remain authoritative and capture-scoped. */
export class OnboardingController {
  #state: ControllerState = { progress: initialProgress(), revision: 0, snapshot: null, error: null, saving: false, publication: "confirmed", command: "idle" };
  #listeners = new Set<(state: ControllerState) => void>(); #unsubscribe: () => void; #disposed = false; #op = 0; #lastEpoch: string | null = null;
  constructor(private readonly deps: OnboardingDependencies) {
    this.#unsubscribe = deps.record.observe(snapshot => this.observe(snapshot));
  }
  get state(): ControllerState { return this.#state; }
  subscribe(fn: (state: ControllerState) => void): () => void { this.#listeners.add(fn); return () => this.#listeners.delete(fn); }
  async load(): Promise<ControllerState> { const op = ++this.#op; try { const snap = await this.deps.progress.read(); if (this.#disposed || op !== this.#op) return this.#state; let progress = normalizeProgress(snap.value); if (progress.stage === "practice-start" && !progress.practiceBaselineIds.length) progress = { ...progress, practiceBaselineIds: this.#state.snapshot?.captures.map(c => c.captureId) ?? [] }; this.set({ ...this.#state, progress, revision: snap.revision, publication: "confirmed", error: null }); } catch (error) { if (!this.#disposed && op === this.#op) this.fail(error instanceof Error ? error.message : "Setup progress could not be loaded."); } return this.#state; }
  dispose(): void { this.#disposed = true; this.#op++; this.#unsubscribe(); this.#listeners.clear(); }
  async refresh(): Promise<CommandResult | null> { return this.execute({ type: "refresh" }); }
  async repair(type: "request-permission" | "open-system-settings", requirementId: string): Promise<CommandResult | null> { return this.execute({ type, requirementId }); }
  async safeRelaunch(): Promise<CommandResult | null> { return this.execute({ type: "request-safe-relaunch" }); }
  async beginShortcutEdit(): Promise<CommandResult | null> { return this.execute({ type: "shortcut-begin-edit" }); }
  async editShortcut(sessionId: string, value: string): Promise<CommandResult | null> { return this.execute({ type: "shortcut-submit", sessionId, value }); }
  async saveShortcut(sessionId: string): Promise<CommandResult | null> { return this.execute({ type: "shortcut-save", sessionId }); }
  async cancelShortcut(sessionId: string): Promise<CommandResult | null> { return this.execute({ type: "shortcut-cancel", sessionId }); }
  async openDesktopShortcutConfig(): Promise<CommandResult | null> { return this.execute({ type: "shortcut-open-desktop-config" }); }
  async retryPracticeCopy(): Promise<CommandResult | null> { const captureId = this.#state.progress.attemptId; if (!captureId || this.#state.progress.stage !== "preparing") return null; const fact = this.#state.snapshot?.captures.find(c => c.captureId === captureId); if (fact?.phase !== "saved") return null; return this.execute({ type: "copy-local", captureId }); }
  async selectPen(value: string): Promise<boolean> {
    const result = await this.execute({ type: "select-pen", value });
    if (result?.status !== "applied" || result.snapshot.pen.effective !== value) { this.fail(result?.status === "outcome-unknown" ? "Your pen choice may not have been saved. Check the current choice before trying again." : result?.status === "refused" ? result.reason : "Your pen choice could not be confirmed. Try again."); return false; }
    return true;
  }
  async completePenSelection(): Promise<ControllerState> {
    if (this.#state.progress.stage !== "pen" || !this.#state.snapshot?.pen.effective) return this.#state;
    return this.persist({ ...this.#state.progress, status: "in-progress", stage: "permissions" });
  }
  async dispatch(command: Command): Promise<ControllerState> {
    if (this.#disposed || this.#state.saving) return this.#state;
    if (this.#state.publication === "unknown" && command !== "retry") return this.#state;
    if (this.#state.command === "unknown" && command !== "refresh") return this.#state;
    const p = this.#state.progress, snapshot = this.#state.snapshot;
    if (command === "refresh") { await this.refresh(); return this.#state; }
    if (command === "defer") return this.persist({ ...p, status: "deferred" });
    if (command === "reopen") return this.persist({ ...p, status: "in-progress", stage: p.status === "completed" ? "welcome" : p.stage, attemptId: p.status === "completed" ? null : p.attemptId, practiceBaselineIds: p.status === "completed" ? [] : p.practiceBaselineIds, manualPasteConfirmed: p.status === "completed" ? false : p.manualPasteConfirmed });
    if (command === "manual-paste-confirm") { if (p.stage !== "paste") return this.#state; return this.persist({ ...p, manualPasteConfirmed: true, stage: "locate" }); }
    if (command === "retry") { if (this.#state.publication === "unknown") return this.load(); if (!this.#state.error) return this.#state; return this.persist(p); }
    if (command === "back") { if (stages.indexOf(p.stage) >= stages.indexOf("screen")) return this.#state; const i = stages.indexOf(p.stage); return this.persist({ ...p, status: "in-progress", stage: stages[Math.max(0, i - 1)] }); }
    if (command !== "continue") return this.#state;
    if (p.stage === "permissions" && this.readiness(snapshot) !== "ready") return this.fail("Some required access is not ready yet. Allow it, then check access again.");
    if (p.stage === "shortcut" && !this.shortcutConfirmed(snapshot)) return this.fail("Your shortcut is not ready yet. Check the shortcut settings and try again.");
    if (p.stage === "practice-start") {
      if (!snapshot || snapshot.connection !== "connected") return this.fail("Reconnect to Apparent before continuing the practice.");
      const capture = eligiblePracticeCapture(snapshot, p.practiceBaselineIds);
      if (!capture) return this.fail("Waiting for a new shortcut-started capture. Start it with your shortcut, then check again.");
      // The packed Record contract has no authoritative local-only admission fact yet.
      return this.fail("Practice will continue when the capture is confirmed to stay on this device.");
    }
    if (p.stage === "screen") return this.fail("Waiting for Screen selection to be confirmed for this capture.");
    if (p.stage === "whiteboard" && !successful(snapshot, p.attemptId, "whiteboard")) return this.fail("Waiting for Whiteboard selection on this capture.");
    if (p.stage === "board-ink" || p.stage === "screen-ink") return this.fail("Waiting for the drawing to be confirmed on the selected surface.");
    if (p.stage === "stop" && !(successful(snapshot, p.attemptId, "processing") && ["processing", "saved"].includes(snapshot?.captures.find(c => c.captureId === p.attemptId)?.phase ?? ""))) return this.fail("Waiting for the capture to finish.");
    if (p.stage === "preparing") {
      const saved = successful(snapshot, p.attemptId, "save") && snapshot?.captures.find(c => c.captureId === p.attemptId)?.phase === "saved";
      if (!saved) return this.fail("Your capture is still being prepared. Check again in a moment.");
      if (!successful(snapshot, p.attemptId, "copy")) return this.fail("Your capture is saved, but the copy did not finish. Retry the copy from the capture controls.");
      return this.persist({ ...p, status: "in-progress", stage: "copied" });
    }
    if (p.stage === "copied" && !successful(snapshot, p.attemptId, "copy")) return this.fail("Your capture is saved, but the copy did not finish. Retry from the capture controls.");
    if (p.stage === "locate") {
      if (!p.manualPasteConfirmed) return this.fail("Confirm the paste yourself before locating the practice capture.");
      if (snapshot && p.attemptId && hasConfirmedArtifactOpen(snapshot, p.attemptId)) return this.persist({ ...p, status: "completed", stage: "complete" });
      return this.fail("Open the saved capture from the Apparent menu to finish this step.");
    }
    if (p.stage === "complete") return this.#state;
    return this.persist({ ...p, status: "in-progress", stage: nextStage(p.stage), ...(p.stage === "shortcut" ? { practiceBaselineIds: snapshot?.captures.map(c => c.captureId) ?? [] } : {}) });
  }
  private readiness(snapshot: CapabilitySnapshot | null): "unknown" | "ready" | "blocked" {
    if (!snapshot || snapshot.connection !== "connected") return "unknown";
    const required = snapshot.readiness.requirements.filter(r => r.required);
    return required.length > 0 && required.every(r => r.usableNow && !r.relaunchNeeded) ? "ready" : required.some(r => !r.usableNow || r.relaunchNeeded) ? "blocked" : "unknown";
  }
  private shortcutConfirmed(snapshot: CapabilitySnapshot | null): boolean { return !!snapshot?.shortcut.effective && !snapshot.shortcut.draft && snapshot.shortcut.registration === "registered"; }
  private observe(snapshot: CapabilitySnapshot): void {
    if (this.#disposed) return;
    if (this.#lastEpoch && snapshot.providerEpoch !== this.#lastEpoch) {
      const p = this.#state.progress;
      const retained = !p.attemptId || snapshot.captures.some(c => c.captureId === p.attemptId);
      const resetPractice = !retained && stages.indexOf(p.stage) >= stages.indexOf("screen") && stages.indexOf(p.stage) < stages.indexOf("complete");
      this.set({ ...this.#state, snapshot, progress: resetPractice ? { ...p, stage: "practice-start", attemptId: null, practiceBaselineIds: snapshot.captures.map(c => c.captureId), manualPasteConfirmed: false } : p, error: retained ? "Apparent reconnected. Check the current capture before continuing." : "The practice capture is no longer available. Start a new one to continue." });
    }
    else if (this.#state.snapshot && snapshot.providerEpoch === this.#state.snapshot.providerEpoch && snapshot.revision <= this.#state.snapshot.revision) return;
    else this.set({ ...this.#state, snapshot });
    this.#lastEpoch = snapshot.providerEpoch;
    this.advanceObservedFacts(snapshot);
  }
  private advanceObservedFacts(snapshot: CapabilitySnapshot): void {
    const p = this.#state.progress;
    if (this.#disposed || this.#state.saving || this.#state.publication !== "confirmed" || this.#state.command === "unknown" || snapshot.connection !== "connected") return;
    const capture = p.attemptId ? snapshot.captures.find(c => c.captureId === p.attemptId) : undefined;
    if (!capture || capture.phase === "cancelled" || capture.phase === "failed" || capture.phase === "unknown") return;
    let stage = p.stage;
    if (stage === "whiteboard" && successful(snapshot, p.attemptId, "whiteboard")) stage = "board-ink";
    if (stage === "stop" && successful(snapshot, p.attemptId, "processing") && ["processing", "saved"].includes(capture.phase)) stage = "preparing";
    if (stage === "preparing" && capture.phase === "saved" && successful(snapshot, p.attemptId, "save") && successful(snapshot, p.attemptId, "copy")) stage = "copied";
    if (stage !== p.stage) void this.persist({ ...p, status: "in-progress", stage });
  }
  private async execute(payload: Payload): Promise<CommandResult | null> {
    const snapshot = this.#state.snapshot;
    if ((this.#state.publication === "unknown" && payload.type !== "refresh") || (this.#state.command === "unknown" && payload.type !== "refresh")) return null;
    if (this.#disposed || !snapshot || snapshot.connection !== "connected") { this.fail("Record is disconnected; refresh before issuing setup commands."); return null; }
    const command = { ...payload, requestId: this.deps.id(), expected: { providerEpoch: snapshot.providerEpoch, revision: snapshot.revision } } as RecordCommand;
    try {
      const result = await this.deps.record.execute(command);
      if (result.requestId !== command.requestId) { this.set({ ...this.#state, command: "unknown", error: "Refresh setup before continuing." }); return null; }
      if (result.snapshot.providerEpoch !== command.expected.providerEpoch || this.#state.snapshot?.providerEpoch !== command.expected.providerEpoch || result.snapshot.connection !== "connected" || result.snapshot.revision < command.expected.revision) { this.set({ ...this.#state, command: "unknown", error: "Refresh setup before continuing." }); return null; }
      if (result.snapshot.providerEpoch === this.#state.snapshot?.providerEpoch && result.snapshot.revision < this.#state.snapshot.revision) { this.set({ ...this.#state, command: "unknown", error: "Refresh setup before continuing." }); return null; }
      if (result.snapshot.providerEpoch === this.#state.snapshot?.providerEpoch && result.snapshot.revision >= (this.#state.snapshot?.revision ?? -1)) this.observe(result.snapshot);
      const state = result.status === "applied" ? "applied" : result.status === "accepted" ? "accepted" : result.status === "outcome-unknown" ? "unknown" : result.status === "refused" ? "refused" : "idle";
      this.set({ ...this.#state, command: state, error: result.status === "refused" ? result.reason : result.status === "outcome-unknown" ? "Refresh setup before continuing." : null });
      return result;
    } catch (error) { this.set({ ...this.#state, command: "unknown", error: error instanceof Error ? error.message : String(error) }); return null; }
  }
  private async persist(value: OnboardingProgress): Promise<ControllerState> {
    const op = ++this.#op; this.set({ ...this.#state, saving: true, error: null });
    try { const result: SaveResult = await this.deps.progress.compareAndSwap(this.#state.revision, value); if (this.#disposed || op !== this.#op) return this.#state;
      if (result.status === "saved") this.set({ ...this.#state, progress: value, revision: result.revision, saving: false, publication: "confirmed", error: null });
      else if (result.status === "conflict") this.set({ ...this.#state, progress: normalizeProgress(result.current), revision: result.revision, saving: false, error: "Setup changed elsewhere. The newer progress was reloaded; review it before continuing." });
      else this.set({ ...this.#state, saving: false, publication: "unknown", error: result.message || "Progress save outcome is unknown. Reload before retrying." });
    } catch (error) { if (op === this.#op) this.set({ ...this.#state, saving: false, publication: "unknown", error: error instanceof Error ? error.message : String(error) }); }
    return this.#state;
  }
  private fail(error: string): ControllerState { this.set({ ...this.#state, error }); return this.#state; }
  private set(state: ControllerState): void { if (this.#disposed) return; this.#state = Object.freeze({ ...state, progress: Object.freeze({ ...state.progress }) }); for (const fn of this.#listeners) { try { fn(this.#state); } catch { this.#listeners.delete(fn); } } }
}
