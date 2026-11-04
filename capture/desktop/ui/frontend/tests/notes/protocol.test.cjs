const { test } = require('node:test');
const assert = require('node:assert/strict');
const { validCommand, validSnapshot, popupBounds, badgeBounds, embeddedBadgeBounds, flightBounds } = require('../../electron/notes/protocol.cjs');
test('Notes fixture diagnostics preserve the real readiness barrier and omit error contents', async () => {
  const { EventEmitter } = require('node:events');
  const { readFileSync } = require('node:fs');
  const { join } = require('node:path');
  const { runInNewContext } = require('node:vm');
  const entry = join(__dirname, '../../electron/notes/main.cjs');
  async function run(diagnostics, reject) {
    const windows = [], pending = [], output = [], errors = [];
    const app = new EventEmitter();
    let quit = 0;
    Object.assign(app, { setName() {}, setPath() {}, setActivationPolicy() {}, whenReady: async () => {}, quit: () => quit++ });
    class Window {
      constructor() {
        this.webContents = new EventEmitter();
        Object.assign(this.webContents, { setWindowOpenHandler() {},
          executeJavaScript: () => new Promise((resolve, fail) => pending.push({ resolve, fail })) });
        windows.push(this);
      }
      removeMenu() {}
      setAlwaysOnTop() {}
      setVisibleOnAllWorkspaces() {}
      loadURL() {}
      getMediaSourceId() { return `window:${windows.indexOf(this) + 1}:0`; }
    }
    const electron = { app, BrowserWindow: Window, ipcMain: { handle() {} }, screen: {}, nativeTheme: {},
      session: { defaultSession: { setPermissionRequestHandler() {}, setPermissionCheckHandler() {} } } };
    runInNewContext(readFileSync(entry, 'utf8'), { __dirname: join(__dirname, '../../electron/notes'),
      process: { platform: 'darwin', argv: ['electron', entry, '--notes-profile=/synthetic', '--capture-frame=0,0,1000,700',
        ...(diagnostics ? ['--notes-fixture-diagnostics'] : [])], stdout: { write: v => output.push(JSON.parse(v)) },
        stderr: { write: v => errors.push(v) } },
      require: name => name === 'electron' ? electron : name === '../runtime.cjs'
        ? { installBuildIdentity() {}, installExceptionReporting() {} } : name === 'node:readline'
        ? { createInterface: () => new EventEmitter() } : name === './protocol.cjs'
        ? require('../../electron/notes/protocol.cjs') : require(name) });
    await new Promise(setImmediate);
    assert.equal(pending.length, 2);
    assert.equal(output.length, 0, 'diagnostics must not manufacture Ready');
    windows[0].webContents.emit('did-finish-load');
    windows[1].webContents.emit('preload-error', {}, '/private', new Error('private note contents'));
    pending[0].resolve('complete');
    await new Promise(setImmediate);
    assert.equal(output.length, 0, 'both original renderer probes remain required');
    assert.equal(errors.join('').includes('popup:probe-complete'), diagnostics);
    assert.ok(!errors.join('').includes('badge:probe-complete'), 'pending badge remains observable without acknowledging Ready');
    if (reject) pending[1].fail(new Error('private note contents'));
    else pending[1].resolve('complete');
    await new Promise(setImmediate);
    if (reject) { assert.equal(output.length, 0); assert.equal(quit, 1); }
    else assert.deepEqual(output, [{ action: 'windows', ids: [1, 2] }, { action: 'ready' }]);
    assert.ok(errors.every(line => /^Notes fixture: [a-z:-]+\n$/.test(line)));
    assert.ok(!errors.join('').includes('private'));
    return errors.join('');
  }
  assert.equal(await run(false, false), '');
  assert.match(await run(true, false), /ready-sent/);
  assert.match(await run(true, true), /ready-probe-failed/);
});
test('embedded flight target follows physical scale and the leading row rather than the pocket', () => {
 for (const scale of [1,1.25,2]) {
  const rect=[-500*scale,34*scale,484*scale,58*scale];
  assert.deepEqual(embeddedBadgeBounds(rect,scale),{x:-88*scale,y:34*scale,width:72*scale,height:58*scale});
 }
});
test('notes bridge accepts only bounded note actions, never renderer paths or window commands', () => {
  assert.equal(validCommand({ action: 'save', pin: 1, text: 'Change this\nnot that' }), true);
  for (const message of [{ action: 'save', pin: 1, text: 'x', path: '/tmp/private' },
    { action: 'windows', ids: [9] }, { action: 'surface', id: 9, rect: [0,0,10,10], focus: true }, { action: 'ready' }, { action: 'flight', active: true }, { action: 'open', pin: 1 },
    { action: 'save', pin: 1, text: 'é'.repeat(4097) }, { action: 'delete', pin: -1 }]) {
    assert.ok(!validCommand(message));
  }
});
test('notes snapshot retains an exact click time and validates draft and saved text', () => {
  const n = { pin: 1, t: 1234, x: .25, y: .5, text: 'Keep me' };
  const s = { mode: 'editor', current: 1, position: [-300, 42], notes: [], drafts: [n], error: null };
  assert.equal(validSnapshot(s), true);
  assert.equal(validSnapshot({ ...s, embedded: true }), true);
  assert.equal(validSnapshot({ ...s, embedded: 'true' }), false);
  assert.equal(validSnapshot({ ...s, drafts: [{ ...n, x: NaN }] }), false);
  assert.equal(validSnapshot({ ...s, notes: [{ ...n, text: '\0' }] }), false);
});
test('popup stays inside work areas including negative display coordinates and small screens', () => {
  assert.deepEqual(popupBounds({ x: -1, y: 999 }, { x: -1280, y: 0, width: 1280, height: 800 }, 220),
    { x: -332, y: 580, width: 332, height: 220 });
  assert.deepEqual(popupBounds({ x: 300, y: 300 }, { x: 0, y: 0, width: 240, height: 180 }, 350),
    { x: 0, y: 0, width: 240, height: 180 });
});

test('counter falls left or below instead of covering the capsule at a display edge', () => {
  const area = { x: -1280, y: 0, width: 1280, height: 800 };
  assert.deepEqual(badgeBounds([-1000, 40, 350, 40], area), { x: -634, y: 38 });
  assert.deepEqual(badgeBounds([-370, 40, 350, 40], area), { x: -502, y: 38 });
  assert.deepEqual(badgeBounds([-1270, 40, 1260, 40], area), { x: -698, y: 96 });
});

test('saved-note flight retains its source and destination with bounded geometry', () => {
  const from = { x: -300, y: 400, width: 332, height: 220 };
  const badge = { x: 600, y: 30, width: 116, height: 44 };
  const flight = flightBounds(from, badge);
  assert.ok(flight.bounds.width > 0 && flight.bounds.height > 0);
  assert.equal(flight.bounds.x + flight.start[0], from.x + from.width / 2 - 16);
  assert.equal(flight.bounds.y + flight.start[1], from.y + from.height / 2 - 16);
  assert.equal(flight.bounds.x + flight.start[0] + flight.delta[0], badge.x + badge.width / 2 - 16);
  assert.equal(flight.bounds.y + flight.start[1] + flight.delta[1], badge.y + badge.height / 2 - 16);
  assert.equal(flightBounds(from, { ...badge, x: 10000 }), null);
});
