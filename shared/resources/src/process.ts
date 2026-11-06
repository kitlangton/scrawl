import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { EventEmitter } from "node:events";
import { constants } from "node:fs";
import { open, lstat } from "node:fs/promises";
import type { FileHandle } from "node:fs/promises";
import { createHash } from "node:crypto";
import { dirname, isAbsolute } from "node:path";
import type { ResourceTarget } from "./artifact.ts";

export type ResourceFailure = "invalid_request" | "provider_unavailable" | "provider_mismatch" | "protocol_error" | "process_failed" | "timeout" | "ownership_transfer_failed" | "lease_lost" | "cancelled";
export class NativeResourceError extends Error {
  readonly category: ResourceFailure;
  constructor(category: ResourceFailure) { super(category); this.name = "NativeResourceError"; this.category = category; }
}
export type ResourceProvider = { executable: string; sha256: string; target: ResourceTarget; protocol: 1 };
export type Frame = "accepted" | "acquired" | "released" | "busy" | "cancelled" | "failed" | "published" | "destination_exists" | "refused" | "publication_uncertain";
const hostTarget = () => process.platform === "linux" && process.arch === "x64" ? "linux-x64"
  : process.platform === "darwin" && process.arch === "x64" ? "macos-x64"
  : process.platform === "darwin" && process.arch === "arm64" ? "macos-arm64" : undefined;

async function verify(provider: ResourceProvider): Promise<void> {
  if (provider.protocol !== 1 || provider.target !== hostTarget() || !isAbsolute(provider.executable) || !/^[a-f0-9]{64}$/.test(provider.sha256)) throw new NativeResourceError("provider_mismatch");
  let binary: FileHandle | undefined;
  try {
    const parent = await lstat(dirname(provider.executable));
    const owners = [0, process.geteuid!()];
    if (!parent.isDirectory() || parent.isSymbolicLink() || !owners.includes(parent.uid) || parent.mode & 0o022) throw new NativeResourceError("provider_mismatch");
    binary = await open(provider.executable, constants.O_RDONLY | constants.O_NOFOLLOW);
    const before = await binary.stat();
    if (!before.isFile() || !owners.includes(before.uid) || before.nlink !== 1 || before.mode & 0o022 || before.mode & 0o7000 || !(before.mode & 0o100) || before.size > 16 * 1024 * 1024) throw new NativeResourceError("provider_mismatch");
    const hash = createHash("sha256").update(await binary.readFile()).digest("hex"), after = await binary.stat();
    const path = await lstat(provider.executable);
    if (hash !== provider.sha256 || before.dev !== after.dev || before.ino !== after.ino || before.size !== after.size || before.mtimeMs !== after.mtimeMs
      || path.isSymbolicLink() || path.dev !== after.dev || path.ino !== after.ino) throw new NativeResourceError("provider_mismatch");
  } catch (error) {
    if (error instanceof NativeResourceError) throw error;
    throw new NativeResourceError("provider_unavailable");
  } finally {
    try { await binary?.close(); } catch { throw new NativeResourceError("provider_unavailable"); }
  }
}

export class ResourceProcess extends EventEmitter {
  readonly child: ChildProcessWithoutNullStreams;
  readonly finished: Promise<{ code: number | null; signal: NodeJS.Signals | null }>;
  #buffer = ""; #frames: Frame[] = []; #count = 0; #failure?: NativeResourceError; #closed = false;
  get running(): boolean { return !this.#closed && !this.#failure; }
  get pending(): boolean { return this.#frames.length > 0; }
  constructor(provider: ResourceProvider, args: string[], fd: number) {
    super();
    this.child = spawn(provider.executable, args, { stdio: ["pipe", "pipe", "pipe", fd] }) as ChildProcessWithoutNullStreams;
    this.child.stdin.on("error", () => this.fail("process_failed"));
    this.child.stdout.setEncoding("utf8");
    this.child.stdout.on("data", (chunk: string) => {
      this.#buffer += chunk;
      if (this.#buffer.length > 1024) { this.fail("protocol_error"); return; }
      let end: number;
      while ((end = this.#buffer.indexOf("\n")) >= 0) {
        const line = this.#buffer.slice(0, end); this.#buffer = this.#buffer.slice(end + 1);
        const match = /^\{"protocol":1,"state":"(accepted|acquired|released|busy|cancelled|failed|published|destination_exists|refused|publication_uncertain)"\}$/.exec(line);
        if (!match || ++this.#count > 2) { this.fail("protocol_error"); return; }
        this.#frames.push(match[1] as Frame); this.emit("frame");
      }
    });
    this.child.stderr.on("data", () => this.fail("protocol_error"));
    this.child.on("error", () => this.fail("process_failed"));
    this.finished = new Promise(resolve => this.child.once("close", (code, signal) => {
      this.#closed = true;
      if (this.#buffer) this.fail("protocol_error");
      resolve({ code, signal }); this.emit("frame"); this.emit("closed");
    }));
  }
  fail(category: ResourceFailure): void {
    if (this.#failure) return;
    this.#failure = new NativeResourceError(category); this.child.kill("SIGKILL"); this.emit("frame"); this.emit("fault");
  }
  async next(milliseconds: number): Promise<Frame> {
    return new Promise((resolve, reject) => {
      const done = () => {
        if (this.#failure) { cleanup(); reject(this.#failure); return; }
        if (this.#frames.length) { cleanup(); resolve(this.#frames.shift()!); return; }
        if (this.#closed) { cleanup(); reject(new NativeResourceError("process_failed")); }
      };
      const cleanup = () => { clearTimeout(timer); this.removeListener("frame", done); };
      const timer = setTimeout(() => { cleanup(); this.fail("timeout"); reject(new NativeResourceError("timeout")); }, milliseconds);
      this.on("frame", done); done();
    });
  }
  async exit(milliseconds = 2000): Promise<void> {
    let timer: NodeJS.Timeout | undefined;
    try {
      const result = await Promise.race([this.finished, new Promise<never>((_, reject) => { timer = setTimeout(() => { this.fail("timeout"); reject(new NativeResourceError("timeout")); }, milliseconds); })]);
      if (this.#failure) throw this.#failure;
      if (result.code !== 0 || result.signal || this.#frames.length) throw new NativeResourceError("protocol_error");
    } finally { clearTimeout(timer); }
  }
  stop(): void { this.child.stdin.end(); }
}

/** Invocation consumes the caller's FileHandle even when validation fails. The
 * duplicate must close before a lease is returned, or helper death cannot release flock. */
export async function launch(provider: ResourceProvider, args: string[], handle: FileHandle, signal?: AbortSignal): Promise<ResourceProcess> {
  let session: ResourceProcess | undefined;
  try {
    if (!Number.isInteger(handle.fd) || handle.fd < 0) throw new NativeResourceError("invalid_request");
    if (signal?.aborted) throw new NativeResourceError("cancelled");
    await verify(provider);
    if (signal?.aborted) throw new NativeResourceError("cancelled");
    session = new ResourceProcess(provider, args, handle.fd);
    return session;
  } catch (error) {
    if (error instanceof NativeResourceError) throw error;
    throw new NativeResourceError("process_failed");
  } finally {
    try { await handle.close(); }
    catch { session?.stop(); throw new NativeResourceError("ownership_transfer_failed"); }
  }
}
