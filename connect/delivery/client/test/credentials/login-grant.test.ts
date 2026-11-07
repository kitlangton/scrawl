import assert from 'node:assert/strict';
import test from 'node:test';
import { prepareLoginCredential } from '../../src/credentials/login-grant.ts';
import { decodeStoredCredential } from '../../src/credentials/wire.ts';

const context = (scope: 'read' | 'upload' = 'upload', extra: Record<string, unknown> = {}) => ({
  approvedServer: 'https://api.example.test/connect', siteOrigin: 'https://approval.example.test',
  scope, generation: 'a'.repeat(32), issuedAt: 1_700_000_000,
  encryptionIdentity: scope === 'read' ? 'b'.repeat(32) : null, ...extra,
});
const reply = (scope: 'read' | 'upload' = 'upload', extra: Record<string, unknown> = {}) => ({
  status: 'approved', scope, accessToken: 'access-token', refreshToken: 'refresh-token',
  grantId: 'grant-id', expiresIn: 3600, refreshExpiresIn: 7200, accountInitial: 'Å1',
  unknown: { private: 'discard me' }, ...extra,
});
const prepare = (r = reply(), c = context()) => prepareLoginCredential(r, c);

test('prepares both scopes with distinct API and approval origins', () => {
  const upload = prepare();
  assert.equal(upload.scope, 'upload');
  assert.equal(upload.server, 'https://api.example.test/connect');
  assert.equal(upload.site, 'https://approval.example.test');
  assert.equal(upload.encryption_identity, null);
  assert.equal(upload.authRequired, false);
  const read = prepare(reply('read'), context('read'));
  assert.equal(read.scope, 'read');
  assert.equal(read.encryption_identity, 'b'.repeat(32));
});

test('uses original issue time and discards unknown reply fields', () => {
  const result = prepare(reply('upload', { accountInitial: 'M' }), context('upload', { issuedAt: 100 }));
  assert.equal(result.expiresAt, 3700);
  assert.equal(result.refreshExpiresAt, 7300);
  assert.equal(result.accountInitial, 'M');
  assert.equal('unknown' in result, false);
});

test('allows upload identity to be absent or valid, while read requires one', () => {
  assert.equal(prepare(reply('upload'), context('upload', { encryptionIdentity: null })).encryption_identity, null);
  assert.equal(prepare(reply('upload'), context('upload', { encryptionIdentity: 'c'.repeat(32) })).encryption_identity, 'c'.repeat(32));
  assert.throws(() => prepare(reply('read'), context('read', { encryptionIdentity: null })));
});

test('rejects invalid status, scope, token, grant, expiry, generation, endpoints and identity', () => {
  const cases: Array<[unknown, Record<string, unknown>]> = [
    [reply('upload', { status: 'pending' }), {}], [reply('read'), {}],
    [reply('upload', { accessToken: '' }), {}], [reply('upload', { refreshToken: 'x'.repeat(8193) }), {}],
    [reply('upload', { grantId: 'x'.repeat(513) }), {}], [reply('upload', { expiresIn: 0 }), {}],
    [reply('upload', { refreshExpiresIn: Number.MAX_SAFE_INTEGER }), { issuedAt: 1 }],
    [reply(), { generation: 'A'.repeat(32) }], [reply(), { approvedServer: 'https://api.example.test/v1' }],
    [reply(), { approvedServer: 'http://example.test' }], [reply(), { siteOrigin: 'https://approval.example.test/path' }],
    [reply(), { siteOrigin: 'https://u:p@approval.example.test' }], [reply(), { siteOrigin: 'http://approval.example.test' }],
    [reply(), { encryptionIdentity: 'C'.repeat(32) }],
  ];
  for (const [r, change] of cases) assert.throws(() => prepare(r, context('upload', change)));
  assert.throws(() => prepare(reply('read'), context('read', { encryptionIdentity: null })));
  assert.throws(() => prepare(reply(), context('upload', { issuedAt: Number.MAX_SAFE_INTEGER })));
});

test('filters invalid account initials and returns a frozen detached scalar snapshot', () => {
  const input = reply('upload', { accountInitial: 'a@b' });
  const result = prepare(input);
  assert.equal(result.accountInitial, null);
  input.accessToken = 'mutated';
  input.accountInitial = 'X';
  assert.equal(result.token, 'access-token');
  assert.equal(result.accountInitial, null);
  assert.equal(Object.isFrozen(result), true);
  assert.throws(() => { (result as { token: string }).token = 'mutated'; });
});

test('errors contain only a safe category and no private values', () => {
  const secret = 'secret-access-value';
  let error: unknown;
  try { prepare(reply('upload', { accessToken: secret }), context('read')); } catch (caught) { error = caught; }
  assert.ok(error instanceof Error);
  assert.equal(error.message, 'Invalid login credential');
  assert.equal(error.message.includes(secret), false);
});

test('prepared credential passes the existing wire reader with original issue time', () => {
  const result = prepare(reply('read'), context('read', { issuedAt: 0 }));
  assert.deepEqual(decodeStoredCredential(new TextEncoder().encode(JSON.stringify(result)), 'read', result.server), result);
  assert.equal(result.expiresAt, 3600);
  assert.equal(prepare(reply('upload', { accountInitial: 'x'.repeat(100_000) })).accountInitial, null);
  for (const accountInitial of ['𐐀𐐁𐐂', 'AB3', '']) {
    assert.equal(prepare(reply('upload', { accountInitial })).accountInitial, accountInitial || null);
  }
});
