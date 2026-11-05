import { createHash, type Hash } from 'node:crypto';
import { constants, type BigIntStats } from 'node:fs';
import { lstat, mkdir, open, opendir, realpath } from 'node:fs/promises';
import { dirname, isAbsolute, join, relative, sep } from 'node:path';
import type { InventoryLimits, RuntimeFile } from './types.js';

export type FileIdentity = { dev: bigint; ino: bigint; size: bigint; mtimeNs: bigint; ctimeNs: bigint };
export type InventoryFile = RuntimeFile & { sourceIdentity: FileIdentity };
const READ_FLAGS = constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK;
const WRITE_FLAGS = constants.O_WRONLY | constants.O_CREAT | constants.O_EXCL | constants.O_NOFOLLOW;

function fail(message: string): never { throw new Error(message); }

function identity(stat: BigIntStats): FileIdentity {
  return { dev: stat.dev, ino: stat.ino, size: stat.size, mtimeNs: stat.mtimeNs, ctimeNs: stat.ctimeNs };
}

export function sameIdentity(left: FileIdentity, right: FileIdentity): boolean {
  return left.dev === right.dev && left.ino === right.ino && left.size === right.size &&
    left.mtimeNs === right.mtimeNs && left.ctimeNs === right.ctimeNs;
}

async function openStableRegular(path: string) {
  const before = await lstat(path, { bigint: true });
  if (!before.isFile() || before.isSymbolicLink()) fail(`expected a regular no-follow file: ${path}`);
  const handle = await open(path, READ_FLAGS);
  try {
    const opened = await handle.stat({ bigint: true });
    if (!opened.isFile() || !sameIdentity(identity(before), identity(opened))) fail(`file changed while opening: ${path}`);
    return { handle, initial: identity(opened) };
  } catch (error) {
    await handle.close();
    throw error;
  }
}

async function verifyStable(handle: Awaited<ReturnType<typeof open>>, initial: FileIdentity, path: string) {
  const after = identity(await handle.stat({ bigint: true }));
  if (!sameIdentity(initial, after)) fail(`file changed while reading: ${path}`);
}

export async function hashRegularFile(path: string, byteLimit: number): Promise<string> {
  const { handle, initial } = await openStableRegular(path);
  const hasher = createHash('sha256');
  const buffer = Buffer.alloc(128 * 1024);
  let position = 0;
  try {
    if (initial.size > BigInt(byteLimit)) fail(`file exceeds size limit: ${path}`);
    while (true) {
      const read = await handle.read(buffer, 0, buffer.length, position);
      if (read.bytesRead === 0) break;
      position += read.bytesRead;
      if (position > byteLimit) fail(`file exceeds size limit while reading: ${path}`);
      hasher.update(buffer.subarray(0, read.bytesRead));
    }
    await verifyStable(handle, initial, path);
    return hasher.digest('hex');
  } finally {
    await handle.close();
  }
}

export async function readRegularFile(path: string, byteLimit: number): Promise<Buffer> {
  const { handle, initial } = await openStableRegular(path);
  const chunks: Buffer[] = [];
  const buffer = Buffer.alloc(64 * 1024);
  let position = 0;
  try {
    if (initial.size > BigInt(byteLimit)) fail(`file exceeds size limit: ${path}`);
    while (true) {
      const read = await handle.read(buffer, 0, buffer.length, position);
      if (read.bytesRead === 0) break;
      position += read.bytesRead;
      if (position > byteLimit) fail(`file exceeds size limit while reading: ${path}`);
      chunks.push(Buffer.from(buffer.subarray(0, read.bytesRead)));
    }
    await verifyStable(handle, initial, path);
    return Buffer.concat(chunks, position);
  } finally {
    await handle.close();
  }
}

