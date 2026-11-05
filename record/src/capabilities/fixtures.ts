import type { CapabilitySnapshot, CommandResult, RecordClient, RecordCommand } from "./index.js";

export const fixtureScenarios = [
  "permission-granted-relaunch-needed", "external-shortcut-save-failure",
  "pen-write-failure-preserves-effective", "unrelated-capture",
  "reconnect-with-saved-history", "ambiguous-command-preserves-capture",
  "unsubscribe-during-recording",
] as const;
export type FixtureScenario = (typeof fixtureScenarios)[number];
type ScriptedResult = "accepted" | "refused" | "cancelled" | "outcome-unknown";
export interface FixtureDriver {
  /** Same-epoch, strictly increasing complete provider facts. Stale frames are ignored. */
  publish(snapshot: CapabilitySnapshot): boolean;
  disconnect(): void;
  /** A changed epoch is explicit; retired epochs can never become current again. */
  reconnect(snapshot?: CapabilitySnapshot): boolean;
  setNextResult(result: ScriptedResult): void;
  current(): CapabilitySnapshot;
  commands(): RecordCommand[];
  captureContinues(): boolean;
}
export interface RecordFixture { client: RecordClient; driver: FixtureDriver }
const clone = <T>(value: T): T => structuredClone(value);

function initial(scenario: FixtureScenario): CapabilitySnapshot {
  const snapshot: CapabilitySnapshot = {
    version: 2, providerEpoch: "fixture-epoch-1", revision: 1, connection: "connected",
    readiness: { requirements: [{ id: "screen-capture", required: true, status: "granted", usableNow: true, relaunchNeeded: false, allowedRepairActions: [] }] },
    shortcut: { effective: "Ctrl+Shift+R", label: "Ctrl Shift R", registration: "registered", externallyManaged: false, draft: null },
    pen: { effective: "ink", supported: ["ink", "laser", "cursor"] }, captures: [],
    // No current production adapter owns an armed-shortcut reservation.
    nextShortcutAdmission: { state: "unavailable" },
  };
  if (scenario === "permission-granted-relaunch-needed") snapshot.readiness.requirements[0] = {
    id: "screen-capture", required: true, status: "granted", usableNow: false, relaunchNeeded: true, allowedRepairActions: ["request-safe-relaunch"],
  };
  if (scenario === "external-shortcut-save-failure") snapshot.shortcut = { ...snapshot.shortcut, effective: null, label: null, registration: "externally-managed", externallyManaged: true };
  if (scenario === "pen-write-failure-preserves-effective") snapshot.pen.error = "fixture write failure";
  if (scenario === "unrelated-capture") snapshot.captures = [{ captureId: "capture-unrelated", delivery: "unknown", phase: "recording", outcomes: [{ kind: "first-frame-start", at: "2025-01-01T00:00:00.000Z", ok: true }] }];
  if (scenario === "reconnect-with-saved-history") snapshot.captures = [{ captureId: "capture-saved", delivery: "unknown", phase: "saved", outcomes: [{ kind: "save", at: "2025-01-01T00:00:01.000Z", ok: true }] }];
  if (scenario === "ambiguous-command-preserves-capture" || scenario === "unsubscribe-during-recording") snapshot.captures = [{ captureId: "capture-active", delivery: "unknown", phase: "recording", outcomes: [{ kind: "first-frame-start", at: "2025-01-01T00:00:00.000Z", ok: true }] }];
  return snapshot;
}

