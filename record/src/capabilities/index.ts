/** Consumer-neutral host boundary. Implementations must report authoritative provider facts. */
export type OsStatus = "granted" | "denied" | "not-determined" | "unavailable";
export type RepairAction = "request-permission" | "open-system-settings" | "request-safe-relaunch";
export interface NativeRequirement {
  id: string;
  required: boolean;
  status: OsStatus;
  usableNow: boolean;
  relaunchNeeded: boolean;
  allowedRepairActions: RepairAction[];
}
export interface ShortcutBinding {
  /** Configured native value, distinct from the binding actually in effect. */
  configured?: string | null;
  effective: string | null;
  /** Opaque identity of the exact effective registration; required for physical trial admission. */
  bindingId?: string;
  label: string | null;
  registration: "registered" | "unregistered" | "externally-managed" | "unknown" | "unavailable";
  externallyManaged: boolean;
  draft: null | { sessionId: string; value: string; error?: string };
}
export interface ScreenStrokeCursor {
  /** Opaque stroke-history incarnation; changes when a history is reset or replaced. */
  historyId: string;
  /** Durable monotone sequence within that incarnation. Timestamps are not causal order. */
  sequence: number;
}
export interface ShortcutTrialBinding {
  sourceEpoch: string;
  sourceRevision: number;
  bindingId: string;
  effectiveBinding: string;
}
/** Exact retained terminal reservation identity used for a guarded new trial. */
export interface ShortcutTrialReservationIdentity extends ShortcutTrialBinding {
  reservationId: string;
}
export type ShortcutTrialFact =
  | { state: "unavailable" }
  | ({ state: "idle" } & ShortcutTrialBinding)
  | ({ state: "reserved" | "pressing" } & ShortcutTrialBinding & { reservationId: string; expiresAt: string })
  | ({ state: "observed" } & ShortcutTrialBinding & { reservationId: string; pressId: string; pressedAt: string; releasedAt: string })
  | ({ state: "expired" | "cancelled" | "unknown" } & ShortcutTrialBinding & { reservationId: string });
export type PenColor = "coral" | "purple" | "blue" | "green";
/** Named penColor preference; drawing tool modes are a separate capability. */
export interface PenPreference {
  effective: PenColor | null;
  supported: PenColor[];
  error?: string;
}
export type CaptureOutcomeKind =
  | "shortcut" | "picker" | "first-frame-start" | "whiteboard" | "accepted-stroke" | "note-save"
  | "processing" | "save" | "copy" | "open-dispatch";
export type CaptureSurface = "screen" | "whiteboard";
export type PickerOutcome =
  | { kind: "picker"; at: string; ok: true; surface: "screen"; target: "display" | "area" | "window" }
  | { kind: "picker"; at: string; ok: false; reason: "cancelled" | "failed" | "unavailable" };
export type AcceptedStrokeOutcome =
  | { kind: "accepted-stroke"; at: string; ok: true; surface: "screen"; strokeId: string; cursor?: ScreenStrokeCursor }
  | { kind: "accepted-stroke"; at: string; ok: true; surface: "whiteboard"; strokeId: string }
  | { kind: "accepted-stroke"; at: string; ok: false; surface: CaptureSurface; reason: "rejected" | "failed" };
/** Success is emitted only after a non-empty note is durably saved. Its barrier
 * is the authoritative screen-stroke cursor read at that save, not arrival time. */
export type DurableNoteSaveOutcome =
  | { kind: "note-save"; at: string; ok: true; noteId: string; receiptId: string; screenStrokeBarrier: ScreenStrokeCursor }
  | { kind: "note-save"; at: string; ok: false; reason: "draft" | "empty" | "failed" | "cancelled" };
export type GenericCaptureOutcomeKind = Exclude<CaptureOutcomeKind, "picker" | "accepted-stroke" | "note-save">;
export type ArtifactOpenOutcome = {
  kind: "artifact-open";
  at: string;
  artifactId: string;
} & (
  | { ok: true; evidence: "viewer-ack" | "os-observation" }
  | { ok: false; reason: "refused" | "failed" }
);
/** Exact supplied-text receipt from a native write/readback operation, scoped to one immutable artifact. */
export type ClipboardCopyOutcome = {
  kind: "copy"; at: string; ok: true; evidence: "native-write-readback";
  artifactId: string; requestId: string; digest: string; byteLength: number;
};
export type CaptureOutcome =
  | { kind: GenericCaptureOutcomeKind; at: string; ok: boolean; detail?: string }
  | PickerOutcome
  | AcceptedStrokeOutcome
  | DurableNoteSaveOutcome
  | ArtifactOpenOutcome
  | ClipboardCopyOutcome;
