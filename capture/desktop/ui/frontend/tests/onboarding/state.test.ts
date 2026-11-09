import {test} from 'node:test';
import assert from 'node:assert/strict';
import {subscribeOnboardingState} from '../../src/onboarding/state.ts';

function fixture() {
  let resolve!: (value: string | null) => void, reject!: (error: Error) => void;
  const pending = new Promise<string | null>((done, failed) => {resolve = done; reject = failed});
  let emit!: (state: string) => void, stopped = 0;
  const received: (string | null)[] = [];
  const stop = subscribeOnboardingState({readState: () => pending, onState: listener => {emit = listener; return () => {stopped++}}}, state => received.push(state));
  return {resolve, reject, emit, received, stop, stopped: () => stopped};
}

test('a delayed empty or older startup snapshot cannot replace a newer screen', async () => {
  for (const snapshot of [null, 'welcome']) {
    const f = fixture();
    f.emit('shortcut');
    f.resolve(snapshot);
    await Promise.resolve();
    assert.deepEqual(f.received, ['shortcut']);
    f.emit('go-again');
    assert.deepEqual(f.received, ['shortcut', 'go-again']);
    f.stop();
  }
});

test('the startup snapshot works when there is no newer live screen', async () => {
  const f = fixture();
  f.resolve('shortcut');
  await Promise.resolve();
  assert.deepEqual(f.received, ['shortcut']);
  f.stop();
});

test('a closed renderer subscription ignores late snapshot and queued events', async () => {
  const f = fixture();
  f.stop();
  f.emit('shortcut');
  f.resolve('welcome');
  await Promise.resolve();
  assert.deepEqual(f.received, []);
  assert.equal(f.stopped(), 1);
});

test('a failed startup read leaves subsequent live state usable', async () => {
  const f = fixture();
  f.reject(Error('synthetic disconnected read'));
  await Promise.resolve();
  f.emit('shortcut');
  assert.deepEqual(f.received, ['shortcut']);
  f.stop();
});
