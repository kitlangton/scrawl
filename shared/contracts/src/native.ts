import type { LegacyConfigValue } from './values.js';

/** Public, consumer-neutral view of OS facts owned by the running Desktop GUI. */
export type NativePermissionStatus = 'granted' | 'denied' | 'not-determined' | 'unavailable';
export type NativePermissionAction = 'request-permission' | 'open-system-settings';
export interface NativePermissionFact {
  id: string;
  status: NativePermissionStatus;
  usableNow: boolean;
  relaunchNeeded: boolean;
  actions: NativePermissionAction[];
}
export type NativeShortcutRegistration = 'registered' | 'unregistered' | 'externally-managed' | 'unknown' | 'unavailable';
export interface NativeShortcutFact {
  configured: { keyCode: number; modifiers: number; label: string } | null;
  effective: { keyCode: number; modifiers: number; label: string } | null;
  registration: NativeShortcutRegistration;
  canValidate: boolean;
  canSave: boolean;
}
export interface NativeHostSnapshot {
  api: 1;
  processEpoch: string;
  revision: number;
  scope: 'running-desktop-process';
  freshness: 'cached-gui-observation';
  permissions: NativePermissionFact[];
  shortcuts: { record: NativeShortcutFact; paste: NativeShortcutFact };
}
export type NativeHostRefusal = 'stale' | 'busy' | 'unavailable' | 'invalid' | 'reserved' | 'registration-refused';
export type NativeMutationResult = { status: 'accepted' } | { status: 'refused'; reason: NativeHostRefusal } | { status: 'outcome-unknown' };
/** Fixed operation set. Transport and GUI owner are supplied by privileged composition. */
export interface NativeHostPort {
  readSnapshot(): Promise<NativeHostSnapshot>;
  requestPermission(permissionId: string, expected: { processEpoch: string; revision: number }): Promise<NativeMutationResult>;
  openSystemSettings(permissionId: string, expected: { processEpoch: string; revision: number }): Promise<NativeMutationResult>;
  validateRecordShortcut(key: string, modifiers: number, expected: { processEpoch: string; revision: number }): Promise<NativeMutationResult & { shortcut?: { keyCode: number; modifiers: number; label: string } }>;
  saveRecordShortcut(shortcut: { keyCode: number; modifiers: number; label: string }, expected: { processEpoch: string; revision: number }): Promise<NativeMutationResult>;
}

/** Narrow Record-owned pen effect. RGB is exact 8-bit sRGB; null selects the legacy default. */
export interface NativePenPort {
  applyStrokeColor(rgb: [number, number, number] | null, expectedPenColor: LegacyConfigValue, expected: { processEpoch: string; revision: number }): Promise<NativePenResult>;
  /** Reads the running GUI's telemetry getter; it never reads config or changes the effect. */
  readStrokeColor(): Promise<NativePenSnapshot>;
}
export interface NativePenSnapshot {
  api: 1;
  processEpoch: string;
  scope: 'running-desktop-process';
  rgb: [number, number, number] | null;
}
export type NativePenResult =
  | { status: 'applied'; rgb: [number, number, number] | null }
  | { status: 'conflict'; current: LegacyConfigValue }
  | { status: 'unavailable' | 'outcome-unknown' };

/** Clipboard text limit accounts for worst-case JSON escaping (6 wire bytes per text byte). */
export const MAX_UTF8_TEXT_BYTES = 21_800;
/** Clipboard request line limit includes its terminating LF; other host operations retain 4096 bytes. */
export const MAX_SERIALIZED_FRAME_BYTES = 131_072;
export interface NativeClipboardPort {
  writeText(text: string, observed: { processEpoch: string; revision: number }): Promise<NativeClipboardWriteResult>;
}
export type NativeClipboardWriteResult =
  | { status: 'applied'; digest: string; byteLength: number }
  | { status: 'refused'; reason: 'stale' | 'busy' | 'unavailable' | 'invalid-text' | 'too-large' }
  | { status: 'outcome-unknown' };

/** Record's narrow compatibility access to its compiled `penColor` config key. */
export interface RecordPenColorConfig {
  read(): Promise<LegacyConfigValue>;
  compareAndSwap(expected: LegacyConfigValue, next: LegacyConfigValue): Promise<
    | { status: 'saved' }
    | { status: 'conflict'; current: LegacyConfigValue }
    | { status: 'unknown'; message: string }
  >;
}
