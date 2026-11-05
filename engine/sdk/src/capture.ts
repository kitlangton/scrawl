import { spawn, type ChildProcessWithoutNullStreams } from "node:child_process";
import { randomBytes } from "node:crypto";
import { dirname, isAbsolute, resolve } from "node:path";
import { isDeepStrictEqual } from "node:util";
import type {
  MaterialsDocument,
  MaterialsKey,
  PublishEnvelope,
  InvocationFact,
} from "./capture-documents/types.ts";
import { validatePublicationPlan } from "./capture-documents/publication.ts";
import type { DerivativeDocumentInput } from "./capture-documents/types.ts";
import { MAX_DOCUMENT_BYTES, relativePath, validateSealedFile } from "./capture-documents/common.ts";
export {
  validateMaterialsDocument,
  validateFactFile,
} from "./capture-documents/index.ts";
export type {
  MaterialsDocument,
  MaterialsManifest,
  MaterialsPayload,
  MaterialsKey,
  ContextFacts,
  InteractionFacts,
  CandidateFacts,
  TranscriptFacts,
  StrokeFacts,
  PublicationPlan,
  PublishEnvelope,
  InvocationFact,
} from "./capture-documents/index.ts";

export const CAPTURE_CHANNEL = "capture" as const;
export const CAPTURE_PROTOCOL_VERSION = 2 as const;
export const CAPTURE_MAX_LINE_BYTES = 64 * 1024;
const MAX_TIMEOUT = 60_000;
const ERROR_CODES = ["malformed_request", "unsupported_version", "unsupported_channel", "identity_conflict", "not_ready", "not_found", "busy", "permission_denied", "unavailable", "source_changed", "source_mismatch", "invalid_materials", "invalid_publication", "lease_busy", "cancelled", "deadline_exceeded", "internal"] as const;
export type CaptureErrorCode = typeof ERROR_CODES[number];
export class CaptureRpcError extends Error {
  readonly code: CaptureErrorCode;
  constructor(code: CaptureErrorCode) {
    super(`Engine capture request failed (${code})`);
    this.code = code;
    this.name = "CaptureRpcError";
  }
}
function errorCode(value: unknown): asserts value is CaptureErrorCode {
  if (!ERROR_CODES.includes(value as CaptureErrorCode)) throw new Error("invalid capture error code");
}
const hex = (n: number) => new RegExp(`^[0-9a-f]{${n}}$`);
const id = (v: unknown): v is string =>
  typeof v === "string" && hex(32).test(v);
const hash = (v: unknown): v is string =>
  typeof v === "string" && hex(64).test(v);
const object = (v: unknown, label: string): Record<string, unknown> => {
  if (!v || typeof v !== "object" || Array.isArray(v))
    throw new Error(`${label} must be an object`);
  return v as Record<string, unknown>;
};
const exact = (
  x: Record<string, unknown>,
  names: readonly string[],
  required = names,
) => {
  if (
    Object.keys(x).some((k) => !names.includes(k)) ||
    required.some((k) => !(k in x))
  )
    throw new Error("invalid capture/2 fields");
};
function text(v: unknown, label: string, max = 4096): asserts v is string {
  if (typeof v !== "string" || !v || v.length > max || v.includes("\0"))
    throw new Error(`${label} invalid`);
}
function abs(v: unknown, label: string): asserts v is string {
  text(v, label);
  if (!isAbsolute(v) || resolve(v) !== v)
    throw new Error(`${label} must be normalized absolute path`);
}
const timeout = (n: number) => {
  if (!Number.isSafeInteger(n) || n < 1 || n > MAX_TIMEOUT)
    throw new RangeError(`timeout must be 1..${MAX_TIMEOUT} ms`);
};

export type CaptureState =
  | "starting"
  | "recording"
  | "stopping"
  | "materials-ready"
  | "publishing"
  | "saved"
  | "cancelled"
  | "failed";
