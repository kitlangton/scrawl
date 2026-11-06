import { isCompletedRenewalFor, recoverCompletedRenewal, sameStoredCredential, type RenewalJournal } from './renewal-journal.ts';
import type { StoredCredential } from './wire.ts';

export type RenewalPlanInput = Readonly<{
  expected: StoredCredential;
  current: StoredCredential;
  now: number;
  force: boolean;
  cancelled: boolean;
  revocationPending: boolean;
  journal: RenewalJournal | null;
  responseBytes: Uint8Array | null;
}>;

export type RenewalPlan =
  | Readonly<{ kind: 'cancelled'; reason: 'cancelled' | 'revocation_pending' }>
  | Readonly<{ kind: 'stale'; reason: 'generation' | 'server' | 'scope' }>
  | Readonly<{ kind: 'auth_required'; reason: 'credential' | 'no_refresh_token' | 'refresh_expired' | 'invalid_saved_response' }>
  | Readonly<{ kind: 'use_current'; credential: StoredCredential }>
  | Readonly<{ kind: 'recover_completed'; credential: StoredCredential; issued: number }>
  | Readonly<{ kind: 'prepare_request'; journal: RenewalJournal; request: Readonly<{ refreshToken: string; issued: number }> }>;

const snapshot = (credential: StoredCredential): StoredCredential => Object.freeze({ ...credential });
const journalSnapshot = (journal: RenewalJournal): RenewalJournal => Object.freeze({
  base: snapshot(journal.base), issued: journal.issued, completed: journal.completed,
});

/** Pure proposal matching the legacy renew() decision order after its scope lock is held. */
export function planRenewal(input: RenewalPlanInput): RenewalPlan {
  if (!Number.isSafeInteger(input.now) || input.now < 0) throw new RenewalPlanError();
  const responseBytes = input.responseBytes === null || input.responseBytes.byteLength > 2_000_000 ? null : new Uint8Array(input.responseBytes);
  try { return decideRenewal(input, responseBytes); }
  finally { responseBytes?.fill(0); }
}

export class RenewalPlanError extends Error {
  readonly category = 'invalid_plan';
  constructor() { super('invalid_plan'); this.name = 'RenewalPlanError'; }
}

function decideRenewal(input: RenewalPlanInput, responseBytes: Uint8Array | null): RenewalPlan {
  const expected = snapshot(input.expected);
  const current = snapshot(input.current);
  const journal = input.journal === null ? null : journalSnapshot(input.journal);

  if (input.cancelled) return Object.freeze({ kind: 'cancelled', reason: 'cancelled' });
  if (input.revocationPending) return Object.freeze({ kind: 'cancelled', reason: 'revocation_pending' });
  if (current.generation !== expected.generation) return Object.freeze({ kind: 'stale', reason: 'generation' });
  if (current.server !== expected.server) return Object.freeze({ kind: 'stale', reason: 'server' });
  if (current.scope !== expected.scope) return Object.freeze({ kind: 'stale', reason: 'scope' });
  if (current.authRequired) return Object.freeze({ kind: 'auth_required', reason: 'credential' });

  // A writer that already rotated this access incarnation wins without another refresh.
  if (!sameStoredCredential(current, expected)) return Object.freeze({ kind: 'use_current', credential: current });
  const due = current.expiresAt !== null && current.expiresAt <= input.now + 30;
  if (!input.force && !due) return Object.freeze({ kind: 'use_current', credential: current });
  if (current.refreshToken === null) return Object.freeze({ kind: 'auth_required', reason: 'no_refresh_token' });

  // Exact-base completed responses are recovered before checking refresh expiry.
  if (journal !== null && journal.completed && isCompletedRenewalFor(current, journal)) {
    if (responseBytes === null) return Object.freeze({ kind: 'auth_required', reason: 'invalid_saved_response' });
    try {
      const credential = snapshot(recoverCompletedRenewal(current, journal, responseBytes));
      return Object.freeze({ kind: 'recover_completed', credential, issued: journal.issued });
    } catch {
      return Object.freeze({ kind: 'auth_required', reason: 'invalid_saved_response' });
    }
  }

  if (current.refreshExpiresAt !== null && current.refreshExpiresAt <= input.now)
    return Object.freeze({ kind: 'auth_required', reason: 'refresh_expired' });

  // An incomplete/mismatched journal or orphan response is replaced with a fresh issue time.
  const next: RenewalJournal = Object.freeze({ base: current, issued: input.now, completed: false });
  return Object.freeze({ kind: 'prepare_request', journal: next,
    request: Object.freeze({ refreshToken: current.refreshToken, issued: input.now }) });
}
