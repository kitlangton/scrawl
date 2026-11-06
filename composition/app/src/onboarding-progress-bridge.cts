import { ipcMain, type BrowserWindow, type IpcMainInvokeEvent } from 'electron';
import type { ProgressStore, Revision, SaveResult } from '@apparent/app-contracts/host';

const READ = 'apparent:onboarding-progress:read';
const CAS = 'apparent:onboarding-progress:compare-and-swap';
const MAX_BYTES = 64 * 1024;
const MAX_DEPTH = 32;
const MAX_NODES = 10_000;
const MAX_ACTIVE = 8;
const MAX_WAITING = 16;
// These fixed channels have one registration owner. Only this module may
// register them; disposal releases ownership before a successor can register.
const registrations = new WeakMap<object, symbol>();

function exact(value: unknown, keys: string[]): value is Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  const actual = Reflect.ownKeys(value);
  return actual.length === keys.length && actual.every(key => typeof key === 'string' && keys.includes(key));
}
function revision(value: unknown): value is Revision {
  return (typeof value === 'string' && value.length > 0 && value.length <= 256)
    || (typeof value === 'number' && Number.isSafeInteger(value) && value >= 0);
}
function boundedJson(value: unknown): unknown {
  const ancestors = new Set<object>();
  let nodes = 0;
  let bytes = 0;
  const charge = (text: string) => {
    bytes += Buffer.byteLength(text, 'utf8');
    if (bytes > MAX_BYTES) throw new TypeError('Progress exceeds size limit');
  };
  const visit = (item: unknown, depth: number): void => {
    if (++nodes > MAX_NODES) throw new TypeError('Progress exceeds node limit');
    if (depth > MAX_DEPTH) throw new TypeError('Progress exceeds nesting limit');
    if (item === null || typeof item === 'boolean') { bytes++; return; }
    if (typeof item === 'string') { charge(item); return; }
    if (typeof item === 'number' && Number.isFinite(item)) return;
    if (!item || typeof item !== 'object' || ancestors.has(item)) throw new TypeError('Invalid progress JSON');
    ancestors.add(item);
    const array = Array.isArray(item);
    if (Object.getPrototypeOf(item) !== (array ? Array.prototype : Object.prototype)
      || Object.getOwnPropertySymbols(item).length) throw new TypeError('Invalid progress JSON');
    const keys = Reflect.ownKeys(item);
    if (array && (item.length > MAX_NODES || keys.length !== item.length + 1 || !keys.includes('length'))) throw new TypeError('Invalid progress JSON');
    if (array && keys.filter(key => key !== 'length').some((key, index) => key !== String(index))) throw new TypeError('Progress arrays must be dense');
    for (const key of keys) {
      if (key === 'length' && array) continue;
      if (typeof key !== 'string' || ['__proto__', 'constructor', 'prototype', 'toJSON'].includes(key)) throw new TypeError('Invalid progress JSON key');
      charge(key);
      const descriptor = Object.getOwnPropertyDescriptor(item, key);
      if (!descriptor?.enumerable || !('value' in descriptor)) throw new TypeError('Invalid progress JSON property');
      visit(descriptor.value, depth + 1);
    }
    ancestors.delete(item);
  };
  visit(value, 0);
  const encoded = JSON.stringify(value);
  if (encoded === undefined || Buffer.byteLength(encoded, 'utf8') > MAX_BYTES) throw new TypeError('Progress exceeds size limit');
  return JSON.parse(encoded);
}

