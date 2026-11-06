import type { Capture2Event, Capture2Host, Capture2Phase, Capture2Receipt, Capture2StartIntent } from "../capture2.js";
import type { SourceDiscovery } from "@apparent/engine-sdk/capture";
import { isDeepStrictEqual } from "node:util";
import type { RecordCommand, RecordState } from "../../workflow/controller.js";
import type { HostStartupState, OwnershipReport, RecordHostRuntimePort, StartupIssue } from "../runtime.js";
import type { RecordCapture2StartPort, RecordStartOutcome } from "../runtime.js";

export interface Capture2RecordRuntimeOptions {
  host: Capture2Host;
  /** Called only after fresh discovery. Return the exact source, SDK intent, and output destination. */
  createStartIntent(requestId: string, discovery: SourceDiscovery): Omit<Capture2StartIntent, "requestId" | "destinationPath"> & { destinationPath: string };
  pollIntervalMs?: number;
}

const terminal = new Set<Capture2Phase>(["cancelled", "failed"]);
const active = new Set<Capture2Phase>(["reserved", "submitted-unknown", "starting", "recording", "stopping", "unknown"]);
const publicState = (receipt: Capture2Receipt, readiness: RecordState["readiness"]): RecordState => {
  const phase: RecordState["phase"] = receipt.phase === "starting" || receipt.phase === "recording" || receipt.phase === "stopping" || receipt.phase === "cancelled" || receipt.phase === "failed"
    ? receipt.phase
    : receipt.phase === "materials-ready" || receipt.phase === "processing" ? "processing"
      : "uncertain";
  return phase === "failed" || phase === "uncertain"
    ? { phase, captureId: receipt.requestId, readiness, message: phase === "failed" ? "Native capture failed" : `Capture state is ${receipt.phase}` }
    : { phase, captureId: receipt.requestId, readiness };
};

