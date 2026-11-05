import { randomUUID } from 'node:crypto';
import { lstat } from 'node:fs/promises';
import { connect } from 'node:net';
import { homedir } from 'node:os';
import { join, normalize } from 'node:path';
import type { NativeHostPort, NativeHostSnapshot, NativeHostRefusal, NativeMutationResult } from '@apparent/app-contracts/host';

const MAX_FRAME = 4096;
const TIMEOUT_MS = 3000;
const PERMISSIONS = new Set(['screen-recording', 'microphone', 'accessibility', 'camera', 'speech-recognition']);
const exact = (value: unknown, keys: string[]): value is Record<string, unknown> => !!value && typeof value === 'object'
  && !Array.isArray(value) && Object.keys(value).length === keys.length && keys.every(key => Object.hasOwn(value, key));

function socketPath(): string {
  const home = homedir();
  if (!home || normalize(home) !== home || !home.startsWith('/')) throw new Error('Native host unavailable');
  return join(home, '.config/apparent/agent-control/capture.sock');
}

async function verifySocket(path: string): Promise<{ dev: number; ino: number }> {
  const home = homedir();
  const config = join(home, '.config');
  const apparent = join(config, 'apparent');
  const parent = join(apparent, 'agent-control');
  const uid = process.getuid?.();
  if (uid === undefined) throw new Error('Native host unavailable');
  for (const [directory, privateMode] of [[home, false], [config, false], [apparent, false], [parent, true]] as const) {
    const meta = await lstat(directory);
    if (!meta.isDirectory() || meta.isSymbolicLink() || meta.uid !== uid
      || (privateMode ? (meta.mode & 0o077) !== 0 : (meta.mode & 0o022) !== 0)) throw new Error('Native host unavailable');
  }
  const socket = await lstat(path);
  if (!socket.isSocket() || socket.isSymbolicLink() || socket.uid !== uid || (socket.mode & 0o777) !== 0o600) {
    throw new Error('Native host unavailable');
  }
  return { dev: socket.dev, ino: socket.ino };
}

function mutation(value: unknown): NativeMutationResult {
  if (exact(value, ['status']) && value.status === 'accepted') return { status: 'accepted' };
  if (exact(value, ['status', 'reason']) && value.status === 'refused'
    && ['stale', 'busy', 'unavailable', 'invalid', 'reserved', 'registration-refused'].includes(String(value.reason))) {
    return { status: 'refused', reason: value.reason as NativeHostRefusal };
  }
  if (exact(value, ['status']) && value.status === 'outcome-unknown') return { status: 'outcome-unknown' };
  throw new Error('Native host unavailable');
}

function snapshot(value: unknown): NativeHostSnapshot {
  const shortcut = (item: unknown): boolean => {
    if (!exact(item, ['configured', 'effective', 'registration', 'canValidate', 'canSave'])
      || !['registered', 'unregistered', 'externally-managed', 'unknown', 'unavailable'].includes(String(item.registration))
      || typeof item.canValidate !== 'boolean' || typeof item.canSave !== 'boolean') return false;
    const valueOk = (value: unknown) => value === null || (exact(value, ['keyCode', 'modifiers', 'label'])
      && Number.isSafeInteger(value.keyCode) && Number.isSafeInteger(value.modifiers) && typeof value.label === 'string' && value.label.length <= 32);
    return valueOk(item.configured) && valueOk(item.effective);
  };
  if (!exact(value, ['api', 'processEpoch', 'revision', 'scope', 'freshness', 'permissions', 'shortcuts'])) throw new Error('Native host unavailable');
  const s = value;
  if (s.api !== 1 || typeof s.processEpoch !== 'string' || !/^[a-zA-Z0-9-]{1,64}$/.test(s.processEpoch)
    || !Number.isSafeInteger(s.revision) || (s.revision as number) < 0
    || s.scope !== 'running-desktop-process' || s.freshness !== 'cached-gui-observation' || !Array.isArray(s.permissions)
    || !s.permissions.every(permission => exact(permission, ['id', 'status', 'usableNow', 'relaunchNeeded', 'actions'])
      && typeof permission.id === 'string' && ['granted', 'denied', 'not-determined', 'unavailable'].includes(String(permission.status))
      && typeof permission.usableNow === 'boolean' && typeof permission.relaunchNeeded === 'boolean' && Array.isArray(permission.actions)
      && permission.actions.every(action => action === 'request-permission' || action === 'open-system-settings'))
    || !exact(s.shortcuts, ['record', 'paste']) || !shortcut(s.shortcuts.record) || !shortcut(s.shortcuts.paste)) throw new Error('Native host unavailable');
  return value as unknown as NativeHostSnapshot;
}

