import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { randomBytes } from "node:crypto";
import { dirname, isAbsolute, resolve } from "node:path";

export const RECOVERY_STARTUP_MS = 4000;
export const RECOVERY_RELEASE_MS = 2000;
const LIFECYCLE_LINE_BYTES = 4096;
export interface RecoveryPeerFacts {
  schema: "apparent.engine-recovery-peer/1";
  peerId: string;
  channel: "capture/2" | "media/2";
  implementation: "native";
  role: "recovery";
  startsFreshCapture: false;
  onInputEof: "retain-admitted-finite-work";
}
export interface RecoveryPeerRelease {
  scope: "client-connection";
  peerId: string;
  connection: "released";
  inputClosure: "confirmed" | "unconfirmed" | "peer-exit-observed";
  runtimeClearance: "not-established";
  reconcile: "original-capture-materials-job-publication-bindings";
}
export interface RecoveryPeer<Client> {
  readonly client: Client;
  readonly lifetime: Readonly<RecoveryPeerFacts>;
  release(): Promise<RecoveryPeerRelease>;
}
export class RecoveryLaunchError extends Error {
  readonly code = "unsupported-recovery-lifetime";
  readonly dispatched = false;
  readonly connection = "released";
  readonly nativeExitObserved: boolean;
  constructor(nativeExitObserved: boolean) {
    super("Recovery lifetime negotiation failed; no operation was dispatched");
    this.name = "RecoveryLaunchError";
    this.nativeExitObserved = nativeExitObserved;
  }
}

