import type { RecordCapture2StartPort, RecordHostRuntimePort } from "../host/runtime.js";
import type {
  CapabilitySnapshot, CaptureFact, CommandResult, NativeRequirement, PenColor,
  PenPreference, RecordClient, RecordCommand,
} from "../../capabilities/index.js";

/**
 * Record-local structural seam for Platform's forthcoming shared host port.
 * This deliberately contains no transport or OS implementation. A missing port
 * is represented as unavailable facts and refused actions.
 */
export interface RecordNativeFacts {
  version: 1;
  providerEpoch: string;
  revision: number;
  readiness: NativeRequirement[];
  shortcut: CapabilitySnapshot["shortcut"];
}

/** Authenticated Record output facts; raw Engine v1 saved material is not enough. */
export interface RecordCaptureFacts {
  version: 1;
  providerEpoch: string;
  revision: number;
  captures: CaptureFact[];
  nextShortcutAdmission: CapabilitySnapshot["nextShortcutAdmission"];
}

export type NativeAction = Extract<RecordCommand,
  { type: "request-permission" | "open-system-settings" | "request-safe-relaunch" |
    "shortcut-begin-edit" | "shortcut-submit" | "shortcut-save" | "shortcut-cancel" |
    "shortcut-reset" | "shortcut-open-desktop-config" | "copy-local" | "open-capture" }>;
export type CaptureAction = Extract<RecordCommand, { type: "arm-next-shortcut" | "cancel-next-shortcut" }>;

export interface RecordNativePort {
  /** Read facts produced by the actual same-user native host transport. */
  readFacts(): Promise<RecordNativeFacts>;
  /** Optional notification only; each notification causes an authoritative reread. */
  observe?(listener: () => void): () => void;
  /** Detaches this adapter only; must not close its caller-owned transport. */
  dispose?(): void;
  /** Dispatch only. This receipt cannot establish an operation outcome. */
  dispatch?(command: NativeAction, observed?: { providerEpoch: string; revision: number }): Promise<"accepted" | "refused" | "cancelled" | "outcome-unknown">;
}

/** Producer seam for assembled, authenticated Record output and reservation facts. */
export interface RecordCaptureFactsPort {
  readFacts(): Promise<RecordCaptureFacts>;
  observe?(listener: () => void): () => void;
  dispatch?(command: CaptureAction): Promise<"accepted" | "refused" | "cancelled" | "outcome-unknown">;
}

/** Owner adapter over the existing authoritative penColor writer. */
export interface RecordPenConfigPort {
  /** Reads the persisted top-level `penColor` and actual currently applied value. */
  read(): Promise<{ persisted: PenColor | null; effective: PenColor | null }>;
  /** Delegates to the existing single config writer and its native apply/readback. */
  writeApplyAndReadBack(color: PenColor): Promise<{ status: "saved"; effective: PenColor } | { status: "unknown" }>;
}

export interface RecordCapabilityProviderOptions {
  runtime: RecordHostRuntimePort;
  /** Capture/2 adapters may provide explicit refusal versus ambiguous start outcomes. */
  capture2Start?: RecordCapture2StartPort;
  /** Undefined means the Platform host binding is not installed. */
  native?: RecordNativePort;
  /** Undefined until captured material has passed Record assembly and authenticated publication. */
  captureFacts?: RecordCaptureFactsPort;
  /** Undefined means the published config backend binding is not installed. */
  penConfig?: RecordPenConfigPort;
  /** Stable for one provider process; a restarted process must use a new value. */
  providerEpoch: string;
}

export interface RecordCapabilityProvider extends RecordClient {
  /** Read-only inspection copy for the host and tests. */
  current(): CapabilitySnapshot;
}

