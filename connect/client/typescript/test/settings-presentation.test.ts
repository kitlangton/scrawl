import assert from 'node:assert/strict';
import { test } from 'node:test';
import { presentConnect } from '../src/settings-presentation.ts';
import type { PresentationState } from '../src/settings-presentation.ts';

const state = (patch: Partial<PresentationState> = {}): PresentationState => ({
  snapshot: { connected: false, uploadsEnabled: false, renewalRequired: false,
    reauthorizationRequired: false, accountInitial: null },
  health: null, error: null, localError: null, consent: false, approval: false,
  operation: null, logout: false, retryDisable: null, confirmation: null,
  closed: false, sharedLinksUrl: null, ...patch,
});
const snapshot = { connected: true, uploadsEnabled: true, renewalRequired: false,
  reauthorizationRequired: false, accountInitial: 'M' };
const controls = [
  { command: 'check' as const, title: 'Check connection', enabled: true },
  { command: 'disable' as const, title: 'Disable uploads', enabled: true },
  { command: 'disconnect' as const, title: 'Disconnect', enabled: true },
];

test('consent and approval distinguish upload permission from sign-in progress', () => {
  const consent = presentConnect(state({ consent: true }), []);
  assert.equal(consent.headline, 'Allow new capture uploads?');
  assert.match(consent.note, /Older captures stay local/);
  assert.match(consent.note, /Sharing is a separate action/);
  const waiting = presentConnect(state({ operation: 'login', approval: true }), []);
  assert.equal(waiting.tone, 'progress');
  assert.match(waiting.note, /Uploads haven’t started/);
});

test('connection, renewal and offline facts retain consent while offering distinct recovery', () => {
  const connected = presentConnect(state({ snapshot }), controls);
  assert.equal(connected.headline, 'You’re connected.');
  assert.equal(connected.accountInitial, 'M');
  assert.deepEqual(connected.menuActions, ['check', 'request-disable', 'request-disconnect']);
  assert.equal(controls[1]!.command, 'disable', 'presentation must not mutate backend controls');
  const renewal = presentConnect(state({ snapshot: { ...snapshot, renewalRequired: true } }), controls);
  assert.equal(renewal.tone, 'warning');
  assert.equal(renewal.menuActions.includes('check'), false, 'renewal action stays visible');
  const offline = presentConnect(state({ snapshot, health: 'offline' }), controls);
  assert.equal(offline.tone, 'failure');
  assert.match(offline.detail, /upload permission has not changed/);
});

test('disable confirmation, durable retry and closed controls remain explicit', () => {
  const confirm = presentConnect(state({ snapshot, confirmation: true }), controls);
  assert.equal(confirm.headline, 'Disconnect this computer?');
  assert.match(confirm.detail, /not deleted/);
  assert.deepEqual(confirm.buttons.map(b => b.command), ['disconnect', 'cancel']);
  assert.deepEqual(confirm.menuActions, []);
  const retry = presentConnect(state({ retryDisable: false, localError: 'local-state' }), controls);
  assert.equal(retry.buttons[1]!.command, 'disable');
  assert.equal(retry.buttons[1]!.title, 'Retry disabling uploads');
  assert.equal(retry.headline, 'Connection settings unavailable.');
  const closed = presentConnect(state({ snapshot, closed: true,
    sharedLinksUrl: 'https://connect.test/connect/shares' }), controls);
  assert.ok(closed.buttons.every(b => !b.enabled));
  assert.equal(closed.buttons.at(-1)!.command, 'open-shared-links');
});

test('local state failure overrides in-flight success and cancellation stays neutral', () => {
  const failed = presentConnect(state({ snapshot, operation: 'login', localError: 'local-state' }), []);
  assert.equal(failed.tone, 'failure');
  assert.equal(failed.compact, false);
  assert.equal(failed.artwork, false);
  const cancelled = presentConnect(state({ error: 'cancelled' }), []);
  assert.equal(cancelled.tone, 'neutral');
  assert.match(cancelled.detail, /Check the current local upload settings/);
});
