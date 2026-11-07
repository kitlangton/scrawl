/** Internal bounded retirement. The caller's fresh fence is the eligibility authority. */
import { constants as C, type BigIntStats } from "node:fs";
import { lstat, open, readdir, unlink, type FileHandle } from "node:fs/promises";
import path from "node:path";
import { cleanupPrivateDirectoryStage } from "../recipient/private-directory.ts";
import { admitTrustedDirectory, directoryIdentity } from "../state/directory.ts";
import { readStateEvidence, stateFileStamp } from "../state/credential-files.ts";
import { checkSignal, samePathFd, type CurrentNamespaceLease, type NamespaceResources, type StateDirectory } from "../state/queue-transaction.ts";
import { decodeJob, encodeJob, type Job } from "./job.ts";
import { decodeStoredEncryptedSpool, type StoredSpoolProfile } from "../sender/spool-record.ts";
import type { Context } from "../recipient/crypto.ts";

export type QueueRetirementState = "retired" | "refused" | "uncertain";
export type QueueRetirementResult = Readonly<{ state: QueueRetirementState }>;
export type QueueRetirementResources = NamespaceResources;
export type QueueRetirementInput = Readonly<{ job: Readonly<Job>; expectedStamp: string; assertFresh(): Promise<void> }>;
const ident = (s: BigIntStats) => directoryIdentity(s);
const result = (state: QueueRetirementState): QueueRetirementResult => Object.freeze({ state });
const noFollow = C.O_RDONLY | (C.O_NOFOLLOW ?? 0) | (C.O_NONBLOCK ?? 0);
type PinnedFile = { identity: string; stamp: string };

async function pinFile(file: string, resources: QueueRetirementResources): Promise<PinnedFile> {
  const handle = await open(file, noFollow); let consumed = false;
  try {
    const before = await handle.stat({ bigint: true }), named = await lstat(file, { bigint: true });
    if (!before.isFile() || before.uid !== BigInt(process.getuid!()) || (before.mode & 0o077n) !== 0n || !named.isFile() || named.isSymbolicLink() || ident(before) !== ident(named)) throw new Error();
    consumed = true;
    if ((await resources.checkAcl(handle)).state !== "accepted") throw new Error();
    const after = await lstat(file, { bigint: true });
    if (!after.isFile() || stateFileStamp(after) !== stateFileStamp(before)) throw new Error();
    return { identity: ident(before), stamp: stateFileStamp(before) };
  } finally { if (!consumed) await handle.close().catch(() => {}); }
}
async function listExact(dir: string, expected: Set<string>): Promise<void> {
  const names = await readdir(dir);
  if (names.length !== expected.size || names.some(name => !expected.has(name))) throw new Error();
}

async function inventorySpool(state: StateDirectory, job: Job, resources: QueueRetirementResources, signal: AbortSignal) {
  const rootPath = path.join(state.path, `${job.id}.sealed`);
  try { await lstat(rootPath); }
  catch (error) { if ((error as NodeJS.ErrnoException).code === "ENOENT") return null; throw error; }
  let root: Awaited<ReturnType<typeof admitTrustedDirectory>>;
  root = await admitTrustedDirectory(rootPath, resources, "private-state", signal);
  try {
    const grouped = job.grouped === true, profile: StoredSpoolProfile = grouped ? "evidence-first-v1" : "root";
    if (!job.encrypted || !job.encryption || job.encryption.context.captureId !== job.id
      || job.encryption.context.origin !== job.server || job.encryption.context.group !== undefined) throw new Error();
    const receipt = await readStateEvidence(root, "spool-state.json", 2_000_000, resources, signal,
      bytes => decodeStoredEncryptedSpool(bytes, { context: { ...job.encryption!.context } as Context, profile }));
    if (!receipt) throw new Error();
    const context = receipt.value.metadata.context;
    if (context.captureId !== job.id || context.origin !== job.server || !receipt.value.metadata.recipientGrants.includes(context.senderGrantId)) throw new Error();
    const expected = new Map(receipt.value.files.map(asset => [asset.path, asset]));
    if (expected.size !== receipt.value.files.length || expected.size > 34) throw new Error();
    const dirs = new Map<string, string>([["", root.identity]]), files = new Map<string, string>(), stamps = new Map<string, string>();
    const rootNames = new Set<string>(["spool-state.json", ...(grouped ? ["evidence", "media"] : [])]);
    if (!grouped) for (const name of expected.keys()) rootNames.add(name.split("/")[0]!);
    await listExact(rootPath, rootNames);
    const receiptPath = path.join(rootPath, "spool-state.json"), receiptStat = await lstat(receiptPath, { bigint: true });
    if (stateFileStamp(receiptStat) !== receipt.expectedStamp) throw new Error();
    files.set("spool-state.json", ident(receiptStat));
    stamps.set("spool-state.json", receipt.expectedStamp);
    for (const group of grouped ? ["evidence", "media"] : [""]) {
      const prefix = group ? `${group}/` : "", names = [...expected.keys()].filter(name => name.startsWith(prefix)).map(name => name.slice(prefix.length));
      const dirPath = group ? path.join(rootPath, group) : rootPath;
      if (group) {
        const admitted = await admitTrustedDirectory(dirPath, resources, "private-state", signal);
        dirs.set(group, admitted.identity); await admitted.handle.close();
        await listExact(dirPath, new Set(names.map(name => name.split("/")[0]!)));
      }
      const chunks = names.filter(name => name.startsWith("sealed/"));
      if (chunks.length) {
        const chunkPath = path.join(dirPath, "sealed"), admitted = await admitTrustedDirectory(chunkPath, resources, "private-state", signal);
        dirs.set(`${group ? `${group}/` : ""}sealed`, admitted.identity); await admitted.handle.close();
        await listExact(chunkPath, new Set(chunks.map(name => name.slice("sealed/".length))));
      }
      for (const name of names) {
        const asset = expected.get(prefix + name)!, file = path.join(rootPath, prefix, name), pinned = await pinFile(file, resources);
        if ((await lstat(file, { bigint: true })).size !== BigInt(asset.size)) throw new Error();
        files.set(prefix + name, pinned.identity);
        stamps.set(prefix + name, pinned.stamp);
      }
    }
    if (files.size !== expected.size + 1 || files.size > 35) throw new Error();
    return { handle: root.handle, identity: root.identity, files, dirs, stamps };
  } catch (error) { await root.handle.close().catch(() => {}); throw error; }
}

