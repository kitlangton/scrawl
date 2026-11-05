import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { randomBytes } from "node:crypto";
import { dirname, isAbsolute, resolve } from "node:path";

export type State =
  | "starting"
  | "recording"
  | "stopping"
  | "finalizing"
  | "saved"
  | "cancelled"
  | "failed"
  | "rejected";
export type ErrorCode =
  | "malformed_request"
  | "unsupported_version"
  | "identity_conflict"
  | "not_ready"
  | "not_found"
  | "busy"
  | "permission_denied"
  | "unavailable"
  | "internal";
export interface Handshake {
  protocolVersion: 1;
  implementation: "native" | "test-double";
  capabilities: {
    screenCapture: boolean;
    audioCapture: boolean;
    windowCapture: boolean;
  };
  readiness: "ready" | "unavailable";
}
export interface Receipt {
  protocolVersion: 1;
  rpcId: string;
  captureId: string;
  state: State;
  bundlePath?: string;
  artifactVersion?: 1;
  artifactProvenance?: "native" | "test-synthetic";
  error?: { code: ErrorCode; message: string };
}
interface BaseRequest {
  protocolVersion: 1;
  clientId: string;
  rpcId: string;
}
export interface StartRequest extends BaseRequest {
  method: "start";
  ownerToken: string;
  captureId: string;
  bundleId: string;
  bundlePath: string;
  options: { screen: true };
}
export interface HandshakeRequest extends BaseRequest {
  method: "handshake";
}
export interface ControlRequest extends BaseRequest {
  method: "inspect" | "stop" | "cancel";
  ownerToken: string;
  captureId: string;
}
export type Request = StartRequest | HandshakeRequest | ControlRequest;
export interface Response {
  protocolVersion: 1;
  rpcId: string;
  result?: Handshake | Receipt;
  error?: { code: ErrorCode; message: string };
}
export interface StartResult {
  receipt: Receipt;
  captureId: string;
  ownerToken: string;
}
const MAX_LINE = 64 * 1024;
const MAX_TIMEOUT = 60_000;
const states = new Set<State>([
  "starting",
  "recording",
  "stopping",
  "finalizing",
  "saved",
  "cancelled",
  "failed",
  "rejected",
]);
const codes = new Set<ErrorCode>([
  "malformed_request",
  "unsupported_version",
  "identity_conflict",
  "not_ready",
  "not_found",
  "busy",
  "permission_denied",
  "unavailable",
  "internal",
]);
const uuid = () => randomBytes(16).toString("hex");
export class EngineRpcError extends Error {
  readonly code: ErrorCode;
  constructor(code: ErrorCode, message: string) {
    super(message);
    this.code = code;
    this.name = "EngineRpcError";
  }
}
function obj(v: unknown): Record<string, unknown> {
  if (!v || typeof v !== "object" || Array.isArray(v))
    throw new Error("invalid protocol object");
  return v as Record<string, unknown>;
}
function fields(
  x: Record<string, unknown>,
  allowed: string[],
  required: string[] = allowed,
) {
  if (
    Object.keys(x).some((k) => !allowed.includes(k)) ||
    required.some((k) => !(k in x))
  )
    throw new Error("invalid protocol fields");
}
function str(v: unknown, n = 4096): v is string {
  return typeof v === "string" && v.length > 0 && v.length <= n;
}
function bundleId(v: unknown): v is string {
  return (
    typeof v === "string" &&
    v.length > 0 &&
    Buffer.byteLength(v, "utf8") <= 255 &&
    v !== "." &&
    v !== ".." &&
    !/[\\/\p{Cc}]/u.test(v)
  );
}
function hex(v: unknown, n: number): v is string {
  return typeof v === "string" && new RegExp(`^[0-9a-f]{${n}}$`).test(v);
}
function timeout(v: number) {
  if (!Number.isSafeInteger(v) || v < 1 || v > MAX_TIMEOUT)
    throw new RangeError(`timeout must be 1..${MAX_TIMEOUT} ms`);
}
function absoluteBundlePath(v: unknown): v is string {
  return (
    typeof v === "string" &&
    isAbsolute(v) &&
    resolve(v) === v &&
    v.length <= 4096 &&
    !v.includes("\0")
  );
}
function validateRequest(v: unknown): Request {
  const x = obj(v);
  if (x.protocolVersion !== 1) throw new Error("unsupported protocol version");
  if (
    !["handshake", "start", "inspect", "stop", "cancel"].includes(
      String(x.method),
    )
  )
    throw new Error("unknown protocol method");
  const method = x.method;
  fields(
    x,
    method === "handshake"
      ? ["protocolVersion", "clientId", "rpcId", "method"]
      : method === "start"
        ? [
            "protocolVersion",
            "clientId",
            "rpcId",
            "method",
            "ownerToken",
            "captureId",
            "bundleId",
            "bundlePath",
            "options",
          ]
        : [
            "protocolVersion",
            "clientId",
            "rpcId",
            "method",
            "ownerToken",
            "captureId",
          ],
  );
  if (!hex(x.clientId, 32) || !hex(x.rpcId, 32))
    throw new Error("invalid RPC identity");
  if (method === "handshake") return x as unknown as HandshakeRequest;
  if (!hex(x.captureId, 32) || !hex(x.ownerToken, 64))
    throw new Error("invalid capture identity or authority");
  if (method === "start") {
    const o = obj(x.options);
    fields(o, ["screen"]);
    if (
      o.screen !== true ||
      !bundleId(x.bundleId) ||
      !absoluteBundlePath(x.bundlePath)
    )
      throw new Error("invalid start parameters");
  }
  return x as unknown as Request;
}
export function validateHandshake(value: unknown): Handshake {
  const x = obj(value);
  fields(x, ["protocolVersion", "implementation", "capabilities", "readiness"]);
  const c = obj(x.capabilities);
  fields(c, ["screenCapture", "audioCapture", "windowCapture"]);
  if (
    x.protocolVersion !== 1 ||
    !["native", "test-double"].includes(String(x.implementation)) ||
    !["ready", "unavailable"].includes(String(x.readiness)) ||
    ![c.screenCapture, c.audioCapture, c.windowCapture].every(
      (v) => typeof v === "boolean",
    )
  )
    throw new Error("invalid handshake");
  if (
    x.implementation === "test-double" &&
    (c.screenCapture || x.readiness === "ready")
  )
    throw new Error("test double cannot claim native readiness");
  return x as unknown as Handshake;
}
export function validateReceipt(
  value: unknown,
  expectedRpcId?: string,
  expectedCaptureId?: string,
): Receipt {
  const x = obj(value);
  fields(
    x,
    [
      "protocolVersion",
      "rpcId",
      "captureId",
      "state",
      "bundlePath",
      "artifactVersion",
      "artifactProvenance",
      "error",
    ],
    ["protocolVersion", "rpcId", "captureId", "state"],
  );
  if (
    x.protocolVersion !== 1 ||
    !hex(x.rpcId, 32) ||
    (expectedRpcId && x.rpcId !== expectedRpcId) ||
    !hex(x.captureId, 32) ||
    (expectedCaptureId && x.captureId !== expectedCaptureId) ||
    !states.has(x.state as State)
  )
    throw new Error("invalid receipt identity or state");
  if (
    x.state === "saved" &&
    (!absoluteBundlePath(x.bundlePath) ||
      x.artifactVersion !== 1 ||
      !["native", "test-synthetic"].includes(String(x.artifactProvenance)) ||
      x.error !== undefined)
  )
    throw new Error("invalid saved receipt");
  if (
    x.state !== "saved" &&
    (x.bundlePath !== undefined ||
      x.artifactVersion !== undefined ||
      x.artifactProvenance !== undefined)
  )
    throw new Error("non-saved receipt has artifact");
  if (x.state === "failed" || x.state === "rejected") {
    const e = obj(x.error);
    fields(e, ["code", "message"]);
    if (!codes.has(e.code as ErrorCode) || !str(e.message, 512))
      throw new Error("invalid terminal error");
    return {
      ...x,
      error: {
        code: e.code as ErrorCode,
        message: `Engine operation failed (${e.code})`,
      },
    } as unknown as Receipt;
  } else if (x.error !== undefined) throw new Error("unexpected receipt error");
  return x as unknown as Receipt;
}
export function encodeRequest(request: Request): string {
  const x = validateRequest(request);
  const line = JSON.stringify(x);
  if (Buffer.byteLength(line) > MAX_LINE)
    throw new Error("request exceeds protocol limit");
  return line + "\n";
}
function response(value: unknown, rpcId: string): Response {
  const x = obj(value);
  fields(
    x,
    ["protocolVersion", "rpcId", "result", "error"],
    ["protocolVersion", "rpcId"],
  );
  if (
    x.protocolVersion !== 1 ||
    x.rpcId !== rpcId ||
    (x.result === undefined) === (x.error === undefined)
  )
    throw new Error("invalid response envelope");
  if (x.error !== undefined) {
    const e = obj(x.error);
    fields(e, ["code", "message"]);
    if (!codes.has(e.code as ErrorCode) || !str(e.message, 512))
      throw new Error("invalid response error");
    throw new EngineRpcError(
      e.code as ErrorCode,
      `Engine request failed (${e.code})`,
    );
  }
  return x as unknown as Response;
}
export type CloseOutcome =
  | "already-exited"
  | "eof-exited"
  | "terminated-after-timeout"
  | "kill-requested-after-timeout";
