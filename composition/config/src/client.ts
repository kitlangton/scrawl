import { spawn } from 'node:child_process';
import { createHash, randomUUID } from 'node:crypto';
import { chmod, lstat, mkdtemp, open, realpath, rm, writeFile } from 'node:fs/promises';
import { constants } from 'node:fs';
import { isAbsolute, join, normalize } from 'node:path';
import { tmpdir } from 'node:os';
import type { ConfigAdoptionSnapshot, JsonValue, LegacyAdoptionBackend, LegacyConfigValue, RecordPenColorConfig, Revision } from '@apparent/app-contracts/host';

const LIMIT = 64 * 1024;
const unavailable = () => new Error('Configuration host unavailable');
const unknown = () => ({ status: 'unknown' as const, message: 'Configuration outcome unknown; reload before retrying' });
const validRevision = (value: unknown): value is number => typeof value === 'number' && Number.isSafeInteger(value) && value >= 0;
function exact(value: unknown, keys: string[]): value is Record<string, unknown> {
  return !!value && typeof value === 'object' && !Array.isArray(value)
    && Object.keys(value).length === keys.length && keys.every(key => Object.hasOwn(value, key));
}
function json(value: unknown, depth = 0, ancestors = new Set<object>()): JsonValue {
  if (value === null || typeof value === 'string' || typeof value === 'boolean') return value;
  if (typeof value === 'number' && Number.isFinite(value)) return value;
  if (!value || typeof value !== 'object' || depth > 32 || ancestors.has(value)
    || Object.getOwnPropertySymbols(value).length) throw new TypeError('Invalid JSON state');
  const array = Array.isArray(value);
  if (Object.getPrototypeOf(value) !== (array ? Array.prototype : Object.prototype)) throw new TypeError('Invalid JSON state');
  const descriptors = Object.getOwnPropertyDescriptors(value);
  const keys = Object.keys(descriptors).filter(key => !(array && key === 'length'));
  if (array && (keys.length !== value.length || keys.some((key, index) => key !== String(index)))) throw new TypeError('Invalid JSON array');
  const result: JsonValue[] | Record<string, JsonValue> = array ? [] : {};
  ancestors.add(value);
  for (const key of keys) {
    const descriptor = descriptors[key];
    if (!descriptor.enumerable || !('value' in descriptor) || ['__proto__', 'constructor', 'prototype', 'toJSON'].includes(key)) throw new TypeError('Invalid JSON state');
    Object.defineProperty(result, key, { enumerable: true, configurable: true, writable: true, value: json(descriptor.value, depth + 1, ancestors) });
  }
  ancestors.delete(value);
  return result;
}

function legacyValue(value: unknown): LegacyConfigValue {
  if (!exact(value, ['present', 'value']) || typeof value.present !== 'boolean'
    || (!value.present && value.value !== null)) throw new TypeError('Invalid legacy snapshot');
  return { present: value.present, value: json(value.value) };
}
function adoptionSnapshot(value: unknown): ConfigAdoptionSnapshot {
  if (!exact(value, ['owner', 'legacy']) || !exact(value.owner, ['present', 'revision', 'value'])
    || typeof value.owner.present !== 'boolean' || !validRevision(value.owner.revision)
    || (!value.owner.present && (value.owner.revision !== 0 || value.owner.value !== null))) {
    throw new TypeError('Invalid adoption snapshot');
  }
  return { owner: { present: value.owner.present, revision: value.owner.revision, value: json(value.owner.value) },
    legacy: legacyValue(value.legacy) };
}

export interface ConfigHostOptions {
  /** Explicit admitted artifact: never a PATH lookup, source builder or package manager. */
  executable: string;
  executableSha256: string;
  configPath: string;
  ownerId: string;
  timeoutMs?: number;
}
export interface ConfigBackend extends LegacyAdoptionBackend {
  /** Explicitly and durably retire only the legacy onboarding progress writer. */
  retireOnboardingLegacyWriter(): Promise<{ status: 'retired' } | { status: 'unknown'; message: string }>;
  close(): Promise<void>;
  readPenColor(): Promise<LegacyConfigValue>;
  compareAndSwapPenColor(expected: LegacyConfigValue, next: LegacyConfigValue): Promise<
    | { status: 'saved' }
    | { status: 'conflict'; current: LegacyConfigValue }
    | { status: 'unknown'; message: string }
  >;
}

