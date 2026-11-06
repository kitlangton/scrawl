import { spawn } from "node:child_process";
import { randomBytes } from "node:crypto";
import { dirname, isAbsolute, resolve } from "node:path";
import { exact, fail, optionalExact, record } from "../capture-documents/common.ts";
import type { MaterialsDocument } from "../capture-documents/types.ts";
import { MediaProcessTransport, type MediaCloseOutcome } from "../media.ts";
import { prepareMedia2Request, encodeContactSheetLayout } from "./request.ts";
import { validateDerivative2Document } from "./receipt.ts";
import { encodeMedia2Rpc, MEDIA2_MAX_LINE, validateMedia2Handshake, validateMedia2Response, validateMedia2Token, type Media2ErrorCode, type Media2Handshake, type Media2RpcRequest, type Media2RpcResponse } from "./protocol.ts";
import type { Derivative2DocumentInput, Media2JobRequest, Media2PreparedRequest } from "./types.ts";

const id = () => randomBytes(16).toString("hex");
const timeout = (v: number) => { if (!Number.isSafeInteger(v) || v < 1 || v > 60_000) throw new RangeError("media2 timeout must be 1..60000ms"); };
export interface Media2Transport { request(request: Media2RpcRequest, timeoutMs?: number): Promise<Media2RpcResponse>; close(): Promise<MediaCloseOutcome> }
export interface Media2JobInput { ownerToken: string; request: Media2JobRequest; materials: MaterialsDocument }
export type Media2JobStatus = { jobId: string; requestSha256: string } & (
  | { state: "running" | "cancelled" }
  | { state: "failed"; error: { code: Media2ErrorCode } }
  | { state: "complete"; derivative: Derivative2DocumentInput }
);
export class Media2PreflightError extends Error {
  readonly dispatched = false;
  constructor() { super("Invalid retained-media input; no request was dispatched"); this.name = "Media2PreflightError"; }
}
export class Media2RpcError extends Error {
  readonly code: Media2ErrorCode;
  readonly dispatched: boolean;
  constructor(code: Media2ErrorCode, dispatched = true) { super(`Engine retained-media request failed (${code})`); this.name = "Media2RpcError"; this.code = code; this.dispatched = dispatched; }
}
export class Media2TransportError extends Error {
  readonly dispatched = "unknown" as const;
  constructor() { super("Media2 transport outcome unknown; the job was not cancelled"); this.name = "Media2TransportError"; }
}
/** Offline preflight for the original request before a caller persists dispatch.
 * This is not native authentication, admission or filesystem validation. */
