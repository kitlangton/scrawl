import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { randomBytes } from "node:crypto";
import { dirname, isAbsolute, relative, resolve, sep } from "node:path";

export const MEDIA_CHANNEL = "media" as const;
export const MEDIA_PROTOCOL_VERSION = 1 as const;
const MAX_LINE = 64 * 1024;
const MAX_TIMEOUT_MS = 60_000;
const MAX_FRAMES = 48;
const MAX_EDGE = 2048;
const MAX_TOTAL_PIXELS = 64_000_000;
const MEDIA_ERRORS = new Set<MediaErrorCode>(["malformed_request", "unsupported_channel", "unsupported_version", "source_mismatch", "source_incomplete", "invalid_output", "limits_exceeded", "decode_failed", "cancelled", "deadline_exceeded", "busy", "internal"]);
const id = () => randomBytes(16).toString("hex");
export const createMediaJobId = id;

export type MediaErrorCode = "malformed_request" | "unsupported_channel" | "unsupported_version" | "source_mismatch" | "source_incomplete" | "invalid_output" | "limits_exceeded" | "decode_failed" | "cancelled" | "deadline_exceeded" | "busy" | "internal";
export interface FileIdentity { device: string; inode: string; size: number; modifiedNs: string }
export interface MediaSource { bundleId: string; bundlePath: string; videoPath: string; videoOffsetMs: number; identity: FileIdentity }
export interface DecodeLimits { maxFrames: number; maxEdge: number; maxTotalPixels: number; deadlineMs: number }
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
export interface MediaHandshakeRequest { channel: typeof MEDIA_CHANNEL; protocolVersion: typeof MEDIA_PROTOCOL_VERSION; clientId: string; rpcId: string; method: "handshake" }
export interface MediaHandshake { channel: typeof MEDIA_CHANNEL; protocolVersion: typeof MEDIA_PROTOCOL_VERSION; implementation: "native" | "test-double"; capabilities: { decodeFrames: boolean } }
export interface MediaFrameRef { requestIndex: number; requestedBundleMs: number; effectiveVideoMs: number; actualVideoMs?: number; clampedToEnd: boolean; path: string; width: number; height: number; byteLength: number; sha256: string }
export interface DecodeResult { jobId: string; state: "complete"; frames: MediaFrameRef[] }
export interface MediaFailure { jobId: string; state: "failed" | "cancelled"; error: { code: MediaErrorCode; message: string }; completedCount: 0 }
export interface MediaResponse { channel: typeof MEDIA_CHANNEL; protocolVersion: typeof MEDIA_PROTOCOL_VERSION; rpcId: string; result?: MediaHandshake | DecodeResult | { jobId: string; state: "cancelling" | "cancelled" }; error?: { code: MediaErrorCode; message: string } }
export type MediaRequest = DecodeRequest | CancelRequest | MediaHandshakeRequest;

function object(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) throw new Error("invalid media protocol object");
  return value as Record<string, unknown>;
}
function exactFields(value: Record<string, unknown>, allowed: string[], required: string[] = allowed) {
  if (Object.keys(value).some((key) => !allowed.includes(key)) || required.some((key) => !(key in value))) throw new Error("invalid media protocol fields");
}
function absolute(value: unknown): value is string { return typeof value === "string" && value.length <= 4096 && isAbsolute(value) && resolve(value) === value && !value.includes("\0"); }
function positiveInt(value: unknown, max: number): value is number { return Number.isSafeInteger(value) && (value as number) > 0 && (value as number) <= max; }
function hex(value: unknown): value is string { return typeof value === "string" && /^[0-9a-f]{32}$/.test(value); }
function timeout(value: number) { if (!positiveInt(value, MAX_TIMEOUT_MS)) throw new RangeError(`media timeout must be 1..${MAX_TIMEOUT_MS} ms`); }

