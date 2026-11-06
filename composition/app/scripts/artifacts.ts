import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { lstat, readFile, readdir } from 'node:fs/promises';
import { join } from 'node:path';
import { APP_SHELL_API, REACT_VERSION, parseStreamManifest } from '@apparent/app-contracts';
import { verifyNativeModes, type NativeFileModes } from './native-modes.ts';

export const digest = (bytes: Uint8Array | string) => createHash('sha256').update(bytes).digest('hex');
export async function readRegular(path: string): Promise<Buffer> {
  const stat = await lstat(path);
  assert(stat.isFile() && !stat.isSymbolicLink(), 'Artifact file must be regular');
  return readFile(path);
}
export async function exactFiles(directory: string, expected: string[]) {
  const stat = await lstat(directory);
  assert(stat.isDirectory() && !stat.isSymbolicLink(), 'Artifact directory must be regular');
  const actual: string[] = [];
  async function walk(root: string, prefix = '') {
    for (const entry of await readdir(root, { withFileTypes: true })) {
      const relative = prefix + entry.name;
      assert(!entry.isSymbolicLink(), 'Artifact symlinks are not supported');
      if (entry.isDirectory()) await walk(join(root, entry.name), relative + '/');
      else { assert(entry.isFile(), 'Unsupported artifact entry'); actual.push(relative); }
    }
  }
  await walk(directory);
  assert.deepEqual(actual.sort(), [...expected].sort(), 'Unexpected or missing artifact files');
}
export async function readStream(directory: string) {
  const manifest = parseStreamManifest(JSON.parse((await readRegular(join(directory, 'manifest.json'))).toString()));
  await exactFiles(directory, ['manifest.json', 'index.js', ...Object.keys(manifest.assets ?? {})]);
  assert.equal(digest(await readRegular(join(directory, manifest.entry))), manifest.sha256, 'Stream checksum mismatch');
  for (const [name, hash] of Object.entries(manifest.assets ?? {})) {
    assert.equal(digest(await readRegular(join(directory, name))), hash, 'Stream asset checksum mismatch');
  }
  return manifest;
}
export const shellFiles = ['main.cjs', 'owner-binding.mjs', 'preload.cjs', 'renderer.js', 'index.html', 'style.css', 'package.json',
  'vendor/react.js', 'vendor/jsx.js', 'vendor/dom.js'];
export type ShellManifest = { schemaVersion: 1; shellApi: 1; reactVersion: string; files: Record<string, string>; nativeFiles?: NativeFileModes };
export async function readShell(directory: string): Promise<ShellManifest> {
  let hasConfigHost = false;
  try { hasConfigHost = (await lstat(join(directory, 'native/apparent-config-host'))).isFile(); }
  catch (error) { if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error; }
  const ownerAssets = ['assets/apparent-app-icon.svg', 'assets/frog.png'];
  const ownerAssetStates = await Promise.all(ownerAssets.map(async name => {
    try { await lstat(join(directory, name)); return true; }
    catch (error) { if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error; return false; }
  }));
  if (ownerAssetStates.some(Boolean) && !ownerAssetStates.every(Boolean)) throw new Error('Incomplete Onboarding assets');
  const hasOwnerAssets = ownerAssetStates.every(Boolean);
  assert.equal(hasOwnerAssets, hasConfigHost, 'Onboarding binding and its native helper/assets must be assembled together');
  const manifest = JSON.parse((await readRegular(join(directory, 'manifest.json'))).toString()) as ShellManifest;
  const hasNative = Object.hasOwn(manifest, 'nativeFiles');
  if (hasNative) await verifyNativeModes(directory, manifest.nativeFiles!);
  const expectedNames = [...shellFiles, ...Object.keys(manifest.nativeFiles ?? {}), ...(hasConfigHost ? ['native/apparent-config-host'] : []), ...(hasOwnerAssets ? ownerAssets : []), 'manifest.json'];
  await exactFiles(directory, expectedNames);
  assert.deepEqual(Object.keys(manifest).sort(), ['files', ...(hasNative ? ['nativeFiles'] : []), 'reactVersion', 'schemaVersion', 'shellApi']);
  assert.equal(manifest.schemaVersion, 1, 'Unsupported shell manifest');
  assert.equal(manifest.shellApi, APP_SHELL_API, 'Incompatible shell API');
  assert.equal(manifest.reactVersion, REACT_VERSION, 'Incompatible React runtime');
  assert.deepEqual(Object.keys(manifest.files).sort(), expectedNames.filter(name => name !== 'manifest.json').sort(), 'Invalid shell file inventory');
  for (const name of expectedNames.filter(name => name !== 'manifest.json')) {
    assert.equal(digest(await readRegular(join(directory, name))), manifest.files[name], 'Shell checksum mismatch');
  }
  return manifest;
}
