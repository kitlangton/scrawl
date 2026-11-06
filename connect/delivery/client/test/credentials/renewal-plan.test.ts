import test from 'node:test';
import assert from 'node:assert/strict';
import { planRenewal, RenewalPlanError } from '../../src/credentials/renewal-plan.ts';
import { decodeRenewalJournal } from '../../src/credentials/renewal-journal.ts';
import { decodeStoredCredential, type StoredCredential } from '../../src/credentials/wire.ts';

const server = 'https://host.example/connect';
const wire = { server, site: null, scope: 'upload', token: 'access', generation: 'a'.repeat(32), refreshToken: 'refresh', expiresAt: 200, refreshExpiresAt: 500, grantId: 'grant', authRequired: false, encryption_identity: 'identity', accountInitial: 'AB' };
const encode = (v: unknown) => new TextEncoder().encode(JSON.stringify(v));
const credential = (patch: Record<string, unknown> = {}): StoredCredential => decodeStoredCredential(encode({ ...wire, ...patch }), 'upload', server);
const altered = (patch: Record<string, unknown>): StoredCredential => ({ ...credential(), ...patch }) as StoredCredential;
const journal = (patch: Record<string, unknown> = {}) => decodeRenewalJournal(encode({ base: wire, issued: 100, completed: true, ...patch }), { scope: 'upload', approvedServer: server });
const reply = { scope: 'upload', accessToken: 'new-access', refreshToken: 'new-refresh', grantId: 'grant', expiresIn: 60, refreshExpiresIn: 300 };
const input = (patch: Record<string, unknown> = {}) => ({ expected: credential(), current: credential(), now: 100, force: false, cancelled: false, revocationPending: false, journal: null, responseBytes: null, ...patch });

test('fences cancellation, revocation, generation, server, scope and authRequired in donor order', () => {
  assert.deepEqual(planRenewal(input({ cancelled: true, revocationPending: true })).kind, 'cancelled');
  assert.deepEqual(planRenewal(input({ revocationPending: true })).kind, 'cancelled');
  assert.deepEqual(planRenewal(input({ current: credential({ generation: 'b'.repeat(32) }) })).kind, 'stale');
  assert.deepEqual(planRenewal(input({ current: altered({ server: 'https://other.example' }) })).kind, 'stale');
  assert.deepEqual(planRenewal(input({ current: altered({ scope: 'read' }) })).kind, 'stale');
  assert.deepEqual(planRenewal(input({ current: credential({ authRequired: true }) })), { kind: 'auth_required', reason: 'credential' });
});

test('uses a changed credential or not-due credential without duplicate renewal; due boundary is inclusive at 30 seconds', () => {
  assert.equal(planRenewal(input({ current: credential({ token: 'rotated' }) })).kind, 'use_current');
  assert.equal(planRenewal(input({ now: 169 })).kind, 'use_current');
  const due = planRenewal(input({ now: 170 }));
  assert.equal(due.kind, 'prepare_request');
  if (due.kind === 'prepare_request') assert.equal(due.journal.issued, 170);
  assert.equal(planRenewal(input({ now: 100, force: true })).kind, 'prepare_request');
});

test('legacy credentials stay usable until due and forced renewal proposes auth-required without refresh token', () => {
  const legacy = credential({ refreshToken: null, expiresAt: 170, refreshExpiresAt: null, grantId: null });
  assert.equal(planRenewal(input({ expected: legacy, current: legacy, now: 100 })).kind, 'use_current');
  assert.deepEqual(planRenewal(input({ expected: legacy, current: legacy, now: 100, force: true })), { kind: 'auth_required', reason: 'no_refresh_token' });
  assert.deepEqual(planRenewal(input({ expected: legacy, current: legacy, now: 170 })), { kind: 'auth_required', reason: 'no_refresh_token' });
});

test('exact-base completed recovery precedes expired refresh rejection and preserves original issue time', () => {
  const expired = credential({ refreshExpiresAt: 1 });
  const saved = decodeRenewalJournal(encode({ base: { ...wire, refreshExpiresAt: 1 }, issued: 100, completed: true }), { scope: 'upload', approvedServer: server });
  const result = planRenewal(input({ expected: expired, current: expired, now: 200, force: true, journal: saved, responseBytes: encode(reply) }));
  assert.equal(result.kind, 'recover_completed');
  if (result.kind === 'recover_completed') {
    assert.equal(result.issued, 100);
    assert.equal(result.credential.expiresAt, 160);
    assert.equal(result.credential.refreshExpiresAt, 400);
  }
});

test('malformed completed reply fails closed; incomplete/mismatched journal starts fresh at current issue time', () => {
  assert.deepEqual(planRenewal(input({ force: true, journal: journal(), responseBytes: encode({ ...reply, grantId: 'wrong' }) })), { kind: 'auth_required', reason: 'invalid_saved_response' });
  for (const saved of [journal({ completed: false }), decodeRenewalJournal(encode({ base: { ...wire, token: 'other' }, issued: 10, completed: false }), { scope: 'upload', approvedServer: server })]) {
    const result = planRenewal(input({ force: true, now: 300, journal: saved, responseBytes: encode(reply) }));
    assert.equal(result.kind, 'prepare_request');
    if (result.kind === 'prepare_request') {
      assert.equal(result.journal.issued, 300);
      assert.equal(result.request.issued, 300);
      assert.equal(result.journal.completed, false);
    }
  }
});

test('plan snapshots caller-owned credentials, journal data and response bytes', () => {
  const expected = { ...wire } as StoredCredential;
  const current = { ...wire } as StoredCredential;
  const base = { ...wire } as StoredCredential;
  const bytes = encode(reply);
  const result = planRenewal(input({ expected, current, now: 100, force: true,
    journal: { base, issued: 40, completed: false }, responseBytes: bytes }));
  assert.equal(result.kind, 'prepare_request');
  if (result.kind === 'prepare_request') {
    expected.token = 'mutated'; current.refreshToken = 'mutated'; base.token = 'mutated'; bytes.fill(0);
    assert.equal(result.journal.base.token, 'access');
    assert.equal(result.request.refreshToken, 'refresh');
    assert.equal(result.journal.issued, 100);
    assert.ok(Object.isFrozen(result) && Object.isFrozen(result.journal) && Object.isFrozen(result.journal.base) && Object.isFrozen(result.request));
  }
});

test('caller-owned Buffer response bytes survive recovery and early refusal', () => {
  const borrowed = Buffer.from(encode(reply)), original = Buffer.from(borrowed);
  assert.equal(planRenewal(input({ force: true, journal: journal(), responseBytes: borrowed })).kind, 'recover_completed');
  assert.deepEqual(borrowed, original);
  assert.equal(planRenewal(input({ cancelled: true, responseBytes: borrowed })).kind, 'cancelled');
  assert.deepEqual(borrowed, original);
});

test('invalid issue clocks fail with an allowlisted category before a journal can be proposed', () => {
  for (const now of [-1, 0.5, NaN, Infinity, Number.MAX_SAFE_INTEGER + 1]) {
    assert.throws(() => planRenewal(input({ force: true, now })), error => error instanceof RenewalPlanError && error.category === 'invalid_plan' && error.message === 'invalid_plan');
  }
});

test('oversized completed replies fail closed while unrelated orphan bytes remain ignored', () => {
  const oversized = new Uint8Array(2_000_001);
  assert.deepEqual(planRenewal(input({ force: true, journal: journal(), responseBytes: oversized })), { kind: 'auth_required', reason: 'invalid_saved_response' });
  assert.equal(planRenewal(input({ force: true, responseBytes: oversized })).kind, 'prepare_request');
});