export type DeliveryEnforcement = "local-only" | "host-default" | "unknown";
export interface CaptureFact {
  captureId: string;
  invocationId?: string;
  /** Effective enforced delivery policy for this capture, never the requested command value. */
  delivery: DeliveryEnforcement;
  /** Request-scoped shortcut admission that produced this capture, when known. */
  shortcutAdmissionId?: string;
  /** Opaque immutable identity of the saved artifact; never its private path. */
  artifactId?: string;
  phase: "starting" | "recording" | "processing" | "saved" | "cancelled" | "failed" | "unknown";
  outcomes: CaptureOutcome[];
}

export interface CapabilitySnapshot {
  version: 2;
  providerEpoch: string;
  revision: number;
  connection: "connected" | "disconnected";
  readiness: { requirements: NativeRequirement[] };
  shortcut: ShortcutBinding;
  pen: PenPreference;
  captures: CaptureFact[];
  nextShortcutAdmission: ShortcutAdmissionFact;
  /** Optional for v2 compatibility; providers publish `unavailable` when no trusted producer exists. */
  shortcutTrial?: ShortcutTrialFact;
  /** Fresh current registration/source selectors for a guarded begin or terminal rearm. */
  shortcutTrialBeginBinding?: ShortcutTrialBinding;
}

export type ShortcutAdmissionFact =
  | { state: "unavailable" }
  | { state: "idle" }
  | { state: "armed"; admissionId: string; delivery: Exclude<DeliveryEnforcement, "unknown">; expiresAt: string }
  | { state: "consumed"; admissionId: string; captureId: string }
  | { state: "cancelled"; admissionId: string }
  | { state: "expired"; admissionId: string };

export interface CommandBase { requestId: string; expected: { providerEpoch: string; revision: number } }
export type RecordCommand = CommandBase & (
  | { type: "refresh" }
  | { type: "request-permission"; requirementId: string }
  | { type: "open-system-settings"; requirementId: string }
  | { type: "request-safe-relaunch" }
  | { type: "shortcut-begin-edit" }
  | { type: "shortcut-submit"; sessionId: string; value: string }
  | { type: "shortcut-save"; sessionId: string }
  | { type: "shortcut-cancel"; sessionId: string }
  | { type: "shortcut-reset" }
  | { type: "shortcut-open-desktop-config" }
  | { type: "select-pen"; value: PenColor }
  | { type: "start-capture"; delivery: "local-only" | "host-default" }
  | { type: "arm-next-shortcut"; admissionId: string; delivery: "local-only" | "host-default"; expiresAt: string }
  | { type: "cancel-next-shortcut"; admissionId: string }
  | { type: "begin-shortcut-trial"; reservationId: string; bindingId: string; effectiveBinding: string; sourceEpoch: string; sourceRevision: number; expiresAt: string; replaces?: ShortcutTrialReservationIdentity }
  | { type: "cancel-shortcut-trial"; reservationId: string }
  | { type: "stop-capture"; captureId: string }
  | { type: "cancel-capture"; captureId: string }
  | { type: "inspect-capture"; captureId: string }
  | { type: "copy-local"; captureId: string }
  | { type: "open-capture"; captureId: string }
);
export type CommandResult =
  /** Dispatch accepted; no operation outcome is implied. Observe provider facts. */
  | { status: "accepted"; requestId: string; snapshot: CapabilitySnapshot }
  | { status: "applied"; requestId: string; snapshot: CapabilitySnapshot; receipt?: { captureId?: string } }
  | { status: "refused"; requestId: string; reason: string; snapshot: CapabilitySnapshot }
  | { status: "cancelled"; requestId: string; snapshot: CapabilitySnapshot }
  | { status: "outcome-unknown"; requestId: string; snapshot: CapabilitySnapshot };
export interface RecordClient {
  /** Immediately emits a complete current snapshot, then complete replacements; unsubscribe is observation only. */
  observe(listener: (snapshot: CapabilitySnapshot) => void): () => void;
  execute(command: RecordCommand): Promise<CommandResult>;
  /** Detaches this observer/client only. It never stops or cancels native capture. */
  detach(): void;
}

export { createRecordCapabilityProvider } from "../backend/capabilities/provider.js";
export { hasConfirmedArtifactOpen, PEN_COLOR_RGB } from "./helpers.js";
export type {
  CaptureAction,
  NativeAction,
  RecordCapabilityProvider,
  RecordCapabilityProviderOptions,
  RecordCaptureFacts,
  RecordCaptureFactsPort,
  RecordClipboardPort,
  RecordClipboardResult,
  RecordNativeFacts,
  RecordNativePort,
  RecordPenConfigPort,
} from "../backend/capabilities/provider.js";
