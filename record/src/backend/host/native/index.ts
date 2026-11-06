import type { NativeAction, RecordNativeFacts, RecordNativePort } from '../../capabilities/provider.js';

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
  shortcuts: { record: { configured: NativeShortcutInput | null; effective: NativeShortcutInput | null; registration: 'registered' | 'unregistered' | 'externally-managed' | 'unknown' | 'unavailable'; canValidate: boolean; canSave: boolean }; paste: unknown };
}
interface NativeShortcutInput { keyCode: number; modifiers: number; label: string }

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
    const bit = modifierBits[part];
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
function validSnapshot(value: NativeHostSnapshotInput): boolean {
  return !!value && value.api === 1 && typeof value.processEpoch === 'string' && Number.isSafeInteger(value.revision) && value.revision >= 0
    && value.scope === 'running-desktop-process' && value.freshness === 'cached-gui-observation'
    && Array.isArray(value.permissions) && !!value.shortcuts?.record && !!value.shortcuts?.paste;
}

/** Record policy adapter over an injected privileged public-contract port. It never owns transport or global settings. */
export function createRecordNativePort(port: NativeHostPortInput, options: NativeRecordPortOptions): RecordNativePort {
  const required = [...new Set(options.requiredPermissionIds)];
  if (required.some(id => !permissionIds.has(id))) throw new TypeError('requiredPermissionIds must name supported native permissions');
  let latest: NativeHostSnapshotInput | undefined;
  let latestFacts: RecordNativeFacts | undefined;
  let candidate: Candidate | undefined;
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
    const value = await port.readSnapshot();
    if (!validSnapshot(value)) throw new Error('Native host snapshot unavailable');
    if (detached) throw new Error('Record native adapter detached');
    if (read === sequence) {
      if (!latest || value.processEpoch !== latest.processEpoch || value.revision >= latest.revision) {
        if (latest && (value.processEpoch !== latest.processEpoch || value.revision !== latest.revision)) { candidate = undefined; edit = undefined; }
        latest = structuredClone(value);
        latestFacts = factsFor(latest);
      }
    }
    if (!latestFacts) throw new Error('Native host snapshot unavailable');
    return structuredClone(latestFacts);
  };
  const dispatch = (command: NativeAction, observed?: { providerEpoch: string; revision: number }) => {
    const work = queue.then(async (): Promise<'accepted' | 'refused' | 'cancelled' | 'outcome-unknown'> => {
      if (detached || !latest || !latestFacts || !observed || observed.providerEpoch !== latestFacts.providerEpoch || observed.revision !== latestFacts.revision) return 'refused';
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
        const decoded = decodeShortcut(command.value);
        if (!decoded) { edit = { ...edit, value: command.value, error: 'Use an unambiguous modifier+key shortcut such as Ctrl+Shift+R' }; candidate = undefined; latestFacts = factsFor(latest); notify(); return 'refused'; }
        const result = await port.validateRecordShortcut(decoded.key, decoded.modifiers, state);
        if (detached || !latestFacts || latestFacts.providerEpoch !== observed.providerEpoch || latestFacts.revision !== observed.revision || edit?.sessionId !== command.sessionId) return 'refused';
        if (result.status === 'accepted' && result.shortcut) {
          candidate = { sessionId: edit.sessionId, value: command.value, state: { providerEpoch: observed.providerEpoch, revision: observed.revision }, shortcut: result.shortcut };
          edit = { sessionId: edit.sessionId, value: command.value }; latestFacts = factsFor(latest); notify(); return 'accepted';
        }
        candidate = undefined; edit = { sessionId: edit.sessionId, value: command.value, error: result.status === 'refused' ? `Native validation refused (${result.reason})` : 'Native validation outcome unknown' };
        latestFacts = factsFor(latest); notify(); return result.status === 'refused' ? 'refused' : 'outcome-unknown';
      }
      if (command.type === 'shortcut-save') {
        if (!edit || edit.sessionId !== command.sessionId || !candidate || candidate.sessionId !== command.sessionId || !sameObservedState(candidate.state, observed) || !latest.shortcuts.record.canSave) return 'refused';
        const admitted = candidate; candidate = undefined;
        return mutationStatus(await port.saveRecordShortcut(admitted.shortcut, state));
      }
      if (command.type === 'shortcut-cancel') {
        if (!edit || edit.sessionId !== command.sessionId) return 'refused';
        edit = undefined; candidate = undefined; latestFacts = factsFor(latest); notify(); return 'cancelled';
      }
      return 'refused';
    });
    queue = work.then(() => undefined, () => undefined);
    return work.catch(() => 'outcome-unknown' as const);
  };
  return { readFacts, dispatch, observe(listener) { if (detached) return () => undefined; listeners.add(listener); return () => listeners.delete(listener); },
    dispose() { detached = true; sequence++; candidate = undefined; edit = undefined; listeners.clear(); } };
}
