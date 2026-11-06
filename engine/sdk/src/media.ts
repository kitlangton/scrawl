import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { randomBytes } from "node:crypto";
import { dirname, isAbsolute, relative, resolve, sep } from "node:path";
import { releaseLocalRequests } from "./recovery/local.ts";

export const MEDIA_CHANNEL = "media" as const;
export const MEDIA_PROTOCOL_VERSION = 1 as const;
const MAX_LINE = 64 * 1024;
const MAX_TIMEOUT_MS = 60_000;
const MAX_FRAMES = 48;
const MAX_EDGE = 4096;
const MAX_TOTAL_PIXELS = 64_000_000;
const MAX_QUEUED_BYTES = 1024 * 1024;
const MAX_TERMINAL_JOBS = 32;
const MEDIA_ERRORS = new Set<MediaErrorCode>([
  "malformed_request",
  "unsupported_channel",
  "unsupported_version",
  "source_mismatch",
  "source_incomplete",
  "invalid_output",
  "limits_exceeded",
  "decode_failed",
  "cancelled",
  "deadline_exceeded",
  "busy",
  "identity_conflict",
  "job_not_found",
  "internal",
]);
const id = () => randomBytes(16).toString("hex");
export const createMediaJobId = id;

export type MediaErrorCode =
  | "malformed_request"
  | "unsupported_channel"
  | "unsupported_version"
  | "source_mismatch"
  | "source_incomplete"
  | "invalid_output"
  | "limits_exceeded"
  | "decode_failed"
  | "cancelled"
  | "deadline_exceeded"
  | "busy"
  | "identity_conflict"
  | "job_not_found"
  | "internal";
export interface FileIdentity {
  device: string;
  inode: string;
  size: number;
  modifiedNs: string;
}
export interface MediaSource {
  bundleId: string;
  bundlePath: string;
  videoPath: string;
  videoOffsetMs: number;
  identity: FileIdentity;
}
export interface DecodeLimits {
  maxFrames: number;
  maxEdge: number;
  maxTotalPixels: number;
  deadlineMs: number;
}
export interface DecodeRequest {
  channel: typeof MEDIA_CHANNEL;
  protocolVersion: typeof MEDIA_PROTOCOL_VERSION;
  clientId: string;
  rpcId: string;
  method: "decode";
  jobId: string;
  source: MediaSource;
  requestedBundleMs: number[];
  outputDirectory: string;
  limits: DecodeLimits;
}
export interface CancelRequest {
  channel: typeof MEDIA_CHANNEL;
  protocolVersion: typeof MEDIA_PROTOCOL_VERSION;
  clientId: string;
  rpcId: string;
  method: "cancel";
  jobId: string;
}
export interface MediaHandshakeRequest {
  channel: typeof MEDIA_CHANNEL;
  protocolVersion: typeof MEDIA_PROTOCOL_VERSION;
  clientId: string;
  rpcId: string;
  method: "handshake";
}
export interface MediaHandshake {
  channel: typeof MEDIA_CHANNEL;
  protocolVersion: typeof MEDIA_PROTOCOL_VERSION;
  implementation: "native" | "test-double";
  capabilities: { decodeFrames: boolean };
}
export interface MediaFrameRef {
  requestIndex: number;
  requestedBundleMs: number;
  effectiveVideoMs: number;
  actualVideoMs?: number;
  clampedToStart: boolean;
  clampedToEnd: boolean;
  path: string;
  width: number;
  height: number;
  byteLength: number;
  sha256: string;
}
export interface DecodeResult {
  jobId: string;
  state: "complete";
  frames: MediaFrameRef[];
}
export interface MediaFailure {
  jobId: string;
  state: "failed" | "cancelled";
  error: { code: MediaErrorCode; message: string };
  completedCount: 0;
}
export type MediaJobStatus =
  | { jobId: string; state: "accepted" | "decoding" | "cancelling" }
  | DecodeResult
  | MediaFailure;
