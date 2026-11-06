import { createHash } from "node:crypto";
import { exact, fail, id, optionalExact, record, sha } from "../capture-documents/common.ts";

export const MEDIA2_CHANNEL = "media" as const;
export const MEDIA2_PROTOCOL_VERSION = 2 as const;
export const MEDIA2_MAX_LINE = 1024 * 1024;
const ERRORS = ["malformed_request", "unsupported_channel", "unsupported_version", "permission_denied", "identity_conflict", "unavailable", "not_found", "job_not_found", "busy", "lease_busy", "invalid_materials", "invalid_output", "source_mismatch", "source_changed", "limits_exceeded", "decode_failed", "cancelled", "deadline_exceeded", "owner_lost", "internal"] as const;
export type Media2ErrorCode = typeof ERRORS[number];
export interface Media2Handshake {
  channel: "media";
  protocolVersion: 2;
  implementation: "native" | "test-double";
  capabilities: { retainedFrames: boolean; contactSheet: boolean; nativeJobAuthority: boolean };
}
interface Base { channel: "media"; protocolVersion: 2; clientId: string; rpcId: string }
interface Authority { captureId: string; ownerToken: string; jobId: string; requestSha256: string }
export type Media2RpcRequest = Base & ({ method: "handshake" } | (Authority & ({ method: "derive"; requestJson: string } | { method: "inspect" | "cancel" })));
export interface Media2RpcResponse { channel: "media"; protocolVersion: 2; rpcId: string; result?: unknown; error?: { code: Media2ErrorCode } }
export function validateMedia2Token(value: unknown): asserts value is string {
  if (typeof value !== "string" || !/^[0-9a-f]{64}$/.test(value)) fail("invalid media2 authority token");
}
/** Internal framing checks; the public client additionally runs full document
 * preflight, and the native peer independently authenticates and validates. */
export function encodeMedia2Rpc(request: Media2RpcRequest): string {
  const r = record(request, "media2 RPC");
  const base = ["channel", "protocolVersion", "clientId", "rpcId", "method"];
  const auth = ["captureId", "ownerToken", "jobId", "requestSha256"];
  exact(r, r.method === "handshake" ? base : r.method === "derive" ? [...base, ...auth, "requestJson"] : [...base, ...auth], "media2 RPC");
  if (r.channel !== MEDIA2_CHANNEL || r.protocolVersion !== MEDIA2_PROTOCOL_VERSION || !["handshake", "derive", "inspect", "cancel"].includes(r.method as string)) fail("unsupported media2 RPC");
  id(r.clientId, "media2 clientId"); id(r.rpcId, "media2 rpcId");
  if (r.method !== "handshake") {
    id(r.captureId, "media2 captureId"); validateMedia2Token(r.ownerToken); id(r.jobId, "media2 jobId"); sha(r.requestSha256, "media2 requestSha256");
    if (r.method === "derive") {
      if (typeof r.requestJson !== "string" || Buffer.byteLength(r.requestJson) > 64 * 1024 || createHash("sha256").update(r.requestJson).digest("hex") !== r.requestSha256) fail("invalid exact media2 request bytes");
    }
  }
  const line = JSON.stringify(request) + "\n";
  if (Buffer.byteLength(line) > MEDIA2_MAX_LINE) fail("media2 request exceeds wire limit");
  return line;
}
export function validateMedia2Response(value: unknown, rpcId: string): Media2RpcResponse {
  const r = record(value, "media2 response");
  optionalExact(r, ["channel", "protocolVersion", "rpcId", "result", "error"], ["channel", "protocolVersion", "rpcId"], "media2 response");
  if (r.channel !== MEDIA2_CHANNEL || r.protocolVersion !== MEDIA2_PROTOCOL_VERSION || r.rpcId !== rpcId || (("result" in r) === ("error" in r))) fail("incompatible media2 response");
  if ("error" in r) {
    const e = record(r.error, "media2 error"); exact(e, ["code"], "media2 error");
    if (!ERRORS.includes(e.code as Media2ErrorCode)) fail("unknown media2 error code");
  }
  return r as unknown as Media2RpcResponse;
}
export function validateMedia2Handshake(value: unknown): Media2Handshake {
  const h = record(value, "media2 handshake"); exact(h, ["channel", "protocolVersion", "implementation", "capabilities"], "media2 handshake");
  if (h.channel !== MEDIA2_CHANNEL || h.protocolVersion !== MEDIA2_PROTOCOL_VERSION || !["native", "test-double"].includes(h.implementation as string)) fail("unsupported media2 handshake");
  const c = record(h.capabilities, "media2 capabilities"); exact(c, ["retainedFrames", "contactSheet", "nativeJobAuthority"], "media2 capabilities");
  if (Object.values(c).some(v => typeof v !== "boolean") || (c.contactSheet && !c.retainedFrames) || (c.retainedFrames && !c.nativeJobAuthority) || (h.implementation === "test-double" && Object.values(c).some(Boolean))) fail("invalid media2 capability claims");
  return structuredClone(h) as unknown as Media2Handshake;
}