/** Bridges the published capture/2 host into Record's existing provider runtime seam. */
export async function createCapture2RecordRuntime(options: Capture2RecordRuntimeOptions): Promise<RecordHostRuntimePort & RecordCapture2StartPort> {
  const interval = options.pollIntervalMs ?? 750;
  if (!Number.isSafeInteger(interval) || interval < 100 || interval > 10_000) throw new Error("Capture2 observer interval must be between 100 and 10000ms");
  let disposed = false;
  let readiness: RecordState["readiness"] = { ready: false, reason: "Native capture readiness has not been checked" };
  let state: RecordState = { phase: "idle", readiness };
  let startup: HostStartupState = { state: "blocked", knownRequestIds: [], unresolved: [] };
  let reconciliation: OwnershipReport["reconciliation"] = "pending";
  let timer: ReturnType<typeof setTimeout> | undefined;
  let polling = false;
  let startInFlight = false;
  let operationRevision = 0;
  const listeners = new Set<(value: RecordState) => void>();
  const known = new Map<string, Capture2Receipt>();
  const publish = (next: RecordState) => {
    if (disposed) return;
    state = Object.freeze({ ...next, readiness: Object.freeze({ ...next.readiness }) });
    for (const listener of [...listeners]) if (listeners.has(listener)) try { listener(state); } catch { listeners.delete(listener); }
    schedulePoll();
  };
  const accept = (receipt: Capture2Receipt) => {
    if (disposed) return;
    const previous = known.get(receipt.requestId);
    const rank: Record<Capture2Phase, number> = { reserved: 0, "submitted-unknown": 1, unknown: 1, starting: 2, recording: 3, stopping: 4, "materials-ready": 5, processing: 6, cancelled: 7, failed: 7 };
    if (previous && rank[receipt.phase] < rank[previous.phase]) return;
    if (previous && terminal.has(previous.phase) && receipt.phase !== previous.phase) return;
    known.set(receipt.requestId, structuredClone(receipt));
    if (active.has(receipt.phase)) {
      const unresolved = startup.state === "blocked" ? startup.unresolved.filter(item => item.requestId !== receipt.requestId) : [];
      unresolved.push({ requestId: receipt.requestId, state: receipt.phase === "unknown" || receipt.phase === "submitted-unknown" ? "unknown" : "active", phase: receipt.phase });
      startup = { state: "blocked", knownRequestIds: [...new Set([...startup.knownRequestIds, receipt.requestId])].sort(), unresolved };
    }
    const currentOwnsActiveCapture = "captureId" in state && ["checking", "starting", "recording", "stopping", "cancelling", "stop-failed", "uncertain"].includes(state.phase);
    if (currentOwnsActiveCapture && "captureId" in state && state.captureId !== receipt.requestId && active.has(receipt.phase)) return;
    if ("captureId" in state && state.captureId === receipt.requestId && terminal.has(state.phase as Capture2Phase) && !terminal.has(receipt.phase)) return;
    if (!("captureId" in state) || state.captureId === receipt.requestId || active.has(receipt.phase)) publish(publicState(receipt, readiness));
  };
  const onEvent = (event: Capture2Event) => {
    if (event.type === "readiness-changed") {
      ++operationRevision;
      readiness = event.readiness.ready ? { ready: true } : { ready: false, reason: event.readiness.reason ?? "Native Record capture/2 unavailable" };
      publish({ ...state, readiness });
    } else if (event.type === "capture-changed") { ++operationRevision; accept(event.receipt); }
  };
  const schedulePoll = () => {
    if (disposed || timer || polling || ![...known.values()].some(receipt => active.has(receipt.phase))) return;
    timer = setTimeout(() => { timer = undefined; void poll(); }, interval);
    timer.unref?.();
  };
  // A host may synchronously deliver its snapshot during subscribe().
  const unsubscribe = options.host.subscribe(onEvent);
  const poll = async () => {
    if (disposed || polling) return;
    polling = true;
    try {
      const candidates = [...known.values()].filter(receipt => active.has(receipt.phase));
      for (const item of candidates) {
        if (disposed) return;
        try { accept(await options.host.inspect(item.requestId)); } catch { /* unknown ownership remains active */ }
      }
    } finally { polling = false; schedulePoll(); }
  };
  const readinessResult = await options.host.readiness();
  if (!disposed) { readiness = readinessResult.ready ? { ready: true } : { ready: false, reason: readinessResult.reason ?? "Native Record capture/2 unavailable" }; publish({ ...state, readiness }); }
  const reconcileStartup = async (): Promise<HostStartupState> => {
    if (disposed) return startup;
    const scanRevision = operationRevision;
    reconciliation = "pending";
    let receipts: readonly Capture2Receipt[];
    try { receipts = await options.host.reconcile(); }
    catch { reconciliation = "failed"; startup = { state: "blocked", knownRequestIds: [...known.keys()], unresolved: [...known.keys()].map(requestId => ({ requestId, state: "unknown" })) }; return startup; }
    if (disposed) return startup;
    for (const receipt of receipts) accept(receipt);
    const unresolved: StartupIssue[] = [...receipts, ...known.values()].filter(receipt => active.has(receipt.phase)).map(receipt => ({ requestId: receipt.requestId, state: receipt.phase === "unknown" || receipt.phase === "submitted-unknown" ? "unknown" : "active", phase: receipt.phase }));
    if (operationRevision === scanRevision) startup = unresolved.length ? { state: "blocked", knownRequestIds: [...new Set([...known.keys(), ...receipts.map(receipt => receipt.requestId)])].sort(), unresolved }
      : { state: "ready", knownRequestIds: [...new Set([...known.keys(), ...receipts.map(receipt => receipt.requestId)])].sort() };
    else {
      const latest = [...known.values()].filter(receipt => active.has(receipt.phase)).map(receipt => ({ requestId: receipt.requestId, state: receipt.phase === "unknown" || receipt.phase === "submitted-unknown" ? "unknown" as const : "active" as const, phase: receipt.phase }));
      startup = latest.length ? { state: "blocked", knownRequestIds: [...new Set([...startup.knownRequestIds, ...known.keys()])].sort(), unresolved: latest }
        : { state: "ready", knownRequestIds: [...new Set([...startup.knownRequestIds, ...known.keys()])].sort() };
    }
    reconciliation = "complete";
    return startup;
  };
  const releaseReservation = (requestId: string, reservation: Capture2Receipt) => {
    if (known.get(requestId) !== reservation) return;
    known.delete(requestId);
    const unresolved: StartupIssue[] = [...known.values()].filter(receipt => active.has(receipt.phase)).map(receipt => ({ requestId: receipt.requestId, state: receipt.phase === "unknown" || receipt.phase === "submitted-unknown" ? "unknown" : "active", phase: receipt.phase }));
    startup = unresolved.length
      ? { state: "blocked", knownRequestIds: startup.knownRequestIds.filter(id => id !== requestId), unresolved }
      : { state: "ready", knownRequestIds: startup.knownRequestIds.filter(id => id !== requestId) };
  };
  await reconcileStartup();
  const performStartCapture = async (requestId: string): Promise<{ state: RecordState; outcome: RecordStartOutcome }> => {
    if (disposed || startup.state !== "ready" || !readiness.ready || [...known.values()].some(receipt => active.has(receipt.phase))) return { state, outcome: "refused" };
    ++operationRevision;
    const reservation: Capture2Receipt = { requestId, phase: "submitted-unknown" };
    known.set(requestId, reservation);
    startup = { state: "blocked", knownRequestIds: [...new Set([...startup.knownRequestIds, requestId])].sort(), unresolved: [{ requestId, state: "unknown", phase: "submitted-unknown" }] };
    let selected: Omit<Capture2StartIntent, "requestId" | "destinationPath"> & { destinationPath: string };
    try {
      const discovery = await options.host.discoverSources();
      if (disposed || discovery.state !== "available") { releaseReservation(requestId, reservation); return { state, outcome: "refused" }; }
      selected = structuredClone(options.createStartIntent(requestId, structuredClone(discovery)));
      if (!selected || !discovery.sources.some(source => isDeepStrictEqual(source, selected.source))) { releaseReservation(requestId, reservation); return { state, outcome: "refused" }; }
    } catch { releaseReservation(requestId, reservation); return { state, outcome: "refused" }; }
    if (disposed) return { state, outcome: "refused" };
    publish({ phase: "checking", captureId: requestId, readiness });
    try {
      const receipt = await options.host.start({ ...selected, requestId });
      if (!disposed) accept(receipt);
      return { state, outcome: receipt.phase === "unknown" || receipt.phase === "submitted-unknown" ? "outcome-unknown" : "accepted" };
    } catch (error) {
      const definitelyNotDispatched = !!error && typeof error === "object" && (error as { dispatched?: unknown }).dispatched === false;
      if (!disposed && definitelyNotDispatched) releaseReservation(requestId, reservation);
      if (!definitelyNotDispatched && !disposed && known.get(requestId) === reservation) { known.set(requestId, { requestId, phase: "unknown" }); startup = { state: "blocked", knownRequestIds: [...new Set([...startup.knownRequestIds, requestId])].sort(), unresolved: [{ requestId, state: "unknown", phase: "unknown" }] }; publish({ phase: "uncertain", captureId: requestId, readiness, message: error instanceof Error ? error.message : String(error) }); }
      return { state, outcome: definitelyNotDispatched ? "refused" : "outcome-unknown" };
    }
  };
  const startCapture = async (requestId: string): Promise<{ state: RecordState; outcome: RecordStartOutcome }> => {
    if (startInFlight) return { state, outcome: "refused" };
    startInFlight = true;
    try { return await performStartCapture(requestId); }
    finally { startInFlight = false; }
  };
  const dispatch = async (command: RecordCommand): Promise<RecordState> => {
    if (disposed) return state;
    if (command.type === "refresh-readiness") {
      const value = await options.host.readiness();
      if (!disposed) { readiness = value.ready ? { ready: true } : { ready: false, reason: value.reason ?? "Native Record capture/2 unavailable" }; publish({ ...state, readiness }); }
      return state;
    }
    if (command.type === "start") {
      await startCapture(command.requestId);
      return state;
    }
    if (command.type === "stop" || command.type === "retry-stop" || command.type === "cancel" || command.type === "reconcile") {
      const captureId = "captureId" in state ? state.captureId : undefined;
      if (!captureId) return state;
      const commandRevision = operationRevision;
      try {
        const receipt = command.type === "cancel" ? await options.host.cancel(captureId)
          : command.type === "reconcile" ? await options.host.inspect(captureId)
            : await options.host.stop(captureId);
        accept(receipt);
        await reconcileStartup();
      } catch {
        const stillOwnsOperation = !disposed && operationRevision === commandRevision && "captureId" in state && state.captureId === captureId && !["cancelled", "failed", "processing"].includes(state.phase);
        if (stillOwnsOperation) publish({ phase: "uncertain", captureId, readiness, message: "Capture control outcome is unknown" });
      }
      return state;
    }
    return state;
  };
  return {
    get startup() { return structuredClone(startup); },
    get state() { return state; },
    subscribe(listener) { if (!disposed) listeners.add(listener); return () => listeners.delete(listener); },
    dispatch,
    startCapture,
    reconcileStartup,
    unresolvedOwnership() { return { requestIds: [...(startup.state === "blocked" ? startup.unresolved.map(item => item.requestId) : [])], storageUnavailable: false, reconciliation }; },
    captures() { return [...known.values()].map(receipt => publicState(receipt, readiness)); },
    completionEligibleCaptureIds() { return [...known.values()].filter(receipt => receipt.phase === "materials-ready" || receipt.phase === "processing").map(receipt => receipt.requestId); },
    canStartCapture() { return !disposed && startup.state === "ready" && ![...known.values()].some(receipt => active.has(receipt.phase)); },
    dispose() {
      if (!disposed) { disposed = true; if (timer) clearTimeout(timer); timer = undefined; unsubscribe(); listeners.clear(); reconciliation = reconciliation === "pending" ? "interrupted" : reconciliation; }
      return { requestIds: startup.state === "blocked" ? startup.unresolved.map(item => item.requestId) : [], storageUnavailable: false, reconciliation };
    },
  };
}
