import { ipcMain, type BrowserWindow, type IpcMainInvokeEvent, type WebContents } from 'electron';
import type { CapabilitySnapshot, CommandResult, RecordClient } from '@apparent/record/capabilities';
import {
  RECORD_CHANNELS, cloneCapabilitySnapshot, cloneCommandResult, cloneJsonDto,
  cloneRecordCommand, isRequestId,
} from './wire.js';

const MAX_ACTIVE = 8;
const MAX_WAITING = 16;
const registrations = new WeakMap<object, symbol>();

type Dict = Record<string, unknown>;
function exact(value: unknown, required: readonly string[]): value is Dict {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return false;
  try {
    if (Object.getPrototypeOf(value) !== Object.prototype || Object.getOwnPropertySymbols(value).length) return false;
    const keys = Reflect.ownKeys(value);
    return keys.length === required.length && keys.every(key => typeof key === 'string'
      && required.includes(key) && !['__proto__', 'constructor', 'prototype', 'toJSON'].includes(key)
      && !!Object.getOwnPropertyDescriptor(value, key)?.enumerable
      && 'value' in Object.getOwnPropertyDescriptor(value, key)!);
  } catch { return false; }
}

/** Binds one already-owned RecordClient to one exact BrowserWindow. */
export function registerRecordBridge(options: {
  window: BrowserWindow;
  entryUrl: string;
  client: RecordClient;
}): () => void {
  if (registrations.has(ipcMain)) throw new Error('Record bridge already registered');
  const token = Symbol('record-bridge');
  registrations.set(ipcMain, token);
  const contents: WebContents = options.window.webContents;
  let active = true;
  let current: CapabilitySnapshot | undefined;
  const retiredEpochs = new Set<string>();
  let subscriptionId: string | undefined;
  let activeCount = 0;
  const queue: Array<() => void> = [];
  const revokedSentinel = Symbol('record-bridge-revoked');
  let revokeNow!: () => void;
  const revoked = new Promise<symbol>(resolve => { revokeNow = () => resolve(revokedSentinel); });
  const installed: string[] = [];
  let unsubscribeClient: (() => void) | undefined;

  const allowed = (event: IpcMainInvokeEvent): boolean => {
    try {
      if (!active || options.window.isDestroyed() || contents.isDestroyed()) return false;
      const frame = event.senderFrame;
      return event.sender === contents && !!frame && frame === contents.mainFrame && frame.url === options.entryUrl;
    } catch { return false; }
  };
  const sendSnapshot = () => {
    if (!active || !subscriptionId || !current) return;
    try {
      if (options.window.isDestroyed() || contents.isDestroyed()) return;
      contents.send(RECORD_CHANNELS.snapshot, { subscriptionId, snapshot: cloneCapabilitySnapshot(current)! });
    } catch { /* Closing contents cannot invalidate another Record observer. */ }
  };
  const acceptSnapshot = (value: unknown): CapabilitySnapshot | undefined => {
    const next = cloneCapabilitySnapshot(value);
    if (!next) return undefined;
    if (current) {
      if (next.providerEpoch === current.providerEpoch) {
        if (next.revision > current.revision) current = next;
      } else if (!retiredEpochs.has(next.providerEpoch) && retiredEpochs.size < 512) {
        retiredEpochs.add(current.providerEpoch);
        current = next;
      }
    } else current = next;
    return current;
  };
  const publishSnapshot = (value: unknown) => {
    if (!active) return;
    const before = current;
    const accepted = acceptSnapshot(value);
    if (accepted && accepted !== before) sendSnapshot();
  };
  const acquire = async (event: IpcMainInvokeEvent): Promise<void> => {
    if (!allowed(event)) throw new Error('Record bridge request refused');
    if (activeCount < MAX_ACTIVE) { activeCount++; return; }
    if (queue.length >= MAX_WAITING) throw new Error('Record bridge busy');
    await new Promise<void>((resolve, reject) => queue.push(() => {
      if (!allowed(event)) { reject(new Error('Record bridge request revoked')); return; }
      activeCount++;
      resolve();
    }));
  };
  const release = () => {
    activeCount--;
    if (active) queue.shift()?.();
  };
  const cached = (): CapabilitySnapshot => {
    const snapshot = cloneCapabilitySnapshot(current);
    if (!snapshot) throw new Error('Record provider snapshot unavailable');
    return snapshot;
  };
  const refusal = (requestId: string, reason: string): CommandResult => ({
    status: 'refused', requestId, reason, snapshot: cached(),
  });
  const unknown = (requestId: string): CommandResult => ({
    status: 'outcome-unknown', requestId, snapshot: cached(),
  });

  const read = async (event: IpcMainInvokeEvent, ...args: unknown[]) => {
    if (!allowed(event)) throw new Error('Record bridge request refused');
    if (args.length !== 1) throw new TypeError('Invalid Record read request');
    const request = cloneJsonDto(args[0]);
    if (!exact(request, ['requestId']) || !isRequestId(request.requestId)) throw new TypeError('Invalid Record read request');
    let acquired = false;
    try {
      await acquire(event); acquired = true;
      if (!allowed(event)) throw new Error('Record bridge request revoked');
      const snapshot = cached();
      if (!allowed(event)) throw new Error('Record bridge request revoked');
      return { requestId: request.requestId, snapshot };
    } finally { if (acquired) release(); }
  };

  const execute = async (event: IpcMainInvokeEvent, ...args: unknown[]) => {
    if (!allowed(event)) throw new Error('Record bridge request refused');
    if (args.length !== 1) throw new TypeError('Invalid Record command request');
    const request = cloneJsonDto(args[0]);
    if (!exact(request, ['requestId', 'command']) || !isRequestId(request.requestId)) throw new TypeError('Invalid Record command request');
    const command = cloneRecordCommand(request.command);
    if (!command || command.requestId !== request.requestId) throw new TypeError('Invalid Record command DTO');
    let acquired = false;
    let dispatched = false;
    try {
      try { await acquire(event); acquired = true; }
      catch (error) {
        if (active && allowed(event) && error instanceof Error && error.message === 'Record bridge busy') {
          return { requestId: request.requestId, result: refusal(request.requestId, 'Record transport is busy') };
        }
        throw error;
      }
      if (!active || !allowed(event)) return { requestId: request.requestId, result: refusal(request.requestId, 'Record bridge was revoked before dispatch') };
      const operation = Promise.resolve().then(() => {
        // Queue promotion and provider dispatch are separate turns; authorize again at the final boundary.
        if (!active || !allowed(event)) throw new Error('Record bridge was revoked before dispatch');
        dispatched = true;
        return options.client.execute(command);
      });
      operation.catch(() => {}); // A provider may finish after its IPC reply has become uncertain.
      let raw: CommandResult | symbol;
      try { raw = await Promise.race([operation, revoked]); }
      catch {
        const result = dispatched ? unknown(request.requestId) : refusal(request.requestId, 'Record bridge was revoked before dispatch');
        return { requestId: request.requestId, result };
      }
      if (raw === revokedSentinel || !active || !allowed(event)) {
        const result = dispatched ? unknown(request.requestId) : refusal(request.requestId, 'Record bridge was revoked before dispatch');
        return { requestId: request.requestId, result };
      }
      const validated = cloneCommandResult(raw, request.requestId);
      if (!validated) return { requestId: request.requestId, result: unknown(request.requestId) };
      const before = current;
      const snapshot = acceptSnapshot(validated.snapshot) ?? cached();
      if (snapshot !== before) sendSnapshot();
      if (!active || !allowed(event)) return { requestId: request.requestId, result: unknown(request.requestId) };
      return { requestId: request.requestId, result: { ...validated, snapshot } };
    } finally { if (acquired) release(); }
  };

  const subscribe = async (event: IpcMainInvokeEvent, ...args: unknown[]) => {
    if (!allowed(event)) throw new Error('Record bridge request refused');
    if (args.length !== 1) throw new TypeError('Invalid Record subscription request');
    const request = cloneJsonDto(args[0]);
    if (!exact(request, ['requestId', 'subscriptionId']) || !isRequestId(request.requestId)
      || !isRequestId(request.subscriptionId)) throw new TypeError('Invalid Record subscription request');
    let acquired = false;
    try {
      await acquire(event); acquired = true;
      if (!allowed(event)) throw new Error('Record bridge request revoked');
      if (subscriptionId && subscriptionId !== request.subscriptionId) throw new Error('Record window already has an observer');
      subscriptionId = request.subscriptionId;
      sendSnapshot();
      if (!allowed(event)) throw new Error('Record bridge request revoked');
      return { requestId: request.requestId, subscriptionId, subscribed: true };
    } finally { if (acquired) release(); }
  };

  const unsubscribe = async (event: IpcMainInvokeEvent, ...args: unknown[]) => {
    if (!allowed(event)) throw new Error('Record bridge request refused');
    if (args.length !== 1) throw new TypeError('Invalid Record unsubscription request');
    const request = cloneJsonDto(args[0]);
    if (!exact(request, ['requestId', 'subscriptionId']) || !isRequestId(request.requestId)
      || !isRequestId(request.subscriptionId)) throw new TypeError('Invalid Record unsubscription request');
    let acquired = false;
    try {
      await acquire(event); acquired = true;
      if (!allowed(event)) throw new Error('Record bridge request revoked');
      if (subscriptionId === request.subscriptionId) subscriptionId = undefined;
      return { requestId: request.requestId, subscriptionId: request.subscriptionId, subscribed: false };
    } finally { if (acquired) release(); }
  };

  const dispose = () => {
    if (!active) return;
    active = false;
    subscriptionId = undefined;
    revokeNow();
    queue.splice(0).forEach(wake => wake());
    options.window.removeListener('closed', dispose);
    contents.removeListener('will-navigate', dispose);
    contents.removeListener('destroyed', dispose);
    contents.removeListener('render-process-gone', dispose);
    try { unsubscribeClient?.(); } catch { /* Unsubscribing this observer cannot change provider ownership. */ }
    if (registrations.get(ipcMain) === token) {
      registrations.delete(ipcMain);
      for (const channel of installed) ipcMain.removeHandler(channel);
    }
  };

  try {
    unsubscribeClient = options.client.observe(publishSnapshot);
    if (typeof unsubscribeClient !== 'function' || !current) throw new Error('RecordClient did not provide its immediate snapshot');
    for (const [channel, handler] of [
      [RECORD_CHANNELS.read, read], [RECORD_CHANNELS.execute, execute],
      [RECORD_CHANNELS.subscribe, subscribe], [RECORD_CHANNELS.unsubscribe, unsubscribe],
    ] as const) {
      ipcMain.handle(channel, handler);
      installed.push(channel);
    }
    options.window.once('closed', dispose);
    contents.on('will-navigate', dispose);
    contents.once('destroyed', dispose);
    contents.on('render-process-gone', dispose);
  } catch (error) {
    active = false;
    revokeNow();
    try { unsubscribeClient?.(); } catch { /* preserve registration error */ }
    if (registrations.get(ipcMain) === token) registrations.delete(ipcMain);
    for (const channel of installed) ipcMain.removeHandler(channel);
    throw error;
  }
  return dispose;
}