export interface CaptureHandshake {
  channel: "capture";
  protocolVersion: 2;
  implementation: "native" | "test-double";
  readiness: "ready" | "unavailable";
  capabilities: {
    sourceDiscovery: boolean;
    displayCapture: boolean;
    areaCapture: boolean;
    audioCapture: boolean;
    contextFacts: boolean;
    interactionFacts: boolean;
    candidates: boolean;
    transcript: boolean;
    strokes: boolean;
    publication: boolean;
    retainedMaterials: boolean;
  };
}
export interface SourceDescriptor {
  id: string;
  generation: string;
  kind: "display" | "area";
  displayId: number;
  bounds: {
    x: number;
    y: number;
    width: number;
    height: number;
    scale: number;
  };
  name?: string;
}
export interface CaptureOptions {
  audio: "none" | "microphone";
  localOnly: true;
}
export interface CaptureStatus {
  captureId: string;
  state: CaptureState;
  materials?: MaterialsDocument["manifest"];
  publicationId?: string;
  planSha256?: string;
  publication?: PublicationReceipt;
  error?: { code: string };
}
export interface PublicationReceipt {
  captureId: string;
  materials: MaterialsKey;
  publicationId: string;
  planSha256: string;
  bundleId: string;
  bundlePath: string;
  artifactVersion: 1;
  artifactProvenance: "native";
  manifestSha256: string;
  inventorySha256: string;
}
/** Restore this binding with the publication ID after reconnect. */
export interface PublicationExpectation {
  materials: MaterialsKey;
  bundleId: string;
  bundlePath: string;
  manifestSha256: string;
}
interface Base {
  channel: "capture";
  protocolVersion: 2;
  clientId: string;
  rpcId: string;
}
export type CaptureRequest = Base &
  (
    | { method: "handshake" }
    | { method: "discoverSources" }
    | {
        method: "start";
        ownerToken: string;
        captureId: string;
        bundleId: string;
        bundlePath: string;
        source: SourceDescriptor;
        options: CaptureOptions;
        invocation: InvocationFact;
      }
    | {
        method: "inspect" | "stop" | "cancel";
        ownerToken: string;
        captureId: string;
      }
    | {
        method: "releaseMaterials";
        ownerToken: string;
        captureId: string;
        materialsId: string;
        leaseId: string;
      }
    | {
        method: "publish";
        ownerToken: string;
        captureId: string;
        leaseId: string;
        publicationId: string;
        materials: MaterialsKey;
        stagingRoot: string;
        planFile: {
          relativePath: string;
          identity: {
            device: string;
            inode: string;
            size: number;
            modifiedNs: string;
          };
          sha256: string;
        };
        planSha256: string;
      }
    | {
        method: "inspectPublication";
        ownerToken: string;
        captureId: string;
        publicationId: string;
        planSha256: string;
      }
  );
export interface CaptureResponse {
  channel: "capture";
  protocolVersion: 2;
  rpcId: string;
  result?: unknown;
  error?: { code: string; message: string };
}
export interface CaptureTransport {
  request(
    request: CaptureRequest,
    timeoutMs?: number,
  ): Promise<CaptureResponse>;
  close(): Promise<CaptureCloseOutcome>;
}
export type CaptureCloseOutcome =
  | "already-exited"
  | "eof-exited"
  | "exit-observed"
  | "signal-exited"
  | "kill-exited"
  | "signal-requested-after-timeout"
  | "kill-requested-after-timeout";
export interface SourceDiscovery {
  state: "available" | "unavailable";
  sources: SourceDescriptor[];
  reason?: string;
}

