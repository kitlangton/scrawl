import assert from 'node:assert/strict';
import { cp, mkdir, mkdtemp, rename, rm, rmdir, writeFile } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { digest, exactFiles, readRegular, readShell, readStream } from './artifacts.ts';
import { verifyNativeModes, type NativeFileModes } from './native-modes.ts';

/** No compiler or owner source access: only consume already-built, validated artifacts. */
export async function assemble(shell: string, streams: string[], destination: string) {
  const shellManifest = await readShell(shell);
  const manifests = await Promise.all(streams.map(readStream));
  const ids = manifests.map(manifest => manifest.id);
  assert.equal(new Set(ids).size, ids.length, 'Duplicate stream identity');
  const output = resolve(destination);
  // Fresh output only. Never overwrite installed apps or a previous assembly.
  await mkdir(dirname(output), { recursive: true });
  const work = await mkdtemp(join(dirname(output), '.app-shell-'));
  const stage = join(work, 'app');
  try {
    await cp(shell, stage, { recursive: true, errorOnExist: true, force: false });
    await mkdir(join(stage, 'streams'));
    for (let index = 0; index < streams.length; index++) {
      await cp(streams[index], join(stage, 'streams', manifests[index].id), { recursive: true,
        errorOnExist: true, force: false });
      // Revalidate copied bytes, not just the source observed before copying.
      assert.deepEqual(await readStream(join(stage, 'streams', manifests[index].id)), manifests[index]);
    }
    for (const [name, hash] of Object.entries(shellManifest.files)) {
      assert.equal(digest(await readRegular(join(stage, name))), hash, 'Shell changed during assembly');
    }
    if (shellManifest.nativeFiles) await verifyNativeModes(stage, shellManifest.nativeFiles);
    await rm(join(stage, 'manifest.json'));
    await writeFile(join(stage, 'registry.js'), 'export const streams = ' + JSON.stringify(manifests) + ';\n');
    const files: Record<string, string> = {};
    const inventory = [...Object.keys(shellManifest.files), 'registry.js', ...manifests.flatMap(manifest =>
      ['manifest.json', 'index.js', ...Object.keys(manifest.assets ?? {})].map(file => `streams/${manifest.id}/${file}`))];
    for (const file of inventory) files[file] = digest(await readRegular(join(stage, file)));
    await writeFile(join(stage, 'assembly.json'), JSON.stringify({ schemaVersion: 1,
      shell: digest(JSON.stringify(shellManifest)), streams: manifests, files,
      ...(shellManifest.nativeFiles ? { nativeFiles: shellManifest.nativeFiles } : {}) }, null, 2) + '\n');
    await verifyAssembly(stage);
    // Reserve a fresh destination exclusively, then replace our own empty reservation.
    // Existing applications/assemblies are never overwritten.
    await mkdir(output);
    try { await rename(stage, output); }
    catch (error) { await rmdir(output); throw error; }
    return output;
  } finally { await rm(work, { recursive: true, force: true }); }
}
export async function verifyAssembly(directory: string) {
  const receipt = JSON.parse((await readRegular(join(directory, 'assembly.json'))).toString()) as {
    schemaVersion: number; files: Record<string, string>; nativeFiles?: NativeFileModes;
  };
  assert.equal(receipt.schemaVersion, 1);
  for (const name of Object.keys(receipt.files)) {
    assert(!name.includes('..') && !name.startsWith('/') && !name.includes('\\'), 'Invalid assembly path');
  }
  await exactFiles(directory, [...Object.keys(receipt.files), 'assembly.json']);
  if (Object.hasOwn(receipt, 'nativeFiles')) {
    await verifyNativeModes(directory, receipt.nativeFiles!);
    assert.deepEqual(Object.keys(receipt.nativeFiles!).sort(), Object.keys(receipt.files).filter(name => name.startsWith('native-record/')).sort(), 'Incomplete native mode inventory');
  } else assert(!Object.keys(receipt.files).some(name => name.startsWith('native-record/')), 'Missing native mode inventory');
  for (const [name, hash] of Object.entries(receipt.files)) {
    assert.equal(digest(await readRegular(join(directory, name))), hash, 'Assembly checksum mismatch');
  }
}
if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) {
  const [shell, destination, ...streams] = process.argv.slice(2);
  assert(shell && destination, 'Usage: assemble.ts <shell-artifact> <new-output> [stream-artifact ...]');
  await assemble(resolve(shell), streams.map(path => resolve(path)), resolve(destination));
}