/** Privileged composition only. Renderers receive a separately bound owner facade. */
export async function createConfigBackend(options: ConfigHostOptions): Promise<ConfigBackend> {
  for (const value of [options.executable, options.configPath]) {
    if (typeof value !== 'string' || !isAbsolute(value) || normalize(value) !== value || value.includes('\0')) throw new TypeError('Host paths must be normalized absolute paths');
  }
  if (!/^[a-z][a-z0-9-]{0,63}$/.test(options.ownerId)) throw new TypeError('Invalid compiled-in owner');
  const timeout = options.timeoutMs ?? 5_000;
  if (!Number.isSafeInteger(timeout) || timeout < 50 || timeout > 30_000) throw new TypeError('Invalid host timeout');
  if (!/^[a-f0-9]{64}$/.test(options.executableSha256)) throw new TypeError('Invalid artifact hash');
  const file = await open(options.executable, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK)
    .catch(() => { throw new Error('Config host artifact verification failed'); });
  let bytes: Buffer;
  try {
    const before = await file.stat();
    if (!before.isFile() || before.mode & 0o022 || !(before.mode & 0o111)
      || before.size < 1 || before.size > 64 * 1024 * 1024) throw new Error('Invalid artifact file');
    const snapshot = Buffer.alloc(before.size + 1);
    let length = 0;
    while (length < snapshot.length) {
      const result = await file.read(snapshot, length, snapshot.length - length, length);
      if (!result.bytesRead) break;
      length += result.bytesRead;
    }
    if (length !== before.size) throw new Error('Artifact size changed during verification');
    bytes = snapshot.subarray(0, length);
    const current = await lstat(options.executable);
    if (current.isSymbolicLink()
      || before.dev !== current.dev || before.ino !== current.ino
      || createHash('sha256').update(bytes).digest('hex') !== options.executableSha256) throw new Error('Config host artifact verification failed');
  } catch { throw new Error('Config host artifact verification failed'); }
  finally { await file.close(); }
  // Execute the verified bytes from a fresh private directory. The admitted
  // source artifact may live in a replaceable directory; never reopen it to run.
  const staging = await realpath(await mkdtemp(join(tmpdir(), 'apparent-config-host-')));
  const executable = join(staging, 'apparent-config-host');
  try {
    await chmod(staging, 0o700);
    await writeFile(executable, bytes, { flag: 'wx', mode: 0o700 });
  } catch (error) { await rm(staging, { recursive: true }); throw error; }
  const child = spawn(executable, ['--config', options.configPath, '--owner', options.ownerId], {
    stdio: ['pipe', 'pipe', 'ignore'], shell: false,
  });
  let active = true;
  let buffer = Buffer.alloc(0);
  let pending: { id: string; resolve(value: unknown): void; reject(error: Error): void; timer: ReturnType<typeof setTimeout> } | undefined;
  let queue = Promise.resolve();
  let queued = 0;
  let closing: Promise<void> | undefined;
  const exited = new Promise<void>(resolve => child.once('close', () => { fail(); resolve(); }));
  function fail() {
    active = false;
    if (pending) { clearTimeout(pending.timer); pending.reject(unavailable()); pending = undefined; }
    child.kill('SIGTERM');
  }
  child.once('error', fail);
  child.stdin.on('error', fail);
  child.stdout.on('data', (chunk: Buffer) => {
    buffer = Buffer.concat([buffer, chunk]);
    if (buffer.length > LIMIT) { fail(); return; }
    const end = buffer.indexOf(10);
    if (end < 0) return;
    const line = buffer.subarray(0, end);
    buffer = buffer.subarray(end + 1);
    try {
      const response: unknown = JSON.parse(new TextDecoder('utf8', { fatal: true }).decode(line));
      if (buffer.length || !pending || !exact(response, ['protocol', 'id', 'result'])
        || response.protocol !== 1 || response.id !== pending.id) throw unavailable();
      const current = pending;
      clearTimeout(current.timer);
      pending = undefined;
      current.resolve(response.result);
    } catch { fail(); }
  });
  async function request(operation: string, fields: Record<string, unknown> = {}): Promise<unknown> {
    if (!active || queued >= 16) throw unavailable();
    const id = randomUUID();
    const line = JSON.stringify({ protocol: 1, id, operation, ...fields }) + '\n';
    if (Buffer.byteLength(line) > LIMIT) throw new TypeError('JSON request exceeds size limit');
    queued++;
    const run = queue.then(() => new Promise<unknown>((resolve, reject) => {
      if (!active) { reject(unavailable()); return; }
      pending = { id, resolve, reject, timer: setTimeout(fail, timeout) };
      child.stdin.write(line, error => { if (error) fail(); });
    }));
    queue = run.then(() => undefined, () => undefined);
    try { return await run; } finally { queued--; }
  }
  function owner(id: string) { if (id !== options.ownerId) throw new TypeError('Config owner does not match binding'); }
  function recordOnly() { if (options.ownerId !== 'record') throw new TypeError('penColor config access requires the compiled record owner'); }
  return {
    async read(id) {
      owner(id);
      const result = await request('read');
      if (!exact(result, ['revision', 'value']) || !validRevision(result.revision)) { fail(); throw unavailable(); }
      try { return { revision: result.revision, value: json(result.value) }; }
      catch { fail(); throw unavailable(); }
    },
    async readForAdoption(id) {
      owner(id);
      try { return adoptionSnapshot(await request('readForAdoption')); }
      catch { fail(); throw unavailable(); }
    },
    async retireOnboardingLegacyWriter() {
      if (options.ownerId !== 'onboarding') throw new TypeError('Onboarding writer ownership requires the compiled onboarding owner');
      try {
        const result = await request('retireOnboardingLegacyWriter');
        if (exact(result, ['status']) && result.status === 'retired') return { status: 'retired' as const };
        fail();
        return unknown();
      } catch { return unknown(); }
    },
    async readPenColor() {
      recordOnly();
      try { return legacyValue(await request('readPenColor')); }
      catch { fail(); throw unavailable(); }
    },
    async compareAndSwapPenColor(expected, next) {
      recordOnly();
      const before = legacyValue(expected);
      const after = legacyValue(next);
      try {
        const result = await request('compareAndSwapPenColor', { expectedLegacy: before, next: after });
        if (exact(result, ['status']) && result.status === 'saved') return { status: 'saved' as const };
        if (exact(result, ['status', 'current']) && result.status === 'conflict') return { status: 'conflict' as const, current: legacyValue(result.current) };
        if (exact(result, ['status', 'message']) && result.status === 'unknown' && typeof result.message === 'string') return { status: 'unknown' as const, message: result.message };
        fail();
        return unknown();
      } catch { return unknown(); }
    },
    async adoptLegacy(id, expectedLegacy, value) {
      owner(id);
      const expected = legacyValue(expectedLegacy);
      const input = json(value);
      try {
        const result = await request('adoptLegacy', { expectedLegacy: expected, value: input });
        if (exact(result, ['status', 'revision']) && result.status === 'saved' && result.revision === 1) return { status: 'saved' as const, revision: 1 };
        if (exact(result, ['status', 'snapshot']) && result.status === 'conflict') return { status: 'conflict' as const, snapshot: adoptionSnapshot(result.snapshot) };
        if (exact(result, ['status', 'message']) && result.status === 'unknown' && typeof result.message === 'string') return unknown();
        fail();
        return unknown();
      } catch { fail(); return unknown(); }
    },
    async compareAndSwap(id: string, expected: Revision, value: JsonValue) {
      owner(id);
      if (!validRevision(expected)) throw new TypeError('Invalid config revision');
      const input = json(value);
      try {
        const result = await request('compareAndSwap', { expected, value: input });
        if (exact(result, ['status', 'revision']) && result.status === 'saved' && validRevision(result.revision)) return { status: 'saved' as const, revision: result.revision };
        if (exact(result, ['status', 'revision', 'current']) && result.status === 'conflict' && validRevision(result.revision)) return { status: 'conflict' as const, revision: result.revision, current: json(result.current) };
        if (exact(result, ['status', 'message']) && result.status === 'unknown' && typeof result.message === 'string') return unknown();
        fail();
        return unknown();
      } catch { return unknown(); }
    },
    async close() {
      closing ??= (async () => {
        fail();
        const kill = setTimeout(() => child.kill('SIGKILL'), 500);
        try { await exited; } finally { clearTimeout(kill); await rm(staging, { recursive: true }); }
      })();
      return closing;
    },
  };
}

/** Owner-facing facade: owner and config key are compiled in; no path/key arguments exist. */
export async function createRecordPenColorConfig(options: Omit<ConfigHostOptions, 'ownerId'>): Promise<RecordPenColorConfig & { close(): Promise<void> }> {
  const backend = await createConfigBackend({ ...options, ownerId: 'record' });
  return {
    read: () => backend.readPenColor(),
    compareAndSwap: (expected, next) => backend.compareAndSwapPenColor(expected, next),
    close: () => backend.close(),
  };
}
