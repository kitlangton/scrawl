import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mkdir, mkdtemp, readFile, rm, symlink, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join, relative, resolve } from 'node:path';
import { createRequire } from 'node:module';
import { build } from 'esbuild';
import { APP_SHELL_API, REACT_VERSION, parseStreamManifest } from '@apparent/app-contracts';
import { readStream, digest } from '../../scripts/artifacts.ts';
import { buildShell } from '../../scripts/build.ts';
import { assemble, verifyAssembly } from '../../scripts/assemble.ts';

test('real packed Record stylesheet/fonts survive sealed compiler-free assembly and reject tampering', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'record-stream-assets-'));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const stream = join(directory, 'record'); await mkdir(stream);
  const require = createRequire(import.meta.url);
  const css = require.resolve('@apparent/record/ui/style.css');
  // Consume precompiled CSS from the qualified package; esbuild only relocates
  // its font URL into the sealed stream inventory. No Record source build.
  const compiled = await build({ entryPoints: [css], outdir: stream, entryNames: 'style', assetNames: 'fonts/[name]-[hash]',
    bundle: true, loader: { '.ttf': 'file', '.woff2': 'file' }, metafile: true });
  const assets: Record<string, string> = {};
  for (const name of Object.keys(compiled.metafile!.outputs)) {
    const asset = relative(stream, resolve(name));
    assets[asset] = digest(await readFile(name));
  }
  assert(assets['style.css']); assert(Object.keys(assets).some(name => name.endsWith('.ttf')));
  await writeFile(join(stream, 'index.js'), 'export default {id:"record",Screen:()=>null,Settings:()=>null};');
  const manifest = parseStreamManifest({ schemaVersion: 1, id: 'record', version: '0.1.0', label: 'Record',
    shellApi: APP_SHELL_API, reactVersion: REACT_VERSION, entry: 'index.js', sha256: digest(await readFile(join(stream, 'index.js'))), assets });
  await writeFile(join(stream, 'manifest.json'), JSON.stringify(manifest));
  await readStream(stream);
  const shell = join(directory, 'shell'); await buildShell(shell);
  const output = await assemble(shell, [stream], join(directory, 'app')); await verifyAssembly(output);
  for (const name of Object.keys(assets)) assert.deepEqual(await readFile(join(output, 'streams/record', name)), await readFile(join(stream, name)));
  const font = Object.keys(assets).find(name => name.endsWith('.ttf'))!;
  const original = await readFile(join(stream, font));
  await writeFile(join(stream, font), 'tampered-font'); await assert.rejects(readStream(stream), /checksum/);
  await writeFile(join(stream, font), original);
  await writeFile(join(stream, 'extra.css'), 'unexpected'); await assert.rejects(readStream(stream), /Unexpected/);
  await rm(join(stream, 'extra.css')); await rm(join(stream, font));
  const outside = join(directory, 'outside.ttf'); await writeFile(outside, original); await symlink(outside, join(stream, font));
  await assert.rejects(readStream(stream), /symlink/);
  await writeFile(join(output, 'streams/record/style.css'), 'tampered-style'); await assert.rejects(verifyAssembly(output), /checksum/);
});