const clone = <T>(value: T): T => structuredClone(value);
const unknownPen = (): PenPreference => ({ effective: null, supported: ["coral", "purple", "blue", "green"], error: "authoritative pen config backend unavailable" });
const unavailableNative = (): RecordNativeFacts => ({
  version: 1, providerEpoch: "unavailable", revision: 0,
  readiness: [{ id: "native-host", required: true, status: "unavailable", usableNow: false, relaunchNeeded: false, allowedRepairActions: [] }],
  shortcut: { effective: null, label: null, registration: "unknown", externallyManaged: false, draft: null },
});
const unavailableCaptureFacts = (): RecordCaptureFacts => ({ version: 1, providerEpoch: "unavailable", revision: 0, captures: [], nextShortcutAdmission: { state: "unavailable" } });
const engineRequirement = (ready: boolean): NativeRequirement => ({
  id: "engine-screen-capture", required: true, status: ready ? "granted" : "unavailable",
  usableNow: ready, relaunchNeeded: false, allowedRepairActions: [],
});
const isPenColor = (value: unknown): value is PenColor => value === "coral" || value === "purple" || value === "blue" || value === "green";
function validNativeFacts(value: unknown): value is RecordNativeFacts {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const facts = value as Record<string, unknown>;
  if (facts.version !== 1 || typeof facts.providerEpoch !== "string" || !facts.providerEpoch.trim() || !Number.isSafeInteger(facts.revision) || Number(facts.revision) < 0) return false;
  const validRequirements = Array.isArray(facts.readiness) && facts.readiness.every(item => !!item && typeof item === "object" &&
    typeof item.id === "string" && typeof item.required === "boolean" && typeof item.usableNow === "boolean" &&
    typeof item.relaunchNeeded === "boolean" && ["granted", "denied", "not-determined", "unavailable"].includes(item.status) &&
    Array.isArray(item.allowedRepairActions) && item.allowedRepairActions.every((action: unknown) => ["request-permission", "open-system-settings", "request-safe-relaunch"].includes(String(action))));
  const shortcut = facts.shortcut as Record<string, unknown> | undefined;
  const validShortcut = !!shortcut && (shortcut.configured === undefined || shortcut.configured === null || typeof shortcut.configured === "string") &&
    (shortcut.effective === null || typeof shortcut.effective === "string") &&
    (shortcut.label === null || typeof shortcut.label === "string") &&
    ["registered", "unregistered", "externally-managed", "unknown", "unavailable"].includes(String(shortcut.registration)) &&
    typeof shortcut.externallyManaged === "boolean" && (shortcut.draft === null || (!!shortcut.draft && typeof shortcut.draft === "object" &&
      typeof (shortcut.draft as Record<string, unknown>).sessionId === "string" && typeof (shortcut.draft as Record<string, unknown>).value === "string"));
  return validRequirements && validShortcut;
}

function validRecordCaptureFacts(value: unknown): value is RecordCaptureFacts {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const facts = value as Record<string, unknown>;
  if (facts.version !== 1 || typeof facts.providerEpoch !== "string" || !facts.providerEpoch.trim() || !Number.isSafeInteger(facts.revision) || Number(facts.revision) < 0) return false;
  const admission = facts.nextShortcutAdmission as Record<string, unknown> | undefined;
  const validAdmission = !!admission && (
    admission.state === "unavailable" || admission.state === "idle" ||
    (admission.state === "armed" && typeof admission.admissionId === "string" &&
      (admission.delivery === "local-only" || admission.delivery === "host-default") && typeof admission.expiresAt === "string") ||
    (admission.state === "consumed" && typeof admission.admissionId === "string" && typeof admission.captureId === "string") ||
    ((admission.state === "cancelled" || admission.state === "expired") && typeof admission.admissionId === "string"));
  const validCaptures = Array.isArray(facts.captures) && facts.captures.every(capture => {
    if (!capture || typeof capture !== "object") return false;
    const item = capture as Record<string, unknown>;
    if (typeof item.captureId !== "string" || !["local-only", "host-default", "unknown"].includes(String(item.delivery)) ||
      !["starting", "recording", "processing", "saved", "cancelled", "failed", "unknown"].includes(String(item.phase)) || !Array.isArray(item.outcomes)) return false;
    const validOutcomes = item.outcomes.every(outcome => {
      if (!outcome || typeof outcome !== "object") return false;
      const fact = outcome as Record<string, unknown>;
      if (typeof fact.at !== "string" || typeof fact.ok !== "boolean") return false;
      if (fact.kind === "picker") return fact.ok
        ? fact.surface === "screen" && ["display", "area", "window"].includes(String(fact.target))
        : ["cancelled", "failed", "unavailable"].includes(String(fact.reason));
      if (fact.kind === "accepted-stroke") return fact.ok
        ? ["screen", "whiteboard"].includes(String(fact.surface)) && typeof fact.strokeId === "string"
        : ["screen", "whiteboard"].includes(String(fact.surface)) && ["rejected", "failed"].includes(String(fact.reason));
      if (fact.kind === "artifact-open") return typeof fact.artifactId === "string" && (fact.ok
        ? ["viewer-ack", "os-observation"].includes(String(fact.evidence))
        : ["refused", "failed"].includes(String(fact.reason)));
      return ["shortcut", "first-frame-start", "whiteboard", "processing", "save", "copy", "open-dispatch"].includes(String(fact.kind));
    });
    if (!validOutcomes) return false;
    return item.phase !== "saved" || item.outcomes.some(outcome => {
      const fact = outcome as Record<string, unknown>;
      return fact.kind === "save" && fact.ok === true;
    });
  });
  return validCaptures && validAdmission;
}