export interface InspectRequest {
  channel: typeof MEDIA_CHANNEL;
  protocolVersion: typeof MEDIA_PROTOCOL_VERSION;
  clientId: string;
  rpcId: string;
  method: "inspect";
  jobId: string;
}
export interface MediaResponse {
  channel: typeof MEDIA_CHANNEL;
  protocolVersion: typeof MEDIA_PROTOCOL_VERSION;
  rpcId: string;
  result?:
    | MediaHandshake
    | DecodeResult
    | MediaFailure
    | {
        jobId: string;
        state: "cancelling" | "cancelled" | "complete" | "failed";
      }
    | MediaJobStatus;
  error?: { code: MediaErrorCode; message: string };
}
export type MediaRequest =
  | DecodeRequest
  | CancelRequest
  | InspectRequest
  | MediaHandshakeRequest;

function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value))
    throw new Error("invalid media protocol object");
  return value as Record<string, unknown>;
}
function exactFields(
  value: Record<string, unknown>,
  allowed: string[],
  required: string[] = allowed,
) {
  if (
    Object.keys(value).some((key) => !allowed.includes(key)) ||
    required.some((key) => !(key in value))
  )
    throw new Error("invalid media protocol fields");
}
function absolute(value: unknown): value is string {
  return (
    typeof value === "string" &&
    value.length <= 4096 &&
    isAbsolute(value) &&
    resolve(value) === value &&
    !value.includes("\0")
  );
}
function positiveInt(value: unknown, max: number): value is number {
  return (
    Number.isSafeInteger(value) &&
    (value as number) > 0 &&
    (value as number) <= max
  );
}
function hex(value: unknown): value is string {
  return typeof value === "string" && /^[0-9a-f]{32}$/.test(value);
}
function timeout(value: number) {
  if (!positiveInt(value, MAX_TIMEOUT_MS))
    throw new RangeError(`media timeout must be 1..${MAX_TIMEOUT_MS} ms`);
}
function jobBinding(request: DecodeRequest): string {
  const s = request.source,
    i = s.identity,
    l = request.limits;
  return JSON.stringify([
    s.bundleId,
    s.bundlePath,
    s.videoPath,
    s.videoOffsetMs,
    i.device,
    i.inode,
    i.size,
    i.modifiedNs,
    [...request.requestedBundleMs],
    request.outputDirectory,
    l.maxFrames,
    l.maxEdge,
    l.maxTotalPixels,
    l.deadlineMs,
  ]);
}

