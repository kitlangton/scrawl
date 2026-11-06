import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { test } from 'node:test';
import { readStream } from '../../scripts/artifacts.ts';
import { buildRecordArtifact } from '../../scripts/build-record.ts';

test('builds a sealed public Record pill stream with compiled fonts and shared React imports', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'record-pill-stream-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const output = join(directory, 'record');

  const built = await buildRecordArtifact(output);
  const manifest = await readStream(output);
  assert.deepEqual(built, manifest);
  assert.equal(manifest.id, 'record');
  assert.equal(manifest.entry, 'index.js');
  assert(manifest.assets?.['style.css']);
  const fonts = Object.keys(manifest.assets ?? {}).filter(name => /^fonts\/.*\.(?:ttf|woff2)$/u.test(name));
  assert(fonts.length > 0, 'The pinned Record UI package must supply its compiled font assets');
  assert(Object.keys(manifest.assets ?? {}).length <= 10);

  const javascript = await readFile(join(output, 'index.js'), 'utf8');
  assert.match(javascript, /from\s*["']react["']/u);
  assert.match(javascript, /from\s*["']react\/jsx-runtime["']/u);
  assert.doesNotMatch(javascript, /from\s*["'](?:node:|electron|@apparent\/native-host)/u);
  const css = await readFile(join(output, 'style.css'), 'utf8');
  assert.match(css, /@font-face/u);
  for (const font of fonts) assert.match(css, new RegExp(font.replace(/[.*+?^${}()|[\]\\]/gu, '\\$&'), 'u'));
  for (const font of fonts) assert((await readFile(join(output, font))).byteLength > 0);

  const original = await readFile(join(output, 'index.js'));
  await writeFile(join(output, 'index.js'), 'tampered');
  await assert.rejects(readStream(output), /checksum/u);
  await writeFile(join(output, 'index.js'), original);
  await readStream(output);
  await assert.rejects(buildRecordArtifact(output), /destination already exists/u);
  assert.deepEqual(await readFile(join(output, 'index.js')), original, 'a second build cannot overwrite an existing stream');
});
