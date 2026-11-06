import assert from 'node:assert/strict';
import test from 'node:test';
import { APP_SHELL_API, parseStreamManifest, REACT_VERSION } from '../src/index.ts';

const validManifest = {
  schemaVersion: 1,
  id: 'owner-settings',
  version: '0.1.0',
  label: 'Owner Settings',
  shellApi: APP_SHELL_API,
  reactVersion: REACT_VERSION,
  entry: 'index.js',
  sha256: 'a'.repeat(64),
};

test('parses a valid stream manifest', () => {
  assert.deepEqual(parseStreamManifest(validManifest), validManifest);
});

test('parses a bounded stylesheet and font inventory without allowing executable or escaping assets', () => {
  const assets = { 'style.css': 'b'.repeat(64), 'fonts/HostGrotesk-Variable.ttf': 'c'.repeat(64) };
  assert.deepEqual(parseStreamManifest({ ...validManifest, assets }), { ...validManifest, assets });
  for (const invalid of [null, [], {}, { 'font.ttf': 'a'.repeat(64) },
    { '../style.css': 'a'.repeat(64) }, { 'style.css': 'x' }, { 'index.js': 'a'.repeat(64) },
    { 'style.css': 'a'.repeat(64), 'fonts/../escape.ttf': 'a'.repeat(64) },
    { 'style.css': 'a'.repeat(64), 'fonts/a%2fttf': 'a'.repeat(64) },
    { 'style.css': 'a'.repeat(64), ...Object.fromEntries(Array.from({ length: 10 }, (_, i) => [`fonts/f${i}.ttf`, 'a'.repeat(64)])) }]) {
    assert.throws(() => parseStreamManifest({ ...validManifest, assets: invalid }));
  }
});

test('rejects non-object values', () => {
  for (const value of [null, [], 'manifest', 1]) {
    assert.throws(() => parseStreamManifest(value));
  }
});

test('rejects missing and unknown fields', () => {
  const { label: _label, ...missing } = validManifest;
  assert.throws(() => parseStreamManifest(missing));
  assert.throws(() => parseStreamManifest({ ...validManifest, extra: true }));
});

test('rejects unsupported versions, ids, labels, entries and hashes', () => {
  const invalidCases: unknown[] = [
    { ...validManifest, schemaVersion: 2 },
    { ...validManifest, shellApi: 2 },
    { ...validManifest, reactVersion: '18.3.1' },
    { ...validManifest, id: 'Owner' },
    { ...validManifest, id: 'a'.repeat(65) },
    { ...validManifest, version: '1.2' },
    { ...validManifest, label: '  ' },
    { ...validManifest, label: 'x'.repeat(121) },
    { ...validManifest, entry: '../index.js' },
    { ...validManifest, sha256: 'A'.repeat(64) },
  ];
  for (const value of invalidCases) assert.throws(() => parseStreamManifest(value));
});
