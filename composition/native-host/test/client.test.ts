import assert from 'node:assert/strict';
import { once } from 'node:events';
import { createServer, type Socket } from 'node:net';
import { chmod, mkdtemp, mkdir, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { createHash } from 'node:crypto';
import { MAX_SERIALIZED_FRAME_BYTES, MAX_UTF8_TEXT_BYTES } from '@apparent/app-contracts/native';
import { createNativeHostPort, createNativeClipboardPort, createNativePenPort } from '../src/client.ts';

const facts = {
  api: 1, processEpoch: 'epoch-a', revision: 4, scope: 'running-desktop-process', freshness: 'cached-gui-observation',
  permissions: [{ id: 'screen-recording', status: 'granted', usableNow: false, relaunchNeeded: true, actions: [] }],
  shortcuts: {
    record: { configured: { keyCode: 1, modifiers: 2560, label: '⌥⇧S' }, effective: null, registration: 'unknown', canValidate: true, canSave: false },
    paste: { configured: null, effective: null, registration: 'unknown', canValidate: true, canSave: false },
  },
};

const DROP = Symbol('close without a reply');
async function withSocket(reply: (request: Record<string, unknown>) => unknown, action: () => Promise<unknown>, corruptId = false) {
  const root = await mkdtemp(join(tmpdir(), 'apparent-native-host-'));
  const socketDir = join(root, '.config/apparent/agent-control');
  await mkdir(socketDir, { recursive: true, mode: 0o700 });
  await chmod(root, 0o700);
  await chmod(join(root, '.config'), 0o700);
  await chmod(join(root, '.config/apparent'), 0o700);
  await chmod(socketDir, 0o700);
  const path = join(socketDir, 'capture.sock');
  const connections = new Set<Socket>();
  const server = createServer(connection => {
    connections.add(connection);
    connection.once('close', () => connections.delete(connection));
    let input = '';
    let handled = false;
    connection.on('data', async chunk => {
      if (handled) return;
      input += chunk.toString('utf8');
      const end = input.indexOf('\n');
      if (end < 0) return;
      handled = true;
      const request = JSON.parse(input.slice(0, end)) as Record<string, unknown>;
      const result = await reply(request);
      if (result === DROP) { connection.end(); return; }
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
    for (const connection of connections) connection.destroy();
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


const expected = { processEpoch: 'epoch-a', revision: 4 };
const shortcut = { keyCode: 1, modifiers: 2560, label: '⌥⇧S' };

test('lost, mismatched, oversized and invalid mutation replies remain unknown without retry', async () => {
  for (const mode of ['drop', 'bad-id', 'oversized', 'invalid-result']) {
    let dispatches = 0;
    await withSocket(() => {
      dispatches++;
      if (mode === 'drop') return DROP;
      if (mode === 'oversized') return { padding: 'x'.repeat(4096) };
      if (mode === 'invalid-result') return { status: 'refused', reason: 'unsupported-reason' };
      return { status: 'accepted' };
    }, async () => {
      assert.deepEqual(await createNativeHostPort().saveRecordShortcut(shortcut, expected), { status: 'outcome-unknown' }, mode);
    }, mode === 'bad-id');
    assert.equal(dispatches, 1, mode);
  }
});

test('every effect method preserves explicit refusal and uncertainty', async () => {
  for (const response of [{ status: 'refused', reason: 'stale' }, { status: 'outcome-unknown' }]) {
    await withSocket(() => response, async () => {
      const port = createNativeHostPort();
      assert.deepEqual(await port.requestPermission('microphone', expected), response);
      assert.deepEqual(await port.openSystemSettings('accessibility', expected), response);
      assert.deepEqual(await port.validateRecordShortcut('s', 2560, expected), response);
      assert.deepEqual(await port.saveRecordShortcut(shortcut, expected), response);
    });
  }
});

test('rejects malformed facts and validation success before they become a save candidate', async () => {
  const malformed = [
    { ...facts, permissions: [...facts.permissions, ...facts.permissions] },
    { ...facts, permissions: [{ ...facts.permissions[0], id: 'unknown-permission' }] },
    { ...facts, shortcuts: { ...facts.shortcuts, record: { ...facts.shortcuts.record, configured: { ...shortcut, keyCode: -1 } } } },
    { ...facts, shortcuts: { ...facts.shortcuts, record: { ...facts.shortcuts.record, canSave: true } } },
  ];
  for (const value of malformed) await withSocket(() => value, async () => {
    await assert.rejects(createNativeHostPort().readSnapshot(), /Native host unavailable/);
  });
  for (const candidate of [{ ...shortcut, keyCode: 256 }, { ...shortcut, label: '' }, { ...shortcut, extra: true }]) {
    await withSocket(() => ({ status: 'accepted', shortcut: candidate }), async () => {
      assert.deepEqual(await createNativeHostPort().validateRecordShortcut('s', 2560, expected), { status: 'outcome-unknown' });
    });
  }
});

test('captures the endpoint and request revision instead of following later caller mutations', async () => {
  await withSocket(request => {
    assert.deepEqual(request.expected, expected);
    return { status: 'accepted' };
  }, async () => {
    const port = createNativeHostPort();
    const state = { ...expected };
    const saving = port.saveRecordShortcut(shortcut, state);
    state.revision = 99;
    process.env.HOME = join(tmpdir(), 'not-the-bound-native-home');
    assert.deepEqual(await saving, { status: 'accepted' });
  });
});

test('bounds concurrent pending connections per injected facade', async () => {
  let release!: () => void, dispatches = 0;
  const gate = new Promise<void>(resolve => { release = resolve; });
  await withSocket(async () => { dispatches++; await gate; return facts; }, async () => {
    const port = createNativeHostPort();
    const pending = Array.from({ length: 16 }, () => port.readSnapshot());
    await assert.rejects(port.readSnapshot(), /Native host unavailable/);
    release();
    await Promise.all(pending);
    assert.equal(dispatches, 16);
  });
});

test('typed pen facade sends raw exact RGB and distinguishes applied readback, conflict, and unknown', async () => {
  const expectedPenColor = { present: true, value: 'purple' };
  await withSocket(request => {
    assert.equal(request.operation, 'applyStrokeColor');
    assert.deepEqual(request.expected, expected);
    assert.deepEqual(request.expectedPenColor, expectedPenColor);
    assert.deepEqual(request.rgb, [122, 88, 199]);
    return { status: 'applied', rgb: [122, 88, 199] };
  }, async () => {
    assert.deepEqual(await createNativePenPort().applyStrokeColor([122, 88, 199], expectedPenColor, expected),
      { status: 'applied', rgb: [122, 88, 199] });
  });
  await withSocket(() => ({ status: 'conflict', current: { present: false, value: null } }), async () => {
    assert.deepEqual(await createNativePenPort().applyStrokeColor(null, { present: false, value: null }, expected),
      { status: 'conflict', current: { present: false, value: null } });
  });
  await withSocket(() => ({ status: 'refused', reason: 'busy' }), async () => {
    assert.deepEqual(await createNativePenPort().applyStrokeColor([1, 2, 3], expectedPenColor, expected), { status: 'unavailable' });
  });
  await withSocket(() => DROP, async () => {
    assert.deepEqual(await createNativePenPort().applyStrokeColor([0, 0, 0], expectedPenColor, expected), { status: 'outcome-unknown' });
  });
  const port = createNativePenPort();
  for (const rgb of [[-1, 0, 0], [0, 1.5, 0], [0, 0, 256], [1, 2], [1, 2, 3, 4]]) {
    await assert.rejects(port.applyStrokeColor(rgb as [number, number, number], expectedPenColor, expected), /Invalid pen request/);
  }
  await assert.rejects(port.applyStrokeColor([1, 2, 3], { present: false, value: 'must-be-null' }, expected), /Invalid pen request/);
});

test('read-only pen snapshots carry actual scoped telemetry, preserve None versus black, and never retry', async () => {
  for (const rgb of [null, [0, 0, 0]]) {
    await withSocket(request => {
      assert.equal(request.operation, 'readStrokeColor');
      assert.deepEqual(Object.keys(request).sort(), ['deadline', 'id', 'operation', 'owner', 'protocol']);
      return { api: 1, processEpoch: 'epoch-read', scope: 'running-desktop-process', rgb };
    }, async () => {
      assert.deepEqual(await createNativePenPort().readStrokeColor(), {
        api: 1, processEpoch: 'epoch-read', scope: 'running-desktop-process', rgb,
      });
    });
  }
  for (const malformed of [
    { api: 1, processEpoch: 'epoch', scope: 'config', rgb: null },
    { api: 1, processEpoch: '../epoch', scope: 'running-desktop-process', rgb: null },
    { api: 1, processEpoch: 'epoch', scope: 'running-desktop-process', rgb: [1, 2, 256] },
    { api: 1, processEpoch: 'epoch', scope: 'running-desktop-process', rgb: [0, 0, 0, 0] },
    { api: 1, processEpoch: 'epoch', scope: 'running-desktop-process', rgb: null, extra: true },
  ]) await withSocket(() => malformed, async () => {
    await assert.rejects(createNativePenPort().readStrokeColor(), /Native host unavailable/);
  });
  let dispatches = 0;
  await withSocket(() => { dispatches++; return DROP; }, async () => {
    await assert.rejects(createNativePenPort().readStrokeColor(), /Native host unavailable/);
  });
  assert.equal(dispatches, 1, 'read reconciliation sends once and has no implicit retry');
});

test('pen snapshot reads cap pending sockets process-wide', async () => {
  let release!: () => void, eighth!: () => void, dispatches = 0;
  const gate = new Promise<void>(resolve => { release = resolve; });
  const started = new Promise<void>(resolve => { eighth = resolve; });
  await withSocket(async () => {
    dispatches++;
    if (dispatches === 8) eighth();
    await gate;
    return { api: 1, processEpoch: 'epoch', scope: 'running-desktop-process', rgb: null };
  }, async () => {
    const reads = Array.from({ length: 8 }, () => createNativePenPort().readStrokeColor());
    await started;
    await assert.rejects(createNativePenPort().readStrokeColor(), /Native host unavailable/);
    release();
    await Promise.all(reads);
    assert.equal(dispatches, 8);
  });
});

test('pen facade admits one process-wide effect and reports excess work unavailable without dispatch', async () => {
  let release!: () => void, dispatched!: () => void, dispatches = 0;
  const gate = new Promise<void>(resolve => { release = resolve; });
  const started = new Promise<void>(resolve => { dispatched = resolve; });
  await withSocket(async () => {
    dispatches++;
    dispatched();
    await gate;
    return { status: 'applied', rgb: [1, 2, 3] };
  }, async () => {
    const first = createNativePenPort().applyStrokeColor([1, 2, 3], { present: false, value: null }, expected);
    await started;
    assert.deepEqual(await createNativePenPort().applyStrokeColor([4, 5, 6], { present: false, value: null }, expected),
      { status: 'unavailable' });
    release();
    assert.deepEqual(await first, { status: 'applied', rgb: [1, 2, 3] });
    assert.equal(dispatches, 1);
  });
});

test('clipboard facade binds exact text digest and byte length without returning clipboard data', async () => {
  const text = 'quote " slash \\ tab\t newline\nλ 🖍';
  const bytes = Buffer.from(text, 'utf8');
  await withSocket(request => {
    assert.equal(request.operation, 'writeClipboardText');
    assert.deepEqual(Object.keys(request).sort(), ['deadline', 'id', 'observed', 'operation', 'owner', 'protocol', 'text']);
    assert.deepEqual(request.observed, expected);
    assert.equal(request.text, text);
    return { status: 'applied', digest: createHash('sha256').update(bytes).digest('hex'), byteLength: bytes.length };
  }, async () => {
    assert.deepEqual(await createNativeClipboardPort().writeText(text, expected), {
      status: 'applied', digest: createHash('sha256').update(bytes).digest('hex'), byteLength: bytes.length,
    });
  });
});

test('clipboard bounds include worst-case JSON escaping, maximum envelope, and the line feed', async () => {
  assert.equal(MAX_SERIALIZED_FRAME_BYTES, 131_072);
  assert.equal(MAX_UTF8_TEXT_BYTES, 21_800);
  const text = '\u0001'.repeat(MAX_UTF8_TEXT_BYTES);
  let dispatches = 0;
  await withSocket(request => {
    dispatches++;
    assert.equal(request.text, text);
    const frameBytes = Buffer.byteLength(`${JSON.stringify(request)}\n`);
    assert.ok(frameBytes <= MAX_SERIALIZED_FRAME_BYTES);
    assert.ok(frameBytes >= MAX_SERIALIZED_FRAME_BYTES - 20);
    const bytes = Buffer.from(text, 'utf8');
    return { status: 'applied', digest: createHash('sha256').update(bytes).digest('hex'), byteLength: bytes.length };
  }, async () => {
    assert.equal((await createNativeClipboardPort().writeText(text, {
      processEpoch: 'e'.repeat(64), revision: Number.MAX_SAFE_INTEGER,
    })).status, 'applied');
  });
  assert.equal(dispatches, 1);
  await withSocket(() => { dispatches++; return { status: 'applied', digest: '0'.repeat(64), byteLength: MAX_UTF8_TEXT_BYTES + 1 }; }, async () => {
    assert.deepEqual(await createNativeClipboardPort().writeText(`${text}a`, expected), { status: 'refused', reason: 'too-large' });
  });
  assert.equal(dispatches, 1);
});

test('clipboard rejects NUL and malformed UTF-16 before transport and never truncates', async () => {
  let dispatches = 0;
  await withSocket(() => { dispatches++; return DROP; }, async () => {
    const port = createNativeClipboardPort();
    for (const text of ['before\0after', '\ud800', '\udc00']) {
      assert.deepEqual(await port.writeText(text, expected), { status: 'refused', reason: 'invalid-text' });
    }
  });
  assert.equal(dispatches, 0);
});

test('clipboard facade admits one write and refuses concurrent work before dispatch', async () => {
  let release!: () => void, dispatched!: () => void, dispatches = 0;
  const gate = new Promise<void>(resolve => { release = resolve; });
  const started = new Promise<void>(resolve => { dispatched = resolve; });
  const digest = createHash('sha256').update('first').digest('hex');
  await withSocket(async () => {
    dispatches++;
    dispatched();
    await gate;
    return { status: 'applied', digest, byteLength: 5 };
  }, async () => {
    const first = createNativeClipboardPort().writeText('first', expected);
    await started;
    assert.deepEqual(await createNativeClipboardPort().writeText('second', expected), { status: 'refused', reason: 'busy' });
    release();
    assert.deepEqual(await first, { status: 'applied', digest, byteLength: 5 });
    assert.equal(dispatches, 1);
  });
});

test('clipboard stale, busy, unavailable, malformed receipt, and lost reply stay truthful without retry', async () => {
  for (const result of [
    { status: 'refused', reason: 'stale' },
    { status: 'refused', reason: 'busy' },
    { status: 'refused', reason: 'unavailable' },
    { status: 'applied', digest: 'bad', byteLength: 2 },
    DROP,
  ]) {
    let dispatches = 0;
    await withSocket(() => { dispatches++; return result; }, async () => {
      const receipt = await createNativeClipboardPort().writeText('exact', expected);
      if (result === DROP || (typeof result === 'object' && result.status === 'applied')) {
        assert.deepEqual(receipt, { status: 'outcome-unknown' });
      } else assert.deepEqual(receipt, result);
    });
    assert.equal(dispatches, 1);
  }
});

test('clipboard refuses a different digest even when its byte length matches', async () => {
  await withSocket(() => ({ status: 'applied', digest: '0'.repeat(64), byteLength: 5 }), async () => {
    assert.deepEqual(await createNativeClipboardPort().writeText('exact', expected), { status: 'outcome-unknown' });
  });
});
