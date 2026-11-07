import type { RecordCapture2StartPort, RecordHostRuntimePort } from "../host/runtime.js";
import type {
  CapabilitySnapshot, CaptureFact, CommandResult, NativeRequirement, PenColor,
  PenPreference, RecordClient, RecordCommand, ScreenStrokeCursor, ShortcutTrialBinding, ShortcutTrialFact, ShortcutTrialReservationIdentity,
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
  /** Omit until a trusted physical press/release producer is available. */
  shortcutTrial?: ShortcutTrialFact;
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
    "shortcut-reset" | "shortcut-open-desktop-config" | "copy-local" | "open-capture" |
    "begin-shortcut-trial" | "cancel-shortcut-trial" }>;
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

/** Privileged exact-text copy consumer. The adapter verifies native digest/length against Record's formatted handoff. */
export type RecordClipboardResult =
  | { status: "applied"; captureId: string; artifactId: string; digest: string; byteLength: number }
  | { status: "refused"; reason: string }
  | { status: "outcome-unknown" };
export type RecordClipboardTextReadbackResult = Exclude<RecordClipboardResult, {status:"applied"}> | { status:"written-readback"; captureId:string; artifactId:string; digest:string; byteLength:number };
export interface RecordClipboardTextReadbackPort {
  copyLocal(input: {captureId:string; artifactId:string}): Promise<RecordClipboardTextReadbackResult>;
}
export interface RecordClipboardPort {
  copyLocal(input: { captureId: string; artifactId: string }): Promise<RecordClipboardResult>;
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
  /** Undefined keeps copy dispatch-only through the legacy native action seam. */
  clipboard?: RecordClipboardPort;
  /** Separate transient text readback; cannot establish selection ownership or paste. */
  clipboardTextReadback?: RecordClipboardTextReadbackPort;
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
  shortcutTrial: { state: "unavailable" },
});
const unavailableCaptureFacts = (): RecordCaptureFacts => ({ version: 1, providerEpoch: "unavailable", revision: 0, captures: [], nextShortcutAdmission: { state: "unavailable" } });
const engineRequirement = (ready: boolean): NativeRequirement => ({
  id: "engine-screen-capture", required: true, status: ready ? "granted" : "unavailable",
  usableNow: ready, relaunchNeeded: false, allowedRepairActions: [],
});
const isPenColor = (value: unknown): value is PenColor => value === "coral" || value === "purple" || value === "blue" || value === "green";
const opaqueId = (value: unknown): value is string => typeof value === "string" && value.trim().length > 0 && value.length <= 256 && !/[\\/\p{Cc}]/u.test(value) && value !== "." && value !== "..";
function validCursor(value: unknown, allowZero = false): value is ScreenStrokeCursor {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const cursor = value as Record<string, unknown>;
  return opaqueId(cursor.historyId) && Number.isSafeInteger(cursor.sequence) && Number(cursor.sequence) >= (allowZero ? 0 : 1) &&
    Object.keys(cursor).sort().join(",") === "historyId,sequence";
}
function validShortcutTrial(value: unknown): value is ShortcutTrialFact {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const trial = value as Record<string, unknown>;
  if (trial.state === "unavailable") return Object.keys(trial).join(",") === "state";
  const base = opaqueId(trial.sourceEpoch) && Number.isSafeInteger(trial.sourceRevision) && Number(trial.sourceRevision) >= 0 &&
    opaqueId(trial.bindingId) && typeof trial.effectiveBinding === "string" && !!trial.effectiveBinding.trim();
  if (!base) return false;
  if (trial.state === "idle") return Object.keys(trial).sort().join(",") === "bindingId,effectiveBinding,sourceEpoch,sourceRevision,state";
  if (trial.state === "reserved" || trial.state === "pressing") return opaqueId(trial.reservationId) && typeof trial.expiresAt === "string" && Number.isFinite(Date.parse(trial.expiresAt)) &&
    Object.keys(trial).sort().join(",") === "bindingId,effectiveBinding,expiresAt,reservationId,sourceEpoch,sourceRevision,state";
  if (trial.state === "observed") return opaqueId(trial.reservationId) && opaqueId(trial.pressId) && typeof trial.pressedAt === "string" && Number.isFinite(Date.parse(trial.pressedAt)) &&
    typeof trial.releasedAt === "string" && Number.isFinite(Date.parse(trial.releasedAt)) && Date.parse(trial.releasedAt) >= Date.parse(trial.pressedAt) &&
    Object.keys(trial).sort().join(",") === "bindingId,effectiveBinding,pressId,pressedAt,releasedAt,reservationId,sourceEpoch,sourceRevision,state";
  if (trial.state === "expired" || trial.state === "cancelled" || trial.state === "unknown") return opaqueId(trial.reservationId) &&
    Object.keys(trial).sort().join(",") === "bindingId,effectiveBinding,reservationId,sourceEpoch,sourceRevision,state";
  return false;
}
function validTrialReservationIdentity(value: unknown): value is ShortcutTrialReservationIdentity {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const identity = value as Record<string, unknown>;
  return Object.keys(identity).sort().join(",") === "bindingId,effectiveBinding,reservationId,sourceEpoch,sourceRevision" &&
    opaqueId(identity.sourceEpoch) && Number.isSafeInteger(identity.sourceRevision) && Number(identity.sourceRevision) >= 0 &&
    opaqueId(identity.bindingId) && typeof identity.effectiveBinding === "string" && !!identity.effectiveBinding.trim() && opaqueId(identity.reservationId);
}
function terminalTrialIdentity(trial: ShortcutTrialFact | undefined): ShortcutTrialReservationIdentity | undefined {
  if (!trial || !["observed", "expired", "cancelled", "unknown"].includes(trial.state) || !("reservationId" in trial)) return undefined;
  return { sourceEpoch: trial.sourceEpoch, sourceRevision: trial.sourceRevision, bindingId: trial.bindingId,
    effectiveBinding: trial.effectiveBinding, reservationId: trial.reservationId };
}
function currentTrialBeginBinding(facts: RecordNativeFacts): ShortcutTrialBinding | undefined {
  const trial = facts.shortcutTrial;
  const shortcut = facts.shortcut;
  if (!trial || trial.state === "unavailable" || trial.state === "reserved" || trial.state === "pressing" ||
      !opaqueId(shortcut.bindingId) || typeof shortcut.effective !== "string" || !shortcut.effective.trim() ||
      shortcut.registration !== "registered" || shortcut.externallyManaged || shortcut.draft !== null) return undefined;
  if (trial.state === "idle") {
    if (trial.sourceEpoch !== facts.providerEpoch || trial.sourceRevision !== facts.revision ||
        trial.bindingId !== shortcut.bindingId || trial.effectiveBinding !== shortcut.effective) return undefined;
  } else if (trial.sourceEpoch === facts.providerEpoch && trial.sourceRevision > facts.revision) return undefined;
  return { sourceEpoch: facts.providerEpoch, sourceRevision: facts.revision, bindingId: shortcut.bindingId, effectiveBinding: shortcut.effective };
}
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
    (shortcut.bindingId === undefined || opaqueId(shortcut.bindingId)) &&
    (shortcut.effective === null || typeof shortcut.effective === "string") &&
    (shortcut.label === null || typeof shortcut.label === "string") &&
    ["registered", "unregistered", "externally-managed", "unknown", "unavailable"].includes(String(shortcut.registration)) &&
    typeof shortcut.externallyManaged === "boolean" && (shortcut.draft === null || (!!shortcut.draft && typeof shortcut.draft === "object" &&
      typeof (shortcut.draft as Record<string, unknown>).sessionId === "string" && typeof (shortcut.draft as Record<string, unknown>).value === "string"));
  return validRequirements && validShortcut && (facts.shortcutTrial === undefined || validShortcutTrial(facts.shortcutTrial));
}

