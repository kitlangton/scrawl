import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { dirname, resolve } from 'node:path';
import test from 'node:test';
import {
  checkDependencyFreshness,
  RECORD_FRESHNESS_FILES,
  SHARED_UI_FRESHNESS_FILES,
} from './check-dependency-freshness.ts';

function fixture(): { root: string; frontend: string; cleanup: () => void } {
  const root = mkdtempSync(resolve(tmpdir(), 'apparent-dependency-freshness-'));
  const frontend = resolve(root, 'capture/desktop/ui/frontend');
  const writePair = (source: string, installed: string, content: string) => {
    for (const path of [resolve(root, source), resolve(frontend, installed)]) {
      mkdirSync(dirname(path), { recursive: true });
      writeFileSync(path, content);
    }
  };
  for (const file of SHARED_UI_FRESHNESS_FILES)
    writePair(`shared/ui/src/${file}`, `node_modules/@apparent/ui/src/${file}`, `shared:${file}`);
  for (const file of RECORD_FRESHNESS_FILES)
    writePair(`record/${file}`, `node_modules/@apparent/record/${file}`, `record:${file}`);
  return { root, frontend, cleanup: () => rmSync(root, { recursive: true, force: true }) };
}

test('frontend dependency freshness accepts a prepared copied-file consumer', () => {
  const state = fixture();
  try {
    assert.doesNotThrow(() => checkDependencyFreshness({ frontend: state.frontend, repository: state.root }));
  } finally {
    state.cleanup();
  }
});

test('frontend dependency freshness rejects a stale copied Record consumer file', () => {
  const state = fixture();
  try {
    writeFileSync(resolve(state.frontend, 'node_modules/@apparent/record/dist/ui/recording-pill.js'), 'stale');
    assert.throws(
      () => checkDependencyFreshness({ frontend: state.frontend, repository: state.root }),
      /Record package changed/,
    );
  } finally {
    state.cleanup();
  }
});
