import type {
  CaptureReceipt,
  EngineClient as RecordEngineClient,
  EngineEvent,
  Readiness,
} from "./client.js";

/** The small structural surface implemented by @apparent/engine-sdk 0.1.0. */
export interface EngineSdkPort {
  readonly clientId: string;
  handshake(): Promise<{
    implementation: "native" | "test-double";
    capabilities: { screenCapture: boolean };
    readiness: "ready" | "unavailable";
  }>;
  prepareStart(bundleId: string, bundlePath: string): PreparedStart;
  start(request: PreparedStart): Promise<{
    receipt: SdkReceipt;
    captureId: string;
    ownerToken: string;
  }>;
  control(
    method: "inspect" | "stop" | "cancel",
    captureId: string,
    ownerToken: string,
  ): Promise<SdkReceipt>;
  close(): Promise<unknown>;
}

export interface PreparedStart {
  protocolVersion: 1;
  clientId: string;
  rpcId: string;
  method: "start";
  captureId: string;
  ownerToken: string;
  bundleId: string;
  bundlePath: string;
  options: { screen: true };
}

export interface SdkReceipt {
  captureId: string;
  state:
    | "starting"
    | "recording"
    | "stopping"
    | "finalizing"
    | "saved"
    | "cancelled"
    | "failed"
    | "rejected";
  bundlePath?: string;
  artifactProvenance?: "native" | "test-synthetic";
  error?: { code: string; message: string };
}

/**
 * A host-owned atomic mapping store. putIfAbsent must bind requestId to one
 * immutable identity and return the identity that won, including across host
 * processes. Persist before resolving putIfAbsent. If a write is ambiguous,
 * getByRequestId must reconcile it before another identity can be created.
 */
export interface CaptureMappingStore {
  /** Lists public Record identities only; SDK IDs and owner tokens stay private. */
  listRequestIds(protocol?: "v1" | "capture2"): Promise<string[]>;
  getByRequestId(requestId: string): Promise<CaptureMapping | undefined>;
  getByCaptureId(captureId: string): Promise<CaptureMapping | undefined>;
  putIfAbsent(mapping: CaptureMapping): Promise<CaptureMapping>;
}

export interface CaptureMapping {
  requestId: string;
  captureId: string;
  ownerToken: string;
  bundleId: string;
  bundlePath: string;
  /** Immutable capture/2 intent; absent on legacy v1 rows. */
  capture2Intent?: string;
}

export interface BundleAllocation {
  bundleId: string;
  bundlePath: string;
}

export interface SdkAdapterOptions {
  mappings: CaptureMappingStore;
  allocateBundle(requestId: string): Promise<BundleAllocation>;
  pollIntervalMs?: number;
}

const idPattern = /^[0-9a-f]{32}$/u;
const tokenPattern = /^[0-9a-f]{64}$/u;
const terminal = new Set<SdkReceipt["state"]>([
  "saved",
  "cancelled",
  "failed",
  "rejected",
]);
const rank: Record<SdkReceipt["state"], number> = {
  starting: 0,
  recording: 1,
  stopping: 2,
  finalizing: 3,
  saved: 4,
  cancelled: 4,
  failed: 4,
  rejected: 4,
};
const phases = new Set<CaptureReceipt["phase"]>([
  "starting",
  "recording",
  "processing",
  "saved",
  "cancelled",
  "failed",
  "rejected",
]);
const safeUnavailable: Readiness = {
  ready: false,
  reason: "Native Engine capture is unavailable",
};

/** Injects an already constructed SDK client; disposal never closes the host. */
export class EngineSdkAdapter implements RecordEngineClient {
  readonly #sdk: EngineSdkPort;
  readonly #mappings: CaptureMappingStore;
  readonly #allocateBundle: SdkAdapterOptions["allocateBundle"];
  readonly #pollIntervalMs: number;
  readonly #listeners = new Set<(event: EngineEvent) => void>();
  readonly #mappingByCaptureId = new Map<string, CaptureMapping>();
  readonly #mappingByRequestId = new Map<string, CaptureMapping>();
  readonly #capturePhases = new Map<string, SdkReceipt["state"]>();
  readonly #submittedUnknown = new Set<string>();
  readonly #inFlightStarts = new Map<string, Promise<CaptureMapping>>();
  readonly #queues = new Map<string, Promise<void>>();
  #pollTimer?: ReturnType<typeof setTimeout>;
  #disposed = false;
  #polling = false;