function envelope(v: unknown, rpcId: string): CaptureResponse {
  const x = object(v, "response");
  exact(
    x,
    ["channel", "protocolVersion", "rpcId", "result", "error"],
    ["channel", "protocolVersion", "rpcId"],
  );
  if (
    x.channel !== CAPTURE_CHANNEL ||
    x.protocolVersion !== CAPTURE_PROTOCOL_VERSION ||
    x.rpcId !== rpcId ||
    (x.result === undefined) === (x.error === undefined)
  )
    throw new Error("invalid capture/2 response envelope");
  if (x.error !== undefined) {
    const e = object(x.error, "error");
    exact(e, ["code", "message"]);
    errorCode(e.code);
    text(e.message, "error message", 256);
    throw new CaptureRpcError(e.code);
  }
  return x as unknown as CaptureResponse;
}
export function encodeCaptureRequest(request: CaptureRequest): string {
  validateCaptureRequest(request);
  const line = JSON.stringify(request);
  if (Buffer.byteLength(line, "utf8") > CAPTURE_MAX_LINE_BYTES)
    throw new Error("capture/2 request exceeds 64 KiB");
  return `${line}\n`;
}
export function validateCaptureRequest(
  v: unknown,
): asserts v is CaptureRequest {
  const x = object(v, "request");
  if (x.channel !== CAPTURE_CHANNEL || x.protocolVersion !== 2)
    throw new Error("unsupported capture channel or protocol version");
  text(x.clientId, "clientId", 32);
  if (!id(x.clientId) || !id(x.rpcId))
    throw new Error("invalid capture RPC identity");
  const method = x.method;
  const fields: Record<string, string[]> = {
    handshake: ["channel", "protocolVersion", "clientId", "rpcId", "method"],
    discoverSources: [
      "channel",
      "protocolVersion",
      "clientId",
      "rpcId",
      "method",
    ],
    start: [
      "channel",
      "protocolVersion",
      "clientId",
      "rpcId",
      "method",
      "ownerToken",
      "captureId",
      "bundleId",
      "bundlePath",
      "source",
      "options",
      "invocation",
    ],
    inspect: [
      "channel",
      "protocolVersion",
      "clientId",
      "rpcId",
      "method",
      "ownerToken",
      "captureId",
    ],
    stop: [
      "channel",
      "protocolVersion",
      "clientId",
      "rpcId",
      "method",
      "ownerToken",
      "captureId",
    ],
    cancel: [
      "channel",
      "protocolVersion",
      "clientId",
      "rpcId",
      "method",
      "ownerToken",
      "captureId",
    ],
    publish: [
      "channel",
      "protocolVersion",
      "clientId",
      "rpcId",
      "method",
      "ownerToken",
      "captureId",
      "leaseId",
      "publicationId",
      "materials",
      "stagingRoot",
      "planFile",
      "planSha256",
    ],
    inspectPublication: [
      "channel",
      "protocolVersion",
      "clientId",
      "rpcId",
      "method",
      "ownerToken",
      "captureId",
      "publicationId",
      "planSha256",
    ],
    releaseMaterials: [
      "channel",
      "protocolVersion",
      "clientId",
      "rpcId",
      "method",
      "ownerToken",
      "captureId",
      "materialsId",
      "leaseId",
    ],
  };
  if (typeof method !== "string" || !fields[method])
    throw new Error("unknown capture/2 method");
  exact(x, fields[method]);
  if (method === "start") {
    if (!id(x.captureId) || !hex(64).test(String(x.ownerToken)))
      throw new Error("invalid start authority");
    text(x.bundleId, "bundleId", 255);
    if (
      x.bundleId === "." ||
      x.bundleId === ".." ||
      /[\\/\p{Cc}]/u.test(x.bundleId)
    )
      throw new Error("invalid bundleId");
    abs(x.bundlePath, "bundlePath");
    const s = object(x.source, "source");
    exact(
      s,
      ["id", "generation", "kind", "displayId", "bounds", "name"],
      ["id", "generation", "kind", "displayId", "bounds"],
    );
    text(s.id, "source id");
    if (!id(s.generation) || !["display", "area"].includes(String(s.kind)))
      throw new Error("invalid source identity");
    const o = object(x.options, "options");
    exact(o, ["audio", "localOnly"]);
    if (
      !["none", "microphone"].includes(String(o.audio)) ||
      o.localOnly !== true
    )
      throw new Error("invalid capture options");
  } else if (!["handshake", "discoverSources"].includes(String(method))) {
    if (!id(x.captureId) || !hex(64).test(String(x.ownerToken)))
      throw new Error("invalid capture authority");
    if (method === "publish" || method === "inspectPublication") {
      if (!id(x.publicationId) || !hash(x.planSha256))
        throw new Error("invalid publication identity");
    }
    if (method === "publish") {
      if (!id(x.leaseId)) throw new Error("invalid lease identity");
      abs(x.stagingRoot, "stagingRoot");
      const key = object(x.materials, "materials key");
      exact(key, ["captureId", "materialsId", "revision"]);
      if (
        !id(key.captureId) ||
        !id(key.materialsId) ||
        !hash(key.revision) ||
        key.captureId !== x.captureId
      )
        throw new Error("invalid materials binding");
      const f = object(x.planFile, "plan file");
      exact(f, ["relativePath", "identity", "sha256"]);
      relativePath(f.relativePath, "plan file relative path");
      validateSealedFile(f, "plan file");
      if (!hash(f.sha256) || f.sha256 !== x.planSha256)
        throw new Error("plan hash does not bind plan file");
      const fi = object(f.identity, "plan file identity");
      exact(fi, ["device", "inode", "size", "modifiedNs"]);
      text(fi.device, "plan file device");
      text(fi.inode, "plan file inode");
      if (!Number.isSafeInteger(fi.size) || Number(fi.size) < 1 || Number(fi.size) > MAX_DOCUMENT_BYTES)
        throw new Error("invalid plan file size");
      text(fi.modifiedNs, "plan file modifiedNs");
    }
    if (method === "releaseMaterials" && (!id(x.materialsId) || !id(x.leaseId)))
      throw new Error("invalid materials release identity");
  }
  if (method === "start") {
    validateSource(x.source);
    validateStartInvocation(x.invocation);
  }
}

