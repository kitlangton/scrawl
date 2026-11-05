import type { CapabilitySnapshot, CommandResult, RecordClient, RecordCommand } from "./index.js";

export const fixtureScenarios = [
  "permission-granted-relaunch-needed",
  "external-shortcut-save-failure",
  "pen-write-failure-preserves-effective",
  "unrelated-capture",
  "reconnect-with-saved-history",
  "ambiguous-command-preserves-capture",
  "unsubscribe-during-recording",
] as const;
export type FixtureScenario = (typeof fixtureScenarios)[number];
export interface FixtureDriver {
  publish(snapshot: CapabilitySnapshot): void;
  disconnect(): void;
  reconnect(snapshot?: CapabilitySnapshot): void;
  setNextResult(result: "applied" | "refused" | "cancelled" | "outcome-unknown"): void;
  current(): CapabilitySnapshot;
  captureContinues(): boolean;
}
export interface RecordFixture { client: RecordClient; driver: FixtureDriver }
const clone = <T>(value: T): T => structuredClone(value);

function initial(scenario: FixtureScenario): CapabilitySnapshot {
  const snapshot: CapabilitySnapshot = {
    version: 1, providerEpoch: "fixture-epoch-1", revision: 1, connection: "connected",
    readiness: { requirements: [{ id: "screen-capture", required: true, status: "granted", usableNow: true, relaunchNeeded: false, allowedRepairActions: [] }] },
    shortcut: { effective: "Ctrl+Shift+R", label: "Ctrl Shift R", registration: "registered", externallyManaged: false, draft: null },
    pen: { effective: "ink", supported: ["ink", "laser", "cursor"] }, captures: [],
  };
  if (scenario === "permission-granted-relaunch-needed") {
    snapshot.readiness.requirements[0] = { id: "screen-capture", required: true, status: "granted", usableNow: false, relaunchNeeded: true, allowedRepairActions: ["request-safe-relaunch"] };
  }
  if (scenario === "external-shortcut-save-failure") snapshot.shortcut = { ...snapshot.shortcut, effective: null, label: null, registration: "externally-managed", externallyManaged: true };
  if (scenario === "pen-write-failure-preserves-effective") snapshot.pen.error = "fixture write failure";
  if (scenario === "unrelated-capture") snapshot.captures = [{ captureId: "capture-unrelated", phase: "recording", outcomes: [{ kind: "first-frame-start", at: "2025-01-01T00:00:00.000Z", ok: true }] }];
  if (scenario === "reconnect-with-saved-history") snapshot.captures = [{ captureId: "capture-saved", phase: "saved", outcomes: [{ kind: "save", at: "2025-01-01T00:00:01.000Z", ok: true, detail: "/tmp/fixture.apparent" }] }];
  if (scenario === "ambiguous-command-preserves-capture" || scenario === "unsubscribe-during-recording") snapshot.captures = [{ captureId: "capture-active", phase: "recording", outcomes: [{ kind: "first-frame-start", at: "2025-01-01T00:00:00.000Z", ok: true }] }];
  return snapshot;
}

/** Explicitly synthetic test fixture; no native, filesystem or Electron access. */
export function createRecordFixture(scenario: FixtureScenario = "permission-granted-relaunch-needed"): RecordFixture {
  let snapshot = initial(scenario);
  let next: FixtureDriver["setNextResult"] extends (v: infer T) => void ? T : never = "applied";
  let detached = false;
  let captureContinues = snapshot.captures.some(c => c.phase === "recording");
  const observers = new Set<(snapshot: CapabilitySnapshot) => void>();
  const emit = () => { for (const listener of [...observers]) { try { listener(clone(snapshot)); } catch { observers.delete(listener); } } };
  const publish = (value: CapabilitySnapshot) => { snapshot = clone(value); emit(); };
  const result = (command: RecordCommand): CommandResult => {
    if (command.expected.providerEpoch !== snapshot.providerEpoch || command.expected.revision !== snapshot.revision) return { status: "refused", requestId: command.requestId, reason: "stale provider epoch or revision", snapshot: clone(snapshot) };
    const status = next; next = "applied";
    if (status === "outcome-unknown") return { status, requestId: command.requestId, snapshot: clone(snapshot) };
    if (status === "refused") return { status, requestId: command.requestId, reason: "fixture refusal", snapshot: clone(snapshot) };
    if (status === "cancelled") return { status, requestId: command.requestId, snapshot: clone(snapshot) };
    snapshot = { ...snapshot, revision: snapshot.revision + 1 };
    if (scenario === "external-shortcut-save-failure" && command.type === "shortcut-save") {
      return { status: "refused", requestId: command.requestId, reason: "shortcut is externally managed", snapshot: clone(snapshot) };
    }
    if (command.type === "start-capture") {
      const captureId = `capture-${command.requestId}`;
      snapshot.captures = [...snapshot.captures, { captureId, invocationId: command.requestId, phase: "recording", outcomes: [{ kind: "first-frame-start", at: "2025-01-01T00:00:00.000Z", ok: true }] }];
      captureContinues = true;
      return { status, requestId: command.requestId, snapshot: clone(snapshot), receipt: { captureId } };
    }
    if (command.type === "select-pen" && scenario === "pen-write-failure-preserves-effective") {
      snapshot = { ...snapshot, pen: { ...snapshot.pen, error: "fixture write failure" } };
      return { status: "refused", requestId: command.requestId, reason: "pen preference write failed", snapshot: clone(snapshot) };
    }
    if (command.type === "stop-capture" || command.type === "cancel-capture") {
      const capture = snapshot.captures.find(c => c.captureId === command.captureId);
      if (!capture) return { status: "refused", requestId: command.requestId, reason: "unknown capture", snapshot: clone(snapshot) };
      capture.phase = command.type === "stop-capture" ? "processing" : "cancelled";
      captureContinues = false;
    }
    return { status, requestId: command.requestId, snapshot: clone(snapshot) };
  };
  const client: RecordClient = {
    observe(listener) { if (!detached) { observers.add(listener); try { listener(clone(snapshot)); } catch { observers.delete(listener); } } return () => observers.delete(listener); },
    async execute(command) { if (detached) return { status: "refused", requestId: command.requestId, reason: "detached", snapshot: clone(snapshot) }; return result(command); },
    detach() { detached = true; observers.clear(); },
  };
  const driver: FixtureDriver = {
    publish,
    disconnect() { snapshot = { ...snapshot, revision: snapshot.revision + 1, connection: "disconnected" }; emit(); },
    reconnect(value) { const nextSnapshot = value ? clone(value) : clone(snapshot); snapshot = { ...nextSnapshot, providerEpoch: `fixture-epoch-${Number(snapshot.providerEpoch.split("-").at(-1)) + 1}`, revision: Math.max(snapshot.revision, nextSnapshot.revision) + 1, connection: "connected" }; emit(); },
    setNextResult(value) { next = value; }, current() { return clone(snapshot); }, captureContinues() { return captureContinues; },
  };
  return { client, driver };
}