function validateIdentity(value: unknown): FileIdentity {
  const x = object(value); exactFields(x, ["device", "inode", "size", "modifiedNs"]);
  if (typeof x.device !== "string" || !/^\d+$/.test(x.device) || typeof x.inode !== "string" || !/^\d+$/.test(x.inode) || !positiveInt(x.size, Number.MAX_SAFE_INTEGER) || typeof x.modifiedNs !== "string" || !/^\d+$/.test(x.modifiedNs)) throw new Error("invalid source identity");
  return x as unknown as FileIdentity;
}
function validateSource(value: unknown): MediaSource {
  const x = object(value); exactFields(x, ["bundleId", "bundlePath", "videoPath", "videoOffsetMs", "identity"]);
  if (typeof x.bundleId !== "string" || x.bundleId.length < 1 || x.bundleId.length > 255 || /[\\/\p{Cc}]/u.test(x.bundleId) || !absolute(x.bundlePath) || !absolute(x.videoPath) || !Number.isSafeInteger(x.videoOffsetMs) || (x.videoOffsetMs as number) < 0) throw new Error("invalid media source");
  return { ...x, identity: validateIdentity(x.identity) } as unknown as MediaSource;
}
function validateLimits(value: unknown): DecodeLimits {
  const x = object(value); exactFields(x, ["maxFrames", "maxEdge", "maxTotalPixels", "deadlineMs"]);
  if (!positiveInt(x.maxFrames, MAX_FRAMES) || !positiveInt(x.maxEdge, MAX_EDGE) || !positiveInt(x.maxTotalPixels, MAX_TOTAL_PIXELS) || !positiveInt(x.deadlineMs, MAX_TIMEOUT_MS)) throw new Error("media limits exceed protocol bounds");
  return x as unknown as DecodeLimits;
}
export function validateMediaRequest(value: unknown): MediaRequest {
  const x = object(value);
  if (x.channel !== MEDIA_CHANNEL) throw new Error("unsupported media channel");
  if (x.protocolVersion !== MEDIA_PROTOCOL_VERSION) throw new Error("unsupported media protocol version");
  const common = ["channel", "protocolVersion", "clientId", "rpcId", "method"];
  if (!hex(x.clientId) || !hex(x.rpcId)) throw new Error("invalid media RPC identity");
  if (x.method === "handshake") { exactFields(x, common); return x as unknown as MediaHandshakeRequest; }
  if (x.method === "cancel") {
    exactFields(x, [...common, "jobId"]);
    if (!hex(x.jobId)) throw new Error("invalid media job identity");
    return x as unknown as CancelRequest;
  }
  if (x.method !== "decode") throw new Error("unsupported media method");
  exactFields(x, [...common, "jobId", "source", "requestedBundleMs", "outputDirectory", "limits"]);
  if (!hex(x.jobId) || !absolute(x.outputDirectory) || !Array.isArray(x.requestedBundleMs) || x.requestedBundleMs.length < 1 || !x.requestedBundleMs.every((time) => Number.isSafeInteger(time) && (time as number) >= 0)) throw new Error("invalid decode request");
  const limits = validateLimits(x.limits);
  if (x.requestedBundleMs.length > limits.maxFrames || x.requestedBundleMs.length > MAX_FRAMES || x.requestedBundleMs.some((time) => (time as number) < (x.source && typeof x.source === "object" && "videoOffsetMs" in x.source ? (x.source as {videoOffsetMs:number}).videoOffsetMs : 0))) throw new Error("requested bundle time precedes the video clock");
  validateSource(x.source);
  return { ...x, limits } as unknown as DecodeRequest;
}
export function encodeMediaRequest(value: MediaRequest): string {
  const request = validateMediaRequest(value);
  const line = JSON.stringify(request);
  if (Buffer.byteLength(line) > MAX_LINE) throw new Error("media request exceeds protocol limit");
  return `${line}\n`;
}
export function validateMediaResponse(value: unknown, expectedRpcId: string, request?: MediaRequest): MediaResponse {
  const x = object(value); exactFields(x, ["channel", "protocolVersion", "rpcId", "result", "error"], ["channel", "protocolVersion", "rpcId"]);
  if (x.channel !== MEDIA_CHANNEL || x.protocolVersion !== MEDIA_PROTOCOL_VERSION || x.rpcId !== expectedRpcId || (x.result === undefined) === (x.error === undefined)) throw new Error("invalid media response envelope");
  if (x.error !== undefined) {
    const e = object(x.error); exactFields(e, ["code", "message"]);
    if (typeof e.code !== "string" || !MEDIA_ERRORS.has(e.code as MediaErrorCode) || typeof e.message !== "string" || e.message.length > 512) throw new Error("invalid media error");
    return x as unknown as MediaResponse;
  }
  const r = object(x.result);
  if (request?.method === "handshake") {
    exactFields(r, ["channel", "protocolVersion", "implementation", "capabilities"]);
    const caps = object(r.capabilities); exactFields(caps, ["decodeFrames"]);
    if (r.channel !== MEDIA_CHANNEL || r.protocolVersion !== MEDIA_PROTOCOL_VERSION || !["native", "test-double"].includes(String(r.implementation)) || typeof caps.decodeFrames !== "boolean" || (r.implementation === "test-double" && caps.decodeFrames)) throw new Error("invalid media handshake");
  } else if (request?.method === "cancel") {
    exactFields(r, ["jobId", "state"]);
    if (r.jobId !== request.jobId || !["cancelling", "cancelled"].includes(String(r.state))) throw new Error("invalid media cancellation result");
  } else {
    if (request?.method !== "decode") throw new Error("decode response has no matching request");
    exactFields(r, ["jobId", "state", "frames"]);
    if (r.jobId !== request.jobId || r.state !== "complete" || !Array.isArray(r.frames) || r.frames.length !== request.requestedBundleMs.length) throw new Error("invalid media decode result");
    const seen = new Set<number>();
    for (const [framePosition, raw] of r.frames.entries()) {
      const f = object(raw); exactFields(f, ["requestIndex", "requestedBundleMs", "effectiveVideoMs", "actualVideoMs", "clampedToEnd", "path", "width", "height", "byteLength", "sha256"], ["requestIndex", "requestedBundleMs", "effectiveVideoMs", "clampedToEnd", "path", "width", "height", "byteLength", "sha256"]);
      if (!positiveInt(f.requestIndex, r.frames.length) || f.requestIndex !== framePosition + 1 || seen.has(f.requestIndex as number) || typeof f.requestedBundleMs !== "number" || typeof f.effectiveVideoMs !== "number" || f.effectiveVideoMs < 0 || !absolute(f.path) || !f.path.endsWith(".png") || !positiveInt(f.width, MAX_EDGE) || !positiveInt(f.height, MAX_EDGE) || !positiveInt(f.byteLength, Number.MAX_SAFE_INTEGER) || typeof f.sha256 !== "string" || !/^[0-9a-f]{64}$/.test(f.sha256) || typeof f.clampedToEnd !== "boolean" || (f.actualVideoMs !== undefined && (typeof f.actualVideoMs !== "number" || !Number.isFinite(f.actualVideoMs) || f.actualVideoMs < 0))) throw new Error("invalid media frame reference");
      const outputRelative = relative(request.outputDirectory, f.path);
      if (!outputRelative || outputRelative === ".." || outputRelative.startsWith(`..${sep}`) || isAbsolute(outputRelative)) throw new Error("media frame reference escapes output directory");
      seen.add(f.requestIndex as number);
      const expectedIndex = f.requestIndex as number;
      const expectedBundleMs = request!.requestedBundleMs[expectedIndex - 1];
      const expectedVideoMs = expectedBundleMs - request!.source.videoOffsetMs;
      if (f.requestedBundleMs !== expectedBundleMs || f.effectiveVideoMs !== expectedVideoMs) throw new Error("media frame order or clock mapping mismatch");
    }
    if (seen.size !== r.frames.length) throw new Error("media frame inventory is incomplete");
    const limits = request!.limits;
    const pixels = (r.frames as Record<string, unknown>[]).reduce((sum, f) => sum + (f.width as number) * (f.height as number), 0);
    if (pixels > limits.maxTotalPixels || r.frames.some((raw) => { const f = raw as Record<string, unknown>; return (f.width as number) > limits.maxEdge || (f.height as number) > limits.maxEdge; })) throw new Error("media frame resource bounds exceeded");
  }
  return x as unknown as MediaResponse;
}