  constructor(sdk: EngineSdkPort, options: SdkAdapterOptions) {
    const pollIntervalMs = options.pollIntervalMs ?? 500;
    if (!Number.isSafeInteger(pollIntervalMs) || pollIntervalMs < 50 || pollIntervalMs > 60_000)
      throw new RangeError("pollIntervalMs must be between 50 and 60000 ms");
    this.#sdk = sdk;
    this.#mappings = options.mappings;
    this.#allocateBundle = options.allocateBundle;
    this.#pollIntervalMs = pollIntervalMs;
  }

  async readiness(): Promise<Readiness> {
    if (this.#disposed) return safeUnavailable;
    try {
      const hello = await this.#sdk.handshake();
      if (this.#disposed) return safeUnavailable;
      return hello.implementation === "native" &&
        hello.readiness === "ready" && hello.capabilities.screenCapture
        ? { ready: true }
        : safeUnavailable;
    } catch {
      return safeUnavailable;
    }
  }

  async start(requestId: string): Promise<CaptureReceipt> {
    if (this.#disposed) throw new Error("Record Engine adapter is disposed");
    if (!requestId.trim()) throw new Error("Record request ID is required");
    const mapping = await this.#mappingFor(requestId);
    return this.#serialize(mapping.captureId, async () => {
      this.#assertActive();
      this.#submittedUnknown.add(mapping.captureId);
      this.#schedulePoll();
      try {
        const result = await this.#sdk.start(this.#prepared(mapping));
        if (result.captureId !== mapping.captureId || result.ownerToken !== mapping.ownerToken)
          throw new Error("SDK identity mismatch");
        return this.#accept(mapping, result.receipt);
      } catch {
        throw new Error("Engine start outcome is unknown; inspect the same capture to reconcile");
      }
    });
  }

  stop(captureId: string): Promise<CaptureReceipt> {
    return this.#control("stop", captureId);
  }

  cancel(captureId: string): Promise<CaptureReceipt> {
    return this.#control("cancel", captureId);
  }

  inspect(captureId: string): Promise<CaptureReceipt> {
    return this.#control("inspect", captureId);
  }

  subscribe(listener: (event: EngineEvent) => void): () => void {
    if (this.#disposed) return () => undefined;
    this.#listeners.add(listener);
    this.#schedulePoll();
    return () => {
      this.#listeners.delete(listener);
      if (!this.#listeners.size && this.#pollTimer) {
        clearTimeout(this.#pollTimer);
        this.#pollTimer = undefined;
      }
    };
  }

  /** Detaches local observation only. Host shutdown owns SDK close(). */
  dispose(): void {
    this.#disposed = true;
    if (this.#pollTimer) clearTimeout(this.#pollTimer);
    this.#pollTimer = undefined;
    this.#listeners.clear();
  }

  async #control(method: "inspect" | "stop" | "cancel", captureId: string): Promise<CaptureReceipt> {
    if (this.#disposed) throw new Error("Record Engine adapter is disposed");
    if (!captureId.trim()) throw new Error("Unknown Record capture");
    const mapping = await this.#mappingForCapture(captureId);
    return this.#serialize(mapping.captureId, async () => {
      this.#assertActive();
      try {
        let receipt = await this.#sdk.control(method, mapping.captureId, mapping.ownerToken);
        // Record has no stopping phase. Wait briefly for a representable native
        // observation; never claim recording, processing, or completion early.
        for (let attempt = 0; receipt.state === "stopping" && attempt < 5; attempt++) {
          await new Promise(resolve => setTimeout(resolve, 100));
          this.#assertActive();
          receipt = await this.#sdk.control("inspect", mapping.captureId, mapping.ownerToken);
        }
        return this.#accept(mapping, receipt);
      } catch {
        throw new Error("Engine operation outcome is unknown; inspect the same capture to reconcile");
      }
    });
  }

  #mappingFor(requestId: string): Promise<CaptureMapping> {
    const pending = this.#inFlightStarts.get(requestId);
    if (pending) return pending;
    const work = this.#loadOrCreateMapping(requestId);
    this.#inFlightStarts.set(requestId, work);
    void work.finally(() => {
      if (this.#inFlightStarts.get(requestId) === work) this.#inFlightStarts.delete(requestId);
    }).catch(() => undefined);
    return work;
  }

  async #loadOrCreateMapping(requestId: string): Promise<CaptureMapping> {
    try {
      const existing = await this.#mappings.getByRequestId(requestId);
      if (existing) return this.#remember(existing, requestId);
      const bundle = await this.#allocateBundle(requestId);
      const prepared = this.#sdk.prepareStart(bundle.bundleId, bundle.bundlePath);
      const candidate: CaptureMapping = {
        requestId,
        captureId: prepared.captureId,
        ownerToken: prepared.ownerToken,
        bundleId: bundle.bundleId,
        bundlePath: bundle.bundlePath,
      };
      return this.#remember(await this.#mappings.putIfAbsent(candidate), requestId);
    } catch {
      throw new Error("Could not persist Record capture identity before Engine dispatch");
    }
  }

  async #mappingForCapture(captureId: string): Promise<CaptureMapping> {
    // Public Record capture identity is its original caller-owned request ID.
    // Native IDs and authority tokens never become consumer control keys.
    const cached = this.#mappingByRequestId.get(captureId);
    if (cached) return cached;
    try {
      const mapping = await this.#mappings.getByRequestId(captureId);
      if (!mapping) throw new Error("missing mapping");
      return this.#remember(mapping, captureId);
    } catch {
      throw new Error("Unknown Record capture");
    }
  }

