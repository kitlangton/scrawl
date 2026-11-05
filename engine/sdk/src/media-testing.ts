import type { CancelRequest, DecodeRequest, MediaHandshakeRequest, MediaRequest, MediaResponse, MediaTransport } from "./media.ts";
import { encodeMediaRequest, MEDIA_CHANNEL, MEDIA_PROTOCOL_VERSION } from "./media.ts";

export type MediaTestScenario = "success" | "partial-decode-failure" | "cancelled" | "timeout" | "source-mismatch" | "output-exists";
/** Test-only peer. Returned paths/digests are synthetic references, never PNG proof. */
export class ReferenceMediaPeer implements MediaTransport {
  private closed = false;
  readonly requests: MediaRequest[] = [];
  private readonly scenario: MediaTestScenario;
  constructor(scenario: MediaTestScenario = "success") { this.scenario = scenario; }
  async request(request: MediaRequest): Promise<MediaResponse> {
    encodeMediaRequest(request);
    if (this.closed) throw new Error("test media peer closed");
    this.requests.push(request);
    if (request.method === "handshake") return {channel: MEDIA_CHANNEL, protocolVersion: MEDIA_PROTOCOL_VERSION, rpcId: request.rpcId, result: {channel: MEDIA_CHANNEL, protocolVersion: MEDIA_PROTOCOL_VERSION, implementation: "test-double", capabilities: {decodeFrames: false}}};
    if (request.method === "cancel") return {channel: MEDIA_CHANNEL, protocolVersion: MEDIA_PROTOCOL_VERSION, rpcId: request.rpcId, result: {jobId: request.jobId, state: "cancelled"}};
    if (this.scenario === "timeout") return new Promise(() => {});
    if (this.scenario !== "success") {
      const code = this.scenario === "source-mismatch" ? "source_mismatch" : this.scenario === "output-exists" ? "invalid_output" : this.scenario === "cancelled" ? "cancelled" : "decode_failed";
      return {channel: MEDIA_CHANNEL, protocolVersion: MEDIA_PROTOCOL_VERSION, rpcId: request.rpcId, error: {code, message: "synthetic test failure"}};
    }
    const req = request as DecodeRequest;
    return {channel: MEDIA_CHANNEL, protocolVersion: MEDIA_PROTOCOL_VERSION, rpcId: request.rpcId, result: {jobId: req.jobId, state: "complete", frames: req.requestedBundleMs.map((requestedBundleMs, index) => ({requestIndex: index + 1, requestedBundleMs, effectiveVideoMs: requestedBundleMs - req.source.videoOffsetMs, clampedToEnd: false, path: `${req.outputDirectory}/frame-${String(index + 1).padStart(3,"0")}.png`, width: Math.min(640, req.limits.maxEdge), height: Math.min(480, req.limits.maxEdge), byteLength: 128, sha256: "a".repeat(64)}))}};
  }
  async close() { this.closed = true; }
}
