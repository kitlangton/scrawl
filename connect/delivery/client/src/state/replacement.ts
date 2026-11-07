import { createHash, randomBytes } from "node:crypto";
import { constants as C } from "node:fs";
import { lstat, open, unlink, type FileHandle } from "node:fs/promises";
import path from "node:path";
import { admitTrustedDirectory, directoryIdentity, type DirectoryResources } from "./directory.ts";
import { consumePrivateFile } from "./private-file.ts";

type FileIdentity = { device: string; inode: string };
export type StateReplacementResult = { state: "committed" | "refused" | "destination_changed" | "publication_uncertain" };
export type StateReplacementResources = DirectoryResources & {
  replaceFile(parent: FileHandle, source: string, destination: string,
    expected: { source: FileIdentity; destination: FileIdentity | null }, options?: { signal?: AbortSignal }): Promise<{
      state: "replaced" | "refused" | "destination_changed" | "publication_uncertain";
    }>;
};
type Fence = { assertHeld(): void; readonly signal: AbortSignal };
const digest = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
const flags = C.O_RDONLY | C.O_NOFOLLOW | C.O_NONBLOCK;
const fileIdentity = (s: { dev: bigint; ino: bigint }): FileIdentity => ({ device: String(s.dev), inode: String(s.ino) });
const safeName = (s: string) => typeof s === "string" && /^[a-zA-Z0-9][a-zA-Z0-9._-]{0,127}$/.test(s) && !s.endsWith(".lock");
const validIdentity = (s: FileIdentity) => [s.device, s.inode].every(v => typeof v === "string" && /^(0|[1-9][0-9]{0,19})$/.test(v) && BigInt(v) <= 18446744073709551615n);

/** Internal preparation for the coordinated writer cutover; no production caller.
 * Caller must own the sole writer and hold the admitted queue lock for mutable
 * state. The sole pre-queue exception is a new immutable revocation-<id>.intent
 * with expectedDestination=null: its caller pins the admitted directory and
 * retains its publication fence throughout the operation. It must never replace
 * an existing intent or use this exception for another state record.
 * This does not authorize a second writer alongside Rust. It replaces
 * bytes only; journal ordering, cancellation epochs and state policy are callers'.
 * After dispatch, uncertainty retains all observed files and forbids retry/cleanup.
 */
export async function replacePrivateStateFile(directoryPath: string, name: string, input: Uint8Array,
  expectedDestination: FileIdentity | null, fence: Fence, resources: StateReplacementResources,
  options: { signal?: AbortSignal } = {}): Promise<StateReplacementResult> {
  let bytes: Uint8Array | undefined, parent: FileHandle | undefined, stageHandle: FileHandle | undefined;
  let stagePath: string | undefined, stageIdentity: string | undefined, parentIdentity: string | undefined;
  let admittedPath: string | undefined, phase: "preparing" | "dispatched" = "preparing";
  let callerSignal: AbortSignal | undefined, leaseSignal: AbortSignal;
  const check = () => { fence.assertHeld(); if (leaseSignal.aborted || callerSignal?.aborted) throw new Error(); };
  const sameParent = async () => {
    const named = await lstat(admittedPath!, { bigint: true });
    if (!named.isDirectory() || named.isSymbolicLink() || directoryIdentity(named) !== parentIdentity) throw new Error();
    if (parent && directoryIdentity(await parent.stat({ bigint: true })) !== parentIdentity) throw new Error();
  };
  // Cleanup only an owned inode under the original admitted private directory.
  const cleanup = async () => {
    if (!stagePath || !stageIdentity) return;
    try {
      check(); await sameParent();
      const named = await lstat(stagePath, { bigint: true });
      if (!named.isFile() || named.isSymbolicLink() || directoryIdentity(named) !== stageIdentity) return;
      await unlink(stagePath); await parent?.sync();
    } catch { /* Retain an unprovable stage; never remove a replacement inode. */ }
  };
  try {
    callerSignal = options.signal; leaseSignal = fence.signal;
    check();
    if (!safeName(name) || !(input instanceof Uint8Array) || input.length > 2_000_000) return { state: "refused" };
    const expected = expectedDestination === null ? null : { device: expectedDestination.device, inode: expectedDestination.inode };
    if (expected !== null && !validIdentity(expected)) return { state: "refused" };
    bytes = new Uint8Array(input);
    const expectedHash = digest(bytes), length = bytes.length;
    const admitted = await admitTrustedDirectory(directoryPath, resources, "private-state", callerSignal);
    parent = admitted.handle; admittedPath = admitted.path; parentIdentity = admitted.identity;
    check(); await sameParent(); check();
    const stage = `.connect-stage-${randomBytes(16).toString("hex")}`;
    stagePath = path.join(admittedPath, stage);
    stageHandle = await open(stagePath, C.O_WRONLY | C.O_CREAT | C.O_EXCL | C.O_NOFOLLOW | C.O_NONBLOCK, 0o600);
    const created = await stageHandle.stat({ bigint: true }); stageIdentity = directoryIdentity(created);
    if (!created.isFile() || created.nlink !== 1n || created.uid !== BigInt(process.getuid!()) || (created.mode & 0o7777n) !== 0o600n) throw new Error();
    await stageHandle.writeFile(bytes); await stageHandle.sync();
    await stageHandle.close(); stageHandle = undefined;
    check(); await sameParent();
    const named = await lstat(stagePath, { bigint: true });
    if (!named.isFile() || named.isSymbolicLink() || directoryIdentity(named) !== stageIdentity) throw new Error();
    check();
    const signal = AbortSignal.any([leaseSignal, ...(callerSignal ? [callerSignal] : [])]);
    const transferred = parent; parent = undefined; phase = "dispatched";
    const result = await resources.replaceFile(transferred, stage, name, { source: fileIdentity(created), destination: expected }, { signal });
    if (result.state === "publication_uncertain") return { state: "publication_uncertain" };
    if (result.state === "refused" || result.state === "destination_changed") {
      // Native result guarantees no effect. Re-admit before cleaning the owned stage.
      const readmitted = await admitTrustedDirectory(admittedPath, resources, "private-state", callerSignal);
      parent = readmitted.handle;
      await sameParent(); await cleanup();
      return { state: result.state };
    }
    if (result.state !== "replaced") return { state: "publication_uncertain" };
    check(); await sameParent();
    const finalPath = path.join(admittedPath, name), handle = await open(finalPath, flags);
    let owns = true;
    try {
      const fd = await handle.stat({ bigint: true }), final = await lstat(finalPath, { bigint: true });
      if (!final.isFile() || final.isSymbolicLink() || directoryIdentity(fd) !== stageIdentity || directoryIdentity(final) !== stageIdentity) throw new Error();
      owns = false;
      const observed = await consumePrivateFile(handle, { maximumBytes: length, signal }, resources, value => ({ length: value.length, hash: digest(value) }));
      check(); await sameParent();
      const after = await lstat(finalPath, { bigint: true });
      if (observed.length !== length || observed.hash !== expectedHash || directoryIdentity(after) !== stageIdentity
        || after.size !== fd.size || after.mtimeNs !== fd.mtimeNs || after.ctimeNs !== fd.ctimeNs) throw new Error();
    } finally { if (owns) await handle.close().catch(() => {}); }
    return { state: "committed" };
  } catch {
    if (phase === "preparing") await cleanup();
    return { state: phase === "dispatched" ? "publication_uncertain" : "refused" };
  } finally {
    bytes?.fill(0);
    await stageHandle?.close().catch(() => {}); await parent?.close().catch(() => {});
  }
}
