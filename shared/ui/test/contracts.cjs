const test = require('node:test');
const assert = require('node:assert/strict');
const { EventEmitter } = require('node:events');
const follow = require('../src/window-theme.cjs');
test('native window follows shared light/dark backgrounds and releases its listener', () => {
  const native = Object.assign(new EventEmitter(), { shouldUseDarkColors: false });
  const colors = [];
  const window = Object.assign(new EventEmitter(), { setBackgroundColor: color => colors.push(color) });
  follow(native, window);
  native.shouldUseDarkColors = true; native.emit('updated');
  native.shouldUseDarkColors = false; native.emit('updated');
  assert.equal(colors.length, 3);
  assert.equal(colors[0], colors[2]); assert.notEqual(colors[0], colors[1]);
  window.emit('closed'); assert.equal(native.listenerCount('updated'), 0);
});