async function request(operation: string, fields: Record<string, unknown> = {}): Promise<unknown> {
  const path = socketPath();
  const identity = await verifySocket(path);
  const id = randomUUID();
  const deadline = Number(process.hrtime.bigint() / 1_000_000n) + TIMEOUT_MS;
  const frame = Buffer.from(`${JSON.stringify({ protocol: 1, id, owner: 'record', operation, ...fields, deadline })}\n`);
  if (frame.length > MAX_FRAME) throw new TypeError('Native host request exceeds limit');
  return await new Promise((resolve, reject) => {
    const socket = connect(path);
    let buffer = Buffer.alloc(0);
    let settled = false;
    const timer = setTimeout(() => finish(new Error('Native host unavailable')), TIMEOUT_MS);
    const finish = (error?: Error, result?: unknown) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      socket.destroy();
      if (error) reject(error); else resolve(result);
    };
    socket.once('connect', async () => {
      try {
        const current = await verifySocket(path);
        if (current.dev !== identity.dev || current.ino !== identity.ino) throw new Error('Native host unavailable');
        socket.write(frame);
      } catch { finish(new Error('Native host unavailable')); }
    });
    socket.on('data', chunk => {
      buffer = Buffer.concat([buffer, chunk]);
      if (buffer.length > MAX_FRAME) return finish(new Error('Native host unavailable'));
      const end = buffer.indexOf(10);
      if (end < 0) return;
      if (end !== buffer.length - 1) return finish(new Error('Native host unavailable'));
      try {
        const response: unknown = JSON.parse(new TextDecoder('utf-8', { fatal: true }).decode(buffer.subarray(0, end)));
        if (!exact(response, ['protocol', 'id', 'result']) || response.protocol !== 1 || response.id !== id) throw new Error();
        finish(undefined, response.result);
      } catch { finish(new Error('Native host unavailable')); }
    });
    socket.once('error', () => finish(new Error('Native host unavailable')));
  });
}

/** Privileged composition binding. Renderers get a Record-bound facade, never a socket/path. */
export function createNativeHostPort(): NativeHostPort {
  const expected = (value: { processEpoch: string; revision: number }) => {
    if (!value || typeof value.processEpoch !== 'string' || !value.processEpoch || !Number.isSafeInteger(value.revision) || value.revision < 0) {
      throw new TypeError('Invalid native host revision');
    }
    return value;
  };
  return {
    async readSnapshot() { return snapshot(await request('readSnapshot')); },
    async requestPermission(permissionId, state) {
      if (!PERMISSIONS.has(permissionId)) throw new TypeError('Unsupported permission');
      return mutation(await request('requestPermission', { permissionId, expected: expected(state) }));
    },
    async openSystemSettings(permissionId, state) {
      if (!PERMISSIONS.has(permissionId)) throw new TypeError('Unsupported permission');
      return mutation(await request('openSystemSettings', { permissionId, expected: expected(state) }));
    },
    async validateRecordShortcut(key, modifiers, state) {
      if (typeof key !== 'string' || key.length < 1 || key.length > 16 || !Number.isSafeInteger(modifiers) || modifiers < 0 || modifiers > 0xffff) throw new TypeError('Invalid shortcut');
      const value = await request('validateRecordShortcut', { key, modifiers, expected: expected(state) });
      if (exact(value, ['status', 'reason'])) return mutation(value);
      if (!exact(value, ['status', 'shortcut']) || value.status !== 'accepted' || !value.shortcut || typeof value.shortcut !== 'object') throw new Error('Native host unavailable');
      const shortcut = value.shortcut as Record<string, unknown>;
      if (!Number.isSafeInteger(shortcut.keyCode) || !Number.isSafeInteger(shortcut.modifiers) || typeof shortcut.label !== 'string') throw new Error('Native host unavailable');
      return { status: 'accepted', shortcut: shortcut as { keyCode: number; modifiers: number; label: string } };
    },
    async saveRecordShortcut(shortcut, state) {
      if (!shortcut || !Number.isSafeInteger(shortcut.keyCode) || shortcut.keyCode < 0 || shortcut.keyCode > 255
        || !Number.isSafeInteger(shortcut.modifiers) || shortcut.modifiers < 0 || shortcut.modifiers > 0xffff
        || typeof shortcut.label !== 'string' || shortcut.label.length > 32) throw new TypeError('Invalid shortcut');
      return mutation(await request('saveRecordShortcut', { shortcut, expected: expected(state) }));
    },
  };
}