function validateIdentity(value: unknown): FileIdentity {
  const x = object(value);
  exactFields(x, ["device", "inode", "size", "modifiedNs"]);
  if (
    typeof x.device !== "string" ||
    !/^\d+$/.test(x.device) ||
    typeof x.inode !== "string" ||
    !/^\d+$/.test(x.inode) ||
    !positiveInt(x.size, Number.MAX_SAFE_INTEGER) ||
    typeof x.modifiedNs !== "string" ||
    !/^\d+$/.test(x.modifiedNs)
  )
    throw new Error("invalid source identity");
  return x as unknown as FileIdentity;
}
function validateSource(value: unknown): MediaSource {
  const x = object(value);
  exactFields(x, [
    "bundleId",
    "bundlePath",
    "videoPath",
    "videoOffsetMs",
    "identity",
  ]);
  if (
    typeof x.bundleId !== "string" ||
    Buffer.byteLength(x.bundleId, "utf8") < 1 ||
    Buffer.byteLength(x.bundleId, "utf8") > 255 ||
    x.bundleId === "." ||
    x.bundleId === ".." ||
    /[\\/\p{Cc}]/u.test(x.bundleId) ||
    !absolute(x.bundlePath) ||
    !absolute(x.videoPath) ||
    !Number.isSafeInteger(x.videoOffsetMs) ||
    (x.videoOffsetMs as number) < 0
  )
    throw new Error("invalid media source");
  const videoRelative = relative(x.bundlePath as string, x.videoPath as string);
  if (
    !videoRelative ||
    videoRelative === ".." ||
    videoRelative.startsWith(`..${sep}`) ||
    isAbsolute(videoRelative)
  )
    throw new Error("invalid media source path");
  return {
    ...x,
    identity: validateIdentity(x.identity),
  } as unknown as MediaSource;
}
function validateLimits(value: unknown): DecodeLimits {
  const x = object(value);
  exactFields(x, ["maxFrames", "maxEdge", "maxTotalPixels", "deadlineMs"]);
  if (
    !positiveInt(x.maxFrames, MAX_FRAMES) ||
    !positiveInt(x.maxEdge, MAX_EDGE) ||
    !positiveInt(x.maxTotalPixels, MAX_TOTAL_PIXELS) ||
    !positiveInt(x.deadlineMs, MAX_TIMEOUT_MS)
  )
    throw new Error("media limits exceed protocol bounds");
  return x as unknown as DecodeLimits;
}
export function validateMediaRequest(value: unknown): MediaRequest {
  const x = object(value);
  if (x.channel !== MEDIA_CHANNEL) throw new Error("unsupported media channel");
  if (x.protocolVersion !== MEDIA_PROTOCOL_VERSION)
    throw new Error("unsupported media protocol version");
  const common = ["channel", "protocolVersion", "clientId", "rpcId", "method"];
  if (!hex(x.clientId) || !hex(x.rpcId))
    throw new Error("invalid media RPC identity");
  if (x.method === "handshake") {
    exactFields(x, common);
    return x as unknown as MediaHandshakeRequest;
  }
  if (x.method === "cancel" || x.method === "inspect") {
    exactFields(x, [...common, "jobId"]);
    if (!hex(x.jobId)) throw new Error("invalid media job identity");
    return x.method === "inspect"
      ? (x as unknown as InspectRequest)
      : (x as unknown as CancelRequest);
  }
  if (x.method !== "decode") throw new Error("unsupported media method");
  exactFields(x, [
    ...common,
    "jobId",
    "source",
    "requestedBundleMs",
    "outputDirectory",
    "limits",
  ]);
  if (
    !hex(x.jobId) ||
    !absolute(x.outputDirectory) ||
    !Array.isArray(x.requestedBundleMs) ||
    x.requestedBundleMs.length < 1 ||
    !x.requestedBundleMs.every(
      (time) => Number.isSafeInteger(time) && (time as number) >= 0,
    )
  )
    throw new Error("invalid decode request");
  const limits = validateLimits(x.limits);
  if (
    x.requestedBundleMs.length > limits.maxFrames ||
    x.requestedBundleMs.length > MAX_FRAMES
  )
    throw new Error("invalid decode request");
  const source = validateSource(x.source);
  return {
    ...x,
    source,
    requestedBundleMs: [...x.requestedBundleMs],
    limits: { ...limits },
  } as unknown as DecodeRequest;
}
export function encodeMediaRequest(value: MediaRequest): string {
  const request = validateMediaRequest(value);
  const line = JSON.stringify(request);
  if (Buffer.byteLength(line) > MAX_LINE)
    throw new Error("media request exceeds protocol limit");
  return `${line}\n`;
}
export function validateMediaResponse(
  value: unknown,
  expectedRpcId: string,
  request?: MediaRequest,
  decodeContext?: DecodeRequest,
): MediaResponse {
  const x = object(value);
  exactFields(
    x,
    ["channel", "protocolVersion", "rpcId", "result", "error"],
    ["channel", "protocolVersion", "rpcId"],
  );
  if (
    x.channel !== MEDIA_CHANNEL ||
    x.protocolVersion !== MEDIA_PROTOCOL_VERSION ||
    x.rpcId !== expectedRpcId ||
    (x.result === undefined) === (x.error === undefined)
  )
    throw new Error("invalid media response envelope");
  if (x.error !== undefined) {
    const e = object(x.error);
    exactFields(e, ["code", "message"]);
    if (
      typeof e.code !== "string" ||
      !MEDIA_ERRORS.has(e.code as MediaErrorCode) ||
      typeof e.message !== "string" ||
      e.message.length > 512
    )
      throw new Error("invalid media error");
    return {
      channel: MEDIA_CHANNEL,
      protocolVersion: MEDIA_PROTOCOL_VERSION,
      rpcId: expectedRpcId,
      error: {
        code: e.code as MediaErrorCode,
        message: `Engine media request failed (${e.code})`,
      },
    };
  }
  const r = object(x.result);
  if (request?.method === "handshake") {
    exactFields(r, [
      "channel",
      "protocolVersion",
      "implementation",
      "capabilities",
    ]);
    const caps = object(r.capabilities);
    exactFields(caps, ["decodeFrames"]);
    if (
      r.channel !== MEDIA_CHANNEL ||
      r.protocolVersion !== MEDIA_PROTOCOL_VERSION ||
      !["native", "test-double"].includes(String(r.implementation)) ||
      typeof caps.decodeFrames !== "boolean" ||
      (r.implementation === "test-double" && caps.decodeFrames)
    )
      throw new Error("invalid media handshake");
  } else if (request?.method === "cancel") {
    exactFields(r, ["jobId", "state"]);
    if (
      r.jobId !== request.jobId ||
      !["cancelling", "cancelled", "complete", "failed"].includes(
        String(r.state),
      )
    )
      throw new Error("invalid media cancellation result");
  } else if (request?.method === "inspect") {
    const status = validateJobStatus(r, request.jobId);
    if (status.state === "complete") {
      if (!decodeContext || decodeContext.jobId !== request.jobId)
        throw new Error("completed media inspection requires matching decode context");
      const context = validateMediaRequest(decodeContext);
      if (context.method !== "decode")
        throw new Error("completed media inspection requires decode context");
      validateMediaResponse(value, expectedRpcId, context);
    }
    return { ...x, result: status } as unknown as MediaResponse;
  } else {
    if (request?.method !== "decode")
      throw new Error("decode response has no matching request");
    exactFields(r, ["jobId", "state", "frames"]);
    if (
      r.jobId !== request.jobId ||
      r.state !== "complete" ||
      !Array.isArray(r.frames) ||
      r.frames.length !== request.requestedBundleMs.length
    )
      throw new Error("invalid media decode result");
    const seen = new Set<number>();
    const paths = new Set<string>();
    for (const [framePosition, raw] of r.frames.entries()) {
      const f = object(raw);
      exactFields(
        f,
        [
          "requestIndex",
          "requestedBundleMs",
          "effectiveVideoMs",
          "actualVideoMs",
          "clampedToStart",
          "clampedToEnd",
          "path",
          "width",
          "height",
          "byteLength",
          "sha256",
        ],
        [
          "requestIndex",
          "requestedBundleMs",
          "effectiveVideoMs",
          "clampedToStart",
          "clampedToEnd",
          "path",
          "width",
          "height",
          "byteLength",
          "sha256",
        ],
      );
      if (
        !positiveInt(f.requestIndex, r.frames.length) ||
        f.requestIndex !== framePosition + 1 ||
        seen.has(f.requestIndex as number) ||
        typeof f.requestedBundleMs !== "number" ||
        typeof f.effectiveVideoMs !== "number" ||
        f.effectiveVideoMs < 0 ||
        !absolute(f.path) ||
        !/^frame-[0-9]{3}\.png$/.test(
          f.path.slice(dirname(f.path).length + 1),
        ) ||
        !positiveInt(f.width, MAX_EDGE) ||
        !positiveInt(f.height, MAX_EDGE) ||
        !positiveInt(f.byteLength, Number.MAX_SAFE_INTEGER) ||
        typeof f.sha256 !== "string" ||
        !/^[0-9a-f]{64}$/.test(f.sha256) ||
        typeof f.clampedToStart !== "boolean" ||
        typeof f.clampedToEnd !== "boolean" ||
        (f.actualVideoMs !== undefined &&
          (typeof f.actualVideoMs !== "number" ||
            !Number.isFinite(f.actualVideoMs) ||
            f.actualVideoMs < 0))
      )
        throw new Error("invalid media frame reference");
      const outputRelative = relative(request.outputDirectory, f.path);
      if (
        !outputRelative ||
        outputRelative === ".." ||
        outputRelative.startsWith(`..${sep}`) ||
        outputRelative.includes(sep) ||
        isAbsolute(outputRelative)
      )
        throw new Error("media frame reference escapes output directory");
      if (paths.has(f.path as string))
        throw new Error("media frame reference aliases another slot");
      paths.add(f.path as string);
      seen.add(f.requestIndex as number);
      const expectedIndex = f.requestIndex as number;
      if (
        f.path !==
        `${request.outputDirectory}/frame-${String(expectedIndex).padStart(3, "0")}.png`
      )
        throw new Error("media frame reference aliases another slot");
      const expectedBundleMs = request!.requestedBundleMs[expectedIndex - 1];
      const expectedVideoMs = Math.max(
        0,
        expectedBundleMs - request!.source.videoOffsetMs,
      );
      if (
        f.requestedBundleMs !== expectedBundleMs ||
        f.effectiveVideoMs !== expectedVideoMs ||
        f.clampedToStart !== expectedBundleMs < request!.source.videoOffsetMs
      )
        throw new Error("media frame order or clock mapping mismatch");
    }
    if (seen.size !== r.frames.length)
      throw new Error("media frame inventory is incomplete");
    const limits = request!.limits;
    const pixels = (r.frames as Record<string, unknown>[]).reduce(
      (sum, f) => sum + (f.width as number) * (f.height as number),
      0,
    );
    if (
      pixels > limits.maxTotalPixels ||
      r.frames.some((raw) => {
        const f = raw as Record<string, unknown>;
        return (
          (f.width as number) > limits.maxEdge ||
          (f.height as number) > limits.maxEdge
        );
      })
    )
      throw new Error("media frame resource bounds exceeded");
  }
  return x as unknown as MediaResponse;
}

