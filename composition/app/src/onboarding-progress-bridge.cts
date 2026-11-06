import { ipcMain, type BrowserWindow, type IpcMainInvokeEvent } from 'electron';
import type { ProgressStore, Revision, SaveResult } from '@apparent/app-contracts/host';

const READ = 'apparent:onboarding-progress:read';
const CAS = 'apparent:onboarding-progress:compare-and-swap';
const MAX_BYTES = 64 * 1024;
const MAX_DEPTH = 32;

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
  const seen = new Set<object>();
  const visit = (item: unknown, depth: number): void => {
    if (depth > MAX_DEPTH) throw new TypeError('Progress exceeds nesting limit');
    if (item === null || typeof item === 'string' || typeof item === 'boolean') return;
    if (typeof item === 'number' && Number.isFinite(item)) return;
    if (!item || typeof item !== 'object' || seen.has(item)) throw new TypeError('Invalid progress JSON');
    seen.add(item);
    const array = Array.isArray(item);
    if (Object.getPrototypeOf(item) !== (array ? Array.prototype : Object.prototype)
      || Object.getOwnPropertySymbols(item).length) throw new TypeError('Invalid progress JSON');
    const keys = Reflect.ownKeys(item);
    if (array && (keys.length !== item.length + 1 || !keys.includes('length'))) throw new TypeError('Invalid progress JSON');
    for (const key of keys) {
      if (key === 'length' && array) continue;
      if (typeof key !== 'string' || ['__proto__', 'constructor', 'prototype', 'toJSON'].includes(key)) throw new TypeError('Invalid progress JSON key');
      const descriptor = Object.getOwnPropertyDescriptor(item, key);
      if (!descriptor?.enumerable || !('value' in descriptor)) throw new TypeError('Invalid progress JSON property');
      visit(descriptor.value, depth + 1);
    }
    seen.delete(item);
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
  let active = true;
  const allowed = (event: IpcMainInvokeEvent) => {
    const frame = event.senderFrame;
    return active && !options.window.isDestroyed() && event.sender === options.window.webContents
      && !!frame && frame === options.window.webContents.mainFrame && frame.url === options.entryUrl;
  };
  const read = async (event: IpcMainInvokeEvent, ...args: unknown[]) => {
    if (args.length || !allowed(event)) throw new Error('Onboarding progress request refused');
    const result = await options.store.read();
    if (!active || !allowed(event)) throw new Error('Onboarding progress request revoked');
    if (!revision(result.revision)) throw new TypeError('Invalid progress revision');
    return { revision: result.revision, value: boundedJson(result.value) };
  };
  const compareAndSwap = async (event: IpcMainInvokeEvent, ...args: unknown[]) => {
    if (args.length !== 1 || !allowed(event) || !exact(args[0], ['expected', 'value']) || !revision(args[0].expected)) {
      throw new TypeError('Invalid onboarding progress request');
    }
    const value = boundedJson(args[0].value) as T;
    const result: SaveResult<T> = await options.store.compareAndSwap(args[0].expected, value);
    if (!active || !allowed(event)) throw new Error('Onboarding progress request revoked');
    if (!result || !exact(result, result.status === 'conflict' ? ['status', 'revision', 'current'] : result.status === 'unknown' ? ['status', 'message'] : ['status', 'revision'])) {
      throw new TypeError('Invalid onboarding progress result');
    }
    if (result.status === 'conflict') return { status: 'conflict', revision: revision(result.revision) ? result.revision : (() => { throw new TypeError('Invalid revision'); })(), current: boundedJson(result.current) };
    if (result.status === 'unknown') return { status: 'unknown', message: typeof result.message === 'string' ? result.message.slice(0, 512) : 'Outcome unknown' };
    if (result.status !== 'saved' || !revision(result.revision)) throw new TypeError('Invalid onboarding progress result');
    return { status: 'saved', revision: result.revision };
  };
  ipcMain.handle(READ, read);
  ipcMain.handle(CAS, compareAndSwap);
  return () => {
    if (!active) return;
    active = false;
    ipcMain.removeHandler(READ);
    ipcMain.removeHandler(CAS);
  };
}
