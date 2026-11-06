import { createHash } from "node:crypto";
import { constants } from "node:fs";
import { chmod, lstat, mkdtemp, open, readdir, realpath, rm, writeFile } from "node:fs/promises";
import type { FileHandle } from "node:fs/promises";
import { tmpdir } from "node:os";
import { isAbsolute, join, normalize } from "node:path";
import { NativeResourceError, type ResourceProvider } from "./process.ts";
import type { NativeResources, NativeLockLease, LockResult, PublicationResult } from "./index.ts";
import { resourceManifest, validateResourceArtifact } from "./artifact.ts";

const EXECUTABLE = "apparent-resource-host";
const MAX_HELPER_BYTES = 16 * 1024 * 1024;
const MAX_MANIFEST_BYTES = 4096;
const REGISTERED_TARGETS = ["linux-x64", "macos-x64"] as const;

export type InstalledResourceTarget = typeof REGISTERED_TARGETS[number];
export interface InstalledNativeResourceOptions {
  /** Canonical artifact directory admitted by a trusted external producer. */
  artifactDirectory: string;
  /** Digest from the external release/producer receipt, not the manifest itself. */
  trustedManifestSha256: string;
  expectedSourceSha: string;
  target: InstalledResourceTarget;
}

export interface InstalledNativeResources extends NativeResources {
  readonly executable: string;
  readonly sha256: string;
  readonly manifestSha256: string;
  readonly sourceSha: string;
  readonly target: InstalledResourceTarget;
  readonly protocol: 1;
  close(): Promise<void>;
}

interface Snapshot {
  readonly sourceSha: string;
  readonly target: InstalledResourceTarget;
  readonly manifest: Buffer;
  readonly executable: Buffer;
  readonly manifestSha256: string;
  readonly sha256: string;
}

function hostTarget(): InstalledResourceTarget | undefined {
  if (process.platform === "linux" && process.arch === "x64") return "linux-x64";
  if (process.platform === "darwin" && process.arch === "x64") return "macos-x64";
  return undefined;
}

function providerMismatch(): NativeResourceError { return new NativeResourceError("provider_mismatch"); }
function unavailable(): NativeResourceError { return new NativeResourceError("provider_unavailable"); }

async function readStableFile(path: string, maximum: number, requiredMode?: number): Promise<Buffer> {
  let handle: FileHandle | undefined;
  try {
    handle = await open(path, constants.O_RDONLY | constants.O_NOFOLLOW | constants.O_NONBLOCK);
    const before = await handle.stat();
    const owners = [0, process.geteuid?.()].filter((value): value is number => value !== undefined);
    if (!before.isFile() || !owners.includes(before.uid) || before.nlink !== 1 || before.size < 1 || before.size > maximum
      || before.mode & 0o022 || before.mode & 0o7000
      || (requiredMode !== undefined && (before.mode & 0o7777) !== requiredMode)) throw providerMismatch();
    const bytes = Buffer.alloc(before.size + 1);
    let length = 0;
    while (length < bytes.length) {
      const result = await handle.read(bytes, length, bytes.length - length, length);
      if (!result.bytesRead) break;
      length += result.bytesRead;
    }
    const after = await handle.stat();
    const pathStat = await lstat(path);
    if (length !== before.size || before.dev !== after.dev || before.ino !== after.ino || before.size !== after.size
      || before.mtimeMs !== after.mtimeMs || pathStat.isSymbolicLink() || pathStat.dev !== after.dev || pathStat.ino !== after.ino) {
      throw providerMismatch();
    }
    return bytes.subarray(0, length);
  } catch (error) {
    if (error instanceof NativeResourceError) throw error;
    throw providerMismatch();
  } finally {
    try { await handle?.close(); } catch { throw new NativeResourceError("ownership_transfer_failed"); }
  }
}

