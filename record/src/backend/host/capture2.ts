import { randomBytes, randomUUID } from "node:crypto";
import { isDeepStrictEqual } from "node:util";
import { validateCaptureRequest, type CaptureClient, type CaptureHandshake, type CaptureOptions, type CaptureStatus, type InvocationFact, type SourceDescriptor, type SourceDiscovery } from "@apparent/engine-sdk/capture";
import type { CaptureMapping } from "../engine/sdk-adapter.js";
import type { CaptureMappingStore } from "../engine/sdk-adapter.js";
import { AbsentDestinationAllocator } from "./absent-destination-allocator.js";

export type Capture2Phase = "reserved" | "submitted-unknown" | "starting" | "recording" | "stopping" | "materials-ready" | "processing" | "cancelled" | "failed" | "unknown";
export interface Capture2Receipt { requestId: string; phase: Capture2Phase; lastKnownPhase?: Capture2Phase; materials?: CaptureStatus["materials"]; publication?: CaptureStatus["publication"] }
export type Capture2Event = { type: "readiness-changed"; readiness: { ready: boolean; reason?: string } } | { type: "discovery-changed"; discovery: SourceDiscovery } | { type: "capture-changed"; receipt: Capture2Receipt };
export interface Capture2MappingStore extends CaptureMappingStore {
  /** Atomic compare-and-insert against the complete capture/2 registry inspected before admission. */
  putCapture2IfUnchanged(mapping: CaptureMapping, inspectedRequestIds: readonly string[]): Promise<CaptureMapping>;
}
export interface Capture2StartIntent { requestId: string; source: SourceDescriptor; options: CaptureOptions; invocation: InvocationFact; destinationPath?: string }
export interface Capture2Host {
  readiness(): Promise<{ ready: boolean; reason?: string }>;
  discoverSources(): Promise<SourceDiscovery>;
  start(intent: Capture2StartIntent): Promise<Capture2Receipt>;
  inspect(requestId: string): Promise<Capture2Receipt>;
  stop(requestId: string): Promise<Capture2Receipt>;
  cancel(requestId: string): Promise<Capture2Receipt>;
  reconcile(): Promise<readonly Capture2Receipt[]>;
  subscribe(listener: (event: Capture2Event) => void): () => void;
  dispose(): void;
}

const canonical = (value: unknown): unknown => Array.isArray(value) ? value.map(canonical) : value && typeof value === "object"
  ? Object.fromEntries(Object.entries(value).sort(([left], [right]) => left.localeCompare(right)).map(([key, entry]) => [key, canonical(entry)]))
  : value;
const intentOf = (source: SourceDescriptor, options: CaptureOptions, invocation: InvocationFact, destinationPath?: string) => JSON.stringify(canonical({ protocol: 2, source, options, invocation, ...(destinationPath === undefined ? {} : { destinationPath }) }));
const publicReceipt = (requestId: string, status: CaptureStatus): Capture2Receipt => ({
  requestId,
  phase: status.state === "saved" || status.state === "publishing" ? "processing" : status.state === "materials-ready" ? "materials-ready" : status.state,
  ...(status.materials ? { materials: structuredClone(status.materials) } : {}),
  ...(status.publication ? { publication: structuredClone(status.publication) } : {}),
});

