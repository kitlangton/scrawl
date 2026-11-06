import { createHash } from "node:crypto";
import { constants } from "node:fs";
import type { BigIntStats } from "node:fs";
import { lstat, open, realpath } from "node:fs/promises";
import { isAbsolute, join, parse, resolve } from "node:path";

export interface SealedFileForRead {
  relativePath: string;
  identity: { device: string; inode: string; size: number; modifiedNs: string };
  sha256: string;
}

const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
const fail = (scope: string, message: string): never => { throw new TypeError(`${scope} read rejected: ${message}`); };

export function normalizeSealedRelative(relative: string, scope: string): string {
  if (!relative || relative.includes("\0") || isAbsolute(relative) || /^[A-Za-z]:/u.test(relative) || relative.split(/[\\/]/u).some(part => !part || part === "." || part === "..")) fail(scope, "invalid or escaping sealed path");
  return relative.replaceAll("\\", "/");
}

function statIdentity(stat: BigIntStats): SealedFileForRead["identity"] {
  return { device: String(stat.dev), inode: String(stat.ino), size: Number(stat.size), modifiedNs: String(stat.mtimeNs) };
}

export async function assertSealedRoot(root: string, scope: string): Promise<void> {
  if (!isAbsolute(root) || resolve(root) !== root || root.includes("\0") || root === parse(root).root) fail(scope, "trusted root must be absolute and normalized");
  const volume = parse(root).root;
  let current = volume;
  for (const component of root.slice(volume.length).split(/[\\/]+/u).filter(Boolean)) {
    current = join(current, component);
    const stat = await lstat(current).catch(() => fail(scope, "trusted root or parent is unavailable"));
    if (!stat.isDirectory() || stat.isSymbolicLink()) fail(scope, "trusted root or parent is symlinked or not a directory");
  }
  if (await realpath(root).catch(() => "") !== root) fail(scope, "trusted root aliases another path");
}

/** Read one caller-attested seal without following aliases; identity and hash are checked around the read. */
export async function readSealedFile(root: string, file: SealedFileForRead, maxFileBytes: number, remainingBytes: number, scope: string): Promise<Uint8Array> {
  const relative = normalizeSealedRelative(file.relativePath, scope);
  const path = join(root, ...relative.split("/"));
  const parentParts = relative.split("/").slice(0, -1);
  let parent = root;
  for (const part of parentParts) {
    parent = join(parent, part);
    const stat = await lstat(parent).catch(() => fail(scope, "sealed path parent is unavailable"));
    if (!stat.isDirectory() || stat.isSymbolicLink()) fail(scope, "sealed path parent is symlinked or not a directory");
  }
  const beforePath = await lstat(path, { bigint: true }).catch(() => fail(scope, "sealed file is unavailable"));
  if (!beforePath.isFile() || beforePath.isSymbolicLink() || beforePath.nlink !== 1n) fail(scope, "sealed file is not a regular private path");
  if (beforePath.size > BigInt(maxFileBytes) || beforePath.size > BigInt(remainingBytes)) fail(scope, "sealed file exceeds byte limit");
  if (constants.O_NOFOLLOW === undefined) fail(scope, "platform does not support no-follow file opens");
  const handle = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW).catch(() => fail(scope, "sealed file could not be opened without following links"));
  try {
    const before = await handle.stat({ bigint: true });
    if (!before.isFile() || String(before.dev) !== file.identity.device || String(before.ino) !== file.identity.inode || Number(before.size) !== file.identity.size || String(before.mtimeNs) !== file.identity.modifiedNs || before.size > BigInt(maxFileBytes) || before.size > BigInt(remainingBytes)) fail(scope, "sealed file identity changed or exceeds byte limit");
    const chunks: Buffer[] = [];
    const buffer = Buffer.allocUnsafe(64 * 1024);
    let total = 0;
    while (true) {
      const { bytesRead } = await handle.read(buffer, 0, Math.min(buffer.length, maxFileBytes - total + 1), null);
      if (!bytesRead) break;
      total += bytesRead;
      if (total > maxFileBytes || total > remainingBytes) fail(scope, "sealed file exceeds byte limit");
      chunks.push(Buffer.from(buffer.subarray(0, bytesRead)));
    }
    const after = await handle.stat({ bigint: true });
    const afterPath = await lstat(path, { bigint: true }).catch(() => fail(scope, "sealed file was replaced while reading"));
    const identity = statIdentity(beforePath);
    if (!same(statIdentity(after), identity) || !same(statIdentity(afterPath), identity) || String(after.dev) !== file.identity.device || String(after.ino) !== file.identity.inode || total !== file.identity.size) fail(scope, "sealed file changed while reading");
    const bytes = Buffer.concat(chunks, total);
    if (createHash("sha256").update(bytes).digest("hex") !== file.sha256) fail(scope, "sealed file hash mismatch");
    return new Uint8Array(bytes);
  } finally { await handle.close(); }
}