function validateJobStatus(value: unknown, jobId: string): MediaJobStatus {
  const status = object(value);
  if (status.jobId !== jobId)
    throw new Error("media job status identity mismatch");
  if (["accepted", "decoding", "cancelling"].includes(String(status.state))) {
    exactFields(status, ["jobId", "state"]);
    return status as unknown as MediaJobStatus;
  }
  if (status.state === "complete") {
    exactFields(status, ["jobId", "state", "frames"]);
    if (!Array.isArray(status.frames))
      throw new Error("invalid media job status");
    return status as unknown as DecodeResult;
  }
  if (status.state === "failed" || status.state === "cancelled") {
    exactFields(status, ["jobId", "state", "error", "completedCount"]);
    const error = object(status.error);
    exactFields(error, ["code", "message"]);
    if (
      !MEDIA_ERRORS.has(error.code as MediaErrorCode) ||
      typeof error.message !== "string" ||
      error.message.length > 512 ||
      (status.state === "cancelled") !== (error.code === "cancelled") ||
      status.completedCount !== 0
    )
      throw new Error("invalid terminal media job status");
    return {
      jobId,
      state: status.state,
      error: {
        code: error.code as MediaErrorCode,
        message: `Engine media job failed (${error.code})`,
      },
      completedCount: 0,
    };
  }
  throw new Error("invalid media job status");
}

