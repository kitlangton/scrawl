import {createHash} from 'node:crypto';
import {execFileSync} from 'node:child_process';
import {lstat, readFile, readlink} from 'node:fs/promises';
import path from 'node:path';

export const contract = 'storage-provider-linux-v1';
export const testName = 'capture::desktop::workflow::agent_capture::tests::storage_electron_rust_roundtrip';
export const digest = (bytes: Buffer | string) => createHash('sha256').update(bytes).digest('hex');
const frontend = 'capture/desktop/ui/frontend/';

// This is deliberately conservative: unrelated tracked inputs invalidate too.
// Only the audited TS consumer surface is outside the native compatibility set.
// Native source anywhere, including a newly added owner directory, stays inside.
export function consumerInput(file: string): boolean {
  if (file.startsWith('record/storage/')) {
    const local = file.slice('record/storage/'.length);
    return /^(?:src\/.*\.ts|test\/.*\.test\.ts|build\.ts|package(?:-lock)?\.json|tsconfig\.json|README\.md|\.gitignore)$/.test(local);
  }
  if (!file.startsWith(frontend)) return false;
  const local = file.slice(frontend.length);
  return /^(?:src\/.*\.(?:ts|tsx|css)|electron\/.*\.(?:ts|cts|cjs)|tests\/.*\.(?:ts|cts|cjs)|scripts\/.*\.(?:ts|mjs)|scripts\/settings-native\/tsconfig\.json|package(?:-lock)?\.json|tsconfig\.json|vite\.config\.ts|README\.md|[^/]+\.html|\.gitignore)$/.test(local);
}

export async function nativeInputs(root: string): Promise<{hash: string; count: number}> {
  const tracked = execFileSync('git', ['ls-files', '-z'], {cwd: root, encoding: 'utf8'}).split('\0').filter(Boolean);
  const untracked = execFileSync('git', ['ls-files', '--others', '--exclude-standard', '-z'],
    {cwd: root, encoding: 'utf8'}).split('\0').filter(Boolean);
  // Do not read unknown/private untracked data to manufacture a compatibility hash.
  if (untracked.some(file => !consumerInput(file))) throw Error('Unadmitted native inputs; commit or isolate them before preparing/checking the provider');
  const entries: Array<[string, string]> = [];
  for (const file of tracked.filter(file => !consumerInput(file)).sort()) {
    const absolute = path.join(root, file);
    const info = await lstat(absolute);
    if (!info.isFile() && !info.isSymbolicLink()) throw Error(`Unsupported native input: ${file}`);
    entries.push([file, digest((info.isSymbolicLink() ? await readlink(absolute) + '\0' : '') + digest(await readFile(absolute)))]);
  }
  return {hash: digest(JSON.stringify(entries)), count: entries.length};
}
