import { contextBridge, ipcRenderer } from 'electron';
import type { OnboardingProgressPort, Revision, SaveResult } from '@apparent/app-contracts/host';

const MAX_BYTES = 64 * 1024;
const MAX_DEPTH = 32;
const MAX_NODES = 10_000;
const READ_TIMEOUT_MS = 10_000;
const CAS_TIMEOUT_MS = 8_000;
const utf8 = (value: string) => new TextEncoder().encode(value).length;
const validRevision = (value: unknown): value is Revision =>
  (typeof value === 'string' && value.length > 0 && value.length <= 256)
  || (typeof value === 'number' && Number.isSafeInteger(value) && value >= 0);
function json(value: unknown): unknown {
  const ancestors = new Set<object>(); let nodes = 0; let bytes = 0;
  const visit = (item: unknown, depth: number) => {
    if (++nodes > MAX_NODES || depth > MAX_DEPTH) throw new TypeError('Invalid progress value');
    if (item === null || typeof item === 'boolean') { bytes++; return; }
    if (typeof item === 'string') bytes += utf8(item);
    else if (typeof item === 'number' && Number.isFinite(item)) return;
    else {
      if (!item || typeof item !== 'object' || ancestors.has(item)) throw new TypeError('Invalid progress value');
      const array = Array.isArray(item);
      if (Object.getPrototypeOf(item) !== (array ? Array.prototype : Object.prototype) || Object.getOwnPropertySymbols(item).length) throw new TypeError('Invalid progress value');
      const keys = Reflect.ownKeys(item);
      if (array && (item.length > MAX_NODES || keys.length !== item.length + 1 || !keys.includes('length'))) throw new TypeError('Invalid progress value');
      if (array && keys.filter(key => key !== 'length').some((key, index) => key !== String(index))) throw new TypeError('Progress arrays must be dense');
      ancestors.add(item);
      for (const key of keys) {
        if (key === 'length' && array) continue;
        if (typeof key !== 'string' || ['__proto__', 'constructor', 'prototype', 'toJSON'].includes(key)) throw new TypeError('Invalid progress value');
        bytes += utf8(key);
        const descriptor = Object.getOwnPropertyDescriptor(item, key);
        if (!descriptor?.enumerable || !('value' in descriptor)) throw new TypeError('Invalid progress value');
        visit(descriptor.value, depth + 1);
      }
      ancestors.delete(item);
    }
    if (bytes > MAX_BYTES) throw new TypeError('Progress value exceeds size limit');
  };
  visit(value, 0);
  const encoded = JSON.stringify(value);
  if (encoded === undefined || utf8(encoded) > MAX_BYTES) throw new TypeError('Progress value exceeds size limit');
  return JSON.parse(encoded);
}
function exact(value: unknown, keys: string[]): value is Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const actual = Reflect.ownKeys(value);
  return actual.length === keys.length && actual.every(key => typeof key === 'string' && keys.includes(key));
}
function casReply(value: unknown): SaveResult<unknown> {
  if (exact(value, ['status', 'revision']) && value.status === 'saved' && validRevision(value.revision)) return { status: 'saved', revision: value.revision };
  if (exact(value, ['status', 'revision', 'current']) && value.status === 'conflict' && validRevision(value.revision)) {
    try { return { status: 'conflict', revision: value.revision, current: json(value.current) }; }
    catch { /* malformed reply after dispatch has an uncertain write outcome */ }
  }
  if (exact(value, ['status', 'message']) && value.status === 'unknown' && typeof value.message === 'string' && value.message.length > 0) {
    return { status: 'unknown', message: value.message.slice(0, 512) };
  }
  return { status: 'unknown', message: 'Configuration outcome unknown; read before retrying.' };
}
function invoke(channel: string, args: unknown[], timeoutMs: number): Promise<unknown> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('Onboarding progress transport timed out')), timeoutMs);
    ipcRenderer.invoke(channel, ...args).then(value => { clearTimeout(timer); resolve(value); }, error => { clearTimeout(timer); reject(error); });
  });
}

const progress: OnboardingProgressPort = Object.freeze({
  async read(...extra: unknown[]) {
    if (extra.length) throw new TypeError('Invalid progress read request');
    const result: unknown = await invoke('apparent:onboarding-progress:read', [], READ_TIMEOUT_MS);
    if (!exact(result, ['revision', 'value']) || !validRevision(result.revision)) throw new Error('Onboarding progress unavailable');
    return { revision: result.revision, value: json(result.value) };
  },
  async compareAndSwap(expected: Revision, value: unknown, ...extra: unknown[]): Promise<SaveResult<unknown>> {
    if (extra.length) throw new TypeError('Invalid progress write request');
    if (!validRevision(expected)) throw new TypeError('Invalid expected revision');
    const safeValue = json(value); // Reject locally; malformed data never reaches IPC.
    try {
      return casReply(await invoke('apparent:onboarding-progress:compare-and-swap', [{ expected, value: safeValue }], CAS_TIMEOUT_MS));
    } catch {
      // Once invoke is dispatched, rejection cannot distinguish a lost reply
      // from a write that did not happen. Never retry here.
      return { status: 'unknown' as const, message: 'Configuration outcome unknown; read before retrying.' };
    }
  },
});
const api = Object.freeze({ onboardingProgress: progress });
contextBridge.exposeInMainWorld('apparent', api);
