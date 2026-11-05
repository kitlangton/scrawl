import assert from 'node:assert/strict';
import { once } from 'node:events';
import { createServer } from 'node:net';
import { chmod, mkdtemp, mkdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { createNativeHostPort } from '../src/client.ts';

const facts = {
  api: 1, processEpoch: 'epoch-a', revision: 4, scope: 'running-desktop-process', freshness: 'cached-gui-observation',
  permissions: [{ id: 'screen-recording', status: 'granted', usableNow: false, relaunchNeeded: true, actions: [] }],
  shortcuts: {
    record: { configured: { keyCode: 1, modifiers: 2560, label: '⌥⇧S' }, effective: null, registration: 'unknown', canValidate: true, canSave: false },
    paste: { configured: null, effective: null, registration: 'unknown', canValidate: true, canSave: false },
  },
};

async function withSocket(reply: (request: Record<string, unknown>) => unknown, action: () => Promise<unknown>, corruptId = false) {
  const root = await mkdtemp(join(tmpdir(), 'apparent-native-host-'));
  const socketDir = join(root, '.config/apparent/agent-control');
  await mkdir(socketDir, { recursive: true, mode: 0o700 });
  await chmod(root, 0o700);
  await chmod(join(root, '.config'), 0o700);
  await chmod(join(root, '.config/apparent'), 0o700);
  await chmod(socketDir, 0o700);
  const path = join(socketDir, 'capture.sock');
  const server = createServer(connection => {
    let input = '';
    connection.on('data', chunk => {
      input += chunk.toString('utf8');
      const end = input.indexOf('\n');
      if (end < 0) return;
      const request = JSON.parse(input.slice(0, end)) as Record<string, unknown>;
      const result = reply(request);
      connection.end(`${JSON.stringify({ protocol: 1, id: corruptId ? `${request.id}-other` : request.id, result })}\n`);
    });
  });
  server.listen(path);
  await once(server, 'listening');
  await chmod(path, 0o600);
  const oldHome = process.env.HOME;
  process.env.HOME = root;
  try { return await action(); }
  finally {
    if (oldHome === undefined) delete process.env.HOME; else process.env.HOME = oldHome;
    await new Promise<void>(resolve => server.close(() => resolve()));
    await rm(root, { recursive: true, force: true });
  }
}

test('uses only the Record-bound fixed socket and validates a correlated snapshot', async () => {
  await withSocket(request => {
    assert.equal(request.protocol, 1);
    assert.equal(request.owner, 'record');
    assert.equal(request.operation, 'readSnapshot');
    assert.equal(typeof request.id, 'string');
    assert.equal(Number.isSafeInteger(request.deadline), true);
    return facts;
  }, async () => {
    const snapshot = await createNativeHostPort().readSnapshot();
    assert.equal(snapshot.processEpoch, 'epoch-a');
    assert.equal(snapshot.permissions[0]?.relaunchNeeded, true);
    assert.equal(snapshot.shortcuts.record.registration, 'unknown');
  });
});

test('rejects a mismatched response correlation id', async () => {
  await withSocket(() => facts, async () => {
    await assert.rejects(createNativeHostPort().readSnapshot(), /Native host unavailable/);
  }, true);
});

test('rejects unsupported operations and permission identifiers before connecting', async () => {
  const port = createNativeHostPort();
  await assert.rejects(port.requestPermission('../config', { processEpoch: 'e', revision: 1 }), /Unsupported permission/);
  await assert.rejects(port.validateRecordShortcut('x'.repeat(17), 2560, { processEpoch: 'e', revision: 1 }), /Invalid shortcut/);
  await assert.rejects(port.saveRecordShortcut({ keyCode: 999, modifiers: 2560, label: '⌥⇧S' }, { processEpoch: 'e', revision: 1 }), /Invalid shortcut/);
});