/** Synthetic dispatch + explicit provider-fact driver. Never an application fallback. */
export function createRecordFixture(scenario: FixtureScenario = "permission-granted-relaunch-needed"): RecordFixture {
  let snapshot = initial(scenario);
  let next: ScriptedResult = scenario === "ambiguous-command-preserves-capture" ? "outcome-unknown" : "accepted";
  let detached = false;
  const retiredEpochs = new Set<string>();
  const commands: RecordCommand[] = [];
  const observers = new Set<(snapshot: CapabilitySnapshot) => void>();
  const notify = (listener: (snapshot: CapabilitySnapshot) => void, value: CapabilitySnapshot) => {
    if (!observers.has(listener)) return;
    try { listener(clone(value)); } catch { observers.delete(listener); }
  };
  // Queue reentrant publications so every observer sees revisions in the same order.
  const pending: { frame: CapabilitySnapshot; listeners: ((snapshot: CapabilitySnapshot) => void)[] }[] = [];
  let emitting = false;
  const emit = () => {
    pending.push({ frame: clone(snapshot), listeners: [...observers] });
    if (emitting) return;
    emitting = true;
    try {
      while (pending.length) {
        const { frame, listeners } = pending.shift()!;
        for (const listener of listeners) notify(listener, frame);
      }
    } finally { emitting = false; }
  };
  const valid = (value: CapabilitySnapshot) => value.version === 2 && !!value.providerEpoch && Number.isSafeInteger(value.revision) && value.revision >= 0;
  const publish = (value: CapabilitySnapshot): boolean => {
    if (!valid(value) || value.providerEpoch !== snapshot.providerEpoch || value.revision <= snapshot.revision || value.connection !== snapshot.connection) return false;
    snapshot = clone(value); emit(); return true;
  };
  const refuse = (command: RecordCommand, reason: string): CommandResult => ({ status: "refused", requestId: command.requestId, reason, snapshot: clone(snapshot) });
  const admissionFailure = (command: RecordCommand): string | undefined => {
    if (snapshot.connection !== "connected") return "provider disconnected";
    if (command.expected.providerEpoch !== snapshot.providerEpoch || command.expected.revision !== snapshot.revision) return "stale provider epoch or revision";
    const active = snapshot.captures.some(c => ["starting", "recording", "processing", "unknown"].includes(c.phase));
    if (command.type === "start-capture") {
      if (active) return "capture already active or unresolved";
      if (snapshot.readiness.requirements.some(r => r.required && (!r.usableNow || r.relaunchNeeded))) return "capture requirements not usable";
    }
    if (command.type === "arm-next-shortcut" && snapshot.nextShortcutAdmission.state !== "idle") return "next-shortcut admission is unavailable or not idle";
    if (command.type === "cancel-next-shortcut" && (snapshot.nextShortcutAdmission.state !== "armed" || snapshot.nextShortcutAdmission.admissionId !== command.admissionId)) return "matching next-shortcut admission is not armed";
    if (command.type === "request-permission" || command.type === "open-system-settings") {
      const requirement = snapshot.readiness.requirements.find(r => r.id === command.requirementId);
      if (!requirement?.allowedRepairActions.includes(command.type)) return "repair action unavailable";
    }
    if (command.type === "request-safe-relaunch" && (active || !snapshot.readiness.requirements.some(r => r.allowedRepairActions.includes("request-safe-relaunch")))) return "safe relaunch unavailable";
    if (["shortcut-begin-edit", "shortcut-submit", "shortcut-save", "shortcut-cancel", "shortcut-reset"].includes(command.type) && snapshot.shortcut.externallyManaged) return "shortcut is externally managed";
    if (command.type === "shortcut-submit" || command.type === "shortcut-save" || command.type === "shortcut-cancel") {
      if (snapshot.shortcut.draft?.sessionId !== command.sessionId) return "shortcut edit session not current";
    }
    if (command.type === "select-pen") {
      if (!snapshot.pen.supported.includes(command.value)) return "unsupported pen";
      if (scenario === "pen-write-failure-preserves-effective") return "pen preference write failed";
    }
    if ("captureId" in command) {
      const capture = snapshot.captures.find(c => c.captureId === command.captureId);
      if (!capture) return "unknown capture";
      if ((command.type === "copy-local" || command.type === "open-capture") && capture.phase !== "saved") return "capture is not saved";
      if ((command.type === "stop-capture" || command.type === "cancel-capture") && !["starting", "recording", "unknown"].includes(capture.phase)) return "capture cannot be controlled in this phase";
    }
  };
  const client: RecordClient = {
    observe(listener) {
      if (!detached) { observers.add(listener); notify(listener, snapshot); }
      return () => { observers.delete(listener); };
    },
    async execute(command) {
      if (detached) return refuse(command, "detached");
      const failure = admissionFailure(command);
      if (failure) return refuse(command, failure);
      commands.push(clone(command));
      const status = next; next = "accepted";
      if (status === "refused") return refuse(command, "fixture refusal");
      // Acceptance is dispatch only. Only the test driver can publish changed facts.
      return { status, requestId: command.requestId, snapshot: clone(snapshot) };
    },
    detach() { detached = true; observers.clear(); },
  };
  const driver: FixtureDriver = {
    publish,
    disconnect() {
      if (snapshot.connection === "disconnected") return;
      snapshot = { ...snapshot, revision: snapshot.revision + 1, connection: "disconnected" }; emit();
    },
    reconnect(value) {
      if (value) {
        if (!valid(value) || retiredEpochs.has(value.providerEpoch)) return false;
        if (value.providerEpoch === snapshot.providerEpoch && value.revision <= snapshot.revision) return false;
        if (value.providerEpoch !== snapshot.providerEpoch) retiredEpochs.add(snapshot.providerEpoch);
        snapshot = { ...clone(value), connection: "connected" };
      } else snapshot = { ...snapshot, revision: snapshot.revision + 1, connection: "connected" };
      emit(); return true;
    },
    setNextResult(value) { next = value; }, current() { return clone(snapshot); }, commands() { return clone(commands); },
    captureContinues() { return snapshot.captures.some(c => c.phase === "recording"); },
  };
  return { client, driver };
}