export class CaptureClient {
  readonly clientId = randomBytes(16).toString("hex");
  private readonly transport: CaptureTransport;
  private allowTestDouble: boolean;
  private handshaken = false;
  private testDouble = false;
  private handshakeValue?: CaptureHandshake;
  private advertisedSources = new Map<string, SourceDescriptor>();
  constructor(
    transport: CaptureTransport,
    options: { allowTestDouble?: boolean } = {},
  ) {
    this.transport = transport;
    this.allowTestDouble = options.allowTestDouble === true;
  }
  private async rpc(
    method: CaptureRequest["method"],
    params: Record<string, unknown> = {},
    timeoutMs = 5000,
  ): Promise<unknown> {
    timeout(timeoutMs);
    if (["channel", "protocolVersion", "clientId", "rpcId", "method"].some(key => Object.hasOwn(params, key))) throw new Error("capture input contains reserved envelope fields");
    const request = {
      ...structuredClone(params),
      channel: CAPTURE_CHANNEL,
      protocolVersion: CAPTURE_PROTOCOL_VERSION,
      clientId: this.clientId,
      rpcId: randomBytes(16).toString("hex"),
      method,
    } as CaptureRequest;
    validateCaptureRequest(request);
    let timer: NodeJS.Timeout | undefined;
    try {
      const response = await Promise.race([
        Promise.resolve().then(() => this.transport.request(request, timeoutMs)).catch(() => {
          throw new Error("capture transport failed; operation outcome is unknown");
        }),
        new Promise<never>((_, reject) => {
          timer = setTimeout(
            () =>
              reject(
                new Error("capture RPC timed out; operation was not cancelled"),
              ),
            timeoutMs,
          );
        }),
      ]);
      return envelope(response, request.rpcId).result;
    } finally {
      if (timer) clearTimeout(timer);
    }
  }
  async handshake(timeoutMs = 5000): Promise<CaptureHandshake> {
    const v = object(await this.rpc("handshake", {}, timeoutMs), "handshake");
    exact(v, [
      "channel",
      "protocolVersion",
      "implementation",
      "readiness",
      "capabilities",
    ]);
    const c = object(v.capabilities, "capabilities");
    exact(c, [
      "sourceDiscovery",
      "displayCapture",
      "areaCapture",
      "audioCapture",
      "contextFacts",
      "interactionFacts",
      "candidates",
      "transcript",
      "strokes",
      "publication",
      "retainedMaterials",
    ]);
    if (
      v.channel !== "capture" ||
      v.protocolVersion !== 2 ||
      !["native", "test-double"].includes(String(v.implementation)) ||
      !["ready", "unavailable"].includes(String(v.readiness)) ||
      Object.values(c).some((b) => typeof b !== "boolean")
    )
      throw new Error("invalid capture/2 handshake");
    if (
      v.implementation === "test-double" &&
      (v.readiness !== "unavailable" || Object.values(c).some(Boolean))
    )
      throw new Error("test double cannot claim capture readiness");
    if (v.implementation === "test-double" && !this.allowTestDouble)
      throw new Error("test Engine requires explicit /capture/testing opt-in");
    if (v.readiness === "ready" && (!c.sourceDiscovery || !(c.displayCapture || c.areaCapture) || !c.retainedMaterials)) throw new Error("capture readiness requires source and retained-material capabilities");
    this.handshaken = true;
    this.testDouble = v.implementation === "test-double";
    this.handshakeValue = structuredClone(v) as unknown as CaptureHandshake;
    this.advertisedSources.clear();
    return structuredClone(this.handshakeValue);
  }
  async discoverSources(timeoutMs = 5000): Promise<SourceDiscovery> {
    this.requireHandshake();
    if (!this.handshakeValue?.capabilities.sourceDiscovery) {
      this.advertisedSources.clear();
      return { state: "unavailable", sources: [], reason: "source discovery is unavailable" };
    }
    const value = object(
      await this.rpc("discoverSources", {}, timeoutMs),
      "source discovery",
    );
    exact(value, ["state", "sources", "reason"], ["state", "sources"]);
    if (
      !Array.isArray(value.sources) ||
      (value.state !== "available" && value.state !== "unavailable") ||
      (value.state === "unavailable" && value.sources.length !== 0)
    )
      throw new Error("invalid source discovery");
    if (value.reason !== undefined)
      text(value.reason, "source discovery reason", 256);
    const sources = value.sources.map(validateSource);
    this.advertisedSources.clear();
    for (const source of sources) {
      if (this.advertisedSources.has(source.id))
        throw new Error("duplicate discovered source identity");
      this.advertisedSources.set(source.id, structuredClone(source));
    }
    return {
      state: value.state as SourceDiscovery["state"],
      sources,
      ...(value.reason === undefined ? {} : { reason: value.reason as string }),
    };
  }
  async start(
    input: {
      ownerToken: string;
      captureId: string;
      bundleId: string;
      bundlePath: string;
      source: SourceDescriptor;
      options: CaptureOptions;
      invocation: InvocationFact;
    },
    timeoutMs = 5000,
  ): Promise<CaptureStatus> {
    this.requireHandshake();
    input = structuredClone(input);
    if (this.handshakeValue?.readiness !== "ready")
      throw new Error("capture provider is unavailable");
    const source = this.advertisedSources.get(input.source.id);
    if (!source || !isDeepStrictEqual(source, input.source))
      throw new Error(
        "start source differs from the advertised provider source generation",
      );
    if (
      (input.source.kind === "display" &&
        !this.handshakeValue.capabilities.displayCapture) ||
      (input.source.kind === "area" &&
        !this.handshakeValue.capabilities.areaCapture)
    )
      throw new Error("selected source kind is unavailable");
    if (
      input.options.audio === "microphone" &&
      !this.handshakeValue.capabilities.audioCapture
    )
      throw new Error("microphone capture is unavailable");
    validateStartInvocation(input.invocation);
    const receipt = await this.rpc("start", input, timeoutMs);
    const status = this.status(receipt, input.captureId);
    if (status.publication && (status.publication.bundleId !== input.bundleId || status.publication.bundlePath !== input.bundlePath)) throw new Error("Saved start receipt does not match original bundle identity");
    return status;
  }
  async inspect(
    authority: { ownerToken: string; captureId: string },
    timeoutMs = 5000,
  ): Promise<CaptureStatus> {
    this.requireHandshake();
    authority = structuredClone(authority);
    return this.status(
      await this.rpc("inspect", authority, timeoutMs),
      authority.captureId,
    );
  }
  async stop(
    authority: { ownerToken: string; captureId: string },
    timeoutMs = 5000,
  ): Promise<CaptureStatus> {
    this.requireHandshake();
    authority = structuredClone(authority);
    return this.status(
      await this.rpc("stop", authority, timeoutMs),
      authority.captureId,
    );
  }
  async cancel(
    authority: { ownerToken: string; captureId: string },
    timeoutMs = 5000,
  ): Promise<CaptureStatus> {
    this.requireHandshake();
    authority = structuredClone(authority);
    return this.status(
      await this.rpc("cancel", authority, timeoutMs),
      authority.captureId,
    );
  }
  async publish(
    authority: {
      ownerToken: string;
      captureId: string;
      leaseId: string;
      publicationId: string;
      planFile: PublishEnvelope;
      planBytes: Uint8Array;
      materials: MaterialsDocument;
      derivatives?: readonly DerivativeDocumentInput[];
    },
    timeoutMs = 10000,
  ): Promise<CaptureStatus> {
    this.requireHandshake();
    if (!this.handshakeValue?.capabilities.publication) throw new Error("publication is unavailable");
    authority = structuredClone(authority);
    if (authority.leaseId !== authority.materials.manifest.leaseId) throw new Error("publication lease binding mismatch");
    const plan = validatePublicationPlan(
      authority.planFile,
      authority.planBytes,
      authority.materials,
      authority.derivatives,
    );
    if (
      plan.publicationId !== authority.publicationId ||
      plan.materials.revision !== authority.materials.manifest.key.revision ||
      plan.materials.captureId !== authority.captureId
    )
      throw new Error("publication binding mismatch");
    const result = this.status(
      await this.rpc(
        "publish",
        {
          ownerToken: authority.ownerToken,
          captureId: authority.captureId,
          leaseId: authority.leaseId,
          publicationId: authority.publicationId,
          materials: plan.materials,
          stagingRoot: authority.planFile.stagingRoot,
          planFile: authority.planFile.planFile,
          planSha256: authority.planFile.planSha256,
        },
        timeoutMs,
      ),
      authority.captureId,
    );
    validatePublicationObservation(result, authority.publicationId, authority.planFile.planSha256, {
      materials: plan.materials, bundleId: plan.bundleId, bundlePath: plan.destination.path,
      manifestSha256: plan.entries.find(entry => entry.relativePath === "apparent.json")!.sha256,
    });
    return result;
  }
  async inspectPublication(
    input: {
      ownerToken: string;
      captureId: string;
      publicationId: string;
      planSha256: string;
      expected: PublicationExpectation;
    },
    timeoutMs = 5000,
  ): Promise<CaptureStatus> {
    this.requireHandshake();
    input = structuredClone(input);
    const result = this.status(
      await this.rpc("inspectPublication", { ownerToken: input.ownerToken, captureId: input.captureId, publicationId: input.publicationId, planSha256: input.planSha256 }, timeoutMs),
      input.captureId,
    );
    validatePublicationObservation(result, input.publicationId, input.planSha256, input.expected);
    return result;
  }
  async releaseMaterials(
    input: {
      ownerToken: string;
      captureId: string;
      materialsId: string;
      leaseId: string;
    },
    timeoutMs = 5000,
  ): Promise<{ released: boolean; captureId: string; materialsId: string }> {
    this.requireHandshake();
    if (!this.handshakeValue?.capabilities.retainedMaterials) throw new Error("materials release is unavailable");
    input = structuredClone(input);
    const x = object(
      await this.rpc("releaseMaterials", input, timeoutMs),
      "release result",
    );
    exact(x, ["released", "captureId", "materialsId"]);
    if (
      typeof x.released !== "boolean" ||
      x.captureId !== input.captureId ||
      x.materialsId !== input.materialsId
    )
      throw new Error("release result identity mismatch");
    return x as { released: boolean; captureId: string; materialsId: string };
  }
  close() {
    return this.transport.close();
  }
  private status(v: unknown, captureId: string) {
    return validateStatus(v, captureId, !this.testDouble);
  }
  private requireHandshake() {
    if (!this.handshaken) throw new Error("capture/2 handshake required");
  }
  get isTestDouble() {
    return this.testDouble;
  }
}
function validatePublicationObservation(status: CaptureStatus, publicationId: string, planSha256: string, expected: PublicationExpectation): void {
  const e = object(expected, "publication expectation");
  exact(e, ["materials", "bundleId", "bundlePath", "manifestSha256"]);
  const key = object(e.materials, "expected materials");
  exact(key, ["captureId", "materialsId", "revision"]);
  if (key.captureId !== status.captureId || !id(key.materialsId) || !hash(key.revision) || !hash(e.manifestSha256)) throw new Error("invalid publication expectation");
  text(e.bundleId, "expected bundleId", 255); abs(e.bundlePath, "expected bundlePath");
  if (status.state === "saved") {
    const p = status.publication!;
    if (p.publicationId !== publicationId || p.planSha256 !== planSha256 || !isDeepStrictEqual(p.materials, expected.materials) || p.bundleId !== expected.bundleId || p.bundlePath !== expected.bundlePath || p.manifestSha256 !== expected.manifestSha256) throw new Error("Saved publication receipt does not match immutable plan");
  } else if (!["publishing", "failed", "cancelled"].includes(status.state) || status.publicationId !== publicationId || status.planSha256 !== planSha256 || (status.materials && !isDeepStrictEqual(status.materials.key, expected.materials))) throw new Error("publication observation identity mismatch");
}
function validateSource(v: unknown): SourceDescriptor {
  const x = object(v, "source");
  exact(
    x,
    ["id", "generation", "kind", "displayId", "bounds", "name"],
    ["id", "generation", "kind", "displayId", "bounds"],
  );
  text(x.id, "source id");
  if (
    !id(x.generation) ||
    !["display", "area"].includes(String(x.kind)) ||
    !Number.isSafeInteger(x.displayId) ||
    Number(x.displayId) < 0
  )
    throw new Error("invalid source descriptor");
  const b = object(x.bounds, "source bounds");
  exact(b, ["x", "y", "width", "height", "scale"]);
  if (
    Object.values(b).some(
      (n) => typeof n !== "number" || !Number.isFinite(n),
    ) ||
    Number(b.width) <= 0 ||
    Number(b.height) <= 0 ||
    Number(b.scale) <= 0
  )
    throw new Error("invalid source bounds");
  if (x.name !== undefined) text(x.name, "source name", 256);
  return x as unknown as SourceDescriptor;
}
function validateStartInvocation(v: unknown): void {
  const x = object(v, "invocation");
  if (x.kind === "explicit") exact(x, ["kind", "invocationId", "observedAtMs"]);
  else if (x.kind === "picker")
    exact(x, ["kind", "invocationId", "observedAtMs", "confirmationId"]);
  else if (x.kind === "shortcut")
    exact(
      x,
      [
        "kind",
        "invocationId",
        "observedAtMs",
        "hostEventId",
        "consumedReservationId",
      ],
      ["kind", "invocationId", "observedAtMs", "hostEventId"],
    );
  else throw new Error("invalid invocation kind");
  for (const k of [
    "invocationId",
    "confirmationId",
    "hostEventId",
    "consumedReservationId",
  ])
    if (x[k] !== undefined && !id(x[k]))
      throw new Error(`invalid invocation ${k}`);
  if (!Number.isSafeInteger(x.observedAtMs) || (x.observedAtMs as number) < 0)
    throw new Error("invalid invocation time");
}
function validateStatus(
  v: unknown,
  captureId: string,
  allowNative = true,
): CaptureStatus {
  const x = object(v, "capture status");
  const states = [
    "starting",
    "recording",
    "stopping",
    "materials-ready",
    "publishing",
    "saved",
    "cancelled",
    "failed",
  ];
  const fields = [
    "captureId",
    "state",
    "materials",
    "publicationId",
    "planSha256",
    "publication",
    "error",
  ];
  exact(x, fields, ["captureId", "state"]);
  if (
    !id(x.captureId) ||
    x.captureId !== captureId ||
    !states.includes(String(x.state))
  )
    throw new Error("capture status identity mismatch");
  if ((x.state === "materials-ready" || x.state === "publishing") && x.materials === undefined) throw new Error("finalized materials reference is required");
  if (x.materials !== undefined) {
    const m = object(x.materials, "materials manifest");
    exact(m, ["schema", "key", "root", "leaseId", "payloadFile"]);
    const k = object(m.key, "materials key");
    exact(k, ["captureId", "materialsId", "revision"]);
    if (
      m.schema !== "apparent.capture-materials/1" ||
      k.captureId !== captureId ||
      !id(k.materialsId) ||
      !hash(k.revision)
    )
      throw new Error("invalid materials manifest identity");
    abs(m.root, "materials root");
    if (!id(m.leaseId)) throw new Error("invalid materials lease");
    const file = object(m.payloadFile, "materials payload reference");
    exact(file, ["relativePath", "identity", "sha256"]);
    relativePath(file.relativePath, "materials payload file");
    validateSealedFile(file, "materials payload file");
    if (!hash(file.sha256) || file.sha256 !== k.revision)
      throw new Error("materials revision does not bind payload file");
    const fi = object(file.identity, "materials payload identity");
    exact(fi, ["device", "inode", "size", "modifiedNs"]);
    for (const key of ["device", "inode", "modifiedNs"])
      text(fi[key], `payload ${key}`);
    if (!Number.isSafeInteger(fi.size) || Number(fi.size) < 1 || Number(fi.size) > MAX_DOCUMENT_BYTES)
      throw new Error("invalid materials payload size");
    if (x.state === "publishing" && (!id(x.publicationId) || !hash(x.planSha256)))
      throw new Error("publishing state requires publication identity");
  }
  if (x.state === "saved") {
    if (!allowNative)
      throw new Error("test double cannot provide native Saved provenance");
    const p = object(x.publication, "publication receipt");
    exact(p, [
      "captureId",
      "materials",
      "publicationId",
      "planSha256",
      "bundleId",
      "bundlePath",
      "artifactVersion",
      "artifactProvenance",
      "manifestSha256",
      "inventorySha256",
    ]);
    const m = object(p.materials, "Saved materials key");
    exact(m, ["captureId", "materialsId", "revision"]);
    if (
      p.captureId !== captureId ||
      m.captureId !== captureId ||
      !id(m.materialsId) ||
      !hash(m.revision) ||
      p.artifactVersion !== 1 ||
      p.artifactProvenance !== "native" ||
      !id(p.publicationId) ||
      !hash(p.planSha256) ||
      !hash(p.manifestSha256) ||
      !hash(p.inventorySha256)
    )
      throw new Error("invalid Saved receipt");
    if (x.materials !== undefined && !isDeepStrictEqual(object(x.materials, "retained materials").key, p.materials)) throw new Error("Saved retained materials identity mismatch");
    text(p.bundleId, "Saved bundleId", 255);
    abs(p.bundlePath, "saved bundlePath");
  }
  if (x.state === "failed") {
    const e = object(x.error, "failure error");
    exact(e, ["code"]);
    errorCode(e.code);
  } else if (x.error !== undefined)
    throw new Error("error is invalid for this capture state");
  if (x.state !== "saved" && x.publication !== undefined)
    throw new Error("publication receipt only valid for Saved");
  if (
    !["materials-ready", "publishing", "saved", "cancelled", "failed"].includes(String(x.state)) &&
    x.materials !== undefined
  )
    throw new Error("materials only valid after material finalization");
  if (
    (x.publicationId !== undefined || x.planSha256 !== undefined) &&
    (!["publishing", "failed", "cancelled"].includes(String(x.state)) || !id(x.publicationId) || !hash(x.planSha256))
  )
    throw new Error("publication identity only valid while publishing");
  return x as unknown as CaptureStatus;
}