async function snapshotArtifact(options: InstalledNativeResourceOptions): Promise<Snapshot> {
  if (!options || typeof options.artifactDirectory !== "string" || !isAbsolute(options.artifactDirectory) || normalize(options.artifactDirectory) !== options.artifactDirectory
    || !/^[a-f0-9]{64}$/.test(options.trustedManifestSha256) || !/^[a-f0-9]{40}$/.test(options.expectedSourceSha)
    || !REGISTERED_TARGETS.includes(options.target) || options.target !== hostTarget()) throw providerMismatch();
  let directory: FileHandle | undefined;
  try {
    if (await realpath(options.artifactDirectory) !== options.artifactDirectory) throw providerMismatch();
    directory = await open(options.artifactDirectory, constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW);
    const before = await directory.stat();
    const owners = [0, process.geteuid?.()].filter((value): value is number => value !== undefined);
    if (!before.isDirectory() || !owners.includes(before.uid) || before.mode & 0o022 || before.mode & 0o7000) throw providerMismatch();
    const names = (await readdir(options.artifactDirectory)).sort();
    if (JSON.stringify(names) !== JSON.stringify([EXECUTABLE, "manifest.json"])) throw providerMismatch();
    const manifestPath = join(options.artifactDirectory, "manifest.json");
    const executablePath = join(options.artifactDirectory, EXECUTABLE);
    const [manifest, executable] = await Promise.all([
      readStableFile(manifestPath, MAX_MANIFEST_BYTES),
      readStableFile(executablePath, MAX_HELPER_BYTES, 0o755),
    ]);
    const after = await directory.stat();
    const current = await lstat(options.artifactDirectory);
    if (before.dev !== after.dev || before.ino !== after.ino || current.isSymbolicLink()
      || current.dev !== after.dev || current.ino !== after.ino
      || JSON.stringify((await readdir(options.artifactDirectory)).sort()) !== JSON.stringify(names)) throw providerMismatch();
    const sha256 = createHash("sha256").update(executable).digest("hex");
    const manifestHash = createHash("sha256").update(manifest).digest("hex");
    if (manifestHash !== options.trustedManifestSha256) throw providerMismatch();
    let parsed: unknown;
    try { parsed = JSON.parse(manifest.toString("utf8")); } catch { throw providerMismatch(); }
    const expected = resourceManifest(options.expectedSourceSha, options.target, executable);
    if (manifest.toString("utf8") !== JSON.stringify(expected, null, 2) + "\n"
      || !parsed || typeof parsed !== "object" || (parsed as Record<string, unknown>).sourceSha !== options.expectedSourceSha
      || (parsed as Record<string, unknown>).target !== options.target || (parsed as Record<string, unknown>).protocol !== 1) {
      throw providerMismatch();
    }
    return { sourceSha: options.expectedSourceSha, target: options.target, manifest, executable, manifestSha256: manifestHash, sha256 };
  } catch (error) {
    if (error instanceof NativeResourceError) throw error;
    throw providerMismatch();
  } finally {
    try { await directory?.close(); } catch { throw new NativeResourceError("ownership_transfer_failed"); }
  }
}

type Phase = "open" | "retiring" | "closed";

