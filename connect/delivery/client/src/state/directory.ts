import { constants as C } from "node:fs";
import { lstat, open, type FileHandle } from "node:fs/promises";
import path from "node:path";

export type DirectoryResources = { checkAcl(handle: FileHandle): Promise<{ state: "accepted" | "refused" }> };
export type DirectoryUse = "publication-parent" | "private-state";
export type DirectoryAdmissionError = "unsafe_directory" | "cancelled";
export class DirectoryAdmissionFailure extends Error {
  readonly category: DirectoryAdmissionError;
  constructor(category: DirectoryAdmissionError) { super(category); this.category = category; this.name = "DirectoryAdmissionFailure"; }
}
const fail = (category: DirectoryAdmissionError): never => { throw new DirectoryAdmissionFailure(category); };
const cancelled = (signal?: AbortSignal) => { if (signal?.aborted) fail("cancelled"); };
const badUnicode = (s: string) => s.includes("\0") || /[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/u.test(s);
export function directoryIdentity(s: { dev: bigint; ino: bigint; mode: bigint; uid: bigint }): string { return `${s.dev}:${s.ino}:${s.mode}:${s.uid}`; }
const directoryFlags = C.O_RDONLY | (C.O_NOFOLLOW ?? 0) | (C.O_DIRECTORY ?? 0);

/** Admit an existing directory by walking every component without following links.
 * ACL resources consume each descriptor; the returned final descriptor remains caller-owned.
 * Same-uid hostile processes remain outside this pathname boundary. */
export async function admitTrustedDirectory(pathInput: string, resources: DirectoryResources,
  use: DirectoryUse, signal?: AbortSignal): Promise<{ handle: FileHandle; identity: string; path: string }> {
  let handle: FileHandle | undefined;
  try {
    if (use !== "publication-parent" && use !== "private-state") fail("unsafe_directory");
    if (typeof pathInput !== "string" || badUnicode(pathInput) || pathInput.split(path.sep).includes("..")) fail("unsafe_directory");
    const full = path.resolve(pathInput);
    const parts = full.split(path.sep).filter(Boolean);
    let prefix = path.parse(full).root;
    let privateAncestor = false;
    let expected = "";
    for (let i = -1; i < parts.length; i++) {
      cancelled(signal);
      if (i >= 0) prefix = path.join(prefix, parts[i]!);
      const leaf = i === parts.length - 1;
      const h = await open(prefix, directoryFlags);
      let consumed = false;
      let st, named;
      try {
        st = await h.stat({ bigint: true }); named = await lstat(prefix, { bigint: true });
        const sticky = (st.mode & 0o1000n) !== 0n;
        const allowedWritable = (sticky && (use === "publication-parent" || !leaf)) || (privateAncestor && !leaf);
        if (!st.isDirectory() || !named.isDirectory() || named.isSymbolicLink() || st.dev !== named.dev || st.ino !== named.ino
          || (st.uid !== 0n && st.uid !== BigInt(process.getuid!())) || ((st.mode & 0o022n) !== 0n && !allowedWritable)
          || (use === "private-state" && leaf && (st.uid !== BigInt(process.getuid!()) || (st.mode & 0o077n) !== 0n))
          || (i === -1 && st.uid !== 0n)) throw new Error();
        const currentIdentity = directoryIdentity(st);
        if (!leaf) {
          const aclHandle = h;
          consumed = true;
          const acl = await resources.checkAcl(aclHandle);
          if (acl.state !== "accepted") throw new Error();
          const after = await lstat(prefix, { bigint: true });
          if (!after.isDirectory() || directoryIdentity(after) !== currentIdentity) throw new Error();
          privateAncestor ||= (st.mode & 0o077n) === 0n;
          continue;
        }
        // Retain the leaf handle: unlike the ACL adapter's ancestors, this is used by the caller.
        consumed = true;
        const acl = await resources.checkAcl(h);
        // ACL consumes this descriptor, so admission uses a second caller-owned descriptor below.
        if (acl.state !== "accepted") throw new Error();
        const namedAfter = await lstat(prefix, { bigint: true });
        if (!namedAfter.isDirectory() || directoryIdentity(namedAfter) !== currentIdentity) { throw new Error(); }
        expected = currentIdentity;
        handle = await open(prefix, directoryFlags);
        const fdAfter = await handle.stat({ bigint: true });
        const pathAfter = await lstat(prefix, { bigint: true });
        if (directoryIdentity(fdAfter) !== expected || directoryIdentity(pathAfter) !== expected) throw new Error();
        return { handle, identity: expected, path: full };
      } catch (e) { if (!consumed) await h.close().catch(() => {}); throw e; }
    }
    return fail("unsafe_directory");
  } catch (e) {
    await handle?.close().catch(() => {});
    if (e instanceof DirectoryAdmissionFailure) throw e;
    if (signal?.aborted) fail("cancelled");
    fail("unsafe_directory");
  }
  return fail("unsafe_directory");
}