function projectedShortcutTrial(facts: RecordNativeFacts): ShortcutTrialFact {
  const trial = facts.shortcutTrial;
  if (!trial || trial.state === "unavailable") return { state: "unavailable" };
  const bindingMatches = trial.sourceEpoch === facts.providerEpoch &&
    (trial.state === "idle" ? trial.sourceRevision === facts.revision : trial.sourceRevision <= facts.revision) &&
    facts.shortcut.registration === "registered" && !facts.shortcut.externallyManaged &&
    facts.shortcut.draft === null &&
    facts.shortcut.bindingId === trial.bindingId && facts.shortcut.effective === trial.effectiveBinding;
  if (bindingMatches) return clone(trial);
  if ("reservationId" in trial) return { state: "unknown", sourceEpoch: trial.sourceEpoch, sourceRevision: trial.sourceRevision,
    bindingId: trial.bindingId, effectiveBinding: trial.effectiveBinding, reservationId: trial.reservationId };
  return { state: "unavailable" };
}

function validCausalOutcome(value: unknown, cursorKeys: Set<string>, noteReceipts: Set<string>): boolean {
  if (!value || typeof value !== "object" || Array.isArray(value)) return false;
  const outcome = value as Record<string, unknown>;
  if (outcome.kind === "note-save") {
    if (typeof outcome.at !== "string" || !Number.isFinite(Date.parse(outcome.at))) return false;
    if (outcome.ok === false) return ["draft", "empty", "failed", "cancelled"].includes(String(outcome.reason)) &&
      Object.keys(outcome).sort().join(",") === "at,kind,ok,reason";
    if (outcome.ok !== true || !opaqueId(outcome.noteId) || !opaqueId(outcome.receiptId) || !validCursor(outcome.screenStrokeBarrier, true) ||
        Object.keys(outcome).sort().join(",") !== "at,kind,noteId,ok,receiptId,screenStrokeBarrier") return false;
    if (noteReceipts.has(outcome.receiptId)) return false;
    noteReceipts.add(outcome.receiptId);
    return true;
  }
  if (outcome.kind === "accepted-stroke" && outcome.ok === true && outcome.cursor !== undefined) {
    if (outcome.surface !== "screen" || !validCursor(outcome.cursor)) return false;
    const cursor = outcome.cursor as ScreenStrokeCursor;
    const key = `${cursor.historyId}:${cursor.sequence}`;
    if (cursorKeys.has(key)) return false;
    cursorKeys.add(key);
  }
  return true;
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
    if (item.transcriptStatus !== undefined) {
      const status = item.transcriptStatus as Record<string, unknown>;
      if (!status || typeof status !== "object" || Array.isArray(status) || (status.state === "unavailable"
        ? Object.keys(status).sort().join(",") !== "state"
        : status.state !== "collected" || !["empty", "nonempty"].includes(String(status.content)) || !Number.isSafeInteger(status.segmentCount) || Number(status.segmentCount) < 0 || Number(status.segmentCount) > 100_000 || Object.keys(status).sort().join(",") !== "content,segmentCount,state")) return false;
      if (item.phase !== "saved" || typeof item.artifactId !== "string" || !item.artifactId) return false;
    }
    if (item.warnings !== undefined && (!Array.isArray(item.warnings) || item.warnings.length > 2 || item.warnings.some(warning => {
      if (!warning || typeof warning !== "object" || Array.isArray(warning)) return true;
      if (warning.kind === "audioInitialGap") return Object.keys(warning).sort().join(",") !== "atMs,kind" || !Number.isSafeInteger(warning.atMs) || warning.atMs < 0;
      return warning.kind !== "transcriptUnavailable" || Object.keys(warning).sort().join(",") !== "kind";
    }) || new Set(item.warnings.map((warning: {kind:string}) => warning.kind)).size !== item.warnings.length)) return false;
    const cursorKeys = new Set<string>(), noteReceipts = new Set<string>();
    const validOutcomes = item.outcomes.every(outcome => {
      if (!outcome || typeof outcome !== "object") return false;
      const fact = outcome as Record<string, unknown>;
      if (typeof fact.at !== "string" || typeof fact.ok !== "boolean") return false;
      if (fact.kind === "note-save") return validCausalOutcome(fact, cursorKeys, noteReceipts);
      if (fact.kind === "picker") return fact.ok
        ? fact.surface === "screen" && ["display", "area", "window"].includes(String(fact.target))
        : ["cancelled", "failed", "unavailable"].includes(String(fact.reason));
      if (fact.kind === "accepted-stroke") return fact.ok
        ? ["screen", "whiteboard"].includes(String(fact.surface)) && opaqueId(fact.strokeId) && validCausalOutcome(fact, cursorKeys, noteReceipts)
        : ["screen", "whiteboard"].includes(String(fact.surface)) && ["rejected", "failed"].includes(String(fact.reason)) && validCausalOutcome(fact, cursorKeys, noteReceipts);
      if (fact.kind === "copy" && "evidence" in fact) return fact.ok === true && ["native-write-readback", "native-text-readback"].includes(String(fact.evidence)) &&
        typeof fact.artifactId === "string" && !!fact.artifactId.trim() && typeof fact.requestId === "string" && !!fact.requestId.trim() && typeof fact.digest === "string" && /^[0-9a-f]{64}$/u.test(fact.digest) &&
        Number.isSafeInteger(fact.byteLength) && Number(fact.byteLength) > 0;
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

/** Validate durable receipt identity against facts already retained by this provider.
 * Older unseen events remain valid; arrival order never supplies a sequence. */
function consistentCausalHistory(previous: CaptureFact | undefined, next: CaptureFact): boolean {
  if (!previous) return true;
  const notes = new Map<string, { noteId: string; barrier: ScreenStrokeCursor }>();
  const strokes = new Map<string, { surface: "screen" | "whiteboard"; cursor?: ScreenStrokeCursor }>();
  for (const outcome of previous.outcomes) {
    if (outcome.kind === "note-save" && outcome.ok) notes.set(outcome.receiptId, { noteId: outcome.noteId, barrier: outcome.screenStrokeBarrier });
    if (outcome.kind === "accepted-stroke" && outcome.ok) strokes.set(outcome.strokeId, { surface: outcome.surface,
      ...(outcome.surface === "screen" && outcome.cursor ? { cursor: outcome.cursor } : {}) });
  }
  for (const outcome of next.outcomes) {
    if (outcome.kind === "note-save" && outcome.ok) {
      const old = notes.get(outcome.receiptId);
      if (old && (old.noteId !== outcome.noteId || old.barrier.historyId !== outcome.screenStrokeBarrier.historyId ||
          old.barrier.sequence !== outcome.screenStrokeBarrier.sequence)) return false;
    }
    if (outcome.kind === "accepted-stroke" && outcome.ok) {
      const old = strokes.get(outcome.strokeId);
      if (old && (old.surface !== outcome.surface || (outcome.surface === "screen" && outcome.cursor && old.cursor &&
          (old.cursor.historyId !== outcome.cursor.historyId || old.cursor.sequence !== outcome.cursor.sequence)))) return false;
    }
  }
  return true;
}

function mergeCapture(previous: CaptureFact | undefined, next: CaptureFact): CaptureFact {
  if (!previous) return clone(next);
  if (previous.artifactId && next.artifactId && previous.artifactId !== next.artifactId) return clone(next);
  const outcomes = [...previous.outcomes];
  for (const outcome of next.outcomes) {
    if (!outcomes.some(value => JSON.stringify(value) === JSON.stringify(outcome))) outcomes.push(clone(outcome));
  }
  const merged = { ...previous, ...clone(next), outcomes };
  if (next.phase !== "saved") delete merged.transcriptStatus;
  return merged;
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
  if (options.clipboard && options.clipboardTextReadback) throw new TypeError("Choose one clipboard effect contract");
  if (!options.providerEpoch.trim()) throw new Error("providerEpoch is required");
  let detached = false;
  let revision = 0;
  let nativeReadSequence = 0;
  let nativeFacts = unavailableNative();
  let outputFacts = unavailableCaptureFacts();
  // Record edit drafts are consumer-owned UI state layered over immutable native facts.
  const acceptNativeSnapshot = revisionedPortSnapshots<RecordNativeFacts>(value => ({ ...value, shortcut: { ...value.shortcut, draft: null } }));
  const acceptOutputRevision = revisionedPortSnapshots<RecordCaptureFacts>();
  let pen = unknownPen();
  const captures = new Map<string, CaptureFact>();
  const acceptOutputSnapshot = (next: RecordCaptureFacts) => next.captures.every(capture => consistentCausalHistory(captures.get(capture.captureId), capture))
    ? acceptOutputRevision(next) : undefined;
  const runtimeObservedAt = new Map<string, string>();
  const listeners = new Set<(snapshot: CapabilitySnapshot) => void>();
  const usedRequests = new Set<string>();
  let commandQueue = Promise.resolve();

  const engineReady = (): { ready: boolean; reason?: string } => {
    const readiness = options.runtime.state.readiness;
    return readiness.ready ? { ready: true } : { ready: false, reason: readiness.reason };
  };
  const acceptedStoppedOutput = (fact: CaptureFact | undefined): boolean => fact?.phase === "saved" ||
    ((fact?.phase === "failed" || fact?.phase === "unknown") && fact.outcomes.some(outcome => outcome.kind === "processing" && !outcome.ok));
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
        const mayComplete = completionEligible.has(fact.captureId) && acceptedStoppedOutput(output);
        merged.set(fact.captureId, mayComplete ? mergeCapture(fact, output!) : mergeCapture(output, fact));
      }
    }
    if (live) {
      const output = merged.get(live.captureId);
      const combined = output ? mergeCapture(live, output) : live;
      // Only stopped native materials can be completed by assembled output.
      // Retained output must never hide live or unresolved runtime ownership.
      if (!completionEligible.has(live.captureId) || !acceptedStoppedOutput(output)) combined.phase = live.phase;
      merged.set(live.captureId, combined);
    }
    const ready = engineReady();
    const trialBeginBinding = currentTrialBeginBinding(nativeFacts);
    return {
      version: 2, providerEpoch: options.providerEpoch, revision,
      connection: detached ? "disconnected" : "connected",
      readiness: { requirements: [...clone(nativeFacts.readiness), engineRequirement(ready.ready)] },
      shortcut: clone(nativeFacts.shortcut), pen: clone(pen),
      captures: [...merged.values()].map(clone),
      nextShortcutAdmission: clone(outputFacts.nextShortcutAdmission),
      shortcutTrial: projectedShortcutTrial(nativeFacts),
      ...(trialBeginBinding ? { shortcutTrialBeginBinding: clone(trialBeginBinding) } : {}),
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
      if (command.type === "copy-local" && (options.clipboard || options.clipboardTextReadback)) {
        const textReadback = !!options.clipboardTextReadback;
        const clipboard = options.clipboardTextReadback ?? options.clipboard!;
        const artifactId = known.artifactId;
        if (!artifactId?.trim()) return refuse(command, "saved artifact identity unavailable");
        usedRequests.add(command.requestId);
        try {
          const receipt = await clipboard.copyLocal({ captureId: known.captureId, artifactId });
          if (detached) return response("outcome-unknown");
          if (receipt?.status === "refused" && typeof receipt.reason === "string") return refuse(command, receipt.reason);
          if (receipt?.status !== (textReadback ? "written-readback" : "applied") || receipt.captureId !== known.captureId || receipt.artifactId !== artifactId ||
              !/^[0-9a-f]{64}$/u.test(receipt.digest) || !Number.isSafeInteger(receipt.byteLength) || receipt.byteLength <= 0)
            return response("outcome-unknown");
          await refresh();
          if (detached || snapshot().captures.find(item => item.captureId === known.captureId)?.artifactId !== artifactId)
            return response("outcome-unknown");
          const latest = captures.get(known.captureId);
          if (!latest || latest.artifactId !== artifactId) return response("outcome-unknown");
          captures.set(known.captureId, mergeCapture(latest, { ...latest, outcomes: [{
            kind: "copy", at: new Date().toISOString(), ok: true, evidence: textReadback ? "native-text-readback" : "native-write-readback",
            artifactId, requestId: command.requestId, digest: receipt.digest, byteLength: receipt.byteLength,
          }] }));
          publish();
          return response(textReadback ? "written-readback" : "applied");
        } catch { return response("outcome-unknown"); }
      }
      if (command.type === "stop-capture" || command.type === "cancel-capture" || command.type === "inspect-capture") {
        const state = options.runtime.state;
        if (!("captureId" in state) || state.captureId !== command.captureId) return refuse(command, "Engine runtime does not own this capture");
        if (command.type === "stop-capture" && state.phase !== "recording") return refuse(command, "Engine runtime cannot stop this phase");
        if (command.type === "cancel-capture" && !["starting", "recording"].includes(state.phase)) return refuse(command, "Engine runtime cannot cancel this phase");
        const processingIssue = options.runtime.completionEligibleCaptureIds?.().includes(command.captureId) &&
          acceptedStoppedOutput(current.captures.find(capture => capture.captureId === command.captureId && capture.phase !== "saved"));
        if (command.type === "inspect-capture" && !["uncertain", "stop-failed"].includes(state.phase) && !processingIssue)
          return refuse(command, "Engine runtime has no reconciliation pending");
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
    if (command.type === "begin-shortcut-trial" || command.type === "cancel-shortcut-trial") {
      const trial = current.shortcutTrial ?? { state: "unavailable" as const };
      if (!options.native?.dispatch) return refuse(command, "physical shortcut trial producer unavailable");
      if (command.type === "begin-shortcut-trial") {
        const rawTrial = nativeFacts.shortcutTrial;
        const beginBinding = currentTrialBeginBinding(nativeFacts);
        const replacesPresent = Object.hasOwn(command, "replaces");
        const previous = terminalTrialIdentity(rawTrial);
        const replacementMatches = previous && validTrialReservationIdentity(command.replaces) &&
          previous.sourceEpoch === command.replaces.sourceEpoch && previous.sourceRevision === command.replaces.sourceRevision &&
          previous.bindingId === command.replaces.bindingId && previous.effectiveBinding === command.replaces.effectiveBinding &&
          previous.reservationId === command.replaces.reservationId;
        const freshSelectorsMatch = !!beginBinding && command.sourceEpoch === beginBinding.sourceEpoch &&
          command.sourceRevision === beginBinding.sourceRevision && command.bindingId === beginBinding.bindingId &&
          command.effectiveBinding === beginBinding.effectiveBinding;
        const commonValid = opaqueId(command.reservationId) && Number.isFinite(Date.parse(command.expiresAt)) && freshSelectorsMatch;
        const idleAdmission = rawTrial?.state === "idle" && !replacesPresent && commonValid && trial.state === "idle";
        const terminalRearm = !!previous && replacesPresent && !!replacementMatches && commonValid &&
          command.reservationId !== previous.reservationId;
        if (!idleAdmission && !terminalRearm)
          return refuse(command, "physical shortcut trial binding or source revision changed");
        const action = {
          type: command.type, requestId: command.requestId,
          expected: { providerEpoch: command.expected.providerEpoch, revision: command.expected.revision },
          reservationId: command.reservationId, bindingId: command.bindingId, effectiveBinding: command.effectiveBinding,
          sourceEpoch: command.sourceEpoch, sourceRevision: command.sourceRevision, expiresAt: command.expiresAt,
          ...(terminalRearm ? { replaces: structuredClone(command.replaces!) } : {}),
        } as Extract<NativeAction, { type: "begin-shortcut-trial" }>;
        usedRequests.add(command.requestId);
        try {
          const status = await options.native.dispatch(action, { providerEpoch: nativeFacts.providerEpoch, revision: nativeFacts.revision });
          await refresh();
          return response(status);
        } catch { await refresh(); return response("outcome-unknown"); }
      } else if ((trial.state !== "reserved" && trial.state !== "pressing") || trial.reservationId !== command.reservationId ||
          trial.sourceEpoch !== nativeFacts.providerEpoch || trial.bindingId !== nativeFacts.shortcut.bindingId || trial.effectiveBinding !== nativeFacts.shortcut.effective)
        return refuse(command, "matching physical shortcut trial reservation is unavailable");
      usedRequests.add(command.requestId);
      try {
        const status = await options.native.dispatch(command, { providerEpoch: nativeFacts.providerEpoch, revision: nativeFacts.revision });
        await refresh();
        return response(status);
      } catch { await refresh(); return response("outcome-unknown"); }
    }
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
      const admitted = clone(command);
      const work = commandQueue.then(() => executeOne(admitted));
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
