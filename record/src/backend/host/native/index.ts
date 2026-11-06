import { isDeepStrictEqual } from "node:util";
import type { NativeAction, RecordNativeFacts, RecordNativePort, RecordPenConfigPort } from '../../capabilities/provider.js';
import type { RecordNativeHostInput, RecordNativePenInput, RecordPenColorConfigInput } from './pen.js';
export type { RecordNativeHostInput, RecordNativePenInput, RecordPenColorConfigInput } from './pen.js';

/** Lazily loads the optional pen adapter so source-level native host tests stay independent of it. */
export function createRecordPenConfigPort(input: {
  config: RecordPenColorConfigInput;
  nativePen: RecordNativePenInput;
  nativeHost: RecordNativeHostInput;
}): RecordPenConfigPort {
  let adapter: Promise<RecordPenConfigPort> | undefined;
  const get = () => adapter ??= import('./pen.js').then(module => module.createRecordPenConfigPort(input));
  return {
    read: async () => (await get()).read(),
    writeApplyAndReadBack: async color => (await get()).writeApplyAndReadBack(color),
  };
}

/** Local structural consumer seam; kept type-only so headless Record has no host-contract dependency. */
export interface NativeHostPortInput {
  readSnapshot(): Promise<NativeHostSnapshotInput>;
  requestPermission(permissionId: string, expected: NativeState): Promise<NativeMutationResultInput>;
  openSystemSettings(permissionId: string, expected: NativeState): Promise<NativeMutationResultInput>;
  validateRecordShortcut(key: string, modifiers: number, expected: NativeState): Promise<NativeMutationResultInput & { shortcut?: { keyCode: number; modifiers: number; label: string } }>;
  saveRecordShortcut(shortcut: { keyCode: number; modifiers: number; label: string }, expected: NativeState): Promise<NativeMutationResultInput>;
}
interface NativeState { processEpoch: string; revision: number }
type NativeMutationResultInput = { status: 'accepted' } | { status: 'outcome-unknown' } | { status: 'refused'; reason: string };
type NativeMutationResult = NativeMutationResultInput;
interface NativeHostSnapshotInput {
  api: 1; processEpoch: string; revision: number; scope: 'running-desktop-process'; freshness: 'cached-gui-observation';
  permissions: Array<{ id: string; status: 'granted' | 'denied' | 'not-determined' | 'unavailable'; usableNow: boolean; relaunchNeeded: boolean; actions: Array<'request-permission' | 'open-system-settings'> }>;
  shortcuts: { record: NativeShortcutFactInput; paste: NativeShortcutFactInput };
}
interface NativeShortcutInput { keyCode: number; modifiers: number; label: string }
interface NativeShortcutFactInput { configured: NativeShortcutInput | null; effective: NativeShortcutInput | null; registration: 'registered' | 'unregistered' | 'externally-managed' | 'unknown' | 'unavailable'; canValidate: boolean; canSave: boolean }

type Candidate = { sessionId: string; value: string; state: { providerEpoch: string; revision: number }; shortcut: { keyCode: number; modifiers: number; label: string } };
export interface NativeRecordPortOptions {
  /** Trusted Record composition declares which OS permission facts this workflow requires. */
  requiredPermissionIds: readonly string[];
}

