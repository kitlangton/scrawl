import { execFileSync } from 'node:child_process';
import { closeSync, constants, fstatSync, lstatSync, mkdirSync, openSync, unlinkSync, writeFileSync } from 'node:fs';
import path from 'node:path';
import { tokenValid } from './protocol.ts';

export function keyDirectory() {
  if (process.platform === 'darwin') return execFileSync('/usr/bin/getconf', ['DARWIN_USER_TEMP_DIR'], { encoding: 'utf8', timeout: 3000 }).trim();
  if (process.platform === 'linux' && process.getuid) return process.env.XDG_RUNTIME_DIR || `/run/user/${process.getuid()}`;
  throw new Error('Unsupported recording bridge platform');
}
function privateDirectory(directory: string) {
  const stat = lstatSync(directory);
  if (!path.isAbsolute(directory) || !stat.isDirectory() || stat.uid !== process.getuid?.() || (stat.mode & 0o777) !== 0o700) throw new Error('Recording bridge requires a private user runtime directory');
}
// The URI contains a public request ID. Only the same OS user may read this key;
// it never enters process arguments, HTTP requests or plugin persistence.
export function publishKey(root: string, token: string, key: string) {
  if (!tokenValid(token) || !tokenValid(key) || token === key) throw new Error('Invalid recording bridge key');
  privateDirectory(root);
  const directory = path.join(root, 'apparent-hermes');
  try { mkdirSync(directory, { mode: 0o700 }); }
  catch (error) { if ((error as NodeJS.ErrnoException).code !== 'EEXIST') throw error; }
  privateDirectory(directory);
  const file = path.join(directory, `${token}.key`);
  const descriptor = openSync(file, constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW, 0o600);
  const owned = fstatSync(descriptor);
  let closed = false;
  const remove = () => {
    if (closed) return;
    try {
      privateDirectory(directory);
      const current = lstatSync(file);
      if (current.isFile() && current.dev === owned.dev && current.ino === owned.ino && current.uid === owned.uid) unlinkSync(file);
    } catch { /* Never remove an unowned replacement. */ }
    finally { closed = true; closeSync(descriptor); }
  };
  try { writeFileSync(descriptor, key); }
  catch (error) { remove(); throw error; }
  return remove;
}
