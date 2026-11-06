import { randomBytes } from "node:crypto";
import { constants as C } from "node:fs";
import { lstat, mkdir, open, rmdir, unlink, type FileHandle } from "node:fs/promises";
import path from "node:path";
import { admitTrustedDirectory, DirectoryAdmissionFailure, directoryIdentity } from "../state/directory.ts";

/** Native resource ports consume their handles; publication must use the same no-replace protocol. */
export type PrivateDirectoryResources = {
  checkAcl(handle: FileHandle): Promise<{ state: "accepted" | "refused" }>;
  publish(parent: FileHandle, sourceBasename: string, destinationBasename: string, options?: { signal?: AbortSignal }): Promise<{
    state: "published" | "destination_exists" | "refused" | "publication_uncertain";
  }>;
};
export type PrivateDirectoryFailure = "unsafe_directory" | "destination_exists" | "cancelled" | "publication_refused" | "invalid_input";
export class PrivateDirectoryError extends Error {
  readonly category: PrivateDirectoryFailure;
  constructor(category: PrivateDirectoryFailure) { super(category); this.name = "PrivateDirectoryError"; this.category = category; }
}
export type OwnedPrivateFile = { path: string; bytes: Uint8Array };
export type PrivateDirectoryResult = { state: "committed"; path: string } | { state: "publication-uncertain" };
const fail = (category: PrivateDirectoryFailure): never => { throw new PrivateDirectoryError(category); };
const cancelled = (signal?: AbortSignal) => { if (signal?.aborted) fail("cancelled"); };
const safeRelativePath = (name: string) => typeof name === "string" && !name.includes("\0") && !/[\uD800-\uDBFF](?![\uDC00-\uDFFF])|(?<![\uD800-\uDBFF])[\uDC00-\uDFFF]/u.test(name)
  && !name.startsWith("/") && !name.includes("\\") && name.split("/").every(part => part !== "" && part !== "." && part !== "..");
const flags = C.O_RDONLY | (C.O_NOFOLLOW ?? 0) | (C.O_DIRECTORY ?? 0);
const dirStat = directoryIdentity;
async function openedDir(name: string): Promise<FileHandle> { return open(name, flags); }
async function cleanupStage(parent: FileHandle, stagePath: string, parentIdentity: string, stageIdentity: string, created: Map<string, string>, createdDirs: Map<string, string>): Promise<boolean> {
  try {
    const [parentNow, parentFd, stageNow] = await Promise.all([lstat(path.dirname(stagePath), { bigint: true }), parent.stat({ bigint: true }), lstat(stagePath, { bigint: true })]);
    if (!parentNow.isDirectory() || dirStat(parentNow) !== parentIdentity || dirStat(parentFd) !== parentIdentity || !stageNow.isDirectory() || dirStat(stageNow) !== stageIdentity) return false;
    for (const [rel, identity] of createdDirs) { const named = await lstat(path.join(stagePath, rel), { bigint: true }); if (!named.isDirectory() || named.isSymbolicLink() || dirStat(named) !== identity) return false; }
    for (const [rel, identity] of created) { const st = await lstat(path.join(stagePath, rel), { bigint: true }); if (!st.isFile() || dirStat(st) !== identity) return false; }
    for (const [rel, identity] of [...created].reverse()) { const target = path.join(stagePath, rel); const st = await lstat(target, { bigint: true }).catch(() => undefined); if (!st) continue; if (!st.isFile() || st.isSymbolicLink() || dirStat(st) !== identity) return false; await unlink(target); }
    const dirs = new Set([...created.keys()].flatMap(rel => { const parts = rel.split("/"); const out: string[] = []; while (parts.length > 1) { parts.pop(); out.push(parts.join("/")); } return out; }));
    for (const rel of [...dirs].sort((a,b) => b.length-a.length)) { const p = path.join(stagePath, rel); const st = await lstat(p, { bigint: true }).catch(() => undefined); if (st) { if (!st.isDirectory() || st.isSymbolicLink()) return false; await rmdir(p); } }
    const parentCheck = await lstat(path.dirname(stagePath), { bigint: true }); const stageCheck = await lstat(stagePath, { bigint: true });
    if (dirStat(parentCheck) !== parentIdentity || dirStat(stageCheck) !== stageIdentity) return false;
    await rmdir(stagePath); await parent.sync(); return true;
  } catch { return false; }
}

