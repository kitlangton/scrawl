import type { StreamModule } from './index.js';
export const HOST_API = 1 as const;

export type JsonValue = null | boolean | number | string | JsonValue[] | { [key: string]: JsonValue };
export type Revision = string | number;
export type SaveResult<T> =
  | { status: 'saved'; revision: Revision }
  | { status: 'conflict'; revision: Revision; current: T }
  | { status: 'unknown'; message: string };

/** Mirrors Onboarding's progress contract. Schema and legacy mapping stay owner-owned. */
export interface ProgressStore<T> {
  read(): Promise<{ revision: Revision; value: T }>;
  compareAndSwap(expected: Revision, value: T): Promise<SaveResult<T>>;
}
/** Renderer-safe, owner-scoped capability injected by trusted composition. */
export interface OnboardingProgressPort {
  read(): Promise<{ revision: Revision; value: unknown }>;
  compareAndSwap(expected: Revision, value: unknown): Promise<SaveResult<unknown>>;
}
export interface ProgressCodec<T> {
  /** Owner handles its stored value here; legacy adoption uses the separate snapshot API. */
  decode(value: JsonValue | null): T;
  encode(value: T): JsonValue;
}

export type ActivationReason = 'firstLaunch' | 'settings' | 'resume';
export interface ActivationContext { reason: ActivationReason }
export interface ActivationHooks {
  activate(context: ActivationContext): Promise<void>;
  dispose(): Promise<void>;
}
export interface HostedContribution {
  module: StreamModule;
  activation: ActivationHooks;
}
/** Dependencies and their shape are owned by the stream that declares this factory. */
export interface ContributionFactory<Dependencies> {
  readonly ownerId: string;
  create(dependencies: Dependencies): Promise<HostedContribution>;
}

export type CoachingAction = { id: string; label: string };
export interface CoachingView<ViewId extends string = string> {
  id: ViewId;
  placement: 'inline' | 'corner' | 'modal';
  title: string;
  body: string;
  actions: CoachingAction[];
}
export type HostEffectResult = { status: 'ok' } | { status: 'unavailable'; capability: string };
export interface CoachingHost<ViewId extends string = string> {
  show(view: CoachingView<ViewId>): Promise<HostEffectResult>;
  update(view: CoachingView<ViewId>): Promise<HostEffectResult>;
  hide(id: ViewId): Promise<HostEffectResult>;
}

/** Runtime guard for a main-side provider using its compiled-in owner view allowlist. */
export function validateCoachingView<ViewId extends string>(
  value: unknown,
  allowedIds: readonly ViewId[],
): CoachingView<ViewId> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new TypeError('Invalid coaching view');
  const view = value as Record<string, unknown>;
  if (Object.keys(view).length !== 5 || !['id', 'placement', 'title', 'body', 'actions'].every(key => Object.hasOwn(view, key))) {
    throw new TypeError('Invalid coaching view fields');
  }
  if (typeof view.id !== 'string' || !allowedIds.includes(view.id as ViewId)) throw new TypeError('Coaching view id is not allowed');
  if (view.placement !== 'inline' && view.placement !== 'corner' && view.placement !== 'modal') throw new TypeError('Invalid coaching placement');
  if (typeof view.title !== 'string' || view.title.length < 1 || view.title.length > 100
    || typeof view.body !== 'string' || view.body.length > 500) throw new TypeError('Coaching text exceeds limits');
  if (!Array.isArray(view.actions) || view.actions.length > 4) throw new TypeError('Invalid coaching actions');
  const actionIds = new Set<string>();
  const actions = view.actions.map(action => {
    if (!action || typeof action !== 'object' || Array.isArray(action)) throw new TypeError('Invalid coaching action');
    const item = action as Record<string, unknown>;
    if (Object.keys(item).length !== 2 || typeof item.id !== 'string' || !/^[a-z][a-z0-9-]{0,31}$/.test(item.id)
      || typeof item.label !== 'string' || item.label.length < 1 || item.label.length > 48 || actionIds.has(item.id)) {
      throw new TypeError('Invalid coaching action fields');
    }
    actionIds.add(item.id);
    return { id: item.id, label: item.label };
  });
  return { id: view.id as ViewId, placement: view.placement, title: view.title, body: view.body, actions };
}

/** Production implementations must make each read/CAS one authoritative atomic operation. */
export interface TransactionalConfigBackend {
  read(ownerId: string): Promise<{ revision: Revision; value: JsonValue | null }>;
  compareAndSwap(ownerId: string, expected: Revision, value: JsonValue): Promise<
    | { status: 'saved'; revision: Revision }
    | { status: 'conflict'; revision: Revision; current: JsonValue | null }
    | { status: 'unknown'; message: string }
  >;
}

/** Absent and stored null must remain distinguishable during one-time adoption. */
export interface LegacyConfigValue { present: boolean; value: JsonValue }
export interface ConfigAdoptionSnapshot {
  owner: { present: boolean; revision: number; value: JsonValue };
  legacy: LegacyConfigValue;
}
export type ConfigAdoptionResult =
  | { status: 'saved'; revision: number }
  | { status: 'conflict'; snapshot: ConfigAdoptionSnapshot }
  | { status: 'unknown'; message: string };

/** Legacy key is bound to ownerId; interpretation and installed cutover remain owner-owned. */
export interface LegacyAdoptionBackend extends TransactionalConfigBackend {
  readForAdoption(ownerId: string): Promise<ConfigAdoptionSnapshot>;
  /** Only creates an absent owner entry if the legacy presence/value still match. */
  adoptLegacy(ownerId: string, expectedLegacy: LegacyConfigValue, value: JsonValue): Promise<ConfigAdoptionResult>;
}

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

/** Record's narrow compatibility access to its compiled `penColor` config key. */
export interface RecordPenColorConfig {
  read(): Promise<LegacyConfigValue>;
  compareAndSwap(expected: LegacyConfigValue, next: LegacyConfigValue): Promise<
    | { status: 'saved' }
    | { status: 'conflict'; current: LegacyConfigValue }
    | { status: 'unknown'; message: string }
  >;
}
