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
  effective: string | null;
  label: string | null;
  registration: "registered" | "unregistered" | "externally-managed" | "unknown";
  externallyManaged: boolean;
  draft: null | { sessionId: string; value: string; error?: string };
}
export interface PenPreference {
  effective: string;
  supported: string[];
  error?: string;
}
export type CaptureOutcomeKind =
  | "shortcut" | "picker" | "first-frame-start" | "whiteboard" | "accepted-stroke"
  | "processing" | "save" | "copy" | "open-dispatch";
export type CaptureSurface = "screen" | "whiteboard";
export type PickerOutcome =
  | { kind: "picker"; at: string; ok: true; surface: "screen"; target: "display" | "area" | "window" }
  | { kind: "picker"; at: string; ok: false; reason: "cancelled" | "failed" | "unavailable" };
export type AcceptedStrokeOutcome =
  | { kind: "accepted-stroke"; at: string; ok: true; surface: CaptureSurface; strokeId: string }
  | { kind: "accepted-stroke"; at: string; ok: false; surface: CaptureSurface; reason: "rejected" | "failed" };
export type GenericCaptureOutcomeKind = Exclude<CaptureOutcomeKind, "picker" | "accepted-stroke">;
export type ArtifactOpenOutcome = {
  kind: "artifact-open";
  at: string;
  artifactId: string;
} & (
  | { ok: true; evidence: "viewer-ack" | "os-observation" }
  | { ok: false; reason: "refused" | "failed" }
);
export type CaptureOutcome =
  | { kind: GenericCaptureOutcomeKind; at: string; ok: boolean; detail?: string }
  | PickerOutcome
  | AcceptedStrokeOutcome
  | ArtifactOpenOutcome;
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

/** Confirmation for the exact capture AND current saved artifact, never dispatch/watch proof. */
export function hasConfirmedArtifactOpen(snapshot: CapabilitySnapshot, captureId: string): boolean {
  const capture = snapshot.captures.find(value => value.captureId === captureId);
  return capture?.phase === "saved" && !!capture.artifactId && capture.outcomes.some(outcome =>
    outcome.kind === "artifact-open" && outcome.ok && outcome.artifactId === capture.artifactId
  );
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
  | { type: "select-pen"; value: string }
  | { type: "start-capture"; delivery: "local-only" | "host-default" }
  | { type: "arm-next-shortcut"; admissionId: string; delivery: "local-only" | "host-default"; expiresAt: string }
  | { type: "cancel-next-shortcut"; admissionId: string }
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
