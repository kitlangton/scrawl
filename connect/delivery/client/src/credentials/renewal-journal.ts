import { decodeStrictJson } from '../recipient/json.ts';
import { decodeStoredCredential, mergeRenewalReply, type StoredCredential } from './wire.ts';

export type RenewalJournal = Readonly<{ base: StoredCredential; issued: number; completed: boolean }>;
export type RenewalJournalFailure = 'invalid_journal' | 'not_recoverable' | 'auth_required';

/** Errors expose only an allowlisted category, never paths, bytes, tokens, or parser causes. */
export class RenewalJournalError extends Error {
  readonly category: RenewalJournalFailure;
  constructor(category: RenewalJournalFailure) {
    super(category);
    this.name = 'RenewalJournalError';
    this.category = category;
  }
}

const object = (value: unknown): value is Record<string, unknown> => value !== null && typeof value === 'object' && !Array.isArray(value);
const invalid = (): never => { throw new RenewalJournalError('invalid_journal'); };
const mismatch = (): never => { throw new RenewalJournalError('not_recoverable'); };

/** Decode the legacy .renewal record without reading files or changing state. */
export function decodeRenewalJournal(bytes: Uint8Array, context: { scope: 'read' | 'upload'; approvedServer: string }): RenewalJournal {
  try {
    const raw = decodeStrictJson(bytes, 65_536);
    if (!object(raw) || !Object.hasOwn(raw, 'base') || !Object.hasOwn(raw, 'issued')) invalid();
    const record = raw as Record<string, unknown>;
    if (typeof record.issued !== 'number' || !Number.isSafeInteger(record.issued) || record.issued < 0) invalid();
    if (Object.hasOwn(record, 'completed') && typeof record.completed !== 'boolean') invalid();
    const baseBytes = new TextEncoder().encode(JSON.stringify(record.base));
    const base = decodeStoredCredential(baseBytes, context.scope, context.approvedServer);
    return Object.freeze({ base, issued: record.issued as number, completed: record.completed === undefined ? false : record.completed as boolean });
  } catch (error) {
    if (error instanceof RenewalJournalError) throw error;
    throw new RenewalJournalError('invalid_journal');
  }
}

const fields: readonly (keyof StoredCredential)[] = [
  'server', 'site', 'scope', 'token', 'generation', 'refreshToken', 'expiresAt', 'refreshExpiresAt', 'grantId',
  'authRequired', 'encryption_identity', 'accountInitial',
];
/** Exact comparison of the complete known stored credential wire shape. */
export const sameStoredCredential = (a: StoredCredential, b: StoredCredential): boolean => fields.every(key => a[key] === b[key]);

/** A completed journal is recoverable only for the exact, still-renewable login incarnation. */
export function isCompletedRenewalFor(current: StoredCredential, journal: RenewalJournal): boolean {
  return journal.completed && !current.authRequired && current.refreshToken !== null && current.grantId !== null &&
    current.expiresAt !== null && current.refreshExpiresAt !== null && sameStoredCredential(current, journal.base);
}

/** Merge a previously completed response using its original issue time. */
export function recoverCompletedRenewal(current: StoredCredential, journal: RenewalJournal, responseBytes: Uint8Array): StoredCredential {
  if (current.authRequired) throw new RenewalJournalError('auth_required');
  if (!isCompletedRenewalFor(current, journal)) mismatch();
  try {
    const reply = decodeStrictJson(responseBytes, 2_000_000);
    return mergeRenewalReply(current, reply, journal.issued);
  } catch {
    throw new RenewalJournalError('invalid_journal');
  }
}