function ownedResourcesClass(Base: typeof NativeResources) {
  return class OwnedNativeResources extends Base implements InstalledNativeResources {
    readonly executable: string;
    readonly sha256: string;
    readonly manifestSha256: string;
    readonly sourceSha: string;
    readonly target: InstalledResourceTarget;
    readonly protocol = 1 as const;
    readonly #directory: string;
    readonly #identity: { dev: number; ino: number };
    #phase: Phase = "open";
    #pending = 0;
    readonly #leases = new Set<NativeLockLease>();

    constructor(input: { directory: string; identity: { dev: number; ino: number }; sha256: string; manifestSha256: string; sourceSha: string; target: InstalledResourceTarget }) {
      const executable = join(input.directory, EXECUTABLE);
      super({ executable, sha256: input.sha256, target: input.target, protocol: 1 } satisfies ResourceProvider);
      this.executable = executable;
      this.sha256 = input.sha256;
      this.manifestSha256 = input.manifestSha256;
      this.sourceSha = input.sourceSha;
      this.target = input.target;
      this.#directory = input.directory;
      this.#identity = input.identity;
      Object.freeze(this);
    }

    async #run<T>(handle: FileHandle, operation: () => Promise<T>): Promise<T> {
      if (this.#phase !== "open") {
        try { await handle.close(); } catch { throw new NativeResourceError("ownership_transfer_failed"); }
        throw unavailable();
      }
      // Increment synchronously before dispatch so close cannot pass between admission and spawn.
      this.#pending++;
      try { return await operation(); }
      finally {
        try {
          if (handle.fd >= 0) await handle.close();
        } catch { throw new NativeResourceError("ownership_transfer_failed"); }
        finally { this.#pending--; }
      }
    }

    override checkAcl(handle: FileHandle): Promise<{ state: "accepted" | "refused" }> {
      return this.#run(handle, () => super.checkAcl(handle));
    }

    override replaceFile(...args: Parameters<NativeResources["replaceFile"]>): ReturnType<NativeResources["replaceFile"]> {
      return this.#run(args[0], () => super.replaceFile(...args));
    }

    override lock(handle: FileHandle, options: { timeoutMs: number; signal?: AbortSignal }): Promise<LockResult> {
      return this.#run(handle, async () => {
        const result = await super.lock(handle, options);
        if (result.state === "acquired") {
          const lease = result.lease;
          if (lease.state === "held") {
            this.#leases.add(lease);
            lease.signal.addEventListener("abort", () => this.#leases.delete(lease), { once: true });
            if (lease.signal.aborted) this.#leases.delete(lease);
          }
        }
        return result;
      });
    }

    override publish(parent: FileHandle, source: string, destination: string,
      options: { timeoutMs?: number; signal?: AbortSignal } = {}): Promise<PublicationResult> {
      return this.#run(parent, () => super.publish(parent, source, destination, options));
    }

    async close(): Promise<void> {
      if (this.#phase === "closed") return;
      if (this.#phase === "open" && (this.#pending > 0 || this.#leases.size > 0)) {
        throw new NativeResourceError("invalid_request");
      }
      this.#phase = "retiring";
      try {
        const current = await lstat(this.#directory);
        if (!current.isDirectory() || current.isSymbolicLink() || current.dev !== this.#identity.dev || current.ino !== this.#identity.ino) {
          throw providerMismatch();
        }
        await rm(this.#directory, { recursive: true, force: false });
        this.#phase = "closed";
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code === "ENOENT") {
          const remaining = await lstat(this.#directory).catch(checkError => {
            if ((checkError as NodeJS.ErrnoException).code === "ENOENT") return undefined;
            throw checkError;
          });
          if (!remaining) { this.#phase = "closed"; return; }
        }
        throw error instanceof NativeResourceError ? error : unavailable();
      }
    }
  };
}

async function removeOwnedDirectory(directory: string, identity: { dev: number; ino: number }): Promise<void> {
  const current = await lstat(directory).catch(error => {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return undefined;
    throw error;
  });
  if (!current) return;
  if (!current.isDirectory() || current.isSymbolicLink() || current.dev !== identity.dev || current.ino !== identity.ino) throw providerMismatch();
  await rm(directory, { recursive: true, force: false });
}

/** Opens a trusted, externally admitted artifact as a process-safe private copy.
 * The returned executable identity is for privileged host composition only. */
export async function createInstalledNativeResources(options: InstalledNativeResourceOptions): Promise<InstalledNativeResources> {
  const admitted = options && Object.freeze({ artifactDirectory: options.artifactDirectory,
    trustedManifestSha256: options.trustedManifestSha256, expectedSourceSha: options.expectedSourceSha, target: options.target });
  const snapshot = await snapshotArtifact(admitted);
  let directory: string | undefined;
  let identity: { dev: number; ino: number } | undefined;
  try {
    directory = await mkdtemp(join(await realpath(tmpdir()), "apparent-resource-host-"));
    const created = await lstat(directory);
    identity = { dev: created.dev, ino: created.ino };
    if (await realpath(directory) !== directory) throw providerMismatch();
    await chmod(directory, 0o700);
    const directoryStat = await lstat(directory);
    if (!directoryStat.isDirectory() || directoryStat.uid !== process.geteuid?.() || directoryStat.mode & 0o077) throw providerMismatch();
    const helper = await open(join(directory, EXECUTABLE), "wx", 0o700);
    try { await helper.writeFile(snapshot.executable); await helper.sync(); }
    finally { await helper.close(); }
    await chmod(join(directory, EXECUTABLE), 0o755);
    await writeFile(join(directory, "manifest.json"), snapshot.manifest, { flag: "wx", mode: 0o600 });
    const manifest = await open(join(directory, "manifest.json"), constants.O_RDONLY | constants.O_NOFOLLOW);
    try { await manifest.sync(); } finally { await manifest.close(); }
    const parent = await open(directory, constants.O_RDONLY | constants.O_DIRECTORY | constants.O_NOFOLLOW);
    try { await parent.sync(); } finally { await parent.close(); }
    const validated = validateResourceArtifact(directory, snapshot.target);
    if (validated.sourceSha !== snapshot.sourceSha || validated.sha256 !== snapshot.sha256 || validated.protocol !== 1) throw providerMismatch();
    // Resolve after this module's dependency graph has initialized; index.ts may
    // re-export this public adapter without a runtime inheritance cycle.
    const { NativeResources } = await import("./index.ts");
    const OwnedNativeResources = ownedResourcesClass(NativeResources);
    return new OwnedNativeResources({ directory, identity, sha256: snapshot.sha256, manifestSha256: snapshot.manifestSha256,
      sourceSha: snapshot.sourceSha, target: snapshot.target });
  } catch (error) {
    if (directory && !identity) {
      throw new AggregateError([error, new Error("Staging directory identity could not be verified; it was left untouched")],
        "Installed resource setup failed before safe staging cleanup was possible");
    }
    if (directory && identity) {
      try { await removeOwnedDirectory(directory, identity); }
      catch (cleanupError) { throw new AggregateError([error, cleanupError], "Installed resource setup failed and its owned staging directory could not be retired"); }
    }
    throw error;
  }
}
