import assert from 'node:assert/strict';
import { EventEmitter } from 'node:events';
import test from 'node:test';
import type { BrowserWindow } from 'electron';
import { bindWindowShutdown } from '../src/host-lifecycle.ts';

class Window extends EventEmitter {
  destroyed = false;
  close() {
    let blocked = false;
    this.emit('close', { preventDefault() { blocked = true; } });
    if (!blocked) { this.destroyed = true; this.emit('closed'); }
  }
  isDestroyed() { return this.destroyed; }
}
const asWindow = (window: Window) => window as unknown as BrowserWindow;

test('active or unknown native ownership keeps the window and controls available', async () => {
  const window = new Window(); let closes = 0, revocations = 0;
  const reasons: string[] = [];
  const lifecycle = bindWindowShutdown({ window: asWindow(window), canClose: async () => false,
    close: async () => { closes++; }, revoke() { revocations++; }, refused(reason) { reasons.push(reason); } });
  window.close(); await lifecycle.settled();
  assert.equal(window.destroyed, false); assert.equal(closes, 0); assert.equal(revocations, 0);
  assert.deepEqual(reasons, ['ownership-unresolved']);
});

test('repeated close requests wait for one authoritative check and provider shutdown', async () => {
  const window = new Window(); let release!: (value: boolean) => void; let checks = 0;
  const pending = new Promise<boolean>(resolve => { release = resolve; });
  const order: string[] = [];
  const lifecycle = bindWindowShutdown({ window: asWindow(window), canClose: () => { checks++; return pending; },
    close: async () => { order.push('close'); }, revoke() { order.push('revoke'); }, refused() { assert.fail(); } });
  window.close(); window.close();
  assert.equal(window.destroyed, false); assert.equal(checks, 1);
  release(true); await lifecycle.settled();
  assert.equal(window.destroyed, true); assert.deepEqual(order, ['revoke', 'close']);
});

test('failed cleanup never claims closure; cleanup can be retried without new admission', async () => {
  const window = new Window(); let attempts = 0, revoked = 0; const reasons: string[] = [];
  const lifecycle = bindWindowShutdown({ window: asWindow(window), canClose: async () => true,
    close: async () => { if (++attempts === 1) throw new Error('close failed'); },
    revoke() { revoked++; }, refused(reason) { reasons.push(reason); } });
  window.close(); await lifecycle.settled();
  assert.equal(window.destroyed, false); assert.deepEqual(reasons, ['shutdown-failed']);
  window.close(); await lifecycle.settled();
  assert.equal(window.destroyed, true); assert.equal(attempts, 2); assert.equal(revoked, 1);
});

test('unexpected destruction does not authorize provider shutdown with unresolved ownership', async () => {
  const window = new Window(); let closes = 0;
  const lifecycle = bindWindowShutdown({ window: asWindow(window), canClose: async () => false,
    close: async () => { closes++; }, revoke() {}, refused() {} });
  window.destroyed = true; window.emit('closed');
  await assert.rejects(lifecycle.settled(), /ownership remains unresolved/);
  assert.equal(closes, 0);
});