const permissionIds = new Set(['screen-recording', 'microphone', 'accessibility', 'camera', 'speech-recognition']);
const modifierBits: Record<string, number> = { Ctrl: 4096, Control: 4096, Shift: 512, Alt: 2048, Option: 2048, Meta: 256, Command: 256 };
function decodeShortcut(value: string): { key: string; modifiers: number } | undefined {
  if (!value || value.length > 80) return undefined;
  const parts = value.split('+');
  if (parts.length < 2 || parts.some(part => !part || part.trim() !== part)) return undefined;
  const key = parts.pop()!;
  if (!/^[a-zA-Z0-9]{1,16}$/.test(key)) return undefined;
  let modifiers = 0;
  for (const part of parts) {
    const bit = Object.hasOwn(modifierBits, part) ? modifierBits[part] : undefined;
    if (bit === undefined || (modifiers & bit) !== 0) return undefined;
    modifiers |= bit;
  }
  return { key, modifiers };
}
function mutationStatus(result: NativeMutationResult): 'accepted' | 'refused' | 'outcome-unknown' {
  return result.status === 'accepted' ? 'accepted' : result.status === 'refused' ? 'refused' : 'outcome-unknown';
}
function sameState(a: NativeState | undefined, b: NativeState): boolean {
  return !!a && a.processEpoch === b.processEpoch && a.revision === b.revision;
}
function sameObservedState(a: Candidate['state'] | undefined, b: Candidate['state']): boolean {
  return !!a && a.providerEpoch === b.providerEpoch && a.revision === b.revision;
}
function object(value: unknown): value is Record<string, unknown> { return !!value && typeof value === 'object' && !Array.isArray(value); }
function validShortcut(value: unknown): value is NativeShortcutInput {
  return object(value) && Number.isInteger(value.keyCode) && Number(value.keyCode) >= 0 && Number(value.keyCode) <= 255 &&
    Number.isInteger(value.modifiers) && Number(value.modifiers) >= 0 && Number(value.modifiers) <= 0xffff &&
    typeof value.label === 'string' && value.label.length > 0 && Buffer.byteLength(value.label) <= 32;
}
function validShortcutFact(value: unknown): value is NativeShortcutFactInput {
  return object(value) && (value.configured === null || validShortcut(value.configured)) && (value.effective === null || validShortcut(value.effective)) &&
    typeof value.registration === 'string' && ['registered','unregistered','externally-managed','unknown','unavailable'].includes(value.registration) &&
    typeof value.canValidate === 'boolean' && typeof value.canSave === 'boolean' && (!value.canSave || value.registration === 'registered');
}
function validSnapshot(value: unknown): value is NativeHostSnapshotInput {
  if (!object(value) || value.api !== 1 || typeof value.processEpoch !== 'string' || !/^[a-zA-Z0-9-]{1,64}$/.test(value.processEpoch) ||
      !Number.isSafeInteger(value.revision) || Number(value.revision) < 0 || value.scope !== 'running-desktop-process' || value.freshness !== 'cached-gui-observation' ||
      !Array.isArray(value.permissions) || value.permissions.length > permissionIds.size || !object(value.shortcuts) ||
      !validShortcutFact(value.shortcuts.record) || !validShortcutFact(value.shortcuts.paste)) return false;
  const ids = new Set<string>();
  return Array.from(value.permissions).every(item => {
    if (!object(item) || typeof item.id !== 'string' || !permissionIds.has(item.id) || ids.has(item.id) ||
        typeof item.status !== 'string' || !['granted','denied','not-determined','unavailable'].includes(item.status) || typeof item.usableNow !== 'boolean' || typeof item.relaunchNeeded !== 'boolean' ||
        !Array.isArray(item.actions) || item.actions.length > 2 || new Set(item.actions).size !== item.actions.length ||
        !Array.from(item.actions).every(action => action === 'request-permission' || action === 'open-system-settings')) return false;
    ids.add(item.id); return true;
  });
}