/**
 * INTERNAL: stage caller-admitted, bounded, detached byte records under an admitted parent.
 * Ownership transfers on entry; every provided bytes view is wiped after all effects settle.
 * Paths must already be allowlisted by the caller. This guards traversal only.
 */
export async function publishPrivateDirectory(parentPath: string, destination: string, files: OwnedPrivateFile[], resources: PrivateDirectoryResources,
  options: { signal?: AbortSignal } = {}): Promise<PrivateDirectoryResult> {
  const { signal } = options;
  let parent: FileHandle | undefined, stage: string | undefined, stagePath: string | undefined;
  const created = new Map<string, string>(), createdDirs = new Map<string, string>(); let phase: "preparing" | "dispatched" | "published" | "uncertain" = "preparing";
  let parentIdentity = "", stageIdentity = "";
  try {
    cancelled(signal);
    if (!Array.isArray(files) || files.some(file => !file || !safeRelativePath(file.path) || !(file.bytes instanceof Uint8Array)) || new Set(files.map(file => file.path)).size !== files.length) fail("invalid_input");
    const requested = path.resolve(destination), basename = path.basename(destination);
    if (path.dirname(requested) !== path.resolve(parentPath)) fail("invalid_input");
    const admitted = await admitTrustedDirectory(parentPath, resources, "publication-parent", signal); parent = admitted.handle;
    const parentBefore = await parent.stat({ bigint: true }); parentIdentity = dirStat(parentBefore); if (admitted.identity !== parentIdentity) fail("unsafe_directory");
    let target: boolean; try { await lstat(requested); target = true; } catch (e) { if ((e as NodeJS.ErrnoException).code !== "ENOENT") fail("unsafe_directory"); target = false; }
    if (target) fail("destination_exists"); cancelled(signal);
    stage = `.apparent-stage-${randomBytes(16).toString("hex")}`; stagePath = path.join(parentPath, stage); await mkdir(stagePath, { mode: 0o700 });
    const stageHandle = await openedDir(stagePath); let stageStat;
    try { stageStat = await stageHandle.stat({ bigint: true }); } catch (e) { await stageHandle.close().catch(() => {}); throw e; }
    stageIdentity = dirStat(stageStat); createdDirs.set("", stageIdentity);
    if (!stageStat.isDirectory() || stageStat.uid !== BigInt(process.getuid!()) || (stageStat.mode & 0o777n) !== 0o700n) { await stageHandle.close(); fail("unsafe_directory"); }
    if ((await resources.checkAcl(stageHandle)).state !== "accepted") fail("unsafe_directory");
    const stageNamed = await lstat(stagePath, { bigint: true }); if (stageNamed.dev !== stageStat.dev || stageNamed.ino !== stageStat.ino || stageNamed.mode !== stageStat.mode || stageNamed.uid !== stageStat.uid) fail("unsafe_directory");
    for (const file of files) {
      cancelled(signal); const targetPath = path.join(stagePath, file.path); const relParts = file.path.split("/"); let dir = stagePath;
      for (const part of relParts.slice(0, -1)) {
        dir = path.join(dir, part); try { await mkdir(dir, { mode: 0o700 }); } catch (e) { if ((e as NodeJS.ErrnoException).code !== "EEXIST") throw e; }
        const dh = await openedDir(dir); let st; let consumed = false;
        try { st = await dh.stat({ bigint: true }); consumed = true; if ((await resources.checkAcl(dh)).state !== "accepted") fail("unsafe_directory"); }
        catch (e) { if (!consumed) await dh.close().catch(() => {}); throw e; }
        const named = await lstat(dir, { bigint: true }); if (!st.isDirectory() || named.dev !== st.dev || named.ino !== st.ino || named.mode !== st.mode || named.uid !== st.uid || (st.mode & 0o777n) !== 0o700n) fail("unsafe_directory");
        createdDirs.set(path.relative(stagePath, dir), dirStat(st));
      }
      const fh = await open(targetPath, C.O_WRONLY | C.O_CREAT | C.O_EXCL | (C.O_NOFOLLOW ?? 0), 0o600);
      try { created.set(file.path, dirStat(await fh.stat({ bigint: true }))); await fh.writeFile(file.bytes); await fh.sync(); const st = await fh.stat(); if (!st.isFile() || (st.mode & 0o777) !== 0o600) fail("unsafe_directory"); } finally { await fh.close(); }
    }
    const directories = new Set(["", ...files.map(f => path.posix.dirname(f.path)).filter(d => d !== ".")]);
    for (const rel of [...directories].sort((a,b) => b.length-a.length)) { const d = await open(path.join(stagePath, rel), flags); try { await d.sync(); } finally { await d.close(); } }
    await parent.sync(); cancelled(signal);
    const stageBeforeDispatch = await lstat(stagePath, { bigint: true });
    if (!stageBeforeDispatch.isDirectory() || stageBeforeDispatch.dev !== stageStat.dev || stageBeforeDispatch.ino !== stageStat.ino || stageBeforeDispatch.uid !== stageStat.uid || stageBeforeDispatch.mode !== stageStat.mode) fail("unsafe_directory");
    const preDispatchPath = await lstat(parentPath, { bigint: true }), preDispatchFd = await parent.stat({ bigint: true });
    if (dirStat(preDispatchPath) !== dirStat(preDispatchFd) || preDispatchFd.dev !== parentBefore.dev || preDispatchFd.ino !== parentBefore.ino || preDispatchFd.mode !== parentBefore.mode || preDispatchFd.uid !== parentBefore.uid) fail("unsafe_directory");
    cancelled(signal); phase = "dispatched"; const dispatchParent = parent; parent = undefined;
    const outcome = await resources.publish(dispatchParent, stage, basename, { signal });
    if (outcome.state === "destination_exists" || outcome.state === "refused") {
      const source = await lstat(stagePath, { bigint: true }).catch(() => undefined), namedParent = await lstat(parentPath, { bigint: true }).catch(() => undefined);
      if (source?.isDirectory() && dirStat(source) === stageIdentity && namedParent?.isDirectory() && dirStat(namedParent) === parentIdentity) {
        const cleanupParent = await openedDir(parentPath); const cleaned = await cleanupStage(cleanupParent, stagePath, parentIdentity, stageIdentity, created, createdDirs); await cleanupParent.close();
        if (!cleaned) return { state: "publication-uncertain" }; phase = "preparing"; if (outcome.state === "destination_exists") fail("destination_exists"); fail("publication_refused");
      }
      phase = "uncertain"; return { state: "publication-uncertain" };
    }
    if (outcome.state !== "published") { phase = "uncertain"; return { state: "publication-uncertain" }; }
    phase = "published"; const namedParent = await openedDir(parentPath); let dest: FileHandle;
    try { dest = await openedDir(requested); } catch (error) { await namedParent.close().catch(() => {}); throw error; }
    try {
      const [pNamed, pFd, dNamed, dFd] = await Promise.all([lstat(parentPath, { bigint: true }), namedParent.stat({ bigint: true }), lstat(requested, { bigint: true }), dest.stat({ bigint: true })]);
      if (dirStat(pNamed) !== dirStat(pFd) || dirStat(pFd) !== parentIdentity || dirStat(dNamed) !== dirStat(dFd) || dFd.dev !== stageStat.dev || dFd.ino !== stageStat.ino || dFd.uid !== BigInt(process.getuid!()) || (dFd.mode & 0o777n) !== 0o700n) return { state: "publication-uncertain" };
      await namedParent.sync();
    } finally { await namedParent.close().catch(() => {}); await dest.close().catch(() => {}); }
    return { state: "committed", path: requested };
  } catch (error) {
    if (phase === "preparing" && parent && stage && stagePath) await cleanupStage(parent, stagePath, parentIdentity, stageIdentity, created, createdDirs);
    if (phase !== "preparing") return { state: "publication-uncertain" };
    if (error instanceof PrivateDirectoryError) throw error;
    if (error instanceof DirectoryAdmissionFailure) fail(error.category === "cancelled" ? "cancelled" : "unsafe_directory");
    fail("publication_refused");
  } finally { await parent?.close().catch(() => {}); if (Array.isArray(files)) for (const file of files) if (file?.bytes instanceof Uint8Array) file.bytes.fill(0); }
  return fail("publication_refused");
}
