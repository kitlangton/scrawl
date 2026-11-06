import { type FileHandle } from "node:fs/promises";

export type PrivateFileResources = { checkAcl(handle: FileHandle): Promise<{ state: "accepted" | "refused" }> };
export type PrivateFileFailure = "invalid_data" | "unsafe_file" | "cancelled" | "unavailable";
export class PrivateFileError extends Error {
  readonly category: PrivateFileFailure;
  constructor(category: PrivateFileFailure) { super(category); this.category = category; this.name = "PrivateFileError"; }
}
const fail = (category: PrivateFileFailure): never => { throw new PrivateFileError(category); };

/** Read one bounded private descriptor, prove stability and ACL before decoding, then wipe bytes.
 * Always consumes the handle. Caller owns the bound and pathname/inode admission. */
export async function consumePrivateFile<T>(handle: FileHandle,
  options: { maximumBytes: number; signal?: AbortSignal }, resources: PrivateFileResources,
  decode: (bytes: Uint8Array) => T): Promise<T> {
  const signal = options.signal, maximumBytes = options.maximumBytes;
  let owns = true, bytes: Buffer | undefined;
  try {
    if (!Number.isSafeInteger(maximumBytes) || maximumBytes < 0 || maximumBytes > 2_000_000) fail("unavailable");
    if (signal?.aborted) fail("cancelled");
    const before = await handle.stat({ bigint: true });
    if (typeof process.getuid !== "function" || !before.isFile() || before.uid !== BigInt(process.getuid())
      || (before.mode & 0o077n) !== 0n || before.size > BigInt(maximumBytes)) fail("unsafe_file");
    bytes = Buffer.alloc(maximumBytes + 1);
    let length = 0;
    while (length < bytes.length) {
      if (signal?.aborted) fail("cancelled");
      const n = await handle.read(bytes, length, bytes.length - length, length);
      if (!n.bytesRead) break;
      length += n.bytesRead;
    }
    if (signal?.aborted) fail("cancelled");
    const after = await handle.stat({ bigint: true });
    if (length > maximumBytes || BigInt(length) !== before.size || !after.isFile() || before.dev !== after.dev || before.ino !== after.ino
      || before.uid !== after.uid || before.mode !== after.mode || before.size !== after.size || before.mtimeNs !== after.mtimeNs || before.ctimeNs !== after.ctimeNs) fail("unsafe_file");
    owns = false;
    const acl = await resources.checkAcl(handle);
    if (acl.state !== "accepted") fail("unsafe_file");
    if (signal?.aborted) fail("cancelled");
    try { return decode(bytes.subarray(0, length)); } catch { return fail("invalid_data"); }
  } catch (e) {
    if (e instanceof PrivateFileError) throw e;
    if (signal?.aborted) fail("cancelled");
    fail("unavailable");
  } finally {
    bytes?.fill(0);
    if (owns) await handle.close().catch(() => {});
  }
  return fail("unavailable");
}