  #remember(mapping: CaptureMapping, requestId: string): CaptureMapping {
    if (mapping.requestId !== requestId ||
      !idPattern.test(mapping.captureId) ||
      !tokenPattern.test(mapping.ownerToken) || !mapping.bundleId || !mapping.bundlePath || mapping.capture2Intent !== undefined)
      throw new Error("invalid mapping");
    const prior = this.#mappingByCaptureId.get(mapping.captureId);
    const priorRequest = this.#mappingByRequestId.get(requestId);
    if (priorRequest && (priorRequest.captureId !== mapping.captureId ||
      priorRequest.ownerToken !== mapping.ownerToken || priorRequest.bundleId !== mapping.bundleId ||
      priorRequest.bundlePath !== mapping.bundlePath)) throw new Error("request identity binding changed");
    if (prior && (prior.requestId !== mapping.requestId || prior.ownerToken !== mapping.ownerToken ||
      prior.bundleId !== mapping.bundleId || prior.bundlePath !== mapping.bundlePath))
      throw new Error("identity binding changed");
    this.#mappingByCaptureId.set(mapping.captureId, Object.freeze({ ...mapping }));
    this.#mappingByRequestId.set(requestId, this.#mappingByCaptureId.get(mapping.captureId)!);
    return this.#mappingByCaptureId.get(mapping.captureId)!;
  }

  #prepared(mapping: CaptureMapping): PreparedStart {
    // SDK start validates and uses this durable identity; rpcId/clientId are
    // supplied by the SDK client and refreshed for every dispatch.
    return {
      protocolVersion: 1,
      clientId: this.#sdk.clientId,
      rpcId: "00000000000000000000000000000000",
      method: "start",
      ownerToken: mapping.ownerToken,
      captureId: mapping.captureId,
      bundleId: mapping.bundleId,
      bundlePath: mapping.bundlePath,
      options: { screen: true },
    };
  }

  #accept(mapping: CaptureMapping, receipt: SdkReceipt): CaptureReceipt {
    this.#assertActive();
    if (receipt.captureId !== mapping.captureId)
      throw new Error("Engine receipt identity mismatch");
    if (receipt.state === "saved") {
      if (receipt.artifactProvenance !== "native" || receipt.bundlePath !== mapping.bundlePath)
        throw new Error("Engine saved receipt is not a matching native artifact");
    }
    const prior = this.#capturePhases.get(mapping.captureId);
    if (receipt.state === "stopping") {
      throw new Error("Engine is stopping; no Record phase represents that observation yet");
    }
    this.#submittedUnknown.delete(mapping.captureId);
    if (prior && (rank[receipt.state] < rank[prior] || (terminal.has(prior) && receipt.state !== prior)))
      return this.#receiptFor(prior, mapping)!;
    this.#capturePhases.set(mapping.captureId, receipt.state);
    const visible = this.#receiptFor(receipt.state, mapping, receipt);
    if (visible) this.#publish({ type: "capture-changed", receipt: visible });
    if (!visible) throw new Error("Engine returned an unsupported Record phase");
    return visible;
  }

  #receiptFor(state: SdkReceipt["state"], mapping: CaptureMapping, source?: SdkReceipt): CaptureReceipt | undefined {
    const phase: CaptureReceipt["phase"] = state === "finalizing" ? "processing" : state as CaptureReceipt["phase"];
    if (!phases.has(phase)) return undefined;
    const result: CaptureReceipt = { captureId: mapping.requestId, phase };
    if (phase === "saved") result.bundlePath = mapping.bundlePath;
    if ((phase === "failed" || phase === "rejected") && source?.error)
      result.error = "Engine confirmed an operation failure";
    return result;
  }

  #publish(event: EngineEvent): void {
    for (const listener of this.#listeners) {
      try { listener(structuredClone(event)); } catch { this.#listeners.delete(listener); }
    }
    if (!this.#listeners.size && this.#pollTimer) {
      clearTimeout(this.#pollTimer);
      this.#pollTimer = undefined;
    }
    this.#schedulePoll();
  }

  #schedulePoll(): void {
    if (this.#disposed || !this.#listeners.size || this.#pollTimer || this.#polling) return;
    this.#pollTimer = setTimeout(() => { void this.#poll(); }, this.#pollIntervalMs);
  }

  async #poll(): Promise<void> {
    this.#pollTimer = undefined;
    if (this.#disposed || !this.#listeners.size || this.#polling) return;
    this.#polling = true;
    try {
      const active = [...this.#mappingByCaptureId.values()].filter(mapping => {
        const phase = this.#capturePhases.get(mapping.captureId);
        return this.#submittedUnknown.has(mapping.captureId) || (phase !== undefined && !terminal.has(phase));
      });
      for (const mapping of active) {
        if (this.#disposed || !this.#listeners.size) break;
        try {
          await this.#serialize(mapping.captureId, async () => {
            if (this.#disposed || !this.#listeners.size) return;
            const receipt = await this.#sdk.control("inspect", mapping.captureId, mapping.ownerToken);
            if (receipt.state !== "stopping") this.#accept(mapping, receipt);
          });
        } catch { /* Ambiguity is reconciled by a later inspect; never terminalize. */ }
      }
    } finally {
      this.#polling = false;
      this.#schedulePoll();
    }
  }

  #serialize<T>(captureId: string, operation: () => Promise<T>): Promise<T> {
    const previous = this.#queues.get(captureId) ?? Promise.resolve();
    const current = previous.catch(() => undefined).then(operation);
    const tail = current.then(() => undefined, () => undefined);
    this.#queues.set(captureId, tail);
    void tail.then(() => {
      if (this.#queues.get(captureId) === tail) this.#queues.delete(captureId);
    });
    return current;
  }

  #assertActive(): void {
    if (this.#disposed) throw new Error("Record Engine adapter is disposed");
  }
}