function revisionedPortSnapshots<T extends { version: 1; providerEpoch: string; revision: number }>(fingerprintOf: (value: T) => unknown = value => value) {
  let epoch: string | undefined;
  let revision = -1;
  let fingerprint = "";
  const retired = new Set<string>();
  return (next: T): T | undefined => {
    if (retired.has(next.providerEpoch)) return undefined;
    const nextFingerprint = JSON.stringify(fingerprintOf(next));
    if (epoch === undefined || epoch !== next.providerEpoch) {
      if (epoch !== undefined) retired.add(epoch);
      epoch = next.providerEpoch;
      revision = next.revision;
      fingerprint = nextFingerprint;
      return next;
    }
    if (next.revision < revision || (next.revision === revision && nextFingerprint !== fingerprint)) return undefined;
    if (next.revision > revision) { revision = next.revision; fingerprint = nextFingerprint; }
    return next;
  };
}

async function readPenPreference(config?: RecordPenConfigPort): Promise<PenPreference> {
  if (!config) return unknownPen();
  try {
    const value = await config.read();
    if (value.persisted !== null && !isPenColor(value.persisted)) return { ...unknownPen(), error: "stored pen color is invalid" };
    if (value.effective !== null && !isPenColor(value.effective)) return { ...unknownPen(), error: "effective pen color is invalid" };
    return { effective: value.effective, supported: ["coral", "purple", "blue", "green"], ...(value.effective === null ? { error: "effective pen color unavailable" } : {}) };
  } catch { return { ...unknownPen(), error: "authoritative pen config/readback unavailable" }; }
}

function mergeCapture(previous: CaptureFact | undefined, next: CaptureFact): CaptureFact {
  if (!previous) return clone(next);
  const outcomes = [...previous.outcomes];
  for (const outcome of next.outcomes) {
    if (!outcomes.some(value => JSON.stringify(value) === JSON.stringify(outcome))) outcomes.push(clone(outcome));
  }
  return { ...previous, ...clone(next), outcomes };
}

function captureFromRuntime(state: RecordHostRuntimePort["state"], observedAt: Map<string, string>): CaptureFact | undefined {
  if (!("captureId" in state)) return undefined;
  const phase: CaptureFact["phase"] = state.phase === "ready" ? "processing"
    : state.phase === "stop-failed" || state.phase === "uncertain" ? "unknown"
      : state.phase === "failed" ? "failed" : state.phase === "cancelled" ? "cancelled"
        : state.phase === "checking" || state.phase === "cancelling" || state.phase === "stopping" ? "unknown"
          : state.phase;
  const outcomes: CaptureFact["outcomes"] = [];
  const eventKey = `${state.captureId}:${state.phase}`;
  const at = observedAt.get(eventKey) ?? new Date().toISOString();
  observedAt.set(eventKey, at);
  // Engine recording/processing receipts are lifecycle evidence. A v1 Engine
  // saved receipt only closes its material operation; it does not prove Record
  // policy assembly or authenticated publication, so it remains processing.
  // Delivery, shortcut, picker, and artifact identity are never inferred here.
  if (state.phase === "recording") outcomes.push({ kind: "first-frame-start", at, ok: true });
  if (state.phase === "processing" || state.phase === "ready") outcomes.push({ kind: "processing", at, ok: true });
  return { captureId: state.captureId, delivery: "unknown", phase, outcomes };
}