export type MediaCloseOutcome =
  | "already-exited"
  | "eof-exited"
  | "term-requested-after-timeout"
  | "kill-requested-after-timeout";
export interface MediaTransport {
  request(request: MediaRequest, timeoutMs?: number): Promise<MediaResponse>;
  close(): Promise<MediaCloseOutcome>;
}
export class MediaRpcError extends Error {
  readonly code: string;
  constructor(code: string, message = `Engine media request failed (${code})`) {
    super(message);
    this.code = code;
    this.name = "MediaRpcError";
  }
}
export class MediaClient {
  readonly clientId = id();
  private implementation?: "native" | "test-double";
  private readonly transport: MediaTransport;
  private readonly options: { allowTestDouble?: boolean };
  private readonly jobs = new Map<string, string>();
  constructor(
    transport: MediaTransport,
    options: { allowTestDouble?: boolean } = {},
  ) {
    this.transport = transport;
    this.options = options;
  }
  async handshake(timeoutMs = 5000): Promise<MediaHandshake> {
    timeout(timeoutMs);
    const request = validateMediaRequest({
      channel: MEDIA_CHANNEL,
      protocolVersion: MEDIA_PROTOCOL_VERSION,
      clientId: this.clientId,
      rpcId: id(),
      method: "handshake",
    });
    const response = validateMediaResponse(
      await this.transport.request(request, timeoutMs),
      request.rpcId,
      request,
    );
    if (response.error) throw new MediaRpcError(response.error.code);
    const handshake = response.result as MediaHandshake;
    if (
      handshake.implementation === "test-double" &&
      this.options.allowTestDouble !== true
    )
      throw new Error("Test media peer requires explicit test-only opt-in");
    this.implementation = handshake.implementation;
    return handshake;
  }
  async decode(
    input: Omit<
      DecodeRequest,
      "channel" | "protocolVersion" | "clientId" | "rpcId" | "method"
    >,
    timeoutMs = input.limits.deadlineMs,
  ): Promise<DecodeResult> {
    if (!this.implementation)
      throw new Error("media handshake required before decode");
    timeout(timeoutMs);
    const request = validateMediaRequest({
      ...input,
      source: structuredClone(input.source),
      requestedBundleMs: [...input.requestedBundleMs],
      limits: { ...input.limits },
      channel: MEDIA_CHANNEL,
      protocolVersion: MEDIA_PROTOCOL_VERSION,
      clientId: this.clientId,
      rpcId: id(),
      method: "decode",
    }) as DecodeRequest;
    const binding = jobBinding(request);
    const previous = this.jobs.get(request.jobId);
    if (previous !== undefined)
      throw new MediaRpcError(
        previous === binding ? "busy" : "identity_conflict",
      );
    if (this.jobs.size >= MAX_TERMINAL_JOBS) throw new MediaRpcError("busy");
    this.jobs.set(request.jobId, binding);
    const response = validateMediaResponse(
      await this.transport.request(request, timeoutMs),
      request.rpcId,
      request,
    );
    if (response.error) throw new MediaRpcError(response.error.code);
    return response.result as DecodeResult;
  }
  async cancel(
    jobId: string,
    timeoutMs = 5000,
  ): Promise<"cancelling" | "cancelled" | "complete" | "failed"> {
    if (!this.implementation)
      throw new Error("media handshake required before cancel");
    timeout(timeoutMs);
    const request = validateMediaRequest({
      channel: MEDIA_CHANNEL,
      protocolVersion: MEDIA_PROTOCOL_VERSION,
      clientId: this.clientId,
      rpcId: id(),
      method: "cancel",
      jobId,
    });
    const response = validateMediaResponse(
      await this.transport.request(request, timeoutMs),
      request.rpcId,
      request,
    );
    if (response.error) throw new MediaRpcError(response.error.code);
    return (
      response.result as {
        state: "cancelling" | "cancelled" | "complete" | "failed";
      }
    ).state;
  }
  async inspect(
    input: Omit<
      DecodeRequest,
      "channel" | "protocolVersion" | "clientId" | "rpcId" | "method"
    >,
    timeoutMs = 5000,
  ): Promise<MediaJobStatus> {
    if (!this.implementation)
      throw new Error("media handshake required before inspect");
    timeout(timeoutMs);
    const context = validateMediaRequest({
      ...input,
      source: structuredClone(input.source),
      requestedBundleMs: [...input.requestedBundleMs],
      limits: { ...input.limits },
      channel: MEDIA_CHANNEL,
      protocolVersion: MEDIA_PROTOCOL_VERSION,
      clientId: this.clientId,
      rpcId: id(),
      method: "decode",
    }) as DecodeRequest;
    const knownBinding = this.jobs.get(context.jobId);
    if (knownBinding !== undefined && knownBinding !== jobBinding(context))
      throw new MediaRpcError("identity_conflict");
    const request = validateMediaRequest({
      channel: MEDIA_CHANNEL,
      protocolVersion: MEDIA_PROTOCOL_VERSION,
      clientId: this.clientId,
      rpcId: id(),
      method: "inspect",
      jobId: context.jobId,
    }) as InspectRequest;
    const response = validateMediaResponse(
      await this.transport.request(request, timeoutMs),
      request.rpcId,
      request,
      context,
    );
    if (response.error) throw new MediaRpcError(response.error.code);
    return response.result as MediaJobStatus;
  }
  close() {
    return this.transport.close();
  }
}

