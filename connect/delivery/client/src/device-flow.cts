import type { SafeError } from './settings-presentation.js';

export type Approval = { verificationUri: string; userCode: string };
export type DeviceTransport = {
  start(): Promise<unknown>;
  poll(deviceCode: string, timeoutMs: number): Promise<unknown>;
  commit(siteOrigin: string): Promise<SafeError | null>;
};

export class DeviceFlowError extends Error {
  constructor(readonly code: SafeError) {
    super(code);
    this.name = 'DeviceFlowError';
  }
}

type Options = { now?: () => number; sleep?: (ms: number, signal: AbortSignal) => Promise<void> };
const safeErrors = new Set<SafeError>([
  'cancelled', 'offline', 'auth-required', 'rejected', 'expired', 'local-state',
  'invalid-server', 'unverified-commit', 'unavailable',
]);
const unavailable = () => new DeviceFlowError('unavailable');
const cancelled = () => new DeviceFlowError('cancelled');
const record = (v: unknown): Record<string, unknown> =>
  v !== null && typeof v === 'object' && !Array.isArray(v) ? v as Record<string, unknown> : {};
const uint = (v: unknown): v is number => Number.isSafeInteger(v) && (v as number) >= 0;
const printable = (v: unknown, max: number): v is string => typeof v === 'string' &&
  v.length > 0 && v.length <= max && /^[\x21-\x7e]+$/.test(v);
const positiveSafe = (v: unknown): v is number => Number.isSafeInteger(v) && (v as number) > 0;

function validApprovalUri(value: unknown, userCode: string, deviceCode: string): { uri: string; origin: string } | null {
  if (typeof value !== 'string' || value.length === 0 || value.length > 4096 || value.includes(deviceCode) ||
      /[\x00-\x20\x7f%#\\"']/.test(value)) return null;
  try {
    const authority = value.match(/^https?:\/\/[^/]+/)?.[0];
    if (!authority || authority.length > 2048) return null;
    const url = new URL(value);
    if (!/^(https?:)\/\/[a-zA-Z0-9.-]+(?::[0-9]+)?\//.test(value) || url.username || url.password || url.hash ||
        !(url.protocol === 'https:' || (url.protocol === 'http:' && ['localhost', '127.0.0.1'].includes(url.hostname))) ||
        (url.search !== '' && url.search !== `?code=${userCode}`)) return null;
    return { uri: value, origin: url.origin };
  } catch { return null; }
}

function defaultSleep(ms: number, signal: AbortSignal): Promise<void> {
  if (signal.aborted) return Promise.reject(cancelled());
  return new Promise((resolve, reject) => {
    const finish = () => { clearTimeout(timer); signal.removeEventListener('abort', abort); };
    const abort = () => { finish(); reject(cancelled()); };
    const timer = setTimeout(() => { finish(); resolve(); }, ms);
    signal.addEventListener('abort', abort, { once: true });
    if (signal.aborted) abort();
  });
}

function checkedSafeError(value: unknown): SafeError | null {
  if (value === null) return null;
  if (typeof value === 'string' && safeErrors.has(value as SafeError)) return value as SafeError;
  throw unavailable();
}

/** Runs the private device protocol. Only validated approval details escape this function. */
export async function authorizeDevice(
  transport: DeviceTransport,
  signal: AbortSignal,
  onApproval: (value: Approval) => void,
  options: Options = {},
): Promise<SafeError | null> {
  const now = options.now ?? (() => performance.now());
  const sleep = options.sleep ?? defaultSleep;
  const checkAbort = () => { if (signal.aborted) throw cancelled(); };
  try {
    checkAbort();
    const started = record(await transport.start());
    checkAbort();
    const deviceCode = started.deviceCode;
    const userCode = started.userCode;
    if (!printable(deviceCode, 8192) || typeof userCode !== 'string' || !/^[a-zA-Z0-9-]{1,128}$/.test(String(userCode)) ||
        String(userCode).includes(String(deviceCode))) throw unavailable();
    const link = validApprovalUri(started.verificationUri, String(userCode), deviceCode);
    if (!link) throw unavailable();
    const lifetime = started.expiresIn === undefined ? 300 : started.expiresIn;
    const cadence = started.interval === undefined ? 5 : started.interval;
    if (!uint(lifetime) || !uint(cadence)) throw unavailable();
    const startedAt = now();
    const deadline = startedAt + Math.min(lifetime, 900) * 1000;
    if (!Number.isFinite(deadline) || deadline <= startedAt) throw new DeviceFlowError('expired');
    checkAbort();
    onApproval({ verificationUri: link.uri, userCode: String(userCode) });
    checkAbort();

    while (true) {
      const remaining = deadline - now();
      if (remaining <= 0) throw new DeviceFlowError('expired');
      await sleep(Math.min(Math.max(1, Math.min(cadence, 30)) * 1000, remaining), signal);
      checkAbort();
      const afterSleep = deadline - now();
      if (afterSleep <= 0) throw new DeviceFlowError('expired');
      const reply = record(await transport.poll(deviceCode, Math.max(1, Math.floor(Math.min(30_000, afterSleep)))));
      checkAbort();
      if (reply.status === 'pending') continue;
      if (reply.status === 'rejected') throw new DeviceFlowError('rejected');
      if (reply.status === 'expired') throw new DeviceFlowError('expired');
      if (reply.status !== 'approved') throw new DeviceFlowError('rejected');
      if (reply.scope !== 'upload' ||
          !printable(reply.accessToken, 8192) || !printable(reply.refreshToken, 8192) ||
          !printable(reply.grantId, 512) || !positiveSafe(reply.expiresIn) ||
          !positiveSafe(reply.refreshExpiresIn)) throw unavailable();
      const issued = Math.floor(Date.now() / 1000);
      if (!Number.isSafeInteger(issued + reply.expiresIn) ||
          !Number.isSafeInteger(issued + reply.refreshExpiresIn)) throw unavailable();
      checkAbort();
      if (now() >= deadline) throw new DeviceFlowError('expired');
      return checkedSafeError(await transport.commit(link.origin));
    }
  } catch (error) {
    if (error instanceof DeviceFlowError && safeErrors.has(error.code)) return error.code;
    return 'unavailable';
  }
}