/** Injected published capture/2 consumer. Ownership of the SDK and transports stays with the caller. */
export function createCapture2Host(input: { client: CaptureClient; mappings: Capture2MappingStore; destinations: AbsentDestinationAllocator }): Capture2Host {
  const { client, mappings, destinations } = input;
  const cache = new Map<string, CaptureMapping>();
  const phases = new Map<string, Capture2Phase>();
  const receipts = new Map<string, Capture2Receipt>();
  const active = new Set<string>();
  const inflight = new Map<string, { intent: string; promise: Promise<Capture2Receipt> }>();
  const listeners = new Set<(event: Capture2Event) => void>();
  const observations = new Map<string, number>();
  const tails = new Map<string, Promise<unknown>>();
  let handshake: CaptureHandshake | undefined;
  let discovery: SourceDiscovery | undefined;
  let disposed = false;
  let freshAdmission = false;

  const ensureLive = () => { if (disposed) throw new Error("Record capture/2 host is disposed"); };
  const emit = (event: Capture2Event) => { if (disposed) return; for (const listener of listeners) try { listener(structuredClone(event)); } catch { listeners.delete(listener); } };
  const serial = <T>(id: string, work: () => Promise<T>): Promise<T> => {
    const previous = tails.get(id) ?? Promise.resolve();
    const current = previous.catch(() => undefined).then(work);
    const tail = current.then(() => undefined, () => undefined);
    tails.set(id, tail);
    void tail.then(() => { if (tails.get(id) === tail) tails.delete(id); });
    return current;
  };
  const hello = async () => {
    if (disposed) throw new Error("Record capture/2 host is disposed");
    if (!handshake || handshake.readiness === "unavailable") try { handshake = await client.handshake(); } catch { handshake = undefined; }
    ensureLive();
    const ready = handshake?.implementation === "native" && handshake.readiness === "ready" && (handshake.capabilities.displayCapture || handshake.capabilities.areaCapture) && handshake.capabilities.sourceDiscovery && handshake.capabilities.retainedMaterials;
    const result = ready ? { ready: true } : { ready: false, reason: "Native Record capture/2 is unavailable" };
    emit({ type: "readiness-changed", readiness: result });
    return result;
  };
  const mappingFor = async (requestId: string): Promise<CaptureMapping> => {
    const cached = cache.get(requestId); if (cached) return cached;
    const row = await mappings.getByRequestId(requestId);
    if (!row?.capture2Intent) throw new Error("Unknown Record capture/2 request");
    cache.set(requestId, Object.freeze({ ...row }));
    return row;
  };
  const observe = (mapping: CaptureMapping, status: CaptureStatus): Capture2Receipt => {
    if (status.captureId !== mapping.captureId) throw new Error("Capture SDK returned a mismatched identity");
    ensureLive();
    if (status.materials && status.materials.key.captureId !== mapping.captureId) throw new Error("Capture materials identity mismatch");
    if (status.publication && (status.publication.captureId !== mapping.captureId || status.publication.bundleId !== mapping.bundleId || status.publication.bundlePath !== mapping.bundlePath)) throw new Error("Capture publication identity mismatch");
    const next = publicReceipt(mapping.requestId, status);
    const previous = receipts.get(mapping.requestId);
    if (previous?.materials && next.materials && !isDeepStrictEqual(previous.materials, next.materials)) throw new Error("Capture materials binding changed");
    // This host never releases materials, so later observations cannot discard a retained lease.
    if (!next.materials && previous?.materials) next.materials = structuredClone(previous.materials);
    const prior = phases.get(mapping.requestId);
    const ranks: Record<Capture2Phase, number> = { reserved: 0, "submitted-unknown": 1, unknown: 1, starting: 2, recording: 3, stopping: 4, "materials-ready": 5, processing: 6, cancelled: 7, failed: 7 };
    if (prior && (ranks[next.phase] < ranks[prior] || (["cancelled", "failed"].includes(prior) && next.phase !== prior))) return structuredClone(receipts.get(mapping.requestId) ?? next);
    // Terminal native material states never mean that Record has accepted a bundle.
    observations.set(mapping.requestId, (observations.get(mapping.requestId) ?? 0) + 1);
    phases.set(mapping.requestId, next.phase);
    receipts.set(mapping.requestId, next);
    if (["starting", "recording", "stopping"].includes(next.phase)) active.add(mapping.requestId); else active.delete(mapping.requestId);
    emit({ type: "capture-changed", receipt: structuredClone(next) });
    return structuredClone(next);
  };
  const unknown = (requestId: string): Capture2Receipt => {
    ensureLive();
    const prior = receipts.get(requestId);
    const receipt: Capture2Receipt = { ...structuredClone(prior), requestId, phase: "unknown", ...(phases.has(requestId) ? { lastKnownPhase: phases.get(requestId)! } : {}) };
    receipts.set(requestId, receipt);
    active.add(requestId);
    emit({ type: "capture-changed", receipt });
    return structuredClone(receipt);
  };
  const call = (method: "inspect" | "stop" | "cancel", requestId: string): Promise<Capture2Receipt> => {
    const work = async () => {
      ensureLive();
      const mapping = await mappingFor(requestId);
      await hello(); // Establish SDK protocol identity on reconnect, without gating owned controls on readiness.
      ensureLive();
      const revision = observations.get(requestId) ?? 0;
      try { return observe(mapping, await client[method]({ captureId: mapping.captureId, ownerToken: mapping.ownerToken })); }
      catch {
        ensureLive();
        const latest = receipts.get(requestId);
        const resolved = method === "inspect" || latest?.phase === "cancelled" || latest?.phase === "failed" || (method === "stop" && (latest?.phase === "materials-ready" || latest?.phase === "processing"));
        if ((observations.get(requestId) ?? 0) !== revision && resolved) return structuredClone(latest!);
        return unknown(requestId);
      }
    };
    // Read-only reconciliation must not hold up Stop/Cancel. Mutating controls remain ordered.
    return method === "inspect" ? work() : serial(requestId, work);
  };

  return {
    readiness: hello,
    async discoverSources() {
      if (!(await hello()).ready) return { state: "unavailable", sources: [], reason: "Native Record capture/2 is unavailable" };
      ensureLive();
      try { discovery = await client.discoverSources(); } catch { discovery = { state: "unavailable", sources: [], reason: "Source discovery is unavailable" }; }
      ensureLive();
      emit({ type: "discovery-changed", discovery: structuredClone(discovery) });
      return structuredClone(discovery);
    },
    start(request) {
      const intent = structuredClone(request);
      const serializedIntent = intentOf(intent.source, intent.options, intent.invocation, intent.destinationPath);
      const pending = inflight.get(intent.requestId);
      if (pending) return pending.intent === serializedIntent
        ? pending.promise.then(value => structuredClone(value))
        : Promise.reject(new Error("Record capture intent conflicts with its in-flight request binding"));
      if (freshAdmission || (active.size && !receipts.has(intent.requestId))) return Promise.reject(new Error("Another Record capture is active or unresolved; request ID was not reserved"));
      freshAdmission = true;
      const work = Promise.resolve().then(async () => {
        const ready = await hello();
        const old = await mappings.getByRequestId(intent.requestId);
        ensureLive();
        if (old) {
          if (old.capture2Intent !== serializedIntent) throw new Error("Record capture intent conflicts with its durable request binding");
          cache.set(intent.requestId, Object.freeze({ ...old }));
          if (!phases.has(intent.requestId)) phases.set(intent.requestId, "submitted-unknown");
          try { return observe(old, await client.inspect({ captureId: old.captureId, ownerToken: old.ownerToken })); }
          catch { ensureLive(); unknown(intent.requestId); throw new Error("Capture start outcome is unknown; inspect the same Record request to reconcile"); }
        }
        if (!ready.ready || handshake?.implementation !== "native") throw Object.assign(new Error("Capture/2 is unavailable; no native start was dispatched"), { dispatched: false });
        if (!discovery || discovery.state !== "available" || !discovery.sources.some(source => isDeepStrictEqual(source, intent.source))) throw new Error("Capture source is not the exact descriptor in the current discovery generation");
        if (!intent.requestId.trim() || intent.options.localOnly !== true || !["none", "microphone"].includes(intent.options.audio)) throw new Error("Explicit local capture intent is required");
        validateCaptureRequest({ channel: "capture", protocolVersion: 2, clientId: client.clientId, rpcId: randomUUID().replaceAll("-", ""), method: "start", ownerToken: "0".repeat(64), captureId: "0".repeat(32), bundleId: "record-preflight.apparent", bundlePath: "/record-preflight.apparent", source: intent.source, options: intent.options, invocation: intent.invocation });
        // Recheck durable capture/2 ownership before reserving a new public ID.
        // Unknown inspection is busy: it cannot prove that another capture ended.
        const inspectedRequestIds = await mappings.listRequestIds("capture2");
        for (const requestId of inspectedRequestIds) {
          if (requestId === intent.requestId) continue;
          const prior = await mappings.getByRequestId(requestId);
          if (!prior?.capture2Intent) continue;
          try {
            const status = await client.inspect({ captureId: prior.captureId, ownerToken: prior.ownerToken });
            if (["starting", "recording", "stopping"].includes(status.state))
              throw new Error("Another Record capture is active; request ID was not reserved");
          } catch {
            throw new Error("Another Record capture is unresolved; request ID was not reserved");
          }
        }
        ensureLive();
        const destination = intent.destinationPath === undefined
          ? await destinations.allocate(intent.requestId)
          : await destinations.reserveExact(intent.requestId, intent.destinationPath);
        ensureLive();
        const mapping: CaptureMapping = { requestId: intent.requestId, captureId: randomUUID().replaceAll("-", ""), ownerToken: randomBytes(32).toString("hex"), bundleId: destination.bundleId, bundlePath: destination.bundlePath, capture2Intent: serializedIntent };
        const winner = await mappings.putCapture2IfUnchanged(mapping, inspectedRequestIds);
        ensureLive();
        if (!isDeepStrictEqual(winner, mapping)) {
          if (winner.capture2Intent !== serializedIntent) throw new Error("Record capture identity conflicts with its durable request binding");
          cache.set(intent.requestId, Object.freeze({ ...winner }));
          try { return observe(winner, await client.inspect({ captureId: winner.captureId, ownerToken: winner.ownerToken })); }
          catch { ensureLive(); unknown(intent.requestId); throw new Error("Capture start outcome is unknown; inspect the same Record request to reconcile"); }
        }
        cache.set(intent.requestId, Object.freeze({ ...winner }));
        const reserved: Capture2Receipt = { requestId: intent.requestId, phase: "reserved" };
        phases.set(intent.requestId, "reserved");
        receipts.set(intent.requestId, reserved);
        emit({ type: "capture-changed", receipt: structuredClone(reserved) });
        phases.set(intent.requestId, "submitted-unknown");
        active.add(intent.requestId);
        const startRevision = observations.get(intent.requestId) ?? 0;
        try {
          ensureLive();
          const status = await client.start({ captureId: winner.captureId, ownerToken: winner.ownerToken, bundleId: winner.bundleId, bundlePath: winner.bundlePath, source: structuredClone(intent.source), options: structuredClone(intent.options), invocation: structuredClone(intent.invocation) });
          return observe(winner, status);
        } catch {
          ensureLive();
          if ((observations.get(intent.requestId) ?? 0) !== startRevision) return structuredClone(receipts.get(intent.requestId)!);
          unknown(intent.requestId);
          throw new Error("Capture start outcome is unknown; inspect the same Record request to reconcile");
        }
      });
      const flight = { intent: serializedIntent, promise: work };
      inflight.set(intent.requestId, flight);
      void work.finally(() => { freshAdmission = false; if (inflight.get(intent.requestId) === flight) inflight.delete(intent.requestId); }).catch(() => undefined);
      return work.then(value => structuredClone(value));
    },
    inspect: requestId => call("inspect", requestId),
    stop: requestId => call("stop", requestId),
    cancel: requestId => call("cancel", requestId),
    async reconcile() {
      ensureLive();
      const ids = await mappings.listRequestIds("capture2");
      ensureLive();
      const results: Capture2Receipt[] = [];
      for (const id of ids) {
        const row = await mappings.getByRequestId(id);
        if (!row?.capture2Intent) continue;
        try { results.push(await call("inspect", id)); }
        catch { ensureLive(); results.push(unknown(id)); }
      }
      return Object.freeze(results.map(value => Object.freeze(structuredClone(value))));
    },
    subscribe(listener) { if (disposed) return () => undefined; listeners.add(listener); return () => listeners.delete(listener); },
    dispose() { disposed = true; listeners.clear(); },
  };
}