export interface Transport {
  request(request: Request, timeoutMs?: number): Promise<Response>;
  close(): Promise<CloseOutcome>;
}
export class EngineClient {
  readonly clientId = uuid();
  private readonly transport: Transport;
  private testDouble = false;
  private readonly allowTestDouble: boolean;
  constructor(transport: Transport, options: { allowTestDouble?: boolean } = {}) {
    this.transport = transport;
    this.allowTestDouble = options.allowTestDouble === true;
  }
  private rpc(
    method: Request["method"],
    data: Omit<Request, keyof BaseRequest | "method">,
    timeoutMs: number,
  ): Promise<Response> {
    timeout(timeoutMs);
    const rpcId = uuid();
    const req = {
      protocolVersion: 1,
      clientId: this.clientId,
      rpcId,
      method,
      ...data,
    } as Request;
    return this.transport
      .request(req, timeoutMs)
      .then((v) => response(v, rpcId));
  }
  async handshake(timeoutMs = 5000) {
    const value = validateHandshake(
      (await this.rpc("handshake", {}, timeoutMs)).result,
    );
    if (value.implementation === "test-double" && !this.allowTestDouble)
      throw new Error("Test Engine requires explicit test-only opt-in");
    this.testDouble = value.implementation === "test-double";
    return value;
  }
  prepareStart(bundleId: string, bundlePath: string): StartRequest {
    const request = validateRequest({
      protocolVersion: 1,
      clientId: this.clientId,
      rpcId: uuid(),
      method: "start",
      ownerToken: randomBytes(32).toString("hex"),
      captureId: uuid(),
      bundleId,
      bundlePath,
      options: { screen: true },
    });
    return request as StartRequest;
  }
  async start(request: StartRequest, timeoutMs = 5000): Promise<StartResult> {
    timeout(timeoutMs);
    validateRequest(request);
    const { rpcId: _old, ...durable } = request;
    const rpcId = uuid();
    const correlated = { ...durable, rpcId } as StartRequest;
    const r = response(
      await this.transport.request(correlated, timeoutMs),
      rpcId,
    );
    const receipt = validateReceipt(r.result, rpcId, request.captureId);
    this.rejectSynthetic(receipt);
    if (receipt.state === "saved" && receipt.bundlePath !== request.bundlePath)
      throw new Error("saved bundle path does not match originating start");
    return {
      receipt,
      captureId: request.captureId,
      ownerToken: request.ownerToken,
    };
  }
  async control(
    method: "inspect" | "stop" | "cancel",
    captureId: string,
    ownerToken: string,
    timeoutMs = 5000,
  ): Promise<Receipt> {
    timeout(timeoutMs);
    const rpcId = uuid();
    const req = validateRequest({
      protocolVersion: 1,
      clientId: this.clientId,
      rpcId,
      method,
      captureId,
      ownerToken,
    });
    const r = response(await this.transport.request(req, timeoutMs), rpcId);
    const receipt = validateReceipt(r.result, rpcId, captureId);
    this.rejectSynthetic(receipt);
    return receipt;
  }
  private rejectSynthetic(receipt: Receipt) {
    if (receipt.artifactProvenance === "test-synthetic" && !this.testDouble)
      throw new Error("synthetic test artifact received from non-test Engine");
  }
  close() {
    return this.transport.close();
  }
}
/** Spawn an explicitly selected executable; the SDK never searches PATH. */
export function launchEngine(
  executablePath: string,
  runtimeDirectory: string,
  shutdownTimeoutMs = 3000,
): ChildProcessTransport {
  if (!isAbsolute(executablePath) || resolve(executablePath) !== executablePath)
    throw new Error("Engine path must be absolute");
  timeout(shutdownTimeoutMs);
  if (
    !isAbsolute(runtimeDirectory) ||
    resolve(runtimeDirectory) !== runtimeDirectory
  )
    throw new Error("Runtime directory must be absolute");
  return new ChildProcessTransport(
    spawn(executablePath, ["--runtime-directory", runtimeDirectory], {
      cwd: dirname(executablePath),
      stdio: ["pipe", "pipe", "pipe"],
      shell: false,
    }),
    shutdownTimeoutMs,
  );
}
export class ChildProcessTransport implements Transport {
  private readonly child: ChildProcessWithoutNullStreams;
  private readonly shutdownMs: number;
  private readonly pending = new Map<
    string,
    {
      resolve: (r: Response) => void;
      reject: (e: Error) => void;
      timer: ReturnType<typeof setTimeout>;
    }
  >();
  private bytes = Buffer.alloc(0);
  private closing = false;
  private closed = false;
  private terminal?: Error;
  private closePromise?: Promise<CloseOutcome>;
  constructor(child: ChildProcessWithoutNullStreams, shutdownTimeoutMs = 3000) {
    timeout(shutdownTimeoutMs);
    this.child = child;
    this.shutdownMs = shutdownTimeoutMs;
    child.stdout.on("data", (chunk: Buffer) => this.onData(Buffer.from(chunk)));
    child.stdout.on("error", () =>
      this.fail(new Error("Engine stdout failed")),
    );
    child.stdin.on("error", () => this.fail(new Error("Engine stdin failed")));
    child.stderr.on("data", () => {
      /* Drain diagnostics without retaining or exposing private details. */
    });
    child.stderr.on("error", () => {
      /* stderr is diagnostic-only. */
    });
    child.on("error", () => this.fail(new Error("Engine process failed")));
    child.on("exit", (code, signal) => {
      this.closed = true;
      this.fail(new Error(`Engine exited (${code ?? signal ?? "unknown"})`));
    });
  }
  private onData(chunk: Buffer) {
    if (this.terminal) return;
    this.bytes = Buffer.concat([this.bytes, chunk]);
    if (this.bytes.length > MAX_LINE && this.bytes.indexOf(10) < 0)
      return this.fail(new Error("Engine response exceeds protocol limit"));
    for (;;) {
      const nl = this.bytes.indexOf(10);
      if (nl < 0) break;
      if (nl > MAX_LINE)
        return this.fail(new Error("Engine response exceeds protocol limit"));
      let line = this.bytes.subarray(0, nl);
      this.bytes = this.bytes.subarray(nl + 1);
      if (line.at(-1) === 13) line = line.subarray(0, -1);
      if (line.includes(13))
        return this.fail(new Error("invalid Engine line ending"));
      let value: unknown;
      try {
        value = JSON.parse(
          new TextDecoder("utf-8", { fatal: true }).decode(line),
        );
      } catch {
        return this.fail(new Error("malformed Engine response"));
      }
      try {
        const x = obj(value);
        fields(
          x,
          ["protocolVersion", "rpcId", "result", "error"],
          ["protocolVersion", "rpcId"],
        );
        if (x.protocolVersion !== 1 || !hex(x.rpcId, 32))
          throw new Error("invalid Engine response envelope");
        const p = this.pending.get(x.rpcId);
        if (!p) continue;
        clearTimeout(p.timer);
        this.pending.delete(x.rpcId);
        p.resolve(x as unknown as Response);
      } catch {
        this.fail(new Error("invalid Engine response"));
        return;
      }
    }
    if (this.bytes.length > MAX_LINE)
      this.fail(new Error("Engine response exceeds protocol limit"));
  }
  request(req: Request, timeoutMs = 5000): Promise<Response> {
    timeout(timeoutMs);
    if (this.closed || this.closing || this.terminal)
      return Promise.reject(this.terminal ?? new Error("Engine disconnected"));
    if (this.pending.size >= 64)
      return Promise.reject(
        new Error("Engine transport has reached its pending RPC limit"),
      );
    const line = encodeRequest(req);
    if (this.pending.has(req.rpcId))
      return Promise.reject(new Error("duplicate RPC id"));
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(req.rpcId);
        reject(new Error("Engine RPC timed out; operation was not cancelled"));
      }, timeoutMs);
      if (
        this.child.stdin.writableLength + Buffer.byteLength(line) >
        1024 * 1024
      ) {
        clearTimeout(timer);
        this.pending.delete(req.rpcId);
        reject(new Error("Engine transport write buffer is full"));
        return;
      }
      this.pending.set(req.rpcId, { resolve, reject, timer });
      this.child.stdin.write(line, (err) => {
        if (!err) return;
        const p = this.pending.get(req.rpcId);
        if (p) {
          clearTimeout(p.timer);
          this.pending.delete(req.rpcId);
          p.reject(new Error("Engine request write failed"));
        }
      });
    });
  }
  private fail(error: Error) {
    if (!this.terminal) this.terminal = error;
    for (const p of this.pending.values()) {
      clearTimeout(p.timer);
      p.reject(error);
    }
    this.pending.clear();
  }
  close(): Promise<CloseOutcome> {
    if (this.closePromise) return this.closePromise;
    this.closing = true;
    this.closePromise = new Promise((resolveClose) => {
      if (this.closed || this.child.exitCode !== null) {
        this.closed = true;
        resolveClose("already-exited");
        return;
      }
      let outcome: CloseOutcome = "eof-exited";
      const eofTimer = setTimeout(() => {
        if (!this.closed) {
          outcome = "terminated-after-timeout";
          this.child.kill("SIGTERM");
        }
      }, this.shutdownMs);
      const killTimer = setTimeout(() => {
        if (!this.closed) {
          outcome = "kill-requested-after-timeout";
          this.child.kill("SIGKILL");
        }
      }, this.shutdownMs * 2);
      const reapTimer = setTimeout(() => {
        this.closed = true;
        resolveClose(outcome);
      }, this.shutdownMs * 3);
      this.child.once("exit", () => {
        clearTimeout(eofTimer);
        clearTimeout(killTimer);
        clearTimeout(reapTimer);
        this.closed = true;
        resolveClose(outcome);
      });
      this.child.stdin.end();
    });
    this.fail(
      new Error(
        "Engine SDK stdin closed; Engine owns EOF stop/finalize semantics",
      ),
    );
    return this.closePromise;
  }
}
