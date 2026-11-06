import type { StreamModule } from './index.js';
export const HOST_API = 1 as const;

import type { JsonValue, LegacyConfigValue } from './values.js';
export type { JsonValue, LegacyConfigValue } from './values.js';
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

export * from './native.js';
