import assert from 'node:assert/strict';
import test from 'node:test';
import { encodeCursor, listShares } from '../src/shares.ts';

const origin = 'https://connect.example.test';
const id = 'a'.repeat(32);
const hash = 'b'.repeat(64);
const row = { id, captureId: 'c'.repeat(32), url: `${origin}/s/${hash}`, status: 'active', createdAt: 123, expiresAt: 456, title: 'Synthetic title' };

test('lists all pages in order and returns the stable terminal CLI schema', async () => {
  const requested: (string | null)[] = [];
  const result = await listShares(origin, async cursor => {
    requested.push(cursor);
    return cursor === null ? { shares: [row], nextCursor: '1234:opaque /?&+' } : { shares: [{ ...row, id: 'd'.repeat(32) }], nextCursor: null };
  });
  assert.deepEqual(requested, [null, '1234:opaque /?&+']);
  assert.deepEqual(result, { shares: [row, { ...row, id: 'd'.repeat(32) }], nextCursor: null });
});

test('cursor encoding is UTF-8 percent encoding with uppercase hex', () => {
  assert.equal(encodeCursor('a /?&+é~'), 'a%20%2F%3F%26%2B%C3%A9~');
});

test('rejects repeated cursors and malformed pages', async () => {
  await assert.rejects(listShares(origin, async () => ({ shares: [], nextCursor: 'cycle' })), /Repeated shares cursor/);
  await assert.rejects(listShares(origin, async () => ({ shares: {}, nextCursor: null })), /Invalid shares response/);
  await assert.rejects(listShares(origin, async () => ({ shares: Array(101).fill(row), nextCursor: null })), /Invalid shares response/);
  await assert.rejects(listShares(origin, async () => ({ shares: [], nextCursor: 'x'.repeat(4097) })), /Invalid shares cursor/);
});

test('validates exact identifiers, capability URLs and inactive terminal rows', async () => {
  const withPage = (share: unknown) => listShares(origin, async () => ({ shares: [share], nextCursor: null }));
  await assert.rejects(withPage({ ...row, id: 'A'.repeat(32) }), /Invalid share identity/);
  await assert.rejects(withPage({ ...row, url: `${origin}/s/${hash}/extra` }), /Invalid share URL/);
  await assert.rejects(withPage({ ...row, status: 'revoked' }), /Inactive share exposed a URL/);
  const inactive = await withPage({ ...row, url: null, status: 'expired' });
  assert.equal(inactive.shares[0]?.url, null);
});

test('propagates request failures without exposing request or credential data', async () => {
  await assert.rejects(listShares(origin, async () => { throw new Error('Hosted API: unauthorized'); }), /Hosted API: unauthorized/);
});