/** Record policy adapter over an injected privileged public-contract port. It never owns transport or global settings. */
export function createRecordNativePort(port: NativeHostPortInput, options: NativeRecordPortOptions): RecordNativePort {
  const required = [...new Set(options.requiredPermissionIds)];
  if (required.some(id => !permissionIds.has(id))) throw new TypeError('requiredPermissionIds must name supported native permissions');
  let latest: NativeHostSnapshotInput | undefined;
  let latestFacts: RecordNativeFacts | undefined;
  let observationAvailable = false;
  let conflictingObservation: NativeState | undefined;
  let candidate: Candidate | undefined;
  let awaitingSaveReadback: { sessionId: string; shortcut: NativeShortcutInput } | undefined;
  const retiredEpochs = new Set<string>();
  let edit: { sessionId: string; value: string; error?: string } | undefined;
  let detached = false;
  let sequence = 0;
  let queue = Promise.resolve();
  const listeners = new Set<() => void>();
  const notify = () => { for (const listener of [...listeners]) if (listeners.has(listener)) { try { listener(); } catch { listeners.delete(listener); } } };
  const factsFor = (snapshot: NativeHostSnapshotInput): RecordNativeFacts => {
    const permissions = new Map(snapshot.permissions.map(item => [item.id, item]));
    const readiness = [...permissions.values()].map(item => ({ id: item.id, required: required.includes(item.id), status: item.status,
      usableNow: item.usableNow, relaunchNeeded: item.relaunchNeeded, allowedRepairActions: [...item.actions] }));
    for (const id of required) if (!permissions.has(id)) readiness.push({ id, required: true, status: 'unavailable' as const, usableNow: false, relaunchNeeded: false, allowedRepairActions: [] });
    const record = snapshot.shortcuts.record;
    const registration = record.registration;
    const effective = record.effective?.label ?? null;
    const configured = record.configured?.label ?? null;
    return { version: 1, providerEpoch: snapshot.processEpoch, revision: snapshot.revision, readiness,
      shortcut: { configured, effective, label: effective, registration, externallyManaged: registration === 'externally-managed', draft: edit ?? null } };
  };
  const readFacts = async (): Promise<RecordNativeFacts> => {
    const read = ++sequence;
    try {
      const value = structuredClone(await port.readSnapshot());
      if (detached) throw new Error('Record native adapter detached');
      if (read !== sequence) {
        if (!observationAvailable || !latestFacts) throw new Error('Native host snapshot unavailable');
        return structuredClone(latestFacts);
      }
      if (!validSnapshot(value)) throw new Error('Native host snapshot unavailable');
      if (retiredEpochs.has(value.processEpoch)) throw new Error('Native host returned a retired process observation');
      if (latest && value.processEpoch === latest.processEpoch && value.revision < latest.revision) throw new Error('Native host returned a stale observation');
      if (latest && sameState(latest, value) && !isDeepStrictEqual(latest, value)) {
        conflictingObservation = {processEpoch: value.processEpoch, revision: value.revision};
        candidate = undefined; edit = undefined; awaitingSaveReadback = undefined;
        throw new Error('Native host changed facts without a new observation');
      }
      if (sameState(conflictingObservation, value)) throw new Error('Native host observation is inconsistent');
      if (!latest || !sameState(latest, value)) conflictingObservation = undefined;
      if (latest && value.processEpoch !== latest.processEpoch) retiredEpochs.add(latest.processEpoch);
      if (latest && !sameState(latest, value)) { candidate = undefined; edit = undefined; awaitingSaveReadback = undefined; }
      const saved = awaitingSaveReadback;
      const record = value.shortcuts.record;
      const matches = (actual: NativeShortcutInput | null) => !!actual && actual.keyCode === saved?.shortcut.keyCode && actual.modifiers === saved.shortcut.modifiers && actual.label === saved.shortcut.label;
      if (saved && edit?.sessionId === saved.sessionId && record.registration === 'registered' && matches(record.configured) && matches(record.effective)) {
        edit = undefined; awaitingSaveReadback = undefined;
      }
      latest = value; latestFacts = factsFor(latest); observationAvailable = true;
      return structuredClone(latestFacts);
    } catch (error) {
      if (read === sequence) observationAvailable = false;
      throw error;
    }
  };
  const dispatch = (input: NativeAction, observation?: { providerEpoch: string; revision: number }) => {
    const command = structuredClone(input);
    const observed = structuredClone(observation);
    const work = queue.then(async (): Promise<'accepted' | 'refused' | 'cancelled' | 'outcome-unknown'> => {
      if (detached || !observationAvailable || conflictingObservation || !latest || !latestFacts || !observed || observed.providerEpoch !== latestFacts.providerEpoch || observed.revision !== latestFacts.revision) return 'refused';
      const state = { processEpoch: observed.providerEpoch, revision: observed.revision };
      const permission = command.type === 'request-permission' || command.type === 'open-system-settings' ? latest.permissions.find(item => item.id === command.requirementId) : undefined;
      if (command.type === 'request-permission') {
        if (!permission?.actions.includes('request-permission')) return 'refused';
        return mutationStatus(await port.requestPermission(command.requirementId, state));
      }
      if (command.type === 'open-system-settings') {
        if (!permission?.actions.includes('open-system-settings')) return 'refused';
        return mutationStatus(await port.openSystemSettings(command.requirementId, state));
      }
      if (command.type === 'shortcut-begin-edit') {
        if (edit || !latest.shortcuts.record.canValidate || latest.shortcuts.record.registration === 'externally-managed') return 'refused';
        edit = { sessionId: globalThis.crypto.randomUUID(), value: '' }; candidate = undefined; latestFacts = factsFor(latest); notify(); return 'accepted';
      }
      if (command.type === 'shortcut-submit') {
        if (!edit || edit.sessionId !== command.sessionId || !latest.shortcuts.record.canValidate) return 'refused';
        awaitingSaveReadback = undefined; // A new explicit edit supersedes only the local pending display, never the admitted effect.
        candidate = undefined;
        edit = { sessionId: edit.sessionId, value: command.value };
        const decoded = decodeShortcut(command.value);
        if (!decoded) { edit = { ...edit, value: command.value, error: 'Use an unambiguous modifier+key shortcut such as Ctrl+Shift+R' }; candidate = undefined; latestFacts = factsFor(latest); notify(); return 'refused'; }
        let result: Awaited<ReturnType<NativeHostPortInput['validateRecordShortcut']>>;
        try { result = await port.validateRecordShortcut(decoded.key, decoded.modifiers, state); }
        catch { result = {status:'outcome-unknown'}; }
        if (detached || !latestFacts || latestFacts.providerEpoch !== observed.providerEpoch || latestFacts.revision !== observed.revision || edit?.sessionId !== command.sessionId) return 'refused';
        if (result.status === 'accepted' && validShortcut(result.shortcut)) {
          candidate = { sessionId: edit.sessionId, value: command.value, state: { providerEpoch: observed.providerEpoch, revision: observed.revision }, shortcut: structuredClone(result.shortcut) };
          edit = { sessionId: edit.sessionId, value: command.value }; latestFacts = factsFor(latest); notify(); return 'accepted';
        }
        candidate = undefined; edit = { sessionId: edit.sessionId, value: command.value, error: result.status === 'refused' ? `Native validation refused (${result.reason})` : 'Native validation outcome unknown' };
        latestFacts = factsFor(latest); notify(); return result.status === 'refused' ? 'refused' : 'outcome-unknown';
      }
      if (command.type === 'shortcut-save') {
        if (!edit || edit.sessionId !== command.sessionId || !candidate || candidate.sessionId !== command.sessionId || !sameObservedState(candidate.state, observed) || !latest.shortcuts.record.canSave) return 'refused';
        const admitted = candidate; candidate = undefined;
        let result: NativeMutationResultInput;
        try { result = await port.saveRecordShortcut(structuredClone(admitted.shortcut), state); }
        catch { result = {status:'outcome-unknown'}; }
        if (!detached && edit?.sessionId === admitted.sessionId) {
          if (result.status === 'accepted') awaitingSaveReadback = { sessionId: admitted.sessionId, shortcut: admitted.shortcut };
          edit = { ...edit, error: result.status === 'accepted' ? 'Awaiting native shortcut save readback' : result.status === 'refused' ? `Native save refused (${result.reason})` : 'Native save outcome unknown' };
          latestFacts = factsFor(latest); notify();
        }
        return mutationStatus(result);
      }
      if (command.type === 'shortcut-cancel') {
        if (!edit || edit.sessionId !== command.sessionId) return 'refused';
        edit = undefined; candidate = undefined; awaitingSaveReadback = undefined; latestFacts = factsFor(latest); notify(); return 'cancelled';
      }
      return 'refused';
    });
    queue = work.then(() => undefined, () => undefined);
    return work.catch(() => 'outcome-unknown' as const);
  };
  return { readFacts, dispatch, observe(listener) { if (detached) return () => undefined; listeners.add(listener); return () => listeners.delete(listener); },
    dispose() { detached = true; observationAvailable = false; sequence++; candidate = undefined; edit = undefined; awaitingSaveReadback = undefined; listeners.clear(); } };
}
