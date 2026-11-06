import type { FileHandle } from "node:fs/promises";
import { launch, NativeResourceError, type ResourceProvider, type ResourceProcess } from "./process.ts";
export { NativeResourceError } from "./process.ts";
export type { ResourceProvider, ResourceFailure } from "./process.ts";
export { resourceManifest, validateResourceArtifact } from "./artifact.ts";
export type { ResourceArtifact, ResourceTarget } from "./artifact.ts";

export type LeaseState = "held" | "releasing" | "released" | "lost";
export class NativeLockLease {
  #session: ResourceProcess; #state: LeaseState = "held"; #release?: Promise<void>;
  #controller = new AbortController(); #caller?: AbortSignal;
  get state(): LeaseState { return this.#state; }
  /** Aborted when released or lost. Consumers must stop work when the lease ends. */
  get signal(): AbortSignal { return this.#controller.signal; }
  #cancel = () => { void this.release().catch(() => {}); };
  #closed = () => { if (this.#state === "held") this.#lost(); };
  #frame = () => { if (this.#state === "held") { this.#session.fail("protocol_error"); this.#lost(); } };
  constructor(session: ResourceProcess, caller?: AbortSignal) {
    this.#session = session; this.#caller = caller;
    session.on("closed", this.#closed); session.on("fault", this.#closed); session.on("frame", this.#frame);
    caller?.addEventListener("abort", this.#cancel, { once: true });
    if (!session.running || session.pending || caller?.aborted) { this.#lost(); session.stop(); }
  }
  assertHeld(): void { if (this.#state !== "held") throw new NativeResourceError("lease_lost"); }
  #cleanup(): void {
    this.#caller?.removeEventListener("abort", this.#cancel);
    this.#session.removeListener("closed", this.#closed); this.#session.removeListener("fault", this.#closed); this.#session.removeListener("frame", this.#frame);
  }
  #lost(): void { this.#state = "lost"; this.#controller.abort("lease_lost"); this.#cleanup(); }
  release(): Promise<void> {
    if (this.#release) return this.#release;
    if (this.#state === "released") return Promise.resolve();
    if (this.#state === "lost") return Promise.reject(new NativeResourceError("lease_lost"));
    this.#state = "releasing";
    this.#release = (async () => {
      try {
        this.#session.stop();
        if (await this.#session.next(2000) !== "released") throw new NativeResourceError("protocol_error");
        await this.#session.exit();
        this.#state = "released"; this.#controller.abort("released"); this.#cleanup();
      } catch (error) { this.#session.fail("process_failed"); this.#lost(); throw error; }
    })();
    return this.#release;
  }
}

export type LockResult = { state: "acquired"; lease: NativeLockLease } | { state: "busy" | "cancelled" | "failed" | "refused" };
export type PublicationResult = { state: "published" | "destination_exists" | "refused" | "publication_uncertain" };
export type FileIdentity = { device: string; inode: string };
export type ReplaceFileResult = { state: "replaced" | "destination_changed" | "refused" | "publication_uncertain" };
function identity(value: FileIdentity): boolean {
  const canonical = (x: unknown) => typeof x === "string" && /^(0|[1-9][0-9]{0,19})$/.test(x) && BigInt(x) <= 18446744073709551615n;
  return !!value && typeof value === "object" && canonical(value.device) && canonical(value.inode);
}
function basename(value: string): boolean {
  if (typeof value !== "string" || !value.length || value.length > 4096 || value === "." || value === ".." || value.includes("/") || value.includes("\0") || Buffer.byteLength(value, "utf8") > 4096) return false;
  for (const character of value) { const cp = character.codePointAt(0)!; if (cp >= 0xd800 && cp <= 0xdfff) return false; }
  return true;
}

/** Privileged resource adapter. Provider bytes/provenance must be admitted by
 * assembly. Never expose this object or inherited descriptors to a renderer. */
export class NativeResources {
  #provider: ResourceProvider;
  constructor(provider: ResourceProvider) { this.#provider = { ...provider }; }
  /** Consumes a file/directory descriptor. This only checks native ACL facts;
   * acceptance does not establish private modes, ancestor safety or authority.
   * Older helpers refuse the unknown operation, so callers fail closed. */
  async checkAcl(handle: FileHandle): Promise<{ state: "accepted" | "refused" }> {
    const session = await launch(this.#provider, ["acl"], handle);
    try {
      const state = await session.next(10_000);
      if (state !== "accepted" && state !== "refused") throw new NativeResourceError("protocol_error");
      await session.exit(); return { state };
    } catch (error) { session.fail("process_failed"); throw error; }
  }
  /** Consumes handle on every outcome. The caller must not retain duplicate FDs
   * and must validate path-to-inode identity and its own lock ordering. */
  async lock(handle: FileHandle, options: { timeoutMs: number; signal?: AbortSignal }): Promise<LockResult> {
    if (!Number.isInteger(options.timeoutMs) || options.timeoutMs < 0 || options.timeoutMs > 60_000) {
      await handle.close(); throw new NativeResourceError("invalid_request");
    }
    if (options.signal?.aborted) { await handle.close(); return { state: "cancelled" }; }
    let session: ResourceProcess;
    try { session = await launch(this.#provider, ["lock", "--timeout-ms", String(options.timeoutMs)], handle, options.signal); }
    catch (error) { if (error instanceof NativeResourceError && error.category === "cancelled") return { state: "cancelled" }; throw error; }
    const cancel = () => session.stop();
    options.signal?.addEventListener("abort", cancel, { once: true });
    try {
      if (options.signal?.aborted) cancel();
      const state = await session.next(options.timeoutMs + 3000);
      if (state === "acquired") {
        if (options.signal?.aborted) {
          session.stop();
          if (await session.next(2000) !== "released") throw new NativeResourceError("protocol_error");
          await session.exit(); return { state: "cancelled" };
        }
        const lease = new NativeLockLease(session, options.signal);
        if (lease.state !== "held") throw new NativeResourceError("lease_lost");
        return { state, lease };
      }
      if (!["busy", "cancelled", "failed", "refused"].includes(state)) throw new NativeResourceError("protocol_error");
      await session.exit(); return { state: state as "busy" | "cancelled" | "failed" | "refused" };
    } catch (error) { session.fail("process_failed"); throw error; }
    finally { options.signal?.removeEventListener("abort", cancel); }
  }
  /** Consumes parent. Caller stages and syncs contents first. Once dispatched,
   * interruption/unknown completion means uncertainty: never retry or delete. */
  async publish(parent: FileHandle, source: string, destination: string, options: { timeoutMs?: number; signal?: AbortSignal } = {}): Promise<PublicationResult> {
    const timeout = options.timeoutMs ?? 10_000;
    if (!basename(source) || !basename(destination) || source === destination || !Number.isInteger(timeout) || timeout < 1 || timeout > 60_000) {
      await parent.close(); return { state: "refused" };
    }
    if (options.signal?.aborted) { await parent.close(); return { state: "refused" }; }
    let session: ResourceProcess;
    try { session = await launch(this.#provider, ["publish", "--source", source, "--destination", destination], parent, options.signal); }
    catch (error) {
      if (error instanceof NativeResourceError && error.category === "cancelled") return { state: "refused" };
      if (error instanceof NativeResourceError && error.category === "ownership_transfer_failed") return { state: "publication_uncertain" };
      throw error;
    }
    const cancel = () => session.fail("process_failed");
    options.signal?.addEventListener("abort", cancel, { once: true });
    try {
      if (options.signal?.aborted) cancel();
      const state = await session.next(timeout);
      if (!["published", "destination_exists", "refused", "publication_uncertain"].includes(state)) throw new NativeResourceError("protocol_error");
      await session.exit(); return { state: state as PublicationResult["state"] };
    } catch { session.fail("process_failed"); return { state: "publication_uncertain" }; }
    finally { options.signal?.removeEventListener("abort", cancel); }
  }
  /** Consumes parent on every outcome. Replacement is atomic under the caller's
   * cooperative lock; expected identities are admission checks, not a same-UID CAS. */
  async replaceFile(parent: FileHandle, source: string, destination: string, expected: { source: FileIdentity; destination: FileIdentity | null }, options: { timeoutMs?: number; signal?: AbortSignal } = {}): Promise<ReplaceFileResult> {
    let timeout: number, signal: AbortSignal | undefined, args: string[];
    try {
      timeout = options.timeoutMs ?? 10_000; signal = options.signal;
      const sourceIdentity = expected.source, destinationIdentity = expected.destination;
      const src = { device: sourceIdentity.device, inode: sourceIdentity.inode };
      const dst = destinationIdentity === null ? null : { device: destinationIdentity.device, inode: destinationIdentity.inode };
      if (!basename(source) || !basename(destination) || source === destination || !identity(src) || dst !== null && !identity(dst)
        || !Number.isInteger(timeout) || timeout < 1 || timeout > 60_000 || signal !== undefined && !(signal instanceof AbortSignal)) throw new Error("invalid_request");
      args = ["replace", "--source", source, "--destination", destination, "--source-device", src.device, "--source-inode", src.inode,
        "--destination-present", dst === null ? "0" : "1", "--destination-device", dst?.device ?? "0", "--destination-inode", dst?.inode ?? "0"];
    } catch {
      await parent.close().catch(() => {}); return { state: "refused" };
    }
    if (signal?.aborted) { await parent.close(); return { state: "refused" }; }
    let session: ResourceProcess;
    try {
      session = await launch(this.#provider, args, parent, signal);
    } catch (error) {
      if (error instanceof NativeResourceError && error.category === "cancelled") return { state: "refused" };
      if (error instanceof NativeResourceError && error.category === "ownership_transfer_failed") return { state: "publication_uncertain" };
      throw error;
    }
    const cancel = () => session.fail("process_failed");
    signal?.addEventListener("abort", cancel, { once: true });
    try {
      if (signal?.aborted) cancel();
      const state = await session.next(timeout);
      if (!["replaced", "destination_changed", "refused", "publication_uncertain"].includes(state)) throw new NativeResourceError("protocol_error");
      await session.exit(); return { state: state as ReplaceFileResult["state"] };
    } catch { session.fail("process_failed"); return { state: "publication_uncertain" }; }
    finally { signal?.removeEventListener("abort", cancel); }
  }
}