export interface MediaTransport { request(request: MediaRequest, timeoutMs?: number): Promise<MediaResponse>; close(): Promise<void> }
export class MediaRpcError extends Error { readonly code: string; constructor(code: string, message = `Engine media request failed (${code})`) { super(message); this.code = code; this.name = "MediaRpcError"; } }
export class MediaClient {
  readonly clientId = id();
  private implementation?: "native" | "test-double";
  private readonly transport: MediaTransport;
  private readonly options: { allowTestDouble?: boolean };
  constructor(transport: MediaTransport, options: { allowTestDouble?: boolean } = {}) { this.transport = transport; this.options = options; }
  async handshake(timeoutMs = 5000): Promise<MediaHandshake> {
    timeout(timeoutMs);
    const request = validateMediaRequest({channel: MEDIA_CHANNEL, protocolVersion: MEDIA_PROTOCOL_VERSION, clientId: this.clientId, rpcId: id(), method: "handshake"});
    const response = validateMediaResponse(await this.transport.request(request, timeoutMs), request.rpcId, request);
    if (response.error) throw new MediaRpcError(response.error.code);
    const handshake = response.result as MediaHandshake;
    if (handshake.implementation === "test-double" && this.options.allowTestDouble !== true) throw new Error("Test media peer requires explicit test-only opt-in");
    this.implementation = handshake.implementation;
    return handshake;
  }
  async decode(input: Omit<DecodeRequest, "channel" | "protocolVersion" | "clientId" | "rpcId" | "method" | "jobId"> & {jobId?: string}, timeoutMs = input.limits.deadlineMs): Promise<DecodeResult> {
    if (!this.implementation) throw new Error("media handshake required before decode");
    timeout(timeoutMs);
    const request = validateMediaRequest({ ...input, channel: MEDIA_CHANNEL, protocolVersion: MEDIA_PROTOCOL_VERSION, clientId: this.clientId, rpcId: id(), method: "decode", jobId: input.jobId ?? id() });
    const response = validateMediaResponse(await this.transport.request(request, timeoutMs), request.rpcId, request);
    if (response.error) throw new MediaRpcError(response.error.code);
    return response.result as DecodeResult;
  }
  async cancel(jobId: string, timeoutMs = 5000): Promise<"cancelling" | "cancelled"> {
    if (!this.implementation) throw new Error("media handshake required before cancel");
    timeout(timeoutMs);
    const request = validateMediaRequest({ channel: MEDIA_CHANNEL, protocolVersion: MEDIA_PROTOCOL_VERSION, clientId: this.clientId, rpcId: id(), method: "cancel", jobId });
    const response = validateMediaResponse(await this.transport.request(request, timeoutMs), request.rpcId, request);
    if (response.error) throw new MediaRpcError(response.error.code);
    return (response.result as {state:"cancelling"|"cancelled"}).state;
  }
  close() { return this.transport.close(); }
}