export async function snapshotRegularFile(
  source: string,
  destination: string,
  expectedHash: string,
  byteLimit: number,
): Promise<void> {
  const { handle: input, initial } = await openStableRegular(source);
  let output: Awaited<ReturnType<typeof open>> | undefined;
  try {
    output = await open(destination, WRITE_FLAGS, 0o600);
    const hasher = createHash('sha256');
    const buffer = Buffer.alloc(128 * 1024);
    let position = 0;
    if (initial.size > BigInt(byteLimit)) fail(`file exceeds size limit: ${source}`);
    while (true) {
      const read = await input.read(buffer, 0, buffer.length, position);
      if (read.bytesRead === 0) break;
      position += read.bytesRead;
      if (position > byteLimit) fail(`file exceeds size limit while reading: ${source}`);
      const chunk = buffer.subarray(0, read.bytesRead);
      hasher.update(chunk);
      let offset = 0;
      while (offset < chunk.length) {
        const written = await output.write(chunk, offset, chunk.length - offset, position - chunk.length + offset);
        if (!written.bytesWritten) fail(`could not snapshot file: ${source}`);
        offset += written.bytesWritten;
      }
    }
    await verifyStable(input, initial, source);
    if (hasher.digest('hex') !== expectedHash) fail(`SHA-256 mismatch while snapshotting: ${source}`);
    await output.sync();
  } finally {
    await output?.close();
    await input.close();
  }
}

export async function inventory(root: string, limits: InventoryLimits): Promise<InventoryFile[]> {
  const files: InventoryFile[] = [];
  let bytes = 0;
  let entries = 0;

  const walk = async (directory: string, prefix: string): Promise<void> => {
    const dir = await opendir(directory);
    for await (const entry of dir) {
      if (!prefix && entry.name === 'manifest.json') continue;
      entries += 1;
      if (entries > limits.maxEntries) fail('native tree exceeds entry-count limit');
      if (!entry.name || entry.name === '.' || entry.name === '..' || entry.name.includes('/') || entry.name.includes('\\')) {
        fail('native tree contains an invalid path entry');
      }
      const path = prefix ? `${prefix}/${entry.name}` : entry.name;
      if (Buffer.byteLength(path) > limits.maxPathBytes) fail('native tree contains an overlong path');
      const absolute = join(directory, entry.name);
      const stat = await lstat(absolute, { bigint: true });
      if (stat.isSymbolicLink()) fail(`native tree contains a symlink: ${path}`);
      if (stat.isDirectory()) {
        await walk(absolute, path);
        continue;
      }
      if (!stat.isFile()) fail(`native tree contains a special file: ${path}`);
      if (files.length >= limits.maxFiles) fail('native tree exceeds file-count limit');
      if (stat.size > BigInt(limits.maxFileBytes)) fail(`native file exceeds size limit: ${path}`);
      bytes += Number(stat.size);
      if (bytes > limits.maxTotalBytes) fail('native tree exceeds aggregate size limit');
      const mode = Number(stat.mode & 0o777n);
      if (mode !== 0o644 && mode !== 0o755) fail(`native file has an invalid mode: ${path}`);
      const sourceIdentity = identity(stat);
      const fileHash = await hashRegularFile(absolute, limits.maxFileBytes);
      const after = await lstat(absolute, { bigint: true });
      if (after.isSymbolicLink() || !sameIdentity(sourceIdentity, identity(after)) || Number(after.mode & 0o777n) !== mode) {
        fail(`native path changed during inventory: ${path}`);
      }
      files.push({ path, mode, sha256: fileHash, sourceIdentity });
    }
  };

  const rootStat = await lstat(root, { bigint: true });
  if (!rootStat.isDirectory() || rootStat.isSymbolicLink()) fail('native root must be a real directory');
  const canonical = await realpath(root);
  if (canonical !== root) fail('native root must be canonical and contain no symlinked ancestors');
  await walk(root, '');
  const finalStat = await lstat(root, { bigint: true });
  if (finalStat.dev !== rootStat.dev || finalStat.ino !== rootStat.ino) fail('native root changed during inventory');
  return files.sort((left, right) => left.path.localeCompare(right.path));
}

