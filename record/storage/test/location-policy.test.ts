import { test } from 'node:test';
import assert from 'node:assert/strict';
import { locationCandidate, normalizeSelectedPath, resolveLocationCandidate } from '../dist/index.js';

const facts = (overrides: Record<string, unknown> = {}) => ({
  saved: { kind: 'string', value: '~/Saved captures' },
  environment: { present: false, value: null },
  homePath: '/home/example',
  builtinPath: '/home/example/.local/share/apparent/captures',
  revision: 'a'.repeat(16),
  ...overrides,
});

test('TypeScript selects tilde-expanded custom paths and native availability controls fallback', () => {
  const input = facts();
  const candidate = locationCandidate(input as never);
  assert.equal(candidate.probePath, '/home/example/Saved captures');
  assert.deepEqual(resolveLocationCandidate(input as never, true), {
    savedPath: '~/Saved captures', effectivePath: '/home/example/Saved captures',
    displayPath: '~/Saved captures', custom: true, source: 'configured',
  });
  assert.deepEqual(resolveLocationCandidate(input as never, false), {
    savedPath: '~/Saved captures', effectivePath: '/home/example/.local/share/apparent/captures',
    displayPath: '~/.local/share/apparent/captures', custom: true, source: 'fallback',
  });
});

test('environment presence wins even when the override is empty or relative', () => {
  for (const value of ['', 'relative/path']) {
    const input = facts({ environment: { present: true, value } });
    const candidate = locationCandidate(input as never);
    assert.equal(candidate.probePath, undefined);
    assert.equal(candidate.snapshot?.source, 'environment');
    assert.equal(candidate.snapshot?.effectivePath, value);
    assert.equal(candidate.snapshot?.savedPath, '~/Saved captures');
  }
});

test('tilde expansion preserves native path spelling and dot segments', () => {
  const input = facts({ saved: { kind: 'string', value: '~/../other' } });
  const candidate = locationCandidate(input as never);
  assert.equal(candidate.probePath, '/home/example/../other');
  assert.equal(resolveLocationCandidate(input as never, true).displayPath, '~/../other');
});

test('display formatting handles root and trailing-slash home paths', () => {
  const rootHome = facts({ homePath: '/', builtinPath: '/captures',
    environment: { present: true, value: '/tmp/captures' } });
  assert.equal(locationCandidate(rootHome as never).snapshot?.displayPath, '~/tmp/captures');
  const trailingHome = facts({ homePath: '/home/example/', builtinPath: '/home/example/captures',
    environment: { present: true, value: '/home/example/captures' } });
  assert.equal(locationCandidate(trailingHome as never).snapshot?.displayPath, '~/captures');
});

test('invalid, absent, null and non-string saved preferences keep native-compatible defaults', () => {
  for (const saved of [
    { kind: 'absent' }, { kind: 'null' }, { kind: 'other' },
    { kind: 'string', value: 'relative' }, { kind: 'string', value: '  ' },
  ]) {
    const candidate = locationCandidate(facts({ saved }) as never);
    assert.equal(candidate.snapshot?.source, 'builtin');
    assert.equal(candidate.snapshot?.custom, false);
  }
});

test('path values enforce UTF-8 byte bounds, NUL exclusion, trimming and absoluteness', () => {
  assert.equal(normalizeSelectedPath('/tmp/ok '), '/tmp/ok');
  assert.equal(normalizeSelectedPath('relative'), undefined);
  assert.equal(normalizeSelectedPath('/tmp/\0bad'), undefined);
  assert.equal(normalizeSelectedPath('/' + 'é'.repeat(2048)), undefined);
  assert.throws(() => locationCandidate(facts({ environment: { present: true, value: '\0' } }) as never));
  const oversizedSaved = locationCandidate(facts({ saved: { kind: 'string', value: 'x'.repeat(4097) } }) as never);
  assert.equal(oversizedSaved.snapshot?.source, 'builtin');
  assert.equal(oversizedSaved.snapshot?.custom, false);
});
