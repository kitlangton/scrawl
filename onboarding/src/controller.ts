import { hasConfirmedArtifactOpen, type CapabilitySnapshot, type PenColor, type RecordClient, type RecordCommand, type CommandResult, type ShortcutTrialReservationIdentity } from "@apparent/record/capabilities";
import { initialProgress, normalizeProgress, nextStage, type OnboardingProgress, type ProgressStore, type Revision, type SaveResult, type Stage } from "./model.js";

export type ControllerState = { progress: OnboardingProgress; revision: Revision; snapshot: CapabilitySnapshot | null; error: string | null; saving: boolean; publication: "confirmed" | "unknown"; command: "idle" | "accepted" | "applied" | "unknown" | "refused" };
export type Command = "continue" | "back" | "defer" | "reopen" | "replay" | "refresh" | "manual-paste-confirm" | "retry";
export type OnboardingDependencies = { progress: ProgressStore; record: RecordClient; id(): string };
const stages: Stage[] = ["welcome", "pen", "permissions", "shortcut", "practice-start", "screen", "whiteboard", "board-ink", "note", "screen-ink", "stop", "preparing", "copied", "paste", "locate", "complete"];
type Payload = RecordCommand extends infer C ? C extends RecordCommand ? Omit<C, "requestId" | "expected"> : never : never;
const successful = (snapshot: CapabilitySnapshot | null, captureId: string | null, kind: string, detail?: string) => {
  const capture = captureId ? snapshot?.captures.find(c => c.captureId === captureId) : undefined;
  return capture?.outcomes.some(o => o.kind === kind && o.ok && (detail === undefined || ("detail" in o && o.detail === detail))) ?? false;
};
const admissionCapture = (snapshot: CapabilitySnapshot | null, p: OnboardingProgress) => {
  const admission = snapshot?.nextShortcutAdmission;
  if (!snapshot || !p.admissionId || admission?.state !== "consumed" || admission.admissionId !== p.admissionId) return undefined;
  const capture = snapshot.captures.find(c => c.captureId === admission.captureId);
  return capture && !p.practiceBaselineIds.includes(capture.captureId) && capture.delivery === "local-only" && capture.shortcutAdmissionId === p.admissionId && successful(snapshot, capture.captureId, "shortcut") ? capture : undefined;
};

