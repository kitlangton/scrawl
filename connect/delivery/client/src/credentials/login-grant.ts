import type { StoredCredential } from './wire.ts';

type LoginContext = Readonly<{
  approvedServer: string;
  siteOrigin: string;
  scope: 'read' | 'upload';
  generation: string;
  issuedAt: number;
  encryptionIdentity: string | null;
}>;

const invalid = (): never => { throw new Error('Invalid login credential'); };
const opaque = (value: unknown, max = 8192): value is string => typeof value === 'string'
  && value.length > 0 && value.length <= max && /^[\x21-\x7e]+$/.test(value);
const identity = (value: unknown): value is string => typeof value === 'string' && /^[0-9a-f]{32}$/.test(value);
const record = (value: unknown): value is Record<string, unknown> => value !== null && typeof value === 'object' && !Array.isArray(value);

function apiServer(value: string): boolean {
  if (value.length > 2048 || /[^\x21-\x7e]|["'\\?#@]/.test(value)) return false;
  const match = /^(https?):\/\/([A-Za-z0-9.-]+(?::[0-9]+)?)(\/connect)?$/.exec(value);
  if (!match) return false;
  if (match[1] === 'http' && !['localhost', '127.0.0.1'].includes(match[2]!.split(':')[0]!)) return false;
  try {
    const url = new URL(value);
    return !url.username && !url.password && !url.search && !url.hash
      && (url.pathname === '/' || url.pathname === '/connect') && url.origin + (url.pathname === '/connect' ? '/connect' : '') === value;
  } catch { return false; }
}

function site(value: string): boolean {
  if (value.length > 2048 || /[^\x21-\x7e]|["'\\?#@]/.test(value)) return false;
  try {
    const url = new URL(value);
    const loopback = ['localhost', '127.0.0.1', '[::1]'].includes(url.hostname);
    return (url.protocol === 'https:' || url.protocol === 'http:' && loopback)
      && !url.username && !url.password && !url.search && !url.hash && url.pathname === '/'
      && url.origin === value;
  } catch { return false; }
}

/** Prepare a detached in-memory credential from an already approved device reply. */
export function prepareLoginCredential(reply: unknown, context: LoginContext): StoredCredential {
  try {
    if (!record(reply) || !record(context)
      || reply.status !== 'approved' || reply.scope !== context.scope
      || context.scope !== 'read' && context.scope !== 'upload'
      || typeof context.approvedServer !== 'string' || !apiServer(context.approvedServer)
      || typeof context.siteOrigin !== 'string' || !site(context.siteOrigin)
      || typeof context.generation !== 'string' || !identity(context.generation)
      || !Number.isSafeInteger(context.issuedAt) || context.issuedAt <= 0
      || !(context.encryptionIdentity === null || identity(context.encryptionIdentity))
      || context.scope === 'read' && context.encryptionIdentity === null) invalid();

    const wire = reply as Record<string, unknown>;
    const token = wire.accessToken;
    const refreshToken = wire.refreshToken;
    const grantId = wire.grantId;
    const accessLife = wire.expiresIn;
    const refreshLife = wire.refreshExpiresIn;
    if (!opaque(token) || !opaque(refreshToken) || !opaque(grantId, 512)
      || !Number.isSafeInteger(accessLife) || (accessLife as number) <= 0
      || !Number.isSafeInteger(refreshLife) || (refreshLife as number) <= 0) invalid();
    const expiresAt = context.issuedAt + (accessLife as number);
    const refreshExpiresAt = context.issuedAt + (refreshLife as number);
    if (!Number.isSafeInteger(expiresAt) || !Number.isSafeInteger(refreshExpiresAt)) invalid();

    const initial = wire.accountInitial;
    const accountInitial = typeof initial === 'string' && [...initial].length >= 1 && [...initial].length <= 3
      && /^[\p{L}\p{N}]+$/u.test(initial) ? initial : null;
    return Object.freeze({
      server: context.approvedServer, site: context.siteOrigin, scope: context.scope,
      token: token as string, generation: context.generation, refreshToken: refreshToken as string, expiresAt, refreshExpiresAt,
      grantId: grantId as string, authRequired: false, encryption_identity: context.encryptionIdentity, accountInitial,
    });
  } catch { return invalid(); }
}
