import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { digest } from './artifacts.ts';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
async function sources(directory: string, section = 'src'): Promise<Record<string, string>> {
  const result: Record<string, string> = {};
  async function walk(path: string, prefix: string) {
    for (const entry of (await readdir(path, { withFileTypes: true })).sort((a, b) => a.name.localeCompare(b.name))) {
      if (entry.isDirectory()) await walk(join(path, entry.name), prefix + entry.name + '/');
      else {
        assert(entry.isFile(), 'Unexpected shared source entry');
        result[prefix + entry.name] = digest(await readFile(join(path, entry.name)));
      }
    }
  }
  await walk(join(directory, section), section + '/');
  result['package.json'] = digest(await readFile(join(directory, 'package.json')));
  return result;
}
/** npm file dependencies are copied. Never test a stale copy after shared edits. */
export async function checkInstalled() {
  for (const [name, source] of [['@apparent/app-contracts', 'contracts'], ['@apparent/ui', 'ui']]) {
    assert.deepEqual(await sources(join(root, 'node_modules', name)), await sources(join(root, '../../shared', source)),
      `${name} changed; run npm ci in shared/contracts, then composition/app before checking/building`);
  }
  assert.deepEqual(await readFile(join(root, 'node_modules/@apparent/app-contracts/tsconfig.build.json')),
    await readFile(join(root, '../../shared/contracts/tsconfig.build.json')), 'Contract build configuration changed; reinstall local dependencies');
  assert.deepEqual(await sources(join(root, 'node_modules/@apparent/app-contracts'), 'dist'),
    await sources(join(root, '../../shared/contracts'), 'dist'), 'Compiled contracts changed; reinstall local dependencies');
}
if (import.meta.url === pathToFileURL(process.argv[1] ?? '').href) await checkInstalled();