/** Onboarding owns lesson/progress policy; Record facts remain authoritative and capture-scoped. */
export class OnboardingController {
  #state: ControllerState = { progress: initialProgress(), revision: 0, snapshot: null, error: null, saving: false, publication: "confirmed", command: "idle" };
  #listeners = new Set<(state: ControllerState) => void>(); #unsubscribe: () => void; #disposed = false; #op = 0; #lastEpoch: string | null = null;
  #loaded = false;
  #confirmedPen: string | null = null;
  #penIntent: { color: PenColor; epoch: string; revision: number } | null = null;
  #reserving = false;
  #trialIntent: ShortcutTrialReservationIdentity | null = null;
  #trialProviderEpoch: string | null = null;
  #trialStarting = false;
  constructor(private readonly deps: OnboardingDependencies) {
    this.#unsubscribe = deps.record.observe(snapshot => this.observe(snapshot));
  }
  get state(): ControllerState { return this.#state; }
  subscribe(fn: (state: ControllerState) => void): () => void { this.#listeners.add(fn); return () => this.#listeners.delete(fn); }
  async load(): Promise<ControllerState> {
    if (this.#disposed || this.#state.saving) return this.#state;
    const op = ++this.#op;
    try {
      const snap = await this.deps.progress.read();
      if (this.#disposed || op !== this.#op) return this.#state;
      let progress = normalizeProgress(snap.value);
      if (progress.stage === "practice-start" && !progress.practiceBaselineIds.length) progress = { ...progress, practiceBaselineIds: this.#state.snapshot?.captures.map(c => c.captureId) ?? [] };
      this.#loaded = true;
      this.set({ ...this.#state, progress, revision: snap.revision, publication: "confirmed", error: null });
      if (this.#state.snapshot) this.advanceObservedFacts(this.#state.snapshot);
    } catch (error) {
      if (!this.#disposed && op === this.#op) this.set({ ...this.#state, publication: "unknown", error: error instanceof Error ? error.message : "Setup progress could not be loaded." });
    }
    return this.#state;
  }
  dispose(): void {
    const p = this.#state.progress, snapshot = this.#state.snapshot;
    if (this.#disposed) return;
    if (p.admissionId && snapshot) this.releaseOnDispose(p.admissionId, snapshot.providerEpoch);
    this.#disposed = true; this.#op++; this.#unsubscribe(); this.#listeners.clear();
  }
  /** Retain only reservation cleanup when an accepted arm outlives this view. */
  private releaseOnDispose(admissionId: string, epoch: string): void {
    let done = false, sent = false;
    let unsubscribe: (() => void) | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    const finish = () => { done = true; unsubscribe?.(); if (timer) clearTimeout(timer); };
    unsubscribe = this.deps.record.observe(snapshot => {
      const fact = snapshot.nextShortcutAdmission;
      if (snapshot.providerEpoch !== epoch) { finish(); return; }
      if (!("admissionId" in fact) || fact.admissionId !== admissionId) return;
      if (["consumed", "cancelled", "expired"].includes(fact.state)) { finish(); return; }
      if (snapshot.connection !== "connected" || fact.state !== "armed" || sent) return;
      sent = true;
      void this.deps.record.execute({ type: "cancel-next-shortcut", admissionId, requestId: this.deps.id(), expected: { providerEpoch: snapshot.providerEpoch, revision: snapshot.revision } }).then(result => {
        if (result.status === "refused" || result.status === "outcome-unknown") sent = false;
      }).catch(() => { sent = false; });
    });
    if (done) unsubscribe();
    else { timer = setTimeout(finish, 5 * 60_000); timer.unref?.(); }
  }
  async refresh(): Promise<CommandResult | null> { return this.execute({ type: "refresh" }); }
  async repair(type: "request-permission" | "open-system-settings", requirementId: string): Promise<CommandResult | null> { return this.execute({ type, requirementId }); }
  async safeRelaunch(): Promise<CommandResult | null> { return this.execute({ type: "request-safe-relaunch" }); }
  async beginShortcutEdit(): Promise<CommandResult | null> { return this.execute({ type: "shortcut-begin-edit" }); }
  async editShortcut(sessionId: string, value: string): Promise<CommandResult | null> { return this.execute({ type: "shortcut-submit", sessionId, value }); }
  async saveShortcut(sessionId: string): Promise<CommandResult | null> { return this.execute({ type: "shortcut-save", sessionId }); }
  async cancelShortcut(sessionId: string): Promise<CommandResult | null> { return this.execute({ type: "shortcut-cancel", sessionId }); }
  async openDesktopShortcutConfig(): Promise<CommandResult | null> { return this.execute({ type: "shortcut-open-desktop-config" }); }
  /** Reserve a non-recording trial for the exact current binding. Setup completes only on its matching observed press and release. */
  async beginShortcutTrial(): Promise<CommandResult | null> {
    if (this.#trialStarting) return null;
    this.#trialStarting = true;
    try {
    if (this.#state.progress.stage !== "shortcut" || this.#state.progress.setupCompleted) { this.fail("The physical shortcut check is only available during unfinished setup."); return null; }
    const s = this.#state.snapshot, binding = s?.shortcutTrialBeginBinding, old = s?.shortcutTrial;
    if (this.readiness(s) !== "ready") { this.fail("Required recording access is not ready. Review access before testing the shortcut."); return null; }
    if (!s || s.connection !== "connected" || !binding || !s.shortcut.effective || !s.shortcut.bindingId || s.shortcut.registration !== "registered" || s.shortcut.draft) { this.fail("A physical shortcut trial is not available yet. Confirm the shortcut and try again."); return null; }
    if (binding.bindingId !== s.shortcut.bindingId || binding.effectiveBinding !== s.shortcut.effective) { this.fail("The shortcut changed. Check its current status before trying the physical test."); return null; }
    if (old?.state === "idle" && (old.sourceEpoch !== binding.sourceEpoch || old.sourceRevision !== binding.sourceRevision || old.bindingId !== binding.bindingId || old.effectiveBinding !== binding.effectiveBinding)) { this.fail("The physical test reservation changed. Refresh setup before trying again."); return null; }
    let replaces: ShortcutTrialReservationIdentity | undefined;
    if (old?.state === "observed" || old?.state === "cancelled" || old?.state === "expired" || old?.state === "unknown") replaces = { reservationId: old.reservationId, sourceEpoch: old.sourceEpoch, sourceRevision: old.sourceRevision, bindingId: old.bindingId, effectiveBinding: old.effectiveBinding };
    else if (old?.state !== "idle" && old?.state !== undefined && old.state !== "unavailable") { this.fail("A shortcut test is already pending. Finish or cancel it before starting another."); return null; }
    const reservationId = this.deps.id();
    this.#trialIntent = { ...binding, reservationId };
    this.#trialProviderEpoch = s.providerEpoch;
    const result = await this.execute({ type: "begin-shortcut-trial", reservationId, ...binding, expiresAt: new Date(Date.now() + 30_000).toISOString(), ...(replaces ? { replaces } : {}) });
    const fact = this.#state.snapshot?.shortcutTrial;
    if (result?.status === "refused" || result?.status === "cancelled" || result?.status === "outcome-unknown" || fact?.state === "unavailable" || !fact || fact.state === "unknown") { this.#trialIntent = null; this.#trialProviderEpoch = null; }
    return result;
    } finally { this.#trialStarting = false; }
  }
  async cancelShortcutTrial(): Promise<CommandResult | null> {
    const fact = this.#state.snapshot?.shortcutTrial;
    if (!fact || (fact.state !== "reserved" && fact.state !== "pressing")) return null;
    const result = await this.execute({ type: "cancel-shortcut-trial", reservationId: fact.reservationId });
    if (result?.status === "outcome-unknown" || result?.status === "refused") return result;
    const current = this.#state.snapshot?.shortcutTrial;
    if (current && (current.state === "cancelled" || current.state === "expired" || current.state === "observed") && current.reservationId === fact.reservationId) { this.#trialIntent = null; this.#trialProviderEpoch = null; }
    return result;
  }
  private async reservePractice(): Promise<ControllerState> {
    if (this.#reserving) return this.#state;
    this.#reserving = true;
    try {
      const snapshot = this.#state.snapshot;
      if (!this.shortcutConfirmed(snapshot)) return this.fail("Your shortcut is not ready yet. Check the shortcut settings and try again.");
      if (this.#state.progress.admissionId && snapshot!.nextShortcutAdmission.state === "armed" && snapshot!.nextShortcutAdmission.admissionId === this.#state.progress.admissionId && snapshot!.nextShortcutAdmission.delivery === "local-only") {
        const admissionId = snapshot!.nextShortcutAdmission.admissionId;
        return this.persist({ ...this.#state.progress, status: "in-progress", stage: "practice-start", admissionId, attemptId: null, practiceBaselineIds: snapshot!.captures.map(c => c.captureId) });
      }
      if (snapshot!.nextShortcutAdmission.state !== "idle") return this.fail("A local-only shortcut reservation is not available. Refresh setup and try again.");
      const admissionId = this.deps.id(), expiresAt = new Date(Date.now() + 5 * 60_000).toISOString();
      // Persist the association before dispatch: acceptance can arrive before
      // the armed fact, or the person can press the shortcut immediately.
      await this.persist({ ...this.#state.progress, status: "in-progress", stage: "practice-start", admissionId, attemptId: null, practiceBaselineIds: snapshot!.captures.map(c => c.captureId) });
      if (this.#state.publication !== "confirmed" || this.#state.progress.admissionId !== admissionId) return this.#state;
      const result = await this.execute({ type: "arm-next-shortcut", admissionId, delivery: "local-only", expiresAt });
      const fact = this.#state.snapshot?.nextShortcutAdmission;
      if (result?.status === "accepted") return this.#state;
      if (result?.status === "refused" || result?.status === "cancelled") {
        await this.persist({ ...this.#state.progress, admissionId: null });
        return this.fail("The practice shortcut could not be prepared. Try again.");
      }
      if (!result || !["accepted", "applied"].includes(result.status) || fact?.state !== "armed" || fact.admissionId !== admissionId || fact.delivery !== "local-only") return this.fail("Apparent did not confirm a local-only shortcut reservation. Refresh before trying again.");
      return this.#state;
    } finally { this.#reserving = false; }
  }
  async retryPracticeCopy(): Promise<CommandResult | null> { const captureId = this.#state.progress.attemptId; if (!captureId || this.#state.progress.stage !== "preparing") return null; const fact = this.#state.snapshot?.captures.find(c => c.captureId === captureId); if (fact?.phase !== "saved") return null; return this.execute({ type: "copy-local", captureId }); }
  async selectPen(value: PenColor): Promise<boolean> {
    this.#confirmedPen = null;
    const before = this.#state.snapshot;
    this.#penIntent = before ? { color: value, epoch: before.providerEpoch, revision: before.revision } : null;
    const result = await this.execute({ type: "select-pen", value });
    if (result?.status === "accepted") return this.penSelectionConfirmed(value);
    if (result?.status !== "applied" || result.snapshot.pen.effective !== value) { this.fail(result?.status === "outcome-unknown" ? "Your pen choice may not have been saved. Check the current choice before trying again." : result?.status === "refused" ? result.reason : "Your pen choice could not be confirmed. Try again."); return false; }
    if (this.#disposed || this.#state.snapshot?.pen.effective !== value) return false;
    this.#confirmedPen = value;
    return true;
  }
  penSelectionConfirmed(value: PenColor): boolean { return this.#confirmedPen === value && this.#state.snapshot?.connection === "connected" && this.#state.snapshot.pen.effective === value; }
  async completePenSelection(): Promise<ControllerState> {
    if (this.#disposed || this.#state.saving || this.#state.publication !== "confirmed" || this.#state.command === "unknown" || this.#state.progress.status === "deferred" || this.#state.progress.stage !== "pen" || !this.#confirmedPen || this.#state.snapshot?.connection !== "connected" || this.#state.snapshot.pen.effective !== this.#confirmedPen) return this.#state;
    return this.persist({ ...this.#state.progress, status: "in-progress", stage: "permissions" });
  }
  async dispatch(command: Command): Promise<ControllerState> {
    if (this.#disposed || this.#state.saving) return this.#state;
    if (this.#state.publication === "unknown" && command !== "retry") return this.#state;
    if (this.#state.command === "unknown" && command !== "refresh") return this.#state;
    const p = this.#state.progress, snapshot = this.#state.snapshot;
    if (command === "refresh") { await this.refresh(); return this.#state; }
    if (["back", "defer", "reopen", "replay"].includes(command)) {
      const trial = snapshot?.shortcutTrial;
      if (this.#trialIntent && trial && (trial.state === "reserved" || trial.state === "pressing") && trial.reservationId === this.#trialIntent.reservationId) return this.fail("Cancel the pending shortcut test before leaving setup.");
      this.#trialIntent = null; this.#trialProviderEpoch = null;
    }
    if (command === "defer") {
      if (this.#reserving) return this.fail("Finishing your practice setup. Try closing it again in a moment.");
      const admission = snapshot?.nextShortcutAdmission;
      const terminal = admission && "admissionId" in admission && admission.admissionId === p.admissionId && ["consumed", "cancelled", "expired"].includes(admission.state);
      if (p.admissionId && !terminal) {
        const result = await this.execute({ type: "cancel-next-shortcut", admissionId: p.admissionId });
        const fact = this.#state.snapshot?.nextShortcutAdmission;
        if (!result || !fact || !("admissionId" in fact) || fact.admissionId !== p.admissionId || !["cancelled", "expired", "consumed"].includes(fact.state)) return this.fail("The pending shortcut reservation could not be released. Setup remains open; refresh and try again.");
      }
      return this.persist({ ...this.#state.progress, status: "deferred" });
    }
    if (command === "reopen" && p.setupCompleted) return this.#state;
    if (command === "reopen") return this.persist({ ...p, status: "in-progress", stage: p.status === "completed" ? "welcome" : p.stage, setupCompleted: p.setupCompleted, attemptId: p.status === "completed" ? null : p.attemptId, practiceBaselineIds: p.status === "completed" ? [] : p.practiceBaselineIds, manualPasteConfirmed: p.status === "completed" ? false : p.manualPasteConfirmed });
    if (command === "replay" && p.admissionId && !(snapshot?.nextShortcutAdmission && "admissionId" in snapshot.nextShortcutAdmission && snapshot.nextShortcutAdmission.admissionId === p.admissionId && ["consumed", "cancelled", "expired"].includes(snapshot.nextShortcutAdmission.state))) return this.fail("Finish or release the current practice reservation before replaying setup.");
    if (command === "replay") return this.persist({ ...p, status: "in-progress", stage: "welcome", setupCompleted: p.setupCompleted, attemptId: null, admissionId: null, practiceBaselineIds: [], manualPasteConfirmed: false });
    if (p.status === "deferred" || p.status === "completed") return this.#state;
    if (command === "manual-paste-confirm") { if (p.stage !== "paste") return this.#state; return this.persist({ ...p, manualPasteConfirmed: true, stage: "locate" }); }
    if (command === "retry") { if (this.#state.publication === "unknown") return this.load(); if (!this.#state.error) return this.#state; return this.persist(p); }
    if (command === "back") { if (stages.indexOf(p.stage) >= stages.indexOf("screen")) return this.#state; const i = stages.indexOf(p.stage); return this.persist({ ...p, status: "in-progress", stage: stages[Math.max(0, i - 1)] }); }
    if (command !== "continue") return this.#state;
    if (p.stage === "shortcut") {
      if (!p.setupCompleted) { await this.beginShortcutTrial(); return this.#state; }
      return this.reservePractice();
    }
    if (p.stage === "pen") return this.fail("Choose a pen and select Use this pen to continue.");
    if (p.stage === "permissions" && this.readiness(snapshot) !== "ready") return this.fail("Some required access is not ready yet. Allow it, then check access again.");
    if (p.stage === "practice-start") return p.admissionId ? this.fail("Waiting for your reserved shortcut to start a new local-only capture.") : this.reservePractice();
    if (["screen", "whiteboard", "board-ink", "note", "screen-ink"].includes(p.stage)) return this.fail("Follow the guide; confirmed capture actions advance this step automatically.");
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
      if (snapshot?.connection === "connected" && p.attemptId && hasConfirmedArtifactOpen(snapshot, p.attemptId)) return this.persist({ ...p, status: "completed", stage: "complete" });
      return this.fail("Open the saved capture from the Apparent menu to finish this step.");
    }
    if (p.stage === "complete") return this.#state;
    return this.persist({ ...p, status: "in-progress", stage: nextStage(p.stage) });
  }
  private readiness(snapshot: CapabilitySnapshot | null): "unknown" | "ready" | "blocked" {
    if (!snapshot || snapshot.connection !== "connected") return "unknown";
    const required = snapshot.readiness.requirements.filter(r => r.required);
    return required.length > 0 && required.every(r => r.usableNow && !r.relaunchNeeded) ? "ready" : required.some(r => !r.usableNow || r.relaunchNeeded) ? "blocked" : "unknown";
  }
  private shortcutConfirmed(snapshot: CapabilitySnapshot | null): boolean { return snapshot?.connection === "connected" && !!snapshot.shortcut.effective && !snapshot.shortcut.draft && ["registered", "externally-managed"].includes(snapshot.shortcut.registration); }
  private observe(snapshot: CapabilitySnapshot): void {
    if (this.#disposed) return;
    const intent = this.#penIntent;
    if (intent && snapshot.providerEpoch === intent.epoch && snapshot.revision > intent.revision && snapshot.pen.effective === intent.color && !snapshot.pen.error) this.#confirmedPen = intent.color;
    if (this.#lastEpoch && snapshot.providerEpoch !== this.#lastEpoch) {
      this.#penIntent = null; this.#confirmedPen = null; this.#trialIntent = null; this.#trialProviderEpoch = null;
      const p = this.#state.progress;
      const retained = !p.attemptId || snapshot.captures.some(c => c.captureId === p.attemptId);
      const resetPractice = !retained && stages.indexOf(p.stage) >= stages.indexOf("screen") && stages.indexOf(p.stage) < stages.indexOf("complete");
      this.set({ ...this.#state, snapshot, progress: resetPractice ? { ...p, stage: "practice-start", attemptId: null, admissionId: null, practiceBaselineIds: snapshot.captures.map(c => c.captureId), manualPasteConfirmed: false } : p, error: retained ? "Apparent reconnected. Check the current capture before continuing." : "The practice capture is no longer available. Start a new one to continue." });
    }
    else if (this.#state.snapshot && snapshot.providerEpoch === this.#state.snapshot.providerEpoch && snapshot.revision <= this.#state.snapshot.revision) return;
    else this.set({ ...this.#state, snapshot });
    this.#lastEpoch = snapshot.providerEpoch;
    this.advanceObservedFacts(snapshot);
  }
  private advanceObservedFacts(snapshot: CapabilitySnapshot): void {
    const p = this.#state.progress;
    const trial = snapshot.shortcutTrial, intent = this.#trialIntent;
    if (!this.#disposed && this.readiness(snapshot) === "ready" && p.stage === "shortcut" && p.status === "in-progress" && this.shortcutConfirmed(snapshot) && intent && trial?.state === "observed" && trial.reservationId === intent.reservationId && trial.sourceEpoch === intent.sourceEpoch && trial.sourceRevision === intent.sourceRevision && trial.bindingId === intent.bindingId && trial.effectiveBinding === intent.effectiveBinding && Number.isFinite(Date.parse(trial.pressedAt)) && Number.isFinite(Date.parse(trial.releasedAt)) && Date.parse(trial.releasedAt) >= Date.parse(trial.pressedAt) && snapshot.providerEpoch === this.#trialProviderEpoch && snapshot.connection === "connected" && snapshot.shortcut.bindingId === intent.bindingId && snapshot.shortcut.effective === intent.effectiveBinding && !p.setupCompleted && this.#loaded && this.#state.publication === "confirmed" && !this.#state.saving && this.#state.command !== "unknown") {
      this.#trialIntent = null; this.#trialProviderEpoch = null;
      void this.persist({ ...p, setupCompleted: true });
    }
    if (this.#disposed || !this.#loaded || p.status !== "in-progress" || this.#state.saving || this.#state.publication !== "confirmed" || this.#state.command === "unknown" || snapshot.connection !== "connected") return;
    const capture = p.attemptId ? snapshot.captures.find(c => c.captureId === p.attemptId) : undefined;
    if (p.stage !== "practice-start" && (!capture || capture.phase === "unknown")) return;
    if ((capture?.phase === "cancelled" || capture?.phase === "failed") && stages.indexOf(p.stage) >= stages.indexOf("screen") && stages.indexOf(p.stage) < stages.indexOf("complete")) {
      void this.persist({ ...p, status: "in-progress", stage: "practice-start", attemptId: null, admissionId: null, practiceBaselineIds: snapshot.captures.map(c => c.captureId), manualPasteConfirmed: false });
      this.set({ ...this.#state, error: "That capture ended before practice was finished. Start another with your shortcut." });
      return;
    }
    let stage = p.stage;
    if (stage === "practice-start") {
      const admission = snapshot.nextShortcutAdmission;
      if (p.admissionId && "admissionId" in admission && admission.admissionId === p.admissionId && ["expired", "cancelled"].includes(admission.state)) {
        void this.persist({ ...p, admissionId: null });
        return;
      }
      const admitted = admissionCapture(snapshot, p);
      if (admitted) {
        this.set({ ...this.#state, progress: { ...p, attemptId: admitted.captureId }, error: null });
        stage = "screen";
      }
    }
    const activeCapture = this.#state.progress.attemptId ? snapshot.captures.find(c => c.captureId === this.#state.progress.attemptId) : undefined;
    const outcomes = activeCapture?.outcomes ?? [];
    const pickedScreen = outcomes.some(o => o.kind === "picker" && o.ok && o.surface === "screen");
    const firstFrame = successful(snapshot, this.#state.progress.attemptId, "first-frame-start");
    const whiteboard = successful(snapshot, this.#state.progress.attemptId, "whiteboard");
    const boardStroke = outcomes.some(o => o.kind === "accepted-stroke" && o.ok && o.surface === "whiteboard");
    const note = outcomes.find(o => o.kind === "note-save" && o.ok);
    const screenStroke = note?.kind === "note-save" && note.ok && outcomes.some(o => o.kind === "accepted-stroke" && o.ok && o.surface === "screen" && o.cursor && o.cursor.historyId === note.screenStrokeBarrier.historyId && o.cursor.sequence > note.screenStrokeBarrier.sequence);
    if (stage === "screen" && pickedScreen && firstFrame) stage = "whiteboard";
    if (stage === "whiteboard" && whiteboard) stage = "board-ink";
    if (stage === "board-ink" && boardStroke) stage = "note";
    if (stage === "note" && note) stage = "screen-ink";
    if (stage === "screen-ink" && screenStroke) stage = "stop";
    const associatedId = this.#state.progress.attemptId;
    const associated = associatedId ? snapshot.captures.find(c => c.captureId === associatedId) : undefined;
    if (stage === "locate" && p.manualPasteConfirmed && associatedId && hasConfirmedArtifactOpen(snapshot, associatedId)) {
      void this.persist({ ...p, status: "completed", stage: "complete" });
      return;
    }
    if (stage === "stop" && successful(snapshot, associatedId, "processing") && ["processing", "saved"].includes(associated?.phase ?? "")) stage = "preparing";
    if (stage === "preparing" && associated?.phase === "saved" && successful(snapshot, associatedId, "save") && successful(snapshot, associatedId, "copy")) stage = "copied";
    if (stage !== p.stage) void this.persist({ ...this.#state.progress, status: "in-progress", stage });
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
    if (this.#disposed || !this.#loaded || this.#state.saving || this.#state.publication !== "confirmed") return this.#state;
    const op = ++this.#op; this.set({ ...this.#state, saving: true, error: null });
    try { const result: SaveResult = await this.deps.progress.compareAndSwap(this.#state.revision, value); if (this.#disposed || op !== this.#op) return this.#state;
      if (result.status === "saved") {
        this.set({ ...this.#state, progress: value, revision: result.revision, saving: false, publication: "confirmed", error: null });
        // Provider updates can arrive during the write. Reconcile the newest full
        // snapshot rather than waiting for another transient event.
        if (this.#state.snapshot) this.advanceObservedFacts(this.#state.snapshot);
      }
      else if (result.status === "conflict") this.set({ ...this.#state, progress: normalizeProgress(result.current), revision: result.revision, saving: false, error: "Setup changed elsewhere. The newer progress was reloaded; review it before continuing." });
      else this.set({ ...this.#state, saving: false, publication: "unknown", error: result.message || "Progress save outcome is unknown. Reload before retrying." });
    } catch (error) { if (op === this.#op) this.set({ ...this.#state, saving: false, publication: "unknown", error: error instanceof Error ? error.message : String(error) }); }
    return this.#state;
  }
  private fail(error: string): ControllerState { this.set({ ...this.#state, error }); return this.#state; }
  private set(state: ControllerState): void { if (this.#disposed) return; this.#state = Object.freeze({ ...state, progress: Object.freeze({ ...state.progress }) }); for (const fn of this.#listeners) { try { fn(this.#state); } catch { this.#listeners.delete(fn); } } }
}