export async function validateFreshPrefix(prefix: string): Promise<void> {
  if (!isAbsolute(prefix) || prefix === '/' || prefix.endsWith('/')) fail('prefix must be a fresh canonical absolute path');
  if (typeof process.getuid !== 'function') fail('cannot verify prefix ownership on this platform');
  const uid = process.getuid();
  const parent = dirname(prefix);
  if (await realpath(parent) !== parent) fail('prefix parent must be canonical');

  let cursor = '/';
  for (const part of parent.split('/').filter(Boolean)) {
    cursor = join(cursor, part);
    const info = await lstat(cursor, { bigint: true });
    if (info.isSymbolicLink() || !info.isDirectory()) fail(`unsafe prefix ancestor: ${cursor}`);
    if (info.uid !== 0n && info.uid !== BigInt(uid)) fail(`prefix ancestor has an untrusted owner: ${cursor}`);
    const sharedWrite = (info.mode & 0o022n) !== 0n;
    const rootSticky = info.uid === 0n && (info.mode & 0o1000n) !== 0n;
    if (sharedWrite && !rootSticky) fail(`prefix ancestor is shared-writable: ${cursor}`);
  }

  try {
    await lstat(prefix);
    fail('prefix already exists; choose a fresh path');
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
  }
}

export async function createPrivateDirectory(path: string): Promise<FileIdentity> {
  await mkdir(path, { mode: 0o700 });
  const info = await lstat(path, { bigint: true });
  if (!info.isDirectory() || info.isSymbolicLink() || (info.mode & 0o077n) !== 0n ||
      (typeof process.getuid === 'function' && info.uid !== BigInt(process.getuid()))) {
    fail(`could not create a private directory: ${path}`);
  }
  return identity(info);
}

export async function copyInventoryFile(
  sourceRoot: string,
  destinationRoot: string,
  item: InventoryFile,
  limits: InventoryLimits,
): Promise<void> {
  const source = join(sourceRoot, item.path);
  const destination = join(destinationRoot, item.path);
  const relativePath = relative(sourceRoot, source);
  if (relativePath.startsWith(`..${sep}`) || relativePath === '..' || isAbsolute(relativePath)) fail('native source path escaped its root');

  const directories: string[] = [];
  for (let directory = dirname(destination); directory !== destinationRoot; directory = dirname(directory)) {
    if (!directory.startsWith(destinationRoot + sep)) fail('native target path escaped its root');
    directories.push(directory);
  }
  for (const directory of directories.reverse()) {
    try {
      const info = await lstat(directory, { bigint: true });
      if (!info.isDirectory() || info.isSymbolicLink() || (info.mode & 0o077n) !== 0n) fail(`unsafe native destination directory: ${directory}`);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== 'ENOENT') throw error;
      await mkdir(directory, { mode: 0o700 });
    }
  }

  const { handle: input, initial } = await openStableRegular(source);
  let output: Awaited<ReturnType<typeof open>> | undefined;
  try {
    output = await open(destination, WRITE_FLAGS, item.mode);
    const hasher: Hash = createHash('sha256');
    const buffer = Buffer.alloc(128 * 1024);
    let position = 0;
    if (!sameIdentity(initial, item.sourceIdentity)) fail(`native source path changed since inventory: ${item.path}`);
    if (initial.size > BigInt(limits.maxFileBytes)) fail(`native file exceeds size limit: ${item.path}`);
    while (true) {
      const read = await input.read(buffer, 0, buffer.length, position);
      if (read.bytesRead === 0) break;
      position += read.bytesRead;
      if (position > limits.maxFileBytes) fail(`native file exceeds size limit while copying: ${item.path}`);
      const chunk = buffer.subarray(0, read.bytesRead);
      hasher.update(chunk);
      let offset = 0;
      while (offset < chunk.length) {
        const written = await output.write(chunk, offset, chunk.length - offset, position - chunk.length + offset);
        if (!written.bytesWritten) fail(`could not copy native file: ${item.path}`);
        offset += written.bytesWritten;
      }
    }
    await verifyStable(input, initial, source);
    if (position !== Number(initial.size) || hasher.digest('hex') !== item.sha256) fail(`native source changed or hash mismatch: ${item.path}`);
    await output.chmod(item.mode);
    await output.sync();
  } finally {
    await output?.close();
    await input.close();
  }
}

export function sameInventorySnapshot(left: InventoryFile[], right: InventoryFile[]): boolean {
  const sort = (files: InventoryFile[]) => files.slice().sort((a, b) => a.path.localeCompare(b.path));
  const first = sort(left), second = sort(right);
  return first.length === second.length && first.every((item, index) => {
    const candidate = second[index];
    return item.path === candidate.path && item.mode === candidate.mode && item.sha256 === candidate.sha256 &&
      sameIdentity(item.sourceIdentity, candidate.sourceIdentity);
  });
}