/** Explicit binary and runtime directory; never searches PATH or builds source. */
export function launchMediaWorker(
  executablePath: string,
  runtimeDirectory: string,
  shutdownTimeoutMs = 3000,
): MediaChildProcessTransport {
  if (!absolute(executablePath) || !absolute(runtimeDirectory))
    throw new Error("media executable and runtime directory must be absolute");
  timeout(shutdownTimeoutMs);
  return new MediaChildProcessTransport(
    spawn(
      executablePath,
      [
        "--media-runtime-directory",
        runtimeDirectory,
        "--media-channel",
        MEDIA_CHANNEL,
        "--media-protocol-version",
        String(MEDIA_PROTOCOL_VERSION),
      ],
      {
        cwd: dirname(executablePath),
        stdio: ["pipe", "pipe", "pipe"],
        shell: false,
      },
    ),
    shutdownTimeoutMs,
  );
}
/** The framing/lifetime implementation is shared by negotiated media versions.
 * Existing callers retain the strict media/1 defaults. */
export class MediaProcessTransport<Request extends { rpcId: string } = MediaRequest, Response = MediaResponse> {
  private readonly child: ChildProcessWithoutNullStreams;
  private readonly shutdownMs: number;
  private bytes = Buffer.alloc(0);
  private readonly pending = new Map<
    string,
    {
      resolve: (value: Response) => void;
      reject: (error: Error) => void;
      timer: ReturnType<typeof setTimeout>;
    }
  >();
  private closed = false;
  private closing = false;
  private terminal?: Error;
  private closePromise?: Promise<MediaCloseOutcome>;
  private stdoutEnded = false;
  private queuedBytes = 0;
  private readonly protocol?: { version: number; encode: (request: Request) => string; maxLine: number };
  constructor(child: ChildProcessWithoutNullStreams, shutdownMs = 3000, protocol?: { version: number; encode: (request: Request) => string; maxLine: number }) {
    timeout(shutdownMs);
    if (protocol && ((!Number.isInteger(protocol.version) || protocol.version < 1 || protocol.version > 2) || !Number.isSafeInteger(protocol.maxLine) || protocol.maxLine < 1 || protocol.maxLine > 1024 * 1024 || typeof protocol.encode !== "function")) throw new RangeError("invalid bounded media protocol binding");
    this.protocol = protocol ? { ...protocol } : undefined;
    this.child = child;
    this.shutdownMs = shutdownMs;
    child.stdout.on("data", (chunk: Buffer) => this.onData(Buffer.from(chunk)));
    child.stdout.on("end", () => {
      this.stdoutEnded = true;
      this.fail(new Error("media worker stdout ended"));
    });
    child.stdout.on("error", () =>
      this.fail(new Error("media worker stdout failed")),
    );
    child.stdin.on("error", () =>
      this.fail(new Error("media worker stdin failed")),
    );
    child.stderr.on("data", () => {});
    child.stderr.on("error", () => {});
    child.on("error", () => {
      if (child.pid === undefined) this.closed = true;
      this.fail(new Error("media worker process failed"));
    });
    child.on("exit", (code, signal) => {
      this.closed = true;
      this.fail(
        new Error(`media worker exited (${code ?? signal ?? "unknown"})`),
      );
    });
  }
  private onData(chunk: Buffer) {
    if (this.terminal) return;
    this.bytes = Buffer.concat([this.bytes, chunk]);
    if (this.bytes.length > this.lineLimit && this.bytes.indexOf(10) < 0)
      return this.fail(new Error("media response exceeds protocol limit"));
    while (true) {
      const nl = this.bytes.indexOf(10);
      if (nl < 0) break;
      if (nl > this.lineLimit)
        return this.fail(new Error("media response exceeds protocol limit"));
      let line = this.bytes.subarray(0, nl);
      this.bytes = this.bytes.subarray(nl + 1);
      if (line.at(-1) === 13) line = line.subarray(0, -1);
      let value: unknown;
      try {
        value = JSON.parse(
          new TextDecoder("utf-8", { fatal: true }).decode(line),
        );
      } catch {
        return this.fail(new Error("malformed media response"));
      }
      try {
        const x = object(value);
        exactFields(
          x,
          ["channel", "protocolVersion", "rpcId", "result", "error"],
          ["channel", "protocolVersion", "rpcId"],
        );
        if (
          x.channel !== MEDIA_CHANNEL ||
          x.protocolVersion !== (this.protocol?.version ?? MEDIA_PROTOCOL_VERSION) ||
          typeof x.rpcId !== "string" ||
          !hex(x.rpcId)
        )
          throw new Error("incompatible media peer");
        const pending = this.pending.get(x.rpcId);
        if (!pending) continue;
        clearTimeout(pending.timer);
        this.pending.delete(x.rpcId);
        pending.resolve(x as unknown as Response);
      } catch {
        this.fail(new Error("incompatible media peer or protocol version"));
        return;
      }
    }
    if (this.bytes.length > this.lineLimit)
      this.fail(new Error("media response exceeds protocol limit"));
  }
  private get lineLimit() { return this.protocol?.maxLine ?? MAX_LINE; }
  request(request: Request, timeoutMs = 5000): Promise<Response> {
    timeout(timeoutMs);
    if (this.closed || this.closing || this.terminal)
      return Promise.reject(
        this.terminal ?? new Error("media worker disconnected"),
      );
    if (this.pending.size >= 32)
      return Promise.reject(new Error("media worker RPC limit reached"));
    const line = this.protocol ? this.protocol.encode(request) : encodeMediaRequest(request as unknown as MediaRequest);
    if (
      this.queuedBytes + Buffer.byteLength(line) > MAX_QUEUED_BYTES ||
      this.child.stdin.writableLength + Buffer.byteLength(line) >
        MAX_QUEUED_BYTES
    )
      return Promise.reject(
        new Error("media worker write buffer limit reached"),
      );
    if (this.pending.has(request.rpcId))
      return Promise.reject(new Error("duplicate media RPC id"));
    return new Promise((resolvePromise, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(request.rpcId);
        reject(new Error("media RPC timed out; decode was not cancelled"));
      }, timeoutMs);
      this.pending.set(request.rpcId, {
        resolve: resolvePromise,
        reject,
        timer,
      });
      const writeBytes = Buffer.byteLength(line);
      this.queuedBytes += writeBytes;
      this.child.stdin.write(line, (error) => {
        this.queuedBytes = Math.max(0, this.queuedBytes - writeBytes);
        if (!error) return;
        this.fail(new Error("media request write failed"));
        const p = this.pending.get(request.rpcId);
        if (p) {
          clearTimeout(p.timer);
          this.pending.delete(request.rpcId);
          p.reject(new Error("media request write failed"));
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
  [releaseLocalRequests](): void {
    this.closing = true;
    this.bytes = Buffer.alloc(0);
    this.fail(new Error("recovery connection released; operation outcome remains unknown"));
  }
  close(): Promise<MediaCloseOutcome> {
    if (this.closePromise) return this.closePromise;
    this.closing = true;
    this.fail(new Error("media SDK transport closing"));
    this.closePromise = new Promise<MediaCloseOutcome>((resolvePromise) => {
      if (
        this.closed ||
        this.child.exitCode !== null ||
        this.child.signalCode !== null
      ) {
        this.closed = true;
        resolvePromise("already-exited");
        return;
      }
      let settled = false;
      let outcome: MediaCloseOutcome = "eof-exited";
      const settle = () => {
        if (settled) return;
        settled = true;
        clearTimeout(term);
        clearTimeout(kill);
        this.closed = true;
        resolvePromise(outcome);
      };
      const term = setTimeout(() => {
        if (!this.closed) {
          this.child.kill("SIGTERM");
          outcome = "term-requested-after-timeout";
        }
      }, this.shutdownMs);
      const kill = setTimeout(() => {
        if (!this.closed) {
          this.child.kill("SIGKILL");
          outcome = "kill-requested-after-timeout";
          settle();
        }
      }, this.shutdownMs * 2);
      this.child.once("exit", () => {
        settle();
      });
      this.child.stdin.end();
      this.child.once("close", () => {
        settle();
      });
    });
    return this.closePromise;
  }
}

/** Preserve the existing media/1 constructor and reflected instance type. */
export class MediaChildProcessTransport extends MediaProcessTransport<MediaRequest, MediaResponse> {
  constructor(child: ChildProcessWithoutNullStreams, shutdownMs = 3000) { super(child, shutdownMs); }
}