/** Explicit binary and runtime directory; never searches PATH or builds source. */
export function launchMediaWorker(executablePath: string, runtimeDirectory: string, shutdownTimeoutMs = 3000): MediaChildProcessTransport {
  if (!absolute(executablePath) || !absolute(runtimeDirectory)) throw new Error("media executable and runtime directory must be absolute");
  timeout(shutdownTimeoutMs);
  return new MediaChildProcessTransport(spawn(executablePath, ["--media-runtime-directory", runtimeDirectory, "--media-channel", MEDIA_CHANNEL, "--media-protocol-version", String(MEDIA_PROTOCOL_VERSION)], { cwd: dirname(executablePath), stdio: ["pipe", "pipe", "pipe"], shell: false }), shutdownTimeoutMs);
}
export class MediaChildProcessTransport implements MediaTransport {
  private readonly child: ChildProcessWithoutNullStreams;
  private readonly shutdownMs: number;
  private bytes = Buffer.alloc(0);
  private readonly pending = new Map<string, {resolve:(value:MediaResponse)=>void; reject:(error:Error)=>void; timer:ReturnType<typeof setTimeout>}>();
  private closed = false;
  private closing = false;
  private terminal?: Error;
  constructor(child: ChildProcessWithoutNullStreams, shutdownMs = 3000) {
    this.child = child;
    this.shutdownMs = shutdownMs;
    child.stdout.on("data", (chunk: Buffer) => this.onData(Buffer.from(chunk)));
    child.stdout.on("error", () => this.fail(new Error("media worker stdout failed")));
    child.stdin.on("error", () => this.fail(new Error("media worker stdin failed")));
    child.stderr.on("data", () => {});
    child.on("error", () => this.fail(new Error("media worker process failed")));
    child.on("exit", (code, signal) => { this.closed = true; this.fail(new Error(`media worker exited (${code ?? signal ?? "unknown"})`)); });
  }
  private onData(chunk: Buffer) {
    if (this.terminal) return;
    this.bytes = Buffer.concat([this.bytes, chunk]);
    if (this.bytes.length > MAX_LINE && this.bytes.indexOf(10) < 0) return this.fail(new Error("media response exceeds protocol limit"));
    while (true) {
      const nl = this.bytes.indexOf(10); if (nl < 0) break;
      if (nl > MAX_LINE) return this.fail(new Error("media response exceeds protocol limit"));
      let line = this.bytes.subarray(0, nl); this.bytes = this.bytes.subarray(nl + 1);
      if (line.at(-1) === 13) line = line.subarray(0, -1);
      let value: unknown;
      try { value = JSON.parse(new TextDecoder("utf-8", {fatal:true}).decode(line)); }
      catch { return this.fail(new Error("malformed media response")); }
      try {
        const x = object(value); exactFields(x, ["channel", "protocolVersion", "rpcId", "result", "error"], ["channel", "protocolVersion", "rpcId"]);
        if (x.channel !== MEDIA_CHANNEL || x.protocolVersion !== MEDIA_PROTOCOL_VERSION || typeof x.rpcId !== "string" || !hex(x.rpcId)) throw new Error("incompatible media peer");
        const pending = this.pending.get(x.rpcId); if (!pending) continue;
        clearTimeout(pending.timer); this.pending.delete(x.rpcId); pending.resolve(x as unknown as MediaResponse);
      } catch { this.fail(new Error("incompatible media peer or protocol version")); return; }
    }
    if (this.bytes.length > MAX_LINE) this.fail(new Error("media response exceeds protocol limit"));
  }
  request(request: MediaRequest, timeoutMs = 5000): Promise<MediaResponse> {
    timeout(timeoutMs);
    if (this.closed || this.closing || this.terminal) return Promise.reject(this.terminal ?? new Error("media worker disconnected"));
    if (this.pending.size >= 32) return Promise.reject(new Error("media worker RPC limit reached"));
    const line = encodeMediaRequest(request);
    if (this.pending.has(request.rpcId)) return Promise.reject(new Error("duplicate media RPC id"));
    return new Promise((resolvePromise, reject) => {
      const timer = setTimeout(() => { this.pending.delete(request.rpcId); reject(new Error("media RPC timed out; decode was not cancelled")); }, timeoutMs);
      this.pending.set(request.rpcId, {resolve:resolvePromise,reject,timer});
      this.child.stdin.write(line, (error) => { if (!error) return; const p = this.pending.get(request.rpcId); if (p) { clearTimeout(p.timer); this.pending.delete(request.rpcId); p.reject(new Error("media request write failed")); } });
    });
  }
  private fail(error: Error) { if (!this.terminal) this.terminal = error; for (const p of this.pending.values()) { clearTimeout(p.timer); p.reject(error); } this.pending.clear(); }
  async close(): Promise<void> {
    if (this.closed) return;
    this.closing = true;
    await new Promise<void>((resolvePromise) => {
      const eof = setTimeout(() => { if (!this.closed) this.child.kill("SIGTERM"); }, this.shutdownMs);
      const kill = setTimeout(() => { if (!this.closed) this.child.kill("SIGKILL"); }, this.shutdownMs * 2);
      const reap = setTimeout(() => { this.closed = true; resolvePromise(); }, this.shutdownMs * 3);
      this.child.once("exit", () => { clearTimeout(eof); clearTimeout(kill); clearTimeout(reap); this.closed = true; resolvePromise(); });
      this.child.stdin.end();
    });
    this.fail(new Error("media SDK stdin closed"));
  }
}
