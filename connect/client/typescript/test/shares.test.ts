import assert from 'node:assert/strict';
import test from 'node:test';
import { spawn } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import { encodeCursor, listShares } from '../src/shares.ts';

const origin = 'https://connect.example.test';
const id = 'a'.repeat(32);
const hash = 'b'.repeat(64);
const row = { id, captureId: 'c'.repeat(32), url: `${origin}/s/${hash}`, status: 'active', createdAt: '2026-11-10T00:00:00.000Z', expiresAt: '2026-11-17T00:00:00.000Z', title: 'Synthetic title' };

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

test('accepts production ISO timestamps and preserves response extensions', async () => {
  const extended = { ...row, futureMetadata: { label: 'preserved' } };
  const result = await listShares(origin, async () => ({ shares: [extended], nextCursor: null }));
  assert.deepEqual(result.shares, [extended]);
  await assert.rejects(listShares(origin, async () => ({ shares: [{ ...row, createdAt: 123 }], nextCursor: null })), /Invalid share response/);
});

test('bounds accumulated data before an endless sequence reaches a terminal page', async () => {
  let calls = 0;
  const large = { ...row, title: 'x'.repeat(1024 * 1024) };
  await assert.rejects(listShares(origin, async () => ({ shares: [large], nextCursor: `page-${++calls}` })), /64 MiB output limit/);
  assert.ok(calls <= 64);
});

test('private protocol accepts a full-size HTTP page plus its envelope', { timeout: 15_000 }, async t => {
  const page = { shares: [{ ...row, title: '' }], nextCursor: null };
  page.shares[0]!.title = 'x'.repeat(2_000_000 - Buffer.byteLength(JSON.stringify(page)));
  assert.equal(Buffer.byteLength(JSON.stringify(page)), 2_000_000);
  const child = spawn(process.execPath, ['--experimental-strip-types', fileURLToPath(new URL('../src/shares.ts', import.meta.url)), origin], { stdio: ['pipe', 'pipe', 'pipe'] });
  t.after(() => { if (child.exitCode === null) child.kill(); });
  let output = '', error = '', sent = false;
  child.stderr.on('data', chunk => { error += chunk; });
  child.stdout.on('data', chunk => {
    output += chunk;
    if (!sent && output.includes('\n')) {
      sent = true;
      child.stdin.end(JSON.stringify({ ok: true, body: page }) + '\n');
    }
  });
  const code = await new Promise((resolve, reject) => { child.once('exit', resolve); child.once('error', reject); });
  assert.equal(code, 0, error);
  const result = JSON.parse(output.trim().split('\n').at(-1)!);
  assert.equal(result.operation, 'complete');
  assert.deepEqual(result.result, page);
});
