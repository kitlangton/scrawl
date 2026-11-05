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
export interface CaptureOutcome { kind: CaptureOutcomeKind; at: string; ok: boolean; detail?: string }
export interface CaptureFact {
  captureId: string;
  invocationId?: string;
  phase: "starting" | "recording" | "processing" | "saved" | "cancelled" | "failed" | "unknown";
  outcomes: CaptureOutcome[];
}
export interface CapabilitySnapshot {
  version: 1;
  providerEpoch: string;
  revision: number;
  connection: "connected" | "disconnected";
  readiness: { requirements: NativeRequirement[] };
  shortcut: ShortcutBinding;
  pen: PenPreference;
  captures: CaptureFact[];
}

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