/** Removes only the validated ID's .sealed subtree and .job record. */
export async function retireQueuedJob(state: StateDirectory, lease: CurrentNamespaceLease,
  resources: QueueRetirementResources, input: QueueRetirementInput): Promise<QueueRetirementResult> {
  let effects = false, spoolHandle: FileHandle | undefined, expectedBytes: Uint8Array | undefined;
  try {
    const expectedStamp = input.expectedStamp, assertFresh = input.assertFresh;
    expectedBytes = encodeJob(input.job as Job);
    const expected = decodeJob(expectedBytes), id = expected.id;
    if (!/^[0-9a-f]{32}$/.test(id) || typeof expectedStamp !== "string" || !expectedStamp
      || typeof assertFresh !== "function") return result("refused");
    const jobPath = path.join(state.path, `${id}.job`), spoolPath = path.join(state.path, `${id}.sealed`);
    const fence = async () => {
      await lease.assertCurrent();
      if (!await samePathFd(state)) throw new Error();
      await assertFresh();
      await lease.assertCurrent();
      if (!await samePathFd(state)) throw new Error();
    };
    const verifyRootOnly = async () => {
      await lease.assertCurrent();
      if (!await samePathFd(state)) throw new Error();
      await lease.assertCurrent();
    };
    const verifyJob = async () => {
      const named = await lstat(jobPath, { bigint: true });
      if (!named.isFile() || named.isSymbolicLink() || stateFileStamp(named) !== expectedStamp) throw new Error();
      const current = await readStateEvidence(state, `${id}.job`, 2_000_000, resources, lease.signal, decodeJob);
      if (!current || current.expectedStamp !== expectedStamp) throw new Error();
      const observed = encodeJob(current.value);
      try { if (!Buffer.from(observed.buffer, observed.byteOffset, observed.byteLength).equals(expectedBytes!)) throw new Error(); }
      finally { observed.fill(0); }
    };
    await fence(); checkSignal(undefined, lease.signal); await verifyJob();
    let spool: Awaited<ReturnType<typeof inventorySpool>>;
    try { spool = await inventorySpool(state, expected, resources, lease.signal); } catch { return result("refused"); }
    if (spool) {
      spoolHandle = spool.handle;
      const authorizedMissingFiles = new Set<string>(), authorizedMissingDirs = new Set<string>();
      const beforeEachEffect = async (relative: string, kind: "file" | "directory") => {
        try {
          await fence(); await verifyJob();
          const root = await lstat(spoolPath, { bigint: true });
          if (authorizedMissingDirs.has("") ? root.isDirectory() : !root.isDirectory() || ident(root) !== spool!.identity) return false;
          for (const [rel, identity] of spool!.dirs) {
            const st = await lstat(rel ? path.join(spoolPath, rel) : spoolPath, { bigint: true }).catch(error => (error as NodeJS.ErrnoException).code === "ENOENT" ? null : undefined);
            if (st === null && authorizedMissingDirs.has(rel)) continue;
            if (!st || !st.isDirectory() || st.isSymbolicLink() || ident(st) !== identity) return false;
          }
          for (const [rel, identity] of spool!.files) {
            const st = await lstat(path.join(spoolPath, rel), { bigint: true }).catch(error => (error as NodeJS.ErrnoException).code === "ENOENT" ? null : undefined);
            if (st === null && authorizedMissingFiles.has(rel)) continue;
            if (!st || !st.isFile() || st.isSymbolicLink() || ident(st) !== identity || stateFileStamp(st) !== spool!.stamps.get(rel)) return false;
          }
          if (kind === "file") authorizedMissingFiles.add(relative);
          else authorizedMissingDirs.add(relative);
          effects = true;
          return true;
        } catch { return false; }
      };
      const cleaned = await cleanupPrivateDirectoryStage(state.handle, spoolPath, state.identity, spool.identity,
        spool.files, spool.dirs, beforeEachEffect);
      if (!cleaned) return result(effects ? "uncertain" : "refused");
      await fence(); await verifyJob();
    }
    await fence(); await verifyJob();
    const final = await lstat(jobPath, { bigint: true });
    if (!final.isFile() || final.isSymbolicLink() || stateFileStamp(final) !== expectedStamp) return result(effects ? "uncertain" : "refused");
    await fence(); await verifyJob();
    effects = true; await unlink(jobPath); await state.handle.sync(); await verifyRootOnly();
    const remains = await lstat(jobPath, { bigint: true }).catch(error => (error as NodeJS.ErrnoException).code === "ENOENT" ? null : undefined);
    return remains === null ? result("retired") : result("uncertain");
  } catch { return result(effects ? "uncertain" : "refused"); }
  finally { expectedBytes?.fill(0); await spoolHandle?.close().catch(() => {}); }
}
