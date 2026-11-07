import { normalizeRecipientServer } from '../recipient/transport.ts';
import { mergeRenewalReply, type StoredCredential } from './wire.ts';

type LoginContext = Readonly<{
  approvedServer: string;
  siteOrigin: string;
  scope: 'read' | 'upload';
  generation: string;
  issuedAt: number;
  encryptionIdentity: string | null;
}>;
const invalid = (): never => { throw new Error('Invalid login credential'); };
const identity = (value: unknown): value is string => typeof value === 'string' && /^[0-9a-f]{32}$/.test(value);
const record = (value: unknown): value is Record<string, unknown> => value !== null && typeof value === 'object' && !Array.isArray(value);

/** Private in-memory proposal only; approval, freshness and durable publication remain caller-owned. */
export function prepareLoginCredential(reply: unknown, context: LoginContext): StoredCredential {
  try {
    if (!record(reply) || !record(context)) return invalid();
    // Copy only known scalar facts; never serialize arbitrary service data.
    const { approvedServer, siteOrigin, scope, generation, issuedAt, encryptionIdentity } = context;
    const { status, scope: replyScope, accessToken, refreshToken, grantId, expiresIn, refreshExpiresIn, accountInitial } = reply;
    if (status !== 'approved' || replyScope !== scope || scope !== 'read' && scope !== 'upload'
      || typeof approvedServer !== 'string' || normalizeRecipientServer(approvedServer) !== approvedServer
      || typeof siteOrigin !== 'string' || normalizeRecipientServer(siteOrigin) !== siteOrigin
      || new URL(siteOrigin).origin !== siteOrigin || !identity(generation)
      || !Number.isSafeInteger(issuedAt) || issuedAt < 0
      || !(encryptionIdentity === null || identity(encryptionIdentity))
      || scope === 'read' && encryptionIdentity === null) invalid();
    // Seed the new grant binding, then use the existing lifecycle validator. A
    // renewal still pins an existing grant; a new login establishes that binding.
    const base: StoredCredential = {
      server: approvedServer, site: siteOrigin, scope, generation, token: '',
      refreshToken: null, expiresAt: null, refreshExpiresAt: null,
      grantId: grantId as string, authRequired: false, encryption_identity: encryptionIdentity, accountInitial: null,
    };
    return mergeRenewalReply(base, { scope: replyScope, accessToken, refreshToken, grantId, expiresIn, refreshExpiresIn,
      // At most three Unicode scalars can survive the shared initials filter.
      accountInitial: typeof accountInitial === 'string' && accountInitial.length <= 6 ? accountInitial : null }, issuedAt);
  } catch { return invalid(); }
}
