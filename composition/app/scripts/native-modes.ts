import assert from 'node:assert/strict';
import { lstat } from 'node:fs/promises';
import { join } from 'node:path';

export type NativeFileModes = Record<string, 0o644 | 0o755>;
export const safeNativePath = (value: unknown): value is string => typeof value === 'string'
  && value.length <= 240 && /^[A-Za-z0-9_.+/-]+$/.test(value)
  && value.split('/').every(part => part !== '.' && part !== '..' && part.length > 0);

export async function verifyNativeModes(directory: string, modes: NativeFileModes) {
  assert(modes && typeof modes === 'object' && !Array.isArray(modes), 'Invalid native mode inventory');
  assert(Object.keys(modes).length > 0 && Object.keys(modes).length <= 3075, 'Invalid native mode inventory');
  for (const [path, mode] of Object.entries(modes)) {
    assert(safeNativePath(path) && /^native-record\/(capture|media|config)\//.test(path)
      && (mode === 0o644 || mode === 0o755), 'Invalid native mode entry');
    const file = await lstat(join(directory, path));
    assert(file.isFile() && !file.isSymbolicLink() && (file.mode & 0o7777) === mode, 'Native runtime mode mismatch');
  }
}
