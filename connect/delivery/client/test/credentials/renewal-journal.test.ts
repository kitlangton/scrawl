import test from 'node:test';
import assert from 'node:assert/strict';
import { decodeRenewalJournal, isCompletedRenewalFor, recoverCompletedRenewal, RenewalJournalError } from '../../src/credentials/renewal-journal.ts';
import { decodeStoredCredential, type StoredCredential } from '../../src/credentials/wire.ts';

const server = 'https://host.example/connect';
const baseWire = { server, scope: 'upload', site: null, token: 'access', generation: 'a'.repeat(32), refreshToken: 'refresh', expiresAt: 200, refreshExpiresAt: 500, grantId: 'grant', authRequired: false, encryption_identity: 'identity', accountInitial: 'AB' };
const encode = (value: unknown) => new TextEncoder().encode(JSON.stringify(value));
const current = (): StoredCredential => decodeStoredCredential(encode(baseWire), 'upload', server);
const journal = (overrides: Record<string, unknown> = {}) => decodeRenewalJournal(encode({ base: baseWire, issued: 100, completed: true, ...overrides }), { scope: 'upload', approvedServer: server });
const reply = { scope: 'upload', accessToken: 'rotated-access', refreshToken: 'rotated-refresh', grantId: 'grant', expiresIn: 60, refreshExpiresIn: 300 };
const failure = (run: () => unknown, category: string) => assert.throws(run, e => e instanceof RenewalJournalError && e.category === category && !String(e).includes('secret'));

test('decodes legacy missing completed as false, ignores outer unknowns, and freezes a normalized fresh base', () => {
  const raw = { base: { ...baseWire, ignoredFutureField: 'discard' }, issued: 0, unknown: { future: true } };
  const decoded = decodeRenewalJournal(encode(raw), { scope: 'upload', approvedServer: server });
  assert.equal(decoded.completed, false);
  assert.equal(decoded.issued, 0);
  assert.equal('ignoredFutureField' in decoded.base, false);
  assert.ok(Object.isFrozen(decoded));
  assert.ok(Object.isFrozen(decoded.base));
});

test('rejects null/wrong journal fields, malformed/duplicate JSON, bounds and unsafe issued times', () => {
  for (const value of [{ base: null, issued: 1 }, { base: baseWire, issued: -1 }, { base: baseWire, issued: Number.MAX_SAFE_INTEGER + 1 }, { base: baseWire, issued: 1, completed: null }, { base: baseWire, issued: 1, completed: 'yes' }])
    failure(() => decodeRenewalJournal(encode(value), { scope: 'upload', approvedServer: server }), 'invalid_journal');
  for (const text of ['{"base":{},"base":{},"issued":1}', '{'])
    failure(() => decodeRenewalJournal(new TextEncoder().encode(text), { scope: 'upload', approvedServer: server }), 'invalid_journal');
  failure(() => decodeRenewalJournal(new Uint8Array(65_537), { scope: 'upload', approvedServer: server }), 'invalid_journal');
});

test('requires exact known credential fields independent of object property order', () => {
  const reordered = Object.fromEntries(Object.entries(baseWire).reverse());
  const decoded = decodeRenewalJournal(encode({ issued: 100, completed: true, base: reordered }), { scope: 'upload', approvedServer: server });
  assert.equal(isCompletedRenewalFor(current(), decoded), true);
  const keys = ['server', 'site', 'scope', 'token', 'generation', 'refreshToken', 'expiresAt', 'refreshExpiresAt', 'grantId', 'authRequired', 'encryption_identity', 'accountInitial'] as const;
  for (const key of keys) {
    const value = key === 'server' ? 'https://other.example/connect'
      : key === 'site' || key === 'encryption_identity' ? `different-${key}`
      : key === 'scope' ? 'read'
      : key === 'generation' ? 'b'.repeat(32)
      : key === 'expiresAt' || key === 'refreshExpiresAt' ? 201
      : key === 'authRequired' ? true
      : key === 'accountInitial' ? 'ZZ'
      : `different-${key}`;
    const altered = { ...baseWire, [key]: value };
    const context = { scope: key === 'scope' ? 'read' as const : 'upload' as const, approvedServer: key === 'server' ? 'https://other.example/connect' : server };
    const changed = decodeRenewalJournal(encode({ base: altered, issued: 100, completed: true }), context);
    assert.equal(isCompletedRenewalFor(current(), changed), false, key);
  }
});

test('incomplete/orphan journals and auth-required credentials cannot recover', () => {
  assert.equal(isCompletedRenewalFor(current(), journal({ completed: false })), false);
  failure(() => recoverCompletedRenewal(current(), journal({ completed: false }), encode(reply)), 'not_recoverable');
  const auth = decodeStoredCredential(encode({ ...baseWire, authRequired: true }), 'upload', server);
  failure(() => recoverCompletedRenewal(auth, journal(), encode(reply)), 'auth_required');
  const legacy = decodeStoredCredential(encode({ server, scope: 'upload', token: 'legacy', generation: 'b'.repeat(32) }), 'upload', server);
  failure(() => recoverCompletedRenewal(legacy, journal(), encode(reply)), 'not_recoverable');
});

test('recovers a completed response after old refresh expiry using original issue time', () => {
  const old = decodeStoredCredential(encode({ ...baseWire, refreshExpiresAt: 1 }), 'upload', server);
  const saved = decodeRenewalJournal(encode({ base: { ...baseWire, refreshExpiresAt: 1 }, issued: 100, completed: true }), { scope: 'upload', approvedServer: server });
  const result = recoverCompletedRenewal(old, saved, encode(reply));
  assert.equal(result.expiresAt, 160);
  assert.equal(result.refreshExpiresAt, 400);
  assert.equal(result.server, server);
  assert.equal(result.generation, old.generation);
  assert.equal(result.encryption_identity, old.encryption_identity);
  assert.equal(result.authRequired, false);
  assert.ok(Object.isFrozen(result));
});

test('rejects malformed, oversized, rotated scope or grant replies without input mutation or unsafe error text', () => {
  const old = current();
  const before = { ...old };
  for (const badReply of [{ ...reply, scope: 'read' }, { ...reply, grantId: 'other' }])
    failure(() => recoverCompletedRenewal(old, journal(), encode(badReply)), 'invalid_journal');
  failure(() => recoverCompletedRenewal(old, journal(), new Uint8Array(2_000_001)), 'invalid_journal');
  failure(() => recoverCompletedRenewal(old, journal(), new TextEncoder().encode('{"scope":"secret","scope":"x"}')), 'invalid_journal');
  assert.deepEqual(old, before);
  assert.ok(Object.isFrozen(old));
});
