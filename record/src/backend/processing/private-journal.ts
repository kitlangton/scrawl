import { constants } from "node:fs";
import { randomUUID } from "node:crypto";
import { link, lstat, open, realpath, rename, rm, stat, unlink } from "node:fs/promises";
import { dirname, isAbsolute, resolve } from "node:path";

export function assertPrivateJournalPath(path: string, label: string): void {
  if (!isAbsolute(path) || resolve(path) !== path || path.includes("\0"))
    throw new TypeError(`${label} path must be absolute and normalized`);
}

function privateFile(info: Awaited<ReturnType<typeof lstat>>, label: string): void {
  if (!info.isFile() || info.isSymbolicLink() || info.nlink !== 1 || info.uid !== process.getuid?.() || (Number(info.mode) & 0o077) !== 0)
    throw new Error(`${label} must be a private owned file`);
}

export async function writePrivateJournal(path: string, value: unknown, label: string, create = false): Promise<void> {
  const parent = dirname(path);
  const dir = await stat(parent);
  if (!dir.isDirectory() || (dir.mode & 0o077) !== 0 || dir.uid !== process.getuid?.() || await realpath(parent) !== parent)
    throw new Error(`${label} directory must be private and owned`);
  if (!create) privateFile(await lstat(path), label);
  const tmp = `${path}.tmp-${randomUUID()}`;
  const handle = await open(tmp, constants.O_CREAT | constants.O_EXCL | constants.O_WRONLY | (constants.O_NOFOLLOW ?? 0), 0o600);
  try { await handle.writeFile(`${JSON.stringify(value)}\n`); await handle.sync(); } finally { await handle.close(); }
  try { if (create) await link(tmp, path); else await rename(tmp, path); }
  finally { await unlink(tmp).catch(() => {}); }
  const directory = await open(parent, constants.O_RDONLY | (constants.O_DIRECTORY ?? 0) | (constants.O_NOFOLLOW ?? 0));
  try { await directory.sync(); } finally { await directory.close(); }
}

export async function readPrivateJournal(path: string, label: string): Promise<unknown> {
  const info = await lstat(path);
  privateFile(info, label);
  if (await realpath(path) !== path) throw new Error(`${label} must be a private owned file`);
  const handle = await open(path, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  try { return JSON.parse(await handle.readFile("utf8")) as unknown; }
  finally { await handle.close(); }
}

export async function removePrivateJournal(path: string, label: string): Promise<void> {
  privateFile(await lstat(path), label);
  await rm(path, { force: false });
}
