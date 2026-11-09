import { test } from 'node:test';
import assert from 'node:assert/strict';
import { availablePort } from './testing/port.ts';
import { chmodSync, lstatSync, mkdtempSync, readFileSync, rmSync, symlinkSync, unlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { publishKey } from './key.ts';
import { mailbox } from './mailbox.ts';
import type { View } from './protocol.ts';
import { mailboxId, sign, authenticatedState } from '../../../../../../hermes/desktop/controller.ts';
const token = '0123456789abcdef0123456789abcdef';
const key = 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';
test('mailbox signs only owned state and uses a separate authenticated acknowledgement', async () => {
  const port = await availablePort();
  let view: View = { phase: 'recording', source: '/private/not-saved.apparent' };
  let acks = 0;
  const server = await mailbox(token, port, key, () => view, () => { acks++; view = { phase: 'cancelled' }; });
  const id = await mailboxId(token);
  const state = `state/${await sign(key, `state:${id}`)}`;
  const ack = `ack/${await sign(key, `ack:${id}`)}`;
  try {
    const read = (suffix = state, origin = 'null', held = id) => fetch(`http://127.0.0.1:${port}/${held}/${suffix}`, { headers: { Origin: origin }, ...(suffix.startsWith('ack/') ? { method: 'POST' } : {}) });
    assert.deepEqual(await authenticatedState(key, await (await read()).json()), { phase: 'recording' });
    assert.deepEqual(await authenticatedState(key, await (await fetch(`http://127.0.0.1:${port}/${id}/${state}`)).json()), { phase: 'recording' });
    assert.equal((await read(state, 'https://evil.example')).status, 403);
    assert.equal((await read(state, 'null', token)).status, 404);
    assert.equal((await read('state')).status, 404);
    assert.equal((await read(`state/${await sign(token, `state:${id}`)}`)).status, 404);
    assert.equal((await read(ack)).status, 404);
    view = { phase: 'saved', source: '/private/owned.apparent' };
    const packet = await (await read()).json();
    assert.equal((await authenticatedState(key, packet)).source, '/private/owned.apparent');
    await assert.rejects(authenticatedState(token, packet), /Unauthenticated/);
    assert.equal((await read(`ack/${await sign(token, `ack:${id}`)}`)).status, 404);
    assert.equal((await read('ack/' + packet.mac)).status, 404);
    assert.equal((await read(ack)).status, 200);
    assert.equal((await read(ack)).status, 404);
    assert.equal(acks, 1);
    await assert.rejects(mailbox(token, port, key, () => view, () => {}));
  } finally { await new Promise<void>(resolve => server.close(() => resolve())); }
});
test('bridge keys require private owned directories, are exclusive, and clean up only their own file', () => {
  const root = mkdtempSync(path.join(tmpdir(), 'hermes-key-'));
  const file = path.join(root, 'apparent-hermes', `${token}.key`);
  try {
    const remove = publishKey(root, token, key);
    assert.equal(readFileSync(file, 'utf8'), key);
    assert.equal(lstatSync(file).mode & 0o777, 0o600);
    assert.equal(lstatSync(path.dirname(file)).mode & 0o777, 0o700);
    assert.throws(() => publishKey(root, token, key), /EEXIST/);
    remove(); remove();
    assert.throws(() => lstatSync(file), /ENOENT/);
    const removeReplacement = publishKey(root, token, key);
    unlinkSync(file);
    const outside = path.join(root, 'unrelated'); writeFileSync(outside, 'preserve'); symlinkSync(outside, file);
    removeReplacement();
    assert.ok(lstatSync(file).isSymbolicLink());
    assert.equal(readFileSync(outside, 'utf8'), 'preserve');
    assert.throws(() => publishKey(root, token, key), /EEXIST/);
    unlinkSync(file);
    chmodSync(path.dirname(file), 0o755);
    assert.throws(() => publishKey(root, token, key), /private user runtime/);
    rmSync(path.dirname(file), { recursive: true }); symlinkSync(root, path.dirname(file));
    assert.throws(() => publishKey(root, token, key), /private user runtime/);
    unlinkSync(path.dirname(file));
    chmodSync(root, 0o755);
    assert.throws(() => publishKey(root, token, key), /private user runtime/);
  } finally { rmSync(root, { recursive: true, force: true }); }
});