export function preflightMedia2Dispatch(value: Media2JobInput, timeoutMs = 5000): { input: Media2JobInput; prepared: Media2PreparedRequest } {
  try {
    timeout(timeoutMs);
    const input = structuredClone(value);
    exact(record(input, "media2 input"), ["ownerToken", "request", "materials"], "media2 input");
    validateMedia2Token(input.ownerToken);
    const prepared = prepareMedia2Request(input.request, input.materials);
    encodeMedia2Rpc({ channel: "media", protocolVersion: 2, clientId: "0".repeat(32), rpcId: "0".repeat(32), method: "derive", captureId: prepared.request.materials.captureId, ownerToken: input.ownerToken, jobId: prepared.request.jobId, requestSha256: prepared.sha256, requestJson: Buffer.from(prepared.bytes).toString("utf8") });
    return { input: { ...input, request: prepared.request }, prepared };
  } catch { throw new Media2PreflightError(); }
}
/** Explicitly negotiated media/2; media/1 never receives these methods. */
export class Media2Client {
  readonly clientId = id();
  private hello?: Media2Handshake;
  private readonly transport: Media2Transport;
  private readonly allowTestDouble: boolean;
  constructor(transport: Media2Transport, options: { allowTestDouble?: boolean } = {}) { this.transport = transport; this.allowTestDouble = options.allowTestDouble === true; }
  private async rpc(request: Media2RpcRequest, timeoutMs: number): Promise<unknown> {
    timeout(timeoutMs); encodeMedia2Rpc(request);
    let raw: unknown;
    try { raw = await this.transport.request(request, timeoutMs); } catch { throw new Media2TransportError(); }
    const response = validateMedia2Response(raw, request.rpcId);
    if (response.error) throw new Media2RpcError(response.error.code);
    return response.result;
  }
  async handshake(timeoutMs = 5000): Promise<Media2Handshake> {
    const result = validateMedia2Handshake(await this.rpc({ channel: "media", protocolVersion: 2, clientId: this.clientId, rpcId: id(), method: "handshake" }, timeoutMs));
    if (result.implementation === "test-double" && !this.allowTestDouble) throw new Error("Test media2 peer requires explicit opt-in");
    this.hello = structuredClone(result);
    return result;
  }
  async derive(input: Media2JobInput, timeoutMs = 5000): Promise<Media2JobStatus> { return this.job("derive", input, timeoutMs); }
  async inspect(input: Media2JobInput, timeoutMs = 5000): Promise<Media2JobStatus> { return this.job("inspect", input, timeoutMs); }
  async cancel(input: Media2JobInput, timeoutMs = 5000): Promise<Media2JobStatus> { return this.job("cancel", input, timeoutMs); }
  private async job(method: "derive" | "inspect" | "cancel", value: Media2JobInput, timeoutMs: number): Promise<Media2JobStatus> {
    if (!this.hello) throw new Error("media2 handshake required");
    const { input, prepared } = preflightMedia2Dispatch(value, timeoutMs);
    if (method === "derive" && (!this.hello.capabilities.retainedFrames || !this.hello.capabilities.nativeJobAuthority || (input.request.contactSheet && !this.hello.capabilities.contactSheet))) throw new Media2RpcError("unavailable", false);
    const base = { channel: "media" as const, protocolVersion: 2 as const, clientId: this.clientId, rpcId: id(), captureId: input.request.materials.captureId, ownerToken: input.ownerToken, jobId: input.request.jobId, requestSha256: prepared.sha256 };
    const request: Media2RpcRequest = method === "derive" ? { ...base, method, requestJson: Buffer.from(prepared.bytes).toString("utf8") } : { ...base, method };
    const result = record(await this.rpc(request, timeoutMs), "media2 job status");
    optionalExact(result, ["jobId", "requestSha256", "state", "derivative", "error"], ["jobId", "requestSha256", "state"], "media2 job status");
    if (result.jobId !== input.request.jobId || result.requestSha256 !== prepared.sha256) fail("media2 job original binding mismatch");
    const common = { jobId: input.request.jobId, requestSha256: prepared.sha256 };
    if (result.state === "complete") {
      if (this.hello.implementation !== "native" || !this.hello.capabilities.nativeJobAuthority || "error" in result) fail("media2 completion lacks native authority capability");
      const derivative = record(result.derivative, "media2 derivative bytes"); exact(derivative, ["envelope", "receiptJson"], "media2 derivative bytes");
      if (typeof derivative.receiptJson !== "string" || Buffer.byteLength(derivative.receiptJson) > MEDIA2_MAX_LINE) fail("media2 receipt metadata exceeds wire bounds");
      const document: Derivative2DocumentInput = { envelope: derivative.envelope, bytes: Buffer.from(derivative.receiptJson, "utf8"), request: input.request, layoutBytes: input.request.contactSheet ? encodeContactSheetLayout(input.request.contactSheet) : null };
      document.envelope = validateDerivative2Document(document, input.materials);
      return { ...common, state: "complete", derivative: document };
    }
    if ("derivative" in result) fail("noncomplete media2 job carries a derivative");
    if (result.state === "failed") {
      const response = validateMedia2Response({ channel: "media", protocolVersion: 2, rpcId: request.rpcId, error: result.error }, request.rpcId);
      return { ...common, state: "failed", error: response.error! };
    }
    if ((result.state !== "running" && result.state !== "cancelled") || "error" in result) fail("invalid media2 terminal state");
    return { ...common, state: result.state };
  }
  close(): Promise<MediaCloseOutcome> { return this.transport.close(); }
}
export function launchMedia2Worker(executable: string, runtimeDirectory: string, shutdownMs = 3000): Media2Transport {
  if (!isAbsolute(executable) || resolve(executable) !== executable || !isAbsolute(runtimeDirectory) || resolve(runtimeDirectory) !== runtimeDirectory) throw new Error("media2 executable and runtime directory must be normalized absolute paths");
  timeout(shutdownMs);
  return new MediaProcessTransport<Media2RpcRequest, Media2RpcResponse>(spawn(executable, ["--media-runtime-directory", runtimeDirectory, "--media-channel", "media", "--media-protocol-version", "2"], { cwd: dirname(executable), stdio: ["pipe", "pipe", "pipe"], shell: false }), shutdownMs, { version: 2, encode: encodeMedia2Rpc, maxLine: MEDIA2_MAX_LINE });
}