/** Registers only the fixed onboarding progress operations for one exact window. */
export function registerOnboardingProgressBridge<T>(options: {
  window: BrowserWindow;
  entryUrl: string;
  store: ProgressStore<T>;
}): () => void {
  if (registrations.has(ipcMain)) throw new Error('Onboarding progress bridge already registered');
  const token = Symbol('onboarding-progress');
  const contents = options.window.webContents;
  let active = true;
  let revokePromise: Promise<void>;
  let revokeNow!: () => void;
  revokePromise = new Promise(resolve => { revokeNow = resolve; });
  let activeCount = 0;
  let waitingCount = 0;
  const queue: Array<() => void> = [];
  const acquire = async () => {
    if (!active) throw new Error('Onboarding progress request revoked');
    if (activeCount < MAX_ACTIVE) { activeCount++; return; }
    if (waitingCount >= MAX_WAITING) throw new Error('Onboarding progress provider busy');
    waitingCount++;
    await new Promise<void>((resolve, reject) => queue.push(() => {
      waitingCount--;
      if (!active) { reject(new Error('Onboarding progress request revoked')); return; }
      activeCount++;
      resolve();
    }));
  };
  const release = () => {
    activeCount--;
    queue.shift()?.();
    if (!activeCount && !active) scheduleHandlerRemoval();
  };
  const allowed = (event: IpcMainInvokeEvent) => {
    try {
      if (!active || options.window.isDestroyed() || contents.isDestroyed()) return false;
      const frame = event.senderFrame;
      return event.sender === contents && !!frame && frame === contents.mainFrame && frame.url === options.entryUrl;
    } catch { return false; } // Frame/window destruction can invalidate native getters.
  };
  const read = async (event: IpcMainInvokeEvent, ...args: unknown[]) => {
    if (args.length || !allowed(event)) throw new Error('Onboarding progress request refused');
    await acquire();
    try {
      if (!allowed(event)) throw new Error('Onboarding progress request revoked');
      const result = await Promise.race([
        options.store.read(),
        revokePromise.then(() => { throw new Error('Onboarding progress request revoked'); }),
      ]);
      if (!active || !allowed(event)) throw new Error('Onboarding progress request revoked');
      if (!revision(result.revision)) throw new TypeError('Invalid progress revision');
      return { revision: result.revision, value: boundedJson(result.value) };
    } finally { release(); }
  };
  const compareAndSwap = async (event: IpcMainInvokeEvent, ...args: unknown[]) => {
    if (args.length !== 1 || !allowed(event) || !exact(args[0], ['expected', 'value']) || !revision(args[0].expected)) {
      throw new TypeError('Invalid onboarding progress request');
    }
    const value = boundedJson(args[0].value) as T;
    const expectedRevision = args[0].expected as Revision;
    let acquired = false;
    try {
      await acquire();
      acquired = true;
      let result: SaveResult<T>;
      const revoked = '__revoked__' as const;
      const write = Promise.resolve().then(() => {
        if (!allowed(event)) throw new Error('Onboarding progress request revoked before dispatch');
        return options.store.compareAndSwap(expectedRevision, value);
      });
      write.catch(() => {}); // The provider may finish after revocation; its result is intentionally ignored.
      let raced: SaveResult<T> | typeof revoked;
      try { raced = await Promise.race([write, revokePromise.then(() => revoked)]); }
      catch { return { status: 'unknown', message: 'Configuration outcome unknown; read before retrying.' }; }
      if (raced === revoked) return { status: 'unknown', message: 'Configuration outcome unknown; read before retrying.' };
      result = raced;
      if (!active || !allowed(event)) return { status: 'unknown', message: 'Configuration outcome unknown; read before retrying.' };
      if (!result || !exact(result, result.status === 'conflict' ? ['status', 'revision', 'current'] : result.status === 'unknown' ? ['status', 'message'] : ['status', 'revision'])) {
        return { status: 'unknown', message: 'Malformed configuration result; read before retrying.' };
      }
      if (result.status === 'conflict') {
        if (!revision(result.revision)) return { status: 'unknown', message: 'Malformed configuration result; read before retrying.' };
        try { return { status: 'conflict', revision: result.revision, current: boundedJson(result.current) }; }
        catch { return { status: 'unknown', message: 'Malformed configuration result; read before retrying.' }; }
      }
      if (result.status === 'unknown') return typeof result.message === 'string' && result.message.length > 0
        ? { status: 'unknown', message: result.message.slice(0, 512) }
        : { status: 'unknown', message: 'Configuration outcome unknown; read before retrying.' };
      if (result.status !== 'saved' || !revision(result.revision)) return { status: 'unknown', message: 'Malformed configuration result; read before retrying.' };
      return { status: 'saved', revision: result.revision };
    } catch {
      return { status: 'unknown', message: 'Configuration outcome unknown; read before retrying.' };
    } finally { if (acquired) release(); }
  };
  ipcMain.handle(READ, read);
  try { ipcMain.handle(CAS, compareAndSwap); }
  catch (error) { ipcMain.removeHandler(READ); active = false; throw error; }
  registrations.set(ipcMain, token);
  let handlersRemoved = false;
  let handlerRemovalScheduled = false;
  const removeOwnedHandlers = () => {
    if (handlersRemoved || activeCount) return;
    handlersRemoved = true;
    if (registrations.get(ipcMain) !== token) return;
    registrations.delete(ipcMain);
    ipcMain.removeHandler(READ);
    ipcMain.removeHandler(CAS);
  };
  const scheduleHandlerRemoval = () => {
    if (handlerRemovalScheduled || handlersRemoved) return;
    handlerRemovalScheduled = true;
    setImmediate(() => { handlerRemovalScheduled = false; removeOwnedHandlers(); });
  };
  const revoke = () => {
    if (!active) return;
    active = false;
    revokeNow();
    for (const wake of queue.splice(0)) wake();
    if (!activeCount) removeOwnedHandlers();
  };
  const dispose = () => {
    revoke();
    options.window.removeListener('closed', dispose);
    contents.removeListener('will-navigate', dispose);
    contents.removeListener('destroyed', dispose);
    // Retire our registration immediately. Deferred completion/removal from
    // this generation cannot remove a successor's handlers.
    if (registrations.get(ipcMain) === token) {
      registrations.delete(ipcMain);
      ipcMain.removeHandler(READ);
      ipcMain.removeHandler(CAS);
    }
  };
  options.window.once('closed', dispose);
  contents.on('will-navigate', dispose);
  contents.once('destroyed', dispose);
  return dispose;
}
