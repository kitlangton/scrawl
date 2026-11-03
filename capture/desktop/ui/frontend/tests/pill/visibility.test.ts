import {test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import {createRequire} from 'node:module';
import ts from 'typescript';

const require = createRequire(import.meta.url);
const protocol = require('../../electron/pill/protocol.cjs');
const source = fs.readFileSync(new URL('../../electron/pill/main.cjs', import.meta.url), 'utf8');
const parsed = ts.createSourceFile('main.cjs', source, ts.ScriptTarget.Latest, true);
const functions = parsed.statements.filter(node => ts.isFunctionDeclaration(node) &&
  ['publish', 'visibility'].includes(node.name?.text ?? ''));
assert.equal(functions.length, 2);
const production = functions.map(node => source.slice(node.getStart(parsed), node.end)).join('\n');
const recording = {revision: 1, phase: 'recording', clock: '00:12', label: 'Recording',
  warning: null, reducedMotion: false, draggable: true, actions: []};

function host(visible: boolean, loaded: boolean, hasWindow = true) {
  const native = {visible, hides: 0, shows: 0, sent: [] as unknown[]};
  const scope = {
    protocol, current: protocol.view(recording), pending: true,
    pasteGesture: {revision: 1}, lastInput: 0,
    loaded, activated: true, rendererReady: false,
    window: hasWindow ? {
      isVisible: () => native.visible,
      isDestroyed: () => false,
      hide: () => {native.hides++; native.visible = false;},
      showInactive: () => {native.shows++; native.visible = true;},
      webContents: {send: (...message: unknown[]) => native.sent.push(message)},
    } : undefined,
    incoming: {...recording, revision: 2, phase: 'hidden'},
  };
  const context = vm.createContext(scope);
  vm.runInContext(production, context);
  return {native, scope, publish: () => vm.runInContext('publish(incoming)', context)};
}

test('dismissal hides native controls during renderer loading', () => {
  const {native, scope, publish} = host(true, false);
  publish();
  assert.equal(native.visible, false, 'A renderer reload must not delay authoritative dismissal');
  assert.equal(native.hides, 1);
  assert.equal(native.shows, 0);
  assert.equal(native.sent.length, 0, 'Loading renderer must not receive state before its load boundary');
  assert.equal(scope.current.phase, 'hidden');
  assert.equal(scope.pending, false);
  assert.equal(scope.pasteGesture, null);
});

test('hidden heartbeats do not repeat native hide requests', () => {
  const {native, publish} = host(false, true);
  publish(); publish(); publish();
  assert.equal(native.hides, 0);
  assert.equal(native.shows, 0);
  assert.equal(native.sent.length, 3);
});

test('state can arrive before the native window exists', () => {
  const {scope, publish} = host(false, false, false);
  publish();
  assert.equal(scope.current.phase, 'hidden');
});
