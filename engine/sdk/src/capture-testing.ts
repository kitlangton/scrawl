import type {
  CaptureHandshake,
  CaptureRequest,
  CaptureResponse,
  CaptureTransport,
} from "./capture.ts";
import { encodeCaptureRequest } from "./capture.ts";

export type CaptureScenario =
  | "handshake-unavailable"
  | "materials-ready"
  | "publishing"
  | "cancelled"
  | "malformed-late-reply"
  | "wrong-version"
  | "timeout";
/** Explicit protocol test double. It never reports capture capability or native facts. */
export class ReferenceCapturePeer implements CaptureTransport {
  readonly handshake: CaptureHandshake = {
    channel: "capture",
    protocolVersion: 2,
    implementation: "test-double",
    readiness: "unavailable",
    capabilities: {
      sourceDiscovery: false,
      displayCapture: false,
      areaCapture: false,
      audioCapture: false,
      contextFacts: false,
      interactionFacts: false,
      candidates: false,
      transcript: false,
      strokes: false,
      publication: false,
      retainedMaterials: false,
    },
  };
  private closed = false;
  private scenario: CaptureScenario;
  constructor(scenario: CaptureScenario = "handshake-unavailable") {
    this.scenario = scenario;
  }
  async request(request: CaptureRequest): Promise<CaptureResponse> {
    encodeCaptureRequest(request);
    if (this.closed) throw new Error("reference capture peer closed");
    if (this.scenario === "timeout") return new Promise(() => {});
    if (this.scenario === "wrong-version")
      return {
        channel: "capture",
        protocolVersion: 1 as 2,
        rpcId: request.rpcId,
        result: this.handshake,
      };
    if (this.scenario === "malformed-late-reply")
      return {
        channel: "capture",
        protocolVersion: 2,
        rpcId: "0".repeat(32),
        result: this.handshake,
      };
    if (request.method === "handshake")
      return {
        channel: "capture",
        protocolVersion: 2,
        rpcId: request.rpcId,
        result: this.handshake,
      };
    if (request.method === "discoverSources")
      return {
        channel: "capture",
        protocolVersion: 2,
        rpcId: request.rpcId,
        result: {
          state: "unavailable",
          sources: [],
          reason: "test peer has no native source provider",
        },
      };
    if (
      request.method === "inspect" &&
      ["materials-ready", "publishing", "cancelled"].includes(this.scenario)
    ) {
      const manifest = {
        schema: "apparent.capture-materials/1",
        key: {
          captureId: request.captureId,
          materialsId: "d".repeat(32),
          revision: "a".repeat(64),
        },
        root: "/synthetic/test-materials",
        leaseId: "e".repeat(32),
        payloadFile: {
          relativePath: "materials.json",
          identity: { device: "test", inode: "1", size: 1, modifiedNs: "1" },
          sha256: "a".repeat(64),
        },
      };
      const result =
        this.scenario === "cancelled"
          ? { captureId: request.captureId, state: "cancelled" }
          : this.scenario === "publishing"
            ? {
                captureId: request.captureId,
                state: "publishing",
                materials: manifest,
                publicationId: "f".repeat(32),
                planSha256: "1".repeat(64),
              }
            : {
                captureId: request.captureId,
                state: "materials-ready",
                materials: manifest,
              };
      return {
        channel: "capture",
        protocolVersion: 2,
        rpcId: request.rpcId,
        result,
      };
    }
    return {
      channel: "capture",
      protocolVersion: 2,
      rpcId: request.rpcId,
      error: {
        code: "unavailable",
        message: "capture/2 test scenario has no native provider",
      },
    };
  }
  async close() {
    this.closed = true;
    return "already-exited" as const;
  }
}