/** Compose the published Engine runtime with narrow host/config producers. */
export function createRecordCapabilityProvider(options: RecordCapabilityProviderOptions): RecordCapabilityProvider {
  if (!options.providerEpoch.trim()) throw new Error("providerEpoch is required");
  let detached = false;
  let revision = 0;
  let nativeReadSequence = 0;
  let nativeFacts = unavailableNative();
  let outputFacts = unavailableCaptureFacts();
  // Record edit drafts are consumer-owned UI state layered over immutable native facts.
  const acceptNativeSnapshot = revisionedPortSnapshots<RecordNativeFacts>(value => ({ ...value, shortcut: { ...value.shortcut, draft: null } }));
  const acceptOutputSnapshot = revisionedPortSnapshots<RecordCaptureFacts>();
  let pen = unknownPen();
  const captures = new Map<string, CaptureFact>();
  const runtimeObservedAt = new Map<string, string>();
  const listeners = new Set<(snapshot: CapabilitySnapshot) => void>();
  const usedRequests = new Set<string>();
  let commandQueue = Promise.resolve();

  const engineReady = (): { ready: boolean; reason?: string } => {
    const readiness = options.runtime.state.readiness;
    return readiness.ready ? { ready: true } : { ready: false, reason: readiness.reason };
  };
  const snapshot = (): CapabilitySnapshot => {
    const live = captureFromRuntime(options.runtime.state, runtimeObservedAt);
    const merged = new Map<string, CaptureFact>();
    const completionEligible = new Set(options.runtime.completionEligibleCaptureIds?.() ??
      (options.runtime.state.phase === "ready" ? [options.runtime.state.captureId] : []));
    for (const item of outputFacts.captures) merged.set(item.captureId, mergeCapture(merged.get(item.captureId), item));
    for (const item of captures.values()) merged.set(item.captureId, mergeCapture(merged.get(item.captureId), item));
    for (const runtimeCapture of options.runtime.captures?.() ?? []) {
      const fact = captureFromRuntime(runtimeCapture, runtimeObservedAt);
      if (fact) {
        const output = merged.get(fact.captureId);
        const mayComplete = completionEligible.has(fact.captureId) && output?.phase === "saved";
        merged.set(fact.captureId, mayComplete ? mergeCapture(fact, output!) : mergeCapture(output, fact));
      }
    }
    if (live) {
      const output = merged.get(live.captureId);
      const combined = output ? mergeCapture(live, output) : live;
      // Only stopped native materials can be completed by assembled output.
      // Retained output must never hide live or unresolved runtime ownership.
      if (!completionEligible.has(live.captureId) || output?.phase !== "saved") combined.phase = live.phase;
      merged.set(live.captureId, combined);
    }
    const ready = engineReady();
    return {
      version: 2, providerEpoch: options.providerEpoch, revision,
      connection: detached ? "disconnected" : "connected",
      readiness: { requirements: [...clone(nativeFacts.readiness), engineRequirement(ready.ready)] },
      shortcut: clone(nativeFacts.shortcut), pen: clone(pen),
      captures: [...merged.values()].map(clone),
      nextShortcutAdmission: clone(outputFacts.nextShortcutAdmission),
    };
  };
  const publish = () => {
    if (detached) return;
    revision++;
    const frame = snapshot();
    for (const listener of [...listeners]) {
      if (!listeners.has(listener)) continue;
      try { listener(clone(frame)); } catch { listeners.delete(listener); }
    }
  };
  const refresh = async (): Promise<void> => {
    const sequence = ++nativeReadSequence;
    const [native, output, configuredPen] = await Promise.all([
      options.native ? options.native.readFacts().catch(() => undefined) : Promise.resolve(undefined),
      options.captureFacts ? options.captureFacts.readFacts().catch(() => undefined) : Promise.resolve(undefined),
      readPenPreference(options.penConfig),
    ]);
    if (detached || sequence !== nativeReadSequence) return;
    const acceptedNative = validNativeFacts(native) ? acceptNativeSnapshot(native) : undefined;
    const acceptedOutput = validRecordCaptureFacts(output) ? acceptOutputSnapshot(output) : undefined;
    nativeFacts = acceptedNative ? clone(acceptedNative) : unavailableNative();
    outputFacts = acceptedOutput ? clone(acceptedOutput) : unavailableCaptureFacts();
    pen = clone(configuredPen);
    for (const capture of outputFacts.captures) captures.set(capture.captureId, mergeCapture(captures.get(capture.captureId), capture));
    publish();
  };
  const unsubscribeRuntime = options.runtime.subscribe(() => publish());
  const unsubscribeNative = options.native?.observe?.(() => { void refresh(); }) ?? (() => undefined);
  const unsubscribeCaptureFacts = options.captureFacts?.observe?.(() => { void refresh(); }) ?? (() => undefined);
  void refresh();

  const refuse = (command: RecordCommand, reason: string): CommandResult => ({ status: "refused", requestId: command.requestId, reason, snapshot: snapshot() });
  const executeOne = async (command: RecordCommand): Promise<CommandResult> => {
    if (detached) return refuse(command, "provider detached");
    const current = snapshot();
    if (command.expected.providerEpoch !== current.providerEpoch || command.expected.revision !== current.revision)
      return refuse(command, "stale provider epoch or revision");
    if (!command.requestId.trim() || usedRequests.has(command.requestId)) return refuse(command, "request ID is empty or already used");
    const response = (status: CommandResult["status"]): CommandResult => status === "refused"
      ? refuse(command, "action refused or unavailable")
      : { status, requestId: command.requestId, snapshot: snapshot() } as CommandResult;
    if (command.type === "refresh") { await refresh(); await options.runtime.dispatch({ type: "refresh-readiness" }); publish(); return response("accepted"); }
    if (command.type === "select-pen") {
      if (!options.penConfig) return refuse(command, "authoritative config backend unavailable");
      if (!pen.supported.includes(command.value)) return refuse(command, "unsupported pen color");
      usedRequests.add(command.requestId);
      try {
        const result = await options.penConfig.writeApplyAndReadBack(command.value);
        if (result.status !== "saved" || result.effective !== command.value) { pen = { ...pen, error: "pen config write/apply readback unknown" }; publish(); return response("outcome-unknown"); }
        pen = { ...pen, effective: result.effective, error: undefined };
        publish(); return response("applied");
      } catch { pen = { ...pen, error: "pen config write/apply outcome unknown" }; publish(); return response("outcome-unknown"); }
    }
    if (command.type === "start-capture") {
      if (command.delivery === "local-only") return refuse(command, "Engine SDK has no local-only enforcement capability");
      if (options.runtime.startup.state !== "ready") return refuse(command, "Engine runtime startup reconciliation is unresolved");
      if (outputFacts.nextShortcutAdmission.state === "armed") return refuse(command, "capture reserved for next shortcut");
      if (current.readiness.requirements.some(requirement => requirement.required && (!requirement.usableNow || requirement.relaunchNeeded))) return refuse(command, "capture requirements unavailable");
      if (options.runtime.canStartCapture ? !options.runtime.canStartCapture() : current.captures.some(capture => ["starting", "recording", "processing", "unknown"].includes(capture.phase))) return refuse(command, "capture active or unresolved");
      usedRequests.add(command.requestId);
      try {
        const result = options.capture2Start
          ? await options.capture2Start.startCapture(command.requestId)
          : { state: await options.runtime.dispatch({ type: "start", requestId: command.requestId }), outcome: undefined };
        const state = result.state;
        publish();
        if (result.outcome === "refused") return response("refused");
        if (result.outcome === "outcome-unknown") return response("outcome-unknown");
        if (!("captureId" in state) || state.captureId !== command.requestId) return response("refused");
        if (state.phase === "uncertain") return response("outcome-unknown");
        return response("accepted");
      }
      catch { return response("outcome-unknown"); }
    }
    if ("captureId" in command) {
      const known = current.captures.find(capture => capture.captureId === command.captureId);
      if (!known) return refuse(command, "unknown capture");
      if ((command.type === "copy-local" || command.type === "open-capture") && known.phase !== "saved") return refuse(command, "capture is not saved");
      if (command.type === "stop-capture" || command.type === "cancel-capture" || command.type === "inspect-capture") {
        const state = options.runtime.state;
        if (!("captureId" in state) || state.captureId !== command.captureId) return refuse(command, "Engine runtime does not own this capture");
        if (command.type === "stop-capture" && state.phase !== "recording") return refuse(command, "Engine runtime cannot stop this phase");
        if (command.type === "cancel-capture" && !["starting", "recording"].includes(state.phase)) return refuse(command, "Engine runtime cannot cancel this phase");
        if (command.type === "inspect-capture" && !["uncertain", "stop-failed"].includes(state.phase)) return refuse(command, "Engine runtime has no reconciliation pending");
        const action = command.type === "stop-capture" ? "stop" : command.type === "cancel-capture" ? "cancel" : "reconcile";
        usedRequests.add(command.requestId);
        try { await options.runtime.dispatch({ type: action }); publish(); return response("accepted"); }
        catch { return response("outcome-unknown"); }
      }
    }
    if (command.type === "request-permission" || command.type === "open-system-settings") {
      const requirement = nativeFacts.readiness.find(item => item.id === command.requirementId);
      if (!requirement?.allowedRepairActions.includes(command.type)) return refuse(command, "permission repair action unavailable");
    }
    if (command.type === "request-safe-relaunch" && (
      nativeFacts.readiness.every(item => !item.allowedRepairActions.includes("request-safe-relaunch")) ||
      current.captures.some(capture => ["starting", "recording", "processing", "unknown"].includes(capture.phase))))
      return refuse(command, "safe relaunch unavailable while capture is active or unresolved");
    if (["shortcut-begin-edit", "shortcut-submit", "shortcut-save", "shortcut-cancel", "shortcut-reset"].includes(command.type) &&
      nativeFacts.shortcut.externallyManaged) return refuse(command, "shortcut is externally managed");
    if (command.type === "shortcut-begin-edit" && nativeFacts.shortcut.draft) return refuse(command, "shortcut edit already active");
    if ((command.type === "shortcut-submit" || command.type === "shortcut-save" || command.type === "shortcut-cancel") &&
      nativeFacts.shortcut.draft?.sessionId !== command.sessionId) return refuse(command, "shortcut edit session not current");
    if (command.type === "arm-next-shortcut" && outputFacts.nextShortcutAdmission.state !== "idle")
      return refuse(command, outputFacts.nextShortcutAdmission.state === "unavailable" ? "shortcut reservation producer unavailable" : "next-shortcut admission is not idle");
    if (command.type === "arm-next-shortcut" && command.delivery === "local-only") return refuse(command, "local-only enforcement unavailable");
    if (command.type === "cancel-next-shortcut" && (outputFacts.nextShortcutAdmission.state !== "armed" || outputFacts.nextShortcutAdmission.admissionId !== command.admissionId))
      return refuse(command, outputFacts.nextShortcutAdmission.state === "unavailable" ? "shortcut reservation producer unavailable" : "matching next-shortcut admission is not armed");
    if (command.type === "arm-next-shortcut" || command.type === "cancel-next-shortcut") {
      if (!options.captureFacts?.dispatch) return refuse(command, "shortcut reservation producer unavailable");
      usedRequests.add(command.requestId);
      try { const status = await options.captureFacts.dispatch(command); await refresh(); return response(status); }
      catch { await refresh(); return response("outcome-unknown"); }
    }
    const nativeCommand = command as NativeAction;
    if (!options.native?.dispatch) return refuse(command, "native host action port unavailable");
    usedRequests.add(command.requestId);
    try {
      const status = await options.native.dispatch(nativeCommand, { providerEpoch: nativeFacts.providerEpoch, revision: nativeFacts.revision });
      await refresh();
      return response(status);
    } catch { await refresh(); return response("outcome-unknown"); }
  };

  const client: RecordCapabilityProvider = {
    current: () => clone(snapshot()),
    observe(listener) {
      if (!detached) { listeners.add(listener); try { listener(clone(snapshot())); } catch { listeners.delete(listener); } }
      return () => listeners.delete(listener);
    },
    execute(command) {
      const work = commandQueue.then(() => executeOne(clone(command)));
      commandQueue = work.then(() => undefined, () => undefined);
      return work;
    },
    detach() {
      if (detached) return;
      detached = true; unsubscribeRuntime(); unsubscribeNative(); unsubscribeCaptureFacts(); listeners.clear();
      // Native adapter cleanup cannot strand the separately owned runtime subscription.
      try { options.native?.dispose?.(); }
      finally { options.runtime.dispose(); }
    },
  };
  return client;
}