/** Internal process adapter. Public callers never receive child handles. */
class RecoveryConnection {
  private readonly child: ChildProcessWithoutNullStreams;
  readonly facts: Readonly<RecoveryPeerFacts>;
  private bytes = Buffer.alloc(0);
  private discardLine = false;
  private readyResolve!: () => void;
  private readyReject!: (e: Error) => void;
  private readonly readyPromise: Promise<void>;
  private readyTimer: NodeJS.Timeout;
  private releasePromise?: Promise<RecoveryPeerRelease>;
  private releaseChanged?: () => void;
  private inputClosed = false;
  private exited = false;
  private ready = false;
  private disconnectLocal?: () => void;
  constructor(child: ChildProcessWithoutNullStreams, channel: RecoveryPeerFacts["channel"], peerId: string) {
    this.child = child;
    this.facts = Object.freeze({ schema: "apparent.engine-recovery-peer/1", peerId, channel, implementation: "native", role: "recovery", startsFreshCapture: false, onInputEof: "retain-admitted-finite-work" });
    this.readyPromise = new Promise((done, reject) => { this.readyResolve = done; this.readyReject = reject; });
    this.readyTimer = setTimeout(() => this.readyReject(new Error("Recovery startup timed out")), RECOVERY_STARTUP_MS);
    child.stderr.on("data", this.onStderr);
    child.stderr.on("error", this.onFailure);
    child.stdin.on("error", this.onFailure);
    child.on("error", this.onFailure);
    child.on("exit", this.onExit);
  }
  private onFailure = () => {
    if (!this.ready) this.readyReject(new Error("Recovery startup failed"));
  };
  private onExit = () => {
    this.exited = true;
    if (!this.ready) this.readyReject(new Error("Recovery peer exited before negotiation"));
    this.releaseChanged?.();
  };
  private onStderr = (chunk: Buffer) => {
    // Never retain an unbounded log chunk. Native lifecycle records are <=4KiB.
    let offset = 0;
    while (offset < chunk.length) {
      const newline = chunk.indexOf(10, offset);
      const end = newline < 0 ? chunk.length : newline;
      const fragment = chunk.subarray(offset, end);
      if (!this.discardLine && this.bytes.length + fragment.length > LIFECYCLE_LINE_BYTES) {
        this.onFailure(); this.discardLine = true; this.bytes = Buffer.alloc(0);
      }
      if (!this.discardLine) this.bytes = Buffer.concat([this.bytes, fragment]);
      offset = newline < 0 ? chunk.length : newline + 1;
      if (newline < 0) break;
      const line = this.bytes;
      const discard = this.discardLine;
      this.bytes = Buffer.alloc(0); this.discardLine = false;
      if (discard) continue;
      let value: unknown;
      try { value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(line)); } catch { continue; }
      if (!value || typeof value !== "object" || Array.isArray(value)) continue;
      const record = value as Record<string, unknown>;
      if (record.schema !== this.facts.schema) continue;
      const expected = { ...this.facts, event: record.event };
      if (Object.keys(record).length !== Object.keys(expected).length || Object.entries(expected).some(([key, v]) => record[key] !== v) || !["ready", "input-closed"].includes(String(record.event))) {
        this.onFailure(); continue;
      }
      if (record.event === "ready") { this.ready = true; this.readyResolve(); }
      else if (this.ready) { this.inputClosed = true; this.releaseChanged?.(); }
    }
  };
  async negotiate(): Promise<void> {
    try { await this.readyPromise; }
    finally { clearTimeout(this.readyTimer); }
  }
  bindLocalRelease(callback: () => void): void { this.disconnectLocal = callback; }
  private releaseHandles(): void {
    clearTimeout(this.readyTimer);
    this.child.stderr.off("data", this.onStderr);
    this.child.stderr.off("error", this.onFailure);
    this.child.stdin.off("error", this.onFailure);
    this.child.off("error", this.onFailure);
    this.child.off("exit", this.onExit);
    // Native Ready is the safety prerequisite, not unref itself. These only
    // release Node's local references after native lifetime responsibility moved.
    for (const stream of [this.child.stdin, this.child.stdout, this.child.stderr]) {
      stream.on("error", () => {});
      stream.destroy();
    }
    this.child.on("error", () => {});
    this.child.unref();
    this.bytes = Buffer.alloc(0);
  }
  release(): Promise<RecoveryPeerRelease> {
    if (this.releasePromise) return this.releasePromise;
    this.disconnectLocal?.(); // Freeze new RPCs and clear pending timers first.
    this.releasePromise = new Promise(done => {
      let settled = false;
      const settle = () => {
        if (settled) return;
        settled = true;
        clearTimeout(timer);
        this.releaseChanged = undefined;
        const inputClosure = this.exited ? "peer-exit-observed" : this.inputClosed ? "confirmed" : "unconfirmed";
        this.releaseHandles();
        done(Object.freeze({ scope: "client-connection", peerId: this.facts.peerId, connection: "released", inputClosure, runtimeClearance: "not-established", reconcile: "original-capture-materials-job-publication-bindings" }));
      };
      const timer = setTimeout(settle, RECOVERY_RELEASE_MS);
      this.releaseChanged = () => { if (this.exited || this.inputClosed) settle(); };
      if (this.exited || this.inputClosed) settle();
      else this.child.stdin.end();
    });
    return this.releasePromise;
  }
  async abortNeverExposed(): Promise<never> {
    // No RPC transport/client was exposed and no request was sent. Signals are
    // confined to this fresh failed launch; never an earlier returned peer.
    if (!this.exited) {
      await new Promise<void>(done => {
        const kill = setTimeout(() => this.child.kill("SIGKILL"), 500);
        const timer = setTimeout(done, 1000);
        this.child.once("exit", () => { clearTimeout(kill); clearTimeout(timer); done(); });
        this.child.kill("SIGTERM");
      });
    }
    this.releaseHandles();
    throw new RecoveryLaunchError(this.exited);
  }
}

/** Internal injection point for source-level process tests, never test-native
 * evidence. Factories below supply fixed native argv without a shell. */
export async function connectRecoveryPeer<Client>(executable: string, args: string[], channel: RecoveryPeerFacts["channel"], makeClient: (child: ChildProcessWithoutNullStreams) => { client: Client; releaseLocal: () => void }): Promise<RecoveryPeer<Client>> {
  if (!isAbsolute(executable) || resolve(executable) !== executable) throw new Error("Recovery executable must be a normalized absolute path");
  const peerId = randomBytes(16).toString("hex");
  const child = spawn(executable, [...args, "--recovery-peer-v1", peerId], { cwd: dirname(executable), stdio: ["pipe", "pipe", "pipe"], shell: false });
  const connection = new RecoveryConnection(child, channel, peerId);
  try {
    await connection.negotiate();
    const { client, releaseLocal } = makeClient(child);
    connection.bindLocalRelease(releaseLocal);
    return Object.freeze({ client, lifetime: connection.facts, release: () => connection.release() });
  } catch { return connection.abortNeverExposed(); }
}