/** Existing engine-native does not implement these capture/2 flags yet. */
export function launchCaptureEngine(
  executablePath: string,
  runtimeDirectory: string,
  shutdownTimeoutMs = 3000,
): ChildProcessCaptureTransport {
  abs(executablePath, "executablePath");
  abs(runtimeDirectory, "runtimeDirectory");
  timeout(shutdownTimeoutMs);
  return new ChildProcessCaptureTransport(
    spawn(
      executablePath,
      [
        "--runtime-directory",
        runtimeDirectory,
        "--capture-channel",
        "capture",
        "--capture-protocol-version",
        "2",
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
export class ChildProcessCaptureTransport implements CaptureTransport {
  private bytes = Buffer.alloc(0);
  private pending = new Map<
    string,
    {
      resolve: (x: CaptureResponse) => void;
      reject: (e: Error) => void;
      timer: NodeJS.Timeout;
    }
  >();
  private terminal?: Error;
  private closed = false;
  private closing = false;
  private closePromise?: Promise<CaptureCloseOutcome>;
  private child: ChildProcessWithoutNullStreams;
  private shutdownMs: number;
  constructor(child: ChildProcessWithoutNullStreams, shutdownMs = 3000) {
    timeout(shutdownMs);
    this.child = child;
    this.shutdownMs = shutdownMs;
    child.stdout.on("data", (chunk) => this.onData(Buffer.from(chunk)));
    child.stdout.on("error", () =>
      this.fail(new Error("capture Engine stdout failed")),
    );
    child.stdout.on("end", () => this.fail(new Error("capture Engine output ended")));
    child.stdin.on("error", () =>
      this.fail(new Error("capture Engine stdin failed")),
    );
    child.stderr.on("data", () => {});
    child.stderr.on("error", () => {});
    child.on("error", () =>
      this.fail(new Error("capture Engine process failed")),
    );
    child.on("exit", () => {
      this.closed = true;
      this.fail(new Error("capture Engine exited"));
    });
  }
  private onData(chunk: Buffer) {
    if (this.terminal) return;
    this.bytes = Buffer.concat([this.bytes, chunk]);
    for (;;) {
      const nl = this.bytes.indexOf(10);
      if (nl < 0) break;
      if (nl > CAPTURE_MAX_LINE_BYTES)
        return this.fail(new Error("capture response line exceeds bound"));
      let line = this.bytes.subarray(0, nl);
      this.bytes = this.bytes.subarray(nl + 1);
      if (line.at(-1) === 13) line = line.subarray(0, -1);
      if (line.includes(13))
        return this.fail(new Error("invalid capture line ending"));
      let v: unknown;
      try {
        v = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(line));
      } catch {
        return this.fail(new Error("invalid capture UTF-8 JSONL"));
      }
      try {
        const x = object(v, "response");
        exact(
          x,
          ["channel", "protocolVersion", "rpcId", "result", "error"],
          ["channel", "protocolVersion", "rpcId"],
        );
        if (x.channel !== "capture" || x.protocolVersion !== 2 || !id(x.rpcId) || (x.result === undefined) === (x.error === undefined))
          throw Error();
        const p = this.pending.get(x.rpcId);
        if (!p) continue;
        clearTimeout(p.timer);
        this.pending.delete(x.rpcId);
        p.resolve(x as unknown as CaptureResponse);
      } catch {
        return this.fail(new Error("invalid capture response envelope"));
      }
    }
    if (this.bytes.length > CAPTURE_MAX_LINE_BYTES)
      return this.fail(new Error("capture response line exceeds bound"));
  }
  request(req: CaptureRequest, timeoutMs = 5000): Promise<CaptureResponse> {
    timeout(timeoutMs);
    if (this.closed || this.closing || this.terminal)
      return Promise.reject(
        this.terminal ?? new Error("capture Engine disconnected"),
      );
    if (this.pending.size >= 64)
      return Promise.reject(new Error("capture pending RPC bound reached"));
    if (this.pending.has(req.rpcId)) return Promise.reject(new Error("duplicate capture RPC identity"));
    const line = encodeCaptureRequest(req);
    if (this.child.stdin.writableLength + Buffer.byteLength(line) > 1024 * 1024)
      return Promise.reject(new Error("capture write queue bound reached"));
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.pending.delete(req.rpcId);
        reject(new Error("capture RPC timed out; operation was not cancelled"));
      }, timeoutMs);
      this.pending.set(req.rpcId, { resolve, reject, timer });
      this.child.stdin.write(line, (e) => {
        if (e) {
          const p = this.pending.get(req.rpcId);
          if (p) {
            clearTimeout(p.timer);
            this.pending.delete(req.rpcId);
            p.reject(new Error("capture request write failed"));
          }
        }
      });
    });
  }
  private fail(e: Error) {
    this.terminal ??= e;
    for (const p of this.pending.values()) {
      clearTimeout(p.timer);
      p.reject(e);
    }
    this.pending.clear();
  }
  close(): Promise<CaptureCloseOutcome> {
    if (this.closePromise) return this.closePromise;
    this.closing = true;
    this.closePromise = new Promise((done) => {
      if (this.closed) {
        done("already-exited");
        return;
      }
      let outcome: CaptureCloseOutcome = "eof-exited";
      let requestedSignal: "SIGTERM" | "SIGKILL" | undefined;
      const eof = setTimeout(() => {
        if (!this.closed) {
          outcome = "signal-requested-after-timeout";
          requestedSignal = "SIGTERM";
          this.child.kill("SIGTERM");
        }
      }, this.shutdownMs);
      const kill = setTimeout(() => {
        if (!this.closed) {
          outcome = "kill-requested-after-timeout";
          requestedSignal = "SIGKILL";
          this.child.kill("SIGKILL");
        }
      }, this.shutdownMs * 2);
      const reap = setTimeout(() => done(outcome), this.shutdownMs * 3);
      this.child.once("exit", (code, signal) => {
        clearTimeout(eof);
        clearTimeout(kill);
        clearTimeout(reap);
        this.closed = true;
        done(
          requestedSignal === "SIGKILL"
            ? "kill-exited"
            : requestedSignal === "SIGTERM"
              ? "signal-exited"
              : code === 0 && signal === null
                ? "eof-exited"
                : "exit-observed",
        );
      });
      this.child.stdin.end();
    });
    this.fail(
      new Error(
        "capture transport closed; native EOF semantics are unimplemented",
      ),
    );
    return this.closePromise;
  }
}
