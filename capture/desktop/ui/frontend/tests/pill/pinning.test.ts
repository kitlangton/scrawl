import {test} from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import ts from 'typescript';

const source = fs.readFileSync(new URL('../../electron/pill/main.cjs', import.meta.url), 'utf8');
const parsed = ts.createSourceFile('main.cjs', source, ts.ScriptTarget.Latest, true);
let registration: string | undefined;
function find(node: ts.Node) {
  if (ts.isForOfStatement(node) && ts.isArrayLiteralExpression(node.expression) &&
      node.expression.elements.map(element => ts.isStringLiteral(element) ? element.text : '').join(',') === 'show,move,resize') {
    assert.equal(registration, undefined, 'Native pin listeners must have one owner');
    registration = source.slice(node.getStart(parsed), node.end);
  }
  ts.forEachChild(node, find);
}
find(parsed);
assert.ok(registration, 'Native pin listener registration is missing');

// Importing main launches Electron. Execute its actual listener registration
// with only the native visibility/event boundary replaced by inert objects.
function nativeEvents(activated: boolean, visible: boolean, rendererReady = true, phase = 'recording') {
  const callbacks = new Map<string, () => void>();
  const emitted: unknown[] = [];
  vm.runInNewContext(registration!, {
    activated,
    rendererReady,
    current: {phase, label: 'Private capture context'},
    window: {isVisible: () => visible, on: (event: string, callback: () => void) => callbacks.set(event, callback)},
    emit: (message: unknown) => emitted.push(message),
  });
  for (const event of ['show', 'move', 'resize']) {
    assert.ok(callbacks.has(event));
    callbacks.get(event)!();
  }
  return JSON.parse(JSON.stringify(emitted));
}

test('native pin requests wait for activation and visibility', () => {
  assert.deepEqual(nativeEvents(false, false), [], 'Hidden startup placement must not spend native pin attempts');
  assert.deepEqual(nativeEvents(false, true), [], 'Parent activation must precede native admission');
  assert.deepEqual(nativeEvents(true, false), [], 'Placement between captures must not pin a hidden surface');
  assert.deepEqual(nativeEvents(true, true, false), [], 'Uncommitted renderer content must not start native admission');
  assert.deepEqual(nativeEvents(true, true, true, 'hidden'), [], 'Logical dismissal must suppress native admission');
  assert.equal(nativeEvents(true, true).length, 3, 'Shown controls must retain show/move/resize pinning');
});

test('native pin diagnostics contain only bounded presentation facts', () => {
  assert.deepEqual(nativeEvents(true, true), Array.from({length: 3}, () => ({
    pin: true, phase: 'recording', activated: true, rendererReady: true, visible: true,
  })));
});
